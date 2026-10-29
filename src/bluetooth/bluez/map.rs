//! Pure mapping from BlueZ's objects (as `GetManagedObjects` and the signals report them) to the
//! model: adapters, devices, kinds, bearers, power states, sorting and error wording. Nothing in
//! here talks to the bus, so all of it is unit tested.

use std::collections::{BTreeMap, HashMap};

use zbus::zvariant::{OwnedValue, Value};

use crate::bluetooth::model::*;

pub const ADAPTER: &str = "org.bluez.Adapter1";
pub const DEVICE: &str = "org.bluez.Device1";
pub const BATTERY: &str = "org.bluez.Battery1";
pub const MEDIA_TRANSPORT: &str = "org.bluez.MediaTransport1";
pub const MEDIA_CONTROL: &str = "org.bluez.MediaControl1";
pub const BEARER_LE: &str = "org.bluez.Bearer.LE1";
pub const BEARER_BREDR: &str = "org.bluez.Bearer.BREDR1";

pub type Props = HashMap<String, OwnedValue>;
/// Object path → interface → properties.
pub type Objects = BTreeMap<String, BTreeMap<String, Props>>;

/// Strips `v` (variant-in-variant) wrappers.
fn plain<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(inner) => plain(inner),
        v => v,
    }
}

fn get<'a>(p: &'a Props, key: &str) -> Option<&'a Value<'static>> {
    p.get(key).map(|v| &**v)
}

fn get_bool(p: &Props, key: &str) -> bool {
    matches!(get(p, key).map(plain), Some(Value::Bool(true)))
}

fn get_str(p: &Props, key: &str) -> Option<String> {
    match plain(get(p, key)?) {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(s) => Some(s.to_string()),
        _ => None,
    }
}

fn get_int(p: &Props, key: &str) -> Option<i64> {
    int(get(p, key)?)
}

fn int(v: &Value) -> Option<i64> {
    Some(match plain(v) {
        Value::U8(n) => *n as i64,
        Value::I16(n) => *n as i64,
        Value::U16(n) => *n as i64,
        Value::I32(n) => *n as i64,
        Value::U32(n) => *n as i64,
        Value::I64(n) => *n,
        Value::U64(n) => *n as i64,
        _ => return None,
    })
}

