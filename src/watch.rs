//! Live reload: polls config.kdl, the theme it starts from, the themes folder and the script,
//! and hands whatever changed to the overlay and the settings window. A few modification times
//! every 700 ms costs nothing and needs no inotify plumbing.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use rune::{AppHandle, OverlayHandle};

use crate::config::Loaded;

/// Everything that comes from files, loaded together.
#[derive(Clone)]
pub struct Config {
    pub loaded: Loaded,
    /// The script's source, if there is one.
    pub script: Option<String>,
}

pub fn load() -> Config {
    let loaded = crate::config::load();
    crate::clock::prefer(loaded.behaviour.clock);
    // Everything drawn from here on speaks the config's language, or the session's.
    crate::i18n::set(
        loaded
            .language
            .unwrap_or_else(crate::i18n::Lang::from_locale),
    );
    Config {
        loaded,
        script: std::fs::read_to_string(crate::script::path()).ok(),
    }
}

fn stamp(p: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn fingerprint(base: &str) -> Vec<Option<SystemTime>> {
    let mut v = vec![
        stamp(&crate::config::path()),
        stamp(&crate::script::path()),
        stamp(&crate::theme::dir()),
    ];
    // Omarchy swaps its theme folder on a switch, which the bundled `omarchy` theme follows.
    v.extend(crate::omarchy_theme::colors_path().map(|p| stamp(&p)));
    if let Some(crate::theme::Found::File(p)) = crate::theme::find(base) {
        v.push(stamp(&p));
    }
    v
}

pub fn spawn(overlay: OverlayHandle<crate::cards::Ui>, window: AppHandle<crate::settings::Ui>) {
    std::thread::spawn(move || {
        let mut base = crate::config::load().base_id;
        let mut last = fingerprint(&base);
        loop {
            std::thread::sleep(Duration::from_millis(700));
            if fingerprint(&base) == last {
                continue;
            }
            let config = load();
            base = config.loaded.base_id.clone();
            last = fingerprint(&base);
            let c = config.clone();
            overlay.update(move |ui, _| ui.configure(c));
            window.update(move |ui, _| ui.configure(config));
        }
    });
}
