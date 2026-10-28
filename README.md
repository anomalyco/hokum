# Hokum

Notifications for Wayland that look like they belong on a computer from this decade.

Hokum is a freedesktop notification daemon (`org.freedesktop.Notifications`) for compositors
with `zwlr_layer_shell_v1`: niri, sway, Hyprland, river and others. It draws its cards with
[Rune](https://github.com/anomalyco/rune), our Rust UI framework, so they slide in, restack on
springs and get out of the way. A settings window picks a theme and sets how they behave.

## Install

From the AUR:

```sh
paru -S hokum-bin
```

On Debian and Ubuntu (use `linux-arm64-deb` on ARM):

```sh
curl -fLo /tmp/hokum.deb https://anoma.ly/release/hokum/desktop/stable/linux-x64-deb
sudo apt install /tmp/hokum.deb
```

On Fedora and openSUSE (use `linux-arm64-rpm` on ARM):

```sh
curl -fLo /tmp/hokum.rpm https://anoma.ly/release/hokum/desktop/stable/linux-x64-rpm
sudo dnf install /tmp/hokum.rpm
```

Or download the `hokum` executable from [Releases](https://github.com/anomalyco/hokum/releases)
and install it on your `PATH` together with the systemd unit, D-Bus activation file and launcher
entry from [`packaging/`](packaging). Development builds of `dev` are on the
[dev channel](https://anoma.ly/release/hokum/desktop/dev.json).

Hokum starts through D-Bus activation on the first notification. To start it with your session
instead, and stop another daemon (mako, dunst, swaync) holding the name:

```sh
systemctl --user enable --now hokum.service
```

Some desktop shells are the notification server themselves rather than a separate daemon
(Omarchy's, and shells like it), and the unit can't stop them: turn off the shell's own
notifications first. On Omarchy, see [docs/omarchy.md](docs/omarchy.md).

The first time it runs, Hokum shows one welcome card of its own with a button into Settings. It
remembers that in `~/.local/state/hokum/welcomed`.

## Use

```sh
hokum                     # the settings window (starts Hokum if it isn't running)
hokum service [--replace] # run in the background (what hokum.service starts)
hokum check [path]        # report problems in config.kdl, or in a theme or config file
hokum dismiss [--all]     # close the newest card, or every card
hokum invoke              # do what clicking the newest card does
hokum dnd [on|off|toggle] # print Do not disturb, or set it (--json, --watch for bars)
hokum --version
```

`dismiss`, `invoke` and `dnd` are for keybindings and bars, like `makoctl`. `hokum dnd on` saves
to `config.kdl` just as the settings window's switch does. They call `app.hokum.Daemon` on the
session bus (`Dismiss`, `Invoke`, `SetDoNotDisturb` and a `DoNotDisturb` property that announces
its changes), which scripts can use directly.

Like Hex, Hokum is one process: the service owns the bus name, draws the cards on a
click-through overlay and hosts the settings window. Launching `hokum` again shows that window.

- **One config file:** `~/.config/hokum/config.kdl` starts from a theme and overrides anything:
  look (`tokens`), layout (`template`), per-app and per-text `rule`s, and `behaviour` (which
  only the config sets; themes are just looks). The
  settings window edits the same file, keeping your comments, and hand edits apply as you save.
  See [docs/theming.md](docs/theming.md).
- **Themes:** eight built in (Soft, Crisp, Capsule, Ribbon, Terminal, Bubble, Signal, Pill), shown
  in a gallery as real cards. Capsule, with app icons, is the default. Custom themes are written
  by your coding agent: the settings window gives you a prompt that points it at
  [hokum.app](https://hokum.app) (the same guide as [docs/themes.md](docs/themes.md)), the
  complete guide to the format. A Rhai script can rewrite
  or drop notifications too.
- **Per app:** mute, how long its cards stay, and breaking through Do not disturb, for every app
  that has sent you a notification.
- **Icons and pictures:** app icons from your icon theme, and the pictures notifications carry
  (avatars, album art). Emoji draw in colour.
- **Behaviour:** screen corner, how long cards stay, how many show, pause while hovered, and
  Do not disturb (only critical notifications and apps you allow show).
- **Cards:** action buttons, a click for the default action (with an XDG activation token, so
  the app can raise its window on Wayland), progress bars (`value`), and
  in-place replacement by `x-dunst-stack-tag` / `x-canonical-private-synchronous` (volume,
  brightness). Critical notifications stay until dismissed. Slack's `[workspace] in #channel`
  titles become the sender, the message, and the channel beside the app's name. Omarchy's
  click commands, buttons and glyphs work too.

Hokum keeps `config.kdl`, your `themes/` and `script.rhai` in `~/.config/hokum/`, and notices
when any of them change.

## Build

```sh
cargo build --release
target/release/hokum service --snap cards.png --scene demo   # headless render, no D-Bus
target/release/hokum --snap settings.png --scene hokum
```

Releases are cut by `.github/workflows/release.yml`; see [docs/releasing.md](docs/releasing.md).

## License

MIT
