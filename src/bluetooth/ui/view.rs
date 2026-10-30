//! The first two levels: the power switch, connected and paired devices; a row opens in place,
//! and "Add device" scans in place with pairing codes under the device being paired.

use rune::{Color, Cursor, El, TextStyle, Transition, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Spinner, Switch,
    TextInput,
};

use super::{Host, Mode, Ui, glyphs};
use crate::bluetooth::model::{
    Command, Connection, Device, PairingKind, PairingRequest, Pending, PowerState, Severity,
};

/// The device mark's diameter; text starts after it.
const MARK: f32 = 30.0;
const GAP: f32 = 12.0;
const ROW: f32 = 46.0;

fn pad(s: &Ui) -> f32 {
    if s.mode == Mode::Popover { 12.0 } else { 16.0 }
}

/// A device's glyph in a circle: filled when connected, a spinner while something's pending.
fn mark(k: &Kit, d: &Device, size: f32) -> El {
    let c = &k.c;
    let on = d.connected;
    let mut m = el()
        .key("mark")
        .size(size, size)
        .radius(size / 2.0)
        .center();
    m = if on {
        m.fill(c.bg_contrast)
    } else {
        m.fill(c.bg_base).border(1.0, c.border_muted)
    };
    if d.pending.is_some() {
        return m.child(
            Spinner::new()
                .size(size * 0.5)
                .color(if on { c.icon_contrast } else { c.icon_muted })
                .build(k)
                .key("spinner"),
        );
    }
    m.child(
        icon(glyphs::kind(d.kind), size * 0.5)
            .key("glyph")
            .color(if on { c.icon_contrast } else { c.icon_muted }),
    )
}

/// A battery: an outline filled to its level, then the number.
fn battery(k: &Kit, level: u8) -> El {
    let c = &k.c;
    let low = level <= 15;
    let color = if low { c.danger.icon } else { c.icon_muted };
    let w = 20.0;
    let inner = ((w - 5.0) * level as f32 / 100.0).max(1.5);
    let body = el()
        .key("cell")
        .size(w, 10.0)
        .radius(2.5)
        .border(1.0, c.icon_faint)
        .padding(1.5)
        .child(el().key("charge").size(inner, 5.0).radius(1.0).fill(color));
    let nub = el()
        .key("nub")
        .size(1.5, 4.0)
        .radius(0.75)
        .fill(c.icon_faint);
    row()
        .key("battery")
        .gap(6.0)
        .center_y()
        .child(
            text(format!("{level}%"))
                .key("level")
                .font(k.ty.small.features("tnum"))
                .color(if low { c.danger.fg } else { c.text_faint }),
        )
        .child(row().key("icon").gap(1.0).center_y().child(body).child(nub))
}

/// Signal strength as four bars, for nearby devices.
fn signal(k: &Kit, rssi: i16) -> El {
    let bars = match rssi {
        r if r >= -55 => 4,
        r if r >= -67 => 3,
        r if r >= -80 => 2,
        _ => 1,
    };
    let c = &k.c;
    row()
        .key("signal")
        .gap(2.0)
        .items_end()
        .h(12.0)
        .children((0..4).map(|i| {
            el().key(format!("bar-{i}"))
                .size(3.0, 3.0 + 3.0 * i as f32)
                .radius(1.0)
                .fill(if i < bars {
                    c.icon_muted
                } else {
                    c.border_base
                })
        }))
}

fn chevron(k: &Kit, open: bool) -> El {
    icon(&glyphs::CHEVRON, 14.0)
        .key("chevron")
        .color(k.c.icon_faint)
        .rotate(if open { std::f32::consts::PI } else { 0.0 })
}

