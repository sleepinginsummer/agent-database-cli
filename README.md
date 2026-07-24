# agent-database-cli

Local multi-database CLI for agents. One command opens a connection, runs, disconnects.
Supports **MySQL · PostgreSQL · Redis · Oracle · MongoDB**, with read-only mode and a command blocklist. Built in Rust, shipped via npm.

## Install

```bash
npm install -g @mejazbese21/agent-database-cli
agent-database-cli --help     # also available as: db-cli
```

Requires Node >= 20. On install, the matching native binary (macOS x64/arm64, Linux x64/arm64, Windows x64) is downloaded from [GitHub Releases](https://github.com/mohsingdp-ai/agent-database-cli/releases) into the package.

Prefer no npm? Download the binary for your OS straight from the [Releases page](https://github.com/mohsingdp-ai/agent-database-cli/releases), put it on your `PATH`, and run it — it's self-contained.

Update / uninstall:

```bash
npm install -g @mejazbese21/agent-database-cli@latest
npm uninstall -g @mejazbese21/agent-database-cli && rm -rf ~/.agent-database-cli
```

## Usage

```bash
agent-database-cli list                                  # types + configured connections
agent-database-cli test --db local-mysql                 # test a connection
agent-database-cli exec --db local-mysql --command "select 1"
agent-database-cli meta --db local-mysql --type tables   # tables/columns/collections/keys
agent-database-cli --format table exec --db local-mysql --command "select 1"
agent-database-cli format table                          # persist a default format (--clear resets)
```

For many queries in a row, reuse one connection:

```bash
printf 'select 1\nselect count(*) from accounts\n' | agent-database-cli repl --db local-mysql
```

**MCP server** (`agent-database-cli-mcp`) — for agents. Stateless tools (`query` / `describe` / `list_databases` / `help`); each call names its database and the first query per database opens a connection that stays warm for the rest of the session:

```bash
claude mcp add agent-db -- agent-database-cli-mcp
```

## Configuration

File: `~/.agent-database-cli/config.json` (override with `AGENT_DATABASE_CLI_CONFIG`).

```json
{
  "defaultFormat": "table",
  "databases": {
    "local-mysql": {
      "type": "mysql",
      "url": "mysql://user:password@localhost:3306/app",
      "readonly": true,
      "blacklist": ["drop", "truncate", "delete"]
    },
    "remote-mysql": {
      "type": "mysql",
      "url": "mysql://user:password@db.internal:3306/app",
      "sshTunnel": { "host": "jump.example.com", "username": "deploy", "privateKeyPath": "~/.ssh/id_rsa" },
      "readonly": true
    }
  }
}
```

Top-level `defaultFormat` (optional) sets the output format used when `--format` is not passed (`compact` | `json` | `table`); without it, row-producing commands default to `compact` and the rest to `json`. Manage it with `agent-database-cli format [<format>|--clear]` instead of editing the file.

Per-connection fields:

- `type` — `mysql` | `postgres` | `redis` | `oracle` | `mongodb`
- `url` — connection string. Postgres TLS via `?sslmode=` (`prefer` default, `require`, `verify-full`, `disable`). For managed DBs with a private CA (e.g. RDS) use `require`.
- `readonly` — default `true`; only set `false` when writes are truly needed.
- `blacklist` — case-insensitive command blocklist, checked before read-only.
- `sshTunnel` — `host`, `port` (22), `username`, and `password` or `privateKeyPath`/`privateKey` (+ optional `passphrase`).
- `redisCluster.nodes` — array of cluster node URLs (cluster mode needs both `url` and `nodes`).
- Oracle: `oracleDriver` (`sqlcl` default | `oracle` | `oracledb`), `sqlclPath`, `javaHome`.

Passwords/passphrases are encrypted on first use (stored as `*Ref` in the config dir); plaintext is cleared automatically.

## Permissions

Use `readonly` **and** `blacklist` together. Order: blacklist is checked first (reject on match), then read-only (reject writes). Read-only also blocks write-semantic reads like Postgres `SELECT INTO` and Mongo `$out` / `$merge`.

High-risk commands to blocklist:

- SQL: `drop, truncate, delete, update, insert, merge, alter, create, grant, revoke`
- Redis: `flushall, flushdb, del, set, expire, rename, keys`
- Mongo: `insertMany, updateMany, deleteMany, drop, dropDatabase, $out, $merge`

## Oracle (SQLcl)

Oracle uses [SQLcl](https://www.oracle.com/database/sqldeveloper/technologies/sqlcl/) by default (no Instant Client needed, works with old versions like Oracle 11). The connect script is passed via stdin so the password is never in process args; blocklist and read-only still apply.

## License

MIT
