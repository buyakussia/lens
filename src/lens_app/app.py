"""Lens — быстрая панель Claude Code поверх окон: вопрос по скриншоту, файлу или выделенному тексту.

Неофициальный проект, не связан с Anthropic. Ответы даёт установленный у пользователя Claude Code (CLI).
"""
import os, sys, json, re, shutil, subprocess, threading, time, html, math

os.environ.setdefault("GDK_BACKEND", "x11")  # XWayland: только так GNOME держит окно поверх остальных
import gi
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gtk, Gdk, Gio, GLib, GdkPixbuf
from lens_common import md_render, portals, ui
from lens_common.ui import CSS, draw_logo, rgba_window
from lens_common.system import Config, GnomeHotkeys, autostart_enabled, set_autostart, find_executable

APP_ID = "io.github.lensdesktop.Lens"
APP_NAME = "Lens"
CLAUDE = find_executable("claude")
LENS_CMD = find_executable("lens")
DATA_DIR = os.path.expanduser("~/.local/share/lens")
WORKDIR = os.path.join(DATA_DIR, "work")  # рабочая папка Claude Code (к ней привязаны сессии чатов)
SHOTS = os.path.expanduser("~/Pictures/Lens")  # папка снимков — меняется в настройках (set_shots_dir)
DEFAULT_SYSTEM_PROMPT = "Отвечай по-русски, кратко."
# конкретная длина ответа — «кратко» Claude понимает слишком мягко
BRIEF = ("[Длина ответа: 1–4 предложения или короткий список до 5 пунктов. Без вступлений, повторов вопроса и "
         "лишних пояснений. Подробнее — только если пользователь прямо попросит.]")
QUIET_BRIEF = ("[Ответ покажется в маленьком всплывающем окне: максимум 2 коротких предложения, "
               "без заголовков и списков.]")
DEFAULTS = {"model": "", "system_prompt": DEFAULT_SYSTEM_PROMPT, "shots_dir": "~/Pictures/Lens",
            "bubble": True, "auto_clip": True}
AUTO_CLIP_FRESH = 300  # прикреплять скопированное не старше 5 минут
DEFAULT_ACTIONS = [
    {"name": "Перевести", "prompt": "Переведи весь текст из вложения на русский (если он уже на русском — на "
                                     "английский). Сохрани форматирование, без лишних пояснений."},
    {"name": "Объяснить", "prompt": "Объясни простыми словами, что здесь изображено или написано и что это значит."},
    {"name": "Распознать текст", "prompt": "Распознай и перепиши весь текст из вложения дословно, сохранив "
                                            "структуру (абзацы, списки, таблицы). Без комментариев."},
    {"name": "Исправить код", "prompt": "Найди ошибки в коде или в сообщении об ошибке во вложении, коротко объясни "
                                         "причину и дай исправленный код целиком в блоке кода."},
    {"name": "Кратко", "prompt": "Перескажи суть в 2–3 предложениях."},
]
SIZE = 54
BAR_W = 660
ACCENT = ui.ACCENT
MODES = {"area": ("Область", "Ctrl+S"),
         "screen": ("Весь экран", "Ctrl+Shift+S"),
         "window": ("Окно (выбор в GNOME)", "Ctrl+Alt+S")}
KEY_S, KEY_N, KEY_Z, KEY_V, KEY_H = 39, 57, 52, 55, 43  # аппаратные коды клавиш S, N, Z, V, H (X11/evdev)
KEY_DIGITS = list(range(10, 19))  # аппаратные коды клавиш 1…9
FILES = os.path.join(SHOTS, "files")  # копии перетащенных/вставленных файлов
MAX_FILE = 25 * 1024 * 1024
IMAGE_EXT = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp"}
TEXT_TYPES = {"application/json", "application/xml", "application/javascript", "application/x-shellscript",
              "application/x-yaml", "application/toml", "application/sql", "application/x-php",
              "application/x-python", "application/typescript", "application/x-ruby", "application/x-perl"}
GLib.set_prgname("lens")

CONFIG = Config("lens", DEFAULTS)


def load_conf():
    return CONFIG.load()


def save_conf(c):
    try:
        CONFIG.save(c)
    except Exception:
        pass


def conf_get(key):
    return load_conf().get(key, DEFAULTS.get(key))


def get_actions():
    acts = load_conf().get("actions")
    if not isinstance(acts, list):
        acts = DEFAULT_ACTIONS
    return [a for a in acts if isinstance(a, dict) and (a.get("name") or "").strip() and (a.get("prompt") or "").strip()]


def conf_set(key, value):
    c = load_conf()
    c[key] = value
    save_conf(c)


def set_shots_dir(path):
    global SHOTS, FILES
    SHOTS = os.path.expanduser(path or DEFAULTS["shots_dir"])
    FILES = os.path.join(SHOTS, "files")
    os.makedirs(SHOTS, exist_ok=True)


HISTORY_DIR = os.path.join(DATA_DIR, "history")
HISTORY_LIMIT = 100
MONTHS = ["янв.", "февр.", "марта", "апр.", "мая", "июня", "июля", "авг.", "сент.", "окт.", "нояб.", "дек."]


class ChatHistory:
    """Один чат = один JSON-файл в ~/.local/share/lens/history. Хранятся последние 100."""

    @staticmethod
    def new_chat():
        cid = time.strftime("%Y%m%d-%H%M%S-") + os.urandom(3).hex()
        now = time.time()
        return {"id": cid, "created": now, "updated": now, "title": "", "session": None, "messages": []}

    @staticmethod
    def path(cid):
        return os.path.join(HISTORY_DIR, os.path.basename(cid) + ".json")

    @classmethod
    def save(cls, chat):
        os.makedirs(HISTORY_DIR, exist_ok=True)
        chat["updated"] = time.time()
        tmp = cls.path(chat["id"]) + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(chat, f, ensure_ascii=False, indent=1)
        os.replace(tmp, cls.path(chat["id"]))
        cls.prune()

    @classmethod
    def load_all(cls):
        chats = []
        try:
            names = os.listdir(HISTORY_DIR)
        except FileNotFoundError:
            return chats
        for n in names:
            if n.endswith(".json"):
                try:
                    with open(os.path.join(HISTORY_DIR, n), encoding="utf-8") as f:
                        c = json.load(f)
                    if isinstance(c, dict) and c.get("id"):
                        chats.append(c)
                except Exception:
                    pass
        chats.sort(key=lambda c: c.get("updated", 0), reverse=True)
        return chats

    @classmethod
    def delete(cls, cid):
        try:
            os.remove(cls.path(cid))
        except FileNotFoundError:
            pass

    @classmethod
    def prune(cls):
        for c in cls.load_all()[HISTORY_LIMIT:]:
            cls.delete(c["id"])

    @staticmethod
    def fmt_date(ts):
        t = time.localtime(ts)
        now = time.localtime()
        if t.tm_year == now.tm_year and t.tm_yday == now.tm_yday:
            return "Сегодня, %s" % time.strftime("%H:%M", t)
        if time.localtime(time.time() - 86400).tm_yday == t.tm_yday and abs(time.time() - ts) < 2 * 86400:
            return "Вчера, %s" % time.strftime("%H:%M", t)
        date = "%d %s" % (t.tm_mday, MONTHS[t.tm_mon - 1])
        if t.tm_year != now.tm_year:
            date += " %d" % t.tm_year
        return "%s, %s" % (date, time.strftime("%H:%M", t))

    @staticmethod
    def title_for(q, atts):
        if q:
            return q.strip().splitlines()[0][:120]
        for a in atts:
            if a["kind"] in ("quote", "text"):
                return "«%s»" % " ".join(a["text"].split())[:100]
        if atts:
            return atts[0].get("label") or "Вложение"
        return "Без названия"


