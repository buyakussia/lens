//! Настройки Lens: модели, подсказка, горячие клавиши, быстрые действия, автозапуск.

use adw::prelude::*;
use gtk::{gdk, glib};
use lens_core::config::{self, Action, Config};
use linux_common::{autostart, hotkeys, portal};
use std::cell::RefCell;
use std::rc::Rc;

pub const APP_ID: &str = "io.github.lensdesktop.Lens";

pub fn hotkeys() -> hotkeys::Hotkeys {
    hotkeys::Hotkeys {
        app_name: "Lens",
        exe: autostart::self_exe("lens"),
        items: vec![
            hotkeys::Hotkey { id: "lens-open", title: "Открыть чат", args: "--chat", default: "<Super>c" },
            hotkeys::Hotkey { id: "lens-select", title: "Сразу выделить область и спросить", args: "--select", default: "" },
            hotkeys::Hotkey { id: "lens-quiet", title: "Тихий вопрос по экрану", args: "--quiet", default: "<Super><Alt>c" },
            hotkeys::Hotkey { id: "lens-selection", title: "Спросить про выделенный текст", args: "--selection", default: "<Super><Shift>c" },
        ],
    }
}

fn model_row(title: &str, subtitle: &str, current: &str) -> adw::ComboRow {
    let names: Vec<&str> = config::MODELS.iter().map(|(_, l)| *l).collect();
    let row = adw::ComboRow::builder().title(title).subtitle(subtitle).build();
    row.set_model(Some(&gtk::StringList::new(&names)));
    row.set_selected(config::MODELS.iter().position(|(m, _)| *m == current).unwrap_or(0) as u32);
    row
}

/// Окно захвата нового сочетания клавиш.
fn capture_hotkey(parent: &impl IsA<gtk::Widget>, title: &str, done: impl Fn(String) + 'static) {
    let dialog = adw::AlertDialog::new(Some(title), Some("Нажмите новое сочетание клавиш.\nBackspace — отключить, Esc — отмена."));
    dialog.add_response("cancel", "Отмена");
    let done = Rc::new(done);
    let keys = gtk::EventControllerKey::new();
    let d = dialog.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if key == gdk::Key::Escape {
            d.close();
            return glib::Propagation::Stop;
        }
        if key == gdk::Key::BackSpace && mods.is_empty() {
            done(String::new());
            d.close();
            return glib::Propagation::Stop;
        }
        if let Some(acc) = hotkeys::accel_from_event(key, mods) {
            done(acc);
            d.close();
        }
        glib::Propagation::Stop
    });
    dialog.add_controller(keys);
    dialog.present(Some(parent));
}

