//! The audio model: the contract between a backend (`crate::sound::pipewire` on PipeWire, `crate::sound::mock`
//! for scenes and UI work) and the UI (`crate::ui`).
//!
//! A backend owns the connection and publishes plain-data [`State`] snapshots; the UI reads the
//! latest one, is told when a new one lands, and asks for changes with [`Command`]s. Nothing in
//! here touches PipeWire types, and nothing in the UI touches PipeWire.
//!
//! ```text
//! backend thread ── Driver::publish(State) ──▶ Audio::state() / on_change(..) ──▶ UI
//!        ▲                                                                    │
//!        └──────────── Driver::commands() ◀── Audio::send(Command) ◀───────────┘
//! ```
//!
//! Rules both sides keep:
//! - A snapshot is complete and consistent: every id it mentions exists in it.
//! - Commands are requests, not changes. The UI never edits its copy of [`State`]; it waits for
//!   the backend to publish the result (optimistic display, e.g. while dragging a slider, is
//!   the UI's own business).
//! - Volumes are cubic (what pavucontrol and wpctl show): `0.0` silent, `1.0` 100 %, up to
//!   [`MAX_VOLUME`] (150 %). The backend converts to and from PipeWire's linear values.
//! - Levels (meters) are peaks in `0.0..=1.0` on the same cubic scale, present only for the
//!   targets the UI asked to meter with [`Command::SetMetering`]. Backends publish at most
//!   ~30 snapshots a second while anything is metered.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// A PipeWire global id (node, device, link, port). Stable while the object exists; ids can be
/// reused after it goes away.
pub type Id = u32;

/// Cubic volume of 150 %, the most the UI offers (as pavucontrol does).
pub const MAX_VOLUME: f32 = 1.5;

/// Everything the UI shows, at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Bumped by the backend on every publish; lets the UI skip identical work.
    pub revision: u64,
    pub connection: Connection,
    /// The sink new playback goes to (`default.audio.sink`), when there is one.
    pub default_sink: Option<Id>,
    /// The source new recordings come from (`default.audio.source`).
    pub default_source: Option<Id>,
    /// Sinks, sources and sink monitors, in a stable order (by priority, then name).
    pub devices: Vec<Device>,
    /// Hardware and Bluetooth cards: profiles live here.
    pub cards: Vec<Card>,
    /// Application playback and recording streams.
    pub streams: Vec<Stream>,
    pub server: ServerInfo,
    /// The full node/port/link graph, for the advanced routing view.
    pub graph: Graph,
    /// Things worth telling the user about (a failed command, a missing session manager).
    pub issues: Vec<Issue>,
}

impl State {
    pub fn device(&self, id: Id) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn card(&self, id: Id) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    pub fn stream(&self, id: Id) -> Option<&Stream> {
        self.streams.iter().find(|s| s.id == id)
    }

    /// Output devices (sinks), hidden ones included.
    pub fn sinks(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.kind == DeviceKind::Sink)
    }

    /// Input devices (sources), not monitors.
    pub fn sources(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.kind == DeviceKind::Source)
    }

    pub fn default_sink(&self) -> Option<&Device> {
        self.default_sink.and_then(|id| self.device(id))
    }

    pub fn default_source(&self) -> Option<&Device> {
        self.default_source.and_then(|id| self.device(id))
    }
}

/// Whether the backend is talking to PipeWire.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Connection {
    #[default]
    Connecting,
    Connected,
    /// The daemon went away or never answered; the backend keeps retrying.
    Lost(String),
}

