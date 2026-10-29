//! The card clock: `{time}` reads `19:54` or `7:54 PM`.
//!
//! `behaviour { clock "auto" }` (the default) follows the desktop:
//!
//! - Omarchy: the bar clock's format in `~/.config/omarchy/shell.json` (`h:mm AP` is 12-hour).
//! - GNOME: `clock-format` from the settings portal (`org.gnome.desktop.interface`). Only on
//!   GNOME, where it's the clock the user sees; elsewhere the portal answers GNOME's default.
//! - Everywhere else: the locale's time format (`LC_ALL`, `LC_TIME`, `LANG`), 12-hour when it
//!   prints AM/PM (`en_US`, `en_CA`, `en_AU`, ...).
//!
//! 24-hour when none of them says. Followed live: the portal's `SettingChanged` signal, and
//! Omarchy's file looked at every second.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, SystemTime};

use rune::{AppHandle, OverlayHandle};

/// `behaviour { clock "..." }`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Clock {
    #[default]
    Auto,
    H12,
    H24,
}

impl Clock {
    pub const ALL: [Clock; 3] = [Clock::Auto, Clock::H12, Clock::H24];

    pub fn word(self) -> &'static str {
        match self {
            Clock::Auto => "auto",
            Clock::H12 => "12h",
            Clock::H24 => "24h",
        }
    }

    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(c: u8) -> Clock {
        match c {
            1 => Clock::H12,
            2 => Clock::H24,
            _ => Clock::Auto,
        }
    }
}

/// When a notification arrived, local time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub hour: u8,
    pub minute: u8,
}

impl Stamp {
    pub fn now() -> Stamp {
        // SAFETY: localtime_r only writes the struct we pass.
        unsafe {
            let t = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&t, &mut tm);
            Stamp {
                hour: tm.tm_hour as u8,
                minute: tm.tm_min as u8,
            }
        }
    }

    pub fn h24(self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    pub fn h12(self) -> String {
        let h = match self.hour % 12 {
            0 => 12,
            h => h,
        };
        let half = if self.hour < 12 { "AM" } else { "PM" };
        format!("{h}:{:02} {half}", self.minute)
    }

    /// In the format `behaviour { clock }` and the desktop decide.
    pub fn show(self) -> String {
        if twelve() { self.h12() } else { self.h24() }
    }
}

/// The config's choice, and what the desktop says (`Auto` while unknown).
static CONFIG: AtomicU8 = AtomicU8::new(0);
static DESKTOP: AtomicU8 = AtomicU8::new(0);

/// Sets the config's choice (from `behaviour { clock }`).
pub fn prefer(c: Clock) {
    CONFIG.store(c.code(), Ordering::Relaxed);
}

/// Stores what the desktop says; true when it changed.
fn set_desktop(c: Option<Clock>) -> bool {
    let c = c.unwrap_or(Clock::Auto).code();
    DESKTOP.swap(c, Ordering::Relaxed) != c
}

fn locale_twelve() -> bool {
    static LOCALE: OnceLock<bool> = OnceLock::new();
    *LOCALE.get_or_init(locale)
}

/// Whether cards show a 12-hour clock now.
pub fn twelve() -> bool {
    resolve(
        Clock::from_code(CONFIG.load(Ordering::Relaxed)),
        Clock::from_code(DESKTOP.load(Ordering::Relaxed)),
        locale_twelve,
    )
}

fn resolve(config: Clock, desktop: Clock, locale: impl FnOnce() -> bool) -> bool {
    match (config, desktop) {
        (Clock::H12, _) | (Clock::Auto, Clock::H12) => true,
        (Clock::H24, _) | (Clock::Auto, Clock::H24) => false,
        (Clock::Auto, Clock::Auto) => locale(),
    }
}

/// A strftime time format (`T_FMT`) that prints AM/PM.
fn strftime_twelve(fmt: &str) -> bool {
    ["%p", "%P", "%r", "%I", "%l"]
        .iter()
        .any(|t| fmt.contains(t))
}

/// Locales that write a 12-hour clock, for when the locale itself isn't installed.
const TWELVE_HOUR: &[&str] = &[
    "en_US", "en_CA", "en_AU", "en_NZ", "en_PH", "en_IN", "es_US", "es_MX",
];

