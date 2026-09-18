<div align="center">

# agent-database-cli

A CLI-based multi-database tool that packages common database connection, query, metadata inspection, and connection reuse capabilities as local commands callable by agents.

MySQL · PostgreSQL · Redis · Oracle · MongoDB · Read-only mode · Command blocklist · SQLcl Oracle · Local daemon

<p>
  <img src="https://img.shields.io/badge/CLI-agent--database--cli-2ea44f" alt="CLI agent-database-cli">
  <img src="https://img.shields.io/badge/License-MIT-green" alt="License MIT">
  <img src="https://img.shields.io/badge/Node.js-%3E%3D20-339933?logo=node.js&logoColor=white" alt="Node.js >=20">
  <img src="https://img.shields.io/badge/npm-%3E%3D10-CB3837?logo=npm&logoColor=white" alt="npm >=10">
  <img src="https://img.shields.io/badge/sys-win%2Fmac%2Flinux-0078D6" alt="sys win/mac/linux">
  <img src="https://img.shields.io/badge/release-v0.2.20-blue" alt="release v0.2.20">
</p>

[AI One-Click Installation](#ai-one-click-installation) · [Installation](#installation) · [Configuration](#configuration) · [Permission Configuration](#permission-configuration) · [Oracle SQLcl](#oracle-sqlcl) · [License](#license) · [Friendly Links](#friendly-links)

[中文](README.md) | English

</div>

## Introduction

`agent-database-cli` is a local multi-database CLI tool for Agents, implemented in Rust to provide connection, query, metadata inspection, read-only control, command blocklist, and password encryption capabilities for MySQL, PostgreSQL, Redis, Oracle, and MongoDB.

What it can do:

- List currently supported database types and locally configured connections
- Execute SQL, Redis commands, or MongoDB JSON commands against a specified database
- Query database metadata such as tables, columns, collections, and Redis keys. Redis keys metadata uses cursor-based `SCAN` instead of blocking `KEYS`
- Enable read-only mode and command blocklists per database configuration
- Auto-start the local daemon on demand; the daemon exits after `1800` idle seconds by default (override with `AGENT_DB_DAEMON_IDLE_SECS`)
- Keep connections alive through the local daemon; each database connection is released after `600` idle seconds by default
- Oracle uses SQLcl by default; native `oracle`/`oracledb` drivers can be selected explicitly when Oracle Instant Client is available
- Never store or print unmasked passwords, tokens, or secrets
- Use named pipes on Windows and Unix sockets on macOS/Linux for the daemon

Driver configuration table:

| Database | `type` | Default driver | Driver switch configuration | Common configuration |
| --- | --- | --- | --- | --- |
| MySQL | `mysql` | Native Rust driver `mysql_async` | Not switchable yet | `readonly`, `blacklist`, `keepAliveSeconds` |
| PostgreSQL | `postgres` | Native Rust driver `tokio-postgres` | Not switchable yet | `readonly`, `blacklist`, `keepAliveSeconds` |
| Redis standalone | `redis` | Native Rust driver `redis` | Configure `url` only | `readonly`, `blacklist`, `keepAliveSeconds` |
| Redis cluster | `redis` | Native Rust driver `redis` | Configure both `url` and `redisCluster.nodes` | `readonly`, `blacklist`, `keepAliveSeconds` |
| Oracle | `oracle` | SQLcl | `oracleDriver: "sqlcl" \| "oracle" \| "oracledb"`; defaults to SQLcl when omitted. Native drivers require Oracle Instant Client | `readonly`, `blacklist`, `keepAliveSeconds` |
| MongoDB | `mongodb` | Native Rust driver `mongodb` | Not switchable yet; `database` can be configured as the default database | `readonly`, `blacklist`, `keepAliveSeconds` |

## Installation

### Requirements

- Node.js `>= 20`
- npm `>= 10`
- System support: Windows / macOS / Linux
- The matching Rust binary subpackage is installed automatically for your platform. Supported targets: macOS x64/arm64, Linux x64/arm64, Windows x64
- Local network access to the target database
- Docker and Docker Compose if you run integration tests
- SQLcl and Java installed locally if Oracle uses SQLcl

Supported platform subpackages:

| OS | Architecture | npm subpackage | Rust target |
| --- | --- | --- | --- |
| macOS | arm64 | `@agent-database-cli/darwin-arm64` | `aarch64-apple-darwin` |
| macOS | x64 | `@agent-database-cli/darwin-x64` | `x86_64-apple-darwin` |
| Linux | arm64 | `@agent-database-cli/linux-arm64` | `aarch64-unknown-linux-gnu` |
| Linux | x64 | `@agent-database-cli/linux-x64` | `x86_64-unknown-linux-gnu` |
| Windows | x64 | `@agent-database-cli/win32-x64` | `x86_64-pc-windows-msvc` |


### AI One-Click Installation

```text
Please read https://github.com/sleepinginsummer/agent-database-cli/blob/main/AI_INSTALL.md, install the CLI as instructed, and add `SKILL.md`.
```

### Manual Global Installation

```bash
npm install -g agent-database-cli
agent-database-cli --help
```

If npm package installation is restricted, use the equivalent source installation flow:

```powershell
git clone https://github.com/sleepinginsummer/agent-database-cli.git
cd agent-database-cli
npm install
npm run build
npm link
agent-database-cli --help
```

Install/update the Agent skill:

```bash
agent-database-cli install-skill --dry-run
agent-database-cli install-skill
```

## Configuration

Default configuration file:

```text
~/.agent-database-cli/config.json
```

You can override the configuration path with an environment variable:

```bash
AGENT_DATABASE_CLI_CONFIG=/path/to/config.json agent-database-cli list
```

The configuration file is an object. Each key under `databases` is a database connection name:

- `type`: Database type, supports `mysql`, `postgres`, `redis`, `oracle`, and `mongodb`
- `url`: Database connection URL. In Redis standalone mode it is the direct target. In Redis cluster mode it is used as the entry node URL
- `redisCluster`: Optional Redis cluster configuration. When configured, cluster mode is used
- `sshTunnel`: Optional SSH tunnel configuration. In standalone mode, the database URL host and port are forwarded through SSH. In Redis cluster mode, a local forwarding port is created for each configured cluster node
- `database`: Optional default MongoDB database name
- `readonly`: Whether read-only mode is enabled, default `true`; only explicitly set `false` when write access is really required
- `blacklist`: Command blocklist array, case-insensitive
- `keepAliveSeconds`: Idle release timeout in seconds for a single database connection, default `600`
- `oracleDriver`: Oracle driver, supports `sqlcl`, `oracle`, or `oracledb`; defaults to `sqlcl` when omitted
- `sqlclPath`: SQLcl executable path, used only when `oracleDriver` is `sqlcl`
- `javaHome`: Optional `JAVA_HOME` used by SQLcl

`redisCluster` currently supports:

- `nodes`: Array of Redis cluster node URLs. At least one node is required. `redis://` and `rediss://` are supported

Redis cluster notes:

- In the current implementation, Redis cluster mode requires both `url` and `redisCluster.nodes`
- `url` is used as the cluster entry node. It is recommended to use any stable reachable cluster node URL
- `redisCluster.nodes` is used as the cluster node list. With SSH tunneling, it is also used to create per-node local forwarding and address mapping
- When `redisCluster.nodes` is configured, the client switches to Redis Cluster mode
- When `sshTunnel` is also configured, the program creates a local forwarded port for each cluster node and uses address mapping for cluster redirects
- With SSH tunneling, `redisCluster.nodes` should cover the cluster node addresses that the client may be redirected to

`sshTunnel` supports password, private key, password plus private key, and passphrase-protected private key authentication:

- `host`: SSH jump host address
- `port`: SSH port, default `22`
- `username`: SSH username
- `password`: Optional SSH password
- `passwordRef`: Local encrypted reference for the SSH password. Generated automatically when a plaintext `password` is first used
- `privateKeyPath`: Optional private key file path, supports `~`
- `privateKey`: Optional private key content, mutually exclusive with `privateKeyPath`
- `passphrase`: Optional private key passphrase, only valid when a private key is configured
- `passphraseRef`: Local encrypted reference for the private key passphrase. Generated automatically when a plaintext `passphrase` is first used
- `readyTimeout`: Optional SSH connection timeout in milliseconds

Sensitive values are passively encrypted the first time the target connection is used. Plaintext database URL passwords, `sshTunnel.password`, and `sshTunnel.passphrase` are stored in `secrets.json` under the config directory, with a local `secret.key`; the config is rewritten with the plaintext cleared and the corresponding `*Ref` populated. Later runs decrypt only in memory. To change a password, write a new plaintext value back into the field and use the connection again.

The blocklist and read-only mode work together with a fixed priority: check the blocklist first, reject immediately on match, and only check read-only mode when no blocklist rule matches.

Read-only mode notes:

- Read-only mode is enabled by default; write operations are still rejected when `readonly` is omitted
- It is recommended to keep all database connections read-only by default. When data changes are needed, let AI generate the SQL or command first, then execute it after your confirmation
- Only set `readonly: false` on a specific connection when write access is truly required

Reference configuration:

```json
{
  "databases": {
    "local-mysql": {
      "type": "mysql",
      "url": "mysql://user:password@localhost:3306/app",
      "readonly": true,
      "blacklist": ["drop", "truncate", "delete"],
      "keepAliveSeconds": 600
    },
    "remote-mysql": {
      "type": "mysql",
      "url": "mysql://user:password@db.internal:3306/app",
      "sshTunnel": {
        "host": "jump.example.com",
        "port": 22,
        "username": "deploy",
        "privateKeyPath": "~/.ssh/id_rsa",
        "passphrase": "key-passphrase"
      },
      "readonly": true,
      "keepAliveSeconds": 600
    },
    "redis-standalone": {
      "type": "redis",
      "url": "redis://localhost:6379",
      "readonly": false,
      "blacklist": ["flushall", "flushdb"],
      "keepAliveSeconds": 600
    },
    "redis-cluster": {
      "type": "redis",
      "url": "redis://10.0.0.11:7001",
      "redisCluster": {
        "nodes": [
          "redis://10.0.0.11:7001",
          "redis://10.0.0.12:7001",
          "redis://10.0.0.13:7001"
        ]
      },
      "readonly": true,
      "blacklist": ["flushall", "flushdb"],
      "keepAliveSeconds": 600
    },
    "redis-cluster-via-ssh": {
      "type": "redis",
      "url": "redis://10.0.0.11:7001",
      "redisCluster": {
        "nodes": [
          "redis://10.0.0.11:7001",
          "redis://10.0.0.12:7001",
          "redis://10.0.0.13:7001"
        ]
      },
      "sshTunnel": {
        "host": "jump.example.com",
        "port": 22,
        "username": "deploy",
        "privateKeyPath": "~/.ssh/id_rsa"
      },
      "readonly": true,
      "blacklist": ["flushall", "flushdb"],
      "keepAliveSeconds": 600
    },
    "oracle-test": {
      "type": "oracle",
      "url": "oracle://USER:password@127.0.0.1:1521/qftest201",
      "oracleDriver": "sqlcl",
      "sqlclPath": "/opt/homebrew/Caskroom/sqlcl/26.1.0.086.1709/sqlcl/bin/sql",
      "javaHome": "/Applications/IntelliJ IDEA Ultimate.app/Contents/jbr/Contents/Home",
      "readonly": true,
      "blacklist": ["drop", "truncate", "delete", "update", "insert", "merge", "alter", "create"],
      "keepAliveSeconds": 600
    }
  }
}
```

## Permission Configuration

It is recommended to use both `readonly` and `blacklist` together for permission control. Do not rely on only one of them.

### Read-only Mode

- The default value is `true`
- When `readonly` is omitted, the connection is still treated as read-only
- Read-only mode also rejects queries with write semantics, such as PostgreSQL `SELECT INTO` and MongoDB aggregate `$out` / `$merge`
- It is recommended to keep all day-to-day query connections read-only by default
- When data changes are needed, let AI generate the SQL or command first, then execute it after your confirmation
- Only dedicated writable connections should explicitly set `readonly: false`

### Command Blocklist

- The blocklist has higher priority than read-only mode
- A command is rejected immediately once it matches the blocklist
- It is suitable for blocking high-risk commands such as dropping data, schema changes, mass writes, and cache-clearing operations
- It is recommended for production databases, shared test databases, and online Redis instances

### Execution Order

1. Check `blacklist` first
2. Reject immediately on match
3. Check `readonly` only when not matched
4. When `readonly` is enabled, only read commands are allowed

### Common High-Risk Commands

Common high-risk SQL for MySQL / PostgreSQL / Oracle:

```json
["drop", "truncate", "delete", "update", "insert", "merge", "alter", "create", "replace", "grant", "revoke"]
```

Common high-risk Redis commands:

```json
["flushall", "flushdb", "del", "unlink", "set", "mset", "expire", "rename", "hset", "lpush", "rpush", "sadd", "zadd", "keys"]
```

Common high-risk MongoDB commands:

```json
["insertOne", "insertMany", "updateOne", "updateMany", "replaceOne", "deleteOne", "deleteMany", "findAndModify", "findOneAndUpdate", "findOneAndDelete", "drop", "dropDatabase", "createIndex", "dropIndex", "$out", "$merge"]
```

### Recommended Configuration Examples

Recommended for production databases:

```json
{
  "type": "mysql",
  "url": "mysql://user:password@prod-db:3306/app",
  "readonly": true,
  "blacklist": ["drop", "truncate", "delete", "update", "insert", "alter", "create"],
  "keepAliveSeconds": 600
}
```

Recommended for a dedicated writable connection:

```json
{
  "type": "postgres",
  "url": "postgres://user:password@write-db:5432/app",
  "readonly": false,
  "blacklist": ["drop", "truncate", "alter"],
  "keepAliveSeconds": 600
}
```

## Oracle SQLcl

Official link: https://www.oracle.com/database/sqldeveloper/technologies/sqlcl/

Oracle uses SQLcl by default, avoiding a mandatory Oracle Instant Client dependency and improving compatibility with older Oracle versions such as Oracle 11. You may omit `oracleDriver` or set it explicitly:

```json
{
  "type": "oracle",
  "url": "oracle://USER:password@127.0.0.1:1521/qftest201",
  "oracleDriver": "sqlcl",
  "sqlclPath": "/opt/homebrew/Caskroom/sqlcl/26.1.0.086.1709/sqlcl/bin/sql",
  "javaHome": "/Applications/IntelliJ IDEA Ultimate.app/Contents/jbr/Contents/Home",
  "readonly": true,
  "blacklist": ["drop", "truncate", "delete", "update", "insert", "merge", "alter", "create"]
}
```

SQLcl mode passes the connection script through stdin to avoid exposing the password in process arguments. Security checks still happen before execution, and both blocklist and read-only mode remain effective.

## Update

```bash
npm install -g agent-database-cli@latest
```

## Uninstall and Cleanup

Uninstall and clean local configuration:

```bash
npm uninstall -g agent-database-cli
npm cache clean --force
rm -rf ~/.agent-database-cli
```


## Rust refactor notes

A Rust CLI scaffold has been added. The current binary is `agent-database-cli`:

```bash
npm run build:rust
npm run dev:rust -- list
npm run dev:rust -- daemon status
```

Oracle keeps two first-class drivers: omitting `oracleDriver` uses SQLcl; native `oracle`/`oracledb` drivers remain available when Oracle Instant Client is installed. The default entry is now the native Rust CLI. Windows, Linux and macOS binaries are distributed through npm platform subpackages. Oracle defaults to SQLcl; native Oracle drivers must be selected explicitly.

## License

[MIT](LICENSE)

## Friendly Links

- [LINUX DO - A New Ideal Community](https://linux.do/)

## Environment variables (daemon)

These are read when the **daemon process starts**. After changing them, run `daemon stop` and let the next command start a fresh daemon.

| Variable | Default | Meaning |
| --- | --- | --- |
| `AGENT_DB_QUERY_TIMEOUT_SECS` | `60` | Response timeout (seconds) for a single test / execute / metadata call. On timeout the client stops waiting; Oracle SQLcl kills the child process, and the native Oracle driver also sets an OCI call timeout. |
| `AGENT_DB_DAEMON_IDLE_SECS` | `1800` | Daemon exits after this many idle seconds with no in-flight requests. |

Per-connection `keepAliveSeconds` now defaults to `600` when unset.
