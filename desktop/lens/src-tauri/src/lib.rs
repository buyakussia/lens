mod claude;
mod config;
mod history;
mod platform;

use base64::Engine;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    thread,
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const PANEL_W: f64 = 680.0;
const MAX_FILE: u64 = 25 * 1024 * 1024;
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

fn emit(app: &AppHandle, payload: Value) {
    let _ = app.emit("lens", payload);
}

// ---------------------------------------------------------------- окно-панель
fn create_panel(app: &AppHandle) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Lens")
        .inner_size(PANEL_W, 120.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()?;
    // крестик/Cmd+W прячут панель, а не завершают приложение
    let w = win.clone();
    win.on_window_event(move |ev| {
        if let WindowEvent::CloseRequested { api, .. } = ev {
            api.prevent_close();
            let _ = w.hide();
        }
    });
    Ok(())
}

fn show_panel(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
        emit(app, json!({"kind": "shown"}));
    }
}

fn toggle_panel(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) && win.is_focused().unwrap_or(false) {
            let _ = win.hide();
        } else {
            show_panel(app);
        }
    }
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

/// Подгоняет высоту окна под содержимое; нижний край остаётся на месте — панель растёт вверх.
#[tauri::command]
fn fit_panel(app: AppHandle, height: f64) {
    let Some(win) = app.get_webview_window("main") else { return };
    let h = height.clamp(60.0, 900.0);
    let monitor = win.current_monitor().ok().flatten().or_else(|| win.primary_monitor().ok().flatten());
    let _ = win.set_size(LogicalSize::new(PANEL_W, h));
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let pos = m.position().to_logical::<f64>(scale);
        let size = m.size().to_logical::<f64>(scale);
        let x = pos.x + (size.width - PANEL_W) / 2.0;
        let y = pos.y + size.height - 90.0 - h;
        let _ = win.set_position(LogicalPosition::new(x, y.max(pos.y + 20.0)));
    }
}

// ---------------------------------------------------------------- действия по горячим клавишам
fn run_capture(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.hide(); // панель не должна попасть в кадр
        }
        let res = platform::capture_area(&app);
        show_panel(&app);
        match res {
            Ok(Some(path)) => emit(&app, json!({"kind": "attach", "type": "image", "path": path.display().to_string()})),
            Ok(None) => {}
            Err(e) => emit(&app, json!({"kind": "notice", "text": e})),
        }
    });
}

fn run_selection(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || match platform::selection_text(&app) {
        Ok(Some(text)) => {
            show_panel(&app);
            emit(&app, json!({"kind": "attach", "type": "quote", "text": text}));
        }
        Ok(None) => {
            show_panel(&app);
            emit(&app, json!({"kind": "notice", "text": "Нет выделенного текста. Выделите текст и попробуйте снова."}));
        }
        Err(e) => {
            show_panel(&app);
            emit(&app, json!({"kind": "notice", "text": e}));
        }
    });
}

#[tauri::command]
fn capture(app: AppHandle) {
    run_capture(&app);
}

#[tauri::command]
fn ask_selection(app: AppHandle) {
    run_selection(&app);
}

// ---------------------------------------------------------------- горячие клавиши
fn register_shortcuts(app: &AppHandle) -> Vec<String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let cfg = config::load(app);
    let mut errors = vec![];
    for (name, label) in [("panel", "панель"), ("capture", "снимок"), ("selection", "выделенный текст")] {
        let accel = cfg["shortcuts"][name].as_str().unwrap_or("").trim().to_string();
        if accel.is_empty() {
            continue;
        }
        let sc = match Shortcut::from_str(&accel) {
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("{label}: «{accel}» — {e}"));
                continue;
            }
        };
        let result = gs.on_shortcut(sc, move |app, _sc, ev| {
            if ev.state() != ShortcutState::Pressed {
                return;
            }
            match name {
                "panel" => toggle_panel(app),
                "capture" => run_capture(app),
                _ => run_selection(app),
            }
        });
        if let Err(e) = result {
            errors.push(format!("{label}: «{accel}» — {e} (возможно, сочетание занято другой программой)"));
        }
    }
    errors
}

#[tauri::command]
fn get_config(app: AppHandle) -> Value {
    config::load(&app)
}

#[tauri::command]
fn set_config(app: AppHandle, patch: Value) -> Result<Value, String> {
    let mut cfg = config::load(&app);
    config::merge(&mut cfg, &patch);
    config::save(&app, &cfg)?;
    let errors = if patch.get("shortcuts").is_some() { register_shortcuts(&app) } else { vec![] };
    Ok(json!({"config": cfg, "shortcut_errors": errors}))
}

