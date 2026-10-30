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

/// The ID a Notify call gets: `replaces_id` when it names one, else the next unused. An app may
/// name an ID from before Hokum (re)started (xdg-desktop-portal-gtk keeps them for each app's
/// notifications); counting past it keeps a later notification from taking the same ID and
/// replacing that card.
fn assign(next: &AtomicU32, replaces_id: u32) -> u32 {
    if replaces_id == 0 {
        return next.fetch_add(1, Ordering::Relaxed);
    }
    next.fetch_max(replaces_id.saturating_add(1), Ordering::Relaxed);
    replaces_id
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

/// Bodies may carry the spec's small markup subset; cards show plain text. Only that subset's
/// tags go: any other `<` is text, as in `3 < 5` or `Ben <ben@example.com>`.
fn plain(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(ch) = rest.chars().next() {
        let skip = match ch {
            '<' => tag(rest).map(|(len, newline)| {
                if newline {
                    out.push('\n');
                }
                len
            }),
            '&' => entity(rest).map(|(len, decoded)| {
                out.push(decoded);
                len
            }),
            _ => None,
        };
        let len = skip.unwrap_or_else(|| {
            out.push(ch);
            ch.len_utf8()
        });
        rest = &rest[len..];
    }
    let out = out.trim();
    // Very long bodies read as a preview.
    match out.char_indices().nth(280) {
        Some((i, _)) => format!("{}…", out[..i].trim_end()),
        None => out.to_owned(),
    }
}

/// The length of the spec's tag at the start of `s` (`<b>`, `</i>`, `<a href="…">`, `<br/>`,
/// `<img … />`), and whether it breaks the line; `None` when `s` starts with a plain `<`.
fn tag(s: &str) -> Option<(usize, bool)> {
    let end = s.find('>')?;
    let inner = &s[1..end];
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(inner) => (true, inner),
        None => (false, inner),
    };
    let name_len = inner
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(inner.len());
    let (name, rest) = inner.split_at(name_len);
    let ok = match name.to_ascii_lowercase().as_str() {
        "b" | "i" | "u" => rest.is_empty(),
        "br" => !closing && matches!(rest.trim(), "" | "/"),
        // Only these two take attributes.
        "a" if closing => rest.is_empty(),
        "a" | "img" => rest.is_empty() || rest.starts_with(char::is_whitespace) || rest == "/",
        _ => false,
    };
    ok.then_some((end + 1, name.eq_ignore_ascii_case("br")))
}

