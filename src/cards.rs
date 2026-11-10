//! The notification stack: an overlay (a click-through layer surface on Wayland) with its own
//! state. It shows for the first card, so it opens on the output focused then, and hides once
//! the last card has left.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use rune::{
    Align, AppHandle, AppSpec, Cursor, Cx, El, OverlayHandle, OverlayInput, OverlayOptions,
    OverlayPlacement, OverlaySpec, ScreenEdge, Transition, Tx, col, el, row, text,
};
use rune_kit::Kit;

use crate::card::{Cards, Note, Timeout, card, samples};
use crate::script::Script;
use crate::style::{Appearance, Behaviour, Position, Side};
use crate::theme::Theme;
use crate::watch::Config;

/// The gap between the stack and the screen's edges.
pub const GUTTER: f32 = 18.0;
/// The overlay's height: room for a full stack, and short enough for small screens.
const HEIGHT: f32 = 820.0;
/// Long enough for the last card's exit to finish before the overlay hides.
const HIDE_AFTER: Duration = Duration::from_millis(900);

/// Why a notification closed (the spec's reasons).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

#[derive(Debug, PartialEq)]
pub enum Out {
    Closed(u32, Reason),
    /// An action key, with an activation token from the click when the compositor gave one.
    Action(u32, String, Option<String>),
    /// Do not disturb turned on or off.
    Dnd(bool),
}

pub struct Ui {
    /// Newest first.
    notes: Vec<Note>,
    paused: bool,
    pub settings: Behaviour,
    pub theme: Theme,
    script: Option<Script>,
    /// Decodes icons and pictures; made with the overlay's waker at startup.
    cache: Option<rune::ImageCache>,
    /// The desktop's colour scheme (`Appearance::System`).
    pub system_dark: bool,
    out: Sender<Out>,
    timer: Sender<Option<Instant>>,
    /// Its own overlay, to show and hide; None when headless.
    pub handle: Option<OverlayHandle<Ui>>,
    /// The settings window, which Hokum's own cards open; None when headless.
    pub window: Option<AppHandle<crate::settings::Ui>>,
    shown: bool,
    hide_at: Option<Instant>,
    /// Each card's height as last laid out, to show only the cards that fit.
    heights: HashMap<u32, f32>,
    /// The overlay's height: `HEIGHT`, or taller to hold a card that is.
    overlay_h: f32,
    /// Plays notification sounds; None when headless.
    pub chime: Option<Sender<crate::chime::Pick>>,
}

impl Ui {
    pub fn new(config: Config, out: Sender<Out>, timer: Sender<Option<Instant>>) -> Ui {
        let mut ui = Ui {
            notes: Vec::new(),
            paused: false,
            settings: config.loaded.behaviour.clone(),
            theme: config.loaded.theme.clone(),
            script: None,
            cache: None,
            system_dark: true,
            out,
            timer,
            handle: None,
            window: None,
            shown: false,
            hide_at: None,
            chime: None,
            heights: HashMap::new(),
            overlay_h: HEIGHT,
        };
        ui.configure(config);
        ui
    }

