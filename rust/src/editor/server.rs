use crate::editor::config_store;
use crate::editor::control;
use crate::editor::http::{self, ApiFailure, HttpResponse, Request, TOKEN_HEADER};
use crate::editor::now_millis;
use crate::editor::EditorState;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::time::{sleep, timeout, Duration};

/// 内嵌编辑页面：源码按片段放在 `rust/assets/editor/`，由 `build.rs` 拼成单文件后内嵌，
/// 交付物仍然是纯 HTML、不依赖任何外部资源，也不引入构建期 JS 工具链。
pub(crate) const INDEX_HTML: &str = include_str!(concat!(env!("OUT_DIR"), "/editor.html"));

/// 页面资源全部内联，用 CSP 锁死外部请求，并禁止被任何页面内嵌。
const HTML_CSP: &str = "default-src 'none'; connect-src 'self'; img-src 'self' data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);
/// 退出前等在途请求收尾的上限：正好撞上空闲退出的保存不该被切断
const INFLIGHT_GRACE: Duration = Duration::from_secs(2);
/// 空闲多久自动退出：页面在用户有操作时会定期打点，停手超过这个时间就收摊。
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);
/// 空闲检查的轮询间隔上限；实际间隔会按配置的空闲时长收窄，测试里才能快速验证。
const IDLE_TICK: Duration = Duration::from_secs(30);

pub async fn run_editor_server(
    port: u16,
    token: Option<String>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    run_editor_server_with_idle(port, token, config_path, IDLE_TIMEOUT).await
}

/// 同 `run_editor_server`，但可以指定空闲退出时长，方便测试。
pub async fn run_editor_server_with_idle(
    port: u16,
    token: Option<String>,
    config_path: Option<PathBuf>,
    idle_timeout: Duration,
) -> Result<()> {
    let config_path = match config_path {
        Some(path) => path,
        None => crate::config::resolve_config_path()?,
    };
    let token = token.unwrap_or_else(crate::editor::generate_token);
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("编辑服务无法监听 127.0.0.1:{port}"))?;
    let bound_port = listener.local_addr()?.port();
    let state = EditorState {
        pid: std::process::id(),
        port: bound_port,
        token: token.clone(),
        url: format!("http://127.0.0.1:{bound_port}/#t={token}"),
        config_path: config_path.display().to_string(),
        started_at: now_millis(),
    };
    control::write_state(&state)?;
    eprintln!("agent-database-cli 编辑服务已启动: {}", state.url);

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    // 在途请求计数，退出前要等它归零
    let inflight = Arc::new(AtomicUsize::new(0));
    // 每次请求都会刷新这个时间点，页面在用户有操作时会定期打点
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let idle_handle = {
        let last_activity = Arc::clone(&last_activity);
        let shutdown_tx = shutdown_tx.clone();
        let tick = (idle_timeout / 4).clamp(Duration::from_millis(20), IDLE_TICK);
        tokio::spawn(async move {
            loop {
                sleep(tick).await;
                let idle = last_activity
                    .lock()
                    .map(|since| since.elapsed())
                    .unwrap_or_default();
                if idle >= idle_timeout {
                    eprintln!(
                        "agent-database-cli 编辑服务空闲 {} 秒，自动退出",
                        idle.as_secs()
                    );
                    let _ = shutdown_tx.send(true);
                    return;
                }
            }
        })
    };

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => break,
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(error) => {
                        debug_log(&format!("接受连接失败: {error}"));
                        continue;
                    }
                };
                let token = token.clone();
                let config_path = config_path.clone();
                let shutdown_tx = shutdown_tx.clone();
                let last_activity = Arc::clone(&last_activity);
                let inflight = Arc::clone(&inflight);
                tokio::spawn(async move {
                    inflight.fetch_add(1, Ordering::SeqCst);
                    if let Ok(mut since) = last_activity.lock() {
                        *since = Instant::now();
                    }
                    let result = timeout(
                        CONNECTION_TIMEOUT,
                        handle_connection(stream, &token, &config_path, bound_port, &shutdown_tx),
                    )
                    .await;
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => debug_log(&format!("处理请求失败: {error}")),
                        Err(_) => debug_log("处理请求超时，已断开连接"),
                    }
                    inflight.fetch_sub(1, Ordering::SeqCst);
                });
            }
        }
    }

    idle_handle.abort();
    // 先等正在处理的请求收尾（例如刚好撞上空闲退出的保存），再留出刷写时间
    let drain_deadline = Instant::now() + INFLIGHT_GRACE;
    while inflight.load(Ordering::SeqCst) > 0 && Instant::now() < drain_deadline {
        sleep(Duration::from_millis(20)).await;
    }
    sleep(SHUTDOWN_GRACE).await;
    control::clear_state(state.pid, &state.token)?;
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    token: &str,
    config_path: &Path,
    port: u16,
    shutdown_tx: &watch::Sender<bool>,
) -> Result<()> {
    let (request, mut stream) = match http::read_request(stream).await {
        Ok(value) => value,
        Err(error) => {
            debug_log(&format!("读取请求失败: {error}"));
            return Ok(());
        }
    };
    let response = route(&request, token, config_path, port, shutdown_tx).await;
    // 调试时能看到页面是否在打点，方便确认空闲逻辑
    debug_log(&format!(
        "{} {} -> {}",
        request.method, request.path, response.status
    ));
    http::write_response(&mut stream, &response).await
}

