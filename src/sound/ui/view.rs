//! The first two levels: output, input and the apps playing; each row opens in place.

use rune::{Color, Cursor, El, Transition, Tx, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, Select, SegmentedControl};

use super::{Host, Mode, Row, Ui, glyphs, slider};
use crate::sound::model::{Availability, Command, Connection, Device, DeviceKind, ProfileKind, Stream, StreamKind, Target};

/// The leading column (mute button, app icon): the slider starts after it.
const LEAD: f32 = 28.0;
const GAP: f32 = 10.0;
/// The trailing column: an app row's disclosure chevron.
const TRAIL: f32 = 20.0;

fn pad(s: &Ui) -> f32 {
    if s.mode == Mode::Popover { 14.0 } else { 18.0 }
}

/// An app's letter on its colour, when it has no icon (as Hokum draws them).
fn tint(app: &str) -> Color {
    let h = app.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    rune::oklch(0.64, 0.13, (h % 360) as f32)
}

pub fn app_mark(s: &Ui, k: &Kit, st: &Stream, size: f32) -> El {
    match s.icon(st) {
        Some(img) => el().key("app-icon").size(size, size).radius(size * 0.25).pixels_fit(img.pixels(), rune::SurfaceFit::Fit),
        None => el().key("app-icon").size(size, size).radius(size * 0.27).center().fill(tint(&st.app_name)).child(
            text(st.app_name.chars().find(|c| c.is_alphanumeric()).unwrap_or('•').to_uppercase().to_string())
                .font(rune::TextStyle { size: (size * 0.55).round(), ..k.ty.ui_medium }.lh(size))
                .color(Color::WHITE),
        ),
    }
}

fn lead_button(k: &Kit, key: &str, label: &str, t: Target, child: El) -> El {
    el().key(key.to_owned())
        .role("button")
        .label(label.to_owned())
        .size(LEAD, LEAD)
        .radius(8.0)
        .center()
        .hover(k.c.hover)
        .press(k.c.pressed)
        .cursor(Cursor::Pointer)
        .on_press(move |h: &mut Host| h.sound.toggle_mute(t))
        .child(child)
}

/// `[lead] ━━━━●──── [trailing]`; every row keeps the trailing column so the tracks line up.
fn volume_line(s: &Ui, k: &Kit, key: String, lead: El, t: Target, level: Option<f32>, trailing: Option<El>) -> El {
    let v = s.shown(t);
    let dragging = s.slide.is_some_and(|d| d.target == t);
    row()
        .key(key)
        .w_full()
        .h(LEAD)
        .gap(GAP)
        .center_y()
        .hover_group()
        .child(lead)
        .child(slider::volume(k, t, v, s.scale(t, v), level, s.muted(t), dragging))
        .child(trailing.unwrap_or_else(|| el().key("trail").size(TRAIL, LEAD).ml(-6.0)))
}

fn chevron(k: &Kit, open: bool, size: f32) -> El {
    icon(&glyphs::CHEVRON, size).key("chevron").color(k.c.icon_faint).rotate(if open { std::f32::consts::PI } else { 0.0 })
}

/// Output or input: the device's name (opens the row), then mute and volume.
fn device_section(s: &Ui, k: &Kit, kind: DeviceKind, layer: &mut Vec<El>) -> Option<El> {
    let c = &k.c;
    let d = match kind {
        DeviceKind::Sink => s.audio.default_sink(),
        _ => s.audio.default_source(),
    }?;
    let r = if kind == DeviceKind::Sink { Row::Output } else { Row::Input };
    let open = s.open.contains(&r);
    let t = Target::Device(d.id);
    let key = if kind == DeviceKind::Sink { "output" } else { "input" };
    let name = row()
        .key("name")
        .role("button")
        .label(d.description.clone())
        .expanded(open)
        .ml(LEAD + GAP - 6.0)
        .px(6.0)
        .h(22.0)
        .gap(4.0)
        .radius(6.0)
        .center_y()
        .self_start()
        .hover(c.hover)
        .cursor(Cursor::Pointer)
        .on_press(move |h: &mut Host| h.sound.toggle(r))
        .child(text(d.description.clone()).key("label").font(k.ty.small_medium).color(c.text_muted).truncate())
        .child(chevron(k, open, 12.0));
    let glyph = glyphs::volume(d, s.shown(t));
    let lead = lead_button(k, "mute", "Mute", t, icon(glyph, 16.0).key("glyph").color(if d.mute { c.icon_faint } else { c.icon_base }));
    let mut section = col()
        .key(key)
        .w_full()
        .gap(2.0)
        .child(name)
        .child(volume_line(s, k, "line".into(), lead, t, d.level, None));
    if open {
        section = section.child(panel(k, format!("{key}-panel"), device_panel(s, k, d, layer)));
    }
    Some(section)
}

