//! The pairing agent (`org.bluez.Agent1`) and who owns the default agent.
//!
//! `hokum service` registers one agent. BlueZ asks the agent of the connection that called
//! `Pair` first, so a pairing started from Hokum always prompts in Hokum. Requests nobody
//! started here (an incoming pairing, a paired but untrusted device asking for a service) go
//! to the *default* agent: Hokum asks to be it only when no other Bluetooth app that keeps one
//! (blueman, blueberry, Plasma's Bluedevil) is running, so it never takes the default from
//! them. BlueZ keeps default agents in a stack, so one started later takes over and hands it
//! back when it exits.
//!
//! Each agent call becomes [`Event::Agent`] for the backend loop, which turns it into
//! `State::pairing` and answers through a channel when `Command::PairingReply` arrives (or a
//! timeout rejects it).

use std::sync::mpsc::Sender;

use zbus::message::Header;
use zbus::zvariant::OwnedObjectPath;

use super::Event;
use crate::bluetooth::model::Reply;

pub const PATH: &str = "/ly/anoma/hokum/bluetooth/agent";
pub const CAPABILITY: &str = "KeyboardDisplay";

/// What an agent call asks.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    PinCode,
    DisplayPinCode(String),
    Passkey,
    DisplayPasskey { passkey: u32, entered: u16 },
    Confirmation(u32),
    Authorization,
    AuthorizeService(String),
}

pub struct Call {
    /// The caller's unique bus name; only BlueZ's is honoured.
    pub sender: String,
    pub device: String,
    pub ask: Ask,
    /// Where the answer goes, for calls that wait on one. Dropping it cancels the call.
    pub answer: Option<async_channel::Sender<Reply>>,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

pub struct Agent {
    pub events: Sender<Event>,
}

fn sender(h: &Header<'_>) -> String {
    h.sender().map(|s| s.to_string()).unwrap_or_default()
}

impl Agent {
    fn tell(&self, h: &Header<'_>, device: OwnedObjectPath, ask: Ask) {
        let _ = self.events.send(Event::Agent(Call { sender: sender(h), device: device.to_string(), ask, answer: None }));
    }

    async fn ask(&self, h: &Header<'_>, device: OwnedObjectPath, ask: Ask) -> Result<Reply, AgentError> {
        let (tx, rx) = async_channel::bounded(1);
        let call = Call { sender: sender(h), device: device.to_string(), ask, answer: Some(tx) };
        if self.events.send(Event::Agent(call)).is_err() {
            return Err(AgentError::Canceled("agent stopped".into()));
        }
        match rx.recv().await {
            Ok(Reply::Reject) => Err(AgentError::Rejected("rejected".into())),
            Ok(r) => Ok(r),
            Err(_) => Err(AgentError::Canceled("cancelled".into())),
        }
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    async fn release(&self) {
        let _ = self.events.send(Event::AgentReleased);
    }

    async fn request_pin_code(&self, #[zbus(header)] h: Header<'_>, device: OwnedObjectPath) -> Result<String, AgentError> {
        match self.ask(&h, device, Ask::PinCode).await? {
            Reply::Pin(p) if !p.is_empty() && p.len() <= 16 => Ok(p),
            _ => Err(AgentError::Rejected("no PIN".into())),
        }
    }

    async fn display_pin_code(&self, #[zbus(header)] h: Header<'_>, device: OwnedObjectPath, pincode: String) -> Result<(), AgentError> {
        self.tell(&h, device, Ask::DisplayPinCode(pincode));
        Ok(())
    }

    async fn request_passkey(&self, #[zbus(header)] h: Header<'_>, device: OwnedObjectPath) -> Result<u32, AgentError> {
        match self.ask(&h, device, Ask::Passkey).await? {
            Reply::Passkey(p) if p <= 999_999 => Ok(p),
            _ => Err(AgentError::Rejected("no passkey".into())),
        }
    }

    async fn display_passkey(&self, #[zbus(header)] h: Header<'_>, device: OwnedObjectPath, passkey: u32, entered: u16) {
        self.tell(&h, device, Ask::DisplayPasskey { passkey, entered });
    }

    async fn request_confirmation(&self, #[zbus(header)] h: Header<'_>, device: OwnedObjectPath, passkey: u32) -> Result<(), AgentError> {
        self.ask(&h, device, Ask::Confirmation(passkey)).await.map(|_| ())
    }

    async fn request_authorization(&self, #[zbus(header)] h: Header<'_>, device: OwnedObjectPath) -> Result<(), AgentError> {
        self.ask(&h, device, Ask::Authorization).await.map(|_| ())
    }

    async fn authorize_service(&self, #[zbus(header)] h: Header<'_>, device: OwnedObjectPath, uuid: String) -> Result<(), AgentError> {
        self.ask(&h, device, Ask::AuthorizeService(uuid)).await.map(|_| ())
    }

    async fn cancel(&self, #[zbus(header)] h: Header<'_>) {
        let _ = self.events.send(Event::AgentCancel(sender(&h)));
    }
}

/// Bluetooth apps that keep their own default agent while they run.
const OTHER_AGENTS: &[&str] = &["blueman-applet", "blueberry-tray", "kded5", "kded6", "overskride"];

/// Whether another Bluetooth app (blueman's applet, blueberry, Plasma's Bluedevil in kded)
/// is running and so holds the default agent. BlueZ has no call that says who does, and
/// `RequestDefaultAgent` would take it from them.
pub fn other_default_agent() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    dir.flatten().any(|e| {
        e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| OTHER_AGENTS.contains(&c.trim()))
    })
}
