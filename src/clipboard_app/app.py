"""Clipboard History — история буфера обмена для Linux, аналог Win+V.

ClipStore   — хранение: ~/.local/share/clipboard-history (history.json + картинки).
ClipWatcher — следит за буфером (owner-change через XWayland) и записывает новое.
ClipPicker  — окно со списком: поиск, закрепление, удаление, вставка в предыдущее окно.
"""
import os
import sys
import json
import tempfile
import subprocess
import time
import hashlib
import html

os.environ.setdefault("GDK_BACKEND", "x11")  # XWayland: окно поверх остальных и слежение за буфером
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gtk, Gdk, GLib, GdkPixbuf, Gio

from lens_common import portals, ui

GLib.set_prgname("clipboard-history")
from lens_common.system import (Config, GnomeHotkeys, autostart_enabled, set_autostart,
                                find_executable)

APP_ID = "io.github.lensdesktop.ClipboardHistory"
APP_NAME = "Clipboard History"
CMD = find_executable("clipboard-history")
LENS_CMD = find_executable("lens")
DIR = os.path.expanduser("~/.local/share/clipboard-history")
FILE = os.path.join(DIR, "history.json")
CONFIG = Config("clipboard-history", {"max_items": 50, "auto_paste": True, "paste_token": ""})
MAX_ITEMS = 50          # незакреплённых записей по умолчанию (меняется в настройках)
MAX_TEXT = 200_000      # слишком длинный текст не храним
SECRET_TARGETS = {"x-kde-passwordManagerHint", "x-kde-passwordmanagerhint"}


class ClipStore:
    def __init__(self):
        os.makedirs(DIR, exist_ok=True)
        self.items = []
        try:
            with open(FILE, encoding="utf-8") as f:
                self.items = [i for i in json.load(f) if isinstance(i, dict) and i.get("kind")]
        except Exception:
            self.items = []
        self.listeners = []

    def save(self):
        tmp = FILE + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(self.items, f, ensure_ascii=False)
        os.replace(tmp, FILE)
        for cb in self.listeners:
            cb()

    def _prune(self):
        limit = int(CONFIG.get("max_items") or MAX_ITEMS)
        unpinned = [i for i in self.items if not i.get("pinned")]
        for old in unpinned[limit:]:
            self._drop(old)

    def _drop(self, item):
        if item in self.items:
            self.items.remove(item)
        if item.get("kind") == "image" and item.get("path"):
            if not any(i.get("path") == item["path"] for i in self.items):
                try:
                    os.remove(item["path"])
                except OSError:
                    pass

    def _new(self, **kw):
        kw.setdefault("pinned", False)
        kw["time"] = time.time()
        kw["id"] = "%x%s" % (int(time.time() * 1000), os.urandom(2).hex())
        return kw

    def add_text(self, text):
        if not text or not text.strip() or len(text) > MAX_TEXT:
            return
        for i in self.items:
            if i["kind"] == "text" and i.get("text") == text:
                self.move_top(i)
                return
        self.items.insert(0, self._new(kind="text", text=text))
        self._prune()
        self.save()

    def add_image(self, pixbuf):
        if pixbuf is None:
            return
        digest = hashlib.sha1(pixbuf.get_pixels()).hexdigest()[:16]
        for i in self.items:
            if i["kind"] == "image" and i.get("hash") == digest:
                self.move_top(i)
                return
        path = os.path.join(DIR, "img_%s.png" % digest)
        pixbuf.savev(path, "png", [], [])
        self.items.insert(0, self._new(kind="image", path=path, hash=digest,
                                       w=pixbuf.get_width(), h=pixbuf.get_height()))
        self._prune()
        self.save()

    def move_top(self, item):
        if item in self.items:
            self.items.remove(item)
            item["time"] = time.time()
            self.items.insert(0, item)
            self.save()

    def toggle_pin(self, item):
        item["pinned"] = not item.get("pinned")
        self.save()

    def remove(self, item):
        self._drop(item)
        self.save()

    def clear(self):
        for i in [i for i in self.items if not i.get("pinned")]:
            self._drop(i)
        self.save()


