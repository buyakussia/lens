# Lens для Windows — план

Пока не начато. Ядро уже не зависит от GUI, поэтому версия для Windows — это только новый интерфейс.

**Что переиспользуется как есть** (`crates/`):
- `lens-core` — запуск Claude Code (`claude.exe` / `claude.cmd`, без окна консоли), настройки, история чатов (`%APPDATA%\lens`).
- `clip-core` — хранилище истории буфера обмена.

**Что нужно написать** (Rust + крейт `windows`, интерфейс на WinUI 3 или Win32/Direct2D):
- Оверлей выделения: окно `WS_EX_TOPMOST | WS_EX_LAYERED` на весь экран, снимок через `Windows.Graphics.Capture` или DXGI Desktop Duplication.
- Тихий режим: окно с `WS_EX_NOACTIVATE`, чтобы не забирать фокус.
- Горячие клавиши: `RegisterHotKey`. Win+V занят системной историей буфера — по умолчанию лучше Win+Shift+V или отключить системную.
- Clipboard History: `AddClipboardFormatListener` (`WM_CLIPBOARDUPDATE`), форматы `CF_UNICODETEXT`, `HTML Format`, `CF_DIBV5`; вставка — `SendInput` Ctrl+V. Секреты: пропускать данные с форматом `ExcludeClipboardContentFromMonitorProcessing`.
- Трей: `Shell_NotifyIconW`.
