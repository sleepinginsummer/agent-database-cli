use super::DatabaseAdapter;
use crate::security;
use crate::types::{MetadataRequest, MetadataType, QueryResult, RedisClusterConfig};
use anyhow::Result;
use async_trait::async_trait;
use redis::{aio::MultiplexedConnection, Client, RedisResult, Value as RedisValue};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio::time::{timeout, Duration};
use url::Url;

const REDIS_SCAN_BATCH_SIZE: usize = 500;

pub struct RedisAdapter {
    url: String,
    redis_cluster: Option<RedisClusterConfig>,
    conn: Option<MultiplexedConnection>,
    cluster_conns: HashMap<String, MultiplexedConnection>,
    first_cluster_route: Option<String>,
}

fn debug_enabled() -> bool {
    std::env::var("AGENT_DATABASE_CLI_DEBUG").is_ok()
}

impl RedisAdapter {
    pub fn new(url: String, redis_cluster: Option<RedisClusterConfig>) -> Self {
        Self {
            url,
            redis_cluster,
            conn: None,
            cluster_conns: HashMap::new(),
            first_cluster_route: None,
        }
    }

    async fn run_command(&mut self, parts: &[String]) -> Result<RedisValue> {
        self.connect().await?;
        let mut cmd = redis::cmd(&parts[0]);
        for part in &parts[1..] {
            cmd.arg(part);
        }
        if !self.cluster_conns.is_empty() {
            return self.run_cluster_command(&cmd).await;
        }
        let value: RedisResult<RedisValue> = cmd.query_async(self.conn.as_mut().unwrap()).await;
        Ok(value?)
    }
}

impl RedisAdapter {
    async fn ensure_redirect_connection(
        &mut self,
        target: &str,
    ) -> Result<Option<&mut MultiplexedConnection>> {
        if self.cluster_conns.contains_key(target) {
            return Ok(self.cluster_conns.get_mut(target));
        }
        let Some(cluster) = &self.redis_cluster else {
            return Ok(None);
        };
        let url = if let Some(map) = &cluster.node_address_map {
            let Some(local) = map.get(target) else {
                return Ok(None);
            };
            rewrite_redis_url(&self.url, &local.host, local.port)?
        } else {
            cluster
                .nodes
                .iter()
                .find(|node| match route_key_from_url(node) {
                    Ok(route) => route == target,
                    Err(_) => false,
                })
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Redis Cluster 节点清单缺少重定向目标: {target}"))?
        };
        if debug_enabled() {
            eprintln!(
                "[rust-db-cli] redis connect redirected {} via {}",
                target, url
            );
        }
        let client = Client::open(url.as_str())?;
        let conn = timeout(
            Duration::from_secs(5),
            client.get_multiplexed_async_connection(),
        )
        .await??;
        if debug_enabled() {
            eprintln!("[rust-db-cli] redis connected redirected {}", target);
        }
        self.cluster_conns.insert(target.to_string(), conn);
        Ok(self.cluster_conns.get_mut(target))
    }
    async fn run_cluster_command(&mut self, cmd: &redis::Cmd) -> Result<RedisValue> {
        let route = self
            .first_cluster_route
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Redis Cluster 连接为空"))?;
        if debug_enabled() {
            eprintln!("[rust-db-cli] redis cluster command via {}", route);
        }
        let first_result: RedisResult<RedisValue> = cmd
            .query_async(
                self.cluster_conns
                    .get_mut(&route)
                    .expect("默认 Redis Cluster 连接存在"),
            )
            .await;
        match first_result {
            Ok(value) => Ok(value),
            Err(error) => {
                if debug_enabled() {
                    eprintln!("[rust-db-cli] redis cluster command error: {}", error);
                }
                if let Some((kind, target)) = parse_cluster_redirect(&error.to_string()) {
                    if debug_enabled() {
                        eprintln!(
                            "[rust-db-cli] redis cluster redirect {} -> {}",
                            kind, target
                        );
                    }
                    if let Some(conn) = self.ensure_redirect_connection(&target).await? {
                        if kind == "ASK" {
                            let _: RedisValue =
                                redis::cmd("ASKING").query_async(&mut *conn).await?;
                        }
                        let value: RedisValue = cmd.query_async(conn).await?;
                        return Ok(value);
                    }
                }
                Err(error.into())
            }
        }
    }
}

fn rewrite_redis_url(base: &str, host: &str, port: u16) -> Result<String> {
    let mut parsed = Url::parse(base)?;
    parsed.set_host(Some(host))?;
    parsed
        .set_port(Some(port))
        .map_err(|_| anyhow::anyhow!("Redis URL 端口改写失败"))?;
    Ok(parsed.to_string())
}

fn route_key_from_url(value: &str) -> Result<String> {
    let parsed = Url::parse(value)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Redis Cluster 节点 URL 必须包含 host"))?;
    Ok(format!("{}:{}", host, parsed.port().unwrap_or(6379)))
}

fn parse_cluster_redirect(message: &str) -> Option<(String, String)> {
    let mut parts = message.split_whitespace();
    while let Some(part) = parts.next() {
        // redis-rs 会把服务端错误格式化成 `Moved:` / `Ask:`，这里统一规整后再判断。
        let kind = part.trim_end_matches(':').to_ascii_uppercase();
        if kind == "MOVED" || kind == "ASK" {
            let _slot = parts.next()?;
            return Some((kind, parts.next()?.to_string()));
        }
    }
    None
}

