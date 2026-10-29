//! The Bluetooth model: the contract between a backend (`crate::bluetooth::bluez` on BlueZ over D-Bus,
//! `crate::bluetooth::mock` for scenes and UI work) and the UI (`crate::ui`).
//!
//! A backend owns the connection and publishes plain-data [`State`] snapshots; the UI reads the
//! latest one, is told when a new one lands, and asks for changes with [`Command`]s. Nothing in
//! here touches zbus or D-Bus types, and nothing in the UI touches D-Bus.
//!
//! ```text
//! backend thread ── Driver::publish(State) ──▶ Bluetooth::state() / on_change(..) ──▶ UI
//!        ▲                                                                         │
//!        └────────────── Driver::commands() ◀── Bluetooth::send(Command) ◀──────────┘
//! ```
//!
//! Rules both sides keep:
//! - A snapshot is complete and consistent: every id it mentions exists in it (a device's
//!   adapter, `default_adapter`, the pairing request's device).
//! - Commands are requests, not changes. The UI never edits its copy of [`State`]; it waits for
//!   the backend to publish the result. While a slow call runs (connect, pair, forget) the
//!   backend marks the device [`Device::pending`] so the UI can show it working.
//! - Failures arrive as [`Issue`]s in plain words ("Couldn't connect to AirPods Pro").
//! - Ids are BlueZ object paths (`/org/bluez/hci0`, `/org/bluez/hci0/dev_AC_80_0A_12_34_56`):
//!   stable while the object exists and across restarts for paired devices.
//! - The backend registers the pairing agent (`org.bluez.Agent1`, capability
//!   `KeyboardDisplay`) as the default agent and turns every agent call into
//!   [`State::pairing`]; the UI answers with [`Command::PairingReply`].

use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// An adapter's D-Bus object path (`/org/bluez/hci0`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AdapterId(pub String);

/// A device's D-Bus object path (`/org/bluez/hci0/dev_AC_80_0A_12_34_56`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub String);

impl fmt::Display for AdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Everything the UI shows, at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Bumped by the backend on every publish; lets the UI skip identical work.
    pub revision: u64,
    pub connection: Connection,
    /// Every adapter (`org.bluez.Adapter1`), in a stable order (by path).
    pub adapters: Vec<Adapter>,
    /// The adapter the UI shows and acts on: the one picked with [`Command::SelectAdapter`],
    /// else the first powered one, else the first. `None` only when there are no adapters.
    pub default_adapter: Option<AdapterId>,
    /// Every device BlueZ knows on every adapter (paired, connected, or seen while
    /// discovering), in a stable order: connected first, then paired, then nearby by signal,
    /// then by name.
    pub devices: Vec<Device>,
    /// The pairing agent is waiting on the user. At most one at a time (BlueZ serialises
    /// agent calls); gone once answered, cancelled or timed out.
    pub pairing: Option<PairingRequest>,
    /// Things worth telling the user about (a failed command, rfkill, BlueZ going away).
    pub issues: Vec<Issue>,
}

impl State {
    pub fn adapter(&self, id: &AdapterId) -> Option<&Adapter> {
        self.adapters.iter().find(|a| &a.id == id)
    }

    pub fn device(&self, id: &DeviceId) -> Option<&Device> {
        self.devices.iter().find(|d| &d.id == id)
    }

    /// The adapter the UI shows.
    pub fn default_adapter(&self) -> Option<&Adapter> {
        self.default_adapter
            .as_ref()
            .and_then(|id| self.adapter(id))
    }

    /// Devices of the default adapter.
    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        let adapter = self.default_adapter.clone();
        self.devices
            .iter()
            .filter(move |d| Some(&d.adapter) == adapter.as_ref())
    }

    /// Connected devices of the default adapter: level one's first list.
    pub fn connected(&self) -> impl Iterator<Item = &Device> {
        self.devices().filter(|d| d.connected)
    }

    /// Paired but not connected devices of the default adapter: level one's second list.
    pub fn paired(&self) -> impl Iterator<Item = &Device> {
        self.devices().filter(|d| d.is_paired() && !d.connected)
    }

    /// Unpaired devices of the default adapter that BlueZ has seen (the scan list). Only the
    /// ones with an `rssi` were heard in the current discovery.
    pub fn nearby(&self) -> impl Iterator<Item = &Device> {
        self.devices().filter(|d| !d.is_paired() && !d.connected)
    }
}

