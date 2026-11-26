//! Тихий режим (Win+Alt+C): снимок всего экрана и короткий ответ в маленьком окне
//! в правом нижнем углу. Окно никогда не забирает фокус — видео продолжает играть.

use crate::chat::{copy_to_clipboard, logo};
use crate::image;
use crate::markdown::MarkdownView;
use adw::prelude::*;
use gtk::{gdk, glib};
use lens_core::claude::{self, Event, Handle};
use lens_core::config::Config;
use lens_core::history::{self, Chat, Message};
use linux_common::x11;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const QUIET_HINT: &str = "[Ответ покажется в маленьком всплывающем окне в углу экрана: максимум 2–3 коротких \
предложения, без заголовков. Пользователь не задал вопрос — скажи самое полезное о том, что сейчас на экране: \
переведи иностранный текст, объясни ошибку, ответь на видимый вопрос или кратко опиши суть.]";
const WIDTH: i32 = 400;
const AUTO_HIDE: u64 = 45;

pub struct Quiet {
    win: gtk::Window,
    card: gtk::Box,
    md: MarkdownView,
    spinner: gtk::Spinner,
    status: gtk::Label,
    text: RefCell<String>,
    render_pending: Cell<bool>,
    running: RefCell<Option<Handle>>,
    chat: RefCell<Chat>,
    hovered: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
    /// Сдвиг вправо при появлении (0 — на месте).
    slide: Cell<f64>,
    closing: Cell<bool>,
    pub on_continue: RefCell<Option<Box<dyn Fn(Chat)>>>,
    pub on_closed: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl Quiet {
    pub fn new(app: &adw::Application) -> Rc<Quiet> {
        let win = gtk::Window::new();
        win.set_application(Some(app));
        win.set_decorated(false);
        win.set_resizable(false);
        win.add_css_class("lens-transparent");

        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("lens-card");
        card.add_css_class("lens-quiet");
        card.set_size_request(WIDTH, -1);
        card.set_opacity(0.0);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        head.add_css_class("lens-quiet-head");
        head.append(&logo(16));
        let spinner = gtk::Spinner::new();
        let status = gtk::Label::new(Some("Смотрю на экран…"));
        status.add_css_class("lens-shimmer");
        status.set_xalign(0.0);
        status.set_hexpand(true);
        status.set_margin_start(4);
        head.append(&status);
        head.append(&spinner);
        let mk = |icon: &str, tip: &str| {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("flat");
            b.set_can_focus(false);
            b.set_focus_on_click(false);
            b.set_tooltip_text(Some(tip));
            b
        };
        let copy = mk("edit-copy-symbolic", "Скопировать ответ");
        let cont = mk("mail-reply-sender-symbolic", "Продолжить в чате");
        let close = mk("window-close-symbolic", "Закрыть");
        head.append(&copy);
        head.append(&cont);
        head.append(&close);
        card.append(&head);

        let md = MarkdownView::new(Rc::new(|t: &str| copy_to_clipboard(t)));
        md.widget.set_margin_start(16);
        md.widget.set_margin_end(16);
        md.widget.set_margin_bottom(14);
        md.widget.set_margin_top(4);
        let sw = gtk::ScrolledWindow::new();
        sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sw.set_propagate_natural_height(true);
        sw.set_max_content_height(300);
        sw.set_child(Some(&md.widget));
        card.append(&sw);
        win.set_child(Some(&card));

        let q = Rc::new(Quiet {
            win: win.clone(),
            card: card.clone(),
            md,
            spinner,
            status,
            text: RefCell::new(String::new()),
            render_pending: Cell::new(false),
            running: RefCell::new(None),
            chat: RefCell::new(Chat::new()),
            hovered: Cell::new(false),
            timer: RefCell::new(None),
            slide: Cell::new(1.0),
            closing: Cell::new(false),
            on_continue: RefCell::new(None),
            on_closed: RefCell::new(None),
        });

        let w = Rc::downgrade(&q);
        copy.connect_clicked(move |b| {
            if let Some(q) = w.upgrade() {
                copy_to_clipboard(&q.text.borrow());
                b.set_icon_name("object-select-symbolic");
                let b = b.clone();
                glib::timeout_add_local_once(Duration::from_millis(1200), move || b.set_icon_name("edit-copy-symbolic"));
            }
        });
        let w = Rc::downgrade(&q);
        cont.connect_clicked(move |_| {
            if let Some(q) = w.upgrade() {
                let chat = q.chat.borrow().clone();
                if let Some(f) = q.on_continue.borrow().as_ref() {
                    f(chat);
                }
                q.close();
            }
        });
        let w = Rc::downgrade(&q);
        close.connect_clicked(move |_| {
            if let Some(q) = w.upgrade() {
                q.close();
            }
        });

        // пока курсор над окном, оно не прячется
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(&q);
        motion.connect_enter(move |_, _, _| {
            if let Some(q) = w.upgrade() {
                q.hovered.set(true);
                q.cancel_timer();
            }
        });
        let w = Rc::downgrade(&q);
        motion.connect_leave(move |_| {
            if let Some(q) = w.upgrade() {
                q.hovered.set(false);
                if q.running.borrow().is_none() {
                    q.schedule_hide(12);
                }
            }
        });
        card.add_controller(motion);

        // окно без фокуса и без управления оконным менеджером; держится у правого нижнего угла
        WidgetExt::realize(&win);
        x11::make_unmanaged(&win);
        if let Some(surface) = win.surface() {
            let w = Rc::downgrade(&q);
            surface.connect_layout(move |_, _, _| {
                if let Some(q) = w.upgrade() {
                    q.place();
                }
            });
        }
        q
    }

    fn place(&self) {
        let wa = x11::workarea(&self.win);
        let (w, h) = (self.win.width(), self.win.height());
        let x = wa.x() + wa.width() - w + (self.slide.get() * 48.0) as i32;
        // не под доком: его автоскрытие выезжает поверх окон
        let y = x11::safe_bottom(&self.win, 0) - h;
        x11::move_to(&self.win, x, y);
    }

    pub fn start(self: &Rc<Self>, tex: gdk::Texture, cfg: &Config) {
        self.place();
        self.win.present();
        self.place();
        self.spinner.start();
        let q = self.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            q.card.set_opacity(v);
            q.slide.set(1.0 - v);
            q.place();
        });
        let anim = adw::TimedAnimation::new(&self.card, 0.0, 1.0, 280, target);
        anim.set_easing(adw::Easing::EaseOutCubic);
        anim.play();

