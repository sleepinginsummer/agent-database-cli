use crate::adapters::{create_adapter, DatabaseAdapter};
use crate::config::get_database_config;
use crate::security::assert_command_allowed;
use crate::ssh_tunnel::{start_ssh_tunnel, StartedSshTunnel};
use crate::types::{AppConfig, DatabaseConfig, MetadataRequest, QueryResult};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;

struct Entry {
    adapter: Box<dyn DatabaseAdapter>,
    config: DatabaseConfig,
    tunnel: Option<StartedSshTunnel>,
    last_used: Instant,
}

enum EntrySlot {
    Ready(Arc<Mutex<Entry>>),
    Initializing(Arc<Notify>),
}

pub struct ConnectionManager {
    config: AppConfig,
    entries: Mutex<HashMap<String, EntrySlot>>,
}

impl ConnectionManager {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub async fn test(&self, name: &str) -> Result<Value> {
        let entry = self.get_entry(name).await?;
        let mut entry = entry.lock().await;
        let timeout_secs = query_timeout_secs();
        match timeout(Duration::from_secs(timeout_secs), entry.adapter.test()).await {
            Ok(Ok(())) => {
                entry.last_used = Instant::now();
                Ok(json!({ "ok": true }))
            }
            Ok(Err(error)) => Err(error),
            Err(_) => {
                let _ = entry.adapter.disconnect().await;
                anyhow::bail!(
                    "查询超时（>{}s）：客户端已停止等待；请检查 SQL / 索引，或用 EXPLAIN 评估。Oracle 原生驱动会同时设置 OCI call timeout",
                    timeout_secs
                );
            }
        }
    }

