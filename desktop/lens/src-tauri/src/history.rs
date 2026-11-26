//! История чатов: один чат — один JSON-файл, хранятся последние 100.

use crate::config;
use serde_json::Value;
use std::fs;
use tauri::AppHandle;

const LIMIT: usize = 100;

fn safe_id(id: &str) -> Option<String> {
    let ok = !id.is_empty()
        && id.len() <= 80
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| id.to_string())
}

fn load_all(app: &AppHandle) -> Vec<(f64, Value)> {
    let mut chats = vec![];
    if let Ok(rd) = fs::read_dir(config::history_dir(app)) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(e.path()) {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    let t = v.get("updated").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    chats.push((t, v));
                }
            }
        }
    }
    chats.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    chats
}

#[tauri::command]
pub fn history_list(app: AppHandle) -> Vec<Value> {
    load_all(&app).into_iter().take(LIMIT).map(|c| c.1).collect()
}

#[tauri::command]
pub fn history_save(app: AppHandle, chat: Value) -> Result<(), String> {
    let id = chat
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(safe_id)
        .ok_or("некорректный id чата")?;
    let dir = config::history_dir(&app);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{id}.json.tmp"));
    fs::write(&tmp, serde_json::to_string(&chat).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, dir.join(format!("{id}.json"))).map_err(|e| e.to_string())?;
    for (_, old) in load_all(&app).into_iter().skip(LIMIT) {
        if let Some(oid) = old.get("id").and_then(|v| v.as_str()).and_then(safe_id) {
            let _ = fs::remove_file(dir.join(format!("{oid}.json")));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn history_delete(app: AppHandle, id: String) -> Result<(), String> {
    let id = safe_id(&id).ok_or("некорректный id чата")?;
    let _ = fs::remove_file(config::history_dir(&app).join(format!("{id}.json")));
    Ok(())
}