# ---------------------------------------------------------------- плавающая кнопка
class Bubble(Gtk.Window):
    def __init__(self, app):
        super().__init__(type=Gtk.WindowType.TOPLEVEL)
        self.app = app
        self.set_application(app)
        self.set_title(APP_NAME)
        self.set_decorated(False)
        self.set_keep_above(True)
        self.set_skip_taskbar_hint(True)
        self.set_skip_pager_hint(True)
        self.set_type_hint(Gdk.WindowTypeHint.UTILITY)
        self.set_accept_focus(False)
        self.set_resizable(False)
        self.set_default_size(SIZE, SIZE)
        rgba_window(self)
        self.stick()
        self.busy = False
        self.hover = False
        self.angle = 0.0
        self.press = None
        self.dragged = False

        area = Gtk.DrawingArea()
        area.set_size_request(SIZE, SIZE)
        area.connect("draw", self.on_draw)
        area.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK |
                        Gdk.EventMask.POINTER_MOTION_MASK | Gdk.EventMask.ENTER_NOTIFY_MASK |
                        Gdk.EventMask.LEAVE_NOTIFY_MASK)
        area.connect("button-press-event", self.on_press)
        area.connect("button-release-event", self.on_release)
        area.connect("motion-notify-event", self.on_motion)
        area.connect("enter-notify-event", lambda *a: self.set_hover(True))
        area.connect("leave-notify-event", lambda *a: self.set_hover(False))
        area.set_tooltip_text("Клик — открыть Claude\nПеретащить — переместить\nПравый клик — меню")
        self.area = area
        self.add(area)

        c = load_conf()
        geo = Gdk.Display.get_default().get_monitor(0).get_geometry()
        self.move(c.get("x", geo.x + geo.width - SIZE - 24), c.get("y", geo.y + geo.height // 2))
        self.connect("configure-event", self.on_configure)

        self.menu = Gtk.Menu()
        for label, cb in (("Открыть Claude", lambda *a: self.app.bar.toggle()),
                          ("Спросить про выделенный текст", lambda *a: self.app.ask_selection()),
                          ("Снимок: область", lambda *a: self.app.bar.open_and_capture("area")),
                          ("Снимок: весь экран", lambda *a: self.app.bar.open_and_capture("screen")),
                          ("Снимок: окно", lambda *a: self.app.bar.open_and_capture("window")),
                          ("Открыть папку скриншотов", lambda *a: Gio.AppInfo.launch_default_for_uri(
                              Gio.File.new_for_path(SHOTS).get_uri(), None)),
                          ("Настройки…", lambda *a: self.app.show_settings()),
                          ("Выйти", lambda *a: self.app.quit())):
            it = Gtk.MenuItem(label=label)
            it.connect("activate", cb)
            self.menu.append(it)
        self.menu.show_all()

    def set_hover(self, v):
        self.hover = v
        self.area.queue_draw()

    def set_busy(self, v):
        was = self.busy
        self.busy = v
        if v and not was:
            GLib.timeout_add(40, self.tick)
        self.area.queue_draw()

    def tick(self):
        self.angle += 0.12
        self.area.queue_draw()
        return self.busy

    def on_draw(self, w, cr):
        cr.set_operator(0)
        cr.paint()
        cr.set_operator(2)
        cx = cy = SIZE / 2
        r = SIZE / 2 - 3
        cr.set_source_rgba(0, 0, 0, 0.25)
        cr.arc(cx, cy + 1.5, r, 0, 2 * math.pi)
        cr.fill()
        k = 1.0 if self.hover else 0.92
        cr.set_source_rgba(ACCENT[0] * k, ACCENT[1] * k, ACCENT[2] * k, 1)
        cr.arc(cx, cy, r - 1, 0, 2 * math.pi)
        cr.fill()
        if self.busy:
            cr.set_source_rgba(1, 1, 1, 0.9)
            cr.set_line_width(3)
            cr.arc(cx, cy, r - 4, self.angle, self.angle + 1.6)
            cr.stroke()
        draw_logo(cr, cx, cy, r * 0.55, angle=self.angle * 0.3 if self.busy else 0)
        return False

    def on_press(self, w, e):
        if e.button == 3:
            self.menu.popup_at_pointer(e)
            return True
        if e.button == 1:
            self.press = (e.x_root, e.y_root, e.button, e.time)
            self.dragged = False
        return True

    def on_motion(self, w, e):
        if self.press and not self.dragged:
            if abs(e.x_root - self.press[0]) + abs(e.y_root - self.press[1]) > 5:
                self.dragged = True
                self.begin_move_drag(self.press[2], int(self.press[0]), int(self.press[1]), self.press[3])
        return True

    def on_release(self, w, e):
        if e.button == 1 and self.press and not self.dragged:
            self.app.bar.toggle(e.time)
        self.press = None
        return True

    def on_configure(self, w, e):
        x, y = self.get_position()
        c = load_conf()
        if (c.get("x"), c.get("y")) != (x, y):
            c.update(x=x, y=y)
            save_conf(c)
        return False


# ---------------------------------------------------------------- быстрая панель
class QuickBar(Gtk.Window):
    def __init__(self, app):
        super().__init__(type=Gtk.WindowType.TOPLEVEL)
        self.app = app
        self.set_application(app)
        self.set_title("Claude")
        self.set_decorated(False)
        self.set_keep_above(True)
        self.set_skip_taskbar_hint(True)
        self.set_skip_pager_hint(True)
        self.set_type_hint(Gdk.WindowTypeHint.DIALOG)
        self.set_resizable(False)
        self.get_style_context().add_class("lens-bar")
        rgba_window(self)
        self.connect("draw", self.on_win_draw)
        self.connect("delete-event", lambda *a: self.hide() or True)
        self.connect("key-press-event", self.on_key)
        self.connect("size-allocate", self.on_alloc)

        self.session = None
        self.chat = None  # текущий чат для истории
        self.attachments = []
        self.busy = False
        self.angle = 0.0
        self.last_answer = ""
        self.bottom_y = None
        self.md_views = {}

        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.outer = outer
        outer.set_size_request(BAR_W, -1)
        outer.get_style_context().add_class("bar")
        self.add(outer)

        # история
        self.sw = Gtk.ScrolledWindow()
        self.sw.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.sw.set_propagate_natural_height(True)
        self.sw.set_max_content_height(440)
        self.log = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        self.log.set_margin_start(18); self.log.set_margin_end(18)
        self.log.set_margin_top(14); self.log.set_margin_bottom(10)
        self.sw.add(self.log)
        self.sw.set_no_show_all(True)
        outer.pack_start(self.sw, True, True, 0)

        # страница «История» (показывается вместо переписки)
        self.hist = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.hist.get_style_context().add_class("hist")
        self.hist_search = Gtk.SearchEntry()
        self.hist_search.set_placeholder_text("Поиск по истории…")
        self.hist_search.connect("search-changed", lambda *a: self.hist_list.invalidate_filter() or self.update_hist_empty())
        self.hist_search.connect("activate", self.on_hist_activate_first)
        self.hist_sw = Gtk.ScrolledWindow()
        self.hist_sw.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.hist_sw.set_propagate_natural_height(True)
        self.hist_sw.set_max_content_height(380)
        self.hist_list = Gtk.ListBox()
        self.hist_list.set_selection_mode(Gtk.SelectionMode.NONE)
        self.hist_list.set_activate_on_single_click(True)
        self.hist_list.set_filter_func(self.hist_filter)
        self.hist_list.connect("row-activated", lambda lb, row: self.open_chat(row.chat))
        self.hist_sw.add(self.hist_list)
        self.hist_empty = Gtk.Label(label="История пуста")
        self.hist_empty.get_style_context().add_class("histempty")
        self.hist_empty.set_no_show_all(True)
        self.hist.pack_start(self.hist_search, False, False, 0)
        self.hist.pack_start(self.hist_sw, True, True, 0)
        self.hist.pack_start(self.hist_empty, False, False, 0)
        self.hist.set_no_show_all(True)
        outer.pack_start(self.hist, True, True, 0)
        self.sep = Gtk.Box()
        self.sep.get_style_context().add_class("sep")
        self.sep.set_no_show_all(True)
        outer.pack_start(self.sep, False, False, 0)

        # вложения: скриншоты, картинки, файлы, текст — чипами с крестиком
        self.chips = Gtk.FlowBox()
        self.chips.set_selection_mode(Gtk.SelectionMode.NONE)
        self.chips.set_max_children_per_line(3)
        self.chips.set_column_spacing(6); self.chips.set_row_spacing(6)
        self.chips.set_homogeneous(False)
        self.chips.set_margin_start(14); self.chips.set_margin_end(14); self.chips.set_margin_top(12)
        self.chips.set_no_show_all(True)
        outer.pack_start(self.chips, False, False, 0)

        # перетаскивание файлов и текста в панель
        self.drag_dest_set(Gtk.DestDefaults.ALL, [], Gdk.DragAction.COPY)
        self.drag_dest_add_uri_targets()
        self.drag_dest_add_text_targets()
        self.connect("drag-data-received", self.on_drop)
        self.connect("drag-motion", lambda *a: self.outer.get_style_context().add_class("dragover") or False)
        self.connect("drag-leave", lambda *a: self.outer.get_style_context().remove_class("dragover"))

        # быстрые действия
        self.actions_box = Gtk.FlowBox()
        self.actions_box.set_selection_mode(Gtk.SelectionMode.NONE)
        self.actions_box.set_max_children_per_line(10)
        self.actions_box.set_column_spacing(6); self.actions_box.set_row_spacing(6)
        self.actions_box.get_style_context().add_class("actions")
        outer.pack_start(self.actions_box, False, False, 0)
        self.action_btns = []
        self.pending_action = None

        # строка ввода
        row = Gtk.Box(spacing=8)
        row.set_margin_start(14); row.set_margin_end(12); row.set_margin_top(10); row.set_margin_bottom(10)
        self.logo = Gtk.DrawingArea()
        self.logo.set_size_request(28, 28)
        self.logo.set_valign(Gtk.Align.CENTER)
        self.logo.connect("draw", self.on_logo_draw)
        self.entry = Gtk.Entry()
        self.entry.set_placeholder_text("Чем могу помочь?")
        self.entry.connect("activate", self.on_send)
        self.entry.connect("button-press-event", lambda *a: self.offer_clipboard() or False)
        self.clip_offered = set()     # что из буфера уже предлагали — второй раз не прикрепляем
        self.capture_started = 0.0
        self.entry.connect("changed", lambda *a: self.update_send())
        GLib.idle_add(lambda: self.rebuild_actions() or False)
        # кнопка скриншота + выпадающее меню режимов
        shot = Gtk.Box(spacing=0)
        sbtn = Gtk.Button.new_from_icon_name("applets-screenshooter-symbolic", Gtk.IconSize.BUTTON)
        if not Gtk.IconTheme.get_default().has_icon("applets-screenshooter-symbolic"):
            sbtn.set_image(Gtk.Image.new_from_icon_name("camera-photo-symbolic", Gtk.IconSize.BUTTON))
        sbtn.get_style_context().add_class("ghost")
        sbtn.set_tooltip_text("Выделить область экрана  (Ctrl+S)")
        sbtn.connect("clicked", lambda *a: self.capture("area"))
        mbtn = Gtk.MenuButton()
        mbtn.set_image(Gtk.Image.new_from_icon_name("pan-down-symbolic", Gtk.IconSize.MENU))
        mbtn.get_style_context().add_class("ghost")
        mbtn.get_style_context().add_class("drop")
        mbtn.set_tooltip_text("Режим скриншота")
        mmenu = Gtk.Menu()
        for mode in ("area", "screen", "window"):
            label, accel = MODES[mode]
            it = Gtk.MenuItem()
            hb = Gtk.Box(spacing=24)
            hb.pack_start(Gtk.Label(label=label, xalign=0), True, True, 0)
            al = Gtk.Label(label=accel)
            al.get_style_context().add_class("dim-label")
            hb.pack_end(al, False, False, 0)
            it.add(hb)
            it.connect("activate", lambda _w, m=mode: self.capture(m))
            mmenu.append(it)
        mmenu.show_all()
        mbtn.set_popup(mmenu)
        mbtn.set_direction(Gtk.ArrowType.UP)
        shot.pack_start(sbtn, False, False, 0)
        shot.pack_start(mbtn, False, False, 0)
        self.newchat = Gtk.Button(label="Новый чат")
        self.newchat.get_style_context().add_class("ghost")
        self.newchat.set_tooltip_text("Начать заново  (Ctrl+N)")
        self.newchat.connect("clicked", lambda *a: self.reset())
        copy = Gtk.Button.new_from_icon_name("edit-copy-symbolic", Gtk.IconSize.BUTTON)
        copy.get_style_context().add_class("ghost")
        copy.set_tooltip_text("Скопировать последний ответ")
        copy.connect("clicked", lambda *a: md_render.copy_to_clipboard(self.last_answer))
        self.copy_btn = copy
        copy.set_no_show_all(True)
        self.send = Gtk.Button.new_from_icon_name("go-up-symbolic", Gtk.IconSize.BUTTON)
        self.send.get_style_context().add_class("send")
        self.send.set_tooltip_text("Отправить  (Enter)")
        self.send.connect("clicked", self.on_send)
        row.pack_start(self.logo, False, False, 0)
        row.pack_start(self.entry, True, True, 0)
        row.pack_start(copy, False, False, 0)
        row.pack_start(shot, False, False, 0)
        self.hist_btn = Gtk.Button.new_from_icon_name("document-open-recent-symbolic", Gtk.IconSize.BUTTON)
        self.hist_btn.get_style_context().add_class("ghost")
        self.hist_btn.set_tooltip_text("История  (Ctrl+H)")
        self.hist_btn.connect("clicked", lambda *a: self.toggle_history())
        row.pack_start(self.hist_btn, False, False, 0)
        row.pack_start(self.newchat, False, False, 0)
        row.pack_start(self.send, False, False, 0)
        outer.pack_start(row, False, False, 0)

        self.status = Gtk.Label()
        self.status.get_style_context().add_class("status")
        self.status.set_xalign(0)
        self.status.set_margin_start(56); self.status.set_margin_bottom(10)
        self.status.set_no_show_all(True)
        outer.pack_start(self.status, False, False, 0)

        outer.show_all()
        self.update_send()

    # --- отрисовка
    def on_win_draw(self, w, cr):
        cr.set_operator(0)
        cr.paint()
        cr.set_operator(2)
        return False

    def on_logo_draw(self, w, cr):
        draw_logo(cr, 14, 14, 12, rgb=ACCENT, angle=self.angle if self.busy else 0)
        return False

    def tick(self):
        self.angle += 0.1
        self.logo.queue_draw()
        return self.busy

    # --- позиция: панель растёт вверх, низ остаётся на месте
    def place(self):
        geo = Gdk.Display.get_default().get_monitor(0).get_workarea()
        self.bottom_y = geo.y + geo.height - 90
        self.left_x = geo.x + (geo.width - BAR_W) // 2

    def on_alloc(self, w, alloc):
        if self.bottom_y is not None and self.get_visible():
            x, y = self.get_position()
            want = self.bottom_y - alloc.height
            if (x, y) != (self.left_x, want):
                self.move(self.left_x, want)

    # --- показ/скрытие
    def open(self, ts=None):
        if self.bottom_y is None:
            self.place()
        self.show()
        self.move(self.left_x, self.bottom_y - self.get_allocated_height())
        self.set_keep_above(True)
        if ts:
            self.present_with_time(ts)
        else:
            self.present()
        gw = self.get_window()
        if gw:
            gw.focus(ts or Gdk.CURRENT_TIME)
        self.entry.grab_focus()
        GLib.timeout_add(300, lambda: self.offer_clipboard() or False)  # даём истории буфера записать новое

    def offer_clipboard(self):
        """Только что скопированный текст или картинку — сразу вложением."""
        clip = getattr(self.app, "last_clip", None)
        if not clip or self.busy or not conf_get("auto_clip"):
            return
        if not clip.changed_at or clip.serial in self.clip_offered:
            return
        self.clip_offered.add(clip.serial)
        t = clip.changed_at
        if time.time() - t > AUTO_CLIP_FRESH:
            return
        # снимок, сделанный самим Lens, GNOME тоже кладёт в буфер — не дублируем
        if self.capture_started and t >= self.capture_started - 0.5 and time.time() - self.capture_started < 180:
            return
        kind, data = clip.read()
        if kind == "image":
            try:
                os.makedirs(SHOTS, exist_ok=True)
                dst = os.path.join(SHOTS, time.strftime("%Y-%m-%d_%H-%M-%S_clip.png"))
                data.savev(dst, "png", [], [])
                self.add_attachment("image", path=dst, label="Скопированная картинка")
            except Exception as e:
                print("auto clip:", e, file=sys.stderr)
        elif kind == "text":
            if data.strip() and not any(a.get("text") == data.strip("\n") for a in self.attachments):
                self.add_text(data, kind="quote")

    def toggle(self, ts=None):
        if self.get_visible():
            self.hide()
        else:
            self.open(ts)

    def open_and_capture(self, mode="area"):
        # панель не показываем заранее: она откроется уже со снимком
        self.capture(mode, reopen=True)

    def on_key(self, w, e):
        st = e.state
        ctrl = st & Gdk.ModifierType.CONTROL_MASK
        shift = st & Gdk.ModifierType.SHIFT_MASK
        alt = st & Gdk.ModifierType.MOD1_MASK
        if e.keyval == Gdk.KEY_Escape:
            if self.hist.get_visible():
                self.toggle_history()
            else:
                self.hide()
            return True
        if alt and not ctrl and e.hardware_keycode in KEY_DIGITS:
            acts = get_actions()
            i = KEY_DIGITS.index(e.hardware_keycode)
            if i < len(acts):
                self.run_action(acts[i])
            return True
        if ctrl and e.hardware_keycode == KEY_H:
            self.toggle_history()
            return True
        # по физической клавише, чтобы работало и в русской раскладке
        if ctrl and e.hardware_keycode == KEY_S:
            self.capture("window" if alt else "screen" if shift else "area")
            return True
        if ctrl and e.hardware_keycode == KEY_N:
            self.reset()
            return True
        if ctrl and not alt and e.hardware_keycode == KEY_V and not self.busy:
            return self.paste()
        return False

    # --- состояние
    def update_send(self):
        # во время ответа кнопка отправки превращается в «Стоп»
        if self.busy:
            self.send.set_image(Gtk.Image.new_from_icon_name("media-playback-stop-symbolic", Gtk.IconSize.BUTTON))
            self.send.set_tooltip_text("Остановить ответ")
            self.send.set_sensitive(True)
        else:
            self.send.set_image(Gtk.Image.new_from_icon_name("go-up-symbolic", Gtk.IconSize.BUTTON))
            self.send.set_tooltip_text("Отправить  (Enter)")
            self.send.set_sensitive(bool(self.entry.get_text().strip()) or bool(self.attachments))
        self.newchat.set_visible(bool(self.session) or len(self.log.get_children()) > 0)
        for b in getattr(self, "action_btns", []):
            b.set_sensitive(not self.busy)

    # --- история
    def remember(self, role, text, atts=None):
        try:
            if self.chat is None:
                self.chat = ChatHistory.new_chat()
                self.chat["title"] = ChatHistory.title_for(text, atts or [])
            msg = {"role": role, "text": text, "time": time.time()}
            if atts:
                msg["atts"] = [{k: a.get(k) for k in ("kind", "path", "text", "label", "marked")} for a in atts]
            self.chat["messages"].append(msg)
            self.chat["session"] = self.session
            ChatHistory.save(self.chat)
        except Exception as e:
            print("history:", e, file=sys.stderr)

    def toggle_history(self):
        if self.hist.get_visible():
            self.hist.hide()
            if self.log.get_children():
                self.sw.show(); self.sep.show()
            else:
                self.sep.hide()
            self.resize(BAR_W, 1)
            self.entry.grab_focus()
            return
        self.sw.hide()
        self.sep.show()
        self.hist.show()  # show_all() не действует на виджет с no_show_all
        self.hist_search.show()
        self.hist_sw.show_all()
        self.hist_search.set_text("")
        self.fill_history()
        self.hist_search.grab_focus()

    def fill_history(self):
        for r in self.hist_list.get_children():
            r.destroy()
        for c in ChatHistory.load_all():
            row = Gtk.ListBoxRow()
            row.chat = c
            row.haystack = " ".join([c.get("title", "")] + [m.get("text") or "" for m in c.get("messages", [])]).lower()
            hb = Gtk.Box(spacing=8)
            vb = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            t = Gtk.Label(label=c.get("title") or "Без названия", xalign=0)
            t.set_ellipsize(3)
            t.get_style_context().add_class("histtitle")
            n = sum(1 for m in c.get("messages", []) if m.get("role") == "user")
            meta = Gtk.Label(label="%s · %d %s" % (ChatHistory.fmt_date(c.get("updated", 0)), n,
                                                    "вопрос" if n % 10 == 1 and n % 100 != 11 else
                                                    "вопроса" if 2 <= n % 10 <= 4 and not 12 <= n % 100 <= 14
                                                    else "вопросов"), xalign=0)
            meta.get_style_context().add_class("histmeta")
            vb.pack_start(t, False, False, 0)
            vb.pack_start(meta, False, False, 0)
            hb.pack_start(vb, True, True, 0)
            dl = Gtk.Button.new_from_icon_name("user-trash-symbolic", Gtk.IconSize.BUTTON)
            dl.get_style_context().add_class("ghost")
            dl.get_style_context().add_class("histdel")
            dl.set_tooltip_text("Удалить чат")
            dl.set_valign(Gtk.Align.CENTER)
            dl.connect("clicked", self.on_hist_delete, row)
            hb.pack_end(dl, False, False, 0)
            row.add(hb)
            self.hist_list.add(row)
        self.hist_list.show_all()
        self.update_hist_empty()

    def hist_filter(self, row):
        q = self.hist_search.get_text().strip().lower()
        return not q or all(w in row.haystack for w in q.split())

    def update_hist_empty(self):
        rows = self.hist_list.get_children()
        visible = [r for r in rows if self.hist_filter(r)]
        self.hist_empty.set_text("История пуста" if not rows else "Ничего не найдено")
        self.hist_empty.set_visible(not visible)
        self.hist_sw.set_visible(bool(visible))
        GLib.idle_add(lambda: self.resize(BAR_W, 1) or False)

    def on_hist_activate_first(self, *a):
        for r in self.hist_list.get_children():
            if self.hist_filter(r):
                self.open_chat(r.chat)
                return

    def on_hist_delete(self, btn, row):
        # первый клик — «Удалить?», второй — удалить
        ctx = btn.get_style_context()
        if not ctx.has_class("confirm"):
            ctx.add_class("confirm")
            btn.set_image(None)
            btn.set_label("Удалить?")
            GLib.timeout_add(3000, lambda: self.unconfirm(btn))
            return
        ChatHistory.delete(row.chat["id"])
        if self.chat and self.chat["id"] == row.chat["id"]:
            self.chat = None
            self.session = None
        row.destroy()
        self.update_hist_empty()

    @staticmethod
    def unconfirm(btn):
        try:
            if btn.get_style_context().has_class("confirm"):
                btn.get_style_context().remove_class("confirm")
                btn.set_label("")
                btn.set_image(Gtk.Image.new_from_icon_name("user-trash-symbolic", Gtk.IconSize.BUTTON))
        except Exception:
            pass
        return False

    def open_chat(self, chat):
        if self.busy:
            return
        self.reset()
        try:
            with open(ChatHistory.path(chat["id"]), encoding="utf-8") as f:
                chat = json.load(f)
        except Exception:
            pass
        self.chat = chat
        self.session = chat.get("session")
        for m in chat.get("messages", []):
            if m.get("role") == "user":
                self.add_msg(m.get("text") or "", mine=True, atts=m.get("atts") or [])
            else:
                self.add_msg(m.get("text") or "")
                self.last_answer = m.get("text") or ""
        if self.last_answer:
            self.copy_btn.show()
        self.update_send()
        self.entry.grab_focus()
        GLib.timeout_add(120, self.scroll_down)

    def set_image(self, path, marked=False):
        """Скриншот: path=None очищает все вложения."""
        if path:
            self.add_attachment("image", path=path, marked=marked,
                                label="Скриншот с разметкой" if marked else "Скриншот")
        else:
            self.clear_attachments()

    # --- вложения
    ICONS = {"pdf": "application-pdf", "file": "text-x-generic", "quote": "format-justify-left-symbolic",
             "text": "edit-paste-symbolic"}

    def add_attachment(self, kind, path=None, text=None, label="", marked=False):
        att = {"kind": kind, "path": path, "text": text, "label": label, "marked": marked}
        chip = Gtk.Box(spacing=6)
        chip.set_halign(Gtk.Align.START)
        chip.get_style_context().add_class("chip")
        icon = None
        if kind == "image":
            try:
                icon = Gtk.Image.new_from_pixbuf(GdkPixbuf.Pixbuf.new_from_file_at_scale(path, 48, 32, True))
            except Exception:
                pass
        if icon is None:
            icon = Gtk.Image.new_from_icon_name(self.ICONS.get(kind, "image-x-generic"), Gtk.IconSize.LARGE_TOOLBAR)
        lbl = Gtk.Label(label=label, xalign=0)
        lbl.set_ellipsize(3)  # END
        lbl.set_max_width_chars(26)
        chip.set_tooltip_text(path or (text[:600] if text else label))
        rm = Gtk.Button.new_from_icon_name("window-close-symbolic", Gtk.IconSize.MENU)
        rm.get_style_context().add_class("ghost")
        rm.get_style_context().add_class("chipx")
        rm.set_tooltip_text("Убрать")
        rm.connect("clicked", lambda *a: self.remove_attachment(att))
        chip.pack_start(icon, False, False, 0)
        chip.pack_start(lbl, False, False, 0)
        chip.pack_start(rm, False, False, 0)
        self.chips.add(chip)
        att["widget"] = chip.get_parent()  # FlowBoxChild
        self.attachments.append(att)
        att["widget"].show_all()
        self.chips.show()  # show_all() не действует на виджет с no_show_all
        self.update_send()
        return att

    def remove_attachment(self, att):
        if att in self.attachments:
            self.attachments.remove(att)
            att["widget"].destroy()
        if not self.attachments:
            self.chips.hide()
            self.resize(BAR_W, 1)
        self.update_send()

    def clear_attachments(self):
        for att in list(self.attachments):
            att["widget"].destroy()
        self.attachments.clear()
        self.chips.hide()
        self.update_send()

    def add_file(self, path):
        """Файл с диска: картинка, PDF или текст/код. Копируется в папку снимков (files)."""
        name = os.path.basename(path)
        try:
            if not os.path.isfile(path):
                raise ValueError("это не файл")
            if os.path.getsize(path) > MAX_FILE:
                raise ValueError("файл больше 25 МБ")
            ext = os.path.splitext(name)[1].lower()
            ctype = Gio.content_type_guess(path, None)[0] or ""
            if ext in IMAGE_EXT or ctype.startswith("image/"):
                kind = "image"
            elif ext == ".pdf" or ctype == "application/pdf":
                kind = "pdf"
            elif Gio.content_type_is_a(ctype, "text/plain") or ctype in TEXT_TYPES or self.looks_like_text(path):
                kind = "file"
            else:
                raise ValueError("Claude умеет читать картинки, PDF и текстовые файлы")
            os.makedirs(FILES, exist_ok=True)
            dst = os.path.join(FILES, time.strftime("%Y-%m-%d_%H-%M-%S_") + name)
            shutil.copy2(path, dst)
            self.add_attachment(kind, path=dst, label=name)
        except Exception as e:
            ui.notify(APP_NAME, "Не могу прикрепить «%s»: %s" % (name, e), "dialog-warning")

    @staticmethod
    def looks_like_text(path):
        try:
            with open(path, "rb") as f:
                chunk = f.read(4096)
            chunk.decode("utf-8")
            return b"\0" not in chunk
        except Exception:
            return False

    @staticmethod
    def text_label(text, kind):
        first = " ".join(text.split())
        if kind == "quote":
            return "«%s»" % (first[:40] + ("…" if len(first) > 40 else ""))
        n = text.count("\n") + 1
        return "Текст: %d %s" % (n, "строка" if n % 10 == 1 and n % 100 != 11 else
                                 "строки" if 2 <= n % 10 <= 4 and not 12 <= n % 100 <= 14 else "строк")

    def add_text(self, text, kind="text"):
        text = text.strip("\n")
        if not text.strip():
            return
        self.add_attachment(kind, text=text, label=self.text_label(text, kind))

    # --- быстрые действия
    def rebuild_actions(self):
        for c in self.actions_box.get_children():
            c.destroy()
        self.action_btns = []
        for i, act in enumerate(get_actions()):
            b = Gtk.Button(label=act["name"])
            b.get_style_context().add_class("action")
            b.set_tooltip_text(act["prompt"] + ("\n\nAlt+%d" % (i + 1) if i < 9 else ""))
            b.connect("clicked", lambda _b, a=act: self.run_action(a))
            self.actions_box.add(b)
            self.action_btns.append(b)
        self.actions_box.show_all()
        self.actions_box.set_visible(bool(self.action_btns))
        self.update_send()

    def run_action(self, act):
        if self.busy:
            return
        if not self.attachments and not self.entry.get_text().strip():
            self.status.set_text("Сначала прикрепите скриншот, файл или текст — или напишите текст в поле.")
            self.status.show()
            GLib.timeout_add(2500, lambda: (not self.busy and self.status.hide()) or False)
            return
        self.pending_action = act
        self.on_send()

    def paste(self):
        """Ctrl+V: картинка или файлы — вложением, длинный текст — чипом, короткий — в поле ввода."""
        cb = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        if cb.wait_is_image_available():
            pb = cb.wait_for_image()
            if pb:
                os.makedirs(SHOTS, exist_ok=True)
                dst = os.path.join(SHOTS, time.strftime("%Y-%m-%d_%H-%M-%S_paste.png"))
                pb.savev(dst, "png", [], [])
                self.add_attachment("image", path=dst, label="Картинка из буфера")
                return True
        if cb.wait_is_uris_available():
            uris = cb.wait_for_uris() or []
            files = [Gio.File.new_for_uri(u).get_path() for u in uris]
            files = [f for f in files if f]
            if files:
                for f in files:
                    self.add_file(f)
                return True
        text = cb.wait_for_text()
        if text and ("\n" in text.strip() or len(text) > 300):
            self.add_text(text)
            return True
        return False  # короткий текст вставит само поле ввода

    def on_drop(self, w, ctx, x, y, data, info, ts):
        self.outer.get_style_context().remove_class("dragover")
        uris = data.get_uris()
        if uris:
            for u in uris:
                f = Gio.File.new_for_uri(u)
                if f.get_path():
                    self.add_file(f.get_path())
        else:
            text = data.get_text()
            if text:
                if "\n" in text.strip() or len(text) > 300:
                    self.add_text(text)
                else:
                    self.entry.insert_text(text, -1)
        self.entry.grab_focus()

    def reset(self):
        if self.busy:
            return
        self.session = None
        self.chat = None
        self.last_answer = ""
        self.md_views.clear()
        if self.hist.get_visible():
            self.hist.hide()
        for c in self.log.get_children():
            c.destroy()
        self.sw.hide(); self.sep.hide(); self.copy_btn.hide(); self.status.hide()
        self.set_image(None)
        self.entry.set_text("")
        self.resize(BAR_W, 1)
        self.update_send()
        self.entry.grab_focus()

    def set_busy(self, v, text=""):
        was = self.busy
        self.busy = v
        self.entry.set_sensitive(not v)
        self.status.set_text(text)
        self.status.set_visible(v)
        self.app.bubble.set_busy(v)
        if v and not was:
            GLib.timeout_add(40, self.tick)
        self.logo.queue_draw()
        self.update_send()

    def add_msg(self, text, mine=False, image=None, final=True, atts=None):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        atts = list(atts or [])
        if image:
            atts.insert(0, {"kind": "image", "path": image})
        imgs = [a for a in atts if a["kind"] == "image"]
        if imgs:
            row = Gtk.Box(spacing=8)
            for a in imgs[:4]:
                try:
                    pb = GdkPixbuf.Pixbuf.new_from_file_at_scale(a["path"], 300 if len(imgs) == 1 else 140, 120, True)
                    ev = Gtk.EventBox()
                    ev.add(Gtk.Image.new_from_pixbuf(pb))
                    ev.set_tooltip_text("Открыть картинку")
                    ev.connect("button-press-event", lambda *x, u="file://" + a["path"]:
                               Gio.AppInfo.launch_default_for_uri(u, None))
                    row.pack_start(ev, False, False, 0)
                except Exception:
                    pass
            box.pack_start(row, False, False, 0)
        others = [a for a in atts if a["kind"] != "image"]
        if others:
            lines = []
            for a in others:
                if a["kind"] in ("quote", "text"):
                    t = " ".join(a["text"].split())
                    lines.append("❝ " + (t[:90] + "…" if len(t) > 90 else t))
                else:
                    lines.append("📄 " + a["label"])
            al = Gtk.Label(label="\n".join(lines), xalign=0)
            al.set_line_wrap(True)
            al.set_line_wrap_mode(2)
            al.get_style_context().add_class("atts")
            box.pack_start(al, False, False, 0)
        if mine:
            lbl = Gtk.Label()
            lbl.set_line_wrap(True)
            lbl.set_line_wrap_mode(2)
            lbl.set_selectable(True)
            lbl.set_can_focus(False)
            lbl.set_xalign(0)
            lbl.set_max_width_chars(70)
            lbl.get_style_context().add_class("mine")
            lbl.set_text(text)
        else:
            # ответ Claude — контейнер, в который md_render раскладывает блоки Markdown
            lbl = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
            lbl.get_style_context().add_class("answer")
            self.set_label_md(lbl, text)
        if text or (not mine and not final):
            box.pack_start(lbl, False, False, 0)
        if text and not mine and final:
            self.add_copy_btn(box, text)
        self.log.pack_start(box, False, False, 0)
        box.show_all()
        self.sw.show(); self.sep.show()
        self.sw.get_child().show_all()
        GLib.timeout_add(60, self.scroll_down)
        return box, lbl

    def add_copy_btn(self, box, text):
        if True:
            cbtn = Gtk.Button()
            cbox = Gtk.Box(spacing=6)
            cbox.pack_start(Gtk.Image.new_from_icon_name("edit-copy-symbolic", Gtk.IconSize.MENU), False, False, 0)
            clbl = Gtk.Label(label="Копировать")
            cbox.pack_start(clbl, False, False, 0)
            cbtn.add(cbox)
            cbtn.get_style_context().add_class("ghost")
            cbtn.get_style_context().add_class("copy")
            cbtn.set_halign(Gtk.Align.START)
            cbtn.set_tooltip_text("Скопировать этот ответ в буфер обмена")
            cbtn.connect("clicked", self.copy_text, text, clbl)
            box.pack_start(cbtn, False, False, 0)
            cbtn.show_all()

    def copy_text(self, btn, text, lbl):
        md_render.copy_to_clipboard(text)
        lbl.set_text("Скопировано ✓")
        GLib.timeout_add(1500, lambda: lbl.set_text("Копировать") or False)

    def scroll_down(self):
        adj = self.sw.get_vadjustment()
        adj.set_value(adj.get_upper())
        return False

    # --- отправка
    def on_send(self, *a):
        if self.busy:
            if a and isinstance(a[0], Gtk.Button):
                self.stop()
            return
        q = self.entry.get_text().strip()
        atts = [dict(a) for a in self.attachments]
        act, self.pending_action = self.pending_action, None
        shown = q
        if act:
            if not atts and q:  # действие над текстом из поля ввода
                atts = [{"kind": "text", "text": q, "label": self.text_label(q, "text"), "path": None}]
                q = ""
            shown = "⚡ " + act["name"] + (": " + q if q else "")
            q = act["prompt"] + ("\n\nДополнительно от пользователя: " + q if q else "")
        if not q and not atts:
            return
        prompt = self.build_prompt(q, atts)
        sp = (conf_get("system_prompt") or "").strip()
        if sp:
            # дублируем в сообщении: иначе правила Claude Code (например, «отвечай на языке вопроса») перевешивают
            prompt += "\n\n[Указания пользователя для всех ответов: %s]" % sp
        prompt += "\n\n" + BRIEF
        if self.hist.get_visible():
            self.toggle_history()
        self.add_msg(shown, mine=True, atts=atts)
        self.remember("user", shown, atts)
        self.entry.set_text("")
        self.clear_attachments()
        kinds = {a["kind"] for a in atts}
        self.set_busy(True, "Claude смотрит на картинку…" if "image" in kinds else
                      "Claude читает файл…" if kinds & {"pdf", "file"} else "Claude думает…")
        self.stream_box = self.stream_lbl = None
        self.stream_text = ""
        self.render_pending = False
        self.stopped = False
        self.proc = None
        threading.Thread(target=self.run_claude, args=(prompt,), daemon=True).start()

    KIND_NAMES = {"image": "картинка/скриншот", "pdf": "PDF", "file": "текстовый файл"}

    def build_prompt(self, q, atts):
        if not atts:
            return q
        parts = []
        for a in atts:
            if a["kind"] == "quote":
                parts.append("Выделенный пользователем текст:\n<<<\n%s\n>>>" % a["text"])
            elif a["kind"] == "text":
                parts.append("Вставленный пользователем текст:\n<<<\n%s\n>>>" % a["text"])
        files = [a for a in atts if a["kind"] in ("image", "pdf", "file")]
        if files:
            parts.append("Прикреплённые файлы (открой их инструментом Read):\n" + "\n".join(
                "- %s — %s%s" % (a["path"], self.KIND_NAMES[a["kind"]],
                                 ", с разметкой пользователя" if a.get("marked") else "") for a in files))
        if any(a.get("marked") for a in atts):
            parts.append("Пользователь сам отметил на скриншоте важное стрелками, рамками, маркером или "
                         "подписями — в первую очередь смотри на отмеченные места.")
        kinds = {a["kind"] for a in atts}
        if q:
            parts.append("Вопрос: " + q)
        elif kinds == {"image"}:
            parts.append("Скажи, что на изображении и что здесь главное. Если там текст на другом языке — "
                         "переведи. В интернете ищи, только если без этого не ответить.")
        elif kinds <= {"quote", "text"}:
            parts.append("Объясни суть этого текста. Если он на другом языке — переведи. "
                         "Если это код или ошибка — скажи причину и как исправить.")
        else:
            parts.append("Скажи, что это и что в нём главное.")
        return "\n\n".join(parts)

    # --- потоковый ответ
    TOOL_STATUS = {"WebSearch": "Ищет в интернете…", "WebFetch": "Читает страницу…",
                   "Read": "Смотрит на вложение…"}

    def run_claude(self, prompt):
        cmd = [CLAUDE, "-p", prompt, "--output-format", "stream-json", "--verbose",
               "--include-partial-messages",
               "--allowedTools", "Read", "WebSearch", "WebFetch",
               "--add-dir", SHOTS]
        model = (conf_get("model") or "").strip()
        if model:
            cmd += ["--model", model]
        sp = (conf_get("system_prompt") or "").strip()
        if sp:
            cmd += ["--append-system-prompt", sp]
        if self.session:
            cmd += ["--resume", self.session]
        result, is_error, err = None, False, ""
        try:
            os.makedirs(WORKDIR, exist_ok=True)
            self.proc = subprocess.Popen(cmd, cwd=WORKDIR, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                         stdin=subprocess.DEVNULL, text=True, bufsize=1)
            timer = threading.Timer(300, self.proc.kill)
            timer.start()
            for line in self.proc.stdout:
                try:
                    d = json.loads(line)
                except Exception:
                    continue
                t = d.get("type")
                if d.get("session_id"):
                    self.session = d["session_id"]
                if t == "stream_event":
                    ev = d.get("event", {})
                    et = ev.get("type")
                    if et == "content_block_delta" and ev.get("delta", {}).get("type") == "text_delta":
                        GLib.idle_add(self.on_delta, ev["delta"].get("text", ""))
                    elif et == "content_block_start" and ev.get("content_block", {}).get("type") == "tool_use":
                        GLib.idle_add(self.on_tool, ev["content_block"].get("name", ""))
                elif t == "result":
                    result = d.get("result")
                    is_error = bool(d.get("is_error"))
            self.proc.wait()
            timer.cancel()
            err = self.proc.stderr.read()
        except Exception as e:
            err = str(e)
        GLib.idle_add(self.finish_answer, result, is_error, err)

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.stopped = True
            self.status.set_text("Останавливаю…")
            self.status.show()
            try:
                self.proc.terminate()
            except Exception:
                pass

    def on_delta(self, chunk):
        if self.stopped:
            return False
        if self.stream_box is None:
            self.stream_box, self.stream_lbl = self.add_msg("", final=False)
        self.status.hide()  # ответ пошёл — индикатор «думает…» больше не нужен
        self.stream_text += chunk
        if not self.render_pending:
            self.render_pending = True
            GLib.timeout_add(120, self.render_stream)
        return False

    def render_stream(self):
        self.render_pending = False
        if not self.busy:  # ответ уже завершён — итоговый текст не затираем
            return False
        if self.stream_lbl:
            self.set_label_md(self.stream_lbl, self.stream_text)
            GLib.idle_add(self.scroll_down)
        return False

    def on_tool(self, name):
        # текст «сейчас поищу…» перед вызовом инструмента в итоговый ответ не входит — убираем
        self.stream_text = ""
        if self.stream_lbl:
            self.set_label_md(self.stream_lbl, "")
        self.status.set_text(self.TOOL_STATUS.get(name, "Работает…"))
        self.status.show()
        return False

    def set_label_md(self, box, text):
        self.md_views[box] = text
        try:
            md_render.render(box, text, md_render.PALETTES[self.app.theme])
        except Exception as e:
            for c in box.get_children():
                c.destroy()
            lbl = Gtk.Label(label=text, xalign=0)
            lbl.set_line_wrap(True)
            lbl.set_selectable(True)
            box.pack_start(lbl, False, False, 0)
            box.show_all()
            print("md_render:", e, file=sys.stderr)

    def apply_theme(self):
        ctx = self.outer.get_style_context()
        (ctx.add_class if self.app.theme == "light" else ctx.remove_class)("light")
        for box, text in list(self.md_views.items()):
            self.set_label_md(box, text)

    def finish_answer(self, result, is_error, err):
        if self.stopped:
            text = (self.stream_text.rstrip() + "\n\n*(остановлено)*") if self.stream_text.strip() else "*(остановлено)*"
        elif result is not None:
            text = ("⚠️ " if is_error else "") + (result or "(пустой ответ)")
        elif self.stream_text.strip():
            text = self.stream_text
        else:
            text = "⚠️ Ошибка Claude:\n" + (err or "нет вывода").strip()[-1500:]
        self.proc = None
        self.set_busy(False)
        if self.stream_box is None:
            self.stream_box, self.stream_lbl = self.add_msg("", final=False)
        self.set_label_md(self.stream_lbl, text)
        self.add_copy_btn(self.stream_box, text)
        self.remember("assistant", text)
        GLib.idle_add(lambda: self.resize(BAR_W, 1) or False)  # подогнать высоту под содержимое
        self.last_answer = text
        self.copy_btn.show()
        self.entry.grab_focus()
        GLib.timeout_add(60, self.scroll_down)
        return False

    # --- скриншот через портал GNOME
    def capture(self, mode="area", reopen=False):
        if self.app.capturing or self.busy:
            return
        self.capture_started = time.time()
        self.app.capturing = True
        self.was_visible = self.get_visible() or reopen
        self.hide()
        self.app.bubble.hide()
        GLib.timeout_add(250, self.app.screenshot, mode, self.on_shot)

    def on_shot(self, path, err):
        self.app.capturing = False
        if conf_get("bubble"):
            self.app.bubble.show_all()
            self.app.bubble.set_keep_above(True)
        if err:
            ui.notify(APP_NAME, err)
        if path:
            try:
                AnnotateWindow(self.app, path, self.after_annotate)
                return
            except Exception as e:
                ui.notify(APP_NAME, "Редактор разметки не открылся: %s" % e)
                self.set_image(path)
        if path or self.was_visible:
            self.open()

    def after_annotate(self, path, marked):
        self.set_image(path, marked)
        self.open()


class QuietPopup(Gtk.Window):
    """Тихий режим: маленькое окно ответа в углу, которое никогда не берёт фокус клавиатуры.

    Это окно override-redirect (Gtk.WindowType.POPUP): GNOME им не управляет и не переключает на него фокус,
    поэтому окно, где вы смотрите видео, остаётся активным и не получает «потерю фокуса».
    """
    W = 400

    def __init__(self, app):
        super().__init__(type=Gtk.WindowType.POPUP)
        self.app = app
        self.set_application(app)
        self.set_accept_focus(False)
        self.set_type_hint(Gdk.WindowTypeHint.NOTIFICATION)
        rgba_window(self)
        self.set_opacity(0.95)
        self.connect("draw", lambda w, cr: (cr.set_operator(0), cr.paint(), cr.set_operator(2)) and False)
        self.connect("size-allocate", self.on_alloc)
        self.session = None
        self.proc = None
        self.busy = False
        self.chat = None

        self.outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.outer.get_style_context().add_class("bar")
        self.outer.set_size_request(self.W, -1)
        self.add(self.outer)
        head = Gtk.Box(spacing=6)
        head.set_margin_start(12); head.set_margin_end(6); head.set_margin_top(6); head.set_margin_bottom(4)
        self.logo = Gtk.DrawingArea()
        self.logo.set_size_request(18, 18)
        self.logo.connect("draw", lambda w, cr: draw_logo(cr, 9, 9, 8, rgb=ACCENT) or False)
        self.title = Gtk.Label(label="Claude", xalign=0)
        self.title.get_style_context().add_class("status")
        head.pack_start(self.logo, False, False, 0)
        head.pack_start(self.title, True, True, 0)
        self.spinner = Gtk.Spinner()
        head.pack_start(self.spinner, False, False, 0)
        self.btns = []
        for icon, tip, cb in (("window-close-symbolic", "Закрыть", lambda *a: self.close_popup()),
                              ("view-restore-symbolic", "Открыть в панели и продолжить разговор", self.to_panel),
                              ("edit-copy-symbolic", "Скопировать ответ", self.copy)):
            b = Gtk.Button.new_from_icon_name(icon, Gtk.IconSize.MENU)
            b.get_style_context().add_class("ghost")
            b.set_tooltip_text(tip)
            b.set_can_focus(False)
            b.connect("clicked", cb)
            head.pack_end(b, False, False, 0)
            self.btns.append(b)
        self.outer.pack_start(head, False, False, 0)
        self.sw = Gtk.ScrolledWindow()
        self.sw.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.sw.set_propagate_natural_height(True)
        self.sw.set_max_content_height(340)
        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.body.get_style_context().add_class("answer")
        self.body.set_margin_start(14); self.body.set_margin_end(14); self.body.set_margin_bottom(12)
        self.sw.add(self.body)
        self.outer.pack_start(self.sw, True, True, 0)
        self.outer.show_all()

    # --- позиция: правый нижний угол, растёт вверх
    def on_alloc(self, w, a):
        geo = Gdk.Display.get_default().get_monitor(0).get_workarea()
        self.move(geo.x + geo.width - self.W - 24, geo.y + geo.height - a.height - 24)

    def apply_theme(self):
        ctx = self.outer.get_style_context()
        (ctx.add_class if self.app.theme == "light" else ctx.remove_class)("light")

    def set_text(self, text):
        md_render.render(self.body, text, md_render.PALETTES[self.app.theme])
        # строки md_render выделяемые — но выделение требует фокуса; в тихом окне отключаем
        for w in self._labels(self.body):
            w.set_selectable(False)
        # всплывающее окно берёт минимальную высоту прокрутки — задаём её по высоте текста (до 340 px)
        h = self.body.get_preferred_height_for_width(self.W - 28)[1] if text else 0
        self.sw.set_min_content_height(min(h, 340))
        self.sw.set_visible(bool(text))
        self.resize(self.W, 1)

    def _labels(self, w):
        if isinstance(w, Gtk.Label):
            yield w
        elif isinstance(w, Gtk.Container):
            for c in w.get_children():
                yield from self._labels(c)

    def start(self, path):
        self.apply_theme()
        self.chat = ChatHistory.new_chat()
        self.session = None
        self.stream_text = ""
        self.last_answer = ""
        self.render_pending = False
        self.stopped = False
        att = {"kind": "image", "path": path, "label": "Экран", "text": None, "marked": False}
        shown = "🤫 Тихий вопрос по экрану"
        self.chat["title"] = shown
        self.chat["messages"].append({"role": "user", "text": shown, "time": time.time(), "atts": [att]})
        prompt = QuickBar.build_prompt(self.app.bar, "", [att])
        sp = (conf_get("system_prompt") or "").strip()
        if sp:
            prompt += "\n\n[Указания пользователя для всех ответов: %s]" % sp
        prompt += "\n\n" + QUIET_BRIEF
        self.set_text("")
        self.title.set_text("Claude смотрит на экран…")
        self.busy = True
        self.spinner.start()
        self.app.bubble.set_busy(True)
        self.resize(self.W, 1)
        self.show_all()
        threading.Thread(target=QuickBar.run_claude, args=(self, prompt), daemon=True).start()

    # --- интерфейс, который ждёт QuickBar.run_claude
    def on_delta(self, chunk):
        if self.stream_text == "":
            self.title.set_text("Claude")
        self.stream_text += chunk
        if not self.render_pending:
            self.render_pending = True
            GLib.timeout_add(150, self.render_stream)
        return False

    def render_stream(self):
        self.render_pending = False
        if not self.busy:
            return False
        self.set_text(self.stream_text)
        adj = self.sw.get_vadjustment()
        GLib.idle_add(lambda: adj.set_value(adj.get_upper()) or False)
        return False

    def on_tool(self, name):
        self.stream_text = ""
        self.set_text("")
        self.title.set_text(QuickBar.TOOL_STATUS.get(name, "Работает…"))
        return False

    def finish_answer(self, result, is_error, err):
        if result is not None:
            text = ("⚠️ " if is_error else "") + (result or "(пустой ответ)")
        elif self.stream_text.strip():
            text = self.stream_text
        else:
            text = "⚠️ Ошибка Claude:\n" + (err or "нет вывода").strip()[-800:]
        self.busy = False
        self.proc = None
        self.spinner.stop()
        self.app.bubble.set_busy(False)
        self.title.set_text("Claude")
        self.last_answer = text
        self.set_text(text)
        self.resize(self.W, 1)
        try:
            self.chat["messages"].append({"role": "assistant", "text": text, "time": time.time()})
            self.chat["session"] = self.session
            ChatHistory.save(self.chat)
        except Exception as e:
            print("history:", e, file=sys.stderr)
        return False

    # --- кнопки
    def copy(self, *a):
        if self.last_answer:
            md_render.copy_to_clipboard(self.last_answer)
            self.title.set_text("Скопировано ✓")
            GLib.timeout_add(1500, lambda: self.title.set_text("Claude") or False)

    def to_panel(self, *a):
        if self.busy or not self.chat:
            return
        chat = self.chat
        self.close_popup()
        self.app.bar.open_chat(chat)
        self.app.bar.open()

    def close_popup(self):
        if self.busy and self.proc and self.proc.poll() is None:
            try:
                self.proc.terminate()
            except Exception:
                pass
        self.hide()


class AnnotateWindow(Gtk.Window):
    """Редактор разметки поверх снимка: стрелка, прямоугольник, маркер, текст, 3 цвета, отмена."""
    COLORS = {"red": ((0.93, 0.26, 0.21), "Красный"),
              "yellow": ((0.98, 0.80, 0.10), "Жёлтый"),
              "green": ((0.20, 0.78, 0.35), "Зелёный")}
    TOOLS = (("arrow", "Стрелка", "1"), ("rect", "Прямоугольник", "2"),
             ("marker", "Маркер", "3"), ("text", "Текст", "4"))

    def __init__(self, app, path, on_done):
        super().__init__(title="Lens — разметка")
        self.app = app
        self.set_application(app)
        self.path = path
        self.on_done = on_done
        self.finished = False
        self.set_keep_above(True)
        self.set_type_hint(Gdk.WindowTypeHint.DIALOG)
        self.get_style_context().add_class("lens-editor")
        self.connect("delete-event", lambda *a: self.finish(False) or True)
        self.connect("key-press-event", self.on_key)

        self.pix = GdkPixbuf.Pixbuf.new_from_file(path)
        self.iw, self.ih = self.pix.get_width(), self.pix.get_height()
        geo = Gdk.Display.get_default().get_monitor(0).get_workarea()
        self.scale = min(1.0, geo.width * 0.85 / self.iw, (geo.height * 0.85 - 70) / self.ih)
        self.vw, self.vh = max(1, int(self.iw * self.scale)), max(1, int(self.ih * self.scale))
        self.lw = max(3.0, max(self.iw, self.ih) / 350.0)  # толщина линий в пикселях снимка

        self.shapes = []
        self.cur = None
        self.tool = "arrow"
        self.color = "red"
        self.text_entry = None

        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        # панель инструментов
        bar = Gtk.Box(spacing=6)
        bar.get_style_context().add_class("ed-bar")
        self.tool_btns = {}
        group = None
        for key, label, num in self.TOOLS:
            b = Gtk.RadioButton.new_with_label_from_widget(group, label)
            group = group or b
            b.set_mode(False)
            b.set_tooltip_text("%s  (%s)" % (label, num))
            b.connect("toggled", self.on_tool, key)
            bar.pack_start(b, False, False, 0)
            self.tool_btns[key] = b
        bar.pack_start(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL), False, False, 4)
        self.color_btns = {}
        group = None
        for key, (rgb, label) in self.COLORS.items():
            b = Gtk.RadioButton.new_from_widget(group)
            group = group or b
            b.set_mode(False)
            da = Gtk.DrawingArea()
            da.set_size_request(18, 18)
            da.connect("draw", self.draw_swatch, rgb)
            b.add(da)
            b.set_tooltip_text(label)
            b.get_style_context().add_class("swatch")
            b.connect("toggled", self.on_color, key)
            bar.pack_start(b, False, False, 0)
            self.color_btns[key] = b
        bar.pack_start(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL), False, False, 4)
        self.undo_btn = Gtk.Button.new_from_icon_name("edit-undo-symbolic", Gtk.IconSize.BUTTON)
        self.undo_btn.set_tooltip_text("Отменить  (Ctrl+Z)")
        self.undo_btn.connect("clicked", lambda *a: self.undo())
        bar.pack_start(self.undo_btn, False, False, 0)
        done = Gtk.Button(label="Готово")
        done.get_style_context().add_class("suggested-action")
        done.set_tooltip_text("Прикрепить снимок с разметкой  (Enter)")
        done.connect("clicked", lambda *a: self.finish(True))
        skip = Gtk.Button(label="Пропустить")
        skip.set_tooltip_text("Прикрепить снимок как есть  (Esc)")
        skip.connect("clicked", lambda *a: self.finish(False))
        bar.pack_end(done, False, False, 0)
        bar.pack_end(skip, False, False, 0)
        root.pack_start(bar, False, False, 0)

        # холст
        self.overlay = Gtk.Overlay()
        self.canvas = Gtk.DrawingArea()
        self.canvas.set_size_request(self.vw, self.vh)
        self.canvas.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK |
                               Gdk.EventMask.POINTER_MOTION_MASK)
        self.canvas.connect("draw", self.on_draw)
        self.canvas.connect("button-press-event", self.on_press)
        self.canvas.connect("motion-notify-event", self.on_motion)
        self.canvas.connect("button-release-event", self.on_release)
        self.canvas.connect("realize", lambda w: w.get_window().set_cursor(
            Gdk.Cursor.new_from_name(w.get_display(), "crosshair")))
        self.overlay.add(self.canvas)
        self.fixed = Gtk.Fixed()
        self.fixed.set_halign(Gtk.Align.START)
        self.fixed.set_valign(Gtk.Align.START)
        self.overlay.add_overlay(self.fixed)
        self.overlay.set_overlay_pass_through(self.fixed, True)
        root.pack_start(self.overlay, True, True, 0)

        self.update_undo()
        self.set_resizable(False)
        self.show_all()
        self.present()

    # --- инструменты
    def on_tool(self, btn, key):
        if btn.get_active():
            self.commit_text()
            self.tool = key

    def on_color(self, btn, key):
        if btn.get_active():
            self.color = key
            if self.text_entry:
                self.text_entry.grab_focus()

    def draw_swatch(self, w, cr, rgb):
        cr.set_source_rgb(*rgb)
        cr.arc(9, 9, 8, 0, 2 * math.pi)
        cr.fill()
        return False

    def update_undo(self):
        self.undo_btn.set_sensitive(bool(self.shapes))

    def undo(self):
        if self.text_entry:
            self.cancel_text()
            return
        if self.shapes:
            self.shapes.pop()
            self.update_undo()
            self.canvas.queue_draw()

    def on_key(self, w, e):
        ctrl = e.state & Gdk.ModifierType.CONTROL_MASK
        if self.text_entry and self.text_entry.has_focus():
            if e.keyval == Gdk.KEY_Escape:
                self.cancel_text()
                return True
            return False
        if ctrl and e.hardware_keycode == KEY_Z:
            self.undo()
            return True
        if e.keyval == Gdk.KEY_Escape:
            self.finish(False)
            return True
        if e.keyval in (Gdk.KEY_Return, Gdk.KEY_KP_Enter):
            self.finish(True)
            return True
        for i, (key, _l, _n) in enumerate(self.TOOLS):
            if e.keyval == Gdk.KEY_1 + i:
                self.tool_btns[key].set_active(True)
                return True
        return False

    # --- мышь (координаты переводим в пиксели снимка)
    def img_xy(self, e):
        return (min(max(e.x, 0), self.vw) / self.scale, min(max(e.y, 0), self.vh) / self.scale)

    def on_press(self, w, e):
        if e.button != 1:
            return False
        x, y = self.img_xy(e)
        if self.tool == "text":
            self.commit_text()
            self.start_text(e.x, e.y, x, y)
            return True
        self.commit_text()
        self.cur = {"t": self.tool, "c": self.COLORS[self.color][0], "p": [(x, y), (x, y)]}
        return True

    def on_motion(self, w, e):
        if not self.cur:
            return False
        p = self.img_xy(e)
        if self.cur["t"] == "marker":
            self.cur["p"].append(p)
        else:
            self.cur["p"][1] = p
        self.canvas.queue_draw()
        return True

    def on_release(self, w, e):
        if not self.cur:
            return False
        (x0, y0), (x1, y1) = self.cur["p"][0], self.cur["p"][-1]
        big = abs(x1 - x0) + abs(y1 - y0) > 4 / self.scale or len(self.cur["p"]) > 3
        if big:
            self.shapes.append(self.cur)
            self.update_undo()
        self.cur = None
        self.canvas.queue_draw()
        return True

    # --- текст
    def start_text(self, vx, vy, ix, iy):
        ent = Gtk.Entry()
        ent.set_placeholder_text("Текст… (Enter)")
        ent.set_width_chars(18)
        ent.connect("activate", lambda *a: self.commit_text())
        ent.connect("focus-out-event", lambda *a: GLib.idle_add(self.commit_text) and False)
        self.fixed.put(ent, int(vx), int(max(0, vy - 16)))
        self.text_entry = ent
        self.text_pos = (ix, iy)
        ent.show()
        ent.grab_focus()

    def commit_text(self):
        ent = self.text_entry
        if not ent:
            return False
        self.text_entry = None
        txt = ent.get_text().strip()
        ent.destroy()
        if txt:
            self.shapes.append({"t": "text", "c": self.COLORS[self.color][0], "p": [self.text_pos], "s": txt})
            self.update_undo()
        self.canvas.queue_draw()
        return False

    def cancel_text(self):
        if self.text_entry:
            ent, self.text_entry = self.text_entry, None
            ent.destroy()

    # --- рисование (в координатах снимка)
    def paint_shape(self, cr, sh):
        lw = self.lw
        r, g, b = sh["c"]
        t = sh["t"]
        cr.set_line_cap(1)
        cr.set_line_join(1)
        if t == "arrow":
            (x0, y0), (x1, y1) = sh["p"][0], sh["p"][-1]
            ang = math.atan2(y1 - y0, x1 - x0)
            head = lw * 5
            cr.set_source_rgb(r, g, b)
            cr.set_line_width(lw)
            cr.move_to(x0, y0)
            cr.line_to(x1 - math.cos(ang) * head * 0.6, y1 - math.sin(ang) * head * 0.6)
            cr.stroke()
            cr.move_to(x1, y1)
            cr.line_to(x1 - head * math.cos(ang - 0.45), y1 - head * math.sin(ang - 0.45))
            cr.line_to(x1 - head * math.cos(ang + 0.45), y1 - head * math.sin(ang + 0.45))
            cr.close_path()
            cr.fill()
        elif t == "rect":
            (x0, y0), (x1, y1) = sh["p"][0], sh["p"][-1]
            cr.set_source_rgb(r, g, b)
            cr.set_line_width(lw)
            cr.rectangle(min(x0, x1), min(y0, y1), abs(x1 - x0), abs(y1 - y0))
            cr.stroke()
        elif t == "marker":
            cr.set_source_rgba(r, g, b, 0.4)
            cr.set_line_width(lw * 5)
            pts = sh["p"]
            cr.move_to(*pts[0])
            for p in pts[1:]:
                cr.line_to(*p)
            cr.stroke()
        elif t == "text":
            x, y = sh["p"][0]
            cr.select_font_face("Sans", 0, 1)
            cr.set_font_size(lw * 6)
            asc = cr.font_extents()[0]
            cr.move_to(x, y + asc * 0.4)
            cr.text_path(sh["s"])
            cr.set_source_rgba(0, 0, 0, 0.85)
            cr.set_line_width(lw * 0.9)
            cr.stroke_preserve()
            cr.set_source_rgb(r, g, b)
            cr.fill()

    def paint_all(self, cr):
        Gdk.cairo_set_source_pixbuf(cr, self.pix, 0, 0)
        cr.paint()
        for sh in self.shapes + ([self.cur] if self.cur else []):
            self.paint_shape(cr, sh)

    def on_draw(self, w, cr):
        cr.scale(self.scale, self.scale)
        self.paint_all(cr)
        return False

    # --- завершение
    def finish(self, marked):
        if self.finished:
            return
        self.commit_text()
        self.finished = True
        out = self.path
        if marked and self.shapes:
            import cairo
            surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, self.iw, self.ih)
            self.paint_all(cairo.Context(surf))
            base, ext = os.path.splitext(self.path)
            out = base + "_marked.png"
            try:
                surf.write_to_png(out)
            except Exception as e:
                ui.notify(APP_NAME, "Не удалось сохранить разметку: %s" % e)
                out = self.path
        self.destroy()
        self.on_done(out, out != self.path)


