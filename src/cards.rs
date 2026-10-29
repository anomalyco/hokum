//! The notification stack: an overlay (a click-through layer surface on Wayland) with its own
//! state. It shows for the first card, so it opens on the output focused then, and hides once
//! the last card has left.

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
            h.resize(size(&self.theme, self.dark(), settings.position));
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
            crate::apps::record(&n.app);
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
        if let Some(i) = self.notes.iter().position(same) {
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
        if self.notes.len() != before {
            if id == crate::update::ID {
                crate::update::closed(reason);
            }
            let _ = self.out.send(Out::Closed(id, reason));
            self.reschedule();
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

/// The stack of cards for `notes` in a corner (or the middle of an edge) of its parent, as the
/// overlay and the settings preview both draw it. Top stacks grow downward, bottom ones upward.
pub fn stack<S: Cards>(
    notes: &[Note],
    s: &Behaviour,
    theme: &Theme,
    k: &Kit,
    more: Option<El>,
) -> El {
    let base = theme.base(k.dark);
    let between = base.spacing;
    let side = s.position.side();
    let bottom = s.position.bottom();
    let shown = notes
        .iter()
        .take(s.max_shown.max(1))
        .map(|n| card::<S>(n, theme, k, s.position));
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
    row()
        .key("more")
        .role("button")
        .label("Clear all")
        .h(28.0)
        .px(12.0)
        .gap(8.0)
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
            text(format!("{hidden} more"))
                .key("count")
                .font(k.ty.small_medium)
                .color(c.text_muted),
        )
        .child(
            text("Clear all")
                .key("clear")
                .font(k.ty.small_medium)
                .color(c.text_base),
        )
        .transition(Transition::opacity().combined(Transition::scale(0.9)))
}

pub fn view(s: &Ui, _: &mut Cx<Ui>) -> El {
    let k = rune_theme_opencode::kit(s.dark());
    let hidden = s.notes.len().saturating_sub(s.settings.max_shown.max(1));
    let pill = (hidden > 0).then(|| more(hidden, k));
    el().key("root")
        .w_full()
        .h_full()
        .child(stack::<Ui>(&s.notes, &s.settings, &s.theme, k, pill))
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

/// A Bluetooth headset connecting, and a mouse running low.
fn bt_connected(s: &mut Ui) {
    for n in crate::integrations::samples().into_iter().skip(1).rev() {
        s.upsert(decoded(n));
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
            ("welcome", welcome),
            ("update", update),
            ("osd", osd),
            ("bt-connected", bt_connected),
            ("soft", |s| bundled(s, "soft")),
            ("crisp", |s| bundled(s, "crisp")),
            ("capsule", |s| bundled(s, "capsule")),
            ("ribbon", |s| bundled(s, "ribbon")),
            ("terminal", |s| bundled(s, "terminal")),
            ("bubble", |s| bundled(s, "bubble")),
            ("signal", |s| bundled(s, "signal")),
            ("pill", |s| bundled(s, "pill")),
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
    fn surface_leaves_room_for_every_shadow() {
        for id in [
            "soft", "crisp", "capsule", "ribbon", "terminal", "bubble", "signal", "pill",
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
}
