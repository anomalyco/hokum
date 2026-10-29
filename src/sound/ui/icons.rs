//! A playing app's icon, found the way a notification's is (`crate::icons`, `crate::entries`):
//! the icon its stream names, else the desktop entry for its binary or its name.

use std::path::PathBuf;

/// The best icon for a stream: what it names, its binary's desktop entry, then its app's name.
pub fn for_app(icon_name: &str, binary: &str, app: &str) -> Option<PathBuf> {
    let index = crate::entries::index();
    crate::icons::icon(icon_name)
        .or_else(|| index.by_name(binary).and_then(|e| e.icon()))
        .or_else(|| crate::icons::for_app("", None, app))
}
