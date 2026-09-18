---
name: agent-database-cli
description: 使用本地 agent-database-cli 安全操作已配置的数据库。适用于列出数据库连接、测试连接、执行 SQL/Redis/MongoDB 命令、查询表/列/集合/keys 元信息、管理本地连接 daemon，以及验证只读模式和命令黑名单的场景。
---

# agent-database-cli 使用说明

`agent-database-cli` 是一个基于本地配置的多数据库命令行工具，用于让 AI 或用户安全地操作数据库。

它能做的事：

- 列出支持的数据库类型和本地已配置数据库连接
- 测试指定数据库连接
- 执行 SQL、Redis 命令或 MongoDB JSON 命令
- 查询表、列、集合、Redis keys 等元信息
- 按单个数据库配置执行命令黑名单和只读模式
- 普通命令会按需自动启动本地 daemon，daemon 默认空闲 `1800` 秒后自动退出（可用 `AGENT_DB_DAEMON_IDLE_SECS` 覆盖）
- 通过本地 daemon 短时间保持连接，单个数据库连接默认空闲 `600` 秒释放
- daemon 在 Windows 使用 named pipe，在 macOS/Linux 使用 Unix socket
- 预编译二进制支持 macOS x64/arm64、Linux x64/arm64、Windows x64
- Oracle 默认使用 SQLcl；显式配置 `oracleDriver: "oracle"` 或 `"oracledb"` 时使用原生 Oracle 驱动

它不做的事：

- 不扫描网络或发现数据库，只使用配置文件中的连接
- 不绕过配置中的黑名单和只读模式
- 不输出未脱敏的密码、token、secret
- 不默认执行写入、删除、DDL 或其它危险命令

## 安全确认

执行任何可能写入、删除、修改结构或影响数据完整性的命令前，必须先确认目标数据库配置是否启用了 `readonly` 和 `blacklist`。

危险操作包括：

- DDL：`drop`、`truncate`、`alter`、`create`
- DML 写入：`insert`、`update`、`delete`、`merge`
- Redis 清空或写入：`flushall`、`flushdb`、`set`、`del`
- MongoDB 写入或删除：`insertOne`、`updateOne`、`deleteMany`、`drop`、`dropDatabase`
- 任何不可逆、影响生产数据、影响结构或权限的命令

如果用户明确要求执行危险命令，先说明目标数据库名、命令、可能影响，并等待用户明确同意。即使用户同意，也不能绕过本项目配置中的黑名单和只读模式。

黑名单优先级高于只读模式。命令执行前先检查 `blacklist`，命中后直接拒绝；未命中时再检查 `readonly`。

读取配置文件json前需要用户确认，防止密钥泄露。

## 环境校验

调用前优先检查 CLI 是否可用：

```bash
agent-database-cli --help
```


如果上面的命令失败，检查基础环境：

```bash
node --version
npm --version
```

如果依赖或构建产物缺失，在项目目录中执行：

```bash
npm install
npm run build
```

默认配置文件：

```text
~/.agent-database-cli/config.json
```

指定其它配置文件：

```bash
AGENT_DATABASE_CLI_CONFIG=/path/to/config.json agent-database-cli list
```

## 配置格式

配置文件是 JSON 对象，根字段为 `databases`：

```json
{
  "databases": {
    "local-mysql": {
      "type": "mysql",
      "url": "mysql://user:password@localhost:3306/app",
      "readonly": true,
      "blacklist": ["drop", "truncate", "delete"],
      "keepAliveSeconds": 600
    }
  }
}
```

字段：

- `type`: `mysql`、`postgres`、`redis`、`oracle`、`mongodb`
- `url`: 数据库连接 URL
- `passwordRef`: 数据库 URL 密码的本地密文引用，首次使用明文 URL 密码时自动生成
- `database`: MongoDB 默认数据库名，可选
- `readonly`: 是否启用只读模式
- `blacklist`: 命令黑名单数组，大小写不敏感
- `keepAliveSeconds`: daemon 连接空闲释放秒数，默认 `600`

查询 / 空闲相关环境变量（daemon **启动时**读取，改完需 `daemon stop` 再生效）：
- `AGENT_DB_QUERY_TIMEOUT_SECS`：单次请求响应超时，默认 `60`
- `AGENT_DB_DAEMON_IDLE_SECS`：daemon 空闲自动退出秒数，默认 `1800`

- `oracleDriver`: Oracle 驱动，可选 `oracledb` 或 `sqlcl`
- `sqlclPath`: SQLcl 可执行文件路径
- `javaHome`: SQLcl 使用的 `JAVA_HOME`
- `sshTunnel.passwordRef`: SSH 密码的本地密文引用，首次使用明文 `sshTunnel.password` 时自动生成
- `sshTunnel.passphraseRef`: SSH 私钥口令的本地密文引用，首次使用明文 `sshTunnel.passphrase` 时自动生成

