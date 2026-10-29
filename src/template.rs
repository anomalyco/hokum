//! Builds a card's elements from its template and tokens.

use rune::{
    Color, Cursor, El, Family, Spring, SurfaceFit, TextStyle, col, el, icon, row, spacer, text,
};
use rune_kit::Kit;

use crate::card::{Cards, Note};
use crate::style::Mark;
use crate::theme::{Align, Cond, Fill, Font, Justify, Node, Paint, Props, Tokens};

/// Everything a template needs to draw one card.
pub struct Ctx<'a> {
    pub n: &'a Note,
    pub t: &'a Tokens,
    pub k: &'a Kit,
    pub accent: Color,
}

impl Ctx<'_> {
    /// A colour for this card. `text`, `muted` and `faint` mean the theme's tokens of those
    /// names (at the given opacity); the tokens themselves resolve against the kit's palette.
    pub fn paint(&self, p: &Paint) -> Color {
        if let Paint::Named(name, a) = p {
            let token = match name.as_str() {
                "text" => Some(&self.t.text),
                "muted" => Some(&self.t.muted),
                "faint" => Some(&self.t.faint),
                _ => None,
            };
            if let Some(token) = token {
                let c = token.resolve(self.k, self.accent);
                return if *a < 1.0 { c.alpha(c.a * a) } else { c };
            }
        }
        p.resolve(self.k, self.accent)
    }

    /// A text colour, opaque: translucent text (`"white/80"`) is mixed over the card's fill
    /// the way a browser would (in sRGB), since glyphs drawn with alpha come out darker than
    /// intended.
    pub fn ink(&self, p: &Paint) -> Color {
        let c = self.paint(p);
        if c.a >= 0.999 {
            return c;
        }
        let under = match &self.t.fill {
            crate::theme::Fill::Solid(f) => self.paint(f),
            crate::theme::Fill::Gradient(a, b) => mix(self.paint(a), self.paint(b), 0.5),
        };
        let under = if under.a < 0.5 {
            self.k.c.layer_01
        } else {
            under
        };
        mix(under, Color { a: 1.0, ..c }, c.a)
    }

    fn has(&self, c: &Cond) -> bool {
        match c {
            Cond::Has(f) => match f.as_str() {
                "icon" => self.n.icon.is_some(),
                "image" => self.n.image.is_some(),
                "actions" => !self.n.actions.is_empty(),
                "value" => self.n.value.is_some(),
                "context" => self.n.context.is_some(),
                field => !crate::theme::field(self.n, field).is_empty(),
            },
            Cond::Urgency(u) => self.n.urgency == *u,
            Cond::App(g) => crate::theme::glob(g, &self.n.app),
            Cond::Not(c) => !self.has(c),
        }
    }
}

/// A line height for `size` on whole even points, so text lands on whole pixels at 1× and 2×
/// and centres without half-pixel offsets (13 → 20, 11 → 16, 10 → 16, 15 → 22).
pub fn line(size: f32) -> f32 {
    ((size * 1.5) / 2.0).round() * 2.0
}

/// A text's line limit `n` at `scale`: fewer lines for bigger text (never under 2, or under `n`
/// for one line), the same at or below the theme's size.
pub fn fewer_lines(n: usize, scale: f32) -> usize {
    if n > 1 && scale > 1.0 {
        ((n as f32 / scale).round() as usize).clamp(2, n)
    } else {
        n
    }
}

/// The nearest even size, so a glyph centres on whole pixels inside an even box.
fn even(x: f32) -> f32 {
    (x / 2.0).round() * 2.0
}

/// `a` to `b` by `t`, mixed in sRGB like CSS, returned as Rune's linear colour.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let enc = |v: f32| {
        if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    let dec = |v: f32| {
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let m = |x: f32, y: f32| dec(enc(x) + (enc(y) - enc(x)) * t);
    Color {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: 1.0,
    }
}

fn family(f: Font) -> Family {
    match f {
        Font::Sans => Family::Sans,
        Font::Mono => Family::Mono,
        Font::Named(n) => Family::Named(n),
    }
}

/// Fill, border, padding and the rest of a box's props.
fn decorate(cx: &Ctx, mut e: El, p: &Props, row: bool) -> El {
    if let Some(g) = p.gap {
        e = e.gap(g);
    }
    if let Some([l, t, r, b]) = p.padding {
        e = e.pl(l).pt(t).pr(r).pb(b);
    }
    if p.indent {
        e = e.pl(cx.t.indent() + p.padding.map_or(0.0, |q| q[0]));
    }
    match &p.fill {
        Some(Fill::Solid(c)) => e = e.fill(cx.paint(c)),
        Some(Fill::Gradient(a, b)) => e = e.gradient(cx.paint(a), cx.paint(b)),
        None => {}
    }
    if let Some(r) = p.radius {
        e = e.radius(r);
    }
    if let Some((w, c)) = &p.border {
        e = e.border(*w, cx.paint(c));
    }
    match (p.width, p.height) {
        (Some(w), Some(h)) => e = e.size(w, h),
        (Some(w), None) => e = e.w(w),
        (None, Some(h)) => e = e.h(h),
        (None, None) => {}
    }
    if p.grow {
        e = e.grow(1.0).shrink();
        if row {
            e = e.w(0.0).grow(1.0);
        }
    }
    e = match p.align {
        Some(Align::Start) => e.items_start(),
        Some(Align::Center) => e.items_center(),
        Some(Align::End) => e.items_end(),
        Some(Align::Stretch) => e.items_stretch(),
        None => e,
    };
    e = match p.justify {
        Some(Justify::Start) => e.justify_start(),
        Some(Justify::Center) => e.justify_center(),
        Some(Justify::End) => e.justify_end(),
        Some(Justify::Between) => e.justify_between(),
        None => e,
    };
    if let Some(o) = p.opacity {
        e = e.opacity(o);
    }
    e
}

