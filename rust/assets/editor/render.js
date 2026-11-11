  /* ==========================================================
     渲染：轨道
     ========================================================== */
  function renderRail() {
    const list = $("#railList");
    const names = Object.keys(databases());
    // 每次渲染内按连接复用状态，供签名和各标签入口共享，不跨次缓存。
    const rowStates = new Map(names.map((name) => [name, { issueCount: issueFor(name).length, dirty: rowDirty(name) }]));
    const signature = JSON.stringify([S.listMode, S.search, names.map((name) => {
      const db = databases()[name];
      return [name, db.type, db.url, databaseGroup(db), databaseTags(db), rowStates.get(name).issueCount, rowStates.get(name).dirty, name === S.selected];
    })]);
    if (signature === S.railSignature) return;
    S.railSignature = signature;
    $("#railCount").textContent = String(names.length).padStart(2, "0");
    const scrollTop = list.scrollTop;
    const scrollLeft = list.scrollLeft;
    const focused = document.activeElement;
    const focusedRow = list.contains(focused) ? focused.closest(".row") : null;
    if (list.dataset.searching !== "true") {
      $$("details", list).forEach((section) => S.collapsedSections.set(section.dataset.key, !section.open));
    }
    const sections = connectionSections(databases(), S.listMode, S.search);
    const row = (name, index, sectionKey) => {
      const db = databases()[name];
      const type = String(db.type || "mysql");
      const state = rowStates.get(name);
      const bad = state.issueCount > 0;
      const cls = ["row"];
      if (name === S.selected) cls.push("on");
      if (bad) cls.push("bad");
      if (state.dirty) cls.push("dirty");
      return '<li><button class="' + cls.join(" ") + '" data-name="' + esc(name) + '" data-section="' + esc(sectionKey) + '" style="--row-accent:var(--' + esc(type) + ');animation-delay:' + (index * 26) + 'ms">' +
        '<span class="dot"></span><span class="meta"><span class="nm">' + esc(name) + '</span><span class="ty" style="color:var(--' + esc(type) + ')">' + esc(type) + '</span></span>' +
        '<span class="flag">' + (bad ? icon("circle-alert", "i-s") : '<span class="dot-dirty"></span>') + '</span></button></li>';
    };
    list.innerHTML = sections.length ? sections.map(({ key, names }) => {
      const sectionKey = JSON.stringify([S.listMode, key]);
      const rows = names.map((name, index) => row(name, index, sectionKey)).join("");
      if (S.listMode === "all") return rows;
      const title = key === null ? (S.listMode === "tags" ? "无标签" : "未分组") : key;
      const open = S.search.trim() || !S.collapsedSections.get(sectionKey);
      return '<li class="rail-section"><details data-key="' + esc(sectionKey) + '"' + (open ? ' open' : '') + '><summary>' + esc(title) + '<b>' + names.length + '</b></summary><ul class="section-rows">' + rows + '</ul></details></li>';
    }).join("") : '<li class="rail-empty">' + (names.length ? "未找到连接" : "还没有连接配置") + '</li>';
    list.dataset.mode = S.listMode;
    list.dataset.searching = String(Boolean(S.search.trim()));
    list.scrollTop = scrollTop;
    list.scrollLeft = scrollLeft;
    if (focusedRow) {
      const target = $$(".row", list).find((button) => button.dataset.name === focusedRow.dataset.name && button.dataset.section === focusedRow.dataset.section);
      if (target) target.focus({ preventScroll: true });
    }
  }

  function issueFor(name) {
    return S.issues.filter((text) => text.indexOf(name) >= 0);
  }

  /* ==========================================================
     渲染：线路
     ========================================================== */
  function renderWire() {
    const db = current();
    const parsed = parseUrl(db.url);
    const tunnel = db.sshTunnel || null;
    const parts = [];
    parts.push('<span class="node local">' + icon("square-terminal") + '<span class="n-k">本机</span></span>');

    if (tunnel && (tunnel.host || tunnel.username)) {
      parts.push('<span class="link" style="--d:.2s"></span>');
      const label = (tunnel.username ? tunnel.username + "@" : "") + (tunnel.host || "?") + (tunnel.port ? ":" + tunnel.port : ":22");
      parts.push('<span class="node ssh">' + icon("git-branch") + '<span class="n-v">' + esc(label) + '</span></span>');
    }

    parts.push('<span class="link' + (parsed.ok ? "" : " dim") + '" style="--d:.5s"></span>');
    if (!parsed.ok) {
      parts.push('<span class="node bad">' + icon("triangle-alert") + '<span class="n-v">URL 无法解析</span></span>');
    } else {
      const target = parsed.host ? parsed.host + (parsed.port ? ":" + parsed.port : "") : "地址缺失";
      parts.push('<span class="node target">' + icon("server") + '<span class="n-v">' + esc(target) + '</span></span>');
      if (parsed.database) {
        parts.push('<span class="wire-sep">/</span><span class="wire-db">' + esc(parsed.database) + '</span>');
      }
    }
    const html = parts.join("");
    const wire = $("#wire");
    if (wire.innerHTML !== html) wire.innerHTML = html;
  }

  /* ==========================================================
     渲染：表单
     ========================================================== */
  function field(label, controlHtml, options) {
    const opts = options || {};
    const cls = ["field"];
    if (opts.wide) cls.push("wide");
    return '<div class="' + cls.join(" ") + '">' +
      '<span class="lab">' + (opts.icon ? icon(opts.icon, "i-s") : "") + '<span>' + esc(label) + '</span>' +
      (opts.optional ? '<span class="opt">可选</span>' : "") + '</span>' +
      controlHtml + '</div>';
  }

  function textInput(bind, value, options) {
    const opts = options || {};
    return '<input class="inp" data-bind="' + esc(bind) + '" data-kind="' + (opts.kind || "text") + '"' +
      ' value="' + esc(value == null ? "" : value) + '"' +
      ' placeholder="' + esc(opts.placeholder || "") + '" spellcheck="false" autocomplete="off"' +
      ' aria-label="' + esc(opts.label || bind) + '">';
  }

  // 密文只在用户点眼睛时按需拉取，解密结果只留在内存，不回写配置
  async function fetchSecret(reference) {
    try {
      const payload = await api("/api/secret", { method: "POST", body: JSON.stringify({ ref: reference }) });
      return payload.value;
    } catch (error) {
      toast(error.message, "err");
      return null;
    }
  }

  const hasCachedSecret = (reference) => Object.prototype.hasOwnProperty.call(S.secrets, reference);
  const isShown = (field) => Boolean(S.shown[field]);

  // 解密预览只落到 DOM property：属性里只有 ref，outerHTML 和表单序列化都拿不到明文
  function applySecretFields() {
    $$("[data-secret-source]").forEach((input) => {
      const reference = input.dataset.secretSource;
      if (hasCachedSecret(reference)) input.value = S.secrets[reference];
    });
  }

  // 密文控件的唯一构造入口：输入框属性、状态 chip、眼睛图标都由 field + 数据派生
  function secretControl(field, options) {
    const opts = options || {};
    const shown = isShown(field);
    const reference = opts.reference || "";
    const plain = opts.plain || "";
    const preview = Boolean(shown && !plain && reference && hasCachedSecret(reference));
    const decryptable = Boolean(!plain && reference);
    const attrs = [];
    if (opts.id) attrs.push(' id="' + opts.id + '"');
    if (opts.bind) attrs.push(' data-bind="' + esc(opts.bind) + '" data-kind="secret"');
    attrs.push(' type="' + (shown ? "text" : "password") + '"');
    attrs.push(preview ? ' data-secret-source="' + esc(reference) + '"' : ' value="' + esc(plain) + '"');
    attrs.push(' placeholder="' + esc(opts.placeholder || "") + '" spellcheck="false" autocomplete="new-password"');
    attrs.push(' aria-label="' + esc(opts.label || field) + '"');
    if (opts.disabled) attrs.push(" disabled");
    const chip = preview
      ? '<span class="pw-state">解密预览</span>'
      : (plain ? '<span class="pw-state plain">明文</span>'
        : (reference ? '<span class="pw-state">' + icon("lock", "i-s") + '已加密</span>' : ""));
    const action = decryptable ? "toggle-secret" : "toggle-password";
    const title = shown ? "隐藏" : (decryptable ? "解密并显示" : "显示");
    return '<input class="inp"' + attrs.join("") + '>' +
      '<span class="pw-tools">' + chip +
      '<button class="icon-btn" type="button" data-act="' + action + '"' +
      (decryptable ? ' data-ref="' + esc(reference) + '"' : "") + ' data-field="' + esc(field) + '" title="' + title + '">' +
      icon(shown ? "eye-off" : "eye", "i-s") + '</button>' +
      (opts.clearable ? '<button class="icon-btn danger" type="button" data-act="clear-password" title="移除凭据">' + icon("trash-2", "i-s") + '</button>' : "") +
      '</span>';
  }

  // 当前连接下各密文字段的数据来源
  function secretOptions(field) {
    const db = current();
    if (field === "url") {
      const parsed = parseUrl(db.url);
      return {
        id: "pwInput",
        reference: db.passwordRef,
        plain: parsed.password,
        label: "数据库密码",
        placeholder: db.passwordRef && !parsed.password ? "使用已保存的密文" : "数据库密码",
        disabled: !parsed.ok,
        clearable: true
      };
    }
    const tunnel = db.sshTunnel || {};
    if (field === "sshTunnel.password") {
      return { bind: field, reference: tunnel.passwordRef, plain: tunnel.password || "", label: "SSH 密码", placeholder: "SSH 密码" };
    }
    return { bind: field, reference: tunnel.passphraseRef, plain: tunnel.passphrase || "", label: "私钥口令", placeholder: "私钥口令" };
  }

  function passwordField() {
    return '<div class="pw-wrap" data-secret-field="url">' + secretControl("url", secretOptions("url")) + '</div>';
  }

  // 只重画一个密文控件，不动表单其它部分
  function refreshSecretField(field) {
    const wrap = $('[data-secret-field="' + field + '"]');
    if (!wrap) return;
    wrap.innerHTML = secretControl(field, secretOptions(field));
    applySecretFields();
  }

  // 明暗切换同理，只重画这一个字段，也不丢焦点
  function setSecretVisibility(field, shown) {
    S.shown[field] = shown;
    refreshSecretField(field);
  }

  function chipsField(values, fieldName) {
    const tags = fieldName === "tags";
    const chips = (values || []).map((value, index) =>
      '<span class="chip"><span>' + esc(value) + '</span><button type="button" data-act="' + (tags ? "rm-tag" : "rm-chip") + '"' +
      (tags ? ' data-index="' + index + '"' : ' data-value="' + esc(value) + '"') +
      ' aria-label="' + (tags ? "移除标签 " + esc(value) : "移除") + '">' + icon("x", "i-s") + '</button></span>'
    ).join("");
    return '<div class="chips" data-chips="' + (tags ? "tags" : "blacklist") + '">' + chips +
      '<input class="chip-input" id="' + (tags ? "tagInput" : "chipInput") + '"' + (tags ? ' role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="tagOptions"' : '') + ' placeholder="' + ((values || []).length ? "继续添加" : (tags ? "输入或选择标签后回车" : "输入命令后回车")) + '" spellcheck="false" autocomplete="off" aria-label="' + (tags ? "添加标签" : "添加黑名单命令") + '"></div>';
  }

  let boardHtml = "";

  function renderBoard() {
    const names = Object.keys(databases());
    if (!names.length) {
      $("#boardIn").innerHTML = '<div class="empty">' + icon("database", "i-l") +
        '<p>还没有连接配置</p>' +
        '<button class="btn btn-primary" data-act="add">' + icon("plus", "i-s") + '<span>新建连接</span></button></div>';
      return;
    }
    if (!S.selected || !databases()[S.selected]) setSelected(names[0]);
    const db = current();
    const type = String(db.type || "mysql");
    const parsed = parseUrl(db.url);
    const issues = issueFor(S.selected);
    const sections = [
      renderHead(db, type, parsed, issues),
      renderConnectionGroup(db, type, parsed),
      renderClassificationGroup(db),
      renderCredentialGroup(db, type, parsed),
      renderSecurityGroup(db, type),
      renderSshGroup(db),
      renderRedisClusterGroup(db, type),
      migrationStrip(parsed)
    ].join("");
    // 同样的内容不重复替换 DOM；缓存放 JS 变量，避免把含敏感值的渲染串写进 DOM 属性
    if (boardHtml !== sections) {
      boardHtml = sections;
      $("#boardIn").innerHTML = sections;
      applySecretFields();
    }
    renderWire();
  }

  // 分组外壳：标题、状态标记和启停开关样式统一在这里生成
  function group(options, body) {
    return '<section class="group' + (options.off ? " off" : "") + '"' + (options.key ? ' data-group="' + options.key + '"' : "") + ' style="animation-delay:' + options.delay + 'ms">' +
      '<header class="group-head">' +
      '<span class="group-title">' + icon(options.icon, "i-s") + '<span>' + esc(options.title) + '</span></span>' +
      (options.flag ? '<span class="flag"' + (options.flagTone ? ' style="color:' + options.flagTone + '"' : "") + '>' + esc(options.flag) + '</span>' : "") +
      (options.toggle ? '<button class="switch" role="switch" data-switch="' + options.toggle.key + '" aria-checked="' + options.toggle.on + '" aria-label="' + esc(options.toggle.label) + '"></button>' : "") +
      '</header>' + (options.off ? "" : body) + '</section>';
  }

  function renderHead(db, type, parsed, issues) {
    return '<div class="head">' +
      '<div class="head-top">' +
      '<span class="eyebrow">' + icon("database", "i-s") + '<span>' + esc(type) + '</span></span>' +
      '<div class="head-tools">' +
      '<span id="connectionResult" class="connection-result" role="status"></span>' +
      '<button class="btn" type="button" data-act="test-connection"' + (testingConnection ? ' disabled' : '') + '>' +
      (testingConnection ? '测试中…' : '测试连接') + '</button>' +
      '<button class="icon-btn" data-act="duplicate" title="复制连接">' + icon("copy", "i-s") + '</button>' +
      '<button class="icon-btn danger" data-act="remove" title="删除连接">' + icon("trash-2", "i-s") + '</button>' +
      '</div></div>' +
      '<input class="name' + (issues.length ? " bad" : "") + '" id="nameInput" value="' + esc(S.selected) + '" spellcheck="false" autocomplete="off" aria-label="连接名称">' +
      '<div class="wire" id="wire"></div>' +
      (issues.length ? '<div class="strip">' + icon("triangle-alert", "i-s") + '<span>' + esc(issues.join("；")) + '</span></div>' : "") +
      '</div>';
  }

  function renderConnectionGroup(db, type, parsed) {
    const body = '<div class="grid">' +
      field("类型", '<div class="seg" data-seg="type">' + TYPES.map((item) =>
        '<button data-value="' + item.id + '" data-focus-key="type-' + item.id + '" style="--seg-accent:var(--' + item.id + ')" aria-pressed="' + (item.id === type) + '">' +
        icon("database", "i-s") + '<span>' + item.label + '</span></button>').join("") + '</div>', { wide: true, icon: "database" }) +
      field("URL", textInput("url", parsed.ok ? parsed.view : (db.url || ""), { label: "连接 URL", placeholder: (TYPES.find((item) => item.id === type) || TYPES[0]).hint }), { wide: true, icon: "type" }) +
      (type === "mongodb" ? field("默认数据库", textInput("database", db.database, { label: "默认数据库", placeholder: "app" }), { icon: "hash" }) : "") +
      field("连接空闲释放", textInput("keepAliveSeconds", db.keepAliveSeconds, { kind: "number", label: "连接空闲释放秒数", placeholder: "180" }), { icon: "hash" }) +
      '</div>' +
      (parsed.params.length ? '<div class="suggest">' + parsed.params.map((pair) =>
        '<span class="chip">' + esc(pair[0]) + (pair[1] ? "=" + esc(pair[1]) : "") + '</span>').join("") + '</div>' : "");
    return group({ title: "连接", icon: "plug-zap", delay: 40, flag: parsed.ok ? (parsed.protocol || "—") : "URL 无法解析", flagTone: parsed.ok ? "" : "var(--danger)" }, body);
  }

  function renderClassificationGroup(db) {
    const groupControl = '<div class="suggest-field"><input class="inp" data-bind="group" value="' + esc(databaseGroup(db) || "") + '" role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="groupOptions" placeholder="未分组" autocomplete="off" aria-label="分组"><div class="suggest-menu" id="groupOptions" role="listbox" aria-label="分组" hidden></div></div>';
    const body = '<div class="grid">' +
      field("分组", groupControl, { icon: "folder-open" }) +
      field("标签", '<div class="suggest-field">' + chipsField(databaseTags(db), "tags") + '<div class="suggest-menu" id="tagOptions" role="listbox" aria-label="标签" hidden></div></div>', { icon: "hash" }) + '</div>';
    return group({ title: "分组与标签", icon: "folder-open", delay: 65 }, body);
  }

  function renderCredentialGroup(db, type, parsed) {
    const rows = [field("密码", passwordField(), { wide: true, icon: "key-round" })];
    if (type === "oracle") {
      const driver = String(db.oracleDriver || "sqlcl");
      rows.push(field("驱动", '<div class="seg" data-seg="oracleDriver">' + ORACLE_DRIVERS.map((item) =>
        '<button data-value="' + item + '" data-focus-key="driver-' + item + '" aria-pressed="' + (item === driver) + '">' +
        '<span>' + item + '</span></button>').join("") + '</div>', { wide: true, icon: "sliders-horizontal" }));
      if (driver === "sqlcl") {
        rows.push(field("SQLcl 路径", textInput("sqlclPath", db.sqlclPath, { label: "SQLcl 可执行文件路径", placeholder: "/opt/sqlcl/bin/sql" }), { wide: true, icon: "folder-open" }));
        rows.push(field("JAVA_HOME", textInput("javaHome", db.javaHome, { label: "JAVA_HOME", placeholder: "/path/to/jdk" }), { wide: true, icon: "folder-open" }));
      }
    }
    const flag = db.passwordRef ? "passwordRef" : (parsed.password ? "明文待加密" : "未设置");
    return group({ title: "凭据", icon: "key-round", delay: 90, flag }, '<div class="grid">' + rows.join("") + '</div>');
  }

  function renderSecurityGroup(db, type) {
    const readonly = db.readonly !== false;
    const body = '<div class="grid">' +
      field("只读模式", '<div class="switch-row"><span class="t"><b' + (readonly ? "" : ' class="risk"') + '>' + (readonly ? "拒绝写操作" : "允许写操作") + '</b></span>' +
        '<button class="switch" role="switch" data-switch="readonly" aria-checked="' + readonly + '" aria-label="只读模式"></button></div>', { wide: true, icon: "shield-check" }) +
      field("命令黑名单", chipsField(db.blacklist), { wide: true, icon: "list-tree" }) +
      '</div>' + suggestRow(type);
    return group({ title: "安全", key: "safety", icon: readonly ? "shield-check" : "shield-alert", delay: 140 }, body);
  }

  function renderSshGroup(db) {
    const tunnel = db.sshTunnel || null;
    const mode = !tunnel ? "password" : (tunnel.privateKeyPath ? "path" : (tunnel.privateKey ? "inline" : "password"));
    const secretField = (label, bind, value, reference, placeholder) => {
      const options = { ...secretOptions(bind), reference, plain: value || "", label, placeholder };
      return '<div class="pw-wrap" data-secret-field="' + esc(bind) + '">' + secretControl(bind, options) + '</div>';
    };
    const body = !tunnel ? "" : '<div class="grid">' +
      field("跳板机", textInput("sshTunnel.host", tunnel.host, { label: "SSH 跳板机地址", placeholder: "jump.example.com" }), { icon: "server" }) +
      field("端口", textInput("sshTunnel.port", tunnel.port, { kind: "number", label: "SSH 端口", placeholder: "22" }), { icon: "hash" }) +
      field("用户名", textInput("sshTunnel.username", tunnel.username, { label: "SSH 用户名", placeholder: "deploy" }), { icon: "type" }) +
      field("超时", textInput("sshTunnel.readyTimeout", tunnel.readyTimeout, { kind: "number", label: "SSH 连接超时毫秒", placeholder: "10000" }), { icon: "hash", optional: true }) +
      field("认证方式", '<div class="seg" data-seg="sshMode">' + SSH_MODES.map((item) =>
        '<button data-value="' + item.id + '" data-focus-key="ssh-' + item.id + '" aria-pressed="' + (item.id === mode) + '"><span>' + item.label + '</span></button>').join("") + '</div>', { wide: true, icon: "git-branch" }) +
      (mode === "password" ? field("SSH 密码", secretField("SSH 密码", "sshTunnel.password", tunnel.password, tunnel.passwordRef, "SSH 密码"), { wide: true, icon: "key-round" }) : "") +
      (mode === "path" ? field("私钥路径", textInput("sshTunnel.privateKeyPath", tunnel.privateKeyPath, { label: "私钥路径", placeholder: "~/.ssh/id_rsa" }), { wide: true, icon: "folder-open" }) : "") +
      (mode === "inline" ? field("私钥内容", '<textarea class="inp" data-bind="sshTunnel.privateKey" data-kind="text" spellcheck="false" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" aria-label="私钥内容">' + esc(tunnel.privateKey || "") + '</textarea>', { wide: true, icon: "file-json" }) : "") +
      (mode !== "password" ? field("私钥口令", secretField("私钥口令", "sshTunnel.passphrase", tunnel.passphrase, tunnel.passphraseRef, "私钥口令"), { wide: true, icon: "key-round", optional: true }) : "") +
      '</div>';
    return group({ title: "SSH 隧道", icon: "git-branch", delay: 190, off: !tunnel, flag: tunnel ? "已启用" : "关闭",
      toggle: { key: "tunnel", on: Boolean(tunnel), label: "启用 SSH 隧道" } }, body);
  }

  function renderRedisClusterGroup(db, type) {
    if (type !== "redis") return "";
    const cluster = db.redisCluster || null;
    const stored = cluster ? (cluster.nodes || []) : [];
    // 末尾固定留一个空行作为新增入口
    const nodes = cluster ? stored.concat([""]) : [];
    const body = !cluster ? "" : '<div class="grid one">' +
      field("节点", '<div class="rows">' + nodes.map((node, index) =>
        '<div class="rrow"><input class="inp" data-bind="redisCluster.nodes.' + index + '" data-rows-index="' + index + '" value="' + esc(node) + '"' +
        ' placeholder="redis://10.0.0.1' + (index ? (index + 1) : "") + ':7001" spellcheck="false" aria-label="集群节点 ' + (index + 1) + '">' +
        (nodes.length > 1 ? '<button class="icon-btn danger" type="button" data-act="rm-node" data-index="' + index + '" title="移除">' + icon("x", "i-s") + '</button>' : "") +
        '</div>').join("") + '</div>', { wide: true, icon: "list-tree" }) +
      '</div>';
    return group({ title: "Redis 集群", icon: "server", delay: 240, off: !cluster,
      flag: cluster ? stored.length + " 个节点" : "关闭", toggle: { key: "cluster", on: Boolean(cluster), label: "启用 Redis 集群" } }, body);
  }

  // 只有存在未加密的明文密码时才提示迁移行为，避免常驻说明文字
  function migrationStrip(parsed) {
    if (!parsed.password) return "";
    return '<div class="strip ok" style="margin-top:26px">' + icon("circle-check", "i-s") +
      '<span>明文密码已就绪，保存后 CLI 会在下一次连接时写入 secrets.json</span></div>';
  }

  // 推荐项由后端随配置一起下发（README 的常见高危命令），页面不再自己维护清单
  function suggestRow(type) {
    const key = type === "redis" ? "redis" : (type === "mongodb" ? "mongodb" : "sql");
    const used = current().blacklist || [];
    const items = (S.recommendedBlacklist[key] || []).filter((item) => used.indexOf(item) < 0);
    if (!items.length) return "";
    return '<div class="suggest"><span class="suggest-lab">推荐</span>' + items.map((item) =>
      '<button type="button" data-act="add-chip" data-value="' + esc(item) + '">' + esc(item) + '</button>').join("") + '</div>';
  }

  /* ==========================================================
     渲染：JSON 打印纸
     ========================================================== */
  function highlight(text) {
    const pattern = /("(?:\\.|[^"\\])*")(\s*:)?|(\b-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b)|\b(true|false|null)\b|([{}[\],])/g;
    let out = "";
    let last = 0;
    let match;
    while ((match = pattern.exec(text)) !== null) {
      out += esc(text.slice(last, match.index));
      if (match[1]) {
        out += match[2]
          ? '<span class="tok-k">' + esc(match[1]) + '</span><span class="tok-p">' + esc(match[2]) + '</span>'
          : '<span class="tok-s">' + esc(match[1]) + '</span>';
      } else if (match[3]) {
        out += '<span class="tok-n">' + esc(match[3]) + '</span>';
      } else if (match[4]) {
        out += '<span class="tok-b">' + esc(match[4]) + '</span>';
      } else {
        out += '<span class="tok-p">' + esc(match[5]) + '</span>';
      }
      last = match.index + match[0].length;
    }
    out += esc(text.slice(last));
    return out;
  }

  // 打印纸只读表单状态，除非用户正在直接编辑文本，此时以文本为准
  function renderPrint() {
    const editing = document.activeElement === rawInput;
    const text = editing ? rawInput.value : serializeScope();
    if (!editing && rawInput.value !== text) rawInput.value = text;
    const lines = text.split("\n");
    const marks = diffLines(baselineLines(), lines);
    // 每行都是块级元素，不能用换行符连接，否则高亮层会出现双倍行距
    const html = lines.map((line, index) =>
      '<span class="line' + (marks[index] ? " m-" + marks[index] : "") + '">' + (highlight(line) || " ") + '</span>'
    ).join("");
    $("#hlLayer").innerHTML = html;
    $("#printSize").textContent = new Blob([text]).size + " B";
    renderScope();
  }

  // 面板范围的唯一真相是 S.scope：标题和按钮选中态都从这里派生
  function renderScope() {
    const single = S.scope === "one";
    $("#printTitle").textContent = single && S.selected
      ? S.selected
      : (S.path.split("/").pop() || "config.json");
    $$('[data-seg="scope"] button').forEach((button) => {
      button.setAttribute("aria-pressed", String(button.dataset.value === S.scope));
    });
  }

  let commitStateHtml = "";
  let saveButtonHtml = "";

  function renderCommit() {
    const dirty = isDirty();
    const element = $("#commitState");
    const names = Object.keys(databases());
    element.className = dirty ? "commit-state dirty" : "commit-state";
    const state = dirty
      ? '<span class="dot-dirty"></span><b>未保存</b><span>·</span><span>' + names.length + ' 个连接</span>'
      : icon("circle-check", "i-s") + '<b>已同步</b><span>·</span><span>' + (S.backup ? "备份 " + esc(S.backup.split("/").pop()) : esc(S.path.split("/").pop() || "config.json")) + '</span>';
    // 每次按键都重建一次 innerHTML 会打断过渡动画，内容没变就不动 DOM
    if (commitStateHtml !== state) {
      commitStateHtml = state;
      element.innerHTML = state;
    }
    const button = $("#saveBtn");
    button.disabled = !dirty || S.busy;
    button.classList.toggle("dirty", dirty);
    const label = dirty ? '<span class="dot-dirty-btn"></span><span>保存</span>' : icon("save", "i-s") + "<span>保存</span>";
    if (saveButtonHtml !== label) {
      saveButtonHtml = label;
      if (!S.busy) button.innerHTML = label;
    }
    $("#discardBtn").disabled = !dirty;
  }

  function renderAll() {
    $("#topPath").innerHTML = "<b>" + esc(S.path) + "</b>";
    renderRail();
    renderBoard();
    renderPrint();
    renderCommit();
  }

  // 结构性改动后的统一收尾：轨道、JSON、提交条各画一次
  function renderDerived() {
    renderRail();
    renderPrint();
    renderCommit();
  }

  // 输入类改动还要刷新线路图，内容没变时不会再动 DOM
  function refreshDerived() {
    renderWire();
    renderDerived();
  }

