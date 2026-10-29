//! `org.freedesktop.Notifications` on the session bus, plus Hokum's own objects on the same
//! name: `app.hokum.Settings.Activate` (a second `hokum` shows this process's window) and
//! `app.hokum.Daemon` (`Reload` re-reads config.kdl; `Dismiss`, `Invoke`, `SetDoNotDisturb`
//! and the `DoNotDisturb` property back `hokum dismiss|invoke|dnd`). Calls arrive on zbus's
//! thread and reach the UI through Rune handles; closings, actions and Do not disturb changes
//! come back on a channel and go out as signals from a thread of their own.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use rune::{AppHandle, OverlayHandle};
use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::OwnedValue;

use crate::card::{Note, Timeout};
use crate::cards::{Out, Reason, Ui};
use crate::icons::Source;
use crate::settings::Daemon;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const SETTINGS_PATH: &str = "/app/hokum/Settings";
const SETTINGS_IFACE: &str = "app.hokum.Settings";
const DAEMON_PATH: &str = "/app/hokum/Daemon";
const DAEMON_IFACE: &str = "app.hokum.Daemon";

struct Server {
    ui: OverlayHandle<Ui>,
    next: AtomicU32,
}

fn hint<T: TryFrom<OwnedValue>>(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<T> {
    hints
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| T::try_from(v).ok())
}

/// The spec's `image-data` hint, `(iiibiiay)`.
fn image_data(v: &OwnedValue) -> Option<Source> {
    let s = zbus::zvariant::Structure::try_from(v.try_clone().ok()?).ok()?;
    let f = s.fields();
    let int = |i: usize| -> Option<i32> { i32::try_from(f.get(i)?).ok() };
    let alpha = bool::try_from(f.get(3)?).ok()?;
    let data = <Vec<u8>>::try_from(f.get(6)?.try_clone().ok()?).ok()?;
    crate::icons::image_data(int(0)?, int(1)?, int(2)?, alpha, int(4)?, int(5)?, &data)
}

