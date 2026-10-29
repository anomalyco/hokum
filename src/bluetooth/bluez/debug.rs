//! `hokum bt debug ...` (hidden): exercise the BlueZ backend from a terminal.
//!
//! ```sh
//! hokum bt debug dump                          # the State as JSON
//! hokum bt debug scan [seconds]                # discover for a while, print what was heard, stop
//! hokum bt debug agent [seconds]               # register the pairing agent (as the service does), wait, unregister
//! hokum bt debug set discoverable|pairable on|off
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};

use super::{Role, spawn};
use crate::bluetooth::model::*;

fn kind_name<T: std::fmt::Debug>(t: T) -> String {
    format!("{t:?}")
}

fn props(p: &[(String, String)]) -> Json {
    Json::Object(
        p.iter()
            .map(|(k, v)| (k.clone(), Json::String(v.clone())))
            .collect(),
    )
}

fn services(s: &[Service]) -> Json {
    s.iter()
        .map(|s| json!({ "uuid": s.uuid, "name": s.name, "audio": s.audio }))
        .collect()
}

pub fn to_json(s: &State) -> Json {
    json!({
        "revision": s.revision,
        "connection": kind_name(&s.connection),
        "default_adapter": s.default_adapter.as_ref().map(|a| a.0.clone()),
        "adapters": s.adapters.iter().map(|a| json!({
            "id": a.id.0, "name": a.name, "alias": a.alias, "address": a.address,
            "address_type": kind_name(a.address_type), "powered": a.powered, "power_state": kind_name(a.power_state),
            "discoverable": a.discoverable, "discoverable_timeout": a.discoverable_timeout,
            "pairable": a.pairable, "pairable_timeout": a.pairable_timeout, "discovering": a.discovering,
            "class": format!("0x{:06x}", a.class), "roles": a.roles, "modalias": a.modalias,
            "services": services(&a.services), "properties": props(&a.properties),
        })).collect::<Vec<_>>(),
        "devices": s.devices.iter().map(|d| json!({
            "id": d.id.0, "adapter": d.adapter.0, "address": d.address, "address_type": kind_name(d.address_type),
            "bearer": kind_name(d.bearer), "name": d.name, "alias": d.alias, "icon": d.icon, "kind": kind_name(d.kind),
            "class": d.class.map(|c| format!("0x{c:06x}")), "appearance": d.appearance.map(|a| format!("0x{a:04x}")),
            "paired": d.paired, "bonded": d.bonded, "trusted": d.trusted, "blocked": d.blocked,
            "connected": d.connected, "services_resolved": d.services_resolved, "legacy_pairing": d.legacy_pairing,
            "battery": d.battery, "rssi": d.rssi, "tx_power": d.tx_power, "audio": d.audio,
            "pending": d.pending.map(kind_name), "services": services(&d.services),
            "raw": {
                "modalias": d.raw.modalias,
                "manufacturer_data": d.raw.manufacturer_data.iter().map(|m| json!({
                    "company": format!("0x{:04x}", m.company), "company_name": m.company_name, "data": super::map::hex(&m.data),
                })).collect::<Vec<_>>(),
                "service_data": d.raw.service_data.iter().map(|x| json!({ "service": x.service.name, "data": super::map::hex(&x.data) })).collect::<Vec<_>>(),
                "advertising_flags": super::map::hex(&d.raw.advertising_flags),
                "properties": props(&d.raw.properties),
            },
        })).collect::<Vec<_>>(),
        "pairing": s.pairing.as_ref().map(|p| json!({ "id": p.id, "device": p.device.0, "kind": kind_name(&p.kind) })),
        "issues": s.issues.iter().map(|i| json!({ "id": i.id, "message": i.message, "device": i.device.as_ref().map(|d| d.0.clone()) })).collect::<Vec<_>>(),
    })
}

/// Waits until `done` holds for a snapshot or `timeout` passes; returns the last snapshot.
fn wait(
    rx: &std::sync::mpsc::Receiver<Arc<State>>,
    bt: &Bluetooth,
    timeout: Duration,
    done: impl Fn(&State) -> bool,
) -> Arc<State> {
    let end = Instant::now() + timeout;
    let mut last = bt.state();
    while !done(&last) {
        match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(s) => last = s,
            Err(_) => break,
        }
    }
    last
}

