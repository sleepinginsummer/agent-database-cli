use crate::secrets::{decrypt_secret, encrypt_secret};
use crate::types::{AppConfig, DatabaseConfig, DatabaseType, OracleDriver};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{env, fs, path::PathBuf};
use url::Url;

pub const CONFIG_ENV: &str = "AGENT_DATABASE_CLI_CONFIG";
const SECRET_REF_PREFIX: &str = "agentdbcli:";

pub fn resolve_config_path() -> Result<PathBuf> {
    if let Ok(path) = env::var(CONFIG_ENV) {
        return Ok(PathBuf::from(path));
    }
    let home = dirs::home_dir().context("could not resolve user home directory")?;
    Ok(home.join(".agent-database-cli").join("config.json"))
}

pub fn load_config(path: Option<PathBuf>) -> Result<AppConfig> {
    let path = match path {
        Some(path) => path,
        None => resolve_config_path()?,
    };
    migrate_plain_secrets(&path)?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let mut config: AppConfig =
        serde_json::from_str(&raw).context("config file is not valid JSON")?;
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
        anyhow::bail!("database config {name} must provide url");
    }
    if db.redis_cluster.is_some() && db.db_type != DatabaseType::Redis {
        anyhow::bail!("database config {name}: redisCluster is only allowed for the redis type");
    }
    if let Some(cluster) = &db.redis_cluster {
        if cluster.nodes.is_empty() {
            anyhow::bail!("database config {name}: redisCluster.nodes must be a non-empty array");
        }
        for (index, node) in cluster.nodes.iter().enumerate() {
            if node.trim().is_empty() {
                anyhow::bail!("database config {name}: redisCluster.nodes[{index}] must be a non-empty string");
            }
            let parsed = url::Url::parse(node).with_context(|| {
                format!("database config {name}: redisCluster.nodes[{index}] is not a valid URL")
            })?;
            if parsed.scheme() != "redis" && parsed.scheme() != "rediss" {
                anyhow::bail!(
                    "database config {name}: redisCluster.nodes[{index}] only supports redis:// or rediss://"
                );
            }
        }
    }
    if let Some(driver) = &db.oracle_driver {
        if db.db_type != DatabaseType::Oracle {
            anyhow::bail!(
                "database config {name}: oracleDriver is only allowed for the oracle type"
            );
        }
        match driver {
            OracleDriver::Oracle | OracleDriver::Sqlcl | OracleDriver::Oracledb => {}
        }
    }
    if let Some(tunnel) = &db.ssh_tunnel {
        if tunnel.host.trim().is_empty() {
            anyhow::bail!("database config {name}: sshTunnel.host must be a non-empty string");
        }
        if tunnel.username.trim().is_empty() {
            anyhow::bail!("database config {name}: sshTunnel.username must be a non-empty string");
        }
        if tunnel.password.as_deref() == Some("") && tunnel.password_ref.is_none() {
            anyhow::bail!("database config {name}: sshTunnel.password must not be an empty string");
        }
        if tunnel.private_key_path.as_deref() == Some("") {
            anyhow::bail!(
                "database config {name}: sshTunnel.privateKeyPath must not be an empty string"
            );
        }
        if tunnel.private_key.as_deref() == Some("") {
            anyhow::bail!(
                "database config {name}: sshTunnel.privateKey must not be an empty string"
            );
        }
        if tunnel.private_key_path.is_some() && tunnel.private_key.is_some() {
            anyhow::bail!(
                "database config {name}: only one of sshTunnel.privateKeyPath and privateKey may be configured"
            );
        }
        if tunnel.password.is_none()
            && tunnel.password_ref.is_none()
            && tunnel.private_key_path.is_none()
            && tunnel.private_key.is_none()
        {
            anyhow::bail!(
                "database config {name}: sshTunnel must configure password, privateKeyPath, or privateKey"
            );
        }
        if tunnel.passphrase.as_deref() == Some("") && tunnel.passphrase_ref.is_none() {
            anyhow::bail!(
                "database config {name}: sshTunnel.passphrase must not be an empty string"
            );
        }
        if (tunnel.passphrase.is_some() || tunnel.passphrase_ref.is_some())
            && tunnel.private_key_path.is_none()
            && tunnel.private_key.is_none()
        {
            anyhow::bail!("database config {name}: sshTunnel.passphrase can only be used together with a private key");
        }
    }
    Ok(())
}

/// Best-effort read of the top-level `defaultFormat` config field, used as the
/// output format when `--format` is not given. Returns None when the config
/// file is missing, unreadable, or not valid JSON — those problems surface
/// later, on commands that actually need the config — so `list` keeps working
/// without one. The value is returned raw; the caller validates it.
pub fn default_output_format() -> Option<String> {
    let path = resolve_config_path().ok()?;
    default_output_format_at(&path)
}

fn default_output_format_at(path: &PathBuf) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&raw).ok()?;
    root.get("defaultFormat")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Persist (or clear, when `value` is None) the top-level `defaultFormat`
/// field, preserving everything else in the config file. Creates the file if
/// it does not exist yet.
pub fn set_default_output_format(value: Option<&str>) -> Result<PathBuf> {
    let path = resolve_config_path()?;
    set_default_output_format_at(&path, value)?;
    Ok(path)
}

