# Writing a Hokum theme

This page is written for coding agents (and people) making a custom theme for
[Hokum](https://hokum.app), a notification daemon for Wayland. Hokum's settings window only
picks one of the built-in themes and sets behaviour; everything about the look of a custom
theme lives in files you write. This page has everything needed to write one from
scratch and check it without a display.

## The short version

1. Write `~/.config/hokum/themes/<id>.kdl` (KDL v2). Pick an `<id>` that isn't a built-in
   theme's (`soft`, `crisp`, `capsule`, `ribbon`, `terminal`, `bubble`, `signal`, `pill`).
2. Point the config at it: in `~/.config/hokum/config.kdl`, set `theme "<id>"` (change only
   that line; keep everything else in the file).
3. Run `hokum check ~/.config/hokum/themes/<id>.kdl` until it says OK, then render it headlessly
   in a test folder, look at the PNG, fix, repeat ([Checking your work](#checking-your-work)).

A running Hokum notices the change within a second and redraws; there's nothing to restart.

## Files

Everything lives in `$XDG_CONFIG_HOME/hokum/` (`~/.config/hokum/` when `XDG_CONFIG_HOME` is
unset or not absolute):

| File | What it is |
|---|---|
| `config.kdl` | The user's config. Starts from a theme with `theme "…"` and overrides anything on top |
| `themes/<id>.kdl` | User themes; `theme "<id>"` refers to `themes/<id>.kdl` |
| `script.rhai` | An optional script that rewrites or drops notifications |

A theme and the config are the same format. The config is a theme of its own, merged on top of
the theme it names, so anything below works in either file. The one difference: only the config
has a `behaviour` block (where notifications show, for how long, light or dark); a theme is only
looks. Prefer putting the look in a theme file and leaving the config to the user's choices.

### Theme references and chains

`theme "…"` (in the config or in a theme) names the theme to start from:

- a built-in id: `theme "signal"` (built-ins win: a `themes/signal.kdl` is never used);
- a file stem in the themes folder: `theme "ember"` → `themes/ember.kdl`;
- a path, when it contains `/` or starts with `~/`: `theme "~/dotfiles/hokum/work.kdl"`.

A theme can itself start from another theme (`theme "capsule"` at the top of `themes/ember.kdl`),
up to four deep; a theme that ends up starting from itself is an error. Without `theme`, a theme
starts from the defaults (the built-in card template and the token defaults below), which is
usually what you want for a theme written from scratch.

How layers merge, bottom to top (defaults → base themes → the theme → the config):

- `tokens`, `tokens "dark"` and `tokens "light"` merge key by key; the top wins.
- `template "name"` replaces a template of the same name.
- `rule`s accumulate: lower layers' rules run first, so later ones win.

## File structure

A theme file has these top-level nodes, in any order. Anything else is an error.

```kdl
name "Ember"            // shown in the settings gallery; defaults to the id
theme "capsule"         // optional: start from another theme
tokens { … }            // the look, for every card
tokens "dark" { … }     // overrides when the appearance is dark
tokens "light" { … }    // overrides when the appearance is light
template "card" { … }   // the layout of a card's contents (any number of templates)
rule app="Slack" { … }  // match notifications and change them (any number)
```

A theme can't have a `behaviour` block; that's an error pointing to config.kdl.

KDL notes: strings are quoted (`"#ff8a4c"`), booleans are `#true`/`#false`, several nodes can
share a line when separated by `;`, and `//` starts a comment. Properties are `key=value` with
no spaces around `=`.

A file with any problem is rejected as a whole and Hokum keeps drawing with what it had. Every
problem is reported with a line number (see [Checking your work](#checking-your-work)).

## Tokens

`tokens { … }` sets values for every card. `tokens "dark" { … }` and `tokens "light" { … }` set
values for one appearance and apply after the plain block (across the whole chain, so a base
theme's `tokens "dark"` beats a plain `tokens` higher up; override in the same kind of block).
Anything unset keeps the value from below, ultimately the default.

Sizes here and in templates are points: logical pixels, like CSS `px`, so `font-size 13` is
13 pixels at a scale of 1. A desktop's "11 pt" is about 14.7 of them.

| Token | Value | Default | What it does |
|---|---|---|---|
| `width` | points | `360` | Card width (also the overlay's width) |
| `radius` | points | `16` | Card corner radius; `999` for a pill |
| `padding` | 1, 2 or 4 numbers | `12` | Inside the card: all; vertical horizontal; top right bottom left |
| `gap` | points | `6` | Between a card's top-level template elements |
| `spacing` | points | `10` | Between cards |
| `badge-radius` | points | `7` | Corner radius of badges and icons |
| `icon-size` | points | `20` | Badge and icon size |
| `font` | `"sans"`, `"mono"` | `"sans"` | Typeface for all text: Inter or IBM Plex Mono. Other names fall back to Inter |
| `font-size` | points | `13` | Text size. The card's other sizes (`size=` on text, buttons, `close`) scale with it, and the desktop's text scale multiplies it (see below) |
| `title-weight` | 100 to 900 | `550` | The weight `weight="title"` uses |
| `fill` | colour, or two colours | `"layer"` | Card background; two colours are a top-to-bottom gradient |
| `text` | colour | `"text"` | Main text colour (templates' `color="text"`) |
| `muted` | colour | `"muted"` | Secondary text colour (`color="muted"`) |
| `faint` | colour | `"faint"` | Tertiary text colour (`color="faint"`) |
| `border` | width and colour, or `"none"` | `1 "border"` | Card border; `border 0` is also none |
| `accent` | colour | `"app"` | The card's accent: badge fill, dot, stripe, glow, `color="accent"` |
| `shadow` | `"flat"`, `"soft"`, `"deep"`, `"glow"` | `"soft"` | `glow` is a halo in the accent colour |
| `mark` | `"icon"`, `"letter"`, `"dot"`, `"stripe"`, `"none"` | `"icon"` | What `badge` draws (below) |
| `enter` | `"slide"`, `"fade"`, `"scale"`, `"drop"` | `"slide"` | How cards arrive and leave |
| `motion` | seconds and bounce (0 to 1) | `0.38 0.12` | The spring cards move on |

Marks:

- `icon`: the app's icon from the icon theme, or its letter on an accent tile when it has none.
- `letter`: the app's first letter, white on an accent tile.
- `dot`: an 8 point accent dot.
- `stripe`: no badge; a 3 point accent bar runs down the card's left edge beside the content.
- `none`: no badge.

On critical notifications, `icon` and `letter` show a white warning glyph on the accent tile
instead.

### Colours

Any colour value is one of:

- a literal: `"#rgb"`, `"#rrggbb"` or `"#rrggbbaa"`;
- a palette name, which follows the light or dark appearance:

| Name | Colour |
|---|---|
| `layer`, `layer-2`, `layer-3` | Raised surfaces, increasingly strong (`layer` is the default card fill) |
| `page`, `base`, `deep` | Background surfaces |
| `text`, `muted`, `faint` | Text colours. In a template, these mean the theme's tokens of those names |
| `border`, `border-strong` | Hairlines |
| `hover` | The translucent hover wash |
| `accent` | The card's accent (the `accent` token) |
| `app` | A stable colour per sending app (a mid-tone hue hashed from the app name) |
| `accent-deep`, `accent-deeper` | The accent darkened until white text on it reads well (WCAG 4.5:1); for coloured cards |
| `danger`, `warning`, `success`, `info` | Status colours |
| `white`, `black`, `transparent` | Themselves |

A palette name takes an opacity as a percentage after `/`: `"accent/30"`, `"black/50"`,
`"white/80"`. Literals take their alpha from `#rrggbbaa` instead (`"#ff8a4c4d"` is 30%).
Translucent text colours are mixed over the card's fill, so `"white/80"` on a coloured card
looks the way it would in a browser.

A literal colour doesn't change between light and dark. When a theme uses literal colours, give
it a `tokens "light"` block (or `tokens "dark"`) with colours for the other appearance, as
[Terminal](../themes/terminal.kdl) does.

## Templates

A template lays out a card's contents. The card itself (fill, border, shadow, padding, radius,
the stripe mark) comes from the tokens; the template fills it. The top-level elements of a
template stack vertically, `gap` apart.

`template "card"` draws every notification unless a rule or script picks another template by
name. A theme without templates uses the built-in card, [default.kdl](../themes/default.kdl):

```kdl
template "card" {
    row gap=8 align="center" {
        badge
        text "{source}" size=11 weight=550 color="muted" lines=1 grow=#true
        text "{time}" size=11 color="faint"
        close
    }
    row gap=10 {
        col gap=2 grow=#true {
            if "summary" { text "{summary}" weight="title" }
            if "body" { text "{body}" color="muted" lines=8 }
            if "value" { progress }
        }
        if "image" { image width=44 height=44 radius=8 }
    }
    actions
}
```

When a template is asked for by name, Hokum looks in the theme (with its chain), then in
default.kdl, then falls back to the theme's `card`, then the built-in `card`.

### Elements

| Element | What it draws |
|---|---|
| `row { … }` | Children side by side, centred vertically unless `align` says otherwise |
| `col { … }` | Children stacked |
| `text "…"` | Text with `{field}` holes; draws nothing when it comes out empty |
| `badge` | The app mark from the `mark` token (nothing for `stripe` and `none`) |
| `icon` | The app's icon, or its letter on an accent tile; ignores `mark` |
| `image` | The notification's own picture (avatar, album art, screenshot); nothing if it has none |
| `progress` | A progress bar, only when the notification has a value |
| `actions` | The notification's buttons, only when it has some; the first is filled |
| `close` | The dismiss button |
| `spacer` | Takes the free space in a row |
| `if … { } else { }` | Draws its children only when the condition holds |

### Properties

Properties are written on the element: `text "{summary}" size=15 weight="title"`.

| Property | On | Value |
|---|---|---|
| `gap` | row, col, actions | points between children |
| `padding` | row, col, actions, progress | points, all sides (one number) |
| `padding-x`, `padding-y` | row, col, actions, progress | points, left and right / top and bottom |
| `fill` | row, col, actions, progress | one colour |
| `border` | row, col, actions, progress | a colour (1 point wide) |
| `radius` | row, col, actions, progress, badge, icon, image, close | points |
| `width`, `height` | row, col, image, progress (`height`) | points |
| `size` | text, badge, icon, image, close | points (text size, or a square) |
| `grow` | row, col, text | `#true`: take the free space in a row |
| `indent` | row, col, actions, progress | `#true`: start past the badge, lining up with text beside it |
| `align` | row, col | `"start"`, `"center"`, `"end"`, `"stretch"`: across the direction |
| `justify` | row, col | `"start"`, `"center"`, `"end"`, `"between"`: along the direction |
| `opacity` | row, col, text | 0 to 1 |
| `weight` | text | 100 to 900, or `"title"` for the `title-weight` token (default 400) |
| `color` | text, close, progress | colour (defaults: text `text`, close `faint`, progress bar `muted`) |
| `lines` | text | most lines shown; `1` truncates with an ellipsis |
| `font` | text | `"sans"` or `"mono"` |
| `tracking` | text | letter spacing in points |

Unknown property names are errors. A known property on an element that doesn't use it is
ignored.

Layout rules worth knowing:

- Text in a column wraps across the column's width. Text in a row stays on one line (and
  truncates) unless it has `lines`; give the text that should take the leftover space
  `grow=#true`, or it keeps its natural width.
- `padding-x`/`padding-y` after `padding` adjust it; `padding` after them replaces both.
- Defaults: `badge`/`icon` use `icon-size` and `badge-radius`; `image` is 44×44 with radius 8
  (`width`/`height` beat `size`); `close` is 22 with radius `min(badge-radius, 6)`;
  `progress` is 6 high on a track of the `text` colour at 16%.
- Action buttons are 26 points high and take the card's font, `font-size - 1` and a radius of
  `radius × 0.6` (2 to 8).
- A template's sizes are at the theme's own `font-size`. When config.kdl or a rule sets another,
  or the desktop's text is bigger, text sizes, buttons and `close` scale with it, so `text
  "{time}" size=11` stays in proportion to the title.
- A row's or column's `fill` doesn't clip to the card's corners. To fill a strip edge to edge,
  give the card a small `padding` and the strip a `radius` of the card's radius minus that
  padding (see Ticket below).

### Fields and filters

Text holes: `{app}`, `{source}` (the app, plus ` · channel` when there's a context), `{context}`
(a Slack channel and the like), `{summary}`, `{body}`, `{time}` (when it arrived: `19:54` or `7:54 PM`, as the user's `clock` setting says),
`{urgency}` (`low`, `normal`, `critical`), `{value}` (`62%`), `{id}`.

Filters follow a `|` and chain: `{app|upper}`, `{summary|lower}`, `{app|initial}` (first letter
or digit, upper case), `{body|trim}`, `{app|initial|lower}`. `{time|12h}` and `{time|24h}` force a clock whatever
the user's setting (they only work on `time`); leave `{time}` plain unless the design needs one.
Write `{{` and `}}` for literal braces. Unknown fields and filters are errors.

### Conditions

| Condition | Holds when |
|---|---|
| `if "summary"` (any field) | the field isn't empty; `"value"` and `"context"` when present |
| `if "icon"`, `if "image"`, `if "actions"` | the card has an icon, a picture, buttons |
| `if urgency="critical"` | the urgency is `low`, `normal` or `critical` |
| `if app="Slack*"` | the app matches the pattern (case insensitive, `*` matches anything) |
| `if not="body"` | the field (or `icon`, `image`, `actions`) is empty or missing |

One condition per `if`; nest `if`s to require several. An `else { … }` must be the node right
after its `if`.

## Rules

A rule matches notifications and changes them. Matchers are properties: `app`, `summary`,
`body` (case insensitive patterns, `*` matches any run of characters) and `urgency` (`low`,
`normal`, `critical`). A rule needs all of its matchers; a rule with none matches everything.
Every matching rule applies, in order.

| Action | Effect |
|---|---|
| `template "name"` | Draw with another template: one from this file, any theme it starts from, or default.kdl |
| `tokens { … }` | Override tokens for these cards (any token above) |
| `timeout 10` / `timeout "never"` | How long they stay; `0` is also never |
| `hide` | Don't show them |
| `interrupt` | Show them even during Do not disturb |

```kdl
rule urgency="critical" {
    template "alert"
    timeout "never"
}
rule app="Slack" summary="*incident*" {
    tokens { fill "danger/20"; border 1 "danger" }
}
```

Before any rule runs, critical notifications get `border 1 "danger"` and `accent "danger"`; a
rule can change them back (Signal sets `border "none"` on critical cards because they're
already red).

A theme's rules are for looks. Rules about which apps to mute or how long they stay belong in
the user's config; the settings window manages `rule app="…"` blocks there.

## Behaviour

Behaviour isn't part of a theme. `behaviour { … }` lives only in `config.kdl`: `position`
(`"top-right"`, `"top-left"`, `"top-center"`, `"bottom-right"`, `"bottom-left"`,
`"bottom-center"`), `stay` (seconds),
`show-at-most` (1 to 20), `pause-on-hover`, `do-not-disturb` and `check-for-updates`
(`#true`/`#false`),
`appearance` (`"system"`, `"light"`, `"dark"`) and `clock` (`"auto"`, `"12h"`, `"24h"`). The settings window writes it, and anything
unset uses Hokum's defaults (top right, 6 seconds, 5 cards, pause on hover, system appearance).
A theme should look right in both light and dark, since the user picks.

## Scripts

`script.rhai` is code that sees every notification before rules run and can rewrite it, pick
its template (`n.template = "alert"`), change how long it stays, or drop it. A theme doesn't
need one. See [Scripts in theming.md](theming.md#scripts).

## Icons and pictures

`icon` and the `icon` mark look the app's icon up in the user's icon theme (GTK's
`gtk-icon-theme-name`, its parents, then hicolor) from the notification's `app_icon`, its
`desktop-entry`, or the app's name; without one they draw the app's letter. `image` is the
picture the notification carries (`image-data` or `image-path`), drawn to fill its box. Emoji in
text draw in colour.

## Example: Ember

A tokens-only theme: warm charcoal cards with the app's letter and a glowing accent, the
built-in card layout, and its own light colours.

```kdl
// Ember: warm charcoal cards with the app's letter and a glowing accent.
name "Ember"

tokens {
    width 380
    radius 14
    badge-radius 8
    padding 14
    gap 6
    spacing 12
    icon-size 22
    fill "#2a1f1c" "#1d1614"
    text "#f6e7dc"
    muted "#d2b8a8"
    faint "#9a8073"
    border 1 "#ff8a4c40"
    accent "#ff8a4c"
    shadow "glow"
    mark "letter"
    title-weight 650
    enter "scale"
    motion 0.32 0.18
}

tokens "light" {
    fill "#fff6ef" "#fde9dc"
    text "#3a2418"
    muted "#6e4b3a"
    faint "#a07f6e"
    border 1 "#e9b89c"
    accent "#e8622a"
    shadow "soft"
}

rule urgency="critical" {
    tokens {
        accent "#ff5a4f"
        border 1 "#ff5a4f"
    }
}
```

## Example: Ticket

A theme with its own layouts: a tinted header strip naming the app, the message below, and a
separate template for critical notifications.

```kdl
// Ticket: a tinted header strip naming the app, the message below it, and an
// alert layout for critical notifications.
name "Ticket"

tokens {
    width 360
    radius 12
    padding 4
    gap 0
    spacing 10
    icon-size 18
    badge-radius 5
    border 1 "border"
    shadow "soft"
    mark "icon"
    enter "drop"
    motion 0.34 0.16
}

template "card" {
    row gap=8 padding-x=10 padding-y=6 radius=8 fill="accent/16" {
        badge
        text "{app|upper}" size=11 weight=700 tracking=0.6 color="accent" grow=#true
        if "context" { text "{context}" size=11 color="muted" }
        text "{time}" size=11 color="faint"
        close size=20
    }
    col gap=6 padding-x=10 padding-y=8 {
        if "summary" { text "{summary}" weight="title" lines=2 }
        if "body" { text "{body}" color="muted" lines=5 }
        if "value" {
            row gap=8 {
                col grow=#true { progress }
                text "{value}" size=11 color="muted"
            }
        }
        if "image" { image width=328 height=160 radius=8 }
        actions
    }
}

template "alert" {
    row gap=10 padding=10 radius=8 fill="danger/18" align="start" {
        badge
        col gap=2 grow=#true {
            text "{summary}" weight=700
            if "body" { text "{body}" color="muted" lines=4 }
        }
        close size=20
    }
    if "actions" {
        col padding-x=10 padding-y=8 { actions }
    }
}

rule urgency="critical" {
    template "alert"
    timeout "never"
}
```

## Checking your work

First, `hokum check` loads a file exactly as Hokum does and lists every problem with its line,
exiting non-zero if there are any:

```sh
hokum check ember.kdl          # a theme file, with the themes it starts from
hokum check                    # ~/.config/hokum/config.kdl, its theme and script.rhai
hokum check path/to/config.kdl # another config
```

A problem reads ``ember.kdl: line 7: unknown token `colour` ``; a clean file prints
`ember.kdl: OK (theme "Ember")`.

Fix everything it reports before looking at pictures. Then render: Hokum renders headlessly
with no display or D-Bus. Work in a throwaway config folder so the user's setup isn't touched
until the theme is right:

```sh
mkdir -p /tmp/hokum-check/hokum/themes
cp ember.kdl /tmp/hokum-check/hokum/themes/ember.kdl
printf 'theme "ember"\n' > /tmp/hokum-check/hokum/config.kdl

# The cards: a Slack message with buttons, a critical battery warning, a volume bar and a download.
XDG_CONFIG_HOME=/tmp/hokum-check hokum service --snap /tmp/hokum-check/cards.png --scene demo
XDG_CONFIG_HOME=/tmp/hokum-check hokum service --snap /tmp/hokum-check/cards-light.png --scene demo --theme light

# The settings window: the theme in the gallery and preview, and any problems in red.
XDG_CONFIG_HOME=/tmp/hokum-check hokum --snap /tmp/hokum-check/settings.png --scene hokum
```

Then look at the PNGs. Check both appearances, that text is readable on the fill, that nothing
is clipped or truncated that shouldn't be, and that the critical card stands out.

A theme with problems doesn't load at all: the snapshot falls back to Capsule, so a render that
looks like the default means run `hokum check` (`hokum service --snap` also prints problems to
stderr, and the settings window lists them in red). The demo scene has no notification with a
picture, so `image` isn't exercised there.

When it looks right, copy the file to `~/.config/hokum/themes/<id>.kdl` and set
`theme "<id>"` in `~/.config/hokum/config.kdl`, keeping the rest of that file as it is.
