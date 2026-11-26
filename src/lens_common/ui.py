"""Общий вид приложений Lens: стили, значок, тема GNOME, уведомления."""
import math
import subprocess

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gtk, Gdk, Gio

ACCENT = (0.388, 0.400, 0.945)  # индиго #6366f1

CSS = b"""
window.lens-bar { background-color: transparent; }
.bar { background-color: #262624; border: 1px solid #3d3d3a; border-radius: 18px; }
.bar entry { background: transparent; border: none; box-shadow: none; color: #ecebe8;
             font-size: 16px; caret-color: #6366f1; padding: 6px 2px; }
.bar entry:disabled { color: #8a8984; }
.bar .ghost { background: transparent; border: 1px solid transparent; box-shadow: none; color: #b5b3ad;
              border-radius: 10px; padding: 4px 8px; }
.bar .ghost:hover { background: #33332f; border-color: #45453f; color: #ecebe8; }
.bar .send { background: #6366f1; color: white; border: none; box-shadow: none; border-radius: 10px;
             min-width: 34px; min-height: 30px; }
.bar .send:hover { background: #818cf8; }
.bar .send:disabled { background: #34365e; color: #a99; }
.bar .chip { background: #30302c; border: 1px solid #45453f; border-radius: 10px; padding: 4px; }
.bar .chip label { color: #b5b3ad; font-size: 12px; }
.bar .answer, .bar .answer label { color: #ecebe8; font-size: 14px; }
.bar .mine { color: #9d9b95; font-size: 13px; }
.bar .status { color: #9d9b95; font-size: 13px; }
.bar .copy { font-size: 12px; padding: 2px 8px; margin-left: -8px; }
.bar .copy label { color: #9d9b95; }
.bar .copy:hover label { color: #ecebe8; }
.bar .drop { padding: 4px 2px; min-width: 0; margin-left: -6px; }
.bar .sep { background: #3d3d3a; min-height: 1px; }
.bar .hint { color: #77756f; font-size: 11px; }
.bar .actions { margin: 8px 14px 0 14px; }
.bar .action { background: #30302c; border: 1px solid #45453f; box-shadow: none; border-radius: 14px;
               padding: 2px 11px; min-height: 0; }
.bar .action label { color: #cfcdc7; font-size: 12px; }
.bar .action:hover { background: #3a3a35; border-color: #6366f1; }
.bar .action:disabled label { color: #6f6d68; }
.bar.light .action { background: #f0eee8; border-color: #dad8d0; }
.bar.light .action label { color: #3d3b37; }
.bar.light .action:hover { background: #e7e4dc; border-color: #6366f1; }
.bar.light .action:disabled label { color: #b0ada5; }
.bar .mdh { margin-top: 4px; }
.bar .quote { border-left: 3px solid #55544f; padding-left: 10px; }
.bar .mdhr { background: #3d3d3a; min-height: 1px; margin: 4px 0; }
.bar .codeblock { background: #1c1c1a; border: 1px solid #3d3d3a; border-radius: 10px; }
.bar .codehead { padding: 2px 4px 2px 12px; border-bottom: 1px solid #33332f; }
.bar .codelang, .bar .answer .codelang { color: #8a8984; font-size: 12px; }
.bar .code, .bar .answer .code { font-family: monospace; font-size: 13px; padding: 10px 12px; color: #e6e4df; }
.bar .codecopy { background: transparent; border: none; box-shadow: none; padding: 2px 8px; border-radius: 8px; }
.bar .codecopy label, .bar .answer .codecopy label { color: #9d9b95; font-size: 12px; }
.bar .codecopy:hover { background: #33332f; }
.bar .codecopy:hover label, .bar .answer .codecopy:hover label { color: #ecebe8; }
.bar .mdtable { border-top: 1px solid #45453f; border-left: 1px solid #45453f; }
.bar .cell { border-right: 1px solid #45453f; border-bottom: 1px solid #45453f; padding: 5px 10px; font-size: 13px; }
.bar .cell.head { background: #30302c; }

.bar.light { background-color: #faf9f5; border-color: #dad8d0; }
.bar.light entry { color: #1f1e1c; }
.bar.light entry:disabled { color: #9a978f; }
.bar.light .ghost { color: #5f5d58; }
.bar.light .ghost:hover { background: #eeece6; border-color: #dad8d0; color: #1f1e1c; }
.bar.light .send:disabled { background: #c7c9f9; color: #fff; }
.bar.light .chip { background: #f0eee8; border-color: #dad8d0; }
.bar.light .chip label { color: #5f5d58; }
.bar.light .answer, .bar.light .answer label { color: #1f1e1c; }
.bar.light .mine, .bar.light .status { color: #6b6964; }
.bar.light .copy label { color: #6b6964; }
.bar.light .copy:hover label { color: #1f1e1c; }
.bar.light .sep, .bar.light .mdhr { background: #dad8d0; }
.bar.light .quote { border-left-color: #cfccc3; }
.bar.light .codeblock { background: #f3f1ea; border-color: #dad8d0; }
.bar.light .codehead { border-bottom-color: #e2e0d8; }
.bar.light .codelang, .bar.light .answer .codelang { color: #7a786f; }
.bar.light .code, .bar.light .answer .code { color: #1f1e1c; }
.bar.light .codecopy label, .bar.light .answer .codecopy label { color: #6b6964; }
.bar.light .codecopy:hover { background: #e7e4dc; }
.bar.light .codecopy:hover label, .bar.light .answer .codecopy:hover label { color: #1f1e1c; }
.bar.light .mdtable { border-color: #dad8d0; }
.bar.light .cell { border-color: #dad8d0; }
.bar.light .cell.head { background: #efede6; }

.bar.dragover { border: 2px dashed #6366f1; }
.bar .hist { margin: 10px 12px 6px 12px; }
.bar .clipsearch { background: #30302c; border: 1px solid #45453f; border-radius: 10px; font-size: 14px;
                   padding: 4px 8px; color: #ecebe8; }
.bar.light .clipsearch { background: #ffffff; border-color: #dad8d0; color: #1f1e1c; }
.bar list row:selected { background: #3a3a35; }
.bar.light list row:selected { background: #e7e4dc; }
.bar .hist entry { background: #30302c; border: 1px solid #45453f; border-radius: 10px; font-size: 14px;
                   padding: 4px 8px; }
.bar list, .bar list row { background: transparent; }
.bar list row { border-radius: 10px; padding: 6px 8px; }
.bar list row:hover { background: #30302c; }
.bar .histtitle { color: #ecebe8; font-size: 14px; }
.bar .histmeta { color: #8a8984; font-size: 12px; }
.bar .histempty { color: #8a8984; font-size: 13px; margin: 18px; }
.bar .histdel { padding: 2px 6px; }
.bar .histdel.confirm { background: #b42318; color: white; border-color: #b42318; }
.bar .histdel.confirm label { color: white; }
.bar.light .hist entry { background: #ffffff; border-color: #dad8d0; color: #1f1e1c; }
.bar.light list row:hover { background: #efede6; }
.bar.light .histtitle { color: #1f1e1c; }
.bar.light .histmeta, .bar.light .histempty { color: #7a786f; }
.bar .chip .chipx { padding: 0 2px; min-height: 0; min-width: 0; }
.bar .atts { color: #9d9b95; font-size: 12px; }
.bar.light .atts { color: #6b6964; }
.lens-editor .ed-bar { padding: 8px 10px; }
.lens-editor .swatch { padding: 3px; min-width: 0; }
"""


