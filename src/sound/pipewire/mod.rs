//! The PipeWire backend: connects to the daemon on its own thread, keeps a [`State`] of
//! devices, cards, streams, the clock and the graph, publishes it through a
//! [`Driver`](crate::sound::model::Driver) and carries out [`Command`](crate::sound::model::Command)s.
//! See `crate::sound::model` for the contract.
//!
//! - `store`: a plain-data mirror of the globals and the pure assembly of a `State`.
//! - `params`: the SPA params read and written (Props, profiles, routes).
//! - `meter`: peak-detect capture streams for live levels.
//! - `debug`: `sound debug ...`, the backend without the window.
//!
//! Volume and mute go where WirePlumber (and so wpctl and pavucontrol) put them: a card's
//! active `Route` (saved) for hardware devices, the node's `Props` for everything else.

pub mod debug;
mod meter;
mod params;
mod store;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::context::ContextRc;
use pw::core::CoreRc;
use pw::loop_::TimerSource;
use pw::main_loop::MainLoopRc;
use pw::properties::properties;
use pw::proxy::ProxyT;
use pw::registry::{GlobalObject, RegistryRc};
use pw::spa::param::ParamType;
use pw::spa::pod::Pod;
use pw::spa::utils::dict::DictRef;
use pw::types::ObjectType;

use crate::sound::model::{
    self, Command, Connection, Driver, Id, Issue, Severity, State, Target, Volume,
};

use self::params::Param;
use self::store::{LinkG, MetadataG, NodeG, PortG, Props, RunState, Store};

/// At most this often a snapshot goes out (~30 Hz).
const FRAME: Duration = Duration::from_millis(33);

enum Msg {
    Command(Command),
    Quit,
}

/// Connects to PipeWire and starts the backend thread. Only fails if a thread can't start;
/// a missing daemon shows as [`Connection::Lost`] and is retried.
pub fn start() -> Result<model::Audio, String> {
    let (audio, mut driver) = model::channel();
    let commands = driver.take_commands();
    let (tx, rx) = pw::channel::channel::<Msg>();
    let stopped = driver.on_stop();
    std::thread::Builder::new()
        .name("sound-pipewire".into())
        .spawn(move || {
            // A bug here mustn't take notifications down with it: the section says Sound stopped.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(driver, rx))).is_err() {
                stopped(crate::i18n::t("Sound stopped working; restart Hokum").into());
            }
        })
        .map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("sound-commands".into())
        .spawn(move || {
            for c in commands.iter() {
                if tx.send(Msg::Command(c)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Msg::Quit);
        })
        .map_err(|e| e.to_string())?;
    Ok(audio)
}

fn run(driver: Driver, rx: pw::channel::Receiver<Msg>) {
    pw::init();
    let lost = |driver: &Driver, e: String| {
        driver.publish(State {
            connection: Connection::Lost(e),
            ..State::default()
        })
    };
    // The loop lives as long as the process: leaking it lets timers be stored anywhere.
    let main_loop: &'static MainLoopRc = match MainLoopRc::new(None) {
        Ok(l) => Box::leak(Box::new(l)),
        Err(e) => return lost(&driver, format!("couldn't start a PipeWire loop: {e}")),
    };
    let context = match ContextRc::new(main_loop, None) {
        Ok(c) => c,
        Err(e) => return lost(&driver, format!("couldn't create a PipeWire context: {e}")),
    };
    let lp = main_loop.loop_();
    let backend = Rc::new_cyclic(|weak: &Weak<RefCell<Backend>>| {
        let w = weak.clone();
        let publish = lp.add_timer(move |_| {
            if let Some(b) = w.upgrade() {
                b.borrow_mut().tick();
            }
        });
        let w = weak.clone();
        let reconnect = lp.add_timer(move |_| {
            if let Some(b) = w.upgrade() {
                Backend::reconnect(&b);
            }
        });
        RefCell::new(Backend {
            weak: weak.clone(),
            driver,
            context,
            conn: None,
            store: Store::default(),
            issues: Vec::new(),
            connection: Connection::Connecting,
            ready: false,
            last: State::default(),
            dirty: false,
            armed: false,
            last_publish: Instant::now() - FRAME,
            publish,
            reconnect,
            backoff: Duration::from_millis(250),
            metering: HashSet::new(),
        })
    });
    let w = Rc::downgrade(&backend);
    let _rx = rx.attach(lp, move |msg| match msg {
        Msg::Command(c) => {
            if let Some(b) = w.upgrade() {
                b.borrow_mut().command(c);
            }
        }
        Msg::Quit => main_loop.quit(),
    });
    Backend::connect(&backend);
    main_loop.run();
    // Tear down proxies before the core.
    let conn = backend.borrow_mut().conn.take();
    drop(conn);
}

