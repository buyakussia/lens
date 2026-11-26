//! История чатов: один чат — один JSON-файл, хранятся последние 100.
//! Картинки в историю не пишутся (только подпись вложения), чтобы не занимать диск:
//! сам Claude Code хранит их в своей сессии, и разговор можно продолжить.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const LIMIT: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Message {
    /// "user" или "assistant"
    pub role: String,
    pub text: String,
    /// Подписи вложений: «Фрагмент экрана», «report.pdf»…
    pub attachments: Vec<String>,
    pub error: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub created: f64,
    pub updated: f64,
    pub session: Option<String>,
    pub messages: Vec<Message>,
}

impl Chat {
    pub fn new() -> Self {
        let t = crate::now();
        Chat {
            id: format!("{:x}{:04x}", (t * 1000.0) as u64, std::process::id() & 0xffff),
            created: t,
            updated: t,
            ..Default::default()
        }
    }

    /// Заголовок из первого вопроса или вложения.
    pub fn make_title(question: &str, attachments: &[String]) -> String {
        let q: String = question.split_whitespace().collect::<Vec<_>>().join(" ");
        let base = if !q.is_empty() {
            q
        } else if let Some(a) = attachments.first() {
            a.clone()
        } else {
            "Новый чат".into()
        };
        let mut t: String = base.chars().take(60).collect();
        if base.chars().count() > 60 {
            t.push('…');
        }
        t
    }
}

pub fn dir() -> PathBuf {
    crate::paths::data_dir(crate::config::APP).join("history")
}

pub fn work_dir() -> PathBuf {
    crate::paths::data_dir(crate::config::APP).join("work")
}

fn safe(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Все чаты, новые сверху.
pub fn list() -> Vec<Chat> {
    let mut chats: Vec<Chat> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Chat>(&t).ok())
        .collect();
    chats.sort_by(|a, b| b.updated.total_cmp(&a.updated));
    chats
}

pub fn save(chat: &Chat) -> std::io::Result<()> {
    if !safe(&chat.id) || chat.messages.is_empty() {
        return Ok(());
    }
    let data = serde_json::to_vec(chat).map_err(std::io::Error::other)?;
    crate::paths::write_atomic(&dir().join(format!("{}.json", chat.id)), &data)?;
    for old in list().into_iter().skip(LIMIT) {
        delete(&old.id);
    }
    Ok(())
}

pub fn delete(id: &str) {
    if safe(id) {
        let _ = std::fs::remove_file(dir().join(format!("{id}.json")));
    }
}