        let att = image::image_attachment(&tex, "Снимок экрана");
        drop(tex);
        {
            let mut chat = self.chat.borrow_mut();
            chat.title = "Тихий вопрос по экрану".into();
            chat.messages.push(Message {
                role: "user".into(),
                text: String::new(),
                attachments: vec!["Снимок экрана".into()],
                error: false,
            });
        }
        let req = claude::Request {
            question: String::new(),
            attachments: vec![att],
            model: if cfg.quiet_model.is_empty() { cfg.model.clone() } else { cfg.quiet_model.clone() },
            system_prompt: cfg.system_prompt.clone(),
            style_hint: QUIET_HINT.into(),
            session: None,
            timeout: Duration::from_secs(cfg.timeout_secs),
        };
        let (tx, rx) = async_channel::unbounded::<Event>();
        match claude::ask(req, &history::work_dir(), move |ev| {
            let _ = tx.send_blocking(ev);
        }) {
            Ok(h) => {
                *self.running.borrow_mut() = Some(h);
                let q = self.clone();
                glib::spawn_future_local(async move {
                    while let Ok(ev) = rx.recv().await {
                        q.on_event(ev);
                    }
                });
            }
            Err(e) => self.finish(None, Some(e)),
        }
    }

    fn on_event(self: &Rc<Self>, ev: Event) {
        match ev {
            Event::Delta(t) => {
                self.text.borrow_mut().push_str(&t);
                self.status.set_text("Lens");
                if !self.render_pending.replace(true) {
                    let q = self.clone();
                    glib::timeout_add_local_once(Duration::from_millis(33), move || {
                        q.render_pending.set(false);
                        q.md.set_text(&q.text.borrow());
                    });
                }
            }
            Event::Tool(n) => self.status.set_text(claude::tool_status(&n)),
            Event::Session(s) => self.chat.borrow_mut().session = Some(s),
            Event::Done(out) => {
                if let Some(s) = &out.session {
                    self.chat.borrow_mut().session = Some(s.clone());
                }
                let err = out.error_text();
                self.finish(out.result, err);
            }
        }
    }

    fn finish(self: &Rc<Self>, result: Option<String>, error: Option<String>) {
        self.running.borrow_mut().take();
        self.spinner.stop();
        self.spinner.set_visible(false);
        let mut text = self.text.borrow().clone();
        if text.trim().is_empty() {
            text = result.unwrap_or_default();
        }
        let is_error = text.trim().is_empty() && error.is_some();
        if is_error {
            text = format!("⚠ {}", error.unwrap_or_default());
            self.md.widget.add_css_class("lens-error");
        }
        *self.text.borrow_mut() = text.clone();
        self.md.set_text(&text);
        self.status.set_text("Lens");
        {
            let mut chat = self.chat.borrow_mut();
            chat.messages.push(Message { role: "assistant".into(), text, attachments: vec![], error: is_error });
            chat.updated = lens_core::now();
            let _ = history::save(&chat);
        }
        if !self.hovered.get() {
            self.schedule_hide(AUTO_HIDE);
        }
    }

    fn cancel_timer(&self) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
    }

    fn schedule_hide(self: &Rc<Self>, secs: u64) {
        self.cancel_timer();
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_secs(secs), move || {
            if let Some(q) = w.upgrade() {
                q.timer.borrow_mut().take();
                q.close();
            }
        });
        *self.timer.borrow_mut() = Some(id);
    }

    pub fn close(self: &Rc<Self>) {
        if self.closing.replace(true) {
            return;
        }
        self.cancel_timer();
        if let Some(h) = self.running.borrow_mut().take() {
            h.stop();
        }
        let q = self.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            q.card.set_opacity(v);
            q.slide.set((1.0 - v) * 0.5);
            q.place();
        });
        let anim = adw::TimedAnimation::new(&self.card, self.card.opacity(), 0.0, 180, target);
        anim.set_easing(adw::Easing::EaseInCubic);
        let q = self.clone();
        anim.connect_done(move |_| {
            q.win.destroy();
            if let Some(cb) = q.on_closed.borrow_mut().take() {
                cb();
            }
            glib::idle_add_local_once(linux_common::trim_memory);
        });
        anim.play();
    }
}
