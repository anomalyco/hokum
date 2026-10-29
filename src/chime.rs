//! Notification sounds, off unless `behaviour { sound #true }`. A sound is a name in the
//! freedesktop sound theme (`message-new-instant`) or a file, played by `pw-play`, `paplay` or
//! (for WAV) `aplay` on a thread of its own. Only new cards chime, at most one per `GAP`.
//!
//! There's deliberately no "run a command on every notification" hook: notification text comes
//! from any app (and web page), and it would end up in a command.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::style::Behaviour;

/// Bursts make one sound.
const GAP: Duration = Duration::from_millis(150);
/// A sound still playing after this is stopped.
const LONGEST: Duration = Duration::from_secs(10);
const AT_ONCE: usize = 4;
const EXTENSIONS: [&str; 3] = ["oga", "ogg", "wav"];

/// What to play.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Name(String),
    File(PathBuf),
}

impl Pick {
    /// A name in the sound theme, or an absolute (or `~/`) path.
    pub fn parse(s: &str) -> Pick {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("~/")
            && let Some(home) = std::env::var_os("HOME")
        {
            return Pick::File(PathBuf::from(home).join(rest));
        }
        match s.strip_prefix("file://") {
            Some(p) => Pick::File(p.into()),
            None if s.starts_with('/') => Pick::File(s.into()),
            None => Pick::Name(s.into()),
        }
    }
}

/// A rule's `sound`: `#false` silences the app, `#true` gives it the usual sound even when
/// sounds are off, a name or path picks one.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Off,
    On,
    Sound(String),
}

/// What the sender asked for in its hints (`suppress-sound`, `sound-file`, `sound-name`).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Hint {
    #[default]
    None,
    Suppress,
    File(PathBuf),
    Name(String),
}

impl Hint {
    /// The sender's file goes through the same checks as its pictures (a regular file, not too
    /// large, nothing under `/proc`, `/sys` or `/dev`); a name must be a plain name.
    pub fn new(suppress: bool, file: Option<&str>, name: Option<&str>) -> Hint {
        if suppress {
            return Hint::Suppress;
        }
        let file = file.map(|f| f.strip_prefix("file://").unwrap_or(f));
        if let Some(path) = file.and_then(|f| crate::icons::picture_file(Path::new(f))) {
            return Hint::File(path);
        }
        match name.filter(|n| plain(n)) {
            Some(n) => Hint::Name(n.to_owned()),
            None => Hint::None,
        }
    }
}

/// A sound theme name: letters, digits, `-`, `_` and `.`, never a path.
pub fn plain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The sound for a new card: the sender's `suppress-sound` wins, then the app's rule, then the
/// sender's own sound, then the configured one for its urgency.
pub fn choose(b: &Behaviour, rule: Option<&Choice>, hint: &Hint, urgency: u8) -> Option<Pick> {
    if *hint == Hint::Suppress {
        return None;
    }
    match rule {
        Some(Choice::Off) => return None,
        Some(Choice::Sound(s)) => return Some(Pick::parse(s)),
        Some(Choice::On) => {}
        None if !b.sound => return None,
        None => {}
    }
    Some(match hint {
        Hint::File(p) => Pick::File(p.clone()),
        Hint::Name(n) => Pick::Name(n.clone()),
        _ if urgency >= 2 => Pick::parse(&b.sound_critical),
        _ => Pick::parse(&b.sound_name),
    })
}

/// One sound per `GAP`.
#[derive(Default)]
pub struct Limiter {
    last: Option<Instant>,
}

impl Limiter {
    pub fn allow(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_some_and(|l| now.saturating_duration_since(l) < GAP)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// The desktop's sound theme (`gtk-sound-theme-name`), else `freedesktop`.
fn theme_name() -> String {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    config
        .into_iter()
        .flat_map(|c| ["gtk-4.0", "gtk-3.0"].map(|g| c.join(g).join("settings.ini")))
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .find_map(|ini| {
            ini.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == "gtk-sound-theme-name")
                    .then(|| v.trim().trim_matches('"').to_owned())
                    .filter(|v| plain(v))
            })
        })
        .unwrap_or_else(|| "freedesktop".into())
}

/// `theme`, the themes it inherits from, and `freedesktop` last.
fn chain(dirs: &[PathBuf], theme: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut todo = vec![theme.to_owned()];
    while let Some(t) = todo.pop() {
        if out.contains(&t) || out.len() >= 8 {
            continue;
        }
        let inherits = dirs
            .iter()
            .find_map(|d| {
                std::fs::read_to_string(d.join("sounds").join(&t).join("index.theme")).ok()
            })
            .and_then(|index| {
                index.lines().find_map(|l| {
                    l.trim().strip_prefix("Inherits=").map(|v| {
                        v.split(',')
                            .map(|s| s.trim().to_owned())
                            .collect::<Vec<_>>()
                    })
                })
            })
            .unwrap_or_default();
        out.push(t);
        todo.extend(inherits.into_iter().rev().filter(|i| plain(i)));
    }
    if !out.iter().any(|t| t == "freedesktop") {
        out.push("freedesktop".into());
    }
    out
}