/// Bodies may carry the spec's small markup subset; cards show plain text.
fn plain(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut tag = false;
    for ch in body.chars() {
        match ch {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(ch),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let out = out.trim();
    // Very long bodies read as a preview.
    match out.char_indices().nth(280) {
        Some((i, _)) => format!("{}…", out[..i].trim_end()),
        None => out.to_owned(),
    }
}

/// Slack titles every message `[workspace] in #channel` (or `[workspace] from Name`) and puts
/// `Sender: message` in the body. Cards read better as sender, message, and the channel next
/// to the app's name.
fn slack(summary: &str, body: &str) -> Option<(String, String, String)> {
    let rest = summary.strip_prefix('[')?.split_once("] ")?.1;
    if let Some(channel) = rest.strip_prefix("in ") {
        let (sender, message) = body
            .split_once(": ")
            .filter(|(s, _)| !s.is_empty() && s.len() <= 80)?;
        return Some((channel.to_owned(), sender.to_owned(), message.to_owned()));
    }
    let sender = rest.strip_prefix("from ")?;
    Some((
        "Direct message".to_owned(),
        sender.to_owned(),
        body.to_owned(),
    ))
}

/// Omarchy's `omarchy-exec-argv` hint: a string holding a JSON array of argv words
/// (`omarchy-notification-send --exec`). Checked like Omarchy's shell does: 1–32 words, no
/// NULs, and a program that isn't empty or an option.
fn exec_argv(json: &str) -> Option<Vec<String>> {
    let argv: Vec<String> = serde_json::from_str(json)
        .ok()
        .filter(|_| json.len() <= 16 * 1024)?;
    let ok = (1..=32).contains(&argv.len())
        && !argv[0].is_empty()
        && !argv[0].starts_with('-')
        && argv.iter().all(|a| !a.contains('\0'));
    ok.then_some(argv)
}

/// Omarchy's `omarchy-actions` hint: a string holding `[{"label": …, "argv": [...]}, …]`, at
/// most three, as buttons keyed `omarchy-0`, `omarchy-1`, ….
fn omarchy_actions(json: &str) -> Vec<(String, String, Vec<String>)> {
    #[derive(serde::Deserialize)]
    struct Action {
        label: String,
        argv: serde_json::Value,
    }
    let Ok(actions) = serde_json::from_str::<Vec<Action>>(json) else {
        return Vec::new();
    };
    actions
        .into_iter()
        .take(3)
        .enumerate()
        .filter(|(_, a)| !a.label.trim().is_empty())
        .filter_map(|(i, a)| {
            Some((
                format!("omarchy-{i}"),
                a.label,
                exec_argv(&a.argv.to_string())?,
            ))
        })
        .collect()
}

/// Omarchy's `omarchy-glyph` hint: a few code points (a Nerd Font icon) for the icon slot.
fn glyph(text: &str) -> Option<String> {
    let ok = !text.is_empty() && text.chars().count() <= 16 && !text.chars().any(char::is_control);
    ok.then(|| text.to_owned())
}

/// Splits Notify's flat `[key, label, …]` list into whether there is a default action and the
/// buttons to show.
fn split_actions(actions: &[String]) -> (bool, Vec<(String, String)>) {
    let mut has_default = false;
    let mut buttons = Vec::new();
    for pair in actions.chunks(2) {
        let (key, label) = (pair[0].clone(), pair.get(1).cloned().unwrap_or_default());
        match key.as_str() {
            "default" => has_default = true,
            // Chromium adds a per-site notification "Settings" button (label localized) to
            // every web notification; KDE Plasma hides it the same way.
            "settings" => {}
            _ if !label.is_empty() => buttons.push((key, label)),
            _ => {}
        }
    }
    (has_default, buttons)
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<&'static str> {
        vec![
            "actions",
            "body",
            "body-markup",
            "persistence",
            "x-canonical-private-synchronous",
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            self.next.fetch_add(1, Ordering::Relaxed)
        };
        let app = Some(app_name)
            .filter(|a| !a.is_empty())
            .or_else(|| hint::<String>(&hints, "desktop-entry"))
            .unwrap_or_else(|| "Notification".into());
        let (summary, body) = (plain(&summary), plain(&body));
        let slacky = app.eq_ignore_ascii_case("slack")
            || hint::<String>(&hints, "desktop-entry").as_deref() == Some("slack");
        let mut n = match slacky.then(|| slack(&summary, &body)).flatten() {
            Some((context, sender, message)) => {
                let mut n = Note::new(id, app, sender, message);
                n.context = Some(context);
                n
            }
            None => Note::new(id, app, summary, body),
        };
        n.urgency = hint::<u8>(&hints, "urgency").unwrap_or(1);
        n.value = hint::<i32>(&hints, "value").map(|v| (v as f32 / 100.0).clamp(0.0, 1.0));
        n.tag = hint::<String>(&hints, "x-dunst-stack-tag")
            .or_else(|| hint::<String>(&hints, "x-canonical-private-synchronous"))
            .filter(|t| !t.is_empty());
        n.resident = hint::<bool>(&hints, "resident").unwrap_or(false);
        let desktop = hint::<String>(&hints, "desktop-entry");
        n.icon_src = crate::icons::for_app(&app_icon, desktop.as_deref(), &n.app).map(Source::File);
        n.desktop = desktop;
        n.image_src = ["image-data", "image_data", "icon_data"]
            .iter()
            .find_map(|k| hints.get(*k).and_then(image_data))
            .or_else(|| {
                let path = hint::<String>(&hints, "image-path")
                    .or_else(|| hint::<String>(&hints, "image_path"))?;
                crate::icons::icon(&path).map(Source::File)
            })
            // The app's own icon isn't a picture of the notification.
            .filter(|img| Some(img) != n.icon_src.as_ref());
        (n.has_default, n.actions) = split_actions(&actions);
        if let Some(argv) = hint::<String>(&hints, "omarchy-exec-argv")
            .as_deref()
            .and_then(exec_argv)
        {
            n.commands.push(("default".into(), argv));
        }
        for (key, label, argv) in hint::<String>(&hints, "omarchy-actions")
            .as_deref()
            .map(omarchy_actions)
            .unwrap_or_default()
        {
            n.actions.push((key.clone(), label));
            n.commands.push((key, argv));
        }
        // Omarchy draws its glyph only when no picture resolves; so does the letter it replaces.
        n.glyph = hint::<String>(&hints, "omarchy-glyph")
            .as_deref()
            .and_then(glyph);
        n.timeout = match expire_timeout {
            0 => Timeout::Never,
            t if t > 0 => Timeout::After(Duration::from_millis(t as u64)),
            _ => Timeout::Default,
        };
        self.ui.update(move |s, _| s.upsert(n));
        id
    }

    fn close_notification(&self, id: u32) {
        self.ui.update(move |s, _| s.close(id, Reason::Closed));
    }

    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("Hokum", "anomalyco", env!("CARGO_PKG_VERSION"), "1.2")
    }
}