HOTKEYS = GnomeHotkeys(APP_NAME, [  # id, подпись, команда, по умолчанию
    ("lens-open", "Открыть панель", LENS_CMD + " --bar", "<Super>c"),
    ("lens-selection", "Спросить про выделенный текст", LENS_CMD + " --selection", "<Super><Shift>c"),
    ("lens-capture", "Снимок области и вопрос", LENS_CMD + " --capture area", ""),
    ("lens-quiet", "Тихий вопрос по экрану", LENS_CMD + " --quiet", "<Super><Alt>c"),
])
Hotkeys = HOTKEYS  # имя, под которым его использует окно настроек
Hotkeys.ITEMS = HOTKEYS.items


def lens_autostart_enabled():
    return autostart_enabled(APP_ID)


def lens_set_autostart(on):
    set_autostart(APP_ID, APP_NAME, "Быстрая панель Claude Code: вопрос по экрану",
                  LENS_CMD + " --background", APP_ID, on)


class SettingsWindow(Gtk.Window):
    MODELS = [("", "По умолчанию (как в Claude Code)"), ("opus", "Opus — самая умная"),
              ("sonnet", "Sonnet — быстрее и дешевле"), ("haiku", "Haiku — самая быстрая")]

    def __init__(self, app):
        super().__init__(title="Lens — настройки")
        self.app = app
        self.set_application(app)
        self.set_default_size(560, 640)
        self.set_position(Gtk.WindowPosition.CENTER)
        self.set_icon_name("preferences-system")
        self.capturing = None
        self.connect("key-press-event", self.on_key)

        sw = Gtk.ScrolledWindow()
        sw.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.add(sw)
        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.body.set_border_width(20)
        sw.add(self.body)

        # --- Claude
        self.section("Claude")
        self.model = Gtk.ComboBoxText()
        for mid, title in self.MODELS:
            self.model.append(mid or "default", title)
        self.model.append("custom", "Свой ID модели…")
        self.model_entry = Gtk.Entry()
        self.model_entry.set_placeholder_text("например, claude-opus-5-5")
        self.model_entry.connect("changed", lambda e: conf_set("model", e.get_text().strip()))
        self.model_entry.set_no_show_all(True)
        cur = conf_get("model") or ""
        ids = [m for m, _t in self.MODELS]
        if cur in ids:
            self.model.set_active_id(cur or "default")
        else:
            self.model.set_active_id("custom")
            self.model_entry.set_text(cur)
            self.model_entry.show()
        self.model.connect("changed", self.on_model)
        mbox = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        mbox.pack_start(self.model, False, False, 0)
        mbox.pack_start(self.model_entry, False, False, 0)
        self.row("Модель", mbox, "Каким Claude отвечать. По умолчанию — модель из настроек Claude Code.")

        self.sp = Gtk.TextView()
        self.sp.set_wrap_mode(Gtk.WrapMode.WORD_CHAR)
        self.sp.set_left_margin(8); self.sp.set_right_margin(8)
        self.sp.set_top_margin(6); self.sp.set_bottom_margin(6)
        self.sp.get_buffer().set_text(conf_get("system_prompt") or "")
        self.sp.get_buffer().connect("changed", self.on_sp_changed)
        self._sp_timer = None
        frame = Gtk.Frame()
        spsw = Gtk.ScrolledWindow()
        spsw.set_min_content_height(90)
        spsw.add(self.sp)
        frame.add(spsw)
        spbox = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        spbox.pack_start(frame, False, False, 0)
        rst = Gtk.Button(label="По умолчанию")
        rst.set_halign(Gtk.Align.START)
        rst.connect("clicked", lambda *a: self.sp.get_buffer().set_text(DEFAULT_SYSTEM_PROMPT))
        spbox.pack_start(rst, False, False, 0)
        self.row("Системный промпт", spbox, "Добавляется к каждому разговору: язык, стиль, длина ответов.",
                 vertical=True)

        # --- горячие клавиши
        self.section("Горячие клавиши")
        self.key_btns = {}
        for kid, title, _arg, default in Hotkeys.ITEMS:
            box = Gtk.Box(spacing=6)
            b = Gtk.Button(label=Hotkeys.label(Hotkeys.get(kid)))
            b.set_size_request(190, -1)
            b.connect("clicked", self.start_capture, kid)
            r = Gtk.Button.new_from_icon_name("edit-undo-symbolic", Gtk.IconSize.BUTTON)
            r.set_tooltip_text("По умолчанию: " + Hotkeys.label(default))
            r.connect("clicked", self.reset_key, kid, default)
            box.pack_start(b, False, False, 0)
            box.pack_start(r, False, False, 0)
            self.key_btns[kid] = b
            self.row(title, box)
        hint = Gtk.Label(xalign=0)
        hint.set_line_wrap(True)
        hint.set_markup("<small>Нажмите на кнопку и затем нужное сочетание. Esc — отмена, Backspace — отключить. "
                        "Внутри панели: Ctrl+S / Ctrl+Shift+S / Ctrl+Alt+S — снимки, Ctrl+H — история, "
                        "Ctrl+N — новый чат.</small>")
        hint.get_style_context().add_class("dim-label")
        self.body.pack_start(hint, False, False, 0)

        # --- снимки
        self.section("Снимки")
        self.folder = Gtk.FileChooserButton(title="Папка для снимков", action=Gtk.FileChooserAction.SELECT_FOLDER)
        self.folder.set_filename(SHOTS)
        self.folder.connect("file-set", self.on_folder)
        self.row("Папка для снимков", self.folder,
                 "Сюда сохраняются скриншоты, разметка и прикреплённые файлы. Обычные скриншоты GNOME не затрагиваются.")
        perm = Gtk.Button(label="Разрешить")
        perm.connect("clicked", self.grant_screenshots)
        self.row("Снимки без подтверждения", perm,
                 "Нужно для режима «Весь экран» и тихого режима: GNOME один раз спросит разрешение.")

        # --- быстрые действия
        self.section("Быстрые действия")
        d = Gtk.Label(xalign=0)
        d.set_markup("<small>Кнопки над полем ввода: отправляют прикреплённый скриншот, файл или текст с "
                     "готовым промптом. Первые девять — также Alt+1…9.</small>")
        d.set_line_wrap(True)
        d.get_style_context().add_class("dim-label")
        self.body.pack_start(d, False, False, 0)
        self.acts = [dict(a) for a in get_actions()]
        self.acts_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.acts_box.set_margin_top(6)
        self.body.pack_start(self.acts_box, False, False, 0)
        abtns = Gtk.Box(spacing=8)
        addb = Gtk.Button.new_with_label("Добавить действие")
        addb.set_image(Gtk.Image.new_from_icon_name("list-add-symbolic", Gtk.IconSize.BUTTON))
        addb.set_always_show_image(True)
        addb.connect("clicked", lambda *a: self.add_action())
        rstb = Gtk.Button(label="Вернуть стандартные")
        rstb.connect("clicked", lambda *a: self.reset_actions())
        abtns.pack_start(addb, False, False, 0)
        abtns.pack_start(rstb, False, False, 0)
        abtns.set_margin_bottom(8)
        self.body.pack_start(abtns, False, False, 0)
        self._acts_timer = None
        self.fill_actions()

        # --- общие
        self.section("Общие")
        self.autostart = Gtk.Switch()
        self.autostart.set_active(lens_autostart_enabled())
        self.autostart.connect("notify::active", lambda w, p: lens_set_autostart(w.get_active()))
        self.row("Запускать при входе в систему", self.autostart)
        self.bubble = Gtk.Switch()
        self.bubble.set_active(bool(conf_get("bubble")))
        self.bubble.connect("notify::active", self.on_bubble)
        self.row("Плавающая кнопка поверх окон", self.bubble)
        self.autoclip = Gtk.Switch()
        self.autoclip.set_active(bool(conf_get("auto_clip")))
        self.autoclip.connect("notify::active", lambda w, p: conf_set("auto_clip", w.get_active()))
        self.row("Прикреплять только что скопированное", self.autoclip,
                 "При открытии панели или клике в поле ввода то, что вы скопировали за последние 5 минут, "
                 "сразу прикрепляется вложением.")

        self.show_all()

    # --- раскладка
    def section(self, title):
        lbl = Gtk.Label(xalign=0)
        lbl.set_markup("<b>%s</b>" % html.escape(title))
        lbl.set_margin_top(14 if self.body.get_children() else 0)
        lbl.set_margin_bottom(4)
        self.body.pack_start(lbl, False, False, 0)

    def row(self, title, widget, desc=None, vertical=False):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL if vertical else Gtk.Orientation.HORIZONTAL, spacing=12)
        box.set_margin_bottom(8)
        tb = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        tl = Gtk.Label(label=title, xalign=0)
        tb.pack_start(tl, False, False, 0)
        if desc:
            dl = Gtk.Label(xalign=0)
            dl.set_markup("<small>%s</small>" % html.escape(desc))
            dl.set_line_wrap(True)
            dl.set_max_width_chars(48)
            dl.get_style_context().add_class("dim-label")
            tb.pack_start(dl, False, False, 0)
        box.pack_start(tb, True, True, 0)
        if vertical:
            box.pack_start(widget, False, False, 0)
        else:
            widget.set_valign(Gtk.Align.CENTER)
            box.pack_end(widget, False, False, 0)
        self.body.pack_start(box, False, False, 0)

    # --- обработчики
    def on_model(self, combo):
        aid = combo.get_active_id()
        if aid == "custom":
            self.model_entry.show()
            self.model_entry.grab_focus()
            conf_set("model", self.model_entry.get_text().strip())
        else:
            self.model_entry.hide()
            conf_set("model", "" if aid in (None, "default") else aid)

    def on_sp_changed(self, buf):
        if self._sp_timer:
            GLib.source_remove(self._sp_timer)
        self._sp_timer = GLib.timeout_add(400, self.save_sp)

    def save_sp(self):
        self._sp_timer = None
        buf = self.sp.get_buffer()
        conf_set("system_prompt", buf.get_text(buf.get_start_iter(), buf.get_end_iter(), False).strip())
        return False

    def on_folder(self, btn):
        path = btn.get_filename()
        if path:
            conf_set("shots_dir", path)
            set_shots_dir(path)

    # --- быстрые действия
    def fill_actions(self, focus_last=False):
        for c in self.acts_box.get_children():
            c.destroy()
        last_name = None
        for i, act in enumerate(self.acts):
            card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
            top = Gtk.Box(spacing=6)
            name = Gtk.Entry()
            name.set_text(act.get("name", ""))
            name.set_placeholder_text("Название кнопки")
            name.set_width_chars(18)
            name.connect("changed", self.on_act_changed, i, "name")
            top.pack_start(name, True, True, 0)
            for icon, tip, delta in (("go-up-symbolic", "Выше", -1), ("go-down-symbolic", "Ниже", 1)):
                b = Gtk.Button.new_from_icon_name(icon, Gtk.IconSize.BUTTON)
                b.set_tooltip_text(tip)
                b.set_sensitive(0 <= i + delta < len(self.acts))
                b.connect("clicked", self.move_action, i, delta)
                top.pack_start(b, False, False, 0)
            rm = Gtk.Button.new_from_icon_name("user-trash-symbolic", Gtk.IconSize.BUTTON)
            rm.set_tooltip_text("Удалить действие")
            rm.connect("clicked", self.del_action, i)
            top.pack_start(rm, False, False, 0)
            pr = Gtk.TextView()
            pr.set_wrap_mode(Gtk.WrapMode.WORD_CHAR)
            pr.set_left_margin(6); pr.set_right_margin(6); pr.set_top_margin(4); pr.set_bottom_margin(4)
            pr.get_buffer().set_text(act.get("prompt", ""))
            pr.get_buffer().connect("changed", self.on_act_prompt, i)
            fr = Gtk.Frame()
            fr.add(pr)
            card.pack_start(top, False, False, 0)
            card.pack_start(fr, False, False, 0)
            self.acts_box.pack_start(card, False, False, 0)
            last_name = name
        if not self.acts:
            e = Gtk.Label(label="Действий нет — добавьте своё или верните стандартные.", xalign=0)
            e.get_style_context().add_class("dim-label")
            self.acts_box.pack_start(e, False, False, 0)
        self.acts_box.show_all()
        if focus_last and last_name:
            last_name.grab_focus()

    def on_act_changed(self, entry, i, key):
        self.acts[i][key] = entry.get_text()
        self.save_actions_later()

    def on_act_prompt(self, buf, i):
        self.acts[i]["prompt"] = buf.get_text(buf.get_start_iter(), buf.get_end_iter(), False)
        self.save_actions_later()

    def save_actions_later(self):
        if self._acts_timer:
            GLib.source_remove(self._acts_timer)
        self._acts_timer = GLib.timeout_add(400, self.save_actions)

    def save_actions(self):
        self._acts_timer = None
        conf_set("actions", [{"name": a.get("name", "").strip(), "prompt": a.get("prompt", "").strip()}
                             for a in self.acts])
        if self.app.bar:
            self.app.bar.rebuild_actions()
        return False

    def add_action(self):
        self.acts.append({"name": "", "prompt": ""})
        self.fill_actions(focus_last=True)

    def del_action(self, _b, i):
        del self.acts[i]
        self.fill_actions()
        self.save_actions()

    def move_action(self, _b, i, delta):
        j = i + delta
        self.acts[i], self.acts[j] = self.acts[j], self.acts[i]
        self.fill_actions()
        self.save_actions()

    def reset_actions(self):
        self.acts = [dict(a) for a in DEFAULT_ACTIONS]
        self.fill_actions()
        self.save_actions()

    def grant_screenshots(self, btn):
        # диалог разрешения GNOME показывает только приложению в фокусе — это окно сейчас в фокусе
        btn.set_sensitive(False)
        btn.set_label("Ждём ответа…")

        def done(path, code):
            btn.set_sensitive(True)
            if path:
                try:
                    os.remove(path)
                except OSError:
                    pass
                btn.set_label("Разрешено ✓")
            else:
                btn.set_label("Не разрешено — ещё раз")

        portals.screenshot(self.app.bus, False, done)

    def on_bubble(self, sw, _p):
        self.app.set_bubble(sw.get_active())

    def start_capture(self, btn, kid):
        self.stop_capture()
        self.capturing = kid
        btn.set_label("Нажмите сочетание…")

    def stop_capture(self):
        if self.capturing:
            kid = self.capturing
            self.capturing = None
            self.key_btns[kid].set_label(Hotkeys.label(Hotkeys.get(kid)))

    def reset_key(self, _b, kid, default):
        self.stop_capture()
        Hotkeys.set(kid, default)
        self.key_btns[kid].set_label(Hotkeys.label(default))

    def on_key(self, w, e):
        if not self.capturing:
            if e.keyval == Gdk.KEY_Escape:
                self.close()
                return True
            return False
        if e.is_modifier:
            return True
        kid = self.capturing
        mods = e.state & Gtk.accelerator_get_default_mod_mask()
        if not mods and e.keyval == Gdk.KEY_Escape:
            self.stop_capture()
            return True
        if not mods and e.keyval == Gdk.KEY_BackSpace:
            self.capturing = None
            Hotkeys.set(kid, "")
            self.key_btns[kid].set_label(Hotkeys.label(""))
            return True
        # латинская буква по физической клавише — чтобы русская раскладка не мешала
        ok, keyval, _g, _l, _c = Gdk.Keymap.get_for_display(self.get_display()).translate_keyboard_state(
            e.hardware_keycode, 0, 0)
        keyval = Gdk.keyval_to_lower(keyval if ok else e.keyval)
        fkey = Gdk.KEY_F1 <= keyval <= Gdk.KEY_F35 or keyval == Gdk.KEY_Print
        if not mods and not fkey:
            self.key_btns[kid].set_label("Нужен Ctrl, Alt или Super…")
            return True
        accel = Gtk.accelerator_name(keyval, mods).replace("<Primary>", "<Control>").replace("<Mod4>", "<Super>")
        self.capturing = None
        Hotkeys.set(kid, accel)
        self.key_btns[kid].set_label(Hotkeys.label(accel))
        return True