    /// The config, its theme and the script from files (at startup, and whenever they change).
    /// A config with problems keeps the last good look.
    pub fn configure(&mut self, config: Config) {
        if !config.loaded.ok() {
            for p in &config.loaded.problems {
                eprintln!("hokum: config.kdl: {p}");
            }
        }
        if config.loaded.ok() || self.theme.rules.is_empty() && self.theme.templates.is_empty() {
            self.theme = config.loaded.theme.clone();
        }
        self.script = config
            .script
            .as_deref()
            .and_then(|src| match Script::compile(src) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("hokum: script.rhai: {e}");
                    None
                }
            });
        let behaviour = if config.loaded.ok() {
            config.loaded.behaviour
        } else {
            self.settings.clone()
        };
        self.apply(behaviour);
    }

    pub fn dark(&self) -> bool {
        match self.settings.appearance {
            Appearance::System => self.system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// New behaviour: the overlay moves and resizes if it must, and cards restyle in place.
    pub fn apply(&mut self, settings: Behaviour) {
        if let Some(h) = &self.handle {
            h.set_placement(Some(placement(&settings)));
            h.resize((
                size(&self.theme, self.dark(), settings.position).0,
                self.overlay_h,
            ));
        }
        if !settings.pause_on_hover {
            self.hover(false);
        }
        if settings.do_not_disturb != self.settings.do_not_disturb {
            let _ = self.out.send(Out::Dnd(settings.do_not_disturb));
        }
        self.settings = settings;
    }

    /// Shows `n`, replacing the card with the same id or stack tag in place.
    pub fn upsert(&mut self, mut n: Note) {
        if let Some(script) = &self.script {
            match script.apply(&mut n) {
                Ok(true) => {}
                Ok(false) => {
                    let _ = self.out.send(Out::Closed(n.id, Reason::Expired));
                    return;
                }
                Err(e) => eprintln!("hokum: script.rhai: {e}"),
            }
        }
        if n.id != crate::welcome::ID && n.id != crate::update::ID {
            crate::apps::record(&n.app, n.entry.as_deref());
        }
        let routing = self.theme.routing(&n);
        if routing.hide {
            let _ = self.out.send(Out::Closed(n.id, Reason::Expired));
            return;
        }
        if let Some(t) = routing.timeout {
            n.timeout = t;
        }
        if let Some(cache) = &self.cache {
            n.decode(cache);
        }
        if self.settings.do_not_disturb && n.urgency < 2 && !routing.interrupt && !n.hud {
            let _ = self.out.send(Out::Closed(n.id, Reason::Expired));
            return;
        }
        let base = self.settings.stay();
        let timeout = match n.timeout {
            Timeout::Never => None,
            Timeout::After(d) => Some(d),
            Timeout::Default if n.urgency >= 2 => None,
            Timeout::Default if n.urgency == 0 => Some(base * 2 / 3),
            // Cards with buttons stay long enough to reach them.
            Timeout::Default if !n.actions.is_empty() => Some(base.max(Duration::from_secs(10))),
            Timeout::Default => Some(base),
        };
        match (self.paused, timeout) {
            (_, None) => {}
            (true, Some(t)) => n.left = Some(t),
            (false, Some(t)) => n.deadline = Some(Instant::now() + t),
        }
        let same = |o: &Note| o.id == n.id || (n.tag.is_some() && o.tag == n.tag && o.app == n.app);
        let replaces = self.notes.iter().position(same);
        // Only new cards chime: not updates in place (progress, a tagged volume card), and never
        // Hokum's own cards.
        let own = n.hud || n.id == crate::welcome::ID || n.id == crate::update::ID;
        if replaces.is_none()
            && !own
            && let Some(chime) = &self.chime
            && let Some(pick) =
                crate::chime::choose(&self.settings, routing.sound.as_ref(), &n.sound, n.urgency)
        {
            let _ = chime.send(pick);
        }
        if let Some(i) = replaces {
            let old = self.notes[i].id;
            n.slot = self.notes[i].slot;
            if old != n.id {
                let _ = self.out.send(Out::Closed(old, Reason::Closed));
            }
            self.notes[i] = n;
        } else {
            // An id reused after its card was taken over by a tagged update needs a card of its own.
            if self.notes.iter().any(|o| o.slot == n.slot) {
                n.slot = u32::MAX - n.id;
            }
            self.notes.insert(0, n);
        }
        if !self.shown
            && let Some(h) = &self.handle
        {
            h.show();
            self.shown = true;
        }
        self.reschedule();
    }

    pub fn showing(&self, id: u32) -> bool {
        self.notes.iter().any(|n| n.id == id)
    }

    pub fn close(&mut self, id: u32, reason: Reason) {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        if self.heights.remove(&id).is_some() {
            self.fit_overlay();
        }
        if self.notes.len() != before {
            if id == crate::update::ID {
                crate::update::closed(reason);
            }
            let _ = self.out.send(Out::Closed(id, reason));
            self.reschedule();
        }
    }

    /// Grows the overlay to hold a card taller than it (very large text, many buttons), and
    /// back to `HEIGHT` when none is, so a card is never cut off.
    fn fit_overlay(&mut self) {
        let shown = self.notes.iter().take(self.settings.max_shown.max(1));
        let tallest = shown
            .filter_map(|n| self.heights.get(&n.id))
            .fold(0.0f32, |a, b| a.max(*b));
        let want = HEIGHT.max((tallest + 2.0 * GUTTER).ceil());
        if (want - self.overlay_h).abs() >= 1.0 {
            self.overlay_h = want;
            if let Some(h) = &self.handle {
                h.resize((
                    size(&self.theme, self.dark(), self.settings.position).0,
                    want,
                ));
            }
        }
    }

    fn clear(&mut self) {
        for n in self.notes.drain(..) {
            if n.id == crate::update::ID {
                crate::update::closed(Reason::Dismissed);
            }
            let _ = self.out.send(Out::Closed(n.id, Reason::Dismissed));
        }
        self.reschedule();
    }

    /// `hokum dismiss [--all]`: closes the newest card, or every card. Returns how many closed.
    pub fn dismiss_newest(&mut self, all: bool) -> u32 {
        let n = if all {
            self.notes.len()
        } else {
            self.notes.len().min(1)
        };
        match (all, self.notes.first().map(|n| n.id)) {
            (true, _) => self.clear(),
            (false, Some(id)) => self.dismiss(id),
            (false, None) => {}
        }
        n as u32
    }

    /// `hokum invoke`: does what clicking the newest card does. False without a card.
    pub fn activate_newest(&mut self, token: Option<String>) -> bool {
        let Some(id) = self.notes.first().map(|n| n.id) else {
            return false;
        };
        self.activate_with(id, token);
        true
    }

    fn activate_with(&mut self, id: u32, token: Option<String>) {
        let default = self
            .notes
            .iter()
            .find(|n| n.id == id)
            .is_some_and(|n| n.has_default || n.command("default").is_some());
        if default {
            self.invoke_with(id, "default", token)
        } else {
            self.dismiss(id)
        }
    }

    fn invoke_with(&mut self, id: u32, key: &str, token: Option<String>) {
        let note = self.notes.iter().find(|n| n.id == id);
        let resident = note.is_some_and(|n| n.resident);
        if id == crate::welcome::ID {
            match (&self.window, token) {
                (Some(w), Some(t)) => w.activate(t),
                (Some(w), None) => w.open(),
                (None, _) => {}
            }
            self.dismiss(id);
            return;
        }
        if id == crate::update::ID {
            if crate::update::click(self.handle.clone(), token) {
                self.dismiss(id);
            }
            return;
        }
        if let Some(argv) = note.and_then(|n| n.command(key)) {
            run(argv, token.as_deref());
        }
        let _ = self.out.send(Out::Action(id, key.to_owned(), token));
        if !resident {
            self.dismiss(id);
        }
    }

    fn expire(&mut self, now: Instant) {
        let gone: Vec<u32> = self
            .notes
            .iter()
            .filter(|n| n.deadline.is_some_and(|d| d <= now))
            .map(|n| n.id)
            .collect();
        for id in gone {
            self.close(id, Reason::Expired);
        }
        if self.notes.is_empty() && self.shown && self.hide_at.is_some_and(|t| t <= now) {
            if let Some(h) = &self.handle {
                h.hide();
            }
            self.shown = false;
            self.paused = false;
        }
        self.reschedule();
    }

    fn reschedule(&mut self) {
        self.hide_at = match (self.notes.is_empty() && self.shown, self.hide_at) {
            (false, _) => None,
            (true, Some(t)) => Some(t),
            (true, None) => Some(Instant::now() + HIDE_AFTER),
        };
        let _ = self.timer.send(
            self.notes
                .iter()
                .filter_map(|n| n.deadline)
                .chain(self.hide_at)
                .min(),
        );
    }
}

impl Cards for Ui {
    fn dismiss(&mut self, id: u32) {
        self.close(id, Reason::Dismissed);
    }

    fn measured(&mut self, id: u32, height: f32) {
        if self
            .heights
            .get(&id)
            .is_some_and(|h| (h - height).abs() < 0.5)
        {
            return;
        }
        self.heights.insert(id, height);
        self.fit_overlay();
    }

    fn invoke(&mut self, id: u32, key: &str) {
        let desktop = self
            .notes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.desktop.as_deref());
        let token = rune::press_activation_token(desktop);
        self.invoke_with(id, key, token);
    }

    fn activate(&mut self, id: u32) {
        let desktop = self
            .notes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.desktop.as_deref());
        let token = rune::press_activation_token(desktop);
        self.activate_with(id, token);
    }

    /// Over the stack: pause every timer. Off it: resume where they were.
    fn hover(&mut self, on: bool) {
        let on = on && self.settings.pause_on_hover;
        if on == self.paused {
            return;
        }
        self.paused = on;
        let now = Instant::now();
        for n in &mut self.notes {
            if on {
                n.left = n.deadline.take().map(|d| d.saturating_duration_since(now));
            } else {
                n.deadline = n
                    .left
                    .take()
                    .map(|l| now + l.max(Duration::from_millis(1200)));
            }
        }
        self.reschedule();
    }
}

