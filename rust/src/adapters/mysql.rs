use super::{DatabaseAdapter, SharedDbHandle};
use crate::types::{MetadataRequest, MetadataType, QueryResult};
use anyhow::Result;
use async_trait::async_trait;
use mysql_async::prelude::Queryable;
use mysql_async::{Opts, OptsBuilder, Pool, PoolConstraints, Row, Value as MyValue};
use serde_json::{Map, Value};
use url::Url;

pub struct MySqlAdapter {
    url: String,
    pool: Option<Pool>,
}

impl MySqlAdapter {
    pub fn new(url: String) -> Self {
        Self { url, pool: None }
    }

    async fn ensure_pool(&mut self) -> Result<&Pool> {
        if self.pool.is_none() {
            let opts = build_opts(&self.url)?;
            self.pool = Some(Pool::new(opts));
        }
        Ok(self.pool.as_ref().expect("pool just initialized"))
    }
}

#[async_trait]
impl DatabaseAdapter for MySqlAdapter {
    async fn connect(&mut self) -> Result<()> {
        let pool = self.ensure_pool().await?.clone();
        let mut conn = pool.get_conn().await?;
        conn.query_drop("SELECT 1").await?;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(pool) = self.pool.take() {
            pool.disconnect().await?;
        }
        Ok(())
    }

    async fn test(&mut self) -> Result<()> {
        self.execute("select 1").await.map(|_| ())
    }

    async fn execute(&mut self, command: &str) -> Result<QueryResult> {
        let pool = self.ensure_pool().await?.clone();
        query_with_pool(&pool, command).await
    }

    async fn metadata(&mut self, request: MetadataRequest) -> Result<QueryResult> {
        let pool = self.ensure_pool().await?.clone();
        metadata_with_pool(&pool, request).await
    }

    fn shared_handle(&self) -> Option<SharedDbHandle> {
        self.pool
            .as_ref()
            .map(|pool| SharedDbHandle::Mysql(pool.clone()))
    }
}

pub(crate) async fn query_with_pool(pool: &Pool, command: &str) -> Result<QueryResult> {
    let mut conn = pool.get_conn().await?;
    match conn.query::<Row, _>(command).await {
        Ok(rows) => Ok(rows_to_result(rows)),
        Err(error) => {
            let _ = conn.disconnect().await;
            Err(error.into())
        }
    }
}

pub(crate) async fn metadata_with_pool(
    pool: &Pool,
    request: MetadataRequest,
) -> Result<QueryResult> {
    match request.request_type {
        MetadataType::Tables => query_with_pool(pool, "show tables").await,
        MetadataType::Columns => {
            let table = request
                .table
                .ok_or_else(|| anyhow::anyhow!("columns 元信息查询必须提供 --table"))?
                .replace('`', "``");
            query_with_pool(pool, &format!("show columns from `{}`", table)).await
        }
        _ => anyhow::bail!("当前数据库不支持元信息类型: {:?}", request.request_type),
    }
}

fn rows_to_result(rows: Vec<Row>) -> QueryResult {
    let fields = rows
        .first()
        .map(|row| {
            row.columns_ref()
                .iter()
                .map(|c| c.name_str().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let values = rows.into_iter().map(row_to_json).collect::<Vec<_>>();
    QueryResult {
        row_count: Some(values.len() as u64),
        rows: values,
        fields: Some(fields),
    }
}

fn row_to_json(row: Row) -> Value {
    let columns = row.columns_ref().to_vec();
    let values = row.unwrap();
    let mut object = Map::new();
    for (index, column) in columns.iter().enumerate() {
        object.insert(
            column.name_str().to_string(),
            mysql_value_to_json(values.get(index).cloned().unwrap_or(MyValue::NULL)),
        );
    }
    Value::Object(object)
}

fn mysql_value_to_json(value: MyValue) -> Value {
    match value {
        MyValue::NULL => Value::Null,
        MyValue::Bytes(bytes) => Value::String(String::from_utf8_lossy(&bytes).to_string()),
        MyValue::Int(value) => Value::Number(value.into()),
        MyValue::UInt(value) => Value::Number(value.into()),
        MyValue::Float(value) => serde_json::Number::from_f64(value as f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        MyValue::Double(value) => serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        other => Value::String(format!("{:?}", other)),
    }
}

fn build_opts(value: &str) -> Result<Opts> {
    let normalized = normalize_mysql_url(value)?;
    let has_pool_max = Url::parse(value)?
        .query_pairs()
        .any(|(key, _)| key == "pool_max");
    let opts = Opts::from_url(&normalized)?;
    if has_pool_max {
        return Ok(opts);
    }
    // Keep URL-derived PoolOpts (ttl / reset / etc.); only fill default max when pool_max is unset.
    // Clamp min so it never exceeds the default max=4 (mysql_async defaults can be min>4).
    let constraints = opts.pool_opts().constraints();
    let min = constraints.min().min(4);
    let pool_opts = opts.pool_opts().clone().with_constraints(
        PoolConstraints::new(min, 4).ok_or_else(|| {
            anyhow::anyhow!("invalid mysql pool constraints")
        })?,
    );
    Ok(Opts::from(
        OptsBuilder::from_opts(opts).pool_opts(pool_opts),
    ))
}

fn normalize_mysql_url(value: &str) -> Result<String> {
    let mut parsed = Url::parse(value)?;
    let supported = [
        "pool_min",
        "pool_max",
        "inactive_connection_ttl",
        "ttl_check_interval",
        "conn_ttl",
        "tcp_keepalive_time_ms",
        "tcp_connect_timeout_ms",
        "stmt_cache_size",
        "prefer_socket",
        "socket",
        "compression",
        "ssl-mode",
    ];
    let pairs = parsed
        .query_pairs()
        .filter(|(key, _)| supported.contains(&key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    if !pairs.is_empty() {
        let query = pairs
            .into_iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect::<Vec<_>>()
            .join("&");
        parsed.set_query(Some(&query));
    }
    Ok(parsed.to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pool_max_preserves_other_pool_opts() {
        let opts = build_opts(
            "mysql://user:pass@127.0.0.1:3306/db?inactive_connection_ttl=60&pool_min=1",
        )
        .unwrap();
        assert_eq!(opts.pool_opts().constraints().min(), 1);
        assert_eq!(opts.pool_opts().constraints().max(), 4);
        assert_eq!(
            opts.pool_opts().inactive_connection_ttl(),
            std::time::Duration::from_secs(60)
        );
    }

    #[test]
    fn default_pool_max_clamps_min_when_above_four() {
        let opts = build_opts(
            "mysql://user:pass@127.0.0.1:3306/db?pool_min=10",
        )
        .unwrap();
        assert_eq!(opts.pool_opts().constraints().min(), 4);
        assert_eq!(opts.pool_opts().constraints().max(), 4);
    }

    #[test]
    fn default_pool_max_on_bare_url() {
        let opts = build_opts("mysql://user:pass@127.0.0.1:3306/db").unwrap();
        assert_eq!(opts.pool_opts().constraints().max(), 4);
        assert!(opts.pool_opts().constraints().min() <= 4);
    }

    #[test]
    fn explicit_pool_max_is_respected() {
        let opts = build_opts("mysql://user:pass@127.0.0.1:3306/db?pool_max=8&pool_min=2").unwrap();
        assert_eq!(opts.pool_opts().constraints().min(), 2);
        assert_eq!(opts.pool_opts().constraints().max(), 8);
    }
}
