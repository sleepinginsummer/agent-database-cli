use crate::daemon::config_manager::DaemonConfigManager;
use crate::types::{DaemonAction, DaemonRequest, DaemonResponse};
use crate::utils::masking::to_error_message;
use anyhow::Result;
use futures::FutureExt;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{panic::AssertUnwindSafe, sync::Arc};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

fn daemon_idle_seconds() -> u64 {
    std::env::var("AGENT_DB_DAEMON_IDLE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1800)
}

pub async fn run_server() -> Result<()> {
    #[cfg(unix)]
    {
        let runtime_dir = crate::daemon::paths::runtime_dir()?;
        tokio::fs::create_dir_all(&runtime_dir).await?;
        let socket_path = crate::daemon::paths::socket_path()?;
        let _ = tokio::fs::remove_file(&socket_path).await;
        let listener = UnixListener::bind(&socket_path)?;
        let pid_path = crate::daemon::paths::pid_path()?;
        tokio::fs::write(&pid_path, std::process::id().to_string()).await?;

        let manager = Arc::new(Mutex::new(DaemonConfigManager::new()));
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let active_requests = Arc::new(AtomicUsize::new(0));
        spawn_idle_shutdown(
            manager.clone(),
            last_activity.clone(),
            active_requests.clone(),
            socket_path.clone(),
            pid_path.clone(),
        );

        loop {
            let (stream, _) = listener.accept().await?;
            let manager = manager.clone();
            let last_activity = last_activity.clone();
            let active_requests = active_requests.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    handle_stream(stream, manager, last_activity, active_requests).await
                {
                    debug_log(&format!(
                        "daemon 请求处理失败: {}",
                        to_error_message(&error)
                    ));
                }
            });
        }
    }
    #[cfg(windows)]
    {
        let runtime_dir = crate::daemon::paths::runtime_dir()?;
        tokio::fs::create_dir_all(&runtime_dir).await?;
        let pipe_name = crate::daemon::paths::socket_path_string()?;
        let pid_path = crate::daemon::paths::pid_path()?;
        tokio::fs::write(&pid_path, std::process::id().to_string()).await?;

        let manager = Arc::new(Mutex::new(DaemonConfigManager::new()));
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let active_requests = Arc::new(AtomicUsize::new(0));
        spawn_idle_shutdown(
            manager.clone(),
            last_activity.clone(),
            active_requests.clone(),
            pid_path.clone(),
        );

        loop {
            let server = ServerOptions::new().create(&pipe_name)?;
            server.connect().await?;
            let manager = manager.clone();
            let last_activity = last_activity.clone();
            let active_requests = active_requests.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    handle_stream(server, manager, last_activity, active_requests).await
                {
                    debug_log(&format!(
                        "daemon 请求处理失败: {}",
                        to_error_message(&error)
                    ));
                }
            });
        }
    }
}

#[cfg(unix)]
async fn handle_stream(
    stream: UnixStream,
    manager: Arc<Mutex<DaemonConfigManager>>,
    last_activity: Arc<Mutex<Instant>>,
    active_requests: Arc<AtomicUsize>,
) -> Result<()> {
    handle_duplex_stream(stream, manager, last_activity, active_requests).await
}

#[cfg(windows)]
async fn handle_stream(
    stream: NamedPipeServer,
    manager: Arc<Mutex<DaemonConfigManager>>,
    last_activity: Arc<Mutex<Instant>>,
    active_requests: Arc<AtomicUsize>,
) -> Result<()> {
    handle_duplex_stream(stream, manager, last_activity, active_requests).await
}

async fn handle_duplex_stream<S>(
    stream: S,
    manager: Arc<Mutex<DaemonConfigManager>>,
    last_activity: Arc<Mutex<Instant>>,
    active_requests: Arc<AtomicUsize>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    *last_activity.lock().await = Instant::now();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    active_requests.fetch_add(1, Ordering::SeqCst);
    let response = build_response(line.trim(), manager).await;
    active_requests.fetch_sub(1, Ordering::SeqCst);
    *last_activity.lock().await = Instant::now();
    let mut stream = reader.into_inner();
    stream
        .write_all(serde_json::to_string(&response)?.as_bytes())
        .await?;
    stream.write_all(b"\n").await?;
    Ok(())
}

