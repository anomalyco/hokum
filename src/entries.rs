//! The installed apps' desktop entries, indexed the ways a notification can name an app: by
//! desktop file ID (`desktop-entry`, a sandbox's app ID, a systemd app unit), by the name it
//! sends (`StartupWMClass`, `Name=`, the program it runs) and, for web apps a browser shows
//! notifications for, by the site its launcher opens (`--app=https://…`).
//!
//! The index is built once and rebuilt when an `applications/` folder changes (looked at, at
//! most, every couple of seconds), which also drops `crate::icons`' cached lookups: installing
//! an app usually installs its icons too.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// One desktop entry, as much of it as finding an app needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// The desktop file ID without `.desktop` (`org.gnome.Nautilus`, `kde4-kate` for
    /// `kde4/kate.desktop`).
    pub id: String,
    pub path: PathBuf,
    /// `Name=`.
    pub name: String,
    /// `Name=` and every `Name[xx]=`.
    names: Vec<String>,
    /// `Icon=`: a theme icon's name or a path.
    pub icon_key: Option<String>,
    wm_class: Option<String>,
    /// The program `Exec=` runs (its file name), past `env VAR=…` and `flatpak run`.
    program: Option<String>,
    /// `Exec=`'s words.
    exec: Vec<String>,
    /// A Flatpak's app ID (`X-Flatpak=`, or `flatpak run`'s).
    pub flatpak: Option<String>,
    /// A Snap's name (`X-SnapInstanceName=`).
    snap: Option<String>,
    /// The site a Chromium-style app window opens (`--app=https://web.whatsapp.com/` →
    /// `web.whatsapp.com`), without `www.`.
    pub site: Option<String>,
    /// `Categories=` has `WebBrowser`.
    pub browser: bool,
    /// `Categories=` has `TerminalEmulator`.
    pub terminal: bool,
}

impl Entry {
    /// The file for `Icon=`.
    pub fn icon(&self) -> Option<PathBuf> {
        crate::icons::icon(self.icon_key.as_deref()?)
    }

    /// Whether `Exec=` runs `path` (an AppImage, say).
    pub fn runs(&self, path: &str) -> bool {
        self.exec.iter().any(|w| w == path)
    }

    /// Whether `Exec=` runs a program called `name`.
    pub fn runs_program(&self, name: &str) -> bool {
        self.program.as_deref().is_some_and(|p| p == name)
    }
}

/// `Exec=`'s words: whitespace-separated, double quotes group, `\` escapes inside them.
fn words(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut escaped, mut any) = (false, false, false);
    for ch in exec.chars() {
        match ch {
            _ if escaped => {
                word.push(ch);
                escaped = false;
            }
            '\\' if quoted => escaped = true,
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                any = false;
            }
            c => word.push(c),
        }
    }
    if any || !word.is_empty() {
        out.push(word);
    }
    out
}

fn file_name(word: &str) -> String {
    word.rsplit('/').next().unwrap_or(word).to_owned()
}

/// The host of an http(s) URL, lowercased, without `www.`.
pub fn site(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    (host.contains('.') && !host.contains(char::is_whitespace)).then_some(host)
}