/// A sink, a source, or a sink's monitor: one PipeWire `Audio/Sink` / `Audio/Source` node.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: Id,
    /// `node.name`, stable across restarts (what config and `SetDefault` persist by).
    pub name: String,
    /// `node.description`: what people read ("Speakers", "WH-1000XM4").
    pub description: String,
    /// A freedesktop icon name (`audio-speakers`, `audio-headphones`, `audio-headset`,
    /// `audio-input-microphone`, ...), from the node, its card or its form factor.
    pub icon_name: String,
    pub kind: DeviceKind,
    pub volume: Volume,
    pub mute: bool,
    /// Output or input ports (jacks): "Speakers", "Headphones", "Line In".
    pub ports: Vec<Port>,
    /// The active port's `Port::name`.
    pub active_port: Option<String>,
    /// The card (PipeWire device) this node belongs to; `None` for virtual nodes.
    pub card_id: Option<Id>,
    pub bus: Bus,
    /// Created by software (null sinks, loopbacks, filter chains) rather than hardware.
    pub is_virtual: bool,
    /// Created by this app with [`Command::CreateVirtualSink`]; only those can be removed.
    pub is_ours: bool,
    /// Kept out of the first level (monitors, the user's hidden devices); still listed.
    pub hidden: bool,
    /// Live peak, `0.0..=1.0`; `None` unless metering is on for this device.
    pub level: Option<f32>,
}

impl Device {
    pub fn is_bluetooth(&self) -> bool {
        self.bus == Bus::Bluetooth
    }

    pub fn port(&self) -> Option<&Port> {
        let active = self.active_port.as_deref()?;
        self.ports.iter().find(|p| p.name == active)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// Plays sound (`Audio/Sink`).
    Sink,
    /// Records sound (`Audio/Source`).
    Source,
    /// What a sink is playing, as a source (`<sink>.monitor`).
    Monitor,
}

/// How a device is attached, for icons and grouping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bus {
    Pci,
    Usb,
    Bluetooth,
    /// HDMI / DisplayPort audio on a GPU.
    Display,
    #[default]
    Virtual,
    Other,
}

/// A jack or route on a device.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    /// Stable id (`analog-output-headphones`).
    pub name: String,
    pub description: String,
    /// Whether something is plugged in, when the hardware can tell.
    pub available: Availability,
    pub priority: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Availability {
    #[default]
    Unknown,
    Yes,
    No,
}

/// A card: one PipeWire `Audio/Device` (a sound card, a USB interface, a Bluetooth headset).
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: Id,
    /// `device.name`.
    pub name: String,
    /// `device.description`.
    pub description: String,
    pub icon_name: String,
    pub bus: Bus,
    pub profiles: Vec<Profile>,
    /// The active profile's `Profile::name`.
    pub active_profile: Option<String>,
    /// Battery level of a Bluetooth device, `0.0..=1.0`, when it reports one.
    pub battery: Option<f32>,
}

impl Card {
    pub fn profile(&self) -> Option<&Profile> {
        let active = self.active_profile.as_deref()?;
        self.profiles.iter().find(|p| p.name == active)
    }
}

/// A card profile ("Analog Stereo Duplex", "High Fidelity Playback (A2DP Sink, codec LDAC)").
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Stable id (`output:analog-stereo+input:analog-stereo`, `a2dp-sink-ldac`, `off`).
    pub name: String,
    pub description: String,
    /// False when the hardware can't use it now (nothing plugged in, headset not connected).
    pub available: bool,
    pub priority: u32,
    /// What it carries, so the UI can say "Music" vs "Calls" without parsing names.
    pub kind: ProfileKind,
    /// The Bluetooth codec it uses (`SBC`, `AAC`, `LDAC`, `aptX`, `mSBC`, `LC3`), if any.
    pub codec: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProfileKind {
    /// Playback only (A2DP, analog/HDMI output).
    Output,
    /// Recording only.
    Input,
    /// Both ways (analog duplex, Bluetooth headset/HFP with a microphone).
    #[default]
    Duplex,
    /// The card is off.
    Off,
}

/// An application's stream: `Stream/Output/Audio` (playback) or `Stream/Input/Audio` (record).
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub id: Id,
    /// `application.name` ("Spotify", "Firefox").
    pub app_name: String,
    /// `application.icon-name` or the binary's icon; may be empty.
    pub icon_name: String,
    /// `media.name`: what it's playing ("Never Gonna Give You Up", "Meet – Standup").
    pub media_name: String,
    /// `application.process.binary`, for icons and per-app rules.
    pub binary: String,
    pub kind: StreamKind,
    pub volume: Volume,
    pub mute: bool,
    /// The device it plays to or records from, once linked.
    pub target_device: Option<Id>,
    /// Paused / corked: present but not producing sound.
    pub corked: bool,
    /// Live peak, `0.0..=1.0`; `None` unless metering is on for this stream.
    pub level: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Playback,
    Record,
}

