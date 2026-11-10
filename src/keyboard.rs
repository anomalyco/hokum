//! The keyboard's backlight, read from sysfs (`/sys/class/leds/*kbd_backlight*`), for the
//! keyboard card and `hokum keyboard-backlight`. No code per maker: Dell (`dell::`), ThinkPads
//! (`tpacpi::`), ASUS (`asus::`), IdeaPads (`platform::`), Framework 13 (`chromeos::`), Macs
//! (`smc::`, `:white:`, a bare `kbd_backlight`) and the rest all show the same files.
//!
//! Firmware that changes the level on its own key says so in `brightness_hw_changed`, which
//! the watch follows; a driver without it (Framework 13, Macs) is read twice a second.
//! Writes go through logind, which lets the session's user set a backlight without root.

use std::path::{Path, PathBuf};

use crate::backlight::{Device, Level, Watch, number};

const ROOT: &str = "/sys/class/leds";

/// One keyboard backlight.
pub struct Led {
    /// The LED's name, as logind takes it (`dell::kbd_backlight`).
    pub name: String,
    pub dir: PathBuf,
    pub max: u32,
    /// The driver says when the firmware changes the level.
    pub reports: bool,
    /// On a keyboard the kernel says can be unplugged, rather than the laptop's own.
    pub removable: bool,
}

impl Led {
    pub fn level(&self) -> Option<Level> {
        self.device().level()
    }

    fn device(&self) -> Device {
        Device {
            dir: self.dir.clone(),
            max: self.max,
            read: &["brightness"],
            watch: &["brightness", "brightness_hw_changed"],
            poll: !self.reports,
        }
    }
}

/// Whether the device the LED hangs off says it can be unplugged. Only a USB port the firmware
/// calls removable says so: the laptop's own keyboard can sit on an internal USB bus (ASUS
/// ROG, T2 Macs), and those ports say `fixed` or `unknown`.
fn removable(dir: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(dir) else {
        return false;
    };
    real.ancestors()
        .find_map(|d| std::fs::read_to_string(d.join("removable")).ok())
        .is_some_and(|r| r.trim() == "removable")
}

/// Every keyboard backlight, the laptop's own first, then by name. Lock lights
/// (`input3::capslock`) and the rest of `/sys/class/leds` don't say `kbd_backlight`; a second
/// LED of the same name gets a suffix (`asus::kbd_backlight_1`).
fn leds(root: &Path) -> Vec<Led> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Led> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.contains("kbd_backlight") {
                return None;
            }
            let dir = e.path();
            let max = number(&dir.join("max_brightness")).filter(|m| *m > 0)?;
            Some(Led {
                reports: dir.join("brightness_hw_changed").exists(),
                removable: removable(&dir),
                name,
                dir,
                max,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.removable, &a.name).cmp(&(b.removable, &b.name)));
    out
}

fn pick(root: &Path) -> Option<Led> {
    leds(root).into_iter().next()
}

/// Calls `on_change` with the keyboard's level now and again whenever it changes (see
/// `crate::backlight::follow`, which also finds a keyboard that shows up later).
pub fn watch(on_change: impl Fn(Level, bool) + Send + 'static) -> Option<Watch> {
    crate::backlight::follow(|| pick(Path::new(ROOT)).map(|l| l.device()), on_change)
}

