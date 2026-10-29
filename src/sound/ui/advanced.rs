//! The third level: every device by card with its profile, virtual outputs, a patchbay of the
//! graph, and the clock.

use std::collections::HashMap;

use rune::{Cursor, El, LineCap, Path, Shape, TextEdit, Tx, canvas, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, SegmentedControl, Select, TextInput};

use super::{Host, Page, Tab, Ui, glyphs};
use crate::sound::model::{Command, Device, DeviceKind, Direction, Id, Profile, ProfileKind, State};

const TABS: [(Tab, &str); 3] = [(Tab::Devices, "Devices"), (Tab::Routing, "Routing"), (Tab::Clock, "Clock")];

fn select(s: &Ui, k: &Kit, key: String, options: Vec<(String, String)>, current: Option<&str>, width: f32, choose: impl Fn(&mut Ui, &str) + Clone + 'static, layer: &mut Vec<El>) -> El {
    let labels: Vec<String> = options.iter().map(|o| o.0.clone()).collect();
    let shown = options.iter().find(|o| Some(o.1.as_str()) == current).map(|o| o.0.clone());
    let (k1, k2) = (key.clone(), key.clone());
    Select::new(key.clone(), &labels)
        .width(width)
        .current(shown.as_deref())
        .open(s.menu.as_deref() == Some(key.as_str()))
        .on_toggle(move |h: &mut Host| { let s = &mut h.sound; s.menu = if s.menu.as_deref() == Some(k1.as_str()) { None } else { Some(k1.clone()) } })
        .on_close(move |h: &mut Host| {
            let s = &mut h.sound;
            if s.menu.as_deref() == Some(k2.as_str()) {
                s.menu = None
            }
        })
        .on_choose(move |h: &mut Host, v: &str, _: &mut Tx| {
            let s = &mut h.sound;
            if let Some(o) = options.iter().find(|o| o.0 == v) {
                choose(s, &o.1);
            }
            s.menu = None;
        })
        .build(k, layer)
}

fn section(k: &Kit, key: String, rows: Vec<El>) -> El {
    col().key(key).w_full().padding(4.0).radius(12.0).fill(k.c.bg_base).border(1.0, k.c.border_muted).children(rows)
}

fn heading(k: &Kit, key: &str, title: &str, trailing: Option<El>) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .h(28.0)
        .pl(12.0)
        .center_y()
        .child(text(title.to_owned()).key("title").font(k.ty.small_medium).color(k.c.text_muted).grow(1.0))
        .children(trailing)
}

/// A profile in a few words: Bluetooth ones by codec ("LDAC", "Headset · mSBC").
pub fn profile_label(p: &Profile) -> String {
    match (&p.codec, p.kind) {
        (Some(codec), ProfileKind::Duplex) => format!("Headset · {codec}"),
        (Some(codec), _) => codec.clone(),
        (None, _) => p.description.clone(),
    }
}

fn node_row(s: &Ui, k: &Kit, d: &Device) -> El {
    let c = &k.c;
    let default = match d.kind {
        DeviceKind::Sink => s.audio.default_sink == Some(d.id),
        DeviceKind::Source => s.audio.default_source == Some(d.id),
        DeviceKind::Monitor => false,
    };
    let id = d.id;
    let quiet = d.hidden || d.kind == DeviceKind::Monitor;
    let mut r = row()
        .key(format!("node-{id}"))
        .w_full()
        .h(30.0)
        .pl(12.0)
        .pr(6.0)
        .gap(10.0)
        .radius(8.0)
        .center_y()
        .child(icon(glyphs::device(d), 14.0).key("glyph").color(if quiet { c.icon_faint } else { c.icon_muted }))
        .child(text(d.description.clone()).key("name").font(k.ty.ui).color(if quiet { c.text_faint } else { c.text_base }).grow(1.0).shrink().w(0.0).truncate())
        .child(icon(&glyphs::CHECK, 14.0).key("default").color(c.icon_base).opacity(if default { 1.0 } else { 0.0 }));
    if d.kind != DeviceKind::Monitor {
        r = r.role("radio").label(d.description.clone()).hover(c.hover).cursor(Cursor::Pointer).on_press(move |h: &mut Host| h.sound.send(Command::SetDefault { device: id }));
    }
    if d.is_ours {
        r = r.child(IconButton::new(&glyphs::CLOSE).variant(IconButtonVariant::GhostMuted).label("Remove").on_press(move |h: &mut Host| h.sound.send(Command::RemoveVirtualSink { device: id })).build(k).key("remove"));
    }
    r
}

