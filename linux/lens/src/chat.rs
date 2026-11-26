//! Панель чата в стиле быстрой панели Claude: поле ввода, вложения, выбор модели прямо в строке,
//! потоковый ответ над полем ввода. Один и тот же виджет живёт то в оверлее выделения,
//! то в отдельном плавающем окне.

use crate::image;
use crate::markdown::{CopyFn, MarkdownView};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use lens_core::claude::{self, Attachment, Event, Handle};
use lens_core::config::{self, Config};
use lens_core::history::{self, Chat, Message};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// Конкретная длина ответа: «кратко» Claude понимает слишком мягко.
const BRIEF: &str = "[Длина ответа: 1–4 предложения или короткий список до 5 пунктов. Без вступлений, \
повторов вопроса и лишних пояснений. Подробнее — только если пользователь прямо попросит.]";

/// Через сколько минут бездействия следующий вызов начинает новый чат.
const FRESH_CHAT_AFTER: f64 = 10.0 * 60.0;

pub struct Att {
    pub att: Attachment,
    pub thumb: Option<gdk::Texture>,
    /// Фрагмент экрана из текущего выделения (заменяется при новом выделении).
    pub fragment: bool,
    chip: gtk::Widget,
}

struct Answer {
    view: MarkdownView,
    status_box: gtk::Box,
    status: gtk::Label,
    text: String,
    render_pending: bool,
}

#[derive(Default)]
pub struct Hooks {
    pub close: Option<Rc<dyn Fn()>>,
    pub detach: Option<Rc<dyn Fn()>>,
    pub settings: Option<Rc<dyn Fn()>>,
}

pub struct Inner {
    pub root: gtk::Box,
    pub handle: gtk::WindowHandle,
    head_rev: gtk::Revealer,
    title: gtk::Label,
    detach_btn: gtk::Button,
    history_btn: gtk::MenuButton,
    msgs_rev: gtk::Revealer,
    msgs: gtk::Box,
    stick_bottom: Cell<bool>,
    actions_rev: gtk::Revealer,
    actions_box: gtk::Box,
    atts_rev: gtk::Revealer,
    atts_box: gtk::Box,
    pub entry: gtk::TextView,
    placeholder: gtk::Label,
    model_btn: gtk::MenuButton,
    send_btn: gtk::Button,
    chat: RefCell<Chat>,
    atts: RefCell<Vec<Att>>,
    running: RefCell<Option<Handle>>,
    answer: RefCell<Option<Answer>>,
    pub cfg: Rc<RefCell<Config>>,
    pub hooks: RefCell<Hooks>,
    floating: Cell<bool>,
    last_activity: Cell<f64>,
}

#[derive(Clone)]
pub struct ChatView(pub Rc<Inner>);

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.add_css_class("circular");
    b.set_tooltip_text(Some(tip));
    b.set_valign(gtk::Align::Center);
    b
}

fn revealer(child: &impl IsA<gtk::Widget>, kind: gtk::RevealerTransitionType, ms: u32) -> gtk::Revealer {
    let r = gtk::Revealer::new();
    r.set_transition_type(kind);
    r.set_transition_duration(ms);
    r.set_child(Some(child));
    r
}

pub fn copy_to_clipboard(text: &str) {
    if let Some(d) = gdk::Display::default() {
        d.clipboard().set_text(text);
    }
}

/// Маленький логотип: линза с искрой.
pub fn logo(size: i32) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(size);
    a.set_content_height(size);
    a.set_valign(gtk::Align::Center);
    a.set_draw_func(|_, cr, w, h| {
        let (r, g, b) = crate::style::ACCENT;
        let s = w.min(h) as f64;
        let c = s / 2.0;
        cr.set_source_rgb(r as f64, g as f64, b as f64);
        cr.set_line_width(s * 0.12);
        cr.arc(c, c, s * 0.36, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        // искра в центре
        let k = s * 0.17;
        cr.move_to(c, c - k);
        cr.curve_to(c + k * 0.15, c - k * 0.15, c + k * 0.15, c - k * 0.15, c + k, c);
        cr.curve_to(c + k * 0.15, c + k * 0.15, c + k * 0.15, c + k * 0.15, c, c + k);
        cr.curve_to(c - k * 0.15, c + k * 0.15, c - k * 0.15, c + k * 0.15, c - k, c);
        cr.curve_to(c - k * 0.15, c - k * 0.15, c - k * 0.15, c - k * 0.15, c, c - k);
        let _ = cr.fill();
    });
    a
}

