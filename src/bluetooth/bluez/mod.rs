//! The BlueZ backend: talks to `org.bluez` on the system bus with zbus on its own thread,
//! keeps a [`State`] of adapters, devices (with `Battery1`) and the pairing agent's requests,
//! publishes it through a [`Publisher`] and carries out [`Command`]s. See `crate::bluetooth::model` for
//! the contract.
//!
//! One loop thread owns everything and reacts to [`Event`]s, one at a time:
//!
//! - commands from the UI (forwarded from the driver's receiver);
//! - signals: `InterfacesAdded`/`InterfacesRemoved` (ObjectManager), `PropertiesChanged`
//!   under `/org/bluez`, and `NameOwnerChanged` for `org.bluez`, each read on its own thread;
//! - results of slow method calls (connect, pair, ...), which run on short-lived threads so
//!   the loop never blocks on a device;
//! - pairing agent calls (see [`agent`]).
//!
//! Snapshots are rebuilt from BlueZ's objects and published at most every 50 ms. When
//! `bluetoothd` goes away the state says `NotRunning` and everything is rebuilt (and the
//! agent registered again) when it returns; when the bus itself fails it says `Lost` and
//! reconnects with backoff. `HOKUM_BLUETOOTH_DEBUG=1` logs what it does to stderr.

mod agent;
pub mod debug;
mod map;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use zbus::blocking::{Connection as Bus, MessageIterator};
use zbus::message::Type as MessageType;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{DBusError, MatchRule, Message};

use crate::bluetooth::model::{self, Bluetooth, Command, Connection, DeviceId, Issue, PairingKind, PairingRequest, Pending, Publisher, Reply, Service, Severity, State};
use map::{ADAPTER, DEVICE, Objects};

const BLUEZ: &str = "org.bluez";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
/// Minimum time between snapshots.
const COALESCE: Duration = Duration::from_millis(50);
/// How long a pairing prompt waits for the user before it's rejected.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ISSUES: usize = 8;

macro_rules! log {
    ($($t:tt)*) => {
        if std::env::var_os("HOKUM_BLUETOOTH_DEBUG").is_some_and(|v| !v.is_empty() && v != "0") {
            eprintln!("bluez: {}", format!($($t)*));
        }
    };
}

/// What this backend does about pairing (see [`agent`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// `hokum service`: registers an agent, and claims the default unless another Bluetooth
    /// app already holds it.
    Service,
    /// Debug tooling: no agent.
    Inspect,
}

/// Connects to the system bus and starts the backend thread, as the service.
pub fn start() -> Result<Bluetooth, String> {
    spawn(Role::Service).map(|(bt, _)| bt)
}