def apply_css():
    prov = Gtk.CssProvider()
    prov.load_from_data(CSS)
    Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), prov,
                                             Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)


def rgba_window(win):
    """Прозрачный фон окна — для скруглённых панелей без рамки."""
    win.set_app_paintable(True)
    vis = win.get_screen().get_rgba_visual()
    if vis:
        win.set_visual(vis)


def draw_logo(cr, cx, cy, r, rgb=(1, 1, 1), angle=0.0):
    """Значок Lens: лупа с искрой. angle — поворот искры (анимация «думает»)."""
    cr.save()
    cr.set_source_rgb(*rgb)
    cr.set_line_cap(1)
    lw = max(1.6, r * 0.2)
    cr.set_line_width(lw)
    lx, ly, lr = cx - r * 0.18, cy - r * 0.18, r * 0.62
    cr.arc(lx, ly, lr, 0, 2 * math.pi)
    cr.stroke()
    a = math.pi / 4
    cr.set_line_width(lw * 1.25)
    cr.move_to(lx + math.cos(a) * lr, ly + math.sin(a) * lr)
    cr.line_to(cx + r * 0.92, cy + r * 0.92)
    cr.stroke()
    cr.restore()
    # искра внутри линзы
    cr.save()
    cr.translate(lx, ly)
    cr.rotate(angle)
    s = lr * 0.55
    cr.set_source_rgb(*rgb)
    cr.move_to(0, -s)
    cr.curve_to(0, -s * 0.2, s * 0.2, 0, s, 0)
    cr.curve_to(s * 0.2, 0, 0, s * 0.2, 0, s)
    cr.curve_to(0, s * 0.2, -s * 0.2, 0, -s, 0)
    cr.curve_to(-s * 0.2, 0, 0, -s * 0.2, 0, -s)
    cr.close_path()
    cr.fill()
    cr.restore()


