//! The window's own line icons, on a 16-point grid in the stroke style of Hokum's settings
//! (shared with Hokum Sound).

use rune::Icon;

use crate::bluetooth::model::Kind;

macro_rules! glyph {
    ($id:ident, $name:literal, $body:literal) => {
        pub static $id: Icon = Icon { name: $name, view: 16.0, body: $body };
    };
}

glyph!(BLUETOOTH, "bluetooth", r#"<path d="M4.5 5l7 6-3.5 3V2l3.5 3-7 6" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(HEADPHONES, "headphones", r#"<path d="M2.5 10V8a5.5 5.5 0 0 1 11 0v2" stroke="currentColor"/><rect x="2.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/><rect x="10.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/>"#);
glyph!(HEADSET, "headset", r#"<path d="M2.5 10V8a5.5 5.5 0 0 1 11 0v2" stroke="currentColor"/><rect x="2.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/><rect x="10.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/><path d="M12 13.5c0 .8-1.4 1.5-3.5 1.5" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(SPEAKER, "speaker", r#"<rect x="4" y="1.5" width="8" height="13" rx="2" stroke="currentColor"/><circle cx="8" cy="9.5" r="2.5" stroke="currentColor"/><circle cx="8" cy="4.5" r=".7" fill="currentColor"/>"#);
glyph!(PHONE, "phone", r#"<rect x="4.5" y="1.5" width="7" height="13" rx="1.8" stroke="currentColor"/><path d="M7 12.5h2" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(COMPUTER, "computer", r#"<rect x="2.5" y="3" width="11" height="7.5" rx="1.2" stroke="currentColor"/><path d="M1 13h14" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(TABLET, "tablet", r#"<rect x="2.5" y="1.5" width="11" height="13" rx="1.8" stroke="currentColor"/><path d="M7 12.5h2" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(KEYBOARD, "keyboard", r#"<rect x="1.5" y="4" width="13" height="8" rx="1.5" stroke="currentColor"/><path d="M4 6.5h.01M6.5 6.5h.01M9 6.5h.01M11.5 6.5h.01M5 9.5h6" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(MOUSE, "mouse", r#"<rect x="4" y="1.5" width="8" height="13" rx="4" stroke="currentColor"/><path d="M8 4v2.5" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(GAMEPAD, "gamepad", r#"<path d="M5 4.5h6a3.5 3.5 0 0 1 3.4 4.3l-.6 2.6a1.8 1.8 0 0 1-3 .9L9.5 11h-3l-1.3 1.3a1.8 1.8 0 0 1-3-.9l-.6-2.6A3.5 3.5 0 0 1 5 4.5Z" stroke="currentColor" stroke-linejoin="round"/><path d="M5.5 7v2M4.5 8h2" stroke="currentColor" stroke-linecap="round"/><circle cx="10.5" cy="7.5" r=".6" fill="currentColor"/><circle cx="11.5" cy="9" r=".6" fill="currentColor"/>"#);
glyph!(PEN, "pen", r#"<path d="M10.5 2.5l3 3-8 8H2.5v-3Z" stroke="currentColor" stroke-linejoin="round"/><path d="M9 4l3 3" stroke="currentColor"/>"#);
glyph!(WATCH, "watch", r#"<rect x="3.5" y="4" width="9" height="8" rx="2.5" stroke="currentColor"/><path d="M5.5 4l.5-2.5h4l.5 2.5M5.5 12l.5 2.5h4l.5-2.5" stroke="currentColor" stroke-linejoin="round"/>"#);
glyph!(DISPLAY, "display", r#"<rect x="1.5" y="2.5" width="13" height="8.5" rx="1.5" stroke="currentColor"/><path d="M6 14h4M8 11v3" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(CAMERA, "camera", r#"<path d="M2 5.5A1.5 1.5 0 0 1 3.5 4h1.6l1-1.5h3.8l1 1.5h1.6A1.5 1.5 0 0 1 14 5.5v6a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5Z" stroke="currentColor" stroke-linejoin="round"/><circle cx="8" cy="8.5" r="2.3" stroke="currentColor"/>"#);
glyph!(PRINTER, "printer", r#"<path d="M4.5 6V2h7v4M4.5 11.5h-2v-5.5h11v5.5h-2" stroke="currentColor" stroke-linejoin="round"/><rect x="4.5" y="9.5" width="7" height="4.5" stroke="currentColor"/>"#);
glyph!(PLAYER, "player", r#"<rect x="3.5" y="1.5" width="9" height="13" rx="1.8" stroke="currentColor"/><circle cx="8" cy="10" r="2.3" stroke="currentColor"/><path d="M6 4.5h4" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(NETWORK, "network", r#"<path d="M2 6.5a8.5 8.5 0 0 1 12 0M4.2 9a5.3 5.3 0 0 1 7.6 0" stroke="currentColor" stroke-linecap="round"/><circle cx="8" cy="12" r="1" fill="currentColor"/>"#);

glyph!(CHEVRON, "chevron", r#"<path d="M4.5 6.5L8 10l3.5-3.5" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(CHECK, "check", r#"<path d="M3.5 8.5l3 3 6-7" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(PLUS, "plus", r#"<path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(CLOSE, "close", r#"<path d="M4.5 4.5l7 7M11.5 4.5l-7 7" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(WARNING, "warning", r#"<path d="M8 2.5l6 10.5H2Z" stroke="currentColor" stroke-linejoin="round"/><path d="M8 6.5v3" stroke="currentColor" stroke-linecap="round"/><circle cx="8" cy="11.2" r=".7" fill="currentColor"/>"#);
glyph!(PENCIL, "pencil", r#"<path d="M10.5 2.5l3 3-8 8H2.5v-3Z" stroke="currentColor" stroke-linejoin="round"/>"#);
glyph!(SHIELD, "shield", r#"<path d="M8 1.8l5 2v4c0 3.1-2.1 5.4-5 6.4-2.9-1-5-3.3-5-6.4v-4Z" stroke="currentColor" stroke-linejoin="round"/><path d="M5.8 8l1.6 1.6 3-3.2" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(SOUND, "sound", r#"<path d="M2.5 6h2.2L8 3.2v9.6L4.7 10H2.5Z" stroke="currentColor" stroke-linejoin="round"/><path d="M10.6 5.6a3.4 3.4 0 0 1 0 4.8" stroke="currentColor" stroke-linecap="round"/><path d="M12.4 3.8a6 6 0 0 1 0 8.4" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(ARROW_OUT, "arrow-out", r#"<path d="M6 3.5h6.5V10M12.5 3.5l-9 9" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(TRASH, "trash", r#"<path d="M2.5 4.5h11M6 4.5V3h4v1.5M4 4.5l.7 9h6.6l.7-9" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);

/// The glyph for a kind of device.
pub fn kind(k: Kind) -> &'static Icon {
    match k {
        Kind::Headphones => &HEADPHONES,
        Kind::Headset => &HEADSET,
        Kind::Speaker => &SPEAKER,
        Kind::Phone => &PHONE,
        Kind::Computer => &COMPUTER,
        Kind::Tablet => &TABLET,
        Kind::Keyboard => &KEYBOARD,
        Kind::Mouse => &MOUSE,
        Kind::Gamepad => &GAMEPAD,
        Kind::Pen => &PEN,
        Kind::Watch => &WATCH,
        Kind::Display => &DISPLAY,
        Kind::Camera => &CAMERA,
        Kind::Printer | Kind::Scanner => &PRINTER,
        Kind::Player => &PLAYER,
        Kind::Network => &NETWORK,
        Kind::Other => &BLUETOOTH,
    }
}
