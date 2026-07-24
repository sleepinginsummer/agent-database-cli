---
name: agent-database-cli
description: Use the local agent-database-cli to safely operate configured databases. Suitable for listing database connections, testing connections, executing SQL/Redis/MongoDB commands, querying metadata for tables/columns/collections/keys, and verifying read-only mode and the command blacklist.
---

# agent-database-cli Usage Guide

`agent-database-cli` is a multi-database command-line tool driven by local configuration, designed to let an AI or a user operate databases safely.

What it can do:

- List supported database types and locally configured database connections
- Test a specified database connection
- Execute SQL, Redis commands, or MongoDB JSON commands
- Query metadata such as tables, columns, collections, and Redis keys
- Enforce a command blacklist and read-only mode per individual database configuration
- Open a direct connection per command, run it, and disconnect; use `repl` to reuse one connection across many statements
- Prebuilt binaries support macOS x64/arm64, Linux x64/arm64, and Windows x64
- Oracle uses SQLcl by default; when `oracleDriver: "oracle"` or `"oracledb"` is explicitly configured, the native Oracle driver is used

What it does not do:

- Does not scan the network or discover databases; it only uses the connections in the configuration file
- Does not bypass the blacklist or read-only mode defined in the configuration
- Does not output unmasked passwords, tokens, or secrets
- Does not execute writes, deletes, DDL, or other dangerous commands by default

## Safety Confirmation

Before executing any command that may write, delete, modify structure, or affect data integrity, you must first confirm whether the target database configuration has `readonly` and `blacklist` enabled.

Dangerous operations include:

- DDL: `drop`, `truncate`, `alter`, `create`
- DML writes: `insert`, `update`, `delete`, `merge`
- Redis flush or write: `flushall`, `flushdb`, `set`, `del`
- MongoDB write or delete: `insertOne`, `updateOne`, `deleteMany`, `drop`, `dropDatabase`
- Any command that is irreversible, affects production data, or affects structure or permissions

If the user explicitly requests a dangerous command, first state the target database name, the command, and its potential impact, then wait for the user's explicit consent. Even with the user's consent, you must not bypass the blacklist or read-only mode defined in this project's configuration.

The blacklist takes precedence over read-only mode. Before executing a command, check the `blacklist` first and reject immediately on a match; only if there is no match, then check `readonly`.

Reading the JSON configuration file requires user confirmation, to prevent secret leakage.

## Environment Check

Before invoking, first check whether the CLI is available:

```bash
agent-database-cli --help
```


If the command above fails, check the base environment:

```bash
node --version
npm --version
```

If dependencies or build artifacts are missing, run the following in the project directory:

```bash
npm install
npm run build
```

Default configuration file:

```text
~/.agent-database-cli/config.json
```

Specify a different configuration file:

```bash
AGENT_DATABASE_CLI_CONFIG=/path/to/config.json agent-database-cli list
```

## Configuration Format

The configuration file is a JSON object whose root field is `databases`:

```json
{
  "databases": {
    "local-mysql": {
      "type": "mysql",
      "url": "mysql://user:password@localhost:3306/app",
      "readonly": true,
      "blacklist": ["drop", "truncate", "delete"]
    }
  }
}
```

Fields:

- `type`: `mysql`, `postgres`, `redis`, `oracle`, `mongodb`
- `url`: database connection URL. For PostgreSQL, an optional `sslmode` query parameter controls TLS: omitted/`prefer` (try TLS, no cert verification, fall back to plaintext), `require` (must encrypt, no verification), `verify-ca`/`verify-full` (encrypt + verify against public CA roots), `disable` (plaintext). SSL-required servers such as AWS RDS work with the default; for RDS's private CA use `require` rather than `verify-full`
- `passwordRef`: local encrypted reference for the database URL password; generated automatically the first time a plaintext URL password is used
- `database`: default MongoDB database name, optional
- `readonly`: whether to enable read-only mode
- `blacklist`: command blacklist array, case-insensitive
- `oracleDriver`: Oracle driver, either `oracledb` or `sqlcl`
- `sqlclPath`: path to the SQLcl executable
- `javaHome`: the `JAVA_HOME` used by SQLcl
- `sshTunnel.passwordRef`: local encrypted reference for the SSH password; generated automatically the first time a plaintext `sshTunnel.password` is used
- `sshTunnel.passphraseRef`: local encrypted reference for the SSH private key passphrase; generated automatically the first time a plaintext `sshTunnel.passphrase` is used

The first time a connection is used, the CLI encrypts the plaintext database URL password, `sshTunnel.password`, and `sshTunnel.passphrase`, saves them to `secrets.json` in the configuration directory, generates a local `secret.key`, and rewrites the configuration file to use the corresponding `*Ref` references. From then on they are decrypted only in memory; to change a password, re-enter the plaintext field to overwrite the old ciphertext.

