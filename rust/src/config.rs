use crate::secrets::{decrypt_secret, encrypt_secret};
use crate::types::{AppConfig, DatabaseConfig, DatabaseType, OracleDriver};
use crate::utils::paths::runtime_dir;
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use url::Url;

pub const CONFIG_ENV: &str = "AGENT_DATABASE_CLI_CONFIG";
const SECRET_REF_PREFIX: &str = "agentdbcli:";

pub fn resolve_config_path() -> Result<PathBuf> {
    if let Ok(path) = env::var(CONFIG_ENV) {
        return Ok(PathBuf::from(path));
    }
    Ok(runtime_dir()?.join("config.json"))
}

pub fn load_config(path: Option<PathBuf>) -> Result<AppConfig> {
    let path = match path {
        Some(path) => path,
        None => resolve_config_path()?,
    };
    migrate_plain_secrets(&path)?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let mut config: AppConfig = serde_json::from_str(&raw).context("配置文件不是合法 JSON")?;
    resolve_secret_refs(&path, &mut config)?;
    validate_config(&config)?;
    Ok(config)
}

pub fn validate_config(config: &AppConfig) -> Result<()> {
    for (name, db) in &config.databases {
        validate_database_config(name, db)?;
    }
    Ok(())
}

fn validate_database_config(name: &str, db: &DatabaseConfig) -> Result<()> {
    if db.url.trim().is_empty() {
        anyhow::bail!("数据库配置 {name} 必须提供 url");
    }
    if db.redis_cluster.is_some() && db.db_type != DatabaseType::Redis {
        anyhow::bail!("数据库配置 {name} 只有 redis 类型允许配置 redisCluster");
    }
    if let Some(cluster) = &db.redis_cluster {
        if cluster.nodes.is_empty() {
            anyhow::bail!("数据库配置 {name} 的 redisCluster.nodes 必须是非空数组");
        }
        for (index, node) in cluster.nodes.iter().enumerate() {
            if node.trim().is_empty() {
                anyhow::bail!("数据库配置 {name} 的 redisCluster.nodes[{index}] 必须是非空字符串");
            }
            let parsed = url::Url::parse(node).with_context(|| {
                format!("数据库配置 {name} 的 redisCluster.nodes[{index}] 不是合法 URL")
            })?;
            if parsed.scheme() != "redis" && parsed.scheme() != "rediss" {
                anyhow::bail!(
                    "数据库配置 {name} 的 redisCluster.nodes[{index}] 只支持 redis:// 或 rediss://"
                );
            }
        }
    }
    if let Some(keep_alive) = db.keep_alive_seconds {
        if keep_alive == 0 {
            anyhow::bail!("数据库配置 {name} 的 keepAliveSeconds 必须是正整数");
        }
    }
    if let Some(driver) = &db.oracle_driver {
        if db.db_type != DatabaseType::Oracle {
            anyhow::bail!("数据库配置 {name} 只有 oracle 类型允许配置 oracleDriver");
        }
        match driver {
            OracleDriver::Oracle | OracleDriver::Sqlcl | OracleDriver::Oracledb => {}
        }
    }
    if let Some(tunnel) = &db.ssh_tunnel {
        if tunnel.host.trim().is_empty() {
            anyhow::bail!("数据库配置 {name} 的 sshTunnel.host 必须是非空字符串");
        }
        if tunnel.username.trim().is_empty() {
            anyhow::bail!("数据库配置 {name} 的 sshTunnel.username 必须是非空字符串");
        }
        if tunnel.password.as_deref() == Some("") && tunnel.password_ref.is_none() {
            anyhow::bail!("数据库配置 {name} 的 sshTunnel.password 不能为空字符串");
        }
        if tunnel.private_key_path.as_deref() == Some("") {
            anyhow::bail!("数据库配置 {name} 的 sshTunnel.privateKeyPath 不能为空字符串");
        }
        if tunnel.private_key.as_deref() == Some("") {
            anyhow::bail!("数据库配置 {name} 的 sshTunnel.privateKey 不能为空字符串");
        }
        if tunnel.private_key_path.is_some() && tunnel.private_key.is_some() {
            anyhow::bail!(
                "数据库配置 {name} 的 sshTunnel.privateKeyPath 和 privateKey 只能配置一个"
            );
        }
        if tunnel.password.is_none()
            && tunnel.password_ref.is_none()
            && tunnel.private_key_path.is_none()
            && tunnel.private_key.is_none()
        {
            anyhow::bail!(
                "数据库配置 {name} 的 sshTunnel 必须配置 password、privateKeyPath 或 privateKey"
            );
        }
        if tunnel.passphrase.as_deref() == Some("") && tunnel.passphrase_ref.is_none() {
            anyhow::bail!("数据库配置 {name} 的 sshTunnel.passphrase 不能为空字符串");
        }
        if (tunnel.passphrase.is_some() || tunnel.passphrase_ref.is_some())
            && tunnel.private_key_path.is_none()
            && tunnel.private_key.is_none()
        {
            anyhow::bail!("数据库配置 {name} 的 sshTunnel.passphrase 只能和私钥一起使用");
        }
    }
    Ok(())
}

