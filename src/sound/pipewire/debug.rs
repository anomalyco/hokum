//! `hokum sound debug ...`: the PipeWire backend without the window, for checking it against
//! `wpctl status` / `pw-dump`. Not in `--help`.
//!
//! ```sh
//! hokum sound debug dump                      # the State as JSON
//! hokum sound debug watch [seconds]           # one line per snapshot
//! hokum sound debug volume <target> <cubic>   # also: mute <target> on|off, balance <target> <-1..1>
//! hokum sound debug default <device>          # port <device> <name>, profile <card> <name>
//! hokum sound debug move <stream> <device>
//! hokum sound debug meter <target>... [--seconds N]
//! hokum sound debug create-virtual <name>     # remove-virtual <device>
//! hokum sound debug rate-quantum <rate|0> <quantum|0>
//! hokum sound debug link <output-port> <input-port>   # unlink <link>
//! ```
//!
//! A target is a node id or a device's/stream's name, description or app name.

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::sound::model::*;

const USAGE: &str = "usage: hokum sound debug dump | watch [s] | volume <t> <v> | mute <t> on|off | balance <t> <b> | default <dev> | port <dev> <name> | profile <card> <name> | move <stream> <dev> | meter <t>... [--seconds N] | create-virtual <name> | remove-virtual <dev> | rate-quantum <rate> <quantum> | link <out> <in> | unlink <link>";

pub fn cli(args: &[String]) -> i32 {
    match run(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("hokum sound debug: {e}");
            1
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let audio = super::start()?;
    let (rx, _sub) = audio.subscribe();
    let state = wait_connected(&audio, &rx)?;
    let arg = |i: usize| {
        args.get(i)
            .map(String::as_str)
            .ok_or_else(|| USAGE.to_string())
    };
    let num = |i: usize| {
        arg(i)?
            .parse::<f32>()
            .map_err(|e| format!("{}: {e}", arg(i).unwrap_or_default()))
    };
    let command = match arg(0)? {
        "dump" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&state_json(&state)).unwrap_or_default()
            );
            return Ok(());
        }
        "watch" => {
            let secs = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);
            let end = Instant::now() + Duration::from_secs_f64(secs);
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                if let Ok(s) = rx.recv_timeout(left) {
                    println!("{}", summary(&s));
                }
            }
            return Ok(());
        }
        "meter" => {
            let mut secs = 3.0;
            let mut targets = Vec::new();
            let mut i = 1;
            while let Some(a) = args.get(i) {
                if a == "--seconds" {
                    secs = num(i + 1)? as f64;
                    i += 2;
                    continue;
                }
                targets.push(target(&state, a)?);
                i += 1;
            }
            audio.send(Command::SetMetering {
                targets: targets.clone(),
                on: true,
            });
            let end = Instant::now() + Duration::from_secs_f64(secs);
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                if let Ok(s) = rx.recv_timeout(left) {
                    let levels: Vec<String> = targets
                        .iter()
                        .map(|t| format!("{t:?}={:.3}", level(&s, *t).unwrap_or(-1.0)))
                        .collect();
                    println!("{}", levels.join(" "));
                }
            }
            audio.send(Command::SetMetering { targets, on: false });
            settle(&rx, Duration::from_millis(300));
            return Ok(());
        }
        "volume" => Command::SetVolume {
            target: target(&state, arg(1)?)?,
            volume: num(2)?,
        },
        "balance" => Command::SetBalance {
            target: target(&state, arg(1)?)?,
            balance: num(2)?,
        },
        "mute" => Command::SetMute {
            target: target(&state, arg(1)?)?,
            mute: matches!(arg(2)?, "on" | "true" | "1"),
        },
        "default" => Command::SetDefault {
            device: device(&state, arg(1)?)?,
        },
        "port" => Command::SetPort {
            device: device(&state, arg(1)?)?,
            port: arg(2)?.into(),
        },
        "profile" => Command::SetProfile {
            card: card(&state, arg(1)?)?,
            profile: arg(2)?.into(),
        },
        "move" => {
            let Target::Stream(stream) = target(&state, arg(1)?)? else {
                return Err("not a stream".into());
            };
            Command::MoveStream {
                stream,
                device: device(&state, arg(2)?)?,
            }
        }
        "create-virtual" => Command::CreateVirtualSink {
            name: arg(1)?.into(),
        },
        "remove-virtual" => Command::RemoveVirtualSink {
            device: device(&state, arg(1)?)?,
        },
        "rate-quantum" => {
            let opt = |v: f32| (v > 0.0).then_some(v as u32);
            Command::SetRateQuantum {
                rate: opt(num(1)?),
                quantum: opt(num(2)?),
            }
        }
        "link" => Command::Link {
            output_port: num(1)? as Id,
            input_port: num(2)? as Id,
        },
        "unlink" => Command::Unlink {
            link: num(1)? as Id,
        },
        _ => return Err(USAGE.into()),
    };
    audio.send(command);
    let state = settle(&rx, Duration::from_millis(500)).unwrap_or(state);
    println!(
        "{}",
        serde_json::to_string_pretty(&state_json(&state)).unwrap_or_default()
    );
    if state.issues.is_empty() {
        Ok(())
    } else {
        Err(state
            .issues
            .iter()
            .map(|i| i.message.clone())
            .collect::<Vec<_>>()
            .join("; "))
    }
}

fn wait_connected(audio: &Audio, rx: &Receiver<Arc<State>>) -> Result<Arc<State>, String> {
    let end = Instant::now() + Duration::from_secs(5);
    let mut state = audio.state();
    while state.connection != Connection::Connected {
        let left = end
            .checked_duration_since(Instant::now())
            .ok_or_else(|| format!("not connected: {:?}", state.connection))?;
        state = rx
            .recv_timeout(left)
            .map_err(|_| format!("not connected: {:?}", state.connection))?;
    }
    Ok(settle(rx, Duration::from_millis(200)).unwrap_or(state))
}