fn set_default_output_format_at(path: &PathBuf, value: Option<&str>) -> Result<()> {
    let mut root: Value = match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).context("config file is not valid JSON")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            serde_json::json!({ "databases": {} })
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read config file: {}", path.display()))
        }
    };
    let object = root
        .as_object_mut()
        .context("config file root must be a JSON object")?;
    match value {
        Some(format) => {
            object.insert(
                "defaultFormat".to_string(),
                Value::String(format.to_string()),
            );
        }
        None => {
            object.remove("defaultFormat");
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&root)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn get_database_config<'a>(config: &'a AppConfig, name: &str) -> Result<&'a DatabaseConfig> {
    config
        .databases
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("database config not found: {name}"))
}

pub fn list_supported_databases() -> Vec<&'static str> {
    vec!["mysql", "postgres", "redis", "oracle", "mongodb"]
}

fn secret_ref_for(db_name: &str, field: &str) -> String {
    format!("{SECRET_REF_PREFIX}{db_name}:{field}")
}

fn migrate_plain_secrets(path: &PathBuf) -> Result<()> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let mut root: Value = serde_json::from_str(&raw).context("config file is not valid JSON")?;
    let databases = root
        .get_mut("databases")
        .and_then(Value::as_object_mut)
        .context("config file is missing the databases object")?;
    let mut migrated = false;
    for (name, value) in databases {
        let object = value
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("database config {name} must be an object"))?;
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
    path: &PathBuf,
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
        .map_err(|_| anyhow::anyhow!("database config {db_name}: url user info is invalid"))?;
    object.insert("url".to_string(), Value::String(parsed.to_string()));
    object.insert("passwordRef".to_string(), Value::String(password_ref));
    Ok(true)
}

fn migrate_nested_secret(
    path: &PathBuf,
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

fn resolve_secret_refs(path: &PathBuf, config: &mut AppConfig) -> Result<()> {
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

fn resolve_url_password_ref(path: &PathBuf, name: &str, db: &mut DatabaseConfig) -> Result<()> {
    if db.password_ref.is_none() {
        return Ok(());
    }
    let mut parsed = Url::parse(&db.url)
        .with_context(|| format!("database config {name}: url is not a valid URL"))?;
    if parsed
        .password()
        .map(|password| !password.is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }
    let password_ref = db
        .password_ref
        .as_deref()
        .expect("password_ref already checked");
    let password = decrypt_secret(path, password_ref)?;
    parsed
        .set_password(Some(&password))
        .map_err(|_| anyhow::anyhow!("database config {name}: url user info is invalid"))?;
    db.url = parsed.to_string();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is earlier than UNIX_EPOCH")
            .as_nanos();
        let dir = env::temp_dir().join(format!("agent-database-cli-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("failed to create temp directory");
        dir.join("config.json")
    }

    #[test]
    fn default_output_format_reads_top_level_field() {
        let path = temp_config_path("default-format");
        fs::write(
            &path,
            r#"{ "defaultFormat": "table", "databases": {} }"#,
        )
        .expect("failed to write config");
        assert_eq!(default_output_format_at(&path), Some("table".to_string()));
    }

    #[test]
    fn default_output_format_absent_or_unreadable_is_none() {
        let path = temp_config_path("no-default-format");
        fs::write(&path, r#"{ "databases": {} }"#).expect("failed to write config");
        assert_eq!(default_output_format_at(&path), None);
        assert_eq!(
            default_output_format_at(&path.with_extension("missing")),
            None
        );
    }

    #[test]
    fn set_default_output_format_creates_updates_and_clears() {
        let path = temp_config_path("set-default-format");

        // creates the file when missing
        set_default_output_format_at(&path, Some("table")).expect("failed to set format");
        assert_eq!(default_output_format_at(&path), Some("table".to_string()));

        // updates in place, preserving other fields
        fs::write(
            &path,
            r#"{ "defaultFormat": "table", "databases": { "db": { "type": "redis", "url": "redis://x" } } }"#,
        )
        .expect("failed to write config");
        set_default_output_format_at(&path, Some("compact")).expect("failed to update format");
        assert_eq!(default_output_format_at(&path), Some("compact".to_string()));
        let raw = fs::read_to_string(&path).expect("failed to read config");
        assert!(raw.contains("redis://x"));

        // clears without touching the rest
        set_default_output_format_at(&path, None).expect("failed to clear format");
        assert_eq!(default_output_format_at(&path), None);
        let raw = fs::read_to_string(&path).expect("failed to read config");
        assert!(raw.contains("redis://x"));
    }

    #[test]
    fn load_config_migrates_database_url_password() {
        let path = temp_config_path("url-password");
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
        .expect("failed to write config");

        let config = load_config(Some(path.clone())).expect("failed to load config");
        let db = config
            .databases
            .get("mysql-app")
            .expect("missing database config");
        assert_eq!(db.password_ref.as_deref(), Some("agentdbcli:mysql-app:url"));
        assert!(db.url.contains("user:secret@"));
        let raw = fs::read_to_string(&path).expect("failed to read config");
        assert!(!raw.contains("secret"));
        assert!(raw.contains(r#""passwordRef": "agentdbcli:mysql-app:url""#));
    }

    #[test]
    fn load_config_migrates_ssh_password_and_passphrase() {
        let path = temp_config_path("ssh-secrets");
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
        .expect("failed to write config");

        let config = load_config(Some(path.clone())).expect("failed to load config");
        let tunnel = config
            .databases
            .get("pg-via-ssh")
            .and_then(|db| db.ssh_tunnel.as_ref())
            .expect("missing SSH tunnel config");
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
        let raw = fs::read_to_string(&path).expect("failed to read config");
        assert!(!raw.contains("ssh-secret"));
        assert!(!raw.contains("key-secret"));
    }
}
