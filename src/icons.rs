//! Finding icons: the freedesktop icon theme spec. A notification names an icon (`app_icon`,
//! `image-path`) or an app (`desktop-entry`, its name, its process); `crate::entries` turns an
//! app into the `Icon=` of its desktop entry and this module turns that into a file the image
//! cache decodes. The theme is the one the desktop asked for: the settings portal's (which
//! `crate::bus` passes in), KDE's `kdeglobals`, GTK's `settings.ini`, then the desktop's own
//! default. Lookups touch the filesystem, so they run on the bus thread and are cached per name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        Some(d) => dirs.push(d),
        None => dirs.extend(home().map(|h| h.join(".local/share"))),
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
    // Flatpak's and Snap's exports, which XDG_DATA_DIRS often lacks outside a full session (a
    // systemd user service gets them only from the packages' environment generators).
    dirs.extend(home().map(|h| h.join(".local/share/flatpak/exports/share")));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    dirs.push(PathBuf::from("/var/lib/snapd/desktop"));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

fn config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
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
    dirs
}

/// The theme the settings portal reports (`org.gnome.desktop.interface icon-theme`, or KDE's
/// `org.kde.kdeglobals.Icons Theme`), which `crate::bus` reads and follows.
static PORTAL_THEME: Mutex<Option<String>> = Mutex::new(None);

/// Uses `name` as the desktop's icon theme (`None`: fall back to the files), dropping every
/// cached lookup when it changes.
pub fn set_portal_theme(name: Option<String>) {
    let name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    let mut current = PORTAL_THEME.lock().unwrap();
    if *current != name {
        *current = name;
        drop(current);
        forget();
    }
}