/// One device: a click connects, disconnects or pairs; the chevron opens it.
fn device_row(s: &Ui, k: &Kit, d: &Device, nearby: bool) -> El {
    let c = &k.c;
    let id = d.id.clone();
    let open = s.open.as_ref() == Some(&d.id);
    let unnamed = d.name.is_none();
    let name_color = if d.connected {
        c.text_base
    } else if unnamed {
        c.text_faint
    } else if nearby {
        c.text_muted
    } else {
        c.text_base
    };
    let label = match (d.connected, d.is_paired()) {
        (true, _) => crate::i18n::connect(d.display_name(), true),
        (false, true) => crate::i18n::connect(d.display_name(), false),
        _ => crate::i18n::pair(d.display_name()),
    };
    let press = {
        let id = id.clone();
        move |s: &mut Ui| s.activate(&id)
    };
    let mut main = row()
        .key("press")
        .role("button")
        .label(label)
        .grow(1.0)
        .w(0.0)
        .h(ROW)
        .pl(8.0)
        .pr(4.0)
        .gap(GAP)
        .radius(10.0)
        .center_y()
        .hover(c.hover)
        .press(c.pressed)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .on_press(super::lift(press))
        .child(mark(k, d, MARK))
        .child(
            text(d.display_name().to_owned())
                .key("name")
                .font(if unnamed { k.ty.small } else { k.ty.ui })
                .color(name_color)
                .grow(1.0)
                .shrink()
                .w(0.0)
                .truncate(),
        );
    if let Some(b) = d.battery.filter(|_| d.connected) {
        main = main.child(battery(k, b));
    }
    if nearby && let Some(r) = d.rssi {
        main = main.child(signal(k, r));
    }
    let mut line = row()
        .key("line")
        .w_full()
        .gap(2.0)
        .center_y()
        .hover_group()
        .child(main);
    if s.mode == Mode::Popover {
    } else if nearby {
        line = line.child(el().key("trail").size(28.0, ROW));
    } else {
        let more = el()
            .key("more")
            .role("button")
            .label(crate::i18n::options(d.display_name()))
            .expanded(open)
            .size(28.0, ROW)
            .radius(8.0)
            .center()
            .hover(c.hover)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .on_press(move |h: &mut Host| h.bluetooth.toggle_open(&id))
            .child(chevron(k, open))
            .group_opacity(if open { 1.0 } else { 0.35 }, 1.0);
        line = line.child(more);
    }
    let mut section = col().key(format!("dev-{}", d.address)).w_full().child(line);
    if let Some(p) = s.bt.pairing.as_ref().filter(|p| p.device == d.id) {
        section = section.child(pairing(s, k, p));
    } else if open && !nearby {
        section = section.child(panel(k, "panel".into(), device_panel(s, k, d)));
    }
    section
}

/// The inset card a row opens into, aligned with the name.
fn panel(k: &Kit, key: String, rows: Vec<El>) -> El {
    col()
        .key(key)
        .ml(8.0 + MARK + GAP - 10.0)
        .mt(2.0)
        .mb(6.0)
        .padding(4.0)
        .gap(2.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_muted)
        .children(rows)
        .transition(Transition::opacity().combined(Transition::offset(0.0, -4.0)))
}

fn hairline(k: &Kit, key: &str) -> El {
    el().key(key.to_owned())
        .h(1.0)
        .mx(8.0)
        .my(3.0)
        .fill(k.c.border_muted)
}

fn panel_row(k: &Kit, key: &str, glyph: &'static rune::Icon, control: El) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .min_h(32.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .child(
            el().key("lead")
                .w(14.0)
                .center()
                .child(icon(glyph, 14.0).color(k.c.icon_faint)),
        )
        .child(control)
}

/// A whole-row action in a panel: glyph and words.
fn action_row(
    k: &Kit,
    key: &str,
    glyph: &'static rune::Icon,
    label: &str,
    color: Color,
    trailing: Option<&'static rune::Icon>,
    press: impl Fn(&mut Host) + 'static,
) -> El {
    let c = &k.c;
    row()
        .key(key.to_owned())
        .role("button")
        .label(label.to_owned())
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(10.0)
        .radius(6.0)
        .center_y()
        .hover(c.hover)
        .press(c.pressed)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .on_press(press)
        .child(
            el().key("lead")
                .w(14.0)
                .center()
                .child(icon(glyph, 14.0).color(color)),
        )
        .child(
            text(label.to_owned())
                .key("label")
                .font(k.ty.ui)
                .color(color)
                .grow(1.0),
        )
        .children(trailing.map(|t| icon(t, 12.0).key("trail").color(c.icon_faint)))
}

