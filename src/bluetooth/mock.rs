//! A fake Bluetooth stack for UI work and headless scenes: one adapter; connected AirPods Pro
//! (battery), an MX Master 3S mouse (battery) and a Keychron keyboard; a paired Pixel phone and
//! JBL speaker that are off; and, while discovering, nearby devices that appear over a few
//! seconds. Commands take as long as they would on real hardware, pairing asks for codes the
//! way each kind of device does, and a few things fail so issues show:
//!
//! - Galaxy Buds2 Pro and the LG TV ask to confirm a passkey; the Keychron K3 shows a passkey
//!   to type (the digits fill in); the Xbox controller pairs with no code.
//! - The unnamed LE device fails to pair (authentication failed).
//! - The JBL speaker is out of range: connecting to it fails.
//!
//! `HOKUM_BLUETOOTH_BACKEND=mock` runs Hokum on it.

use std::time::{Duration, Instant};

use crate::bluetooth::model::*;

const HCI0: &str = "/org/bluez/hci0";

const AIRPODS: &str = "AC:80:0A:12:34:56";
const MOUSE: &str = "E4:5F:01:7A:22:C9";
const KEYBOARD: &str = "DC:2C:26:0B:41:8E";
const PHONE: &str = "58:24:29:9C:10:F3";
const SPEAKER: &str = "B8:F6:53:04:6E:2A";

const STRANGER: &str = "5C:3A:91:0E:22:7F";
const CONTROLLER: &str = "98:7A:14:55:C0:11";
const BUDS: &str = "64:03:7F:A1:9B:30";
const NEW_KEYBOARD: &str = "C8:6F:1D:42:7E:05";
const TV: &str = "A4:36:C7:88:12:6D";

/// When each nearby device is first heard after discovery starts, and its usual signal.
const NEARBY: [(&str, u64, i16); 5] = [
    (STRANGER, 600, -88),
    (CONTROLLER, 1200, -52),
    (BUDS, 2000, -61),
    (NEW_KEYBOARD, 3000, -67),
    (TV, 4500, -74),
];

const A2DP_SINK: &str = "0000110b-0000-1000-8000-00805f9b34fb";
const A2DP_SOURCE: &str = "0000110a-0000-1000-8000-00805f9b34fb";
const AVRCP: &str = "0000110e-0000-1000-8000-00805f9b34fb";
const AVRCP_TARGET: &str = "0000110c-0000-1000-8000-00805f9b34fb";
const HFP: &str = "0000111e-0000-1000-8000-00805f9b34fb";
const HFP_GATEWAY: &str = "0000111f-0000-1000-8000-00805f9b34fb";
const HSP: &str = "00001108-0000-1000-8000-00805f9b34fb";
const HID: &str = "00001124-0000-1000-8000-00805f9b34fb";
const HOGP: &str = "00001812-0000-1000-8000-00805f9b34fb";
const BATTERY: &str = "0000180f-0000-1000-8000-00805f9b34fb";
const DEVICE_INFO: &str = "0000180a-0000-1000-8000-00805f9b34fb";
const GAP: &str = "00001800-0000-1000-8000-00805f9b34fb";
const GATT: &str = "00001801-0000-1000-8000-00805f9b34fb";
const PNP: &str = "00001200-0000-1000-8000-00805f9b34fb";
const PAN_NAP: &str = "00001116-0000-1000-8000-00805f9b34fb";
const OPP: &str = "00001105-0000-1000-8000-00805f9b34fb";
const PBAP: &str = "0000112f-0000-1000-8000-00805f9b34fb";
const MAP: &str = "00001132-0000-1000-8000-00805f9b34fb";
const FAST_PAIR: &str = "0000fe2c-0000-1000-8000-00805f9b34fb";

pub fn adapter_id() -> AdapterId {
    AdapterId(HCI0.into())
}

/// The device id BlueZ would give `address` on hci0.
pub fn device_id(address: &str) -> DeviceId {
    DeviceId(format!("{HCI0}/dev_{}", address.replace(':', "_")))
}

