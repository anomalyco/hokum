//! The modules beside notifications, in the service: starts and stops Sound's and Bluetooth's
//! backends as config.kdl turns them on and off (`modules { sound #true; bluetooth #true }`),
//! hands their snapshots to the window and the tray panel, and keeps the one tray icon.
//!
//! Each backend runs on its own threads. One that can't start, or that panics later, leaves a
//! quiet line in its section (and nothing in the panel); notifications never notice.

use std::sync::atomic::{AtomicBool, Ordering};
// Volume and Bluetooth cards come from `crate::integrations`.
use std::sync::{Arc, Mutex};

use rune::{AppHandle, OverlayHandle, Tray, TrayEvent, TrayIcon, TrayItem, TraySpec};

use crate::settings::{Section, Ui};
use crate::watch::Config;

#[derive(Default)]
struct Tip {
    sound: String,
    bluetooth: String,
    shown: String,
}

pub struct Wiring {
    window: AppHandle<Ui>,
    panel: OverlayHandle<Ui>,
    cards: OverlayHandle<crate::cards::Ui>,
    osd: Mutex<crate::integrations::Osd>,
    devices: Mutex<crate::integrations::Devices>,
    /// `modules { osd; bluetooth-notifications }`.
    show_osd: AtomicBool,
    show_devices: AtomicBool,
    /// Whether the panel is showing.
    shown: Arc<AtomicBool>,
    tray: Mutex<Option<(Arc<Tray>, bool, bool)>>,
    tip: Arc<Mutex<Tip>>,
}

/// Shows the window, on `section` when given.
pub fn open(window: &AppHandle<Ui>, section: Option<Section>, token: Option<String>) {
    window.update(move |w, _| {
        if let Some(s) = section {
            w.section = s;
        }
        w.shown();
    });
    match token.filter(|t| !t.is_empty()) {
        Some(t) => window.activate(t),
        None => {
            window.open();
            window.focus();
        }
    }
}

impl Wiring {
    pub fn new(
        window: AppHandle<Ui>,
        panel: OverlayHandle<Ui>,
        cards: OverlayHandle<crate::cards::Ui>,
    ) -> Arc<Wiring> {
        let w = Arc::new(Wiring {
            window,
            panel,
            cards,
            osd: Mutex::default(),
            devices: Mutex::default(),
            show_osd: AtomicBool::new(true),
            show_devices: AtomicBool::new(true),
            shown: Arc::new(AtomicBool::new(false)),
            tray: Mutex::new(None),
            tip: Arc::new(Mutex::new(Tip::default())),
        });
        let hide = w.hider();
        w.panel.update(move |p, _| p.dismiss = Some(hide));
        w
    }

    fn hider(&self) -> Arc<dyn Fn() + Send + Sync> {
        let (panel, shown) = (self.panel.clone(), self.shown.clone());
        Arc::new(move || {
            shown.store(false, Ordering::SeqCst);
            panel.hide();
            panel.update(|p, _| p.hidden());
        })
    }

    /// The tray's click: the panel, or away with it.
    fn toggle_panel(&self) {
        if self.shown.swap(true, Ordering::SeqCst) {
            self.hider()();
            return;
        }
        self.panel.update(|p, _| {
            p.shown();
            p.sound.open.clear();
            p.sound.page = crate::sound::ui::Page::Main;
            p.bluetooth.open = None;
            p.bluetooth.page = crate::bluetooth::ui::Page::Main;
        });
        self.panel.show();
        self.panel.request_keys();
    }

    /// A new config: modules on or off, the tray's menu, and the panel's copy of it.
    pub fn configure(self: &Arc<Self>, ui: &mut Ui, config: &Config) {
        let m = config.loaded.modules;
        self.show_osd.store(m.osd, Ordering::Relaxed);
        self.show_devices
            .store(m.bluetooth_notifications, Ordering::Relaxed);
        let sound_on = ui.sound.handle.is_some() || ui.sound_error.is_some();
        if m.sound && !sound_on {
            self.start_sound(ui);
        } else if !m.sound && sound_on {
            ui.sound_error = None;
            ui.sound.attach(None, None, None);
            self.panel.update(|p, _| p.sound.attach(None, None, None));
            self.tip
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sound
                .clear();
        }
        let bt_on = ui.bluetooth.handle.is_some() || ui.bluetooth_error.is_some();
        if m.bluetooth && !bt_on {
            self.start_bluetooth(ui);
        } else if !m.bluetooth && bt_on {
            ui.bluetooth_error = None;
            ui.bluetooth.attach(None, None);
            self.panel.update(|p, _| p.bluetooth.attach(None, None));
            self.tip
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .bluetooth
                .clear();
        }
        self.sync_tray(m, config.loaded.behaviour.do_not_disturb);
        let c = config.clone();
        self.panel.update(move |p, _| p.configure(c));
    }