/// Level two for a known device: its name, trust, Sound, forget.
fn device_panel(s: &Ui, k: &Kit, d: &Device) -> Vec<El> {
    let c = &k.c;
    let name = TextInput::new(&s.rename)
        .plain()
        .width(None)
        .on_edit(|h: &mut Host| &mut h.bluetooth.rename)
        .on_submit(|h: &mut Host, _: &mut Tx| h.bluetooth.commit_rename())
        .build(k)
        .key("name-field")
        .ml(-8.0)
        .grow(1.0);
    let id = d.id.clone();
    let trusted = d.trusted;
    let trust = Switch::new(trusted)
        .label(crate::i18n::t("Trusted"))
        .hide_label()
        .on_toggle(move |h: &mut Host| {
            let s = &mut h.bluetooth;
            let device = id.clone();
            s.send(if trusted {
                Command::Untrust { device }
            } else {
                Command::Trust { device }
            })
        })
        .build(k)
        .key("trust-switch");
    let mut rows = vec![
        panel_row(k, "rename", &glyphs::PENCIL, name),
        panel_row(
            k,
            "trust",
            &glyphs::SHIELD,
            row()
                .key("trust-row")
                .grow(1.0)
                .center_y()
                .child(
                    text(crate::i18n::t("Trusted"))
                        .key("label")
                        .font(k.ty.ui)
                        .color(c.text_muted)
                        .grow(1.0),
                )
                .child(trust),
        ),
    ];
    if d.audio && s.sound {
        rows.push(action_row(
            k,
            "sound",
            &glyphs::SOUND,
            crate::i18n::t("Sound settings"),
            c.text_muted,
            Some(&glyphs::ARROW_OUT),
            |h: &mut Host| h.show_section(crate::settings::Section::Sound),
        ));
    }
    rows.push(hairline(k, "sep"));
    let id = d.id.clone();
    let forgetting = d.pending == Some(Pending::Removing);
    rows.push(action_row(
        k,
        "forget",
        &glyphs::TRASH,
        crate::i18n::t("Forget"),
        c.danger.fg,
        None,
        move |h: &mut Host| {
            let s = &mut h.bluetooth;
            if !forgetting {
                s.send(Command::Remove { device: id.clone() });
            }
        },
    ));
    rows
}

/// A passkey as two groups of three ("381 904"); `entered` digits typed so far stand out.
fn digits(k: &Kit, code: &str, entered: Option<usize>) -> El {
    let c = &k.c;
    let style = TextStyle {
        size: 28.0,
        ..k.ty.ui_medium
    }
    .lh(36.0)
    .features("tnum");
    row()
        .key("digits")
        .gap(2.0)
        .center_y()
        .children(code.chars().enumerate().map(|(i, ch)| {
            let lit = entered.is_none_or(|n| i < n);
            let gap = if i == 3 && code.len() == 6 { 10.0 } else { 0.0 };
            text(ch.to_string())
                .key(format!("d{i}"))
                .font(style)
                .color(if lit { c.text_base } else { c.text_faint })
                .ml(gap)
        }))
}

/// The pairing agent's question, under the device it's about.
fn pairing(s: &Ui, k: &Kit, p: &PairingRequest) -> El {
    let c = &k.c;
    let cancel = |label: &str| {
        Button::new(label.to_owned())
            .variant(ButtonVariant::Ghost)
            .on_press(|h: &mut Host| h.bluetooth.reply(false))
            .build(k)
            .key("cancel")
    };
    let accept = |label: &str| {
        Button::new(label.to_owned())
            .variant(ButtonVariant::Contrast)
            .on_press(|h: &mut Host| h.bluetooth.reply(true))
            .build(k)
            .key("accept")
    };
    let buttons = |b: Vec<El>| {
        row()
            .key("buttons")
            .w_full()
            .gap(8.0)
            .justify_end()
            .children(b)
    };
    let mut body = col().key("pairing").padding(12.0).gap(12.0);
    match &p.kind {
        PairingKind::Confirm { passkey } => {
            body = body.child(row().key("code").w_full().justify_center().child(digits(
                k,
                &format!("{passkey:06}"),
                None,
            )));
            body = body.child(buttons(vec![
                cancel(crate::i18n::t("Cancel")),
                accept(crate::i18n::t("Pair")),
            ]));
        }
        PairingKind::DisplayPasskey { passkey, entered } => {
            body = body.child(row().key("code").w_full().justify_center().child(digits(
                k,
                &format!("{passkey:06}"),
                Some(*entered as usize),
            )));
            body = body.child(buttons(vec![cancel(crate::i18n::t("Cancel"))]));
        }
        PairingKind::DisplayPin { pin } => {
            body = body.child(
                row()
                    .key("code")
                    .w_full()
                    .justify_center()
                    .child(digits(k, pin, None)),
            );
            body = body.child(buttons(vec![cancel(crate::i18n::t("Cancel"))]));
        }
        PairingKind::RequestPin | PairingKind::RequestPasskey => {
            let placeholder = if p.kind == PairingKind::RequestPin {
                "PIN"
            } else {
                "Passkey"
            };
            let field = TextInput::new(&s.code)
                .placeholder(placeholder)
                .width(None)
                .on_edit(|h: &mut Host| &mut h.bluetooth.code)
                .on_submit(|h: &mut Host, _: &mut Tx| h.bluetooth.reply(true))
                .build(k)
                .key("code-field")
                .grow(1.0);
            body = body.child(
                row()
                    .key("entry")
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(field)
                    .child(cancel(crate::i18n::t("Cancel")))
                    .child(accept(crate::i18n::t("Pair"))),
            );
        }
        PairingKind::AuthorizePairing => {
            body = body.child(buttons(vec![
                cancel(crate::i18n::t("Deny")),
                accept(crate::i18n::t("Allow")),
            ]));
        }
        PairingKind::Authorize { service } => {
            body = body.child(
                row()
                    .key("ask")
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text(service.name.clone())
                            .key("service")
                            .font(k.ty.ui)
                            .color(c.text_muted)
                            .grow(1.0)
                            .shrink()
                            .w(0.0)
                            .truncate(),
                    )
                    .child(cancel(crate::i18n::t("Deny")))
                    .child(accept(crate::i18n::t("Allow"))),
            );
        }
    }
    body.ml(8.0 + MARK + GAP - 10.0)
        .mt(2.0)
        .mb(6.0)
        .radius(10.0)
        .fill(c.bg_base)
        .border(1.0, c.border_muted)
        .transition(Transition::opacity().combined(Transition::offset(0.0, -4.0)))
}