/// A bound global and its listener, held to keep both alive. Listeners first: fields drop in
/// order.
#[allow(dead_code)]
enum Bound {
    Node(pw::node::NodeListener, pw::node::Node),
    Device(pw::device::DeviceListener, pw::device::Device),
    Port(pw::port::PortListener, pw::port::Port),
    Link(pw::link::LinkListener, pw::link::Link),
    Client(pw::client::ClientListener, pw::client::Client),
    Metadata(pw::metadata::MetadataListener, pw::metadata::Metadata),
}

/// One connection to the daemon. Field order is drop order: proxies before the core.
struct Conn {
    meters: HashMap<Target, meter::Meter>,
    objects: HashMap<Id, Bound>,
    /// Proxies of objects we asked factories to create (links, virtual sinks).
    created: Vec<pw::proxy::Proxy>,
    /// Last seen flags per `(object, param id)`, to spot param changes.
    param_flags: HashMap<(Id, u32), u32>,
    /// Core sync seq → the enumerations it closes: `(object, param id)`.
    enum_syncs: HashMap<i32, Vec<(Id, u32)>>,
    next_seq: i32,
    /// The startup syncs still outstanding: (sync seq, round).
    init_sync: Option<(i32, u8)>,
    /// What each proxy was last asked to do, for error messages.
    pending: HashMap<u32, String>,
    _registry_listener: pw::registry::Listener,
    registry: RegistryRc,
    _core_listener: pw::core::Listener,
    core: CoreRc,
}

struct Backend {
    weak: Weak<RefCell<Backend>>,
    driver: Driver,
    context: ContextRc,
    conn: Option<Conn>,
    store: Store,
    issues: Vec<Issue>,
    connection: Connection,
    /// The first full picture after connecting is in.
    ready: bool,
    last: State,
    dirty: bool,
    armed: bool,
    last_publish: Instant,
    publish: TimerSource<'static>,
    reconnect: TimerSource<'static>,
    backoff: Duration,
    metering: HashSet<Target>,
}

fn props_of(dict: Option<&DictRef>) -> Props {
    dict.map(|d| {
        d.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    })
    .unwrap_or_default()
}

impl Backend {
    // ---- connection ------------------------------------------------------------------------

    fn connect(rc: &Rc<RefCell<Backend>>) {
        let mut b = rc.borrow_mut();
        let props = properties! {
            "application.name" => "Hokum Sound",
            "application.id" => "hokum-sound",
            "application.icon-name" => "multimedia-volume-control",
        };
        let core = match b.context.connect_rc(Some(props)) {
            Ok(c) => c,
            Err(e) => {
                b.lost(crate::i18n::failed(
                    "can't reach PipeWire: {}",
                    &[&e.to_string()],
                ));
                return;
            }
        };
        let registry = match core.get_registry_rc() {
            Ok(r) => r,
            Err(e) => {
                b.lost(crate::i18n::failed(
                    "PipeWire registry: {}",
                    &[&e.to_string()],
                ));
                return;
            }
        };
        let w = b.weak.clone();
        let w2 = b.weak.clone();
        let w3 = b.weak.clone();
        let core_listener = core
            .add_listener_local()
            .info(move |info| {
                if let Some(rc) = w.upgrade() {
                    let mut b = rc.borrow_mut();
                    b.store.core_name = info.name().to_string();
                    b.store.core_version = info.version().to_string();
                    b.mark_dirty();
                }
            })
            .done(move |_id, seq| {
                if let Some(rc) = w2.upgrade() {
                    rc.borrow_mut().done(seq.seq());
                }
            })
            .error(move |id, _seq, res, message| {
                if let Some(rc) = w3.upgrade() {
                    rc.borrow_mut().error(id, res, message);
                }
            })
            .register();
        let w = b.weak.clone();
        let w2 = b.weak.clone();
        let registry_listener = registry
            .add_listener_local()
            .global(move |global| {
                if let Some(rc) = w.upgrade() {
                    rc.borrow_mut().global(global);
                }
            })
            .global_remove(move |id| {
                if let Some(rc) = w2.upgrade() {
                    let removed = {
                        let mut b = rc.borrow_mut();
                        b.store.remove(id);
                        b.mark_dirty();
                        b.conn.as_mut().and_then(|c| c.objects.remove(&id))
                    };
                    drop(removed);
                }
            })
            .register();
        let init = core.sync(0).map(|s| s.seq()).unwrap_or(0);
        b.store = Store::default();
        b.ready = false;
        b.connection = Connection::Connecting;
        b.conn = Some(Conn {
            meters: HashMap::new(),
            objects: HashMap::new(),
            created: Vec::new(),
            param_flags: HashMap::new(),
            enum_syncs: HashMap::new(),
            next_seq: 1,
            init_sync: Some((init, 0)),
            pending: HashMap::new(),
            _registry_listener: registry_listener,
            registry,
            _core_listener: core_listener,
            core,
        });
        b.mark_dirty();
    }

