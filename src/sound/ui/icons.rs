//! Finding an app's icon: the freedesktop icon theme spec, simplified (as Hokum does it). A
//! stream names an icon or a binary; either resolves to a file the image cache decodes. Lookups
//! touch the filesystem and are cached per name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        Some(d) => dirs.push(d),
        None => dirs.extend(home.as_ref().map(|h| h.join(".local/share"))),
    }
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let system = if system.is_empty() {
        "/usr/local/share:/usr/share".to_owned()
    } else {
        system
    };
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    // Flatpak exports, which XDG_DATA_DIRS often lacks outside a full session.
    dirs.extend(home.map(|h| h.join(".local/share/flatpak/exports/share")));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    dirs
}

/// The configured icon theme (GTK's setting), then its parents, then hicolor.
fn themes() -> &'static Vec<String> {
    static THEMES: OnceLock<Vec<String>> = OnceLock::new();
    THEMES.get_or_init(|| {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let configured = ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
            .iter()
            .find_map(|f| {
                let s = std::fs::read_to_string(home.join(".config").join(f)).ok()?;
                s.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("gtk-icon-theme-name")?
                        .trim_start()
                        .strip_prefix('=')
                        .map(|v| v.trim().trim_matches('"').to_owned())
                })
            });
        let mut out: Vec<String> = Vec::new();
        let mut queue: Vec<String> = configured.into_iter().collect();
        while let Some(theme) = queue.pop() {
            if out.contains(&theme) || out.len() > 8 {
                continue;
            }
            for dir in data_dirs() {
                if let Ok(index) =
                    std::fs::read_to_string(dir.join("icons").join(&theme).join("index.theme"))
                {
                    let parents = index
                        .lines()
                        .find_map(|l| l.strip_prefix("Inherits="))
                        .unwrap_or("");
                    queue.extend(
                        parents
                            .split(',')
                            .rev()
                            .map(|p| p.trim().to_owned())
                            .filter(|p| !p.is_empty()),
                    );
                    break;
                }
            }
            out.push(theme);
        }
        if !out.iter().any(|t| t == "hicolor") {
            out.push("hicolor".into());
        }
        out
    })
}

const SIZES: [&str; 10] = [
    "scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "symbolic", "32x32",
    "24x24",
];
const KINDS: [&str; 6] = [
    "apps",
    "status",
    "devices",
    "places",
    "categories",
    "mimetypes",
];

/// An icon name in the theme, or a path.
pub fn icon(name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Some(path) = name.strip_prefix("file://") {
        return Some(PathBuf::from(path)).filter(|p| p.is_file());
    }
    if name.starts_with('/') {
        return Some(PathBuf::from(name)).filter(|p| p.is_file());
    }
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(hit) = cache.lock().unwrap().get(name) {
        return hit.clone();
    }
    let found = lookup(name);
    cache.lock().unwrap().insert(name.to_owned(), found.clone());
    found
}

fn lookup(name: &str) -> Option<PathBuf> {
    let dirs = data_dirs();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for theme in themes() {
        let roots = dirs
            .iter()
            .map(|d| d.join("icons").join(theme))
            .chain([home.join(".icons").join(theme)]);
        for root in roots.filter(|r| r.is_dir()) {
            for size in SIZES {
                for kind in KINDS {
                    // Most themes use size/kind; some (Adwaita's newer layout) use kind/size.
                    for dir in [root.join(size).join(kind), root.join(kind).join(size)] {
                        for ext in ["svg", "png"] {
                            let p = dir.join(format!("{name}.{ext}"));
                            if p.is_file() {
                                return Some(p);
                            }
                        }
                    }
                }
            }
        }
    }
    dirs.iter()
        .map(|d| d.join("pixmaps"))
        .chain([PathBuf::from("/usr/share/pixmaps")])
        .flat_map(|d| ["svg", "png", "xpm"].map(|e| d.join(format!("{name}.{e}"))))
        .find(|p| p.is_file() && p.extension().is_some_and(|e| e != "xpm"))
}

/// The icon a desktop entry names (`slack` → slack.desktop → `Icon=slack`).
pub fn desktop_entry(id: &str) -> Option<PathBuf> {
    let id = id.trim().trim_end_matches(".desktop");
    if id.is_empty() {
        return None;
    }
    for dir in data_dirs() {
        let apps = dir.join("applications");
        let direct = apps.join(format!("{id}.desktop"));
        let file = if direct.is_file() {
            Some(direct)
        } else {
            find_entry(&apps, id)
        };
        if let Some(file) = file
            && let Ok(text) = std::fs::read_to_string(&file)
            && let Some(name) = text.lines().find_map(|l| l.strip_prefix("Icon="))
        {
            return icon(name);
        }
    }
    None
}

/// Entries named like `com.slack.Slack.desktop` when asked for `slack`.
fn find_entry(apps: &Path, id: &str) -> Option<PathBuf> {
    let id = id.to_lowercase();
    std::fs::read_dir(apps)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            p.extension().is_some_and(|e| e == "desktop")
                && stem.rsplit('.').next() == Some(id.as_str())
        })
}

/// The best icon for a stream: what it names, its binary's desktop entry, then its app's name.
pub fn for_app(icon_name: &str, binary: &str, app: &str) -> Option<PathBuf> {
    icon(icon_name)
        .or_else(|| {
            (!binary.is_empty())
                .then(|| desktop_entry(&binary.to_lowercase()))
                .flatten()
        })
        .or_else(|| {
            let slug = app.trim().to_lowercase().replace(' ', "-");
            desktop_entry(&slug).or_else(|| icon(&slug))
        })
}
