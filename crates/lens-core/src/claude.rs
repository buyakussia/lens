//! Запуск Claude Code (`claude -p`) с потоковым ответом.
//!
//! Сообщение передаётся через stdin в формате stream-json: картинки и PDF идут прямо
//! в сообщении (base64), поэтому снимки экрана не нужно сохранять на диск.

use base64::Engine;
use serde_json::{json, Value};
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

/// Вложение к сообщению.
#[derive(Clone, Debug)]
pub enum Attachment {
    /// Картинка в памяти (например, выделенная область экрана). mime: image/png, image/jpeg…
    Image { mime: String, bytes: Arc<Vec<u8>>, label: String },
    /// PDF-документ в памяти.
    Pdf { bytes: Arc<Vec<u8>>, label: String },
    /// Текст: выделенный мышью (quote = true) или вставленный / из текстового файла.
    Text { text: String, label: String, quote: bool },
    /// Прочий файл на диске: Claude откроет его сам инструментом Read.
    File { path: PathBuf, label: String },
}

impl Attachment {
    pub fn label(&self) -> &str {
        match self {
            Attachment::Image { label, .. }
            | Attachment::Pdf { label, .. }
            | Attachment::Text { label, .. }
            | Attachment::File { label, .. } => label,
        }
    }
    pub fn is_image(&self) -> bool {
        matches!(self, Attachment::Image { .. })
    }
}

/// Что спросить и как.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub question: String,
    pub attachments: Vec<Attachment>,
    pub model: String,
    pub system_prompt: String,
    /// Дополнительное указание к ответу (например, «очень коротко» для тихого режима).
    pub style_hint: String,
    /// id сессии Claude Code, чтобы продолжить разговор.
    pub session: Option<String>,
    pub timeout: Duration,
}

/// События, которые приходят во время ответа.
#[derive(Clone, Debug)]
pub enum Event {
    /// Очередной кусочек текста ответа.
    Delta(String),
    /// Claude начал пользоваться инструментом (WebSearch, Read…).
    Tool(String),
    /// Стал известен id сессии.
    Session(String),
    /// Ответ закончен.
    Done(Outcome),
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub result: Option<String>,
    pub is_error: bool,
    pub stopped: bool,
    pub timed_out: bool,
    pub stderr: String,
    pub session: Option<String>,
}

impl Outcome {
    /// Понятное сообщение об ошибке, если она была.
    pub fn error_text(&self) -> Option<String> {
        if self.stopped {
            return None;
        }
        if self.timed_out {
            return Some("Claude не ответил вовремя и был остановлен.".into());
        }
        if self.is_error || self.result.is_none() {
            let raw = self
                .result
                .clone()
                .filter(|r| !r.trim().is_empty())
                .unwrap_or_else(|| self.stderr.trim().to_string());
            let low = raw.to_lowercase();
            if low.contains("login") || low.contains("log in") || low.contains("authenticat") {
                return Some("Claude Code не авторизован. Выполните в терминале: claude, затем /login.".into());
            }
            return Some(if raw.is_empty() { "Claude Code завершился без ответа.".into() } else { raw });
        }
        None
    }
}

/// Управление запущенным ответом.
#[derive(Clone)]
pub struct Handle {
    child: Arc<Mutex<Option<Child>>>,
    stopped: Arc<AtomicBool>,
}

impl Handle {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// Папки, где обычно лежит claude. У программ, запущенных из меню, PATH бывает почти пустым.
fn extra_dirs() -> Vec<PathBuf> {
    let h = home();
    let mut v = vec![
        h.join(".local/bin"),
        h.join(".claude/local"),
        h.join(".npm-global/bin"),
        h.join(".bun/bin"),
        h.join(".volta/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ];
    if let Some(appdata) = env::var_os("APPDATA") {
        v.push(PathBuf::from(appdata).join("npm"));
    }
    v
}

fn search_path() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(p) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&p));
    }
    dirs.extend(extra_dirs());
    dirs
}