/// Whether the backend is talking to BlueZ.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Connection {
    #[default]
    Connecting,
    /// `org.bluez` is on the system bus. There may still be no adapter.
    Ready,
    /// The system bus works but `bluetoothd` isn't running (or isn't installed). The backend
    /// watches for it to appear.
    NotRunning,
    /// The system bus itself failed; the backend keeps retrying.
    Lost(String),
}

/// One controller (`org.bluez.Adapter1`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adapter {
    pub id: AdapterId,
    /// `Name`: the system name, e.g. the hostname ("romulus").
    pub name: String,
    /// `Alias`: what other devices see; equals `name` unless renamed.
    pub alias: String,
    /// `Address`, "AA:BB:CC:DD:EE:FF".
    pub address: String,
    pub address_type: AddressType,
    pub powered: bool,
    /// `PowerState`, which also says when rfkill blocks the radio.
    pub power_state: PowerState,
    pub discoverable: bool,
    /// Seconds discoverable stays on; 0 means until turned off.
    pub discoverable_timeout: u32,
    pub pairable: bool,
    /// Seconds pairable stays on; 0 means until turned off.
    pub pairable_timeout: u32,
    pub discovering: bool,
    /// `Class` of device the adapter announces (BR/EDR).
    pub class: u32,
    /// `Roles` it supports ("central", "peripheral", "central-peripheral").
    pub roles: Vec<String>,
    /// `Modalias` ("usb:v1D6Bp0246d0554").
    pub modalias: Option<String>,
    /// Services the adapter offers (`UUIDs`).
    pub services: Vec<Service>,
    /// Every Adapter1 property, name → rendered value, for the raw view.
    pub properties: Vec<(String, String)>,
}

impl Adapter {
    /// Alias, else name, else address.
    pub fn display_name(&self) -> &str {
        [&self.alias, &self.name]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or(&self.address)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PowerState {
    On,
    #[default]
    Off,
    /// `off-enabling`: turning on.
    TurningOn,
    /// `on-disabling`: turning off.
    TurningOff,
    /// `off-blocked`: rfkill (a hardware switch or airplane mode) holds it off;
    /// [`Command::SetPowered`] won't help until it's unblocked.
    Blocked,
}

/// `AddressType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AddressType {
    #[default]
    Public,
    /// A random (often rotating) LE address.
    Random,
}

/// How a device talks to us, derived from what BlueZ reports (`Class` means BR/EDR,
/// advertising data or an `Appearance` means LE).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bearer {
    /// Classic Bluetooth.
    BrEdr,
    /// Bluetooth Low Energy.
    Le,
    /// Both (most phones, many headphones).
    Dual,
    #[default]
    Unknown,
}

/// One remote device (`org.bluez.Device1`, plus `org.bluez.Battery1`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub id: DeviceId,
    pub adapter: AdapterId,
    /// `Address`, "AA:BB:CC:DD:EE:FF".
    pub address: String,
    pub address_type: AddressType,
    pub bearer: Bearer,
    /// `Name` as the device reports it; `None` for devices that never said.
    pub name: Option<String>,
    /// `Alias`: what people read. BlueZ falls back to the name, then the address with
    /// dashes; [`Command::SetAlias`] with "" returns to that.
    pub alias: String,
    /// `Icon`, a freedesktop icon name BlueZ derives ("audio-headset", "input-mouse").
    pub icon: String,
    /// What it is, from `icon` (see [`Kind::from_icon`]), refined by `class`/`appearance`.
    pub kind: Kind,
    /// `Class` of device (BR/EDR).
    pub class: Option<u32>,
    /// `Appearance` (LE).
    pub appearance: Option<u16>,
    pub paired: bool,
    /// Keys are stored (`Bonded`); a bonded device reconnects without pairing again.
    pub bonded: bool,
    /// May connect and use services without asking.
    pub trusted: bool,
    /// Refused: BlueZ rejects every connection from it.
    pub blocked: bool,
    pub connected: bool,
    /// `ServicesResolved`: connected and its services are known.
    pub services_resolved: bool,
    /// Pairs with a PIN code (pre-2.1 devices), not SSP.
    pub legacy_pairing: bool,
    /// Battery percentage (`org.bluez.Battery1.Percentage`), when it reports one.
    pub battery: Option<u8>,
    /// Signal strength in dBm, only while heard in the current discovery.
    pub rssi: Option<i16>,
    /// Advertised transmit power in dBm.
    pub tx_power: Option<i16>,
    /// Profiles and services it offers (`UUIDs`), with friendly names.
    pub services: Vec<Service>,
    /// Plays or records sound (A2DP, HFP/HSP, LE Audio): the UI hands codec and profile
    /// choices to the Sound section rather than doing them here.
    pub audio: bool,
    /// A slow call this app started and is still waiting on.
    pub pending: Option<Pending>,
    /// Advertising and identity details for the raw view.
    pub raw: Raw,
}