fn adapter() -> Adapter {
    let services: Vec<Service> = [
        GAP,
        GATT,
        DEVICE_INFO,
        A2DP_SINK,
        A2DP_SOURCE,
        AVRCP,
        AVRCP_TARGET,
        HFP_GATEWAY,
        PNP,
    ]
    .into_iter()
    .map(Service::new)
    .collect();
    let mut a = Adapter {
        id: adapter_id(),
        name: "romulus".into(),
        alias: "romulus".into(),
        address: "F4:26:79:3A:5C:01".into(),
        address_type: AddressType::Public,
        powered: true,
        power_state: PowerState::On,
        discoverable: false,
        discoverable_timeout: 180,
        pairable: true,
        pairable_timeout: 0,
        discovering: false,
        class: 0x6c010c,
        roles: vec![
            "central".into(),
            "peripheral".into(),
            "central-peripheral".into(),
        ],
        modalias: Some("usb:v1D6Bp0246d0554".into()),
        services,
        properties: Vec::new(),
    };
    a.properties = adapter_properties(&a);
    a
}

fn adapter_properties(a: &Adapter) -> Vec<(String, String)> {
    vec![
        ("Address".into(), a.address.clone()),
        ("AddressType".into(), "public".into()),
        ("Name".into(), a.name.clone()),
        ("Alias".into(), a.alias.clone()),
        ("Class".into(), format!("0x{:06x}", a.class)),
        ("Powered".into(), a.powered.to_string()),
        ("Discoverable".into(), a.discoverable.to_string()),
        (
            "DiscoverableTimeout".into(),
            a.discoverable_timeout.to_string(),
        ),
        ("Pairable".into(), a.pairable.to_string()),
        ("PairableTimeout".into(), a.pairable_timeout.to_string()),
        ("Discovering".into(), a.discovering.to_string()),
        ("Modalias".into(), a.modalias.clone().unwrap_or_default()),
        ("Roles".into(), a.roles.join(", ")),
    ]
}

struct Spec<'a> {
    address: &'a str,
    name: Option<&'a str>,
    icon: &'a str,
    class: Option<u32>,
    appearance: Option<u16>,
    random: bool,
    bearer: Bearer,
    uuids: &'a [&'a str],
    modalias: Option<&'a str>,
    company: Option<(u16, &'a [u8])>,
}

fn device(s: Spec) -> Device {
    let services: Vec<Service> = s.uuids.iter().copied().map(Service::new).collect();
    let audio = services.iter().any(|x| x.audio);
    let mut kind = Kind::from_icon(s.icon);
    if kind == Kind::Other && s.appearance.is_some_and(|a| a >> 6 == 0x003) {
        kind = Kind::Watch;
    }
    Device {
        id: device_id(s.address),
        adapter: adapter_id(),
        address: s.address.into(),
        address_type: if s.random {
            AddressType::Random
        } else {
            AddressType::Public
        },
        bearer: s.bearer,
        name: s.name.map(Into::into),
        alias: s
            .name
            .map_or_else(|| s.address.replace(':', "-"), Into::into),
        icon: s.icon.into(),
        kind,
        class: s.class,
        appearance: s.appearance,
        services,
        audio,
        raw: Raw {
            modalias: s.modalias.map(Into::into),
            manufacturer_data: s
                .company
                .map(|(company, data)| ManufacturerData {
                    company,
                    company_name: company_name(company).map(Into::into),
                    data: data.to_vec(),
                })
                .into_iter()
                .collect(),
            ..Raw::default()
        },
        ..Device::default()
    }
}

fn paired(mut d: Device, connected: bool, battery: Option<u8>) -> Device {
    d.paired = true;
    d.bonded = true;
    d.trusted = true;
    d.connected = connected;
    d.services_resolved = connected;
    d.battery = battery;
    d
}

