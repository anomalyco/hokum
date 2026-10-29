//! The apps that have sent notifications, newest first, so the settings window can offer
//! per-app choices for them. Kept in `$XDG_STATE_HOME/hokum/apps`, one name per line.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const KEEP: usize = 40;

fn path() -> PathBuf {
    crate::style::state_dir().join("apps")
}

fn list() -> &'static Mutex<Vec<String>> {
    static APPS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    APPS.get_or_init(|| {
        let text = std::fs::read_to_string(path()).unwrap_or_default();
        Mutex::new(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    })
}

/// Notes that `app` sent something; saves when the list changes.
pub fn record(app: &str) {
    let app = app.trim();
    if app.is_empty() {
        return;
    }
    let mut apps = list().lock().unwrap();
    if apps.first().is_some_and(|a| a == app) {
        return;
    }
    apps.retain(|a| a != app);
    apps.insert(0, app.to_owned());
    apps.truncate(KEEP);
    let text = apps.join("\n") + "\n";
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text);
}

pub fn seen() -> Vec<String> {
    list().lock().unwrap().clone()
}
