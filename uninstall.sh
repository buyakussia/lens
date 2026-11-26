#!/usr/bin/env bash
# Удаление Lens и Clipboard History. Данные (история, настройки, снимки) остаются;
#   ./uninstall.sh --purge — удалить и их.
set -uo pipefail
PREFIX="${PREFIX:-$HOME/.local}"
LIB="$PREFIX/share/lens-desktop"

for cmd in lens clipboard-history; do
  [ -x "$PREFIX/bin/$cmd" ] && "$PREFIX/bin/$cmd" --quit >/dev/null 2>&1 || true
done
# горячие клавиши GNOME
[ -d "$LIB" ] && PYTHONPATH="$LIB" python3 - <<'PY' || true
from lens_common.system import GnomeHotkeys
h = GnomeHotkeys("", [])
for kid in ("lens-open", "lens-selection", "lens-capture", "lens-quiet", "clipboard-history-open"):
    h.remove(kid)
PY
for id in io.github.lensdesktop.Lens io.github.lensdesktop.ClipboardHistory; do
  rm -f "$PREFIX/share/applications/$id.desktop" "$HOME/.config/autostart/$id.desktop" \
        "$PREFIX/share/icons/hicolor/scalable/apps/$id.svg"
done
rm -f "$PREFIX/share/icons/hicolor/symbolic/apps/io.github.lensdesktop.Lens-symbolic.svg" \
      "$PREFIX/bin/lens" "$PREFIX/bin/clipboard-history"
rm -rf "$LIB"
if [ "${1:-}" = "--purge" ]; then
  rm -rf "$HOME/.config/lens" "$HOME/.config/clipboard-history" \
         "$HOME/.local/share/lens" "$HOME/.local/share/clipboard-history"
  echo "Удалено вместе с данными. Снимки в ~/Pictures/Lens не тронуты."
else
  echo "Удалено. Данные остались в ~/.config/{lens,clipboard-history} и ~/.local/share/{lens,clipboard-history}."
fi