class ClipWatcher:
    """Записывает всё, что копируется в любом приложении."""

    def __init__(self, store):
        self.store = store
        self.ignore_until = 0.0
        self.pending = False
        self.cb = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        self.cb.connect("owner-change", self.on_change)

    def ignore(self, seconds=1.5):
        self.ignore_until = time.time() + seconds

    def on_change(self, *_a):
        if time.time() < self.ignore_until or self.pending:
            return
        self.pending = True
        GLib.timeout_add(120, self.capture)  # даём приложению-источнику закончить копирование

    def capture(self):
        self.pending = False
        try:
            ok, atoms = self.cb.wait_for_targets()
            names = {a.name() for a in atoms} if ok and atoms else set()
            if names & SECRET_TARGETS:
                return False  # менеджер паролей пометил содержимое как секретное
            if any(n.startswith("image/") for n in names):
                self.store.add_image(self.cb.wait_for_image())
            else:
                self.store.add_text(self.cb.wait_for_text())
        except Exception as e:
            print("clipboard:", e)
        return False


def _ago(ts):
    d = time.time() - ts
    if d < 60:
        return "только что"
    if d < 3600:
        return "%d мин назад" % (d // 60)
    if d < 86400:
        return "%d ч назад" % (d // 3600)
    return time.strftime("%d.%m %H:%M", time.localtime(ts))


class ClipPicker(Gtk.Window):
    W = 420

    def __init__(self, app, store, watcher):
        super().__init__(type=Gtk.WindowType.TOPLEVEL)
        self.app = app
        self.store = store
        self.watcher = watcher
        self.set_application(app)
        self.set_title("Буфер обмена")
        self.set_decorated(False)
        self.set_keep_above(True)
        self.set_skip_taskbar_hint(True)
        self.set_skip_pager_hint(True)
        self.set_type_hint(Gdk.WindowTypeHint.DIALOG)
        self.set_resizable(False)
        self.get_style_context().add_class("lens-bar")
        self.set_app_paintable(True)
        vis = self.get_screen().get_rgba_visual()
        if vis:
            self.set_visual(vis)
        self.connect("draw", lambda w, cr: (cr.set_operator(0), cr.paint(), cr.set_operator(2)) and False)
        self.connect("delete-event", lambda *a: self.hide() or True)
        self.connect("key-press-event", self.on_key)
        self.connect("focus-out-event", self.on_focus_out)
        store.listeners.append(lambda: self.get_visible() and self.fill())

        self.outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.outer.get_style_context().add_class("bar")
        self.outer.set_size_request(self.W, -1)
        self.add(self.outer)

        head = Gtk.Box(spacing=6)
        head.set_margin_start(14); head.set_margin_end(8); head.set_margin_top(10)
        t = Gtk.Label(xalign=0)
        t.set_markup("<b>Буфер обмена</b>")
        t.get_style_context().add_class("answer")
        head.pack_start(t, True, True, 0)
        clr = Gtk.Button(label="Очистить")
        clr.get_style_context().add_class("ghost")
        clr.set_tooltip_text("Удалить всё, кроме закреплённого")
        clr.connect("clicked", lambda *a: self.store.clear())
        gear = Gtk.Button.new_from_icon_name("emblem-system-symbolic", Gtk.IconSize.BUTTON)
        gear.get_style_context().add_class("ghost")
        gear.set_tooltip_text("Настройки")
        gear.connect("clicked", lambda *a: (self.hide(), self.app.show_settings()))
        head.pack_end(gear, False, False, 0)
        head.pack_end(clr, False, False, 0)
        self.outer.pack_start(head, False, False, 0)

        self.search = Gtk.SearchEntry()
        self.search.set_placeholder_text("Поиск…")
        self.search.set_margin_start(12); self.search.set_margin_end(12); self.search.set_margin_top(8)
        self.search.get_style_context().add_class("clipsearch")
        self.search.connect("search-changed", lambda *a: self.refilter())
        self.search.connect("activate", lambda *a: self.activate_selected())
        self.outer.pack_start(self.search, False, False, 0)

        self.sw = Gtk.ScrolledWindow()
        self.sw.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.sw.set_propagate_natural_height(True)
        self.sw.set_max_content_height(440)
        self.sw.set_margin_top(6); self.sw.set_margin_bottom(10)
        self.list = Gtk.ListBox()
        self.list.set_selection_mode(Gtk.SelectionMode.BROWSE)
        self.list.set_filter_func(self.filter)
        self.list.connect("row-activated", lambda lb, row: self.paste(row.item))
        self.list.set_margin_start(8); self.list.set_margin_end(8)
        self.sw.add(self.list)
        self.outer.pack_start(self.sw, True, True, 0)

        self.empty = Gtk.Label(label="Пока пусто — скопируйте что-нибудь.")
        self.empty.get_style_context().add_class("histempty")
        self.empty.set_no_show_all(True)
        self.outer.pack_start(self.empty, False, False, 0)

        hint = Gtk.Label(label="Enter — вставить · Shift+Enter — в терминал · Delete — удалить · Esc — закрыть",
                         xalign=0)
        hint.get_style_context().add_class("hint")
        hint.set_margin_start(14); hint.set_margin_bottom(10)
        self.outer.pack_start(hint, False, False, 0)
        self.outer.show_all()

    # --- показ
    def toggle(self):
        if self.get_visible():
            self.hide()
            return
        self.apply_theme()
        self.search.set_text("")
        self.fill()
        geo = Gdk.Display.get_default().get_monitor(0).get_workarea()
        self.resize(self.W, 1)
        self.show()
        w, h = self.get_size()
        self.move(geo.x + (geo.width - self.W) // 2, geo.y + int(geo.height * 0.18))
        self.set_keep_above(True)
        self.present()
        gw = self.get_window()
        if gw:
            gw.focus(Gdk.CURRENT_TIME)
        self.search.grab_focus()
        self.opened_at = time.time()

    def apply_theme(self):
        ctx = self.outer.get_style_context()
        (ctx.add_class if getattr(self.app, "theme", "dark") == "light" else ctx.remove_class)("light")

    def on_focus_out(self, *a):
        # закрываемся, когда пользователь ушёл в другое окно (но не сразу после открытия)
        if time.time() - getattr(self, "opened_at", 0) > 0.4:
            GLib.timeout_add(150, lambda: (not self.is_active() and self.hide()) or False)
        return False

    # --- список
    def fill(self):
        sel = self.list.get_selected_row()
        sel_id = sel.item["id"] if sel else None
        for r in self.list.get_children():
            r.destroy()
        items = sorted(self.store.items, key=lambda i: (not i.get("pinned"), -i.get("time", 0)))
        for item in items:
            self.list.add(self.make_row(item))
        self.list.show_all()
        self.refilter(keep=sel_id)

    def make_row(self, item):
        row = Gtk.ListBoxRow()
        row.item = item
        row.haystack = (item.get("text") or "картинка изображение image").lower()
        hb = Gtk.Box(spacing=8)
        hb.set_margin_top(2); hb.set_margin_bottom(2)
        vb = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        if item["kind"] == "image":
            try:
                pb = GdkPixbuf.Pixbuf.new_from_file_at_scale(item["path"], 300, 90, True)
                img = Gtk.Image.new_from_pixbuf(pb)
                img.set_halign(Gtk.Align.START)
                vb.pack_start(img, False, False, 0)
            except Exception:
                vb.pack_start(Gtk.Label(label="(картинка недоступна)", xalign=0), False, False, 0)
            meta = "Картинка %s×%s · %s" % (item.get("w", "?"), item.get("h", "?"), _ago(item.get("time", 0)))
        else:
            text = item.get("text", "")
            preview = "\n".join(text.strip().splitlines()[:3])
            lbl = Gtk.Label(label=preview[:400], xalign=0)
            lbl.set_line_wrap(True)
            lbl.set_line_wrap_mode(2)
            lbl.set_lines(3)
            lbl.set_ellipsize(3)
            lbl.set_max_width_chars(44)
            lbl.get_style_context().add_class("histtitle")
            vb.pack_start(lbl, False, False, 0)
            n = len(text)
            meta = "%d символов · %s" % (n, _ago(item.get("time", 0)))
        m = Gtk.Label(label=("📌 " if item.get("pinned") else "") + meta, xalign=0)
        m.get_style_context().add_class("histmeta")
        vb.pack_start(m, False, False, 0)
        hb.pack_start(vb, True, True, 0)

        btns = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)
        btns.set_valign(Gtk.Align.CENTER)
        for icon, tip, cb in (
                ("starred-symbolic" if item.get("pinned") else "non-starred-symbolic",
                 "Открепить" if item.get("pinned") else "Закрепить (не удалится при очистке)",
                 lambda *a: self.store.toggle_pin(item)),
                ("user-trash-symbolic", "Удалить", lambda *a: self.store.remove(item)),
                ("system-search-symbolic", "Спросить Claude в Lens", lambda *a: self.ask_claude(item))):
            if icon == "system-search-symbolic" and not os.path.exists(LENS_CMD):
                continue  # Lens не установлен
            b = Gtk.Button.new_from_icon_name(icon, Gtk.IconSize.MENU)
            b.get_style_context().add_class("ghost")
            b.set_tooltip_text(tip)
            b.set_can_focus(False)
            b.connect("clicked", cb)
            btns.pack_start(b, False, False, 0)
        hb.pack_end(btns, False, False, 0)
        row.add(hb)
        return row

    def filter(self, row):
        q = self.search.get_text().strip().lower()
        return not q or all(w in row.haystack for w in q.split())

    def refilter(self, keep=None):
        self.list.invalidate_filter()
        visible = [r for r in self.list.get_children() if self.filter(r)]
        self.empty.set_text("Пока пусто — скопируйте что-нибудь." if not self.store.items else "Ничего не найдено")
        self.empty.set_visible(not visible)
        self.sw.set_visible(bool(visible))
        target = next((r for r in visible if keep and r.item["id"] == keep), visible[0] if visible else None)
        if target:
            self.list.select_row(target)
        self.resize(self.W, 1)

    def move_sel(self, delta):
        visible = [r for r in self.list.get_children() if self.filter(r)]
        if not visible:
            return
        cur = self.list.get_selected_row()
        i = visible.index(cur) if cur in visible else -1
        r = visible[max(0, min(len(visible) - 1, i + delta))]
        self.list.select_row(r)
        adj = self.sw.get_vadjustment()
        a = r.get_allocation()
        if a.y < adj.get_value():
            adj.set_value(a.y)
        elif a.y + a.height > adj.get_value() + adj.get_page_size():
            adj.set_value(a.y + a.height - adj.get_page_size())

    def activate_selected(self, shift=False):
        r = self.list.get_selected_row()
        if r and self.filter(r):
            self.paste(r.item, shift)

    def on_key(self, w, e):
        k = e.keyval
        if k == Gdk.KEY_Escape:
            self.hide()
            return True
        if k in (Gdk.KEY_Down, Gdk.KEY_Tab):
            self.move_sel(1)
            return True
        if k in (Gdk.KEY_Up, Gdk.KEY_ISO_Left_Tab):
            self.move_sel(-1)
            return True
        if k in (Gdk.KEY_Return, Gdk.KEY_KP_Enter):
            self.activate_selected(shift=bool(e.state & Gdk.ModifierType.SHIFT_MASK))
            return True
        if k == Gdk.KEY_Delete and not self.search.get_text():
            r = self.list.get_selected_row()
            if r:
                self.store.remove(r.item)
            return True
        return False

    # --- действия
    def set_clipboard(self, item):
        cb = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        self.watcher.ignore()
        if item["kind"] == "image":
            cb.set_image(GdkPixbuf.Pixbuf.new_from_file(item["path"]))
        else:
            cb.set_text(item["text"], -1)
        # store() не вызываем: приложение работает постоянно и само отдаёт содержимое буфера

    def paste(self, item, shift=False):
        try:
            self.set_clipboard(item)
        except Exception as e:
            self.app.notify("Не удалось скопировать: %s" % e)
            return
        self.store.move_top(item)
        self.hide()
        if CONFIG.get("auto_paste"):
            # ждём, пока фокус вернётся в прежнее окно, затем нажимаем Ctrl+V через портал
            GLib.timeout_add(250, lambda: self.app.auto_paste(shift) or False)

    def ask_claude(self, item):
        """Передать запись в Lens (если установлен) — там её можно отправить Claude."""
        self.hide()
        try:
            if item["kind"] == "image":
                args = [LENS_CMD, "--attach-image", item["path"]]
            else:
                fd, path = tempfile.mkstemp(prefix="clip-", suffix=".txt",
                                            dir=os.environ.get("XDG_RUNTIME_DIR") or None)
                with os.fdopen(fd, "w", encoding="utf-8") as f:
                    f.write(item["text"])
                args = [LENS_CMD, "--attach-text-file", path]
            subprocess.Popen(args, start_new_session=True)
        except Exception as e:
            self.app.notify("Не удалось открыть Lens: %s" % e)


# ---------------------------------------------------------------- приложение
HOTKEYS = GnomeHotkeys(APP_NAME, [
    ("clipboard-history-open", "Открыть историю буфера", CMD, "<Super>v"),
])


class SettingsWindow(Gtk.Window):
    def __init__(self, app):
        super().__init__(title="Clipboard History — настройки")
        self.app = app
        self.set_application(app)
        self.set_default_size(480, -1)
        self.set_position(Gtk.WindowPosition.CENTER)
        self.set_icon_name(APP_ID)
        self.capturing = False
        self.connect("key-press-event", self.on_key)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        box.set_border_width(20)
        self.add(box)

        def row(title, widget, desc=None):
            hb = Gtk.Box(spacing=12)
            vb = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            vb.pack_start(Gtk.Label(label=title, xalign=0), False, False, 0)
            if desc:
                d = Gtk.Label(xalign=0)
                d.set_markup("<small>%s</small>" % GLib.markup_escape_text(desc))
                d.set_line_wrap(True)
                d.set_max_width_chars(42)
                d.get_style_context().add_class("dim-label")
                vb.pack_start(d, False, False, 0)
            hb.pack_start(vb, True, True, 0)
            widget.set_valign(Gtk.Align.CENTER)
            hb.pack_end(widget, False, False, 0)
            box.pack_start(hb, False, False, 0)

        self.key_btn = Gtk.Button(label=GnomeHotkeys.label(HOTKEYS.get("clipboard-history-open")))
        self.key_btn.set_size_request(170, -1)
        self.key_btn.connect("clicked", self.start_capture)
        row("Горячая клавиша", self.key_btn,
            "Нажмите на кнопку, затем сочетание. Esc — отмена, Backspace — отключить."
            if HOTKEYS.available else "Глобальные сочетания пока поддерживаются только в GNOME.")
        self.key_btn.set_sensitive(HOTKEYS.available)

        spin = Gtk.SpinButton.new_with_range(10, 500, 10)
        spin.set_value(int(CONFIG.get("max_items")))
        spin.connect("value-changed", lambda w: CONFIG.set("max_items", int(w.get_value())))
        row("Сколько записей хранить", spin, "Закреплённые записи не считаются и не удаляются.")

        ap = Gtk.Switch()
        ap.set_active(bool(CONFIG.get("auto_paste")))
        ap.connect("notify::active", lambda w, p: CONFIG.set("auto_paste", w.get_active()))
        row("Вставлять сразу", ap,
            "После выбора записи нажимать Ctrl+V в окне, где вы были. GNOME один раз спросит разрешение "
            "на управление клавиатурой и на долю секунды покажет значок в верхней панели.")

        reset = Gtk.Button(label="Сбросить")
        reset.connect("clicked", lambda *a: (CONFIG.set("paste_token", ""), reset.set_label("Сброшено ✓")))
        row("Разрешение на вставку", reset, "GNOME спросит разрешение заново при следующей вставке.")

        auto = Gtk.Switch()
        auto.set_active(autostart_enabled(APP_ID))
        auto.connect("notify::active", lambda w, p: set_autostart(
            APP_ID, APP_NAME, "История буфера обмена", CMD + " --background", APP_ID, w.get_active()))
        row("Запускать при входе в систему", auto, "Без этого история не записывается, пока приложение закрыто.")

        clr = Gtk.Button(label="Очистить историю")
        clr.get_style_context().add_class("destructive-action")
        clr.connect("clicked", lambda *a: self.app.store.clear())
        row("Удалить всё, кроме закреплённого", clr)
        self.show_all()

    def start_capture(self, *_a):
        self.capturing = True
        self.key_btn.set_label("Нажмите сочетание…")

    def on_key(self, w, e):
        if not self.capturing:
            if e.keyval == Gdk.KEY_Escape:
                self.close()
                return True
            return False
        status, accel = ui.accel_from_key_event(self.get_display(), e)
        if status == "ignore":
            return True
        if status == "need_mod":
            self.key_btn.set_label("Нужен Ctrl, Alt или Super…")
            return True
        self.capturing = False
        if status in ("ok", "clear"):
            HOTKEYS.set("clipboard-history-open", accel)
        self.key_btn.set_label(GnomeHotkeys.label(HOTKEYS.get("clipboard-history-open")))
        return True


class ClipboardApp(Gtk.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.picker = None
        self.settings_win = None
        self.theme = "dark"

    def do_startup(self):
        Gtk.Application.do_startup(self)
        ui.apply_css()
        self.themes = ui.ThemeWatcher(self.on_theme)
        self.theme = self.themes.theme
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.store = ClipStore()
        self.watcher = ClipWatcher(self.store)
        self.paster = portals.RemotePaste(self.bus, lambda: CONFIG.get("paste_token") or "",
                                          lambda t: CONFIG.set("paste_token", t))
        try:
            HOTKEYS.ensure_defaults()
        except Exception as e:
            print("hotkeys:", e, file=sys.stderr)
        self.hold()

    def on_theme(self, theme):
        self.theme = theme
        if self.picker and self.picker.get_visible():
            self.picker.apply_theme()

    def do_command_line(self, cmdline):
        args = cmdline.get_arguments()[1:]
        if self.picker is None:
            self.picker = ClipPicker(self, self.store, self.watcher)
        if "--background" in args:
            pass  # автозапуск: только следить за буфером
        elif "--settings" in args:
            self.show_settings()
        elif "--clear" in args:
            self.store.clear()
        elif "--quit" in args:
            self.quit()
        else:
            self.picker.toggle()
        return 0

    def show_settings(self):
        if self.settings_win is None:
            self.settings_win = SettingsWindow(self)
            self.settings_win.connect("destroy", lambda *a: setattr(self, "settings_win", None))
        self.settings_win.present()

    def auto_paste(self, shift=False):
        def done(ok, err):
            if not ok:
                self.notify("Скопировано — нажмите Ctrl+V. Автовставка не сработала: %s" % err)
        self.paster.paste(shift, done)

    @staticmethod
    def notify(text):
        ui.notify(APP_NAME, text, "edit-paste")


def main():
    return ClipboardApp().run(sys.argv)