impl ChatView {
    pub fn new(cfg: Rc<RefCell<Config>>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("lens-card");
        root.set_width_request(680);
        root.set_overflow(gtk::Overflow::Hidden);

        // --- заголовок (виден, когда есть разговор или окно откреплено)
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        head.add_css_class("lens-head");
        head.append(&logo(18));
        let title = gtk::Label::new(Some("Lens"));
        title.add_css_class("lens-title");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        title.set_margin_start(6);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        head.append(&title);
        let history_btn = gtk::MenuButton::new();
        history_btn.set_icon_name("document-open-recent-symbolic");
        history_btn.add_css_class("flat");
        history_btn.add_css_class("circular");
        history_btn.set_tooltip_text(Some("История чатов (Ctrl+H)"));
        head.append(&history_btn);
        let new_btn = icon_button("list-add-symbolic", "Новый чат (Ctrl+N)");
        head.append(&new_btn);
        let settings_btn = icon_button("emblem-system-symbolic", "Настройки");
        head.append(&settings_btn);
        let detach_btn = icon_button("view-pin-symbolic", "Открепить окно и продолжить работу");
        head.append(&detach_btn);
        let close_btn = icon_button("window-close-symbolic", "Закрыть (Esc)");
        head.append(&close_btn);
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&head));
        let head_rev = revealer(&handle, gtk::RevealerTransitionType::SlideDown, 220);
        root.append(&head_rev);

        // --- сообщения
        let msgs = gtk::Box::new(gtk::Orientation::Vertical, 14);
        msgs.add_css_class("lens-messages");
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(460);
        scroller.set_child(Some(&msgs));
        let msgs_rev = revealer(&scroller, gtk::RevealerTransitionType::SlideUp, 260);
        root.append(&msgs_rev);

        // --- быстрые действия и вложения
        let actions_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions_box.add_css_class("lens-actions");
        actions_box.set_margin_start(14);
        actions_box.set_margin_end(14);
        actions_box.set_margin_top(10);
        let actions_rev = revealer(&actions_box, gtk::RevealerTransitionType::SlideDown, 180);
        root.append(&actions_rev);

        let atts_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        atts_box.set_margin_start(12);
        atts_box.set_margin_end(12);
        atts_box.set_margin_top(10);
        let atts_scroll = gtk::ScrolledWindow::new();
        atts_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        atts_scroll.set_propagate_natural_height(true);
        atts_scroll.set_child(Some(&atts_box));
        let atts_rev = revealer(&atts_scroll, gtk::RevealerTransitionType::SlideDown, 200);
        root.append(&atts_rev);

        // --- строка ввода: [+] [текст] [модель ⌄] [↑]
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("lens-input-row");
        let attach_btn = icon_button("list-add-symbolic", "Прикрепить файл (или перетащите / вставьте Ctrl+V)");
        attach_btn.set_valign(gtk::Align::End);
        attach_btn.add_css_class("lens-round");
        row.append(&attach_btn);

        let entry = gtk::TextView::new();
        entry.add_css_class("lens-entry");
        entry.set_wrap_mode(gtk::WrapMode::WordChar);
        entry.set_accepts_tab(false);
        entry.set_top_margin(6);
        entry.set_bottom_margin(6);
        entry.set_left_margin(4);
        entry.set_right_margin(4);
        let placeholder = gtk::Label::new(Some("Спросите что угодно…"));
        placeholder.add_css_class("lens-placeholder");
        placeholder.set_halign(gtk::Align::Start);
        placeholder.set_valign(gtk::Align::Start);
        placeholder.set_margin_start(5);
        placeholder.set_margin_top(6);
        placeholder.set_can_target(false);
        let entry_overlay = gtk::Overlay::new();
        entry_overlay.set_child(Some(&entry));
        entry_overlay.add_overlay(&placeholder);
        entry_overlay.set_valign(gtk::Align::End);
        let entry_scroll = gtk::ScrolledWindow::new();
        entry_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        entry_scroll.set_propagate_natural_height(true);
        entry_scroll.set_max_content_height(160);
        entry_scroll.set_hexpand(true);
        entry_scroll.set_valign(gtk::Align::End);
        entry_scroll.set_child(Some(&entry_overlay));
        row.append(&entry_scroll);

        let model_btn = gtk::MenuButton::new();
        model_btn.add_css_class("lens-model");
        model_btn.set_valign(gtk::Align::End);
        model_btn.set_margin_bottom(2);
        model_btn.set_always_show_arrow(true);
        model_btn.set_tooltip_text(Some("Модель"));
        row.append(&model_btn);

        let send_btn = gtk::Button::from_icon_name("go-up-symbolic");
        send_btn.add_css_class("lens-round");
        send_btn.add_css_class("lens-send");
        send_btn.set_valign(gtk::Align::End);
        send_btn.set_tooltip_text(Some("Отправить (Enter)"));
        row.append(&send_btn);
        root.append(&row);

        let view = ChatView(Rc::new(Inner {
            root,
            handle,
            head_rev,
            title,
            detach_btn: detach_btn.clone(),
            history_btn: history_btn.clone(),
            msgs_rev,
            msgs,
            stick_bottom: Cell::new(true),
            actions_rev,
            actions_box,
            atts_rev,
            atts_box,
            entry: entry.clone(),
            placeholder,
            model_btn: model_btn.clone(),
            send_btn: send_btn.clone(),
            chat: RefCell::new(Chat::new()),
            atts: RefCell::new(vec![]),
            running: RefCell::new(None),
            answer: RefCell::new(None),
            cfg,
            hooks: RefCell::new(Hooks::default()),
            floating: Cell::new(false),
            last_activity: Cell::new(lens_core::now()),
        }));

        // автопрокрутка вниз, пока пользователь сам не прокрутил вверх
        let adj = scroller.vadjustment();
        let v = view.clone();
        adj.connect_value_changed(move |a| {
            v.0.stick_bottom.set(a.value() + a.page_size() >= a.upper() - 24.0);
        });
        let v = view.clone();
        adj.connect_changed(move |a| {
            if v.0.stick_bottom.get() {
                a.set_value(a.upper() - a.page_size());
            }
        });

        let v = view.clone();
        entry.buffer().connect_changed(move |_| v.update_controls());
        let v = view.clone();
        send_btn.connect_clicked(move |_| {
            if v.0.running.borrow().is_some() {
                v.stop();
            } else {
                v.send(None);
            }
        });
        let v = view.clone();
        attach_btn.connect_clicked(move |_| v.pick_files());
        let v = view.clone();
        new_btn.connect_clicked(move |_| v.new_chat());
        let v = view.clone();
        close_btn.connect_clicked(move |_| v.request_close());
        let v = view.clone();
        detach_btn.connect_clicked(move |_| {
            let hook = v.0.hooks.borrow().detach.clone();
            if let Some(h) = hook {
                h();
            }
        });
        let v = view.clone();
        settings_btn.connect_clicked(move |_| {
            let hook = v.0.hooks.borrow().settings.clone();
            if let Some(h) = hook {
                h();
            }
        });

        view.setup_keys();
        view.setup_paste_and_drop();
        view.setup_history();
        view.rebuild_model_menu();
        view.rebuild_actions();
        view.update_controls();
        view
    }

    pub fn root(&self) -> &gtk::Box {
        &self.0.root
    }

    // ------------------------------------------------------------------ режимы

    /// В оверлее заголовок появляется только с разговором; в отдельном окне — всегда.
    pub fn set_floating(&self, floating: bool) {
        self.0.floating.set(floating);
        self.0.detach_btn.set_visible(!floating);
        if floating {
            self.0.root.add_css_class("floating");
        } else {
            self.0.root.remove_css_class("floating");
        }
        self.update_controls();
    }

    /// Вызывается при каждом открытии: после долгого перерыва начинаем новый чат.
    pub fn on_open(&self) {
        let idle = lens_core::now() - self.0.last_activity.get();
        if idle > FRESH_CHAT_AFTER && self.0.running.borrow().is_none() && !self.0.chat.borrow().messages.is_empty() {
            self.new_chat();
        }
        self.0.last_activity.set(lens_core::now());
        self.rebuild_actions();
        self.rebuild_model_menu();
    }

    pub fn focus(&self) {
        self.0.entry.grab_focus();
    }

    fn request_close(&self) {
        let hook = self.0.hooks.borrow().close.clone();
        if let Some(h) = hook {
            h();
        }
    }

    fn text(&self) -> String {
        let b = self.0.entry.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }

    fn update_controls(&self) {
        let i = &self.0;
        let text_empty = self.text().trim().is_empty();
        let has_atts = !i.atts.borrow().is_empty();
        let running = i.running.borrow().is_some();
        let has_msgs = !i.chat.borrow().messages.is_empty();
        i.placeholder.set_visible(text_empty);
        let has_fragment = i.atts.borrow().iter().any(|a| a.fragment);
        i.placeholder.set_text(if has_fragment {
            "Спросите об этом фрагменте…"
        } else if has_atts {
            "Спросите о вложении…"
        } else if has_msgs {
            "Ответить…"
        } else if !i.floating.get() {
            "Выделите область экрана или спросите что угодно…"
        } else {
            "Спросите что угодно…"
        });
        i.atts_rev.set_reveal_child(has_atts);
        i.actions_rev.set_reveal_child(has_atts && text_empty && !running && !i.cfg.borrow().actions.is_empty());
        i.head_rev.set_reveal_child(has_msgs || i.floating.get());
        i.msgs_rev.set_reveal_child(has_msgs);
        if running {
            i.send_btn.set_icon_name("media-playback-stop-symbolic");
            i.send_btn.add_css_class("stop");
            i.send_btn.set_tooltip_text(Some("Остановить"));
            i.send_btn.set_sensitive(true);
        } else {
            i.send_btn.set_icon_name("go-up-symbolic");
            i.send_btn.remove_css_class("stop");
            i.send_btn.set_tooltip_text(Some("Отправить (Enter)"));
            i.send_btn.set_sensitive(!text_empty || has_atts);
        }
        let title = i.chat.borrow().title.clone();
        i.title.set_text(if title.is_empty() { "Lens" } else { &title });
    }

    // ------------------------------------------------------------------ модель

    fn rebuild_model_menu(&self) {
        let current = self.0.cfg.borrow().model.clone();
        let label = config::model_label(&current);
        self.0.model_btn.set_label(&label);
        let pop = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let mut ids: Vec<(String, String)> =
            config::MODELS.iter().map(|(m, l)| (m.to_string(), l.to_string())).collect();
        if !ids.iter().any(|(m, _)| *m == current) {
            ids.push((current.clone(), current.clone()));
        }
        for (id, name) in ids {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let l = gtk::Label::new(Some(&name));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            row.append(&l);
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.set_opacity(if id == current { 1.0 } else { 0.0 });
            row.append(&check);
            b.set_child(Some(&row));
            let v = self.clone();
            let p = pop.clone();
            b.connect_clicked(move |_| {
                {
                    let mut c = v.0.cfg.borrow_mut();
                    c.model = id.clone();
                    let _ = config::save(&c);
                }
                p.popdown();
                v.rebuild_model_menu();
            });
            list.append(&b);
        }
        let hint = gtk::Label::new(Some("Авто — модель по умолчанию в Claude Code"));
        hint.add_css_class("dim-label");
        hint.add_css_class("caption");
        hint.set_margin_top(4);
        hint.set_margin_start(8);
        hint.set_margin_end(8);
        list.append(&hint);
        pop.set_child(Some(&list));
        self.0.model_btn.set_popover(Some(&pop));
    }

    // ------------------------------------------------------------------ быстрые действия

    pub fn rebuild_actions(&self) {
        let b = &self.0.actions_box;
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
        for (n, act) in self.0.cfg.borrow().actions.iter().take(6).enumerate() {
            let btn = gtk::Button::with_label(&act.name);
            btn.set_tooltip_text(Some(&format!("{} (Alt+{})", act.prompt, n + 1)));
            btn.set_can_focus(false);
            let v = self.clone();
            let prompt = act.prompt.clone();
            btn.connect_clicked(move |_| v.send(Some(prompt.clone())));
            b.append(&btn);
        }
    }

    pub fn run_action(&self, n: usize) {
        let p = self.0.cfg.borrow().actions.get(n).map(|a| a.prompt.clone());
        if let Some(p) = p {
            self.send(Some(p));
        }
    }

    // ------------------------------------------------------------------ вложения

    pub fn add_attachment(&self, att: Attachment, thumb: Option<gdk::Texture>, fragment: bool) {
        if fragment {
            self.remove_fragment();
        }
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        chip.add_css_class("lens-chip");
        if let Some(t) = &thumb {
            let pic = gtk::Picture::for_paintable(t);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_size_request(44, 44);
            pic.set_can_shrink(true);
            let frame = gtk::AspectFrame::new(0.5, 0.5, 1.0, false);
            frame.set_child(Some(&pic));
            frame.set_size_request(44, 44);
            chip.append(&frame);
        } else {
            let icon = gtk::Image::from_icon_name(match &att {
                Attachment::Pdf { .. } => "x-office-document-symbolic",
                Attachment::Text { quote: true, .. } => "format-text-bold-symbolic",
                Attachment::Text { .. } => "text-x-generic-symbolic",
                _ => "folder-documents-symbolic",
            });
            icon.add_css_class("lens-chip-icon");
            chip.append(&icon);
        }
        let label = gtk::Label::new(Some(att.label()));
        label.set_max_width_chars(22);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        chip.append(&label);
        let x = gtk::Button::from_icon_name("window-close-symbolic");
        x.add_css_class("flat");
        x.add_css_class("lens-chip-x");
        x.set_valign(gtk::Align::Center);
        x.set_can_focus(false);
        x.set_tooltip_text(Some("Убрать"));
        let chip_w: gtk::Widget = chip.clone().upcast();
        let v = self.clone();
        let cw = chip_w.clone();
        x.connect_clicked(move |_| v.remove_chip(&cw));
        chip.append(&x);
        self.0.atts_box.append(&chip);
        self.0.atts.borrow_mut().push(Att { att, thumb, fragment, chip: chip_w });
        self.update_controls();
    }

    fn remove_chip(&self, chip: &gtk::Widget) {
        self.0.atts.borrow_mut().retain(|a| &a.chip != chip);
        self.0.atts_box.remove(chip);
        self.update_controls();
    }

    pub fn remove_fragment(&self) {
        let chips: Vec<gtk::Widget> =
            self.0.atts.borrow().iter().filter(|a| a.fragment).map(|a| a.chip.clone()).collect();
        for c in chips {
            self.remove_chip(&c);
        }
    }

    fn clear_attachments(&self) -> Vec<Att> {
        let atts: Vec<Att> = self.0.atts.borrow_mut().drain(..).collect();
        for a in &atts {
            self.0.atts_box.remove(&a.chip);
        }
        atts
    }

    pub fn add_files(&self, files: Vec<gio::File>) {
        for f in files {
            let Some(path) = f.path() else { continue };
            match image::from_file(&path) {
                Ok((att, thumb)) => self.add_attachment(att, thumb, false),
                Err(e) => self.show_error(&format!("Не удалось прикрепить: {e}")),
            }
        }
    }

    pub fn add_text(&self, text: String, quote: bool) {
        let first: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect();
        let label = if quote { format!("«{first}…»") } else { format!("Текст: {first}…") };
        self.add_attachment(Attachment::Text { text, label, quote }, None, false);
    }

    fn pick_files(&self) {
        let dialog = gtk::FileDialog::builder().title("Прикрепить файлы").modal(true).build();
        let parent = self.0.root.root().and_downcast::<gtk::Window>();
        let v = self.clone();
        dialog.open_multiple(parent.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Ok(list) = res {
                let files: Vec<gio::File> = (0..list.n_items()).filter_map(|i| list.item(i).and_downcast()).collect();
                v.add_files(files);
            }
            v.focus();
        });
    }

    fn setup_paste_and_drop(&self) {
        // Ctrl+V с картинкой или файлами прикрепляет их, а не вставляет текст
        let v = self.clone();
        self.0.entry.connect_paste_clipboard(move |tv| {
            let cb = tv.clipboard();
            let formats = cb.formats();
            if formats.contains_type(gdk::FileList::static_type()) {
                tv.stop_signal_emission_by_name("paste-clipboard");
                let v = v.clone();
                cb.read_value_async(gdk::FileList::static_type(), glib::Priority::DEFAULT, gio::Cancellable::NONE, move |r| {
                    if let Ok(val) = r {
                        if let Ok(fl) = val.get::<gdk::FileList>() {
                            v.add_files(fl.files());
                        }
                    }
                });
            } else if formats.contains_type(gdk::Texture::static_type()) && !formats.contain_mime_type("text/plain") {
                tv.stop_signal_emission_by_name("paste-clipboard");
                let v = v.clone();
                cb.read_texture_async(gio::Cancellable::NONE, move |r| {
                    if let Ok(Some(tex)) = r {
                        v.add_attachment(image::image_attachment(&tex, "Картинка из буфера"), Some(tex), false);
                    }
                });
            }
        });

        let drop = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        drop.set_types(&[gdk::FileList::static_type(), gdk::Texture::static_type()]);
        let v = self.clone();
        drop.connect_drop(move |_, value, _, _| {
            if let Ok(fl) = value.get::<gdk::FileList>() {
                v.add_files(fl.files());
                return true;
            }
            if let Ok(tex) = value.get::<gdk::Texture>() {
                v.add_attachment(image::image_attachment(&tex, "Картинка"), Some(tex), false);
                return true;
            }
            false
        });
        self.0.root.add_controller(drop);
    }

    // ------------------------------------------------------------------ клавиатура

    fn setup_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let v = self.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            use gdk::Key;
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = mods.contains(gdk::ModifierType::ALT_MASK);
            match key {
                Key::Return | Key::KP_Enter if !shift && !ctrl => {
                    if v.0.running.borrow().is_none() {
                        v.send(None);
                    }
                    glib::Propagation::Stop
                }
                Key::Escape => {
                    if v.0.history_btn.is_active() {
                        return glib::Propagation::Proceed;
                    }
                    v.request_close();
                    glib::Propagation::Stop
                }
                Key::n | Key::N if ctrl => {
                    v.new_chat();
                    glib::Propagation::Stop
                }
                Key::h | Key::H if ctrl => {
                    v.0.history_btn.popup();
                    glib::Propagation::Stop
                }
                _ if alt => {
                    if let Some(d) = key.to_unicode().and_then(|c| c.to_digit(10)) {
                        if d >= 1 {
                            v.run_action(d as usize - 1);
                            return glib::Propagation::Stop;
                        }
                    }
                    glib::Propagation::Proceed
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.0.root.add_controller(keys);
    }

    // ------------------------------------------------------------------ сообщения

    fn user_bubble(&self, text: &str, atts: &[Att], labels: &[String]) -> gtk::Widget {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        col.set_halign(gtk::Align::End);
        col.set_margin_start(80);
        let thumbs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        thumbs.set_halign(gtk::Align::End);
        for a in atts {
            if let Some(t) = &a.thumb {
                let pic = gtk::Picture::for_paintable(t);
                pic.set_content_fit(gtk::ContentFit::Contain);
                pic.set_can_shrink(true);
                let k = (t.width() as f64 / t.height().max(1) as f64).clamp(0.3, 4.0);
                let h = 96.0f64.min(t.height() as f64);
                pic.set_size_request((h * k).min(300.0) as i32, h as i32);
                pic.add_css_class("card");
                thumbs.append(&pic);
            }
        }
        if thumbs.first_child().is_some() {
            col.append(&thumbs);
        }
        let other: Vec<&String> = labels
            .iter()
            .filter(|l| !atts.iter().any(|a| a.thumb.is_some() && a.att.label() == l.as_str()))
            .collect();
        if !other.is_empty() {
            let l = gtk::Label::new(Some(&format!(
                "📎 {}",
                other.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            )));
            l.set_xalign(1.0);
            l.set_wrap(true);
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            b.add_css_class("lens-msg-atts");
            b.set_halign(gtk::Align::End);
            b.append(&l);
            col.append(&b);
        }
        if !text.trim().is_empty() {
            let bubble = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            bubble.add_css_class("lens-user");
            bubble.set_halign(gtk::Align::End);
            let l = gtk::Label::new(Some(text.trim()));
            l.set_wrap(true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            l.set_xalign(0.0);
            l.set_selectable(true);
            l.set_can_focus(false);
            bubble.append(&l);
            col.append(&bubble);
        }
        col.upcast()
    }

    fn copy_fn() -> CopyFn {
        Rc::new(|t: &str| copy_to_clipboard(t))
    }

    fn assistant_widget(&self, text: &str, error: bool) -> (gtk::Box, MarkdownView, gtk::Box, gtk::Label) {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let spinner = gtk::Spinner::new();
        spinner.start();
        status_box.append(&spinner);
        let status = gtk::Label::new(Some("Думает…"));
        status.add_css_class("lens-status");
        status_box.append(&status);
        status_box.set_visible(false);
        col.append(&status_box);
        let view = MarkdownView::new(Self::copy_fn());
        if error {
            view.widget.add_css_class("lens-error");
        }
        view.set_text(text);
        col.append(&view.widget);
        (col, view, status_box, status)
    }

    fn append_msg(&self, w: &impl IsA<gtk::Widget>) {
        self.0.stick_bottom.set(true);
        self.0.msgs.append(w);
    }

    pub fn show_error(&self, text: &str) {
        let (w, _, _, _) = self.assistant_widget(&format!("⚠ {text}"), true);
        self.append_msg(&w);
        self.0.msgs_rev.set_reveal_child(true);
    }

    pub fn new_chat(&self) {
        self.stop();
        *self.0.chat.borrow_mut() = Chat::new();
        *self.0.answer.borrow_mut() = None;
        while let Some(c) = self.0.msgs.first_child() {
            self.0.msgs.remove(&c);
        }
        self.update_controls();
        self.focus();
    }

    /// Загружает чат из истории (продолжение через сессию Claude Code).
    pub fn load_chat(&self, chat: Chat) {
        self.new_chat();
        for m in &chat.messages {
            if m.role == "user" {
                let w = self.user_bubble(&m.text, &[], &m.attachments);
                self.append_msg(&w);
            } else {
                let (w, _, _, _) = self.assistant_widget(&m.text, m.error);
                self.append_msg(&w);
            }
        }
        *self.0.chat.borrow_mut() = chat;
        self.0.last_activity.set(lens_core::now());
        self.update_controls();
        self.focus();
    }

    // ------------------------------------------------------------------ отправка

    pub fn send(&self, override_prompt: Option<String>) {
        let i = &self.0;
        if i.running.borrow().is_some() {
            return;
        }
        let typed = self.text();
        let question = match &override_prompt {
            Some(p) if !typed.trim().is_empty() => format!("{p}\n\n{}", typed.trim()),
            Some(p) => p.clone(),
            None => typed.trim().to_string(),
        };
        if question.is_empty() && i.atts.borrow().is_empty() {
            return;
        }
        let atts = self.clear_attachments();
        let labels: Vec<String> = atts.iter().map(|a| a.att.label().to_string()).collect();
        i.entry.buffer().set_text("");

        let shown = if override_prompt.is_some() {
            // для быстрого действия показываем его название, а не длинный промпт
            let name = i.cfg.borrow().actions.iter().find(|a| Some(&a.prompt) == override_prompt.as_ref()).map(|a| a.name.clone());
            match name {
                Some(n) if typed.trim().is_empty() => n,
                Some(n) => format!("{n}: {}", typed.trim()),
                None => question.clone(),
            }
        } else {
            question.clone()
        };
        let bubble = self.user_bubble(&shown, &atts, &labels);
        self.append_msg(&bubble);
        {
            let mut chat = i.chat.borrow_mut();
            if chat.messages.is_empty() {
                chat.title = Chat::make_title(&shown, &labels);
            }
            chat.messages.push(Message { role: "user".into(), text: shown, attachments: labels, error: false });
            chat.updated = lens_core::now();
        }

        let (w, view, status_box, status) = self.assistant_widget("", false);
        status_box.set_visible(true);
        self.append_msg(&w);
        *i.answer.borrow_mut() = Some(Answer { view, status_box, status, text: String::new(), render_pending: false });

        let cfg = i.cfg.borrow().clone();
        let req = claude::Request {
            question,
            attachments: atts.into_iter().map(|a| a.att).collect(),
            model: cfg.model.clone(),
            system_prompt: cfg.system_prompt.clone(),
            style_hint: BRIEF.into(),
            session: i.chat.borrow().session.clone(),
            timeout: Duration::from_secs(cfg.timeout_secs),
        };
        let (tx, rx) = async_channel::unbounded::<Event>();
        match claude::ask(req, &history::work_dir(), move |ev| {
            let _ = tx.send_blocking(ev);
        }) {
            Ok(h) => {
                *i.running.borrow_mut() = Some(h);
                let v = self.clone();
                glib::spawn_future_local(async move {
                    while let Ok(ev) = rx.recv().await {
                        v.on_event(ev);
                    }
                });
            }
            Err(e) => self.finish(None, Some(e)),
        }
        i.last_activity.set(lens_core::now());
        self.update_controls();
        self.focus();
    }

    pub fn stop(&self) {
        let h = self.0.running.borrow().clone();
        if let Some(h) = h {
            h.stop();
        }
    }

    fn on_event(&self, ev: Event) {
        match ev {
            Event::Delta(t) => {
                let mut a = self.0.answer.borrow_mut();
                let Some(ans) = a.as_mut() else { return };
                ans.text.push_str(&t);
                ans.status_box.set_visible(false);
                if !ans.render_pending {
                    ans.render_pending = true;
                    let v = self.clone();
                    // не чаще ~30 раз в секунду: плавно и без лишней нагрузки
                    glib::timeout_add_local_once(Duration::from_millis(33), move || v.render_answer());
                }
            }
            Event::Tool(name) => {
                if let Some(ans) = self.0.answer.borrow().as_ref() {
                    ans.status.set_text(claude::tool_status(&name));
                    ans.status_box.set_visible(true);
                }
            }
            Event::Session(s) => self.0.chat.borrow_mut().session = Some(s),
            Event::Done(out) => {
                if let Some(s) = &out.session {
                    self.0.chat.borrow_mut().session = Some(s.clone());
                }
                let err = out.error_text();
                self.finish(out.result, err);
            }
        }
    }

    fn render_answer(&self) {
        let mut a = self.0.answer.borrow_mut();
        if let Some(ans) = a.as_mut() {
            ans.render_pending = false;
            ans.view.set_text(&ans.text);
        }
    }

    fn finish(&self, result: Option<String>, error: Option<String>) {
        self.0.running.borrow_mut().take();
        let ans = self.0.answer.borrow_mut().take();
        let mut text = ans.as_ref().map(|a| a.text.clone()).unwrap_or_default();
        if text.trim().is_empty() {
            text = result.unwrap_or_default();
        }
        let is_error = error.is_some() && text.trim().is_empty();
        if is_error {
            text = format!("⚠ {}", error.unwrap_or_default());
        }
        if let Some(a) = ans {
            a.status_box.set_visible(false);
            if is_error {
                a.view.widget.add_css_class("lens-error");
            }
            a.view.set_text(if text.trim().is_empty() { "_Остановлено._" } else { &text });
        }
        {
            let mut chat = self.0.chat.borrow_mut();
            if !text.trim().is_empty() {
                chat.messages.push(Message { role: "assistant".into(), text, attachments: vec![], error: is_error });
            }
            chat.updated = lens_core::now();
            let _ = history::save(&chat);
        }
        self.0.last_activity.set(lens_core::now());
        self.update_controls();
    }

    // ------------------------------------------------------------------ история

    fn setup_history(&self) {
        let pop = gtk::Popover::new();
        pop.set_size_request(380, -1);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Поиск по чатам"));
        col.append(&search);
        let list = gtk::ListBox::new();
        list.add_css_class("lens-history");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_activate_on_single_click(true);
        let sw = gtk::ScrolledWindow::new();
        sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sw.set_propagate_natural_height(true);
        sw.set_max_content_height(380);
        sw.set_child(Some(&list));
        col.append(&sw);
        let empty = gtk::Label::new(Some("Пока нет сохранённых чатов"));
        empty.add_css_class("dim-label");
        empty.set_margin_top(12);
        empty.set_margin_bottom(12);
        col.append(&empty);
        pop.set_child(Some(&col));
        self.0.history_btn.set_popover(Some(&pop));

        let chats: Rc<RefCell<Vec<Chat>>> = Rc::new(RefCell::new(vec![]));
        let fill = {
            let (list, chats, search, empty, pop) = (list.clone(), chats.clone(), search.clone(), empty.clone(), pop.clone());
            let v = self.clone();
            Rc::new(move || {
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                let q = search.text().to_lowercase();
                let all = chats.borrow();
                let mut n = 0;
                for chat in all.iter() {
                    let hay = format!(
                        "{} {}",
                        chat.title.to_lowercase(),
                        chat.messages.iter().map(|m| m.text.to_lowercase()).collect::<Vec<_>>().join(" ")
                    );
                    if !q.is_empty() && !hay.contains(&q) {
                        continue;
                    }
                    n += 1;
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let txt = gtk::Box::new(gtk::Orientation::Vertical, 2);
                    txt.set_hexpand(true);
                    let t = gtk::Label::new(Some(&chat.title));
                    t.set_xalign(0.0);
                    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    txt.append(&t);
                    let d = gtk::Label::new(Some(&lens_core::ago(chat.updated)));
                    d.set_xalign(0.0);
                    d.add_css_class("dim");
                    txt.append(&d);
                    row.append(&txt);
                    let del = gtk::Button::from_icon_name("user-trash-symbolic");
                    del.add_css_class("flat");
                    del.add_css_class("circular");
                    del.set_tooltip_text(Some("Удалить"));
                    del.set_valign(gtk::Align::Center);
                    row.append(&del);
                    let lrow = gtk::ListBoxRow::new();
                    lrow.set_child(Some(&row));
                    let (id, l, r) = (chat.id.clone(), list.clone(), lrow.clone());
                    del.connect_clicked(move |_| {
                        history::delete(&id);
                        l.remove(&r);
                    });
                    let (c, v, p) = (chat.clone(), v.clone(), pop.clone());
                    lrow.connect_activate(move |_| {
                        p.popdown();
                        v.load_chat(c.clone());
                    });
                    list.append(&lrow);
                }
                empty.set_visible(n == 0);
            })
        };
        list.connect_row_activated(|_, row| row.emit_by_name::<()>("activate", &[]));
        let (f, c) = (fill.clone(), chats.clone());
        pop.connect_show(move |_| {
            *c.borrow_mut() = history::list();
            f();
        });
        let f = fill.clone();
        search.connect_search_changed(move |_| f());
        let l = list.clone();
        search.connect_activate(move |_| {
            if let Some(r) = l.row_at_index(0) {
                r.emit_by_name::<()>("activate", &[]);
            }
        });
    }

    pub fn open_history(&self) {
        self.0.history_btn.popup();
    }
}
