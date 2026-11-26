//! Глобальные сочетания клавиш через пользовательские комбинации GNOME (media-keys).
//! GNOME сам ловит клавишу и запускает команду, например `lens --select`;
//! уже запущенное приложение получает её через D-Bus (GApplication), так что это мгновенно.

use gtk::gio;
use gtk::prelude::*;

const BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
const LIST_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const ITEM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

#[derive(Clone, Debug)]
pub struct Hotkey {
    /// Уникальный id (часть пути в dconf).
    pub id: &'static str,
    pub title: &'static str,
    /// Аргументы команды (после пути к программе).
    pub args: &'static str,
    pub default: &'static str,
}

pub struct Hotkeys {
    pub app_name: &'static str,
    pub exe: String,
    pub items: Vec<Hotkey>,
}

impl Hotkeys {
    pub fn available() -> bool {
        gio::SettingsSchemaSource::default()
            .map(|s| s.lookup(LIST_SCHEMA, true).is_some() && s.lookup(ITEM_SCHEMA, true).is_some())
            .unwrap_or(false)
    }

    fn item(id: &str) -> gio::Settings {
        gio::Settings::with_path(ITEM_SCHEMA, &format!("{BASE}{id}/"))
    }

    fn meta(&self, id: &str) -> Option<&Hotkey> {
        self.items.iter().find(|h| h.id == id)
    }

    pub fn get(&self, id: &str) -> String {
        if !Self::available() {
            return String::new();
        }
        Self::item(id).string("binding").to_string()
    }

    fn registered(id: &str) -> bool {
        let p = format!("{BASE}{id}/");
        gio::Settings::new(LIST_SCHEMA).strv("custom-keybindings").iter().any(|s| s.as_str() == p)
    }

    pub fn set(&self, id: &str, accel: &str) {
        if !Self::available() {
            return;
        }
        let Some(m) = self.meta(id) else { return };
        let it = Self::item(id);
        let _ = it.set_string("name", &format!("{} — {}", self.app_name, m.title));
        let _ = it.set_string("command", format!("{} {}", self.exe, m.args).trim());
        let _ = it.set_string("binding", accel);
        gio::Settings::sync();
        // список задаём после заполнения ключей, иначе GNOME может подхватить пустую запись
        let lst = gio::Settings::new(LIST_SCHEMA);
        let path = format!("{BASE}{id}/");
        let mut paths: Vec<String> = lst.strv("custom-keybindings").iter().map(|s| s.to_string()).collect();
        let idx = paths.iter().position(|p| *p == path);
        if let Some(i) = idx {
            paths.remove(i);
        }
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let _ = lst.set_strv("custom-keybindings", refs.as_slice());
        gio::Settings::sync();
        paths.insert(idx.unwrap_or(paths.len()), path);
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let _ = lst.set_strv("custom-keybindings", refs.as_slice());
        gio::Settings::sync();
    }

    /// Регистрирует сочетания по умолчанию (уже настроенные не трогает) и обновляет путь
    /// к программе в существующих — на случай, если её переустановили в другое место.
    pub fn ensure(&self) {
        if !Self::available() {
            return;
        }
        for h in &self.items {
            if !Self::registered(h.id) {
                if !h.default.is_empty() {
                    self.set(h.id, h.default);
                }
            } else {
                let cmd = format!("{} {}", self.exe, h.args).trim().to_string();
                let it = Self::item(h.id);
                if it.string("command").as_str() != cmd {
                    let _ = it.set_string("command", &cmd);
                }
            }
        }
        gio::Settings::sync();
    }

    /// Удаляет свои сочетания (и `extra` — id из прежних версий).
    pub fn remove_all(&self, extra: &[&str]) {
        if !Self::available() {
            return;
        }
        let lst = gio::Settings::new(LIST_SCHEMA);
        let ours: Vec<String> = self
            .items
            .iter()
            .map(|h| h.id)
            .chain(extra.iter().copied())
            .map(|id| format!("{BASE}{id}/"))
            .collect();
        let paths: Vec<String> = lst
            .strv("custom-keybindings")
            .iter()
            .map(|s| s.to_string())
            .filter(|p| !ours.contains(p))
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let _ = lst.set_strv("custom-keybindings", refs.as_slice());
        gio::Settings::sync();
    }
}

/// «Super+C» из «<Super>c».
pub fn label(accel: &str) -> String {
    if accel.is_empty() {
        return "Не задано".into();
    }
    match gtk::accelerator_parse(accel) {
        Some((key, mods)) => gtk::accelerator_get_label(key, mods).to_string(),
        None => accel.to_string(),
    }
}

/// Строка сочетания для GNOME из нажатой клавиши, или None для одиночных модификаторов.
pub fn accel_from_event(key: gtk::gdk::Key, mods: gtk::gdk::ModifierType) -> Option<String> {
    use gtk::gdk::ModifierType as M;
    let mods = mods & (M::CONTROL_MASK | M::SHIFT_MASK | M::ALT_MASK | M::SUPER_MASK);
    let name = key.name()?.to_string();
    if name.starts_with("Shift") || name.starts_with("Control") || name.starts_with("Alt")
        || name.starts_with("Super") || name.starts_with("Meta") || name.starts_with("ISO_")
    {
        return None;
    }
    let key = key.to_lower();
    Some(
        gtk::accelerator_name(key, mods)
            .to_string()
            .replace("<Primary>", "<Control>")
            .replace("<Mod4>", "<Super>"),
    )
}
