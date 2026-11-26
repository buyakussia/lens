//! Настройки приложения: JSON-файл в папке конфигурации + значения по умолчанию.

use serde_json::{json, Value};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

pub fn default_actions() -> Value {
    json!([
        {"name": "Перевести", "prompt": "Переведи весь текст из вложения на русский (если он уже на русском — на английский). Сохрани форматирование, без лишних пояснений."},
        {"name": "Объяснить", "prompt": "Объясни простыми словами, что здесь изображено или написано и что это значит."},
        {"name": "Распознать текст", "prompt": "Распознай и перепиши весь текст из вложения дословно, сохранив структуру (абзацы, списки, таблицы). Без комментариев."},
        {"name": "Исправить код", "prompt": "Найди ошибки в коде или в сообщении об ошибке во вложении, коротко объясни причину и дай исправленный код целиком в блоке кода."},
        {"name": "Кратко", "prompt": "Перескажи суть в 2–3 предложениях."}
    ])
}

pub fn defaults() -> Value {
    json!({
        "model": "",
        "system_prompt": "Отвечай по-русски, кратко.",
        "shots_dir": "",
        "shortcuts": {
            "panel": "Alt+Shift+Space",
            "capture": "Alt+Shift+S",
            "selection": "Alt+Shift+C"
        },
        "actions": default_actions()
    })
}

/// Рекурсивно накладывает patch на base: объекты сливаются, остальное заменяется.
pub fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, v) => *slot = v.clone(),
    }
}

fn path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("config.json")
}

pub fn load(app: &AppHandle) -> Value {
    let mut cfg = defaults();
    if let Ok(text) = fs::read_to_string(path(app)) {
        if let Ok(saved) = serde_json::from_str::<Value>(&text) {
            merge(&mut cfg, &saved);
        }
    }
    cfg
}

pub fn save(app: &AppHandle, cfg: &Value) -> Result<(), String> {
    let p = path(app);
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn get_str(cfg: &Value, key: &str) -> String {
    cfg.get(key).and_then(|v| v.as_str()).unwrap_or("").trim().to_string()
}

/// Папка снимков: из настроек, иначе «Изображения/Lens».
pub fn shots_dir(app: &AppHandle) -> PathBuf {
    let custom = get_str(&load(app), "shots_dir");
    if !custom.is_empty() {
        return PathBuf::from(custom);
    }
    app.path()
        .picture_dir()
        .unwrap_or_else(|_| app.path().home_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("Lens")
}

/// Рабочая папка Claude Code: к ней привязаны его сессии, поэтому она постоянная.
pub fn work_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("work")
}

pub fn history_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("history")
}
