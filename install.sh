#!/usr/bin/env bash
# Сборка и установка Lens и Clipboard History для текущего пользователя (в ~/.local).
#   ./install.sh               — оба приложения
#   ./install.sh lens          — только Lens
#   ./install.sh clipboard     — только Clipboard History
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
WHAT="${1:-all}"
case "$WHAT" in all|lens|clipboard) ;; *) echo "Использование: $0 [all|lens|clipboard]"; exit 1;; esac
want() { [ "$WHAT" = all ] || [ "$WHAT" = "$1" ]; }

if want lens; then
  command -v claude >/dev/null || [ -x "$HOME/.local/bin/claude" ] \
    || echo "⚠ Не найден Claude Code (команда claude). Lens отвечает через него: https://claude.com/claude-code"
fi

if [ -f "$HERE/Cargo.toml" ]; then
  # --- сборка из исходников
  if ! command -v cargo >/dev/null; then
    echo "Нужен Rust (cargo): https://rustup.rs  — или пакет rust/cargo вашего дистрибутива."; exit 1
  fi
  if ! pkg-config --exists 'gtk4 >= 4.14' 'libadwaita-1 >= 1.5' 2>/dev/null; then
    echo "Нужны библиотеки разработки GTK 4.14+ и libadwaita 1.5+:"
    echo "  Arch:          sudo pacman -S gtk4 libadwaita"
    echo "  Fedora:        sudo dnf install gtk4-devel libadwaita-devel"
    echo "  Debian/Ubuntu: sudo apt install libgtk-4-dev libadwaita-1-dev"
    exit 1
  fi
  pkgs=()
  want lens && pkgs+=(-p lens)
  want clipboard && pkgs+=(-p clipboard-history)
  echo "Сборка (первый раз — несколько минут)…"
  (cd "$HERE" && cargo build --release --locked "${pkgs[@]}")
fi
# иначе это готовый архив из Releases: программы уже лежат в target/release

# --- остановить запущенные версии (в том числе прежнюю на Python)
stop() { "$PREFIX/bin/$1" --quit >/dev/null 2>&1 || true; }
want lens && stop lens
want clipboard && stop clipboard-history
sleep 0.3
rm -rf "$PREFIX/share/lens-desktop"  # файлы прежней версии на Python

mkdir -p "$PREFIX/bin" "$PREFIX/share/applications" \
         "$PREFIX/share/icons/hicolor/scalable/apps" "$PREFIX/share/icons/hicolor/symbolic/apps" \
         "$HOME/.config/autostart"
install_app() {  # $1 — команда, $2 — id
  install -m 755 "$HERE/target/release/$1" "$PREFIX/bin/$1"
  sed "s|^Exec=$1|Exec=$PREFIX/bin/$1|" "$HERE/data/applications/$2.desktop" > "$PREFIX/share/applications/$2.desktop"
  cp "$HERE/data/icons/scalable/$2.svg" "$PREFIX/share/icons/hicolor/scalable/apps/"
  # автозапуск в фоне (если пользователь его не отключал)
  if [ ! -f "$HOME/.config/autostart/$2.desktop" ] || grep -q "share/lens-desktop\|python3" "$HOME/.config/autostart/$2.desktop"; then
    printf '[Desktop Entry]\nType=Application\nName=%s\nExec=%s --background\nIcon=%s\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n' \
      "$3" "$PREFIX/bin/$1" "$2" > "$HOME/.config/autostart/$2.desktop"
  fi
  echo "✓ $1"
}
want lens && install_app lens io.github.lensdesktop.Lens Lens
want lens && cp "$HERE/data/icons/symbolic/io.github.lensdesktop.Lens-symbolic.svg" "$PREFIX/share/icons/hicolor/symbolic/apps/"
want clipboard && install_app clipboard-history io.github.lensdesktop.ClipboardHistory "Clipboard History"

command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true

# --- запуск (горячие клавиши GNOME приложения регистрируют сами)
want lens && (setsid "$PREFIX/bin/lens" --background >/dev/null 2>&1 < /dev/null &)
want clipboard && (setsid "$PREFIX/bin/clipboard-history" --background >/dev/null 2>&1 < /dev/null &)
echo
echo "Готово."
want lens && echo "  Lens: Super+C — выделить область и спросить, Super+Alt+C — тихий вопрос по экрану."
want clipboard && echo "  Clipboard History: Super+V."
echo "  Первый раз Lens попросит разрешить снимки экрана, а история буфера — автовставку."