/// Per-channel cubic volume. `channels[i]` is at `positions[i]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Volume {
    pub channels: Vec<f32>,
    pub positions: Vec<ChannelPosition>,
}

impl Volume {
    /// The same volume on every channel of a stereo pair.
    pub fn stereo(v: f32) -> Self {
        Volume {
            channels: vec![v, v],
            positions: vec![ChannelPosition::FL, ChannelPosition::FR],
        }
    }

    pub fn mono(v: f32) -> Self {
        Volume {
            channels: vec![v],
            positions: vec![ChannelPosition::Mono],
        }
    }

    /// The one number a slider shows: the loudest channel (as pavucontrol does).
    pub fn combined(&self) -> f32 {
        self.channels.iter().copied().fold(0.0, f32::max)
    }

    /// Left/right balance in `-1.0..=1.0`: `-1.0` only left, `0.0` centred.
    pub fn balance(&self) -> f32 {
        let (mut left, mut right) = (0.0f32, 0.0f32);
        for (v, p) in self.channels.iter().zip(&self.positions) {
            match p.side() {
                Side::Left => left = left.max(*v),
                Side::Right => right = right.max(*v),
                Side::Centre => {}
            }
        }
        let loud = left.max(right);
        if loud <= 0.0 {
            0.0
        } else {
            (right - left) / loud
        }
    }

    /// This volume scaled so `combined()` is `v`, keeping the balance between channels.
    pub fn with_combined(&self, v: f32) -> Self {
        let v = v.clamp(0.0, MAX_VOLUME);
        let current = self.combined();
        let channels = if current <= 0.0 {
            vec![v; self.channels.len()]
        } else {
            self.channels.iter().map(|c| c / current * v).collect()
        };
        Volume {
            channels,
            positions: self.positions.clone(),
        }
    }

    /// This volume with `balance` applied: the far side quieter, the near side at `combined()`.
    pub fn with_balance(&self, balance: f32) -> Self {
        let b = balance.clamp(-1.0, 1.0);
        let top = self.combined();
        let channels = self
            .channels
            .iter()
            .zip(&self.positions)
            .map(|(_, p)| match p.side() {
                Side::Left => top * (1.0 - b.max(0.0)),
                Side::Right => top * (1.0 + b.min(0.0)),
                Side::Centre => top,
            })
            .collect();
        Volume {
            channels,
            positions: self.positions.clone(),
        }
    }
}

/// Speaker positions (SPA's), the ones the UI cares about by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelPosition {
    Mono,
    FL,
    FR,
    FC,
    LFE,
    RL,
    RR,
    SL,
    SR,
    /// Anything else (aux channels, top speakers), by SPA position number.
    Other(u32),
}

enum Side {
    Left,
    Right,
    Centre,
}

impl ChannelPosition {
    fn side(self) -> Side {
        match self {
            ChannelPosition::FL | ChannelPosition::RL | ChannelPosition::SL => Side::Left,
            ChannelPosition::FR | ChannelPosition::RR | ChannelPosition::SR => Side::Right,
            _ => Side::Centre,
        }
    }
}

/// The PipeWire daemon and its clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerInfo {
    /// `core.name` / `core.version`, e.g. "pipewire-0" / "1.4.2".
    pub name: String,
    pub version: String,
    /// `clock.rate`, the graph's sample rate in Hz.
    pub rate: u32,
    /// `clock.allowed-rates`.
    pub allowed_rates: Vec<u32>,
    /// `clock.quantum`, the buffer size in samples.
    pub quantum: u32,
    pub min_quantum: u32,
    pub max_quantum: u32,
    /// `clock.force-rate` / `clock.force-quantum` when set (0 in PipeWire means unset).
    pub forced_rate: Option<u32>,
    pub forced_quantum: Option<u32>,
}