pub fn get_database_config<'a>(config: &'a AppConfig, name: &str) -> Result<&'a DatabaseConfig> {
    config
        .databases
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("未找到数据库配置: {name}"))
}

pub fn list_supported_databases() -> Vec<&'static str> {
    vec!["mysql", "postgres", "redis", "oracle", "mongodb"]
}

fn secret_ref_for(db_name: &str, field: &str) -> String {
    format!("{SECRET_REF_PREFIX}{db_name}:{field}")
}

fn migrate_plain_secrets(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let mut root: Value = serde_json::from_str(&raw).context("配置文件不是合法 JSON")?;
    let databases = root
        .get_mut("databases")
        .and_then(Value::as_object_mut)
        .context("配置文件缺少 databases 对象")?;
    let mut migrated = false;
    for (name, value) in databases {
        let object = value
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("数据库配置 {name} 必须是对象"))?;
        if migrate_url_password(path, name, object)? {
            migrated = true;
        }
        if migrate_nested_secret(path, name, object, "sshTunnel", "password", "sshPassword")? {
            migrated = true;
        }
        if migrate_nested_secret(
            path,
            name,
            object,
            "sshTunnel",
            "passphrase",
            "sshPassphrase",
        )? {
            migrated = true;
        }
    }
    if migrated {
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&root)?)?;
        fs::rename(tmp, path)?;
    }
    Ok(())
}

fn migrate_url_password(
    path: &Path,
    db_name: &str,
    object: &mut serde_json::Map<String, Value>,
) -> Result<bool> {
    let Some(url_value) = object.get("url").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Ok(mut parsed) = Url::parse(url_value) else {
        return Ok(false);
    };
    let Some(password) = parsed.password() else {
        return Ok(false);
    };
    if password.trim().is_empty() {
        return Ok(false);
    }
    let password = password.to_string();
    let password_ref = object
        .get("passwordRef")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| secret_ref_for(db_name, "url"));
    encrypt_secret(path, &password_ref, &password)?;
    parsed
        .set_password(Some(""))
        .map_err(|_| anyhow::anyhow!("数据库配置 {db_name} 的 url 用户信息非法"))?;
    object.insert("url".to_string(), Value::String(parsed.to_string()));
    object.insert("passwordRef".to_string(), Value::String(password_ref));
    Ok(true)
}

fn migrate_nested_secret(
    path: &Path,
    db_name: &str,
    object: &mut serde_json::Map<String, Value>,
    parent_key: &str,
    field_key: &str,
    ref_suffix: &str,
) -> Result<bool> {
    let Some(parent) = object.get_mut(parent_key).and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    let Some(secret) = parent.get(field_key).and_then(Value::as_str) else {
        return Ok(false);
    };
    if secret.trim().is_empty() {
        return Ok(false);
    }
    let secret = secret.to_string();
    let ref_key = format!("{field_key}Ref");
    let secret_ref = parent
        .get(&ref_key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| secret_ref_for(db_name, ref_suffix));
    encrypt_secret(path, &secret_ref, &secret)?;
    parent.insert(field_key.to_string(), Value::String(String::new()));
    parent.insert(ref_key, Value::String(secret_ref));
    Ok(true)
}