/// Reads a desktop entry's `[Desktop Entry]` group; `None` for anything that isn't a shown or
/// hidden application (Links, Directories, `Hidden=true` entries, which mean "deleted").
pub fn parse(id: &str, path: &Path, text: &str) -> Option<Entry> {
    let mut e = Entry {
        id: id.to_owned(),
        path: path.to_owned(),
        ..Entry::default()
    };
    let mut inside = false;
    let mut kind = None;
    let mut exec = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| inside) else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Type" => kind = Some(value.to_owned()),
            "Hidden" if value == "true" => return None,
            "Name" => {
                e.name = value.to_owned();
                e.names.push(value.to_owned());
            }
            k if k.starts_with("Name[") => e.names.push(value.to_owned()),
            "Icon" if !value.is_empty() => e.icon_key = Some(value.to_owned()),
            "StartupWMClass" if !value.is_empty() => e.wm_class = Some(value.to_owned()),
            "Exec" => exec = Some(value.to_owned()),
            "X-Flatpak" if !value.is_empty() => e.flatpak = Some(value.to_owned()),
            "X-SnapInstanceName" if !value.is_empty() => e.snap = Some(value.to_owned()),
            "Categories" => {
                let has = |c: &str| value.split(';').any(|x| x == c);
                e.browser = has("WebBrowser");
                e.terminal = has("TerminalEmulator");
            }
            _ => {}
        }
    }
    if kind.as_deref().is_some_and(|k| k != "Application") {
        return None;
    }
    if let Some(exec) = exec {
        let w = words(&exec);
        let mut i = 0;
        if w.first().is_some_and(|p| file_name(p) == "env") {
            i = 1;
            while w
                .get(i)
                .is_some_and(|a| a.contains('=') && !a.starts_with('-'))
            {
                i += 1;
            }
        }
        if let Some(p) = w.get(i) {
            let program = file_name(p);
            if program == "flatpak" && w.get(i + 1).is_some_and(|a| a == "run") {
                let app = w[i + 2..]
                    .iter()
                    .find(|a| !a.starts_with('-') && !a.starts_with('@'));
                e.flatpak = e.flatpak.take().or_else(|| app.cloned());
            }
            e.program = Some(program);
        }
        // `--app=URL`, also inside `sh -c "…"` (Linux Mint's Web Apps); else the site a
        // launcher script that isn't a browser opens (Omarchy's `omarchy-launch-webapp URL`).
        let nested: Vec<String> = w
            .iter()
            .filter(|a| a.contains(' '))
            .flat_map(|a| words(a))
            .collect();
        e.site = w
            .iter()
            .chain(&nested)
            .find_map(|a| site(a.strip_prefix("--app=")?.trim_matches(['"', '\''])));
        if e.site.is_none() && !e.browser {
            e.site = w.iter().skip(i + 1).find_map(|a| site(a));
        }
        e.exec = w;
    }
    Some(e)
}

/// The desktop file ID of `path` under `apps` (`apps/kde4/kate.desktop` → `kde4-kate`).
fn id_of(apps: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(apps).ok()?.to_str()?;
    Some(rel.strip_suffix(".desktop")?.replace('/', "-"))
}

/// Every entry in `dirs`' `applications/` folders; an ID in an earlier dir hides later ones.
pub fn scan(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in dirs {
        let apps = dir.join("applications");
        let mut files = Vec::new();
        collect(&apps, &mut files, 0);
        files.sort();
        for file in files {
            let Some(id) = id_of(&apps, &file) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            out.extend(parse(&id, &file, &text));
        }
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for e in read.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "desktop") {
            out.push(p);
        } else if depth < 2 && e.file_type().is_ok_and(|t| t.is_dir()) {
            collect(&p, out, depth + 1);
        }
    }
}

/// Lookups over a scan.
#[derive(Default)]
pub struct Index {
    pub entries: Vec<Entry>,
}

pub fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

/// `Name=` without a trailing `(…)`, which AppImage launchers and betas often add
/// (`bb Nightly (0.46.2-nightly.1)`).
pub fn bare(name: &str) -> &str {
    match name
        .trim_end()
        .strip_suffix(')')
        .and_then(|n| n.rfind(" (").map(|i| &name[..i]))
    {
        Some(b) if !b.trim().is_empty() => b.trim(),
        _ => name.trim(),
    }
}