    /// The daemon went away (or never answered): show it and retry with backoff.
    fn lost(&mut self, why: String) {
        self.connection = Connection::Lost(why);
        self.ready = false;
        self.store = Store::default();
        self.reconnect.update_timer(Some(self.backoff), None);
        self.backoff = (self.backoff * 2).min(Duration::from_secs(5));
        self.mark_dirty();
    }

    fn reconnect(rc: &Rc<RefCell<Backend>>) {
        // Drop the old connection outside the borrow: tearing down proxies can emit events.
        let old = rc.borrow_mut().conn.take();
        drop(old);
        Backend::connect(rc);
    }

    fn done(&mut self, seq: i32) {
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        if let Some(list) = conn.enum_syncs.remove(&seq) {
            for (object, id) in list {
                self.store.param_done(object, id);
            }
            self.dirty = true;
        }
        if let Some((init, round)) = conn.init_sync
            && init == seq
        {
            // Round 0 lists the globals, round 1 their info, round 2 the params info asked for.
            if round < 2 {
                conn.init_sync = conn.core.sync(0).ok().map(|s| (s.seq(), round + 1));
            } else {
                conn.init_sync = None;
            }
        }
        if !self.ready && conn.init_sync.is_none() && conn.enum_syncs.is_empty() {
            self.ready = true;
            self.connection = Connection::Connected;
            self.backoff = Duration::from_millis(250);
            self.dirty = true;
            if !self.store.has_session_manager() {
                self.issue(
                    Severity::Warning,
                    crate::i18n::t("No session manager is running (WirePlumber)").into(),
                );
            }
        }
        if self.dirty {
            self.mark_dirty();
        }
    }

    fn error(&mut self, id: u32, res: i32, message: &str) {
        if id == pw::sys::PW_ID_CORE {
            if res == -libc::EPIPE {
                self.lost(crate::i18n::detail(
                    crate::i18n::t("PipeWire went away"),
                    message,
                ));
            } else {
                self.issue(Severity::Error, crate::i18n::detail("PipeWire", message));
            }
            return;
        }
        // Errors for objects we didn't ask anything of (a global that vanished mid-bind) are
        // nothing to tell the user.
        if let Some(what) = self.conn.as_mut().and_then(|c| c.pending.remove(&id)) {
            self.issue(Severity::Error, crate::i18n::detail(&what, message));
        }
    }

    fn issue(&mut self, severity: Severity, message: String) {
        if !self.issues.iter().any(|i| i.message == message) {
            self.issues.push(Issue { severity, message });
        }
        self.mark_dirty();
    }

    // ---- globals ---------------------------------------------------------------------------