/// The app's letter in its accent colour, or the glyph the notification asked for. Glyphs are
/// usually Nerd Font icons, so they draw in the system's monospace font.
fn letter(cx: &Ctx, size: f32, radius: f32) -> El {
    let badge = el()
        .key("badge")
        .size(size, size)
        .radius(radius)
        .center()
        .fill(cx.accent);
    if cx.n.urgency >= 2 {
        return badge.child(
            icon(&rune_theme_opencode::icons::WARNING_BOLD, even(size * 0.6)).color(Color::WHITE),
        );
    }
    let glyph = even(size * 0.55);
    if let Some(g) = &cx.n.glyph {
        let font = TextStyle::new(glyph, 400.0)
            .lh(glyph)
            .family(Family::SystemMono);
        // A right margin of twice the shift moves the centred glyph left by the shift.
        let shift = crate::glyph::ink_shift(g) * glyph;
        return badge.child(
            text(g.clone())
                .font(font)
                .color(Color::WHITE)
                .mr(2.0 * shift),
        );
    }
    let ch =
        cx.n.app
            .chars()
            .find(|c| c.is_alphanumeric())
            .unwrap_or('•')
            .to_uppercase()
            .to_string();
    badge.child(
        text(ch)
            .font(TextStyle::new(glyph, 650.0).lh(glyph))
            .color(Color::WHITE),
    )
}

/// The app's icon, or its letter without one.
fn app_icon(cx: &Ctx, size: f32, radius: f32) -> El {
    match &cx.n.icon {
        Some(img) if cx.n.urgency < 2 => el()
            .key("icon")
            .size(size, size)
            .radius(radius)
            .pixels_fit(img.pixels(), SurfaceFit::Fit),
        _ => letter(cx, size, radius),
    }
}

fn badge(cx: &Ctx, p: &Props) -> Option<El> {
    let t = cx.t;
    let size = p.size.map_or(t.icon_size, |s| s * t.scale);
    let radius = p.radius.unwrap_or(t.badge_radius);
    match t.mark {
        Mark::Letter => Some(letter(cx, size, radius)),
        Mark::Icon => Some(app_icon(cx, size, radius)),
        Mark::Dot => {
            let d = even(8.0 * t.scale);
            Some(el().key("dot").size(d, d).radius(d / 2.0).fill(cx.accent))
        }
        Mark::Stripe | Mark::None => None,
    }
}

