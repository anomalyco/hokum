//! The tray panel: a click on Hokum's tray icon opens the modules' first screens, compact.
//! The output's volume and device, the input while something records (or it's muted), the
//! Bluetooth devices (a click connects or disconnects), Do not disturb, and a way into the
//! window. Outside the card, clicks pass through.

use rune::{
    Align, Cursor, El, KeyPolicy, OverlayInput, OverlayOptions, OverlayPlacement, OverlaySpec,
    ScreenEdge, col, el, row, text,
};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, Switch};

use crate::settings::{Surface, Ui};

/// The card's width.
pub const WIDTH: f32 = 340.0;
/// Room around the card for its shadow.
pub const MARGIN: f32 = 12.0;
/// Tall enough for the card with a row open.
const HEIGHT: f32 = 720.0;

static BELL: rune::Icon = rune::Icon {
    name: "bell",
    view: 16.0,
    body: r#"<path d="M4.5 10.5V7a3.5 3.5 0 0 1 7 0v3.5l1 1.5h-9l1-1.5Z" stroke="currentColor" stroke-linejoin="round"/><path d="M6.5 14h3" stroke="currentColor"/>"#,
};

fn rule(k: &Kit, key: &str) -> El {
    el().key(key.to_owned())
        .h(1.0)
        .mx(14.0)
        .fill(k.c.border_muted)
}

/// Do not disturb, and the window.
fn footer(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let dnd = s.loaded.behaviour.do_not_disturb;
    let toggle = |s: &mut Ui| {
        if let Some(w) = &s.handle {
            w.update(|w, _| w.toggle_dnd());
        }
    };
    let open = IconButton::new(&crate::sound::ui::glyphs::EXPAND)
        .variant(IconButtonVariant::GhostMuted)
        .label(crate::i18n::t("Open Hokum"))
        .on_press(|s: &mut Ui| {
            if let Some(w) = &s.handle {
                crate::modules::open(w, None, None);
            }
            if let Some(d) = &s.dismiss {
                d();
            }
        })
        .build(k)
        .key("open-window");
    let switch = Switch::new(dnd)
        .label(crate::i18n::t("Do not disturb"))
        .hide_label()
        .control_size(ControlSize::Small)
        .on_toggle(toggle)
        .build(k)
        .key("dnd-switch");
    row()
        .key("footer")
        .w_full()
        .h(44.0)
        .pl(8.0)
        .pr(6.0)
        .gap(6.0)
        .center_y()
        .child(
            row()
                .key("dnd")
                .role("switch")
                .label(crate::i18n::t("Do not disturb"))
                .grow(1.0)
                .h(32.0)
                .pl(6.0)
                .pr(6.0)
                .gap(10.0)
                .radius(8.0)
                .center_y()
                .hover(c.hover)
                .cursor(Cursor::Pointer)
                .on_press(toggle)
                .child(rune::icon(&BELL, 16.0).key("glyph").color(if dnd {
                    c.icon_base
                } else {
                    c.icon_muted
                }))
                .child(
                    text(crate::i18n::t("Do not disturb"))
                        .key("label")
                        .font(k.ty.ui)
                        .color(c.text_base)
                        .grow(1.0),
                )
                .child(switch),
        )
        .child(open.opacity(0.7))
}

pub fn view(s: &Ui, k: &Kit) -> El {
    let c = &k.c;
    let mut layer = Vec::new();
    let mut card = col().key("panel").w_full();
    let mut parts = 0;
    if s.loaded.modules.sound && s.sound_error.is_none() {
        let rows = crate::sound::ui::panel(&s.sound, k, &mut layer);
        if !rows.is_empty() {
            card = card.child(
                col()
                    .key("sound")
                    .w_full()
                    .px(14.0)
                    .pt(12.0)
                    .pb(14.0)
                    .gap(14.0)
                    .children(rows),
            );
            parts += 1;
        }
    }
    if s.loaded.modules.bluetooth && s.bluetooth_error.is_none() {
        let rows = crate::bluetooth::ui::panel(&s.bluetooth, k);
        if !rows.is_empty() {
            if parts > 0 {
                card = card.child(rule(k, "rule-bluetooth"));
            }
            card = card.child(col().key("bluetooth").w_full().padding(6.0).children(rows));
            parts += 1;
        }
    }
    if parts > 0 {
        card = card.child(rule(k, "rule-footer"));
    }
    let card = card
        .child(footer(s, k))
        .radius(14.0)
        .fill(c.page)
        .border(1.0, c.border_base)
        .hit();
    let card = k
        .elevation
        .overlay
        .iter()
        .fold(card, |e, sh| e.box_shadow(*sh));
    col()
        .key("panel-frame")
        .w(WIDTH + 2.0 * MARGIN)
        .padding(MARGIN)
        .child(card)
        .children(layer.into_iter().map(El::hit))
}

/// The panel's overlay: under the top edge at the right, where tray icons usually sit.
pub fn overlay() -> rune::ConfiguredOverlay<Ui> {
    OverlaySpec::new(
        "hokum-panel",
        (WIDTH + 2.0 * MARGIN, HEIGHT),
        crate::settings::view,
    )
    .options(OverlayOptions {
        placement: Some(OverlayPlacement::Edge {
            edge: ScreenEdge::Top,
            margin: 4.0,
            align: Align::End,
            inset: 4.0,
        }),
        keys: KeyPolicy::OnDemand,
        input: OverlayInput::Regions,
        ..OverlayOptions::default()
    })
    .on_key(crate::settings::on_key)
    .on_focus(|s: &mut Ui, focused, _| {
        if !focused
            && s.sound.visible
            && let Some(d) = &s.dismiss
        {
            d();
        }
    })
    .setup(crate::settings::setup)
}

/// The panel's state: the window's, drawn as the panel.
pub fn state(window: rune::AppHandle<Ui>) -> Ui {
    let mut ui = crate::settings::init();
    ui.surface = Surface::Panel;
    ui.sound.mode = crate::sound::ui::Mode::Popover;
    ui.bluetooth.mode = crate::bluetooth::ui::Mode::Popover;
    ui.hidden();
    ui.handle = Some(window);
    ui
}
