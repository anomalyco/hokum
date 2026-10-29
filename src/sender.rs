//! Which app a notification's sender is, from its process: the D-Bus connection's PID (asked
//! of the bus, or the `sender-pid` hint libnotify and Electron add). Used only when the
//! notification's own hints don't find an app.
//!
//! A sandbox says for certain: a Flatpak's app ID (`/.flatpak-info` in its sandbox, or its
//! systemd scope, which also holds the D-Bus proxy that connects for it), a Snap's
//! (`snap.NAME.APP-….scope`). Weaker, and tried after the app's name: the systemd unit desktops
//! launch apps into (`app[-launcher]-ID-RANDOM.scope`, `app[-launcher]-ID@RANDOM.service`, per
//! systemd.io/DESKTOP_ENVIRONMENTS), the desktop file GLib's and BAMF's launchers leave in the
//! environment, an AppImage's path. Those are inherited and sometimes guessed: Electron apps
//! that don't name themselves put themselves in `app-org.chromium.Chromium-PID.scope`, and
//! everything run in a terminal sits in the terminal's unit. So a browser or terminal found that
//! way counts only if it is the very program sending, and a generic sender (`notify-send`, a
//! shell, an interpreter) never counts.

use std::path::Path;

use crate::entries::{Entry, Index};

/// What a process says about the app it belongs to.
#[derive(Debug, Default, PartialEq)]
pub struct Clues {
    pub flatpak: Option<String>,
    /// Snap name and app.
    pub snap: Option<(String, String)>,
    /// The application ID of its systemd unit, and the launcher prefix's possible split points.
    pub unit: Vec<String>,
    /// `GIO_LAUNCHED_DESKTOP_FILE` / `BAMF_DESKTOP_FILE_HINT`: a desktop file's path.
    pub launched: Option<String>,
    /// `APPIMAGE`: the AppImage's path.
    pub appimage: Option<String>,
    /// The executable's file name.
    pub exe: Option<String>,
}

/// Programs that send notifications for whoever runs them.
const GENERIC: [&str; 22] = [
    "notify-send",
    "gdbus",
    "dbus-send",
    "busctl",
    "qdbus",
    "qdbus6",
    "kdialog",
    "zenity",
    "sh",
    "bash",
    "dash",
    "zsh",
    "fish",
    "python",
    "python3",
    "perl",
    "ruby",
    "node",
    "bun",
    "deno",
    "lua",
    "osascript",
];

fn generic(exe: &str) -> bool {
    GENERIC.contains(&exe) || exe.starts_with("python3.")
}

