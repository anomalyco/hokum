//! Plain-data mirror of the PipeWire globals the backend tracks, and the pure assembly of a
//! [`State`] from them. Nothing here touches PipeWire, so all of it is unit-tested.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::sound::model::*;

use super::params::{self, Param, ProfileParam, RouteParam, VolumeProps};

pub type Props = BTreeMap<String, String>;

/// `node.name` prefix of virtual sinks this app creates.
pub const VIRTUAL_PREFIX: &str = "hokum-sound-";
/// Node property marking a virtual sink this app created.
pub const OURS_KEY: &str = "hokum.sound.virtual";
/// Node property marking this app's peak-meter streams.
pub const METER_KEY: &str = "hokum.sound.meter";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Creating,
    Suspended,
    Idle,
    Running,
    Error,
}

#[derive(Clone, Debug, Default)]
pub struct NodeG {
    pub props: Props,
    pub state: RunState,
}

#[derive(Clone, Debug, Default)]
pub struct PortG {
    pub props: Props,
    pub output: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LinkG {
    pub output_node: Id,
    pub output_port: Id,
    pub input_node: Id,
    pub input_port: Id,
    pub active: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MetadataG {
    pub name: String,
    /// `(subject, key)` → value.
    pub entries: BTreeMap<(u32, String), String>,
}

/// One enumeration of a param id on an object: the request's seq and its results.
#[derive(Clone, Debug, Default)]
pub struct ParamList {
    pub seq: i32,
    pub items: Vec<Param>,
}

#[derive(Debug, Default)]
pub struct Store {
    pub core_name: String,
    pub core_version: String,
    pub nodes: BTreeMap<Id, NodeG>,
    /// PipeWire devices (cards), with their props.
    pub devices: BTreeMap<Id, Props>,
    pub ports: BTreeMap<Id, PortG>,
    pub links: BTreeMap<Id, LinkG>,
    pub clients: BTreeMap<Id, Props>,
    pub metadata: BTreeMap<Id, MetadataG>,
    /// `(object, param id)` → latest enumeration.
    pub params: HashMap<(Id, u32), ParamList>,
    /// Enumerations asked for that haven't returned anything yet.
    pub awaiting: HashSet<(Id, u32)>,
}

impl Store {
    pub fn remove(&mut self, id: Id) {
        self.nodes.remove(&id);
        self.devices.remove(&id);
        self.ports.remove(&id);
        self.links.remove(&id);
        self.clients.remove(&id);
        self.metadata.remove(&id);
        self.params.retain(|(o, _), _| *o != id);
        self.awaiting.retain(|(o, _)| *o != id);
    }

    /// A fresh enumeration of `id` on `object` was asked for: its first result replaces the
    /// list, and if none arrives by [`Store::param_done`] the list is empty.
    pub fn param_request(&mut self, object: Id, id: u32) {
        self.awaiting.insert((object, id));
    }

    /// A param result. `seq` is what the remote put on it: one value per enumeration (not the
    /// one the request carried: objects exported by the session manager answer with their own).
    pub fn param(&mut self, object: Id, id: u32, seq: i32, param: Option<Param>) {
        let first = self.awaiting.remove(&(object, id));
        let list = self.params.entry((object, id)).or_default();
        if first || list.seq != seq {
            list.seq = seq;
            list.items.clear();
        }
        if let Some(p) = param {
            list.items.push(p);
        }
    }

    /// The enumeration asked for finished (a core sync after it came back).
    pub fn param_done(&mut self, object: Id, id: u32) {
        if self.awaiting.remove(&(object, id))
            && let Some(list) = self.params.get_mut(&(object, id))
        {
            list.items.clear();
        }
    }

    pub fn params(&self, object: Id, id: u32) -> &[Param] {
        self.params.get(&(object, id)).map(|l| l.items.as_slice()).unwrap_or(&[])
    }

    /// A node's volume props: nodes list a second, volume-less `Props` object too.
    pub fn node_props_param(&self, node: Id) -> Option<&VolumeProps> {
        let mut all = self.params(node, params::PROPS).iter().filter_map(|p| match p {
            Param::Props(v) => Some(v),
            _ => None,
        });
        let first = all.next()?;
        if first.volumes.is_empty() { all.find(|v| !v.volumes.is_empty()).or(Some(first)) } else { Some(first) }
    }

    pub fn enum_profiles(&self, device: Id) -> impl Iterator<Item = &ProfileParam> {
        self.params(device, params::ENUM_PROFILE).iter().filter_map(|p| match p {
            Param::Profile(p) => Some(p),
            _ => None,
        })
    }

    pub fn active_profile(&self, device: Id) -> Option<&ProfileParam> {
        self.params(device, params::PROFILE).iter().find_map(|p| match p {
            Param::Profile(p) => Some(p),
            _ => None,
        })
    }

    pub fn enum_routes(&self, device: Id) -> impl Iterator<Item = &RouteParam> {
        self.params(device, params::ENUM_ROUTE).iter().filter_map(|p| match p {
            Param::Route(r) => Some(r),
            _ => None,
        })
    }

    pub fn active_routes(&self, device: Id) -> impl Iterator<Item = &RouteParam> {
        self.params(device, params::ROUTE).iter().filter_map(|p| match p {
            Param::Route(r) => Some(r),
            _ => None,
        })
    }

    /// The card a sink/source node belongs to and its `card.profile.device`, when both exist.
    pub fn node_card(&self, node: Id) -> Option<(Id, i32)> {
        let props = &self.nodes.get(&node)?.props;
        let card: Id = props.get("device.id")?.parse().ok()?;
        if !self.devices.contains_key(&card) {
            return None;
        }
        let dev: i32 = props.get("card.profile.device")?.parse().ok()?;
        Some((card, dev))
    }

    /// The card's active route for a node: where hardware volume and the port live.
    pub fn node_route(&self, node: Id) -> Option<(Id, &RouteParam)> {
        let (card, dev) = self.node_card(node)?;
        let route = self.active_routes(card).find(|r| r.device == Some(dev))?;
        Some((card, route))
    }

    pub fn metadata_named(&self, name: &str) -> Option<(Id, &MetadataG)> {
        self.metadata.iter().find(|(_, m)| m.name == name).map(|(id, m)| (*id, m))
    }

    fn meta_value(&self, name: &str, subject: u32, key: &str) -> Option<&str> {
        self.metadata_named(name)?.1.entries.get(&(subject, key.to_string())).map(String::as_str)
    }

    pub fn node_by_name(&self, name: &str) -> Option<Id> {
        self.nodes.iter().find(|(_, n)| n.props.get("node.name").is_some_and(|n| n == name)).map(|(id, _)| *id)
    }

    fn node_by_serial(&self, serial: &str) -> Option<Id> {
        self.nodes.iter().find(|(_, n)| n.props.get("object.serial").is_some_and(|s| s == serial)).map(|(id, _)| *id)
    }

    pub fn has_session_manager(&self) -> bool {
        self.metadata_named("default").is_some()
    }
}

// ---- conversions ---------------------------------------------------------------------------

/// PipeWire's linear amplitude to the cubic scale wpctl and pavucontrol show.
pub fn cubic(linear: f32) -> f32 {
    linear.max(0.0).cbrt()
}

/// The cubic scale back to PipeWire's linear amplitude.
pub fn linear(cubic: f32) -> f32 {
    let c = cubic.max(0.0);
    c * c * c
}

pub fn position(spa: u32) -> ChannelPosition {
    use pipewire::spa::sys::*;
    match spa {
        SPA_AUDIO_CHANNEL_MONO => ChannelPosition::Mono,
        SPA_AUDIO_CHANNEL_FL => ChannelPosition::FL,
        SPA_AUDIO_CHANNEL_FR => ChannelPosition::FR,
        SPA_AUDIO_CHANNEL_FC => ChannelPosition::FC,
        SPA_AUDIO_CHANNEL_LFE => ChannelPosition::LFE,
        SPA_AUDIO_CHANNEL_RL => ChannelPosition::RL,
        SPA_AUDIO_CHANNEL_RR => ChannelPosition::RR,
        SPA_AUDIO_CHANNEL_SL => ChannelPosition::SL,
        SPA_AUDIO_CHANNEL_SR => ChannelPosition::SR,
        n => ChannelPosition::Other(n),
    }
}

/// A position from its SPA short name (`audio.channel` on ports: "FL", "MONO", "AUX3").
pub fn position_named(name: &str) -> Option<ChannelPosition> {
    use pipewire::spa::sys::SPA_AUDIO_CHANNEL_AUX0;
    Some(match name {
        "MONO" => ChannelPosition::Mono,
        "FL" => ChannelPosition::FL,
        "FR" => ChannelPosition::FR,
        "FC" => ChannelPosition::FC,
        "LFE" => ChannelPosition::LFE,
        "RL" => ChannelPosition::RL,
        "RR" => ChannelPosition::RR,
        "SL" => ChannelPosition::SL,
        "SR" => ChannelPosition::SR,
        "UNK" | "" => return None,
        n => ChannelPosition::Other(SPA_AUDIO_CHANNEL_AUX0 + n.strip_prefix("AUX")?.parse::<u32>().ok()?),
    })
}

/// Linear PipeWire props to the model's cubic volume.
pub fn volume_of(props: &VolumeProps) -> Volume {
    let channels: Vec<f32> = props.volumes.iter().map(|v| cubic(*v)).collect();
    let positions = (0..channels.len())
        .map(|i| match props.map.get(i) {
            Some(p) => position(*p),
            None if channels.len() == 1 => ChannelPosition::Mono,
            None if channels.len() == 2 => [ChannelPosition::FL, ChannelPosition::FR][i],
            None => ChannelPosition::Other(0),
        })
        .collect();
    Volume { channels, positions }
}

/// The linear channel volumes to write for `want` on a node whose current volume is `current`:
/// matched by position, then by index, and sized to the node's channel count.
pub fn linear_channels(current: &Volume, want: &Volume) -> Vec<f32> {
    let n = current.channels.len().max(1);
    let fallback = want.combined();
    (0..n)
        .map(|i| {
            let by_position = current.positions.get(i).and_then(|p| want.positions.iter().position(|w| w == p)).and_then(|j| want.channels.get(j));
            let v = by_position.or_else(|| if want.channels.len() == n { want.channels.get(i) } else { None }).copied().unwrap_or(fallback);
            linear(v.clamp(0.0, MAX_VOLUME))
        })
        .collect()
}

/// The device name from a `default` metadata value (`{"name": "alsa_output..."}`).
pub fn metadata_name(value: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(value).ok()?;
    v.get("name")?.as_str().map(str::to_string)
}

/// The numbers in `clock.allowed-rates` (`"[ 44100 48000 ]"`, or JSON with commas).
pub fn rate_list(value: &str) -> Vec<u32> {
    value.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect()
}

/// The Bluetooth codec a profile uses, from "codec X" in its description or its name.
pub fn codec(name: &str, description: &str) -> Option<String> {
    if let Some(i) = description.find("codec ") {
        let rest = &description[i + 6..];
        let end = rest.find([')', ',']).unwrap_or(rest.len());
        let c = rest[..end].trim();
        if !c.is_empty() {
            return Some(c.to_string());
        }
    }
    let suffix = ["a2dp-sink-", "a2dp-duplex-", "headset-head-unit-", "audio-gateway-", "a2dp-source-"].iter().find_map(|p| name.strip_prefix(p))?;
    Some(
        match suffix {
            "sbc" => "SBC",
            "sbc_xq" => "SBC-XQ",
            "aac" => "AAC",
            "ldac" => "LDAC",
            "aptx" => "aptX",
            "aptx_hd" => "aptX HD",
            "aptx_ll" | "aptx_ll_duplex" => "aptX LL",
            "faststream" | "faststream_duplex" => "FastStream",
            "opus_05" | "opus_05_duplex" | "opus_g" => "Opus",
            "lc3plus_h3" => "LC3plus",
            "lc3" => "LC3",
            "lc3_swb" => "LC3-SWB",
            "msbc" => "mSBC",
            "cvsd" => "CVSD",
            other => return Some(other.to_uppercase()),
        }
        .to_string(),
    )
}

fn profile_kind(p: &ProfileParam) -> ProfileKind {
    if p.name == "off" {
        return ProfileKind::Off;
    }
    match (p.sinks > 0, p.sources > 0) {
        (true, true) => ProfileKind::Duplex,
        (true, false) => ProfileKind::Output,
        (false, true) => ProfileKind::Input,
        (false, false) if p.name.starts_with("a2dp-sink") || p.name.starts_with("output:") => ProfileKind::Output,
        (false, false) if p.name.starts_with("input:") => ProfileKind::Input,
        _ => ProfileKind::Duplex,
    }
}

fn availability(spa: u32) -> Availability {
    match spa {
        pipewire::spa::sys::SPA_PARAM_AVAILABILITY_yes => Availability::Yes,
        pipewire::spa::sys::SPA_PARAM_AVAILABILITY_no => Availability::No,
        _ => Availability::Unknown,
    }
}

fn bus_of(props: &Props) -> Bus {
    if props.get("device.api").is_some_and(|a| a == "bluez5") {
        return Bus::Bluetooth;
    }
    match props.get("device.bus").map(String::as_str) {
        Some("bluetooth") => Bus::Bluetooth,
        Some("usb") => Bus::Usb,
        Some("pci") => Bus::Pci,
        Some(_) => Bus::Other,
        None => Bus::Other,
    }
}

fn get<'a>(props: &'a Props, key: &str) -> Option<&'a str> {
    props.get(key).map(String::as_str).filter(|s| !s.is_empty())
}

