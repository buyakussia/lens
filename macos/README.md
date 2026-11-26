# Lens для macOS — план

Пока не начато. Ядро уже не зависит от GUI, поэтому версия для macOS — это только новый интерфейс.

**Что переиспользуется как есть** (`crates/`):
- `lens-core` — запуск Claude Code (`claude -p`, картинки передаются в сообщении, без файлов), настройки, история чатов. Пути уже кросс-платформенные (`~/Library/Application Support/lens`).
- `clip-core` — хранилище истории буфера обмена с лимитами и закреплением.

**Что нужно написать** (SwiftUI + AppKit):
- Связка Swift ↔ Rust: `uniffi` или C ABI через `cbindgen`; ядро собирается как `staticlib`.
- Оверлей выделения: `NSPanel` на весь экран поверх всех окон и `ScreenCaptureKit` (`SCScreenshotManager`) для снимка — разрешение «Запись экрана» спрашивается один раз.
- Тихий режим: `NSPanel` с `.nonactivatingPanel`, чтобы не забирать фокус.
- Горячие клавиши: `RegisterEventHotKey` (Carbon) или `KeyboardShortcuts`.
- Clipboard History: опрос `NSPasteboard.changeCount` раз в ~0,5 с (в macOS нет события изменения буфера); вставка — `CGEvent` ⌘V (нужно разрешение «Универсальный доступ»).
- Меню-бар: `NSStatusItem`.