/// Путь к команде claude или None. Переменная LENS_CLAUDE задаёт его явно.
pub fn find_claude() -> Option<PathBuf> {
    if let Some(p) = env::var_os("LENS_CLAUDE") {
        return Some(PathBuf::from(p));
    }
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    search_path()
        .into_iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

pub const NOT_FOUND: &str =
    "Не найден Claude Code (команда claude). Установите его и войдите в аккаунт: https://claude.com/claude-code";

/// Текст сообщения: вопрос + текстовые вложения + список файлов.
pub fn build_text(req: &Request) -> String {
    let mut parts: Vec<String> = vec![];
    let mut files = vec![];
    for a in &req.attachments {
        match a {
            Attachment::Text { text, quote: true, .. } => {
                parts.push(format!("Выделенный пользователем текст:\n<<<\n{text}\n>>>"))
            }
            Attachment::Text { text, label, .. } => {
                parts.push(format!("Вложение «{label}»:\n<<<\n{text}\n>>>"))
            }
            Attachment::File { path, .. } => files.push(format!("- {}", path.display())),
            _ => {}
        }
    }
    if !files.is_empty() {
        parts.push(format!("Прикреплённые файлы (открой их инструментом Read):\n{}", files.join("\n")));
    }
    let q = req.question.trim();
    let has_image = req.attachments.iter().any(|a| a.is_image());
    if !q.is_empty() {
        parts.push(q.to_string());
    } else if has_image {
        parts.push(
            "Посмотри на этот фрагмент экрана и скажи, что здесь главное. Если там текст на другом \
             языке — переведи. Если ошибка или код — объясни причину и как исправить. \
             В интернете ищи, только если без этого не ответить."
                .into(),
        );
    } else if !req.attachments.is_empty() {
        parts.push("Объясни суть. Если текст на другом языке — переведи. Если это код или ошибка — скажи причину и как исправить.".into());
    }
    if !req.style_hint.is_empty() {
        parts.push(req.style_hint.clone());
    }
    parts.join("\n\n")
}

/// Сообщение пользователя в формате stream-json.
pub fn build_message(req: &Request) -> Value {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut content = vec![];
    for a in &req.attachments {
        match a {
            Attachment::Image { mime, bytes, .. } => content.push(json!({
                "type": "image",
                "source": {"type": "base64", "media_type": mime, "data": b64.encode(bytes.as_slice())}
            })),
            Attachment::Pdf { bytes, .. } => content.push(json!({
                "type": "document",
                "source": {"type": "base64", "media_type": "application/pdf", "data": b64.encode(bytes.as_slice())}
            })),
            _ => {}
        }
    }
    content.push(json!({"type": "text", "text": build_text(req)}));
    json!({"type": "user", "message": {"role": "user", "content": content}})
}

fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

/// Запускает Claude Code. События приходят в `on_event` из фонового потока.
/// `work_dir` — постоянная папка: к ней привязаны сессии Claude Code.
pub fn ask<F>(req: Request, work_dir: &Path, on_event: F) -> Result<Handle, String>
where
    F: Fn(Event) + Send + Sync + 'static,
{
    let claude = find_claude().ok_or(NOT_FOUND)?;
    std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;

    let mut cmd = Command::new(&claude);
    cmd.current_dir(work_dir)
        .env("PATH", env::join_paths(search_path()).unwrap_or_default())
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--allowedTools",
            "Read,WebSearch,WebFetch",
        ]);
    let mut dirs: Vec<&Path> = vec![];
    for a in &req.attachments {
        if let Attachment::File { path, .. } = a {
            if let Some(d) = path.parent() {
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
    }
    for d in dirs {
        cmd.arg("--add-dir").arg(d);
    }
    if !req.model.trim().is_empty() {
        cmd.args(["--model", req.model.trim()]);
    }
    if !req.system_prompt.trim().is_empty() {
        cmd.args(["--append-system-prompt", req.system_prompt.trim()]);
    }
    if let Some(s) = req.session.as_deref().filter(|s| !s.is_empty()) {
        cmd.args(["--resume", s]);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let mut child = cmd.spawn().map_err(|e| format!("Не удалось запустить Claude Code: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("нет stdin")?;
    let stdout = child.stdout.take().ok_or("нет stdout")?;
    let mut stderr = child.stderr.take().ok_or("нет stderr")?;

    let handle = Handle { child: Arc::new(Mutex::new(Some(child))), stopped: Arc::new(AtomicBool::new(false)) };
    let timed_out = Arc::new(AtomicBool::new(false));

    // сообщение пишем в отдельном потоке: большая картинка может не влезть в буфер канала
    let msg = build_message(&req);
    thread::spawn(move || {
        let mut line = serde_json::to_vec(&msg).unwrap_or_default();
        line.push(b'\n');
        let _ = stdin.write_all(&line);
        // stdin закрывается при выходе из потока — так Claude понимает, что сообщение одно
    });

    let err_buf = Arc::new(Mutex::new(String::new()));
    {
        let err_buf = err_buf.clone();
        thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            *err_buf.lock().unwrap() = s;
        });
    }

    // страховка от зависания
    {
        let h = handle.clone();
        let timed_out = timed_out.clone();
        let timeout = if req.timeout.is_zero() { Duration::from_secs(300) } else { req.timeout };
        thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < timeout {
                thread::sleep(Duration::from_millis(500));
                if h.child.lock().unwrap().is_none() {
                    return;
                }
            }
            if let Some(c) = h.child.lock().unwrap().as_mut() {
                timed_out.store(true, Ordering::SeqCst);
                let _ = c.kill();
            }
        });
    }

    let h = handle.clone();
    thread::spawn(move || {
        let mut out = Outcome { session: req.session.clone(), ..Default::default() };
        let mut streamed = false;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Ok(d) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(s) = d.get("session_id").and_then(Value::as_str) {
                if out.session.as_deref() != Some(s) {
                    out.session = Some(s.to_string());
                    on_event(Event::Session(s.to_string()));
                }
            }
            match d.get("type").and_then(Value::as_str) {
                Some("stream_event") => {
                    let ev = &d["event"];
                    match ev.get("type").and_then(Value::as_str) {
                        Some("content_block_delta") if ev["delta"]["type"] == "text_delta" => {
                            if let Some(t) = ev["delta"]["text"].as_str() {
                                streamed = true;
                                on_event(Event::Delta(t.to_string()));
                            }
                        }
                        Some("content_block_start") if ev["content_block"]["type"] == "tool_use" => {
                            let n = ev["content_block"]["name"].as_str().unwrap_or("");
                            on_event(Event::Tool(n.to_string()));
                        }
                        _ => {}
                    }
                }
                // без --include-partial-messages (старые версии) текст приходит целиком
                Some("assistant") if !streamed => {
                    if let Some(arr) = d["message"]["content"].as_array() {
                        for c in arr {
                            if c["type"] == "text" {
                                if let Some(t) = c["text"].as_str() {
                                    on_event(Event::Delta(t.to_string()));
                                }
                            }
                        }
                    }
                }
                Some("result") => {
                    out.result = d.get("result").and_then(Value::as_str).map(String::from);
                    out.is_error = d.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                }
                _ => {}
            }
        }
        if let Some(mut c) = h.child.lock().unwrap().take() {
            let _ = c.wait();
        }
        thread::sleep(Duration::from_millis(30)); // даём дочитать stderr
        out.stderr = tail(&err_buf.lock().unwrap(), 1500);
        out.stopped = h.stopped.load(Ordering::SeqCst);
        out.timed_out = timed_out.load(Ordering::SeqCst);
        on_event(Event::Done(out));
    });
    Ok(handle)
}

/// Название инструмента для строки состояния.
pub fn tool_status(name: &str) -> &'static str {
    match name {
        "WebSearch" => "Ищет в интернете…",
        "WebFetch" => "Читает страницу…",
        "Read" => "Открывает вложение…",
        _ => "Думает…",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_has_image_then_text() {
        let req = Request {
            question: "что это?".into(),
            attachments: vec![Attachment::Image {
                mime: "image/png".into(),
                bytes: Arc::new(vec![1, 2, 3]),
                label: "x".into(),
            }],
            ..Default::default()
        };
        let m = build_message(&req);
        let c = m["message"]["content"].as_array().unwrap();
        assert_eq!(c[0]["type"], "image");
        assert_eq!(c[0]["source"]["data"], "AQID");
        assert_eq!(c[1]["text"], "что это?");
    }
}