/// The entity at the start of `s` (`&amp;`, `&#8217;`, `&#x2014;`, ...) as its length and
/// character. Decoded once, so `&amp;lt;` reads `&lt;`.
fn entity(s: &str) -> Option<(usize, char)> {
    let end = s.bytes().take(12).position(|b| b == b';')?;
    let ch = match &s[1..end] {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        n => {
            let n = n.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((end + 1, ch))
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

/// Chromium-family browsers start a web notification's body with its site, as a link
/// (`<a href="https://web.whatsapp.com/">web.whatsapp.com</a>`) when the server takes markup or
/// as text, then a blank line, then the message. The site, and the body after it.
fn web_site(body: &str) -> Option<(String, &str)> {
    let (first, rest) = body.split_once('\n').unwrap_or((body, ""));
    let first = first.trim();
    let site = match first.strip_prefix("<a href=\"") {
        Some(link) => {
            let (url, text) = link.split_once("\">")?;
            text.strip_suffix("</a>")?;
            crate::entries::site(url)?
        }
        // As text, only with a message after it: a body that is just a domain is a message.
        None if first.contains(['<', ' ', '/']) || rest.is_empty() => return None,
        None => crate::entries::site(&format!("https://{first}"))?,
    };
    let rest = if rest.is_empty() {
        rest
    } else {
        rest.strip_prefix('\n')
            .or_else(|| rest.strip_prefix("\r\n"))?
    };
    Some((site, rest))
}

/// Who sent a notification, as a card shows it.
#[derive(Debug, Default, PartialEq)]
struct Who {
    app: String,
    /// The desktop entry found for it.
    entry: Option<String>,
    icon: Option<std::path::PathBuf>,
    /// For a browser's notification from a site that isn't an installed web app: the site.
    context: Option<String>,
    /// The body without a browser's site line.
    body: String,
    /// The icon is only one named like the app: an icon the sender names elsewhere beats it.
    guessed: bool,
}

/// Finds the app behind a notification and its icon, most specific first: a browser's site
/// that is an installed web app; the icon the notification names; its `desktop-entry`; its
/// sandbox (`crate::sender`); its name (`crate::entries::Index::by_name`); the unit, launcher
/// or AppImage its process runs in; an icon called like it. `clues` are the sender's process's.
fn identify(
    index: &crate::entries::Index,
    app: &str,
    app_icon: &str,
    desktop: Option<&str>,
    body: &str,
    clues: &crate::sender::Clues,
) -> Who {
    let named = desktop
        .and_then(|d| index.by_id(d))
        .or_else(|| index.by_name(app));
    let mut who = Who {
        app: app.to_owned(),
        body: body.to_owned(),
        ..Who::default()
    };
    if named.is_some_and(|e| e.browser)
        && let Some((site, rest)) = web_site(body)
    {
        who.body = rest.to_owned();
        if let Some(web) = index.by_site(&site) {
            who.app = web.name.clone();
            who.entry = Some(web.id.clone());
            who.icon = web.icon();
            return who;
        }
        who.context = Some(site);
    }
    if let Some(icon) = crate::icons::icon(app_icon) {
        who.icon = Some(icon);
        who.entry = named.map(|e| e.id.clone());
        return who;
    }
    // The first entry found names the app; the first with an icon that resolves draws it.
    let candidates = [
        desktop.and_then(|d| index.by_id(d)),
        crate::sender::sandboxed(index, clues),
        index.by_name(app),
        crate::sender::launched(index, clues),
    ];
    for e in candidates.into_iter().flatten() {
        if who.entry.is_none() {
            who.entry = Some(e.id.clone());
            // An app that sends its ID for a name (Flatpaks through the portal) reads as its name.
            if (crate::entries::same(app, &e.id) || e.flatpak.as_deref() == Some(app))
                && !e.name.trim().is_empty()
            {
                who.app = crate::entries::bare(&e.name).to_owned();
            }
        }
        if let Some(icon) = e.icon() {
            who.icon = Some(icon);
            return who;
        }
    }
    let slug = app.trim().to_lowercase().replace(' ', "-");
    who.icon = crate::icons::icon(app.trim()).or_else(|| crate::icons::icon(&slug));
    who.guessed = who.icon.is_some();
    who
}

/// A Notify call, as `build` turns it into a card.
struct Request {
    id: u32,
    /// The sender's process as the bus reports it.
    bus_pid: Option<u32>,
    app_name: String,
    app_icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    hints: HashMap<String, OwnedValue>,
    expire_timeout: i32,
}

/// Whether a sender's Omarchy `omarchy-exec-argv` and `omarchy-actions` hints count. Those make
/// Hokum run a command outside whatever sandbox the sender is in, so only on Omarchy (whose own
/// `omarchy-notification-send` sets them), only for a sender the bus names a process for (not
/// the `sender-pid` hint, which is the sender's word) whose cgroup can be read, and never for a
/// Flatpak or Snap. Any unsandboxed process of the user's could run the command itself.
fn may_run_commands(
    bus_pid: Option<u32>,
    clues: &crate::sender::Clues,
    proc_root: &std::path::Path,
    omarchy: impl FnOnce() -> bool,
) -> bool {
    let Some(pid) = bus_pid else {
        return false;
    };
    clues.flatpak.is_none()
        && clues.snap.is_none()
        && std::fs::read_to_string(proc_root.join(pid.to_string()).join("cgroup"))
            .is_ok_and(|c| !c.trim().is_empty())
        && omarchy()
}

/// The card a Notify call asks for.
fn build(r: Request) -> Note {
    let Request {
        id,
        bus_pid,
        app_name,
        app_icon,
        summary,
        body,
        actions,
        hints,
        expire_timeout,
    } = r;
    let app = Some(crate::apps::clean(&app_name))
        .filter(|a| !a.is_empty())
        .or_else(|| hint::<String>(&hints, "desktop-entry").map(|d| crate::apps::clean(&d)))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "Notification".into());
    // The bus knows the sender's process; the hint is the sender's word for it.
    let pid = bus_pid
        .or_else(|| hint::<i64>(&hints, "sender-pid").and_then(|p| u32::try_from(p).ok()))
        .or_else(|| hint::<u32>(&hints, "sender-pid"));
    let desktop = hint::<String>(&hints, "desktop-entry").filter(|d| !d.is_empty());
    let proc_root = std::path::Path::new("/proc");
    let clues = pid
        .map(|p| crate::sender::clues(proc_root, p))
        .unwrap_or_default();
    let who = identify(
        &crate::entries::index(),
        &app,
        &app_icon,
        desktop.as_deref(),
        &body,
        &clues,
    );
    let who = Who {
        app: crate::apps::clean(&who.app),
        ..who
    };
    // The spec has no markup in the summary: `Ben <ben@example.com>` is the whole of it.
    let (summary, body) = (summary.trim().to_owned(), plain(&who.body));
    let slacky = app.eq_ignore_ascii_case("slack") || desktop.as_deref() == Some("slack");
    let mut n = match slacky.then(|| slack(&summary, &body)).flatten() {
        Some((context, sender, message)) => {
            let mut n = Note::new(id, who.app, sender, message);
            n.context = Some(context);
            n
        }
        None => Note::new(id, who.app, summary, body),
    };
    n.context = n.context.take().or(who.context);
    n.urgency = hint::<u8>(&hints, "urgency").unwrap_or(1);
    n.value = hint::<i32>(&hints, "value").map(|v| (v as f32 / 100.0).clamp(0.0, 1.0));
    n.tag = hint::<String>(&hints, "x-dunst-stack-tag")
        .or_else(|| hint::<String>(&hints, "x-canonical-private-synchronous"))
        .filter(|t| !t.is_empty());
    n.resident = hint::<bool>(&hints, "resident").unwrap_or(false);
    n.sound = crate::chime::Hint::new(
        hint::<bool>(&hints, "suppress-sound").unwrap_or(false),
        hint::<String>(&hints, "sound-file").as_deref(),
        hint::<String>(&hints, "sound-name").as_deref(),
    );
    n.icon_src = who.icon.map(Source::File);
    let guessed = who.guessed;
    n.entry = who.entry;
    // The token names the app that raises a window: the sender's own entry, else the one found.
    n.desktop = desktop.or_else(|| n.entry.clone());
    let image_path =
        hint::<String>(&hints, "image-path").or_else(|| hint::<String>(&hints, "image_path"));
    // A theme icon's name there (`notify-send -i drive-harddisk` with newer libnotify) is an
    // icon, not a picture: it stands in for an app that has none, or only a guessed one.
    if (n.icon_src.is_none() || guessed)
        && let Some(icon) = image_path
            .as_deref()
            .filter(|p| !p.starts_with('/') && !p.contains("://"))
            .and_then(crate::icons::icon)
    {
        n.icon_src = Some(Source::File(icon));
    }
    n.image_src = ["image-data", "image_data", "icon_data"]
        .iter()
        .find_map(|k| hints.get(*k).and_then(image_data))
        .or_else(|| crate::icons::icon(image_path.as_deref()?).map(Source::File))
        // The app's own icon isn't a picture of the notification.
        .filter(|img| match &n.icon_src {
            Some(Source::File(icon)) => !crate::icons::repeats(img, icon),
            _ => true,
        });
    (n.has_default, n.actions) = split_actions(&actions);
    let exec = hint::<String>(&hints, "omarchy-exec-argv");
    let buttons = hint::<String>(&hints, "omarchy-actions");
    let commands = (exec.is_some() || buttons.is_some())
        && may_run_commands(bus_pid, &clues, proc_root, crate::omarchy_theme::present);
    if (exec.is_some() || buttons.is_some()) && !commands {
        eprintln!(
            "hokum: ignoring the Omarchy command hints of {app}'s notification (not on Omarchy, or a sandboxed sender)"
        );
    }
    if let Some(argv) = exec.filter(|_| commands).as_deref().and_then(exec_argv) {
        n.commands.push(("default".into(), argv));
    }
    for (key, label, argv) in buttons
        .filter(|_| commands)
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
    // Measured here, off the UI thread, so the badge finds it cached.
    if let Some(g) = &n.glyph {
        crate::glyph::ink_shift(g);
    }
    n.timeout = match expire_timeout {
        0 => Timeout::Never,
        t if t > 0 => Timeout::After(Duration::from_millis(t as u64)),
        _ => Timeout::Default,
    };
    n
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
    async fn notify(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = assign(&self.next, replaces_id);
        let bus_pid = match header.sender() {
            Some(sender) => match zbus::fdo::DBusProxy::new(conn).await {
                Ok(bus) => bus
                    .get_connection_unix_process_id(sender.clone().into())
                    .await
                    .ok(),
                Err(_) => None,
            },
            None => None,
        };
        let request = Request {
            id,
            bus_pid,
            app_name,
            app_icon,
            summary,
            body,
            actions,
            hints,
            expire_timeout,
        };
        // Built on a thread of its own, off zbus's executor (it reads files and decodes
        // pictures), and caught: a hint that trips a bug loses its card, but the sender still
        // gets its answer instead of waiting out the D-Bus timeout.
        let (tx, rx) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("hokum-notify".into())
            .spawn(move || {
                let note =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(request)));
                if note.is_err() {
                    eprintln!("hokum: notification {id} tripped a bug and isn't shown");
                }
                let _ = tx.send_blocking(note.ok());
            });
        if spawned.is_ok()
            && let Ok(Some(n)) = rx.recv().await
        {
            self.ui.update(move |s, _| s.upsert(n));
        }
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

/// The icon theme from the settings portal (GNOME's key, or KDE's), followed as it changes.
/// Read before claiming the name, so the first card already uses it.
fn follow_icon_theme(conn: &Connection) {
    const KEYS: [(&str, &str); 2] = [
        ("org.gnome.desktop.interface", "icon-theme"),
        ("org.kde.kdeglobals.Icons", "Theme"),
    ];
    let Ok(proxy) = zbus::blocking::Proxy::new(
        conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    ) else {
        return;
    };
    fn text(v: &zbus::zvariant::Value<'_>) -> Option<String> {
        match v {
            zbus::zvariant::Value::Str(s) => Some(s.to_string()),
            zbus::zvariant::Value::Value(inner) => text(inner),
            _ => None,
        }
    }
    let read = |proxy: &zbus::blocking::Proxy<'_>| {
        KEYS.iter().find_map(|k| {
            proxy
                .call::<_, _, OwnedValue>("ReadOne", k)
                .ok()
                .and_then(|v| text(&v))
                .filter(|t| !t.is_empty())
        })
    };
    // A portal that is still starting (or stuck) gets a second, then answers whenever it can.
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        crate::icons::set_portal_theme(read(&proxy));
        let _ = done.send(());
        let Ok(signals) = proxy.receive_signal("SettingChanged") else {
            return;
        };
        for m in signals {
            if let Ok((ns, key, _)) = m.body().deserialize::<(String, String, OwnedValue)>()
                && KEYS.iter().any(|(n, k)| *n == ns && *k == key)
            {
                crate::icons::set_portal_theme(read(&proxy));
            }
        }
    });
    let _ = wait.recv_timeout(Duration::from_secs(1));
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
    // Read the installed apps now rather than on the first notification.
    std::thread::spawn(crate::entries::index);
    follow_icon_theme(&conn);
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
    use super::{exec_argv, glyph, identify, omarchy_actions, split_actions, web_site};

    #[test]
    fn browsers_put_the_site_first() {
        let linked = "<a href=\"https://calendar.example.org/\">calendar.example.org</a>\n\nMeeting moved to 3pm.";
        assert_eq!(
            web_site(linked),
            Some(("calendar.example.org".into(), "Meeting moved to 3pm."))
        );
        assert_eq!(
            web_site("www.web.whatsapp.com\n\nsee you at 8"),
            Some(("web.whatsapp.com".into(), "see you at 8"))
        );
        assert_eq!(
            web_site("example.com"),
            None,
            "a body that is only a domain is a message"
        );
        assert_eq!(web_site("Meeting at 5.30\n\nbring notes"), None);
        assert_eq!(web_site("ok.\nsecond line"), None);
        assert_eq!(web_site("<b>bold</b>\n\nx"), None);
    }

    #[test]
    fn markup_goes_but_literal_brackets_stay() {
        for (body, shown) in [
            ("3 < 5 errors remain", "3 < 5 errors remain"),
            (
                "Ben <ben@example.com> sent you a file",
                "Ben <ben@example.com> sent you a file",
            ),
            ("if (a<b) { return; }", "if (a<b) { return; }"),
            ("x<b and y>z", "x<b and y>z"),
            ("a -> b <- c", "a -> b <- c"),
            ("<b>bold</b>, <i>it</i>, <u>under</u>", "bold, it, under"),
            ("<a href=\"https://x.org\">click</a>", "click"),
            ("<img src=\"file:///a.png\" alt=\"pic\"/>caption", "caption"),
            (
                "line1<br>line2<br/>line3<br />line4",
                "line1\nline2\nline3\nline4",
            ),
            ("It&#8217;s &#x2014; done", "It\u{2019}s \u{2014} done"),
            (
                "&lt;b&gt; &amp; &quot;q&quot; &apos;s&apos; &#39;",
                "<b> & \"q\" 's' '",
            ),
            ("&amp;lt; stays escaped", "&lt; stays escaped"),
            ("&#38;lt; too", "&lt; too"),
            (
                "AT&T &unknown; &#xZZ; &#1114112;",
                "AT&T &unknown; &#xZZ; &#1114112;",
            ),
            ("  <b></b>padded  ", "padded"),
            ("&éééééé; &", "&éééééé; &"),
        ] {
            assert_eq!(super::plain(body), shown, "{body:?}");
        }
        let long = "é".repeat(300);
        assert_eq!(super::plain(&long), format!("{}…", "é".repeat(280)));
    }

    #[test]
    fn the_summary_is_plain_text_already() {
        let mut r = request(Vec::new());
        r.summary = "Ben <ben@example.com> & co".into();
        r.body = "<b>hi</b>".into();
        let n = super::build(r);
        assert_eq!(n.summary, "Ben <ben@example.com> & co");
        assert_eq!(n.body, "hi");
    }

    fn fixture() -> (crate::entries::Index, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("hokum-identify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "chrome.png",
            "whatsapp.png",
            "bb.png",
            "hylki.png",
            "logo.png",
        ] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let d = dir.display();
        let e = |id: &str, t: String| {
            crate::entries::parse(id, std::path::Path::new("/a"), &t).unwrap()
        };
        let index = crate::entries::Index {
            entries: vec![
                e(
                    "google-chrome",
                    format!(
                        "[Desktop Entry]\nName=Google Chrome\nIcon={d}/chrome.png\nExec=/usr/bin/google-chrome-stable %U\nCategories=Network;WebBrowser;"
                    ),
                ),
                e(
                    "WhatsApp",
                    format!(
                        "[Desktop Entry]\nName=WhatsApp\nIcon={d}/whatsapp.png\nExec=omarchy-launch-webapp https://web.whatsapp.com/"
                    ),
                ),
                e(
                    "bb_nightly",
                    format!(
                        "[Desktop Entry]\nName=bb Nightly (0.46.2)\nStartupWMClass=bb Nightly\nIcon={d}/bb.png\nExec=env A=1 /x/bb.appimage"
                    ),
                ),
                e(
                    "co.hyprlab.Hylki.Beta",
                    format!("[Desktop Entry]\nName=Hylki (beta)\nIcon={d}/hylki.png"),
                ),
                e(
                    "electron-noicon",
                    "[Desktop Entry]\nName=Broken\nIcon=/nowhere/x.png".into(),
                ),
            ],
        };
        (index, dir)
    }

    #[test]
    fn senders_are_found_most_specific_first() {
        let (index, dir) = fixture();
        let logo = format!("file://{}/logo.png", dir.display());
        // A web app: its own name and icon, not the browser's logo, and no site line.
        let w = identify(
            &index,
            "Google Chrome",
            &logo,
            Some("google-chrome"),
            "<a href=\"https://web.whatsapp.com/\">web.whatsapp.com</a>\n\nhi",
            &Default::default(),
        );
        assert_eq!(
            (
                w.app.as_str(),
                w.entry.as_deref(),
                w.body.as_str(),
                w.context
            ),
            ("WhatsApp", Some("WhatsApp"), "hi", None)
        );
        assert_eq!(w.icon, Some(dir.join("whatsapp.png")));
        // Any other site: the browser, with the site beside its name.
        let o = identify(
            &index,
            "Google Chrome",
            &logo,
            Some("google-chrome"),
            "example.com\n\nhello",
            &Default::default(),
        );
        assert_eq!(
            (o.app.as_str(), o.context.as_deref(), o.body.as_str()),
            ("Google Chrome", Some("example.com"), "hello")
        );
        assert_eq!(o.icon, Some(dir.join("logo.png")));
        // An app that isn't a browser keeps a body that starts like a site.
        let b = identify(
            &index,
            "bb Nightly",
            "",
            Some("bb-desktop"),
            "example.com\n\nhello",
            &Default::default(),
        );
        assert_eq!(
            (b.body.as_str(), b.entry.as_deref()),
            ("example.com\n\nhello", Some("bb_nightly"))
        );
        assert_eq!(
            b.icon,
            Some(dir.join("bb.png")),
            "desktop-entry names no file; StartupWMClass finds it"
        );
        let h = identify(
            &index,
            "co.hyprlab.Hylki.Beta",
            "",
            None,
            "",
            &Default::default(),
        );
        assert_eq!(h.icon, Some(dir.join("hylki.png")), "an ID in its own case");
        assert_eq!(h.app, "Hylki", "an ID for a name reads as the entry's name");
        assert_eq!(
            identify(&index, "bb Nightly", "", None, "", &Default::default()).app,
            "bb Nightly",
            "a name stays as sent"
        );
        let n = identify(&index, "Broken", "", None, "", &Default::default());
        assert_eq!(
            (n.entry.as_deref(), n.icon.is_none()),
            (Some("electron-noicon"), true)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_replaced_id_is_never_handed_out_again() {
        use super::assign;
        let next = std::sync::atomic::AtomicU32::new(1);
        assert_eq!(assign(&next, 0), 1);
        // An ID from before a restart, as the GTK portal sends it.
        assert_eq!(assign(&next, 7), 7);
        assert_eq!(assign(&next, 0), 8);
        assert_eq!(
            assign(&next, 3),
            3,
            "replacing an older one doesn't move the count back"
        );
        assert_eq!(assign(&next, 0), 9);
    }

    fn request(hints: Vec<(&str, zbus::zvariant::Value<'static>)>) -> super::Request {
        super::Request {
            id: 1,
            bus_pid: None,
            app_name: "Test\nApp\tname".into(),
            app_icon: "/proc/self/environ".into(),
            summary: "s".into(),
            body: "b".into(),
            actions: Vec::new(),
            hints: hints
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.to_owned(),
                        zbus::zvariant::OwnedValue::try_from(v).unwrap(),
                    )
                })
                .collect(),
            expire_timeout: -1,
        }
    }

    #[test]
    fn crafted_image_data_is_dropped_not_a_panic() {
        use zbus::zvariant::Value;
        for (w, h, stride) in [(2, 2, -8), (-2, 2, 6), (2, -2, 6), (2, 2, i32::MIN)] {
            let image = Value::from((w, h, stride, false, 8i32, 3i32, vec![0u8; 12]));
            let n = super::build(request(vec![("image-data", image)]));
            assert_eq!(n.image_src, None, "{w}x{h} stride {stride}");
        }
        let fine = Value::from((2i32, 2i32, 6i32, false, 8i32, 3i32, vec![0u8; 12]));
        assert!(
            super::build(request(vec![("image-data", fine)]))
                .image_src
                .is_some()
        );
    }

    #[test]
    fn app_names_and_paths_from_a_sender_are_tamed() {
        let n = super::build(request(vec![(
            "image-path",
            zbus::zvariant::Value::from("/proc/self/environ"),
        )]));
        assert_eq!(n.app, "Test App name");
        assert_ne!(
            n.icon_src,
            Some(crate::icons::Source::File("/proc/self/environ".into()))
        );
        assert_eq!(n.image_src, None);
    }

    #[test]
    fn command_hints_need_omarchy_and_an_unsandboxed_sender() {
        use crate::sender::Clues;
        let exec = zbus::zvariant::Value::from(r#"["touch","/tmp/x"]"#);
        let buttons = zbus::zvariant::Value::from(r#"[{"label":"Go","argv":["true"]}]"#);
        // No process the bus vouches for: never, even on Omarchy.
        let n = super::build(request(vec![
            ("omarchy-exec-argv", exec),
            ("omarchy-actions", buttons),
        ]));
        assert!(n.commands.is_empty());
        assert!(n.actions.is_empty());

        let root = std::env::temp_dir().join(format!("hokum-may-run-{}", std::process::id()));
        std::fs::create_dir_all(root.join("42")).unwrap();
        std::fs::write(
            root.join("42/cgroup"),
            "0::/user.slice/app.slice/app-Hyprland-kitty-1.scope\n",
        )
        .unwrap();
        let plain = Clues::default();
        let flatpak = Clues {
            flatpak: Some("org.telegram.desktop".into()),
            ..Clues::default()
        };
        let snap = Clues {
            snap: Some(("slack".into(), "slack".into())),
            ..Clues::default()
        };
        use super::may_run_commands as may;
        assert!(may(Some(42), &plain, &root, || true));
        assert!(!may(Some(42), &plain, &root, || false), "not on Omarchy");
        assert!(!may(Some(42), &flatpak, &root, || true), "a Flatpak");
        assert!(!may(Some(42), &snap, &root, || true), "a Snap");
        assert!(!may(None, &plain, &root, || true), "no PID from the bus");
        assert!(
            !may(Some(43), &plain, &root, || true),
            "a process that's gone"
        );
        let _ = std::fs::remove_dir_all(root);
    }

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