/// The file to play: a file as it is (after the same checks as a sender's), a name through the
/// theme chain, falling back as the spec says (`message-new-instant`, then `message-new`, then
/// `message`).
pub fn resolve(p: &Pick, dirs: &[PathBuf], theme: &str) -> Option<PathBuf> {
    let name = match p {
        Pick::File(path) => return crate::icons::picture_file(path),
        Pick::Name(n) if plain(n) => n.as_str(),
        Pick::Name(_) => return None,
    };
    let themes = chain(dirs, theme);
    let mut names = vec![name];
    let mut rest = name;
    while let Some((head, _)) = rest.rsplit_once('-') {
        names.push(head);
        rest = head;
    }
    names.iter().find_map(|n| {
        themes.iter().find_map(|t| {
            dirs.iter().find_map(|d| {
                let dir = d.join("sounds").join(t).join("stereo");
                EXTENSIONS
                    .iter()
                    .map(|e| dir.join(format!("{n}.{e}")))
                    .find(|f| f.is_file())
            })
        })
    })
}

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

/// The command that plays `file`, or None without a player for it.
pub fn player(file: &Path) -> Option<Command> {
    let wav = file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    if let Some(p) = on_path("pw-play") {
        return Some(with(p, &["--media-role", "Notification"], file));
    }
    if let Some(p) = on_path("paplay") {
        return Some(with(p, &["--property=media.role=event"], file));
    }
    on_path("aplay")
        .filter(|_| wav)
        .map(|p| with(p, &["-q"], file))
}

fn with(program: PathBuf, args: &[&str], file: &Path) -> Command {
    let mut c = Command::new(program);
    c.args(args)
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

/// Starts the thread that plays sounds; send it what to play.
pub fn start() -> Option<Sender<Pick>> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("hokum-chime".into())
        .spawn(move || run(rx))
        .ok()?;
    Some(tx)
}

fn run(rx: Receiver<Pick>) {
    let mut limiter = Limiter::default();
    let mut playing: Vec<(Child, Instant)> = Vec::new();
    let mut warned = false;
    loop {
        let next = if playing.is_empty() {
            match rx.recv() {
                Ok(p) => Some(p),
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(p) => Some(p),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => return,
            }
        };
        playing.retain_mut(|(child, started)| {
            if started.elapsed() > LONGEST {
                let _ = child.kill();
            }
            !matches!(child.try_wait(), Ok(Some(_)) | Err(_))
        });
        let Some(pick) = next else {
            continue;
        };
        if playing.len() >= AT_ONCE || !limiter.allow(Instant::now()) {
            continue;
        }
        let Some(file) = resolve(&pick, &crate::icons::data_dirs(), &theme_name()) else {
            eprintln!("hokum: no sound file for {pick:?}");
            continue;
        };
        match player(&file).map(|mut c| c.spawn()) {
            Some(Ok(child)) => playing.push((child, Instant::now())),
            Some(Err(e)) => eprintln!("hokum: couldn't play {}: {e}", file.display()),
            None if !warned => {
                warned = true;
                eprintln!("hokum: no player for notification sounds (pw-play, paplay or aplay)");
            }
            None => {}
        }
    }
}

