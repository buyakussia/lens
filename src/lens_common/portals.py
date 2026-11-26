"""Работа с xdg-desktop-portal: скриншоты и нажатие клавиш (автовставка).

Порталы — стандартный способ для обычных приложений на Wayland получить то, что раньше
делало расширение GNOME Shell: снять экран и нажать Ctrl+V в другом окне.
"""
import os
import time

from gi.repository import Gio, GLib

DEST = "org.freedesktop.portal.Desktop"
PATH = "/org/freedesktop/portal/desktop"

# коды клавиш evdev — не зависят от раскладки
KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_V = 29, 42, 47
DEVICE_KEYBOARD = 1
PERSIST_PERMANENT = 2


class PortalRequest:
    """Вызов метода портала, который отвечает сигналом Request.Response."""

    _n = 0

    def __init__(self, bus):
        self.bus = bus

    def call(self, iface, method, args_before, options, on_response, sig="(sa{sv})"):
        PortalRequest._n += 1
        token = "lens%d_%d" % (int(time.time() * 1000) % 10_000_000, PortalRequest._n)
        sender = self.bus.get_unique_name()[1:].replace(".", "_")
        req_path = "%s/request/%s/%s" % (PATH, sender, token)
        opts = dict(options)
        opts["handle_token"] = GLib.Variant("s", token)
        state = {}

        def on_signal(_c, _s, _p, _i, _n, params, _u):
            self.bus.signal_unsubscribe(state["sub"])
            code, results = params.unpack()
            on_response(code, results)

        def on_called(bus, res, _u):
            try:
                bus.call_finish(res)
            except Exception as e:
                self.bus.signal_unsubscribe(state["sub"])
                on_response(-1, {"error": str(e)})

        state["sub"] = self.bus.signal_subscribe(DEST, "org.freedesktop.portal.Request", "Response", req_path,
                                                 None, Gio.DBusSignalFlags.NO_MATCH_RULE, on_signal, None)
        self.bus.call(DEST, PATH, iface, method, GLib.Variant(sig, tuple(args_before) + (opts,)),
                      None, Gio.DBusCallFlags.NONE, -1, None, on_called, None)


def screenshot(bus, interactive, cb):
    """cb(path_or_None, code). interactive=True — окно выбора GNOME (область/окно/экран).

    interactive=False снимает весь экран сразу; в первый раз GNOME спрашивает разрешение
    (диалог показывается, только если приложение в фокусе), потом запоминает его.
    code: 0 — готово, 1 — пользователь отменил, 2 — отказано/не удалось.
    """
    def done(code, results):
        if code == 0 and "uri" in results:
            cb(Gio.File.new_for_uri(results["uri"]).get_path(), 0)
        else:
            cb(None, code)

    PortalRequest(bus).call("org.freedesktop.portal.Screenshot", "Screenshot", ("",),
                            {"interactive": GLib.Variant("b", interactive)}, done)


class RemotePaste:
    """Нажимает Ctrl+V (или Ctrl+Shift+V) в активном окне через портал Remote Desktop.

    Первый раз GNOME показывает диалог разрешения; токен восстановления сохраняется через
    get_token/set_token, и дальше вставка идёт без вопросов. Пока сессия открыта (доли секунды),
    GNOME показывает в верхней панели значок удалённого управления.
    """

    def __init__(self, bus, get_token, set_token):
        self.bus = bus
        self.get_token = get_token
        self.set_token = set_token
        self.busy = False

    def paste(self, shift=False, cb=None):
        cb = cb or (lambda ok, err: None)
        if self.busy:
            cb(False, "занято")
            return
        self.busy = True
        req = PortalRequest(self.bus)
        iface = "org.freedesktop.portal.RemoteDesktop"

        def fail(msg):
            self.busy = False
            cb(False, msg)

        def created(code, res):
            if code != 0 or "session_handle" not in res:
                return fail("не удалось создать сессию (%s)" % code)
            session = res["session_handle"]
            opts = {"types": GLib.Variant("u", DEVICE_KEYBOARD),
                    "persist_mode": GLib.Variant("u", PERSIST_PERMANENT)}
            tok = self.get_token()
            if tok:
                opts["restore_token"] = GLib.Variant("s", tok)
            req.call(iface, "SelectDevices", (session,), opts,
                     lambda c, r: selected(session, c, r), sig="(oa{sv})")

        def selected(session, code, res):
            if code != 0:
                self.close(session)
                return fail("устройство не выбрано (%s)" % code)
            req.call(iface, "Start", (session, ""), {}, lambda c, r: started(session, c, r), sig="(osa{sv})")

        def started(session, code, res):
            if code != 0:
                self.close(session)
                return fail("отказано в доступе" if code == 1 else "не удалось начать (%s)" % code)
            if res.get("restore_token"):
                self.set_token(res["restore_token"])
            keys = [KEY_LEFTCTRL] + ([KEY_LEFTSHIFT] if shift else []) + [KEY_V]
            try:
                for k in keys:
                    self.key(session, k, 1)
                for k in reversed(keys):
                    self.key(session, k, 0)
            except Exception as e:
                self.close(session)
                return fail(str(e))
            # закрываем чуть позже, чтобы нажатия успели дойти
            GLib.timeout_add(300, lambda: self.close(session) or False)
            self.busy = False
            cb(True, None)

        req.call(iface, "CreateSession", (), {"session_handle_token": GLib.Variant("s", "lens%d" % os.getpid())},
                 created, sig="(a{sv})")

    def key(self, session, code, state):
        self.bus.call_sync(DEST, PATH, "org.freedesktop.portal.RemoteDesktop", "NotifyKeyboardKeycode",
                           GLib.Variant("(oa{sv}iu)", (session, {}, code, state)), None,
                           Gio.DBusCallFlags.NONE, 2000, None)

    def close(self, session):
        try:
            self.bus.call_sync(DEST, session, "org.freedesktop.portal.Session", "Close", None, None,
                               Gio.DBusCallFlags.NONE, 2000, None)
        except Exception:
            pass