fn ready(s: &State) -> bool {
    s.connection != Connection::Connecting
}

pub fn cli(argv: &[String]) -> i32 {
    let args: Vec<&str> = argv.iter().skip(2).map(String::as_str).collect();
    let sub = argv.get(1).map(String::as_str).unwrap_or("");
    let role = if sub == "agent" {
        Role::Service
    } else {
        Role::Inspect
    };
    let (bt, handle) = match spawn(role) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("hokum: {e}");
            return 1;
        }
    };
    let (rx, sub_guard) = bt.subscribe();
    let state = wait(&rx, &bt, Duration::from_secs(5), ready);
    let secs = |i: usize, default: u64| {
        args.get(i)
            .and_then(|s| s.parse().ok())
            .map_or(Duration::from_secs(default), Duration::from_secs)
    };
    let code = match sub {
        "dump" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&to_json(&state)).unwrap_or_default()
            );
            0
        }
        "scan" => match state.default_adapter.clone() {
            None => {
                eprintln!("no adapter");
                1
            }
            Some(adapter) => {
                bt.send(Command::StartDiscovery {
                    adapter: adapter.clone(),
                });
                let s = wait(&rx, &bt, Duration::from_secs(3), |s| {
                    s.default_adapter().is_some_and(|a| a.discovering)
                });
                println!(
                    "discovering: {}",
                    s.default_adapter().is_some_and(|a| a.discovering)
                );
                let end = Instant::now() + secs(0, 5);
                let s = wait(
                    &rx,
                    &bt,
                    end.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                    |_| Instant::now() >= end,
                );
                for d in s.nearby().filter(|d| d.rssi.is_some()) {
                    println!(
                        "{:>5} dBm  {:<17}  {:?}/{:?}  {}",
                        d.rssi.unwrap_or(0),
                        d.address,
                        d.kind,
                        d.bearer,
                        d.display_name()
                    );
                    if args.contains(&"-v") {
                        println!("           {:?}", d.raw.properties);
                    }
                }
                if let Some(i) = s.issues.first() {
                    println!("issue: {}", i.message);
                }
                bt.send(Command::StopDiscovery { adapter });
                let s = wait(&rx, &bt, Duration::from_secs(5), |s| {
                    s.default_adapter().is_some_and(|a| !a.discovering)
                });
                println!(
                    "discovering after stop: {}",
                    s.default_adapter().is_some_and(|a| a.discovering)
                );
                0
            }
        },
        "agent" => {
            let end = Instant::now() + secs(0, 3);
            let s = wait(
                &rx,
                &bt,
                end.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                |_| Instant::now() >= end,
            );
            println!("connection: {:?}; pairing: {:?}", s.connection, s.pairing);
            0
        }
        "set" => {
            let (Some(prop), Some(value), Some(adapter)) =
                (args.first(), args.get(1), state.default_adapter.clone())
            else {
                eprintln!("usage: hokum bt debug set discoverable|pairable on|off");
                return 2;
            };
            let on = *value == "on";
            let (command, check): (Command, fn(&Adapter) -> bool) = match *prop {
                "discoverable" => (Command::SetDiscoverable { adapter, on }, |a| a.discoverable),
                "pairable" => (Command::SetPairable { adapter, on }, |a| a.pairable),
                _ => {
                    eprintln!("usage: hokum bt debug set discoverable|pairable on|off");
                    return 2;
                }
            };
            bt.send(command);
            let s = wait(&rx, &bt, Duration::from_secs(5), |s| {
                s.default_adapter().is_some_and(|a| check(a) == on) || !s.issues.is_empty()
            });
            println!("{prop}: {}", s.default_adapter().is_some_and(check));
            for i in &s.issues {
                println!("issue: {}", i.message);
            }
            0
        }
        _ => {
            eprintln!(
                "usage: hokum bt debug dump | scan [seconds] | agent [seconds] | set discoverable|pairable on|off"
            );
            2
        }
    };
    drop(sub_guard);
    drop(rx);
    drop(bt);
    let _ = handle.join();
    code
}
