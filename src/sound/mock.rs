//! A fake audio system for UI work and headless scenes: a laptop's speakers and microphone,
//! HDMI, USB headphones and a USB microphone, a Bluetooth headset with codecs, and a few apps.
//! It applies every [`Command`] the way PipeWire would and animates levels for whatever is
//! metered. `HOKUM_SOUND_BACKEND=mock` runs Hokum on it.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::sound::model::*;

const SPEAKERS: Id = 41;
const HDMI: Id = 42;
const HEADPHONES: Id = 43;
const HEADSET: Id = 44;
const MIC: Id = 45;
const USB_MIC: Id = 46;
const HEADSET_MIC: Id = 47;
const SPEAKERS_MONITOR: Id = 48;

const CARD_PCI: Id = 30;
const CARD_HDMI: Id = 31;
const CARD_USB_DAC: Id = 32;
const CARD_HEADSET: Id = 33;
const CARD_USB_MIC: Id = 34;

const SPOTIFY: Id = 60;
const FIREFOX: Id = 61;
const DISCORD_OUT: Id = 62;
const DISCORD_IN: Id = 63;
const OBS: Id = 64;

fn port(name: &str, description: &str, available: Availability, priority: u32) -> Port {
    Port {
        name: name.into(),
        description: description.into(),
        available,
        priority,
    }
}

fn profile(
    name: &str,
    description: &str,
    kind: ProfileKind,
    codec: Option<&str>,
    available: bool,
    priority: u32,
) -> Profile {
    Profile {
        name: name.into(),
        description: description.into(),
        available,
        priority,
        kind,
        codec: codec.map(Into::into),
    }
}

#[allow(clippy::too_many_arguments)]
fn device(
    id: Id,
    name: &str,
    description: &str,
    icon: &str,
    kind: DeviceKind,
    volume: Volume,
    card: Option<Id>,
    bus: Bus,
) -> Device {
    Device {
        id,
        name: name.into(),
        description: description.into(),
        icon_name: icon.into(),
        kind,
        volume,
        mute: false,
        ports: Vec::new(),
        active_port: None,
        card_id: card,
        bus,
        is_virtual: card.is_none(),
        is_ours: false,
        hidden: kind == DeviceKind::Monitor,
        level: None,
    }
}

fn stream(
    id: Id,
    app: &str,
    icon: &str,
    media: &str,
    binary: &str,
    kind: StreamKind,
    volume: f32,
    target: Id,
) -> Stream {
    Stream {
        id,
        app_name: app.into(),
        icon_name: icon.into(),
        media_name: media.into(),
        binary: binary.into(),
        kind,
        volume: Volume::stereo(volume),
        mute: false,
        target_device: Some(target),
        corked: false,
        level: None,
    }
}

