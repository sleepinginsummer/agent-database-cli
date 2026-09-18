use super::DatabaseAdapter;
use crate::types::{MetadataRequest, MetadataType, QueryResult};
use crate::utils::masking::mask_secret;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::{fs, path::Path, process::Stdio};
use tokio::process::Command;
use url::Url;
use uuid::Uuid;

pub struct OracleSqlclAdapter {
    url: String,
    sqlcl_path: String,
    java_home: Option<String>,
}

impl OracleSqlclAdapter {
    pub fn new(url: String, sqlcl_path: String, java_home: Option<String>) -> Self {
        Self {
            url,
            sqlcl_path,
            java_home,
        }
    }

    async fn run_sqlcl(&self, command: &str) -> Result<QueryResult> {
        let markers = Markers::new();
        let dir = std::env::temp_dir().join(format!("agent-database-cli-sqlcl-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir)?;
        let script_path = dir.join("command.sql");
        fs::write(&script_path, self.build_script(command, &markers))?;
        let result = self.spawn_sqlcl(&script_path, &markers).await;
        let _ = fs::remove_dir_all(&dir);
        result
    }

    async fn spawn_sqlcl(&self, script_path: &Path, markers: &Markers) -> Result<QueryResult> {
        let mut command = Command::new(&self.sqlcl_path);
        command
            .arg("-S")
            .arg("/nolog")
            .arg(format!("@{}", script_path.display()))
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(java_home) = &self.java_home {
            command.env("JAVA_HOME", java_home);
        }
        let child = command
            .spawn()
            .map_err(|error| anyhow::anyhow!("SQLcl 启动失败: {}", error))?;
        let output = child
            .wait_with_output()
            .await
            .map_err(|error| anyhow::anyhow!("SQLcl 等待失败: {}", error))?;
        let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout));
        let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
        let combined = [stdout.trim(), stderr.trim()]
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if debug_enabled() {
            eprintln!(
                "[rust-db-cli] sqlcl stdout={} stderr={}",
                mask_secret(stdout.trim()),
                mask_secret(stderr.trim())
            );
        }
        if contains_sqlcl_error(&combined) || !output.status.success() {
            anyhow::bail!("{}", mask_secret(&format!("SQLcl 执行失败: {combined}")));
        }
        parse_sqlcl_output(&stdout, markers)
    }

    fn build_script(&self, command: &str, markers: &Markers) -> String {
        format!("set heading on\nset feedback off\nset pagesize 50000\nset linesize 32767\nset sqlformat json\nwhenever sqlerror exit sql.sqlcode\nconnect {}\nprompt {}\n{};\nprompt {}\nexit", self.build_connect_string(), markers.begin, normalize_sqlcl_sql(command), markers.end)
    }

    fn build_connect_string(&self) -> String {
        let parsed = Url::parse(&self.url).expect("Oracle URL 已在配置阶段校验");
        let user = percent_decode(parsed.username());
        let password = quote_password(&percent_decode(parsed.password().unwrap_or("")));
        let service = parsed.path().trim_start_matches('/');
        format!(
            "{user}/{password}@//{}:{}/{}",
            parsed.host_str().unwrap_or("localhost"),
            parsed.port().unwrap_or(1521),
            service
        )
    }
}

#[async_trait]
impl DatabaseAdapter for OracleSqlclAdapter {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
    async fn test(&mut self) -> Result<()> {
        self.execute("select 1 from dual").await.map(|_| ())
    }
    async fn execute(&mut self, command: &str) -> Result<QueryResult> {
        self.run_sqlcl(command).await
    }
    async fn metadata(&mut self, request: MetadataRequest) -> Result<QueryResult> {
        match request.request_type {
            MetadataType::Tables => {
                self.execute("select table_name from user_tables order by table_name")
                    .await
            }
            MetadataType::Columns => {
                let table = request
                    .table
                    .ok_or_else(|| anyhow::anyhow!("columns 元信息查询必须提供 --table"))?
                    .replace('\'', "''")
                    .to_uppercase();
                self.execute(&format!("select table_name, column_name, data_type from user_tab_columns where table_name = '{}' order by column_id", table)).await
            }
            _ => anyhow::bail!("当前数据库不支持元信息类型: {:?}", request.request_type),
        }
    }
}