    pub async fn execute(&self, name: &str, command: &str) -> Result<QueryResult> {
        let config = get_database_config(&self.config, name)?;
        assert_command_allowed(config, command)?;
        let entry = self.get_entry(name).await?;
        let mut entry = entry.lock().await;
        let timeout_secs = query_timeout_secs();
        match timeout(
            Duration::from_secs(timeout_secs),
            entry.adapter.execute(command),
        )
        .await
        {
            Ok(Ok(result)) => {
                entry.last_used = Instant::now();
                Ok(result)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => {
                let _ = entry.adapter.disconnect().await;
                anyhow::bail!(
                    "查询超时（>{}s）：请检查 SQL 是否缺索引条件，或用 EXPLAIN 评估",
                    timeout_secs
                );
            }
        }
    }

    pub async fn metadata(&self, name: &str, request: MetadataRequest) -> Result<QueryResult> {
        let entry = self.get_entry(name).await?;
        let mut entry = entry.lock().await;
        let timeout_secs = query_timeout_secs();
        match timeout(
            Duration::from_secs(timeout_secs),
            entry.adapter.metadata(request),
        )
        .await
        {
            Ok(Ok(result)) => {
                entry.last_used = Instant::now();
                Ok(result)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => {
                let _ = entry.adapter.disconnect().await;
                anyhow::bail!(
                    "查询超时（>{}s）：请检查 SQL 是否缺索引条件，或用 EXPLAIN 评估",
                    timeout_secs
                );
            }
        }
    }

    pub async fn reset(&self, name: &str) -> Result<Value> {
        let entry = match self.entries.lock().await.remove(name) {
            Some(EntrySlot::Ready(entry)) => Some(entry),
            Some(EntrySlot::Initializing(notify)) => {
                notify.notify_waiters();
                None
            }
            None => None,
        };
        if let Some(entry) = entry {
            let mut entry = entry.lock().await;
            entry.adapter.disconnect().await?;
        }
        Ok(json!({ "reset": name }))
    }

    pub async fn close_all(&self) -> Result<()> {
        let entries = self
            .entries
            .lock()
            .await
            .drain()
            .filter_map(|(_, slot)| match slot {
                EntrySlot::Ready(entry) => Some(entry),
                EntrySlot::Initializing(notify) => {
                    notify.notify_waiters();
                    None
                }
            })
            .collect::<Vec<_>>();
        for entry in entries {
            let mut entry = entry.lock().await;
            entry.adapter.disconnect().await?;
        }
        Ok(())
    }

    pub async fn cleanup_idle(&self) -> Result<()> {
        let now = Instant::now();
        let entries = self.entries.lock().await;
        let mut expired = Vec::new();
        for (name, slot) in entries.iter() {
            let EntrySlot::Ready(entry) = slot else {
                continue;
            };
            let Ok(entry) = entry.try_lock() else {
                continue;
            };
            let keep_alive = Duration::from_secs(entry.config.keep_alive_seconds.unwrap_or(600));
            if now.duration_since(entry.last_used) >= keep_alive {
                expired.push(name.clone());
            }
        }
        drop(entries);
        for name in expired {
            self.reset(&name).await?;
        }
        Ok(())
    }

    pub async fn status(&self) -> Value {
        let entries = self.entries.lock().await;
        let mut connections = Vec::new();
        for (name, slot) in entries.iter() {
            let EntrySlot::Ready(entry) = slot else {
                connections.push(json!({
                    "name": name,
                    "initializing": true,
                }));
                continue;
            };
            let Ok(entry) = entry.try_lock() else {
                connections.push(json!({
                    "name": name,
                    "busy": true,
                }));
                continue;
            };
            connections.push(json!({
                "name": name,
                "type": format!("{:?}", entry.config.db_type).to_lowercase(),
                "keepAliveSeconds": entry.config.keep_alive_seconds.unwrap_or(600),
                "sshTunnel": entry.tunnel.is_some(),
                "busy": false,
            }));
        }
        json!({ "connections": connections })
    }

    async fn get_entry(&self, name: &str) -> Result<Arc<Mutex<Entry>>> {
        loop {
            let notify = {
                let mut entries = self.entries.lock().await;
                match entries.get(name) {
                    Some(EntrySlot::Ready(entry)) => return Ok(entry.clone()),
                    Some(EntrySlot::Initializing(notify)) => notify.clone(),
                    None => {
                        let notify = Arc::new(Notify::new());
                        entries.insert(name.to_string(), EntrySlot::Initializing(notify.clone()));
                        drop(entries);
                        return self.initialize_entry(name, notify).await;
                    }
                }
            };
            notify.notified().await;
        }
    }

    async fn initialize_entry(&self, name: &str, notify: Arc<Notify>) -> Result<Arc<Mutex<Entry>>> {
        let entry = match create_entry(&self.config, name).await {
            Ok(entry) => Arc::new(Mutex::new(entry)),
            Err(error) => {
                self.remove_initializing_slot(name, &notify).await;
                notify.notify_waiters();
                return Err(error);
            }
        };
        let mut entries = self.entries.lock().await;
        let still_current = matches!(
            entries.get(name),
            Some(EntrySlot::Initializing(current)) if Arc::ptr_eq(current, &notify)
        );
        if !still_current {
            drop(entries);
            entry.lock().await.adapter.disconnect().await?;
            anyhow::bail!("数据库连接初始化已取消: {name}");
        }
        entries.insert(name.to_string(), EntrySlot::Ready(entry.clone()));
        notify.notify_waiters();
        Ok(entry)
    }

    async fn remove_initializing_slot(&self, name: &str, notify: &Arc<Notify>) {
        let mut entries = self.entries.lock().await;
        let should_remove = matches!(
            entries.get(name),
            Some(EntrySlot::Initializing(current)) if Arc::ptr_eq(current, notify)
        );
        if should_remove {
            entries.remove(name);
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

async fn create_entry(config: &AppConfig, name: &str) -> Result<Entry> {
    let config = get_database_config(config, name)?.clone();
    let tunnel = start_ssh_tunnel(&config).await?;
    let mut adapter_config = config.clone();
    if let Some(tunnel) = &tunnel {
        adapter_config.url = tunnel.url.clone();
        if tunnel.redis_cluster.is_some() {
            adapter_config.redis_cluster = tunnel.redis_cluster.clone();
        }
    }
    let mut adapter = create_adapter(&adapter_config)?;
    adapter.connect().await?;
    Ok(Entry {
        adapter,
        config,
        tunnel,
        last_used: Instant::now(),
    })
}