/// Runs a notification's own command (Omarchy's hints) directly, with no shell, detached from
/// Hokum's terminal and process group, and reaps it when it exits.
fn run(argv: &[String], token: Option<&str>) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if let Some(t) = token {
        cmd.env("XDG_ACTIVATION_TOKEN", t);
    }
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("hokum: couldn't run {}: {e}", argv[0]),
    }
}

/// The stack sits `GUTTER` from the screen's edge; the side away from it gets room for the
/// theme's shadow, so the shadow isn't cut off at the surface's edge. A centered stack gets
/// that room on both sides. The room is click-through: only cards take input.
fn size(theme: &Theme, dark: bool, position: Position) -> (f32, f32) {
    let room = crate::card::shadow_reach(theme).max(GUTTER);
    let sides = match position.side() {
        Side::Center => 2.0 * room,
        Side::Left | Side::Right => GUTTER + room,
    };
    (theme.base(dark).width + sides, HEIGHT)
}

/// The corner, or the middle of the top or bottom edge, of the screen for `s`.
fn placement(s: &Behaviour) -> OverlayPlacement {
    OverlayPlacement::Edge {
        edge: if s.position.bottom() {
            ScreenEdge::Bottom
        } else {
            ScreenEdge::Top
        },
        margin: 0.0,
        align: match s.position.side() {
            Side::Left => Align::Start,
            Side::Center => Align::Center,
            Side::Right => Align::End,
        },
        inset: 0.0,
    }
}