/// The level in words: a keyboard with a few steps names them, a finer one is a percentage.
pub fn text(l: Level) -> String {
    let names: &[&str] = match l.max {
        1 => &["Off", "On"],
        2 => &["Off", "Low", "High"],
        3 => &["Off", "Low", "Medium", "High"],
        _ => &[],
    };
    match names.get(l.now as usize) {
        Some(name) => name.to_string(),
        None => format!("{}%", (l.fraction() * 100.0).round()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ask {
    Up,
    Down,
    Toggle,
    To(u32),
    Percent(f32),
}

fn parse(word: &str) -> Option<Ask> {
    Some(match word {
        "up" => Ask::Up,
        "down" => Ask::Down,
        "toggle" => Ask::Toggle,
        w => match w.strip_suffix('%') {
            Some(p) => Ask::Percent(p.parse::<f32>().ok().filter(|p| *p >= 0.0)? / 100.0),
            None => Ask::To(w.parse().ok()?),
        },
    })
}

/// One step up or down: one level on a keyboard with a few, a tenth of the way on a finer one.
fn step(max: u32) -> u32 {
    ((max as f32 / 10.0).round() as u32).max(1)
}

/// The level `ask` leads to from `l`. Toggling on goes back to `saved`, the level it was
/// toggled off from, else full.
fn target(ask: Ask, l: Level, saved: Option<u32>) -> u32 {
    match ask {
        Ask::Up => (l.now + step(l.max)).min(l.max),
        Ask::Down => l.now.saturating_sub(step(l.max)),
        Ask::Toggle if l.now > 0 => 0,
        Ask::Toggle => saved.filter(|s| (1..=l.max).contains(s)).unwrap_or(l.max),
        Ask::To(v) => v.min(l.max),
        Ask::Percent(p) => ((p * l.max as f32).round() as u32).min(l.max),
    }
}

/// Where toggling off keeps the level to come back to.
fn memory() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty())?;
    Some(PathBuf::from(dir).join("hokum").join("keyboard-backlight"))
}

/// Holds the lock beside `memory` until dropped: a held key starts many `up`s at once, and
/// each must read the level the one before it wrote.
fn lock(memory: &Path) -> Option<std::fs::File> {
    let dir = memory.parent()?;
    let _ = std::fs::create_dir_all(dir);
    let file = std::fs::File::create(dir.join("keyboard-backlight.lock")).ok()?;
    // SAFETY: flock on a descriptor `file` owns; the lock goes with it.
    (unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_EX) } == 0)
        .then_some(file)
}

fn remember(path: &Path, level: u32) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{level}\n"));
}

/// Sets `led` to `value`: through logind (no root needed), else by writing the file (a udev
/// rule or a group may allow it). Says which worked.
fn set(led: &Led, value: u32) -> Result<&'static str, String> {
    let logind = zbus::blocking::Connection::system().and_then(|bus| {
        bus.call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/session/auto",
            Some("org.freedesktop.login1.Session"),
            "SetBrightness",
            &("leds", led.name.as_str(), value),
        )
        .map(|_| ())
    });
    let Err(e) = logind else {
        return Ok("logind");
    };
    let path = led.dir.join("brightness");
    match std::fs::write(&path, format!("{value}\n")) {
        Ok(()) => Ok("sysfs"),
        Err(w) => Err(format!(
            "can't set {}: logind: {e}; {}: {w}",
            led.name,
            path.display()
        )),
    }
}

/// `hokum keyboard-backlight [up | down | toggle | <level> | <percent>%] [--probe]`: print the
/// level, or change it and print the result. The service sees the change like any other and
/// shows the card.
pub fn cli(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--probe") {
        return probe(Path::new(ROOT));
    }
    let Some(led) = pick(Path::new(ROOT)) else {
        eprintln!("hokum: no keyboard backlight (`hokum keyboard-backlight --probe` says more)");
        return 1;
    };
    let memory = memory();
    let _lock = memory.as_deref().and_then(lock);
    let Some(l) = led.level() else {
        eprintln!("hokum: can't read {}", led.dir.join("brightness").display());
        return 1;
    };
    let Some(word) = args.first() else {
        println!("{}", text(l));
        return 0;
    };
    let Some(ask) = parse(word) else {
        eprintln!(
            "hokum: keyboard-backlight takes up, down, toggle, a level or a percentage, not `{word}`"
        );
        return 2;
    };
    let to = target(ask, l, memory.as_deref().and_then(number));
    if to != l.now
        && let Err(e) = set(&led, to)
    {
        eprintln!("hokum: {e}");
        return 1;
    }
    // Only once it's off: a toggle that failed leaves the level to come back to as it was.
    if let Some(path) = &memory
        && ask == Ask::Toggle
        && l.now > 0
    {
        remember(path, l.now);
    }
    println!(
        "{}",
        text(Level {
            now: to,
            max: l.max
        })
    );
    0
}