async fn route(
    request: &Request,
    token: &str,
    config_path: &Path,
    port: u16,
    shutdown_tx: &watch::Sender<bool>,
) -> HttpResponse {
    if !http::is_local_host(request) {
        return HttpResponse::failure(ApiFailure::bad_request("只允许本机访问"));
    }
    let method = request.method.as_str();
    let path = request.path.as_str();

    if method == "GET" && (path == "/" || path == "/index.html") {
        return HttpResponse::html(INDEX_HTML, vec![("content-security-policy", HTML_CSP)]);
    }
    if !path.starts_with("/api/") {
        return HttpResponse::failure(ApiFailure::not_found(format!("未知路径: {path}")));
    }
    if method != "GET" && method != "POST" {
        return HttpResponse::failure(ApiFailure::method_not_allowed(format!(
            "不支持的请求方法: {method}"
        )));
    }
    if request.header(TOKEN_HEADER) != Some(token) {
        return HttpResponse::failure(ApiFailure::unauthorized(
            "访问令牌无效，请重新执行 agent-database-cli editor start 获取页面地址",
        ));
    }

    match (method, path) {
        ("GET", "/api/health") => HttpResponse::json(200, &health_payload(port, config_path)),
        ("GET", "/api/config") => match config_store::load_config_payload(config_path) {
            Ok(payload) => HttpResponse::json(200, &payload),
            Err(failure) => HttpResponse::failure(failure),
        },
        ("POST", "/api/config") => match request
            .json_body()
            .and_then(|body| config_store::save_config_payload(config_path, &body))
        {
            Ok(payload) => HttpResponse::json(200, &payload),
            Err(failure) => HttpResponse::failure(failure),
        },
        // 页面上点眼睛时按需解密，不在加载配置时批量下发明文
        ("POST", "/api/secret") => match request
            .json_body()
            .and_then(|body| config_store::load_secret_payload(config_path, &body))
        {
            Ok(payload) => HttpResponse::json(200, &payload),
            Err(failure) => HttpResponse::failure(failure),
        },
        ("POST", "/api/test-connection") => match request.json_body() {
            Ok(body) => match test_connection(config_path, &body).await {
                Ok(payload) => HttpResponse::json(200, &payload),
                Err(failure) => HttpResponse::failure(failure),
            },
            Err(failure) => HttpResponse::failure(failure),
        },
        ("POST", "/api/shutdown") => {
            let _ = shutdown_tx.send(true);
            HttpResponse::json(200, &json!({ "stopping": true }))
        }
        _ => HttpResponse::failure(ApiFailure::not_found(format!("未知接口: {path}"))),
    }
}