/// The last snapshot once none has arrived for `quiet` (at most 3 s).
fn settle(rx: &Receiver<Arc<State>>, quiet: Duration) -> Option<Arc<State>> {
    let end = Instant::now() + Duration::from_secs(3);
    let mut last = None;
    while Instant::now() < end {
        match rx.recv_timeout(quiet) {
            Ok(s) => last = Some(s),
            Err(_) => break,
        }
    }
    last
}

fn level(s: &State, t: Target) -> Option<f32> {
    match t {
        Target::Device(d) => s.device(d)?.level,
        Target::Stream(id) => s.stream(id)?.level,
    }
}

fn target(s: &State, a: &str) -> Result<Target, String> {
    if let Ok(id) = a.parse::<Id>() {
        if s.device(id).is_some() {
            return Ok(Target::Device(id));
        }
        if s.stream(id).is_some() {
            return Ok(Target::Stream(id));
        }
        return Err(format!("no device or stream {id}"));
    }
    if let Some(d) = s.devices.iter().find(|d| d.name == a || d.description == a) {
        return Ok(Target::Device(d.id));
    }
    if let Some(st) = s
        .streams
        .iter()
        .find(|st| st.app_name == a || st.media_name == a)
    {
        return Ok(Target::Stream(st.id));
    }
    Err(format!("no device or stream {a:?}"))
}

fn device(s: &State, a: &str) -> Result<Id, String> {
    match target(s, a)? {
        Target::Device(d) => Ok(d),
        Target::Stream(_) => Err(format!("{a} is a stream, not a device")),
    }
}

fn card(s: &State, a: &str) -> Result<Id, String> {
    if let Ok(id) = a.parse::<Id>()
        && s.card(id).is_some()
    {
        return Ok(id);
    }
    s.cards
        .iter()
        .find(|c| c.name == a || c.description == a)
        .map(|c| c.id)
        .ok_or_else(|| format!("no card {a:?}"))
}

fn summary(s: &State) -> String {
    let dev: Vec<String> = s
        .devices
        .iter()
        .map(|d| {
            format!(
                "{}:{:.2}{}",
                d.id,
                d.volume.combined(),
                if d.mute { "M" } else { "" }
            )
        })
        .collect();
    let st: Vec<String> = s
        .streams
        .iter()
        .map(|x| {
            format!(
                "{}:{:.2}{}→{:?}",
                x.id,
                x.volume.combined(),
                if x.mute { "M" } else { "" },
                x.target_device
            )
        })
        .collect();
    format!(
        "rev {} {:?} sink={:?} source={:?} devices[{}] streams[{}] issues={}",
        s.revision,
        s.connection,
        s.default_sink,
        s.default_source,
        dev.join(" "),
        st.join(" "),
        s.issues.len()
    )
}

fn round(v: f32) -> f64 {
    (v as f64 * 1000.0).round() / 1000.0
}

fn volume_json(v: &Volume) -> Value {
    json!({
        "combined": round(v.combined()),
        "balance": round(v.balance()),
        "channels": v.channels.iter().map(|c| round(*c)).collect::<Vec<_>>(),
        "positions": v.positions.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>(),
    })
}

pub fn state_json(s: &State) -> Value {
    json!({
        "revision": s.revision,
        "connection": format!("{:?}", s.connection),
        "default_sink": s.default_sink,
        "default_source": s.default_source,
        "devices": s.devices.iter().map(|d| json!({
            "id": d.id, "name": d.name, "description": d.description, "icon": d.icon_name,
            "kind": format!("{:?}", d.kind), "volume": volume_json(&d.volume), "mute": d.mute,
            "ports": d.ports.iter().map(|p| json!({"name": p.name, "description": p.description, "available": format!("{:?}", p.available), "priority": p.priority})).collect::<Vec<_>>(),
            "active_port": d.active_port, "card": d.card_id, "bus": format!("{:?}", d.bus),
            "virtual": d.is_virtual, "ours": d.is_ours, "hidden": d.hidden, "level": d.level.map(round),
        })).collect::<Vec<_>>(),
        "cards": s.cards.iter().map(|c| json!({
            "id": c.id, "name": c.name, "description": c.description, "icon": c.icon_name, "bus": format!("{:?}", c.bus),
            "active_profile": c.active_profile, "battery": c.battery,
            "profiles": c.profiles.iter().map(|p| json!({"name": p.name, "description": p.description, "available": p.available, "priority": p.priority, "kind": format!("{:?}", p.kind), "codec": p.codec})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "streams": s.streams.iter().map(|x| json!({
            "id": x.id, "app": x.app_name, "icon": x.icon_name, "media": x.media_name, "binary": x.binary,
            "kind": format!("{:?}", x.kind), "volume": volume_json(&x.volume), "mute": x.mute,
            "target": x.target_device, "corked": x.corked, "level": x.level.map(round),
        })).collect::<Vec<_>>(),
        "server": {
            "name": s.server.name, "version": s.server.version, "rate": s.server.rate, "allowed_rates": s.server.allowed_rates,
            "quantum": s.server.quantum, "min_quantum": s.server.min_quantum, "max_quantum": s.server.max_quantum,
            "forced_rate": s.server.forced_rate, "forced_quantum": s.server.forced_quantum,
        },
        "graph": { "nodes": s.graph.nodes.len(), "ports": s.graph.ports.len(), "links": s.graph.links.len() },
        "issues": s.issues.iter().map(|i| format!("{:?}: {}", i.severity, i.message)).collect::<Vec<_>>(),
    })
}