/// A locale name (`en_US.UTF-8@euro`) from `TWELVE_HOUR`.
fn named_twelve(name: &str) -> bool {
    let base = name.split(['.', '@']).next().unwrap_or("");
    TWELVE_HOUR.contains(&base)
}

/// The locale the time follows: `LC_ALL`, then `LC_TIME`, then `LANG`.
fn locale_name() -> Option<String> {
    ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// The locale's own time format when it's installed, else its name.
fn locale() -> bool {
    let name = locale_name().unwrap_or_default();
    // SAFETY: newlocale reads the environment's locale into a fresh object we free; nothing
    // global changes, so this is safe beside other threads.
    let fmt = unsafe {
        let loc = libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut());
        if loc.is_null() {
            None
        } else {
            let p = libc::nl_langinfo_l(libc::T_FMT, loc);
            let s =
                (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned());
            libc::freelocale(loc);
            s
        }
    };
    match fmt {
        // A C locale here with a named locale set means it isn't installed.
        Some(f) if !(f == "%H:%M:%S" && named_twelve(&name)) => strftime_twelve(&f),
        _ => named_twelve(&name),
    }
}

/// A Qt date format (Omarchy's bar clock): AM/PM makes it 12-hour, an hour without it 24-hour,
/// neither says nothing. Quoted text isn't format.
fn qt_format(fmt: &str) -> Option<Clock> {
    let mut plain = String::new();
    let mut quoted = false;
    for c in fmt.chars() {
        if c == '\'' {
            quoted = !quoted;
        } else if !quoted {
            plain.push(c);
        }
    }
    if plain.contains(['a', 'A']) {
        Some(Clock::H12)
    } else if plain.contains(['h', 'H']) {
        Some(Clock::H24)
    } else {
        None
    }
}

const OMARCHY_DEFAULT: &str = "dddd HH:mm";

/// The bar clock's format in Omarchy's `shell.json`.
fn omarchy(json: &str) -> Option<Clock> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let layout = v.get("bar")?.get("layout")?.as_object()?;
    let clock = layout
        .values()
        .filter_map(|s| s.as_array())
        .flatten()
        .find(|w| w.get("id").and_then(|i| i.as_str()) == Some("omarchy.clock"))?;
    qt_format(
        clock
            .get("format")
            .and_then(|f| f.as_str())
            .unwrap_or(OMARCHY_DEFAULT),
    )
}