impl Index {
    /// The entry a desktop file ID names: exact, then in any case, then (for `slack`) one whose
    /// reverse-DNS ID ends in it (`com.slack.Slack`).
    pub fn by_id(&self, id: &str) -> Option<&Entry> {
        let id = id.trim().trim_end_matches(".desktop");
        if id.is_empty() {
            return None;
        }
        let id = id.rsplit('/').next().unwrap_or(id);
        self.entries
            .iter()
            .find(|e| e.id == id)
            .or_else(|| self.entries.iter().find(|e| same(&e.id, id)))
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| e.flatpak.as_deref() == Some(id))
            })
            .or_else(|| {
                (!id.contains('.')).then(|| {
                    self.entries.iter().find(|e| {
                        e.id.contains('.') && e.id.rsplit('.').next().is_some_and(|t| same(t, id))
                    })
                })?
            })
    }

    /// The entry for an app's name as it sent it: an ID, then `StartupWMClass` (what the app
    /// calls itself to the compositor, the usual source of the name), then `Name=` (with or
    /// without a `(version)`), then the program `Exec=` runs. Web apps (launchers opening one
    /// site) only match by ID or name, never by their browser's program.
    pub fn by_name(&self, app: &str) -> Option<&Entry> {
        let app = app.trim();
        if app.is_empty() {
            return None;
        }
        let slug = app.to_lowercase().replace(' ', "-");
        self.by_id(app)
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| e.wm_class.as_deref().is_some_and(|w| same(w, app)))
            })
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| e.names.iter().any(|n| same(n, app)))
            })
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| e.names.iter().any(|n| same(bare(n), app)))
            })
            .or_else(|| (slug != app).then(|| self.by_id(&slug)).flatten())
            .or_else(|| {
                self.entries.iter().find(|e| {
                    e.site.is_none()
                        && e.program
                            .as_deref()
                            .is_some_and(|p| same(p, app) || same(p, &slug))
                })
            })
    }

    /// A web app's launcher for `site` (`web.whatsapp.com`).
    pub fn by_site(&self, site: &str) -> Option<&Entry> {
        let site = site.strip_prefix("www.").unwrap_or(site);
        self.entries
            .iter()
            .find(|e| e.site.as_deref() == Some(site))
    }

    /// The launcher that runs `path` (an AppImage's).
    pub fn by_exec_path(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.runs(path))
    }

    /// A Snap's entry: `firefox_firefox` for the `firefox` app of snap `firefox`.
    pub fn by_snap(&self, snap: &str, app: &str) -> Option<&Entry> {
        self.by_id(&format!("{snap}_{app}")).or_else(|| {
            self.entries
                .iter()
                .find(|e| e.snap.as_deref() == Some(snap))
        })
    }
}

struct Live {
    index: Arc<Index>,
    stamp: Vec<Option<SystemTime>>,
    checked: Instant,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn stamp(dirs: &[PathBuf]) -> Vec<Option<SystemTime>> {
    let mut v = Vec::new();
    for d in dirs {
        let apps = d.join("applications");
        v.push(std::fs::metadata(&apps).and_then(|m| m.modified()).ok());
        // Subfolders (kde4/, wine/) change without touching their parent.
        if let Ok(read) = std::fs::read_dir(&apps) {
            let mut subs: Vec<_> = read
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect();
            subs.sort();
            v.extend(
                subs.iter()
                    .map(|s| std::fs::metadata(s).and_then(|m| m.modified()).ok()),
            );
        }
    }
    v
}

/// The current index, rebuilt if an `applications/` folder changed.
pub fn index() -> Arc<Index> {
    let mut live = LIVE.lock().unwrap();
    if let Some(l) = live.as_mut()
        && l.checked.elapsed() < Duration::from_secs(2)
    {
        return l.index.clone();
    }
    let dirs = crate::icons::data_dirs();
    let now = stamp(&dirs);
    if let Some(l) = live.as_mut() {
        l.checked = Instant::now();
        if l.stamp == now {
            return l.index.clone();
        }
        crate::icons::forget();
    }
    let index = Arc::new(Index {
        entries: scan(&dirs),
    });
    *live = Some(Live {
        index: index.clone(),
        stamp: now,
        checked: Instant::now(),
    });
    index
}

pub fn by_id(id: &str) -> Option<Entry> {
    index().by_id(id).cloned()
}

pub fn by_name(app: &str) -> Option<Entry> {
    index().by_name(app).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, text: &str) -> Entry {
        parse(id, Path::new("/x"), text).unwrap()
    }

    fn index(entries: &[(&str, &str)]) -> Index {
        Index {
            entries: entries.iter().map(|(id, t)| entry(id, t)).collect(),
        }
    }