impl Device {
    /// What to call it: the alias (which BlueZ keeps filled in).
    pub fn display_name(&self) -> &str {
        if self.alias.is_empty() {
            self.name.as_deref().unwrap_or(&self.address)
        } else {
            &self.alias
        }
    }

    /// Paired or bonded: shows under "paired" and connects with one click.
    pub fn is_paired(&self) -> bool {
        self.paired || self.bonded
    }

    pub fn service(&self, uuid: &str) -> Option<&Service> {
        self.services
            .iter()
            .find(|s| s.uuid.eq_ignore_ascii_case(uuid))
    }
}

/// A slow call in flight for a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Connecting,
    Disconnecting,
    Pairing,
    /// Forgetting it ([`Command::Remove`]).
    Removing,
    /// Connecting or disconnecting one profile.
    Profile,
}

/// What a device is, for its icon and grouping. From BlueZ's `Icon` string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    Headphones,
    /// Headphones with a microphone (`audio-headset`).
    Headset,
    /// Speakers and other audio sinks (`audio-card`).
    Speaker,
    Phone,
    Computer,
    Tablet,
    Keyboard,
    Mouse,
    Gamepad,
    /// Drawing tablets (`input-tablet`).
    Pen,
    Watch,
    Display,
    Camera,
    Printer,
    Scanner,
    /// Media players (`multimedia-player`).
    Player,
    /// Modems and access points (`modem`, `network-wireless`).
    Network,
    #[default]
    Other,
}

impl Kind {
    /// BlueZ's icon names (`src/dbus-common.c` class/appearance → icon) to a kind.
    pub fn from_icon(icon: &str) -> Kind {
        match icon {
            "audio-headphones" => Kind::Headphones,
            "audio-headset" => Kind::Headset,
            "audio-card" | "audio-speakers" => Kind::Speaker,
            "phone" => Kind::Phone,
            "computer" => Kind::Computer,
            "input-keyboard" => Kind::Keyboard,
            "input-mouse" => Kind::Mouse,
            "input-gaming" => Kind::Gamepad,
            "input-tablet" => Kind::Pen,
            "video-display" => Kind::Display,
            "camera-video" | "camera-photo" => Kind::Camera,
            "printer" => Kind::Printer,
            "scanner" => Kind::Scanner,
            "multimedia-player" => Kind::Player,
            "modem" | "network-wireless" => Kind::Network,
            _ => Kind::Other,
        }
    }

    /// A freedesktop icon name for the kind, for when the device's own is missing.
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Headphones => "audio-headphones",
            Kind::Headset => "audio-headset",
            Kind::Speaker => "audio-speakers",
            Kind::Phone => "phone",
            Kind::Computer => "computer",
            Kind::Tablet => "tablet",
            Kind::Keyboard => "input-keyboard",
            Kind::Mouse => "input-mouse",
            Kind::Gamepad => "input-gaming",
            Kind::Pen => "input-tablet",
            Kind::Watch => "watch",
            Kind::Display => "video-display",
            Kind::Camera => "camera-photo",
            Kind::Printer => "printer",
            Kind::Scanner => "scanner",
            Kind::Player => "multimedia-player",
            Kind::Network => "network-wireless",
            Kind::Other => "bluetooth",
        }
    }
}

/// A profile or GATT service a device offers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Service {
    /// Lowercase 128-bit UUID ("0000110b-0000-1000-8000-00805f9b34fb").
    pub uuid: String,
    /// Friendly name ("Audio Sink"), else the short form ("0xfe2c"), else the UUID.
    pub name: String,
    /// Carries sound.
    pub audio: bool,
}

impl Service {
    pub fn new(uuid: &str) -> Service {
        let uuid = uuid.to_ascii_lowercase();
        let (name, audio) = match service_name(&uuid) {
            Some((name, audio)) => (name.to_owned(), audio),
            None => (
                short_uuid(&uuid).map_or_else(|| uuid.clone(), |s| format!("0x{s:04x}")),
                false,
            ),
        };
        Service { uuid, name, audio }
    }
}

