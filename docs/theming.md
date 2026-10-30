# Configuring and theming Hokum

Everything about Hokum lives in one file, `~/.config/hokum/config.kdl` (`$XDG_CONFIG_HOME`).
It starts from a theme and overrides whatever you like:

```kdl
theme "signal"               // start from a theme (default: "capsule")
language "fr"                // this computer's words: en, fr (default: the session's)

behaviour {                  // only here, never in a theme
    position "bottom-right"
    stay 6                   // seconds, when the app doesn't say
    show-at-most 5
    pause-on-hover #true
    do-not-disturb #false
    appearance "system"      // system, light, dark
    clock "auto"             // auto, 12h, 24h
    check-for-updates #true  // offer newer Hokum builds with a card
    sound #false             // #true, or a sound's name or file, to chime for new cards
}

tokens {                     // anything about the look
    radius 10
    border "none"
}

rule app="Slack" { tokens { accent "#d38be0" } }
rule app="Spotify" { hide }
```

The settings window edits part of this file: picking a theme writes `theme "…"`, the
appearance switch and the Behaviour page write `behaviour`, the Language row writes
`language "…"`, and each app's page under
Applications writes a `rule app="…"` block (mute, how long cards stay, breaking through Do not
disturb). Changes keep your comments and layout, and choosing a default value removes the
setting rather than repeating it. The look itself
(`tokens`, `template`, styling in rules) isn't in the window: write it in the file, or have a
coding agent write a theme (below). Edit the file by hand and the window and the notifications
update as you save; problems show in the window with line numbers, and Hokum keeps drawing with
the last config that worked. `hokum check` reports the same problems in a terminal.

**Themes** use the same format. Built-in ones are fixed: [Soft](../themes/soft.kdl),
[Crisp](../themes/crisp.kdl), [Capsule](../themes/capsule.kdl), [Ribbon](../themes/ribbon.kdl),
[Terminal](../themes/terminal.kdl), [Bubble](../themes/bubble.kdl), [Signal](../themes/signal.kdl),
[Pill](../themes/pill.kdl) and [Omarchy](../themes/omarchy.kdl), whose colours follow the current Omarchy
theme. Referencing one keeps you on its latest version with your
overrides on top. Your own themes live in `~/.config/hokum/themes/`: `theme "ember"` refers to
`themes/ember.kdl`, and any path works too (`theme "~/dotfiles/hokum/work.kdl"`). A theme can
itself start from another theme. The settings window shows your themes beside the built-in ones,
and a theme by path as a Custom entry.

A theme is only looks: `behaviour` is yours, set in config.kdl (a theme with a `behaviour`
block is reported as a problem).