/// systemd's `\xNN` escapes in a unit name.
fn unescape(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && b.get(i + 1) == Some(&b'x')
            && let Some(v) = s
                .get(i + 2..i + 4)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The app IDs a cgroup path's leaf names, most likely first: `app-gnome-org.gnome.Nautilus-4211.scope`
/// → `org.gnome.Nautilus`, then `gnome-org.gnome.Nautilus` (the launcher prefix is optional,
/// and a `-` inside an ID is escaped as `\x2d`, so the words between `-` are ID or launcher).
pub fn unit_ids(cgroup: &str) -> Vec<String> {
    let leaf = cgroup.trim().rsplit('/').next().unwrap_or("");
    let Some(rest) = leaf.strip_prefix("app-") else {
        return Vec::new();
    };
    let words: Vec<&str> = if let Some(r) = rest.strip_suffix(".scope") {
        // The last word is the random part.
        let mut w: Vec<&str> = r.split('-').collect();
        if w.len() < 2 {
            return Vec::new();
        }
        w.pop();
        w
    } else if let Some(r) = rest.strip_suffix(".service") {
        r.split('@').next().unwrap_or(r).split('-').collect()
    } else {
        return Vec::new();
    };
    // The ID is the last word; a launcher may also have been joined to it unescaped.
    let mut out = Vec::new();
    for start in (0..words.len()).rev() {
        let id = unescape(&words[start..].join("-"));
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// A Snap's name and app from a cgroup leaf: `snap.firefox.firefox-<uuid>.scope` for an app
/// snapd started, `snap.NAME.APP.service` for a daemon.
pub fn snap(cgroup: &str) -> Option<(String, String)> {
    let leaf = cgroup.trim().rsplit('/').next()?;
    let rest = leaf.strip_prefix("snap.")?;
    let rest = match rest.strip_suffix(".scope") {
        Some(r) => {
            // The random part: a UUID, or (older snapd) one hex word.
            let uuid = r.len() > 37
                && r.as_bytes()[r.len() - 37] == b'-'
                && r[r.len() - 36..].chars().enumerate().all(|(i, c)| {
                    if [8, 13, 18, 23].contains(&i) {
                        c == '-'
                    } else {
                        c.is_ascii_hexdigit()
                    }
                });
            if uuid {
                &r[..r.len() - 37]
            } else {
                r.rsplit_once('-')
                    .filter(|(_, h)| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
                    .map_or(r, |(a, _)| a)
            }
        }
        None => rest.strip_suffix(".service")?,
    };
    let (name, app) = rest.split_once('.')?;
    let app = app.split('.').next().unwrap_or(app);
    (!name.is_empty() && !app.is_empty()).then(|| (name.to_owned(), app.to_owned()))
}

/// The unified hierarchy's line (`0::/user.slice/…`) of `/proc/PID/cgroup`.
fn cgroup_path(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .or_else(|| text.lines().find_map(|l| l.splitn(3, ':').nth(2)))
}

/// Reads `pid`'s clues from `proc_root` (`/proc`).
pub fn clues(proc_root: &Path, pid: u32) -> Clues {
    let dir = proc_root.join(pid.to_string());
    let mut c = Clues::default();
    let cgroup = std::fs::read_to_string(dir.join("cgroup")).unwrap_or_default();
    if let Some(path) = cgroup_path(&cgroup) {
        let ids = unit_ids(path);
        // `app-flatpak-ID-RANDOM.scope`: the launcher is "flatpak".
        if path
            .rsplit('/')
            .next()
            .is_some_and(|l| l.starts_with("app-flatpak-"))
        {
            c.flatpak = ids.first().cloned();
        } else {
            c.unit = ids;
        }
        c.snap = snap(path);
    }
    if let Ok(info) = std::fs::read_to_string(dir.join("root/.flatpak-info"))
        && let Some(name) = crate::icons::ini_value(&info, "Application", "name")
    {
        c.flatpak = Some(name);
    }
    if let Ok(env) = std::fs::read(dir.join("environ")) {
        for var in env.split(|b| *b == 0) {
            let var = String::from_utf8_lossy(var);
            let Some((k, v)) = var.split_once('=') else {
                continue;
            };
            match k {
                "GIO_LAUNCHED_DESKTOP_FILE" | "BAMF_DESKTOP_FILE_HINT"
                    if c.launched.is_none() && !v.is_empty() =>
                {
                    c.launched = Some(v.to_owned())
                }
                "APPIMAGE" if !v.is_empty() => c.appimage = Some(v.to_owned()),
                "FLATPAK_ID" if c.flatpak.is_none() && !v.is_empty() => {
                    c.flatpak = Some(v.to_owned())
                }
                _ => {}
            }
        }
    }
    c.exe = std::fs::read_link(dir.join("exe"))
        .ok()
        .and_then(|p| {
            p.file_name().map(|n| {
                n.to_string_lossy()
                    .trim_end_matches(" (deleted)")
                    .to_owned()
            })
        })
        .or_else(|| {
            std::fs::read_to_string(dir.join("comm"))
                .ok()
                .map(|s| s.trim().to_owned())
        });
    c
}

/// The entry a sandbox's ID names.
pub fn sandboxed<'a>(index: &'a Index, c: &Clues) -> Option<&'a Entry> {
    c.flatpak
        .as_deref()
        .and_then(|id| index.by_id(id))
        .or_else(|| {
            c.snap
                .as_ref()
                .and_then(|(name, app)| index.by_snap(name, app))
        })
}

/// The entry the process's unit, launcher or AppImage point to, if they can be believed.
pub fn launched<'a>(index: &'a Index, c: &Clues) -> Option<&'a Entry> {
    let exe = c.exe.as_deref().unwrap_or("");
    if exe.is_empty() || generic(exe) {
        return None;
    }
    let believable = |e: &&Entry| !(e.browser || e.terminal) || e.runs_program(exe);
    c.unit
        .iter()
        .find_map(|id| index.by_id(id).filter(believable))
        .or_else(|| {
            let launched = c.launched.as_deref()?;
            let file = launched.rsplit('/').next()?;
            let e = index
                .entries
                .iter()
                .find(|e| e.path == Path::new(launched))
                .or_else(|| index.by_id(file))?;
            Some(e).filter(believable)
        })
        .or_else(|| {
            index
                .by_exec_path(c.appimage.as_deref()?)
                .filter(|e| !e.terminal)
        })
}

