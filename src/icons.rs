//! Finding an app's icon: the freedesktop icon theme spec, simplified. A notification names
//! an icon (`app_icon`, `image-path`) or a desktop entry (`desktop-entry`); either resolves to
//! a file the image cache decodes. Lookups touch the filesystem, so they run on the bus thread
//! and are cached per name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Where a picture comes from, before the UI thread decodes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    File(PathBuf),
    /// Straight-alpha sRGB RGBA8, as the spec's `image-data` hint carries (after conversion).
    Rgba {
        width: u32,
        height: u32,
        bytes: std::sync::Arc<[u8]>,
    },
}

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

/// The best icon for a notification: what it names, its desktop entry, then its app's name.
pub fn for_app(app_icon: &str, desktop: Option<&str>, app: &str) -> Option<PathBuf> {
    icon(app_icon)
        .or_else(|| desktop.and_then(desktop_entry))
        .or_else(|| {
            let slug = app.trim().to_lowercase().replace(' ', "-");
            desktop_entry(&slug).or_else(|| icon(&slug))
        })
}

/// The spec's `image-data` (`(iiibiiay)`: width, height, rowstride, has alpha, bits per
/// sample, channels, data) as RGBA8.
pub fn image_data(
    width: i32,
    height: i32,
    stride: i32,
    alpha: bool,
    bits: i32,
    channels: i32,
    data: &[u8],
) -> Option<Source> {
    let (w, h, stride, channels) = (
        width as usize,
        height as usize,
        stride as usize,
        channels as usize,
    );
    if bits != 8
        || w == 0
        || h == 0
        || w > 4096
        || h > 4096
        || !(3..=4).contains(&channels)
        || (alpha && channels != 4)
    {
        return None;
    }
    if data.len() < stride * (h - 1) + w * channels {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        for px in data[row * stride..row * stride + w * channels].chunks_exact(channels) {
            out.extend_from_slice(&[px[0], px[1], px[2], if channels == 4 { px[3] } else { 255 }]);
        }
    }
    Some(Source::Rgba {
        width: w as u32,
        height: h as u32,
        bytes: out.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_data_drops_row_padding_and_fills_alpha() {
        // 2×1 RGB with a 2-byte row pad.
        let src = image_data(2, 1, 8, false, 8, 3, &[1, 2, 3, 4, 5, 6, 0, 0]).unwrap();
        let Source::Rgba {
            width,
            height,
            bytes,
        } = src
        else {
            panic!()
        };
        assert_eq!((width, height), (2, 1));
        assert_eq!(&*bytes, &[1, 2, 3, 255, 4, 5, 6, 255]);
        assert!(
            image_data(2, 1, 8, false, 16, 3, &[0; 8]).is_none(),
            "only 8-bit samples"
        );
        assert!(
            image_data(2, 2, 6, false, 8, 3, &[0; 7]).is_none(),
            "short data"
        );
    }
}
