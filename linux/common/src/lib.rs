//! Интеграция с GNOME / Linux, общая для Lens и Clipboard History.

pub mod autostart;
pub mod hotkeys;
pub mod portal;
pub mod x11;

/// Вернуть операционной системе освобождённую память (после закрытия окна со снимком экрана
/// glibc иначе держит её у процесса). Дёшево, вызывается только при закрытии окон.
pub fn trim_memory() {
    #[cfg(target_env = "gnu")]
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Окна должны работать через XWayland: только так GNOME позволяет держать окно поверх других,
/// показывать окно без фокуса и следить за буфером обмена. Вызывать до gtk::init.
pub fn use_x11_backend() {
    if std::env::var_os("GDK_BACKEND").is_none() {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}

/// Подключает CSS к экрану.
pub fn load_css(css: &str) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
