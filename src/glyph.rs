//! Centring Omarchy's badge glyphs (`omarchy-glyph`). They're Nerd Font icons drawn in the
//! system's monospace font, and in a Nerd Font that isn't "Mono" or "Propo" an icon is wider
//! than the one cell it advances: JetBrainsMono Nerd Font's camera (U+F030) is 923 units of ink
//! on a 600 unit advance. Centring the advance leaves the icon off to the right, so the badge
//! moves it back by how far its ink sits from the advance's middle. Measuring reads the font
//! file, so the bus thread does it when a notification arrives and drawing finds it cached.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};

/// How far `glyph`'s ink sits right of the middle of its advance, as a fraction of the font
/// size; 0 for more than one character, or when the font has no outline for it.
pub fn ink_shift(glyph: &str) -> f32 {
    let mut chars = glyph.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return 0.0;
    };
    static SHIFTS: OnceLock<Mutex<HashMap<char, f32>>> = OnceLock::new();
    let shifts = SHIFTS.get_or_init(Default::default);
    if let Some(s) = shifts.lock().ok().and_then(|m| m.get(&ch).copied()) {
        return s;
    }
    let s = measure(ch).unwrap_or(0.0);
    if let Ok(mut m) = shifts.lock() {
        m.insert(ch, s);
    }
    s
}

fn measure(ch: char) -> Option<f32> {
    let (path, index) = monospace()?;
    let data = std::fs::read(path).ok()?;
    let font = FontRef::from_index(&data, *index).ok()?;
    let id = font.charmap().map(ch)?;
    let glyphs = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let ink = glyphs.bounds(id)?;
    let advance = glyphs.advance_width(id)?;
    let em = font
        .metrics(Size::unscaled(), LocationRef::default())
        .units_per_em;
    (em > 0).then(|| offset(ink.x_min, ink.x_max, advance) / em as f32)
}

/// The middle of the ink less the middle of the advance, in font units.
fn offset(x_min: f32, x_max: f32, advance: f32) -> f32 {
    (x_min + x_max - advance) / 2.0
}

/// The file (and face in it) fontconfig gives `monospace`: the font Rune draws
/// `Family::SystemMono` in, asked the same way.
fn monospace() -> Option<&'static (PathBuf, u32)> {
    static ANSWER: OnceLock<Option<(PathBuf, u32)>> = OnceLock::new();
    ANSWER
        .get_or_init(|| {
            let out = std::process::Command::new("fc-match")
                .args(["--format=%{file}\n%{index}", "monospace"])
                .output()
                .ok()?;
            let out = String::from_utf8(out.stdout).ok()?;
            let (file, index) = out.split_once('\n')?;
            (!file.is_empty()).then(|| (file.into(), index.trim().parse().unwrap_or(0)))
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::{ink_shift, offset};

    #[test]
    fn ink_wider_than_its_advance_sits_right() {
        // JetBrainsMono Nerd Font's camera: ink 0..923 on a 600 advance.
        assert_eq!(offset(0.0, 923.0, 600.0), 161.5);
        assert_eq!(offset(100.0, 500.0, 600.0), 0.0);
        assert_eq!(offset(0.0, 400.0, 600.0), -100.0);
    }

    #[test]
    fn only_single_characters_shift() {
        assert_eq!(ink_shift(""), 0.0);
        assert_eq!(ink_shift("ab"), 0.0);
    }
}