struct Control {
    ui: OverlayHandle<Ui>,
    settings: AppHandle<crate::settings::Ui>,
    dnd: Arc<AtomicBool>,
}

impl Control {
    /// Runs `f` on the overlay's state and waits for its answer.
    fn ask<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Ui) -> T + Send + 'static,
    ) -> zbus::fdo::Result<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.ui.update(move |s, _| {
            let _ = tx.send(f(s));
        });
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| zbus::fdo::Error::Failed("Hokum's overlay didn't answer".into()))
    }
}

#[zbus::interface(name = "app.hokum.Daemon")]
impl Control {
    /// Re-read config.kdl, its theme and the script, and restyle live.
    fn reload(&self) {
        let config = crate::watch::load();
        self.ui.update(move |s, _| s.configure(config));
    }

    /// Close the newest card, or every card with `all`. Returns how many closed.
    fn dismiss(&self, all: bool) -> zbus::fdo::Result<u32> {
        self.ask(move |s| s.dismiss_newest(all))
    }

    /// Do what clicking the newest card does: its default action (and Omarchy's
    /// `omarchy-exec-argv` command), or dismiss it without one. False when there's no card.
    fn invoke(&self, token: String) -> zbus::fdo::Result<bool> {
        self.ask(move |s| s.activate_newest(Some(token).filter(|t| !t.is_empty())))
    }

    /// Turn Do not disturb on or off. Saved to config.kdl exactly as the settings window's
    /// switch saves it; returns the state afterwards.
    fn set_do_not_disturb(&self, on: bool) -> zbus::fdo::Result<bool> {
        let failed = zbus::fdo::Error::Failed;
        if !crate::config::load().ok() {
            return Err(failed(
                "config.kdl has problems (see `hokum check`); fix them first".into(),
            ));
        }
        let mut e = crate::config::Edit::open().map_err(failed)?;
        if on == crate::style::Behaviour::default().do_not_disturb {
            e.remove_behaviour("do-not-disturb");
        } else {
            e.set_behaviour("do-not-disturb", crate::config::Value::Flag(on));
        }
        e.save()
            .map_err(|err| failed(format!("couldn't save config.kdl: {err}")))?;
        let config = crate::watch::load();
        let now = config.loaded.behaviour.do_not_disturb;
        // The overlay applies it a moment later; the property answers right away.
        self.dnd.store(now, Ordering::Relaxed);
        let c = config.clone();
        self.ui.update(move |s, _| s.configure(c));
        self.settings.update(move |s, _| s.configure(config));
        Ok(now)
    }

    /// Whether Do not disturb is on. Changes are announced with `PropertiesChanged`.
    #[zbus(property)]
    fn do_not_disturb(&self) -> bool {
        self.dnd.load(Ordering::Relaxed)
    }
}

struct Window {
    settings: AppHandle<crate::settings::Ui>,
}

#[zbus::interface(name = "app.hokum.Settings")]
impl Window {
    /// Show the window, focused with the launcher's activation token when there is one.
    fn activate(&self, token: String) {
        crate::modules::open(&self.settings, None, Some(token));
    }

    /// Show the window on a section (`notifications`, `sound`, `bluetooth`).
    fn open(&self, section: String, token: String) {
        crate::modules::open(
            &self.settings,
            crate::settings::Section::from_word(&section),
            Some(token),
        );
    }
}

