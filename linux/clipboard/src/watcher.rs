//! Слежение за буфером обмена: всё, что копируется в любом приложении, попадает в историю.
//!
//! Как в Windows: принимаются обычный текст, форматированный текст (HTML) и картинки.
//! Файлы и папки не записываются, как и то, что менеджеры паролей помечают секретным.

use clip_core::Store;
use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const SECRET: &[&str] = &["x-kde-passwordManagerHint", "x-kde-passwordmanagerhint"];
const FILES: &[&str] = &["text/uri-list", "x-special/gnome-copied-files", "application/x-kde-cutselection"];
const THUMB_W: i32 = 320;
const THUMB_H: i32 = 180;

pub struct Watcher {
    store: Rc<RefCell<Store>>,
    clipboard: gdk::Clipboard,
    pending: Cell<bool>,
    /// Своё копирование (при вставке из истории) не записываем повторно.
    ignore_until: Cell<f64>,
    pub on_change: RefCell<Option<Box<dyn Fn()>>>,
}

fn decode_text(bytes: &[u8]) -> String {
    // некоторые приложения отдают HTML в UTF-16
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u);
    }
    String::from_utf8_lossy(bytes).trim_end_matches('\0').to_string()
}

/// Маленькое превью картинки для списка (чтобы не держать в памяти большие изображения).
pub fn thumbnail(png: &[u8]) -> Option<Vec<u8>> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(png));
    let pb = Pixbuf::from_stream(&stream, gio::Cancellable::NONE).ok()?;
    let (w, h) = (pb.width() as f64, pb.height() as f64);
    let k = (THUMB_W as f64 / w).min(THUMB_H as f64 / h).min(1.0);
    let small = pb.scale_simple(((w * k) as i32).max(1), ((h * k) as i32).max(1), InterpType::Bilinear)?;
    small.save_to_bufferv("png", &[]).ok()
}

impl Watcher {
    pub fn new(store: Rc<RefCell<Store>>) -> Rc<Watcher> {
        let clipboard = gdk::Display::default().expect("нет дисплея").clipboard();
        let w = Rc::new(Watcher {
            store,
            clipboard: clipboard.clone(),
            pending: Cell::new(false),
            ignore_until: Cell::new(0.0),
            on_change: RefCell::new(None),
        });
        let weak = Rc::downgrade(&w);
        clipboard.connect_changed(move |_| {
            let Some(w) = weak.upgrade() else { return };
            if w.pending.replace(true) {
                return;
            }
            // даём приложению-источнику закончить копирование
            let w2 = w.clone();
            glib::timeout_add_local_once(Duration::from_millis(150), move || w2.capture());
        });
        w
    }

    pub fn ignore(&self, secs: f64) {
        self.ignore_until.set(lens_core::now() + secs);
    }

    fn changed(&self) {
        let _ = self.store.borrow().save();
        if let Some(f) = self.on_change.borrow().as_ref() {
            f();
        }
    }

    fn capture(self: &Rc<Self>) {
        self.pending.set(false);
        if lens_core::now() < self.ignore_until.get() || self.clipboard.is_local() {
            return;
        }
        let formats = self.clipboard.formats();
        let mimes: Vec<String> = formats.mime_types().iter().map(|m| m.to_string()).collect();
        if mimes.iter().any(|m| SECRET.contains(&m.as_str())) {
            return; // менеджер паролей пометил содержимое как секретное
        }
        if mimes.iter().any(|m| FILES.contains(&m.as_str())) {
            return; // файлы и папки в историю не попадают
        }
        let has_text = mimes.iter().any(|m| m.starts_with("text/plain") || m == "UTF8_STRING" || m == "STRING");
        let has_image = mimes.iter().any(|m| m.starts_with("image/"));
        if has_image && !has_text {
            let w = self.clone();
            self.clipboard.read_texture_async(gio::Cancellable::NONE, move |r| {
                let Ok(Some(tex)) = r else { return };
                let png = tex.save_to_png_bytes();
                let thumb = thumbnail(&png);
                let res = w.store.borrow_mut().add_image(&png, tex.width() as u32, tex.height() as u32, thumb.as_deref());
                if res.is_ok() {
                    w.changed();
                }
            });
            return;
        }
        if !has_text {
            return;
        }
        let has_html = mimes.iter().any(|m| m == "text/html");
        let w = self.clone();
        self.clipboard.read_text_async(gio::Cancellable::NONE, move |r| {
            let Ok(Some(text)) = r else { return };
            let text = text.to_string();
            if !has_html {
                if w.store.borrow_mut().add_text(&text, None).is_ok() {
                    w.changed();
                }
                return;
            }
            let w2 = w.clone();
            w.clipboard.read_async(&["text/html"], glib::Priority::DEFAULT, gio::Cancellable::NONE, move |r| {
                let html = match r {
                    Ok((stream, _)) => {
                        let out = gio::MemoryOutputStream::new_resizable();
                        let _ = out.splice(&stream, gio::OutputStreamSpliceFlags::CLOSE_SOURCE | gio::OutputStreamSpliceFlags::CLOSE_TARGET, gio::Cancellable::NONE);
                        Some(decode_text(&out.steal_as_bytes()))
                    }
                    Err(_) => None,
                };
                if w2.store.borrow_mut().add_text(&text, html.as_deref()).is_ok() {
                    w2.changed();
                }
            });
        });
    }
}