首次使用连接时，CLI 会把数据库 URL 明文密码、`sshTunnel.password`、`sshTunnel.passphrase` 加密保存到配置目录的 `secrets.json`，生成本地 `secret.key`，并把配置文件改写为对应 `*Ref`。后续只在内存中解密使用；改密码时重新填入明文字段即可覆盖旧密文。

## 全局参数

- `--format <format>`: 输出格式，支持 `json` 或 `table`，默认 `json`
- `--help`, `-h`: 输出帮助
- `--version`, `-V`: 输出版本

配置路径通过环境变量传递：

```bash
AGENT_DATABASE_CLI_CONFIG=/path/to/config.json
```

## list

列出支持的数据库类型、已配置连接和配置文件路径。

```bash
agent-database-cli list
agent-database-cli --format table list
```


返回值：

- 成功时 stdout 输出 JSON 或表格
- 输出包含 `supported`、`configured`、`configPath`
- 配置文件不存在时仍会输出支持列表，`configured` 为空
- 退出码为 `0`

## test

测试指定数据库连接。

```bash
agent-database-cli test --db "<databaseName>"
```

返回值：

- 成功时 stdout 输出 `{ "ok": true }`
- 连接失败、配置缺失或认证失败时 stderr 输出错误，退出码为 `1`

## exec

统一执行 SQL、Redis 命令或 MongoDB JSON 命令。

```bash
agent-database-cli exec --db "<databaseName>" --command "<command>"
```

示例：

```bash
agent-database-cli exec --db local-mysql --command "select 1"
agent-database-cli exec --db cache --command "GET user:1"
agent-database-cli exec --db local-mongodb --command '{"find":{"collection":"users","filter":{},"limit":1}}'
```

返回值：

- 成功时 stdout 输出 `rows`、`fields`、`rowCount`
- 命中黑名单、违反只读模式、命令执行失败时 stderr 输出错误，退出码为 `1`
- SQLcl Oracle 模式会解析 SQLcl JSON 输出，成功时同样返回统一的 `rows`、`fields`、`rowCount`；仅在无法解析为 JSON 时才以 `output` 字段返回原始文本

## meta

查询数据库元信息。

```bash
agent-database-cli meta --db "<databaseName>" --type tables
agent-database-cli meta --db "<databaseName>" --type columns --table users
agent-database-cli meta --db "<databaseName>" --type collections
agent-database-cli meta --db "<databaseName>" --type keys --pattern "user:*"
```

参数：

- `--db <name>`: 数据库配置名
- `--type <type>`: `tables`、`columns`、`collections`、`keys`
- `--table <table>`: `columns` 查询所需表名
- `--pattern <pattern>`: Redis keys 匹配模式

返回值：

- 成功时 stdout 输出查询结果
- 当前数据库不支持的元信息类型会失败并返回错误

## daemon

管理本地连接守护进程。普通 `test`、`exec`、`meta`、`reset` 命令会在 daemon 未运行时自动启动 daemon，已运行时直接复用，不会重复启动。daemon 使用 Unix socket，不暴露网络端口，默认空闲 `1800` 秒后自动退出（可用 `AGENT_DB_DAEMON_IDLE_SECS` 覆盖）。

```bash
agent-database-cli daemon start
agent-database-cli daemon status
agent-database-cli daemon stop
```

返回值：

- `start` 成功时输出 socket 路径
- `status` 成功时输出当前连接列表
- `stop` 成功时输出停止结果

## reset

重置指定数据库连接。

```bash
agent-database-cli reset --db "<databaseName>"
```

如果 daemon 正在运行，会断开并清理该数据库连接；下一次命令会重新连接。

## Oracle SQLcl

当 Oracle `oracledb` Thin mode 不支持目标库版本时，可以改用 SQLcl。

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

SQLcl 模式通过 stdin 传入连接脚本，避免密码出现在命令行参数列表中。执行前仍会先走本地黑名单和只读检查；输出会按内部标记截取 SQLcl 查询结果并解析为统一结果结构。

## 错误规则

- 配置文件 JSON 无效时失败
- `databases` 缺失或数据库配置名不存在时失败
- 未知 `type`、未知 `oracleDriver` 或非法 `keepAliveSeconds` 会失败
- `exec` 缺少 `--db` 或 `--command` 会失败
- `meta columns` 缺少 `--table` 会失败
- 命中黑名单时失败，错误中包含 `黑名单拒绝执行命令`
- 违反只读模式时失败，错误中包含 `只读模式拒绝执行命令`
- 所有失败统一在 stderr 输出错误信息，退出码为 `1`