async fn build_response(payload: &str, manager: Arc<Mutex<DaemonConfigManager>>) -> DaemonResponse {
    let request = match serde_json::from_str::<DaemonRequest>(payload) {
        Ok(request) => request,
        Err(error) => {
            return DaemonResponse {
                ok: false,
                data: None,
                error: Some(error.to_string()),
            };
        }
    };

    // 单个请求内部的业务错误必须通过协议写回，避免客户端误报 daemon 无响应。
    match AssertUnwindSafe(handle_request(request, manager))
        .catch_unwind()
        .await
    {
        Ok(Ok(data)) => DaemonResponse {
            ok: true,
            data: Some(data),
            error: None,
        },
        Ok(Err(error)) => DaemonResponse {
            ok: false,
            data: None,
            error: Some(to_error_message(&error)),
        },
        Err(payload) => DaemonResponse {
            ok: false,
            data: None,
            error: Some(format!("daemon 请求处理 panic: {}", panic_message(payload))),
        },
    }
}

async fn handle_request(
    request: DaemonRequest,
    manager: Arc<Mutex<DaemonConfigManager>>,
) -> Result<Value> {
    match request.action {
        DaemonAction::Status => {
            let current = manager.lock().await.current_manager();
            match current {
                Some(current) => Ok(current.status().await),
                None => Ok(json!({ "connections": [] })),
            }
        }
        DaemonAction::Stop => {
            tokio::spawn(async {
                sleep(Duration::from_millis(20)).await;
                std::process::exit(0);
            });
            Ok(json!({ "stopped": true }))
        }
        DaemonAction::Test
        | DaemonAction::Execute
        | DaemonAction::Metadata
        | DaemonAction::Reset => {
            let db = request
                .db
                .ok_or_else(|| anyhow::anyhow!("daemon 请求必须提供 db"))?;
            let current = {
                let mut guard = manager.lock().await;
                guard.get_manager(request.config_path).await?
            };
            match request.action {
                DaemonAction::Test => current.test(&db).await,
                DaemonAction::Execute => {
                    let command = request
                        .command
                        .ok_or_else(|| anyhow::anyhow!("execute 请求必须提供 command"))?;
                    Ok(serde_json::to_value(current.execute(&db, &command).await?)?)
                }
                DaemonAction::Metadata => {
                    let metadata = request
                        .metadata
                        .ok_or_else(|| anyhow::anyhow!("metadata 请求必须提供 metadata"))?;
                    Ok(serde_json::to_value(
                        current.metadata(&db, metadata).await?,
                    )?)
                }
                DaemonAction::Reset => current.reset(&db).await,
                _ => unreachable!(),
            }
        }
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return message.to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "未知 panic".to_string()
}

fn debug_log(message: &str) {
    if std::env::var("AGENT_DATABASE_CLI_DEBUG").is_ok() {
        eprintln!("[agent-database-cli daemon] {message}");
    }
}

#[cfg(unix)]
fn spawn_idle_shutdown(
    manager: Arc<Mutex<DaemonConfigManager>>,
    last_activity: Arc<Mutex<Instant>>,
    active_requests: Arc<AtomicUsize>,
    socket_path: std::path::PathBuf,
    pid_path: std::path::PathBuf,
) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;
            if active_requests.load(Ordering::SeqCst) > 0 {
                continue;
            }
            let mut guard = manager.lock().await;
            let _ = guard.cleanup_idle().await;
            if active_requests.load(Ordering::SeqCst) > 0 {
                continue;
            }
            let idle_for = Instant::now().duration_since(*last_activity.lock().await);
            if idle_for >= Duration::from_secs(daemon_idle_seconds()) {
                let _ = guard.close_all().await;
                let _ = tokio::fs::remove_file(&socket_path).await;
                let _ = tokio::fs::remove_file(&pid_path).await;
                std::process::exit(0);
            }
        }
    });
}

#[cfg(windows)]
fn spawn_idle_shutdown(
    manager: Arc<Mutex<DaemonConfigManager>>,
    last_activity: Arc<Mutex<Instant>>,
    active_requests: Arc<AtomicUsize>,
    pid_path: std::path::PathBuf,
) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(5)).await;
            if active_requests.load(Ordering::SeqCst) > 0 {
                continue;
            }
            let mut guard = manager.lock().await;
            let _ = guard.cleanup_idle().await;
            if active_requests.load(Ordering::SeqCst) > 0 {
                continue;
            }
            let idle_for = Instant::now().duration_since(*last_activity.lock().await);
            if idle_for >= Duration::from_secs(daemon_idle_seconds()) {
                let _ = guard.close_all().await;
                let _ = tokio::fs::remove_file(&pid_path).await;
                std::process::exit(0);
            }
        }
    });
}