/// Follows the desktop's colour scheme (`org.freedesktop.appearance color-scheme`: 1 dark,
/// 2 light, 0 no preference, which reads as dark like the rest of Hokum).
fn follow_color_scheme(conn: &Connection, ui: OverlayHandle<Ui>) {
    let Ok(proxy) = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    ) else {
        return;
    };
    // Portals answer `v`, and some wrap it twice.
    fn scheme(v: &zbus::zvariant::Value<'_>) -> Option<u32> {
        match v {
            zbus::zvariant::Value::U32(x) => Some(*x),
            zbus::zvariant::Value::Value(inner) => scheme(inner),
            _ => None,
        }
    }
    let apply =
        |ui: &OverlayHandle<Ui>, scheme: u32| ui.update(move |s, _| s.system_dark = scheme != 2);
    if let Ok(v) =
        proxy.call::<_, _, OwnedValue>("ReadOne", &("org.freedesktop.appearance", "color-scheme"))
        && let Some(sc) = scheme(&v)
    {
        apply(&ui, sc);
    }
    std::thread::spawn(move || {
        let Ok(signals) = proxy.receive_signal("SettingChanged") else {
            return;
        };
        for m in signals {
            if let Ok((ns, key, v)) = m.body().deserialize::<(String, String, OwnedValue)>()
                && ns == "org.freedesktop.appearance"
                && key == "color-scheme"
                && let Some(sc) = scheme(&v)
            {
                apply(&ui, sc);
            }
        }
    });
}

/// Asks for `org.freedesktop.Notifications` (from its owner too, with `replace`) and says who
/// has it afterwards.
pub fn claim(conn: &Connection, replace: bool) -> Daemon {
    let mut flags = RequestNameFlags::AllowReplacement | RequestNameFlags::DoNotQueue;
    if replace {
        flags |= RequestNameFlags::ReplaceExisting;
    }
    match conn.request_name_with_flags(NAME, flags) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Daemon::Hokum,
        _ => {
            let reply = conn.call_method(Some(NAME), PATH, Some(NAME), "GetServerInformation", &());
            let name = reply
                .and_then(|m| m.body().deserialize::<(String, String, String, String)>())
                .map(|i| i.0);
            Daemon::Other(name.unwrap_or_else(|_| "Another daemon".into()))
        }
    }
}

/// The process name (`/proc/<pid>/comm`) of whoever owns `org.freedesktop.Notifications`.
pub fn owner_comm(conn: &Connection) -> String {
    let pid = zbus::blocking::fdo::DBusProxy::new(conn)
        .ok()
        .and_then(|p| {
            p.get_connection_unix_process_id(zbus::names::BusName::try_from(NAME).ok()?)
                .ok()
        });
    pid.and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
        .map(|c| c.trim().to_owned())
        .unwrap_or_default()
}

/// Shows `body` through whatever notification server the desktop has, for a Hokum that was
/// launched from a menu and can't run.
pub fn tell(body: &str) {
    let Ok(conn) = Connection::session() else {
        return;
    };
    let hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    let args = (
        "Hokum",
        0u32,
        "hokum",
        "Hokum can't run here",
        body,
        Vec::<&str>::new(),
        hints,
        -1i32,
    );
    let _ = conn.call_method(Some(NAME), PATH, Some(NAME), "Notify", &args);
}

