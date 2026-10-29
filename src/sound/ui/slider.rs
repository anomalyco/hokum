//! The volume slider: a thin track whose fill glows with the live level, a knob, cubic
//! volume from the model. 0–100 % fills the track; dragging past its end overdrives to 150 %,
//! after which the track rescales with a tick at 100 %. Balance uses the same track, centred.

use rune::{Drag, El, Phase, Spring, Tx, WheelEvent, el, row, text};
use rune_kit::Kit;

use super::Host;
use crate::sound::model::{MAX_VOLUME, Target};

pub const KNOB: f32 = 14.0;
const TRACK: f32 = 4.0;
pub const HEIGHT: f32 = 24.0;
/// Points past the track's end that take the volume from 100 % to 150 %.
const OVERDRIVE: f32 = 140.0;

/// The range the track shows for a volume at rest.
pub fn scale_for(v: f32) -> f32 {
    if v > 1.0 + 1e-3 { MAX_VOLUME } else { 1.0 }
}

/// An in-flight drag: the range it started on stays put until release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    pub target: Target,
    pub scale: f32,
    pub start: f32,
    pub value: f32,
}

/// Where the pointer puts the volume on a track `w` wide showing `scale`.
pub fn value_at(x: f32, w: f32, scale: f32) -> f32 {
    let inner = (w - KNOB).max(1.0);
    let x = x - KNOB / 2.0;
    let v = if scale <= 1.0 && x > inner { 1.0 + (x - inner) / OVERDRIVE * (MAX_VOLUME - 1.0) } else { x / inner * scale };
    let v = v.clamp(0.0, MAX_VOLUME);
    // A detent at 100 %.
    if (v - 1.0).abs() < 0.015 { 1.0 } else { v }
}

fn track(k: &Kit, children: Vec<El>) -> El {
    row().key("track").pin(Some(KNOB / 2.0), Some((HEIGHT - TRACK) / 2.0), Some(KNOB / 2.0), None).h(TRACK).radius(TRACK / 2.0).fill(k.c.layer_03).children(children)
}

fn knob(k: &Kit, frac: f32, label: Option<El>) -> El {
    let mut knob = el().key("knob").size(KNOB, KNOB).ml(-frac * KNOB).radius(KNOB / 2.0).fill(k.c.knob).box_shadow_list(&k.elevation.knob);
    if !k.dark {
        knob = knob.border(1.0, k.c.border_base);
    }
    if let Some(l) = label {
        knob = knob.child(row().key("bubble").absolute(KNOB / 2.0 - 24.0, -22.0).w(48.0).justify_center().child(l));
    }
    row()
        .key("knob-rail")
        .pin(Some(0.0), Some((HEIGHT - KNOB) / 2.0), Some(0.0), None)
        .h(KNOB)
        .child(el().key("before").w_frac(frac).h(KNOB).shrink())
        .child(knob)
}

trait Shadows {
    fn box_shadow_list(self, list: &[rune::Shadow]) -> Self;
}

impl Shadows for El {
    fn box_shadow_list(self, list: &[rune::Shadow]) -> Self {
        list.iter().fold(self, |e, s| e.box_shadow(*s))
    }
}

