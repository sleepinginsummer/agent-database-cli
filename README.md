<div align="center">

# agent-database-cli

基于 CLI 的多数据库操作工具，将常见数据库连接、查询、元信息读取和连接复用能力封装为 Agent 可调用的本地命令。

MySQL · PostgreSQL · Redis · Oracle · MongoDB · 只读模式 · 命令黑名单 · SQLcl Oracle · 本地 daemon

<p>
  <img src="https://img.shields.io/badge/CLI-agent--database--cli-2ea44f" alt="CLI agent-database-cli">
  <img src="https://img.shields.io/badge/License-MIT-green" alt="License MIT">
  <img src="https://img.shields.io/badge/Node.js-%3E%3D20-339933?logo=node.js&logoColor=white" alt="Node.js >=20">
  <img src="https://img.shields.io/badge/npm-%3E%3D10-CB3837?logo=npm&logoColor=white" alt="npm >=10">
  <img src="https://img.shields.io/badge/sys-win%2Fmac%2Flinux-0078D6" alt="sys win/mac/linux">
  <img src="https://img.shields.io/badge/release-v0.2.20-blue" alt="release v0.2.20">
</p>

[AI 一键安装](#ai-一键安装) · [安装](#安装) · [配置](#配置) · [权限配置](#权限配置) · [Oracle SQLcl](#oracle-sqlcl) · [许可证](#许可证) · [友情链接](#友情链接)

中文 | [English](README_EN.md)

</div>

## 简介

`agent-database-cli` 是一个面向 Agent 的本地多数据库 CLI 工具，用 Rust 封装 MySQL、PostgreSQL、Redis、Oracle、MongoDB 的连接、查询、元信息读取、只读控制、命令黑名单和密码加密能力。

它能做的事：

- 列出当前支持的数据库类型和本地已配置连接
- 对指定数据库执行 SQL、Redis 命令或 MongoDB JSON 命令
- 查询数据库元信息，例如表、列、集合、Redis keys；Redis keys 元信息使用 `SCAN` 分批读取，避免阻塞式 `KEYS`
- 按单个数据库配置启用只读模式和命令黑名单
- Oracle 默认使用 SQLcl；需要 Oracle Instant Client 时可显式切换到 `oracle`/`oracledb` 原生驱动
- 不保存或输出脱敏前的密码、token、secret

驱动配置表：

| 数据库 | `type` | 默认驱动 | 驱动切换配置 |
| --- | --- | --- | --- |
| MySQL | `mysql` | Rust 原生驱动 `mysql_async` | 暂不支持切换 |
| PostgreSQL | `postgres` | Rust 原生驱动 `tokio-postgres` | 暂不支持切换 |
| Redis 单机 | `redis` | Rust 原生驱动 `redis` | 仅配置 `url` |
| Redis 集群 | `redis` | Rust 原生驱动 `redis` | 同时配置 `url` 和 `redisCluster.nodes` |
| Oracle | `oracle` | SQLcl | 支持两种驱动：默认 SQLcl （需要自行安装）；可切换oracledb（需要 Oracle Instant Client） |
| MongoDB | `mongodb` | Rust 原生驱动 `mongodb` | 暂不支持切换；可配 `database` 指定默认库 |

## 安装

### 环境要求

- Node.js `>= 20`
- npm `>= 10`
- 系统支持 Windows / macOS / Linux
- 安装时会自动拉取对应平台的 Rust 二进制子包，当前支持 macOS x64/arm64、Linux x64/arm64、Windows x64
- 本机网络可访问目标数据库
- 如使用 Docker 集成测试，需要 Docker 和 Docker Compose
- 如 Oracle 使用 SQLcl，需要本机可运行 SQLcl 和 Java21


### AI 一键安装

```text
安装请阅读 https://github.com/sleepinginsummer/agent-database-cli/blob/main/AI_INSTALL.md，按说明安装 CLI 并添加 `SKILL.md`。
```

### 手动全局安装

```bash
npm install -g agent-database-cli
agent-database-cli --help
```

如果 npm 包安装受限，使用等价的源码安装方式：

```powershell
git clone https://github.com/sleepinginsummer/agent-database-cli.git
cd agent-database-cli
npm install
npm run build
npm link
agent-database-cli --help
```

安装/更新 Agent skill：

```bash
agent-database-cli install-skill --dry-run
agent-database-cli install-skill
```

## 配置

默认配置文件：

```text
~/.agent-database-cli/config.json
```

可以通过环境变量修改配置位置：

```bash
AGENT_DATABASE_CLI_CONFIG=/path/to/config.json agent-database-cli list
```

配置文件是一个对象，`databases` 中每个 key 是一个数据库连接名。

连接配置：

| 字段 | 适用范围 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `type` | 全部数据库 | 无 | 数据库类型，支持 `mysql`、`postgres`、`redis`、`oracle`、`mongodb` |
| `url` | 全部数据库 | 无 | 数据库连接 URL；Redis 单机模式直接连接该地址，Redis 集群模式下作为入口节点 URL |
| `passwordRef` | 全部数据库 | 无 | 数据库 URL 密码的本地密文引用；首次使用明文 URL 密码时自动生成 |
| `database` | MongoDB | 无 | MongoDB 默认数据库名 |
| `oracleDriver` | Oracle | `sqlcl` | Oracle 驱动：`sqlcl` 或原生驱动 |
| `sqlclPath` | Oracle SQLcl | 无 | SQLcl 可执行文件路径，仅 `oracleDriver: "sqlcl"` 时使用 |
| `javaHome` | Oracle SQLcl | 无 | SQLcl 使用的 `JAVA_HOME` |
| `redisCluster` | Redis | 无 | Redis 集群配置，配置后会使用 Redis Cluster 模式 |
| `sshTunnel` | 全部数据库 | 无 | SSH 隧道配置；单机模式转发数据库 URL 的 host/port，Redis 集群模式为每个节点分别建立本地转发 |
| `readonly` | 全部数据库 | `true` | 是否启用只读模式；仅在明确需要写入时才建议显式设为 `false` |
| `blacklist` | 全部数据库 | 无 | 命令黑名单数组，大小写不敏感 |
| `keepAliveSeconds` | 全部数据库 | `600` | 单个数据库连接空闲释放秒数 |

PostgreSQL URL 支持 `sslmode` 参数：`disable`、`prefer`、`require`、`verify-ca`、`verify-full`。例如云数据库常用 `postgres://user:password@host:5432/app?sslmode=require`；生产环境需要校验证书时优先使用 `verify-full`。

Redis 集群配置：

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `nodes` | 无 | Redis 集群节点 URL 数组，至少配置一个，支持 `redis://` 和 `rediss://` |

Redis 集群使用规则：

| 场景 | 要求 |
| --- | --- |
| 启用集群模式 | 必须同时配置 `url` 和 `redisCluster.nodes` |
| `url` | 用作集群入口节点，建议填写任意一个稳定可达的集群节点 URL |
| `redisCluster.nodes` | 用作集群节点清单；如走 SSH 隧道，也用于为每个节点建立本地转发和地址映射 |
| 同时配置 `sshTunnel` | 程序会给每个集群节点分别建立本地端口转发，并通过地址映射接管集群节点跳转 |
| 通过 SSH 隧道访问集群 | `redisCluster.nodes` 需要覆盖客户端实际可能访问到的集群节点地址 |

SSH 隧道配置支持密码、私钥、密码加私钥、带通行短语的私钥认证。

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `host` | 无 | SSH 跳板机地址 |
| `port` | `22` | SSH 端口 |
| `username` | 无 | SSH 用户名 |
| `password` | 无 | SSH 密码，可选 |
| `passwordRef` | 无 | SSH 密码的本地密文引用；首次使用明文 `password` 时自动生成 |
| `privateKeyPath` | 无 | 私钥文件路径，可选，支持 `~` |
| `privateKey` | 无 | 私钥内容，可选，和 `privateKeyPath` 二选一 |
| `passphrase` | 无 | 私钥通行短语，可选，仅配置私钥时允许使用 |
| `passphraseRef` | 无 | 私钥通行短语的本地密文引用；首次使用明文 `passphrase` 时自动生成 |
| `readyTimeout` | 无 | SSH 连接超时时间，单位毫秒，可选 |

敏感信息会在首次使用对应连接时被动加密保存。数据库 URL 中的明文密码、`sshTunnel.password` 和 `sshTunnel.passphrase` 会写入配置目录下的 `secrets.json`，本地密钥写入 `secret.key`，并把配置中的明文字段清空或移除密码内容后写入对应 `*Ref`。后续运行只通过引用解密到内存中使用；如需修改密码，把明文字段重新填成新值，下次使用会覆盖旧密文。

安全策略：

| 策略 | 说明 |
| --- | --- |
| 检查优先级 | 先检查黑名单，命中直接拒绝；未命中再检查只读模式 |
| 只读默认值 | 默认启用只读模式，未显式配置 `readonly` 时也会拒绝写操作 |
| 推荐用法 | 所有数据库连接默认保持只读，需要变更数据时，让 AI 先给出对应 SQL 或命令，再由你确认后执行 |
| 写入配置 | 某个连接确实需要写入时，再单独将该连接配置为 `readonly: false` |

参考配置：

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


## 权限配置

权限控制建议同时使用 `readonly` 和 `blacklist`，不要只依赖其中一个。

### 只读模式

- 默认值是 `true`
- 不配置 `readonly` 时，仍然会按只读模式处理
- 只读模式会额外拒绝存在写入语义的查询，例如 PostgreSQL `SELECT INTO` 和 MongoDB aggregate 中的 `$out`、`$merge`
- 推荐所有日常查询连接都保持默认只读
- 需要修改数据时，建议先让 AI 生成对应 SQL 或命令，再由你确认后执行
- 只有明确需要写入的专用连接，才单独配置 `readonly: false`

### 命令黑名单

- 黑名单优先级高于只读模式
- 命中黑名单后会直接拒绝，不再继续判断是否只读
- 适合拦截高危命令，避免误执行删库、删表、结构变更、批量写入、清空缓存等操作
- 建议生产库、共享测试库、线上 Redis 都配置黑名单

### 执行顺序

1. 先检查 `blacklist`
2. 命中则直接拒绝
3. 未命中再检查 `readonly`
4. `readonly` 生效时只允许读命令

### 常见高危命令

MySQL / PostgreSQL / Oracle 常见高危 SQL：

```json
["drop", "truncate", "delete", "update", "insert", "merge", "alter", "create", "replace", "grant", "revoke"]
```

Redis 常见高危命令：

```json
["flushall", "flushdb", "del", "unlink", "set", "mset", "expire", "rename", "hset", "lpush", "rpush", "sadd", "zadd", "keys"]
```

MongoDB 常见高危命令：

```json
["insertOne", "insertMany", "updateOne", "updateMany", "replaceOne", "deleteOne", "deleteMany", "findAndModify", "findOneAndUpdate", "findOneAndDelete", "drop", "dropDatabase", "createIndex", "dropIndex", "$out", "$merge"]
```

### 推荐配置示例

生产库推荐：

```json
{
  "type": "mysql",
  "url": "mysql://user:password@prod-db:3306/app",
  "readonly": true,
  "blacklist": ["drop", "truncate", "delete", "update", "insert", "alter", "create"],
  "keepAliveSeconds": 600
}
```

允许写入的专用连接推荐：

```json
{
  "type": "postgres",
  "url": "postgres://user:password@write-db:5432/app",
  "readonly": false,
  "blacklist": ["drop", "truncate", "alter"],
  "keepAliveSeconds": 600
}
```


Oracle 保留双驱动设计：

- 不配置 `oracleDriver`：默认 SQLcl。
- `oracleDriver: "sqlcl"`：显式使用 SQLcl，适合 Oracle 11 等老库、无法安装 Instant Client 或原生驱动兼容性不稳定的环境。
- `oracleDriver: "oracle"`：显式使用 Rust Oracle 原生驱动，依赖 Oracle Instant Client / ODPI-C。
- `oracleDriver: "oracledb"`：Node 版原生驱动兼容值；Rust 版按 `oracle` 原生入口处理。

当前默认入口已切换为 Rust 原生 CLI，并通过 npm 平台子包分发 Windows、Linux、macOS 二进制；Oracle 默认 SQLcl，原生 Oracle 驱动需显式配置。

## Oracle SQLcl

官方链接：https://www.oracle.com/database/sqldeveloper/technologies/sqlcl/

Oracle 默认使用 SQLcl，避免默认依赖 Oracle Instant Client，也更适合 Oracle 11 等老库。可以不配置 `oracleDriver`，或显式配置为 SQLcl：

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

SQLcl 模式会通过 stdin 传入连接脚本，避免密码出现在命令行参数列表中。安全检查仍在执行前完成，黑名单和只读模式都会生效。

## 更新

```bash
npm install -g agent-database-cli@latest
```

## 卸载和清理

```bash
npm uninstall -g agent-database-cli
npm cache clean --force
rm -rf ~/.agent-database-cli
```

## 许可证

[MIT](LICENSE)

## 友情链接

- [LINUX DO - 新的理想型社区](https://linux.do/)

## 环境变量（daemon）

这些变量在 **daemon 进程启动时** 读取；修改后需要 `daemon stop` 再让下一次命令拉起新 daemon，才会生效。

| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `AGENT_DB_QUERY_TIMEOUT_SECS` | `60` | 单次 test / execute / metadata 的响应超时（秒）。超时后客户端停止等待；Oracle SQLcl 会 kill 子进程，Oracle 原生驱动会设置 OCI call timeout。 |
| `AGENT_DB_DAEMON_IDLE_SECS` | `1800` | daemon 在无 in-flight 请求且空闲达到该秒数后自动退出。 |

连接级 `keepAliveSeconds` 默认已改为 `600`（未配置时）。
