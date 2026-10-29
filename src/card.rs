//! One notification and the card that draws it, shared by the overlay and the settings preview.
//! A card is its theme's template over its tokens (`theme.rs`, `template.rs`); this file adds
//! the parts every card has: the surface, the press, the entrance and the exit.

use std::time::{Duration, Instant};

use rune::{Color, Cursor, Edge, El, Image, Shadow, Spring, Transition, col, el, oklch, row};
use rune_kit::Kit;

use crate::icons::Source;
use crate::style::{Mark, Position, Side};
use crate::template::{Ctx, render};
use crate::theme::{Enter, Fill, Lift, Theme};

/// How long a notification stays when the app leaves it to the daemon.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Timeout {
    Default,
    Never,
    After(Duration),
}

pub struct Note {
    pub id: u32,
    /// The card this note draws into. A note that replaces another keeps its card, so the
    /// update lands in place instead of one card leaving and another arriving.
    pub slot: u32,
    pub app: String,
    /// Where in the app it came from (a Slack channel), shown after the app's name.
    pub context: Option<String>,
    pub summary: String,
    pub body: String,
    /// (key, label) without the default action or Chromium's "settings".
    pub actions: Vec<(String, String)>,
    pub has_default: bool,
    /// Programs to run for action keys (`default` for a click), from Omarchy's
    /// `omarchy-exec-argv` and `omarchy-actions` hints.
    pub commands: Vec<(String, Vec<String>)>,
    /// A glyph (Omarchy's `omarchy-glyph`) for the badge, in place of the app's letter.
    pub glyph: Option<String>,
    /// The sender's `desktop-entry` (else the entry Hokum found for it), named when asking for
    /// an activation token.
    pub desktop: Option<String>,
    /// The desktop entry Hokum found for the app (`crate::entries`): a web app's own, not its
    /// browser's.
    pub entry: Option<String>,
    /// 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    pub value: Option<f32>,
    pub tag: Option<String>,
    pub resident: bool,
    pub timeout: Timeout,
    pub time: crate::clock::Stamp,
    /// A template a script picked, by name.
    pub template: Option<String>,
    /// Where the app's icon and the notification's picture come from; decoded on the UI
    /// thread into `icon` and `image`.
    pub icon_src: Option<Source>,
    pub image_src: Option<Source>,
    pub icon: Option<Image>,
    pub image: Option<Image>,
    pub deadline: Option<Instant>,
    /// Time left while timers are paused.
    pub left: Option<Duration>,
    /// Hokum's own feedback (the volume card): shown through Do not disturb.
    pub hud: bool,
    /// The sender's sound hints.
    pub sound: crate::chime::Hint,
}

impl Note {
    pub fn new(
        id: u32,
        app: impl Into<String>,
        summary: impl Into<String>,
        body: impl Into<String>,
    ) -> Note {
        Note {
            id,
            slot: id,
            app: app.into(),
            context: None,
            summary: summary.into(),
            body: body.into(),
            actions: Vec::new(),
            has_default: false,
            commands: Vec::new(),
            glyph: None,
            desktop: None,
            urgency: 1,
            value: None,
            tag: None,
            resident: false,
            timeout: Timeout::Default,
            time: crate::clock::Stamp::now(),
            template: None,
            entry: None,
            icon_src: None,
            image_src: None,
            icon: None,
            image: None,
            deadline: None,
            left: None,
            hud: false,
            sound: crate::chime::Hint::None,
        }
    }

    /// The program to run for action `key`, if the notification carries one.
    pub fn command(&self, key: &str) -> Option<&[String]> {
        self.commands
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, argv)| argv.as_slice())
    }

    /// Decodes the icon and picture with `cache` (the decode itself runs on the cache's worker).
    pub fn decode(&mut self, cache: &rune::ImageCache) {
        let load = |src: &Option<Source>| -> Option<Image> {
            Some(match src.as_ref()? {
                Source::File(path) => cache.file(path, 64.0, 2.0),
                Source::Rgba {
                    width,
                    height,
                    bytes,
                } => cache.rgba(*width, *height, bytes.clone()),
            })
        };
        self.icon = load(&self.icon_src);
        self.image = load(&self.image_src);
    }
}

