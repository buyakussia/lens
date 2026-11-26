# Lens & Clipboard History

Two small desktop apps for Linux (GNOME on Wayland first):

- **Lens** — a floating quick panel for [Claude Code](https://claude.com/claude-code): select part of the screen, a file or some text and ask about it. Think "Google Lens, but you can talk to it".
- **Clipboard History** — clipboard history with search and pinning, opened with <kbd>Super</kbd>+<kbd>V</kbd> like on Windows.

> Unofficial community project. Not affiliated with or endorsed by Anthropic. "Claude" is a trademark of Anthropic, PBC.
> Lens does not talk to any API itself: it runs the `claude` command you have installed and signed in to.

*По-русски — ниже.*

## Lens

| | |
|---|---|
| <kbd>Super</kbd>+<kbd>C</kbd> | open the panel |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> | ask about the text selected with the mouse |
| <kbd>Super</kbd>+<kbd>Alt</kbd>+<kbd>C</kbd> | **quiet mode**: snapshot the screen and show the answer in a small corner popup that never takes keyboard focus (videos keep playing) |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> / <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>S</kbd> | in the panel: screenshot of an area / the whole screen / a window |
| <kbd>Ctrl</kbd>+<kbd>H</kbd>, <kbd>Ctrl</kbd>+<kbd>N</kbd>, <kbd>Alt</kbd>+<kbd>1…9</kbd> | chat history, new chat, quick actions |

- Annotate screenshots before sending (arrow, box, highlighter, text).
- Attach images, PDFs, code and text: paste, drag & drop, or just copy something — it is attached automatically when you open the panel.
- Streaming answers with Markdown, tables and highlighted code blocks with a Copy button.
- Quick action buttons (Translate, Explain, Recognize text, Fix code, Summarize) — editable, add your own.
- Chat history (last 100 chats) with search; old chats can be continued.
- Settings: model, system prompt, hotkeys, snapshot folder, autostart, floating button. Light and dark GNOME themes.

## Clipboard History

- Records text and images copied in any app; up to 50 entries (configurable), pinned ones are kept.
- <kbd>Enter</kbd> pastes into the window you were in, <kbd>Shift</kbd>+<kbd>Enter</kbd> pastes into a terminal (<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd>).
- Entries that password managers mark as secret are not recorded.
- Optional: send an entry to Lens to ask Claude about it.

## Install

Requirements: Python 3, PyGObject and GTK 3. For Lens also [Claude Code](https://claude.com/claude-code) (`claude` in `PATH` or `~/.local/bin`), signed in. Optional: `libayatana-appindicator` (tray icon), `wl-clipboard`.

```bash
git clone <this repo> && cd lens
./install.sh            # both apps; or: ./install.sh lens | ./install.sh clipboard
```

Everything goes to `~/.local`; both apps start in the background at login. `./uninstall.sh` removes the apps (add `--purge` to remove history and settings too).

### GNOME notes

- **Tray icon.** GNOME does not show tray icons by itself. Install the AppIndicator extension (Arch: `gnome-shell-extension-appindicator`, Ubuntu has it already) — or use the floating button.
- **Full-screen and quiet-mode snapshots** use the system screenshot portal. Allow them once: Lens → Settings → *Screenshots without confirmation* → *Allow*.
- **Auto-paste** in Clipboard History uses the Remote Desktop portal: GNOME asks for permission once and briefly shows a "remote control" icon in the top bar while pasting. You can turn auto-paste off in its settings.
- Global hotkeys are registered as GNOME custom shortcuts and can be changed in each app's settings.

## Privacy

Everything stays on your computer: chat history in `~/.local/share/lens`, clipboard history in `~/.local/share/clipboard-history`, snapshots in `~/Pictures/Lens`. Only what you send to Claude goes through Claude Code to Anthropic.

## Status

Linux only for now. Built with Python + GTK 3 (windows run through XWayland so they can stay on top). Tested on GNOME 51 / Wayland. Ports to macOS and Windows are planned.

## License

MIT — see [LICENSE](LICENSE).

---

## По-русски

**Lens** — плавающая панель для Claude Code: снимок области экрана, файл или выделенный текст и вопрос о нём. **Clipboard History** — история буфера обмена на <kbd>Super</kbd>+<kbd>V</kbd>, как в Windows.

Установка: `./install.sh`. Нужен установленный и авторизованный Claude Code. Значок в трее в GNOME появится с расширением `gnome-shell-extension-appindicator`. Снимки всего экрана и тихий режим нужно один раз разрешить в настройках Lens, автовставку — в диалоге GNOME при первой вставке.

Неофициальный проект, не связан с Anthropic.
