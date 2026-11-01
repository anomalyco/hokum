# D-Bus

Hokum serves three objects on the session bus, all on the name
`org.freedesktop.Notifications`. `hokum dismiss`, `invoke` and `dnd` and a second `hokum` are
thin clients of the two `app.hokum` ones; bars, keybindings and scripts can call them directly.

| Object | Interface | For |
|---|---|---|
| `/org/freedesktop/Notifications` | `org.freedesktop.Notifications` | apps sending notifications ([spec 1.2](https://specifications.freedesktop.org/notification-spec/latest/)) |
| `/app/hokum/Daemon` | `app.hokum.Daemon` | the cards and Do not disturb |
| `/app/hokum/Settings` | `app.hokum.Settings` | the window |

Calls reach whatever owns the name. With Hokum installed and nothing else running, the first call
starts it (`app.hokum.Notifications.service` activates `hokum.service`). If another daemon owns
the name, the `app.hokum` calls reach it and fail.

## app.hokum.Daemon

| Member | Signature | Does |
|---|---|---|
| `Reload()` | | re-read `config.kdl`, its theme and the script, and restyle the cards live |
| `Dismiss(b all) → u` | `b` → `u` | close the newest card, or every card with `all`; returns how many closed |
| `Invoke(s token) → b` | `s` → `b` | do what clicking the newest card does (its default action, or dismiss it); `token` is an `XDG_ACTIVATION_TOKEN` or `""`; false when there's no card |
| `SetDoNotDisturb(b on) → b` | `b` → `b` | turn Do not disturb on or off, saved to `config.kdl` as the settings window saves it; returns the state afterwards. Fails while `config.kdl` has problems (`hokum check`) |
| `DoNotDisturb` | property, `b`, read | whether Do not disturb is on |

Each change to `DoNotDisturb`, from this method, the window or an edit to `config.kdl`, is
announced as `org.freedesktop.DBus.Properties.PropertiesChanged` on `/app/hokum/Daemon` with
`("app.hokum.Daemon", {"DoNotDisturb": <b>}, [])`.

```sh
busctl --user call org.freedesktop.Notifications /app/hokum/Daemon app.hokum.Daemon Reload
busctl --user call org.freedesktop.Notifications /app/hokum/Daemon app.hokum.Daemon Dismiss b true
busctl --user call org.freedesktop.Notifications /app/hokum/Daemon app.hokum.Daemon Invoke s ""
busctl --user call org.freedesktop.Notifications /app/hokum/Daemon app.hokum.Daemon SetDoNotDisturb b true
busctl --user get-property org.freedesktop.Notifications /app/hokum/Daemon app.hokum.Daemon DoNotDisturb
```

To follow Do not disturb from a bar, `hokum dnd --watch --json` prints `{"dnd":true}` on every
change. The same with `dbus-monitor`:

```sh
dbus-monitor --session "type='signal',path='/app/hokum/Daemon',member='PropertiesChanged'"
```

## app.hokum.Settings

| Member | Signature | Does |
|---|---|---|
| `Activate(s token)` | `s` | show the window, focused with the launcher's activation token when there is one |
| `Open(s section, s token)` | `ss` | show the window on `notifications`, `sound` or `bluetooth` (`bt`); an unknown section opens it where it was |

```sh
busctl --user call org.freedesktop.Notifications /app/hokum/Settings app.hokum.Settings Open ss sound ""
```

## org.freedesktop.Notifications

Hokum implements the spec's methods and signals: `Notify`, `CloseNotification`,
`GetCapabilities` (`actions`, `body`, `body-markup`, `persistence`,
`x-canonical-private-synchronous`), `GetServerInformation` (`Hokum`, `anomalyco`, the version,
`1.2`), and the signals `NotificationClosed(u id, u reason)` (1 expired, 2 dismissed, 3 closed by
the app or replaced), `ActionInvoked(u id, s key)` and, before it when the click carried one,
`ActivationToken(u id, s token)`.

Hints Hokum reads:

| Hint | Type | Effect |
|---|---|---|
| `urgency` | `y` | 0 low, 1 normal, 2 critical (critical cards stay and pass Do not disturb) |
| `value` | `i` | a progress bar, 0 to 100 |
| `x-dunst-stack-tag`, `x-canonical-private-synchronous` | `s` | a card with the same tag from the same app replaces this one in place |
| `resident` | `b` | the card stays after an action |
| `desktop-entry` | `s` | the sender's desktop entry: its name and icon, and the app a click raises |
| `image-data` (`image_data`, `icon_data`) | `(iiibiiay)` | a picture, at most 4096 × 4096, 8 bits, 3 or 4 channels |
| `image-path` (`image_path`) | `s` | a picture's file or URI, or an icon name |
| `sound-file`, `sound-name`, `suppress-sound` | `s`, `s`, `b` | the card's sound when sounds are on |
| `sender-pid` | `x` or `u` | the sender's process, when the bus can't say |
| `omarchy-glyph` | `s` | a Nerd Font glyph for the badge |
| `omarchy-exec-argv`, `omarchy-actions` | `s` (JSON) | commands Hokum runs on a click; only on Omarchy and only from unsandboxed senders |