/// Serves Hokum on the session bus and claims the notifications name. Losing the name later
/// (to a newer Hokum) quits this one.
pub fn serve(
    overlay: OverlayHandle<Ui>,
    settings: AppHandle<crate::settings::Ui>,
    out: Receiver<Out>,
    replace: bool,
    dnd: bool,
) -> zbus::Result<(Connection, Daemon)> {
    let dnd = Arc::new(AtomicBool::new(dnd));
    let control = Control {
        ui: overlay.clone(),
        settings: settings.clone(),
        dnd: dnd.clone(),
    };
    let conn = zbus::blocking::connection::Builder::session()?
        .serve_at(
            PATH,
            Server {
                ui: overlay.clone(),
                next: AtomicU32::new(1),
            },
        )?
        .serve_at(DAEMON_PATH, control)?
        .serve_at(
            SETTINGS_PATH,
            Window {
                settings: settings.clone(),
            },
        )?
        .build()?;
    let daemon = claim(&conn, replace);
    follow_color_scheme(&conn, overlay);
    let lost = conn.clone();
    std::thread::spawn(move || {
        let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(&lost) else {
            return;
        };
        let Ok(mut signals) = proxy.receive_name_lost() else {
            return;
        };
        if signals.any(|s| s.args().is_ok_and(|a| a.name == NAME)) {
            settings.quit();
        }
    });
    let emit = conn.clone();
    std::thread::spawn(move || {
        for out in out {
            let _ = match out {
                Out::Closed(id, reason) => emit.emit_signal(
                    None::<&str>,
                    PATH,
                    NAME,
                    "NotificationClosed",
                    &(id, reason as u32),
                ),
                Out::Action(id, key, token) => {
                    // Spec 1.2: the token first, so the app can raise its window on Wayland.
                    if let Some(token) = token {
                        let _ = emit.emit_signal(
                            None::<&str>,
                            PATH,
                            NAME,
                            "ActivationToken",
                            &(id, token),
                        );
                    }
                    emit.emit_signal(None::<&str>, PATH, NAME, "ActionInvoked", &(id, key))
                }
                Out::Dnd(on) => {
                    dnd.store(on, Ordering::Relaxed);
                    let changed =
                        HashMap::from([("DoNotDisturb", zbus::zvariant::Value::from(on))]);
                    let body = (DAEMON_IFACE, changed, Vec::<&str>::new());
                    emit.emit_signal(
                        None::<&str>,
                        DAEMON_PATH,
                        "org.freedesktop.DBus.Properties",
                        "PropertiesChanged",
                        &body,
                    )
                }
            };
        }
    });
    Ok((conn, daemon))
}

/// Asks the running Hokum to show its settings window (D-Bus activation starts one if
/// nothing owns the name). False when no Hokum answered.
pub fn forward(token: &str, section: Option<crate::settings::Section>) -> bool {
    let Ok(conn) = Connection::session() else {
        return false;
    };
    if let Some(s) = section
        && conn
            .call_method(
                Some(NAME),
                SETTINGS_PATH,
                Some(SETTINGS_IFACE),
                "Open",
                &(s.word(), token),
            )
            .is_ok()
    {
        return true;
    }
    // A Hokum from before sections only has Activate.
    conn.call_method(
        Some(NAME),
        SETTINGS_PATH,
        Some(SETTINGS_IFACE),
        "Activate",
        &(token,),
    )
    .is_ok()
}