/// 用单次连接管理器测试当前草稿，避免写配置或复用 daemon 中的旧连接。
async fn test_connection(path: &Path, body: &Value) -> Result<Value, ApiFailure> {
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiFailure::bad_request("连接名称不能为空"))?;
    let db: crate::types::DatabaseConfig = serde_json::from_value(body["database"].clone())
        .map_err(|_| ApiFailure::bad_request("连接配置格式不正确，请检查字段"))?;
    // SQLcl 会启动可配置的程序并执行脚本；编辑器接口不开放这条执行路径。
    if db.db_type == crate::types::DatabaseType::Oracle
        && !matches!(
            db.oracle_driver,
            Some(crate::types::OracleDriver::Oracle | crate::types::OracleDriver::Oracledb)
        )
    {
        return Err(ApiFailure::bad_request(
            "Oracle SQLcl 请保存后使用 CLI 的 test 命令",
        ));
    }
    // 与显隐凭据接口保持一致，只接受当前配置中已经引用的密钥。
    let tunnel = db.ssh_tunnel.as_ref();
    for reference in [
        db.password_ref.as_deref(),
        tunnel.and_then(|value| value.password_ref.as_deref()),
        tunnel.and_then(|value| value.passphrase_ref.as_deref()),
    ]
    .into_iter()
    .flatten()
    {
        config_store::load_secret_payload(path, &json!({ "ref": reference }))?;
    }
    let mut config = crate::types::AppConfig {
        databases: [(name.to_owned(), db)].into(),
    };
    crate::config::validate_config(&config)
        .map_err(|error| ApiFailure::bad_request(error.to_string()))?;
    crate::config::resolve_secret_refs(path, &mut config)
        .map_err(|_| ApiFailure::bad_request("无法读取已保存的凭据，请重新填写后测试"))?;
    let manager = crate::daemon::connection_manager::ConnectionManager::new(config);
    // 给 HTTP 请求留出接收和回写时间；驱动原始错误可能包含凭据，不直接返回。
    let result = timeout(Duration::from_secs(10), manager.test(name)).await;
    match timeout(Duration::from_secs(1), manager.close_all()).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => debug_log("测试连接清理失败"),
        Err(_) => debug_log("测试连接清理超时"),
    }
    match result {
        Ok(Ok(payload)) => Ok(payload),
        Ok(Err(_)) => Err(ApiFailure::bad_request("连接失败，请检查地址、凭据和网络")),
        Err(_) => Err(ApiFailure::bad_request("连接测试超时，请检查地址和网络")),
    }
}

fn health_payload(port: u16, config_path: &Path) -> Value {
    json!({
        "ok": true,
        "pid": std::process::id(),
        "port": port,
        "version": env!("CARGO_PKG_VERSION"),
        "configPath": config_path.display().to_string(),
    })
}

