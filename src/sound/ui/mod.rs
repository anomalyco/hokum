//! Sound's section of the Hokum window and its rows in the tray panel, drawn with Rune in
//! Hokum's look. Both read [`State`] snapshots from an [`Audio`] handle and send [`Command`]s;
//! neither touches PipeWire.
//!
//! Progressive disclosure:
//! 1. `view::main`: output and input (mute, volume, device) and the apps playing. That's all.
//! 2. A row opens in place: devices to switch to, ports, the Bluetooth codec, balance, an app's
//!    output, recording apps.
//! 3. `advanced`: every device and card profile, virtual outputs, routing, the clock.
//!
//! Handlers take the window's state ([`Host`]) and reach this section's through [`lift`] or
//! `h.sound`. Meters run only for what's on screen ([`Meters`]).

mod advanced;
pub mod glyphs;
mod icons;
mod slider;
mod view;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rune::{Drag, El, Image, Phase, TextEdit, Tx, WheelEvent};
use rune_kit::Kit;

use crate::sound::model::{Audio, Command, Id, MAX_VOLUME, State, StreamKind, Subscription, Target};
use slider::Slide;

/// The state every handler gets: the window's (or the tray panel's).
pub type Host = crate::settings::Ui;

/// A handler written for this section's state, as one for the window's.
pub fn lift(f: impl Fn(&mut Ui) + 'static) -> impl Fn(&mut Host) + 'static {
    move |h: &mut Host| f(&mut h.sound)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Window,
    Popover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Row {
    Output,
    Input,
    Stream(Id),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Devices,
    Routing,
    Clock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Advanced(Tab),
}

/// Which view of the two (window, popover) wants which targets metered; the backend hears the
/// union. `SetMetering` isn't counted, so one view turning a target off mustn't stop the
/// other's meter.
pub struct Meters {
    audio: Audio,
    views: Mutex<[HashSet<Target>; 2]>,
}

impl Meters {
    pub fn new(audio: Audio) -> Arc<Meters> {
        Arc::new(Meters { audio, views: Mutex::new([HashSet::new(), HashSet::new()]) })
    }

    pub fn want(&self, mode: Mode, targets: HashSet<Target>) {
        let mut views = self.views.lock().unwrap_or_else(|e| e.into_inner());
        let i = mode as usize;
        if views[i] == targets {
            return;
        }
        let before: HashSet<Target> = views[0].union(&views[1]).copied().collect();
        views[i] = targets;
        let after: HashSet<Target> = views[0].union(&views[1]).copied().collect();
        let on: Vec<Target> = after.difference(&before).copied().collect();
        let off: Vec<Target> = before.difference(&after).copied().collect();
        if !on.is_empty() {
            self.audio.send(Command::SetMetering { targets: on, on: true });
        }
        if !off.is_empty() {
            self.audio.send(Command::SetMetering { targets: off, on: false });
        }
    }
}

pub struct Ui {
    pub audio: Arc<State>,
    /// `None` in headless scenes.
    pub handle: Option<Audio>,
    pub meters: Option<Arc<Meters>>,
    pub mode: Mode,
    /// Whether this view is on screen (meters stop when it isn't).
    pub visible: bool,
    pub page: Page,
    pub open: HashSet<Row>,
    pub slide: Option<Slide>,
    /// Volumes sent but not yet published: (value, revision when sent).
    pending: HashMap<Target, (f32, u64)>,
    /// The open select menu, by key.
    pub menu: Option<String>,
    /// The name field for a new virtual output, while it's open.
    pub naming: Option<TextEdit>,
    /// The routing view's picked source node.
    pub route_from: Option<Id>,
    icons: HashMap<String, Option<Image>>,
    cache: Option<rune::ImageCache>,
    pub _subscription: Option<Arc<Subscription>>,
}

impl Ui {
    pub fn new(audio: Option<Audio>, mode: Mode) -> Ui {
        let state = audio.as_ref().map_or_else(|| Arc::new(State::default()), Audio::state);
        Ui {
            audio: state,
            meters: audio.clone().map(Meters::new),
            handle: audio,
            mode,
            visible: true,
            page: Page::Main,
            open: HashSet::new(),
            slide: None,
            pending: HashMap::new(),
            menu: None,
            naming: None,
            route_from: None,
            icons: HashMap::new(),
            cache: None,
            _subscription: None,
        }
    }

    /// Swaps the backend (`None` when the module turns off), keeping what's on screen.
    pub fn attach(&mut self, audio: Option<Audio>, meters: Option<Arc<Meters>>, subscription: Option<Subscription>) {
        self.audio = audio.as_ref().map_or_else(|| Arc::new(State::default()), Audio::state);
        self.handle = audio;
        self.meters = meters;
        self._subscription = subscription.map(Arc::new);
        self.pending.clear();
        self.open.clear();
        self.page = Page::Main;
        self.decode_icons();
    }

    pub fn send(&self, c: Command) {
        if let Some(a) = &self.handle {
            a.send(c);
        }
    }

    /// A new snapshot: drop optimistic volumes it confirms (or that went stale).
    pub fn receive(&mut self, state: Arc<State>) {
        let rev = state.revision;
        self.pending.retain(|t, (v, sent)| {
            let now = match *t {
                Target::Device(id) => state.device(id).map(|d| d.volume.combined()),
                Target::Stream(id) => state.stream(id).map(|s| s.volume.combined()),
            };
            now.is_some_and(|n| (n - *v).abs() > 0.004) && rev < *sent + 60
        });
        self.open.retain(|r| match r {
            Row::Stream(id) => state.stream(*id).is_some(),
            _ => true,
        });
        self.audio = state;
        self.decode_icons();
    }

    fn volume_now(&self, t: Target) -> Option<(f32, bool)> {
        match t {
            Target::Device(id) => self.audio.device(id).map(|d| (d.volume.combined(), d.mute)),
            Target::Stream(id) => self.audio.stream(id).map(|s| (s.volume.combined(), s.mute)),
        }
    }

    /// What the slider for `t` shows: the drag, a volume in flight, or the snapshot's.
    pub fn shown(&self, t: Target) -> f32 {
        if let Some(s) = self.slide.filter(|s| s.target == t) {
            return s.value;
        }
        if let Some((v, _)) = self.pending.get(&t) {
            return *v;
        }
        self.volume_now(t).map_or(0.0, |v| v.0)
    }

    pub fn scale(&self, t: Target, v: f32) -> f32 {
        self.slide.filter(|s| s.target == t).map_or_else(|| slider::scale_for(v), |s| s.scale)
    }

    pub fn muted(&self, t: Target) -> bool {
        self.volume_now(t).is_some_and(|v| v.1)
    }

    pub fn set_volume(&mut self, t: Target, v: f32) {
        let v = v.clamp(0.0, MAX_VOLUME);
        self.pending.insert(t, (v, self.audio.revision));
        if self.muted(t) && v > 0.0 {
            self.send(Command::SetMute { target: t, mute: false });
        }
        self.send(Command::SetVolume { target: t, volume: v });
    }

    pub fn slide(&mut self, t: Target, d: &Drag) {
        if d.phase == Phase::Down {
            let start = self.shown(t);
            self.slide = Some(Slide { target: t, scale: slider::scale_for(start), start, value: start });
        }
        let Some(mut s) = self.slide.filter(|s| s.target == t) else { return };
        if d.phase == Phase::Up && d.cancelled() {
            self.slide = None;
            self.set_volume(t, s.start);
            return;
        }
        s.value = slider::value_at(d.x, d.w, s.scale);
        self.slide = (d.phase != Phase::Up).then_some(s);
        self.set_volume(t, s.value);
    }

    pub fn step(&mut self, t: Target, n: i32) {
        let v = self.shown(t);
        let ceiling = if v > 1.0 { MAX_VOLUME } else { 1.0 };
        // Land on multiples of 5 %.
        let next = ((v * 20.0).round() + n as f32) / 20.0;
        self.set_volume(t, next.clamp(0.0, ceiling));
    }

    pub fn wheel(&mut self, t: Target, e: WheelEvent) {
        let d = e.dy - e.dx;
        if d == 0.0 {
            return;
        }
        if e.precise {
            let v = self.shown(t);
            let ceiling = if v > 1.0 { MAX_VOLUME } else { 1.0 };
            self.set_volume(t, (v + d * 0.002).clamp(0.0, ceiling.max(v.min(MAX_VOLUME))));
        } else {
            self.step(t, if d > 0.0 { 1 } else { -1 });
        }
    }

    pub fn toggle_mute(&mut self, t: Target) {
        let mute = !self.muted(t);
        self.send(Command::SetMute { target: t, mute });
    }

    pub fn balance_of(&self, t: Target) -> f32 {
        match t {
            Target::Device(id) => self.audio.device(id).map(|d| d.volume.balance()),
            Target::Stream(id) => self.audio.stream(id).map(|s| s.volume.balance()),
        }
        .unwrap_or(0.0)
    }

    pub fn set_balance(&mut self, t: Target, b: f32) {
        self.send(Command::SetBalance { target: t, balance: b });
    }

    pub fn toggle(&mut self, r: Row) {
        if !self.open.remove(&r) {
            self.open.insert(r);
        }
    }

    /// Streams the first level shows: playback that's producing sound.
    pub fn playing(&self) -> impl Iterator<Item = &crate::sound::model::Stream> {
        self.audio.streams.iter().filter(|s| s.kind == StreamKind::Playback && !s.corked)
    }

    /// What's on screen, for [`Meters`].
    fn metered(&self) -> HashSet<Target> {
        let mut out = HashSet::new();
        if !self.visible || self.page != Page::Main {
            return out;
        }
        let a = &self.audio;
        out.extend(a.default_sink.map(Target::Device));
        if self.mode == Mode::Popover {
            if view::input_matters(self) {
                out.extend(a.default_source.map(Target::Device));
            }
            return out;
        }
        out.extend(a.default_source.map(Target::Device));
        out.extend(self.playing().map(|s| Target::Stream(s.id)));
        if self.open.contains(&Row::Input) {
            out.extend(a.streams.iter().filter(|s| s.kind == StreamKind::Record).map(|s| Target::Stream(s.id)));
        }
        out
    }

    pub fn sync_meters(&self) {
        if let Some(m) = &self.meters {
            m.want(self.mode, self.metered());
        }
    }

    /// Off screen without being hidden (another section is showing): no meters.
    pub fn unmeter(&self) {
        if let Some(m) = &self.meters {
            m.want(self.mode, HashSet::new());
        }
    }

    pub fn hidden(&mut self) {
        self.visible = false;
        self.slide = None;
        self.menu = None;
        self.sync_meters();
    }

    fn icon_key(s: &crate::sound::model::Stream) -> String {
        format!("{}\u{1f}{}\u{1f}{}", s.icon_name, s.binary, s.app_name)
    }

    fn decode_icons(&mut self) {
        let Some(cache) = &self.cache else { return };
        for s in &self.audio.streams {
            let key = Self::icon_key(s);
            self.icons.entry(key).or_insert_with(|| icons::for_app(&s.icon_name, &s.binary, &s.app_name).map(|p| cache.file(p, 40.0, 2.0)));
        }
    }

    pub fn icon(&self, s: &crate::sound::model::Stream) -> Option<Image> {
        self.icons.get(&Self::icon_key(s)).cloned().flatten()
    }

    /// Esc: close what's open, one layer at a time.
    pub fn escape(&mut self, tx: &mut Tx) -> bool {
        if self.menu.take().is_some() || self.naming.take().is_some() || self.route_from.take().is_some() {
            return true;
        }
        if let Page::Advanced(_) = self.page {
            tx.instant();
            self.page = Page::Main;
            return true;
        }
        if !self.open.is_empty() {
            self.open.clear();
            return true;
        }
        false
    }
}

/// The section: the first screen with its rows, or the advanced page. Select menus go to `layer`.
pub fn section(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> El {
    s.sync_meters();
    match s.page {
        Page::Main => view::main(s, k, layer),
        Page::Advanced(tab) => advanced::page(s, k, tab, layer),
    }
}

/// The tray panel's rows: output, and input when something's recording or it's muted.
pub fn panel(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    s.sync_meters();
    view::tray_rows(s, k, layer)
}

/// Keys for the section: Esc, M (mute the output), Ctrl+, (advanced).
pub fn on_key(s: &mut Ui, e: &rune::KeyEvent, tx: &mut Tx) -> bool {
    use rune::Key;
    match &e.key {
        Key::Escape => s.escape(tx),
        Key::Text(t) if t.eq_ignore_ascii_case("m") && !e.mods.cmd && s.page == Page::Main => {
            if let Some(id) = s.audio.default_sink {
                s.toggle_mute(Target::Device(id));
            }
            true
        }
        Key::Command(',') if s.mode == Mode::Window => {
            tx.instant();
            s.page = if s.page == Page::Main { Page::Advanced(Tab::Devices) } else { Page::Main };
            true
        }
        _ => false,
    }
}

/// Decodes app icons now (headless renders have no event loop to finish decodes).
pub fn decode_now(s: &mut Ui) {
    s.cache = Some(rune::ImageCache::new(Arc::new(|| {})));
    s.decode_icons();
    for img in s.icons.values().flatten() {
        let _ = img.wait();
    }
}

pub fn setup(s: &mut Ui, waker: rune::Waker) {
    s.cache = Some(rune::ImageCache::new(waker));
    s.decode_icons();
}

/// Scenes: the mock sample, then each level.
pub mod scenes {
    use super::*;

    pub fn main(s: &mut Ui) {
        s.audio = Arc::new(crate::sound::mock::sample());
        decode_now(s);
    }

    /// On the Bluetooth headset, so the codec shows, with the output and an app open.
    pub fn expanded(s: &mut Ui) {
        let mut a = crate::sound::mock::sample();
        let headset = a.sinks().find(|d| d.is_bluetooth()).map(|d| d.id);
        if let Some(id) = headset {
            crate::sound::mock::apply(&mut a, Command::SetDefault { device: id }, &mut HashSet::new());
        }
        s.audio = Arc::new(a);
        decode_now(s);
        s.open.insert(Row::Output);
        let first = s.playing().next().map(|st| st.id);
        s.open.extend(first.map(Row::Stream));
    }

    pub fn advanced(s: &mut Ui) {
        main(s);
        s.page = Page::Advanced(Tab::Devices);
    }

    pub fn lost(s: &mut Ui) {
        let mut a = crate::sound::mock::sample();
        a.connection = crate::sound::model::Connection::Lost("PipeWire isn't running".into());
        s.audio = Arc::new(a);
        decode_now(s);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meters_turn_off_only_what_no_view_shows() {
        let (audio, driver) = crate::sound::model::channel();
        let m = Meters::new(audio);
        let a = Target::Device(1);
        let b = Target::Stream(2);
        m.want(Mode::Window, [a, b].into());
        m.want(Mode::Popover, [a].into());
        m.want(Mode::Window, HashSet::new());
        let got: Vec<Command> = driver.commands().try_iter().collect();
        assert_eq!(got.len(), 2);
        assert!(matches!(&got[0], Command::SetMetering { on: true, targets } if targets.len() == 2));
        assert_eq!(got[1], Command::SetMetering { targets: vec![b], on: false });
    }
}
