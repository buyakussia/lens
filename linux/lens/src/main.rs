//! Lens для Linux (GNOME): выделите часть экрана и спросите Claude Code о ней.
//!
//! Команды (их вызывают горячие клавиши GNOME; работающая копия получает их мгновенно):
//!   lens --chat       открыть / закрыть чат (Super+C); область выделяется кнопкой в чате
//!   lens --select     сразу выделить область и спросить (по умолчанию без сочетания)
//!   lens --quiet      тихий вопрос по всему экрану, ответ в углу (Super+Alt+C)
//!   lens --selection  спросить про выделенный мышью текст (Super+Shift+C)
//!   lens --history    история чатов
//!   lens --settings   настройки
//!   lens --attach-image ФАЙЛ / --attach-text-file ФАЙЛ  — прикрепить (из Clipboard History)
//!   lens --background запуск в фоне при входе в систему
//!   lens --quit
//!   lens --remove-hotkeys  убрать сочетания клавиш из GNOME (при удалении)

mod chat;
mod image;
mod markdown;
mod overlay;
mod quiet;
mod selection;
mod settings;
mod style;

use adw::prelude::*;
use chat::ChatView;
use gtk::{gdk, gio, glib};
use lens_core::config::{self, Config};
use linux_common::{portal, x11};
use overlay::Overlay;
use quiet::Quiet;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct App {
    app: adw::Application,
    cfg: Rc<RefCell<Config>>,
    chat: ChatView,
    overlay: RefCell<Option<Rc<Overlay>>>,
    floating: RefCell<Option<gtk::Window>>,
    float_moved: Rc<Cell<bool>>,
    quiet: RefCell<Option<Rc<Quiet>>>,
    capturing: Cell<bool>,
}

#[derive(Clone, Copy, PartialEq)]
enum Shot {
    /// Оверлей с чатом (отдельное сочетание клавиш).
    Select,
    /// Кнопка в чате: только выбрать область и вернуться в чат.
    Pick,
    Quiet,
}

