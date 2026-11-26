//! Поведение окон через XWayland, которого GTK4 не даёт сделать штатно:
//! «поверх всех окон», окно без фокуса (тихий режим) и точное положение на экране.

use gtk::prelude::*;
use std::cell::RefCell;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, EventMask,
    PropMode, StackMode,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

struct X {
    conn: RustConnection,
    root: u32,
}

thread_local! {
    static CONN: RefCell<Option<X>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&X) -> Option<R>) -> Option<R> {
    CONN.with(|c| {
        if c.borrow().is_none() {
            let (conn, screen) = x11rb::connect(None).ok()?;
            let root = conn.setup().roots.get(screen)?.root;
            *c.borrow_mut() = Some(X { conn, root });
        }
        let b = c.borrow();
        let r = f(b.as_ref()?);
        let _ = b.as_ref()?.conn.flush();
        r
    })
}

fn atom(x: &X, name: &str) -> Option<u32> {
    Some(x.conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom)
}

/// X11-идентификатор окна (окно должно быть уже создано: realize()).
pub fn xid(win: &impl IsA<gtk::Native>) -> Option<u32> {
    let surface = win.as_ref().surface()?;
    let s = surface.downcast_ref::<gdk_x11::X11Surface>()?;
    Some(s.xid() as u32)
}

pub fn is_x11() -> bool {
    gtk::gdk::Display::default().map(|d| d.is::<gdk_x11::X11Display>()).unwrap_or(false)
}

/// Держать окно поверх остальных и не показывать в панели задач / Alt+Tab.
/// Вызывать после realize() (до показа) — и можно ещё раз после показа.
pub fn keep_above(win: &impl IsA<gtk::Native>) {
    let Some(id) = xid(win) else { return };
    with(|x| {
        let state = atom(x, "_NET_WM_STATE")?;
        let atoms = [atom(x, "_NET_WM_STATE_ABOVE")?, atom(x, "_NET_WM_STATE_SKIP_TASKBAR")?, atom(x, "_NET_WM_STATE_SKIP_PAGER")?];
        // до показа окна достаточно свойства, после — нужно попросить оконный менеджер
        x.conn.change_property32(PropMode::REPLACE, id, state, AtomEnum::ATOM, &atoms).ok()?;
        for a in atoms {
            let ev = ClientMessageEvent::new(32, id, state, [1, a, 0, 1, 0]);
            x.conn
                .send_event(false, x.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev)
                .ok()?;
        }
        Some(())
    });
}

/// Включить или выключить «поверх всех окон» у уже показанного окна.
pub fn set_above(win: &impl IsA<gtk::Native>, on: bool) {
    let Some(id) = xid(win) else { return };
    with(|x| {
        let state = atom(x, "_NET_WM_STATE")?;
        let above = atom(x, "_NET_WM_STATE_ABOVE")?;
        let ev = ClientMessageEvent::new(32, id, state, [u32::from(on), above, 0, 1, 0]);
        x.conn
            .send_event(false, x.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev)
            .ok()?;
        Some(())
    });
}

/// Окно, которое никогда не получает фокус и которым не управляет оконный менеджер
/// (override-redirect): видео в активном окне продолжает играть, фокус не теряется.
/// Вызывать после realize() и до показа.
pub fn make_unmanaged(win: &impl IsA<gtk::Native>) {
    let Some(id) = xid(win) else { return };
    with(|x| {
        x.conn
            .change_window_attributes(id, &ChangeWindowAttributesAux::new().override_redirect(1))
            .ok()?;
        let wtype = atom(x, "_NET_WM_WINDOW_TYPE")?;
        let notif = atom(x, "_NET_WM_WINDOW_TYPE_NOTIFICATION")?;
        x.conn.change_property32(PropMode::REPLACE, id, wtype, AtomEnum::ATOM, &[notif]).ok()?;
        Some(())
    });
}

/// Переместить окно (координаты в логических пикселях GTK).
pub fn move_to(win: &impl IsA<gtk::Native>, x_pos: i32, y_pos: i32) {
    let Some(id) = xid(win) else { return };
    let scale = win.as_ref().surface().map(|s| s.scale_factor()).unwrap_or(1).max(1);
    with(|x| {
        x.conn
            .configure_window(
                id,
                &ConfigureWindowAux::new().x(x_pos * scale).y(y_pos * scale).stack_mode(StackMode::ABOVE),
            )
            .ok()?;
        Some(())
    });
}

/// Поднять окно наверх стопки.
pub fn raise(win: &impl IsA<gtk::Native>) {
    let Some(id) = xid(win) else { return };
    with(|x| {
        x.conn.configure_window(id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE)).ok()?;
        Some(())
    });
}

fn monitor_of(widget: &impl IsA<gtk::Widget>) -> Option<gtk::gdk::Monitor> {
    let display = widget.as_ref().display();
    widget
        .as_ref()
        .native()
        .and_then(|n| n.surface())
        .and_then(|s| display.monitor_at_surface(&s))
        .or_else(|| display.monitors().item(0).and_then(|m| m.downcast::<gtk::gdk::Monitor>().ok()))
}

/// Рабочая область монитора (без верхней панели GNOME), в логических пикселях.
pub fn workarea(widget: &impl IsA<gtk::Widget>) -> gtk::gdk::Rectangle {
    let Some(m) = monitor_of(widget) else { return gtk::gdk::Rectangle::new(0, 0, 1920, 1080) };
    if let Some(xm) = m.downcast_ref::<gdk_x11::X11Monitor>() {
        return xm.workarea();
    }
    m.geometry()
}

/// Высота дока с запасом: столько оставляем снизу, если док может выехать поверх окон.
const DOCK_GAP: i32 = 96;

/// Нижняя граница, до которой можно ставить окно, чтобы его не закрыл док.
///
/// Закреплённый док (Dash to Dock, Ubuntu Dock) резервирует место, и GNOME уже вычитает его
/// из рабочей области — тогда хватит маленького отступа. Док с автоскрытием места не резервирует
/// и выезжает поверх окон, поэтому если снизу ничего не зарезервировано, оставляем место под него.
pub fn safe_bottom(widget: &impl IsA<gtk::Widget>, margin: i32) -> i32 {
    let wa = workarea(widget);
    let bottom = wa.y() + wa.height();
    let screen_bottom = monitor_of(widget).map(|m| m.geometry()).map(|g| g.y() + g.height()).unwrap_or(bottom);
    if bottom >= screen_bottom {
        bottom - margin.max(DOCK_GAP)
    } else {
        bottom - margin
    }
}

/// Первый монитор (на XWayland GTK не знает, где курсор на Wayland-окнах).
pub fn primary_monitor() -> Option<gtk::gdk::Monitor> {
    let display = gtk::gdk::Display::default()?;
    display.monitors().item(0).and_then(|m| m.downcast::<gtk::gdk::Monitor>().ok())
}