/// The fake system at rest: what scenes render (levels filled in so meters show).
pub fn sample() -> State {
    let mut speakers = device(
        SPEAKERS,
        "alsa_output.pci-0000_00_1f.3.analog-stereo",
        "Speakers",
        "audio-speakers",
        DeviceKind::Sink,
        Volume::stereo(0.62),
        Some(CARD_PCI),
        Bus::Pci,
    );
    speakers.ports = vec![
        port(
            "analog-output-speaker",
            "Speakers",
            Availability::Unknown,
            100,
        ),
        port(
            "analog-output-headphones",
            "Headphones",
            Availability::No,
            200,
        ),
    ];
    speakers.active_port = Some("analog-output-speaker".into());
    let mut hdmi = device(
        HDMI,
        "alsa_output.pci-0000_01_00.1.hdmi-stereo",
        "LG UltraFine",
        "video-display",
        DeviceKind::Sink,
        Volume::stereo(1.0),
        Some(CARD_HDMI),
        Bus::Display,
    );
    hdmi.ports = vec![port(
        "hdmi-output-0",
        "HDMI / DisplayPort",
        Availability::Yes,
        590,
    )];
    hdmi.active_port = Some("hdmi-output-0".into());
    let headphones = device(
        HEADPHONES,
        "alsa_output.usb-Apple_USB-C_to_3.5mm_Headphone_Jack_Adapter-00.analog-stereo",
        "USB-C Headphones",
        "audio-headphones",
        DeviceKind::Sink,
        Volume::stereo(0.45),
        Some(CARD_USB_DAC),
        Bus::Usb,
    );
    let mut headset = device(
        HEADSET,
        "bluez_output.AC_80_0A_12_34_56.1",
        "WH-1000XM4",
        "audio-headset",
        DeviceKind::Sink,
        Volume::stereo(0.8),
        Some(CARD_HEADSET),
        Bus::Bluetooth,
    );
    headset.volume = Volume {
        channels: vec![0.8, 0.72],
        positions: vec![ChannelPosition::FL, ChannelPosition::FR],
    };
    let mut mic = device(
        MIC,
        "alsa_input.pci-0000_00_1f.3.analog-stereo",
        "Microphone",
        "audio-input-microphone",
        DeviceKind::Source,
        Volume::stereo(0.7),
        Some(CARD_PCI),
        Bus::Pci,
    );
    mic.ports = vec![
        port(
            "analog-input-internal-mic",
            "Internal Microphone",
            Availability::Unknown,
            89,
        ),
        port("analog-input-mic", "Microphone Jack", Availability::No, 87),
    ];
    mic.active_port = Some("analog-input-internal-mic".into());
    let usb_mic = device(
        USB_MIC,
        "alsa_input.usb-Shure_Inc_Shure_MV7-00.mono-fallback",
        "Shure MV7",
        "audio-input-microphone",
        DeviceKind::Source,
        Volume::mono(0.85),
        Some(CARD_USB_MIC),
        Bus::Usb,
    );
    let mut headset_mic = device(
        HEADSET_MIC,
        "bluez_input.AC_80_0A_12_34_56.0",
        "WH-1000XM4",
        "audio-headset",
        DeviceKind::Source,
        Volume::mono(1.0),
        Some(CARD_HEADSET),
        Bus::Bluetooth,
    );
    headset_mic.hidden = true;
    let mut monitor = device(
        SPEAKERS_MONITOR,
        "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor",
        "Monitor of Speakers",
        "audio-speakers",
        DeviceKind::Monitor,
        Volume::stereo(1.0),
        Some(CARD_PCI),
        Bus::Pci,
    );
    monitor.is_virtual = false;

    let cards = vec![
        Card {
            id: CARD_PCI,
            name: "alsa_card.pci-0000_00_1f.3".into(),
            description: "Built-in Audio".into(),
            icon_name: "audio-card".into(),
            bus: Bus::Pci,
            profiles: vec![
                profile(
                    "output:analog-stereo+input:analog-stereo",
                    "Analog Stereo Duplex",
                    ProfileKind::Duplex,
                    None,
                    true,
                    6565,
                ),
                profile(
                    "output:analog-stereo",
                    "Analog Stereo Output",
                    ProfileKind::Output,
                    None,
                    true,
                    6500,
                ),
                profile("off", "Off", ProfileKind::Off, None, true, 0),
            ],
            active_profile: Some("output:analog-stereo+input:analog-stereo".into()),
            battery: None,
        },
        Card {
            id: CARD_HDMI,
            name: "alsa_card.pci-0000_01_00.1".into(),
            description: "GPU HDMI Audio".into(),
            icon_name: "video-display".into(),
            bus: Bus::Display,
            profiles: vec![
                profile(
                    "output:hdmi-stereo",
                    "Digital Stereo (HDMI) Output",
                    ProfileKind::Output,
                    None,
                    true,
                    5900,
                ),
                profile(
                    "output:hdmi-surround",
                    "Digital Surround 5.1 (HDMI) Output",
                    ProfileKind::Output,
                    None,
                    true,
                    800,
                ),
                profile("off", "Off", ProfileKind::Off, None, true, 0),
            ],
            active_profile: Some("output:hdmi-stereo".into()),
            battery: None,
        },
        Card {
            id: CARD_USB_DAC,
            name: "alsa_card.usb-Apple_USB-C_to_3.5mm_Headphone_Jack_Adapter-00".into(),
            description: "USB-C Headphones".into(),
            icon_name: "audio-headphones".into(),
            bus: Bus::Usb,
            profiles: vec![
                profile(
                    "output:analog-stereo",
                    "Analog Stereo Output",
                    ProfileKind::Output,
                    None,
                    true,
                    6500,
                ),
                profile("off", "Off", ProfileKind::Off, None, true, 0),
            ],
            active_profile: Some("output:analog-stereo".into()),
            battery: None,
        },
        Card {
            id: CARD_HEADSET,
            name: "bluez_card.AC_80_0A_12_34_56".into(),
            description: "WH-1000XM4".into(),
            icon_name: "audio-headset".into(),
            bus: Bus::Bluetooth,
            profiles: vec![
                profile(
                    "a2dp-sink-ldac",
                    "High Fidelity Playback (A2DP Sink, codec LDAC)",
                    ProfileKind::Output,
                    Some("LDAC"),
                    true,
                    40,
                ),
                profile(
                    "a2dp-sink-aac",
                    "High Fidelity Playback (A2DP Sink, codec AAC)",
                    ProfileKind::Output,
                    Some("AAC"),
                    true,
                    30,
                ),
                profile(
                    "a2dp-sink-sbc",
                    "High Fidelity Playback (A2DP Sink, codec SBC)",
                    ProfileKind::Output,
                    Some("SBC"),
                    true,
                    20,
                ),
                profile(
                    "headset-head-unit-msbc",
                    "Headset Head Unit (HSP/HFP, codec mSBC)",
                    ProfileKind::Duplex,
                    Some("mSBC"),
                    true,
                    10,
                ),
                profile(
                    "headset-head-unit-cvsd",
                    "Headset Head Unit (HSP/HFP, codec CVSD)",
                    ProfileKind::Duplex,
                    Some("CVSD"),
                    true,
                    5,
                ),
                profile("off", "Off", ProfileKind::Off, None, true, 0),
            ],
            active_profile: Some("a2dp-sink-ldac".into()),
            battery: Some(0.7),
        },
        Card {
            id: CARD_USB_MIC,
            name: "alsa_card.usb-Shure_Inc_Shure_MV7-00".into(),
            description: "Shure MV7".into(),
            icon_name: "audio-input-microphone".into(),
            bus: Bus::Usb,
            profiles: vec![
                profile(
                    "input:mono-fallback",
                    "Mono Input",
                    ProfileKind::Input,
                    None,
                    true,
                    100,
                ),
                profile("off", "Off", ProfileKind::Off, None, true, 0),
            ],
            active_profile: Some("input:mono-fallback".into()),
            battery: None,
        },
    ];

    let mut firefox = stream(
        FIREFOX,
        "Firefox",
        "firefox",
        "YouTube — Lo-fi beats to code to",
        "firefox",
        StreamKind::Playback,
        1.0,
        SPEAKERS,
    );
    firefox.corked = true;
    let streams = vec![
        stream(
            SPOTIFY,
            "Spotify",
            "spotify-client",
            "Daft Punk — Digital Love",
            "spotify",
            StreamKind::Playback,
            0.8,
            SPEAKERS,
        ),
        firefox,
        stream(
            DISCORD_OUT,
            "Discord",
            "discord",
            "Voice — #engineering",
            "Discord",
            StreamKind::Playback,
            0.9,
            HEADSET,
        ),
        stream(
            DISCORD_IN,
            "Discord",
            "discord",
            "Voice — #engineering",
            "Discord",
            StreamKind::Record,
            1.0,
            USB_MIC,
        ),
        stream(
            OBS,
            "OBS Studio",
            "com.obsproject.Studio",
            "Desktop Audio",
            "obs",
            StreamKind::Record,
            1.0,
            SPEAKERS_MONITOR,
        ),
    ];

    let devices = vec![
        speakers,
        hdmi,
        headphones,
        headset,
        mic,
        usb_mic,
        headset_mic,
        monitor,
    ];
    let mut state = State {
        revision: 0,
        connection: Connection::Connected,
        default_sink: Some(SPEAKERS),
        default_source: Some(USB_MIC),
        graph: Graph::default(),
        devices,
        cards,
        streams,
        server: ServerInfo {
            name: "pipewire-0".into(),
            version: "1.4.9".into(),
            rate: 48000,
            allowed_rates: vec![44100, 48000, 96000],
            quantum: 1024,
            min_quantum: 32,
            max_quantum: 2048,
            forced_rate: None,
            forced_quantum: None,
        },
        issues: Vec::new(),
    };
    state.graph = graph(&state);
    for d in &mut state.devices {
        d.level = Some(
            if d.kind == DeviceKind::Sink && Some(d.id) == state.default_sink {
                0.55
            } else {
                0.0
            },
        );
    }
    for s in &mut state.streams {
        s.level = Some(if s.corked { 0.0 } else { 0.4 });
    }
    state
}

