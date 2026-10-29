//! The third level: adapters and their settings; every device's LE details, services and raw
//! properties.

use rune::{Cursor, El, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, SegmentedControl, Select, Switch, TextInput};

use super::{Host, Page, Tab, Ui, glyphs};
use crate::bluetooth::model::{Adapter, AddressType, Bearer, Command, Device};

const TABS: [(Tab, &str); 2] = [(Tab::Adapter, "Adapter"), (Tab::Devices, "Devices")];
const TIMEOUTS: [(u32, &str); 5] = [(60, "1 minute"), (180, "3 minutes"), (300, "5 minutes"), (900, "15 minutes"), (0, "Never")];

fn select(s: &Ui, k: &Kit, key: String, options: Vec<(String, String)>, current: &str, width: f32, choose: impl Fn(&mut Ui, &str) + Clone + 'static, layer: &mut Vec<El>) -> El {
    let labels: Vec<String> = options.iter().map(|o| o.0.clone()).collect();
    let shown = options.iter().find(|o| o.1 == current).map(|o| o.0.clone());
    let (k1, k2) = (key.clone(), key.clone());
    Select::new(key.clone(), &labels)
        .width(width)
        .current(shown.as_deref())
        .open(s.menu.as_deref() == Some(key.as_str()))
        .on_toggle(move |h: &mut Host| { let s = &mut h.bluetooth; s.menu = if s.menu.as_deref() == Some(k1.as_str()) { None } else { Some(k1.clone()) } })
        .on_close(move |h: &mut Host| {
            let s = &mut h.bluetooth;
            if s.menu.as_deref() == Some(k2.as_str()) {
                s.menu = None
            }
        })
        .on_choose(move |h: &mut Host, v: &str, _: &mut Tx| {
            let s = &mut h.bluetooth;
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

fn sep(k: &Kit, key: &str) -> El {
    el().key(key.to_owned()).h(1.0).mx(12.0).fill(k.c.border_muted)
}

fn setting(k: &Kit, key: &str, title: &str, control: El) -> El {
    row().key(key.to_owned()).w_full().h(44.0).px(12.0).gap(16.0).center_y().child(text(title.to_owned()).key("title").font(k.ty.ui).color(k.c.text_base).grow(1.0)).child(control)
}

fn heading(k: &Kit, key: &str, title: &str) -> El {
    text(title.to_owned()).key(key.to_owned()).font(k.ty.small_medium).color(k.c.text_muted).px(12.0).pt(4.0)
}

/// Facts in two columns: a faint name, a selectable value.
fn facts(k: &Kit, key: String, pairs: Vec<(String, String)>) -> El {
    let c = &k.c;
    col().key(key).w_full().px(12.0).py(8.0).gap(6.0).children(pairs.into_iter().enumerate().map(|(i, (name, value))| {
        row()
            .key(format!("f{i}"))
            .w_full()
            .gap(12.0)
            .child(text(name).key("name").font(k.ty.small).color(c.text_faint).w(112.0).truncate())
            .child(text(value).key("value").font(k.ty.small.features("tnum")).color(c.text_muted).grow(1.0).shrink().w(0.0).wrap_anywhere().selectable(true))
    }))
}

/// Services by name, each with its UUID underneath.
fn services(k: &Kit, key: String, list: &[crate::bluetooth::model::Service]) -> El {
    let c = &k.c;
    col().key(key).w_full().px(12.0).py(8.0).gap(8.0).children(list.iter().enumerate().map(|(i, x)| {
        let named = x.name != x.uuid;
        let mut e = col().key(format!("u{i}")).w_full().gap(1.0);
        if named {
            e = e.child(text(x.name.clone()).key("name").font(k.ty.small).color(c.text_muted).truncate());
        }
        e.child(text(x.uuid.clone()).key("uuid").font(k.ty.small.features("tnum")).color(c.text_faint).wrap_anywhere().selectable(true))
    }))
}

fn timeout_options(current: u32) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = TIMEOUTS.iter().map(|(s, l)| (l.to_string(), s.to_string())).collect();
    if !TIMEOUTS.iter().any(|t| t.0 == current) {
        out.insert(0, (format!("{current} seconds"), current.to_string()));
    }
    out
}

fn switch(k: &Kit, key: &str, label: &str, on: bool, f: impl Fn(&mut Ui) + 'static) -> El {
    Switch::new(on).label(label.to_owned()).hide_label().on_toggle(super::lift(f)).build(k).key(key.to_owned())
}

fn adapter_tab(s: &Ui, k: &Kit, layer: &mut Vec<El>) -> Vec<El> {
    let c = &k.c;
    let mut out = Vec::new();
    if s.bt.adapters.len() > 1 {
        let rows = s
            .bt
            .adapters
            .iter()
            .map(|a| {
                let on = s.bt.default_adapter.as_ref() == Some(&a.id);
                let id = a.id.clone();
                row()
                    .key(format!("pick-{}", a.address))
                    .role("radio")
                    .label(a.display_name().to_owned())
                    .w_full()
                    .h(36.0)
                    .px(12.0)
                    .gap(10.0)
                    .radius(8.0)
                    .center_y()
                    .hover(c.hover)
                    .cursor(Cursor::Pointer)
                    .on_press(move |h: &mut Host| h.bluetooth.send(Command::SelectAdapter { adapter: id.clone() }))
                    .child(text(a.display_name().to_owned()).key("name").font(k.ty.ui).color(if on { c.text_base } else { c.text_muted }).grow(1.0))
                    .child(text(a.address.clone()).key("address").font(k.ty.small.features("tnum")).color(c.text_faint))
                    .child(icon(&glyphs::CHECK, 14.0).key("check").color(c.icon_base).opacity(if on { 1.0 } else { 0.0 }))
            })
            .collect();
        out.push(section(k, "adapters".into(), rows));
    }
    let Some(a) = s.bt.default_adapter() else { return out };
    out.push(settings(s, k, a, layer));
    out.push(heading(k, "about-head", "Properties"));
    out.push(section(k, "about".into(), vec![facts(k, "props".into(), a.properties.clone())]));
    if !a.services.is_empty() {
        out.push(heading(k, "services-head", "Services"));
        out.push(section(k, "services".into(), vec![services(k, "uuids".into(), &a.services)]));
    }
    out
}

fn settings(s: &Ui, k: &Kit, a: &Adapter, layer: &mut Vec<El>) -> El {
    let id = a.id.clone();
    let name = match &s.alias {
        Some(edit) => TextInput::new(edit)
            .width(Some(170.0))
            .on_edit(|h: &mut Host| h.bluetooth.alias.get_or_insert_with(|| TextEdit::new("")))
            .on_submit(move |h: &mut Host, _: &mut Tx| {
                let s = &mut h.bluetooth;
                if let Some(e) = s.alias.take() {
                    s.send(Command::SetAdapterAlias { adapter: id.clone(), alias: e.text.trim().to_owned() });
                }
            })
            .build(k)
            .key("alias-field"),
        None => {
            let alias = a.display_name().to_owned();
            row()
                .key("alias")
                .role("button")
                .label("Rename")
                .h(28.0)
                .px(8.0)
                .gap(8.0)
                .radius(6.0)
                .center_y()
                .hover(k.c.hover)
                .cursor(Cursor::Pointer)
                .on_press(move |h: &mut Host| h.bluetooth.alias = Some(TextEdit::new(&alias)))
                .child(text(a.display_name().to_owned()).key("value").font(k.ty.ui).color(k.c.text_muted))
                .child(icon(&glyphs::PENCIL, 12.0).key("glyph").color(k.c.icon_faint))
        }
    };
    let (i1, i2, i3, i4, i5) = (a.id.clone(), a.id.clone(), a.id.clone(), a.id.clone(), a.id.clone());
    let (powered, discoverable, pairable) = (a.powered, a.discoverable, a.pairable);
    let power = switch(k, "power", "Powered", powered, move |s: &mut Ui| s.send(Command::SetPowered { adapter: i1.clone(), on: !powered }));
    let disc = switch(k, "discoverable", "Visible", discoverable, move |s: &mut Ui| s.send(Command::SetDiscoverable { adapter: i2.clone(), on: !discoverable }));
    let pair = switch(k, "pairable", "Pairable", pairable, move |s: &mut Ui| s.send(Command::SetPairable { adapter: i3.clone(), on: !pairable }));
    let dt = select(
        s,
        k,
        "discoverable-timeout".into(),
        timeout_options(a.discoverable_timeout),
        &a.discoverable_timeout.to_string(),
        140.0,
        move |s: &mut Ui, v: &str| {
            if let Ok(seconds) = v.parse() {
                s.send(Command::SetDiscoverableTimeout { adapter: i4.clone(), seconds })
            }
        },
        layer,
    );
    let pt = select(
        s,
        k,
        "pairable-timeout".into(),
        timeout_options(a.pairable_timeout),
        &a.pairable_timeout.to_string(),
        140.0,
        move |s: &mut Ui, v: &str| {
            if let Ok(seconds) = v.parse() {
                s.send(Command::SetPairableTimeout { adapter: i5.clone(), seconds })
            }
        },
        layer,
    );
    section(
        k,
        "settings".into(),
        vec![
            setting(k, "name-row", "Name", name),
            sep(k, "s0"),
            setting(k, "power-row", "Powered", power),
            sep(k, "s1"),
            setting(k, "discoverable-row", "Visible to others", disc),
            setting(k, "discoverable-timeout-row", "Visible for", dt),
            sep(k, "s2"),
            setting(k, "pairable-row", "Pairable", pair),
            setting(k, "pairable-timeout-row", "Pairable for", pt),
        ],
    )
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn device_facts(d: &Device) -> Vec<(String, String)> {
    let mut f = vec![
        ("Address".to_owned(), d.address.clone()),
        ("Address type".to_owned(), if d.address_type == AddressType::Random { "Random" } else { "Public" }.to_owned()),
        (
            "Bearer".to_owned(),
            match d.bearer {
                Bearer::BrEdr => "Classic",
                Bearer::Le => "Low Energy",
                Bearer::Dual => "Classic + Low Energy",
                Bearer::Unknown => "Unknown",
            }
            .to_owned(),
        ),
    ];
    if let Some(r) = d.rssi {
        f.push(("RSSI".into(), format!("{r} dBm")));
    }
    if let Some(t) = d.tx_power {
        f.push(("TX power".into(), format!("{t} dBm")));
    }
    if let Some(c) = d.class {
        f.push(("Class".into(), format!("0x{c:06x}")));
    }
    if let Some(a) = d.appearance {
        f.push(("Appearance".into(), format!("0x{a:04x}")));
    }
    if let Some(m) = &d.raw.modalias {
        f.push(("Modalias".into(), m.clone()));
    }
    f
}

fn device_card(s: &Ui, k: &Kit, d: &Device) -> El {
    let c = &k.c;
    let open = s.details.contains(&d.id);
    let id = d.id.clone();
    let head = row()
        .key("head")
        .role("button")
        .label(d.display_name().to_owned())
        .expanded(open)
        .w_full()
        .h(36.0)
        .pl(12.0)
        .pr(8.0)
        .gap(10.0)
        .radius(8.0)
        .center_y()
        .hover(c.hover)
        .cursor(Cursor::Pointer)
        .on_press(move |h: &mut Host| {
            let s = &mut h.bluetooth;
            if !s.details.remove(&id) {
                s.details.insert(id.clone());
            }
        })
        .child(icon(glyphs::kind(d.kind), 14.0).key("glyph").color(if d.connected { c.icon_base } else { c.icon_faint }))
        .child(text(d.display_name().to_owned()).key("name").font(k.ty.ui).color(if d.is_paired() || d.connected { c.text_base } else { c.text_muted }).grow(1.0).shrink().w(0.0).truncate())
        .child(text(d.address.clone()).key("address").font(k.ty.small.features("tnum")).color(c.text_faint))
        .child(icon(&glyphs::CHEVRON, 12.0).key("chevron").color(c.icon_faint).rotate(if open { std::f32::consts::PI } else { 0.0 }));
    let mut rows = vec![head];
    if open {
        rows.push(sep(k, "s0"));
        rows.push(facts(k, "facts".into(), device_facts(d)));
        let (id, blocked) = (d.id.clone(), d.blocked);
        rows.push(setting(
            k,
            "block-row",
            "Blocked",
            switch(k, "block", "Blocked", blocked, move |s: &mut Ui| {
                let device = id.clone();
                s.send(if blocked { Command::Unblock { device } } else { Command::Block { device } })
            }),
        ));
        if !d.services.is_empty() {
            rows.push(sep(k, "s1"));
            rows.push(heading(k, "services-head", "Services"));
            rows.push(services(k, "services".into(), &d.services));
        }
        let mut adv: Vec<(String, String)> = d.raw.manufacturer_data.iter().map(|m| (m.company_name.clone().unwrap_or_else(|| format!("0x{:04x}", m.company)), hex(&m.data))).collect();
        adv.extend(d.raw.service_data.iter().map(|x| (x.service.name.clone(), hex(&x.data))));
        if !d.raw.advertising_flags.is_empty() {
            adv.push(("Flags".into(), hex(&d.raw.advertising_flags)));
        }
        if !adv.is_empty() {
            rows.push(sep(k, "s2"));
            rows.push(heading(k, "adv-head", "Advertising"));
            rows.push(facts(k, "adv".into(), adv));
        }
        rows.push(sep(k, "s3"));
        rows.push(heading(k, "props-head", "Properties"));
        rows.push(facts(k, "props".into(), d.raw.properties.clone()));
    }
    section(k, format!("dev-{}", d.address), rows)
}

fn devices_tab(s: &Ui, k: &Kit) -> Vec<El> {
    s.bt.devices().map(|d| device_card(s, k, d)).collect()
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
            IconButton::new(&glyphs::BACK)
                .variant(IconButtonVariant::Ghost)
                .label("Back")
                .on_press_tx(|h: &mut Host, tx: &mut Tx| {
                    let s = &mut h.bluetooth;
                    tx.instant();
                    s.page = Page::Main;
                    s.menu = None;
                    s.alias = None;
                })
                .build(k)
                .key("back"),
        )
        .child(
            SegmentedControl::new(&labels, current)
                .width(None)
                .on_pick(|h: &mut Host, i: usize, tx: &mut Tx| {
                    let s = &mut h.bluetooth;
                    tx.instant();
                    s.menu = None;
                    s.page = Page::Advanced(TABS[i].0);
                })
                .build(k)
                .key("tabs")
                .grow(1.0),
        );
    let content = match tab {
        Tab::Adapter => adapter_tab(s, k, layer),
        Tab::Devices => devices_tab(s, k),
    };
    let body = col().key(format!("tab-{}", current.unwrap_or(0))).w_full().px(16.0).pt(4.0).pb(24.0).gap(10.0).children(content);
    col().key("advanced").w_full().child(row().key("bar").w_full().px(12.0).py(12.0).child(head)).child(body)
}
