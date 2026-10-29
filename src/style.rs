//! The behaviour every theme and config resolves to, where files live, and the vocabulary of
//! settings.json (from before config.kdl), read once to migrate.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corners {
    Sharp,
    Rounded,
    Soft,
    Round,
}

impl Corners {
    /// (radius, badge-radius)
    pub fn radii(self) -> (f32, f32) {
        match self {
            Corners::Sharp => (4.0, 4.0),
            Corners::Rounded => (10.0, 6.0),
            Corners::Soft => (16.0, 7.0),
            Corners::Round => (24.0, 10.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Density {
    Compact,
    Comfortable,
    Spacious,
}

impl Density {
    /// (padding, gap inside a card, spacing between cards)
    pub fn spacing(self) -> (f32, f32, f32) {
        match self {
            Density::Compact => (10.0, 4.0, 8.0),
            Density::Comfortable => (12.0, 6.0, 10.0),
            Density::Spacious => (16.0, 8.0, 12.0),
        }
    }
}

/// How a card says which app sent it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mark {
    Letter,
    /// The app's icon from the icon theme, or its letter when it has none.
    Icon,
    Dot,
    Stripe,
    None,
}

impl Mark {
    pub fn word(self) -> &'static str {
        match self {
            Mark::Letter => "letter",
            Mark::Icon => "icon",
            Mark::Dot => "dot",
            Mark::Stripe => "stripe",
            Mark::None => "none",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Depth {
    Flat,
    Soft,
    Deep,
}

impl Depth {
    pub fn word(self) -> &'static str {
        match self {
            Depth::Flat => "flat",
            Depth::Soft => "soft",
            Depth::Deep => "deep",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Width {
    Narrow,
    Medium,
    Wide,
}

impl Width {
    pub fn points(self) -> f32 {
        match self {
            Width::Narrow => 320.0,
            Width::Medium => 360.0,
            Width::Wide => 420.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    TopRight,
    TopLeft,
    TopCenter,
    BottomRight,
    BottomLeft,
    BottomCenter,
}

/// Where a position sits across the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Center,
    Right,
}

impl Position {
    pub const ALL: [Position; 6] = [
        Position::TopLeft,
        Position::TopCenter,
        Position::TopRight,
        Position::BottomLeft,
        Position::BottomCenter,
        Position::BottomRight,
    ];

    pub fn side(self) -> Side {
        match self {
            Position::TopLeft | Position::BottomLeft => Side::Left,
            Position::TopCenter | Position::BottomCenter => Side::Center,
            Position::TopRight | Position::BottomRight => Side::Right,
        }
    }
    pub fn bottom(self) -> bool {
        matches!(
            self,
            Position::BottomRight | Position::BottomLeft | Position::BottomCenter
        )
    }
    pub fn word(self) -> &'static str {
        match self {
            Position::TopRight => "top-right",
            Position::TopLeft => "top-left",
            Position::TopCenter => "top-center",
            Position::BottomRight => "bottom-right",
            Position::BottomLeft => "bottom-left",
            Position::BottomCenter => "bottom-center",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn word(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }
}

/// How notifications come and go, resolved from the theme and the config.
#[derive(Clone, PartialEq, Debug)]
pub struct Behaviour {
    pub position: Position,
    pub appearance: Appearance,
    /// Seconds ordinary notifications stay when the app doesn't say.
    pub stay: f32,
    pub max_shown: usize,
    pub pause_on_hover: bool,
    /// Only critical notifications (and apps allowed to interrupt) show.
    pub do_not_disturb: bool,
    /// Fetch the release manifest at startup and hourly, and log when a newer version is out.
    pub check_for_updates: bool,
    /// `{time}`'s format: the desktop's (`auto`), 12-hour or 24-hour.
    pub clock: crate::clock::Clock,
}

impl Default for Behaviour {
    fn default() -> Self {
        Behaviour {
            position: Position::TopRight,
            appearance: Appearance::System,
            stay: 6.0,
            max_shown: 5,
            pause_on_hover: true,
            do_not_disturb: false,
            check_for_updates: true,
            clock: crate::clock::Clock::Auto,
        }
    }
}

impl Behaviour {
    pub fn stay(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(self.stay.max(0.5))
    }
}

/// `$XDG_CONFIG_HOME/hokum/`, the folder for the config, themes and script.
pub fn dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hokum")
}

/// `$XDG_STATE_HOME/hokum/`, for what Hokum remembers on its own.
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hokum")
}

/// A file in Hokum's folder (`themes/` and `script.rhai` sit beside the config).
pub fn path() -> PathBuf {
    dir().join("config.kdl")
}

/// The settings file before config.kdl, read once to migrate.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Legacy {
    pub style: Option<LegacyStyle>,
    pub position: Option<Position>,
    pub appearance: Option<Appearance>,
    pub linger: Option<String>,
    pub max_shown: Option<usize>,
    pub pause_on_hover: Option<bool>,
    pub do_not_disturb: Option<bool>,
    pub theme: Option<String>,
}

#[derive(Deserialize, Clone, Copy)]
pub struct LegacyStyle {
    pub corners: Corners,
    pub density: Density,
    pub mark: Mark,
    pub depth: Depth,
    pub width: Width,
    pub border: bool,
}