fn debug_log(message: &str) {
    if std::env::var("AGENT_DATABASE_CLI_DEBUG").is_ok() {
        eprintln!("[editor] {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn request(method: &str, path: &str, headers: &[(&str, &str)]) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            body: Vec::new(),
        }
    }

    fn shutdown() -> watch::Sender<bool> {
        watch::channel(false).0
    }

    #[tokio::test]
    async fn page_is_served_without_token_but_locked_down_by_csp() {
        let path = std::env::temp_dir().join("agent-database-cli-editor-route.json");
        let response = route(&request("GET", "/", &[]), "secret", &path, 1, &shutdown()).await;

        assert_eq!(response.status, 200);
        assert!(response
            .extra_headers
            .iter()
            .any(|(name, value)| *name == "content-security-policy"
                && value.contains("frame-ancestors 'none'")));
    }

    #[tokio::test]
    async fn api_requires_token() {
        let path = std::env::temp_dir().join("agent-database-cli-editor-route.json");
        let without_token = route(
            &request("GET", "/api/config", &[]),
            "secret",
            &path,
            1,
            &shutdown(),
        )
        .await;
        assert_eq!(without_token.status, 401);

        let wrong_token = route(
            &request("GET", "/api/config", &[(TOKEN_HEADER, "other")]),
            "secret",
            &path,
            1,
            &shutdown(),
        )
        .await;
        assert_eq!(wrong_token.status, 401);
    }

    #[tokio::test]
    async fn test_connection_checks_draft_without_saving() -> Result<()> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let dir = crate::editor::test_support::TempDir::new("editor-test-connection");
        let path = dir.path().join("config.json");
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            // 消费 Redis 客户端初始化命令，再回复实际的 PING。
            while reader.read_line(&mut line).await? != 0 {
                let count: usize = line.trim_end().trim_start_matches('*').parse()?;
                let mut ping = false;
                for index in 0..count {
                    line.clear();
                    reader.read_line(&mut line).await?;
                    let size: usize = line.trim_end().trim_start_matches('$').parse()?;
                    let mut argument = vec![0; size + 2];
                    reader.read_exact(&mut argument).await?;
                    ping |= index == 0 && argument == b"PING\r\n";
                }
                let reply: &[u8] = if ping { b"+PONG\r\n" } else { b"+OK\r\n" };
                reader.get_mut().write_all(reply).await?;
                if ping {
                    return Ok::<(), anyhow::Error>(());
                }
                line.clear();
            }
            anyhow::bail!("未收到 PING")
        });
        let mut request = request("POST", "/api/test-connection", &[(TOKEN_HEADER, "secret")]);
        request.body = serde_json::to_vec(&json!({
            "name": "draft",
            "database": {"type": "redis", "url": format!("redis://{address}/")}
        }))?;
        assert_eq!(
            route(&request, "wrong-token", &path, 1, &shutdown())
                .await
                .status,
            401
        );
        let response = route(&request, "secret", &path, 1, &shutdown()).await;
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        server.await??;

        // 同一地址的测试服务已退出，重测应失败，不复用旧连接。
        let response = route(&request, "secret", &path, 1, &shutdown()).await;
        assert_eq!(response.status, 400);
        assert!(!path.exists());
        Ok(())
    }
    #[tokio::test]
    async fn test_connection_rejects_invalid_drafts_and_unknown_secrets() {
        let dir = crate::editor::test_support::TempDir::new("editor-invalid-test");
        let path = dir.path().join("config.json");
        for body in [
            json!({ "name": "", "database": {} }),
            json!({ "name": "draft", "database": {"type": "redis", "url": ""} }),
            json!({ "name": "draft", "database": {
                "type": "redis", "url": "redis://127.0.0.1/", "passwordRef": "unknown"
            } }),
            json!({ "name": "draft", "database": {
                "type": "oracle", "url": "oracle://user:pass@127.0.0.1/service"
            } }),
        ] {
            assert_eq!(test_connection(&path, &body).await.unwrap_err().status, 400);
        }
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn health_reports_running_process() {
        let path = std::env::temp_dir().join("agent-database-cli-editor-route.json");
        let response = route(
            &request("GET", "/api/health", &[(TOKEN_HEADER, "secret")]),
            "secret",
            &path,
            52341,
            &shutdown(),
        )
        .await;
        assert_eq!(response.status, 200);
        let payload: Value = serde_json::from_slice(&response.body).expect("解析响应失败");
        assert_eq!(payload["ok"], json!(true));
        assert_eq!(payload["port"], json!(52341));
    }

    /// 起服务 → 请求续命 → 停手后自动退出，验证空闲逻辑两个方向都对。
    #[tokio::test]
    async fn idle_timeout_closes_server_when_no_request_arrives() -> Result<()> {
        let _guard = crate::editor::state_env_lock().lock().await;
        let dir = crate::editor::test_support::TempDir::new("editor-idle");
        std::env::set_var(crate::editor::STATE_DIR_ENV, dir.path());

        let config_path = dir.path().join("config.json");
        fs::write(&config_path, "{\"databases\":{}}")?;
        let token = crate::editor::generate_token();
        let server_token = token.clone();
        let task = tokio::spawn(run_editor_server_with_idle(
            0,
            Some(server_token),
            Some(config_path),
            Duration::from_millis(400),
        ));

        // 等状态文件拿到真实端口
        let mut port = 0u16;
        for _ in 0..100 {
            if let Ok(Some(state)) = control::read_state() {
                if state.token == token {
                    port = state.port;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(port > 0, "编辑服务未启动");

        // 空闲窗口内打一次点，服务应继续存活；不打点则超时退出
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            probe_status(port, &token).await?,
            200,
            "空闲窗口内应仍然可用"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(probe_status(port, &token).await?, 200, "请求应刷新空闲计时");

        let mut exited = false;
        for _ in 0..60 {
            if control::read_state()?.is_none() {
                exited = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        std::env::remove_var(crate::editor::STATE_DIR_ENV);
        let _ = task.await;
        assert!(exited, "空闲超时后服务应退出并清理状态文件");
        Ok(())
    }

    /// 直接手写一个 GET，避免测试再引入 HTTP 客户端。
    async fn probe_status(port: u16, token: &str) -> Result<u16> {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        let request = format!(
            "GET /api/health HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\nconnection: close\r\n{TOKEN_HEADER}: {token}\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        let head = String::from_utf8_lossy(&raw);
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| anyhow!("响应状态行非法"))?;
        Ok(status)
    }

    #[test]
    fn page_script_has_no_dangling_calls() {
        // 页面脚本没法在 cargo test 里执行，这里静态检查「调用了未声明的本地函数」
        let script =
            crate::editor::page_check::extract_script(INDEX_HTML).expect("页面缺少 script");
        let missing = crate::editor::page_check::undefined_calls(&script);
        assert!(
            missing.is_empty(),
            "页面脚本调用了未声明的名字: {missing:?}"
        );
    }

    #[test]
    fn page_uses_lucide_icons_and_no_emoji() {
        // 设计约束：图标统一 Lucide，界面全程禁止表情符号
        assert!(INDEX_HTML.contains("Lucide"));
        assert!(INDEX_HTML.contains("<symbol id=\"i-database\""));
        assert!(!INDEX_HTML
            .chars()
            .any(|ch| ('\u{1F300}'..='\u{1FAFF}').contains(&ch)));
    }

    #[test]
    fn empty_config_template_is_valid_json() {
        assert!(serde_json::from_str::<Value>(config_store::EMPTY_CONFIG_RAW).is_ok());
    }
}
