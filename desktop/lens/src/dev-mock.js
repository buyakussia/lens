// Подмена бэкенда для просмотра интерфейса в обычном браузере (без Tauri). В приложении не используется.
(() => {
  if (window.__TAURI__) return;
  const handlers = {};
  const emit = (name, payload) => (handlers[name] || []).forEach((h) => h({ payload }));
  const LS = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch (_) { return d; } };
  let cfg = LS("mock.cfg", {
    model: "", system_prompt: "Отвечай по-русски, кратко.", shots_dir: "",
    shortcuts: { panel: "Alt+Shift+Space", capture: "Alt+Shift+S", selection: "Alt+Shift+C" },
    actions: [
      { name: "Перевести", prompt: "Переведи текст." }, { name: "Объяснить", prompt: "Объясни." },
      { name: "Распознать текст", prompt: "Распознай." }, { name: "Исправить код", prompt: "Исправь код." }, { name: "Кратко", prompt: "Кратко." },
    ],
  });
  const ANSWER = "Это **ошибка импорта**: модуль `requests` не установлен.\n\n### Что делать\n1. Установите пакет\n2. Перезапустите скрипт\n\n| Команда | Что делает |\n|---|---|\n| `pip install requests` | ставит пакет |\n\n```bash\npip install requests\n```\n\n```python\nimport requests\nprint(requests.get('https://example.com').status_code)\n```\n\nПодробнее — в [документации](https://pip.pypa.io).";
  let stopped = false;
  const pic = (c) => "data:image/svg+xml;utf8," + encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' width='300' height='100'><rect width='300' height='100' fill='${c}'/><text x='20' y='58' font-size='28' fill='white'>OPEN 24 HOURS</text></svg>`);
  const cmds = {
    get_config: () => cfg,
    set_config: ({ patch }) => { const m = (b, p) => { for (const k in p) b[k] = p[k] && typeof p[k] === "object" && !Array.isArray(p[k]) ? m(b[k] || {}, p[k]) : p[k]; return b; }; cfg = m(cfg, patch); localStorage.setItem("mock.cfg", JSON.stringify(cfg)); return { config: cfg, shortcut_errors: [] }; },
    claude_info: () => ({ path: "/opt/homebrew/bin/claude" }),
    check_permissions: () => ({ os: "macos", screen: false, accessibility: true }),
    open_privacy_settings: () => {}, autostart_get: () => false, autostart_set: () => {},
    fit_panel: () => {}, hide_panel: () => {}, copy_text: ({ text }) => { console.log("copy:", text.slice(0, 40)); }, open_url: ({ url }) => console.log("open", url),
    capture: () => emit("lens", { kind: "attach", type: "image", path: "/mock/shot.png" }),
    file_data_url: ({ path }) => pic(path.includes("paste") ? "#0f766e" : "#4f46e5"),
    add_file: ({ path }) => ({ kind: "file", path, label: path.split("/").pop() }),
    save_pasted_image: () => "/mock/paste.png",
    history_list: () => LS("mock.hist", []).sort((a, b) => b.updated - a.updated),
    history_save: ({ chat }) => { const l = LS("mock.hist", []).filter((c) => c.id !== chat.id); l.push(chat); localStorage.setItem("mock.hist", JSON.stringify(l)); },
    history_delete: ({ id }) => localStorage.setItem("mock.hist", JSON.stringify(LS("mock.hist", []).filter((c) => c.id !== id))),
    stop: () => { stopped = true; },
    ask: () => {
      stopped = false;
      setTimeout(() => emit("lens", { kind: "tool", name: "WebSearch" }), 300);
      let i = 0;
      const tick = () => {
        if (stopped) return emit("lens", { kind: "done", result: null, stopped: true, session_id: "mock-session" });
        if (i < ANSWER.length) { emit("lens", { kind: "delta", text: ANSWER.slice(i, i + 6) }); i += 6; return setTimeout(tick, 25); }
        emit("lens", { kind: "done", result: ANSWER, is_error: false, session_id: "mock-session", stopped: false });
      };
      setTimeout(tick, 900);
    },
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { if (!(cmd in cmds)) console.warn("mock: нет команды", cmd); return cmds[cmd] ? cmds[cmd](args || {}) : undefined; } },
    event: { listen: async (name, h) => { (handlers[name] = handlers[name] || []).push(h); return () => {}; } },
  };
  window.__mock = { emit };
})();
