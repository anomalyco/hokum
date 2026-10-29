# Hokum on Omarchy

Omarchy 4 has no mako. Its shell (quickshell) is the notification server, through its
`omarchy.notifications` plugin, and the shell runs inside the Hyprland session rather than as
a unit of its own, so `hokum.service` can't stop it the way it stops mako or dunst. Turn the
plugin off first.

## Switch over

Add `"omarchy.notifications"` to `disabledPlugins` in `~/.config/omarchy/shell.json` (create
the list if it isn't there), then restart the shell and start Hokum:

```sh
omarchy-restart-shell
systemctl --user enable --now hokum.service
```

`busctl --user status org.freedesktop.Notifications` should now name `hokum`. To go back, remove
the entry, `systemctl --user disable --now hokum.service` and restart the shell.

## Keybindings

Omarchy's `SUPER + ,` bindings talk to the shell's plugin. Point them at Hokum in
`~/.config/hypr/bindings.lua` (these are Omarchy 4.0.5's keys; `omarchy menu keybindings
--print` shows yours):

```lua
hl.unbind("SUPER + comma")
o.bind("SUPER + comma", "Dismiss last notification", "hokum dismiss")
hl.unbind("SUPER + SHIFT + comma")
o.bind("SUPER + SHIFT + comma", "Dismiss all notifications", "hokum dismiss --all")
hl.unbind("SUPER + CTRL + comma")
o.bind("SUPER + CTRL + comma", "Toggle silencing notifications", "hokum dnd toggle")
hl.unbind("SUPER + ALT + comma")
o.bind("SUPER + ALT + comma", "Invoke last notification", "hokum invoke")
hl.unbind("SUPER + SHIFT + ALT + comma")
```

Hokum keeps no history, so the last line only frees `showHistory`'s key.

As with Omarchy's own cards, a right-click dismisses a card.

## Omarchy's own notifications

Hokum reads the hints `omarchy-notification-send` sets: clicking a card (or `hokum invoke`)
runs its `omarchy-exec-argv` command directly, without a shell, `omarchy-actions` become the
card's buttons, and an `omarchy-glyph` takes the place of the app's letter when there is no
icon. Glyphs are Nerd Font icons and draw in your `monospace` font, which Omarchy sets to one.

## What stays with Omarchy

- `omarchy-notification-wait` also asks the shell (`omarchy-shell notifications ping`), so with
  the plugin off it waits its full timeout: crash notices from `omarchy-crash-watch` are dropped,
  and migration and first-run notices arrive 10 seconds late.
- The bar's Do not disturb indicator reads the shell's plugin. For an indicator of your own,
  `hokum dnd` prints `on` or `off`, `--json` prints `{"dnd":true}`, and `--watch` keeps printing
  whenever it changes; bars that speak D-Bus can watch the `DoNotDisturb` property of
  `app.hokum.Daemon` at `/app/hokum/Daemon` instead.

## Themes

The built-in `omarchy` theme follows Omarchy's: `theme "omarchy"` in config.kdl, or pick it in
Settings. It draws cards the way Omarchy's own notifications looked (square, monospace, a 2 point
border in the Hyprland active-border colour, progress in the accent, urgent cards in red, no close
button) in the colours of `~/.local/state/omarchy/current/theme/colors.toml`, and redraws when you
switch themes. Set `appearance "system"` too, so Hokum turns light with a light Omarchy theme.
Tweaks sit on top in config.kdl as usual, such as `tokens { motion 0 0 }` for no animation.
