//! Bluetooth's section of the Hokum window and its rows in the tray panel, drawn with Rune in
//! Hokum's look. Both read [`State`] snapshots from a [`Bluetooth`] handle and send
//! [`Command`]s; neither touches D-Bus.
//!
//! Progressive disclosure:
//! 1. `view::main`: the power switch, connected devices (a click disconnects), paired devices
//!    (a click connects). That's all.
//! 2. A row opens in place: rename, trust, forget, Sound for audio devices. "Add device" scans
//!    in place, and pairing codes show under the device being paired.
//! 3. `advanced`: adapters and their settings, every device's LE details and raw properties.
//!
//! Handlers take the window's state ([`Host`]) and reach this section's through [`lift`] or
//! `h.bluetooth`.

mod advanced;
pub mod glyphs;
mod view;

use std::collections::HashSet;
use std::sync::Arc;

use rune::{El, TextEdit, Tx};
use rune_kit::Kit;

use crate::bluetooth::model::{
    Bluetooth, Command, DeviceId, PairingKind, Pending, State, Subscription,
};

/// The state every handler gets: the window's (or the tray panel's).
pub type Host = crate::settings::Ui;

/// A handler written for this section's state, as one for the window's.
pub fn lift(f: impl Fn(&mut Ui) + 'static) -> impl Fn(&mut Host) + 'static {
    move |h: &mut Host| f(&mut h.bluetooth)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Window,
    Popover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Adapter,
    Devices,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Advanced(Tab),
}

pub struct Ui {
    pub bt: Arc<State>,
    /// `None` in headless scenes.
    pub handle: Option<Bluetooth>,
    pub mode: Mode,
    pub visible: bool,
    pub page: Page,
    /// The device row that's open (level two).
    pub open: Option<DeviceId>,
    /// The open row's name, being edited.
    pub rename: TextEdit,
    /// "Add device" is open: discovery runs while it is.
    pub adding: bool,
    /// A PIN or passkey being typed for the pairing request.
    pub code: TextEdit,
    /// The open select menu, by key.
    pub menu: Option<String>,
    /// The adapter's alias, being edited (advanced).
    pub alias: Option<TextEdit>,
    /// Devices whose details are open on the advanced page.
    pub details: HashSet<DeviceId>,
    /// The Sound section is on: audio devices link to it.
    pub sound: bool,
    pub _subscription: Option<Arc<Subscription>>,
}

impl Ui {
    pub fn new(bt: Option<Bluetooth>, mode: Mode) -> Ui {
        let state = bt
            .as_ref()
            .map_or_else(|| Arc::new(State::default()), Bluetooth::state);
        Ui {
            bt: state,
            handle: bt,
            mode,
            visible: true,
            page: Page::Main,
            open: None,
            rename: TextEdit::new(""),
            adding: false,
            code: TextEdit::new(""),
            menu: None,
            alias: None,
            details: HashSet::new(),
            sound: false,
            _subscription: None,
        }
    }

    /// Swaps the backend (`None` when the module turns off), keeping what's on screen.
    pub fn attach(&mut self, bt: Option<Bluetooth>, subscription: Option<Subscription>) {
        self.bt = bt
            .as_ref()
            .map_or_else(|| Arc::new(State::default()), Bluetooth::state);
        self.handle = bt;
        self._subscription = subscription.map(Arc::new);
        self.open = None;
        self.adding = false;
        self.page = Page::Main;
    }

    pub fn send(&self, c: Command) {
        if let Some(b) = &self.handle {
            b.send(c);
        }
    }

    pub fn receive(&mut self, state: Arc<State>) {
        if self
            .open
            .as_ref()
            .is_some_and(|id| state.device(id).is_none())
        {
            self.open = None;
        }
        self.details.retain(|id| state.device(id).is_some());
        if state.pairing.as_ref().map(|p| p.id) != self.bt.pairing.as_ref().map(|p| p.id) {
            self.code = TextEdit::new("");
        }
        // A device being paired that isn't in a list on screen opens the scan list.
        if let Some(p) = &state.pairing
            && state.device(&p.device).is_some_and(|d| !d.is_paired())
        {
            self.adding = true;
        }
        let powered = state.default_adapter().is_some_and(|a| a.powered);
        if !powered {
            self.adding = false;
        }
        self.bt = state;
    }

    fn adapter(&self) -> Option<crate::bluetooth::model::AdapterId> {
        self.bt.default_adapter.clone()
    }

    pub fn powered(&self) -> bool {
        self.bt.default_adapter().is_some_and(|a| a.powered)
    }

    pub fn set_powered(&mut self, on: bool) {
        if let Some(adapter) = self.adapter() {
            self.send(Command::SetPowered { adapter, on });
        }
    }

    /// A connected device's row: disconnect; a paired one's: connect.
    pub fn activate(&mut self, id: &DeviceId) {
        let Some(d) = self.bt.device(id) else { return };
        let device = id.clone();
        match d.pending {
            Some(Pending::Connecting) => self.send(Command::Disconnect { device }),
            Some(_) => {}
            None if d.connected => self.send(Command::Disconnect { device }),
            None if d.is_paired() => self.send(Command::Connect { device }),
            None => self.send(Command::Pair { device }),
        }
    }

    pub fn toggle_open(&mut self, id: &DeviceId) {
        if self.open.as_ref() == Some(id) {
            self.open = None;
            return;
        }
        let alias = self
            .bt
            .device(id)
            .map(|d| d.display_name().to_owned())
            .unwrap_or_default();
        self.rename = TextEdit::new(&alias);
        self.open = Some(id.clone());
    }

    pub fn commit_rename(&mut self) {
        let Some(id) = self.open.clone() else { return };
        let Some(d) = self.bt.device(&id) else { return };
        let alias = self.rename.text.trim().to_owned();
        if alias != d.display_name() {
            let reported = d.name.clone().unwrap_or_default();
            let alias = if alias == reported {
                String::new()
            } else {
                alias
            };
            self.send(Command::SetAlias { device: id, alias });
        }
    }

    pub fn set_adding(&mut self, on: bool) {
        if self.adding == on {
            return;
        }
        self.adding = on;
        let Some(adapter) = self.adapter() else {
            return;
        };
        if on {
            self.send(Command::StartDiscovery { adapter });
        } else {
            if let Some(p) = self.bt.pairing.clone()
                && self.bt.device(&p.device).is_some_and(|d| !d.is_paired())
            {
                self.send(Command::CancelPair { device: p.device });
            }
            self.send(Command::StopDiscovery { adapter });
        }
    }

    pub fn reply(&mut self, accept: bool) {
        let Some(p) = self.bt.pairing.clone() else {
            return;
        };
        let reply = if !accept {
            crate::bluetooth::model::Reply::Reject
        } else {
            let typed = self.code.text.trim().to_owned();
            match p.kind {
                PairingKind::RequestPin => crate::bluetooth::model::Reply::Pin(typed),
                PairingKind::RequestPasskey => match typed.parse() {
                    Ok(n) => crate::bluetooth::model::Reply::Passkey(n),
                    Err(_) => return,
                },
                PairingKind::DisplayPasskey { .. } | PairingKind::DisplayPin { .. } => {
                    self.send(Command::CancelPair { device: p.device });
                    return;
                }
                _ => crate::bluetooth::model::Reply::Accept,
            }
        };
        self.send(Command::PairingReply { id: p.id, reply });
    }

    pub fn hidden(&mut self) {
        self.visible = false;
        self.menu = None;
        self.set_adding(false);
    }

    /// Esc: close what's open, one layer at a time.
    pub fn escape(&mut self) -> bool {
        if self.menu.take().is_some() || self.alias.take().is_some() {
            return true;
        }
        if self.adding {
            self.set_adding(false);
            return true;
        }
        if self.open.take().is_some() {
            return true;
        }
        false
    }
}

/// The section's first screen with its rows. Select menus go to `layer`.
pub fn section(s: &Ui, k: &Kit, _layer: &mut Vec<El>) -> El {
    view::main(s, k)
}

/// The advanced page, beside the first screen in the window.
pub fn advanced(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> El {
    let tab = match s.page {
        Page::Advanced(t) => t,
        Page::Main => Tab::Adapter,
    };
    advanced::page(s, k, tab, layer)
}

/// The tray panel's rows: connected devices, then paired ones to connect.
pub fn panel(s: &Ui, k: &Kit) -> Vec<El> {
    view::tray_rows(s, k)
}

/// Whether the tray panel has anything to show.
pub fn ready(s: &Ui) -> bool {
    s.bt.connection == crate::bluetooth::model::Connection::Ready && !s.bt.adapters.is_empty()
}

/// Keys for the section: Esc.
pub fn on_key(s: &mut Ui, e: &rune::KeyEvent, _tx: &mut Tx) -> bool {
    use rune::Key;
    match &e.key {
        Key::Escape => s.escape(),
        _ => false,
    }
}

/// Scenes: the mock sample, then each level.
pub mod scenes {
    use super::*;

    pub fn main(s: &mut Ui) {
        s.bt = Arc::new(crate::bluetooth::mock::sample());
    }

    pub fn expanded(s: &mut Ui) {
        main(s);
        let id = s.bt.connected().find(|d| d.audio).map(|d| d.id.clone());
        if let Some(id) = id {
            s.toggle_open(&id);
        }
    }

    pub fn pairing(s: &mut Ui) {
        s.bt = Arc::new(crate::bluetooth::mock::pairing());
        s.adding = true;
    }

    pub fn scan(s: &mut Ui) {
        s.bt = Arc::new(crate::bluetooth::mock::scanning());
        s.adding = true;
    }

    /// A keyboard asking for its passkey to be typed on it, three digits in.
    pub fn typing(s: &mut Ui) {
        let mut st = crate::bluetooth::mock::scanning();
        let kb = st
            .nearby()
            .find(|d| d.kind == crate::bluetooth::model::Kind::Keyboard)
            .map(|d| d.id.clone());
        if let Some(kb) = kb {
            if let Some(d) = st.devices.iter_mut().find(|d| d.id == kb) {
                d.pending = Some(Pending::Pairing);
            }
            st.pairing = Some(crate::bluetooth::model::PairingRequest {
                id: 2,
                device: kb,
                kind: PairingKind::DisplayPasskey {
                    passkey: 205117,
                    entered: 3,
                },
            });
        }
        s.bt = Arc::new(st);
        s.adding = true;
    }

    /// Connecting to a speaker, and a failed connect's issue.
    pub fn pending(s: &mut Ui) {
        let mut st = crate::bluetooth::mock::sample();
        if let Some(d) = st
            .devices
            .iter_mut()
            .find(|d| d.is_paired() && !d.connected)
        {
            d.pending = Some(Pending::Connecting);
        }
        st.issues.push(crate::bluetooth::model::Issue {
            id: 9,
            severity: crate::bluetooth::model::Severity::Error,
            message: "Couldn't connect to JBL Flip 6".into(),
            device: None,
        });
        s.bt = Arc::new(st);
    }

    /// The advanced Devices tab with the mouse's details open.
    pub fn devices(s: &mut Ui) {
        main(s);
        s.page = Page::Advanced(Tab::Devices);
        let id =
            s.bt.devices()
                .find(|d| d.kind == crate::bluetooth::model::Kind::Mouse)
                .map(|d| d.id.clone());
        s.details.extend(id);
    }

    pub fn advanced(s: &mut Ui) {
        main(s);
        s.page = Page::Advanced(Tab::Adapter);
    }

    pub fn off(s: &mut Ui) {
        s.bt = Arc::new(crate::bluetooth::mock::not_running());
    }
}