/// Problems, quietly, and only when there are some.
fn notices(s: &Ui, k: &Kit) -> Vec<El> {
    let c = &k.c;
    s.bt.issues
        .iter()
        .map(|issue| {
            let color = if issue.severity == Severity::Error {
                c.danger.icon
            } else {
                c.warning.icon
            };
            let id = issue.id;
            row()
                .key(format!("issue-{id}"))
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
                .child(
                    text(issue.message.clone())
                        .key("message")
                        .font(k.ty.small)
                        .color(c.text_base)
                        .grow(1.0)
                        .shrink()
                        .w(0.0)
                        .wrap(),
                )
                .child(
                    IconButton::new(&glyphs::CLOSE)
                        .variant(IconButtonVariant::Ghost)
                        .label(crate::i18n::t("Dismiss"))
                        .on_press(move |h: &mut Host| {
                            h.bluetooth.send(Command::DismissIssue { id })
                        })
                        .build(k)
                        .key("dismiss"),
                )
        })
        .collect()
}

/// The power switch, beside Bluetooth's mark.
fn header(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let a = s.bt.default_adapter();
    let state = a.map_or(PowerState::Off, |a| a.power_state);
    let on = matches!(state, PowerState::On | PowerState::TurningOn);
    let busy = matches!(state, PowerState::TurningOn | PowerState::TurningOff);
    let blocked = state == PowerState::Blocked;
    let mut head = row()
        .key("header")
        .w_full()
        .h(32.0)
        .pl(8.0 + (MARK - 16.0) / 2.0)
        .gap(10.0)
        .center_y()
        .child(
            icon(&glyphs::BLUETOOTH, 16.0)
                .key("glyph")
                .color(c.icon_base)
                .opacity(if on { 1.0 } else { 0.0 }),
        );
    if s.bt.adapters.len() > 1
        && let Some(a) = a
    {
        head = head.child(
            text(a.display_name().to_owned())
                .key("adapter")
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate(),
        );
    }
    if blocked {
        head = head.child(
            text(crate::i18n::t("Blocked"))
                .key("blocked")
                .font(k.ty.small)
                .color(c.text_faint),
        );
    }
    head = head.child(el().key("fill").grow(1.0));
    if busy {
        head = head.child(Spinner::new().size(12.0).build(k).key("busy"));
    }
    if a.is_some() {
        let switch = Switch::new(on)
            .label(crate::i18n::t("Bluetooth"))
            .hide_label()
            .control_size(ControlSize::Small)
            .disabled(blocked || busy)
            .on_toggle(move |h: &mut Host| h.bluetooth.set_powered(!on))
            .build(k)
            .key("power");
        head = head.child(switch.mr(8.0));
    }
    head
}

/// A calm empty body: Bluetooth's mark, large and faint, and at most one short line.
fn empty(k: &Kit, line: Option<String>) -> El {
    let c = &k.c;
    col()
        .key("empty")
        .w_full()
        .py(36.0)
        .gap(12.0)
        .items_center()
        .child(
            icon(&glyphs::BLUETOOTH, 40.0)
                .key("mark")
                .color(c.border_strong),
        )
        .children(line.map(|l| text(l).key("line").font(k.ty.small).color(c.text_faint)))
}