fn devices(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let c = &k.c;
    let mut out = Vec::new();
    for card in &s.audio.cards {
        let options: Vec<(String, String)> = card.profiles.iter().filter(|p| p.available || Some(&p.name) == card.active_profile.as_ref()).map(|p| (profile_label(p), p.name.clone())).collect();
        let id = card.id;
        let profile = select(s, k, format!("profile-{id}"), options, card.active_profile.as_deref(), 170.0, move |s: &mut Ui, v: &str| s.send(Command::SetProfile { card: id, profile: v.to_owned() }), layer);
        let head = row()
            .key("card")
            .w_full()
            .h(36.0)
            .pl(12.0)
            .pr(4.0)
            .gap(10.0)
            .center_y()
            .child(text(card.description.clone()).key("name").font(k.ty.ui_medium).color(c.text_base).grow(1.0).shrink().w(0.0).min_w(96.0).truncate())
            .child(profile);
        let mut rows = vec![head];
        rows.extend(s.audio.devices.iter().filter(|d| d.card_id == Some(card.id)).map(|d| node_row(s, k, d)));
        out.push(section(k, format!("card-{id}"), rows));
    }
    let add = IconButton::new(&glyphs::PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .label("New virtual output")
        .on_press(|h: &mut Host| h.sound.naming = Some(TextEdit::new("")))
        .build(k)
        .key("add");
    let mut rows = vec![heading(k, "virtual-head", "Virtual", Some(add))];
    rows.extend(s.audio.devices.iter().filter(|d| d.card_id.is_none()).map(|d| node_row(s, k, d)));
    if let Some(edit) = &s.naming {
        let field = TextInput::new(edit)
            .placeholder("Name")
            .width(None)
            .on_edit(|h: &mut Host| h.sound.naming.get_or_insert_with(|| TextEdit::new("")))
            .on_submit(|h: &mut Host, _: &mut Tx| {
                let s = &mut h.sound;
                let name = s.naming.take().map(|e| e.text.trim().to_owned()).unwrap_or_default();
                if !name.is_empty() {
                    s.send(Command::CreateVirtualSink { name });
                }
            })
            .build(k)
            .key("name-field");
        rows.push(row().key("naming").w_full().px(8.0).py(4.0).child(field.grow(1.0)));
    }
    out.push(section(k, "virtual".into(), rows));
    out
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) { format!("{} kHz", rate / 1000) } else { format!("{:.1} kHz", rate as f32 / 1000.0) }
}

fn setting(k: &Kit, key: &str, title: &str, control: El) -> El {
    row().key(key.to_owned()).w_full().h(44.0).px(12.0).gap(16.0).center_y().child(text(title.to_owned()).key("title").font(k.ty.ui).color(k.c.text_base).grow(1.0)).child(control)
}