/// `key` in `[group]` of an ini-style file (`index.theme`, `kdeglobals`, `settings.ini`).
pub fn ini_value(text: &str, group: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = g == group;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().trim_matches('"').to_owned()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// The icon theme the desktop asked for: the portal's, KDE's `kdeglobals`, GTK's settings
/// (4, 3, then 2's `~/.gtkrc-2.0`), else KDE's and GNOME's defaults.
fn configured() -> Option<String> {
    let portal = PORTAL_THEME.lock().unwrap().clone();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    configured_from(portal, &config_dirs(), home().as_deref(), &desktop)
}

fn configured_from(
    portal: Option<String>,
    config_dirs: &[PathBuf],
    home: Option<&Path>,
    desktop: &str,
) -> Option<String> {
    if portal.is_some() {
        return portal;
    }
    let read = |dir: &Path, file: &str| std::fs::read_to_string(dir.join(file)).ok();
    let files = config_dirs
        .iter()
        .find_map(|d| read(d, "kdeglobals").and_then(|t| ini_value(&t, "Icons", "Theme")))
        .or_else(|| {
            ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
                .iter()
                .find_map(|f| {
                    config_dirs.iter().find_map(|d| {
                        read(d, f).and_then(|t| ini_value(&t, "Settings", "gtk-icon-theme-name"))
                    })
                })
        })
        .or_else(|| {
            let text = std::fs::read_to_string(home?.join(".gtkrc-2.0")).ok()?;
            text.lines().find_map(|l| {
                let v = l
                    .trim()
                    .strip_prefix("gtk-icon-theme-name")?
                    .trim_start()
                    .strip_prefix('=')?;
                Some(v.trim().trim_matches('"').to_owned()).filter(|v| !v.is_empty())
            })
        });
    files.or_else(|| {
        let desktop = desktop.to_uppercase();
        let has = |d: &str| desktop.split(':').any(|x| x == d);
        if has("KDE") {
            Some("breeze".into())
        } else if [
            "GNOME",
            "UNITY",
            "BUDGIE",
            "PANTHEON",
            "X-CINNAMON",
            "CINNAMON",
        ]
        .iter()
        .any(|d| has(d))
        {
            Some("Adwaita".into())
        } else {
            None
        }
    })
}

/// One of a theme's directories, from its `index.theme`.
#[derive(Debug)]
struct Dir {
    path: String,
    /// Pixels: `Size` × `Scale`.
    px: u32,
    scalable: bool,
    symbolic: bool,
}

#[derive(Debug)]
struct Theme {
    name: String,
    /// Best first for a card's icon: scalable, then the nearest size at or above it, then
    /// smaller ones, then symbolic directories.
    dirs: Vec<Dir>,
}

/// What a card draws an icon at: 64pt at 2×.
const TARGET_PX: u32 = 128;

/// A theme's directories from its `index.theme` (the first one found wins, as the spec says).
fn parse_theme(name: &str, text: &str) -> (Theme, Vec<String>) {
    let parents = ini_value(text, "Icon Theme", "Inherits")
        .map(|p| {
            p.split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let listed = ["Directories", "ScaledDirectories"]
        .iter()
        .filter_map(|k| ini_value(text, "Icon Theme", k))
        .flat_map(|v| {
            v.split(',')
                .map(|d| d.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .filter(|d| !d.is_empty());
    let mut seen = std::collections::HashSet::new();
    let mut dirs: Vec<Dir> = listed
        .filter(|d| seen.insert(d.clone()))
        .map(|d| {
            let num = |k: &str| ini_value(text, &d, k).and_then(|v| v.parse::<u32>().ok());
            let size = num("Size").unwrap_or(48);
            let scale = num("Scale").unwrap_or(1).max(1);
            let scalable =
                ini_value(text, &d, "Type").is_some_and(|t| t.eq_ignore_ascii_case("scalable"));
            let symbolic = d.contains("symbolic");
            Dir {
                px: size * scale,
                scalable,
                symbolic,
                path: d,
            }
        })
        .collect();
    dirs.sort_by_key(|d| {
        let distance = match (d.scalable, d.px) {
            (true, _) => 0,
            (false, px) if px >= TARGET_PX => 1 + px - TARGET_PX,
            (false, px) => 100_000 + TARGET_PX - px,
        };
        (d.symbolic, distance)
    });
    (
        Theme {
            name: name.to_owned(),
            dirs,
        },
        parents,
    )
}

/// The older layouts, for a theme without an `index.theme` listing its directories.
fn legacy_dirs() -> Vec<Dir> {
    const SIZES: [&str; 10] = [
        "scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "symbolic",
        "32x32", "24x24",
    ];
    const KINDS: [&str; 6] = [
        "apps",
        "status",
        "devices",
        "places",
        "categories",
        "mimetypes",
    ];
    let mut dirs = Vec::new();
    for size in SIZES {
        for kind in KINDS {
            for path in [format!("{size}/{kind}"), format!("{kind}/{size}")] {
                dirs.push(Dir {
                    path,
                    px: 0,
                    scalable: false,
                    symbolic: size == "symbolic",
                });
            }
        }
    }
    dirs
}

static THEMES: Mutex<Option<Arc<Vec<Theme>>>> = Mutex::new(None);

/// The configured theme, its parents, hicolor, then Adwaita (for the status icons Hokum's own
/// cards use when the configured theme has none).
fn themes() -> Arc<Vec<Theme>> {
    if let Some(t) = THEMES.lock().unwrap().clone() {
        return t;
    }
    let themes = Arc::new(chain(configured(), &theme_roots()));
    *THEMES.lock().unwrap() = Some(themes.clone());
    themes
}

/// `configured`, the themes it inherits from, then the fallbacks, as found under `roots`.
fn chain(configured: Option<String>, roots: &[PathBuf]) -> Vec<Theme> {
    let mut out: Vec<Theme> = Vec::new();
    let mut queue: Vec<String> = configured.into_iter().collect();
    let mut tail = vec!["Adwaita".to_owned(), "hicolor".to_owned()];
    while let Some(name) = queue.pop().or_else(|| tail.pop()) {
        if out.iter().any(|t| t.name == name) || out.len() > 12 || name.contains('/') {
            continue;
        }
        let index = roots
            .iter()
            .find_map(|r| std::fs::read_to_string(r.join(&name).join("index.theme")).ok());
        if index.is_none() && !roots.iter().any(|r| r.join(&name).is_dir()) {
            continue;
        }
        let (mut theme, parents) = parse_theme(&name, index.as_deref().unwrap_or(""));
        if theme.dirs.is_empty() {
            theme.dirs = legacy_dirs();
        }
        // Parents come before the fallbacks, in their listed order.
        queue.extend(parents.into_iter().rev());
        out.push(theme);
    }
    out
}

/// Where themes live: `icons/` in every data dir, then `~/.icons`.
fn theme_roots() -> Vec<PathBuf> {
    data_dirs()
        .into_iter()
        .map(|d| d.join("icons"))
        .chain(home().map(|h| h.join(".icons")))
        .collect()
}

fn cache() -> &'static Mutex<HashMap<String, Option<PathBuf>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// Drops every cached lookup and the theme chain: the theme changed, or apps were installed.
pub fn forget() {
    cache().lock().unwrap().clear();
    *THEMES.lock().unwrap() = None;
}

/// An icon name in the theme, or a path (`file://` URIs too).
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
    if let Some(hit) = cache().lock().unwrap().get(name) {
        return hit.clone();
    }
    let found = lookup(name);
    cache()
        .lock()
        .unwrap()
        .insert(name.to_owned(), found.clone());
    found
}

fn lookup(name: &str) -> Option<PathBuf> {
    // Icon= should name no extension, but plenty of entries say `app.png`.
    let name = [".png", ".svg", ".xpm"]
        .iter()
        .find_map(|e| name.strip_suffix(e))
        .unwrap_or(name);
    if name.contains('/') {
        return None;
    }
    find(name, &themes(), &theme_roots()).or_else(|| {
        data_dirs()
            .iter()
            .map(|d| d.join("pixmaps"))
            .chain([PathBuf::from("/usr/share/pixmaps")])
            .flat_map(|d| ["svg", "png"].map(|e| d.join(format!("{name}.{e}"))))
            .find(|p| p.is_file())
    })
}

/// `name` in `themes`, each looked for under every root, best directory first.
fn find(name: &str, themes: &[Theme], roots: &[PathBuf]) -> Option<PathBuf> {
    for theme in themes {
        let bases: Vec<PathBuf> = roots
            .iter()
            .map(|r| r.join(&theme.name))
            .filter(|b| b.is_dir())
            .collect();
        for dir in &theme.dirs {
            for base in &bases {
                for ext in ["png", "svg"] {
                    let p = base.join(&dir.path).join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

/// The best icon for an app: the icon it names, else its desktop entry's (`crate::entries`
/// finds the entry from `desktop-entry`, its process or its name), else an icon called like it.
pub fn for_app(app_icon: &str, desktop: Option<&str>, app: &str) -> Option<PathBuf> {
    icon(app_icon)
        .or_else(|| {
            desktop
                .and_then(crate::entries::by_id)
                .and_then(|e| e.icon())
        })
        .or_else(|| crate::entries::by_name(app).and_then(|e| e.icon()))
        .or_else(|| {
            let slug = app.trim().to_lowercase().replace(' ', "-");
            icon(app.trim()).or_else(|| icon(&slug))
        })
}

/// A picture boiled down to compare with another: 16×16 RGB over grey, and its aspect.
type Thumb = (Vec<f32>, f32);

fn thumb(width: u32, height: u32, rgba: &[u8]) -> Option<Thumb> {
    const N: usize = 16;
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let mut out = vec![0f32; N * N * 3];
    let mut count = vec![0f32; N * N];
    for y in 0..h {
        for x in 0..w {
            let p = &rgba[(y * w + x) * 4..][..4];
            let a = p[3] as f32 / 255.0;
            let cell = (y * N / h) * N + x * N / w;
            for c in 0..3 {
                out[cell * 3 + c] += (p[c] as f32 / 255.0) * a + 0.5 * (1.0 - a);
            }
            count[cell] += 1.0;
        }
    }
    for (i, v) in out.iter_mut().enumerate() {
        *v /= count[i / 3].max(1.0);
    }
    Some((out, w as f32 / h as f32))
}

fn file_thumb(path: &Path) -> Option<Thumb> {
    static THUMBS: OnceLock<Mutex<HashMap<PathBuf, Option<Thumb>>>> = OnceLock::new();
    let thumbs = THUMBS.get_or_init(Mutex::default);
    if let Some(t) = thumbs.lock().unwrap().get(path) {
        return t.clone();
    }
    let t = rune::image::app_icon_rgba(path, 64)
        .ok()
        .and_then(|(w, h, rgba)| thumb(w, h, &rgba));
    let mut thumbs = thumbs.lock().unwrap();
    if thumbs.len() > 256 {
        thumbs.clear();
    }
    thumbs.insert(path.to_owned(), t.clone());
    t
}

/// Whether `picture` is the app's icon again (Electron apps send their icon as the picture):
/// the same file, or the same image once both are shrunk to a few pixels.
pub fn repeats(picture: &Source, icon: &Path) -> bool {
    let a = match picture {
        Source::File(p) if p == icon => return true,
        Source::File(p) => file_thumb(p),
        Source::Rgba {
            width,
            height,
            bytes,
        } => thumb(*width, *height, bytes),
    };
    let (Some((a, ra)), Some((b, rb))) = (a, file_thumb(icon)) else {
        return false;
    };
    let difference = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32;
    (ra / rb - 1.0).abs() < 0.1 && difference < 0.04
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

    fn tree(files: &[(&str, &str)]) -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("hokum-icons-{}-{n}", std::process::id()));
        for (path, text) in files {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        root
    }

    #[test]
    fn the_theme_comes_from_the_portal_kde_gtk_then_the_desktop() {
        let kde = tree(&[(
            "kdeglobals",
            "[General]\nTheme=x\n[Icons]\nTheme=Papirus-Dark\n",
        )]);
        let gtk = tree(&[(
            "gtk-3.0/settings.ini",
            "[Settings]\ngtk-icon-theme-name = \"Yaru-purple\"\n",
        )]);
        let gtk2 = tree(&[(
            ".gtkrc-2.0",
            "gtk-theme-name=\"x\"\ngtk-icon-theme-name=\"elementary\"\n",
        )]);
        let none = tree(&[]);
        assert_eq!(
            configured_from(
                Some("Portal".into()),
                std::slice::from_ref(&kde),
                None,
                "KDE"
            ),
            Some("Portal".into())
        );
        assert_eq!(
            configured_from(None, &[none.clone(), kde.clone(), gtk.clone()], None, ""),
            Some("Papirus-Dark".into())
        );
        assert_eq!(
            configured_from(None, &[none.clone(), gtk.clone()], None, ""),
            Some("Yaru-purple".into())
        );
        assert_eq!(
            configured_from(None, std::slice::from_ref(&none), Some(&gtk2), ""),
            Some("elementary".into())
        );
        assert_eq!(
            configured_from(None, std::slice::from_ref(&none), None, "KDE"),
            Some("breeze".into())
        );
        assert_eq!(
            configured_from(None, std::slice::from_ref(&none), None, "ubuntu:GNOME"),
            Some("Adwaita".into())
        );
        assert_eq!(
            configured_from(None, std::slice::from_ref(&none), None, "Hyprland"),
            None
        );
        for d in [kde, gtk, gtk2, none] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn themes_inherit_then_fall_back_and_look_in_their_listed_directories() {
        // Breeze keeps apps in apps/48 (kind/size), Papirus in 64x64/apps, Yaru inherits hicolor.
        let usr = tree(&[
            (
                "breeze/index.theme",
                "[Icon Theme]\nInherits=hicolor\nDirectories=apps/48,apps/16,status/22\n[apps/48]\nSize=48\nType=Fixed\n[apps/16]\nSize=16\n[status/22]\nSize=22\n",
            ),
            ("breeze/apps/48/org.kde.dolphin.svg", "<svg/>"),
            ("breeze/apps/16/org.kde.dolphin.svg", "<svg/>"),
            (
                "Papirus/index.theme",
                "[Icon Theme]\nInherits=breeze,hicolor\nDirectories=64x64/apps,symbolic/apps\n[64x64/apps]\nSize=64\n[symbolic/apps]\nSize=16\nType=Scalable\n",
            ),
            ("Papirus/symbolic/apps/firefox-symbolic.svg", "<svg/>"),
            ("Papirus/64x64/apps/firefox.svg", "<svg/>"),
            (
                "hicolor/index.theme",
                "[Icon Theme]\nDirectories=48x48/apps,256x256/apps,scalable/apps\n[48x48/apps]\nSize=48\n[256x256/apps]\nSize=256\n[scalable/apps]\nSize=128\nType=Scalable\n",
            ),
            ("hicolor/48x48/apps/firefox.png", "x"),
            ("hicolor/256x256/apps/slack.png", "x"),
            ("hicolor/48x48/apps/slack.png", "x"),
            (
                "Adwaita/index.theme",
                "[Icon Theme]\nDirectories=symbolic/status\n[symbolic/status]\nSize=16\nType=Scalable\n",
            ),
            (
                "Adwaita/symbolic/status/audio-volume-high-symbolic.svg",
                "<svg/>",
            ),
        ]);
        // A user's own hicolor icons, in a data dir without an index.theme.
        let local = tree(&[("hicolor/256x256/apps/bb.png", "x")]);
        let roots = vec![local.clone(), usr.clone()];
        let names: Vec<String> = chain(Some("Papirus".into()), &roots)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["Papirus", "breeze", "hicolor", "Adwaita"]);
        let themes = chain(Some("Papirus".into()), &roots);
        assert_eq!(
            find("firefox", &themes, &roots),
            Some(usr.join("Papirus/64x64/apps/firefox.svg")),
            "the theme's own first"
        );
        assert_eq!(
            find("org.kde.dolphin", &themes, &roots),
            Some(usr.join("breeze/apps/48/org.kde.dolphin.svg")),
            "the bigger one"
        );
        assert_eq!(
            find("slack", &themes, &roots),
            Some(usr.join("hicolor/256x256/apps/slack.png")),
            "at or above 128 first"
        );
        assert_eq!(
            find("bb", &themes, &roots),
            Some(local.join("hicolor/256x256/apps/bb.png")),
            "every data dir's copy of a theme"
        );
        assert_eq!(
            find("audio-volume-high-symbolic", &themes, &roots).is_some(),
            true,
            "Adwaita backs the rest"
        );
        assert_eq!(
            chain(Some("Missing".into()), &roots)
                .first()
                .map(|t| t.name.as_str()),
            Some("hicolor")
        );
        assert_eq!(
            chain(Some("../etc".into()), &roots)
                .first()
                .map(|t| t.name.as_str()),
            Some("hicolor")
        );
        let _ = std::fs::remove_dir_all(usr);
        let _ = std::fs::remove_dir_all(local);
    }

    #[test]
    fn a_picture_that_is_the_icon_again_repeats_it() {
        let dir = tree(&[]);
        std::fs::create_dir_all(&dir).unwrap();
        // A 32×32 icon: red square on transparent.
        let mut px = vec![0u8; 32 * 32 * 4];
        for y in 8..24 {
            for x in 8..24 {
                px[(y * 32 + x) * 4..][..4].copy_from_slice(&[220, 30, 30, 255]);
            }
        }
        let icon = dir.join("icon.png");
        rune::render::write_png(&icon, 32, 32, &px).unwrap();
        // The same picture at another size, as image-data.
        let mut big = vec![0u8; 64 * 64 * 4];
        for y in 0..64 {
            for x in 0..64 {
                big[(y * 64 + x) * 4..][..4]
                    .copy_from_slice(&px[((y / 2) * 32 + x / 2) * 4..][..4]);
            }
        }
        let same = Source::Rgba {
            width: 64,
            height: 64,
            bytes: big.into(),
        };
        assert!(repeats(&same, &icon));
        let avatar = Source::Rgba {
            width: 64,
            height: 64,
            bytes: vec![40u8; 64 * 64 * 4].into(),
        };
        assert!(!repeats(&avatar, &icon));
        assert!(repeats(&Source::File(icon.clone()), &icon));
        let _ = std::fs::remove_dir_all(dir);
    }
}
