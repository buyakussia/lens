//! Картинки в памяти: обрезка снимка экрана, уменьшение перед отправкой, вложения из файлов.

use gtk::gdk;
use gtk::gdk_pixbuf::{Colorspace, InterpType, Pixbuf};
use gtk::glib;
use gtk::prelude::*;
use lens_core::claude::Attachment;
use std::path::Path;
use std::sync::Arc;

/// Больше Claude всё равно не смотрит — уменьшаем заранее, чтобы ответ был быстрее.
const MAX_SIDE: i32 = 1568;

/// Пиксели текстуры в формате RGBA (скачиваются один раз на снимок).
pub struct Pixels {
    pub bytes: glib::Bytes,
    pub stride: usize,
    pub width: i32,
    pub height: i32,
}

pub fn download(tex: &gdk::Texture) -> Pixels {
    let mut d = gdk::TextureDownloader::new(tex);
    d.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = d.download_bytes();
    Pixels { bytes, stride, width: tex.width(), height: tex.height() }
}

/// Вырезает прямоугольник (в пикселях снимка) в новую текстуру.
pub fn crop(px: &Pixels, x: i32, y: i32, w: i32, h: i32) -> Option<gdk::MemoryTexture> {
    let x = x.clamp(0, px.width - 1);
    let y = y.clamp(0, px.height - 1);
    let w = w.clamp(1, px.width - x);
    let h = h.clamp(1, px.height - y);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let src = px.bytes.as_ref();
    for row in y..y + h {
        let start = row as usize * px.stride + x as usize * 4;
        out.extend_from_slice(src.get(start..start + w as usize * 4)?);
    }
    Some(gdk::MemoryTexture::new(
        w,
        h,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(out),
        (w * 4) as usize,
    ))
}

fn pixbuf_of(tex: &gdk::Texture) -> Pixbuf {
    let px = download(tex);
    Pixbuf::from_bytes(&px.bytes, Colorspace::Rgb, true, 8, px.width, px.height, px.stride as i32)
}

/// PNG для отправки Claude: уменьшенный, если снимок слишком большой.
pub fn png_for_claude(tex: &gdk::Texture) -> Vec<u8> {
    let (w, h) = (tex.width(), tex.height());
    if w.max(h) <= MAX_SIDE {
        return tex.save_to_png_bytes().to_vec();
    }
    let k = MAX_SIDE as f64 / w.max(h) as f64;
    let pb = pixbuf_of(tex);
    let small = pb
        .scale_simple(((w as f64 * k) as i32).max(1), ((h as f64 * k) as i32).max(1), InterpType::Hyper)
        .unwrap_or(pb);
    small.save_to_bufferv("png", &[]).unwrap_or_default()
}

pub fn image_attachment(tex: &gdk::Texture, label: &str) -> Attachment {
    Attachment::Image { mime: "image/png".into(), bytes: Arc::new(png_for_claude(tex)), label: label.into() }
}

const TEXT_EXT: &[&str] = &[
    "txt", "md", "markdown", "rs", "py", "js", "ts", "tsx", "jsx", "json", "toml", "yaml", "yml", "xml", "html",
    "css", "scss", "c", "h", "cpp", "hpp", "cc", "java", "kt", "go", "rb", "php", "sh", "bash", "zsh", "fish",
    "sql", "csv", "tsv", "log", "ini", "cfg", "conf", "swift", "cs", "lua", "pl", "r", "dart", "vue", "svelte",
];
const MAX_FILE: u64 = 25 * 1024 * 1024;
const MAX_INLINE_TEXT: u64 = 400 * 1024;

/// Вложение из файла и (для картинок) превью.
pub fn from_file(path: &Path) -> Result<(Attachment, Option<gdk::Texture>), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        return Ok((Attachment::File { path: path.to_path_buf(), label: name }, None));
    }
    if meta.len() > MAX_FILE {
        return Err(format!("«{name}» больше 25 МБ"));
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => {
            let tex = gdk::Texture::from_filename(path).map_err(|e| e.to_string())?;
            Ok((image_attachment(&tex, &name), Some(tex)))
        }
        "pdf" => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            Ok((Attachment::Pdf { bytes: Arc::new(bytes), label: name }, None))
        }
        _ => {
            if meta.len() <= MAX_INLINE_TEXT && (TEXT_EXT.contains(&ext.as_str()) || ext.is_empty()) {
                if let Ok(text) = std::fs::read_to_string(path) {
                    return Ok((Attachment::Text { text, label: name, quote: false }, None));
                }
            }
            Ok((Attachment::File { path: path.to_path_buf(), label: name }, None))
        }
    }
}