#[async_trait]
impl DatabaseAdapter for RedisAdapter {
    async fn connect(&mut self) -> Result<()> {
        if let Some(cluster) = &self.redis_cluster {
            if self.cluster_conns.is_empty() {
                if let Some(map) = &cluster.node_address_map {
                    let first_route = map
                        .keys()
                        .next()
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Redis Cluster 地址映射不能为空"))?;
                    let local = map.get(&first_route).ok_or_else(|| {
                        anyhow::anyhow!("Redis Cluster 地址映射缺少入口节点: {}", first_route)
                    })?;
                    let url = rewrite_redis_url(&self.url, &local.host, local.port)?;
                    if debug_enabled() {
                        eprintln!(
                            "[rust-db-cli] redis connect mapped entry {} via {}:{}",
                            first_route, local.host, local.port
                        );
                    }
                    let client = Client::open(url.as_str())?;
                    let conn = timeout(
                        Duration::from_secs(5),
                        client.get_multiplexed_async_connection(),
                    )
                    .await??;
                    if debug_enabled() {
                        eprintln!("[rust-db-cli] redis connected mapped entry {}", first_route);
                    }
                    self.cluster_conns.insert(first_route.clone(), conn);
                    self.first_cluster_route = Some(first_route);
                } else {
                    let node = cluster
                        .nodes
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("Redis Cluster 节点不能为空"))?;
                    let route = route_key_from_url(node)?;
                    if debug_enabled() {
                        eprintln!("[rust-db-cli] redis connect cluster entry {}", node);
                    }
                    let client = Client::open(node.as_str())?;
                    let conn = timeout(
                        Duration::from_secs(5),
                        client.get_multiplexed_async_connection(),
                    )
                    .await??;
                    if debug_enabled() {
                        eprintln!("[rust-db-cli] redis connected cluster entry {}", node);
                    }
                    self.cluster_conns.insert(route.clone(), conn);
                    self.first_cluster_route = Some(route);
                }
            }
            return Ok(());
        }
        if self.conn.is_none() {
            let client = Client::open(self.url.as_str())?;
            self.conn = Some(
                timeout(
                    Duration::from_secs(10),
                    client.get_multiplexed_async_connection(),
                )
                .await??,
            );
        }
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.conn = None;
        self.cluster_conns.clear();
        self.first_cluster_route = None;
        Ok(())
    }
    async fn test(&mut self) -> Result<()> {
        self.run_command(&["PING".to_string()]).await.map(|_| ())
    }
    async fn execute(&mut self, command: &str) -> Result<QueryResult> {
        let parts =
            shell_words::split(command).map_err(|_| anyhow::anyhow!("Redis 命令解析失败"))?;
        if parts.is_empty() {
            anyhow::bail!("Redis 命令不能为空");
        }
        let _readonly =
            security::is_read_only_command(&crate::types::DatabaseType::Redis, command)?;
        let result = redis_value_to_json(self.run_command(&parts).await?);
        Ok(QueryResult {
            rows: vec![json!({ "result": result })],
            fields: None,
            row_count: Some(1),
        })
    }
    async fn metadata(&mut self, request: MetadataRequest) -> Result<QueryResult> {
        if request.request_type != MetadataType::Keys {
            anyhow::bail!("Redis 不支持元信息类型: {:?}", request.request_type);
        }
        self.connect().await?;
        let pattern = request.pattern.unwrap_or_else(|| "*".to_string());
        let keys: Vec<String> = if !self.cluster_conns.is_empty() {
            let mut keys = HashSet::new();
            for conn in self.cluster_conns.values_mut() {
                let node_keys = scan_keys(conn, &pattern).await?;
                keys.extend(node_keys);
            }
            keys.into_iter().collect()
        } else {
            scan_keys(self.conn.as_mut().unwrap(), &pattern).await?
        };
        Ok(QueryResult {
            row_count: Some(keys.len() as u64),
            fields: Some(vec!["key".to_string()]),
            rows: keys.into_iter().map(|key| json!({ "key": key })).collect(),
        })
    }
}

async fn scan_keys(conn: &mut MultiplexedConnection, pattern: &str) -> Result<Vec<String>> {
    let mut cursor = 0_u64;
    let mut keys = Vec::new();
    loop {
        let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(REDIS_SCAN_BATCH_SIZE)
            .query_async(conn)
            .await?;
        keys.extend(batch);
        if next_cursor == 0 {
            break;
        }
        cursor = next_cursor;
    }
    Ok(keys)
}

fn redis_value_to_json(value: RedisValue) -> Value {
    match value {
        RedisValue::Nil => Value::Null,
        RedisValue::Int(value) => Value::Number(value.into()),
        RedisValue::BulkString(bytes) => {
            Value::String(String::from_utf8_lossy(bytes.as_ref()).to_string())
        }
        RedisValue::SimpleString(value) => Value::String(value),
        RedisValue::Array(values) => {
            Value::Array(values.into_iter().map(redis_value_to_json).collect())
        }
        other => Value::String(format!("{:?}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::ConnectionAddr;

    #[test]
    fn rediss_url_is_parsed_as_tls_address() {
        let client = Client::open("rediss://cache.example.com:6379").expect("rediss URL 应可解析");
        match &client.get_connection_info().addr {
            ConnectionAddr::TcpTls { insecure, .. } => assert!(!insecure),
            other => panic!("rediss 应解析为 TcpTls，实际为 {:?}", other),
        }
    }

    #[test]
    fn redis_url_is_parsed_as_plain_address() {
        let client = Client::open("redis://cache.example.com:6379").expect("redis URL 应可解析");
        assert!(matches!(
            &client.get_connection_info().addr,
            ConnectionAddr::Tcp(..)
        ));
    }
}
