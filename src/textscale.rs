//! The desktop's text scale: how much bigger (or smaller) than its default the user has made
//! the desktop's text. Cards multiply their sizes by it, so Large Text and bigger desktop fonts
//! reach them while stock desktops keep Hokum's look.
//!
//! - Plasma: its font size over its 10 pt default (`org.kde.kdeglobals.General font` from the
//!   settings portal, else `kdeglobals`).
//! - Omarchy: the shell's `[font] base-size` over its default 12 (`~/.config/omarchy/shell.toml`
//!   over the current theme's `shell.toml`), which the shell's own type scale follows.
//! - Everywhere else: GNOME's `text-scaling-factor` from the settings portal (GNOME, and any
//!   desktop whose portal answers from GSettings).
//!
//! 1.0 when none of them says. Followed live: the portal's `SettingChanged` signal, and the files
//! looked at every second.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use rune::{AppHandle, OverlayHandle};

static SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0

/// The current scale.
pub fn get() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

/// Stores `v` (clamped to a sane range); true when it changed.
fn set(v: f32) -> bool {
    let v = clamp(v);
    SCALE.swap(v.to_bits(), Ordering::Relaxed) != v.to_bits()
}

fn clamp(v: f32) -> f32 {
    if v.is_finite() {
        (v * 100.0).round().clamp(50.0, 300.0) / 100.0
    } else {
        1.0
    }
}

const PLASMA_DEFAULT_PT: f32 = 10.0;
const OMARCHY_DEFAULT_PX: f32 = 12.0;

/// Plasma's scale from a Qt font string (`Noto Sans,12,-1,5,400,0,0,0,0,0,0,0,0,0,0,1`): the
/// point size over Plasma's 10 pt default.
pub fn plasma(font: &str) -> Option<f32> {
    let pt: f32 = font.split(',').nth(1)?.trim().parse().ok()?;
    (pt > 0.0).then(|| pt / PLASMA_DEFAULT_PT)
}

/// `[font] base-size` in an Omarchy `shell.toml`.
fn base_size(toml: &str) -> Option<f32> {
    let mut section = "";
    for line in toml.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.trim();
            continue;
        }
        if section == "font"
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "base-size"
        {
            return v
                .trim()
                .trim_matches('"')
                .parse::<f32>()
                .ok()
                .filter(|v| *v > 0.0);
        }
    }
    None
}

/// Omarchy's scale: the user's `base-size` over the theme's, over its default 12.
pub fn omarchy(theme: Option<&str>, user: Option<&str>) -> Option<f32> {
    let size = user
        .and_then(base_size)
        .or_else(|| theme.and_then(base_size))?;
    Some(size / OMARCHY_DEFAULT_PX)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Omarchy's `shell.toml` files: the current theme's, then the user's (which wins).
fn omarchy_files() -> Option<[PathBuf; 2]> {
    let home = home()?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    // The shell reads these exact paths (Commons/Color.qml).
    let files = [
        home.join(".local/state/omarchy/current/theme/shell.toml"),
        config.join("omarchy/shell.toml"),
    ];
    files.iter().any(|f| f.is_file()).then_some(files)
}

fn kdeglobals_files() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    match std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        Some(d) => dirs.push(d),
        None => dirs.extend(home().map(|h| h.join(".config"))),
    }
    let system = std::env::var("XDG_CONFIG_DIRS").unwrap_or_default();
    let system = if system.is_empty() {
        "/etc/xdg".to_owned()
    } else {
        system
    };
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs.into_iter().map(|d| d.join("kdeglobals")).collect()
}