fn is_true(props: &Props, key: &str) -> bool {
    props.get(key).is_some_and(|v| v == "true" || v == "1")
}

fn form_factor_icon(ff: &str, kind: DeviceKind) -> Option<&'static str> {
    Some(match ff {
        "headphone" | "headphones" => "audio-headphones",
        "headset" | "hands-free" | "handset" => "audio-headset",
        "speaker" | "speakers" => "audio-speakers",
        "microphone" | "mic" => "audio-input-microphone",
        "webcam" => "camera-web",
        "hifi" | "tv" | "car" | "portable" => {
            if kind == DeviceKind::Sink {
                "audio-speakers"
            } else {
                "audio-input-microphone"
            }
        }
        _ => return None,
    })
}

/// A UI-friendly app name: "PipeWire ALSA [spotifyd]" → "spotifyd".
fn app_name(name: &str) -> String {
    name.strip_prefix("PipeWire ALSA [").and_then(|n| n.strip_suffix(']')).unwrap_or(name).to_string()
}

pub fn is_meter(props: &Props) -> bool {
    is_true(props, METER_KEY) || is_true(props, "stream.monitor")
}

fn device_kind(class: &str) -> Option<DeviceKind> {
    if class.starts_with("Audio/Sink") {
        Some(DeviceKind::Sink)
    } else if class.starts_with("Audio/Source") {
        Some(DeviceKind::Source)
    } else {
        None
    }
}

