//! Папки приложения: настройки и данные по стандартам каждой ОС
//! (Linux: ~/.config/<app>, ~/.local/share/<app>; macOS: ~/Library/...; Windows: %APPDATA%).

use std::path::PathBuf;

pub fn config_dir(app: &str) -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(app)
}

pub fn data_dir(app: &str) -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(app)
}

/// Записывает файл атомарно: сначала во временный, потом переименование.
/// Так при сбое питания не остаётся наполовину записанного JSON.
pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}
