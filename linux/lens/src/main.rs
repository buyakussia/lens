//! Lens для Linux (GNOME): выделите часть экрана и спросите Claude Code о ней.
//!
//! Команды (их вызывают горячие клавиши GNOME; работающая копия получает их мгновенно):
//!   lens --select     выделить область и спросить (Super+C)
//!   lens --quiet      тихий вопрос по всему экрану, ответ в углу (Super+Alt+C)
//!   lens --selection  спросить про выделенный мышью текст (Super+Shift+C)
//!   lens --chat       открыть чат без выделения
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
    Select,
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
        *chat.0.hooks.borrow_mut() = chat::Hooks { close: Some(close), detach: Some(detach), settings: Some(settings) };
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
            Shot::Quiet => self.open_quiet(tex),
        }
    }

    /// GNOME спрашивает разрешение только у приложения в фокусе, поэтому показываем своё окно.
    fn ask_permission(self: &Rc<Self>, kind: Shot, err: &str) {
        eprintln!("lens: screenshot: {err}");
        let d = adw::AlertDialog::new(
            Some("Разрешите Lens снимать экран"),
            Some("Чтобы выделять область без окна скриншота GNOME, нужно один раз разрешить снимки экрана. \
                  GNOME покажет вопрос — ответьте «Разрешить»."),
        );
        d.add_response("cancel", "Не сейчас");
        d.add_response("gnome", "Окно скриншота GNOME");
        d.add_response("allow", "Разрешить");
        d.set_response_appearance("allow", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("allow"));
        let me = self.clone();
        d.connect_response(None, move |_, r| {
            let me = me.clone();
            let interactive = match r {
                "allow" => false,
                "gnome" => true,
                _ => return,
            };
            glib::spawn_future_local(async move {
                match portal::screenshot(interactive).await {
                    Ok(p) => me.got_shot(kind, &p),
                    Err(portal::ShotError::Cancelled) => {}
                    Err(_) => me.notify("GNOME не разрешил снимок экрана. Попробуйте ещё раз в настройках Lens."),
                }
            });
        });
        d.present(None::<&gtk::Widget>);
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
        let ov = Overlay::open(&self.app, tex, &self.chat);
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
        } else if has("--chat") || first || args.is_empty() {
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