**Making a theme** is a job for your coding agent: **Make your own theme** under Appearance in
the settings window has a prompt to copy. It points the agent at
[hokum.app](https://hokum.app) (the same guide as [themes.md](themes.md) here), a complete
guide to the format (every token, the template language, rules, two worked examples, and how to
check a theme with `hokum check` and render it headlessly).

A theme names itself with `name "…"` (shown in the gallery). The built-in card is a template,
[default.kdl](../themes/default.kdl), which every theme falls back to.

A **script** (`~/.config/hokum/script.rhai`) is separate because it's code: it sees every
notification first and can rewrite, route or drop it.

## Modules

Sound and Bluetooth are on unless config.kdl turns them off (themes don't set them):

```kdl
modules {
    sound #true      // PipeWire: output, input, apps, devices, routing
    bluetooth #false // BlueZ: devices, pairing, adapters
    osd #true        // a Volume card when the output changes outside Hokum
    brightness #true // a Brightness card when the screen's backlight changes
    bluetooth-notifications #true // devices connecting, batteries running low
}
```

The end of each module's advanced pane in the window turns it off, and its empty section turns
it back on; either writes this block. `osd`, `brightness` and `bluetooth-notifications` only
decide which cards show, and are set here.

## Language

Hokum's own words (the window, the panel, the cards it draws itself) follow your session's
language, and nothing else: an app's notification is shown as the app wrote it, and device and
profile names come from PipeWire and BlueZ as they are.

```kdl
language "fr"    // en, fr; leave it out to follow the session
```

Leave it out and Hokum reads `$LANGUAGE`, `$LC_ALL`, `$LC_MESSAGES` then `$LANG`, and falls back
to English. Only some languages are translated so far (English and French); the Language row
under Files in the settings window does the same as this line, with **Auto** to follow the
session. Translation is in `src/i18n.rs`: English is the key, so a new language is a new column
there and a word in `Lang` (fixed sentences in `FR`, failures that name a device or an app in
`FAILED`). The problems `hokum check` and the window report stay in English, like a compiler's,
and so does `hokum --help`.

## Behaviour

Behaviour comes only from config.kdl; anything unset uses the default (top right, 6 seconds, 5
cards, pause on hover, Do not disturb off, system appearance, check for updates). The settings window shows
appearance under Appearance and the rest under Behaviour.

| Key | Value |
|---|---|
| `position` | `top-right`, `top-left`, `top-center`, `bottom-right`, `bottom-left`, `bottom-center` (on the focused screen) |
| `stay` | seconds ordinary notifications stay when the app doesn't say |
| `show-at-most` | cards shown at once; the rest wait behind a "N more" pill |
| `pause-on-hover` | `#true`: timers wait while the pointer is over a card |
| `do-not-disturb` | `#true`: only critical notifications, and apps whose rule has `interrupt`, show |
| `appearance` | `system`, `light`, `dark` |
| `clock` | `{time}` on cards: `12h` (`7:54 PM`), `24h` (`19:54`), or `auto` (the default), which follows Omarchy's bar clock, GNOME's clock setting, or else your locale (`en_US` and other AM/PM locales get 12-hour). A theme can force one with `{time|12h}` or `{time|24h}` |
| `sound` | `#true`: play a sound when a new card shows (off by default). A name from the sound theme (`"message-new-instant"`, the default) or an absolute path to an `.oga`, `.ogg` or `.wav` file picks the sound and turns it on |
| `sound-critical` | the sound for critical notifications (default `"dialog-warning"`) |
| `check-for-updates` | `#true`: fetch the release manifest hourly (30 minutes on dev) and show a card when a different build is out; nothing installs without a click. `#false` turns it off |

### Sounds

With `sound` on, every new card plays a sound: not updates to a card already showing (progress,
the same tagged notification again), not cards Do not disturb holds back or a rule hides, and
never Hokum's own volume and brightness cards. Names come from the desktop's sound theme
(`gtk-sound-theme-name`, else `freedesktop`; the `sound-theme-freedesktop` package has the
defaults), and Hokum plays them with `pw-play`, else `paplay` (or `aplay` for WAV). An app's own
`sound-file` or `sound-name` hint replaces the configured sound and `suppress-sound` silences
it; a rule's `sound` wins over the app. A burst of notifications makes one sound.
`hokum check` reports a sound it can't find.

```kdl
behaviour {
    sound "message-new-instant"
    sound-critical "~/sounds/alarm.oga"
}
rule app="Slack" { sound "bell" }
rule app="Spotify" { sound #false }
```

There is no hook to run a command for every notification: the text comes from any app or web
page, and it would end up in the command.

## Tokens and templates

Text follows the desktop's text scale on top of `font-size`: GNOME's `text-scaling-factor`
(Large Text), Plasma's font size over its default 10 pt, or Omarchy's `[font] base-size` over its
default 12. Stock desktops scale by 1, and changes apply live. `font-size` itself stays a
config.kdl setting, in points (logical pixels).

`tokens { ... }` sets the look of every card (`width`, `radius`, `padding`, `fill`, `text`,
`border`, `accent`, `shadow`, `mark`, `enter` and more); `tokens "dark" { ... }` and
`tokens "light" { ... }` set them for one appearance. `template "card" { ... }` lays out a
card's contents from elements like `row`, `col`, `text "{summary}"`, `badge`, `actions` and
`if "body" { ... }`. Both work in config.kdl as well as in a theme, so small tweaks can sit on
top of a built-in theme:

```kdl
theme "capsule"
tokens {
    radius 10
    mark "letter"
}
```

[themes.md](themes.md) is the full reference: every token and its values, colours and the
palette, every element, property, field, filter and condition.

## Rules

A rule matches notifications and changes them. Matchers are `app`, `summary`, `body` (case
insensitive, `*` matches anything) and `urgency`; a rule with several matchers needs them all.
Every matching rule applies in order, a theme's before the config's, so later rules win.
Actions: `template "…"` (from the theme, anything it starts from, or this file), `tokens { … }`, `timeout` (seconds or `"never"`), `hide`, and
`interrupt` (show even during Do not disturb), and `sound` (`#false` silences the app, `#true`
gives it the usual sound even when sounds are off, a name or path picks its own).

The settings window manages rules that match one app by its exact name (`rule app="Slack"`),
setting `hide`, `timeout` and `interrupt`; anything else in such a rule (`tokens`, `template`)
stays as you wrote it. Rules matching text or urgency are yours to edit in the file, and the
window counts them.

```kdl
rule app="Slack" summary="*incident*" {
    template "alert"          // a template from this theme
    tokens { fill "danger/20" }
    timeout "never"           // or seconds
}
rule app="Spotify" { hide }
```

Critical notifications get a `danger` border and accent before any rule runs; a rule can
change them back.

## Scripts

`~/.config/hokum/script.rhai` defines `notify(n)`, which runs before rules for every
notification. Return `n` (changed or not) to show it, or `false` to drop it. **Create script**
in the settings window writes a starter.

```rhai
fn notify(n) {
    if n.app == "Spotify" { return false; }
    if n.body.contains("deploy failed") {
        n.urgency = 2;
        n.template = "alert";
    }
    n.summary.replace("[anoma-ly] ", "");   // string methods edit in place
    n
}
```

| Field | Type |
|---|---|
| `n.app`, `n.summary`, `n.body`, `n.context` | strings |
| `n.urgency` | 0 low, 1 normal, 2 critical |
| `n.value` | 0.0 to 1.0, or `()` |
| `n.timeout` | seconds, `0` until dismissed, `()` for the default |
| `n.template` | a template name, or `()` |
| `n.actions` | `[#{ key: "reply", label: "Reply" }, ...]` |
| `n.desktop` | the app's desktop entry (`"org.gnome.Nautilus"`); set it, or change `n.app`, and the card takes that app's icon |
| `n.icon` | the icon's file; set an icon name or a path to use another |
| `n.id` | read only |

Scripts have an operation budget: one that loops forever fails that notification, which then
shows unchanged, and Hokum carries on.

## Icons and pictures

`icon` and the `icon` mark show the app's icon from your icon theme: the one the settings
portal reports (GNOME's or KDE's setting), else KDE's `kdeglobals`, GTK's `settings.ini`, or
your desktop's default, then the themes it inherits from, hicolor and Adwaita. Hokum finds the
app, most specific first:

1. A browser's notification from a site that has a web app launcher (`--app=https://…`, as
   Omarchy's and Linux Mint's Web Apps write) is that web app's, with its name and icon. From any
   other site it's the browser's, with the site beside its name (`{context}`).
2. The icon the notification names (`app_icon`).
3. Its `desktop-entry`.
4. Its process: a Flatpak's or Snap's ID, the systemd unit the desktop launched it in
   (`app-…-ID-….scope`), or the AppImage it runs. Not for `notify-send` or a script in a
   terminal: those aren't the terminal.
5. Its name, against desktop entries' IDs (in any case), `StartupWMClass` and `Name=`.

`image` shows the picture a notification carries (`image-data` or `image-path`): an avatar, an
album cover, a screenshot; a picture that is just the app's icon again (as Electron apps send)
isn't shown twice. A script can set `n.desktop` or `n.icon` when none of this finds the app.