fn clock(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let c = &k.c;
    let sv = &s.audio.server;
    let mut rates = sv.allowed_rates.clone();
    if rates.is_empty() {
        rates = vec![44100, 48000, 96000];
    }
    let mut rate_options = vec![("Automatic".to_owned(), "auto".to_owned())];
    rate_options.extend(rates.iter().map(|r| (khz(*r), r.to_string())));
    let forced_rate = sv.forced_rate.map(|r| r.to_string());
    let forced_quantum = sv.forced_quantum.map(|q| q.to_string());
    let (fq, fr) = (sv.forced_quantum, sv.forced_rate);
    let rate = select(s, k, "rate".into(), rate_options, Some(forced_rate.as_deref().unwrap_or("auto")), 150.0, move |s: &mut Ui, v: &str| s.send(Command::SetRateQuantum { rate: v.parse().ok(), quantum: fq }), layer);
    let lo = sv.min_quantum.max(16);
    let hi = sv.max_quantum.max(lo);
    let mut quantum_options = vec![("Automatic".to_owned(), "auto".to_owned())];
    let mut q = lo.next_power_of_two();
    while q <= hi {
        quantum_options.push((format!("{q} samples"), q.to_string()));
        q *= 2;
    }
    let quantum = select(s, k, "quantum".into(), quantum_options, Some(forced_quantum.as_deref().unwrap_or("auto")), 150.0, move |s: &mut Ui, v: &str| s.send(Command::SetRateQuantum { rate: fr, quantum: v.parse().ok() }), layer);
    let latency = if sv.rate > 0 { format!("{} · {} samples · {:.1} ms", khz(sv.rate), sv.quantum, sv.quantum as f32 * 1000.0 / sv.rate as f32) } else { String::new() };
    let about = if sv.version.is_empty() { String::new() } else { format!("PipeWire {}", sv.version) };
    vec![
        section(k, "clock".into(), vec![setting(k, "rate-row", "Sample rate", rate), el().key("sep").h(1.0).mx(12.0).fill(c.border_muted), setting(k, "quantum-row", "Buffer size", quantum)]),
        col().key("now").w_full().px(12.0).gap(4.0).child(text(latency).key("latency").font(k.ty.small.features("tnum")).color(c.text_muted)).child(text(about).key("version").font(k.ty.small).color(c.text_faint)),
    ]
}

/// Nodes with output ports (left) and input ports (right).
fn sides(a: &State) -> (Vec<Id>, Vec<Id>) {
    let g = &a.graph;
    let has = |n: Id, dir: Direction| g.ports.iter().any(|p| p.node == n && p.direction == dir && !(dir == Direction::Output && p.is_monitor));
    let left = g.nodes.iter().filter(|n| has(n.id, Direction::Output)).map(|n| n.id).collect();
    let right = g.nodes.iter().filter(|n| has(n.id, Direction::Input)).map(|n| n.id).collect();
    (left, right)
}

/// Links `from` to `to` channel by channel, or removes every link between them.
pub fn toggle_route(s: &mut Ui, from: Id, to: Id) {
    let g = &s.audio.graph;
    let existing: Vec<Id> = g.links.iter().filter(|l| l.output_node == from && l.input_node == to).map(|l| l.id).collect();
    if !existing.is_empty() {
        for link in existing {
            s.send(Command::Unlink { link });
        }
        return;
    }
    let outs: Vec<_> = g.ports.iter().filter(|p| p.node == from && p.direction == Direction::Output && !p.is_monitor).collect();
    let ins: Vec<_> = g.ports.iter().filter(|p| p.node == to && p.direction == Direction::Input).collect();
    let mut commands = Vec::new();
    for (i, o) in outs.iter().enumerate() {
        let target = ins.iter().find(|p| p.channel.is_some() && p.channel == o.channel).or_else(|| ins.get(i.min(ins.len().saturating_sub(1))));
        if let Some(p) = target {
            commands.push(Command::Link { output_port: o.id, input_port: p.id });
        }
    }
    for c in commands {
        s.send(c);
    }
}

const NODE: f32 = 28.0;
const NODE_GAP: f32 = 6.0;
const SIDE: f32 = 0.38;