    #[test]
    fn exec_words_unwrap_env_flatpak_and_quotes() {
        let e = entry(
            "a",
            "[Desktop Entry]\nType=Application\nExec=env DESKTOPINTEGRATION=1 /home/u/AppImages/bb.appimage --no-sandbox %U",
        );
        assert_eq!(e.program.as_deref(), Some("bb.appimage"));
        assert!(e.runs("/home/u/AppImages/bb.appimage"));
        let f = entry(
            "b",
            "[Desktop Entry]\nExec=/usr/bin/flatpak run --branch=beta --arch=x86_64 --command=hylki --file-forwarding co.hyprlab.Hylki.Beta @@u %U @@",
        );
        assert_eq!(f.flatpak.as_deref(), Some("co.hyprlab.Hylki.Beta"));
        let q = entry(
            "c",
            "[Desktop Entry]\nExec=\"/opt/My App/run\" --flag \"a \\\"b\\\"\"",
        );
        assert_eq!(q.exec, vec!["/opt/My App/run", "--flag", "a \"b\""]);
        assert_eq!(q.program.as_deref(), Some("run"));
    }

    #[test]
    fn hidden_links_and_other_groups_are_skipped() {
        assert!(
            parse(
                "a",
                Path::new("/x"),
                "[Desktop Entry]\nType=Application\nHidden=true\nName=A"
            )
            .is_none()
        );
        assert!(parse("a", Path::new("/x"), "[Desktop Entry]\nType=Link\nName=A").is_none());
        let e = entry(
            "a",
            "[Desktop Entry]\nName=A\nIcon=a\n[Desktop Action new]\nName=New\nIcon=b\nExec=x --app=https://b.com",
        );
        assert_eq!(
            (e.name.as_str(), e.icon_key.as_deref(), e.site.as_deref()),
            ("A", Some("a"), None)
        );
    }

    #[test]
    fn sites_from_app_mode_launchers() {
        let e = entry(
            "WhatsApp",
            "[Desktop Entry]\nName=WhatsApp\nExec=omarchy-launch-webapp https://web.whatsapp.com/",
        );
        assert_eq!(
            e.site.as_deref(),
            Some("web.whatsapp.com"),
            "a launcher script's URL"
        );
        let e = entry(
            "m",
            "[Desktop Entry]\nName=Gmail\nExec=sh -c 'XAPP_FORCE_GTKWINDOW_ICON=\"/i.svg\" chromium --app=\"https://mail.google.com/\" --class=WebApp-x'",
        );
        assert_eq!(
            e.site.as_deref(),
            Some("mail.google.com"),
            "Mint's Web Apps"
        );
        let e = entry(
            "f",
            "[Desktop Entry]\nName=Firefox\nCategories=Network;WebBrowser;\nExec=firefox https://start.example.com %u",
        );
        assert_eq!(
            e.site, None,
            "a browser opening its start page isn't a web app"
        );
        let e = entry(
            "w",
            "[Desktop Entry]\nName=WhatsApp\nExec=chromium --app=https://www.web.whatsapp.com/ --profile-directory=Default",
        );
        assert_eq!(e.site.as_deref(), Some("web.whatsapp.com"));
        assert_eq!(
            site("https://user@Example.com:8443/x?y"),
            Some("example.com".into())
        );
        assert_eq!(site("http://localhost:3000"), None, "no dot, no site");
        assert_eq!(site("file:///tmp/x"), None);
    }