/// How many of `notes` (newest first) to show: at most `max_shown`, and only as many as fit in
/// `room` points with `spacing` between them and, when some must wait, the `pill` under them.
/// A card not laid out yet counts as fitting; the first always shows.
pub fn fitting(
    notes: &[Note],
    max_shown: usize,
    heights: &HashMap<u32, f32>,
    spacing: f32,
    room: f32,
    pill: f32,
) -> usize {
    let take = |room: f32| {
        let (mut used, mut n) = (0.0, 0);
        for note in notes.iter().take(max_shown.max(1)) {
            let add =
                heights.get(&note.id).copied().unwrap_or(0.0) + if n == 0 { 0.0 } else { spacing };
            if n > 0 && used + add > room {
                break;
            }
            used += add;
            n += 1;
        }
        n
    };
    let n = take(room);
    if n < notes.len() {
        take(room - pill - spacing).max(1)
    } else {
        n
    }
}

/// The stack of the first `limit` of `notes` in a corner (or the middle of an edge) of its
/// parent, as the overlay and the settings preview both draw it. Top stacks grow downward,
/// bottom ones upward.
pub fn stack<S: Cards>(
    notes: &[Note],
    limit: usize,
    s: &Behaviour,
    theme: &Theme,
    k: &Kit,
    more: Option<El>,
) -> El {
    let base = theme.base(k.dark);
    let between = base.spacing;
    let side = s.position.side();
    let bottom = s.position.bottom();
    let shown = notes.iter().take(limit.max(1)).map(|n| {
        let id = n.id;
        card::<S>(n, theme, k, s.position)
            .on_layout(move |st: &mut S, r: [f32; 4]| st.measured(id, r[3]))
    });
    let cards: Vec<El> = if bottom {
        shown.rev().collect()
    } else {
        shown.collect()
    };
    let (t, b) = if bottom {
        (None, Some(GUTTER))
    } else {
        (Some(GUTTER), None)
    };
    let mut col = col()
        .key("stack")
        .w(base.width)
        .gap(between)
        .on_hover_change(|s: &mut S, inside, _: &mut Tx| s.hover(inside));
    col = match side {
        Side::Left => col.pin(Some(GUTTER), t, None, b),
        Side::Right => col.pin(None, t, Some(GUTTER), b),
        Side::Center => col,
    };
    if base.still() {
        col = col.instant_changes();
    }
    let more = more.map(|m| {
        let row = row().key("more-row").w_full().child(m);
        match side {
            Side::Left => row,
            Side::Center => row.justify_center(),
            Side::Right => row.justify_end(),
        }
    });
    if bottom {
        col = col.children(more).children(cards);
    } else {
        col = col.children(cards).children(more);
    }
    match side {
        Side::Center => row()
            .key("middle")
            .pin(Some(0.0), t, Some(0.0), b)
            .justify_center()
            .child(col),
        Side::Left | Side::Right => col,
    }
}