/// The inset card a row opens into, aligned with the sliders.
fn panel(k: &Kit, key: String, rows: Vec<El>) -> El {
    col()
        .key(key)
        .ml(LEAD + GAP - 8.0)
        .mt(6.0)
        .mb(4.0)
        .padding(4.0)
        .gap(2.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_muted)
        .children(rows)
        .transition(Transition::opacity().combined(Transition::offset(0.0, -4.0)))
}

fn hairline(k: &Kit, key: &str) -> El {
    el().key(key.to_owned()).h(1.0).mx(8.0).my(3.0).fill(k.c.border_muted)
}

/// One device to pick: icon, name, a check on the current one.
fn pick_row(k: &Kit, key: String, d: &Device, on: bool, extra: Option<String>, press: impl Fn(&mut Ui) + 'static) -> El {
    let c = &k.c;
    row()
        .key(key)
        .role("radio")
        .label(d.description.clone())
        .h(28.0)
        .px(8.0)
        .gap(10.0)
        .radius(6.0)
        .center_y()
        .hover(c.hover)
        .press(c.pressed)
        .cursor(Cursor::Pointer)
        .on_press(super::lift(press))
        .child(icon(glyphs::device(d), 14.0).key("glyph").color(if on { c.icon_base } else { c.icon_muted }))
        .child(text(d.description.clone()).key("name").font(k.ty.ui).color(if on { c.text_base } else { c.text_muted }).grow(1.0).shrink().w(0.0).truncate())
        .children(extra.map(|e| {
            row()
                .key("battery")
                .gap(4.0)
                .center_y()
                .child(icon(&glyphs::BATTERY, 14.0).key("glyph").color(c.icon_faint))
                .child(text(e).key("level").font(k.ty.small.features("tnum")).color(c.text_faint))
        }))
        .child(icon(&glyphs::CHECK, 14.0).key("check").color(c.icon_base).opacity(if on { 1.0 } else { 0.0 }))
}

