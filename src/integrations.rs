//! What the modules tell the notifications: a volume card when the default output's volume or
//! mute changes from outside Hokum (keys, wpctl, an app), a brightness card when the screen's
//! backlight changes, and Bluetooth cards when a paired device connects or its battery runs
//! low. Each watcher sees every snapshot and decides; the service shows what they return on the
//! cards overlay (`crate::modules`).

use std::collections::HashSet;
use std::time::Duration;

use crate::card::{Note, Timeout};
use crate::i18n;
use crate::icons::Source;

/// The volume card's id: every change lands in the same card.
pub const OSD_ID: u32 = u32::MAX / 2 + 2;
/// The brightness card's id: every change lands in the same card.
pub const BRIGHTNESS_ID: u32 = u32::MAX / 2 + 3;
/// Bluetooth cards take ids from here, two per device.
const DEVICE_IDS: u32 = u32::MAX / 2 + 16;
const LOW: u8 = 20;
const RECHARGED: u8 = 30;

fn icon(name: &str) -> Option<Source> {
    crate::icons::icon(name).map(Source::File)
}

/// The volume card for `device` at `volume` (cubic, up to 1.5).
pub fn osd_note(device: &str, volume: f32, mute: bool) -> Note {
    let mut n = Note::new(
        OSD_ID,
        i18n::t("Volume"),
        device,
        if mute { i18n::t("Muted") } else { "" },
    );
    n.value = Some(if mute { 0.0 } else { volume.clamp(0.0, 1.0) });
    n.tag = Some("volume".into());
    n.hud = true;
    n.timeout = Timeout::After(Duration::from_millis(1600));
    let name = match (mute, volume) {
        (true, _) => "audio-volume-muted",
        (_, v) if v <= 0.0 => "audio-volume-muted",
        (_, v) if v < 0.34 => "audio-volume-low",
        (_, v) if v < 0.67 => "audio-volume-medium",
        _ => "audio-volume-high",
    };
    n.icon_src = icon(name).or_else(|| icon("audio-volume-high"));
    n
}

/// Watches the default output for volume and mute changes.
#[derive(Default)]
pub struct Osd {
    /// (device, volume, mute) last seen; None until connected.
    last: Option<(u32, f32, bool)>,
}

impl Osd {
    /// The card for this snapshot, if the default output's volume or mute changed. The first
    /// snapshot, a switch to another device and changes Hokum made itself (`local`) only move
    /// the baseline.
    pub fn see(&mut self, s: &crate::sound::model::State, local: bool) -> Option<Note> {
        let sink = s
            .default_sink()
            .filter(|_| s.connection == crate::sound::model::Connection::Connected);
        let Some(d) = sink else {
            self.last = None;
            return None;
        };
        let now = (d.id, d.volume.combined(), d.mute);
        let before = self.last.replace(now)?;
        if before.0 != now.0 || ((before.1 - now.1).abs() < 0.004 && before.2 == now.2) || local {
            return None;
        }
        Some(osd_note(&d.description, now.1, now.2))
    }
}

/// The brightness card at `level` (0 to 1).
pub fn brightness_note(level: f32) -> Note {
    let level = level.clamp(0.0, 1.0);
    let mut n = Note::new(
        BRIGHTNESS_ID,
        i18n::t("Brightness"),
        format!("{}%", (level * 100.0).round()),
        "",
    );
    n.value = Some(level);
    n.tag = Some("brightness".into());
    n.hud = true;
    n.timeout = Timeout::After(Duration::from_millis(1600));
    n.icon_src = [
        "display-brightness",
        "display-brightness-symbolic",
        "video-display",
    ]
    .into_iter()
    .find_map(icon);
    n
}

/// Watches the screen's backlight for changes.
#[derive(Default)]
pub struct Brightness {
    /// The level last seen; None until the first reading.
    last: Option<f32>,
}

impl Brightness {
    /// The card for this reading, if the level changed. The first reading only sets the
    /// baseline, so Hokum starting doesn't flash a card.
    pub fn see(&mut self, level: f32) -> Option<Note> {
        let before = self.last.replace(level)?;
        ((before - level).abs() >= 0.004).then(|| brightness_note(level))
    }
}

fn device_slot(id: &crate::bluetooth::model::DeviceId) -> u32 {
    let h =
        id.0.bytes()
            .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    DEVICE_IDS + (h % 4096) * 2
}

fn device_icon(d: &crate::bluetooth::model::Device) -> Option<Source> {
    Some(d.icon.as_str())
        .filter(|i| !i.is_empty())
        .and_then(icon)
        .or_else(|| icon("bluetooth"))
}

pub fn connected_note(d: &crate::bluetooth::model::Device) -> Note {
    let summary = i18n::connected(d.display_name(), d.battery);
    let mut n = Note::new(device_slot(&d.id), i18n::t("Bluetooth"), summary, "");
    n.timeout = Timeout::After(Duration::from_secs(4));
    n.icon_src = device_icon(d);
    n
}

pub fn low_battery_note(d: &crate::bluetooth::model::Device, level: u8) -> Note {
    let mut n = Note::new(
        device_slot(&d.id) + 1,
        i18n::t("Bluetooth"),
        i18n::battery_low(d.display_name()),
        i18n::left(level),
    );
    n.value = Some(level as f32 / 100.0);
    n.icon_src = device_icon(d);
    n
}

