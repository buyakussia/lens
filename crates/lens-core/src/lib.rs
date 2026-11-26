//! Ядро Lens без привязки к GUI. Используется версией для Linux (GTK4) и пригодится
//! будущим нативным версиям для macOS (SwiftUI) и Windows (WinUI) через FFI.

pub mod claude;
pub mod config;
pub mod history;
pub mod paths;

/// Время в секундах с 1970 года (как f64 — так его удобно хранить в JSON).
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// «только что», «5 мин назад», «3 ч назад», «вчера», «4 дн назад».
pub fn ago(ts: f64) -> String {
    let d = (now() - ts).max(0.0);
    if d < 60.0 {
        "только что".into()
    } else if d < 3600.0 {
        format!("{} мин назад", (d / 60.0) as u64)
    } else if d < 86400.0 {
        format!("{} ч назад", (d / 3600.0) as u64)
    } else if d < 2.0 * 86400.0 {
        "вчера".into()
    } else {
        format!("{} дн назад", (d / 86400.0) as u64)
    }
}