## Global Parameters

- `--format <format>`: output format, supports `compact`, `json`, or `table`. The default comes from the config file's top-level `defaultFormat` field when set; otherwise `compact` for row-producing commands (`exec`, `meta`) and `json` for structural commands (`list`, `test`, `install-skill`). `compact` is single-line JSON with column names listed once in `fields` and each row as a positional array (most token-efficient); `json` is minified single-line JSON with keys repeated per row; `table` is a padded text table for human reading.
- `--help`, `-h`: print help
- `--version`, `-V`: print version

The configuration path is passed via an environment variable:

```bash
AGENT_DATABASE_CLI_CONFIG=/path/to/config.json
```

## list

List supported database types, configured connections, and the configuration file path.

```bash
agent-database-cli list
agent-database-cli --format table list
```


Return values:

- On success, outputs JSON or a table to stdout
- The output includes `supported`, `configured`, and `configPath`
- If the configuration file does not exist, the supported list is still output and `configured` is empty
- The exit code is `0`

## test

Test a specified database connection.

```bash
agent-database-cli test --db "<databaseName>"
```

Return values:

- On success, outputs `{ "ok": true }` to stdout
- On connection failure, missing configuration, or authentication failure, outputs an error to stderr with exit code `1`

## exec

Uniformly execute SQL, Redis commands, or MongoDB JSON commands.

```bash
agent-database-cli exec --db "<databaseName>" --command "<command>"
```

Examples:

```bash
agent-database-cli exec --db local-mysql --command "select 1"
agent-database-cli exec --db cache --command "GET user:1"
agent-database-cli exec --db local-mongodb --command '{"find":{"collection":"users","filter":{},"limit":1}}'
```

Return values:

- On success, outputs `rows`, `fields`, and `rowCount` to stdout
- On a blacklist match, a read-only mode violation, or a command execution failure, outputs an error to stderr with exit code `1`
- In SQLcl Oracle mode, the SQLcl JSON output is parsed and, on success, the same unified `rows`, `fields`, and `rowCount` are returned; only when the output cannot be parsed as JSON is the raw text returned in an `output` field

## meta

Query database metadata.

```bash
agent-database-cli meta --db "<databaseName>" --type tables
agent-database-cli meta --db "<databaseName>" --type columns --table users
agent-database-cli meta --db "<databaseName>" --type collections
agent-database-cli meta --db "<databaseName>" --type keys --pattern "user:*"
```

Parameters:

- `--db <name>`: database configuration name
- `--type <type>`: `tables`, `columns`, `collections`, `keys`
- `--table <table>`: the table name required for a `columns` query
- `--pattern <pattern>`: Redis keys match pattern

Return values:

- On success, outputs the query result to stdout
- A metadata type not supported by the current database fails and returns an error

## format

Show, set, or clear the default output format (stored as top-level `defaultFormat` in the config file; `--format` on any call still overrides it).

```bash
agent-database-cli format             # show: { "defaultFormat": "table" } or null
agent-database-cli format table       # persist table as the default
agent-database-cli format --clear     # remove it, restoring built-in defaults
```

## Oracle SQLcl

When Oracle `oracledb` Thin mode does not support the target database version, you can switch to SQLcl.

```json
{
  "type": "oracle",
  "url": "oracle://USER:password@192.0.2.20:1521/qftest201",
  "oracleDriver": "sqlcl",
  "sqlclPath": "/opt/homebrew/Caskroom/sqlcl/26.1.0.086.1709/sqlcl/bin/sql",
  "javaHome": "/Applications/IntelliJ IDEA Ultimate.app/Contents/jbr/Contents/Home",
  "readonly": true,
  "blacklist": ["drop", "truncate", "delete", "update", "insert", "merge", "alter", "create"]
}
```

SQLcl mode passes the connection script via stdin to avoid the password appearing in the command-line argument list. The local blacklist and read-only checks still run before execution; the output is sliced by internal markers to extract the SQLcl query result and parsed into the unified result structure.

## Error Rules

- Fails when the configuration file JSON is invalid
- Fails when `databases` is missing or the database configuration name does not exist
- Fails on an unknown `type` or an unknown `oracleDriver`
- `exec` fails when `--db` or `--command` is missing
- `meta columns` fails when `--table` is missing
- Fails on a blacklist match, with an error indicating the command was rejected by the blacklist
- Fails on a read-only mode violation, with an error indicating the command was rejected by read-only mode
- All failures uniformly output an error message to stderr with exit code `1`