/// What `hokum check` reports about sounds that are on: a configured sound that doesn't
/// resolve, or no player.
pub fn problems(b: &Behaviour, rules: &[crate::theme::Rule]) -> Vec<String> {
    let mut sounds: Vec<&str> = Vec::new();
    if b.sound {
        sounds.extend([b.sound_name.as_str(), b.sound_critical.as_str()]);
    }
    sounds.extend(rules.iter().filter_map(|r| match &r.sound {
        Some(Choice::Sound(s)) => Some(s.as_str()),
        _ => None,
    }));
    if sounds.is_empty() && !rules.iter().any(|r| r.sound == Some(Choice::On)) {
        return Vec::new();
    }
    let (dirs, theme) = (crate::icons::data_dirs(), theme_name());
    let mut out: Vec<String> = Vec::new();
    for s in sounds {
        match Pick::parse(s) {
            p @ Pick::Name(_) if resolve(&p, &dirs, &theme).is_none() => out.push(format!(
                "sound \"{s}\" isn't in the sound theme ({theme}); sound-theme-freedesktop has the default sounds"
            )),
            p @ Pick::File(_) if resolve(&p, &dirs, &theme).is_none() => {
                out.push(format!("sound \"{s}\" isn't a readable sound file"))
            }
            _ => {}
        }
    }
    if on_path("pw-play").is_none() && on_path("paplay").is_none() && on_path("aplay").is_none() {
        out.push("no player for notification sounds: install pw-play (PipeWire) or paplay".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (PathBuf, Vec<PathBuf>) {
        let root = std::env::temp_dir().join(format!("hokum-chime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (user, system) = (root.join("user"), root.join("system"));
        let file = |p: PathBuf| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        file(system.join("sounds/freedesktop/stereo/message-new-instant.oga"));
        file(system.join("sounds/freedesktop/stereo/dialog-warning.oga"));
        file(system.join("sounds/freedesktop/stereo/message.oga"));
        file(user.join("sounds/mine/stereo/bell.wav"));
        std::fs::write(
            user.join("sounds/mine/index.theme"),
            "[Sound Theme]\nInherits=freedesktop\n",
        )
        .unwrap();
        (root, vec![user, system])
    }

    #[test]
    fn names_resolve_through_the_theme_chain_and_fall_back() {
        let (root, dirs) = tree();
        let found = |n: &str, theme: &str| resolve(&Pick::Name(n.into()), &dirs, theme);
        assert!(
            found("bell", "mine")
                .unwrap()
                .ends_with("mine/stereo/bell.wav")
        );
        assert!(
            found("message-new-instant", "mine")
                .unwrap()
                .ends_with("freedesktop/stereo/message-new-instant.oga"),
            "inherited"
        );
        assert!(
            found("message-new-email", "nope")
                .unwrap()
                .ends_with("freedesktop/stereo/message.oga"),
            "falls back to `message`, and to freedesktop"
        );
        assert_eq!(found("bell", "freedesktop"), None);
        assert_eq!(found("../../etc/passwd", "mine"), None);
        let file = root.join("system/sounds/freedesktop/stereo/message.oga");
        assert!(resolve(&Pick::File(file.clone()), &dirs, "x").is_some());
        assert_eq!(
            resolve(&Pick::File("/proc/self/environ".into()), &dirs, "x"),
            None
        );
        assert_eq!(resolve(&Pick::File("/dev/zero".into()), &dirs, "x"), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn hints_are_checked_like_pictures() {
        assert_eq!(Hint::new(true, Some("/etc/hostname"), None), Hint::Suppress);
        assert_eq!(
            Hint::new(false, Some("/proc/self/environ"), None),
            Hint::None
        );
        assert_eq!(Hint::new(false, Some("file:///dev/zero"), None), Hint::None);
        assert_eq!(
            Hint::new(false, Some("relative.oga"), Some("bell")),
            Hint::Name("bell".into())
        );
        assert_eq!(Hint::new(false, None, Some("../x")), Hint::None);
        assert_eq!(Hint::new(false, None, Some("a/b")), Hint::None);
    }

    #[test]
    fn choosing_follows_config_rules_and_hints() {
        let off = Behaviour::default();
        let on = Behaviour {
            sound: true,
            ..Behaviour::default()
        };
        let name = |s: &str| Some(Pick::Name(s.into()));
        assert_eq!(choose(&off, None, &Hint::None, 1), None, "off by default");
        assert_eq!(choose(&off, None, &Hint::Name("bell".into()), 1), None);
        assert_eq!(
            choose(&on, None, &Hint::None, 1),
            name("message-new-instant")
        );
        assert_eq!(choose(&on, None, &Hint::None, 2), name("dialog-warning"));
        assert_eq!(
            choose(&on, None, &Hint::Name("bell".into()), 2),
            name("bell")
        );
        assert_eq!(choose(&on, None, &Hint::Suppress, 1), None);
        assert_eq!(choose(&on, Some(&Choice::Off), &Hint::None, 2), None);
        assert_eq!(
            choose(&off, Some(&Choice::On), &Hint::None, 1),
            name("message-new-instant")
        );
        assert_eq!(
            choose(
                &off,
                Some(&Choice::Sound("/x/ping.oga".into())),
                &Hint::Name("bell".into()),
                1
            ),
            Some(Pick::File("/x/ping.oga".into()))
        );
        assert_eq!(
            choose(&on, Some(&Choice::Sound("bell".into())), &Hint::Suppress, 1),
            None
        );
    }

    #[test]
    fn bursts_make_one_sound() {
        let mut l = Limiter::default();
        let t = Instant::now();
        assert!(l.allow(t));
        assert!(!l.allow(t + Duration::from_millis(50)));
        assert!(!l.allow(t + Duration::from_millis(149)));
        assert!(l.allow(t + Duration::from_millis(150)));
    }
}