/// Watches Bluetooth devices connecting and their batteries.
#[derive(Default)]
pub struct Devices {
    /// Paired devices connected at the last snapshot; None until BlueZ is ready.
    connected: Option<HashSet<crate::bluetooth::model::DeviceId>>,
    /// Devices already warned about since their battery was last above `RECHARGED`.
    warned: HashSet<crate::bluetooth::model::DeviceId>,
}

impl Devices {
    /// Cards for this snapshot: paired devices that connected since the last one (not those
    /// already connected when Hokum or bluetoothd started), and batteries that fell below 20 %
    /// (once until they're back above 30 %).
    pub fn see(&mut self, s: &crate::bluetooth::model::State) -> Vec<Note> {
        if s.connection != crate::bluetooth::model::Connection::Ready {
            self.connected = None;
            return Vec::new();
        }
        let mut out = Vec::new();
        let now: HashSet<_> = s
            .connected()
            .filter(|d| d.is_paired())
            .map(|d| d.id.clone())
            .collect();
        if let Some(before) = &self.connected {
            out.extend(
                s.connected()
                    .filter(|d| now.contains(&d.id) && !before.contains(&d.id))
                    .map(connected_note),
            );
        }
        for d in s.connected() {
            match d.battery {
                Some(b) if b < LOW && self.warned.insert(d.id.clone()) => {
                    out.push(low_battery_note(d, b))
                }
                Some(b) if b > RECHARGED => {
                    self.warned.remove(&d.id);
                }
                _ => {}
            }
        }
        self.connected = Some(now);
        out
    }
}

/// Cards for the integrations' scenes, as the service would show them.
pub fn samples() -> Vec<Note> {
    let bt = crate::bluetooth::mock::sample();
    let mut out = vec![osd_note("Speakers", 0.62, false)];
    if let Some(d) = bt.connected().find(|d| d.audio) {
        out.push(connected_note(d));
    }
    if let Some(d) = bt.connected().find(|d| d.battery.is_some() && !d.audio) {
        out.push(low_battery_note(d, 14));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::model::Command;

    fn change(s: &mut crate::sound::model::State, c: Command) {
        crate::sound::mock::apply(s, c, &mut HashSet::new());
    }

    #[test]
    fn the_volume_card_follows_outside_changes_only() {
        let mut osd = Osd::default();
        let mut s = crate::sound::mock::sample();
        assert!(osd.see(&s, false).is_none(), "not at startup");
        let sink = s.default_sink.unwrap();
        let t = crate::sound::model::Target::Device(sink);
        change(
            &mut s,
            Command::SetVolume {
                target: t,
                volume: 0.3,
            },
        );
        let n = osd.see(&s, false).expect("a card");
        assert_eq!((n.id, n.app.as_str()), (OSD_ID, crate::i18n::t("Volume")));
        assert!((n.value.unwrap() - 0.3).abs() < 0.01);
        assert!(osd.see(&s, false).is_none(), "nothing changed");
        change(
            &mut s,
            Command::SetMute {
                target: t,
                mute: true,
            },
        );
        assert_eq!(osd.see(&s, false).unwrap().body, crate::i18n::t("Muted"));
        change(
            &mut s,
            Command::SetVolume {
                target: t,
                volume: 0.5,
            },
        );
        assert!(osd.see(&s, true).is_none(), "Hokum's own change");
        let other = s.sinks().find(|d| d.id != sink).unwrap().id;
        change(&mut s, Command::SetDefault { device: other });
        assert!(
            osd.see(&s, false).is_none(),
            "a device switch isn't a volume change"
        );
    }

    #[test]
    fn the_brightness_card_follows_changes_only() {
        let mut b = Brightness::default();
        assert!(b.see(0.5).is_none(), "not at startup");
        assert!(b.see(0.5).is_none(), "nothing changed");
        assert!(b.see(0.501).is_none(), "less than a step");
        let n = b.see(0.55).expect("a card");
        assert_eq!(
            (n.id, n.app.as_str(), n.summary.as_str()),
            (BRIGHTNESS_ID, crate::i18n::t("Brightness"), "55%")
        );
        assert!((n.value.unwrap() - 0.55).abs() < 0.001 && n.hud);
        assert_eq!(b.see(0.0).unwrap().summary, "0%");
        assert_eq!(brightness_note(1.4).summary, "100%", "never past full");
    }

    #[test]
    fn devices_say_when_they_connect_and_run_low() {
        let mut w = Devices::default();
        let mut s = crate::bluetooth::mock::sample();
        assert!(w.see(&s).is_empty(), "not for devices connected at startup");
        let i = s
            .devices
            .iter()
            .position(|d| d.is_paired() && !d.connected)
            .unwrap();
        s.devices[i].connected = true;
        s.devices[i].battery = Some(82);
        let notes = w.see(&s);
        assert_eq!(notes.len(), 1);
        assert_eq!(
            notes[0].summary,
            crate::i18n::connected(s.devices[i].display_name(), Some(82)),
        );
        s.devices[i].battery = Some(15);
        assert_eq!(w.see(&s)[0].body, crate::i18n::left(15));
        s.devices[i].battery = Some(12);
        assert!(w.see(&s).is_empty(), "once per discharge");
        s.devices[i].battery = Some(40);
        assert!(w.see(&s).is_empty());
        s.devices[i].battery = Some(10);
        assert_eq!(w.see(&s).len(), 1, "again after a recharge");
    }
}
