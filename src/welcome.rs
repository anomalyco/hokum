//! The first run: one card from Hokum itself, with a way into the settings window. A marker,
//! `$XDG_STATE_HOME/hokum/welcomed`, keeps it to once.

use std::path::Path;

use crate::card::{Note, Timeout};
use crate::icons::Source;

/// Far from the ids the bus hands out and the settings window's test cards.
pub const ID: u32 = u32::MAX / 2;

pub fn due() -> bool {
    due_in(&crate::style::state_dir())
}

/// Remembers that the welcome was shown.
pub fn done() {
    mark(&crate::style::state_dir());
}

fn due_in(state: &Path) -> bool {
    !state.join("welcomed").exists()
}

fn mark(state: &Path) {
    let _ = std::fs::create_dir_all(state);
    let _ = std::fs::write(state.join("welcomed"), "");
}

pub fn note() -> Note {
    let mut n = Note::new(
        ID,
        "Hokum",
        "Welcome to Hokum",
        "This is how your notifications look now. Make them yours in Settings.",
    );
    n.actions = vec![("open-settings".into(), "Open Settings".into())];
    n.has_default = true;
    n.timeout = Timeout::Never;
    n.desktop = Some("hokum".into());
    n.icon_src = crate::icons::for_app("", Some("hokum"), "Hokum").map(Source::File);
    n
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_shows_until_the_marker_is_written() {
        let state = std::env::temp_dir().join(format!("hokum-welcome-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        assert!(super::due_in(&state));
        super::mark(&state);
        assert!(!super::due_in(&state));
        let _ = std::fs::remove_dir_all(&state);
    }
}