fn detail_row(k: &Kit, key: &str, glyph: &'static rune::Icon, control: El) -> El {
    row()
        .key(key.to_owned())
        .min_h(32.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .child(el().key("lead").w(14.0).center().child(icon(glyph, 14.0).color(k.c.icon_faint)))
        .child(control)
}

fn battery(s: &Ui, d: &Device) -> Option<String> {
    let b = d.card_id.and_then(|c| s.audio.card(c))?.battery?;
    Some(format!("{}%", (b * 100.0).round()))
}

/// Devices to switch to, then the current one's port, Bluetooth codec and balance; under the
/// input, the apps recording.
fn device_panel(s: &Ui, k: &Kit, d: &Device, layer: &mut Vec<El>) -> Vec<El> {
    let mut rows: Vec<El> = s
        .audio
        .devices
        .iter()
        .filter(|o| o.kind == d.kind && !o.hidden)
        .map(|o| {
            let id = o.id;
            pick_row(k, format!("pick-{id}"), o, o.id == d.id, battery(s, o), move |s: &mut Ui| s.send(Command::SetDefault { device: id }))
        })
        .collect();
    let mut details = Vec::new();
    let available: Vec<_> = d.ports.iter().filter(|p| p.available != Availability::No || Some(&p.name) == d.active_port.as_ref()).collect();
    if available.len() > 1 {
        let labels: Vec<&str> = available.iter().map(|p| p.description.as_str()).collect();
        let current = available.iter().position(|p| Some(&p.name) == d.active_port.as_ref());
        let names: Vec<String> = available.iter().map(|p| p.name.clone()).collect();
        let id = d.id;
        details.push(detail_row(
            k,
            "port",
            &glyphs::JACK,
            SegmentedControl::new(&labels, current)
                .width(None)
                .on_pick(move |h: &mut Host, i: usize, _: &mut Tx| h.sound.send(Command::SetPort { device: id, port: names[i].clone() }))
                .build(k)
                .key("port-control")
                .grow(1.0),
        ));
    }
    if d.is_bluetooth()
        && let Some(card) = d.card_id.and_then(|c| s.audio.card(c))
    {
        let options: Vec<(String, String)> = card
            .profiles
            .iter()
            .filter(|p| p.available && p.kind != ProfileKind::Off)
            .map(|p| {
                (super::advanced::profile_label(p), p.name.clone())
            })
            .collect();
        if options.len() > 1 {
            let labels: Vec<String> = options.iter().map(|o| o.0.clone()).collect();
            let current = options.iter().find(|o| Some(&o.1) == card.active_profile.as_ref()).map(|o| o.0.clone());
            let key = format!("codec-{}", card.id);
            let card_id = card.id;
            let (k1, k2) = (key.clone(), key.clone());
            let select = Select::new(key.clone(), &labels)
                .width(150.0)
                .current(current.as_deref())
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
                        s.send(Command::SetProfile { card: card_id, profile: o.1.clone() });
                    }
                    s.menu = None;
                })
                .build(k, layer);
            details.push(detail_row(k, "codec", &glyphs::BLUETOOTH, row().key("codec-row").grow(1.0).child(select)));
        }
    }
    if d.volume.positions.len() >= 2 {
        details.push(balance_row(s, k, Target::Device(d.id)));
    }
    if d.kind == DeviceKind::Source {
        for st in s.audio.streams.iter().filter(|st| st.kind == StreamKind::Record) {
            let t = Target::Stream(st.id);
            let lead = lead_button(k, "mute", &st.app_name, t, muted_mark(s, k, st, 18.0));
            details.push(row().key(format!("rec-{}", st.id)).px(4.0).child(volume_line(s, k, "line".into(), lead, t, st.level, None)));
        }
    }
    if !details.is_empty() {
        rows.push(hairline(k, "sep"));
        rows.extend(details);
    }
    rows
}

