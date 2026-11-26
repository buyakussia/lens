//! Хранилище истории буфера обмена, независимое от GUI.
//!
//! Как Win+V, но:
//! - вся история (а не только закреплённое) переживает перезагрузку;
//! - лимиты больше и настраиваются.
//!
//! На диске: `index.json` со списком записей и папка `blobs/` с HTML, картинками и превью.
//! Одинаковое содержимое не дублируется: при повторном копировании запись поднимается наверх.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const APP: &str = "clipboard-history";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Image,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    /// Обычный текст (для картинок пусто).
    #[serde(default)]
    pub text: String,
    /// Есть ли HTML-версия (форматированный текст).
    #[serde(default)]
    pub html: bool,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Размер содержимого в байтах.
    #[serde(default)]
    pub size: u64,
    pub hash: String,
    #[serde(default)]
    pub pinned: bool,
    pub time: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Сколько незакреплённых записей хранить (закреплённые не считаются и не удаляются).
    pub max_items: usize,
    pub max_text_bytes: usize,
    pub max_image_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        // Windows: 25 записей, картинки до 4 МБ. Здесь — больше.
        Limits { max_items: 200, max_text_bytes: 1024 * 1024, max_image_bytes: 16 * 1024 * 1024 }
    }
}

/// Почему запись не попала в историю.
#[derive(Debug, PartialEq, Eq)]
pub enum Skip {
    Empty,
    TooLarge,
}

pub struct Store {
    dir: PathBuf,
    items: Vec<Item>,
    pub limits: Limits,
}

