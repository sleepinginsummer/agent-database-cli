  /* ==========================================================
     校验
     ========================================================== */
  // 配置合法性以保存接口的 validate_config 为唯一来源，页面只保留交互层校验：
  // 连接名不能为空（它是 databases 的 key，服务端校验覆盖不到），JSON 语法错误单独拦截。
  function interactionIssues() {
    const name = $("#nameInput") ? $("#nameInput").value.trim() : S.selected;
    return name ? [] : ["连接名称不能为空"];
  }

  // 把校验结果贴回页面：轨道行标记 + 当前连接的提示条 + 一次提示
  function flashIssues(messages) {
    S.issues = messages;
    renderRail();
    renderBoard();
    toast(messages[0], "err");
  }

  /* ==========================================================
     动作
     ========================================================== */
  function uniqueName(base) {
    const names = Object.keys(databases());
    if (names.indexOf(base) < 0) return base;
    let index = 2;
    while (names.indexOf(base + "-" + index) >= 0) index += 1;
    return base + "-" + index;
  }

  function addConnection() {
    const name = uniqueName("new-mysql");
    databases()[name] = { type: "mysql", url: "", readonly: true };
    setSelected(name);
    renderAll();
    const input = $("#nameInput");
    if (input) { input.focus(); input.select(); }
  }

  function duplicateConnection() {
    const source = clone(current());
    if (source.passwordRef) delete source.passwordRef;
    const name = uniqueName(S.selected + "-copy");
    databases()[name] = source;
    setSelected(name);
    renderAll();
    toast("已复制为 " + name);
  }

  function removeConnection() {
    const name = S.selected;
    if (!window.confirm("删除连接 " + name + "？")) return;
    delete databases()[name];
    setSelected(Object.keys(databases())[0] || null);
    renderAll();
    toast("已删除 " + name);
  }

  // 纯数据操作：只重排 key 并更新当前选中项，不碰 DOM、不渲染、不提示
  // 返回 null 表示成功，返回 { code, message } 表示失败，由调用方决定怎么反馈
  function renameDatabaseKey(from, to) {
    const name = String(to || "").trim();
    if (!name) return { code: "empty", message: "连接名称不能为空" };
    if (name === from) return null;
    if (databases()[name]) return { code: "duplicate", message: "连接名已存在: " + name };
    const rebuilt = {};
    for (const [key, value] of Object.entries(databases())) {
      rebuilt[key === from ? name : key] = value;
    }
    S.data.databases = rebuilt;
    setSelected(name);
    return null;
  }

  let testingConnection = false;

  async function testConnection() {
    if (testingConnection || !current()) return;
    if (S.jsonError) { toast("JSON 无法解析，请先修正右侧文本", "err"); return; }
    const issues = interactionIssues();
    if (issues.length) { toast(issues[0], "err"); return; }
    const name = S.selected;
    const body = JSON.stringify({ name, database: cleanDatabase(current()) });
    testingConnection = true;
    const button = $('[data-act="test-connection"]');
    button.disabled = true;
    button.textContent = "测试中…";
    try {
      await api("/api/test-connection", { method: "POST", body });
      toast(name + "：连接成功");
    } catch (error) {
      toast(name + "：" + error.message, "err");
    } finally {
      testingConnection = false;
      const activeButton = $('[data-act="test-connection"]');
      if (activeButton) {
        activeButton.disabled = false;
        activeButton.textContent = "测试连接";
      }
    }
  }
  async function save() {
    if (S.busy) return;
    // 右侧文本解析失败时不能只写表单里的旧数据，先让用户修好文本
    if (S.jsonError) { toast("JSON 无法解析，请先修正右侧文本", "err"); return; }
    const issues = interactionIssues();
    if (issues.length) { flashIssues(issues); return; }
    S.busy = true;
    renderCommit();
    const button = $("#saveBtn");
    button.innerHTML = icon("loader-circle", "i-s spin") + "<span>写入</span>";
    const raw = serialize();
    try {
      const result = await api("/api/config", { method: "POST", body: JSON.stringify({ raw }) });
      S.baselineData = clone(S.data);
      S.backup = result.backup || null;
      S.issues = [];
      $("#rawInput").value = raw;
      S.textDirty = false;
      renderPrint();
      button.classList.add("done");
      button.innerHTML = icon("check", "i-s") + "<span>已保存</span>";
      $("#commit").classList.add("saving");
      setTimeout(() => { $("#commit").classList.remove("saving"); button.classList.remove("done"); }, 900);
      toast("已写入 " + result.bytes + " B" + (result.backup ? "，备份 " + result.backup.split("/").pop() : ""));
      renderAll();
    } catch (error) {
      // 保存失败的规则来自服务端 validate_config，按原文贴回页面即可
      toast(error.message, "err");
      S.issues = [error.message];
      renderRail();
      renderBoard();
    } finally {
      S.busy = false;
      setTimeout(() => {
        button.innerHTML = icon("save", "i-s") + "<span>保存</span>";
        renderCommit();
      }, 900);
    }
  }

  async function load(text, keepSelection) {
    const payload = await api("/api/config");
    S.path = payload.path;
    S.data = payload.data && payload.data.databases ? payload.data : { databases: {} };
    S.issues = payload.issues || [];
    S.recommendedBlacklist = payload.recommendedBlacklist || {};
    S.baselineData = clone(S.data);
    if (!keepSelection || !databases()[S.selected]) setSelected(Object.keys(databases())[0] || null);
    $("#rawInput").value = serializeScope();
    S.textDirty = false;
    renderAll();
    renderPrint();
    if (text) toast(text);
  }

  async function quit() {
    if (isDirty() && !window.confirm("有未保存的改动，仍要关闭编辑服务？")) return;
    try { await api("/api/shutdown", { method: "POST", body: "{}" }); } catch (error) { /* 服务已关闭 */ }
    document.body.innerHTML = '<div class="closed">' + icon("power", "i-l") +
      '<p>编辑服务已关闭<br>重新启动：agent-database-cli editor start</p></div>';
  }

  function toast(message, kind) {
    const element = document.createElement("div");
    element.className = "toast" + (kind === "err" ? " err" : "");
    element.innerHTML = icon(kind === "err" ? "circle-alert" : "circle-check", "i-s") + "<span>" + esc(message) + "</span>";
    $("#toasts").appendChild(element);
    setTimeout(() => {
      element.classList.add("out");
      setTimeout(() => element.remove(), 300);
    }, 3200);
  }

  /* ==========================================================
     事件
     ========================================================== */
  const board = $("#board");
  const rawInput = $("#rawInput");
  $("#railSearch").addEventListener("input", (event) => {
    S.search = event.target.value;
    renderRail();
  });

  function bindValue(element) {
    const kind = element.dataset.kind || "text";
    const value = element.value;
    if (kind === "number") {
      if (value.trim() === "") return undefined;
      const parsed = Number(value);
      return Number.isFinite(parsed) ? parsed : undefined;
    }
    if (kind === "secret") return value === "" ? undefined : value;
    return value;
  }

  board.addEventListener("input", (event) => {
    const element = event.target;
    if (element.id === "nameInput") {
      const failure = renameDatabaseKey(S.selected, element.value);
      element.classList.toggle("bad", Boolean(failure));
      if (!failure) {
        renderRail();
        renderPrint();
        renderCommit();
      } else if (failure.code === "duplicate") {
        toast(failure.message, "err");
      }
      return;
    }
    if (element.id === "pwInput") {
      const db = current();
      db.url = composeUrl(parseUrl(db.url).ok ? parseUrl(db.url).view : db.url, element.value);
      refreshDerived();
      return;
    }
    if (element.id === "chipInput" || element.id === "tagInput") return;
    if (!element.dataset.bind) return;
    const bind = element.dataset.bind;
    const value = bindValue(element);
    if (bind === "url") {
      const parsed = parseUrl(value);
      // 直接粘贴带密码的 URL 时，把密码收进独立字段，URL 里不再留明文
      const password = parsed.ok ? parsed.password : parseUrl(current().url).password;
      current().url = parsed.ok ? composeUrl(parsed.view, password) : value;
      // 只重画密码控件本身，让状态 chip 和遮罩立刻跟上，不重建整张表单
      if (parsed.ok) refreshSecretField("url");
      refreshDerived();
      return;
    }
    // Redis 集群节点：最后一行继续输入时自动补一行空行
    if (element.dataset.rowsIndex !== undefined) {
      const index = Number(element.dataset.rowsIndex);
      const nodes = current().redisCluster.nodes;
      nodes[index] = value;
      if (index === nodes.length - 1 && value.trim() !== "") {
        current().redisCluster.nodes = nodes.concat([""]);
        renderBoard();
        const next = $('[data-rows-index="' + (index + 1) + '"]');
        if (next) next.focus();
        renderDerived();
        return;
      }
      current().redisCluster.nodes = nodes;
      refreshDerived();
      return;
    }
    if (value === undefined) setPath(current(), bind, undefined);
    else setPath(current(), bind, value);
    refreshDerived();
  });

  board.addEventListener("change", (event) => {
    const element = event.target;
    if (element.id === "chipInput") {
      element.value.split(/[\s,，]+/).filter(Boolean).forEach(addChip);
      element.value = "";
      renderBoard();
      renderDerived();
      const input = $("#chipInput");
      if (input) input.focus();
      return;
    }
    if (!element.dataset.bind) return;
    const bind = element.dataset.bind;
    const value = bindValue(element);
    if (element.dataset.rowsIndex !== undefined) return;
    if (bind === "url") {
      // 失焦时把 URL 里的明文密码收回到密码字段，URL 本身保持不含密码
      const parsed = parseUrl(current().url);
      if (parsed.ok && element.value !== parsed.view) element.value = parsed.view;
      return;
    }
    if (value === undefined) setPath(current(), bind, undefined);
    else setPath(current(), bind, value);
    refreshDerived();
  });

  function commitTagInput() {
    const input = $("#tagInput");
    const value = input.value.trim();
    if (!value) return;
    current().tags = [...new Set([...databaseTags(current()), value])];
    input.value = "";
    renderBoard();
    renderDerived();
    $("#tagInput").focus();
  }

  function addChip(value) {
    const db = current();
    const list = db.blacklist || [];
    const item = String(value).trim();
    if (!item || list.indexOf(item) >= 0) return;
    db.blacklist = list.concat([item]);
  }


  // 选中项变更的唯一入口：切到别的连接就收起已显示的明文，
  // 避免上一个连接的显隐状态被下一个连接继承
  function setSelected(name) {
    if (name !== S.selected && Object.keys(S.shown).length) S.shown = {};
    S.selected = name;
  }

  // 切换连接：轨道标记 + 表单 + JSON 面板 + 提交条，不重写顶栏
  function selectConnection(name) {
    setSelected(name);
    // 只挪动高亮那一行，不重建整个列表
    $$("#railList .row").forEach((row) => row.classList.toggle("on", row.dataset.name === name));
    renderBoard();
    renderPrint();
    renderCommit();
  }

  function focusByKey(key) {
    const target = $('[data-focus-key="' + key + '"]');
    if (target) target.focus();
  }

  // 分段控件按动作收窄渲染范围：只有改变表单结构的切换才重建表单
  function applySegment(key, value) {
    const db = current();
    if (key === "listMode") {
      S.listMode = value;
      $$('[data-seg="listMode"] button').forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.value === value)));
      renderRail();
      return;
    }
    if (key === "scope") {
      // 面板范围不影响表单和轨道，重画 JSON 面板即可
      S.scope = value === "all" ? "all" : "one";
      renderPrint();
      focusByKey("scope-" + value);
      return;
    }
    if (key === "type") {
      db.type = value;
      if (value !== "oracle") delete db.oracleDriver;
      if (value !== "redis") delete db.redisCluster;
    }
    if (key === "oracleDriver") db.oracleDriver = value;
    if (key === "sshMode") {
      const tunnel = db.sshTunnel || {};
      // 三种认证方式互斥，切换时清理另一类字段，保证写出的内容一定合法
      if (value === "password") { delete tunnel.privateKey; delete tunnel.privateKeyPath; delete tunnel.passphrase; delete tunnel.passphraseRef; }
      if (value === "path") { delete tunnel.privateKey; delete tunnel.password; delete tunnel.passwordRef; }
      if (value === "inline") { delete tunnel.privateKeyPath; delete tunnel.password; delete tunnel.passwordRef; }
      db.sshTunnel = tunnel;
    }
    renderBoard();
    renderDerived();
    const focusKey = key === "type" ? "type-" + value : (key === "oracleDriver" ? "driver-" + value : "ssh-" + value);
    focusByKey(focusKey);
  }

  function applySwitch(key, on) {
    const db = current();
    if (key === "readonly") {
      // 开关表示“只读生效”，点击后取反；显式写 true/false，不靠删字段表达默认值
      const readonly = !on;
      db.readonly = readonly;
      const button = $('[data-group="safety"] [data-switch="readonly"]');
      if (button) {
        button.setAttribute("aria-checked", String(readonly));
        const label = button.closest(".switch-row").querySelector(".t b");
        if (label) {
          label.textContent = readonly ? "拒绝写操作" : "允许写操作";
          label.classList.toggle("risk", !readonly);
        }
      }
      const use = $('[data-group="safety"] .group-title use');
      if (use) use.setAttribute("href", readonly ? "#i-shield-check" : "#i-shield-alert");
      renderDerived();
      return;
    }
    if (key === "tunnel") db.sshTunnel = on ? undefined : { host: "", port: 22, username: "", password: "" };
    if (key === "cluster") db.redisCluster = on ? undefined : { nodes: [""] };
    renderBoard();
    renderDerived();
  }

  const ACTIONS = {
    add: () => addConnection(),
    duplicate: () => duplicateConnection(),
    "test-connection": () => testConnection(),
    remove: () => removeConnection(),
    "add-chip": (target) => { addChip(target.dataset.value); renderBoard(); renderDerived(); },
    "rm-tag": (target) => {
      // 用展示数组索引定位，避免 HTML 属性解析转换标签中的 CR/CRLF。
      current().tags = databaseTags(current()).filter((_, index) => index !== Number(target.dataset.index));
      renderBoard();
      renderDerived();
      $("#tagInput").focus();
    },
    "rm-chip": (target) => {
      const db = current();
      db.blacklist = (db.blacklist || []).filter((item) => item !== target.dataset.value);
      renderBoard();
      renderDerived();
    },
    "rm-node": (target) => {
      const db = current();
      if (!db.redisCluster) return;
      const index = Number(target.dataset.index);
      const nodes = (db.redisCluster.nodes || []).filter((_, position) => position !== index);
      db.redisCluster.nodes = nodes.length ? nodes : [""];
      renderBoard();
      renderDerived();
    },
    // toggle-password 只切该字段的显隐；toggle-secret 才会按需向后端要一次解密结果，
    // 两者都只重画这一个字段，不触碰表单其它节点
    "toggle-password": (target) => setSecretVisibility(target.dataset.field, !isShown(target.dataset.field)),
    "toggle-secret": async (target) => {
      const field = target.dataset.field;
      const reference = target.dataset.ref;
      if (isShown(field)) { setSecretVisibility(field, false); return; }
      if (reference && !hasCachedSecret(reference)) {
        const value = await fetchSecret(reference);
        if (value === null) return;
        S.secrets[reference] = value;
      }
      setSecretVisibility(field, true);
    },
    "clear-password": () => {
      const db = current();
      const parsed = parseUrl(db.url);
      if (parsed.ok) db.url = composeUrl(parsed.view, "");
      delete db.passwordRef;
      renderBoard();
      renderDerived();
      toast("已移除保存的凭据");
    }
  };


  board.addEventListener("keydown", (event) => {
    if (event.target.id === "tagInput") {
      if ((event.key === "Enter" || event.key === ",") && !event.isComposing) {
        event.preventDefault();
        commitTagInput();
      }
      return;
    }
    if (event.target.id !== "chipInput") return;
    if (event.key === "Enter" || event.key === ",") {
      event.preventDefault();
      if (event.target.value.trim()) {
        addChip(event.target.value);
        event.target.value = "";
        renderBoard();
        renderDerived();
        const input = $("#chipInput");
        if (input) input.focus();
      }
    }
    if (event.key === "Backspace" && event.target.value === "") {
      const db = current();
      if ((db.blacklist || []).length) {
        db.blacklist = db.blacklist.slice(0, -1);
        renderBoard();
        renderDerived();
        const input = $("#chipInput");
        if (input) input.focus();
      }
    }
  });

  // 点击统一在文档层分派：轨道、画布、顶栏共用同一份动作实现
  document.addEventListener("click", (event) => {
    const target = event.target.closest("[data-act],[data-seg] button,[data-switch]") || event.target.closest(".row");
    if (!target) return;
    const row = target.closest(".row");
    if (row) { selectConnection(row.dataset.name); return; }
    const seg = target.closest("[data-seg]");
    if (seg) { applySegment(seg.dataset.seg, target.dataset.value); return; }
    if (target.dataset.switch) { applySwitch(target.dataset.switch, target.getAttribute("aria-checked") === "true"); return; }
    const action = target.dataset.act;
    if (ACTIONS[action]) { ACTIONS[action](target); return; }
    if (action === "save") save();
    if (action === "discard") {
      if (!isDirty() || window.confirm("丢弃未保存的改动？")) load("已重新载入磁盘内容", true);
    }
    if (action === "reload") {
      if (!isDirty() || window.confirm("丢弃未保存的改动并重新载入？")) load("已重新载入磁盘内容", true);
    }
    if (action === "toggle-json") {
      S.jsonOpen = !S.jsonOpen;
      document.body.classList.toggle("json-open", S.jsonOpen);
      $$('[data-act="toggle-json"]').forEach((element) => element.setAttribute("aria-pressed", String(S.jsonOpen)));
      if (S.jsonOpen) renderPrint();
    }
    if (action === "copy") {
      navigator.clipboard.writeText($("#rawInput").value).then(() => toast("已复制 JSON"));
    }
    if (action === "quit") quit();
  });

  // 高亮层在文本层下面，滚动必须跟随文本层
  rawInput.addEventListener("scroll", () => {
    const layer = $("#hlLayer");
    layer.scrollTop = rawInput.scrollTop;
    layer.scrollLeft = rawInput.scrollLeft;
  });

  rawInput.addEventListener("input", () => {
    const text = rawInput.value;
    S.textDirty = true;
    renderPrint();
    let failure = null;
    try {
      const parsed = JSON.parse(text);
      if (S.scope === "one") failure = applyScopedJson(parsed);
      else if (parsed && typeof parsed === "object" && parsed.databases) S.data = parsed;
      else failure = "缺少 databases 对象";
    } catch (error) {
      failure = String(error.message || error);
    }
    if (failure) {
      showJsonError(failure);
      return;
    }
    S.jsonError = null;
    $("#jsonError").hidden = true;
    // 文本解析成功后表单要跟着走，否则左侧还停留在旧值
    renderRail();
    renderBoard();
    renderCommit();
    // 面板标题跟着键名走，但不重写文本，避免打断正在输入的内容
    renderScope();
  });

  rawInput.addEventListener("blur", () => {
    if (!S.textDirty) return;
    S.textDirty = false;
    // 无效文本（语法错误或结构不合法）在失焦时回填最后一次有效内容，并明确告知，
    // 避免表单和文本长期不一致，也避免带着一份没生效的文本去保存
    if (!S.jsonError) return;
    S.jsonError = null;
    $("#jsonError").hidden = true;
    renderPrint();
    toast("JSON 未生效，已恢复为当前配置", "err");
  });

  function showJsonError(message) {
    const element = $("#jsonError");
    element.hidden = false;
    S.jsonError = message;
    element.lastElementChild.textContent = message;
  }

  // 单个连接视图只接受一条连接：键名变化按重命名处理，保序写回 databases
  function applyScopedJson(parsed) {
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return "需要一个连接对象";
    const names = Object.keys(parsed);
    if (names.length !== 1) return "当前连接视图只能包含一个连接";
    const name = names[0];
    const value = parsed[name];
    if (!value || typeof value !== "object" || Array.isArray(value)) return "连接配置必须是对象";
    // 键名变化按重命名处理：只动数据，渲染由 input handler 统一做一次
    const failure = renameDatabaseKey(S.selected, name);
    if (failure) return failure.message;
    S.data.databases[S.selected] = value;
    return null;
  }

  document.addEventListener("keydown", (event) => {
    const meta = event.metaKey || event.ctrlKey;
    if (meta && event.key.toLowerCase() === "s") { event.preventDefault(); save(); return; }
    if (meta && event.key.toLowerCase() === "j") {
      event.preventDefault();
      const button = $('.top-tools [data-act="toggle-json"]');
      if (button) button.click();
      return;
    }
    if (event.target.closest("input,textarea")) return;
    const names = Object.keys(databases());
    if (!names.length) return;
    const index = names.indexOf(S.selected);
    if (event.key === "ArrowDown" || event.key === "j") {
      event.preventDefault();
      selectConnection(names[Math.min(index + 1, names.length - 1)]);
    }
    if (event.key === "ArrowUp" || event.key === "k") {
      event.preventDefault();
      selectConnection(names[Math.max(index - 1, 0)]);
    }
    if (event.key === "n") { event.preventDefault(); addConnection(); }
  });

  window.addEventListener("beforeunload", (event) => {
    if (!isDirty()) return;
    event.preventDefault();
    event.returnValue = "";
  });

  /* 指针光晕 */
  let frame = 0;
  window.addEventListener("pointermove", (event) => {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      document.documentElement.style.setProperty("--mx", event.clientX + "px");
      document.documentElement.style.setProperty("--my", event.clientY + "px");
    });
  });