fn more(hidden: usize, k: &Kit) -> El {
    let c = &k.c;
    // It sits under the cards, so it grows with the desktop's text as they do.
    let scale = crate::textscale::get();
    let mut font = k.ty.small_medium;
    font.size *= scale;
    font.line_height *= scale;
    row()
        .key("more")
        .role("button")
        .label(crate::i18n::t("Clear all"))
        .h((28.0 * scale).round())
        .px(12.0 * scale)
        .gap(8.0 * scale)
        .center_y()
        .radius(999.0)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .hover(c.hover)
        .press(c.pressed)
        .cursor(Cursor::Pointer)
        .hit()
        .on_press(|s: &mut Ui| s.clear())
        .child(
            text(crate::i18n::more_hidden(hidden))
                .key("count")
                .font(font)
                .color(c.text_muted),
        )
        .child(
            text(crate::i18n::t("Clear all"))
                .key("clear")
                .font(font)
                .color(c.text_base),
        )
        .transition(Transition::opacity().combined(Transition::scale(0.9)))
}

pub fn view(s: &Ui, _: &mut Cx<Ui>) -> El {
    let k = rune_theme_opencode::kit(s.dark());
    let pill_h = (28.0 * crate::textscale::get()).round();
    let spacing = s.theme.base(s.dark()).spacing;
    let shown = fitting(
        &s.notes,
        s.settings.max_shown,
        &s.heights,
        spacing,
        // Below the gutter the stack may reach the overlay's edge, as it always could.
        s.overlay_h - GUTTER,
        pill_h,
    );
    let hidden = s.notes.len().saturating_sub(shown);
    let pill = (hidden > 0).then(|| more(hidden, k));
    el().key("root").w_full().h_full().child(stack::<Ui>(
        &s.notes,
        shown,
        &s.settings,
        &s.theme,
        k,
        pill,
    ))
}

pub fn overlay(config: &Config) -> rune::ConfiguredOverlay<Ui> {
    OverlaySpec::new(
        "hokum",
        size(&config.loaded.theme, true, config.loaded.behaviour.position),
        view,
    )
    .options(OverlayOptions {
        placement: Some(placement(&config.loaded.behaviour)),
        input: OverlayInput::Regions,
        ..OverlayOptions::default()
    })
    .setup(|ui, waker| ui.cache = Some(rune::ImageCache::new(waker)))
    // The overlay hides while empty, so the first card of a burst arrives in the update that
    // shows it; without this, showing would land that card at rest.
    .animate_reveal()
}