    fn start_sound(self: &Arc<Self>, ui: &mut Ui) {
        let mock = std::env::var("HOKUM_SOUND_BACKEND").is_ok_and(|b| b == "mock");
        let source = if mock {
            crate::sound::model::Source::Mock
        } else {
            crate::sound::model::Source::PipeWire
        };
        let audio = match crate::sound::model::start(source) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("hokum: sound: {e}");
                ui.sound_error = Some(format!("Sound couldn't start: {e}"));
                return;
            }
        };
        let meters = crate::sound::ui::Meters::new(audio.clone());
        let sub = {
            let (window, panel, me) = (self.window.clone(), self.panel.clone(), self.clone());
            audio.on_change(move |s| {
                me.set_tip(|t| t.sound = sound_tip(s));
                let card = me
                    .osd
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .see(s, crate::sound::ui::touched_recently());
                if let Some(n) = card.filter(|_| me.show_osd.load(Ordering::Relaxed)) {
                    me.cards.update(move |c, _| c.upsert(n));
                }
                let (a, b) = (s.clone(), s.clone());
                window.update(move |w, _| w.sound.receive(a));
                panel.update(move |p, _| p.sound.receive(b));
            })
        };
        let (a, m) = (audio.clone(), meters.clone());
        self.panel
            .update(move |p, _| p.sound.attach(Some(a), Some(m), None));
        ui.sound.attach(Some(audio), Some(meters), Some(sub));
    }

    fn start_bluetooth(self: &Arc<Self>, ui: &mut Ui) {
        let mock = std::env::var("HOKUM_BLUETOOTH_BACKEND").is_ok_and(|b| b == "mock");
        let source = if mock {
            crate::bluetooth::model::Source::Mock
        } else {
            crate::bluetooth::model::Source::Bluez
        };
        let bt = match crate::bluetooth::model::start(source) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("hokum: bluetooth: {e}");
                ui.bluetooth_error = Some(format!("Bluetooth couldn't start: {e}"));
                return;
            }
        };
        let sub = {
            let (window, panel, me) = (self.window.clone(), self.panel.clone(), self.clone());
            bt.on_change(move |s| {
                me.set_tip(|t| t.bluetooth = bluetooth_tip(s));
                let cards = me.devices.lock().unwrap_or_else(|e| e.into_inner()).see(s);
                if me.show_devices.load(Ordering::Relaxed) {
                    for n in cards {
                        me.cards.update(move |c, _| c.upsert(n));
                    }
                }
                let (a, b) = (s.clone(), s.clone());
                window.update(move |w, _| w.bluetooth.receive(a));
                panel.update(move |p, _| p.bluetooth.receive(b));
            })
        };
        let b = bt.clone();
        self.panel
            .update(move |p, _| p.bluetooth.attach(Some(b), None));
        ui.bluetooth.attach(Some(bt), Some(sub));
    }

    fn set_tip(&self, f: impl FnOnce(&mut Tip)) {
        let mut t = self.tip.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut t);
        let parts: Vec<&str> = [t.sound.as_str(), t.bluetooth.as_str()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        let tip = if parts.is_empty() {
            "Hokum".to_owned()
        } else {
            parts.join(" · ")
        };
        if tip != t.shown {
            if let Some((tray, ..)) = &*self.tray.lock().unwrap_or_else(|e| e.into_inner()) {
                tray.set_tooltip(tip.clone());
            }
            t.shown = tip;
        }
    }

    /// One tray icon while any module is on: a speaker (or Bluetooth's rune with Sound off).
    fn sync_tray(self: &Arc<Self>, m: crate::theme::Modules, dnd: bool) {
        // Tip before tray, the order `set_tip` takes them in.
        let tooltip = self
            .tip
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shown
            .clone();
        let mut slot = self.tray.lock().unwrap_or_else(|e| e.into_inner());
        if !(m.sound || m.bluetooth) {
            *slot = None;
            return;
        }
        if let Some((tray, speaker, was_dnd)) = &mut *slot
            && *speaker == m.sound
        {
            if *was_dnd != dnd {
                tray.set_menu(menu(dnd));
                *was_dnd = dnd;
            }
            return;
        }
        let spec = TraySpec::new(
            "hokum",
            "Hokum",
            TrayIcon::rgba(32, 32, if m.sound { speaker(32) } else { rune_mark(32) }),
        )
        .tooltip(if tooltip.is_empty() {
            "Hokum".into()
        } else {
            tooltip
        })
        .menu(menu(dnd));
        let me = Arc::downgrade(self);
        let window = self.window.clone();
        match Tray::new(spec, move |e| match e {
            TrayEvent::Activate { .. } => {
                if let Some(me) = me.upgrade() {
                    me.toggle_panel();
                }
            }
            TrayEvent::Menu(id) if id == "open" => open(&window, None, None),
            TrayEvent::Menu(id) if id == "dnd" => window.update(|w, _| w.toggle_dnd()),
            TrayEvent::Menu(id) if id == "quit" => window.quit(),
            _ => {}
        }) {
            Ok(t) => *slot = Some((Arc::new(t), m.sound, dnd)),
            Err(e) => {
                eprintln!("hokum: no tray icon: {e}");
                *slot = None;
            }
        }
    }
}

