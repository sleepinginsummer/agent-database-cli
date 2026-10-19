use super::DatabaseAdapter;
use crate::types::{MetadataRequest, MetadataType, QueryResult};
use anyhow::Result;
use async_trait::async_trait;
use oracle::Connection;
use serde_json::{Map, Value};
use tokio::task;
use url::Url;

pub struct OracleAdapter {
    url: String,
}

impl OracleAdapter {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    async fn query(&mut self, command: &str) -> Result<QueryResult> {
        let url = self.url.clone();
        let command = command.to_string();
        let timeout_secs = query_timeout_secs();
        task::spawn_blocking(move || {
            let connection = connect_oracle(&url)?;
            // OCI call timeout cancels work in the blocking pool, not only the Tokio deadline.
            connection.set_call_timeout(Some(std::time::Duration::from_secs(timeout_secs)))?;
            execute_query(&connection, &command)
        })
        .await?
    }
}

#[async_trait]
impl DatabaseAdapter for OracleAdapter {
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
        self.query(command).await
    }

    async fn metadata(&mut self, request: MetadataRequest) -> Result<QueryResult> {
        match request.request_type {
            MetadataType::Tables => {
                self.query("select table_name from user_tables order by table_name")
                    .await
            }
            MetadataType::Columns => {
                let table = request
                    .table
                    .ok_or_else(|| anyhow::anyhow!("columns 元信息查询必须提供 --table"))?
                    .replace('\'', "''")
                    .to_uppercase();
                self.query(&format!("select table_name, column_name, data_type from user_tab_columns where table_name = '{}' order by column_id", table)).await
            }
            _ => anyhow::bail!("当前数据库不支持元信息类型: {:?}", request.request_type),
        }
    }
}

fn query_timeout_secs() -> u64 {
    std::env::var("AGENT_DB_QUERY_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(60)
}

fn connect_oracle(url: &str) -> Result<Connection> {
    let parsed = Url::parse(url)?;
    let user = percent_decode(parsed.username())?;
    let password = percent_decode(parsed.password().unwrap_or(""))?;
    let port = parsed.port().unwrap_or(1521);
    let service = parsed.path().trim_start_matches('/');
    let connect_string = format!(
        "{}:{}{}",
        parsed.host_str().unwrap_or("localhost"),
        port,
        if service.is_empty() {
            String::new()
        } else {
            format!("/{service}")
        }
    );
    Ok(Connection::connect(user, password, connect_string)?)
}

fn percent_decode(value: &str) -> Result<String> {
    Ok(url::form_urlencoded::parse(value.as_bytes())
        .next()
        .map(|(k, _)| k.into_owned())
        .unwrap_or_else(|| value.to_string()))
}

fn execute_query(connection: &Connection, command: &str) -> Result<QueryResult> {
    let rows = connection.query(command, &[])?;
    let column_names = rows
        .column_info()
        .iter()
        .map(|info| info.name().to_string())
        .collect::<Vec<_>>();
    let mut values = Vec::new();
    for row_result in rows {
        let row = row_result?;
        let mut object = Map::new();
        for (index, column) in column_names.iter().enumerate() {
            let value: Option<String> = row.get(index)?;
            object.insert(
                column.clone(),
                value.map(Value::String).unwrap_or(Value::Null),
            );
        }
        values.push(Value::Object(object));
    }
    Ok(QueryResult {
        row_count: Some(values.len() as u64),
        rows: values,
        fields: Some(column_names),
    })
}
