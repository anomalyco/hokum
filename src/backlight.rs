//! The screen's backlight, read from sysfs (`/sys/class/backlight`), for the brightness card:
//! the level of the device userspace should drive, and a watch that says when it changes,
//! whoever changed it (keys, `brightnessctl`, an auto-dimmer, the firmware). The watch itself
//! (`follow`) also serves the keyboard's backlight (`crate::keyboard`).

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const ROOT: &str = "/sys/class/backlight";

/// A reading: the raw level and the device's maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub now: u32,
    pub max: u32,
}

impl Level {
    /// The level as a fraction of the maximum.
    pub fn fraction(self) -> f32 {
        (self.now as f32 / self.max.max(1) as f32).clamp(0.0, 1.0)
    }
}

/// One backlight device, and how to follow it.
pub struct Device {
    pub dir: PathBuf,
    pub max: u32,
    /// The files that hold the level, the first one there winning.
    pub read: &'static [&'static str],
    /// The files whose changes say the level moved.
    pub watch: &'static [&'static str],
    /// Read it twice a second too: the driver doesn't say when the firmware changes it.
    pub poll: bool,
}

impl Device {
    pub fn level(&self) -> Option<Level> {
        let now = self.read.iter().find_map(|f| number(&self.dir.join(f)))?;
        Some(Level {
            now: now.min(self.max),
            max: self.max,
        })
    }
}

pub fn number(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The device to follow: the kernel's own advice is firmware, then platform, then raw (the
/// last driving the panel directly), and the first by name among equals.
fn pick(root: &Path) -> Option<Device> {
    let rank = |dir: &Path| match std::fs::read_to_string(dir.join("type"))
        .unwrap_or_default()
        .trim()
    {
        "firmware" => 0,
        "platform" => 1,
        _ => 2,
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort_by_key(|d| (rank(d), d.clone()));
    dirs.into_iter().find_map(|dir| {
        let max = number(&dir.join("max_brightness")).filter(|m| *m > 0)?;
        // `actual_brightness` is what the hardware reports, when the driver has it;
        // `brightness` is what was asked for. A write through logind or sysfs changes
        // `brightness`; a change the hardware makes on its own shows in `actual_brightness`.
        Some(Device {
            dir,
            max,
            read: &["actual_brightness", "brightness"],
            watch: &["brightness", "actual_brightness"],
            poll: false,
        })
    })
}

/// Calls `on_change` with the screen's level now and again whenever it changes (see
/// `follow`).
pub fn watch(on_change: impl Fn(Level, bool) + Send + 'static) -> Option<Watch> {
    follow(|| pick(Path::new(ROOT)), on_change)
}

/// What a module hands a watch: called with each level, and whether it's quiet (see `follow`).
pub type OnChange = Box<dyn Fn(Level, bool) + Send>;

pub struct Watch {
    stop: Arc<AtomicBool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// How often to look for a device while there is none (a driver that loads late, a keyboard
/// plugged in), and after the one followed went away.
const LOOK_AGAIN: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(5)
};

/// How long after waking every change is a new baseline: firmware and restorers
/// (`brightnessctl -r`, an idle daemon) set the light back a moment after resume.
const SETTLE: Duration = Duration::from_secs(3);

/// How long the system has been asleep since boot: the boot clock counts suspend, the
/// monotonic one doesn't.
fn asleep() -> Duration {
    let read = |clock| {
        let mut t = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `t` is a valid timespec for the call.
        unsafe { libc::clock_gettime(clock, &mut t) };
        Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
    };
    read(libc::CLOCK_BOOTTIME).saturating_sub(read(libc::CLOCK_MONOTONIC))
}

/// Tells a suspend from the clocks: the system slept when more than a second of `asleep()`
/// passed between two looks (the clocks drift apart by a hair even awake).
struct Sleep {
    slept: Duration,
    /// Until when changes are baselines, after the last wake.
    settle: Option<Instant>,
}

impl Sleep {
    fn new(slept: Duration) -> Sleep {
        Sleep {
            slept,
            settle: None,
        }
    }

    /// (just woke, still settling) for a look at `slept` (an `asleep()` reading) at `now`.
    fn look(&mut self, slept: Duration, now: Instant) -> (bool, bool) {
        let woke = slept > self.slept + Duration::from_secs(1);
        self.slept = slept;
        if woke {
            self.settle = Some(now + SETTLE);
        }
        (woke, self.settle.is_some_and(|t| now < t))
    }
}

/// What a reading tells: the level when it moved since `last` (which it becomes), or any
/// level when `force`d (a new baseline). Nothing for an unreadable one.
fn news(last: &mut Option<Level>, now: Option<Level>, force: bool) -> Option<Level> {
    let l = now?;
    (now != *last || force).then(|| {
        *last = now;
        l
    })
}

/// An inotify descriptor on `device`'s watched files; None when it can't watch any and won't
/// poll either.
fn notify(device: &Device) -> Option<OwnedFd> {
    // SAFETY: inotify_init1 returns a new descriptor or -1; the OwnedFd takes the only copy.
    let fd = unsafe {
        let raw = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
        if raw < 0 {
            return None;
        }
        OwnedFd::from_raw_fd(raw)
    };
    let mut watching = 0;
    for file in device.watch {
        let path = device.dir.join(file);
        if let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
            // SAFETY: `c` is a NUL-terminated path that outlives the call.
            let id =
                unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c.as_ptr(), libc::IN_MODIFY) };
            watching += (id >= 0) as u32;
        }
    }
    (watching > 0 || device.poll).then_some(fd)
}