/// A plausible patchbay: stereo ports on every node, links from each stream to its device.
fn graph(state: &State) -> Graph {
    let mut g = Graph::default();
    let mut next = 1000;
    let mut ports = |g: &mut Graph, node: Id, direction: Direction, prefix: &str, monitor: bool| {
        let mut ids = Vec::new();
        for (ch, pos) in [("FL", ChannelPosition::FL), ("FR", ChannelPosition::FR)] {
            next += 1;
            g.ports.push(GraphPort {
                id: next,
                node,
                name: format!("{prefix}_{ch}"),
                direction,
                channel: Some(pos),
                is_monitor: monitor,
            });
            ids.push(next);
        }
        ids
    };
    let mut inputs = std::collections::HashMap::new();
    let mut outputs = std::collections::HashMap::new();
    for d in &state.devices {
        let class = match d.kind {
            DeviceKind::Sink => "Audio/Sink",
            DeviceKind::Source | DeviceKind::Monitor => "Audio/Source",
        };
        g.nodes.push(GraphNode {
            id: d.id,
            name: d.description.clone(),
            media_class: class.into(),
        });
        match d.kind {
            DeviceKind::Sink => {
                inputs.insert(
                    d.id,
                    ports(&mut g, d.id, Direction::Input, "playback", false),
                );
                outputs.insert(
                    d.id,
                    ports(&mut g, d.id, Direction::Output, "monitor", true),
                );
            }
            _ => {
                outputs.insert(
                    d.id,
                    ports(&mut g, d.id, Direction::Output, "capture", false),
                );
            }
        }
    }
    let mut link_id = 2000;
    for s in &state.streams {
        let (class, dir, prefix) = match s.kind {
            StreamKind::Playback => ("Stream/Output/Audio", Direction::Output, "output"),
            StreamKind::Record => ("Stream/Input/Audio", Direction::Input, "input"),
        };
        g.nodes.push(GraphNode {
            id: s.id,
            name: s.app_name.clone(),
            media_class: class.into(),
        });
        let own = ports(&mut g, s.id, dir, prefix, false);
        let Some(target) = s.target_device else {
            continue;
        };
        let peer = match s.kind {
            StreamKind::Playback => inputs.get(&target),
            StreamKind::Record => outputs.get(&target),
        };
        for (i, p) in peer.into_iter().flatten().enumerate() {
            link_id += 1;
            let (op, ip, on, inode) = match s.kind {
                StreamKind::Playback => (own[i], *p, s.id, target),
                StreamKind::Record => (*p, own[i], target, s.id),
            };
            g.links.push(Link {
                id: link_id,
                output_node: on,
                output_port: op,
                input_node: inode,
                input_port: ip,
                active: !s.corked,
            });
        }
    }
    g
}