const BASE_UUID: &str = "-0000-1000-8000-00805f9b34fb";

/// The 16-bit (or 32-bit) number of a UUID on the Bluetooth base, if it is one.
pub fn short_uuid(uuid: &str) -> Option<u32> {
    let head = uuid.strip_suffix(BASE_UUID)?;
    u32::from_str_radix(head, 16).ok()
}

/// Friendly names for the profiles and services people meet, and whether they carry sound.
pub fn service_name(uuid: &str) -> Option<(&'static str, bool)> {
    Some(match short_uuid(uuid)? {
        0x1101 => ("Serial Port", false),
        0x1103 => ("Dial-up Networking", false),
        0x1105 => ("Object Push", false),
        0x1106 => ("File Transfer", false),
        0x1108 => ("Headset", true),
        0x110a => ("Audio Source", true),
        0x110b => ("Audio Sink", true),
        0x110c => ("Remote Control Target", false),
        0x110d => ("Advanced Audio", true),
        0x110e => ("Remote Control", false),
        0x110f => ("Remote Control Controller", false),
        0x1112 => ("Headset Gateway", true),
        0x1115 => ("Personal Area Network", false),
        0x1116 => ("Network Access Point", false),
        0x1117 => ("Group Network", false),
        0x111e => ("Handsfree", true),
        0x111f => ("Handsfree Gateway", true),
        0x1124 => ("Human Interface Device", false),
        0x112d => ("SIM Access", false),
        0x112f => ("Phonebook Access", false),
        0x1132 => ("Message Access", false),
        0x1133 => ("Message Notification", false),
        0x1134 => ("Message Access Profile", false),
        0x1200 => ("PnP Information", false),
        0x1203 => ("Generic Audio", true),
        0x1800 => ("Generic Access", false),
        0x1801 => ("Generic Attribute", false),
        0x180a => ("Device Information", false),
        0x180d => ("Heart Rate", false),
        0x180f => ("Battery", false),
        0x1812 => ("Human Interface Device (LE)", false),
        0x1813 => ("Scan Parameters", false),
        0x1844 => ("Volume Control", true),
        0x1845 => ("Volume Offset Control", true),
        0x1846 => ("Coordinated Set", false),
        0x1848 => ("Media Control", false),
        0x1849 => ("Generic Media Control", false),
        0x184e => ("Audio Stream Control", true),
        0x184f => ("Broadcast Audio Scan", true),
        0x1850 => ("Published Audio Capabilities", true),
        0x1853 => ("Common Audio", true),
        0x1854 => ("Hearing Access", true),
        0xfe2c => ("Google Fast Pair", false),
        0xfd6f => ("Exposure Notification", false),
        _ => return None,
    })
}

/// A Bluetooth SIG company id's name, for the common ones in manufacturer data.
pub fn company_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x0006 => "Microsoft",
        0x000a => "Qualcomm",
        0x000f => "Broadcom",
        0x001d => "Qualcomm",
        0x0046 => "MediaTek",
        0x004c => "Apple",
        0x0057 => "Harman",
        0x0059 => "Nordic Semiconductor",
        0x0075 => "Samsung",
        0x0087 => "Garmin",
        0x009e => "Bose",
        0x00e0 => "Google",
        0x012d => "Sony",
        0x0171 => "Amazon",
        0x01da => "Logitech",
        0x02e5 => "Espressif",
        0x038f => "Xiaomi",
        _ => return None,
    })
}