pub(crate) fn resolve_secret_refs(path: &Path, config: &mut AppConfig) -> Result<()> {
    for (name, db) in &mut config.databases {
        resolve_url_password_ref(path, name, db)?;
        if let Some(tunnel) = &mut db.ssh_tunnel {
            if tunnel.password.as_deref().unwrap_or_default().is_empty() {
                if let Some(password_ref) = tunnel.password_ref.as_deref() {
                    tunnel.password = Some(decrypt_secret(path, password_ref)?);
                }
            }
            if tunnel.passphrase.as_deref().unwrap_or_default().is_empty() {
                if let Some(passphrase_ref) = tunnel.passphrase_ref.as_deref() {
                    tunnel.passphrase = Some(decrypt_secret(path, passphrase_ref)?);
                }
            }
        }
    }
    Ok(())
}

fn resolve_url_password_ref(path: &Path, name: &str, db: &mut DatabaseConfig) -> Result<()> {
    if db.password_ref.is_none() {
        return Ok(());
    }
    let mut parsed =
        Url::parse(&db.url).with_context(|| format!("数据库配置 {name} 的 url 不是合法 URL"))?;
    if parsed
        .password()
        .map(|password| !password.is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }
    let password_ref = db.password_ref.as_deref().expect("password_ref 已检查");
    let password = decrypt_secret(path, password_ref)?;
    parsed
        .set_password(Some(&password))
        .map_err(|_| anyhow::anyhow!("数据库配置 {name} 的 url 用户信息非法"))?;
    db.url = parsed.to_string();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 返回 (临时目录守卫, 配置路径)：守卫负责在用例结束时删掉目录
    fn temp_config_path(name: &str) -> (crate::editor::test_support::TempDir, PathBuf) {
        let dir = crate::editor::test_support::TempDir::new(name);
        let path = dir.path().join("config.json");
        (dir, path)
    }

    #[test]
    fn load_config_migrates_database_url_password() {
        let (_dir, path) = temp_config_path("url-password");
        fs::write(
            &path,
            r#"{
  "databases": {
    "mysql-app": {
      "type": "mysql",
      "url": "mysql://user:secret@127.0.0.1:3306/app"
    }
  }
}"#,
        )
        .expect("写入配置失败");

        let config = load_config(Some(path.clone())).expect("加载配置失败");
        let db = config.databases.get("mysql-app").expect("缺少数据库配置");
        assert_eq!(db.password_ref.as_deref(), Some("agentdbcli:mysql-app:url"));
        assert!(db.url.contains("user:secret@"));
        let raw = fs::read_to_string(&path).expect("读取配置失败");
        assert!(!raw.contains("secret"));
        assert!(raw.contains(r#""passwordRef": "agentdbcli:mysql-app:url""#));
    }

    #[test]
    fn load_config_migrates_ssh_password_and_passphrase() {
        let (_dir, path) = temp_config_path("ssh-secrets");
        fs::write(
            &path,
            r#"{
  "databases": {
    "pg-via-ssh": {
      "type": "postgres",
      "url": "postgres://user:@127.0.0.1:5432/app",
      "sshTunnel": {
        "host": "127.0.0.1",
        "username": "root",
        "password": "ssh-secret",
        "privateKeyPath": "~/.ssh/id_rsa",
        "passphrase": "key-secret"
      }
    }
  }
}"#,
        )
        .expect("写入配置失败");

        let config = load_config(Some(path.clone())).expect("加载配置失败");
        let tunnel = config
            .databases
            .get("pg-via-ssh")
            .and_then(|db| db.ssh_tunnel.as_ref())
            .expect("缺少 SSH 隧道配置");
        assert_eq!(tunnel.password.as_deref(), Some("ssh-secret"));
        assert_eq!(tunnel.passphrase.as_deref(), Some("key-secret"));
        assert_eq!(
            tunnel.password_ref.as_deref(),
            Some("agentdbcli:pg-via-ssh:sshPassword")
        );
        assert_eq!(
            tunnel.passphrase_ref.as_deref(),
            Some("agentdbcli:pg-via-ssh:sshPassphrase")
        );
        let raw = fs::read_to_string(&path).expect("读取配置失败");
        assert!(!raw.contains("ssh-secret"));
        assert!(!raw.contains("key-secret"));
    }
}
