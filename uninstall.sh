#!/usr/bin/env bash
# Удаление Lens и Clipboard History. Данные (история, настройки) остаются;
#   ./uninstall.sh --purge — удалить и их.
set -uo pipefail
PREFIX="${PREFIX:-$HOME/.local}"

for cmd in lens clipboard-history; do
  if [ -x "$PREFIX/bin/$cmd" ]; then
    "$PREFIX/bin/$cmd" --quit >/dev/null 2>&1 || true
    sleep 0.3
    "$PREFIX/bin/$cmd" --remove-hotkeys >/dev/null 2>&1 || true
  fi
done
for id in io.github.lensdesktop.Lens io.github.lensdesktop.ClipboardHistory; do
  rm -f "$PREFIX/share/applications/$id.desktop" "$HOME/.config/autostart/$id.desktop" \
        "$PREFIX/share/icons/hicolor/scalable/apps/$id.svg"
done
rm -f "$PREFIX/share/icons/hicolor/symbolic/apps/io.github.lensdesktop.Lens-symbolic.svg" \
      "$PREFIX/bin/lens" "$PREFIX/bin/clipboard-history"
rm -rf "$PREFIX/share/lens-desktop"
if [ "${1:-}" = "--purge" ]; then
  rm -rf "$HOME/.config/lens" "$HOME/.config/clipboard-history" \
         "$HOME/.local/share/lens" "$HOME/.local/share/clipboard-history"
  echo "Удалено вместе с данными."
else
  echo "Удалено. Данные остались в ~/.config/{lens,clipboard-history} и ~/.local/share/{lens,clipboard-history}."
fi