/// A volume slider for `target` showing `value` on a track of `scale`.
/// The percent shows above the knob while the row is hovered, or always while `dragging`.
pub fn volume(k: &Kit, target: Target, value: f32, scale: f32, level: Option<f32>, muted: bool, dragging: bool) -> El {
    let c = &k.c;
    let frac = (value / scale).clamp(0.0, 1.0);
    let fill = if muted { c.border_strong } else { c.icon_faint };
    let mut parts = vec![el().key("fill").w_frac(frac).h_full().radius(TRACK / 2.0).fill(fill)];
    if let Some(l) = level.filter(|_| !muted) {
        parts.push(
            el().key("level")
                .absolute(0.0, 0.0)
                .w_frac((l / scale).clamp(0.0, frac))
                .h_full()
                .radius(TRACK / 2.0)
                .fill(c.text_base)
                .motion(Spring::new(0.12, 0.0)),
        );
    }
    if scale > 1.0 {
        parts.push(row().key("unity").absolute(0.0, -3.0).w_frac(1.0 / scale).h(TRACK + 6.0).justify_end().child(el().key("tick").w(1.0).h_full().fill(c.border_strong)));
    }
    let percent = text(format!("{}%", (value * 100.0).round()))
        .key("percent")
        .font(k.ty.small_medium.features("tnum"))
        .color(if value > 1.0 { c.text_base } else { c.text_muted })
        .group_opacity(if dragging { 1.0 } else { 0.0 }, 1.0);
    el().key("slider")
        .h(HEIGHT)
        .grow(1.0)
        .w(0.0)
        .role("slider")
        .label(format!("{} %", (value * 100.0).round()))
        .cursor(rune::Cursor::Pointer)
        .child(track(k, parts))
        .child(knob(k, frac, Some(percent)))
        .on_drag(move |h: &mut Host, d: &mut Drag, _tx: &mut Tx| h.sound.slide(target, d))
        .on_wheel(move |h: &mut Host, e: WheelEvent, _tx: &mut Tx| {
            let s = &mut h.sound;
            s.wheel(target, e);
            true
        })
        .on_step(move |h: &mut Host, n: i32, _tx: &mut Tx| h.sound.step(target, n))
        .on_step_limit(move |h: &mut Host, max: bool, _tx: &mut Tx| h.sound.set_volume(target, if max { 1.0 } else { 0.0 }))
        .on_key(move |h: &mut Host, e: &rune::KeyEvent, _tx: &mut Tx| { let s = &mut h.sound; match &e.key {
            rune::Key::Text(t) if t.eq_ignore_ascii_case("m") && !e.mods.cmd => {
                s.toggle_mute(target);
                true
            }
            _ => false,
        } })
}

/// Left/right balance in `-1.0..=1.0`, filled from the centre.
pub fn balance(k: &Kit, target: Target, value: f32) -> El {
    let c = &k.c;
    let frac = (value + 1.0) / 2.0;
    let (lo, hi) = (frac.min(0.5), frac.max(0.5));
    let parts = vec![
        el().key("before").w_frac(lo).h_full(),
        el().key("fill").w_frac(hi - lo).h_full().radius(TRACK / 2.0).fill(c.icon_faint),
    ];
    let centre = row().key("centre-rail").pin(Some(KNOB / 2.0), Some((HEIGHT - 10.0) / 2.0), Some(KNOB / 2.0), None).h(10.0).child(el().key("half").w_frac(0.5).h_full()).child(el().key("tick").w(1.0).h_full().fill(c.border_strong));
    el().key("balance")
        .h(HEIGHT)
        .grow(1.0)
        .w(0.0)
        .role("slider")
        .label("Balance")
        .cursor(rune::Cursor::Pointer)
        .child(centre)
        .child(track(k, parts))
        .child(knob(k, frac, None))
        .on_drag(move |h: &mut Host, d: &mut Drag, _tx: &mut Tx| {
            let s = &mut h.sound;
            let inner = (d.w - KNOB).max(1.0);
            let mut b = ((d.x - KNOB / 2.0) / inner * 2.0 - 1.0).clamp(-1.0, 1.0);
            if b.abs() < 0.06 {
                b = 0.0;
            }
            if d.phase == Phase::Up && d.cancelled() {
                return;
            }
            s.set_balance(target, b);
        })
        .on_step(move |h: &mut Host, n: i32, _tx: &mut Tx| {
            let s = &mut h.sound;
            let b = s.balance_of(target);
            s.set_balance(target, (b + n as f32 * 0.05).clamp(-1.0, 1.0));
        })
        .on_double_press(move |h: &mut Host| h.sound.set_balance(target, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_track_maps_to_cubic_volume() {
        let w = 200.0 + KNOB;
        assert_eq!(value_at(KNOB / 2.0, w, 1.0), 0.0);
        assert!((value_at(KNOB / 2.0 + 100.0, w, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(value_at(KNOB / 2.0 + 199.0, w, 1.0), 1.0, "detent");
        assert!((value_at(KNOB / 2.0 + 200.0 + OVERDRIVE, w, 1.0) - MAX_VOLUME).abs() < 1e-6);
        assert!((value_at(KNOB / 2.0 + 100.0, w, MAX_VOLUME) - 0.75).abs() < 1e-6);
        assert_eq!(value_at(10_000.0, w, MAX_VOLUME), MAX_VOLUME);
    }
}
