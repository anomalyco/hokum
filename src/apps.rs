//! The apps that have sent notifications, newest first, so the settings window can offer
//! per-app choices for them. Kept in `$XDG_STATE_HOME/hokum/apps`, one per line: the name, then
//! (after a tab) the desktop entry Hokum found for it, so the list shows the cards' icons.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const KEEP: usize = 40;

/// An app's name and the desktop entry found for it.
type App = (String, Option<String>);

fn path() -> PathBuf {
    crate::style::state_dir().join("apps")
}

fn list() -> &'static Mutex<Vec<App>> {
    static APPS: OnceLock<Mutex<Vec<App>>> = OnceLock::new();
    APPS.get_or_init(|| Mutex::new(parse(&std::fs::read_to_string(path()).unwrap_or_default())))
}

fn parse(text: &str) -> Vec<App> {
    text.lines()
        .filter_map(|l| {
            let (name, entry) = l.split_once('\t').map_or((l, None), |(n, e)| {
                (n, Some(e.trim().to_owned()).filter(|e| !e.is_empty()))
            });
            Some((name.trim().to_owned(), entry)).filter(|(n, _)| !n.is_empty())
        })
        .collect()
}

/// An app's name as Hokum shows and keeps it: control characters (newlines and tabs too,
/// which would split this file's lines and fields) become spaces, runs of space collapse, and
/// it is at most 80 characters.
pub fn clean(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars().map(|c| if c.is_control() { ' ' } else { c }) {
        if ch == ' ' && (out.is_empty() || out.ends_with(' ')) {
            continue;
        }
        out.push(ch);
    }
    let out = out.trim_end();
    match out.char_indices().nth(80) {
        Some((i, _)) => out[..i].trim_end().to_owned(),
        None => out.to_owned(),
    }
}

/// Notes that `app` sent something (found as desktop entry `entry`); saves when the list changes.
pub fn record(app: &str, entry: Option<&str>) {
    let app = clean(app);
    let app = app.as_str();
    if app.is_empty() {
        return;
    }
    let entry = entry.map(clean).filter(|e| !e.is_empty());
    let item = (app.to_owned(), entry);
    let mut apps = list().lock().unwrap();
    if apps.first() == Some(&item) {
        return;
    }
    apps.retain(|(a, _)| a != app);
    apps.insert(0, item);
    apps.truncate(KEEP);
    let text: String = apps
        .iter()
        .map(|(a, e)| {
            format!(
                "{a}{}\n",
                e.as_deref().map(|e| format!("\t{e}")).unwrap_or_default()
            )
        })
        .collect();
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text);
}

pub fn seen() -> Vec<String> {
    list()
        .lock()
        .unwrap()
        .iter()
        .map(|(a, _)| a.clone())
        .collect()
}

/// The desktop entry found for `app` when it last sent something.
pub fn entry(app: &str) -> Option<String> {
    list()
        .lock()
        .unwrap()
        .iter()
        .find(|(a, _)| a == app)
        .and_then(|(_, e)| e.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_old_and_new_lines() {
        let apps = super::parse("WhatsApp\tWhatsApp\nSlack\n\nbb Nightly\tbb_nightly\n");
        assert_eq!(apps[0], ("WhatsApp".into(), Some("WhatsApp".into())));
        assert_eq!(apps[1], ("Slack".into(), None));
        assert_eq!(apps[2], ("bb Nightly".into(), Some("bb_nightly".into())));
    }

    #[test]
    fn names_lose_control_characters() {
        assert_eq!(super::clean("Evil\nInjected\tentry"), "Evil Injected entry");
        assert_eq!(super::clean("  a\r\n\u{7}b  "), "a b");
        assert_eq!(super::clean("\n\t"), "");
        assert_eq!(super::clean(&"x".repeat(200)).len(), 80);
        assert_eq!(super::clean("Slack"), "Slack");
    }
}