/// Every node, port and link: the routing view's patchbay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub ports: Vec<GraphPort>,
    pub links: Vec<Link>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: Id,
    /// `node.nick` / `node.description` / `node.name`, the first that's set.
    pub name: String,
    /// `media.class` ("Audio/Sink", "Stream/Output/Audio", "Audio/Filter", ...).
    pub media_class: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphPort {
    pub id: Id,
    pub node: Id,
    /// `port.alias` or `port.name` ("playback_FL", "monitor_FR").
    pub name: String,
    pub direction: Direction,
    pub channel: Option<ChannelPosition>,
    pub is_monitor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Input,
    Output,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: Id,
    pub output_node: Id,
    pub output_port: Id,
    pub input_node: Id,
    pub input_port: Id,
    /// Running (`active` state), not just negotiated or paused.
    pub active: bool,
}

/// Something to tell the user.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    /// Short, plain words ("Couldn't switch to LDAC").
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// What a volume, mute or balance command applies to: a device or a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Device(Id),
    Stream(Id),
}

/// A change the UI asks the backend for. The result arrives as a new [`State`]; failures as an
/// [`Issue`].
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Sets the combined volume (`Volume::with_combined`), keeping the channel balance.
    SetVolume {
        target: Target,
        volume: f32,
    },
    /// Sets every channel exactly.
    SetChannelVolumes {
        target: Target,
        volume: Volume,
    },
    SetMute {
        target: Target,
        mute: bool,
    },
    /// `-1.0..=1.0`, see `Volume::with_balance`.
    SetBalance {
        target: Target,
        balance: f32,
    },
    /// Makes a sink or source the default (persisted by the session manager).
    SetDefault {
        device: Id,
    },
    SetPort {
        device: Id,
        port: String,
    },
    SetProfile {
        card: Id,
        profile: String,
    },
    /// Moves a stream to another device (and remembers it for the app, as pavucontrol does).
    MoveStream {
        stream: Id,
        device: Id,
    },
    /// Turns live levels on or off for these targets. Metering costs CPU; the UI meters only
    /// what's on screen.
    SetMetering {
        targets: Vec<Target>,
        on: bool,
    },
    /// Creates a null sink named `name` (a "virtual device" to route apps into).
    CreateVirtualSink {
        name: String,
    },
    /// Removes a virtual sink this app created (`Device::is_ours`).
    RemoveVirtualSink {
        device: Id,
    },
    /// Forces the graph's rate and quantum; `None` returns either to automatic.
    SetRateQuantum {
        rate: Option<u32>,
        quantum: Option<u32>,
    },
    /// Links two ports (routing view).
    Link {
        output_port: Id,
        input_port: Id,
    },
    Unlink {
        link: Id,
    },
    /// Drops an issue from `State::issues` once the user has seen it.
    DismissIssue {
        index: usize,
    },
}

type Listener = Box<dyn Fn(&Arc<State>) + Send + Sync>;

struct Shared {
    state: Mutex<Arc<State>>,
    listeners: Mutex<Vec<(u64, Listener)>>,
    next: Mutex<u64>,
}

/// The UI's side: the latest snapshot, change notifications, and commands. Cheap to clone.
#[derive(Clone)]
pub struct Audio {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

impl Audio {
    /// The latest snapshot.
    pub fn state(&self) -> Arc<State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Asks the backend for a change. Never blocks; dropped silently if the backend is gone.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Calls `f` on the backend's thread with every new snapshot, until the returned
    /// [`Subscription`] is dropped. Keep `f` short: hand the snapshot to the UI thread, e.g.
    /// `move |s| { let s = s.clone(); handle.update(move |ui, _| ui.audio = s) }`.
    pub fn on_change(&self, f: impl Fn(&Arc<State>) + Send + Sync + 'static) -> Subscription {
        let id = {
            let mut next = self.shared.next.lock().unwrap_or_else(|e| e.into_inner());
            *next += 1;
            *next
        };
        self.shared
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, Box::new(f)));
        Subscription {
            shared: Arc::downgrade(&self.shared),
            id,
        }
    }

    /// Every new snapshot on a channel, for code that would rather poll.
    pub fn subscribe(&self) -> (Receiver<Arc<State>>, Subscription) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sub = self.on_change(move |s| {
            let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(s.clone());
        });
        (rx, sub)
    }
}

