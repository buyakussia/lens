//! xdg-desktop-portal: снимок экрана без диалога и нажатие Ctrl+V в другом окне.
//! Это стандартный для Wayland способ, который не требует расширений GNOME Shell.

use gtk::gio;
use gtk::glib::{self, variant::ObjectPath, Variant, VariantDict};
use gtk::prelude::*;
use std::cell::Cell;
use std::path::PathBuf;

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";

// коды клавиш evdev — не зависят от раскладки
const KEY_LEFTCTRL: i32 = 29;
const KEY_LEFTSHIFT: i32 = 42;
const KEY_V: i32 = 47;

thread_local! { static COUNTER: Cell<u32> = const { Cell::new(0) }; }

fn token() -> String {
    let n = COUNTER.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    format!("lens{}_{}", std::process::id(), n)
}

fn bus() -> Result<gio::DBusConnection, String> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map_err(|e| e.to_string())
}

/// Ответ портала: 0 — готово, 1 — пользователь отменил, 2 — отказано/ошибка.
pub struct Response {
    pub code: u32,
    pub results: VariantDict,
}

/// Вызывает метод портала, который отвечает сигналом Request.Response.
/// `args` — аргументы перед словарём опций.
async fn request(
    bus: &gio::DBusConnection,
    iface: &str,
    method: &str,
    args: Vec<Variant>,
    options: VariantDict,
) -> Result<Response, String> {
    let tok = token();
    let sender = bus.unique_name().map(|s| s.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let req_path = format!("{PATH}/request/{sender}/{tok}");
    options.insert_value("handle_token", &tok.to_variant());

    let (tx, rx) = async_channel::bounded::<(u32, VariantDict)>(1);
    #[allow(deprecated)]
    let sub = bus.signal_subscribe(
        Some(DEST),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(&req_path),
        None,
        gio::DBusSignalFlags::NONE,
        move |_c, _s, _p, _i, _n, params| {
            let code = params.child_value(0).get::<u32>().unwrap_or(2);
            let results = VariantDict::new(Some(&params.child_value(1)));
            let _ = tx.try_send((code, results));
        },
    );
    let mut all = args;
    all.push(options.end());
    let call = bus
        .call_future(Some(DEST), PATH, iface, method, Some(&Variant::tuple_from_iter(all)), None, gio::DBusCallFlags::NONE, -1)
        .await;
    if let Err(e) = call {
        #[allow(deprecated)]
        bus.signal_unsubscribe(sub);
        return Err(e.to_string());
    }
    let res = rx.recv().await.map_err(|e| e.to_string());
    #[allow(deprecated)]
    bus.signal_unsubscribe(sub);
    let (code, results) = res?;
    Ok(Response { code, results })
}

pub enum ShotError {
    Cancelled,
    /// Нет разрешения снимать экран без подтверждения (или портал недоступен).
    Denied(String),
}

/// Снимок всего экрана без диалога. Возвращает путь к файлу, который портал записал на диск.
/// Вызывающий должен прочитать его и сразу удалить ([`take_file`]).
///
/// В первый раз GNOME спрашивает разрешение (диалог показывается, только если приложение
/// в фокусе), потом запоминает. `interactive = true` — стандартное окно скриншота GNOME.
pub async fn screenshot(interactive: bool) -> Result<PathBuf, ShotError> {
    // для проверки без GNOME: LENS_FAKE_SCREENSHOT=/путь/к/картинке.png
    if let Some(p) = std::env::var_os("LENS_FAKE_SCREENSHOT") {
        let dst = std::env::temp_dir().join(format!("lens-fake-{}.png", std::process::id()));
        std::fs::copy(&p, &dst).map_err(|e| ShotError::Denied(e.to_string()))?;
        return Ok(dst);
    }
    let bus = bus().map_err(ShotError::Denied)?;
    let opts = VariantDict::new(None);
    opts.insert_value("interactive", &interactive.to_variant());
    let r = request(&bus, "org.freedesktop.portal.Screenshot", "Screenshot", vec!["".to_variant()], opts)
        .await
        .map_err(ShotError::Denied)?;
    match r.code {
        0 => {
            let uri: String = r.results.lookup("uri").ok().flatten().ok_or(ShotError::Denied("нет uri".into()))?;
            gio::File::for_uri(&uri).path().ok_or(ShotError::Denied("не файл".into()))
        }
        1 => Err(ShotError::Cancelled),
        c => Err(ShotError::Denied(format!("код {c}"))),
    }
}

/// Читает файл в память и удаляет его — чтобы снимки не копились в папке «Скриншоты».
pub fn take_file(path: &std::path::Path) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok();
    let _ = std::fs::remove_file(path);
    data
}