/// Applies `command` the way PipeWire and the session manager would.
pub fn apply(state: &mut State, command: Command, metered: &mut HashSet<Target>) {
    fn volume_of(state: &mut State, target: Target) -> Option<(&mut Volume, &mut bool)> {
        match target {
            Target::Device(id) => state
                .devices
                .iter_mut()
                .find(|d| d.id == id)
                .map(|d| (&mut d.volume, &mut d.mute)),
            Target::Stream(id) => state
                .streams
                .iter_mut()
                .find(|s| s.id == id)
                .map(|s| (&mut s.volume, &mut s.mute)),
        }
    }
    let mut regraph = false;
    match command {
        Command::SetVolume { target, volume } => {
            if let Some((v, _)) = volume_of(state, target) {
                *v = v.with_combined(volume);
            }
        }
        Command::SetChannelVolumes { target, volume } => {
            if let Some((v, _)) = volume_of(state, target) {
                *v = volume;
            }
        }
        Command::SetMute { target, mute } => {
            if let Some((_, m)) = volume_of(state, target) {
                *m = mute;
            }
        }
        Command::SetBalance { target, balance } => {
            if let Some((v, _)) = volume_of(state, target) {
                *v = v.with_balance(balance);
            }
        }
        Command::SetDefault { device } => match state.device(device).map(|d| d.kind) {
            Some(DeviceKind::Sink) => {
                state.default_sink = Some(device);
                // Streams without a pinned target follow the default.
                for s in state
                    .streams
                    .iter_mut()
                    .filter(|s| s.kind == StreamKind::Playback)
                {
                    s.target_device = Some(device);
                }
                regraph = true;
            }
            Some(_) => state.default_source = Some(device),
            None => {}
        },
        Command::SetPort { device, port } => {
            if let Some(d) = state.devices.iter_mut().find(|d| d.id == device)
                && d.ports.iter().any(|p| p.name == port)
            {
                d.active_port = Some(port);
            }
        }
        Command::SetProfile { card, profile } => {
            if let Some(c) = state.cards.iter_mut().find(|c| c.id == card)
                && c.profiles.iter().any(|p| p.name == profile)
            {
                c.active_profile = Some(profile);
            }
        }
        Command::MoveStream { stream, device } => {
            if state.device(device).is_some()
                && let Some(s) = state.streams.iter_mut().find(|s| s.id == stream)
            {
                s.target_device = Some(device);
                regraph = true;
            }
        }
        Command::SetMetering { targets, on } => {
            for t in targets {
                if on {
                    metered.insert(t);
                } else {
                    metered.remove(&t);
                }
            }
        }
        Command::CreateVirtualSink { name } => {
            let id = state
                .devices
                .iter()
                .map(|d| d.id)
                .chain(state.streams.iter().map(|s| s.id))
                .max()
                .unwrap_or(100)
                + 1;
            let mut d = device(
                id,
                &format!("hokum-sound.{}", name.to_lowercase().replace(' ', "-")),
                &name,
                "audio-speakers",
                DeviceKind::Sink,
                Volume::stereo(1.0),
                None,
                Bus::Virtual,
            );
            d.is_ours = true;
            state.devices.push(d);
            regraph = true;
        }
        Command::RemoveVirtualSink { device } => {
            if state.device(device).is_some_and(|d| d.is_ours) {
                state.devices.retain(|d| d.id != device);
                let fallback = state.sinks().next().map(|d| d.id);
                if state.default_sink == Some(device) {
                    state.default_sink = fallback;
                }
                for s in state
                    .streams
                    .iter_mut()
                    .filter(|s| s.target_device == Some(device))
                {
                    s.target_device = fallback;
                }
                regraph = true;
            }
        }
        Command::SetRateQuantum { rate, quantum } => {
            state.server.forced_rate = rate;
            state.server.forced_quantum = quantum;
            state.server.rate = rate.unwrap_or(48000);
            state.server.quantum = quantum.unwrap_or(1024);
        }
        Command::Link {
            output_port,
            input_port,
        } => {
            let node = |id: Id| {
                state
                    .graph
                    .ports
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.node)
            };
            if let (Some(on), Some(inode)) = (node(output_port), node(input_port)) {
                let id = state.graph.links.iter().map(|l| l.id).max().unwrap_or(2000) + 1;
                state.graph.links.push(Link {
                    id,
                    output_node: on,
                    output_port,
                    input_node: inode,
                    input_port,
                    active: true,
                });
            }
        }
        Command::Unlink { link } => state.graph.links.retain(|l| l.id != link),
        Command::DismissIssue { index } => {
            if index < state.issues.len() {
                state.issues.remove(index);
            }
        }
    }
    if regraph {
        state.graph = graph(state);
    }
}

