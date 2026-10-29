# Hokum Agent Guide

## Speed

- Move fast and fix forward: merge once it compiles; wait only for checks
  that gate the merge.
- No fixed sleeps to wait on CI: use `gh run watch <id> --exit-status`.
- Never end your turn while still waiting on something.
- Dev releases never cancel each other: a running release always finishes,
  and pushes in between are skipped so only the newest pending run goes next
  (`cancel-in-progress: false` on Release's per-channel group). Never add
  `cancel-in-progress: true` or a cancel step to a build or publish workflow.

## What Hokum is

One app, one binary (`hokum`): a control center for Wayland compositors (niri,
sway, Hyprland, river). `hokum service` hosts the notification daemon, the Sound
backend (PipeWire) and the Bluetooth backend (BlueZ over zbus), each on its own
threads, plus one tray icon whose click opens a panel. Sound and Bluetooth are
modules (`modules { sound #true; bluetooth #true }` in config.kdl, on by
default); a module that can't start or panics shows one quiet line in its
section and never takes notifications down.

## Design

- Minimal. Every control has to earn its place. Almost no copy: labels only
  where an icon can't say it.
- The window's sidebar has exactly one item per module: Notifications, Sound,
  Bluetooth. A module's deeper levels live inside its section, never as more
  sidebar items.
- Progressive disclosure, at most about six controls on a first level: Sound's
  first level is output, input and the apps playing (rows open in place; the
  quiet Advanced button holds profiles, routing, the clock). Bluetooth's is the
  power switch and connected and paired devices (rows open in place; pairing
  codes inline; Advanced holds adapters and raw properties). Notifications
  keeps Appearance, Behaviour and Applications as tabs in its section.
- The tray panel: output volume and device, the input only while something
  records or it's muted, Bluetooth devices, Do not disturb, open the window.
- Review every UI change from screenshots on mock backends before calling it
  done: `hokum --snap <png> --scene <name>` (scenes: `hokum`, `behaviour`,
  `applications`, `app`, `other`, `sound`, `sound-expanded`, `sound-advanced`,
  `sound-lost`, `sound-off`, `bluetooth`, `bluetooth-expanded`,
  `bluetooth-pairing`, `bluetooth-advanced`, `bluetooth-off`, `panel` with
  `--size 364x440`) and `hokum service --snap <png> --scene demo` for cards.

## Layout

- `src/settings.rs`: the window (sidebar, Notifications' section, the host
  for the modules' sections); the same state draws the tray panel.
- `src/modules.rs`: starts and stops the backends as config.kdl changes, feeds
  snapshots to the window and the panel, owns the tray icon. `src/panel.rs`:
  the panel.
- `src/sound/`: `model.rs` (the contract), `pipewire/` (the backend), `mock.rs`,
  `ui/` (handlers take the window's state; `lift` adapts the section's own).
- `src/bluetooth/`: `model.rs`, `bluez/` (the backend and the pairing agent),
  `mock.rs`, `ui/`.
- The rest is notifications: `bus.rs`, `cards.rs`, `card.rs`, `theme.rs`,
  `config.rs`, `update.rs`, ...

## Building

Needs the PipeWire headers and clang for bindgen (`pipewire` and `clang` on
Arch; `libpipewire-0.3-dev libspa-0.2-dev clang libclang-dev pkg-config` on
Ubuntu). `cargo test`, `cargo build --release`.

## Testing on real hardware

Never unpair, forget, block or untrust real Bluetooth devices, never change a
real adapter's settings without restoring them, and don't run a second
`hokum service` on someone's desktop: use the mock backends
(`HOKUM_SOUND_BACKEND=mock`, `HOKUM_BLUETOOTH_BACKEND=mock`) and the scenes.