fn rule(k: &Kit, key: &str) -> El {
    el().key(key.to_owned())
        .h(1.0)
        .mx(8.0)
        .my(6.0)
        .fill(k.c.border_muted)
}

/// "Add device", open: what discovery hears, best signal first; nameless ones last.
fn nearby(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let discovering = s.bt.default_adapter().is_some_and(|a| a.discovering);
    let mut found: Vec<&Device> =
        s.bt.nearby()
            .filter(|d| {
                d.rssi.is_some()
                    || d.pending.is_some()
                    || s.bt.pairing.as_ref().is_some_and(|p| p.device == d.id)
            })
            .collect();
    found.sort_by_key(|d| d.name.is_none());
    let head = row()
        .key("nearby-head")
        .w_full()
        .h(32.0)
        .pl(8.0 + (MARK - 16.0) / 2.0)
        .gap(10.0)
        .center_y()
        .child(icon(&glyphs::PLUS, 16.0).key("glyph").color(c.icon_muted))
        .child(
            text(crate::i18n::t("Add device"))
                .key("title")
                .font(k.ty.ui_medium)
                .color(c.text_muted),
        )
        .child(el().key("fill").grow(1.0))
        .children(discovering.then(|| Spinner::new().size(12.0).build(k).key("scanning")))
        .child(
            IconButton::new(&glyphs::CLOSE)
                .variant(IconButtonVariant::GhostMuted)
                .label(crate::i18n::t("Stop"))
                .on_press(|h: &mut Host| h.bluetooth.set_adding(false))
                .build(k)
                .key("close"),
        );
    col()
        .key("nearby")
        .w_full()
        .gap(0.0)
        .child(head)
        .children(found.into_iter().map(|d| device_row(s, k, d, true)))
        .transition(Transition::opacity())
}

pub fn main(s: &Ui, k: &Kit) -> El {
    let p = pad(s);
    let mut body = col()
        .key("main")
        .w_full()
        .padding(p - 4.0)
        .pt(p - 2.0)
        .pb(p - 6.0)
        .gap(2.0);
    let issues = notices(s, k);
    if !issues.is_empty() {
        body = body.child(
            col()
                .key("issues")
                .w_full()
                .px(4.0)
                .pb(8.0)
                .gap(6.0)
                .children(issues),
        );
    }
    match &s.bt.connection {
        Connection::NotRunning => {
            return body.child(empty(
                k,
                Some(crate::i18n::t("Bluetooth isn't running").into()),
            ));
        }
        Connection::Lost(msg) => return body.child(empty(k, Some(msg.clone()))),
        Connection::Connecting => return body.child(empty(k, None)),
        Connection::Ready => {}
    }
    if s.bt.adapters.is_empty() {
        return body.child(empty(
            k,
            Some(crate::i18n::t("No Bluetooth adapter").into()),
        ));
    }
    body = body.child(header(s, k));
    if !s.powered() {
        return body.child(empty(k, None));
    }
    let connected: Vec<El> =
        s.bt.connected()
            .map(|d| device_row(s, k, d, false))
            .collect();
    let paired: Vec<El> = s.bt.paired().map(|d| device_row(s, k, d, false)).collect();
    body = body.child(rule(k, "rule-top"));
    let any = !connected.is_empty() || !paired.is_empty();
    body = body.children(connected);
    body = body.children(paired);
    if any {
        body = body.child(rule(k, "rule-bottom"));
    }
    if s.adding {
        body = body.child(nearby(s, k));
    }
    let add = (!s.adding).then(|| {
        Button::new(crate::i18n::t("Add device"))
            .variant(ButtonVariant::GhostMuted)
            .icon(&glyphs::PLUS)
            .on_press(|h: &mut Host| h.bluetooth.set_adding(true))
            .build(k)
            .key("add")
    });
    body.child(
        row()
            .key("corner")
            .w_full()
            .mt(2.0)
            .pl(2.0)
            .center_y()
            .children(add),
    )
}

/// The tray panel's rows: connected devices (a click disconnects), then paired ones (a click
/// connects). Nothing when Bluetooth is off or missing.
pub fn tray_rows(s: &Ui, k: &Kit) -> Vec<El> {
    if !super::ready(s) || !s.powered() {
        return Vec::new();
    }
    s.bt.connected()
        .chain(s.bt.paired())
        .take(4)
        .map(|d| device_row(s, k, d, false))
        .collect()
}
