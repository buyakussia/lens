//! Настройки Lens. Неизвестные и отсутствующие поля заполняются значениями по умолчанию,
//! поэтому старый файл настроек не ломается при обновлении.

use serde::{Deserialize, Serialize};

pub const APP: &str = "lens";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Action {
    pub name: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Модель для Claude Code: "" — по умолчанию, иначе opus / sonnet / haiku или полный id.
    pub model: String,
    /// Модель для тихого режима (быстрая по умолчанию).
    pub quiet_model: String,
    pub system_prompt: String,
    pub actions: Vec<Action>,
    /// Сколько секунд ждать ответа, прежде чем остановить Claude.
    pub timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: String::new(),
            quiet_model: "haiku".into(),
            system_prompt: "Отвечай по-русски, кратко.".into(),
            actions: default_actions(),
            timeout_secs: 300,
        }
    }
}

pub fn default_actions() -> Vec<Action> {
    let a = |n: &str, p: &str| Action { name: n.into(), prompt: p.into() };
    vec![
        a("Перевести", "Переведи весь текст из вложения на русский (если он уже на русском — на английский). Сохрани форматирование, без лишних пояснений."),
        a("Объяснить", "Объясни простыми словами, что здесь изображено или написано и что это значит."),
        a("Распознать текст", "Распознай и перепиши весь текст из вложения дословно, сохранив структуру (абзацы, списки, таблицы). Без комментариев."),
        a("Исправить код", "Найди ошибки в коде или в сообщении об ошибке во вложении, коротко объясни причину и дай исправленный код целиком в блоке кода."),
        a("Кратко", "Перескажи суть в 2–3 предложениях."),
    ]
}

/// Модели, которые показываются в переключателе в чате: (значение для --model, подпись).
pub const MODELS: &[(&str, &str)] = &[
    ("", "Авто"),
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("haiku", "Haiku"),
];

pub fn model_label(id: &str) -> String {
    MODELS
        .iter()
        .find(|(m, _)| *m == id)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| id.to_string())
}

fn path() -> std::path::PathBuf {
    crate::paths::config_dir(APP).join("config.json")
}

pub fn load() -> Config {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let data = serde_json::to_vec_pretty(cfg).map_err(std::io::Error::other)?;
    crate::paths::write_atomic(&path(), &data)
}