#[cfg(test)]
fn entry<'a>(index: &'a Index, c: &Clues) -> Option<&'a Entry> {
    sandboxed(index, c).or_else(|| launched(index, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_from_each_launcher() {
        // GNOME Shell, KDE Plasma, uwsm (Hyprland, niri, sway), app2unit, an Electron app's own scope.
        assert_eq!(
            unit_ids(
                "/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-org.gnome.Nautilus-4211.scope"
            )[0],
            "org.gnome.Nautilus"
        );
        assert_eq!(
            unit_ids("app-org.kde.dolphin-4211.scope"),
            vec!["org.kde.dolphin"]
        );
        assert_eq!(
            unit_ids("app-Hyprland-com.discordapp.Discord@a1b2c3.service")[0],
            "com.discordapp.Discord"
        );
        assert_eq!(unit_ids("app-niri-firefox@9f.service")[0], "firefox");
        assert_eq!(
            unit_ids("app-bb\\x2ddesktop-120010.scope"),
            vec!["bb-desktop"]
        );
        assert_eq!(
            unit_ids("app-gnome-google\\x2dchrome-77.scope")[0],
            "google-chrome"
        );
        assert_eq!(
            unit_ids("app-com.google.Chrome-1774749.scope"),
            vec!["com.google.Chrome"]
        );
        assert!(unit_ids("session-2.scope").is_empty());
        assert!(unit_ids("app.slice").is_empty());
        assert!(unit_ids("hokum.service").is_empty());
    }

    #[test]
    fn snaps() {
        assert_eq!(
            snap(
                "/user.slice/user-1000.slice/user@1000.service/app.slice/snap.firefox.firefox-2f5d.scope"
            ),
            Some(("firefox".into(), "firefox".into()))
        );
        assert_eq!(
            snap(
                "app.slice/snap.telegram-desktop.telegram-desktop-4b1a6c3e-9d2f-4c1b-8a7e-1f2e3d4c5b6a.scope"
            ),
            Some(("telegram-desktop".into(), "telegram-desktop".into()))
        );
        assert_eq!(
            snap("snap.telegram-desktop.telegram-desktop.service"),
            Some(("telegram-desktop".into(), "telegram-desktop".into()))
        );
        assert_eq!(snap("app-gnome-firefox-1.scope"), None);
    }

    fn fake_proc(
        pid: u32,
        cgroup: &str,
        env: &[&str],
        exe: &str,
        flatpak_info: Option<&str>,
    ) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("hokum-proc-{}-{pid}", std::process::id()));
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cgroup"), format!("0::{cgroup}\n")).unwrap();
        std::fs::write(dir.join("environ"), env.join("\0")).unwrap();
        std::fs::write(dir.join("comm"), format!("{exe}\n")).unwrap();
        if let Some(info) = flatpak_info {
            std::fs::create_dir_all(dir.join("root")).unwrap();
            std::fs::write(dir.join("root/.flatpak-info"), info).unwrap();
        }
        root
    }

    fn index() -> Index {
        let e = |id: &str, t: &str| {
            crate::entries::parse(id, Path::new(&format!("/a/{id}.desktop")), t).unwrap()
        };
        Index {
            entries: vec![
                e(
                    "org.gnome.Nautilus",
                    "[Desktop Entry]\nName=Files\nExec=nautilus",
                ),
                e(
                    "co.hyprlab.Hylki.Beta",
                    "[Desktop Entry]\nName=Hylki (beta)\nX-Flatpak=co.hyprlab.Hylki.Beta",
                ),
                e(
                    "firefox_firefox",
                    "[Desktop Entry]\nName=Firefox\nX-SnapInstanceName=firefox",
                ),
                e(
                    "foot",
                    "[Desktop Entry]\nName=Foot\nExec=foot\nCategories=System;TerminalEmulator;",
                ),
                e(
                    "bb_nightly",
                    "[Desktop Entry]\nName=bb Nightly (1)\nExec=env X=1 /home/u/AppImages/bb_nightly.appimage %U",
                ),
                e(
                    "org.chromium.Chromium",
                    "[Desktop Entry]\nName=Chromium\nExec=chromium %U\nCategories=Network;WebBrowser;",
                ),
            ],
        }
    }

    #[test]
    fn processes_name_their_apps() {
        let idx = index();
        let check = |pid, cgroup, env: &[&str], exe, info: Option<&str>| {
            let root = fake_proc(pid, cgroup, env, exe, info);
            let found = entry(&idx, &clues(&root, pid)).map(|e| e.id.clone());
            let _ = std::fs::remove_dir_all(&root);
            found
        };
        assert_eq!(
            check(
                1,
                "/app.slice/app-gnome-org.gnome.Nautilus-42.scope",
                &[],
                "nautilus",
                None
            )
            .as_deref(),
            Some("org.gnome.Nautilus")
        );
        // The D-Bus proxy connects for a Flatpak; its scope says which.
        assert_eq!(
            check(
                2,
                "/app.slice/app-flatpak-co.hyprlab.Hylki.Beta-9.scope",
                &[],
                "xdg-dbus-proxy",
                None
            )
            .as_deref(),
            Some("co.hyprlab.Hylki.Beta")
        );
        assert_eq!(
            check(
                3,
                "/app.slice/whatever.scope",
                &[],
                "hylki",
                Some("[Application]\nname=co.hyprlab.Hylki.Beta\n")
            )
            .as_deref(),
            Some("co.hyprlab.Hylki.Beta")
        );
        assert_eq!(
            check(
                4,
                "/app.slice/snap.firefox.firefox-2f5d.scope",
                &[],
                "firefox",
                None
            )
            .as_deref(),
            Some("firefox_firefox")
        );
        assert_eq!(
            check(
                5,
                "/app.slice/app-bb\\x2ddesktop-7.scope",
                &["APPIMAGE=/home/u/AppImages/bb_nightly.appimage"],
                "bb-nightly",
                None
            )
            .as_deref(),
            Some("bb_nightly")
        );
        // notify-send from a script, and a program run in a terminal, aren't the terminal.
        assert_eq!(
            check(
                6,
                "/app.slice/app-Hyprland-foot@1.service",
                &[],
                "notify-send",
                None
            ),
            None
        );
        assert_eq!(
            check(
                7,
                "/app.slice/app-Hyprland-foot@1.service",
                &[],
                "backup-tool",
                None
            ),
            None
        );
        assert_eq!(
            check(
                8,
                "/app.slice/app-Hyprland-foot@1.service",
                &[],
                "foot",
                None
            )
            .as_deref(),
            Some("foot")
        );
        assert_eq!(
            check(
                9,
                "/session.slice/x.scope",
                &["GIO_LAUNCHED_DESKTOP_FILE=/a/org.gnome.Nautilus.desktop"],
                "nautilus",
                None
            )
            .as_deref(),
            Some("org.gnome.Nautilus")
        );
        // Electron apps that don't name themselves claim to be Chromium; a browser counts only as itself.
        assert_eq!(
            check(
                11,
                "/app.slice/app-org.chromium.Chromium-2390306.scope",
                &[],
                "codex",
                None
            ),
            None
        );
        assert_eq!(
            check(
                12,
                "/app.slice/app-org.chromium.Chromium-1.scope",
                &[],
                "chromium",
                None
            )
            .as_deref(),
            Some("org.chromium.Chromium")
        );
        // Omarchy launches through gtk-launch, which names no app; GLib's variable does.
        assert_eq!(
            check(
                13,
                "/app.slice/app-Hyprland-gtk\\x2dlaunch-5d2a0a16.scope",
                &["GIO_LAUNCHED_DESKTOP_FILE=/a/org.gnome.Nautilus.desktop"],
                "nautilus",
                None
            )
            .as_deref(),
            Some("org.gnome.Nautilus")
        );
        // A sandbox's ID counts even through a generic sender.
        assert_eq!(
            check(
                10,
                "/app.slice/app-flatpak-co.hyprlab.Hylki.Beta-9.scope",
                &[],
                "notify-send",
                None
            )
            .as_deref(),
            Some("co.hyprlab.Hylki.Beta")
        );
    }
}