struct Markers {
    begin: String,
    end: String,
}
impl Markers {
    fn new() -> Self {
        let id = Uuid::new_v4();
        Self {
            begin: format!("__AGENT_DATABASE_CLI_BEGIN_{id}__"),
            end: format!("__AGENT_DATABASE_CLI_END_{id}__"),
        }
    }
}
fn normalize_sqlcl_sql(command: &str) -> String {
    command.trim().trim_end_matches(';').to_string()
}
fn quote_password(password: &str) -> String {
    format!("\"{}\"", password.replace('"', "\\\""))
}
fn percent_decode(value: &str) -> String {
    url::form_urlencoded::parse(value.as_bytes())
        .next()
        .map(|(value, _)| value.into_owned())
        .unwrap_or_else(|| value.to_string())
}
fn strip_ansi(value: &str) -> String {
    regex::Regex::new(r"\x1b\[[0-9;]*m")
        .unwrap()
        .replace_all(value, "")
        .to_string()
}
fn contains_sqlcl_error(value: &str) -> bool {
    ["ORA-", "SP2-", "SQL Error"]
        .iter()
        .any(|item| value.contains(item))
}
fn debug_enabled() -> bool {
    std::env::var("AGENT_DATABASE_CLI_DEBUG").is_ok()
}
fn parse_sqlcl_output(stdout: &str, markers: &Markers) -> Result<QueryResult> {
    let payload = extract_marked_output(stdout, markers).unwrap_or(stdout);
    let json_start = find_json_start(payload);
    if let Some(start) = json_start {
        let slice = &payload[start..];
        if let Some(value) = parse_first_json_value(slice) {
            return Ok(sqlcl_json_to_query_result(value));
        }
    }
    let text = payload.trim();
    Ok(QueryResult {
        rows: vec![serde_json::json!({ "output": text })],
        fields: Some(vec!["output".to_string()]),
        row_count: if text.is_empty() { Some(0) } else { Some(1) },
    })
}

fn extract_marked_output<'a>(stdout: &'a str, markers: &Markers) -> Option<&'a str> {
    let begin = stdout.find(&markers.begin)? + markers.begin.len();
    let end = stdout[begin..].find(&markers.end)? + begin;
    Some(&stdout[begin..end])
}

fn parse_first_json_value(value: &str) -> Option<Value> {
    serde_json::Deserializer::from_str(value.trim())
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok)
}

fn find_json_start(value: &str) -> Option<usize> {
    match (value.find('{'), value.find('[')) {
        (Some(object), Some(array)) => Some(object.min(array)),
        (Some(object), None) => Some(object),
        (None, Some(array)) => Some(array),
        (None, None) => None,
    }
}

fn sqlcl_json_to_query_result(value: Value) -> QueryResult {
    if let Some(result) = value
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| results.first())
    {
        let rows = result
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let fields = result
            .get("columns")
            .and_then(Value::as_array)
            .map(|columns| {
                columns
                    .iter()
                    .filter_map(|column| {
                        column
                            .get("name")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|fields| !fields.is_empty());
        return QueryResult {
            row_count: Some(rows.len() as u64),
            rows,
            fields,
        };
    }
    let rows = value.as_array().cloned().unwrap_or_else(|| vec![value]);
    QueryResult {
        row_count: Some(rows.len() as u64),
        rows,
        fields: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sqlcl_output_uses_markers_and_sqlcl_json_shape() {
        let markers = Markers {
            begin: "__BEGIN__".to_string(),
            end: "__END__".to_string(),
        };
        let stdout = r#"
SQLcl: Release 26
__BEGIN__
{"results":[{"columns":[{"name":"ONE","type":"NUMBER"}],"items":[{"one":1}]}]}
__END__
Disconnected from Oracle Database
"#;

        let result = parse_sqlcl_output(stdout, &markers).unwrap();

        assert_eq!(result.row_count, Some(1));
        assert_eq!(result.fields, Some(vec!["ONE".to_string()]));
        assert_eq!(result.rows, vec![serde_json::json!({ "one": 1 })]);
    }

    #[test]
    fn parse_sqlcl_output_ignores_trailing_text_after_json() {
        let markers = Markers {
            begin: "__BEGIN__".to_string(),
            end: "__END__".to_string(),
        };
        let stdout = "__BEGIN__\n[{\"ONE\":1}]\n1 row selected.\n__END__";

        let result = parse_sqlcl_output(stdout, &markers).unwrap();

        assert_eq!(result.row_count, Some(1));
        assert_eq!(result.rows, vec![serde_json::json!({ "ONE": 1 })]);
    }

    #[test]
    fn build_connect_string_decodes_url_credentials() {
        let adapter = OracleSqlclAdapter::new(
            "oracle://test%40user:p%40ss%2Fword@127.0.0.1:1521/service".to_string(),
            "sql".to_string(),
            None,
        );

        assert_eq!(
            adapter.build_connect_string(),
            "test@user/\"p@ss/word\"@//127.0.0.1:1521/service"
        );
    }
}