impl App {
    fn new(app: &adw::Application) -> Rc<App> {
        let cfg = Rc::new(RefCell::new(config::load()));
        let chat = ChatView::new(cfg.clone());
        let me = Rc::new(App {
            app: app.clone(),
            cfg,
            chat: chat.clone(),
            overlay: RefCell::new(None),
            floating: RefCell::new(None),
            float_moved: Rc::new(Cell::new(false)),
            quiet: RefCell::new(None),
            capturing: Cell::new(false),
        });
        let w = Rc::downgrade(&me);
        let close: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(a) = w.upgrade() {
                a.close_chat();
            }
        });
        let w = Rc::downgrade(&me);
        let detach: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(a) = w.upgrade() {
                a.detach();
            }
        });
        let w = Rc::downgrade(&me);
        let settings: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(a) = w.upgrade() {
                a.show_settings();
            }
        });
        let w = Rc::downgrade(&me);
        let select: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(a) = w.upgrade() {
                a.pick_area();
            }
        });
        *chat.0.hooks.borrow_mut() =
            chat::Hooks { close: Some(close), detach: Some(detach), settings: Some(settings), select: Some(select) };
        me
    }

    fn notify(&self, text: &str) {
        let n = gio::Notification::new("Lens");
        n.set_body(Some(text));
        self.app.send_notification(None, &n);
    }

    // ------------------------------------------------------------------ снимок экрана

    fn shoot(self: &Rc<Self>, kind: Shot) {
        if self.capturing.replace(true) {
            return;
        }
        let me = self.clone();
        glib::spawn_future_local(async move {
            let res = portal::screenshot(false).await;
            me.capturing.set(false);
            match res {
                Ok(path) => me.got_shot(kind, &path),
                Err(portal::ShotError::Cancelled) => {}
                Err(portal::ShotError::Denied(e)) => me.ask_permission(kind, &e),
            }
        });
    }

    fn got_shot(self: &Rc<Self>, kind: Shot, path: &std::path::Path) {
        // файл, который записал портал, сразу удаляем: снимок живёт только в памяти
        let Some(bytes) = portal::take_file(path) else { return };
        let tex = match gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) {
            Ok(t) => t,
            Err(e) => return self.notify(&format!("Не удалось прочитать снимок экрана: {e}")),
        };
        match kind {
            Shot::Select => self.open_overlay(tex),
            Shot::Pick => self.open_picker(tex),
            Shot::Quiet => self.open_quiet(tex),
        }
    }

    /// GNOME спрашивает разрешение только у приложения, окно которого в фокусе. Поэтому окно
    /// остаётся открытым, пока GNOME не ответит; после разрешения снимок делается заново,
    /// уже без этого окна на экране.
    fn ask_permission(self: &Rc<Self>, kind: Shot, err: &str) {
        eprintln!("lens: screenshot: {err}");
        let win = adw::Window::new();
        win.set_application(Some(&self.app));
        win.set_title(Some("Lens"));
        win.set_default_size(420, -1);
        win.set_resizable(false);
        let page = adw::StatusPage::builder()
            .icon_name("camera-photo-symbolic")
            .title("Разрешите Lens снимать экран")
            .description("Один раз — чтобы выделять область без окна скриншота GNOME.\n\
                          Нажмите «Разрешить», а затем «Разрешить» в вопросе GNOME.")
            .build();
        let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
        col.set_margin_start(32);
        col.set_margin_end(32);
        col.set_margin_bottom(28);
        let allow = gtk::Button::with_label("Разрешить");
        allow.add_css_class("suggested-action");
        allow.add_css_class("pill");
        let gnome = gtk::Button::with_label("Использовать окно скриншота GNOME");
        gnome.add_css_class("pill");
        let cancel = gtk::Button::with_label("Не сейчас");
        cancel.add_css_class("flat");
        col.append(&allow);
        col.append(&gnome);
        col.append(&cancel);
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        outer.append(&page);
        outer.append(&col);
        win.set_content(Some(&outer));

        let w = win.clone();
        cancel.connect_clicked(move |_| w.close());

        let (me, w, p) = (self.clone(), win.clone(), page.clone());
        allow.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Ждём ответа GNOME…");
            let (me, w, p, b) = (me.clone(), w.clone(), p.clone(), b.clone());
            glib::spawn_future_local(async move {
                match portal::screenshot(false).await {
                    Ok(path) => {
                        // этот снимок с нашим окном на экране — не нужен
                        portal::take_file(&path);
                        w.close();
                        glib::timeout_future(std::time::Duration::from_millis(350)).await;
                        me.shoot(kind);
                    }
                    Err(e) => {
                        if let portal::ShotError::Denied(e) = e {
                            eprintln!("lens: screenshot permission: {e}");
                        }
                        p.set_description(Some(
                            "GNOME не дал разрешение. Попробуйте ещё раз или используйте окно скриншота GNOME.",
                        ));
                        b.set_label("Попробовать ещё раз");
                        b.set_sensitive(true);
                    }
                }
            });
        });

        let (me, w) = (self.clone(), win.clone());
        gnome.connect_clicked(move |_| {
            w.close();
            let me = me.clone();
            glib::spawn_future_local(async move {
                glib::timeout_future(std::time::Duration::from_millis(300)).await;
                match portal::screenshot(true).await {
                    Ok(p) => me.got_shot(kind, &p),
                    Err(portal::ShotError::Cancelled) => {}
                    Err(_) => me.notify("Не удалось сделать снимок экрана."),
                }
            });
        });
        win.set_default_widget(Some(&allow));
        win.present();
        allow.grab_focus();
    }

    // ------------------------------------------------------------------ Win+C

    fn toggle_select(self: &Rc<Self>) {
        let ov = self.overlay.borrow().clone();
        if let Some(ov) = ov {
            if !ov.is_closing() {
                ov.close();
                return;
            }
        }
        self.shoot(Shot::Select);
    }

    fn open_overlay(self: &Rc<Self>, tex: gdk::Texture) {
        // чат переезжает из отдельного окна в оверлей
        self.close_floating_window();
        self.chat.on_open();
        let ov = Overlay::open(&self.app, tex, &self.chat, false);
        let w = Rc::downgrade(self);
        let me_ov = Rc::downgrade(&ov);
        ov.connect_closed(move || {
            if let Some(a) = w.upgrade() {
                let same = a.overlay.borrow().as_ref().map(Rc::as_ptr) == me_ov.upgrade().map(|o| Rc::as_ptr(&o));
                if same {
                    a.overlay.borrow_mut().take();
                }
            }
        });
        *self.overlay.borrow_mut() = Some(ov);
    }

    /// Кнопка в чате: чат прячется, экран застывает, выделенная область возвращается в чат.
    fn pick_area(self: &Rc<Self>) {
        if self.capturing.get() || self.overlay.borrow().is_some() {
            return;
        }
        self.close_floating_window();
        let me = self.clone();
        // даём окну чата исчезнуть, чтобы оно не попало в снимок
        glib::timeout_add_local_once(std::time::Duration::from_millis(220), move || me.shoot(Shot::Pick));
    }

    fn open_picker(self: &Rc<Self>, tex: gdk::Texture) {
        let ov = Overlay::open(&self.app, tex, &self.chat, true);
        let chat = self.chat.clone();
        ov.connect_pick(move |t| {
            chat.add_attachment(image::image_attachment(&t, "Фрагмент экрана"), Some(t), true);
        });
        let w = Rc::downgrade(self);
        ov.connect_closed(move || {
            if let Some(a) = w.upgrade() {
                a.overlay.borrow_mut().take();
                // и при выборе, и при отмене возвращаемся в чат
                a.show_floating();
            }
        });
        *self.overlay.borrow_mut() = Some(ov);
    }

    /// Super+C: открыть чат или закрыть, если он уже открыт.
    fn toggle_chat(self: &Rc<Self>) {
        let ov = self.overlay.borrow().clone();
        if let Some(ov) = ov {
            ov.close();
            return;
        }
        if self.floating.borrow().is_some() {
            self.close_floating_window();
            return;
        }
        self.show_floating();
    }

    // ------------------------------------------------------------------ чат в отдельном окне

    fn show_floating(self: &Rc<Self>) {
        if let Some(ov) = self.overlay.borrow().as_ref() {
            ov.win.present();
            self.chat.focus();
            return;
        }
        if let Some(w) = self.floating.borrow().as_ref() {
            w.present();
            x11::keep_above(w);
            self.chat.focus();
            return;
        }
        self.chat.on_open();
        let win = gtk::Window::new();
        win.set_application(Some(&self.app));
        win.set_title(Some("Lens"));
        win.set_decorated(false);
        win.set_resizable(false);
        win.add_css_class("lens-transparent");
        self.chat.set_floating(true);
        if let Some(p) = self.chat.root().parent() {
            if let Some(b) = p.downcast_ref::<gtk::Box>() {
                b.remove(self.chat.root());
            }
        }
        win.set_child(Some(self.chat.root()));
        let me = Rc::downgrade(self);
        win.connect_close_request(move |_| {
            if let Some(a) = me.upgrade() {
                a.close_floating_window();
            }
            glib::Propagation::Stop
        });

        // пока пользователь не передвинул окно, оно растёт вверх от нижнего края экрана
        self.float_moved.set(false);
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let moved = self.float_moved.clone();
        drag.connect_drag_begin(move |_, _, _| moved.set(true));
        self.chat.0.handle.add_controller(drag.clone());

        WidgetExt::realize(&win);
        x11::keep_above(&win);
        if let Some(surface) = win.surface() {
            let (w, moved) = (win.downgrade(), self.float_moved.clone());
            surface.connect_layout(move |_, width, height| {
                if moved.get() {
                    return;
                }
                if let Some(w) = w.upgrade() {
                    let wa = x11::workarea(&w);
                    x11::move_to(&w, wa.x() + (wa.width() - width) / 2, wa.y() + wa.height() - height - 12);
                }
            });
        }
        win.present();
        x11::keep_above(&win);
        self.chat.focus();
        *self.floating.borrow_mut() = Some(win);
    }

    fn close_floating_window(&self) {
        if let Some(w) = self.floating.borrow_mut().take() {
            if self.chat.root().parent().as_ref() == Some(w.upcast_ref()) {
                w.set_child(None::<&gtk::Widget>);
            }
            w.destroy();
            glib::idle_add_local_once(linux_common::trim_memory);
        }
    }

    /// Чат из оверлея переезжает в отдельное окно поверх остальных, оверлей закрывается.
    fn detach(self: &Rc<Self>) {
        if let Some(ov) = self.overlay.borrow_mut().take() {
            ov.destroy();
        }
        self.show_floating();
    }

    fn close_chat(self: &Rc<Self>) {
        let ov = self.overlay.borrow().clone();
        if let Some(ov) = ov {
            ov.close();
        } else {
            self.close_floating_window();
        }
    }

    // ------------------------------------------------------------------ тихий режим

    fn toggle_quiet(self: &Rc<Self>) {
        let q = self.quiet.borrow().clone();
        if let Some(q) = q {
            // повторное нажатие закрывает ответ
            q.close();
            return;
        }
        self.shoot(Shot::Quiet);
    }

    fn open_quiet(self: &Rc<Self>, tex: gdk::Texture) {
        let q = Quiet::new(&self.app);
        let w = Rc::downgrade(self);
        *q.on_continue.borrow_mut() = Some(Box::new(move |chat| {
            if let Some(a) = w.upgrade() {
                a.chat.load_chat(chat);
                a.show_floating();
            }
        }));
        let w = Rc::downgrade(self);
        *q.on_closed.borrow_mut() = Some(Box::new(move || {
            if let Some(a) = w.upgrade() {
                a.quiet.borrow_mut().take();
            }
        }));
        *self.quiet.borrow_mut() = Some(q.clone());
        let cfg = self.cfg.borrow().clone();
        q.start(tex, &cfg);
    }

    // ------------------------------------------------------------------ прочее

    fn ask_selection(self: &Rc<Self>) {
        let Some(display) = gdk::Display::default() else { return };
        let me = self.clone();
        display.primary_clipboard().read_text_async(gio::Cancellable::NONE, move |r| {
            let text = r.ok().flatten().map(|t| t.to_string()).unwrap_or_default();
            if text.trim().is_empty() {
                me.notify("Нет выделенного текста. Выделите текст мышью и попробуйте снова.");
                return;
            }
            me.show_floating();
            me.chat.add_text(text, true);
        });
    }

    fn attach_external(self: &Rc<Self>, kind: &str, path: &str) {
        let p = std::path::Path::new(path);
        match kind {
            "image" => match gdk::Texture::from_filename(p) {
                Ok(t) => {
                    self.show_floating();
                    self.chat.add_attachment(image::image_attachment(&t, "Картинка из буфера"), Some(t), false);
                }
                Err(e) => self.notify(&format!("Не удалось прикрепить: {e}")),
            },
            _ => {
                // временный файл от Clipboard History: читаем и удаляем
                if let Ok(text) = std::fs::read_to_string(p) {
                    let _ = std::fs::remove_file(p);
                    self.show_floating();
                    self.chat.add_text(text, false);
                }
            }
        }
    }

    fn show_settings(self: &Rc<Self>) {
        let chat = self.chat.clone();
        settings::open(self.cfg.clone(), Rc::new(move || chat.rebuild_actions()));
    }

    fn command(self: &Rc<Self>, args: &[String], first: bool) {
        let arg = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let has = |flag: &str| args.iter().any(|a| a == flag);
        if has("--background") {
        } else if has("--select") || has("--capture") {
            self.toggle_select();
        } else if has("--quiet") {
            self.toggle_quiet();
        } else if has("--selection") {
            self.ask_selection();
        } else if has("--settings") {
            self.show_settings();
        } else if has("--history") {
            self.show_floating();
            self.chat.open_history();
        } else if let Some(p) = arg("--attach-image") {
            self.attach_external("image", &p);
        } else if let Some(p) = arg("--attach-text-file") {
            self.attach_external("text", &p);
        } else if has("--quit") {
            self.app.quit();
        } else if has("--remove-hotkeys") {
            settings::hotkeys().remove_all(&["lens-capture"]);
            self.app.quit();
        } else if has("--chat") {
            self.toggle_chat();
        } else if first || args.is_empty() {
            self.show_floating();
        }
    }
}

fn main() -> glib::ExitCode {
    linux_common::use_x11_backend();
    glib::set_prgname(Some("lens"));
    glib::set_application_name("Lens");
    let app = adw::Application::builder()
        .application_id(settings::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let state: Rc<RefCell<Option<Rc<App>>>> = Rc::new(RefCell::new(None));

    app.connect_startup(|app| {
        linux_common::load_css(style::CSS);
        // Lens живёт в фоне и ждёт горячих клавиш
        std::mem::forget(app.hold());
        let removing = std::env::args().any(|a| a == "--remove-hotkeys");
        if !removing {
            settings::hotkeys().ensure();
        }
    });
    let st = state.clone();
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).map(|a| a.to_string_lossy().to_string()).collect();
        let first = st.borrow().is_none();
        if first {
            *st.borrow_mut() = Some(App::new(app));
        }
        let me = st.borrow().clone().unwrap();
        // запуск из меню приложений без аргументов открывает чат; повторный — тоже
        me.command(&args, first && args.is_empty());
        glib::ExitCode::SUCCESS
    });
    app.run()
}