fn balance_row(s: &Ui, k: &Kit, t: Target) -> El {
    let c = &k.c;
    let side = |l: &str| text(l.to_owned()).key(format!("side-{l}")).font(k.ty.small_medium).color(c.text_faint).w(14.0).align(rune::Align::Center);
    row()
        .key("balance-row")
        .h(32.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .child(side("L"))
        .child(slider::balance(k, t, s.balance_of(t)))
        .child(side("R"))
}

/// An app's icon, dimmed with a crossed speaker while muted.
fn muted_mark(s: &Ui, k: &Kit, st: &Stream, size: f32) -> El {
    let mark = app_mark(s, k, st, size).opacity(if st.mute { 0.35 } else { 1.0 });
    let mut e = el().key("mark").size(size, size).child(mark);
    if st.mute {
        e = e.child(
            el().key("muted")
                .pin(None, None, Some(-4.0), Some(-4.0))
                .size(13.0, 13.0)
                .radius(6.5)
                .center()
                .fill(k.c.page)
                .child(icon(&glyphs::SPEAKER_MUTED, 10.0).color(k.c.icon_base)),
        );
    }
    e
}

fn app_row(s: &Ui, k: &Kit, st: &Stream) -> El {
    let c = &k.c;
    let t = Target::Stream(st.id);
    let r = Row::Stream(st.id);
    let open = s.open.contains(&r);
    let lead = lead_button(k, "mute", &st.app_name, t, muted_mark(s, k, st, 20.0)).help(st.app_name.clone());
    let more = el()
        .key("more")
        .role("button")
        .label(format!("{} options", st.app_name))
        .expanded(open)
        .size(TRAIL, LEAD)
        .ml(-6.0)
        .radius(6.0)
        .center()
        .hover(c.hover)
        .cursor(Cursor::Pointer)
        .on_press(move |h: &mut Host| h.sound.toggle(r))
        .child(chevron(k, open, 14.0))
        .group_opacity(if open { 1.0 } else { 0.0 }, 1.0);
    let mut section = col().key(format!("app-{}", st.id)).w_full().child(volume_line(s, k, "line".into(), lead, t, st.level, Some(more)));
    if open {
        section = section.child(panel(k, "panel".into(), app_panel(s, k, st)));
    }
    section
}

/// An app's output and balance, under its name and what it's playing.
fn app_panel(s: &Ui, k: &Kit, st: &Stream) -> Vec<El> {
    let c = &k.c;
    let mut title = row().key("title").h(28.0).px(8.0).gap(6.0).center_y().child(text(st.app_name.clone()).key("app").font(k.ty.small_medium).color(c.text_base));
    if !st.media_name.is_empty() {
        title = title.child(text(st.media_name.clone()).key("media").font(k.ty.small).color(c.text_faint).grow(1.0).shrink().w(0.0).truncate());
    }
    let mut rows = vec![title];
    let id = st.id;
    for d in s.audio.sinks().filter(|d| !d.hidden || st.target_device == Some(d.id)) {
        let dev = d.id;
        rows.push(pick_row(k, format!("to-{dev}"), d, st.target_device == Some(dev), None, move |s: &mut Ui| s.send(Command::MoveStream { stream: id, device: dev })));
    }
    if st.volume.positions.len() >= 2 {
        rows.push(hairline(k, "sep"));
        rows.push(balance_row(s, k, Target::Stream(st.id)));
    }
    rows
}

/// Problems, quietly: a lost connection, then the backend's issues.
fn notices(s: &Ui, k: &Kit) -> Vec<El> {
    let c = &k.c;
    let line = |key: String, color: Color, msg: String, dismiss: Option<usize>| {
        row()
            .key(key)
            .w_full()
            .min_h(32.0)
            .pl(10.0)
            .pr(4.0)
            .gap(8.0)
            .center_y()
            .radius(8.0)
            .fill(c.bg_base)
            .border(1.0, c.border_muted)
            .child(icon(&glyphs::WARNING, 14.0).key("glyph").color(color))
            .child(text(msg).key("message").font(k.ty.small).color(c.text_base).grow(1.0).shrink().w(0.0).wrap())
            .children(dismiss.map(|i| {
                IconButton::new(&glyphs::CLOSE).variant(IconButtonVariant::Ghost).label("Dismiss").on_press(move |h: &mut Host| h.sound.send(Command::DismissIssue { index: i })).build(k).key("dismiss")
            }))
    };
    let mut out = Vec::new();
    if let Connection::Lost(msg) = &s.audio.connection {
        out.push(line("lost".into(), c.danger.icon, msg.clone(), None));
    }
    for (i, issue) in s.audio.issues.iter().enumerate() {
        let color = if issue.severity == crate::sound::model::Severity::Error { c.danger.icon } else { c.warning.icon };
        out.push(line(format!("issue-{i}"), color, issue.message.clone(), Some(i)));
    }
    out
}

/// A calm empty body: the speaker, large and faint, and one short line.
fn empty(k: &Kit, line: String) -> El {
    col()
        .key("empty")
        .w_full()
        .py(36.0)
        .gap(12.0)
        .items_center()
        .child(icon(&glyphs::SPEAKER, 40.0).key("mark").color(k.c.border_strong))
        .child(text(line).key("line").font(k.ty.small).color(k.c.text_faint))
}

pub fn main(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut body = col().key("main").w_full().padding(pad(s)).pb(pad(s) - 6.0).gap(16.0);
    if let Connection::Lost(msg) = &s.audio.connection {
        return body.child(empty(k, msg.clone()));
    }
    body = body.children(notices(s, k));
    body = body.children(device_section(s, k, DeviceKind::Sink, layer));
    body = body.children(device_section(s, k, DeviceKind::Source, layer));
    let apps: Vec<El> = s.playing().map(|st| app_row(s, k, st)).collect();
    if !apps.is_empty() {
        body = body.child(col().key("apps").w_full().gap(4.0).child(el().key("rule").h(1.0).mb(10.0).fill(c.border_muted)).children(apps));
    }
    body
}

/// Whether the panel shows the input: something's recording, or it's muted.
pub fn input_matters(s: &Ui) -> bool {
    s.audio.default_source().is_some_and(|d| d.mute) || s.audio.streams.iter().any(|st| st.kind == StreamKind::Record && !st.corked)
}

/// The tray panel's rows: the output, and the input when it matters.
pub fn tray_rows(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    if s.audio.connection != Connection::Connected {
        return Vec::new();
    }
    let mut rows: Vec<El> = device_section(s, k, DeviceKind::Sink, layer).into_iter().collect();
    if input_matters(s) {
        rows.extend(device_section(s, k, DeviceKind::Source, layer));
    }
    rows
}