class LastClip:
    """Помнит, когда в последний раз что-то скопировали в любом приложении (для автоприкрепления).

    Историю не хранит — этим занимается отдельное приложение Clipboard History.
    """

    def __init__(self):
        self.serial = 0
        self.changed_at = 0.0
        self.ignore_until = 0.0
        self.cb = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        self.cb.connect("owner-change", self.on_change)

    def ignore(self, seconds=1.5):
        """Своё копирование (кнопки «Копировать») не считаем «только что скопированным»."""
        self.ignore_until = time.time() + seconds

    def on_change(self, *_a):
        if time.time() < self.ignore_until:
            return
        self.serial += 1
        self.changed_at = time.time()

    def read(self):
        try:
            if self.cb.wait_is_image_available():
                pb = self.cb.wait_for_image()
                return ("image", pb) if pb else (None, None)
            text = self.cb.wait_for_text()
            return ("text", text) if text else (None, None)
        except Exception:
            return None, None


def make_tray(app):
    """Значок в трее (StatusNotifierItem). В GNOME виден с расширением AppIndicator, в KDE/Ubuntu — сразу."""
    try:
        gi.require_version("AyatanaAppIndicator3", "0.1")
        from gi.repository import AyatanaAppIndicator3 as AI
    except (ValueError, ImportError):
        return None
    ind = AI.Indicator.new(APP_ID, APP_ID + "-symbolic", AI.IndicatorCategory.APPLICATION_STATUS)
    ind.set_title(APP_NAME)
    ind.set_status(AI.IndicatorStatus.ACTIVE)
    menu = Gtk.Menu()

    def item(label, cb):
        it = Gtk.MenuItem(label=label)
        it.connect("activate", lambda *_a: cb())
        menu.append(it)
        return it

    item("Открыть панель", lambda: app.bar.open())
    menu.append(Gtk.SeparatorMenuItem())
    item("Снимок: область", lambda: app.bar.open_and_capture("area"))
    item("Снимок: весь экран", lambda: app.bar.open_and_capture("screen"))
    item("Снимок: окно", lambda: app.bar.open_and_capture("window"))
    item("Спросить про выделенный текст", app.ask_selection)
    item("История чатов", app.show_history)
    menu.append(Gtk.SeparatorMenuItem())
    app.tray_bubble_item = Gtk.CheckMenuItem(label="Плавающая кнопка")
    app.tray_bubble_item.set_active(bool(conf_get("bubble")))
    app.tray_bubble_item.connect("toggled", lambda w: app.set_bubble(w.get_active()))
    menu.append(app.tray_bubble_item)
    item("Настройки…", app.show_settings)
    item("Выйти", app.quit)
    menu.show_all()
    ind.set_menu(menu)
    return ind


