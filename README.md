# Hokum

A control center for Wayland that looks like it belongs on a computer from this decade.

Hokum is one app for compositors with `zwlr_layer_shell_v1` (niri, sway, Hyprland, river and
others), drawn with [Rune](https://github.com/anomalyco/rune), our Rust UI framework:

- **Notifications:** a freedesktop notification daemon (`org.freedesktop.Notifications`) whose
  cards slide in, restack on springs and get out of the way.
- **Sound:** output, input and the apps playing, on PipeWire. Devices, ports, Bluetooth
  codecs and balance a row down; profiles, virtual outputs, routing and the clock in
  the pane beside them.
- **Bluetooth:** power, connected and paired devices with battery, on BlueZ. Pairing (codes
  inline), trust, rename and forget a row down; adapters and raw properties in the pane beside them.

One window holds all three, one section each. One tray icon opens a panel with the output's
volume and device, the microphone while something records, your Bluetooth devices and Do not
disturb.

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

Sound needs PipeWire (with WirePlumber) and Bluetooth needs BlueZ; without them their sections
say so in one line and the tray panel leaves them out. The packages depend on `libpipewire`
and recommend `pipewire`, `wireplumber` and `bluez`.

## Use

```sh
hokum                     # the window (starts Hokum if it isn't running)
hokum sound               # the window on Sound
hokum bt                  # the window on Bluetooth (or hokum bluetooth)
hokum service [--replace] # run in the background (what hokum.service starts)
hokum check [path]        # report problems in config.kdl, or in a theme or config file
hokum dismiss [--all]     # close the newest card, or every card
hokum invoke              # do what clicking the newest card does
hokum dnd [on|off|toggle] # print Do not disturb, or set it (--json, --watch for bars)
hokum update [--install]  # check for a new build now (and install it)
hokum --version
```

`dismiss`, `invoke` and `dnd` are for keybindings and bars, like `makoctl`. `hokum dnd on` saves
to `config.kdl` just as the settings window's switch does. They call `app.hokum.Daemon` on the
session bus (`Dismiss`, `Invoke`, `SetDoNotDisturb` and a `DoNotDisturb` property that announces
its changes), which scripts can use directly.

Like Hex, Hokum is one process: the service owns the bus name, draws the cards on a
click-through overlay, hosts the window, runs Sound and Bluetooth on their own threads and keeps
the tray icon. Launching `hokum` again shows that window. A module that can't start, or fails
later, says so in its section and leaves the rest running.

### Modules

Sound and Bluetooth are on unless `config.kdl` turns them off; the end of each section's
advanced pane has a button for it too. With both off there's no tray icon.

```kdl
modules {
    sound #true
    bluetooth #false
}
```

### Cards from the modules

When the default output's volume or mute changes from outside Hokum (volume keys, `wpctl`, an
app), one Volume card shows the level and updates in place; it shows through Do not disturb.
When a paired Bluetooth device connects, a card says so with its battery, and one more warns once
when a device's battery drops below 20 % (again only after it's been back above 30 %). Turn them
off with `osd #false` and `bluetooth-notifications #false` in `modules`.

### Bluetooth pairing

Hokum registers a BlueZ pairing agent, so pairings started from Hokum prompt in Hokum, with the
code under the device. It asks to be the *default* agent (the one that answers pairings a
device starts, and untrusted devices asking for a service) only when no other Bluetooth app
that keeps one is running (blueman's applet, blueberry, Plasma's Bluedevil); then those keep
answering them.

- **One config file:** `~/.config/hokum/config.kdl` starts from a theme and overrides anything:
  look (`tokens`), layout (`template`), per-app and per-text `rule`s, `behaviour` and `modules`
  (which only the config sets; themes are just looks). The
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

## Updates

Hokum never updates itself without a click. A minute after it starts, and then hourly (every 30
minutes on dev builds), the service fetches its channel's release manifest
(`https://anoma.ly/release/hokum/desktop/<channel>.json`, sent as `hokum/<channel>/<version>`).
When the published executable for your machine differs from the running one (compared by
sha256), one Hokum card says so:

- Installed as a plain file you can write (`~/.local/bin/hokum`): **Update** downloads it,
  checks its size and sha256, replaces the executable and restarts Hokum, which then says
  "Hokum updated". If anything fails, the card says what and the old executable stays.
- Installed by a package manager (AUR, .deb, .rpm): the button copies the command that updates
  it (`paru -Syu hokum-bin`, for instance) with `wl-copy`, or opens hokum.app.

Dismiss the card and that build stays quiet until Hokum restarts or a day passes. Like Hokum's
other cards it waits out Do not disturb. Failures are silent and retried at the next check. To
turn checks off, set `check-for-updates #false` in `behaviour` in `config.kdl`. Builds run from a
cargo `target/` folder never check. `hokum update` checks now; `hokum update --install` does what
the card's button does.

## Build

Needs the PipeWire headers and clang for bindgen (`pipewire` and `clang` on Arch;
`libpipewire-0.3-dev libspa-0.2-dev clang libclang-dev pkg-config` on Debian and Ubuntu).

```sh
cargo build --release
target/release/hokum service --snap cards.png --scene demo   # headless render, no D-Bus
target/release/hokum --snap settings.png --scene hokum
target/release/hokum --snap sound.png --scene sound          # Sound and Bluetooth on mocks
target/release/hokum --snap panel.png --scene panel --size 364x440
```

`HOKUM_SOUND_BACKEND=mock` and `HOKUM_BLUETOOTH_BACKEND=mock` run the service on fake devices.

Releases are cut by `.github/workflows/release.yml`; see [docs/releasing.md](docs/releasing.md).

## License

MIT