/// Нажимает Ctrl+V (или Ctrl+Shift+V для терминала) в активном окне через портал Remote Desktop.
///
/// Первый раз GNOME показывает диалог разрешения; токен восстановления сохраняется через
/// `restore_token`, и дальше вставка идёт без вопросов. Возвращает новый токен.
pub async fn paste(shift: bool, restore_token: Option<String>) -> Result<Option<String>, String> {
    let bus = bus()?;
    let iface = "org.freedesktop.portal.RemoteDesktop";
    let opts = VariantDict::new(None);
    opts.insert_value("session_handle_token", &token().to_variant());
    let r = request(&bus, iface, "CreateSession", vec![], opts).await?;
    if r.code != 0 {
        return Err(format!("не удалось создать сессию ({})", r.code));
    }
    let session: String = r.results.lookup("session_handle").ok().flatten().ok_or("нет сессии")?;
    let session_path = ObjectPath::try_from(session.clone()).map_err(|e| e.to_string())?;

    let result = async {
        let opts = VariantDict::new(None);
        opts.insert_value("types", &1u32.to_variant()); // клавиатура
        opts.insert_value("persist_mode", &2u32.to_variant()); // запомнить навсегда
        if let Some(t) = restore_token.filter(|t| !t.is_empty()) {
            opts.insert_value("restore_token", &t.to_variant());
        }
        let r = request(&bus, iface, "SelectDevices", vec![session_path.to_variant()], opts).await?;
        if r.code != 0 {
            return Err(format!("устройство не выбрано ({})", r.code));
        }
        let r = request(&bus, iface, "Start", vec![session_path.to_variant(), "".to_variant()], VariantDict::new(None)).await?;
        if r.code != 0 {
            return Err(if r.code == 1 { "отказано в доступе".into() } else { format!("не удалось начать ({})", r.code) });
        }
        let new_token: Option<String> = r.results.lookup("restore_token").ok().flatten();
        let mut keys = vec![KEY_LEFTCTRL];
        if shift {
            keys.push(KEY_LEFTSHIFT);
        }
        keys.push(KEY_V);
        let key = |code: i32, state: u32| {
            let args = Variant::tuple_from_iter([
                session_path.to_variant(),
                VariantDict::new(None).end(),
                code.to_variant(),
                state.to_variant(),
            ]);
            bus.call_future(Some(DEST), PATH, iface, "NotifyKeyboardKeycode", Some(&args), None, gio::DBusCallFlags::NONE, 2000)
        };
        for k in &keys {
            key(*k, 1).await.map_err(|e| e.to_string())?;
        }
        for k in keys.iter().rev() {
            key(*k, 0).await.map_err(|e| e.to_string())?;
        }
        // даём нажатиям дойти, прежде чем закрыть сессию
        glib::timeout_future(std::time::Duration::from_millis(250)).await;
        Ok(new_token)
    }
    .await;

    let _ = bus
        .call_future(Some(DEST), &session, "org.freedesktop.portal.Session", "Close", None, None, gio::DBusCallFlags::NONE, 2000)
        .await;
    result
}