// ---------------------------------------------------------------- вложения и буфер обмена
fn is_text_file(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 4096];
    let Ok(mut f) = fs::File::open(path) else { return false };
    let n = f.read(&mut buf).unwrap_or(0);
    // обрезанный на границе буфера символ UTF-8 не считаем ошибкой
    let slice = &buf[..n];
    let valid = match std::str::from_utf8(slice) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    valid && !slice.contains(&0)
}

/// Файл с диска → копия в папке снимков (чтобы Claude прочитал его, даже если оригинал переместят).
#[tauri::command]
fn add_file(app: AppHandle, path: String) -> Result<Value, String> {
    let src = PathBuf::from(&path);
    let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    let meta = fs::metadata(&src).map_err(|e| format!("{name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name}: это не файл"));
    }
    if meta.len() > MAX_FILE {
        return Err(format!("{name}: файл больше 25 МБ"));
    }
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let kind = if IMAGE_EXT.contains(&ext.as_str()) {
        "image"
    } else if ext == "pdf" {
        "pdf"
    } else if is_text_file(&src) {
        "file"
    } else {
        return Err(format!("{name}: Claude читает картинки, PDF и текстовые файлы"));
    };
    let dir = config::shots_dir(&app).join("files");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dst = dir.join(format!("{stamp}_{name}"));
    fs::copy(&src, &dst).map_err(|e| e.to_string())?;
    Ok(json!({"kind": kind, "path": dst.display().to_string(), "label": name}))
}

/// Картинка из буфера обмена, вставленная в окно (приходит base64).
#[tauri::command]
fn save_pasted_image(app: AppHandle, b64: String) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("картинка больше 25 МБ".into());
    }
    let path = platform::new_shot_path(&app)?;
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Миниатюра для интерфейса. Читаем только файлы из папки снимков.
#[tauri::command]
fn file_data_url(app: AppHandle, path: String) -> Result<String, String> {
    let root = fs::canonicalize(config::shots_dir(&app)).map_err(|e| e.to_string())?;
    let p = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    if !p.starts_with(&root) {
        return Err("файл вне папки снимков".into());
    }
    if fs::metadata(&p).map_err(|e| e.to_string())?.len() > 15 * 1024 * 1024 {
        return Err("слишком большая картинка для превью".into());
    }
    let mime = match p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    };
    let bytes = fs::read(&p).map_err(|e| e.to_string())?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[tauri::command]
fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// Открывает только http(s)-ссылки из ответов Claude.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("разрешены только http(s)-ссылки".into());
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).status();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &url]).status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).status();
    r.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn autostart_get(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn autostart_set(app: AppHandle, on: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if on { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- значок в строке меню / трее
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Открыть панель", true, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "Снимок области", true, None::<&str>)?;
    let selection = MenuItem::with_id(app, "selection", "Спросить про выделенный текст", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Настройки…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выйти", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep1, &capture, &selection, &sep2, &settings, &quit])?;

    TrayIconBuilder::with_id("lens-tray")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true) // в строке меню macOS значок сам подстраивается под светлую/тёмную тему
        .tooltip("Lens")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => show_panel(app),
            "capture" => run_capture(app),
            "selection" => run_selection(app),
            "settings" => {
                show_panel(app);
                emit(app, json!({"kind": "open-settings"}));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                toggle_panel(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    let background = std::env::args().any(|a| a == "--background");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_panel(app)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .manage(claude::Running::default())
        .invoke_handler(tauri::generate_handler![
            claude::ask,
            claude::stop,
            claude::claude_info,
            get_config,
            set_config,
            capture,
            ask_selection,
            add_file,
            save_pasted_image,
            file_data_url,
            copy_text,
            open_url,
            hide_panel,
            fit_panel,
            autostart_get,
            autostart_set,
            history::history_list,
            history::history_save,
            history::history_delete,
            platform::check_permissions,
            platform::open_privacy_settings,
        ])
        .setup(move |app| {
            // без значка в Dock: приложение живёт в строке меню
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            create_panel(&handle)?;
            build_tray(&handle)?;
            let errors = register_shortcuts(&handle);
            for e in &errors {
                eprintln!("shortcut: {e}");
            }
            if !background {
                show_panel(&handle); // запуск из Finder/меню — сразу показываем панель
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("ошибка запуска Lens");
}