/// `hokum dismiss`, `hokum invoke` and `hokum dnd`: drive the running Hokum like makoctl
/// drives mako. Prints what a script or bar needs and returns the exit code.
pub fn control(args: &[String]) -> i32 {
    let fail = |e: &dyn std::fmt::Display| {
        eprintln!("hokum: {e}");
        1
    };
    let conn = match Connection::session() {
        Ok(c) => c,
        Err(e) => return fail(&format!("no session bus: {e}")),
    };
    let call = |method: &str, body: &dyn Body| body.call(&conn, method);
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("dismiss") => match call("Dismiss", &(flag("--all") || flag("-a"),))
            .and_then(|m| m.body().deserialize::<u32>())
        {
            Ok(0) => fail(&"no notifications"),
            Ok(_) => 0,
            Err(e) => fail(&e),
        },
        Some("invoke") => {
            let token = std::env::var("XDG_ACTIVATION_TOKEN").unwrap_or_default();
            match call("Invoke", &(token,)).and_then(|m| m.body().deserialize::<bool>()) {
                Ok(true) => 0,
                Ok(false) => fail(&"no notifications"),
                Err(e) => fail(&e),
            }
        }
        _ => {
            let json = flag("--json");
            let print = |on: bool| {
                if json {
                    println!("{{\"dnd\":{on}}}")
                } else {
                    println!("{}", if on { "on" } else { "off" })
                }
            };
            let get = || -> zbus::Result<bool> {
                let reply = conn.call_method(
                    Some(NAME),
                    DAEMON_PATH,
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &(DAEMON_IFACE, "DoNotDisturb"),
                )?;
                let v: OwnedValue = reply.body().deserialize()?;
                Ok(bool::try_from(v)?)
            };
            let want = match args
                .iter()
                .skip(1)
                .find(|a| !a.starts_with('-'))
                .map(String::as_str)
            {
                None => None,
                Some("on") => Some(Some(true)),
                Some("off") => Some(Some(false)),
                Some("toggle") => Some(None),
                Some(other) => {
                    return fail(&format!("unknown `dnd {other}`; use on, off or toggle"));
                }
            };
            let result = match want {
                None => get(),
                Some(want) => match want.map_or_else(|| get().map(|v| !v), Ok) {
                    Ok(on) => {
                        call("SetDoNotDisturb", &(on,)).and_then(|m| m.body().deserialize::<bool>())
                    }
                    Err(e) => Err(e),
                },
            };
            match result {
                Ok(on) => print(on),
                Err(e) => return fail(&e),
            }
            if !flag("--watch") {
                return 0;
            }
            let Ok(proxy) = zbus::blocking::fdo::PropertiesProxy::builder(&conn)
                .destination(NAME)
                .and_then(|b| b.path(DAEMON_PATH))
                .and_then(|b| b.build())
            else {
                return fail(&"couldn't watch Hokum");
            };
            let Ok(changes) = proxy.receive_properties_changed() else {
                return fail(&"couldn't watch Hokum");
            };
            for change in changes {
                if let Ok(args) = change.args()
                    && args.interface_name == DAEMON_IFACE
                    && let Some(v) = args.changed_properties.get("DoNotDisturb")
                    && let Ok(on) = bool::try_from(v)
                {
                    print(on);
                }
            }
            0
        }
    }
}

/// A method body of any type, so one closure can call every `app.hokum.Daemon` method.
trait Body {
    fn call(&self, conn: &Connection, method: &str) -> zbus::Result<zbus::message::Message>;
}

impl<T: serde::Serialize + zbus::zvariant::DynamicType> Body for T {
    fn call(&self, conn: &Connection, method: &str) -> zbus::Result<zbus::message::Message> {
        conn.call_method(Some(NAME), DAEMON_PATH, Some(DAEMON_IFACE), method, self)
    }
}

#[cfg(test)]
mod tests {
    use super::{exec_argv, glyph, omarchy_actions, split_actions};

    #[test]
    fn omarchy_hints() {
        assert_eq!(
            exec_argv(r#"["omarchy-launch-floating-terminal-with-presentation","omarchy-update"]"#)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(exec_argv(r#"["-rf"]"#), None);
        assert_eq!(exec_argv("[]"), None);
        assert_eq!(exec_argv("xdg-open https://x"), None);
        let actions = omarchy_actions(
            r#"[{"label":"Open","argv":["xdg-open","/tmp"]},{"label":"Bad","argv":"rm"}]"#,
        );
        assert_eq!(
            actions,
            vec![(
                "omarchy-0".into(),
                "Open".into(),
                vec!["xdg-open".into(), "/tmp".into()]
            )]
        );
        assert_eq!(glyph("\u{f0e7}").as_deref(), Some("\u{f0e7}"));
        assert_eq!(glyph("a\nb"), None);
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn actions_skip_default_settings_and_unlabeled() {
        let (has_default, buttons) = split_actions(&strings(&[
            "default",
            "Activate",
            "reply",
            "Reply",
            "settings",
            "Einstellungen",
            "mute",
            "",
        ]));
        assert!(has_default);
        assert_eq!(buttons, vec![("reply".to_string(), "Reply".to_string())]);
    }

    #[test]
    fn odd_trailing_key_is_dropped() {
        let (has_default, buttons) = split_actions(&strings(&["open", "Open", "dangling"]));
        assert!(!has_default);
        assert_eq!(buttons, vec![("open".to_string(), "Open".to_string())]);
    }
}