/// Keeps an [`Audio::on_change`] listener registered; dropping it unregisters.
pub struct Subscription {
    shared: std::sync::Weak<Shared>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            shared
                .listeners
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|(id, _)| *id != self.id);
        }
    }
}

/// The backend's side: publishes snapshots and receives commands.
pub struct Driver {
    shared: Arc<Shared>,
    commands: Receiver<Command>,
}

impl Driver {
    /// Replaces the snapshot (setting its `revision`) and notifies every listener.
    pub fn publish(&self, mut state: State) {
        let state = {
            let mut current = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.revision = current.revision + 1;
            let state = Arc::new(state);
            *current = state.clone();
            state
        };
        for (_, f) in self
            .shared
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            f(&state);
        }
    }

    /// Says the backend stopped, for after it panicked and dropped this driver.
    pub fn on_stop(&self) -> impl Fn(String) + Send + 'static {
        let shared = self.shared.clone();
        move |why: String| {
            Driver {
                shared: shared.clone(),
                commands: mpsc::channel().1,
            }
            .publish(State {
                connection: Connection::Lost(why),
                ..State::default()
            })
        }
    }

    /// The latest published snapshot.
    pub fn current(&self) -> Arc<State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Commands from the UI, in order. Disconnected once every [`Audio`] is dropped.
    pub fn commands(&self) -> &Receiver<Command> {
        &self.commands
    }

    /// Takes the command receiver, for backends that wait on commands on another thread (and
    /// forward them into their own event loop). `commands()` is empty afterwards.
    pub fn take_commands(&mut self) -> Receiver<Command> {
        std::mem::replace(&mut self.commands, mpsc::channel().1)
    }
}

/// A connected pair: give [`Audio`] to the UI and [`Driver`] to the backend's thread.
pub fn channel() -> (Audio, Driver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(Arc::new(State::default())),
        listeners: Mutex::new(Vec::new()),
        next: Mutex::new(0),
    });
    let (tx, rx) = mpsc::channel();
    (
        Audio {
            shared: shared.clone(),
            commands: tx,
        },
        Driver {
            shared,
            commands: rx,
        },
    )
}

/// Which backend the Sound module runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// PipeWire (`crate::sound::pipewire`).
    PipeWire,
    /// Fake devices and apps with animated levels (`crate::sound::mock`):
    /// `HOKUM_SOUND_BACKEND=mock`, and every headless scene.
    Mock,
}

/// Starts the chosen backend on its own thread.
pub fn start(source: Source) -> Result<Audio, String> {
    match source {
        Source::PipeWire => crate::sound::pipewire::start(),
        Source::Mock => Ok(crate::sound::mock::start()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_volume_keeps_the_balance() {
        let v = Volume {
            channels: vec![0.5, 1.0],
            positions: vec![ChannelPosition::FL, ChannelPosition::FR],
        };
        assert_eq!(v.combined(), 1.0);
        assert!((v.balance() - 0.5).abs() < 1e-6);
        let half = v.with_combined(0.5);
        assert_eq!(half.channels, vec![0.25, 0.5]);
        assert!((half.balance() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn balance_quietens_the_far_side() {
        let v = Volume::stereo(0.8).with_balance(-0.5);
        assert_eq!(v.channels, vec![0.8, 0.4]);
        assert!((v.balance() + 0.5).abs() < 1e-6);
        assert_eq!(Volume::mono(0.3).balance(), 0.0);
    }

    #[test]
    fn listeners_see_publishes_until_dropped() {
        let (audio, driver) = channel();
        let (rx, sub) = audio.subscribe();
        driver.publish(State::default());
        assert_eq!(rx.recv().unwrap().revision, 1);
        assert_eq!(audio.state().revision, 1);
        drop(sub);
        driver.publish(State::default());
        assert!(rx.try_recv().is_err());
        audio.send(Command::SetDefault { device: 3 });
        assert_eq!(
            driver.commands().recv().unwrap(),
            Command::SetDefault { device: 3 }
        );
    }
}
