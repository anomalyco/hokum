# Contributing

Pull requests go to `dev`. [AGENTS.md](AGENTS.md) holds the project's rules (design, layout,
testing on real hardware); this page is the short path to a PR.

## Build and test

Needs the PipeWire headers and clang for bindgen: `pipewire` and `clang` on Arch;
`libpipewire-0.3-dev libspa-0.2-dev clang libclang-dev pkg-config` on Debian and Ubuntu.

```sh
cargo fmt --check
cargo test
cargo build --release
```

## Check UI changes from screenshots

Render the change headless on mock devices, before and after, and look at both:

```sh
export HOKUM_SOUND_BACKEND=mock HOKUM_BLUETOOTH_BACKEND=mock
target/release/hokum service --snap cards.png --scene demo   # cards; also osd, brightness, bt-connected
target/release/hokum --snap window.png --scene hokum         # the window; --list-scenes for the rest
target/release/hokum --snap panel.png --scene panel --size 364x440
```

Snapshots claim no D-Bus name, so they're safe beside a running notification daemon. Don't run a
second `hokum service` on a desktop you use, and never unpair, forget or block a real Bluetooth
device, or change a real adapter without putting it back.

## The pull request

Say what was wrong and why, what changed, and how you tested it: the `cargo test` count, a
before/after for anything you can measure, `--snap` screenshots for anything visual, and what
you couldn't verify. Changes under `.github/` need the maintainer.

[docs/dbus.md](docs/dbus.md) describes the D-Bus API; [docs/theming.md](docs/theming.md) and
[docs/themes.md](docs/themes.md) the config and themes; [docs/releasing.md](docs/releasing.md)
how releases are cut.
