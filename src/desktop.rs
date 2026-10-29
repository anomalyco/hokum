//! Desktops Hokum can't run on, and what to tell people there: compositors without
//! `zwlr_layer_shell_v1` (GNOME), and desktop shells that are the notification server
//! themselves and never let `--replace` take the name (GNOME Shell, Plasma, quickshell).

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle};

/// Exit status for "Hokum can't run here as things are": hokum.service doesn't restart on it.
pub const UNSUPPORTED: i32 = 78;

const OMARCHY: &str = "https://github.com/anomalyco/hokum/blob/dev/docs/omarchy.md";

/// A desktop shell holding `org.freedesktop.Notifications`. None of them ask for the name
/// with ALLOW_REPLACEMENT, so REPLACE_EXISTING can't take it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shell {
    Gnome,
    Plasma,
    Quickshell,
}

/// The shell behind the owner's `GetServerInformation` name and its process name.
pub fn shell(name: &str, comm: &str) -> Option<Shell> {
    let (name, comm) = (name.to_lowercase(), comm.to_lowercase());
    if comm == "gnome-shell" || name == "gnome-shell" {
        Some(Shell::Gnome)
    } else if comm == "plasmashell" || name == "plasma" {
        Some(Shell::Plasma)
    } else if comm.starts_with("quickshell") || comm == "qs" || name.contains("quickshell") {
        Some(Shell::Quickshell)
    } else {
        None
    }
}

/// What to say when `owner` kept the name, and the exit status for it.
pub fn refusal(owner: &str, shell: Option<Shell>, replace: bool) -> (String, i32) {
    let text = match shell {
        Some(Shell::Gnome) => "GNOME Shell shows notifications itself and doesn't let another daemon take over, and GNOME has no layer-shell for Hokum's cards, so Hokum can't run on GNOME.".to_owned(),
        Some(Shell::Plasma) => "Plasma shows notifications itself and doesn't let another daemon take over. Turn Plasma's notifications off (System Tray settings → Entries → Notifications: Disabled), then start Hokum again: systemctl --user enable --now hokum.service (log out and back in if Plasma still holds it).".to_owned(),
        Some(Shell::Quickshell) => format!("{owner} (your desktop shell) shows notifications itself and doesn't let another daemon take over. Turn its notifications off, then start Hokum again; on Omarchy see {OMARCHY}"),
        None if replace => format!("{owner} owns org.freedesktop.Notifications and doesn't let another daemon take it; stop {owner}, then start Hokum again."),
        None => return (format!("{owner} owns org.freedesktop.Notifications; pass --replace to take it."), 1),
    };
    (text, UNSUPPORTED)
}

/// What to say on a compositor without layer-shell.
pub fn no_layer_shell() -> String {
    let gnome =
        std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("GNOME"));
    let here = if gnome {
        "GNOME doesn't support layer-shell"
    } else {
        "This compositor doesn't support layer-shell"
    };
    format!(
        "{here} (zwlr_layer_shell_v1), which Hokum draws its cards and panel with, so Hokum can't run here. It runs on niri, sway, Hyprland, river and KDE Plasma."
    )
}

struct Probe;

impl Dispatch<WlRegistry, GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Whether the Wayland compositor offers `zwlr_layer_shell_v1`; None without one to ask.
pub fn layer_shell() -> Option<bool> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, _) = registry_queue_init::<Probe>(&conn).ok()?;
    Some(
        globals
            .contents()
            .with_list(|l| l.iter().any(|g| g.interface == "zwlr_layer_shell_v1")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_the_shells() {
        assert_eq!(shell("gnome-shell", "gnome-shell"), Some(Shell::Gnome));
        assert_eq!(shell("Plasma", "plasmashell"), Some(Shell::Plasma));
        assert_eq!(shell("Another daemon", "plasmashell"), Some(Shell::Plasma));
        assert_eq!(shell("whatever", "quickshell"), Some(Shell::Quickshell));
        assert_eq!(shell("mako", "mako"), None);
    }

    #[test]
    fn never_says_pass_replace_after_replace() {
        for shell in [
            None,
            Some(Shell::Gnome),
            Some(Shell::Plasma),
            Some(Shell::Quickshell),
        ] {
            let (text, code) = refusal("mako", shell, true);
            assert!(!text.contains("--replace"), "{text}");
            assert_eq!(code, UNSUPPORTED);
        }
        let (text, code) = refusal("mako", None, false);
        assert!(text.contains("pass --replace"));
        assert_eq!(code, 1);
    }

    #[test]
    fn shells_say_what_to_do() {
        assert!(
            refusal("gnome-shell", Some(Shell::Gnome), false)
                .0
                .contains("can't run on GNOME")
        );
        assert!(
            refusal("Plasma", Some(Shell::Plasma), false)
                .0
                .contains("Notifications: Disabled")
        );
        assert!(
            refusal("quickshell", Some(Shell::Quickshell), false)
                .0
                .contains("omarchy.md")
        );
    }
}