/// Everything the mock can show, nearby devices included (not yet heard).
fn nearby_device(address: &str) -> Device {
    let mut d = match address {
        STRANGER => device(Spec {
            address: STRANGER,
            name: None,
            icon: "",
            class: None,
            appearance: None,
            random: true,
            bearer: Bearer::Le,
            uuids: &[],
            modalias: None,
            company: Some((0x004c, &[0x10, 0x05, 0x0b, 0x1c, 0x8a, 0x4f, 0x21])),
        }),
        CONTROLLER => device(Spec {
            address: CONTROLLER,
            name: Some("Xbox Wireless Controller"),
            icon: "input-gaming",
            class: None,
            appearance: Some(0x03c4),
            random: false,
            bearer: Bearer::Le,
            uuids: &[HOGP, BATTERY, DEVICE_INFO],
            modalias: Some("usb:v045Ep0B13d0517"),
            company: Some((0x0006, &[0x03, 0x00, 0x80])),
        }),
        BUDS => device(Spec {
            address: BUDS,
            name: Some("Galaxy Buds2 Pro"),
            icon: "audio-headset",
            class: Some(0x240404),
            appearance: None,
            random: false,
            bearer: Bearer::Dual,
            uuids: &[A2DP_SINK, AVRCP, HFP, HSP, FAST_PAIR],
            modalias: Some("bluetooth:v0075pA013d0001"),
            company: Some((0x0075, &[0x42, 0x09, 0x81, 0x02, 0x14])),
        }),
        NEW_KEYBOARD => device(Spec {
            address: NEW_KEYBOARD,
            name: Some("Keychron K3"),
            icon: "input-keyboard",
            class: Some(0x002540),
            appearance: None,
            random: false,
            bearer: Bearer::BrEdr,
            uuids: &[HID, PNP],
            modalias: Some("usb:v05ACp024Fd0001"),
            company: None,
        }),
        _ => device(Spec {
            address: TV,
            name: Some("[LG] webOS TV OLED55C3"),
            icon: "video-display",
            class: Some(0x08043c),
            appearance: None,
            random: false,
            bearer: Bearer::Dual,
            uuids: &[A2DP_SOURCE, AVRCP_TARGET, PNP],
            modalias: None,
            company: None,
        }),
    };
    d.legacy_pairing = false;
    d
}

/// The fake system at rest: what most scenes render.
pub fn sample() -> State {
    let airpods = paired(
        device(Spec {
            address: AIRPODS,
            name: Some("AirPods Pro"),
            icon: "audio-headphones",
            class: Some(0x240418),
            appearance: None,
            random: false,
            bearer: Bearer::Dual,
            uuids: &[
                A2DP_SINK,
                AVRCP,
                AVRCP_TARGET,
                HFP,
                "74ec2172-0bad-4d01-8f77-997b2be0722a",
            ],
            modalias: Some("bluetooth:v004Cp2014d0E10"),
            company: Some((0x004c, &[0x07, 0x19, 0x01, 0x14, 0x20, 0x2b, 0x99, 0x8f])),
        }),
        true,
        Some(82),
    );
    let mouse = paired(
        device(Spec {
            address: MOUSE,
            name: Some("MX Master 3S"),
            icon: "input-mouse",
            class: None,
            appearance: Some(0x03c2),
            random: true,
            bearer: Bearer::Le,
            uuids: &[
                GAP,
                GATT,
                DEVICE_INFO,
                BATTERY,
                HOGP,
                "00010000-0000-1000-8000-011f2000046d",
            ],
            modalias: Some("usb:v046DpB034d0007"),
            company: Some((0x01da, &[0x01, 0xb0, 0x34])),
        }),
        true,
        Some(64),
    );
    let keyboard = paired(
        device(Spec {
            address: KEYBOARD,
            name: Some("Keychron K2"),
            icon: "input-keyboard",
            class: Some(0x002540),
            appearance: None,
            random: false,
            bearer: Bearer::BrEdr,
            uuids: &[HID, PNP],
            modalias: Some("usb:v05ACp024Fd0001"),
            company: None,
        }),
        true,
        None,
    );
    let phone = paired(
        device(Spec {
            address: PHONE,
            name: Some("Pixel 8"),
            icon: "phone",
            class: Some(0x5a020c),
            appearance: None,
            random: false,
            bearer: Bearer::Dual,
            uuids: &[
                A2DP_SOURCE,
                AVRCP_TARGET,
                HFP_GATEWAY,
                PAN_NAP,
                OPP,
                PBAP,
                MAP,
                PNP,
            ],
            modalias: Some("bluetooth:v00E0p1200d1436"),
            company: None,
        }),
        false,
        None,
    );
    let speaker = paired(
        device(Spec {
            address: SPEAKER,
            name: Some("JBL Flip 6"),
            icon: "audio-card",
            class: Some(0x240414),
            appearance: None,
            random: false,
            bearer: Bearer::BrEdr,
            uuids: &[A2DP_SINK, AVRCP, HFP, PNP],
            modalias: Some("bluetooth:v0057p2201d0100"),
            company: None,
        }),
        false,
        None,
    );
    let mut state = State {
        revision: 0,
        connection: Connection::Ready,
        adapters: vec![adapter()],
        default_adapter: Some(adapter_id()),
        devices: vec![airpods, mouse, keyboard, phone, speaker],
        pairing: None,
        issues: Vec::new(),
    };
    refresh(&mut state);
    state
}

