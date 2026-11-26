//! Запуск Claude Code (`claude -p`) и потоковая передача ответа в окно.
//!
//! Промпт передаётся через stdin: так не нужно экранировать аргументы (особенно в Windows)
//! и нет ограничений на длину.

use crate::config;
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};

const TIMEOUT_SECS: u64 = 300;

#[derive(Default)]
pub struct Running {
    child: Mutex<Option<Child>>,
    stopped: AtomicBool,
    generation: AtomicU64,
}

fn home() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Папки, где обычно лежит claude. У программ, запущенных из Dock/меню, PATH почти пустой.
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
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(local).join("Programs").join("claude"));
    }
    v
}

pub fn find_claude() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    let mut dirs = extra_dirs();
    if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path));
    }
    for d in dirs {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    // последняя попытка: спросить оболочку входа (подхватит PATH из ~/.zshrc)
    #[cfg(unix)]
    {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = Command::new(shell).args(["-lc", "command -v claude"]).output() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() && PathBuf::from(&s).is_file() {
                return Some(PathBuf::from(s));
            }
        }
    }
    None
}

fn child_path_env() -> std::ffi::OsString {
    let mut dirs = extra_dirs();
    if let Some(p) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&p));
    }
    env::join_paths(dirs).unwrap_or_default()
}

fn emit(app: &AppHandle, payload: Value) {
    let _ = app.emit("lens", payload);
}

#[tauri::command]
pub fn claude_info() -> Value {
    json!({ "path": find_claude().map(|p| p.display().to_string()) })
}

#[tauri::command]
pub fn ask(
    app: AppHandle,
    state: State<'_, Running>,
    prompt: String,
    session: Option<String>,
) -> Result<(), String> {
    if state.child.lock().unwrap().is_some() {
        return Err("Claude уже отвечает".into());
    }
    let claude = find_claude().ok_or(
        "Не найден Claude Code (команда claude). Установите его и войдите в аккаунт: https://claude.com/claude-code",
    )?;
    let cfg = config::load(&app);
    let work = config::work_dir(&app);
    let shots = config::shots_dir(&app);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    fs::create_dir_all(&shots).map_err(|e| e.to_string())?;

    let mut cmd = Command::new(&claude);
    cmd.current_dir(&work)
        .env("PATH", child_path_env())
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--allowedTools",
            "Read,WebSearch,WebFetch",
        ])
        .arg("--add-dir")
        .arg(&shots);
    let model = config::get_str(&cfg, "model");
    if !model.is_empty() {
        cmd.args(["--model", &model]);
    }
    let sp = config::get_str(&cfg, "system_prompt");
    if !sp.is_empty() {
        cmd.args(["--append-system-prompt", &sp]);
    }
    if let Some(s) = session.filter(|s| !s.is_empty()) {
        cmd.args(["--resume", &s]);
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
    *state.child.lock().unwrap() = Some(child);
    state.stopped.store(false, Ordering::SeqCst);
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;

    thread::spawn(move || {
        let _ = stdin.write_all(prompt.as_bytes());
        drop(stdin);
    });

    // страховка от зависания
    {
        let app = app.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(TIMEOUT_SECS));
            let st = app.state::<Running>();
            if st.generation.load(Ordering::SeqCst) == generation {
                if let Some(c) = st.child.lock().unwrap().as_mut() {
                    let _ = c.kill();
                }
            }
        });
    }

    let app2 = app.clone();
    thread::spawn(move || {
        let err_buf = Arc::new(Mutex::new(String::new()));
        {
            let err_buf = err_buf.clone();
            thread::spawn(move || {
                let mut s = String::new();
                let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
                *err_buf.lock().unwrap() = s;
            });
        }
        let mut result: Option<String> = None;
        let mut is_error = false;
        let mut session_id: Option<String> = None;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Ok(d) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(s) = d.get("session_id").and_then(|v| v.as_str()) {
                session_id = Some(s.to_string());
            }
            match d.get("type").and_then(|v| v.as_str()) {
                Some("stream_event") => {
                    let ev = &d["event"];
                    match ev.get("type").and_then(|v| v.as_str()) {
                        Some("content_block_delta")
                            if ev["delta"]["type"] == "text_delta" =>
                        {
                            let t = ev["delta"]["text"].as_str().unwrap_or("");
                            emit(&app2, json!({"kind": "delta", "text": t}));
                        }
                        Some("content_block_start") if ev["content_block"]["type"] == "tool_use" => {
                            let n = ev["content_block"]["name"].as_str().unwrap_or("");
                            emit(&app2, json!({"kind": "tool", "name": n}));
                        }
                        _ => {}
                    }
                }
                Some("result") => {
                    result = d.get("result").and_then(|v| v.as_str()).map(String::from);
                    is_error = d.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                }
                _ => {}
            }
        }
        let st = app2.state::<Running>();
        if let Some(mut c) = st.child.lock().unwrap().take() {
            let _ = c.wait();
        }
        thread::sleep(Duration::from_millis(50));
        let stderr_text = err_buf.lock().unwrap().clone();
        emit(
            &app2,
            json!({
                "kind": "done",
                "result": result,
                "is_error": is_error,
                "session_id": session_id,
                "stopped": st.stopped.load(Ordering::SeqCst),
                "stderr": stderr_text.chars().rev().take(1500).collect::<Vec<_>>().into_iter().rev().collect::<String>()
            }),
        );
    });
    Ok(())
}

#[tauri::command]
pub fn stop(state: State<'_, Running>) {
    state.stopped.store(true, Ordering::SeqCst);
    if let Some(c) = state.child.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}