/// Fake meters: music-ish movement for playing streams and whatever device they feed.
fn animate(state: &mut State, metered: &HashSet<Target>, t: f32) {
    let wobble = |seed: f32| {
        let v = 0.45
            + 0.25 * (t * 7.3 + seed).sin()
            + 0.15 * (t * 13.1 + seed * 2.0).sin()
            + 0.1 * (t * 23.7 + seed * 3.0).sin();
        v.clamp(0.0, 1.0)
    };
    let mut fed: HashSet<Id> = HashSet::new();
    for s in &mut state.streams {
        let playing = !s.corked && !s.mute;
        if playing && let Some(d) = s.target_device {
            fed.insert(d);
        }
        s.level = metered.contains(&Target::Stream(s.id)).then(|| {
            if playing {
                wobble(s.id as f32) * s.volume.combined().min(1.0)
            } else {
                0.0
            }
        });
    }
    for d in &mut state.devices {
        let live = fed.contains(&d.id)
            || (d.kind == DeviceKind::Source && Some(d.id) == state.default_source);
        d.level = metered.contains(&Target::Device(d.id)).then(|| {
            if live && !d.mute {
                wobble(d.id as f32 * 0.7) * d.volume.combined().min(1.0)
            } else {
                0.0
            }
        });
    }
}

