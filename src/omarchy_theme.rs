//! The bundled `omarchy` theme follows Omarchy's current theme. Its source holds placeholders in
//! Omarchy's template syntax (`{{ accent }}`, `{{ mix foreground background 25% }}`,
//! `{{ gradient_start hyprland_active_border accent }}`), filled from the theme's colors.toml
//! each time it loads; the watcher reloads when Omarchy swaps the theme folder. Without Omarchy,
//! Tokyo Night (its default theme) stands in.

use std::collections::HashMap;
use std::path::PathBuf;

/// The bundled theme's id.
pub const ID: &str = "omarchy";

/// Tokyo Night, for machines without Omarchy and for keys a theme leaves out.
const FALLBACK: &[(&str, &str)] = &[
    ("background", "#1a1b26"),
    ("foreground", "#a9b1d6"),
    ("accent", "#7aa2f7"),
    ("red", "#f7768e"),
];

type Rgb = [u8; 3];

/// The current Omarchy theme's palette.
pub fn colors_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".local/state/omarchy/current/theme/colors.toml"))
}

/// Whether this is an Omarchy machine, so the window offers the theme.
pub fn present() -> bool {
    colors_path().is_some_and(|p| p.exists())
}

/// `src` with its placeholders filled from the current Omarchy theme.
pub fn render(src: &str) -> String {
    let toml = colors_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    fill(src, &toml)
}

/// `src` with its placeholders filled from `colors_toml`. One that can't be filled stays, so the
/// theme parser names it.
pub fn fill(src: &str, colors_toml: &str) -> String {
    let palette = palette(colors_toml);
    let color = |key: &str| {
        palette.get(key).and_then(|v| first_color(v)).or_else(|| {
            FALLBACK
                .iter()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| first_color(v))
        })
    };
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else {
            break;
        };
        let words: Vec<&str> = rest[start + 2..start + len].split_whitespace().collect();
        let value = match words.as_slice() {
            [key] => color(key),
            ["mix", a, b, amount] => color(a)
                .zip(color(b))
                .zip(fraction(amount))
                .map(|((a, b), t)| mix(a, b, t)),
            ["gradient_start", key, fallback] => color(key).or_else(|| color(fallback)),
            _ => None,
        };
        out.push_str(&rest[..start]);
        match value {
            Some([r, g, b]) => out.push_str(&format!("#{r:02x}{g:02x}{b:02x}")),
            None => out.push_str(&rest[start..start + len + 2]),
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// colors.toml's `key = "value"` lines. Values stay raw, as a border can be a gradient.
fn palette(toml: &str) -> HashMap<&str, &str> {
    toml.lines()
        .filter_map(|line| {
            let (k, v) = line.split_once('=')?;
            Some((k.trim(), v.trim().strip_prefix('"')?.split('"').next()?))
        })
        .collect()
}

/// The first colour in a value: `#rrggbb`, `#rrggbbaa`, or `rgb(rrggbb)` / `rgba(rrggbbaa)` as
/// Hyprland border gradients write them.
fn first_color(v: &str) -> Option<Rgb> {
    let word = v.split_whitespace().next()?;
    let hex = match word.strip_prefix('#') {
        Some(hex) => hex,
        None => word
            .strip_prefix("rgba(")
            .or_else(|| word.strip_prefix("rgb("))?
            .strip_suffix(')')?,
    };
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let at = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([at(0)?, at(2)?, at(4)?])
}

/// `25%`, or `0.25`, as Omarchy reads a mix amount.
fn fraction(s: &str) -> Option<f32> {
    let t = match s.strip_suffix('%') {
        Some(pct) => pct.parse::<f32>().ok()? / 100.0,
        None => s
            .parse::<f32>()
            .ok()
            .map(|t| if t > 1.0 { t / 100.0 } else { t })?,
    };
    Some(t.clamp(0.0, 1.0))
}

/// `t` of the way from `a` to `b`, rounded as Omarchy rounds.
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let ch = |i: usize| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8;
    [ch(0), ch(1), ch(2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO: &str = "mode = \"dark\"\naccent = \"#7aa2f7\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"\nred = \"#f7768e\"\n";

    #[test]
    fn fills_like_omarchys_own_templates() {
        // Omarchy's renderer gave these for Tokyo Night.
        let src = "{{ background }} {{ mix foreground background 25% }} {{ mix foreground background 50% }} {{ gradient_start hyprland_active_border accent }}";
        assert_eq!(fill(src, TOKYO), "#1a1b26 #858caa #62667e #7aa2f7");
    }

    #[test]
    fn a_border_gradient_gives_its_first_colour() {
        let toml =
            format!("{TOKYO}hyprland_active_border = \"rgba(26a269ee) rgba(2ec27eee) 45deg\"\n");
        assert_eq!(
            fill("{{ gradient_start hyprland_active_border accent }}", &toml),
            "#26a269"
        );
    }

    #[test]
    fn without_omarchy_tokyo_night_stands_in_and_unknowns_stay() {
        assert_eq!(
            fill("fill \"{{ background }}\" {{ nope }}", ""),
            "fill \"#1a1b26\" {{ nope }}"
        );
    }
}