/// Discovering, with every nearby device heard.
pub fn scanning() -> State {
    let mut s = sample();
    s.adapters[0].discovering = true;
    for (address, _, rssi) in NEARBY {
        let mut d = nearby_device(address);
        d.rssi = Some(rssi);
        s.devices.push(d);
    }
    refresh(&mut s);
    s
}

/// Pairing Galaxy Buds2 Pro: confirm the passkey.
pub fn pairing() -> State {
    let mut s = scanning();
    let buds = device_id(BUDS);
    if let Some(d) = s.devices.iter_mut().find(|d| d.id == buds) {
        d.pending = Some(Pending::Pairing);
    }
    s.pairing = Some(PairingRequest {
        id: 1,
        device: buds,
        kind: PairingKind::Confirm { passkey: 381904 },
    });
    s
}

/// The radio is off.
pub fn powered_off() -> State {
    let mut s = sample();
    set_powered(&mut s, false);
    refresh(&mut s);
    s
}

/// bluetoothd isn't running.
pub fn not_running() -> State {
    State {
        connection: Connection::NotRunning,
        ..State::default()
    }
}

fn set_powered(s: &mut State, on: bool) {
    let a = &mut s.adapters[0];
    a.powered = on;
    a.power_state = if on { PowerState::On } else { PowerState::Off };
    if !on {
        a.discovering = false;
        a.discoverable = false;
        for d in &mut s.devices {
            d.connected = false;
            d.services_resolved = false;
            d.rssi = None;
            d.pending = None;
        }
        s.pairing = None;
    }
}