fn get_strs(p: &Props, key: &str) -> Vec<String> {
    match get(p, key).map(plain) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match plain(v) {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn bytes(v: &Value) -> Option<Vec<u8>> {
    match plain(v) {
        Value::Array(a) => a
            .iter()
            .map(|b| match plain(b) {
                Value::U8(b) => Some(*b),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// A property value as text for the raw view.
pub fn render(v: &Value) -> String {
    match plain(v) {
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => s.to_string(),
        Value::ObjectPath(s) => s.to_string(),
        Value::Array(a) => {
            if let Some(b) = bytes(v)
                && !a.is_empty()
            {
                return hex(&b);
            }
            a.iter().map(render).collect::<Vec<_>>().join(", ")
        }
        Value::Dict(d) => d
            .iter()
            .map(|(k, v)| format!("{}: {}", render(k), render(v)))
            .collect::<Vec<_>>()
            .join("; "),
        v => int(v).map_or_else(|| format!("{v}"), |n| n.to_string()),
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `Adapter1.PowerState`; without it (older BlueZ), `Powered`.
pub fn power_state(state: Option<&str>, powered: bool) -> PowerState {
    match state {
        Some("on") => PowerState::On,
        Some("off") => PowerState::Off,
        Some("off-enabling") => PowerState::TurningOn,
        Some("on-disabling") => PowerState::TurningOff,
        Some("off-blocked") => PowerState::Blocked,
        _ if powered => PowerState::On,
        _ => PowerState::Off,
    }
}

fn address_type(s: Option<String>) -> AddressType {
    if s.as_deref() == Some("random") {
        AddressType::Random
    } else {
        AddressType::Public
    }
}

/// Raw properties of one interface, rendered and sorted, keys prefixed with `prefix`.
fn rendered(p: &Props, prefix: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = p
        .iter()
        .map(|(k, v)| (format!("{prefix}{k}"), render(v)))
        .collect();
    out.sort();
    out
}

pub fn adapter(path: &str, p: &Props) -> Adapter {
    let powered = get_bool(p, "Powered");
    Adapter {
        id: AdapterId(path.to_owned()),
        name: get_str(p, "Name").unwrap_or_default(),
        alias: get_str(p, "Alias").unwrap_or_default(),
        address: get_str(p, "Address").unwrap_or_default(),
        address_type: address_type(get_str(p, "AddressType")),
        powered,
        power_state: power_state(get_str(p, "PowerState").as_deref(), powered),
        discoverable: get_bool(p, "Discoverable"),
        discoverable_timeout: get_int(p, "DiscoverableTimeout").unwrap_or(0) as u32,
        pairable: get_bool(p, "Pairable"),
        pairable_timeout: get_int(p, "PairableTimeout").unwrap_or(0) as u32,
        discovering: get_bool(p, "Discovering"),
        class: get_int(p, "Class").unwrap_or(0) as u32,
        roles: get_strs(p, "Roles"),
        modalias: get_str(p, "Modalias"),
        services: get_strs(p, "UUIDs")
            .iter()
            .map(|u| Service::new(u))
            .collect(),
        properties: rendered(p, ""),
    }
}

/// What a device is: BlueZ's icon, refined by the class of device and LE appearance for the
/// kinds BlueZ has no icon for (watches, tablets).
pub fn kind(icon: &str, class: Option<u32>, appearance: Option<u16>) -> Kind {
    let from_icon = Kind::from_icon(icon);
    if let Some(c) = class {
        let (major, minor) = ((c >> 8) & 0x1f, (c >> 2) & 0x3f);
        match (major, minor) {
            (0x01, 0x06) | (0x07, 0x01) => return Kind::Watch,
            (0x01, 0x07) => return Kind::Tablet,
            _ => {}
        }
    }
    if from_icon == Kind::Other
        && let Some(a) = appearance
    {
        match a >> 6 {
            0x003 => return Kind::Watch,
            0x001 => return Kind::Phone,
            0x002 => return Kind::Computer,
            0x00a => return Kind::Player,
            _ => {}
        }
    }
    from_icon
}

/// BR/EDR, LE or both: the `org.bluez.Bearer.*` interfaces when BlueZ has them, else what the
/// properties imply.
pub fn bearer(ifaces: &BTreeMap<String, Props>, p: &Props, uuids: &[String]) -> Bearer {
    let (le, bredr) = (
        ifaces.contains_key(BEARER_LE),
        ifaces.contains_key(BEARER_BREDR),
    );
    if le || bredr {
        return match (le, bredr) {
            (true, true) => Bearer::Dual,
            (true, false) => Bearer::Le,
            _ => Bearer::BrEdr,
        };
    }
    let shorts: Vec<u32> = uuids
        .iter()
        .filter_map(|u| short_uuid(&u.to_ascii_lowercase()))
        .collect();
    let bredr = p.contains_key("Class") || shorts.iter().any(|s| (0x1100..=0x12ff).contains(s));
    let le = get_str(p, "AddressType").as_deref() == Some("random")
        || p.contains_key("Appearance")
        || p.contains_key("AdvertisingFlags")
        || shorts.iter().any(|s| (0x1802..=0x18ff).contains(s));
    match (bredr, le) {
        (true, true) => Bearer::Dual,
        (true, false) => Bearer::BrEdr,
        (false, true) => Bearer::Le,
        (false, false) => Bearer::Unknown,
    }
}

/// The parent of an object path (`/org/bluez/hci0/dev_X` → `/org/bluez/hci0`).
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

pub fn device(path: &str, objects: &Objects) -> Option<Device> {
    let ifaces = objects.get(path)?;
    let p = ifaces.get(DEVICE)?;
    let adapter = get_str(p, "Adapter").unwrap_or_else(|| parent(path).to_owned());
    let address = get_str(p, "Address").unwrap_or_default();
    let uuids = get_strs(p, "UUIDs");
    let services: Vec<Service> = uuids.iter().map(|u| Service::new(u)).collect();
    let icon = get_str(p, "Icon").unwrap_or_default();
    let class = get_int(p, "Class").map(|c| c as u32);
    let appearance = get_int(p, "Appearance").map(|a| a as u16);
    let child = format!("{path}/");
    let media = ifaces.contains_key(MEDIA_CONTROL)
        || objects
            .range(child.clone()..)
            .take_while(|(k, _)| k.starts_with(&child))
            .any(|(_, i)| i.contains_key(MEDIA_TRANSPORT));
    let battery = ifaces
        .get(BATTERY)
        .and_then(|b| get_int(b, "Percentage"))
        .map(|n| n.clamp(0, 100) as u8);
    let mut properties = rendered(p, "");
    for (iface, prefix) in [
        (BATTERY, "Battery1."),
        (BEARER_LE, "Bearer.LE1."),
        (BEARER_BREDR, "Bearer.BREDR1."),
    ] {
        if let Some(extra) = ifaces.get(iface) {
            properties.extend(rendered(extra, prefix));
        }
    }
    let manufacturer_data = match get(p, "ManufacturerData").map(plain) {
        Some(Value::Dict(d)) => d
            .iter()
            .filter_map(|(k, v)| {
                let company = int(k)? as u16;
                Some(ManufacturerData {
                    company,
                    company_name: company_name(company).map(Into::into),
                    data: bytes(v).unwrap_or_default(),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    let service_data = match get(p, "ServiceData").map(plain) {
        Some(Value::Dict(d)) => d
            .iter()
            .filter_map(|(k, v)| match plain(k) {
                Value::Str(u) => Some(ServiceData {
                    service: Service::new(u),
                    data: bytes(v).unwrap_or_default(),
                }),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let audio = media || services.iter().any(|s| s.audio);
    Some(Device {
        id: DeviceId(path.to_owned()),
        adapter: AdapterId(adapter),
        address_type: address_type(get_str(p, "AddressType")),
        bearer: bearer(ifaces, p, &uuids),
        name: get_str(p, "Name"),
        alias: get_str(p, "Alias").unwrap_or_else(|| address.replace(':', "-")),
        address,
        kind: kind(&icon, class, appearance),
        icon,
        class,
        appearance,
        paired: get_bool(p, "Paired"),
        bonded: get_bool(p, "Bonded"),
        trusted: get_bool(p, "Trusted"),
        blocked: get_bool(p, "Blocked"),
        connected: get_bool(p, "Connected"),
        services_resolved: get_bool(p, "ServicesResolved"),
        legacy_pairing: get_bool(p, "LegacyPairing"),
        battery,
        rssi: get_int(p, "RSSI").map(|n| n as i16),
        tx_power: get_int(p, "TxPower").map(|n| n as i16),
        services,
        audio,
        pending: None,
        raw: Raw {
            modalias: get_str(p, "Modalias"),
            manufacturer_data,
            service_data,
            advertising_flags: get(p, "AdvertisingFlags")
                .and_then(bytes)
                .unwrap_or_default(),
            properties,
        },
    })
}

/// Connected first, then paired, then nearby by signal, then by name.
pub fn sort_devices(devices: &mut [Device]) {
    devices.sort_by(|a, b| {
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
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// The selected adapter if it exists, else the first powered one, else the first.
pub fn default_adapter(adapters: &[Adapter], selected: Option<&AdapterId>) -> Option<AdapterId> {
    selected
        .filter(|s| adapters.iter().any(|a| &a.id == *s))
        .cloned()
        .or_else(|| {
            adapters
                .iter()
                .find(|a| a.powered)
                .or(adapters.first())
                .map(|a| a.id.clone())
        })
}

/// Adapters (by path) and devices (sorted, only those whose adapter exists) in `objects`.
pub fn build(objects: &Objects) -> (Vec<Adapter>, Vec<Device>) {
    let adapters: Vec<Adapter> = objects
        .iter()
        .filter_map(|(path, i)| i.get(ADAPTER).map(|p| adapter(path, p)))
        .collect();
    let mut devices: Vec<Device> = objects
        .keys()
        .filter_map(|path| device(path, objects))
        .filter(|d| adapters.iter().any(|a| a.id == d.adapter))
        .collect();
    sort_devices(&mut devices);
    (adapters, devices)
}

/// Plain words for why a BlueZ call failed, from the D-Bus error name and message. `None` when
/// there's nothing more useful to say than "couldn't".
pub fn reason(name: &str, message: Option<&str>) -> Option<&'static str> {
    let message = message.unwrap_or("");
    let short = name.rsplit('.').next().unwrap_or(name);
    if name.starts_with("org.freedesktop.DBus.Error.") {
        return Some(match short {
            "NoReply" | "Timeout" | "TimedOut" => "Bluetooth didn't answer",
            "AccessDenied" => "not allowed",
            "ServiceUnknown" | "NameHasNoOwner" => "Bluetooth isn't running",
            "UnknownObject" | "UnknownMethod" => "it's gone",
            _ => return None,
        });
    }
    Some(match short {
        "AuthenticationFailed" => "wrong code or the device refused",
        "AuthenticationCanceled" => "cancelled",
        "AuthenticationRejected" => "rejected",
        "AuthenticationTimeout" => "timed out",
        "ConnectionAttemptFailed" => "it's off or out of range",
        "AlreadyConnected" => "already connected",
        "AlreadyExists" => "already paired",
        "InProgress" => "already in progress",
        "NotReady" => "Bluetooth isn't ready",
        "NotPowered" => "Bluetooth is off",
        "NotConnected" => "not connected",
        "DoesNotExist" => "it's gone",
        "NotSupported" => "not supported",
        "NotAuthorized" | "NotPermitted" => "not allowed",
        "NotAvailable" => "not available",
        "InvalidArguments" | "InvalidArgs" => "not accepted",
        "Blocked" | "Rfkill" => "blocked by airplane mode or a switch",
        _ => return failed(message),
    })
}

/// `org.bluez.Error.Failed` messages, which carry the real cause.
fn failed(message: &str) -> Option<&'static str> {
    let m = message.to_ascii_lowercase();
    Some(
        if m.contains("page-timeout")
            || m.contains("host is down")
            || m.contains("timed out")
            || m.contains("timeout")
        {
            "it's off or out of range"
        } else if m.contains("profile-unavailable") || m.contains("protocol not available") {
            "no audio service is running"
        } else if m.contains("abort-by-local")
            || m.contains("abort-by-remote")
            || m.contains("connection refused")
        {
            "the device refused"
        } else if m.contains("rfkill") || m.contains("blocked") {
            "blocked by airplane mode or a switch"
        } else if m.contains("not powered") || m.contains("resource not ready") {
            "Bluetooth is off"
        } else if m.contains("in progress") || m.contains("busy") {
            "already in progress"
        } else {
            return None;
        },
    )
}

/// The issue text for a failed command: "Couldn't connect to AirPods Pro: it's off or out of
/// range".
pub fn issue_text(what: &str, error_name: Option<&str>, message: Option<&str>) -> String {
    match error_name.and_then(|n| reason(n, message)) {
        Some(r) => format!("{what}: {r}"),
        None => what.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v<'a>(x: impl Into<Value<'a>>) -> OwnedValue {
        x.into().try_to_owned().unwrap()
    }

    fn props(kv: Vec<(&str, OwnedValue)>) -> Props {
        kv.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }

    fn sample() -> Objects {
        let mut o = Objects::new();
        o.entry("/org/bluez/hci0".into()).or_default().insert(
            ADAPTER.into(),
            props(vec![
                ("Address", v("F4:26:79:3A:5C:01")),
                ("Name", v("apollo")),
                ("Alias", v("apollo")),
                ("Powered", v(true)),
                ("PowerState", v("on")),
                ("Discoverable", v(false)),
                ("DiscoverableTimeout", v(180u32)),
                ("Class", v(0x6c010cu32)),
                ("Roles", v(vec!["central", "peripheral"])),
            ]),
        );
        let headphones = "/org/bluez/hci0/dev_AC_80_0A_12_34_56";
        let dev = o.entry(headphones.into()).or_default();
        dev.insert(
            DEVICE.into(),
            props(vec![
                ("Address", v("AC:80:0A:12:34:56")),
                ("AddressType", v("public")),
                ("Alias", v("WH-1000XM4")),
                ("Name", v("WH-1000XM4")),
                ("Icon", v("audio-headset")),
                ("Class", v(0x240404u32)),
                ("Paired", v(true)),
                ("Bonded", v(true)),
                ("Connected", v(true)),
                (
                    "Adapter",
                    v(zbus::zvariant::ObjectPath::try_from("/org/bluez/hci0").unwrap()),
                ),
                (
                    "UUIDs",
                    v(vec![
                        "0000110b-0000-1000-8000-00805f9b34fb",
                        "0000111e-0000-1000-8000-00805f9b34fb",
                    ]),
                ),
            ]),
        );
        dev.insert(BATTERY.into(), props(vec![("Percentage", v(70u8))]));
        let mut md = HashMap::new();
        md.insert(0x004cu16, Value::new(vec![1u8, 2, 3]));
        o.entry("/org/bluez/hci0/dev_5C_3A_91_0E_22_7F".into())
            .or_default()
            .insert(
                DEVICE.into(),
                props(vec![
                    ("Address", v("5C:3A:91:0E:22:7F")),
                    ("AddressType", v("random")),
                    ("Alias", v("5C-3A-91-0E-22-7F")),
                    ("RSSI", v(-80i16)),
                    ("ManufacturerData", v(md)),
                ]),
            );
        o.entry("/org/bluez/hci9/dev_00_00_00_00_00_01".into())
            .or_default()
            .insert(
                DEVICE.into(),
                props(vec![("Address", v("00:00:00:00:00:01"))]),
            );
        o
    }

    #[test]
    fn builds_adapters_and_devices() {
        let (adapters, devices) = build(&sample());
        assert_eq!(adapters.len(), 1);
        let a = &adapters[0];
        assert_eq!(
            (
                a.display_name(),
                a.power_state,
                a.discoverable_timeout,
                a.class
            ),
            ("apollo", PowerState::On, 180, 0x6c010c)
        );
        assert_eq!(a.roles, ["central", "peripheral"]);
        assert_eq!(devices.len(), 2, "devices of a missing adapter are dropped");
        let h = &devices[0];
        assert_eq!(
            (h.display_name(), h.kind, h.battery, h.bearer),
            ("WH-1000XM4", Kind::Headset, Some(70), Bearer::BrEdr)
        );
        assert!(h.audio && h.connected && h.is_paired());
        assert!(
            h.raw
                .properties
                .iter()
                .any(|(k, v)| k == "Battery1.Percentage" && v == "70")
        );
        let s = &devices[1];
        assert_eq!(
            (s.address_type, s.bearer, s.rssi, s.name.clone()),
            (AddressType::Random, Bearer::Le, Some(-80), None)
        );
        assert_eq!(
            s.raw.manufacturer_data[0].company_name.as_deref(),
            Some("Apple")
        );
        assert_eq!(s.raw.manufacturer_data[0].data, [1, 2, 3]);
    }

    #[test]
    fn media_transports_mark_audio() {
        let mut o = sample();
        let path = "/org/bluez/hci0/dev_5C_3A_91_0E_22_7F";
        assert!(!device(path, &o).unwrap().audio);
        o.entry(format!("{path}/fd0"))
            .or_default()
            .insert(MEDIA_TRANSPORT.into(), Props::new());
        assert!(device(path, &o).unwrap().audio);
    }

    #[test]
    fn bearer_interfaces_win() {
        let mut o = sample();
        let path = "/org/bluez/hci0/dev_AC_80_0A_12_34_56";
        o.get_mut(path)
            .unwrap()
            .insert(BEARER_LE.into(), Props::new());
        o.get_mut(path)
            .unwrap()
            .insert(BEARER_BREDR.into(), Props::new());
        assert_eq!(device(path, &o).unwrap().bearer, Bearer::Dual);
    }

    #[test]
    fn kinds_refine_icons() {
        assert_eq!(kind("audio-headphones", None, None), Kind::Headphones);
        assert_eq!(kind("input-gaming", None, Some(0x03c4)), Kind::Gamepad);
        assert_eq!(kind("", None, Some(0x00c1)), Kind::Watch);
        assert_eq!(kind("computer", Some(0x00011c), None), Kind::Tablet);
        assert_eq!(kind("", Some(0x000704), None), Kind::Watch);
        assert_eq!(kind("", None, None), Kind::Other);
    }

    #[test]
    fn power_states_parse() {
        assert_eq!(power_state(Some("off-blocked"), false), PowerState::Blocked);
        assert_eq!(
            power_state(Some("off-enabling"), false),
            PowerState::TurningOn
        );
        assert_eq!(
            power_state(Some("on-disabling"), true),
            PowerState::TurningOff
        );
        assert_eq!(power_state(None, true), PowerState::On);
        assert_eq!(power_state(Some("weird"), false), PowerState::Off);
    }

    #[test]
    fn errors_read_plainly() {
        assert_eq!(
            issue_text(
                "Couldn't connect to Buds",
                Some("org.bluez.Error.Failed"),
                Some("br-connection-page-timeout")
            ),
            "Couldn't connect to Buds: it's off or out of range"
        );
        assert_eq!(
            issue_text(
                "Couldn't pair with Buds",
                Some("org.bluez.Error.AuthenticationFailed"),
                None
            ),
            "Couldn't pair with Buds: wrong code or the device refused"
        );
        assert_eq!(
            issue_text(
                "Couldn't connect to Buds",
                Some("org.bluez.Error.Failed"),
                Some("br-connection-profile-unavailable")
            ),
            "Couldn't connect to Buds: no audio service is running"
        );
        assert_eq!(
            issue_text(
                "Couldn't connect to Buds",
                Some("org.bluez.Error.Failed"),
                Some("something new")
            ),
            "Couldn't connect to Buds"
        );
        assert_eq!(
            reason("org.freedesktop.DBus.Error.NoReply", None),
            Some("Bluetooth didn't answer")
        );
        assert_eq!(
            issue_text("Couldn't connect", None, None),
            "Couldn't connect"
        );
    }

    #[test]
    fn sorting_follows_the_model() {
        let d = |alias: &str, connected: bool, paired: bool, rssi: Option<i16>| Device {
            id: DeviceId(alias.into()),
            alias: alias.into(),
            connected,
            paired,
            rssi,
            ..Device::default()
        };
        let mut v = vec![
            d("zeta", false, false, Some(-90)),
            d("beta", false, true, None),
            d("alpha", false, false, Some(-40)),
            d("Gamma", true, true, None),
            d("aardvark", false, false, None),
            d("Alpha", false, true, None),
        ];
        sort_devices(&mut v);
        let names: Vec<_> = v.iter().map(|d| d.alias.as_str()).collect();
        assert_eq!(
            names,
            ["Gamma", "Alpha", "beta", "alpha", "zeta", "aardvark"]
        );
    }

    #[test]
    fn default_adapter_prefers_selected_then_powered() {
        let a = |p: &str, powered: bool| Adapter {
            id: AdapterId(p.into()),
            powered,
            ..Adapter::default()
        };
        let list = [a("/org/bluez/hci0", false), a("/org/bluez/hci1", true)];
        assert_eq!(default_adapter(&list, None).unwrap().0, "/org/bluez/hci1");
        assert_eq!(
            default_adapter(&list, Some(&AdapterId("/org/bluez/hci0".into())))
                .unwrap()
                .0,
            "/org/bluez/hci0"
        );
        assert_eq!(
            default_adapter(&list, Some(&AdapterId("/org/bluez/hci7".into())))
                .unwrap()
                .0,
            "/org/bluez/hci1"
        );
        assert_eq!(default_adapter(&[], None), None);
    }
}