fn menu(dnd: bool) -> Vec<TrayItem> {
    vec![
        TrayItem::action("open", "Open Hokum"),
        TrayItem::action(
            "dnd",
            if dnd {
                "Turn off Do not disturb"
            } else {
                "Turn on Do not disturb"
            },
        ),
        TrayItem::Separator,
        TrayItem::action("quit", "Quit"),
    ]
}

fn sound_tip(s: &crate::sound::model::State) -> String {
    match s.default_sink() {
        Some(d) if d.mute => format!("{} muted", d.description),
        Some(d) => format!(
            "{} {}%",
            d.description,
            (d.volume.combined() * 100.0).round()
        ),
        None => String::new(),
    }
}

fn bluetooth_tip(s: &crate::bluetooth::model::State) -> String {
    if !s.default_adapter().is_some_and(|a| a.powered) {
        return String::new();
    }
    s.connected()
        .map(|d| match d.battery {
            Some(b) => format!("{} {b}%", d.display_name()),
            None => d.display_name().to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// RGBA, white on transparent, 4×4 supersampled.
fn raster(size: u32, alpha: u32, inside: impl Fn(f32, f32) -> bool) -> Vec<u8> {
    let s = size as f32 / 16.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    if inside(
                        (px as f32 + (sx as f32 + 0.5) / 4.0) / s,
                        (py as f32 + (sy as f32 + 0.5) / 4.0) / s,
                    ) {
                        hits += 1;
                    }
                }
            }
            rgba.extend_from_slice(&[255, 255, 255, (hits * alpha / 16) as u8]);
        }
    }
    rgba
}

/// A speaker with two waves, on a 16-point grid.
fn speaker(size: u32) -> Vec<u8> {
    let body = [
        [2.5, 6.0],
        [4.7, 6.0],
        [8.0, 3.2],
        [8.0, 12.8],
        [4.7, 10.0],
        [2.5, 10.0],
    ];
    raster(size, 255, |x, y| {
        let mut odd = false;
        let mut j = body.len() - 1;
        for i in 0..body.len() {
            let ([xi, yi], [xj, yj]) = (body[i], body[j]);
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                odd = !odd;
            }
            j = i;
        }
        if odd {
            return true;
        }
        let (dx, dy) = (x - 8.6, y - 8.0);
        let r = (dx * dx + dy * dy).sqrt();
        dx > 0.0 && dy.atan2(dx).abs() < 0.95 && ((r - 2.9).abs() < 0.65 || (r - 5.4).abs() < 0.65)
    })
}

/// Bluetooth's rune, on a 16-point grid.
fn rune_mark(size: u32) -> Vec<u8> {
    let pts = [
        [4.5, 5.0],
        [11.5, 11.0],
        [8.0, 14.0],
        [8.0, 2.0],
        [11.5, 5.0],
        [4.5, 11.0],
    ];
    raster(size, 255, |x, y| {
        pts.windows(2).any(|w| {
            let ([ax, ay], [bx, by]) = (w[0], w[1]);
            let (dx, dy) = (bx - ax, by - ay);
            let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            let (px, py) = (ax + t * dx - x, ay + t * dy - y);
            px * px + py * py < 0.85 * 0.85
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icons_are_drawn() {
        for icon in [speaker(32), rune_mark(32)] {
            assert_eq!(icon.len(), 32 * 32 * 4);
            assert!(icon.chunks(4).any(|p| p[3] == 255) && icon.chunks(4).any(|p| p[3] == 0));
        }
    }

    #[test]
    fn the_tooltip_says_volume_and_devices() {
        assert_eq!(sound_tip(&crate::sound::mock::sample()).is_empty(), false);
        let bt = crate::bluetooth::mock::sample();
        let tip = bluetooth_tip(&bt);
        assert!(
            bt.connected().all(|d| tip.contains(d.display_name())),
            "{tip}"
        );
    }
}
