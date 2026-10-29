//! The window's own line icons, on a 16-point grid in the stroke style of Hokum's settings.

use rune::Icon;

use crate::sound::model::{Bus, Device, DeviceKind};

macro_rules! glyph {
    ($id:ident, $name:literal, $body:literal) => {
        pub static $id: Icon = Icon { name: $name, view: 16.0, body: $body };
    };
}

glyph!(SPEAKER, "speaker", r#"<path d="M2.5 6h2.2L8 3.2v9.6L4.7 10H2.5Z" stroke="currentColor" stroke-linejoin="round"/><path d="M10.6 5.6a3.4 3.4 0 0 1 0 4.8" stroke="currentColor" stroke-linecap="round"/><path d="M12.4 3.8a6 6 0 0 1 0 8.4" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(SPEAKER_LOW, "speaker-low", r#"<path d="M2.5 6h2.2L8 3.2v9.6L4.7 10H2.5Z" stroke="currentColor" stroke-linejoin="round"/><path d="M10.6 5.6a3.4 3.4 0 0 1 0 4.8" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(SPEAKER_MUTED, "speaker-muted", r#"<path d="M2.5 6h2.2L8 3.2v9.6L4.7 10H2.5Z" stroke="currentColor" stroke-linejoin="round"/><path d="M10.5 6l3 4M13.5 6l-3 4" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(MIC, "mic", r#"<rect x="5.75" y="2" width="4.5" height="8" rx="2.25" stroke="currentColor"/><path d="M3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(MIC_MUTED, "mic-muted", r#"<path d="M10.25 7.2V4.25a2.25 2.25 0 0 0-4.4-.7M5.75 6.5v1.25a2.25 2.25 0 0 0 3.6 1.8" stroke="currentColor" stroke-linecap="round"/><path d="M3.5 7.5a4.5 4.5 0 0 0 7.4 3.45M12.5 7.5a4.5 4.5 0 0 1-.3 1.6M8 12v2M2.5 2.5l11 11" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(HEADPHONES, "headphones", r#"<path d="M2.5 10V8a5.5 5.5 0 0 1 11 0v2" stroke="currentColor"/><rect x="2.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/><rect x="10.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/>"#);
glyph!(HEADSET, "headset", r#"<path d="M2.5 10V8a5.5 5.5 0 0 1 11 0v2" stroke="currentColor"/><rect x="2.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/><rect x="10.5" y="9" width="3" height="4.5" rx="1.2" stroke="currentColor"/><path d="M12 13.5c0 .8-1.4 1.5-3.5 1.5" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(DISPLAY, "display", r#"<rect x="1.5" y="2.5" width="13" height="8.5" rx="1.5" stroke="currentColor"/><path d="M6 14h4M8 11v3" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(MONITOR, "monitor", r#"<circle cx="8" cy="8" r="2" stroke="currentColor"/><path d="M4.5 4.5a5 5 0 0 0 0 7M11.5 4.5a5 5 0 0 1 0 7" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(VIRTUAL, "virtual", r#"<path d="M2.5 8h2l1.5-4 2 8 1.5-4h4" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(CHEVRON, "chevron", r#"<path d="M4.5 6.5L8 10l3.5-3.5" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(BACK, "back", r#"<path d="M9.5 4L5.5 8l4 4" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(CHECK, "check", r#"<path d="M3.5 8.5l3 3 6-7" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(TUNE, "tune", r#"<path d="M3 4.5h6M12 4.5h1M3 11.5h1M7 11.5h6" stroke="currentColor" stroke-linecap="round"/><circle cx="10.5" cy="4.5" r="1.5" stroke="currentColor"/><circle cx="5.5" cy="11.5" r="1.5" stroke="currentColor"/>"#);
glyph!(EXPAND, "expand", r#"<path d="M9.5 2.5h4v4M6.5 13.5h-4v-4M13.5 2.5L9 7M2.5 13.5L7 9" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(PLUS, "plus", r#"<path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(CLOSE, "close", r#"<path d="M4.5 4.5l7 7M11.5 4.5l-7 7" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(BLUETOOTH, "bluetooth", r#"<path d="M4.5 5l7 6-3.5 3V2l3.5 3-7 6" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#);
glyph!(JACK, "jack", r#"<path d="M8 2v4M6 6h4v3.5a2 2 0 0 1-4 0Z" stroke="currentColor" stroke-linejoin="round"/><path d="M8 11.5V14" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(BATTERY, "battery", r#"<rect x="1.5" y="5" width="11.5" height="6" rx="1.5" stroke="currentColor"/><path d="M14.5 7v2" stroke="currentColor" stroke-linecap="round"/>"#);
glyph!(WARNING, "warning", r#"<path d="M8 2.5l6 10.5H2Z" stroke="currentColor" stroke-linejoin="round"/><path d="M8 6.5v3" stroke="currentColor" stroke-linecap="round"/><circle cx="8" cy="11.2" r=".7" fill="currentColor"/>"#);

/// The glyph for a device, from its icon name and bus.
pub fn device(d: &Device) -> &'static Icon {
    if d.kind == DeviceKind::Monitor {
        return &MONITOR;
    }
    if d.kind == DeviceKind::Source {
        return &MIC;
    }
    let name = d.icon_name.as_str();
    if name.contains("headset") {
        &HEADSET
    } else if name.contains("headphone") {
        &HEADPHONES
    } else if name.contains("display") || name.contains("video") || d.bus == Bus::Display {
        &DISPLAY
    } else if d.is_virtual {
        &VIRTUAL
    } else {
        &SPEAKER
    }
}

/// The mute button's glyph: the device's own, or the crossed speaker or microphone.
pub fn volume(d: &Device, volume: f32) -> &'static Icon {
    match (d.kind, d.mute) {
        (DeviceKind::Sink, true) => &SPEAKER_MUTED,
        (DeviceKind::Sink, false) if matches!(device(d).name, "speaker") && volume < 0.34 => &SPEAKER_LOW,
        (_, true) => &MIC_MUTED,
        _ => device(d),
    }
}