/// Advertising and identity details for the raw view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Raw {
    /// `Modalias` ("usb:v046DpB034d0007", "bluetooth:v004Cp2014d0E10").
    pub modalias: Option<String>,
    /// `ManufacturerData`, by company.
    pub manufacturer_data: Vec<ManufacturerData>,
    /// `ServiceData`, by service UUID.
    pub service_data: Vec<ServiceData>,
    /// `AdvertisingFlags`.
    pub advertising_flags: Vec<u8>,
    /// Every Device1 (and Battery1) property, name → rendered value.
    pub properties: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ManufacturerData {
    pub company: u16,
    /// [`company_name`], when known.
    pub company_name: Option<String>,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceData {
    pub service: Service,
    pub data: Vec<u8>,
}

/// The pairing agent asks the user something.
#[derive(Clone, Debug, PartialEq)]
pub struct PairingRequest {
    /// Unique per request: replies name it, so a late reply never answers a newer one.
    pub id: u64,
    pub device: DeviceId,
    pub kind: PairingKind,
}

/// The agent call behind a [`PairingRequest`] and how to answer it with [`Reply`].
#[derive(Clone, Debug, PartialEq)]
pub enum PairingKind {
    /// `RequestConfirmation`: does the device show this 6-digit passkey? Accept or Reject.
    Confirm { passkey: u32 },
    /// `DisplayPasskey`: type this passkey on the device (a keyboard). `entered` digits so
    /// far. Nothing to answer; cancel with [`Command::CancelPair`].
    DisplayPasskey { passkey: u32, entered: u16 },
    /// `DisplayPinCode`: type this PIN on the device. Nothing to answer; cancel with
    /// [`Command::CancelPair`].
    DisplayPin { pin: String },
    /// `RequestPinCode`: the PIN the device expects (legacy pairing, often "0000"). Answer
    /// with `Reply::Pin` or Reject.
    RequestPin,
    /// `RequestPasskey`: the 6-digit passkey the device shows. Answer with `Reply::Passkey`
    /// or Reject.
    RequestPasskey,
    /// `RequestAuthorization`: a device asks to pair with no code ("just works", incoming).
    /// Accept or Reject.
    AuthorizePairing,
    /// `AuthorizeService`: a paired but untrusted device asks to use a service. Accept or
    /// Reject (trusting it stops the asking).
    Authorize { service: Service },
}

/// The UI's answer to a [`PairingRequest`].
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Accept,
    Reject,
    Pin(String),
    Passkey(u32),
}

/// Something to tell the user.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    /// Unique per issue, for [`Command::DismissIssue`].
    pub id: u64,
    pub severity: Severity,
    /// Short, plain words ("Couldn't connect to AirPods Pro").
    pub message: String,
    /// The device it's about, if any.
    pub device: Option<DeviceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// A change the UI asks the backend for. The result arrives as a new [`State`]; failures as an