fn label<S: Cards>(cx: &Ctx, n: &Node, in_row: bool) -> Vec<El> {
    let k = cx.k;
    let t = cx.t;
    let id = cx.n.id;
    match n {
        Node::Box {
            row: is_row,
            props,
            children,
        } => {
            let base = if *is_row { row().center_y() } else { col() };
            let kids = children
                .iter()
                .flat_map(|c| label::<S>(cx, c, *is_row))
                .collect::<Vec<_>>();
            vec![decorate(cx, base, props, *is_row).children(kids)]
        }
        Node::Text {
            text: interp,
            props,
        } => {
            let content = interp.render(cx.n);
            if content.is_empty() {
                return Vec::new();
            }
            // A template's own sizes are at the theme's font-size; they scale with the card's.
            let size = props.size.map_or(t.font_size, |s| s * t.scale);
            let weight = if props.title {
                t.title_weight
            } else {
                props.weight.unwrap_or(400.0)
            };
            let style = TextStyle {
                family: family(props.font.unwrap_or(t.font)),
                tracking: props.tracking.unwrap_or(0.0),
                ..TextStyle::new(size, weight).lh(line(size))
            };
            let color = props.color.as_ref().unwrap_or(&t.text);
            let mut e = text(content).font(style).color(cx.ink(color));
            // In a column text wraps across the width; in a row it keeps its own width (one
            // line) unless it grows into the row's free space.
            // Bigger text gets fewer lines, so a card's text keeps about the height it has at
            // the theme's size and ends in an ellipsis rather than running off the stack.
            let lines = props
                .lines
                .map(|n| fewer_lines(n, t.scale))
                .or(in_row.then_some(1));
            e = match lines {
                Some(1) => e.truncate(),
                Some(n) => e.wrap().max_lines(n),
                None => e.wrap(),
            };
            e = if props.grow {
                e.grow(1.0).shrink().w(0.0).grow(1.0)
            } else if !in_row {
                e.w_full()
            } else {
                e.shrink()
            };
            if let Some(o) = props.opacity {
                e = e.opacity(o);
            }
            vec![e]
        }
        Node::Badge { props } => badge(cx, props).into_iter().collect(),
        Node::Icon { props } => vec![app_icon(
            cx,
            props.size.map_or(t.icon_size, |s| s * t.scale),
            props.radius.unwrap_or(t.badge_radius),
        )],
        Node::Image { props } => match &cx.n.image {
            Some(img) => {
                let (w, h) = (
                    props.width.or(props.size).unwrap_or(44.0) * t.scale,
                    props.height.or(props.size).unwrap_or(44.0) * t.scale,
                );
                vec![
                    el().key("image")
                        .size(w, h)
                        .radius(props.radius.unwrap_or(8.0))
                        .pixels_fit(img.pixels(), SurfaceFit::Fill),
                ]
            }
            None => Vec::new(),
        },
        Node::Progress { props } => match cx.n.value {
            // The theme's own colours: a faint track and a bar in its muted text colour (or
            // `color`), so it reads on any fill. No margins of its own: a line of text above
            // already leaves room, and in a row it centres with its neighbours.
            Some(v) => {
                let h = props.height.unwrap_or(6.0);
                let track = k.alpha(cx.paint(&t.text), 0.16);
                let fill = cx.paint(props.color.as_ref().unwrap_or(&t.muted));
                // A replaced note keeps its card, so a new value glides from the old one.
                let bar = el()
                    .key("bar")
                    .w_frac(v.clamp(0.0, 1.0))
                    .h_full()
                    .radius(h / 2.0)
                    .fill(fill)
                    .motion(Spring::new(0.15, 0.0));
                let e = el()
                    .key("progress")
                    .w_full()
                    .h(h)
                    .radius(h / 2.0)
                    .fill(track)
                    .clip_soft(0.0)
                    .child(bar);
                vec![decorate(
                    cx,
                    e,
                    &Props {
                        height: None,
                        ..props.clone()
                    },
                    false,
                )]
            }
            None => Vec::new(),
        },
        Node::Actions { props } => {
            if cx.n.actions.is_empty() {
                return Vec::new();
            }
            // Buttons take the theme's font, text colours and corners: the first is filled, the
            // rest are quiet.
            let size = (t.font_size - t.scale).max(10.0 * t.scale);
            let style = TextStyle {
                family: family(t.font),
                ..TextStyle::new(size, 550.0).lh(line(size))
            };
            let radius = (t.radius * 0.6).clamp(2.0, 8.0);
            let buttons = cx.n.actions.iter().enumerate().map(|(i, (key, label))| {
                let key = key.clone();
                let primary = i == 0;
                let mut b = row()
                    .key(format!("action-{i}"))
                    .role("button")
                    .label(label.clone())
                    .h(even(26.0 * t.scale))
                    .px(10.0 * t.scale)
                    .center_y()
                    .radius(radius)
                    .hover(k.c.hover)
                    .press(k.c.pressed)
                    .cursor(Cursor::Pointer)
                    .on_press(move |s: &mut S| s.invoke(id, &key))
                    .child(
                        text(label.clone())
                            .key("label")
                            .font(style)
                            .color(cx.ink(if primary { &t.text } else { &t.muted })),
                    );
                if primary {
                    // Rune blends in linear light; `Kit::alpha` re-solves the opacity so it looks
                    // like 8% does in a browser instead of three times that.
                    b = b.fill(k.alpha(cx.paint(&t.text), 0.08));
                }
                b
            });
            let mut r = row().key("actions").gap(4.0).flex_wrap().children(buttons);
            r = decorate(cx, r, props, true);
            vec![r]
        }
        Node::Close { props } => {
            let size = even(props.size.unwrap_or(22.0) * t.scale);
            let glyph = even(size * 0.64);
            let color = props.color.as_ref().unwrap_or(&t.faint);
            vec![
                row()
                    .key("close")
                    // The hit area overhangs the padding, so the glyph itself lines up with the
                    // card's edge like everything else does.
                    .mr(-(size - glyph) / 2.0)
                    .role("button")
                    .label("Dismiss")
                    .size(size, size)
                    .center()
                    .radius(props.radius.unwrap_or(t.badge_radius.min(6.0)))
                    .hover(k.c.hover)
                    .press(k.c.pressed)
                    .cursor(Cursor::Pointer)
                    .on_press(move |s: &mut S| s.dismiss(id))
                    .child(icon(k.glyph.close, glyph).color(cx.ink(color))),
            ]
        }
        Node::Spacer => vec![spacer()],
        Node::If {
            cond,
            then,
            otherwise,
        } => {
            let branch = if cx.has(cond) { then } else { otherwise };
            branch
                .iter()
                .flat_map(|c| label::<S>(cx, c, in_row))
                .collect()
        }
    }
}

/// The template's elements for this card, in order.
pub fn render<S: Cards>(cx: &Ctx, nodes: &[Node]) -> Vec<El> {
    nodes
        .iter()
        .flat_map(|n| label::<S>(cx, n, false))
        .collect()
}
