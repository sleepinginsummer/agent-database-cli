use crate::types::{DatabaseConfig, DatabaseType, MetadataRequest, QueryResult};
use anyhow::Result;
use async_trait::async_trait;

pub mod mongodb;
pub mod mysql;
pub mod oracle_driver;
pub mod oracle_sqlcl;
pub mod postgres;
pub mod redis_driver;

#[derive(Clone)]
pub enum SharedDbHandle {
    Mysql(mysql_async::Pool),
}

impl SharedDbHandle {
    pub async fn test(&self) -> Result<()> {
        self.execute("select 1").await.map(|_| ())
    }

    pub async fn execute(&self, command: &str) -> Result<QueryResult> {
        match self {
            SharedDbHandle::Mysql(pool) => mysql::query_with_pool(pool, command).await,
        }
    }

    pub async fn metadata(&self, request: MetadataRequest) -> Result<QueryResult> {
        match self {
            SharedDbHandle::Mysql(pool) => mysql::metadata_with_pool(pool, request).await,
        }
    }
}

#[async_trait]
pub trait DatabaseAdapter: Send {
    async fn connect(&mut self) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;
    async fn test(&mut self) -> Result<()>;
    async fn execute(&mut self, command: &str) -> Result<QueryResult>;
    async fn metadata(&mut self, request: MetadataRequest) -> Result<QueryResult>;

    /// When present, the connection manager can run queries without holding the entry lock.
    fn shared_handle(&self) -> Option<SharedDbHandle> {
        None
    }
}

pub fn create_adapter(config: &DatabaseConfig) -> Result<Box<dyn DatabaseAdapter>> {
    match config.db_type {
        DatabaseType::Mysql => Ok(Box::new(mysql::MySqlAdapter::new(config.url.clone()))),
        DatabaseType::Postgres => Ok(Box::new(postgres::PostgresAdapter::new(config.url.clone()))),
        DatabaseType::Redis => Ok(Box::new(redis_driver::RedisAdapter::new(
            config.url.clone(),
            config.redis_cluster.clone(),
        ))),
        DatabaseType::Mongodb => Ok(Box::new(mongodb::MongoDbAdapter::new(
            config.url.clone(),
            config.database.clone(),
        ))),
        DatabaseType::Oracle => {
            if matches!(
                config.oracle_driver,
                Some(crate::types::OracleDriver::Oracle | crate::types::OracleDriver::Oracledb)
            ) {
                Ok(Box::new(oracle_driver::OracleAdapter::new(
                    config.url.clone(),
                )))
            } else {
                Ok(Box::new(oracle_sqlcl::OracleSqlclAdapter::new(
                    config.url.clone(),
                    config
                        .sqlcl_path
                        .clone()
                        .unwrap_or_else(|| "sql".to_string()),
                    config.java_home.clone(),
                )))
            }
        }
    }
}