/// `[General] font=` in the first `kdeglobals` that sets it.
fn kdeglobals_font() -> Option<String> {
    kdeglobals_files().iter().find_map(|f| {
        let text = std::fs::read_to_string(f).ok()?;
        let mut general = false;
        text.lines().map(str::trim).find_map(|l| {
            if l.starts_with('[') {
                general = l == "[General]";
                return None;
            }
            l.strip_prefix("font=")
                .filter(|_| general)
                .map(str::to_owned)
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Desktop {
    Plasma,
    Omarchy,
    Other,
}

fn desktop() -> Desktop {
    let current = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_uppercase();
    if current.split(':').any(|d| d == "KDE") {
        Desktop::Plasma
    } else if omarchy_files().is_some() {
        Desktop::Omarchy
    } else {
        Desktop::Other
    }
}

/// A portal value as text or a number, through any `v` wrapping.
fn text(v: &zbus::zvariant::Value<'_>) -> Option<String> {
    match v {
        zbus::zvariant::Value::Str(s) => Some(s.to_string()),
        zbus::zvariant::Value::Value(inner) => text(inner),
        _ => None,
    }
}

fn number(v: &zbus::zvariant::Value<'_>) -> Option<f64> {
    match v {
        zbus::zvariant::Value::F64(x) => Some(*x),
        zbus::zvariant::Value::Value(inner) => number(inner),
        _ => None,
    }
}

const GNOME: (&str, &str) = ("org.gnome.desktop.interface", "text-scaling-factor");
const PLASMA: (&str, &str) = ("org.kde.kdeglobals.General", "font");

fn read(desktop: Desktop, portal: Option<&zbus::blocking::Proxy<'_>>) -> f32 {
    let ask = |key: (&str, &str)| {
        portal.and_then(|p| {
            p.call::<_, _, zbus::zvariant::OwnedValue>("ReadOne", &key)
                .ok()
        })
    };
    let scale = match desktop {
        Desktop::Plasma => ask(PLASMA)
            .and_then(|v| text(&v))
            .or_else(kdeglobals_font)
            .and_then(|f| plasma(&f)),
        Desktop::Omarchy => omarchy_files().and_then(|[theme, user]| {
            omarchy(
                std::fs::read_to_string(theme).ok().as_deref(),
                std::fs::read_to_string(user).ok().as_deref(),
            )
        }),
        Desktop::Other => ask(GNOME).and_then(|v| number(&v)).map(|v| v as f32),
    };
    scale.unwrap_or(1.0)
}

fn stamps(desktop: Desktop) -> Vec<Option<SystemTime>> {
    let files: Vec<PathBuf> = match desktop {
        Desktop::Plasma => kdeglobals_files(),
        Desktop::Omarchy => omarchy_files().map(Vec::from).unwrap_or_default(),
        Desktop::Other => Vec::new(),
    };
    files
        .iter()
        .map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok())
        .collect()
}

/// Reads the scale now and follows it, redrawing the cards and the settings preview when it
/// changes. Waits at most a second for the portal so the first card already has it.
pub fn follow(overlay: OverlayHandle<crate::cards::Ui>, window: AppHandle<crate::settings::Ui>) {
    let (done, wait) = std::sync::mpsc::channel();
    let redraw = move || {
        overlay.update(|_, _| {});
        window.update(|_, _| {});
    };
    std::thread::spawn(move || {
        let desktop = desktop();
        let conn = zbus::blocking::Connection::session().ok();
        let portal = conn.as_ref().and_then(|c| {
            zbus::blocking::Proxy::new(
                c,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Settings",
            )
            .ok()
        });
        set(read(desktop, portal.as_ref()));
        let _ = done.send(());
        // The portal's changes arrive on a thread of their own; files are looked at here.
        if let Some(p) = portal.clone() {
            let redraw = redraw.clone();
            std::thread::spawn(move || {
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
                    let ours = match desktop {
                        Desktop::Plasma => (ns.as_str(), key.as_str()) == PLASMA,
                        Desktop::Other => (ns.as_str(), key.as_str()) == GNOME,
                        Desktop::Omarchy => false,
                    };
                    if ours && set(read(desktop, Some(&p))) {
                        redraw();
                    }
                }
            });
        }
        let mut last = stamps(desktop);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = stamps(desktop);
            if now != last {
                last = now;
                if set(read(desktop, portal.as_ref())) {
                    redraw();
                }
            }
        }
    });
    let _ = wait.recv_timeout(Duration::from_secs(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plasma_scales_by_its_10pt_default() {
        assert_eq!(
            plasma("Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"),
            Some(1.0)
        );
        assert_eq!(plasma("Noto Sans,12,-1,5,50,0,0,0,0,0"), Some(1.2));
        assert_eq!(plasma("Inter,11.5,-1,5,400"), Some(1.15));
        assert_eq!(plasma("Noto Sans"), None);
        assert_eq!(
            plasma("Noto Sans,-1,16,5"),
            None,
            "a pixel size isn't a point size"
        );
    }

    #[test]
    fn omarchy_takes_the_users_base_size_over_the_themes() {
        let theme =
            "[bar]\nsize-horizontal = 26\n\n[font]\n# base-size is the rem root\nbase-size = 12\n";
        let user = "[font]\nbase-size = 14 # bigger\n";
        assert_eq!(omarchy(Some(theme), None), Some(1.0));
        assert_eq!(
            omarchy(Some(theme), Some(user)).map(|s| (s * 1000.0).round()),
            Some(1167.0)
        );
        assert_eq!(
            omarchy(None, Some("[bar]\nbase-size = 20\n")),
            None,
            "only [font]'s base-size"
        );
        assert_eq!(
            omarchy(Some(theme), Some("[font]\nfamily = \"x\"\n")),
            Some(1.0),
            "a user file without it keeps the theme's"
        );
    }

    #[test]
    fn scales_stay_sane() {
        assert_eq!(clamp(1.1818), 1.18);
        assert_eq!(clamp(0.1), 0.5);
        assert_eq!(clamp(9.0), 3.0);
        assert_eq!(clamp(f32::NAN), 1.0);
    }
}