/// Sorts devices the way the contract says and refreshes the raw property lists.
fn refresh(s: &mut State) {
    for a in &mut s.adapters {
        a.properties = adapter_properties(a);
    }
    for d in &mut s.devices {
        let mut p = vec![
            ("Address".into(), d.address.clone()),
            (
                "AddressType".into(),
                if d.address_type == AddressType::Random {
                    "random"
                } else {
                    "public"
                }
                .into(),
            ),
            ("Alias".into(), d.alias.clone()),
            ("Icon".into(), d.icon.clone()),
            ("Paired".into(), d.paired.to_string()),
            ("Bonded".into(), d.bonded.to_string()),
            ("Trusted".into(), d.trusted.to_string()),
            ("Blocked".into(), d.blocked.to_string()),
            ("Connected".into(), d.connected.to_string()),
            ("ServicesResolved".into(), d.services_resolved.to_string()),
            ("LegacyPairing".into(), d.legacy_pairing.to_string()),
            (
                "UUIDs".into(),
                d.services
                    .iter()
                    .map(|s| s.uuid.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ];
        if let Some(n) = &d.name {
            p.push(("Name".into(), n.clone()));
        }
        if let Some(c) = d.class {
            p.push(("Class".into(), format!("0x{c:06x}")));
        }
        if let Some(a) = d.appearance {
            p.push(("Appearance".into(), format!("0x{a:04x}")));
        }
        if let Some(r) = d.rssi {
            p.push(("RSSI".into(), r.to_string()));
        }
        if let Some(m) = &d.raw.modalias {
            p.push(("Modalias".into(), m.clone()));
        }
        if let Some(b) = d.battery {
            p.push(("Battery1.Percentage".into(), b.to_string()));
        }
        p.sort();
        d.raw.properties = p;
    }
    s.devices.sort_by(|a, b| {
        let rank = |d: &Device| {
            if d.connected {
                0
            } else if d.is_paired() {
                1
            } else {
                2
            }
        };
        rank(a)
            .cmp(&rank(b))
            .then(b.rssi.unwrap_or(i16::MIN).cmp(&a.rssi.unwrap_or(i16::MIN)))
            .then_with(|| {
                a.display_name()
                    .to_lowercase()
                    .cmp(&b.display_name().to_lowercase())
            })
    });
}

/// Something the mock does later, as hardware would.
#[derive(Clone, Debug, PartialEq)]
enum Event {
    Heard(&'static str),
    Connected(DeviceId),
    ConnectFailed(DeviceId),
    Disconnected(DeviceId),
    Prompt(DeviceId),
    Typed(u64),
    Paired(DeviceId),
    PairFailed(DeviceId),
    Removed(DeviceId),
    ProfileDone(DeviceId),
    Powered(bool),
    /// Signals drift while discovering.
    Drift,
}

/// The fake stack: a state, and what happens next on a timeline the caller drives.
pub struct Mock {
    pub state: State,
    queue: Vec<(Duration, Event)>,
    next_id: u64,
}

impl Mock {
    pub fn new(state: State) -> Mock {
        Mock {
            state,
            queue: Vec::new(),
            next_id: 1,
        }
    }

    fn later(&mut self, now: Duration, ms: u64, e: Event) {
        self.queue.push((now + Duration::from_millis(ms), e));
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn issue(&mut self, message: String, device: Option<DeviceId>) {
        let id = self.id();
        self.state.issues.push(Issue {
            id,
            severity: Severity::Error,
            message,
            device,
        });
    }

    fn dev(&mut self, id: &DeviceId) -> Option<&mut Device> {
        self.state.devices.iter_mut().find(|d| &d.id == id)
    }

    fn name(&self, id: &DeviceId) -> String {
        self.state
            .device(id)
            .map_or_else(String::new, |d| d.display_name().to_owned())
    }

    /// When the next scheduled thing happens.
    pub fn next_due(&self) -> Option<Duration> {
        self.queue.iter().map(|(t, _)| *t).min()
    }

    /// Applies `command` at time `now` (since the mock started).
    pub fn apply(&mut self, command: Command, now: Duration) {
        let powered = self.state.adapters[0].powered;
        match command {
            Command::SetPowered { on, .. } => {
                self.state.adapters[0].power_state = if on {
                    PowerState::TurningOn
                } else {
                    PowerState::TurningOff
                };
                self.later(now, 300, Event::Powered(on));
            }
            Command::SetDiscoverable { on, .. } => {
                self.state.adapters[0].discoverable = on && powered
            }
            Command::SetDiscoverableTimeout { seconds, .. } => {
                self.state.adapters[0].discoverable_timeout = seconds
            }
            Command::SetPairable { on, .. } => self.state.adapters[0].pairable = on,
            Command::SetPairableTimeout { seconds, .. } => {
                self.state.adapters[0].pairable_timeout = seconds
            }
            Command::SetAdapterAlias { alias, .. } => {
                let a = &mut self.state.adapters[0];
                a.alias = if alias.is_empty() {
                    a.name.clone()
                } else {
                    alias
                };
            }
            Command::StartDiscovery { .. } if powered && !self.state.adapters[0].discovering => {
                self.state.adapters[0].discovering = true;
                for (address, ms, _) in NEARBY {
                    self.later(now, ms, Event::Heard(address));
                }
                self.later(now, 500, Event::Drift);
            }
            Command::StartDiscovery { .. } => {}
            Command::StopDiscovery { .. } => {
                self.state.adapters[0].discovering = false;
                self.queue
                    .retain(|(_, e)| !matches!(e, Event::Heard(_) | Event::Drift));
                for d in &mut self.state.devices {
                    d.rssi = None;
                }
            }
            Command::Connect { device } => {
                if !powered {
                    return;
                }
                let Some(d) = self.dev(&device) else { return };
                if d.connected || d.blocked {
                    return;
                }
                d.pending = Some(Pending::Connecting);
                let e = if d.address == SPEAKER {
                    Event::ConnectFailed(device)
                } else {
                    Event::Connected(device)
                };
                self.later(
                    now,
                    if matches!(e, Event::ConnectFailed(_)) {
                        2500
                    } else {
                        700
                    },
                    e,
                );
            }
            Command::Disconnect { device } => {
                if let Some(d) = self.dev(&device)
                    && d.connected
                {
                    d.pending = Some(Pending::Disconnecting);
                    self.later(now, 400, Event::Disconnected(device));
                }
            }
            Command::Pair { device } => {
                if !powered {
                    return;
                }
                if let Some(d) = self.dev(&device)
                    && !d.is_paired()
                {
                    d.pending = Some(Pending::Pairing);
                    self.later(now, 600, Event::Prompt(device));
                }
            }
            Command::CancelPair { device } => {
                self.queue.retain(|(_, e)| !matches!(e, Event::Prompt(d) | Event::Paired(d) | Event::PairFailed(d) if *d == device));
                self.queue.retain(|(_, e)| !matches!(e, Event::Typed(_)));
                if self
                    .state
                    .pairing
                    .as_ref()
                    .is_some_and(|p| p.device == device)
                {
                    self.state.pairing = None;
                }
                if let Some(d) = self.dev(&device)
                    && d.pending == Some(Pending::Pairing)
                {
                    d.pending = None;
                }
            }
            Command::Trust { device } => {
                self.dev(&device).into_iter().for_each(|d| d.trusted = true)
            }
            Command::Untrust { device } => self
                .dev(&device)
                .into_iter()
                .for_each(|d| d.trusted = false),
            Command::Block { device } => self.dev(&device).into_iter().for_each(|d| {
                d.blocked = true;
                d.connected = false;
                d.services_resolved = false;
            }),
            Command::Unblock { device } => self
                .dev(&device)
                .into_iter()
                .for_each(|d| d.blocked = false),
            Command::Remove { device } => {
                if let Some(d) = self.dev(&device) {
                    d.pending = Some(Pending::Removing);
                    self.later(now, 300, Event::Removed(device));
                }
            }
            Command::SetAlias { device, alias } => {
                if let Some(d) = self.dev(&device) {
                    d.alias = if alias.is_empty() {
                        d.name
                            .clone()
                            .unwrap_or_else(|| d.address.replace(':', "-"))
                    } else {
                        alias
                    };
                }
            }
            Command::ConnectProfile { device, uuid }
            | Command::DisconnectProfile { device, uuid } => {
                if let Some(d) = self.dev(&device)
                    && d.service(&uuid).is_some()
                {
                    d.pending = Some(Pending::Profile);
                    self.later(now, 500, Event::ProfileDone(device));
                }
            }
            Command::PairingReply { id, reply } => {
                let Some(p) = self.state.pairing.clone().filter(|p| p.id == id) else {
                    return;
                };
                self.state.pairing = None;
                let ok = match (&p.kind, &reply) {
                    (_, Reply::Reject) => false,
                    (PairingKind::RequestPin, Reply::Pin(_))
                    | (PairingKind::RequestPasskey, Reply::Passkey(_)) => true,
                    (PairingKind::RequestPin | PairingKind::RequestPasskey, _) => false,
                    _ => true,
                };
                if ok {
                    let e = if p.device == device_id(STRANGER) {
                        Event::PairFailed(p.device)
                    } else {
                        Event::Paired(p.device)
                    };
                    self.later(now, 800, e);
                } else if let Some(d) = self.dev(&p.device) {
                    d.pending = None;
                }
            }
            Command::SelectAdapter { adapter } => {
                if self.state.adapter(&adapter).is_some() {
                    self.state.default_adapter = Some(adapter);
                }
            }
            Command::DismissIssue { id } => self.state.issues.retain(|i| i.id != id),
        }
        refresh(&mut self.state);
    }

    /// Runs everything due by `now`; true when the state changed.
    pub fn advance(&mut self, now: Duration) -> bool {
        let mut changed = false;
        loop {
            self.queue.sort_by_key(|(t, _)| *t);
            if self.queue.first().is_none_or(|(t, _)| *t > now) {
                break;
            }
            let (at, e) = self.queue.remove(0);
            self.happen(e, at);
            changed = true;
        }
        if changed {
            refresh(&mut self.state);
        }
        changed
    }

    fn happen(&mut self, e: Event, now: Duration) {
        match e {
            Event::Heard(address) => {
                let rssi = NEARBY.iter().find(|n| n.0 == address).map(|n| n.2);
                let id = device_id(address);
                match self.dev(&id) {
                    Some(d) => d.rssi = rssi,
                    None => {
                        let mut d = nearby_device(address);
                        d.rssi = rssi;
                        self.state.devices.push(d);
                    }
                }
            }
            Event::Drift => {
                let t = now.as_millis() as f32 / 1000.0;
                for (address, _, base) in NEARBY {
                    let id = device_id(address);
                    if let Some(d) = self.dev(&id)
                        && d.rssi.is_some()
                    {
                        d.rssi = Some(base + (3.0 * (t * 1.7 + base as f32).sin()) as i16);
                    }
                }
                self.later(now, 500, Event::Drift);
            }
            Event::Connected(id) => {
                if let Some(d) = self.dev(&id) {
                    d.pending = None;
                    d.connected = true;
                    d.services_resolved = true;
                    if d.battery.is_none() && d.service(BATTERY).is_some() {
                        d.battery = Some(90);
                    }
                }
            }
            Event::ConnectFailed(id) => {
                if let Some(d) = self.dev(&id) {
                    d.pending = None;
                }
                let name = self.name(&id);
                self.issue(format!("Couldn't connect to {name}"), Some(id));
            }
            Event::Disconnected(id) => {
                if let Some(d) = self.dev(&id) {
                    d.pending = None;
                    d.connected = false;
                    d.services_resolved = false;
                }
            }
            Event::Prompt(id) => {
                let Some(address) = self.state.device(&id).map(|d| d.address.clone()) else {
                    return;
                };
                let kind = match address.as_str() {
                    CONTROLLER => {
                        self.later(now, 1000, Event::Paired(id));
                        return;
                    }
                    NEW_KEYBOARD => PairingKind::DisplayPasskey {
                        passkey: 205117,
                        entered: 0,
                    },
                    BUDS => PairingKind::Confirm { passkey: 381904 },
                    TV => PairingKind::Confirm { passkey: 570263 },
                    _ => PairingKind::Confirm { passkey: 114862 },
                };
                let request = self.id();
                if matches!(kind, PairingKind::DisplayPasskey { .. }) {
                    self.later(now, 900, Event::Typed(request));
                }
                self.state.pairing = Some(PairingRequest {
                    id: request,
                    device: id,
                    kind,
                });
            }
            Event::Typed(request) => {
                let Some(p) = self.state.pairing.as_mut().filter(|p| p.id == request) else {
                    return;
                };
                let PairingKind::DisplayPasskey { entered, .. } = &mut p.kind else {
                    return;
                };
                *entered += 1;
                if *entered >= 6 {
                    let device = p.device.clone();
                    self.state.pairing = None;
                    self.later(now, 300, Event::Paired(device));
                } else {
                    self.later(now, 400, Event::Typed(request));
                }
            }
            Event::Paired(id) => {
                if let Some(d) = self.dev(&id) {
                    d.paired = true;
                    d.bonded = true;
                    d.trusted = true;
                    d.pending = Some(Pending::Connecting);
                    self.later(now, 600, Event::Connected(id));
                }
            }
            Event::PairFailed(id) => {
                if let Some(d) = self.dev(&id) {
                    d.pending = None;
                }
                let name = self.name(&id);
                self.issue(format!("Couldn't pair with {name}"), Some(id));
            }
            Event::Removed(id) => {
                self.state.devices.retain(|d| d.id != id);
                if self.state.pairing.as_ref().is_some_and(|p| p.device == id) {
                    self.state.pairing = None;
                }
            }
            Event::ProfileDone(id) => {
                if let Some(d) = self.dev(&id) {
                    d.pending = None;
                    d.connected = true;
                    d.services_resolved = true;
                }
            }
            Event::Powered(on) => {
                set_powered(&mut self.state, on);
                if !on {
                    self.queue.clear();
                }
            }
        }
    }
}

/// Runs the fake stack on its own thread.
pub fn start() -> Bluetooth {
    let (bt, driver) = channel();
    std::thread::Builder::new()
        .name("mock-bluetooth".into())
        .spawn(move || {
            let begun = Instant::now();
            let mut mock = Mock::new(sample());
            driver.publish(mock.state.clone());
            loop {
                let now = begun.elapsed();
                let wait = mock
                    .next_due()
                    .map_or(Duration::from_secs(3600), |t| t.saturating_sub(now));
                let mut changed = false;
                match driver.commands().recv_timeout(wait) {
                    Ok(c) => {
                        mock.apply(c, begun.elapsed());
                        while let Ok(c) = driver.commands().try_recv() {
                            mock.apply(c, begun.elapsed());
                        }
                        changed = true;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                changed |= mock.advance(begun.elapsed());
                if changed {
                    driver.publish(mock.state.clone());
                }
            }
        })
        .expect("spawn the mock bluetooth thread");
    bt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn consistent(s: &State) {
        assert!(
            s.default_adapter
                .as_ref()
                .is_none_or(|a| s.adapter(a).is_some())
        );
        for d in &s.devices {
            assert!(
                s.adapter(&d.adapter).is_some(),
                "{} has no adapter",
                d.alias
            );
        }
        assert!(
            s.pairing
                .as_ref()
                .is_none_or(|p| s.device(&p.device).is_some())
        );
    }

    #[test]
    fn the_samples_are_consistent() {
        for s in [
            sample(),
            scanning(),
            pairing(),
            powered_off(),
            not_running(),
        ] {
            consistent(&s);
        }
        let s = sample();
        let connected: Vec<_> = s.connected().map(|d| d.display_name()).collect();
        assert_eq!(connected, ["AirPods Pro", "Keychron K2", "MX Master 3S"]);
        let paired: Vec<_> = s.paired().map(|d| d.display_name()).collect();
        assert_eq!(paired, ["JBL Flip 6", "Pixel 8"]);
        assert_eq!(s.nearby().count(), 0);
        assert!(s.device(&device_id(AIRPODS)).unwrap().audio);
        assert!(!s.device(&device_id(MOUSE)).unwrap().audio);
        assert_eq!(scanning().nearby().count(), NEARBY.len());
    }

    #[test]
    fn discovery_finds_devices_over_time() {
        let mut m = Mock::new(sample());
        let a = adapter_id();
        m.apply(Command::StartDiscovery { adapter: a.clone() }, ms(0));
        assert!(m.state.adapters[0].discovering);
        m.advance(ms(1300));
        assert_eq!(m.state.nearby().count(), 2);
        m.advance(ms(5000));
        assert_eq!(m.state.nearby().count(), NEARBY.len());
        m.apply(Command::StopDiscovery { adapter: a }, ms(5000));
        assert!(m.state.nearby().all(|d| d.rssi.is_none()));
        assert!(m.next_due().is_none());
    }

    #[test]
    fn pairing_confirms_then_connects() {
        let mut m = Mock::new(scanning());
        let buds = device_id(BUDS);
        m.apply(
            Command::Pair {
                device: buds.clone(),
            },
            ms(0),
        );
        assert_eq!(
            m.state.device(&buds).unwrap().pending,
            Some(Pending::Pairing)
        );
        m.advance(ms(600));
        let p = m.state.pairing.clone().unwrap();
        assert_eq!(p.kind, PairingKind::Confirm { passkey: 381904 });
        m.apply(
            Command::PairingReply {
                id: p.id + 100,
                reply: Reply::Accept,
            },
            ms(700),
        );
        assert!(m.state.pairing.is_some(), "a stale reply is ignored");
        m.apply(
            Command::PairingReply {
                id: p.id,
                reply: Reply::Accept,
            },
            ms(700),
        );
        m.advance(ms(3000));
        let d = m.state.device(&buds).unwrap();
        assert!(d.paired && d.trusted && d.connected && d.pending.is_none());
        consistent(&m.state);
    }

    #[test]
    fn a_displayed_passkey_fills_in() {
        let mut m = Mock::new(scanning());
        let kb = device_id(NEW_KEYBOARD);
        m.apply(Command::Pair { device: kb.clone() }, ms(0));
        m.advance(ms(1600));
        assert!(matches!(
            m.state.pairing.as_ref().unwrap().kind,
            PairingKind::DisplayPasskey { entered: 1, .. }
        ));
        m.advance(ms(10_000));
        assert!(m.state.pairing.is_none());
        assert!(m.state.device(&kb).unwrap().connected);
    }

    #[test]
    fn failures_become_issues() {
        let mut m = Mock::new(sample());
        m.apply(
            Command::Connect {
                device: device_id(SPEAKER),
            },
            ms(0),
        );
        m.advance(ms(5000));
        assert_eq!(m.state.issues.len(), 1);
        assert_eq!(m.state.issues[0].message, "Couldn't connect to JBL Flip 6");
        let id = m.state.issues[0].id;
        m.apply(Command::DismissIssue { id }, ms(5000));
        assert!(m.state.issues.is_empty());
    }

    #[test]
    fn power_off_drops_connections_and_remove_forgets() {
        let mut m = Mock::new(sample());
        m.apply(
            Command::Remove {
                device: device_id(PHONE),
            },
            ms(0),
        );
        m.advance(ms(400));
        assert!(m.state.device(&device_id(PHONE)).is_none());
        m.apply(
            Command::SetPowered {
                adapter: adapter_id(),
                on: false,
            },
            ms(400),
        );
        assert_eq!(m.state.adapters[0].power_state, PowerState::TurningOff);
        m.advance(ms(800));
        assert!(!m.state.adapters[0].powered);
        assert_eq!(m.state.connected().count(), 0);
        m.apply(
            Command::SetAlias {
                device: device_id(AIRPODS),
                alias: "Dax's AirPods".into(),
            },
            ms(800),
        );
        assert_eq!(
            m.state.device(&device_id(AIRPODS)).unwrap().display_name(),
            "Dax's AirPods"
        );
        m.apply(
            Command::SetAlias {
                device: device_id(AIRPODS),
                alias: String::new(),
            },
            ms(800),
        );
        assert_eq!(
            m.state.device(&device_id(AIRPODS)).unwrap().display_name(),
            "AirPods Pro"
        );
    }
}