    #[test]
    fn ids_match_exactly_then_in_any_case_then_by_reverse_dns_tail() {
        let i = index(&[
            (
                "co.hyprlab.Hylki.Beta",
                "[Desktop Entry]\nName=Hylki (beta)\nIcon=/h.png",
            ),
            ("com.slack.Slack", "[Desktop Entry]\nName=Slack"),
            ("org.gnome.Nautilus", "[Desktop Entry]\nName=Files"),
        ]);
        assert_eq!(
            i.by_id("co.hyprlab.Hylki.Beta").unwrap().id,
            "co.hyprlab.Hylki.Beta"
        );
        assert_eq!(
            i.by_id("co.hyprlab.hylki.beta.desktop").unwrap().id,
            "co.hyprlab.Hylki.Beta"
        );
        assert_eq!(i.by_id("slack").unwrap().id, "com.slack.Slack");
        assert_eq!(i.by_id("nautilus").unwrap().id, "org.gnome.Nautilus");
        assert!(
            i.by_id("beta").is_none() || i.by_id("beta").unwrap().id == "co.hyprlab.Hylki.Beta"
        );
        assert!(
            i.by_id("files").is_none(),
            "an ID lookup doesn't read names"
        );
    }

    #[test]
    fn names_match_wm_class_name_version_suffix_then_program() {
        let i = index(&[
            (
                "bb_nightly",
                "[Desktop Entry]\nName=bb Nightly (0.46.2-nightly.38756742631.1)\nStartupWMClass=bb Nightly\nExec=env DESKTOPINTEGRATION=1 /home/u/AppImages/bb_nightly.appimage %U",
            ),
            (
                "org.kde.dolphin",
                "[Desktop Entry]\nName=Dolphin\nName[ar]=دولفين\nExec=dolphin %u",
            ),
            (
                "chrome-abc-Default",
                "[Desktop Entry]\nName=Team Tickets\nExec=/opt/google/chrome/google-chrome --profile-directory=Default --app-id=abc\nStartupWMClass=crx_abc",
            ),
            (
                "google-chrome",
                "[Desktop Entry]\nName=Google Chrome\nExec=/usr/bin/google-chrome-stable %U\nCategories=Network;WebBrowser;",
            ),
            (
                "x",
                "[Desktop Entry]\nName=X\nExec=chromium --app=https://x.com/",
            ),
            (
                "telegram",
                "[Desktop Entry]\nName=Telegram Desktop\nExec=Telegram -- %u",
            ),
        ]);
        assert_eq!(i.by_name("bb Nightly").unwrap().id, "bb_nightly");
        assert_eq!(i.by_name("Dolphin").unwrap().id, "org.kde.dolphin");
        assert_eq!(i.by_name("دولفين").unwrap().id, "org.kde.dolphin");
        assert_eq!(i.by_name("dolphin").unwrap().id, "org.kde.dolphin");
        assert_eq!(i.by_name("Team Tickets").unwrap().id, "chrome-abc-Default");
        assert_eq!(i.by_name("Google Chrome").unwrap().id, "google-chrome");
        assert_eq!(
            i.by_name("google-chrome-stable").unwrap().id,
            "google-chrome"
        );
        assert_eq!(i.by_name("telegram").unwrap().id, "telegram");
        assert_eq!(
            i.by_name("chromium"),
            None,
            "a web app's launcher isn't its browser"
        );
        assert_eq!(i.by_site("www.x.com").unwrap().id, "x");
        assert_eq!(
            i.by_exec_path("/home/u/AppImages/bb_nightly.appimage")
                .unwrap()
                .id,
            "bb_nightly"
        );
        assert!(i.by_name("Nightly").is_none());
    }

    #[test]
    fn earlier_dirs_hide_later_ids_and_subfolders_join_with_dashes() {
        let root = std::env::temp_dir().join(format!("hokum-desktop-{}", std::process::id()));
        let (home, usr) = (root.join("home"), root.join("usr"));
        for (dir, file, name) in [
            (&home, "a.desktop", "Mine"),
            (&usr, "a.desktop", "System"),
            (&usr, "kde4/kate.desktop", "Kate"),
        ] {
            let p = dir.join("applications").join(file);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(
                p,
                format!("[Desktop Entry]\nType=Application\nName={name}\n"),
            )
            .unwrap();
        }
        let entries = scan(&[home, usr]);
        let _ = std::fs::remove_dir_all(&root);
        let names: Vec<_> = entries
            .iter()
            .map(|e| (e.id.as_str(), e.name.as_str()))
            .collect();
        assert_eq!(names, vec![("a", "Mine"), ("kde4-kate", "Kate")]);
    }
}