fn stream_kind(class: &str) -> Option<StreamKind> {
    match class {
        "Stream/Output/Audio" => Some(StreamKind::Playback),
        "Stream/Input/Audio" => Some(StreamKind::Record),
        _ => None,
    }
}

// ---- assembly ------------------------------------------------------------------------------

/// The model's view of the store. `levels` are already on the cubic scale; connection, issues
/// and revision are the caller's.
pub fn build(store: &Store, levels: &HashMap<Target, f32>) -> State {
    let mut state = State::default();

    // Cards.
    for (&id, props) in &store.devices {
        if get(props, "media.class") != Some("Audio/Device") {
            continue;
        }
        let profiles: Vec<Profile> = store
            .enum_profiles(id)
            .map(|p| Profile {
                name: p.name.clone(),
                description: if p.description.is_empty() { p.name.clone() } else { p.description.clone() },
                available: p.available != pipewire::spa::sys::SPA_PARAM_AVAILABILITY_no,
                priority: p.priority.max(0) as u32,
                kind: profile_kind(p),
                codec: codec(&p.name, &p.description),
            })
            .collect();
        let active_profile = store.active_profile(id).map(|a| {
            if a.name.is_empty() { store.enum_profiles(id).find(|p| p.index == a.index).map(|p| p.name.clone()).unwrap_or_default() } else { a.name.clone() }
        });
        let bus = bus_of(props);
        let icon_name = get(props, "device.icon-name")
            .or(get(props, "device.icon_name"))
            .or_else(|| get(props, "device.form-factor").and_then(|f| form_factor_icon(f, DeviceKind::Sink)))
            .unwrap_or(if bus == Bus::Bluetooth { "audio-headset" } else { "audio-card" })
            .to_string();
        state.cards.push(Card {
            id,
            name: get(props, "device.name").unwrap_or_default().to_string(),
            description: get(props, "device.description").or(get(props, "device.nick")).or(get(props, "device.name")).unwrap_or_default().to_string(),
            icon_name,
            bus,
            profiles,
            active_profile: active_profile.filter(|n| !n.is_empty()),
            battery: get(props, "bluez5.battery").or(get(props, "device.battery")).and_then(|b| b.parse::<f32>().ok()).map(|b| if b > 1.0 { b / 100.0 } else { b }),
        });
    }

    // Devices.
    let mut devices: Vec<(i64, Device)> = Vec::new();
    for (&id, node) in &store.nodes {
        let p = &node.props;
        let Some(kind) = get(p, "media.class").and_then(device_kind) else { continue };
        let card = store.node_card(id);
        let card_props = card.and_then(|(c, _)| store.devices.get(&c));
        let route = store.node_route(id).map(|(_, r)| r);
        let volume_props = route.and_then(|r| r.props.as_ref()).filter(|v| !v.volumes.is_empty()).or_else(|| store.node_props_param(id));
        let volume = volume_props.map(volume_of).unwrap_or_default();
        let mute = route.and_then(|r| r.props.as_ref()).and_then(|v| v.mute).or_else(|| store.node_props_param(id).and_then(|v| v.mute)).unwrap_or(false);

        let route_direction = if kind == DeviceKind::Sink { pipewire::spa::sys::SPA_DIRECTION_OUTPUT } else { pipewire::spa::sys::SPA_DIRECTION_INPUT };
        let mut ports: Vec<Port> = match card {
            Some((c, dev)) => {
                let profile = store.active_profile(c).map(|p| p.index);
                store
                    .enum_routes(c)
                    .filter(|r| r.direction == route_direction)
                    .filter(|r| r.devices.is_empty() || r.devices.contains(&dev))
                    .filter(|r| profile.is_none_or(|pi| r.profiles.is_empty() || r.profiles.contains(&pi)))
                    .map(|r| Port {
                        name: r.name.clone(),
                        description: if r.description.is_empty() { r.name.clone() } else { r.description.clone() },
                        available: availability(r.available),
                        priority: r.priority.max(0) as u32,
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        ports.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.name.cmp(&b.name)));

        let name = get(p, "node.name").unwrap_or_default().to_string();
        let description = get(p, "node.description").or(get(p, "node.nick")).unwrap_or(&name).to_string();
        let mut bus = match card_props {
            Some(cp) => bus_of(cp),
            None if get(p, "device.api") == Some("bluez5") => Bus::Bluetooth,
            None => Bus::Virtual,
        };
        let port_hint = route.map(|r| r.name.to_lowercase()).unwrap_or_default();
        let lower = format!("{} {}", name.to_lowercase(), description.to_lowercase());
        if bus != Bus::Bluetooth && card.is_some() && (port_hint.contains("hdmi") || lower.contains("hdmi") || lower.contains("displayport")) {
            bus = Bus::Display;
        }
        let form_factor = get(p, "device.form-factor").or(card_props.and_then(|cp| get(cp, "device.form-factor")));
        let port_type = route.and_then(|r| r.info.get("port.type").map(String::as_str));
        let icon_name = get(p, "device.icon-name")
            .or(get(p, "device.icon_name"))
            .map(str::to_string)
            .or_else(|| form_factor.and_then(|f| form_factor_icon(f, kind)).map(str::to_string))
            .or_else(|| port_type.and_then(|t| form_factor_icon(t, kind)).map(str::to_string))
            .or_else(|| card_props.and_then(|cp| get(cp, "device.icon-name").or(get(cp, "device.icon_name"))).map(str::to_string))
            .unwrap_or_else(|| match (kind, bus) {
                (DeviceKind::Sink, Bus::Display) => "video-display".into(),
                (DeviceKind::Sink, Bus::Bluetooth) => "audio-headphones".into(),
                (DeviceKind::Sink, _) => "audio-speakers".into(),
                (_, Bus::Bluetooth) => "audio-headset".into(),
                _ => "audio-input-microphone".into(),
            });
        let priority = p.get("priority.session").or(p.get("priority.driver")).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        devices.push((
            priority,
            Device {
                id,
                is_ours: is_true(p, OURS_KEY),
                name,
                description,
                icon_name,
                kind,
                volume,
                mute,
                active_port: route.map(|r| r.name.clone()).filter(|n| ports.iter().any(|p| &p.name == n)),
                ports,
                card_id: card.map(|(c, _)| c),
                bus,
                is_virtual: card.is_none(),
                hidden: false,
                level: levels.get(&Target::Device(id)).copied(),
            },
        ));
    }
    devices.sort_by(|(pa, a), (pb, b)| pb.cmp(pa).then_with(|| a.description.cmp(&b.description)).then(a.id.cmp(&b.id)));
    state.devices = devices.into_iter().map(|(_, d)| d).collect();

    // Defaults.
    let default_of = |key: &str, kind: DeviceKind| {
        let name = metadata_name(store.meta_value("default", 0, key)?)?;
        state.devices.iter().find(|d| d.kind == kind && d.name == name).map(|d| d.id)
    };
    let default_sink = default_of("default.audio.sink", DeviceKind::Sink);
    let default_source = default_of("default.audio.source", DeviceKind::Source);
    state.default_sink = default_sink;
    state.default_source = default_source;

    // Streams.
    for (&id, node) in &store.nodes {
        let p = &node.props;
        let Some(kind) = get(p, "media.class").and_then(stream_kind) else { continue };
        if is_meter(p) {
            continue;
        }
        let client = p.get("client.id").and_then(|c| c.parse::<Id>().ok()).and_then(|c| store.clients.get(&c));
        let pick = |key: &str| get(p, key).or_else(|| client.and_then(|c| get(c, key)));
        // A link to a device (other links go to meters, filters, recorders).
        let linked = store.links.values().find_map(|l| match kind {
            StreamKind::Playback if l.output_node == id => Some(l.input_node),
            StreamKind::Record if l.input_node == id => Some(l.output_node),
            _ => None,
        }.filter(|n| state.device(*n).is_some()));
        let target = linked.or_else(|| {
            let v = store.meta_value("default", id, "target.object").or(get(p, "target.object"))?;
            let n = if v.chars().all(|c| c.is_ascii_digit()) { store.node_by_serial(v) } else { store.node_by_name(v) }?;
            state.device(n).map(|d| d.id)
        });
        let props_param = store.node_props_param(id);
        let corked = is_true(p, "pulse.corked") || node.state != RunState::Running;
        state.streams.push(Stream {
            id,
            app_name: app_name(pick("application.name").or(get(p, "node.description")).or(get(p, "node.name")).unwrap_or_default()),
            icon_name: pick("application.icon-name").or(pick("application.icon_name")).unwrap_or_default().to_string(),
            media_name: get(p, "media.name").unwrap_or_default().to_string(),
            binary: pick("application.process.binary").unwrap_or_default().to_string(),
            kind,
            volume: props_param.map(volume_of).unwrap_or_default(),
            mute: props_param.and_then(|v| v.mute).unwrap_or(false),
            target_device: target,
            corked,
            level: levels.get(&Target::Stream(id)).copied(),
        });
    }

    // Server.
    let setting = |key: &str| store.meta_value("settings", 0, key).and_then(|v| v.trim().parse::<u32>().ok());
    state.server = ServerInfo {
        name: store.core_name.clone(),
        version: store.core_version.clone(),
        rate: setting("clock.rate").unwrap_or(0),
        allowed_rates: store.meta_value("settings", 0, "clock.allowed-rates").map(rate_list).unwrap_or_default(),
        quantum: setting("clock.quantum").unwrap_or(0),
        min_quantum: setting("clock.min-quantum").unwrap_or(0),
        max_quantum: setting("clock.max-quantum").unwrap_or(0),
        forced_rate: setting("clock.force-rate").filter(|r| *r != 0),
        forced_quantum: setting("clock.force-quantum").filter(|q| *q != 0),
    };

    // Graph.
    for (&id, node) in &store.nodes {
        let p = &node.props;
        let Some(class) = get(p, "media.class") else { continue };
        if is_true(p, METER_KEY) {
            continue;
        }
        state.graph.nodes.push(GraphNode {
            id,
            name: get(p, "node.nick").or(get(p, "node.description")).or(get(p, "node.name")).unwrap_or_default().to_string(),
            media_class: class.to_string(),
        });
    }
    let has_node = |n: Id| state.graph.nodes.iter().any(|g| g.id == n);
    for (&id, port) in &store.ports {
        let Some(node) = port.props.get("node.id").and_then(|n| n.parse::<Id>().ok()) else { continue };
        if !has_node(node) {
            continue;
        }
        let output = match port.props.get("port.direction").map(String::as_str) {
            Some("out") => true,
            Some("in") => false,
            _ => port.output,
        };
        state.graph.ports.push(GraphPort {
            id,
            node,
            name: get(&port.props, "port.alias").or(get(&port.props, "port.name")).unwrap_or_default().to_string(),
            direction: if output { Direction::Output } else { Direction::Input },
            channel: port.props.get("audio.channel").and_then(|c| position_named(c)),
            is_monitor: is_true(&port.props, "port.monitor"),
        });
    }
    for (&id, l) in &store.links {
        let has_port = |p: Id| state.graph.ports.iter().any(|g| g.id == p);
        if has_node(l.output_node) && has_node(l.input_node) && has_port(l.output_port) && has_port(l.input_port) {
            state.graph.links.push(Link { id, output_node: l.output_node, output_port: l.output_port, input_node: l.input_node, input_port: l.input_port, active: l.active });
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use pipewire::spa::sys::*;

    fn props(pairs: &[(&str, &str)]) -> Props {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn cubic_round_trips_and_matches_wpctl() {
        // wpctl shows 0.87 for a linear 0.658503.
        assert!((cubic(0.658503) - 0.87).abs() < 1e-4);
        for v in [0.0, 0.1, 0.5, 0.94, 1.0, 1.5] {
            assert!((cubic(linear(v)) - v).abs() < 1e-5);
        }
        assert_eq!(linear(-1.0), 0.0);
    }

    #[test]
    fn channel_volumes_follow_positions() {
        let current = Volume { channels: vec![0.5, 0.5], positions: vec![ChannelPosition::FL, ChannelPosition::FR] };
        let want = Volume { channels: vec![1.0, 0.5], positions: vec![ChannelPosition::FR, ChannelPosition::FL] };
        assert_eq!(linear_channels(&current, &want), vec![0.125, 1.0]);
        // Balance math on the cubic scale, then to linear.
        let balanced = current.with_combined(0.8).with_balance(0.5);
        let lin = linear_channels(&current, &balanced);
        assert!((lin[0] - 0.4f32.powi(3)).abs() < 1e-6 && (lin[1] - 0.8f32.powi(3)).abs() < 1e-6);
        // A mono node gets the combined volume of a stereo request.
        let mono = Volume::mono(0.2);
        assert_eq!(linear_channels(&mono, &Volume::stereo(1.0)), vec![1.0]);
    }

    #[test]
    fn codecs_from_names_and_descriptions() {
        assert_eq!(codec("a2dp-sink-ldac", "High Fidelity Playback (A2DP Sink, codec LDAC)").as_deref(), Some("LDAC"));
        assert_eq!(codec("a2dp-sink-aptx_hd", "").as_deref(), Some("aptX HD"));
        assert_eq!(codec("headset-head-unit-msbc", "Headset Head Unit (HSP/HFP)").as_deref(), Some("mSBC"));
        assert_eq!(codec("a2dp-sink", "High Fidelity Playback (A2DP Sink)"), None);
        assert_eq!(codec("output:analog-stereo", "Analog Stereo Output"), None);
    }

    #[test]
    fn metadata_and_settings_parse() {
        assert_eq!(metadata_name(r#"{ "name": "alsa_output.usb-x.pro-output-0" }"#).as_deref(), Some("alsa_output.usb-x.pro-output-0"));
        assert_eq!(metadata_name("-1"), None);
        assert_eq!(rate_list("[ 44100 48000 ]"), vec![44100, 48000]);
        assert_eq!(rate_list("[44100,96000]"), vec![44100, 96000]);
        assert_eq!(position_named("AUX1"), Some(ChannelPosition::Other(SPA_AUDIO_CHANNEL_AUX0 + 1)));
    }

    fn route(index: i32, name: &str, dir: u32, device: Option<i32>, profiles: Vec<i32>, vols: Option<Vec<f32>>) -> RouteParam {
        RouteParam {
            index,
            direction: dir,
            device,
            name: name.into(),
            description: name.to_uppercase(),
            priority: index * 10,
            available: SPA_PARAM_AVAILABILITY_yes,
            profiles,
            devices: vec![device.unwrap_or(0)],
            props: vols.map(|v| VolumeProps { volumes: v, map: vec![SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR], mute: Some(false) }),
            info: BTreeMap::new(),
        }
    }

    fn fake() -> Store {
        let mut s = Store { core_name: "pipewire-0".into(), core_version: "1.6.7".into(), ..Default::default() };
        s.devices.insert(10, props(&[("media.class", "Audio/Device"), ("device.name", "alsa_card.pci"), ("device.description", "Built-in Audio"), ("device.bus", "pci"), ("device.api", "alsa")]));
        s.param(10, params::ENUM_PROFILE, 1, Some(Param::Profile(ProfileParam { index: 0, name: "off".into(), description: "Off".into(), ..Default::default() })));
        s.param(10, params::ENUM_PROFILE, 1, Some(Param::Profile(ProfileParam { index: 1, name: "output:analog-stereo".into(), description: "Analog Stereo Output".into(), priority: 6500, sinks: 1, available: SPA_PARAM_AVAILABILITY_yes, ..Default::default() })));
        s.param(10, params::PROFILE, 2, Some(Param::Profile(ProfileParam { index: 1, name: "output:analog-stereo".into(), ..Default::default() })));
        s.param(10, params::ENUM_ROUTE, 3, Some(Param::Route(route(0, "analog-output-speaker", SPA_DIRECTION_OUTPUT, Some(0), vec![1], None))));
        s.param(10, params::ENUM_ROUTE, 3, Some(Param::Route(route(1, "analog-output-headphones", SPA_DIRECTION_OUTPUT, Some(0), vec![1], None))));
        s.param(10, params::ENUM_ROUTE, 3, Some(Param::Route(route(2, "analog-input-mic", SPA_DIRECTION_INPUT, Some(1), vec![2], None))));
        s.param(10, params::ROUTE, 4, Some(Param::Route(route(0, "analog-output-speaker", SPA_DIRECTION_OUTPUT, Some(0), vec![1], Some(vec![0.125, 0.125])))));
        s.nodes.insert(20, NodeG { props: props(&[("media.class", "Audio/Sink"), ("node.name", "alsa_output.pci.analog-stereo"), ("node.description", "Speakers"), ("device.id", "10"), ("card.profile.device", "0"), ("priority.session", "1000"), ("object.serial", "200")]), state: RunState::Running });
        // The node's own props say something else: the route wins.
        s.param(20, params::PROPS, 5, Some(Param::Props(VolumeProps { volumes: vec![1.0, 1.0], map: vec![SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR], mute: Some(true) })));
        s.nodes.insert(21, NodeG { props: props(&[("media.class", "Audio/Sink"), ("node.name", "hokum-sound-music"), ("node.description", "Music"), (OURS_KEY, "true"), ("object.serial", "210")]), state: RunState::Suspended });
        s.param(21, params::PROPS, 6, Some(Param::Props(VolumeProps { volumes: vec![0.001], map: vec![SPA_AUDIO_CHANNEL_MONO], mute: Some(false) })));
        s.nodes.insert(30, NodeG { props: props(&[("media.class", "Stream/Output/Audio"), ("node.name", "spotify"), ("media.name", "Song"), ("client.id", "40"), ("object.serial", "300")]), state: RunState::Running });
        s.param(30, params::PROPS, 7, Some(Param::Props(VolumeProps { volumes: vec![0.216, 0.216], map: vec![], mute: Some(false) })));
        s.nodes.insert(31, NodeG { props: props(&[("media.class", "Stream/Input/Audio"), ("node.name", "pavucontrol"), ("stream.monitor", "true")]), state: RunState::Running });
        s.nodes.insert(32, NodeG { props: props(&[("media.class", "Stream/Output/Audio"), ("application.name", "PipeWire ALSA [mpv]"), ("object.serial", "320")]), state: RunState::Idle });
        s.clients.insert(40, props(&[("application.name", "Spotify"), ("application.icon-name", "spotify"), ("application.process.binary", "spotify")]));
        s.ports.insert(50, PortG { props: props(&[("node.id", "30"), ("port.name", "output_FL"), ("port.direction", "out"), ("audio.channel", "FL")]), output: true });
        s.ports.insert(51, PortG { props: props(&[("node.id", "20"), ("port.name", "playback_FL"), ("port.direction", "in"), ("audio.channel", "FL")]), output: false });
        // pavucontrol metering spotify: not where it plays.
        s.links.insert(59, LinkG { output_node: 30, output_port: 50, input_node: 31, input_port: 99, active: true });
        s.links.insert(60, LinkG { output_node: 30, output_port: 50, input_node: 20, input_port: 51, active: true });
        s.metadata.insert(70, MetadataG { name: "default".into(), entries: [((0, "default.audio.sink".to_string()), r#"{"name":"alsa_output.pci.analog-stereo"}"#.to_string()), ((32, "target.object".to_string()), "210".to_string())].into() });
        s.metadata.insert(71, MetadataG { name: "settings".into(), entries: [((0, "clock.rate".to_string()), "48000".to_string()), ((0, "clock.allowed-rates".to_string()), "[ 44100 48000 ]".to_string()), ((0, "clock.quantum".to_string()), "1024".to_string()), ((0, "clock.force-quantum".to_string()), "0".to_string()), ((0, "clock.force-rate".to_string()), "96000".to_string())].into() });
        s
    }

    #[test]
    fn state_from_fake_globals() {
        let s = fake();
        let state = build(&s, &[(Target::Stream(30), 0.5)].into());
        assert_eq!(state.devices.iter().map(|d| d.id).collect::<Vec<_>>(), vec![20, 21]);
        let speakers = state.device(20).unwrap();
        assert_eq!(speakers.card_id, Some(10));
        assert_eq!(speakers.bus, Bus::Pci);
        assert!((speakers.volume.combined() - 0.5).abs() < 1e-5);
        assert!(!speakers.mute);
        assert_eq!(speakers.ports.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["analog-output-headphones", "analog-output-speaker"]);
        assert_eq!(speakers.active_port.as_deref(), Some("analog-output-speaker"));
        let ours = state.device(21).unwrap();
        assert!(ours.is_ours && ours.is_virtual && ours.card_id.is_none());
        assert!((ours.volume.combined() - 0.1).abs() < 1e-5);
        assert_eq!(state.default_sink, Some(20));
        assert_eq!(state.default_source, None);

        let card = state.card(10).unwrap();
        assert_eq!(card.active_profile.as_deref(), Some("output:analog-stereo"));
        assert_eq!(card.profiles[0].kind, ProfileKind::Off);
        assert_eq!(card.profiles[1].kind, ProfileKind::Output);

        assert_eq!(state.streams.len(), 2, "the peak-detect stream is hidden");
        let spotify = state.stream(30).unwrap();
        assert_eq!((spotify.app_name.as_str(), spotify.icon_name.as_str(), spotify.binary.as_str()), ("Spotify", "spotify", "spotify"));
        assert_eq!(spotify.target_device, Some(20));
        assert!((spotify.volume.combined() - 0.6).abs() < 1e-5);
        assert_eq!(spotify.level, Some(0.5));
        assert!(!spotify.corked);
        let mpv = state.stream(32).unwrap();
        assert_eq!(mpv.app_name, "mpv");
        assert!(mpv.corked);
        assert_eq!(mpv.target_device, Some(21), "unlinked: from target.object metadata (a serial)");

        assert_eq!(state.server.rate, 48000);
        assert_eq!(state.server.allowed_rates, vec![44100, 48000]);
        assert_eq!(state.server.forced_quantum, None);
        assert_eq!(state.server.forced_rate, Some(96000));
        assert_eq!(state.graph.links.len(), 1);
        assert_eq!(state.graph.ports.len(), 2);
    }

    #[test]
    fn empty_enumerations_clear_params() {
        let mut s = fake();
        assert_eq!(s.enum_routes(10).count(), 3);
        // A new request that returned nothing.
        s.param_request(10, params::ENUM_ROUTE);
        s.param_done(10, params::ENUM_ROUTE);
        assert_eq!(s.enum_routes(10).count(), 0);
        // A new request replaces, not appends, even if the remote reuses a seq.
        s.param(10, params::ENUM_ROUTE, 3, Some(Param::Route(route(0, "x", SPA_DIRECTION_OUTPUT, Some(0), vec![], None))));
        s.param(10, params::ENUM_ROUTE, 3, Some(Param::Route(route(1, "y", SPA_DIRECTION_OUTPUT, Some(0), vec![], None))));
        s.param_request(10, params::ENUM_ROUTE);
        s.param(10, params::ENUM_ROUTE, 3, Some(Param::Route(route(0, "x", SPA_DIRECTION_OUTPUT, Some(0), vec![], None))));
        s.param_done(10, params::ENUM_ROUTE);
        assert_eq!(s.enum_routes(10).count(), 1);
        // Spontaneous results (a new seq) replace too.
        s.param(10, params::ENUM_ROUTE, 4, Some(Param::Route(route(2, "z", SPA_DIRECTION_OUTPUT, Some(0), vec![], None))));
        assert_eq!(s.enum_routes(10).map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["z"]);
        s.remove(10);
        assert_eq!(s.enum_routes(10).count(), 0);
        let state = build(&s, &HashMap::new());
        assert_eq!(state.device(20).unwrap().card_id, None, "a removed card no longer counts");
    }
}