/// `--probe`: what Hokum sees, for a bug report from a laptop we don't have.
fn probe(root: &Path) -> i32 {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    println!("kernel {}", kernel.trim());
    let all = leds(root);
    if all.is_empty() {
        println!(
            "no keyboard backlight in {}: the laptop's firmware handles it on its own, or the \
             kernel has no driver for it",
            root.display()
        );
        return 1;
    }
    for (i, led) in all.iter().enumerate() {
        let level = match led.level() {
            Some(l) => format!("{}/{} ({})", l.now, l.max, text(l)),
            None => "unreadable".into(),
        };
        println!(
            "{}  {level}  {}  {}{}",
            led.name,
            if led.reports {
                "reports changes"
            } else {
                "read twice a second"
            },
            if led.removable {
                "removable"
            } else {
                "built-in"
            },
            if i == 0 { "  <- used" } else { "" }
        );
    }
    // Setting the level it already has shows whether writing works, and changes nothing. Not
    // while it's off: writing 0 to an LED drops any trigger on it.
    let led = &all[0];
    match led.level().filter(|l| l.now > 0).map(|l| set(led, l.now)) {
        Some(Ok(how)) => println!("write: {how} ok"),
        Some(Err(e)) => println!("write: {e}"),
        None => println!("write: not tried while it's off"),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backlight::tests::scratch;

    fn led(root: &Path, name: &str, max: u32, now: u32, reports: bool) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
        std::fs::write(dir.join("brightness"), format!("{now}\n")).unwrap();
        if reports {
            std::fs::write(dir.join("brightness_hw_changed"), format!("{now}\n")).unwrap();
        }
        dir
    }

    fn level(now: u32, max: u32) -> Level {
        Level { now, max }
    }

    #[test]
    fn every_makers_keyboard_is_found_by_its_name() {
        for (name, max, reports) in [
            ("dell::kbd_backlight", 2, true),
            ("tpacpi::kbd_backlight", 2, true),
            ("asus::kbd_backlight", 3, true),
            ("platform::kbd_backlight", 2, true),
            ("chromeos::kbd_backlight", 100, false),
            ("smc::kbd_backlight", 255, false),
            (":white:kbd_backlight", 255, false),
            ("kbd_backlight", 512, false),
        ] {
            let root = scratch("kbd-names");
            led(&root, "input3::capslock", 1, 0, false);
            led(&root, "phy0rx", 1, 1, false);
            led(&root, name, max, 1, reports);
            let l = pick(&root).unwrap_or_else(|| panic!("{name} found"));
            assert_eq!((l.name.as_str(), l.max, l.reports), (name, max, reports));
            assert_eq!(l.level(), Some(level(1, max)));
            assert_eq!(l.device().poll, !reports, "{name}: polled without reports");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn lock_lights_and_dark_ones_are_not_keyboards() {
        let root = scratch("kbd-none");
        led(&root, "input3::capslock", 1, 0, false);
        led(&root, "input3::scrolllock", 1, 0, false);
        led(&root, "broken::kbd_backlight", 0, 0, false);
        assert!(pick(&root).is_none());
        assert!(pick(&root.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_laptops_own_keyboard_comes_before_a_removable_one() {
        let root = scratch("kbd-removable");
        let class = root.join("class");
        std::fs::create_dir_all(&class).unwrap();
        // A plugged-in keyboard, a built-in one on internal USB, and one on the platform.
        for (port, says, name) in [
            ("usb1/1-2", Some("removable"), "a::kbd_backlight"),
            ("usb3/3-5", Some("fixed"), "m::kbd_backlight"),
            ("platform/z", None, "z::kbd_backlight"),
        ] {
            let dev = root.join("devices").join(port);
            std::fs::create_dir_all(dev.join("leds")).unwrap();
            if let Some(r) = says {
                std::fs::write(dev.join("removable"), format!("{r}\n")).unwrap();
            }
            let dir = led(&dev.join("leds"), name, 2, 1, false);
            std::os::unix::fs::symlink(&dir, class.join(name)).unwrap();
        }
        let all = leds(&class);
        let names: Vec<_> = all.iter().map(|l| (l.name.as_str(), l.removable)).collect();
        assert_eq!(
            names,
            [
                ("m::kbd_backlight", false),
                ("z::kbd_backlight", false),
                ("a::kbd_backlight", true)
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_renamed_duplicate_is_still_a_keyboard() {
        let root = scratch("kbd-dup");
        led(&root, "asus::kbd_backlight_1", 3, 2, true);
        assert_eq!(pick(&root).unwrap().name, "asus::kbd_backlight_1");
        led(&root, "asus::kbd_backlight", 3, 2, true);
        assert_eq!(
            pick(&root).unwrap().name,
            "asus::kbd_backlight",
            "the first registered before its duplicate"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn held_keys_take_turns() {
        let root = scratch("kbd-lock");
        let memory = root.join("hokum/keyboard-backlight");
        let first = lock(&memory).expect("a lock");
        let (tx, rx) = std::sync::mpsc::channel();
        let m = memory.clone();
        let other = std::thread::spawn(move || {
            let _second = lock(&m);
            let _ = tx.send(());
        });
        let wait = std::time::Duration::from_millis(300);
        assert!(
            rx.recv_timeout(wait).is_err(),
            "waits while the first holds it"
        );
        drop(first);
        assert!(rx.recv_timeout(std::time::Duration::from_secs(3)).is_ok());
        other.join().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn few_steps_have_names_and_fine_ones_a_percentage() {
        assert_eq!(text(level(1, 1)), "On");
        assert_eq!(text(level(0, 2)), "Off");
        assert_eq!(text(level(1, 2)), "Low");
        assert_eq!(text(level(2, 2)), "High");
        assert_eq!(text(level(2, 3)), "Medium");
        assert_eq!(text(level(40, 100)), "40%");
        assert_eq!(text(level(255, 255)), "100%");
    }

    #[test]
    fn up_and_down_step_and_stop_at_the_ends() {
        assert_eq!(target(Ask::Up, level(1, 2), None), 2);
        assert_eq!(target(Ask::Up, level(2, 2), None), 2, "never past full");
        assert_eq!(target(Ask::Down, level(0, 3), None), 0, "never below off");
        assert_eq!(target(Ask::Up, level(40, 100), None), 50);
        assert_eq!(target(Ask::Down, level(5, 100), None), 0);
        assert_eq!(target(Ask::Up, level(0, 255), None), 26);
    }

    #[test]
    fn toggle_goes_off_and_back_to_where_it_was() {
        assert_eq!(target(Ask::Toggle, level(1, 2), Some(2)), 0);
        assert_eq!(target(Ask::Toggle, level(0, 2), Some(1)), 1, "back");
        assert_eq!(target(Ask::Toggle, level(0, 2), None), 2, "else full");
        assert_eq!(
            target(Ask::Toggle, level(0, 2), Some(9)),
            2,
            "a level from another keyboard"
        );
    }

    #[test]
    fn toggling_off_remembers_the_level_to_come_back_to() {
        let root = scratch("kbd-memory");
        let path = root.join("hokum/keyboard-backlight");
        assert_eq!(number(&path), None, "nothing yet");
        remember(&path, 1);
        assert_eq!(number(&path), Some(1), "its folder made");
        let back = target(Ask::Toggle, level(0, 2), number(&path));
        assert_eq!(back, 1);
        remember(&path, 2);
        assert_eq!(number(&path), Some(2), "the latest wins");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_keyboard_is_watched_like_the_screen() {
        let root = scratch("kbd-watch");
        let dir = led(&root, "dell::kbd_backlight", 2, 1, true);
        let (tx, rx) = std::sync::mpsc::channel();
        let at = root.clone();
        let w = crate::backlight::follow(
            move || pick(&at).map(|l| l.device()),
            move |l, quiet| {
                let _ = tx.send((l, quiet));
            },
        )
        .expect("a watch");
        let wait = std::time::Duration::from_secs(3);
        assert_eq!(rx.recv_timeout(wait).unwrap(), (level(1, 2), true));
        // The firmware's own key: the level moves and `brightness_hw_changed` says so.
        std::fs::write(dir.join("brightness"), "2\n").unwrap();
        std::fs::write(dir.join("brightness_hw_changed"), "2\n").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), (level(2, 2), false));
        // `hokum keyboard-backlight down`, as logind writes it.
        std::fs::write(dir.join("brightness"), "0\n").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), (level(0, 2), false));
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn probing_without_a_keyboard_fails() {
        let root = scratch("kbd-probe");
        led(&root, "input3::capslock", 1, 0, false);
        assert_eq!(probe(&root), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn levels_and_percentages_are_words_too() {
        assert_eq!(parse("up"), Some(Ask::Up));
        assert_eq!(parse("toggle"), Some(Ask::Toggle));
        assert_eq!(parse("1"), Some(Ask::To(1)));
        assert_eq!(parse("50%"), Some(Ask::Percent(0.5)));
        assert_eq!(parse("brighter"), None);
        assert_eq!(parse("-5%"), None);
        assert_eq!(target(Ask::Percent(0.5), level(0, 3), None), 2);
        assert_eq!(target(Ask::To(9), level(0, 2), None), 2);
    }
}