/// What a card's handlers do in whichever state draws it.
pub trait Cards: 'static {
    fn dismiss(&mut self, id: u32);
    fn invoke(&mut self, id: u32, key: &str);
    /// A click on the card itself.
    fn activate(&mut self, id: u32);
    /// The pointer entered or left the stack (cards and the gaps between them).
    fn hover(&mut self, on: bool);
    /// Card `id` was laid out `height` points tall.
    fn measured(&mut self, _id: u32, _height: f32) {}
}

/// A stable colour per app, so the same sender always reads the same at a glance.
pub fn tint(app: &str) -> Color {
    let h = app
        .bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    oklch(0.64, 0.13, (h % 360) as f32)
}

/// The shadows a card casts for `lift`.
pub fn shadows(lift: Lift, k: &Kit, accent: Color) -> Vec<Shadow> {
    match lift {
        Lift::Flat => Vec::new(),
        Lift::Soft | Lift::Deep => {
            let mut v = k.elevation.overlay.clone();
            if lift == Lift::Deep {
                let a = if k.dark { 0.45 } else { 0.16 };
                v.push(Shadow {
                    y: 14.0,
                    blur: 36.0,
                    color: Color::BLACK.alpha(a),
                    ..Default::default()
                });
            }
            v
        }
        Lift::Glow => vec![
            Shadow {
                blur: 28.0,
                color: accent.alpha(0.45),
                ..Default::default()
            },
            Shadow {
                blur: 6.0,
                color: accent.alpha(0.35),
                ..Default::default()
            },
        ],
    }
}

/// How far the shadows `theme` can give a card reach past its sides, in either appearance: the
/// Gaussian fades out by three standard deviations (1.5 × blur), plus sideways offset and spread.
pub fn shadow_reach(theme: &Theme) -> f32 {
    [false, true]
        .into_iter()
        .flat_map(|dark| {
            let k = rune_theme_opencode::kit(dark);
            theme
                .lifts(dark)
                .into_iter()
                .flat_map(move |l| shadows(l, k, Color::BLACK))
        })
        .filter(|sh| !sh.inset)
        .map(|sh| sh.x.abs() + sh.spread.max(0.0) + sh.blur * 1.5)
        .fold(0.0, f32::max)
        .ceil()
}

