# Changelog

## 0.4.3

- 修复 GitHub Actions 的 macOS 14 runner 退役导致发布失败，Intel/ARM 构建分别使用 macos-15-intel 和 macos-15。
- 包含 0.4.2 的页面连接测试功能；0.4.2 仅部分平台包发布成功。

## 0.4.2

- 编辑页面新增「测试连接」，使用当前草稿验证连接，支持未保存修改和 SSH 隧道，不写配置文件。
- 测试中禁用重复点击，返回成功、失败或超时提示；校验令牌、配置与密钥引用。Oracle SQLcl 仍使用 CLI 测试。
- 验证：56 项测试通过，5 项实库测试忽略；覆盖连接成功/失败、鉴权、非法配置和浏览器按钮交互。

## 0.4.1

- 修复同一 MySQL 连接配置下慢查询阻塞后续请求：使用连接池并释放查询期间的管理器锁，缺省池上限为 4；显式 `pool_min` 较大时相应提高缺省上限，保留 URL 池参数。
- 新增 `AGENT_DB_QUERY_TIMEOUT_SECS`，默认响应期限 60 秒，覆盖连接初始化、排队和查询读取；MySQL 超时或任务取消仅清理当前连接，避免连接池槽位被慢查询占满。
- 修复空闲回收与新请求之间的竞态，使用请求使用权跟踪执行和排队状态；`daemon status` 新增 `inFlight` 并准确显示 `busy`。
- 新增 `AGENT_DB_DAEMON_IDLE_SECS`，daemon 默认空闲退出时间调整为 1800 秒；活跃请求覆盖接收到响应写完，避免长请求或响应写入期间误退出。单库 `keepAliveSeconds` 默认仍为 180 秒。
- Oracle SQLcl 改为异步子进程，超时或取消时终止进程并清理临时脚本；原生驱动接入 OCI 取消，仅 Client ≥ 18.1 启用往返超时，保留 Client 11.2/12.1/12.2 的查询兼容性。
- 同步中英文 README 和两份内置 Agent SKILL，说明超时语义、连接池会话差异和环境变量重启要求。
- 验证：59 项测试全部通过，包含 5 项 MySQL 8.4 实库集成测试与旧 Oracle Client 版本分支回归；release 构建、版本一致性、格式及 npm 打包预览检查通过。Oracle 原生取消路径尚未使用实库验证。

## 0.4.0

- 连接配置新增可选单层 `group` 和多个 `tags`，兼容旧配置，标签去除首尾空白、空项与重复值。
- `list` 新增 `--group`、`--tag` 精确筛选，同时传入时取交集；保留 `configured` 名称数组，新增 `connections` 展示元数据，不输出 URL 或凭据。
- 编辑页面支持「全部 / 按分组 / 按标签」展示、分类折叠、搜索和分组/标签编辑，多标签入口编辑同一连接。
- 每轮列表渲染按连接复用脏状态和错误状态，减少多标签展示的重复计算。
- 标签删除改用归一化数组索引，修复含 CR/CRLF 的标签经 HTML 属性转换后无法删除的问题。
- 同步中英文 README、示例配置及两份内置 Agent SKILL；清理 npm lockfile 中仍指向旧版本平台产物的记录。
- 验证：43 项测试通过，包含真实 Chrome 分类展示、标签增删、CRLF/LF 标签分别删除、保存重读与清空标签；release 构建、版本一致性、CLI 筛选冒烟和格式检查通过。

## 0.3.0

- 新增 `editor` 命令：`editor start` 启动本地编辑服务并打开页面，`editor stop` 关闭，`editor status` 查看状态；服务只监听 `127.0.0.1`，`/api/*` 需要一次性访问令牌。
- 页面支持按数据库类型可视化编辑连接配置、实时预览将要写入的 JSON、就地编辑 JSON 文本，并展示本机到数据库的访问路径（含 SSH 跳板）。
- 保存走完整校验：JSON 结构、配置规则通过后先备份 `config.json.bak`，再以临时文件加 rename 原子替换，并保留原文件权限。
- 页面只读写配置文件原文，不触发明文密码迁移；`passwordRef` / `passphraseRef` 默认显示为已加密，点眼睛才按需解密，解密结果只用于显示、不写回配置文件。
- 右侧 JSON 面板可与磁盘内容做行级 diff，只在边栏标出新增行和修改行，保存后标记清空。
- 页面源码按 `core / api / render / actions / boot` 拆到 `rust/assets/editor/`，由 `build.rs` 拼成单文件 HTML 后内嵌，对外交付仍是纯 HTML，构建期不引入 JS 工具链。
- 新增两层页面回归防线：页面脚本静态自检（抓到过一次真实的「调用已删除函数」），以及基于 headless Chrome 的页面 smoke（密文显隐、URL 输入实时同步、JSON 面板范围）。
- 编辑服务新增空闲自动退出：超过 `10` 分钟没有操作会自己退出并清理状态文件；页面在有操作时按分钟打点保活，临近超时会提示。
- 页面在服务已退出时给出可照做的报错（提示重新执行 `editor start`），不再只抛 `Failed to fetch`；心跳失败也会提醒一次，避免继续编辑却保存不进去。
- 修复「已加密」状态胶囊里图标与文字没有垂直居中（胶囊高度由行盒改为固定高度 + 弹性居中），并加了对应的布局回归断言。
- 编辑服务状态文件支持 `AGENT_DATABASE_CLI_EDITOR_STATE_DIR` 覆盖目录，仅用于测试隔离。

## 0.2.19
- 安全优化：数据库 URL 明文密码、SSH 隧道密码和私钥口令在首次使用连接时自动迁移到本地加密存储，配置文件仅保留 `passwordRef` / `passphraseRef`。
- 兼容性优化：保留明文字段改密入口，重新填写明文密码后下次使用会覆盖旧密文。

## 0.2.18

- Bugfix: `daemon status` 在 daemon 未运行或 Unix socket 残留时返回明确的未运行状态，避免直接暴露 `Connection refused`。

## 0.2.17

- 性能优化：daemon 数据库请求不再持有全局配置锁，改为按数据库连接粒度串行执行；不同数据库的慢查询不会互相阻塞。
- 稳定性优化：daemon 首次初始化同一数据库连接时增加初始化占位，避免并发冷启动重复创建 SSH 隧道和数据库连接。
- 安全优化：Redis keys 元信息查询改用 `SCAN` 分批读取，避免使用阻塞式 `KEYS`；只读模式下 Redis `KEYS` 命令会被拒绝。
- 安全优化：只读模式额外拒绝 PostgreSQL `SELECT INTO`、CTE 后接写操作，以及 MongoDB aggregate `$out` / `$merge` 等具备写入语义的查询。
- 易用性优化：daemon 空响应时返回明确错误信息，避免暴露底层 `EOF while parsing a value`。
- 质量优化：清理 clippy 报出的冗余代码写法，保持 `cargo fmt`、`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过。

## 0.2.16

- Bugfix: 修复 Windows 下 daemon named pipe 客户端和服务端未实现，导致 `test`、`exec`、`meta`、`reset`、`daemon status` 等命令无法正常通过本地 daemon 工作的问题。