/// Runs the fake system on its own thread.
pub fn start() -> Audio {
    let (audio, driver) = channel();
    std::thread::Builder::new()
        .name("mock-audio".into())
        .spawn(move || {
            let begun = Instant::now();
            let mut state = sample();
            for d in &mut state.devices {
                d.level = None;
            }
            for s in &mut state.streams {
                s.level = None;
            }
            let mut metered = HashSet::new();
            driver.publish(state.clone());
            loop {
                let wait = if metered.is_empty() {
                    Duration::from_secs(3600)
                } else {
                    Duration::from_millis(33)
                };
                let mut changed = false;
                match driver.commands().recv_timeout(wait) {
                    Ok(c) => {
                        apply(&mut state, c, &mut metered);
                        while let Ok(c) = driver.commands().try_recv() {
                            apply(&mut state, c, &mut metered);
                        }
                        changed = true;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                if !metered.is_empty() || changed {
                    animate(&mut state, &metered, begun.elapsed().as_secs_f32());
                    driver.publish(state.clone());
                }
            }
        })
        .expect("spawn the mock audio thread");
    audio
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_is_consistent() {
        let s = sample();
        assert!(s.default_sink().is_some() && s.default_source().is_some());
        for d in &s.devices {
            assert!(
                d.card_id.is_none_or(|c| s.card(c).is_some()),
                "{} has no card",
                d.name
            );
        }
        for st in &s.streams {
            assert!(
                st.target_device.is_some_and(|d| s.device(d).is_some()),
                "{} has no device",
                st.app_name
            );
        }
        for l in &s.graph.links {
            assert!(
                s.graph.ports.iter().any(|p| p.id == l.output_port)
                    && s.graph.ports.iter().any(|p| p.id == l.input_port)
            );
        }
    }

    #[test]
    fn commands_change_the_state() {
        let mut s = sample();
        let mut metered = HashSet::new();
        apply(
            &mut s,
            Command::SetDefault { device: HEADSET },
            &mut metered,
        );
        assert_eq!(s.default_sink, Some(HEADSET));
        assert_eq!(s.stream(SPOTIFY).unwrap().target_device, Some(HEADSET));
        apply(
            &mut s,
            Command::SetProfile {
                card: CARD_HEADSET,
                profile: "a2dp-sink-aac".into(),
            },
            &mut metered,
        );
        assert_eq!(
            s.card(CARD_HEADSET)
                .unwrap()
                .profile()
                .unwrap()
                .codec
                .as_deref(),
            Some("AAC")
        );
        apply(
            &mut s,
            Command::CreateVirtualSink {
                name: "Stream Mix".into(),
            },
            &mut metered,
        );
        let mix = s.devices.last().unwrap().id;
        apply(
            &mut s,
            Command::MoveStream {
                stream: SPOTIFY,
                device: mix,
            },
            &mut metered,
        );
        apply(
            &mut s,
            Command::RemoveVirtualSink { device: mix },
            &mut metered,
        );
        assert!(s.device(mix).is_none());
        assert!(s.stream(SPOTIFY).unwrap().target_device.is_some());
        apply(
            &mut s,
            Command::SetMetering {
                targets: vec![Target::Device(SPEAKERS)],
                on: true,
            },
            &mut metered,
        );
        assert!(metered.contains(&Target::Device(SPEAKERS)));
    }
}