/// The card for `n` in `theme` at `position`, which picks the edge it slides in from: the side
/// of the screen for a corner, the top or bottom edge for a centered stack.
pub fn card<S: Cards>(n: &Note, theme: &Theme, k: &Kit, position: Position) -> El {
    let look = theme.look(n, k.dark);
    let t = &look.tokens;
    let tint = tint(&n.app);
    let accent = t.accent.resolve(k, tint);
    let cx = Ctx { n, t, k, accent };
    let id = n.id;
    let slot = n.slot;
    let [pl, pt, pr, pb] = t.padding;
    let mut root = col()
        .key(format!("note-{slot}"))
        .role("status")
        .label(n.summary.clone())
        .w_full()
        .pl(pl)
        .pt(pt)
        .pr(pr)
        .pb(pb)
        .gap(t.gap)
        .radius(t.radius)
        .hover(k.alpha(k.c.white, if k.dark { 0.02 } else { 0.0 }))
        .cursor(Cursor::Pointer)
        // The overlay takes the pointer only over cards; everywhere else clicks pass through.
        .hit()
        .on_press(move |s: &mut S| s.activate(id))
        // A right-click dismisses, as Omarchy's and swaync's cards do.
        .on_secondary_press(move |s: &mut S, _, _| s.dismiss(id));
    root = match &t.fill {
        Fill::Solid(c) => root.fill(cx.paint(c)),
        Fill::Gradient(a, b) => root.gradient(cx.paint(a), cx.paint(b)),
    };
    if let Some((w, c)) = &t.border {
        root = root.border(*w, cx.paint(c));
    }
    for sh in shadows(t.shadow, k, accent) {
        root = root.box_shadow(sh);
    }
    let content = render::<S>(&cx, &look.template);
    root = if t.mark == Mark::Stripe {
        // The stripe stretches with the content, inset from the card's edge.
        root.pl((pl - 4.0).max(4.0)).child(
            row()
                .key("ribbon")
                .w_full()
                .gap(9.0)
                .items_stretch()
                .child(el().key("stripe").w(3.0).radius(1.5).fill(accent))
                .child(
                    col()
                        .key("main")
                        .grow(1.0)
                        .shrink()
                        .w(0.0)
                        .gap(t.gap)
                        .children(content),
                ),
        )
    } else {
        root.children(content)
    };
    let (enter, exit) = match t.enter {
        Enter::Slide => slide(position),
        Enter::Fade => (Transition::opacity(), Transition::opacity()),
        Enter::Scale => (Transition::scale(0.9), Transition::scale(0.95)),
        Enter::Drop => (
            Transition::offset(0.0, -24.0).combined(Transition::opacity()),
            Transition::offset(0.0, -12.0).combined(Transition::opacity()),
        ),
    };
    // Still cards land at once through the stack's `instant_changes`.
    if t.still() {
        return root;
    }
    root.transition(Transition::asymmetric(enter, exit))
        .motion(Spring::new(t.motion.0, t.motion.1))
}

/// A card sliding in from, and gliding out toward, its side of the screen. Centered stacks
/// move vertically from their edge and leave by a short glide rather than a full edge move,
/// which would sweep across the cards next to it.
fn slide(position: Position) -> (Transition, Transition) {
    let enter = |dx: f32, dy: f32| {
        Transition::offset(dx, dy)
            .combined(Transition::opacity())
            .combined(Transition::scale(0.97))
    };
    let edge = |e: Edge| Transition::move_edge(e).combined(Transition::opacity());
    let up = if position.bottom() { 1.0 } else { -1.0 };
    match position.side() {
        Side::Right => (enter(48.0, 0.0), edge(Edge::Trailing)),
        Side::Left => (enter(-48.0, 0.0), edge(Edge::Leading)),
        Side::Center => (enter(0.0, 40.0 * up), enter(0.0, 24.0 * up)),
    }
}

/// Notes that show off every part of a card, for previews and snapshots.
pub fn samples() -> Vec<Note> {
    let mut a = Note::new(
        1,
        "Slack",
        "Dax",
        "hokum looks sick 👋 ship it before anyone sees the mako ones",
    );
    a.context = Some("#abattoir".into());
    a.actions = vec![
        ("reply".into(), "Reply".into()),
        ("later".into(), "Later".into()),
    ];
    a.has_default = true;
    a.icon_src = crate::icons::for_app("", Some("slack"), "Slack").map(Source::File);
    let mut b = Note::new(2, "Volume", "Speakers", "");
    b.value = Some(0.62);
    b.tag = Some("volume".into());
    b.icon_src = crate::icons::icon("audio-volume-high").map(Source::File);
    let mut c = Note::new(
        3,
        "Firefox",
        "Download complete",
        "hokum-x86_64.tar.zst · 4.2 MB",
    );
    c.icon_src = crate::icons::for_app("", Some("firefox"), "Firefox").map(Source::File);
    let mut d = Note::new(
        4,
        "Battery",
        "Battery critically low",
        "4% remaining. Plug in now or the session will suspend.",
    );
    d.urgency = 2;
    d.timeout = Timeout::Never;
    vec![a, d, b, c]
}