    fn global(&mut self, g: &GlobalObject<&DictRef>) {
        let id = g.id;
        let props = props_of(g.props);
        let weak = self.weak.clone();
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        let bound = match g.type_ {
            ObjectType::Node => {
                let Ok(node) = conn.registry.bind::<pw::node::Node, _>(g) else {
                    return;
                };
                let (w, w2) = (weak.clone(), weak.clone());
                let l = node
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(rc) = w.upgrade() {
                            let state = match info.state() {
                                pw::node::NodeState::Running => RunState::Running,
                                pw::node::NodeState::Idle => RunState::Idle,
                                pw::node::NodeState::Suspended => RunState::Suspended,
                                pw::node::NodeState::Creating => RunState::Creating,
                                pw::node::NodeState::Error(_) => RunState::Error,
                            };
                            let mut b = rc.borrow_mut();
                            let node = b.store.nodes.entry(id).or_default();
                            node.state = state;
                            // Props come only with the PROPS change bit; otherwise they're empty.
                            if info.change_mask().contains(pw::node::NodeChangeMask::PROPS) {
                                node.props = props_of(info.props());
                            }
                            let changed = b.param_changes(id, info.params(), &[params::PROPS]);
                            b.enumerate(id, &changed);
                            b.mark_dirty();
                        }
                    })
                    .param(move |seq, pid, _, _, pod| {
                        if let Some(rc) = w2.upgrade() {
                            rc.borrow_mut().param(id, pid, seq, pod);
                        }
                    })
                    .register();
                self.store.nodes.insert(
                    id,
                    NodeG {
                        props,
                        state: RunState::Creating,
                    },
                );
                Bound::Node(l, node)
            }
            ObjectType::Device => {
                let Ok(device) = conn.registry.bind::<pw::device::Device, _>(g) else {
                    return;
                };
                let (w, w2) = (weak.clone(), weak.clone());
                let l = device
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(rc) = w.upgrade() {
                            let mut b = rc.borrow_mut();
                            if info
                                .change_mask()
                                .contains(pw::device::DeviceChangeMask::PROPS)
                            {
                                b.store.devices.insert(id, props_of(info.props()));
                            }
                            let changed = b.param_changes(
                                id,
                                info.params(),
                                &[
                                    params::ENUM_PROFILE,
                                    params::PROFILE,
                                    params::ENUM_ROUTE,
                                    params::ROUTE,
                                ],
                            );
                            b.enumerate(id, &changed);
                            b.mark_dirty();
                        }
                    })
                    .param(move |seq, pid, _, _, pod| {
                        if let Some(rc) = w2.upgrade() {
                            rc.borrow_mut().param(id, pid, seq, pod);
                        }
                    })
                    .register();
                self.store.devices.insert(id, props);
                Bound::Device(l, device)
            }
            ObjectType::Port => {
                let Ok(port) = conn.registry.bind::<pw::port::Port, _>(g) else {
                    return;
                };
                let w = weak.clone();
                let l = port
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(rc) = w.upgrade() {
                            let mut b = rc.borrow_mut();
                            let port = b.store.ports.entry(id).or_default();
                            port.output = info.direction() == pw::spa::utils::Direction::Output;
                            if info.change_mask().contains(pw::port::PortChangeMask::PROPS) {
                                port.props = props_of(info.props());
                            }
                            b.mark_dirty();
                        }
                    })
                    .register();
                self.store.ports.insert(
                    id,
                    PortG {
                        output: props.get("port.direction").is_some_and(|d| d == "out"),
                        props,
                    },
                );
                Bound::Port(l, port)
            }
            ObjectType::Link => {
                let Ok(link) = conn.registry.bind::<pw::link::Link, _>(g) else {
                    return;
                };
                let w = weak.clone();
                let l = link
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(rc) = w.upgrade() {
                            let mut b = rc.borrow_mut();
                            b.store.links.insert(
                                id,
                                LinkG {
                                    output_node: info.output_node_id(),
                                    output_port: info.output_port_id(),
                                    input_node: info.input_node_id(),
                                    input_port: info.input_port_id(),
                                    active: matches!(info.state(), pw::link::LinkState::Active),
                                },
                            );
                            b.mark_dirty();
                        }
                    })
                    .register();
                let num = |k: &str| props.get(k).and_then(|v| v.parse().ok()).unwrap_or(0);
                self.store.links.insert(
                    id,
                    LinkG {
                        output_node: num("link.output.node"),
                        output_port: num("link.output.port"),
                        input_node: num("link.input.node"),
                        input_port: num("link.input.port"),
                        active: false,
                    },
                );
                Bound::Link(l, link)
            }
            ObjectType::Client => {
                let Ok(client) = conn.registry.bind::<pw::client::Client, _>(g) else {
                    return;
                };
                let w = weak.clone();
                let l = client
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(rc) = w.upgrade() {
                            let mut b = rc.borrow_mut();
                            if info
                                .change_mask()
                                .contains(pw::client::ClientChangeMask::PROPS)
                            {
                                b.store.clients.insert(id, props_of(info.props()));
                            }
                            b.mark_dirty();
                        }
                    })
                    .register();
                self.store.clients.insert(id, props);
                Bound::Client(l, client)
            }
            ObjectType::Metadata => {
                let name = props.get("metadata.name").cloned().unwrap_or_default();
                if !matches!(name.as_str(), "default" | "settings") {
                    return;
                }
                let Ok(metadata) = conn.registry.bind::<pw::metadata::Metadata, _>(g) else {
                    return;
                };
                let w = weak.clone();
                let l = metadata
                    .add_listener_local()
                    .property(move |subject, key, _type, value| {
                        if let Some(rc) = w.upgrade() {
                            let mut b = rc.borrow_mut();
                            if let Some(m) = b.store.metadata.get_mut(&id) {
                                match (key, value) {
                                    (None, _) => m.entries.retain(|(s, _), _| *s != subject),
                                    (Some(k), None) => {
                                        m.entries.remove(&(subject, k.to_string()));
                                    }
                                    (Some(k), Some(v)) => {
                                        m.entries.insert((subject, k.to_string()), v.to_string());
                                    }
                                }
                            }
                            b.mark_dirty();
                        }
                        0
                    })
                    .register();
                self.store.metadata.insert(
                    id,
                    MetadataG {
                        name,
                        entries: Default::default(),
                    },
                );
                Bound::Metadata(l, metadata)
            }
            _ => return,
        };
        if let Some(conn) = self.conn.as_mut() {
            conn.objects.insert(id, bound);
        }
        self.mark_dirty();
    }

    /// The watched param ids whose flags changed since the last info (all of them the first
    /// time): PipeWire toggles a param's serial flag whenever it changes.
    fn param_changes(
        &mut self,
        object: Id,
        infos: &[pw::spa::param::ParamInfo],
        watch: &[u32],
    ) -> Vec<u32> {
        let Some(conn) = self.conn.as_mut() else {
            return Vec::new();
        };
        let mut changed = Vec::new();
        for info in infos {
            let id = info.id().as_raw();
            if !watch.contains(&id) || !info.flags().contains(pw::spa::param::ParamInfoFlags::READ)
            {
                continue;
            }
            let flags = info.flags().bits();
            if conn.param_flags.insert((object, id), flags) != Some(flags) {
                changed.push(id);
            }
        }
        changed
    }

    /// Asks for fresh enumerations of `ids` on `object`, closed by one core sync.
    fn enumerate(&mut self, object: Id, ids: &[u32]) {
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let mut list = Vec::new();
        for &id in ids {
            conn.next_seq += 1;
            let seq = conn.next_seq;
            match conn.objects.get(&object) {
                Some(Bound::Node(_, n)) => {
                    n.enum_params(seq, Some(ParamType::from_raw(id)), 0, u32::MAX)
                }
                Some(Bound::Device(_, d)) => {
                    d.enum_params(seq, Some(ParamType::from_raw(id)), 0, u32::MAX)
                }
                _ => continue,
            }
            self.store.param_request(object, id);
            list.push((object, id));
        }
        if let Ok(sync) = conn.core.sync(0) {
            conn.enum_syncs.insert(sync.seq(), list);
        }
    }

    fn param(&mut self, object: Id, id: ParamType, seq: i32, pod: Option<&Pod>) {
        let id = id.as_raw();
        let parsed: Option<Param> = pod.and_then(|p| params::parse(id, p));
        self.store.param(object, id, seq, parsed);
        self.mark_dirty();
    }

    // ---- publishing ------------------------------------------------------------------------

    fn mark_dirty(&mut self) {
        self.dirty = true;
        if !self.armed {
            let wait = FRAME
                .saturating_sub(self.last_publish.elapsed())
                .max(Duration::from_millis(1));
            self.publish.update_timer(Some(wait), None);
            self.armed = true;
        }
    }

    fn tick(&mut self) {
        self.armed = false;
        self.sync_meters();
        let metering = self.conn.as_ref().is_some_and(|c| !c.meters.is_empty());
        if self.dirty || metering {
            self.dirty = false;
            self.publish_now();
        }
        if metering {
            self.publish.update_timer(Some(FRAME), None);
            self.armed = true;
        }
    }

    fn publish_now(&mut self) {
        let levels: HashMap<Target, f32> = match self.conn.as_mut() {
            Some(c) => c
                .meters
                .iter_mut()
                .map(|(t, m)| (*t, m.take_level()))
                .collect(),
            None => HashMap::new(),
        };
        let mut state = if self.ready {
            store::build(&self.store, &levels)
        } else {
            State::default()
        };
        state.connection = self.connection.clone();
        state.issues = self.issues.clone();
        self.last_publish = Instant::now();
        self.last = state.clone();
        self.driver.publish(state);
    }

    /// Starts meters for metered targets that exist and stops the rest.
    fn sync_meters(&mut self) {
        let Some(conn) = self.conn.as_mut() else {
            return;
        };
        let mut wanted: HashMap<Target, (Id, bool)> = HashMap::new();
        if self.ready {
            let state = store::build(&self.store, &HashMap::new());
            for t in &self.metering {
                let node = match *t {
                    Target::Device(d) => state
                        .device(d)
                        .map(|d| (d.id, d.kind == model::DeviceKind::Sink)),
                    Target::Stream(s) => state.stream(s).and_then(|s| match s.kind {
                        model::StreamKind::Playback => Some((s.id, false)),
                        // What a recording hears: its source.
                        model::StreamKind::Record => s.target_device.map(|d| (d, false)),
                    }),
                };
                if let Some(n) = node {
                    wanted.insert(*t, n);
                }
            }
        }
        conn.meters
            .retain(|t, m| wanted.get(t).is_some_and(|(n, _)| *n == m.node));
        for (t, (node, sink)) in wanted {
            if conn.meters.contains_key(&t) {
                continue;
            }
            let Some(serial) = self
                .store
                .nodes
                .get(&node)
                .and_then(|n| n.props.get("object.serial"))
                .cloned()
            else {
                continue;
            };
            match meter::Meter::new(&conn.core, node, &serial, sink) {
                Ok(m) => {
                    conn.meters.insert(t, m);
                }
                Err(e) => eprintln!("sound: meter for {node}: {e}"),
            }
        }
    }

    // ---- commands --------------------------------------------------------------------------

    fn command(&mut self, command: Command) {
        if let Command::DismissIssue { index } = command {
            if index < self.issues.len() {
                self.issues.remove(index);
                self.mark_dirty();
            }
            return;
        }
        if let Command::SetMetering { targets, on } = command {
            for t in targets {
                if on {
                    self.metering.insert(t);
                } else {
                    self.metering.remove(&t);
                }
            }
            self.mark_dirty();
            return;
        }
        if !self.ready {
            self.issue(
                Severity::Error,
                crate::i18n::t("Not connected to PipeWire").into(),
            );
            return;
        }
        if let Err(e) = self.apply(command) {
            self.issue(Severity::Error, e);
        }
    }

    fn apply(&mut self, command: Command) -> Result<(), String> {
        match command {
            Command::SetVolume { target, volume } => {
                let v = self.volume_of(target)?.with_combined(volume);
                self.write_volume(target, Some(v), None)
            }
            Command::SetChannelVolumes { target, volume } => {
                self.write_volume(target, Some(volume), None)
            }
            Command::SetBalance { target, balance } => {
                let v = self.volume_of(target)?.with_balance(balance);
                self.write_volume(target, Some(v), None)
            }
            Command::SetMute { target, mute } => self.write_volume(target, None, Some(mute)),
            Command::SetDefault { device } => {
                let d = self
                    .last
                    .device(device)
                    .ok_or(crate::i18n::t("That device is gone"))?;
                let key = match d.kind {
                    model::DeviceKind::Sink => "default.configured.audio.sink",
                    model::DeviceKind::Source => "default.configured.audio.source",
                    model::DeviceKind::Monitor => {
                        return Err(crate::i18n::t("A monitor can't be the default").into());
                    }
                };
                let value = serde_json::json!({ "name": d.name }).to_string();
                self.set_metadata(
                    "default",
                    0,
                    key,
                    Some("Spa:String:JSON"),
                    Some(&value),
                    crate::i18n::t("Couldn't change the default device"),
                )
            }
            Command::SetPort { device, port } => {
                let (card, dev) = self
                    .store
                    .node_card(device)
                    .ok_or(crate::i18n::t("That device has no ports"))?;
                let profile = self.store.active_profile(card).map(|p| p.index);
                let route = self
                    .store
                    .enum_routes(card)
                    .find(|r| {
                        r.name == port
                            && (r.devices.is_empty() || r.devices.contains(&dev))
                            && profile
                                .is_none_or(|p| r.profiles.is_empty() || r.profiles.contains(&p))
                    })
                    .ok_or(crate::i18n::t("That port isn't available"))?;
                let pod = params::route_pod(route.index, dev, None);
                self.set_device_param(
                    card,
                    params::ROUTE,
                    &pod,
                    crate::i18n::t("Couldn't switch the port"),
                )
            }
            Command::SetProfile { card, profile } => {
                let p = self
                    .store
                    .enum_profiles(card)
                    .find(|p| p.name == profile)
                    .ok_or(crate::i18n::t("That profile isn't available"))?;
                let pod = params::profile_pod(p.index);
                let what = match store::codec(&p.name, &p.description) {
                    Some(c) => crate::i18n::failed("Couldn't switch to {}", &[&c]),
                    None => crate::i18n::t("Couldn't switch the profile").into(),
                };
                self.set_device_param(card, params::PROFILE, &pod, &what)
            }
            Command::MoveStream { stream, device } => {
                self.last
                    .stream(stream)
                    .ok_or(crate::i18n::t("That app is gone"))?;
                let serial = self
                    .store
                    .nodes
                    .get(&device)
                    .and_then(|n| n.props.get("object.serial"))
                    .cloned()
                    .ok_or(crate::i18n::t("That device is gone"))?;
                // As pipewire-pulse does: WirePlumber follows target.object (and remembers it).
                self.set_metadata(
                    "default",
                    stream,
                    "target.object",
                    Some("Spa:Id"),
                    Some(&serial),
                    crate::i18n::t("Couldn't move the app"),
                )?;
                self.set_metadata(
                    "default",
                    stream,
                    "target.node",
                    Some("Spa:Id"),
                    Some(&device.to_string()),
                    crate::i18n::t("Couldn't move the app"),
                )
            }
            Command::CreateVirtualSink { name } => {
                let slug: String = name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() {
                            c.to_ascii_lowercase()
                        } else {
                            '-'
                        }
                    })
                    .collect();
                let node_name = format!("{}{}", store::VIRTUAL_PREFIX, slug.trim_matches('-'));
                if self.store.node_by_name(&node_name).is_some() {
                    return Err(crate::i18n::failed(
                        "There's already a device called {}",
                        &[&name],
                    ));
                }
                let conn = self
                    .conn
                    .as_mut()
                    .ok_or(crate::i18n::t("Not connected to PipeWire"))?;
                let props = properties! {
                    "factory.name" => "support.null-audio-sink",
                    "node.name" => node_name.as_str(),
                    "node.description" => name.as_str(),
                    "media.class" => "Audio/Sink",
                    "audio.position" => "[ FL FR ]",
                    "monitor.channel-volumes" => "true",
                    "monitor.passthrough" => "true",
                    // Outlives this app, until PipeWire restarts or it's removed.
                    "object.linger" => "true",
                    store::OURS_KEY => "true",
                };
                let node: pw::node::Node =
                    conn.core.create_object("adapter", &props).map_err(|e| {
                        crate::i18n::failed("Couldn't create {}: {}", &[&name, &e.to_string()])
                    })?;
                let proxy = node.upcast();
                conn.pending.insert(
                    proxy.id(),
                    crate::i18n::failed("Couldn't create {}", &[&name]),
                );
                conn.created.push(proxy);
                Ok(())
            }
            Command::RemoveVirtualSink { device } => {
                let d = self
                    .last
                    .device(device)
                    .ok_or(crate::i18n::t("That device is gone"))?;
                if !d.is_ours {
                    return Err(crate::i18n::t("Only devices made here can be removed").into());
                }
                let conn = self
                    .conn
                    .as_ref()
                    .ok_or(crate::i18n::t("Not connected to PipeWire"))?;
                conn.registry
                    .destroy_global(device)
                    .into_result()
                    .map(|_| ())
                    .map_err(|e| {
                        crate::i18n::failed(
                            "Couldn't remove {}: {}",
                            &[&d.description, &e.to_string()],
                        )
                    })
            }
            Command::SetRateQuantum { rate, quantum } => {
                let r = rate.unwrap_or(0).to_string();
                let q = quantum.unwrap_or(0).to_string();
                self.set_metadata(
                    "settings",
                    0,
                    "clock.force-rate",
                    None,
                    Some(&r),
                    crate::i18n::t("Couldn't change the sample rate"),
                )?;
                self.set_metadata(
                    "settings",
                    0,
                    "clock.force-quantum",
                    None,
                    Some(&q),
                    crate::i18n::t("Couldn't change the buffer size"),
                )
            }
            Command::Link {
                output_port,
                input_port,
            } => {
                let node_of = |p: Id| {
                    self.store
                        .ports
                        .get(&p)
                        .and_then(|p| p.props.get("node.id"))
                        .cloned()
                };
                let (out_node, in_node) = (
                    node_of(output_port).ok_or(crate::i18n::t("That port is gone"))?,
                    node_of(input_port).ok_or(crate::i18n::t("That port is gone"))?,
                );
                let conn = self
                    .conn
                    .as_mut()
                    .ok_or(crate::i18n::t("Not connected to PipeWire"))?;
                let props = properties! {
                    "link.output.node" => out_node.as_str(),
                    "link.output.port" => output_port.to_string(),
                    "link.input.node" => in_node.as_str(),
                    "link.input.port" => input_port.to_string(),
                    "object.linger" => "true",
                };
                let link: pw::link::Link = conn
                    .core
                    .create_object("link-factory", &props)
                    .map_err(|e| {
                        crate::i18n::failed("Couldn't connect those: {}", &[&e.to_string()])
                    })?;
                let proxy = link.upcast();
                conn.pending
                    .insert(proxy.id(), crate::i18n::t("Couldn't connect those").into());
                conn.created.push(proxy);
                Ok(())
            }
            Command::Unlink { link } => {
                let conn = self
                    .conn
                    .as_ref()
                    .ok_or(crate::i18n::t("Not connected to PipeWire"))?;
                conn.registry
                    .destroy_global(link)
                    .into_result()
                    .map(|_| ())
                    .map_err(|e| {
                        crate::i18n::failed("Couldn't disconnect those: {}", &[&e.to_string()])
                    })
            }
            Command::SetMetering { .. } | Command::DismissIssue { .. } => Ok(()),
        }
    }

    fn volume_of(&self, target: Target) -> Result<Volume, String> {
        match target {
            Target::Device(d) => self
                .last
                .device(d)
                .map(|d| d.volume.clone())
                .ok_or_else(|| crate::i18n::t("That device is gone").into()),
            Target::Stream(s) => self
                .last
                .stream(s)
                .map(|s| s.volume.clone())
                .ok_or_else(|| crate::i18n::t("That app is gone").into()),
        }
    }

    /// Writes volume and/or mute where WirePlumber keeps them: the card's active route for
    /// hardware, the node's props otherwise.
    fn write_volume(
        &mut self,
        target: Target,
        volume: Option<Volume>,
        mute: Option<bool>,
    ) -> Result<(), String> {
        let (node, what) = match target {
            Target::Device(d) => (
                d,
                if volume.is_some() {
                    crate::i18n::t("Couldn't change the volume")
                } else {
                    crate::i18n::t("Couldn't mute")
                },
            ),
            Target::Stream(s) => (
                s,
                if volume.is_some() {
                    crate::i18n::t("Couldn't change the app's volume")
                } else {
                    crate::i18n::t("Couldn't mute the app")
                },
            ),
        };
        let current = self.volume_of(target)?;
        let linear = volume.map(|v| store::linear_channels(&current, &v));
        if let Target::Device(_) = target
            && let Some((card, route)) = self.store.node_route(node)
            && route.props.as_ref().is_some_and(|p| !p.volumes.is_empty())
        {
            let n = route.props.as_ref().map_or(0, |p| p.volumes.len());
            let linear = linear.map(|mut l| {
                l.resize(n, l.last().copied().unwrap_or(0.0));
                l
            });
            let pod = params::route_pod(
                route.index,
                route.device.unwrap_or(0),
                Some(params::props_object(linear.as_deref(), mute)),
            );
            return self.set_device_param(card, params::ROUTE, &pod, what);
        }
        let pod = params::props_pod(linear.as_deref(), mute);
        let conn = self
            .conn
            .as_mut()
            .ok_or(crate::i18n::t("Not connected to PipeWire"))?;
        let Some(Bound::Node(_, n)) = conn.objects.get(&node) else {
            return Err(crate::i18n::t("That device is gone").into());
        };
        n.set_param(
            ParamType::Props,
            0,
            Pod::from_bytes(&pod).ok_or(crate::i18n::t("bad pod"))?,
        );
        conn.pending.insert(n.upcast_ref().id(), what.into());
        Ok(())
    }

    fn set_device_param(
        &mut self,
        card: Id,
        id: u32,
        pod: &[u8],
        what: &str,
    ) -> Result<(), String> {
        let conn = self
            .conn
            .as_mut()
            .ok_or(crate::i18n::t("Not connected to PipeWire"))?;
        let Some(Bound::Device(_, d)) = conn.objects.get(&card) else {
            return Err(crate::i18n::t("That card is gone").into());
        };
        d.set_param(
            ParamType::from_raw(id),
            0,
            Pod::from_bytes(pod).ok_or(crate::i18n::t("bad pod"))?,
        );
        conn.pending.insert(d.upcast_ref().id(), what.into());
        Ok(())
    }

    fn set_metadata(
        &mut self,
        name: &str,
        subject: u32,
        key: &str,
        type_: Option<&str>,
        value: Option<&str>,
        what: &str,
    ) -> Result<(), String> {
        let (id, _) = self.store.metadata_named(name).ok_or(crate::i18n::t(
            "No session manager is running (WirePlumber)",
        ))?;
        let conn = self
            .conn
            .as_mut()
            .ok_or(crate::i18n::t("Not connected to PipeWire"))?;
        let Some(Bound::Metadata(_, m)) = conn.objects.get(&id) else {
            return Err(crate::i18n::t("No session manager is running (WirePlumber)").into());
        };
        m.set_property(subject, key, type_, value);
        conn.pending.insert(m.upcast_ref().id(), what.into());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disconnect (what a PipeWire restart looks like) drops the picture, shows Lost and
    /// arms the reconnect with growing backoff; the next ready connection resets it.
    #[test]
    fn disconnect_shows_lost_and_backs_off() {
        pw::init();
        let main_loop: &'static MainLoopRc = Box::leak(Box::new(MainLoopRc::new(None).unwrap()));
        let context = ContextRc::new(main_loop, None).unwrap();
        let lp = main_loop.loop_();
        let (audio, driver) = model::channel();
        let mut b = Backend {
            weak: Weak::new(),
            driver,
            context,
            conn: None,
            store: Store::default(),
            issues: Vec::new(),
            connection: Connection::Connected,
            ready: true,
            last: State::default(),
            dirty: false,
            armed: false,
            last_publish: Instant::now() - FRAME,
            publish: lp.add_timer(|_| {}),
            reconnect: lp.add_timer(|_| {}),
            backoff: Duration::from_millis(250),
            metering: HashSet::new(),
        };
        b.store.nodes.insert(1, NodeG::default());
        b.error(pw::sys::PW_ID_CORE, -libc::EPIPE, "connection reset");
        assert!(matches!(b.connection, Connection::Lost(_)));
        assert!(!b.ready && b.store.nodes.is_empty());
        assert_eq!(b.backoff, Duration::from_millis(500));
        b.lost("again".into());
        b.lost("again".into());
        b.lost("again".into());
        b.lost("again".into());
        assert_eq!(b.backoff, Duration::from_secs(5), "capped");
        b.publish_now();
        assert!(matches!(audio.state().connection, Connection::Lost(_)));
        assert!(audio.state().devices.is_empty());
    }
}
