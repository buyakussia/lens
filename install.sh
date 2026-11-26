#!/usr/bin/env bash
# Установка Lens и Clipboard History для текущего пользователя (в ~/.local).
#   ./install.sh               — оба приложения
#   ./install.sh lens          — только Lens
#   ./install.sh clipboard     — только Clipboard History
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
LIB="$PREFIX/share/lens-desktop"
WHAT="${1:-all}"
case "$WHAT" in all|lens|clipboard) ;; *) echo "Использование: $0 [all|lens|clipboard]"; exit 1;; esac
want() { [ "$WHAT" = all ] || [ "$WHAT" = "$1" ]; }

# --- зависимости
missing=()
python3 -c 'import gi; gi.require_version("Gtk", "3.0"); from gi.repository import Gtk' 2>/dev/null \
  || missing+=("PyGObject + GTK 3 (Arch: python-gobject gtk3; Debian/Ubuntu: python3-gi gir1.2-gtk-3.0)")
if [ ${#missing[@]} -gt 0 ]; then
  printf 'Не хватает: %s\n' "${missing[@]}"; exit 1
fi
if want lens; then
  command -v claude >/dev/null || [ -x "$HOME/.local/bin/claude" ] \
    || echo "⚠ Не найден Claude Code (команда claude). Lens отвечает через него: https://claude.com/claude-code"
  python3 -c 'import gi; gi.require_version("AyatanaAppIndicator3", "0.1")' 2>/dev/null \
    || echo "ℹ Нет libayatana-appindicator — значка в трее не будет (останется плавающая кнопка)."
fi

# --- файлы программы
mkdir -p "$LIB" "$PREFIX/bin" "$PREFIX/share/applications" \
         "$PREFIX/share/icons/hicolor/scalable/apps" "$PREFIX/share/icons/hicolor/symbolic/apps"
rm -rf "$LIB/lens_common"
cp -r "$HERE/src/lens_common" "$LIB/"
install_app() {  # $1 — пакет, $2 — команда, $3 — id
  rm -rf "$LIB/$1"
  cp -r "$HERE/src/$1" "$LIB/"
  find "$LIB" -name __pycache__ -prune -exec rm -rf {} +
  cat > "$PREFIX/bin/$2" <<SH
#!/bin/sh
PYTHONPATH="$LIB\${PYTHONPATH:+:\$PYTHONPATH}" exec python3 -m $1 "\$@"
SH
  chmod +x "$PREFIX/bin/$2"
  cp "$HERE/data/applications/$3.desktop" "$PREFIX/share/applications/"
  cp "$HERE/data/icons/scalable/$3.svg" "$PREFIX/share/icons/hicolor/scalable/apps/"
  # автозапуск в фоне
  mkdir -p "$HOME/.config/autostart"
  if [ ! -f "$HOME/.config/autostart/$3.desktop" ]; then
    sed -e "s|^Exec=$2\$|Exec=$PREFIX/bin/$2 --background|" -e '/^Actions=/d' -e '/^\[Desktop Action/,$d' \
        "$HERE/data/applications/$3.desktop" > "$HOME/.config/autostart/$3.desktop"
    echo "X-GNOME-Autostart-enabled=true" >> "$HOME/.config/autostart/$3.desktop"
  fi
  echo "✓ $2"
}
want lens && install_app lens_app lens io.github.lensdesktop.Lens
want lens && cp "$HERE/data/icons/symbolic/io.github.lensdesktop.Lens-symbolic.svg" \
                "$PREFIX/share/icons/hicolor/symbolic/apps/"
want clipboard && install_app clipboard_app clipboard-history io.github.lensdesktop.ClipboardHistory

command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true

case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "ℹ Добавьте $PREFIX/bin в PATH, чтобы запускать команды lens и clipboard-history.";; esac

# --- запуск (горячие клавиши GNOME приложения регистрируют сами при первом запуске)
want lens && (setsid "$PREFIX/bin/lens" --background >/dev/null 2>&1 < /dev/null &)
want clipboard && (setsid "$PREFIX/bin/clipboard-history" --background >/dev/null 2>&1 < /dev/null &)
echo "Готово. Lens: Super+C, Clipboard History: Super+V (меняются в настройках приложений)."
