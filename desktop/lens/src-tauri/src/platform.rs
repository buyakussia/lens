//! То, что зависит от системы: снимок области, выделенный текст, разрешения.
//! macOS реализован полностью; Windows и Linux пока возвращают понятную ошибку.

use crate::config;
use serde_json::{json, Value};
use std::{fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};
use tauri::AppHandle;

fn stamp() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

pub fn new_shot_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = config::shots_dir(app);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{}.png", stamp())))
}

// ---------------------------------------------------------------- macOS
#[cfg(target_os = "macos")]
mod mac {
    use core_graphics::access::ScreenCaptureAccess;
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    pub fn screen_allowed() -> bool {
        ScreenCaptureAccess.preflight()
    }
    pub fn request_screen() -> bool {
        ScreenCaptureAccess.request()
    }
    pub fn accessibility_allowed() -> bool {
        macos_accessibility_client::accessibility::application_is_trusted()
    }
    pub fn request_accessibility() -> bool {
        macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
    }

    /// Нажимает ⌘+клавиша. Коды — виртуальные (не зависят от раскладки): C = 8, V = 9.
    pub fn press_cmd(key: CGKeyCode) -> Result<(), String> {
        let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "не удалось создать источник событий".to_string())?;
        for down in [true, false] {
            let ev = CGEvent::new_keyboard_event(src.clone(), key, down)
                .map_err(|_| "не удалось создать событие клавиши".to_string())?;
            ev.set_flags(CGEventFlags::CGEventFlagCommand);
            ev.post(CGEventTapLocation::HID);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub fn capture_area(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    if !mac::screen_allowed() {
        mac::request_screen();
        return Err("Нужно разрешение «Запись экрана»: Системные настройки → Конфиденциальность и безопасность → \
                    Запись экрана → включите Lens и перезапустите приложение.".into());
    }
    let path = new_shot_path(app)?;
    // -i — выделение области мышью (пробел — выбрать окно), -x — без звука; Esc — отмена
    std::process::Command::new("screencapture")
        .args(["-i", "-x"])
        .arg(&path)
        .status()
        .map_err(|e| format!("screencapture: {e}"))?;
    let ok = fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false);
    Ok(ok.then_some(path))
}

#[cfg(target_os = "macos")]
pub fn selection_text(app: &AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    if !mac::accessibility_allowed() {
        mac::request_accessibility();
        return Err("Нужно разрешение «Универсальный доступ»: Системные настройки → Конфиденциальность и \
                    безопасность → Универсальный доступ → включите Lens.".into());
    }
    let cb = app.clipboard();
    let old = cb.read_text().ok();
    let sentinel = format!("__lens_{}__", stamp());
    let _ = cb.write_text(sentinel.clone());
    mac::press_cmd(8)?; // ⌘C в приложении, где выделен текст
    let mut got = None;
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if let Ok(t) = cb.read_text() {
            if t != sentinel {
                got = Some(t);
                break;
            }
        }
    }
    // возвращаем буфер как был
    match old {
        Some(o) => {
            let _ = cb.write_text(o);
        }
        None => {
            let _ = cb.clear();
        }
    }
    Ok(got.filter(|t| !t.trim().is_empty()))
}

#[cfg(target_os = "macos")]
fn perms() -> (bool, bool) {
    (mac::screen_allowed(), mac::accessibility_allowed())
}

#[cfg(target_os = "macos")]
pub fn open_privacy(which: &str) {
    let pane = if which == "screen" { "Privacy_ScreenCapture" } else { "Privacy_Accessibility" };
    let _ = std::process::Command::new("open")
        .arg(format!("x-apple.systempreferences:com.apple.preference.security?{pane}"))
        .status();
}

// ---------------------------------------------------------------- остальные системы (пока)
#[cfg(not(target_os = "macos"))]
pub fn capture_area(_app: &AppHandle) -> Result<Option<PathBuf>, String> {
    Err("Снимок экрана пока доступен только на macOS. Прикрепите картинку перетаскиванием или вставкой.".into())
}

#[cfg(not(target_os = "macos"))]
pub fn selection_text(_app: &AppHandle) -> Result<Option<String>, String> {
    Err("Выделенный текст пока доступен только на macOS. Скопируйте текст и вставьте его в панель (Ctrl+V).".into())
}

#[cfg(not(target_os = "macos"))]
fn perms() -> (bool, bool) {
    (true, true)
}

#[cfg(not(target_os = "macos"))]
pub fn open_privacy(_which: &str) {}

#[tauri::command]
pub fn check_permissions() -> Value {
    let (screen, accessibility) = perms();
    json!({ "os": std::env::consts::OS, "screen": screen, "accessibility": accessibility })
}

#[tauri::command]
pub fn open_privacy_settings(which: String) {
    open_privacy(&which);
}