class LensApp(Gtk.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.bubble = None
        self.theme = "dark"
        self.bar = None
        self.capturing = False
        self.tray = None
        self.tray_bubble_item = None

    def do_startup(self):
        Gtk.Application.do_startup(self)
        set_shots_dir(conf_get("shots_dir"))
        os.makedirs(WORKDIR, exist_ok=True)
        self.settings_win = None
        self.quiet_popup = None
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        ui.apply_css()
        self.themes = ui.ThemeWatcher(self.on_theme)
        self.theme = self.themes.theme
        self.last_clip = LastClip()
        md_render.on_copy = self.last_clip.ignore
        try:
            HOTKEYS.ensure_defaults()
        except Exception as e:
            print("hotkeys:", e, file=sys.stderr)
        self.hold()

    def on_theme(self, theme):
        self.theme = theme
        if self.bar:
            self.bar.apply_theme()

    def do_command_line(self, cmdline):
        args = cmdline.get_arguments()[1:]
        first = self.bubble is None
        if first:
            self.bubble = Bubble(self)
            self.bar = QuickBar(self)
            self.bar.apply_theme()
            if "--no-bubble" not in args and conf_get("bubble"):
                self.bubble.show_all()
            self.tray = make_tray(self)

        def arg_after(flag):
            i = args.index(flag)
            return args[i + 1] if i + 1 < len(args) else None

        if "--background" in args:
            pass  # автозапуск: только кнопка и значок
        elif "--toggle-bubble" in args:
            self.set_bubble(not self.bubble.get_visible())
        elif "--capture" in args:
            mode = arg_after("--capture")
            self.bar.open_and_capture(mode if mode in MODES else "area")
        elif "--quiet" in args:
            self.quiet()
        elif "--settings" in args:
            self.show_settings()
        elif "--selection" in args:
            self.ask_selection()
        elif "--history" in args:
            self.show_history()
        elif "--attach-image" in args:
            self.attach_external("image", arg_after("--attach-image"))
        elif "--attach-text-file" in args:
            self.attach_external("text", arg_after("--attach-text-file"))
        elif "--quit" in args:
            self.quit()
        elif "--bar" in args or not first:
            self.bar.toggle()
        else:
            self.bar.open()  # запуск из меню приложений
        return 0

    # --- действия
    def set_bubble(self, on):
        conf_set("bubble", on)
        if self.bubble:
            self.bubble.show_all() if on else self.bubble.hide()
        if self.tray_bubble_item and self.tray_bubble_item.get_active() != on:
            self.tray_bubble_item.set_active(on)

    def show_history(self):
        self.bar.open()
        if not self.bar.hist.get_visible():
            self.bar.toggle_history()

    def attach_external(self, kind, path):
        """Вложение из другого приложения (например, Clipboard History → «Спросить Claude»)."""
        if not path or not os.path.exists(path):
            return
        try:
            if kind == "image":
                os.makedirs(SHOTS, exist_ok=True)
                dst = os.path.join(SHOTS, time.strftime("%Y-%m-%d_%H-%M-%S_clip.png"))
                shutil.copy2(path, dst)
                self.bar.add_attachment("image", path=dst, label="Картинка из буфера")
            else:
                with open(path, encoding="utf-8") as f:
                    self.bar.add_text(f.read())
                try:
                    os.remove(path)
                except OSError:
                    pass
        except Exception as e:
            self.notify("Не удалось прикрепить: %s" % e)
            return
        self.bar.clip_offered.add(self.last_clip.serial)  # не прикреплять то же самое ещё раз
        self.bar.open()

    @staticmethod
    def notify(text):
        ui.notify(APP_NAME, text)

    PERMISSION_HINT = ("GNOME не разрешил снимать экран без подтверждения. Откройте настройки Lens → "
                       "«Снимки без подтверждения» → «Разрешить».")

    def quiet(self):
        """Снимок всего экрана без переключения фокуса и ответ в углу экрана."""
        if self.quiet_popup is None:
            self.quiet_popup = QuietPopup(self)
        pop = self.quiet_popup
        if pop.get_visible() and not pop.busy:  # повторное нажатие закрывает ответ
            pop.close_popup()
            return
        if pop.busy or self.capturing:
            return
        self.capturing = True

        def got(path, code):
            self.capturing = False
            if path:
                dst = self.new_shot_path()
                try:
                    shutil.move(path, dst)
                except Exception:
                    dst = path
                pop.start(dst)
            elif code != 1:
                self.notify(self.PERMISSION_HINT)

        portals.screenshot(self.bus, False, got)

    def show_settings(self):
        if self.settings_win is None:
            self.settings_win = SettingsWindow(self)
            self.settings_win.connect("destroy", lambda *a: setattr(self, "settings_win", None))
        self.settings_win.present()

    def ask_selection(self):
        """Берёт выделенный мышью текст (PRIMARY selection) и прикрепляет его цитатой."""
        text = Gtk.Clipboard.get(Gdk.SELECTION_PRIMARY).wait_for_text()
        if not (text and text.strip()):
            try:
                text = subprocess.run(["wl-paste", "--primary", "--no-newline"], capture_output=True,
                                      text=True, timeout=3).stdout
            except Exception:
                text = ""
        if not (text and text.strip()):
            self.notify("Нет выделенного текста. Выделите текст мышью и попробуйте снова.")
            return
        self.bar.add_text(text, kind="quote")
        self.bar.open()

    @staticmethod
    def new_shot_path():
        return os.path.join(SHOTS, time.strftime("%Y-%m-%d_%H-%M-%S.png"))

    def screenshot(self, mode, cb):
        """area/window — окно выбора GNOME; screen — весь экран сразу (нужно разрешение один раз)."""
        def got(path, code):
            if path:
                dst = self.new_shot_path()
                try:
                    shutil.move(path, dst)  # уносим из обычной папки скриншотов
                except Exception:
                    dst = path
                cb(dst, None)
            elif mode == "screen" and code == 2:
                self.notify(self.PERMISSION_HINT + " Пока выберите «Экран» в окне скриншота.")
                portals.screenshot(self.bus, True, got_interactive)
            else:
                cb(None, None if code == 1 else "Не удалось сделать скриншот.")

        def got_interactive(path, code):
            got(path, code) if path else cb(None, None)

        portals.screenshot(self.bus, mode != "screen", got)
        return False


def main():
    return LensApp().run(sys.argv)
