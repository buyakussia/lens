//! Clipboard History для Linux (GNOME) — история буфера обмена как Win+V, но вся история
//! переживает перезагрузку, а лимиты больше.
//!
//!   clipboard-history            открыть / закрыть историю (Super+V)
//!   clipboard-history --settings настройки
//!   clipboard-history --background  запуск в фоне при входе в систему
//!   clipboard-history --quit

mod style;
mod watcher;

use adw::prelude::*;
use clip_core::{settings::Settings, Item, Kind, Store};
use gtk::{gdk, gio, glib};
use linux_common::{autostart, hotkeys, portal, x11};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use watcher::Watcher;

const APP_ID: &str = "io.github.lensdesktop.ClipboardHistory";

fn hotkeys() -> hotkeys::Hotkeys {
    hotkeys::Hotkeys {
        app_name: "Clipboard History",
        exe: autostart::self_exe("clipboard-history"),
        items: vec![hotkeys::Hotkey { id: "clipboard-history-open", title: "Открыть историю буфера", args: "--toggle", default: "<Super>v" }],
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Plain,
    Terminal,
}

struct Picker {
    win: gtk::Window,
    card: gtk::Box,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    empty: gtk::Label,
}

struct App {
    app: adw::Application,
    store: Rc<RefCell<Store>>,
    watcher: Rc<Watcher>,
    settings: RefCell<Settings>,
    picker: RefCell<Option<Rc<Picker>>>,
}

fn ago_and_size(item: &Item) -> String {
    let when = lens_core::ago(item.time);
    match item.kind {
        Kind::Image => format!("Картинка {}×{} · {when}", item.width, item.height),
        Kind::Text => {
            let n = item.text.chars().count();
            let fmt = if item.html { " · форматированный" } else { "" };
            format!("{n} симв.{fmt} · {when}")
        }
    }
}

impl App {
    fn notify(&self, text: &str) {
        let n = gio::Notification::new("Clipboard History");
        n.set_body(Some(text));
        self.app.send_notification(None, &n);
    }

    // ------------------------------------------------------------------ окно истории

    fn toggle(self: &Rc<Self>) {
        if let Some(p) = self.picker.borrow_mut().take() {
            Self::destroy_picker(&p);
            return;
        }
        self.open_picker();
    }

    fn destroy_picker(p: &Picker) {
        p.win.destroy();
        glib::idle_add_local_once(linux_common::trim_memory);
    }

    fn close_picker(&self) {
        if let Some(p) = self.picker.borrow_mut().take() {
            Self::destroy_picker(&p);
        }
    }

    fn open_picker(self: &Rc<Self>) {
        let win = gtk::Window::new();
        win.set_application(Some(&self.app));
        win.set_title(Some("Буфер обмена"));
        win.set_decorated(false);
        win.set_resizable(false);
        win.add_css_class("clip-window");

        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("clip-card");
        card.set_size_request(440, -1);
        card.set_hexpand(false);
        card.set_opacity(0.0);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        head.add_css_class("clip-head");
        let title = gtk::Label::new(Some("Буфер обмена"));
        title.add_css_class("clip-title");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        head.append(&title);
        let clear = gtk::Button::with_label("Очистить всё");
        clear.add_css_class("flat");
        clear.set_tooltip_text(Some("Удалить всё, кроме закреплённого"));
        clear.set_can_focus(false);
        head.append(&clear);
        let gear = gtk::Button::from_icon_name("emblem-system-symbolic");
        gear.add_css_class("flat");
        gear.add_css_class("circular");
        gear.set_tooltip_text(Some("Настройки"));
        gear.set_can_focus(false);
        head.append(&gear);
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&head));
        card.append(&handle);

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Поиск…"));
        search.add_css_class("clip-search");
        card.append(&search);

        let list = gtk::ListBox::new();
        list.add_css_class("clip-list");
        list.set_selection_mode(gtk::SelectionMode::Browse);
        list.set_activate_on_single_click(true);
        let sw = gtk::ScrolledWindow::new();
        sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        sw.set_propagate_natural_height(true);
        sw.set_max_content_height(470);
        sw.set_child(Some(&list));
        card.append(&sw);
        let empty = gtk::Label::new(Some("Здесь появится всё, что вы копируете.\nИстория сохраняется и после перезагрузки."));
        empty.add_css_class("clip-empty");
        empty.set_justify(gtk::Justification::Center);
        empty.set_wrap(true);
        empty.set_max_width_chars(40);
        card.append(&empty);
        let hint = gtk::Label::new(Some("Enter — вставить · Shift+Enter — в терминал · Ctrl+Enter — без форматирования · Ctrl+P — закрепить · Del — удалить"));
        hint.add_css_class("clip-hint");
        hint.set_wrap(true);
        hint.set_xalign(0.0);
        hint.set_max_width_chars(48);
        hint.set_width_chars(48);
        card.append(&hint);
        win.set_child(Some(&card));

        let p = Rc::new(Picker { win: win.clone(), card: card.clone(), search: search.clone(), list: list.clone(), empty });
        *self.picker.borrow_mut() = Some(p.clone());
        self.fill();

        let me = Rc::downgrade(self);
        clear.connect_clicked(move |_| {
            if let Some(a) = me.upgrade() {
                a.store.borrow_mut().clear();
                let _ = a.store.borrow().save();
                a.fill();
            }
        });
        let me = Rc::downgrade(self);
        gear.connect_clicked(move |_| {
            if let Some(a) = me.upgrade() {
                a.close_picker();
                a.show_settings();
            }
        });
        let l = list.clone();
        search.connect_search_changed(move |_| {
            l.invalidate_filter();
            if let Some(r) = first_visible(&l) {
                l.select_row(Some(&r));
            }
        });
        let s = search.clone();
        list.set_filter_func(move |row| {
            let q = s.text().to_lowercase();
            q.is_empty() || row.tooltip_text().map(|t| t.to_lowercase().contains(&q)).unwrap_or(false)
        });
        let me = Rc::downgrade(self);
        list.connect_row_activated(move |_, row| {
            if let Some(a) = me.upgrade() {
                a.paste(&row.widget_name(), Mode::Normal);
            }
        });

        // клавиатура: ввод идёт в поиск, стрелки и Enter — по списку
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let me = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            use gdk::Key;
            let Some(a) = me.upgrade() else { return glib::Propagation::Proceed };
            let Some(p) = a.picker.borrow().clone() else { return glib::Propagation::Proceed };
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
            let selected = p.list.selected_row().map(|r| r.widget_name().to_string());
            match key {
                Key::Escape => {
                    a.close_picker();
                    glib::Propagation::Stop
                }
                Key::Return | Key::KP_Enter => {
                    if let Some(id) = selected {
                        let mode = if shift { Mode::Terminal } else if ctrl { Mode::Plain } else { Mode::Normal };
                        a.paste(&id, mode);
                    }
                    glib::Propagation::Stop
                }
                Key::Down | Key::Up => {
                    move_selection(&p.list, if key == Key::Down { 1 } else { -1 });
                    glib::Propagation::Stop
                }
                Key::Delete if p.search.text().is_empty() || !p.search.has_focus() => {
                    if let Some(id) = selected {
                        a.remove(&id);
                    }
                    glib::Propagation::Stop
                }
                Key::p | Key::P if ctrl => {
                    if let Some(id) = selected {
                        a.toggle_pin(&id);
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        win.add_controller(keys);

        // закрываемся, когда пользователь ушёл в другое окно
        let me = Rc::downgrade(self);
        win.connect_is_active_notify(move |w| {
            if !w.is_active() {
                let me = me.clone();
                let w = w.downgrade();
                glib::timeout_add_local_once(Duration::from_millis(150), move || {
                    let (Some(a), Some(w)) = (me.upgrade(), w.upgrade()) else { return };
                    let ours = a.picker.borrow().as_ref().map(|p| p.win == w).unwrap_or(false);
                    if ours && !w.is_active() {
                        a.close_picker();
                    }
                });
            }
        });

        WidgetExt::realize(&win);
        x11::keep_above(&win);
        if let Some(surface) = win.surface() {
            let w = win.downgrade();
            surface.connect_layout(move |_, width, height| {
                if let Some(w) = w.upgrade() {
                    let wa = x11::workarea(&w);
                    x11::move_to(&w, wa.x() + (wa.width() - width) / 2, wa.y() + (wa.height() - height) * 3 / 5);
                }
            });
        }
        win.present();
        x11::keep_above(&win);
        search.grab_focus();

        // мягкое появление: проявление и небольшой подъём
        let c = card.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            c.set_opacity(v);
            let off = ((1.0 - v) * 10.0) as i32;
            c.set_margin_top(24 + off);
            c.set_margin_bottom(24 - off);
        });
        let anim = adw::TimedAnimation::new(&card, 0.0, 1.0, 200, target);
        anim.set_easing(adw::Easing::EaseOutCubic);
        anim.play();
    }

    fn fill(self: &Rc<Self>) {
        let Some(p) = self.picker.borrow().clone() else { return };
        let keep = p.list.selected_row().map(|r| r.widget_name().to_string());
        while let Some(c) = p.list.first_child() {
            p.list.remove(&c);
        }
        let items: Vec<Item> = self.store.borrow().items().to_vec();
        for item in &items {
            p.list.append(&self.make_row(item));
        }
        p.empty.set_visible(items.is_empty());
        p.list.set_visible(!items.is_empty());
        let mut target = None;
        if let Some(k) = keep {
            let mut c = p.list.first_child();
            while let Some(w) = c {
                if w.widget_name() == k {
                    target = w.downcast_ref::<gtk::ListBoxRow>().cloned();
                }
                c = w.next_sibling();
            }
        }
        if let Some(r) = target.or_else(|| first_visible(&p.list)) {
            p.list.select_row(Some(&r));
        }
        let _ = &p.card;
    }

    fn make_row(self: &Rc<Self>, item: &Item) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.set_widget_name(&item.id);
        row.set_tooltip_text(Some(if item.kind == Kind::Image { "картинка изображение image" } else { &item.text }));
        // подсказка нужна для поиска, но не должна всплывать
        row.set_has_tooltip(false);
        let hb = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        hb.add_css_class("clip-item");
        let vb = gtk::Box::new(gtk::Orientation::Vertical, 4);
        vb.set_hexpand(true);
        match item.kind {
            Kind::Image => {
                let path = self.store.borrow().thumb_path(item);
                let path = if path.exists() { path } else { self.store.borrow().image_path(item) };
                let pic = gtk::Picture::for_filename(&path);
                pic.set_can_shrink(true);
                pic.set_content_fit(gtk::ContentFit::ScaleDown);
                pic.set_halign(gtk::Align::Start);
                pic.set_size_request(-1, 90.min(item.height as i32).max(24));
                vb.append(&pic);
            }
            Kind::Text => {
                let preview: String = item.text.trim().lines().take(4).collect::<Vec<_>>().join("\n");
                let preview: String = preview.chars().take(400).collect();
                let l = gtk::Label::new(Some(&preview));
                l.add_css_class("clip-text");
                l.set_xalign(0.0);
                l.set_wrap(true);
                l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                l.set_lines(4);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(40);
                vb.append(&l);
            }
        }
        let meta = gtk::Label::new(Some(&ago_and_size(item)));
        meta.add_css_class("clip-meta");
        meta.set_xalign(0.0);
        vb.append(&meta);
        hb.append(&vb);

        let side = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        side.set_valign(gtk::Align::Start);
        let pin = gtk::Button::from_icon_name("view-pin-symbolic");
        pin.add_css_class("flat");
        pin.set_can_focus(false);
        pin.set_tooltip_text(Some(if item.pinned { "Открепить" } else { "Закрепить (не удалится при очистке)" }));
        if item.pinned {
            pin.add_css_class("clip-pinned");
        } else {
            pin.set_opacity(0.55);
        }
        let (me, id) = (Rc::downgrade(self), item.id.clone());
        pin.connect_clicked(move |_| {
            if let Some(a) = me.upgrade() {
                a.toggle_pin(&id);
            }
        });
        side.append(&pin);

        let more = gtk::MenuButton::new();
        more.set_icon_name("view-more-symbolic");
        more.add_css_class("flat");
        more.set_can_focus(false);
        more.set_tooltip_text(Some("Ещё"));
        let pop = gtk::Popover::new();
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let add = |label: &str, f: Box<dyn Fn(&Rc<App>)>| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (me, pop) = (Rc::downgrade(self), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                if let Some(a) = me.upgrade() {
                    f(&a);
                }
            });
            menu.append(&b);
        };
        if item.kind == Kind::Text {
            let id = item.id.clone();
            add("Вставить без форматирования", Box::new(move |a| a.paste(&id, Mode::Plain)));
        }
        let id = item.id.clone();
        add("Вставить в терминал", Box::new(move |a| a.paste(&id, Mode::Terminal)));
        let id = item.id.clone();
        add("Спросить Lens", Box::new(move |a| a.ask_lens(&id)));
        let id = item.id.clone();
        add("Удалить", Box::new(move |a| a.remove(&id)));
        pop.set_child(Some(&menu));
        more.set_popover(Some(&pop));
        side.append(&more);
        hb.append(&side);
        row.set_child(Some(&hb));
        row
    }

    // ------------------------------------------------------------------ действия

    fn toggle_pin(self: &Rc<Self>, id: &str) {
        let pinned = self.store.borrow().get(id).map(|i| i.pinned).unwrap_or(false);
        self.store.borrow_mut().set_pinned(id, !pinned);
        let _ = self.store.borrow().save();
        self.fill();
    }

    fn remove(self: &Rc<Self>, id: &str) {
        // выделение переходит на соседнюю запись
        if let Some(p) = self.picker.borrow().as_ref() {
            if let Some(r) = p.list.selected_row() {
                let next = r.next_sibling().or_else(|| r.prev_sibling()).and_downcast::<gtk::ListBoxRow>();
                if let Some(n) = next {
                    p.list.select_row(Some(&n));
                }
            }
        }
        self.store.borrow_mut().remove(id);
        let _ = self.store.borrow().save();
        self.fill();
    }

    /// Кладёт запись в буфер обмена и (если включено) вставляет её в окно, где вы были.
    fn paste(self: &Rc<Self>, id: &str, mode: Mode) {
        let Some(item) = self.store.borrow().get(id).cloned() else { return };
        let Some(display) = gdk::Display::default() else { return };
        let cb = display.clipboard();
        self.watcher.ignore(1.0);
        match item.kind {
            Kind::Text => {
                let html = if mode == Mode::Normal { self.store.borrow().html(&item) } else { None };
                match html {
                    Some(h) => {
                        let provider = gdk::ContentProvider::new_union(&[
                            gdk::ContentProvider::for_bytes("text/html", &glib::Bytes::from_owned(h.into_bytes())),
                            gdk::ContentProvider::for_value(&item.text.to_value()),
                        ]);
                        let _ = cb.set_content(Some(&provider));
                    }
                    None => cb.set_text(&item.text),
                }
            }
            Kind::Image => match gdk::Texture::from_filename(self.store.borrow().image_path(&item)) {
                Ok(t) => cb.set_texture(&t),
                Err(e) => return self.notify(&format!("Картинка недоступна: {e}")),
            },
        }
        self.store.borrow_mut().touch(id);
        let _ = self.store.borrow().save();
        self.close_picker();
        if !self.settings.borrow().auto_paste {
            return;
        }
        let me = self.clone();
        glib::spawn_future_local(async move {
            // ждём, пока фокус вернётся в предыдущее окно
            glib::timeout_future(Duration::from_millis(180)).await;
            let token = me.settings.borrow().paste_token.clone();
            match portal::paste(mode == Mode::Terminal, Some(token)).await {
                Ok(Some(t)) => {
                    let mut s = me.settings.borrow_mut();
                    if s.paste_token != t {
                        s.paste_token = t;
                        let _ = clip_core::settings::save(&s);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("clipboard-history: paste: {e}");
                    me.notify("Скопировано. Вставьте сочетанием Ctrl+V (автовставка недоступна).");
                }
            }
        });
    }

    fn lens_exe() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("lens")))
            .filter(|p| p.is_file())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "lens".into())
    }

    /// Отправляет запись в Lens, чтобы спросить Claude о ней.
    fn ask_lens(self: &Rc<Self>, id: &str) {
        let Some(item) = self.store.borrow().get(id).cloned() else { return };
        let (flag, path) = match item.kind {
            Kind::Image => ("--attach-image", self.store.borrow().image_path(&item)),
            Kind::Text => {
                let p = std::env::temp_dir().join(format!("clipboard-history-{}.txt", item.id));
                if std::fs::write(&p, &item.text).is_err() {
                    return;
                }
                ("--attach-text-file", p)
            }
        };
        self.close_picker();
        if std::process::Command::new(Self::lens_exe()).arg(flag).arg(&path).spawn().is_err() {
            self.notify("Lens не установлен.");
        }
    }

    // ------------------------------------------------------------------ настройки

    fn show_settings(self: &Rc<Self>) {
        let dialog = adw::PreferencesDialog::new();
        dialog.set_title("Clipboard History — настройки");
        let page = adw::PreferencesPage::new();
        let g = adw::PreferencesGroup::builder().title("История").description("Вся история сохраняется на диске и переживает перезагрузку").build();
        let s = self.settings.borrow().clone();
        let items = adw::SpinRow::with_range(10.0, 5000.0, 10.0);
        items.set_title("Сколько записей хранить");
        items.set_subtitle("Закреплённые не считаются и не удаляются");
        items.set_value(s.max_items as f64);
        let me = Rc::downgrade(self);
        items.connect_value_notify(move |r| {
            if let Some(a) = me.upgrade() {
                a.settings.borrow_mut().max_items = r.value() as usize;
                a.apply_settings();
            }
        });
        g.add(&items);
        let img = adw::SpinRow::with_range(1.0, 128.0, 1.0);
        img.set_title("Максимальный размер картинки, МБ");
        img.set_value(s.max_image_mb as f64);
        let me = Rc::downgrade(self);
        img.connect_value_notify(move |r| {
            if let Some(a) = me.upgrade() {
                a.settings.borrow_mut().max_image_mb = r.value() as usize;
                a.apply_settings();
            }
        });
        g.add(&img);
        let wipe = adw::ActionRow::builder().title("Удалить всю историю").subtitle("Включая закреплённые записи").build();
        let wb = gtk::Button::with_label("Удалить");
        wb.add_css_class("destructive-action");
        wb.set_valign(gtk::Align::Center);
        let me = Rc::downgrade(self);
        wb.connect_clicked(move |b| {
            if let Some(a) = me.upgrade() {
                let ids: Vec<String> = a.store.borrow().items().iter().map(|i| i.id.clone()).collect();
                for id in ids {
                    a.store.borrow_mut().remove(&id);
                }
                let _ = a.store.borrow().save();
                b.set_label("Удалено ✓");
                b.set_sensitive(false);
            }
        });
        wipe.add_suffix(&wb);
        g.add(&wipe);
        page.add(&g);

        let g = adw::PreferencesGroup::builder().title("Вставка").build();
        let ap = adw::SwitchRow::builder()
            .title("Сразу вставлять в окно")
            .subtitle("Иначе запись только копируется. В первый раз GNOME спросит разрешение на управление клавиатурой")
            .build();
        ap.set_active(s.auto_paste);
        let me = Rc::downgrade(self);
        ap.connect_active_notify(move |r| {
            if let Some(a) = me.upgrade() {
                a.settings.borrow_mut().auto_paste = r.is_active();
                a.apply_settings();
            }
        });
        g.add(&ap);
        let hk = Rc::new(hotkeys());
        let row = adw::ActionRow::builder().title("Горячая клавиша").build();
        let b = gtk::Button::with_label(&hotkeys::label(&hk.get("clipboard-history-open")));
        b.set_valign(gtk::Align::Center);
        let d = dialog.clone();
        b.connect_clicked(move |b| {
            let alert = adw::AlertDialog::new(Some("Горячая клавиша"), Some("Нажмите новое сочетание клавиш.\nEsc — отмена."));
            alert.add_response("cancel", "Отмена");
            let keys = gtk::EventControllerKey::new();
            let (al, hk, b) = (alert.clone(), hk.clone(), b.clone());
            keys.connect_key_pressed(move |_, key, _, mods| {
                if key == gdk::Key::Escape {
                    al.close();
                } else if let Some(acc) = hotkeys::accel_from_event(key, mods) {
                    hk.set("clipboard-history-open", &acc);
                    b.set_label(&hotkeys::label(&acc));
                    al.close();
                }
                glib::Propagation::Stop
            });
            alert.add_controller(keys);
            alert.present(Some(&d));
        });
        row.add_suffix(&b);
        g.add(&row);
        let auto = adw::SwitchRow::builder().title("Запускать при входе в систему").subtitle("Иначе история не записывается, пока программа не запущена").build();
        auto.set_active(autostart::enabled(APP_ID));
        auto.connect_active_notify(|r| {
            let exe = autostart::self_exe("clipboard-history");
            let _ = autostart::set(APP_ID, "Clipboard History", &format!("{exe} --background"), APP_ID, r.is_active());
        });
        g.add(&auto);
        page.add(&g);
        dialog.add(&page);
        dialog.present(None::<&gtk::Widget>);
    }

    fn apply_settings(&self) {
        let s = self.settings.borrow().clone();
        let _ = clip_core::settings::save(&s);
        self.store.borrow_mut().set_limits(s.limits());
        let _ = self.store.borrow().save();
    }

    fn command(self: &Rc<Self>, args: &[String], first: bool) {
        let has = |f: &str| args.iter().any(|a| a == f);
        if has("--background") {
        } else if has("--settings") {
            self.show_settings();
        } else if has("--quit") {
            self.app.quit();
        } else if has("--remove-hotkeys") {
            hotkeys().remove_all(&[]);
            self.app.quit();
        } else if has("--toggle") || !first || args.is_empty() {
            self.toggle();
        }
    }
}

