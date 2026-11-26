"""Конфиг в ~/.config/<app>, автозапуск и глобальные горячие клавиши GNOME."""
import json
import os
import shutil

from gi.repository import Gio


class Config:
    """JSON-конфиг с значениями по умолчанию: ~/.config/<name>/config.json."""

    def __init__(self, name, defaults):
        self.dir = os.path.join(os.path.expanduser("~/.config"), name)
        self.path = os.path.join(self.dir, "config.json")
        self.defaults = defaults

    def load(self):
        try:
            with open(self.path, encoding="utf-8") as f:
                data = json.load(f)
                return data if isinstance(data, dict) else {}
        except Exception:
            return {}

    def save(self, data):
        os.makedirs(self.dir, exist_ok=True)
        tmp = self.path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=1)
        os.replace(tmp, self.path)

    def get(self, key):
        return self.load().get(key, self.defaults.get(key))

    def set(self, key, value):
        data = self.load()
        data[key] = value
        self.save(data)


def find_executable(name):
    """Путь к установленной команде: PATH, затем ~/.local/bin."""
    return shutil.which(name) or os.path.expanduser("~/.local/bin/" + name)


# ---------------------------------------------------------------- автозапуск
AUTOSTART_DIR = os.path.expanduser("~/.config/autostart")


def autostart_path(desktop_id):
    return os.path.join(AUTOSTART_DIR, desktop_id + ".desktop")


def autostart_enabled(desktop_id):
    try:
        with open(autostart_path(desktop_id)) as f:
            t = f.read()
        return "X-GNOME-Autostart-enabled=false" not in t and "Hidden=true" not in t
    except FileNotFoundError:
        return False


def set_autostart(desktop_id, name, comment, exec_line, icon, on):
    os.makedirs(AUTOSTART_DIR, exist_ok=True)
    with open(autostart_path(desktop_id), "w") as f:
        f.write("[Desktop Entry]\nType=Application\nName=%s\nComment=%s\nExec=%s\nIcon=%s\n"
                "Terminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=%s\n"
                % (name, comment, exec_line, icon, "true" if on else "false"))


# ---------------------------------------------------------------- горячие клавиши GNOME
class GnomeHotkeys:
    """Глобальные сочетания клавиш через пользовательские комбинации GNOME (media-keys).

    items: [(id, подпись, команда, сочетание по умолчанию)]. Работает в GNOME;
    в других окружениях методы тихо ничего не делают (available == False).
    """
    BASE = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/"
    LIST_SCHEMA = "org.gnome.settings-daemon.plugins.media-keys"
    ITEM_SCHEMA = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding"

    def __init__(self, app_name, items):
        self.app_name = app_name
        self.items = items
        src = Gio.SettingsSchemaSource.get_default()
        self.available = bool(src and src.lookup(self.LIST_SCHEMA, True) and src.lookup(self.ITEM_SCHEMA, True))

    def _item(self, kid):
        return Gio.Settings.new_with_path(self.ITEM_SCHEMA, self.BASE + kid + "/")

    def _meta(self, kid):
        for k, title, cmd, default in self.items:
            if k == kid:
                return title, cmd, default
        raise KeyError(kid)

    def get(self, kid):
        if not self.available:
            return ""
        return self._item(kid).get_string("binding")

    def registered(self, kid):
        if not self.available:
            return False
        return self.BASE + kid + "/" in Gio.Settings.new(self.LIST_SCHEMA).get_strv("custom-keybindings")

    def set(self, kid, accel):
        if not self.available:
            return
        title, cmd, _d = self._meta(kid)
        it = self._item(kid)
        it.set_string("name", "%s — %s" % (self.app_name, title))
        it.set_string("command", cmd)
        it.set_string("binding", accel)
        Gio.Settings.sync()
        # список задаём после заполнения ключей — иначе GNOME может подхватить пустую запись
        lst = Gio.Settings.new(self.LIST_SCHEMA)
        paths = list(lst.get_strv("custom-keybindings"))
        path = self.BASE + kid + "/"
        idx = paths.index(path) if path in paths else len(paths)
        if path in paths:
            paths.remove(path)
        lst.set_strv("custom-keybindings", paths)
        Gio.Settings.sync()
        paths.insert(idx, path)
        lst.set_strv("custom-keybindings", paths)
        Gio.Settings.sync()

    def ensure_defaults(self):
        """При первом запуске регистрирует сочетания по умолчанию (уже настроенные не трогает)."""
        for kid, _t, _c, default in self.items:
            if not self.registered(kid):
                self.set(kid, default)

    def remove(self, kid):
        if not self.available:
            return
        lst = Gio.Settings.new(self.LIST_SCHEMA)
        paths = [p for p in lst.get_strv("custom-keybindings") if p != self.BASE + kid + "/"]
        lst.set_strv("custom-keybindings", paths)
        self._item(kid).reset("binding")
        Gio.Settings.sync()

    @staticmethod
    def label(accel):
        from gi.repository import Gtk
        if not accel:
            return "Не задано"
        key, mods = Gtk.accelerator_parse(accel)
        return Gtk.accelerator_get_label(key, mods) if key else accel