fn routing(s: &Ui, k: &Kit) -> Vec<El> {
    let c = &k.c;
    let a = &s.audio;
    let (left, right) = sides(a);
    let name = |id: Id| a.graph.nodes.iter().find(|n| n.id == id).map_or_else(String::new, |n| n.name.clone());
    let y = |i: usize| i as f32 * (NODE + NODE_GAP) + NODE / 2.0;
    let rows = left.len().max(right.len());
    let h = rows as f32 * (NODE + NODE_GAP) - NODE_GAP;
    // Curves in a 100-wide box stretched over the patchbay.
    let mut pairs: HashMap<(Id, Id), bool> = HashMap::new();
    for l in &a.graph.links {
        let e = pairs.entry((l.output_node, l.input_node)).or_insert(false);
        *e |= l.active;
    }
    let mut shape = Shape::new();
    for ((from, to), active) in &pairs {
        let (Some(i), Some(j)) = (left.iter().position(|n| n == from), right.iter().position(|n| n == to)) else { continue };
        let picked = s.route_from == Some(*from);
        let color = if picked { c.text_base } else if *active { c.icon_muted } else { c.border_strong };
        let (x0, x1) = (SIDE * 100.0, (1.0 - SIDE) * 100.0);
        let mid = (x0 + x1) / 2.0;
        let path = Path::new().move_to([x0, y(i)]).cubic_to([mid, y(i)], [mid, y(j)], [x1, y(j)]);
        shape = shape.stroke_path(&path, 1.5, LineCap::Round, color);
    }
    let lines = canvas(shape.view_box(100.0, h.max(1.0))).key("links").absolute(0.0, 0.0).w_full().h(h.max(1.0));
    let pill = |key: String, id: Id, on_left: bool| {
        let picked = s.route_from == Some(id);
        let mut p = row()
            .key(key)
            .role("button")
            .label(name(id))
            .w_full()
            .h(NODE)
            .px(10.0)
            .radius(8.0)
            .center_y()
            .fill(c.bg_base)
            .border(1.0, if picked { c.border_strong } else { c.border_muted })
            .hover(c.hover)
            .cursor(Cursor::Pointer)
            .child(text(name(id)).key("name").font(k.ty.small).color(if picked { c.text_base } else { c.text_muted }).grow(1.0).shrink().w(0.0).truncate());
        p = if on_left {
            p.on_press(move |h: &mut Host| { let s = &mut h.sound; s.route_from = if s.route_from == Some(id) { None } else { Some(id) } })
        } else {
            p.on_press(move |h: &mut Host| {
                let s = &mut h.sound;
                if let Some(from) = s.route_from {
                    toggle_route(s, from, id);
                }
            })
        };
        p
    };
    let column = |key: &str, ids: &[Id], on_left: bool| col().key(key.to_owned()).w_frac(SIDE).gap(NODE_GAP).children(ids.iter().map(|id| pill(format!("{key}-{id}"), *id, on_left)));
    vec![
        row().key("patchbay").w_full().h(h.max(1.0)).justify_between().child(lines).child(column("from", &left, true)).child(column("to", &right, false)),
    ]
}

pub fn page(s: &Ui, k: &Kit, tab: Tab, layer: &mut Vec<El>) -> El {
    let labels: Vec<&str> = TABS.iter().map(|t| t.1).collect();
    let current = TABS.iter().position(|t| t.0 == tab);
    let head = row()
        .key("head")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            SegmentedControl::new(&labels, current)
                .width(None)
                .on_pick(|h: &mut Host, i: usize, tx: &mut Tx| {
                    let s = &mut h.sound;
                    tx.instant();
                    s.menu = None;
                    s.page = Page::Advanced(TABS[i].0);
                })
                .build(k)
                .key("tabs")
                .grow(1.0),
        );
    let content = match tab {
        Tab::Devices => devices(s, k, layer),
        Tab::Routing => routing(s, k),
        Tab::Clock => clock(s, k, layer),
    };
    let body = col().key(format!("tab-{}", current.unwrap_or(0))).w_full().px(18.0).pt(4.0).pb(24.0).gap(12.0).children(content);
    col().key("advanced").w_full().child(row().key("bar").w_full().px(12.0).py(12.0).child(head)).child(body)
}