fn first_visible(list: &gtk::ListBox) -> Option<gtk::ListBoxRow> {
    let mut c = list.first_child();
    while let Some(w) = c {
        if let Some(r) = w.downcast_ref::<gtk::ListBoxRow>() {
            if r.is_child_visible() {
                return Some(r.clone());
            }
        }
        c = w.next_sibling();
    }
    None
}

fn move_selection(list: &gtk::ListBox, dir: i32) {
    let cur = list.selected_row();
    let mut c: Option<gtk::Widget> = match &cur {
        Some(r) if dir > 0 => r.next_sibling(),
        Some(r) => r.prev_sibling(),
        None => list.first_child(),
    };
    while let Some(w) = c {
        if let Some(r) = w.downcast_ref::<gtk::ListBoxRow>() {
            if r.is_child_visible() {
                list.select_row(Some(r));
                r.grab_focus();
                return;
            }
        }
        c = if dir > 0 { w.next_sibling() } else { w.prev_sibling() };
    }
}

fn main() -> glib::ExitCode {
    linux_common::use_x11_backend();
    glib::set_prgname(Some("clipboard-history"));
    glib::set_application_name("Clipboard History");
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let state: Rc<RefCell<Option<Rc<App>>>> = Rc::new(RefCell::new(None));
    app.connect_startup(|app| {
        linux_common::load_css(style::CSS);
        std::mem::forget(app.hold());
        if !std::env::args().any(|a| a == "--remove-hotkeys") {
            hotkeys().ensure();
        }
    });
    let st = state.clone();
    app.connect_command_line(move |app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).map(|a| a.to_string_lossy().to_string()).collect();
        let first = st.borrow().is_none();
        if first {
            let settings = clip_core::settings::load();
            let store = Rc::new(RefCell::new(Store::open(Store::default_dir(), settings.limits())));
            let watcher = Watcher::new(store.clone());
            let me = Rc::new(App { app: app.clone(), store, watcher: watcher.clone(), settings: RefCell::new(settings), picker: RefCell::new(None) });
            let w = Rc::downgrade(&me);
            *watcher.on_change.borrow_mut() = Some(Box::new(move || {
                if let Some(a) = w.upgrade() {
                    a.fill();
                }
            }));
            *st.borrow_mut() = Some(me);
        }
        let me = st.borrow().clone().unwrap();
        me.command(&args, first);
        glib::ExitCode::SUCCESS
    });
    app.run()
}