/// [`Issue`].
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Turns the radio on or off. Remembered by BlueZ across restarts.
    SetPowered {
        adapter: AdapterId,
        on: bool,
    },
    SetDiscoverable {
        adapter: AdapterId,
        on: bool,
    },
    /// Seconds; 0 keeps it discoverable until turned off.
    SetDiscoverableTimeout {
        adapter: AdapterId,
        seconds: u32,
    },
    SetPairable {
        adapter: AdapterId,
        on: bool,
    },
    /// Seconds; 0 keeps it pairable until turned off.
    SetPairableTimeout {
        adapter: AdapterId,
        seconds: u32,
    },
    /// Renames the adapter as other devices see it; "" returns to the system name.
    SetAdapterAlias {
        adapter: AdapterId,
        alias: String,
    },
    /// Starts scanning (BR/EDR and LE). The UI starts it when the scan view opens and stops
    /// it when it closes; the backend also stops it when the UI goes away.
    StartDiscovery {
        adapter: AdapterId,
    },
    StopDiscovery {
        adapter: AdapterId,
    },
    /// Connects every auto-connectable profile (`Device1.Connect`).
    Connect {
        device: DeviceId,
    },
    Disconnect {
        device: DeviceId,
    },
    /// Pairs, then on success trusts and connects it: what people mean by "pair".
    /// Code prompts arrive as [`State::pairing`].
    Pair {
        device: DeviceId,
    },
    /// Abandons a pairing in progress (`CancelPairing`), including a displayed code.
    CancelPair {
        device: DeviceId,
    },
    Trust {
        device: DeviceId,
    },
    Untrust {
        device: DeviceId,
    },
    /// Refuses all connections from it (and disconnects it).
    Block {
        device: DeviceId,
    },
    Unblock {
        device: DeviceId,
    },
    /// Forgets it: unpairs and drops it from BlueZ (`Adapter1.RemoveDevice`).
    Remove {
        device: DeviceId,
    },
    /// Renames it locally; "" returns to the name it reports.
    SetAlias {
        device: DeviceId,
        alias: String,
    },
    /// Connects one profile by UUID (`Device1.ConnectProfile`).
    ConnectProfile {
        device: DeviceId,
        uuid: String,
    },
    DisconnectProfile {
        device: DeviceId,
        uuid: String,
    },
    /// Answers [`State::pairing`] with the same `id`; ignored if that request is gone.
    PairingReply {
        id: u64,
        reply: Reply,
    },
    /// Shows and acts on another adapter.
    SelectAdapter {
        adapter: AdapterId,
    },
    /// Drops an issue from `State::issues` once the user has seen it.
    DismissIssue {
        id: u64,
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
pub struct Bluetooth {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

impl Bluetooth {
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
    /// `move |s| { let s = s.clone(); handle.update(move |ui, _| ui.bt = s) }`.
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

/// Keeps a [`Bluetooth::on_change`] listener registered; dropping it unregisters.
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
    pub fn publish(&self, state: State) {
        Driver::publish_shared(&self.shared, state)
    }

    fn publish_shared(shared: &Shared, mut state: State) {
        let state = {
            let mut current = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.revision = current.revision + 1;
            let state = Arc::new(state);
            *current = state.clone();
            state
        };
        for (_, f) in shared
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            f(&state);
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

    /// Commands from the UI, in order. Disconnected once every [`Bluetooth`] is dropped.
    pub fn commands(&self) -> &Receiver<Command> {
        &self.commands
    }

    /// Splits into a [`Publisher`] (shareable across threads) and the command receiver, for a
    /// backend that waits on commands on one thread and publishes from another.
    pub fn split(self) -> (Publisher, Receiver<Command>) {
        (
            Publisher {
                shared: self.shared,
            },
            self.commands,
        )
    }
}

/// The publishing half of a [`Driver`] (see [`Driver::split`]).
#[derive(Clone)]
pub struct Publisher {
    shared: Arc<Shared>,
}

impl Publisher {
    /// Same as [`Driver::publish`].
    pub fn publish(&self, state: State) {
        Driver::publish_shared(&self.shared, state)
    }

    pub fn current(&self) -> Arc<State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// A connected pair: give [`Bluetooth`] to the UI and [`Driver`] to the backend's thread.
pub fn channel() -> (Bluetooth, Driver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(Arc::new(State::default())),
        listeners: Mutex::new(Vec::new()),
        next: Mutex::new(0),
    });
    let (tx, rx) = mpsc::channel();
    (
        Bluetooth {
            shared: shared.clone(),
            commands: tx,
        },
        Driver {
            shared,
            commands: rx,
        },
    )
}

/// Which backend the Bluetooth module runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// BlueZ on the system bus (`crate::bluetooth::bluez`).
    Bluez,
    /// Fake adapter and devices (`crate::bluetooth::mock`): `HOKUM_BLUETOOTH_BACKEND=mock`,
    /// and every headless scene.
    Mock,
}

/// Starts the chosen backend on its own thread.
pub fn start(source: Source) -> Result<Bluetooth, String> {
    match source {
        Source::Bluez => crate::bluetooth::bluez::start(),
        Source::Mock => Ok(crate::bluetooth::mock::start()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_get_friendly_names() {
        let s = Service::new("0000110B-0000-1000-8000-00805F9B34FB");
        assert_eq!(s.uuid, "0000110b-0000-1000-8000-00805f9b34fb");
        assert_eq!(s.name, "Audio Sink");
        assert!(s.audio);
        assert_eq!(
            Service::new("0000abcd-0000-1000-8000-00805f9b34fb").name,
            "0xabcd"
        );
        let custom = "f000aa00-0451-4000-b000-000000000000";
        assert_eq!(Service::new(custom).name, custom);
    }

    #[test]
    fn icons_map_to_kinds() {
        assert_eq!(Kind::from_icon("audio-headset"), Kind::Headset);
        assert_eq!(Kind::from_icon("input-mouse"), Kind::Mouse);
        assert_eq!(Kind::from_icon(""), Kind::Other);
    }

    #[test]
    fn listeners_see_publishes_until_dropped() {
        let (bt, driver) = channel();
        let (rx, sub) = bt.subscribe();
        driver.publish(State::default());
        assert_eq!(rx.recv().unwrap().revision, 1);
        assert_eq!(bt.state().revision, 1);
        drop(sub);
        driver.publish(State::default());
        assert!(rx.try_recv().is_err());
        let device = DeviceId("/org/bluez/hci0/dev_00_11_22_33_44_55".into());
        bt.send(Command::Connect {
            device: device.clone(),
        });
        assert_eq!(
            driver.commands().recv().unwrap(),
            Command::Connect { device }
        );
    }
}