pub fn open(cfg: Rc<RefCell<Config>>, on_change: Rc<dyn Fn()>) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Lens — настройки");
    dialog.set_search_enabled(false);

    let save = {
        let (cfg, on_change) = (cfg.clone(), on_change.clone());
        Rc::new(move || {
            let _ = config::save(&cfg.borrow());
            on_change();
        })
    };

    // ---------------------------------------------------------------- Общие
    let page = adw::PreferencesPage::builder().title("Общие").icon_name("emblem-system-symbolic").build();

    let g = adw::PreferencesGroup::builder().title("Ответы").build();
    let model = model_row("Модель в чате", "Её можно сменить прямо в чате", &cfg.borrow().model);
    let (c, s) = (cfg.clone(), save.clone());
    model.connect_selected_notify(move |r| {
        c.borrow_mut().model = config::MODELS[r.selected() as usize].0.to_string();
        s();
    });
    g.add(&model);
    let quiet = model_row("Модель тихого режима", "Быстрая модель отвечает за пару секунд", &cfg.borrow().quiet_model);
    let (c, s) = (cfg.clone(), save.clone());
    quiet.connect_selected_notify(move |r| {
        c.borrow_mut().quiet_model = config::MODELS[r.selected() as usize].0.to_string();
        s();
    });
    g.add(&quiet);
    let sp = adw::EntryRow::builder().title("Дополнительная инструкция для Claude").build();
    sp.set_text(&cfg.borrow().system_prompt);
    sp.set_show_apply_button(true);
    let (c, s) = (cfg.clone(), save.clone());
    sp.connect_apply(move |e| {
        c.borrow_mut().system_prompt = e.text().to_string();
        s();
    });
    g.add(&sp);
    page.add(&g);

    let g = adw::PreferencesGroup::builder().title("Система").build();
    let auto = adw::SwitchRow::builder().title("Запускать при входе в систему").subtitle("Lens ждёт в фоне и открывается мгновенно").build();
    auto.set_active(autostart::enabled(APP_ID));
    auto.connect_active_notify(|r| {
        let exe = autostart::self_exe("lens");
        let _ = autostart::set(APP_ID, "Lens", &format!("{exe} --background"), APP_ID, r.is_active());
    });
    g.add(&auto);
    let perm = adw::ActionRow::builder()
        .title("Снимки экрана без подтверждения")
        .subtitle("Нужно один раз, чтобы выделение и тихий режим работали без окна скриншота GNOME")
        .build();
    let btn = gtk::Button::with_label("Разрешить");
    btn.set_valign(gtk::Align::Center);
    btn.connect_clicked(|b| {
        b.set_sensitive(false);
        b.set_label("Ждём ответа…");
        let b = b.clone();
        glib::spawn_future_local(async move {
            match portal::screenshot(false).await {
                Ok(p) => {
                    portal::take_file(&p);
                    b.set_label("Разрешено ✓");
                }
                Err(_) => {
                    b.set_label("Не разрешено — ещё раз");
                    b.set_sensitive(true);
                }
            }
        });
    });
    perm.add_suffix(&btn);
    g.add(&perm);
    page.add(&g);

    let g = adw::PreferencesGroup::builder()
        .title("Горячие клавиши")
        .description(if hotkeys::Hotkeys::available() { "Работают во всей системе (сочетания GNOME)" } else { "Нужен GNOME: в другом окружении назначьте команды вручную" })
        .build();
    let hk = Rc::new(hotkeys());
    for item in hk.items.clone() {
        let row = adw::ActionRow::builder().title(item.title).subtitle(format!("lens {}", item.args)).build();
        let b = gtk::Button::with_label(&hotkeys::label(&hk.get(item.id)));
        b.set_valign(gtk::Align::Center);
        let (hk2, it, d) = (hk.clone(), item.clone(), dialog.clone());
        b.connect_clicked(move |b| {
            let (hk, b2, id) = (hk2.clone(), b.clone(), it.id);
            capture_hotkey(&d, it.title, move |acc| {
                hk.set(id, &acc);
                b2.set_label(&hotkeys::label(&acc));
            });
        });
        row.add_suffix(&b);
        g.add(&row);
    }
    page.add(&g);
    dialog.add(&page);

    // ---------------------------------------------------------------- Быстрые действия
    let page = adw::PreferencesPage::builder().title("Быстрые действия").icon_name("starred-symbolic").build();
    let g = adw::PreferencesGroup::builder()
        .title("Быстрые действия")
        .description("Кнопки у выделения и над полем ввода; Alt+1…9 в чате")
        .build();
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.add_css_class("flat");
    add.set_tooltip_text(Some("Добавить"));
    let reset = gtk::Button::with_label("Сбросить");
    reset.add_css_class("flat");
    let suffix = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    suffix.append(&reset);
    suffix.append(&add);
    g.set_header_suffix(Some(&suffix));
    page.add(&g);

    let rows: Rc<RefCell<Vec<adw::ExpanderRow>>> = Rc::new(RefCell::new(vec![]));
    type Fill = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
    let fill: Fill = Rc::new(RefCell::new(None));
    let f = {
        let (g, rows, cfg, save, fill2) = (g.clone(), rows.clone(), cfg.clone(), save.clone(), fill.clone());
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                g.remove(&r);
            }
            let actions = cfg.borrow().actions.clone();
            for (i, a) in actions.iter().enumerate() {
                let row = adw::ExpanderRow::builder().title(glib::markup_escape_text(&a.name).as_str()).subtitle(format!("Alt+{}", i + 1)).build();
                let name = adw::EntryRow::builder().title("Название").build();
                name.set_text(&a.name);
                let (c, s, r) = (cfg.clone(), save.clone(), row.clone());
                name.connect_changed(move |e| {
                    if let Some(x) = c.borrow_mut().actions.get_mut(i) {
                        x.name = e.text().to_string();
                    }
                    r.set_title(glib::markup_escape_text(&e.text()).as_str());
                    s();
                });
                row.add_row(&name);
                let prompt = adw::EntryRow::builder().title("Что попросить у Claude").build();
                prompt.set_text(&a.prompt);
                let (c, s) = (cfg.clone(), save.clone());
                prompt.connect_changed(move |e| {
                    if let Some(x) = c.borrow_mut().actions.get_mut(i) {
                        x.prompt = e.text().to_string();
                    }
                    s();
                });
                row.add_row(&prompt);
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("flat");
                del.set_valign(gtk::Align::Center);
                del.set_tooltip_text(Some("Удалить"));
                let (c, s, fl) = (cfg.clone(), save.clone(), fill2.clone());
                del.connect_clicked(move |_| {
                    if i < c.borrow().actions.len() {
                        c.borrow_mut().actions.remove(i);
                    }
                    s();
                    let f = fl.borrow().clone();
                    if let Some(f) = f {
                        glib::idle_add_local_once(move || f());
                    }
                });
                row.add_suffix(&del);
                g.add(&row);
                rows.borrow_mut().push(row);
            }
        })
    };
    *fill.borrow_mut() = Some(f.clone());
    f();
    let (c, s, f2) = (cfg.clone(), save.clone(), f.clone());
    add.connect_clicked(move |_| {
        c.borrow_mut().actions.push(Action { name: "Новое действие".into(), prompt: "Что сделать с вложением…".into() });
        s();
        f2();
    });
    let (c, s, f2) = (cfg.clone(), save.clone(), f.clone());
    reset.connect_clicked(move |_| {
        c.borrow_mut().actions = config::default_actions();
        s();
        f2();
    });
    dialog.add(&page);

    dialog.present(None::<&gtk::Widget>);
}