class ThemeWatcher:
    """Следит за светлой/тёмной темой GNOME; callback(theme) при смене."""

    def __init__(self, callback=None):
        self.callback = callback
        self.theme = "dark"
        try:
            self.iface = Gio.Settings.new("org.gnome.desktop.interface")
            self.iface.connect("changed::color-scheme", lambda *a: self.update())
        except Exception:
            self.iface = None
        self.update(initial=True)

    def update(self, initial=False):
        scheme = self.iface.get_string("color-scheme") if self.iface else "prefer-dark"
        self.theme = "dark" if scheme == "prefer-dark" else "light"
        Gtk.Settings.get_default().set_property("gtk-application-prefer-dark-theme", self.theme == "dark")
        if not initial and self.callback:
            self.callback(self.theme)


def notify(app_name, text, icon="dialog-information"):
    try:
        subprocess.Popen(["notify-send", "-a", app_name, "-i", icon, app_name, text])
    except FileNotFoundError:
        print(app_name + ":", text)


def accel_from_key_event(display, e):
    """Разбор нажатия при записи горячей клавиши.

    Возвращает (статус, сочетание): "ok", "cancel" (Esc), "clear" (Backspace),
    "need_mod" (нажата буква без Ctrl/Alt/Super), "ignore" (одиночный модификатор).
    Буква берётся по физической клавише, поэтому русская раскладка не мешает.
    """
    if e.is_modifier:
        return "ignore", None
    mods = e.state & Gtk.accelerator_get_default_mod_mask()
    if not mods and e.keyval == Gdk.KEY_Escape:
        return "cancel", None
    if not mods and e.keyval == Gdk.KEY_BackSpace:
        return "clear", ""
    ok, keyval, _g, _l, _c = Gdk.Keymap.get_for_display(display).translate_keyboard_state(
        e.hardware_keycode, 0, 0)
    keyval = Gdk.keyval_to_lower(keyval if ok else e.keyval)
    fkey = Gdk.KEY_F1 <= keyval <= Gdk.KEY_F35 or keyval == Gdk.KEY_Print
    if not mods and not fkey:
        return "need_mod", None
    accel = Gtk.accelerator_name(keyval, mods).replace("<Primary>", "<Control>").replace("<Mod4>", "<Super>")
    return "ok", accel
