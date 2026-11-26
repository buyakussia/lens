// Lens — интерфейс панели. Без сборщика: Tauri отдаёт файлы как есть, API берём из window.__TAURI__.
(() => {
  "use strict";
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = (id) => document.getElementById(id);
  const IS_MAC = /Mac/i.test(navigator.platform);

  // ---------------------------------------------------------------- состояние
  const st = {
    cfg: null,
    chat: null,          // {id, created, updated, title, session, messages[]}
    atts: [],            // вложения: {kind, path?, text?, label}
    busy: false,
    streamText: "",
    streamEl: null,
    renderTimer: 0,
    lastAnswer: "",
    view: "chat",        // chat | history | settings
    histChats: [],
    recording: null,     // имя горячей клавиши, которую сейчас записываем
    claudePath: null,
  };

  const BRIEF = "[Длина ответа: 1–4 предложения или короткий список до 5 пунктов. Без вступлений, повторов вопроса и лишних пояснений. Подробнее — только если пользователь прямо попросит.]";
  const KIND_NAMES = { image: "картинка/скриншот", pdf: "PDF", file: "текстовый файл" };
  const ICONS = { pdf: "📕", file: "📄", quote: "❝", text: "📋" };
  const TOOL_STATUS = { WebSearch: "Ищет в интернете…", WebFetch: "Читает страницу…", Read: "Смотрит на вложение…" };

  // ---------------------------------------------------------------- вспомогательное
  const esc = (s) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const newId = () => new Date().toISOString().replace(/[-:T.Z]/g, "").slice(0, 14) + "-" + Math.random().toString(16).slice(2, 8);
  const plural = (n, one, few, many) => (n % 10 === 1 && n % 100 !== 11 ? one : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 12 || n % 100 > 14) ? few : many);
  const ago = (ts) => {
    const d = new Date(ts * 1000), now = new Date(), hm = d.toLocaleTimeString("ru", { hour: "2-digit", minute: "2-digit" });
    if (d.toDateString() === now.toDateString()) return "Сегодня, " + hm;
    if (new Date(now - 864e5).toDateString() === d.toDateString()) return "Вчера, " + hm;
    return d.toLocaleDateString("ru", { day: "numeric", month: "short", year: d.getFullYear() !== now.getFullYear() ? "numeric" : undefined }) + ", " + hm;
  };
  const attLabel = (a) => a.label || (a.kind === "quote" ? "Цитата" : "Вложение");
  const textLabel = (text, kind) => {
    const first = text.split(/\s+/).join(" ");
    if (kind === "quote") return "«" + first.slice(0, 40) + (first.length > 40 ? "…" : "") + "»";
    const n = text.split("\n").length;
    return `Текст: ${n} ${plural(n, "строка", "строки", "строк")}`;
  };
  const toast = (text, ms = 4000) => setStatus(text, false, ms);

  let fitRaf = 0;
  function fit() {
    cancelAnimationFrame(fitRaf);
    fitRaf = requestAnimationFrame(() => invoke("fit_panel", { height: Math.ceil($("panel").getBoundingClientRect().height) + 2 }).catch(() => {}));
  }
  new ResizeObserver(fit).observe($("panel"));

  // ---------------------------------------------------------------- Markdown
  marked.setOptions({ gfm: true, breaks: false });
  function renderMd(el, text) {
    const html = DOMPurify.sanitize(marked.parse(text || ""), { USE_PROFILES: { html: true } });
    el.innerHTML = html;
    el.querySelectorAll("pre > code").forEach((code) => {
      const pre = code.parentElement;
      const lang = ((code.className.match(/language-([\w+#-]+)/) || [])[1] || "").toLowerCase();
      try { lang && hljs.getLanguage(lang) ? hljs.highlightElement(code) : hljs.highlightElement(code); } catch (_) {}
      const wrap = document.createElement("div");
      wrap.className = "codeblock";
      wrap.innerHTML = `<div class="codehead"><span>${esc(lang || "код")}</span><button class="copybtn">Копировать</button></div>`;
      pre.replaceWith(wrap);
      wrap.appendChild(pre);
      const btn = wrap.querySelector("button");
      btn.onclick = () => copy(code.textContent, btn);
    });
    el.querySelectorAll("a[href]").forEach((a) => { a.dataset.url = a.getAttribute("href"); a.removeAttribute("href"); a.style.cursor = "pointer"; });
  }
  document.addEventListener("click", (e) => {
    const a = e.target.closest("a[data-url]");
    if (a) { e.preventDefault(); invoke("open_url", { url: a.dataset.url }).catch((err) => toast(String(err))); }
  });

  async function copy(text, btn) {
    try { await invoke("copy_text", { text }); } catch (e) { return toast("Не удалось скопировать: " + e); }
    if (btn) { const old = btn.textContent; btn.textContent = "Скопировано ✓"; setTimeout(() => (btn.textContent = old), 1500); }
  }

  // ---------------------------------------------------------------- переписка
  function scrollDown() { const l = $("log"); l.scrollTop = l.scrollHeight; }

  async function thumb(path) {
    try { return await invoke("file_data_url", { path }); } catch (_) { return null; }
  }

  async function addUserMsg(text, atts) {
    const box = document.createElement("div");
    box.className = "msg-user";
    const imgs = atts.filter((a) => a.kind === "image");
    if (imgs.length) {
      const row = document.createElement("div");
      row.className = "thumbs";
      for (const a of imgs.slice(0, 4)) {
        const url = await thumb(a.path);
        if (url) { const im = new Image(); im.src = url; row.appendChild(im); }
      }
      box.appendChild(row);
    }
    if (text) { const t = document.createElement("div"); t.textContent = text; box.appendChild(t); }
    atts.filter((a) => a.kind !== "image").forEach((a) => {
      const d = document.createElement("div");
      d.className = "att";
      d.textContent = a.kind === "quote" || a.kind === "text" ? "❝ " + a.text.split(/\s+/).join(" ").slice(0, 90) : "📄 " + attLabel(a);
      box.appendChild(d);
    });
    $("log").appendChild(box);
    scrollDown();
  }

  function addAnswerBox() {
    const box = document.createElement("div");
    const body = document.createElement("div");
    body.className = "answer";
    box.appendChild(body);
    $("log").appendChild(box);
    scrollDown();
    return { box, body };
  }

  function finalizeAnswer(entry, text) {
    renderMd(entry.body, text);
    const btn = document.createElement("button");
    btn.className = "copybtn answer-copy";
    btn.textContent = "Копировать";
    btn.onclick = () => copy(text, btn);
    entry.box.appendChild(btn);
  }

  // ---------------------------------------------------------------- вложения
  function renderChips() {
    const c = $("chips");
    c.innerHTML = "";
    c.hidden = st.atts.length === 0;
    st.atts.forEach(async (a) => {
      const chip = document.createElement("div");
      chip.className = "chip";
      const head = document.createElement("span");
      if (a.kind === "image") {
        const url = await thumb(a.path);
        if (url) { const im = new Image(); im.src = url; head.replaceWith(im); chip.prepend(im); }
      } else { head.className = "ico"; head.textContent = ICONS[a.kind] || "📎"; chip.prepend(head); }
      const l = document.createElement("span");
      l.className = "lbl"; l.textContent = attLabel(a); l.title = a.path || (a.text || "").slice(0, 600);
      const x = document.createElement("button");
      x.className = "x"; x.textContent = "✕"; x.title = "Убрать";
      x.onclick = () => { st.atts = st.atts.filter((q) => q !== a); renderChips(); updateSend(); };
      chip.append(l, x);
      c.appendChild(chip);
    });
    updateSend();
  }
  function addAtt(a) { st.atts.push(a); renderChips(); }
  function addText(text, kind = "text") {
    text = text.replace(/^\n+|\n+$/g, "");
    if (!text.trim()) return;
    addAtt({ kind, text, label: textLabel(text, kind) });
  }
  async function addFilePath(path) {
    try { addAtt(await invoke("add_file", { path })); } catch (e) { toast(String(e)); }
  }

  // вставка из буфера: картинка → вложение, длинный текст → вложение, короткий — в поле
  $("input").addEventListener("paste", async (e) => {
    const items = [...(e.clipboardData?.items || [])];
    const img = items.find((i) => i.kind === "file" && i.type.startsWith("image/"));
    if (img) {
      e.preventDefault();
      const f = img.getAsFile();
      const b64 = await new Promise((res) => { const r = new FileReader(); r.onload = () => res(String(r.result).split(",")[1]); r.readAsDataURL(f); });
      try { addAtt({ kind: "image", path: await invoke("save_pasted_image", { b64 }), label: "Картинка из буфера" }); } catch (err) { toast(String(err)); }
      return;
    }
    const text = e.clipboardData?.getData("text/plain") || "";
    if (text.trim() && (text.trim().includes("\n") || text.length > 300)) { e.preventDefault(); addText(text); }
  });

  // перетаскивание файлов (события Tauri дают пути на диске)
  listen("tauri://drag-enter", () => $("panel").classList.add("dragover"));
  listen("tauri://drag-leave", () => $("panel").classList.remove("dragover"));
  listen("tauri://drag-drop", (ev) => { $("panel").classList.remove("dragover"); (ev.payload.paths || []).forEach(addFilePath); $("input").focus(); });

  // ---------------------------------------------------------------- промпт
  function buildPrompt(q, atts) {
    const parts = [];
    for (const a of atts) {
      if (a.kind === "quote") parts.push("Выделенный пользователем текст:\n<<<\n" + a.text + "\n>>>");
      else if (a.kind === "text") parts.push("Вставленный пользователем текст:\n<<<\n" + a.text + "\n>>>");
    }
    const files = atts.filter((a) => ["image", "pdf", "file"].includes(a.kind));
    if (files.length) parts.push("Прикреплённые файлы (открой их инструментом Read):\n" + files.map((a) => `- ${a.path} — ${KIND_NAMES[a.kind]}`).join("\n"));
    const kinds = new Set(atts.map((a) => a.kind));
    if (q) parts.push("Вопрос: " + q);
    else if (kinds.size === 1 && kinds.has("image")) parts.push("Скажи, что на изображении и что здесь главное. Если там текст на другом языке — переведи. В интернете ищи, только если без этого не ответить.");
    else if ([...kinds].every((k) => k === "quote" || k === "text")) parts.push("Объясни суть этого текста. Если он на другом языке — переведи. Если это код или ошибка — скажи причину и как исправить.");
    else parts.push("Скажи, что это и что в нём главное.");
    const sp = (st.cfg.system_prompt || "").trim();
    if (sp) parts.push("[Указания пользователя для всех ответов: " + sp + "]");
    parts.push(BRIEF);
    return parts.join("\n\n");
  }

  // ---------------------------------------------------------------- отправка
  function setBusy(v, text = "") {
    st.busy = v;
    $("logo").classList.toggle("busy", v);
    $("input").disabled = v;
    document.querySelectorAll(".action").forEach((b) => (b.disabled = v));
    setStatus(v ? text : "", v);
    updateSend();
  }
  let statusTimer = 0;
  function setStatus(text, keep = true, ms = 0) {
    clearTimeout(statusTimer);
    $("status").hidden = !text;
    $("status").textContent = text;
    if (text && ms) statusTimer = setTimeout(() => { if (!st.busy) $("status").hidden = true; }, ms);
  }
  function updateSend() {
    const b = $("btn-send");
    b.classList.toggle("stop", st.busy);
    b.title = st.busy ? "Остановить ответ" : "Отправить (Enter)";
    b.disabled = !st.busy && !($("input").value.trim() || st.atts.length);
    $("btn-new").hidden = !(st.chat && st.chat.messages.length) || st.busy;
    $("btn-copy-last").hidden = !st.lastAnswer;
  }

  async function send(action) {
    if (st.busy) return;
    let q = $("input").value.trim();
    let atts = st.atts.slice();
    let shown = q;
    if (action) {
      if (!atts.length && q) { atts = [{ kind: "text", text: q, label: textLabel(q, "text") }]; q = ""; }
      shown = "⚡ " + action.name + (q ? ": " + q : "");
      q = action.prompt + (q ? "\n\nДополнительно от пользователя: " + q : "");
    }
    if (!q && !atts.length) return;
    if (st.view !== "chat") showView("chat");
    const prompt = buildPrompt(q, atts);
    await addUserMsg(shown, atts);
    remember("user", shown, atts);
    $("input").value = ""; autoGrow();
    st.atts = []; renderChips();
    st.streamText = ""; st.streamEl = null;
    const kinds = new Set(atts.map((a) => a.kind));
    setBusy(true, kinds.has("image") ? "Claude смотрит на картинку…" : kinds.has("pdf") || kinds.has("file") ? "Claude читает файл…" : "Claude думает…");
    try { await invoke("ask", { prompt, session: st.chat?.session || null }); }
    catch (e) { finish({ result: null, stderr: String(e) }); }
  }

  function onDelta(text) {
    if (!st.streamEl) { st.streamEl = addAnswerBox(); setStatus("", true); }
    st.streamText += text;
    if (!st.renderTimer) st.renderTimer = setTimeout(() => {
      st.renderTimer = 0;
      if (st.busy && st.streamEl) { renderMd(st.streamEl.body, st.streamText); scrollDown(); }
    }, 120);
  }
  function onTool(name) {
    // текст «сейчас поищу…» перед вызовом инструмента в итоговый ответ не входит
    st.streamText = "";
    if (st.streamEl) st.streamEl.body.innerHTML = "";
    setStatus(TOOL_STATUS[name] || "Работает…", true);
  }
  function finish(p) {
    clearTimeout(st.renderTimer); st.renderTimer = 0;
    let text;
    if (p.stopped) text = (st.streamText.trim() ? st.streamText.trimEnd() + "\n\n*(остановлено)*" : "*(остановлено)*");
    else if (p.result != null) text = (p.is_error ? "⚠️ " : "") + (p.result || "(пустой ответ)");
    else if (st.streamText.trim()) text = st.streamText;
    else text = "⚠️ Ошибка Claude:\n" + ((p.stderr || "нет вывода").trim());
    if (p.session_id && st.chat) st.chat.session = p.session_id;
    if (!st.streamEl) st.streamEl = addAnswerBox();
    finalizeAnswer(st.streamEl, text);
    st.streamEl = null;
    st.lastAnswer = text;
    remember("assistant", text);
    setBusy(false);
    $("input").focus();
    scrollDown();
  }

  // ---------------------------------------------------------------- история
  function remember(role, text, atts) {
    if (!st.chat) {
      const now = Date.now() / 1000;
      const title = (text || (atts && atts[0] && attLabel(atts[0])) || "Без названия").split("\n")[0].slice(0, 120);
      st.chat = { id: newId(), created: now, updated: now, title, session: null, messages: [] };
    }
    const m = { role, text, time: Date.now() / 1000 };
    if (atts && atts.length) m.atts = atts.map(({ kind, path, text, label }) => ({ kind, path, text, label }));
    st.chat.messages.push(m);
    st.chat.updated = Date.now() / 1000;
    invoke("history_save", { chat: st.chat }).catch((e) => console.warn("history", e));
  }

  async function renderHistory() {
    st.histChats = await invoke("history_list").catch(() => []);
    drawHistory();
  }
  function drawHistory() {
    const q = $("hist-search").value.trim().toLowerCase();
    const list = $("hist-list");
    list.innerHTML = "";
    const rows = st.histChats.filter((c) => !q || q.split(/\s+/).every((w) => (c.title + " " + c.messages.map((m) => m.text || "").join(" ")).toLowerCase().includes(w)));
    if (!rows.length) { list.innerHTML = `<div class="empty">${st.histChats.length ? "Ничего не найдено" : "История пуста"}</div>`; return; }
    rows.forEach((c, i) => {
      const n = c.messages.filter((m) => m.role === "user").length;
      const row = document.createElement("div");
      row.className = "hist-row" + (i === 0 ? " sel" : "");
      row.innerHTML = `<div class="t"><div class="title">${esc(c.title || "Без названия")}</div><div class="meta">${ago(c.updated)} · ${n} ${plural(n, "вопрос", "вопроса", "вопросов")}</div></div>`;
      const del = document.createElement("button");
      del.className = "hist-del"; del.textContent = "🗑"; del.title = "Удалить чат";
      del.onclick = async (e) => {
        e.stopPropagation();
        if (!del.classList.contains("confirm")) { del.classList.add("confirm"); del.textContent = "Удалить?"; setTimeout(() => { del.classList.remove("confirm"); del.textContent = "🗑"; }, 3000); return; }
        await invoke("history_delete", { id: c.id });
        if (st.chat && st.chat.id === c.id) newChat();
        st.histChats = st.histChats.filter((x) => x.id !== c.id); drawHistory();
      };
      row.appendChild(del);
      row.onclick = () => openChat(c);
      list.appendChild(row);
    });
  }
  async function openChat(c) {
    if (st.busy) return;
    newChat(true);
    st.chat = c;
    showView("chat");
    for (const m of c.messages) {
      if (m.role === "user") await addUserMsg(m.text || "", m.atts || []);
      else { const e = addAnswerBox(); finalizeAnswer(e, m.text || ""); st.lastAnswer = m.text || ""; }
    }
    updateSend(); $("input").focus(); scrollDown();
  }
  function newChat(silent) {
    if (st.busy) return;
    st.chat = null; st.lastAnswer = ""; st.atts = []; st.streamEl = null;
    $("log").innerHTML = ""; $("input").value = ""; autoGrow();
    renderChips(); setStatus("");
    if (!silent) { showView("chat"); $("input").focus(); }
    updateSend();
  }

  // ---------------------------------------------------------------- вид: чат / история / настройки
  function showView(v) {
    st.view = v;
    for (const n of ["chat", "history", "settings"]) $("view-" + n).hidden = n !== v;
    if (v === "history") { $("hist-search").value = ""; renderHistory(); $("hist-search").focus(); }
    if (v === "settings") loadSettings();
    if (v === "chat") $("input").focus();
    fit();
  }
  const toggleView = (v) => showView(st.view === v ? "chat" : v);

  // ---------------------------------------------------------------- быстрые действия
  function renderActions() {
    const box = $("actions");
    box.innerHTML = "";
    (st.cfg.actions || []).filter((a) => a.name && a.prompt).forEach((a, i) => {
      const b = document.createElement("button");
      b.className = "action"; b.textContent = a.name;
      b.title = a.prompt + (i < 9 ? "\n\n" + (IS_MAC ? "⌥" : "Alt+") + (i + 1) : "");
      b.onclick = () => runAction(a);
      box.appendChild(b);
    });
  }
  function runAction(a) {
    if (st.busy) return;
    if (!st.atts.length && !$("input").value.trim()) return toast("Сначала прикрепите скриншот, файл или текст — или напишите текст в поле.", 2500);
    send(a);
  }

  // ---------------------------------------------------------------- настройки
  const MODELS = ["", "opus", "sonnet", "haiku"];
  const MOD = IS_MAC ? { Alt: "⌥", Shift: "⇧", Control: "⌃", Super: "⌘" } : { Alt: "Alt", Shift: "Shift", Control: "Ctrl", Super: "Win" };
  function prettyAccel(a) {
    if (!a) return "Не задано";
    const parts = a.split("+");
    return parts.map((p) => MOD[p] || (p === "Command" || p === "Cmd" ? MOD.Super : p)).join(IS_MAC ? "" : "+");
  }
  async function loadSettings() {
    const c = st.cfg;
    const known = MODELS.includes(c.model);
    $("set-model").value = known ? c.model : "__custom";
    $("set-model-custom").hidden = known;
    $("set-model-custom").value = known ? "" : c.model;
    $("set-prompt").value = c.system_prompt || "";
    document.querySelectorAll("button.key").forEach((b) => { b.textContent = prettyAccel(c.shortcuts[b.dataset.key]); b.classList.remove("rec"); });
    $("set-autostart").checked = await invoke("autostart_get").catch(() => false);
    $("set-claude").textContent = st.claudePath ? st.claudePath : "не найден — установите Claude Code";
    const p = await invoke("check_permissions").catch(() => ({ os: "", screen: true, accessibility: true }));
    const mac = p.os === "macos";
    $("set-perm-title").hidden = $("perm-screen-row").hidden = $("perm-ax-row").hidden = !mac;
    for (const [id, key, which] of [["perm-screen", "screen", "screen"], ["perm-ax", "accessibility", "accessibility"]]) {
      const b = $(id);
      b.className = p[key] ? "ok" : "";
      b.textContent = p[key] ? "Разрешено ✓" : "Открыть настройки…";
      b.onclick = p[key] ? null : () => invoke("open_privacy_settings", { which });
    }
  }
  async function patchConfig(patch) {
    const res = await invoke("set_config", { patch });
    st.cfg = res.config;
    if (res.shortcut_errors && res.shortcut_errors.length) toast("Горячие клавиши: " + res.shortcut_errors.join("; "), 7000);
    renderActions();
  }
  $("set-model").onchange = (e) => {
    const v = e.target.value;
    $("set-model-custom").hidden = v !== "__custom";
    if (v !== "__custom") patchConfig({ model: v }); else $("set-model-custom").focus();
  };
  $("set-model-custom").oninput = (e) => patchConfig({ model: e.target.value.trim() });
  let spTimer = 0;
  $("set-prompt").oninput = (e) => { clearTimeout(spTimer); spTimer = setTimeout(() => patchConfig({ system_prompt: e.target.value.trim() }), 400); };
  $("set-autostart").onchange = (e) => invoke("autostart_set", { on: e.target.checked }).catch((err) => { toast(String(err)); e.target.checked = !e.target.checked; });

  // запись горячей клавиши
  document.querySelectorAll("button.key").forEach((b) => {
    b.onclick = () => { st.recording = b.dataset.key; document.querySelectorAll("button.key").forEach((x) => x.classList.remove("rec")); b.classList.add("rec"); b.textContent = "Нажмите сочетание…"; };
  });
  function accelFromEvent(e) {
    if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return null;
    const mods = [];
    if (e.ctrlKey) mods.push("Control"); if (e.altKey) mods.push("Alt"); if (e.shiftKey) mods.push("Shift"); if (e.metaKey) mods.push("Super");
    let key = e.code;
    if (key.startsWith("Key")) key = key.slice(3); else if (key.startsWith("Digit")) key = key.slice(5);
    return { mods, key };
  }
  document.addEventListener("keydown", (e) => {
    if (st.recording) {
      e.preventDefault(); e.stopPropagation();
      const name = st.recording, btn = document.querySelector(`button.key[data-key="${name}"]`);
      const done = (accel) => { st.recording = null; patchConfig({ shortcuts: { [name]: accel } }).then(() => (btn.textContent = prettyAccel(st.cfg.shortcuts[name]))); };
      if (e.key === "Escape") { st.recording = null; btn.classList.remove("rec"); btn.textContent = prettyAccel(st.cfg.shortcuts[name]); return; }
      if (e.key === "Backspace") return done("");
      const r = accelFromEvent(e);
      if (!r) return;
      if (!r.mods.length && !/^F\d+$/.test(r.key)) { btn.textContent = "Нужен модификатор (⌥ ⌃ ⌘ ⇧)…"; return; }
      btn.classList.remove("rec");
      return done([...r.mods, r.key].join("+"));
    }
    const cmd = IS_MAC ? e.metaKey : e.ctrlKey;
    if (e.key === "Escape") { if (st.view !== "chat") showView("chat"); else invoke("hide_panel"); return; }
    if (cmd && e.code === "KeyH") { e.preventDefault(); return toggleView("history"); }
    if (cmd && e.code === "Comma") { e.preventDefault(); return toggleView("settings"); }
    if (cmd && e.code === "KeyN") { e.preventDefault(); return newChat(); }
    if (e.altKey && !cmd && /^Digit[1-9]$/.test(e.code)) { e.preventDefault(); const a = (st.cfg.actions || []).filter((x) => x.name && x.prompt)[+e.code.slice(5) - 1]; if (a) runAction(a); return; }
    if (st.view === "history" && (e.key === "Enter")) { const first = document.querySelector(".hist-row"); first && first.click(); }
  }, true);

  // ---------------------------------------------------------------- ввод
  function autoGrow() {
    const t = $("input");
    t.style.height = "auto";
    if (!t.value) { t.style.height = ""; return; } // пустое поле — одна строка (плейсхолдер не должен влиять на замер)
    t.style.height = Math.min(t.scrollHeight, 120) + "px";
  }
  window.addEventListener("resize", autoGrow);
  $("input").addEventListener("input", () => { autoGrow(); updateSend(); });
  $("input").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); if (!st.busy) send(); }
  });
  $("btn-send").onclick = () => (st.busy ? invoke("stop") : send());
  $("btn-shot").onclick = () => invoke("capture").catch((e) => toast(String(e)));
  $("btn-history").onclick = () => toggleView("history");
  $("btn-settings").onclick = () => toggleView("settings");
  $("btn-new").onclick = () => newChat();
  $("btn-copy-last").onclick = (e) => copy(st.lastAnswer, e.currentTarget);
  $("hist-search").oninput = drawHistory;

  // ---------------------------------------------------------------- события из Rust
  listen("lens", (ev) => {
    const p = ev.payload;
    switch (p.kind) {
      case "delta": return onDelta(p.text);
      case "tool": return onTool(p.name);
      case "done": return finish(p);
      case "attach": if (p.type === "image") addAtt({ kind: "image", path: p.path, label: "Скриншот" }); else addText(p.text, "quote"); return $("input").focus();
      case "notice": return toast(p.text, 8000);
      case "open-settings": return showView("settings");
      case "shown": if (st.view === "chat") $("input").focus(); return fit();
    }
  });

  // ---------------------------------------------------------------- старт
  (async () => {
    st.cfg = await invoke("get_config");
    renderActions();
    invoke("claude_info").then((i) => { st.claudePath = i.path; if (!i.path) toast("Claude Code не найден. Установите его и войдите в аккаунт: claude.com/claude-code", 10000); });
    document.documentElement.dataset.os = IS_MAC ? "mac" : "other";
    updateSend(); autoGrow(); fit();
  })();
})();