/// Waits up to 500 ms for `fd`, then drains it: whether anything came. The timeout is how soon
/// a dropped Watch is noticed, and how often a polled device is read.
fn wait(fd: &OwnedFd) -> bool {
    let mut p = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one pollfd, valid for the call.
    if unsafe { libc::poll(&mut p, 1, 500) } <= 0 {
        return false;
    }
    // Drain every event queued, then look once: a held key is many writes.
    let mut buf = [0u8; 1024];
    // SAFETY: reads into `buf`, at most its length.
    while unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
    true
}

/// Follows the device `pick` finds, on a thread of its own that stops when the returned
/// `Watch` drops: `on_change(level, quiet)` with the level now and again whenever it changes.
/// `quiet` marks a new baseline rather than a change to show: the first reading of a device,
/// and any in the moments after the system woke (a laptop restoring its light isn't someone
/// changing it). Without a device, or when it goes away, it looks again every few seconds.
pub fn follow(
    pick: impl Fn() -> Option<Device> + Send + 'static,
    on_change: impl Fn(Level, bool) + Send + 'static,
) -> Option<Watch> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("hokum-backlight".into())
        .spawn(move || {
            let mut current: Option<(Device, OwnedFd)> = None;
            let mut last = None;
            let mut next_look = Instant::now();
            let mut sleep = Sleep::new(asleep());
            while !flag.load(Ordering::Relaxed) {
                if current.is_none() && Instant::now() >= next_look {
                    next_look = Instant::now() + LOOK_AGAIN;
                    current = pick().and_then(|d| notify(&d).map(|fd| (d, fd)));
                    if let Some((d, _)) = &current {
                        last = None;
                        if let Some(l) = news(&mut last, d.level(), true) {
                            on_change(l, true);
                        }
                    }
                }
                let ready = match &current {
                    Some((_, fd)) => wait(fd),
                    None => {
                        std::thread::sleep(Duration::from_millis(500));
                        false
                    }
                };
                let (woke, quiet) = sleep.look(asleep(), Instant::now());
                let Some((device, _)) = &current else {
                    continue;
                };
                if !(ready || woke || quiet || device.poll) {
                    continue;
                }
                let now = device.level();
                if now.is_none() && !device.dir.exists() {
                    // Gone (a keyboard unplugged, a driver unloaded): find another.
                    current = None;
                    continue;
                }
                if let Some(l) = news(&mut last, now, woke) {
                    on_change(l, quiet);
                }
                if ready {
                    // A brightness that moves in many small steps settles into one card update.
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        })
        .ok()?;
    Some(Watch { stop })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::mpsc;

    fn device(root: &Path, name: &str, kind: &str, max: u32, now: u32) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
        std::fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
        std::fs::write(dir.join("brightness"), format!("{now}\n")).unwrap();
        dir
    }

    pub fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hokum-backlight-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn level(now: u32, max: u32) -> Option<Level> {
        Some(Level { now, max })
    }

    #[test]
    fn the_level_is_read_against_the_maximum() {
        let root = scratch("level");
        let dir = device(&root, "intel_backlight", "raw", 200, 50);
        let d = pick(&root).unwrap();
        assert_eq!(d.level(), level(50, 200));
        assert_eq!(d.level().unwrap().fraction(), 0.25);
        std::fs::write(dir.join("actual_brightness"), "100\n").unwrap();
        assert_eq!(d.level(), level(100, 200), "the hardware's figure wins");
        std::fs::write(dir.join("brightness"), "9999\n").unwrap();
        std::fs::remove_file(dir.join("actual_brightness")).unwrap();
        assert_eq!(d.level(), level(200, 200), "never past full");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn firmware_beats_platform_beats_raw() {
        let root = scratch("pick");
        device(&root, "a_raw", "raw", 100, 10);
        device(&root, "b_platform", "platform", 100, 20);
        device(&root, "c_firmware", "firmware", 100, 30);
        assert!(pick(&root).unwrap().dir.ends_with("c_firmware"));
        std::fs::remove_dir_all(root.join("c_firmware")).unwrap();
        assert!(pick(&root).unwrap().dir.ends_with("b_platform"));
        std::fs::remove_dir_all(root.join("b_platform")).unwrap();
        assert!(pick(&root).unwrap().dir.ends_with("a_raw"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_backlight_means_nothing_to_watch() {
        let root = scratch("none");
        assert!(pick(&root).is_none());
        assert!(pick(&root.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_watch_reports_the_level_and_each_change() {
        let root = scratch("watch");
        let dir = device(&root, "intel_backlight", "raw", 100, 40);
        let (tx, rx) = mpsc::channel();
        let at = root.clone();
        let w = follow(
            move || pick(&at),
            move |l, quiet| {
                let _ = tx.send((l.now, quiet));
            },
        )
        .expect("a watch");
        let wait = Duration::from_secs(3);
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            (40, true),
            "the level it starts at, as a baseline"
        );
        std::fs::write(dir.join("brightness"), "70\n").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), (70, false));
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_suspend_is_told_from_the_clocks_drifting() {
        let s = Duration::from_secs;
        let t0 = Instant::now();
        let mut sleep = Sleep::new(s(5));
        assert_eq!(sleep.look(s(5), t0), (false, false), "awake");
        assert_eq!(
            sleep.look(s(5) + Duration::from_millis(3), t0),
            (false, false),
            "a hair of drift"
        );
        assert_eq!(sleep.look(s(65), t0), (true, true), "a minute asleep");
        assert_eq!(
            sleep.look(s(65), t0 + s(1)),
            (false, true),
            "still settling a second later"
        );
        assert_eq!(sleep.look(s(65), t0 + SETTLE), (false, false), "settled");
        let mut real = Sleep::new(asleep());
        assert_eq!(real.look(asleep(), Instant::now()), (false, false));
    }

    #[test]
    fn only_moves_and_wakings_are_news() {
        let l = |now| Some(Level { now, max: 100 });
        let mut last = l(40);
        assert_eq!(news(&mut last, l(40), false), None, "the same level");
        assert_eq!(news(&mut last, l(70), false), l(70), "a move");
        assert_eq!(last, l(70), "the new baseline");
        assert_eq!(news(&mut last, None, false), None, "unreadable");
        assert_eq!(last, l(70), "an unreadable level isn't a baseline");
        assert_eq!(
            news(&mut last, l(70), true),
            l(70),
            "after a sleep, even unmoved"
        );
        assert_eq!(news(&mut last, l(10), true), l(10));
        assert_eq!(last, l(10));
    }

    #[test]
    fn a_polled_device_is_read_without_events() {
        let root = scratch("poll");
        let dir = device(&root, "chromeos::kbd_backlight", "raw", 100, 40);
        let at = dir.clone();
        let (tx, rx) = mpsc::channel();
        let w = follow(
            move || {
                Some(Device {
                    dir: at.clone(),
                    max: 100,
                    read: &["brightness"],
                    watch: &[],
                    poll: true,
                })
            },
            move |l, _| {
                let _ = tx.send(l.now);
            },
        )
        .expect("a watch");
        let wait = Duration::from_secs(3);
        assert_eq!(rx.recv_timeout(wait).unwrap(), 40);
        // Rewritten in place without a watch on it: only reading it again sees this.
        std::fs::write(dir.join("brightness"), "60\n").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), 60);
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_device_that_comes_late_or_goes_is_looked_for_again() {
        let root = scratch("late");
        let (tx, rx) = mpsc::channel();
        let at = root.clone();
        let w = follow(
            move || pick(&at),
            move |l, quiet| {
                let _ = tx.send((l.now, quiet));
            },
        )
        .expect("a watch even with nothing to follow yet");
        let wait = Duration::from_secs(3);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        let first = device(&root, "a_late", "raw", 100, 30);
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            (30, true),
            "found, as a baseline"
        );
        std::fs::write(first.join("brightness"), "50\n").unwrap();
        assert_eq!(rx.recv_timeout(wait).unwrap(), (50, false));
        device(&root, "b_other", "raw", 100, 80);
        std::fs::remove_dir_all(&first).unwrap();
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            (80, true),
            "the next one, once the first went away"
        );
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }
}