fn omarchy_file() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("omarchy/shell.json"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Desktop {
    Omarchy,
    Gnome,
    Other,
}

fn desktop() -> Desktop {
    let current = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_uppercase();
    if omarchy_file().is_some_and(|f| f.is_file()) {
        Desktop::Omarchy
    } else if current.split(':').any(|d| d == "GNOME") {
        Desktop::Gnome
    } else {
        Desktop::Other
    }
}

fn text(v: &zbus::zvariant::Value<'_>) -> Option<String> {
    match v {
        zbus::zvariant::Value::Str(s) => Some(s.to_string()),
        zbus::zvariant::Value::Value(inner) => text(inner),
        _ => None,
    }
}

fn gnome(v: &str) -> Option<Clock> {
    match v {
        "12h" => Some(Clock::H12),
        "24h" => Some(Clock::H24),
        _ => None,
    }
}

const GNOME: (&str, &str) = ("org.gnome.desktop.interface", "clock-format");

fn read(desktop: Desktop, portal: Option<&zbus::blocking::Proxy<'_>>) -> Option<Clock> {
    match desktop {
        Desktop::Omarchy => omarchy(&std::fs::read_to_string(omarchy_file()?).ok()?),
        Desktop::Gnome => {
            let v = portal?
                .call::<_, _, zbus::zvariant::OwnedValue>("ReadOne", &GNOME)
                .ok()?;
            gnome(&text(&v)?)
        }
        Desktop::Other => None,
    }
}

/// Reads the desktop's clock now and follows it, redrawing the cards and the settings preview
/// when it changes. Waits at most a second for the portal so the first card already has it.
pub fn follow(overlay: OverlayHandle<crate::cards::Ui>, window: AppHandle<crate::settings::Ui>) {
    let (done, wait) = std::sync::mpsc::channel();
    let redraw = move || {
        overlay.update(|_, _| {});
        window.update(|_, _| {});
    };
    std::thread::spawn(move || {
        let desktop = desktop();
        locale_twelve();
        let portal = (desktop == Desktop::Gnome)
            .then(|| zbus::blocking::Connection::session().ok())
            .flatten()
            .and_then(|c| {
                zbus::blocking::Proxy::new_owned(
                    c,
                    "org.freedesktop.portal.Desktop",
                    "/org/freedesktop/portal/desktop",
                    "org.freedesktop.portal.Settings",
                )
                .ok()
            });
        set_desktop(read(desktop, portal.as_ref()));
        let _ = done.send(());
        match desktop {
            Desktop::Gnome => {
                let Some(p) = portal else { return };
                let Ok(signals) = p.receive_signal("SettingChanged") else {
                    return;
                };
                for m in signals {
                    let Ok((ns, key, _)) = m
                        .body()
                        .deserialize::<(String, String, zbus::zvariant::OwnedValue)>()
                    else {
                        continue;
                    };
                    if (ns.as_str(), key.as_str()) == GNOME && set_desktop(read(desktop, Some(&p)))
                    {
                        redraw();
                    }
                }
            }
            Desktop::Omarchy => {
                let stamp = || {
                    omarchy_file()
                        .and_then(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok())
                };
                let mut last: Option<SystemTime> = stamp();
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let now = stamp();
                    if now != last {
                        last = now;
                        if set_desktop(read(desktop, None)) {
                            redraw();
                        }
                    }
                }
            }
            Desktop::Other => {}
        }
    });
    let _ = wait.recv_timeout(Duration::from_secs(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let at = |hour, minute| Stamp { hour, minute };
        assert_eq!(at(19, 54).h24(), "19:54");
        assert_eq!(at(7, 5).h24(), "07:05");
        assert_eq!(at(19, 54).h12(), "7:54 PM");
        assert_eq!(at(0, 3).h12(), "12:03 AM");
        assert_eq!(at(12, 0).h12(), "12:00 PM");
        assert_eq!(at(11, 59).h12(), "11:59 AM");
    }

    #[test]
    fn the_config_wins_then_the_desktop_then_the_locale() {
        let no = || -> bool { panic!("the locale isn't asked") };
        assert!(resolve(Clock::H12, Clock::H24, no));
        assert!(!resolve(Clock::H24, Clock::H12, no));
        assert!(resolve(Clock::Auto, Clock::H12, no));
        assert!(!resolve(Clock::Auto, Clock::H24, no));
        assert!(resolve(Clock::Auto, Clock::Auto, || true));
        assert!(!resolve(Clock::Auto, Clock::Auto, || false));
    }

    #[test]
    fn locales() {
        assert!(strftime_twelve("%r")); // en_US
        assert!(strftime_twelve("%I:%M:%S %p"));
        assert!(!strftime_twelve("%T")); // de_DE
        assert!(!strftime_twelve("%H:%M:%S"));
        assert!(named_twelve("en_US.UTF-8"));
        assert!(named_twelve("en_AU"));
        assert!(!named_twelve("en_GB.UTF-8"));
        assert!(!named_twelve("C.UTF-8"));
        assert!(!named_twelve(""));
    }

    #[test]
    fn omarchy_reads_the_bar_clock() {
        assert_eq!(qt_format("dddd HH:mm"), Some(Clock::H24));
        assert_eq!(qt_format("h:mm AP"), Some(Clock::H12));
        assert_eq!(qt_format("h\n—\nmm\nap"), Some(Clock::H12));
        assert_eq!(qt_format("d MMMM 'W'ww yyyy"), None);
        assert_eq!(
            qt_format("'at' HH:mm"),
            Some(Clock::H24),
            "quoted a isn't AM/PM"
        );
        let json = |format: &str| {
            format!(
                r#"{{"bar":{{"layout":{{"left":[{{"id":"omarchy.menu"}}],"center":[{{"id":"omarchy.clock"{format}}}]}}}}}}"#
            )
        };
        assert_eq!(
            omarchy(&json(r#","format":"dddd h:mm AP""#)),
            Some(Clock::H12)
        );
        assert_eq!(omarchy(&json("")), Some(Clock::H24), "Omarchy's default");
        assert_eq!(omarchy(r#"{"bar":{"layout":{"left":[]}}}"#), None);
        assert_eq!(omarchy("{"), None);
    }
}