/// Sleeps until the earliest deadline, then expires what's due; each reschedule replaces it.
pub fn timers(rx: Receiver<Option<Instant>>, handle: OverlayHandle<Ui>) {
    std::thread::spawn(move || {
        let mut next: Option<Instant> = None;
        loop {
            let msg = match next {
                Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(t) => next = t,
                Err(RecvTimeoutError::Timeout) => {
                    next = None;
                    handle.update(|s, _| s.expire(Instant::now()));
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

/// Headless runs have no event loop to finish decodes, so decode and wait here.
fn decoded(mut n: Note) -> Note {
    let cache = rune::ImageCache::new(std::sync::Arc::new(|| {}));
    n.decode(&cache);
    for img in n.icon.iter().chain(&n.image) {
        let _ = img.wait();
    }
    n
}

fn demo(s: &mut Ui) {
    for n in samples().into_iter().rev() {
        s.upsert(decoded(n));
    }
}

fn welcome(s: &mut Ui) {
    s.upsert(decoded(crate::welcome::note()));
}

fn update(s: &mut Ui) {
    s.upsert(decoded(crate::update::sample()));
}

/// The volume card, as a change from outside Hokum shows it.
fn osd(s: &mut Ui) {
    s.upsert(decoded(crate::integrations::samples().remove(0)));
}

/// The brightness card, as a change from outside Hokum shows it.
fn brightness(s: &mut Ui) {
    s.upsert(decoded(crate::integrations::brightness_note(0.55)));
}

/// A Bluetooth headset connecting, and a mouse running low.
fn bt_connected(s: &mut Ui) {
    for n in crate::integrations::samples().into_iter().skip(1).rev() {
        s.upsert(decoded(n));
    }
}

/// Omarchy's glyph badges (`omarchy-glyph`): Nerd Font icons, centred in the badge.
fn glyphs(s: &mut Ui) {
    let cards = [
        (
            11,
            "Screenshot saved and copied",
            "919 × 642 · 217.7 kB",
            "\u{f030}",
        ),
        (12, "Screen recording saved", "00:42 · 8.1 MB", "\u{f0100}"),
        (13, "Animations disabled", "", "\u{f16a1}"),
    ];
    for (id, summary, body, glyph) in cards.into_iter().rev() {
        let mut n = Note::new(id, "Omarchy", summary, body);
        n.glyph = Some(glyph.into());
        s.upsert(n);
    }
}

/// Scenes for the bundled themes.
fn bundled(s: &mut Ui, id: &str) {
    if let Ok(t) = crate::theme::resolve(id) {
        s.theme = t;
    }
    demo(s);
}

fn overflow(s: &mut Ui) {
    demo(s);
    for i in 0..3 {
        s.upsert(Note::new(
            10 + i,
            "GitHub",
            format!("Review requested #{}", 412 + i),
            "anomalyco/hokum: first release",
        ));
    }
}

fn headless_state() -> Ui {
    let mut ui = Ui::new(
        crate::watch::load(),
        std::sync::mpsc::channel().0,
        std::sync::mpsc::channel().0,
    );
    // `--theme light|dark` picks the appearance, as the desktop would.
    ui.system_dark = !std::env::args().any(|a| a == "light");
    ui
}

/// The overlay's view over a wallpaper-like gradient: headless renders clear to opaque black,
/// where an overlay would show the desktop behind it.
fn headless_view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let (top, bottom) = if s.dark() {
        (
            rune::oklch(0.32, 0.06, 265.0),
            rune::oklch(0.22, 0.05, 300.0),
        )
    } else {
        (
            rune::oklch(0.86, 0.05, 230.0),
            rune::oklch(0.78, 0.07, 290.0),
        )
    };
    el().key("desktop")
        .w_full()
        .h_full()
        .gradient(top, bottom)
        .child(view(s, cx))
}

/// The cards as an app of their own, for headless snapshots (`hokum service --snap`).
pub fn headless() {
    let loaded = crate::config::load();
    let theme = loaded.theme;
    let spec = AppSpec {
        scenes: &[
            ("demo", demo),
            ("overflow", overflow),
            ("glyphs", glyphs),
            ("welcome", welcome),
            ("update", update),
            ("osd", osd),
            ("brightness", brightness),
            ("bt-connected", bt_connected),
            ("soft", |s| bundled(s, "soft")),
            ("crisp", |s| bundled(s, "crisp")),
            ("capsule", |s| bundled(s, "capsule")),
            ("ribbon", |s| bundled(s, "ribbon")),
            ("terminal", |s| bundled(s, "terminal")),
            ("bubble", |s| bundled(s, "bubble")),
            ("signal", |s| bundled(s, "signal")),
            ("pill", |s| bundled(s, "pill")),
            ("omarchy", |s| bundled(s, "omarchy")),
        ],
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        ..AppSpec::new(
            "Hokum",
            size(&theme, true, loaded.behaviour.position),
            headless_state,
            headless_view,
        )
    };
    rune::App::new(spec).run();
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};

    use super::*;

    fn ui() -> (Ui, Receiver<Out>) {
        let (out, rx) = channel();
        let mut ui = Ui::new(crate::watch::load(), out, channel().0);
        ui.script = None;
        ui.settings = Behaviour::default();
        ui.theme = crate::theme::resolve("").unwrap();
        (ui, rx)
    }

    #[test]
    fn only_cards_that_fit_show_and_the_rest_wait_behind_the_pill() {
        let notes: Vec<Note> = (1..=5).map(|i| Note::new(i, "App", "s", "b")).collect();
        let h = |v: &[(u32, f32)]| v.iter().copied().collect::<HashMap<u32, f32>>();
        // 5 cards of 100 with 10 between fit in 600 when nothing waits.
        assert_eq!(
            fitting(
                &notes,
                5,
                &h(&[(1, 100.), (2, 100.), (3, 100.), (4, 100.), (5, 100.)]),
                10.0,
                600.0,
                28.0
            ),
            5
        );
        // At 200 each, 3 fit in 784 with the pill under them (3 × 200 + 2 × 10 + 10 + 28 = 658).
        let big = h(&[(1, 200.), (2, 200.), (3, 200.), (4, 200.), (5, 200.)]);
        assert_eq!(fitting(&notes, 5, &big, 10.0, 784.0, 28.0), 3);
        // show-at-most still caps it.
        assert_eq!(fitting(&notes, 2, &big, 10.0, 784.0, 28.0), 2);
        // A card taller than the room still shows (the overlay grows for it).
        assert_eq!(fitting(&notes, 5, &h(&[(1, 2000.)]), 10.0, 784.0, 28.0), 1);
        // Cards not laid out yet count as fitting.
        assert_eq!(fitting(&notes, 5, &HashMap::new(), 10.0, 784.0, 28.0), 5);
    }

    #[test]
    fn bigger_text_gets_fewer_lines() {
        use crate::template::fewer_lines;
        assert_eq!(
            (
                fewer_lines(8, 1.0),
                fewer_lines(8, 0.5),
                fewer_lines(8, 2.0),
                fewer_lines(8, 3.0)
            ),
            (8, 8, 4, 3)
        );
        assert_eq!(
            (
                fewer_lines(1, 3.0),
                fewer_lines(2, 3.0),
                fewer_lines(3, 1.17)
            ),
            (1, 2, 3)
        );
    }

    #[test]
    fn surface_leaves_room_for_every_shadow() {
        for id in [
            "soft", "crisp", "capsule", "ribbon", "terminal", "bubble", "signal", "pill", "omarchy",
        ] {
            let theme = crate::theme::resolve(id).unwrap();
            let reach = crate::card::shadow_reach(&theme);
            if theme.base(true).shadow == crate::theme::Lift::Deep {
                assert!(
                    reach >= 54.0,
                    "{id}: deep's blur 36 fades out 54pt away, got {reach}"
                );
            }
            for dark in [false, true] {
                let width = theme.base(dark).width;
                let (w, _) = size(&theme, dark, Position::TopRight);
                // The card stays GUTTER from the screen's edge; the inner side holds the shadow.
                assert!(
                    w - width - GUTTER >= reach.max(GUTTER),
                    "{id}: {w} for a {width} card"
                );
                // A centered card has the shadow's room on both sides.
                let (w, _) = size(&theme, dark, Position::BottomCenter);
                assert!(
                    (w - width) / 2.0 >= reach.max(GUTTER),
                    "{id}: {w} for a centered {width} card"
                );
            }
        }
    }

    #[test]
    fn centered_stacks_sit_in_the_middle_of_their_edge() {
        let at = |position| {
            placement(&Behaviour {
                position,
                ..Behaviour::default()
            })
        };
        let edge = |p| match at(p) {
            OverlayPlacement::Edge { edge, align, .. } => (edge, align),
            other => panic!("{other:?}"),
        };
        assert_eq!(edge(Position::TopCenter), (ScreenEdge::Top, Align::Center));
        assert_eq!(
            edge(Position::BottomCenter),
            (ScreenEdge::Bottom, Align::Center)
        );
        assert_eq!(edge(Position::TopLeft), (ScreenEdge::Top, Align::Start));
        assert_eq!(
            edge(Position::BottomRight),
            (ScreenEdge::Bottom, Align::End)
        );
    }

    fn note(id: u32, default: bool) -> Note {
        let mut n = Note::new(id, "Slack", "Dax", "ship it");
        n.has_default = default;
        n.desktop = Some("slack".into());
        n.actions = vec![("reply".into(), "Reply".into())];
        n
    }

    #[test]
    fn clicking_a_card_invokes_default_then_closes_it_dismissed() {
        let (mut ui, rx) = ui();
        ui.notes.push(note(7, true));
        ui.activate(7);
        let out: Vec<Out> = rx.try_iter().collect();
        assert_eq!(
            out,
            vec![
                Out::Action(7, "default".into(), None),
                Out::Closed(7, Reason::Dismissed)
            ]
        );
    }

    #[test]
    fn a_button_invokes_its_own_key() {
        let (mut ui, rx) = ui();
        ui.notes.push(note(7, true));
        ui.invoke(7, "reply");
        let out: Vec<Out> = rx.try_iter().collect();
        assert_eq!(
            out,
            vec![
                Out::Action(7, "reply".into(), None),
                Out::Closed(7, Reason::Dismissed)
            ]
        );
    }

    #[test]
    fn resident_cards_stay_after_an_action() {
        let (mut ui, rx) = ui();
        let mut n = note(7, true);
        n.resident = true;
        ui.notes.push(n);
        ui.activate(7);
        let out: Vec<Out> = rx.try_iter().collect();
        assert_eq!(out, vec![Out::Action(7, "default".into(), None)]);
    }

    #[test]
    fn the_welcome_opens_settings_instead_of_signalling() {
        let (mut ui, rx) = ui();
        ui.notes.push(crate::welcome::note());
        ui.invoke(crate::welcome::ID, "open-settings");
        let out: Vec<Out> = rx.try_iter().collect();
        assert_eq!(
            out,
            vec![Out::Closed(crate::welcome::ID, Reason::Dismissed)]
        );
    }

    #[test]
    fn without_a_default_action_a_click_only_dismisses() {
        let (mut ui, rx) = ui();
        ui.notes.push(note(7, false));
        ui.activate(7);
        let out: Vec<Out> = rx.try_iter().collect();
        assert_eq!(out, vec![Out::Closed(7, Reason::Dismissed)]);
    }

    #[test]
    fn an_omarchy_command_makes_a_click_invoke_and_hokum_invoke_takes_the_newest() {
        let (mut ui, rx) = ui();
        let mut n = note(7, false);
        n.commands = vec![("default".into(), vec!["true".into()])];
        ui.notes.push(n);
        ui.notes.insert(0, note(8, false));
        assert_eq!(ui.dismiss_newest(false), 1);
        assert!(ui.activate_newest(None));
        assert!(!ui.activate_newest(None));
        let out: Vec<Out> = rx.try_iter().collect();
        assert_eq!(
            out,
            vec![
                Out::Closed(8, Reason::Dismissed),
                Out::Action(7, "default".into(), None),
                Out::Closed(7, Reason::Dismissed)
            ]
        );
    }

    fn volume(id: u32, value: f32) -> Note {
        let mut n = Note::new(
            id,
            "pamixer",
            format!("Volume [{}%]", (value * 100.0) as u32),
            "",
        );
        n.value = Some(value);
        n.tag = Some("volume".into());
        n
    }

    #[test]
    fn a_tagged_update_keeps_the_card_in_place() {
        let (mut ui, rx) = ui();
        ui.upsert(volume(1, 0.4));
        ui.upsert(Note::new(2, "Slack", "Dax", "ship it"));
        ui.upsert(volume(3, 0.45));
        let notes: Vec<(u32, u32, Option<f32>)> =
            ui.notes.iter().map(|n| (n.id, n.slot, n.value)).collect();
        assert_eq!(notes, vec![(2, 2, None), (3, 1, Some(0.45))]);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![Out::Closed(1, Reason::Closed)]
        );
    }

    #[test]
    fn replaces_id_keeps_the_card_and_restarts_its_timer() {
        let (mut ui, _rx) = ui();
        ui.upsert(volume(5, 0.2));
        let first = ui.notes[0].deadline.unwrap();
        ui.upsert(volume(5, 0.3));
        assert_eq!(ui.notes.len(), 1);
        assert_eq!((ui.notes[0].slot, ui.notes[0].value), (5, Some(0.3)));
        assert!(ui.notes[0].deadline.unwrap() >= first);
    }

    #[test]
    fn an_update_after_its_card_left_enters_fresh() {
        let (mut ui, _rx) = ui();
        ui.upsert(volume(1, 0.4));
        ui.close(1, Reason::Expired);
        ui.upsert(volume(2, 0.5));
        assert_eq!((ui.notes[0].id, ui.notes[0].slot), (2, 2));
    }

    #[test]
    fn only_new_cards_chime() {
        let (mut ui, _rx) = ui();
        let (tx, chimes) = channel();
        ui.chime = Some(tx);
        let heard = || chimes.try_iter().count();
        ui.upsert(Note::new(1, "App", "s", "b"));
        assert_eq!(heard(), 0, "off by default");
        ui.settings.sound = true;
        ui.upsert(Note::new(2, "App", "s", "b"));
        assert_eq!(heard(), 1);
        ui.upsert(Note::new(2, "App", "s", "changed"));
        assert_eq!(heard(), 0, "an update in place");
        let mut quiet = Note::new(3, "App", "s", "b");
        quiet.sound = crate::chime::Hint::Suppress;
        ui.upsert(quiet);
        assert_eq!(heard(), 0, "suppress-sound");
        let mut osd = Note::new(4, "Volume", "s", "b");
        osd.hud = true;
        ui.upsert(osd);
        assert_eq!(heard(), 0, "Hokum's own card");
        ui.settings.do_not_disturb = true;
        ui.upsert(Note::new(5, "App", "s", "b"));
        assert_eq!(heard(), 0, "held back by Do not disturb");
        let mut critical = Note::new(6, "App", "s", "b");
        critical.urgency = 2;
        ui.upsert(critical);
        assert_eq!(heard(), 1, "critical breaks through");
        ui.settings.do_not_disturb = false;
        ui.theme = crate::theme::parse(
            "rule app=\"Mute\" { hide }\nrule app=\"Hush\" { sound #false }\n",
            "x",
        )
        .unwrap();
        ui.upsert(Note::new(7, "Mute", "s", "b"));
        ui.upsert(Note::new(8, "Hush", "s", "b"));
        assert_eq!(heard(), 0, "hidden and silenced apps");
    }
}