/// FNV-1a: быстрый стабильный хэш для поиска дубликатов.
pub fn hash(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

impl Store {
    pub fn default_dir() -> PathBuf {
        lens_core::paths::data_dir(APP)
    }

    pub fn open(dir: impl Into<PathBuf>, limits: Limits) -> Store {
        let dir = dir.into();
        let items: Vec<Item> = fs::read_to_string(dir.join("index.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut s = Store { dir, items, limits };
        if s.items.is_empty() {
            s.import_legacy();
        }
        s.sort();
        s.collect_garbage();
        s
    }

    fn blobs(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    /// Записи: сначала закреплённые, затем по времени (новые сверху).
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn html_path(&self, item: &Item) -> PathBuf {
        self.blobs().join(format!("{}.html", item.hash))
    }
    pub fn image_path(&self, item: &Item) -> PathBuf {
        self.blobs().join(format!("{}.png", item.hash))
    }
    pub fn thumb_path(&self, item: &Item) -> PathBuf {
        self.blobs().join(format!("{}.thumb.png", item.hash))
    }

    pub fn html(&self, item: &Item) -> Option<String> {
        if item.html {
            fs::read_to_string(self.html_path(item)).ok()
        } else {
            None
        }
    }

    fn sort(&mut self) {
        self.items.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.time.total_cmp(&a.time)));
    }

    fn new_id() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        format!("{:x}{:x}", (lens_core::now() * 1000.0) as u64, N.fetch_add(1, Ordering::Relaxed))
    }

    /// Если такое уже есть — поднимает наверх и возвращает true.
    fn bump(&mut self, kind: Kind, hash: &str) -> bool {
        if let Some(i) = self.items.iter_mut().find(|i| i.kind == kind && i.hash == hash) {
            i.time = lens_core::now();
            self.sort();
            return true;
        }
        false
    }

    /// Добавляет текст (и, если есть, его HTML-версию). Возвращает id записи.
    pub fn add_text(&mut self, text: &str, html: Option<&str>) -> Result<String, Skip> {
        if text.trim().is_empty() {
            return Err(Skip::Empty);
        }
        if text.len() > self.limits.max_text_bytes {
            return Err(Skip::TooLarge);
        }
        let h = hash(text.as_bytes());
        if self.bump(Kind::Text, &h) {
            return Ok(self.items.iter().find(|i| i.hash == h).map(|i| i.id.clone()).unwrap_or_default());
        }
        let html = html.filter(|h| !h.trim().is_empty() && h.len() <= self.limits.max_text_bytes * 4);
        let mut item = Item {
            id: Self::new_id(),
            kind: Kind::Text,
            text: text.to_string(),
            html: false,
            width: 0,
            height: 0,
            size: text.len() as u64,
            hash: h,
            pinned: false,
            time: lens_core::now(),
        };
        if let Some(html) = html {
            let _ = fs::create_dir_all(self.blobs());
            item.html = fs::write(self.html_path(&item), html).is_ok();
        }
        let id = item.id.clone();
        self.items.insert(0, item);
        self.sort();
        self.prune();
        Ok(id)
    }

    /// Добавляет картинку (PNG) и её маленькое превью для списка.
    pub fn add_image(&mut self, png: &[u8], width: u32, height: u32, thumb_png: Option<&[u8]>) -> Result<String, Skip> {
        if png.is_empty() {
            return Err(Skip::Empty);
        }
        if png.len() > self.limits.max_image_bytes {
            return Err(Skip::TooLarge);
        }
        let h = hash(png);
        if self.bump(Kind::Image, &h) {
            return Ok(self.items.iter().find(|i| i.hash == h).map(|i| i.id.clone()).unwrap_or_default());
        }
        let item = Item {
            id: Self::new_id(),
            kind: Kind::Image,
            text: String::new(),
            html: false,
            width,
            height,
            size: png.len() as u64,
            hash: h,
            pinned: false,
            time: lens_core::now(),
        };
        let _ = fs::create_dir_all(self.blobs());
        fs::write(self.image_path(&item), png).map_err(|_| Skip::Empty)?;
        if let Some(t) = thumb_png {
            let _ = fs::write(self.thumb_path(&item), t);
        }
        let id = item.id.clone();
        self.items.insert(0, item);
        self.sort();
        self.prune();
        Ok(id)
    }

    pub fn touch(&mut self, id: &str) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.time = lens_core::now();
        }
        self.sort();
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.pinned = pinned;
        }
        self.sort();
        self.prune();
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let item = self.items.remove(pos);
            self.drop_blobs(&item);
        }
    }

    /// Удаляет всё, кроме закреплённого.
    pub fn clear(&mut self) {
        let (keep, drop): (Vec<Item>, Vec<Item>) = self.items.drain(..).partition(|i| i.pinned);
        self.items = keep;
        for i in drop {
            self.drop_blobs(&i);
        }
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
        self.prune();
    }

    fn prune(&mut self) {
        let max = self.limits.max_items.max(1);
        let mut unpinned = 0;
        let mut drop = vec![];
        self.items.retain(|i| {
            if i.pinned {
                return true;
            }
            unpinned += 1;
            if unpinned > max {
                drop.push(i.clone());
                false
            } else {
                true
            }
        });
        for i in drop {
            self.drop_blobs(&i);
        }
    }

    fn drop_blobs(&self, item: &Item) {
        if self.items.iter().any(|i| i.hash == item.hash) {
            return;
        }
        for p in [self.html_path(item), self.image_path(item), self.thumb_path(item)] {
            let _ = fs::remove_file(p);
        }
    }

    /// Переносит историю из прежней версии (history.json + картинки img_*.png).
    fn import_legacy(&mut self) {
        let old = self.dir.join("history.json");
        let Ok(text) = fs::read_to_string(&old) else { return };
        let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else { return };
        let _ = fs::create_dir_all(self.blobs());
        for v in list.iter().rev() {
            let pinned = v["pinned"].as_bool().unwrap_or(false);
            let time = v["time"].as_f64().unwrap_or_else(lens_core::now);
            let id = match v["kind"].as_str() {
                Some("text") => v["text"].as_str().and_then(|t| self.add_text(t, None).ok()),
                Some("image") => {
                    let path = v["path"].as_str().map(PathBuf::from);
                    let png = path.as_ref().and_then(|p| fs::read(p).ok());
                    let r = png.and_then(|png| {
                        let w = v["w"].as_u64().unwrap_or(0) as u32;
                        let h = v["h"].as_u64().unwrap_or(0) as u32;
                        self.add_image(&png, w, h, None).ok()
                    });
                    if let Some(p) = path {
                        let _ = fs::remove_file(p);
                    }
                    r
                }
                _ => None,
            };
            if let Some(id) = id {
                if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
                    i.pinned = pinned;
                    i.time = time;
                }
            }
        }
        self.sort();
        if self.save().is_ok() {
            let _ = fs::rename(&old, self.dir.join("history.json.imported"));
        }
    }

    /// Удаляет файлы, на которые не ссылается ни одна запись (например, после сбоя).
    fn collect_garbage(&self) {
        let Ok(rd) = fs::read_dir(self.blobs()) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let h = name.split('.').next().unwrap_or("");
            if !self.items.iter().any(|i| i.hash == h) {
                let _ = fs::remove_file(e.path());
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let data = serde_json::to_vec(&self.items).map_err(std::io::Error::other)?;
        lens_core::paths::write_atomic(&self.dir.join("index.json"), &data)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clip-core-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn persists_dedups_and_prunes() {
        let d = tmp("a");
        let mut s = Store::open(&d, Limits { max_items: 2, ..Default::default() });
        let a = s.add_text("один", Some("<b>один</b>")).unwrap();
        s.add_text("два", None).unwrap();
        s.set_pinned(&a, true);
        s.add_text("три", None).unwrap();
        s.add_text("четыре", None).unwrap();
        // закреплённое + 2 последних
        let texts: Vec<_> = s.items().iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["один", "четыре", "три"]);
        // повтор поднимает наверх, а не дублирует
        s.add_text("три", None).unwrap();
        assert_eq!(s.items()[1].text, "три");
        assert_eq!(s.items().len(), 3);
        s.save().unwrap();

        let s2 = Store::open(&d, Limits::default());
        assert_eq!(s2.items().len(), 3);
        assert_eq!(s2.html(&s2.items()[0]).as_deref(), Some("<b>один</b>"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn imports_previous_version() {
        let d = tmp("c");
        fs::create_dir_all(&d).unwrap();
        let img = d.join("img_x.png");
        fs::write(&img, [7, 7, 7]).unwrap();
        let old = format!(
            r#"[{{"kind":"text","text":"новое","pinned":false,"time":20.0,"id":"a"}},
                {{"kind":"image","path":"{}","w":2,"h":3,"pinned":true,"time":10.0,"id":"b"}}]"#,
            img.display()
        );
        fs::write(d.join("history.json"), old).unwrap();
        let s = Store::open(&d, Limits::default());
        assert_eq!(s.items().len(), 2);
        assert!(s.items()[0].pinned && s.items()[0].kind == Kind::Image && s.items()[0].height == 3);
        assert_eq!(s.items()[1].text, "новое");
        assert!(d.join("index.json").exists() && !d.join("history.json").exists() && !img.exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_large_and_empty() {
        let d = tmp("b");
        let mut s = Store::open(&d, Limits { max_image_bytes: 3, ..Default::default() });
        assert_eq!(s.add_text("  ", None), Err(Skip::Empty));
        assert_eq!(s.add_image(&[1, 2, 3, 4], 1, 1, None), Err(Skip::TooLarge));
        let id = s.add_image(&[1, 2], 1, 1, Some(&[9])).unwrap();
        let item = s.get(&id).unwrap().clone();
        assert!(s.image_path(&item).exists());
        s.remove(&id);
        assert!(!s.image_path(&item).exists());
        let _ = fs::remove_dir_all(&d);
    }
}

/// Настройки Clipboard History.
pub mod settings {
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(default)]
    pub struct Settings {
        /// Сколько незакреплённых записей хранить (у Windows — 25).
        pub max_items: usize,
        /// Максимальный размер картинки, МБ (у Windows — 4).
        pub max_image_mb: usize,
        /// Сразу вставлять выбранное в окно, где вы были (иначе только копировать).
        pub auto_paste: bool,
        /// Токен портала Remote Desktop, чтобы GNOME не спрашивал разрешение каждый раз.
        pub paste_token: String,
    }

    impl Default for Settings {
        fn default() -> Self {
            Settings { max_items: 200, max_image_mb: 16, auto_paste: true, paste_token: String::new() }
        }
    }

    impl Settings {
        pub fn limits(&self) -> super::Limits {
            super::Limits {
                max_items: self.max_items.max(1),
                max_image_bytes: self.max_image_mb.max(1) * 1024 * 1024,
                ..Default::default()
            }
        }
    }

    fn path() -> std::path::PathBuf {
        lens_core::paths::config_dir(super::APP).join("config.json")
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(s: &Settings) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?;
        lens_core::paths::write_atomic(&path(), &data)
    }
}