/// Starts the backend; the handle finishes once every [`Bluetooth`] is dropped and the agent
/// is unregistered and discovery stopped.
fn spawn(role: Role) -> Result<(Bluetooth, JoinHandle<()>), String> {
    let (bt, driver) = model::channel();
    let (publisher, commands) = driver.split();
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        thread::Builder::new()
            .name("bluez-commands".into())
            .spawn(move || {
                for c in commands.iter() {
                    if tx.send(Event::Command(c)).is_err() {
                        return;
                    }
                }
                let _ = tx.send(Event::CommandsClosed);
            })
            .map_err(|e| e.to_string())?;
    }
    let stopped = publisher.clone();
    let handle = thread::Builder::new()
        .name("bluez".into())
        .spawn(move || {
            // A bug here mustn't take notifications down with it: the section says it stopped.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Backend::new(role, publisher, tx).run(rx))).is_err() {
                stopped.publish(State { connection: Connection::Lost("Bluetooth stopped working; restart Hokum".into()), ..State::default() });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok((bt, handle))
}

pub enum Event {
    Command(Command),
    CommandsClosed,
    /// A signal read on connection generation `.0`.
    Signal(u64, Message),
    /// A signal reader on generation `.0` stopped: the bus connection is gone.
    BusLost(u64, String),
    Done(Done, Result<(), Fail>),
    /// A multi-step call moved on (pair → connect).
    Progress(DeviceId, Pending),
    Agent(agent::Call),
    AgentCancel(String),
    AgentReleased,
}

/// What a finished call was about.
pub struct Done {
    device: Option<DeviceId>,
    /// Issue text if it fails ("Couldn't connect to AirPods Pro").
    what: String,
    /// The call set the device's pending state.
    pending: bool,
}

/// A failed call, optionally with its own issue text (a later step of a multi-step call).
pub struct Fail {
    what: Option<String>,
    error: Box<zbus::Error>,
}

impl From<zbus::Error> for Fail {
    fn from(error: zbus::Error) -> Fail {
        Fail { what: None, error: Box::new(error) }
    }
}

fn error_parts(e: &zbus::Error) -> (Option<String>, Option<String>) {
    match e {
        zbus::Error::MethodError(name, message, _) => (Some(name.to_string()), message.clone()),
        zbus::Error::FDO(f) => (Some(f.name().to_string()), f.description().map(Into::into)),
        _ => (None, None),
    }
}

fn is_error(e: &zbus::Error, short: &str) -> bool {
    error_parts(e).0.is_some_and(|n| n.rsplit('.').next() == Some(short))
}

fn call(bus: &Bus, path: &str, iface: &str, method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> zbus::Result<Message> {
    bus.call_method(Some(BLUEZ), path, Some(iface), method, body)
}

fn set(bus: &Bus, path: &str, iface: &str, prop: &str, value: Value<'_>) -> zbus::Result<()> {
    call(bus, path, PROPERTIES, "Set", &(iface, prop, value)).map(|_| ())
}

/// A pairing prompt the user hasn't answered yet.
struct Live {
    request: PairingRequest,
    answer: Option<async_channel::Sender<Reply>>,
    deadline: Instant,
}

struct Backend {
    role: Role,
    publisher: Publisher,
    events: Sender<Event>,
    bus: Option<Bus>,
    /// Bumped on every (re)connect so stale signal readers are ignored.
    generation: u64,
    /// `org.bluez`'s unique name while it runs.
    owner: Option<String>,
    connection: Connection,
    objects: Objects,
    selected: Option<model::AdapterId>,
    pending: HashMap<DeviceId, Pending>,
    /// Pairings the user cancelled or rejected: their failure isn't an issue.
    cancelled: HashSet<DeviceId>,
    pairing: Option<Live>,
    issues: Vec<Issue>,
    next_id: u64,
    /// Adapters this process started discovery on.
    discovering: HashSet<model::AdapterId>,
    agent_registered: bool,
    /// This process should be the default agent.
    claim_default: bool,
    dirty: bool,
    last_publish: Instant,
    retry_at: Option<Instant>,
    backoff: Duration,
}

impl Backend {
    fn new(role: Role, publisher: Publisher, events: Sender<Event>) -> Backend {
        let claim_default = role == Role::Service;
        Backend {
            role,
            publisher,
            events,
            bus: None,
            generation: 0,
            owner: None,
            connection: Connection::Connecting,
            objects: Objects::new(),
            selected: None,
            pending: HashMap::new(),
            cancelled: HashSet::new(),
            pairing: None,
            issues: Vec::new(),
            next_id: 0,
            discovering: HashSet::new(),
            agent_registered: false,
            claim_default,
            dirty: true,
            last_publish: Instant::now() - COALESCE,
            retry_at: None,
            backoff: Duration::from_secs(1),
        }
    }

    fn run(mut self, rx: Receiver<Event>) {
        self.publish();
        self.connect();
        loop {
            self.tick();
            let deadline = [self.dirty.then(|| self.last_publish + COALESCE), self.retry_at, self.pairing.as_ref().map(|l| l.deadline)].into_iter().flatten().min();
            let event = match deadline {
                Some(d) => rx.recv_timeout(d.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match event {
                Ok(Event::CommandsClosed) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(e) => self.handle(e),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        self.shutdown();
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.retry_at.is_some_and(|t| t <= now) {
            self.retry_at = None;
            self.connect();
        }
        if self.pairing.as_ref().is_some_and(|l| l.deadline <= now)
            && let Some(live) = self.pairing.take()
        {
            log!("pairing request {} timed out", live.request.id);
            if let Some(answer) = live.answer {
                let _ = answer.try_send(Reply::Reject);
                self.cancelled.insert(live.request.device.clone());
                let name = self.name(&live.request.device);
                self.issue(format!("Pairing with {name} timed out"), Some(live.request.device));
            }
            self.dirty = true;
        }
        if self.dirty && self.last_publish + COALESCE <= now {
            self.publish();
        }
    }

    fn publish(&mut self) {
        let (adapters, mut devices) = map::build(&self.objects);
        for d in &mut devices {
            d.pending = self.pending.get(&d.id).copied();
        }
        let default_adapter = map::default_adapter(&adapters, self.selected.as_ref());
        let pairing = self.pairing.as_ref().map(|l| l.request.clone()).filter(|p| devices.iter().any(|d| d.id == p.device));
        self.publisher.publish(State { revision: 0, connection: self.connection.clone(), adapters, default_adapter, devices, pairing, issues: self.issues.clone() });
        self.dirty = false;
        self.last_publish = Instant::now();
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn issue(&mut self, message: String, device: Option<DeviceId>) {
        log!("issue: {message}");
        let id = self.id();
        self.issues.push(Issue { id, severity: Severity::Error, message, device });
        if self.issues.len() > MAX_ISSUES {
            self.issues.remove(0);
        }
        self.dirty = true;
    }

    fn name(&self, id: &DeviceId) -> String {
        map::device(&id.0, &self.objects).map_or_else(|| "the device".into(), |d| d.display_name().to_owned())
    }

    fn adapter_name(&self, id: &model::AdapterId) -> String {
        self.objects.get(&id.0).and_then(|i| i.get(ADAPTER)).map_or_else(String::new, |p| map::adapter(&id.0, p).display_name().to_owned())
    }

    // Connection lifecycle.

    fn connect(&mut self) {
        let bus = match Bus::system() {
            Ok(b) => b,
            Err(e) => return self.lost(format!("{e}")),
        };
        self.generation += 1;
        if self.role != Role::Inspect
            && let Err(e) = bus.object_server().at(agent::PATH, agent::Agent { events: self.events.clone() })
        {
            log!("couldn't export the agent: {e}");
        }
        let rules = (|| -> zbus::Result<Vec<MatchRule<'static>>> {
            Ok(vec![
                MatchRule::builder().msg_type(MessageType::Signal).sender("org.freedesktop.DBus")?.interface("org.freedesktop.DBus")?.member("NameOwnerChanged")?.add_arg(BLUEZ)?.build(),
                MatchRule::builder().msg_type(MessageType::Signal).interface("org.freedesktop.DBus.ObjectManager")?.path("/")?.build(),
                MatchRule::builder().msg_type(MessageType::Signal).interface(PROPERTIES)?.member("PropertiesChanged")?.path_namespace("/org/bluez")?.build(),
            ])
        })();
        let rules = match rules {
            Ok(r) => r,
            Err(e) => return self.lost(format!("{e}")),
        };
        for rule in rules {
            let iter = match MessageIterator::for_match_rule(rule, &bus, Some(1024)) {
                Ok(i) => i,
                Err(e) => return self.lost(format!("{e}")),
            };
            let (tx, generation) = (self.events.clone(), self.generation);
            let _ = thread::Builder::new().name("bluez-signals".into()).spawn(move || {
                for m in iter {
                    match m {
                        Ok(m) => {
                            if tx.send(Event::Signal(generation, m)).is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Event::BusLost(generation, format!("{e}")));
                            return;
                        }
                    }
                }
                let _ = tx.send(Event::BusLost(generation, "the system bus closed".into()));
            });
        }
        let owner = zbus::blocking::fdo::DBusProxy::new(&bus).and_then(|p| Ok(p.get_name_owner(BLUEZ.try_into()?)?));
        self.bus = Some(bus);
        self.backoff = Duration::from_secs(1);
        match owner {
            Ok(owner) => self.bluez_up(owner.to_string()),
            Err(_) => self.bluez_down(),
        }
    }

    fn lost(&mut self, reason: String) {
        log!("bus lost: {reason}");
        self.bluez_down();
        self.bus = None;
        self.connection = Connection::Lost(reason);
        self.retry_at = Some(Instant::now() + self.backoff);
        self.backoff = (self.backoff * 2).min(Duration::from_secs(15));
    }

    fn bluez_down(&mut self) {
        log!("bluetoothd is not running");
        self.owner = None;
        self.objects.clear();
        self.pending.clear();
        self.pairing = None;
        self.discovering.clear();
        self.agent_registered = false;
        self.connection = Connection::NotRunning;
        self.dirty = true;
    }

    fn bluez_up(&mut self, owner: String) {
        log!("bluetoothd is {owner}");
        self.owner = Some(owner);
        self.connection = Connection::Ready;
        self.dirty = true;
        let Some(bus) = self.bus.clone() else { return };
        type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
        match call(&bus, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &()).and_then(|m| m.body().deserialize::<Managed>()) {
            Ok(objects) => {
                self.objects = objects.into_iter().map(|(path, ifaces)| (path.to_string(), ifaces.into_iter().collect::<BTreeMap<_, _>>())).collect();
            }
            Err(e) => log!("GetManagedObjects failed: {e}"),
        }
        if self.role != Role::Inspect {
            self.register_agent(&bus);
        }
    }

    fn register_agent(&mut self, bus: &Bus) {
        let path = ObjectPath::from_static_str_unchecked(agent::PATH);
        match call(bus, "/org/bluez", AGENT_MANAGER, "RegisterAgent", &(&path, agent::CAPABILITY)) {
            Ok(_) => self.agent_registered = true,
            Err(e) if is_error(&e, "AlreadyExists") => self.agent_registered = true,
            Err(e) => log!("RegisterAgent failed: {e}"),
        }
        log!("agent registered: {}", self.agent_registered);
        if self.agent_registered && self.claim_default && !agent::other_default_agent() {
            self.request_default(bus);
        } else if self.agent_registered {
            log!("another Bluetooth app is the default agent; ours answers pairings started here");
        }
    }

    fn request_default(&mut self, bus: &Bus) {
        let path = ObjectPath::from_static_str_unchecked(agent::PATH);
        match call(bus, "/org/bluez", AGENT_MANAGER, "RequestDefaultAgent", &(&path,)) {
            Ok(_) => log!("default agent"),
            Err(e) => log!("RequestDefaultAgent failed: {e}"),
        }
    }

    fn shutdown(&mut self) {
        let Some(bus) = self.bus.clone() else { return };
        if self.owner.is_some() {
            for adapter in std::mem::take(&mut self.discovering) {
                let r = call(&bus, &adapter.0, ADAPTER, "StopDiscovery", &());
                log!("stopped discovery on {adapter}: {:?}", r.err());
            }
            if self.agent_registered {
                let path = ObjectPath::from_static_str_unchecked(agent::PATH);
                let r = call(&bus, "/org/bluez", AGENT_MANAGER, "UnregisterAgent", &(&path,));
                log!("agent unregistered: {:?}", r.err());
            }
        }
        let _ = bus.object_server().remove::<agent::Agent, _>(agent::PATH);
    }

    // Events.

    fn handle(&mut self, event: Event) {
        match event {
            Event::Command(c) => self.command(c),
            Event::CommandsClosed => {}
            Event::Signal(generation, m) if generation == self.generation => self.signal(m),
            Event::Signal(..) => {}
            Event::BusLost(generation, reason) if generation == self.generation && self.bus.is_some() => self.lost(reason),
            Event::BusLost(..) => {}
            Event::Done(done, result) => self.done(done, result),
            Event::Progress(device, pending) => {
                self.pending.insert(device, pending);
                self.dirty = true;
            }
            Event::Agent(call) => self.agent_call(call),
            Event::AgentCancel(sender) => {
                if self.owner.as_deref() == Some(&sender) && self.pairing.take().is_some() {
                    log!("agent request cancelled by BlueZ");
                    self.dirty = true;
                }
            }
            Event::AgentReleased => {
                log!("agent released by BlueZ");
                self.agent_registered = false;
            }
        }
    }

    fn signal(&mut self, m: Message) {
        let header = m.header();
        let member = header.member().map(|m| m.to_string()).unwrap_or_default();
        let sender = header.sender().map(|s| s.to_string());
        if member == "NameOwnerChanged" {
            if let Ok((name, _old, new)) = m.body().deserialize::<(String, String, String)>()
                && name == BLUEZ
            {
                if new.is_empty() {
                    self.bluez_down();
                } else if self.owner.as_deref() != Some(&new) {
                    self.bluez_down();
                    self.bluez_up(new);
                }
            }
            return;
        }
        if self.owner.is_none() || sender != self.owner {
            return;
        }
        let path = header.path().map(|p| p.to_string()).unwrap_or_default();
        match member.as_str() {
            "InterfacesAdded" => {
                if let Ok((path, ifaces)) = m.body().deserialize::<(OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>)>() {
                    self.objects.entry(path.to_string()).or_default().extend(ifaces);
                    self.dirty = true;
                }
            }
            "InterfacesRemoved" => {
                if let Ok((path, ifaces)) = m.body().deserialize::<(OwnedObjectPath, Vec<String>)>() {
                    let path = path.to_string();
                    if let Some(entry) = self.objects.get_mut(&path) {
                        for i in &ifaces {
                            entry.remove(i);
                        }
                        if entry.is_empty() {
                            self.objects.remove(&path);
                        }
                    }
                    if ifaces.iter().any(|i| i == DEVICE) {
                        let id = DeviceId(path);
                        self.pending.remove(&id);
                        if self.pairing.as_ref().is_some_and(|l| l.request.device == id) {
                            self.pairing = None;
                        }
                    }
                    self.dirty = true;
                }
            }
            "PropertiesChanged" => {
                let Ok((iface, changed, invalidated)) = m.body().deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>() else { return };
                let Some(props) = self.objects.get_mut(&path).and_then(|i| i.get_mut(&iface)) else { return };
                let paired = iface == DEVICE && changed.get("Paired").is_some_and(|v| matches!(&**v, Value::Bool(true)));
                props.extend(changed);
                for k in invalidated {
                    props.remove(&k);
                }
                if paired && self.pairing.as_ref().is_some_and(|l| l.request.device.0 == path && l.answer.is_none()) {
                    self.pairing = None;
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn done(&mut self, done: Done, result: Result<(), Fail>) {
        if done.pending
            && let Some(d) = &done.device
        {
            self.pending.remove(d);
        }
        let user_cancelled = done.device.as_ref().is_some_and(|d| self.cancelled.remove(d));
        if let Some(d) = &done.device
            && self.pairing.as_ref().is_some_and(|l| &l.request.device == d && l.answer.is_none())
        {
            self.pairing = None;
        }
        self.dirty = true;
        let Err(fail) = result else { return };
        log!("{} failed: {}", done.what, fail.error);
        if user_cancelled || is_error(&fail.error, "AuthenticationCanceled") && fail.what.is_none() {
            return;
        }
        let (name, message) = error_parts(&fail.error);
        let text = map::issue_text(fail.what.as_deref().unwrap_or(&done.what), name.as_deref(), message.as_deref());
        self.issue(text, done.device);
    }

    /// Runs `f` on its own thread and reports back with [`Event::Done`].
    fn spawn(&mut self, device: Option<DeviceId>, pending: Option<Pending>, what: String, f: impl FnOnce(&Bus, &Sender<Event>) -> Result<(), Fail> + Send + 'static) {
        let Some(bus) = self.bus.clone() else { return };
        if let (Some(d), Some(p)) = (&device, pending) {
            self.pending.insert(d.clone(), p);
            self.dirty = true;
        }
        let tx = self.events.clone();
        let done = Done { device, what, pending: pending.is_some() };
        let _ = thread::Builder::new().name("bluez-call".into()).spawn(move || {
            let result = f(&bus, &tx);
            let _ = tx.send(Event::Done(done, result));
        });
    }

    fn set_adapter(&mut self, adapter: model::AdapterId, prop: &'static str, value: Value<'static>, what: String) {
        self.spawn(None, None, what, move |bus, _| Ok(set(bus, &adapter.0, ADAPTER, prop, value)?));
    }

    fn set_device(&mut self, device: DeviceId, prop: &'static str, value: Value<'static>, what: String) {
        let path = device.0.clone();
        self.spawn(Some(device), None, what, move |bus, _| Ok(set(bus, &path, DEVICE, prop, value)?));
    }

    fn device_call(&mut self, device: DeviceId, pending: Pending, method: &'static str, body: Option<String>, what: String) {
        if self.pending.get(&device) == Some(&pending) {
            return;
        }
        let path = device.0.clone();
        self.spawn(Some(device), Some(pending), what, move |bus, _| {
            match body {
                Some(uuid) => call(bus, &path, DEVICE, method, &(uuid,))?,
                None => call(bus, &path, DEVICE, method, &())?,
            };
            Ok(())
        });
    }

    fn command(&mut self, c: Command) {
        log!("command {c:?}");
        match c {
            Command::PairingReply { id, reply } => return self.reply(id, reply),
            Command::SelectAdapter { adapter } => {
                if self.objects.get(&adapter.0).is_some_and(|i| i.contains_key(ADAPTER)) {
                    self.selected = Some(adapter);
                    self.dirty = true;
                }
                return;
            }
            Command::DismissIssue { id } => {
                self.issues.retain(|i| i.id != id);
                self.dirty = true;
                return;
            }
            _ => {}
        }
        if self.bus.is_none() || self.owner.is_none() {
            return self.issue("Bluetooth isn't running".into(), None);
        }
        match c {
            Command::SetPowered { adapter, on } => {
                let what = if on { "Couldn't turn Bluetooth on" } else { "Couldn't turn Bluetooth off" };
                self.set_adapter(adapter, "Powered", Value::Bool(on), what.into());
            }
            Command::SetDiscoverable { adapter, on } => {
                let what = format!("Couldn't make {} {}", self.adapter_name(&adapter), if on { "visible" } else { "hidden" });
                self.set_adapter(adapter, "Discoverable", Value::Bool(on), what);
            }
            Command::SetDiscoverableTimeout { adapter, seconds } => self.set_adapter(adapter, "DiscoverableTimeout", Value::U32(seconds), "Couldn't change the timeout".into()),
            Command::SetPairable { adapter, on } => self.set_adapter(adapter, "Pairable", Value::Bool(on), "Couldn't change pairable".into()),
            Command::SetPairableTimeout { adapter, seconds } => self.set_adapter(adapter, "PairableTimeout", Value::U32(seconds), "Couldn't change the timeout".into()),
            Command::SetAdapterAlias { adapter, alias } => self.set_adapter(adapter, "Alias", Value::from(alias), "Couldn't rename the adapter".into()),
            Command::StartDiscovery { adapter } => {
                if !self.discovering.insert(adapter.clone()) {
                    return;
                }
                self.spawn(None, None, "Couldn't scan for devices".into(), move |bus, _| {
                    let filter: HashMap<&str, Value> = HashMap::from([("Transport", Value::from("auto"))]);
                    if let Err(e) = call(bus, &adapter.0, ADAPTER, "SetDiscoveryFilter", &(filter,)) {
                        log!("SetDiscoveryFilter failed: {e}");
                    }
                    match call(bus, &adapter.0, ADAPTER, "StartDiscovery", &()) {
                        Err(e) if !is_error(&e, "InProgress") => Err(e.into()),
                        _ => Ok(()),
                    }
                });
            }
            Command::StopDiscovery { adapter } => {
                if self.discovering.remove(&adapter) {
                    self.spawn(None, None, String::new(), move |bus, _| {
                        let _ = call(bus, &adapter.0, ADAPTER, "StopDiscovery", &());
                        Ok(())
                    });
                }
            }
            Command::Connect { device } => {
                let what = format!("Couldn't connect to {}", self.name(&device));
                self.device_call(device, Pending::Connecting, "Connect", None, what);
            }
            Command::Disconnect { device } => {
                let what = format!("Couldn't disconnect {}", self.name(&device));
                self.device_call(device, Pending::Disconnecting, "Disconnect", None, what);
            }
            Command::Pair { device } => {
                if self.pending.get(&device) == Some(&Pending::Pairing) {
                    return;
                }
                let name = self.name(&device);
                self.cancelled.remove(&device);
                let (path, id) = (device.0.clone(), device.clone());
                self.spawn(Some(device), Some(Pending::Pairing), format!("Couldn't pair with {name}"), move |bus, tx| {
                    match call(bus, &path, DEVICE, "Pair", &()) {
                        Err(e) if !is_error(&e, "AlreadyExists") => return Err(e.into()),
                        _ => {}
                    }
                    if let Err(e) = set(bus, &path, DEVICE, "Trusted", Value::Bool(true)) {
                        log!("couldn't trust {path}: {e}");
                    }
                    let _ = tx.send(Event::Progress(id, Pending::Connecting));
                    match call(bus, &path, DEVICE, "Connect", &()) {
                        Err(e) => Err(Fail { what: Some(format!("Couldn't connect to {name}")), error: Box::new(e) }),
                        Ok(_) => Ok(()),
                    }
                });
            }
            Command::CancelPair { device } => {
                self.cancelled.insert(device.clone());
                if self.pairing.as_ref().is_some_and(|l| l.request.device == device) {
                    self.pairing = None;
                    self.dirty = true;
                }
                let path = device.0.clone();
                self.spawn(None, None, String::new(), move |bus, _| {
                    let _ = call(bus, &path, DEVICE, "CancelPairing", &());
                    Ok(())
                });
            }
            Command::Trust { device } => {
                let what = format!("Couldn't trust {}", self.name(&device));
                self.set_device(device, "Trusted", Value::Bool(true), what);
            }
            Command::Untrust { device } => {
                let what = format!("Couldn't stop trusting {}", self.name(&device));
                self.set_device(device, "Trusted", Value::Bool(false), what);
            }
            Command::Block { device } => {
                let what = format!("Couldn't block {}", self.name(&device));
                self.set_device(device, "Blocked", Value::Bool(true), what);
            }
            Command::Unblock { device } => {
                let what = format!("Couldn't unblock {}", self.name(&device));
                self.set_device(device, "Blocked", Value::Bool(false), what);
            }
            Command::SetAlias { device, alias } => {
                let what = format!("Couldn't rename {}", self.name(&device));
                self.set_device(device, "Alias", Value::from(alias), what);
            }
            Command::Remove { device } => {
                let Some(d) = map::device(&device.0, &self.objects) else { return };
                let what = format!("Couldn't forget {}", d.display_name());
                let (adapter, path) = (d.adapter.0, device.0.clone());
                self.spawn(Some(device), Some(Pending::Removing), what, move |bus, _| {
                    let path = ObjectPath::try_from(path.as_str()).map_err(zbus::Error::from)?;
                    call(bus, &adapter, ADAPTER, "RemoveDevice", &(path,))?;
                    Ok(())
                });
            }
            Command::ConnectProfile { device, uuid } => {
                let what = format!("Couldn't connect {} on {}", Service::new(&uuid).name, self.name(&device));
                self.device_call(device, Pending::Profile, "ConnectProfile", Some(uuid), what);
            }
            Command::DisconnectProfile { device, uuid } => {
                let what = format!("Couldn't disconnect {} on {}", Service::new(&uuid).name, self.name(&device));
                self.device_call(device, Pending::Profile, "DisconnectProfile", Some(uuid), what);
            }
            Command::PairingReply { .. } | Command::SelectAdapter { .. } | Command::DismissIssue { .. } => {}
        }
    }

    // The pairing agent.

    fn agent_call(&mut self, call: agent::Call) {
        if self.owner.as_deref() != Some(call.sender.as_str()) {
            log!("ignoring an agent call from {}", call.sender);
            return;
        }
        let device = DeviceId(call.device);
        log!("agent: {:?} for {device}", call.ask);
        if let (agent::Ask::DisplayPasskey { passkey, entered }, Some(live)) = (&call.ask, self.pairing.as_mut())
            && live.request.device == device
            && matches!(live.request.kind, PairingKind::DisplayPasskey { .. })
        {
            live.request.kind = PairingKind::DisplayPasskey { passkey: *passkey, entered: *entered };
            self.dirty = true;
            return;
        }
        let kind = match call.ask {
            agent::Ask::PinCode => PairingKind::RequestPin,
            agent::Ask::DisplayPinCode(pin) => PairingKind::DisplayPin { pin },
            agent::Ask::Passkey => PairingKind::RequestPasskey,
            agent::Ask::DisplayPasskey { passkey, entered } => PairingKind::DisplayPasskey { passkey, entered },
            agent::Ask::Confirmation(passkey) => PairingKind::Confirm { passkey },
            agent::Ask::Authorization => PairingKind::AuthorizePairing,
            agent::Ask::AuthorizeService(uuid) => PairingKind::Authorize { service: Service::new(&uuid) },
        };
        let id = self.id();
        // Replacing an unanswered request drops its answer channel, which cancels it.
        self.pairing = Some(Live { request: PairingRequest { id, device, kind }, answer: call.answer, deadline: Instant::now() + PAIRING_TIMEOUT });
        self.dirty = true;
    }

    fn reply(&mut self, id: u64, reply: Reply) {
        let Some(live) = self.pairing.take_if(|l| l.request.id == id) else { return };
        self.dirty = true;
        let device = live.request.device;
        if reply == Reply::Reject {
            self.cancelled.insert(device.clone());
        }
        match live.answer {
            Some(answer) => {
                let _ = answer.try_send(reply);
            }
            None if reply == Reply::Reject => self.command(Command::CancelPair { device }),
            None => {}
        }
    }
}
