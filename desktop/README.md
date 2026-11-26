# Lens for macOS and Windows (Tauri)

Cross-platform version of Lens: a small panel for [Claude Code](https://claude.com/claude-code) that lives in the menu bar / system tray.
Same idea as the Linux app in the repository root, rewritten with [Tauri v2](https://tauri.app) (Rust + plain HTML/JS, no bundler).

> Unofficial community project, not affiliated with Anthropic. It runs the `claude` command you have installed and signed in to.

## Status

| | macOS | Windows |
|---|---|---|
| Panel, streaming answers, Markdown, code blocks with Copy | ✅ | ✅ |
| Chat history (last 100), search, delete | ✅ | ✅ |
| Attach: paste image/text, drag & drop files | ✅ | ✅ |
| Quick actions, settings, model, system prompt | ✅ | ✅ |
| Global hotkeys, tray icon, autostart | ✅ | ✅ |
| Area screenshot | ✅ `screencapture -i` | ⏳ planned (Snipping Tool) |
| Ask about selected text | ✅ simulates ⌘C, restores clipboard | ⏳ planned |
| Screenshot annotation, quiet mode | ⏳ planned | ⏳ planned |

**Not yet tested on real macOS/Windows hardware.** The macOS code type-checks (`cargo check --target aarch64-apple-darwin`) and the interface was exercised in a browser with a mocked backend. Real builds come from GitHub Actions (`.github/workflows/desktop.yml`).

## Default hotkeys

`⌥⇧Space` open the panel · `⌥⇧S` screenshot of an area · `⌥⇧C` selected text. All can be changed in Settings (⌘, / Ctrl+,).
In the panel: `⌘H` history, `⌘N` new chat, `⌥1…9` quick actions, `Esc` close.

## macOS permissions

- **Screen Recording** — for screenshots. macOS asks on first use; then enable *Lens* in System Settings → Privacy & Security → Screen Recording and restart the app.
- **Accessibility** — for "ask about selected text" (the app presses ⌘C in the other app). System Settings → Privacy & Security → Accessibility.

Settings → *Permissions* shows the current state with shortcuts to those panes.

## Build

Requirements: Node 20+, Rust (stable). On Linux also `webkit2gtk-4.1`, `libayatana-appindicator`, `librsvg`.

```bash
cd desktop/lens
npm install
npm run vendor      # copies marked / DOMPurify / highlight.js into src/vendor
npm run dev         # run in development
npm run build       # bundle (.dmg / .app on macOS, .exe / .msi on Windows)
```

A debug build needs several GB of disk space.
Unsigned builds: macOS shows "unidentified developer" (right-click → Open); Windows SmartScreen asks to confirm.

## Layout

```
desktop/lens/
  src/                 interface (index.html, style.css, app.js, vendor/)
  src/dev-mock.js      fake backend, only active when opened in a plain browser
  src-tauri/src/
    lib.rs             window, tray, hotkeys, commands
    claude.rs          runs `claude -p` (prompt over stdin) and streams events
    platform.rs        screenshot / selected text / permissions (macOS implemented)
    config.rs          settings JSON, defaults
    history.rs         chat history files
```
