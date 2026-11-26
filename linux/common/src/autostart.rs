//! Автозапуск при входе в систему: ~/.config/autostart/<id>.desktop

use std::path::PathBuf;

fn path(desktop_id: &str) -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("autostart").join(format!("{desktop_id}.desktop"))
}

pub fn enabled(desktop_id: &str) -> bool {
    std::fs::read_to_string(path(desktop_id))
        .map(|t| !t.contains("X-GNOME-Autostart-enabled=false") && !t.contains("Hidden=true"))
        .unwrap_or(false)
}

pub fn set(desktop_id: &str, name: &str, exec: &str, icon: &str, on: bool) -> std::io::Result<()> {
    let p = path(desktop_id);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(
        p,
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={icon}\nTerminal=false\n\
             NoDisplay=true\nX-GNOME-Autostart-enabled={}\n",
            if on { "true" } else { "false" }
        ),
    )
}

/// Полный путь к своей программе (для команд горячих клавиш и автозапуска).
pub fn self_exe(fallback: &str) -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| fallback.to_string())
}
