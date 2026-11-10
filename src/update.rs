//! Updates, only when asked: the service fetches its channel's manifest from the Anomaly release
//! service as `hokum/<channel>/<version>` (a minute after start, then hourly, or every 30
//! minutes on dev) and, when the published executable for this machine differs from the
//! running one (by sha256; versions are only shown), puts one card up. Clicking it replaces the
//! executable and restarts Hokum, or, for a package manager's install, copies the command that
//! updates it. Nothing installs without a click, and a dismissed card stays away for that build
//! until the next start or a day. Failures are silent; `check-for-updates #false` in
//! config.kdl's `behaviour` turns the checks off.
//!
//! `hokum update` checks now and prints the result; `hokum update --install` does what the
//! card's button does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rune::OverlayHandle;
use serde::Deserialize;

use crate::card::{Note, Timeout};
use crate::cards::{Reason, Ui};
use crate::icons::Source;

/// Next to the welcome's id, far from the bus's.
pub const ID: u32 = u32::MAX / 2 + 1;
const HOST: &str = "https://anoma.ly";
const SITE: &str = "https://hokum.app";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const FIRST: Duration = Duration::from_secs(60);
/// How long a dismissed build stays away.
const QUIET: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Channel {
    Dev,
    Beta,
    Stable,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Channel::Dev => "dev",
            Channel::Beta => "beta",
            Channel::Stable => "stable",
        }
    }

    fn every(self) -> Duration {
        match self {
            Channel::Dev => Duration::from_secs(30 * 60),
            _ => Duration::from_secs(60 * 60),
        }
    }
}

/// Dev builds are numbered `0.0.0-dev-N` and betas `<v>-beta.N`; everything else is stable.
pub fn channel(version: &str) -> Channel {
    if version.starts_with("0.0.0-dev-") {
        Channel::Dev
    } else if version.contains("-beta.") {
        Channel::Beta
    } else {
        Channel::Stable
    }
}

#[derive(Deserialize)]
struct Manifest {
    release: Release,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub version: String,
    #[serde(default)]
    targets: HashMap<String, Target>,
    #[serde(default)]
    distributions: Vec<Distribution>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Target {
    url: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Distribution {
    kind: String,
    command: Option<String>,
    package: Option<String>,
}

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

/// The build to offer: this machine's target, unless it is the one running.
pub fn newer<'a>(release: &'a Release, running: &str) -> Option<&'a Target> {
    release
        .targets
        .get(&format!("linux-{}", arch()))
        .filter(|t| !t.sha256.eq_ignore_ascii_case(running))
}

/// Whether to put the card up for `sha`, given the last dismissal.
pub fn due(sha: &str, dismissed: Option<&(String, Instant)>, now: Instant) -> bool {
    !dismissed.is_some_and(|(d, at)| d == sha && now.duration_since(*at) < QUIET)
}

/// How this executable was installed, which decides what the card's button does.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    /// Replace the file and restart (`~/.local/bin/hokum`).
    Direct(PathBuf),
    /// A package manager owns it: pacman, dpkg or rpm, and the package's name.
    Package {
        manager: &'static str,
        package: String,
    },
    /// Not writable and no package claims it.
    Unknown,
    /// A cargo build: never offered updates.
    Source,
}

pub fn classify(path: &Path, owner: Option<(&'static str, String)>, writable: bool) -> Install {
    let s = path.to_string_lossy();
    if s.contains("/target/release/") || s.contains("/target/debug/") {
        return Install::Source;
    }
    if let Some((manager, package)) = owner {
        return Install::Package { manager, package };
    }
    if writable && !s.starts_with("/usr/") {
        Install::Direct(path.to_owned())
    } else {
        Install::Unknown
    }
}

fn quiet(cmd: &mut Command) -> Option<String> {
    let out = cmd
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn has(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

fn owner(path: &Path) -> Option<(&'static str, String)> {
    if let Some(p) = quiet(Command::new("pacman").arg("-Qqo").arg(path)) {
        return Some(("pacman", p.lines().next()?.to_owned()));
    }
    if let Some(p) = quiet(Command::new("dpkg").arg("-S").arg(path)) {
        return Some(("dpkg", p.split(':').next()?.trim().to_owned()));
    }
    quiet(
        Command::new("rpm")
            .args(["-qf", "--qf", "%{NAME}"])
            .arg(path),
    )
    .map(|p| ("rpm", p))
}

fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let ok = |p: &Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes())
            .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
    };
    ok(path) && path.parent().is_some_and(ok)
}

/// The executable's path, read before an update can replace (and unlink) it.
pub fn exe() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let p = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/hokum"));
        let s = p.to_string_lossy();
        s.strip_suffix(" (deleted)").map(PathBuf::from).unwrap_or(p)
    })
}

pub fn install() -> &'static Install {
    static INSTALL: OnceLock<Install> = OnceLock::new();
    INSTALL.get_or_init(|| {
        let path = exe();
        let source = classify(path, None, false) == Install::Source;
        if source {
            Install::Source
        } else {
            classify(path, owner(path), writable(path))
        }
    })
}

fn sha256(path: &Path) -> Option<String> {
    let out = quiet(Command::new("sha256sum").arg(path))?;
    out.split_whitespace().next().map(str::to_lowercase)
}

/// The running executable's sha256 (its inode, even after the file was replaced).
fn running() -> Option<&'static str> {
    static SHA: OnceLock<Option<String>> = OnceLock::new();
    // `/proc/self` in sha256sum would be sha256sum itself.
    SHA.get_or_init(|| {
        sha256(&PathBuf::from(format!("/proc/{}/exe", std::process::id())))
            .or_else(|| sha256(exe()))
    })
    .as_deref()
}

fn agent(timeout: Duration) -> ureq::Agent {
    let ua = format!("hokum/{}/{VERSION}", channel(VERSION).name());
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(ua)
        .build()
        .into()
}

fn fetch(channel: Channel) -> Result<Release, String> {
    let url = format!("{HOST}/release/hokum/desktop/{}.json", channel.name());
    let body = agent(Duration::from_secs(10))
        .get(&url)
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|e| format!("couldn't fetch {url}: {e}"))?;
    serde_json::from_str::<Manifest>(&body)
        .map(|m| m.release)
        .map_err(|e| format!("bad manifest at {url}: {e}"))
}

/// The command that updates a package manager's install, from the manifest when it names
/// this package (with the AUR helper that is actually installed).
pub fn command(
    release: &Release,
    manager: &str,
    package: &str,
    helper: impl Fn(&str) -> bool,
) -> Option<String> {
    let kind = match manager {
        "pacman" => "aur".to_owned(),
        "dpkg" => format!("deb-{}", arch()),
        "rpm" => format!("rpm-{}", arch()),
        _ => return None,
    };
    let d = release
        .distributions
        .iter()
        .find(|d| d.kind == kind && d.package.as_deref() == Some(package))?;
    let cmd = d.command.clone()?;
    if manager == "pacman"
        && let Some(rest) = cmd.strip_prefix("yay ")
        && !helper("yay")
        && helper("paru")
    {
        return Some(format!("paru {rest}"));
    }
    Some(cmd)
}

/// What the card offers.
#[derive(Clone, Debug)]
struct Offer {
    version: String,
    target: Target,
    /// For a package manager's install: the command, if known.
    command: Option<String>,
}

#[derive(Default)]
struct State {
    offer: Option<Offer>,
    /// The sha the card shows now.
    shown: Option<String>,
    dismissed: Option<(String, Instant)>,
    busy: bool,
}

fn state() -> std::sync::MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn note(summary: impl Into<String>, body: impl Into<String>, button: Option<&str>) -> Note {
    let mut n = Note::new(ID, "Hokum", summary, body);
    if let Some(label) = button {
        n.actions = vec![("update".into(), label.into())];
        n.has_default = true;
    }
    n.timeout = Timeout::Never;
    n.desktop = Some("hokum".into());
    n.icon_src = crate::icons::for_app("", Some("hokum"), "Hokum").map(Source::File);
    n
}

fn available(o: &Offer) -> Note {
    let title = crate::i18n::update_available(&o.version);
    match (install(), &o.command) {
        (Install::Direct(_), _) => note(
            title,
            crate::i18n::update_now(),
            Some(crate::i18n::t("Update")),
        ),
        (Install::Package { .. }, Some(cmd)) => note(
            title,
            crate::i18n::update_with(cmd),
            Some(crate::i18n::t("Copy command")),
        ),
        (Install::Package { package, .. }, None) => note(
            title,
            crate::i18n::update_package(package),
            Some(crate::i18n::t("Open hokum.app")),
        ),
        _ => note(
            title,
            crate::i18n::update_elsewhere(),
            Some(crate::i18n::t("Open hokum.app")),
        ),
    }
}

fn show(handle: &OverlayHandle<Ui>, n: Note, sha: Option<String>) {
    handle.update(move |ui, _| {
        ui.upsert(n);
        let visible = ui.showing(ID);
        state().shown = if visible { sha } else { None };
    });
}

/// The card as a direct install sees it, for `hokum service --snap --scene update`.
pub fn sample() -> Note {
    note(
        "Hokum 0.2.0 is available",
        "Update now; Hokum restarts in a moment.",
        Some("Update"),
    )
}

/// The service's checks, until it exits.
pub fn spawn(handle: OverlayHandle<Ui>) {
    std::thread::spawn(move || {
        if *install() == Install::Source {
            return;
        }
        let channel = channel(VERSION);
        std::thread::sleep(FIRST);
        loop {
            if crate::config::load().behaviour.check_for_updates
                && let Ok(Some(offer)) = check(channel)
            {
                offer_card(&handle, offer);
            }
            let jitter = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_millis() as u64)
                * 300;
            std::thread::sleep(channel.every() + Duration::from_millis(jitter));
        }
    });
}

fn check(channel: Channel) -> Result<Option<Offer>, String> {
    let running = running().ok_or("couldn't read the running executable")?;
    let release = fetch(channel)?;
    let Some(target) = newer(&release, running) else {
        return Ok(None);
    };
    let command = match install() {
        Install::Package { manager, package } => command(&release, manager, package, has),
        _ => None,
    };
    Ok(Some(Offer {
        version: release.version.clone(),
        target: target.clone(),
        command,
    }))
}

fn offer_card(handle: &OverlayHandle<Ui>, offer: Offer) {
    let sha = offer.target.sha256.clone();
    let n = {
        let mut s = state();
        s.offer = Some(offer.clone());
        if s.busy
            || s.shown.as_deref() == Some(sha.as_str())
            || !due(&sha, s.dismissed.as_ref(), Instant::now())
        {
            return;
        }
        available(&offer)
    };
    show(handle, n, Some(sha));
}

/// The card left; a dismissal keeps that build quiet for a while.
pub fn closed(reason: Reason) {
    let mut s = state();
    if let (Reason::Dismissed, Some(sha)) = (reason, s.shown.take()) {
        s.dismissed = Some((sha, Instant::now()));
    }
}

/// The card's button or a click on it. True when the card should close.
pub fn click(handle: Option<OverlayHandle<Ui>>, token: Option<String>) -> bool {
    let (offer, sha) = {
        let s = state();
        match (&s.offer, s.busy) {
            (Some(o), false) => (o.clone(), s.shown.clone()),
            (_, true) => return false,
            (None, false) => return true,
        }
    };
    match install() {
        Install::Direct(path) => {
            let Some(handle) = handle else { return false };
            state().busy = true;
            let path = path.clone();
            show(
                &handle,
                note(
                    crate::i18n::updating(&offer.version),
                    crate::i18n::t("Downloading and checking it."),
                    None,
                ),
                sha.clone(),
            );
            std::thread::spawn(move || {
                let result = replace(&path, &offer.target).and_then(|()| {
                    mark();
                    restart()
                });
                state().busy = false;
                if let Err(e) = result {
                    eprintln!("hokum: update: {e}");
                    show(
                        &handle,
                        note(
                            crate::i18n::t("Hokum couldn't update"),
                            format!("{e} Still running {VERSION}."),
                            Some(crate::i18n::t("Try again")),
                        ),
                        sha,
                    );
                }
            });
            false
        }
        Install::Package { .. } if offer.command.is_some() => {
            let cmd = offer.command.clone().unwrap_or_default();
            std::thread::spawn(move || {
                if !copy(&cmd) {
                    open(token);
                }
            });
            closed(Reason::Dismissed);
            true
        }
        _ => {
            open(token);
            closed(Reason::Dismissed);
            true
        }
    }
}

fn copy(text: &str) -> bool {
    use std::io::Write;
    let Ok(mut child) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|s| s.success()) && wrote
}

fn open(token: Option<String>) {
    let mut cmd = Command::new("xdg-open");
    cmd.arg(SITE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(t) = token {
        cmd.env("XDG_ACTIVATION_TOKEN", t);
    }
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || child.wait());
    }
}

/// Downloads `t` next to `path`, checks its size and sha256 and renames it over `path`. The
/// old executable stays on any failure.
fn replace(path: &Path, t: &Target) -> Result<(), String> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().ok_or("no folder for the executable")?;
    let tmp = dir.join(format!(".hokum-update-{}", std::process::id()));
    let url = if t.url.starts_with("http") {
        t.url.clone()
    } else {
        format!("{HOST}{}", t.url)
    };
    let fail = |msg: String| {
        let _ = std::fs::remove_file(&tmp);
        Err(msg)
    };
    let download = || -> Result<(), String> {
        let mut r = agent(Duration::from_secs(15 * 60))
            .get(&url)
            .call()
            .map_err(|e| e.to_string())?;
        let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut r.body_mut().as_reader().take(t.size + 1), &mut file)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())
    };
    if let Err(e) = download() {
        return fail(crate::i18n::download_failed(&e));
    }
    if std::fs::metadata(&tmp).map(|m| m.len()).ok() != Some(t.size) {
        return fail(crate::i18n::t("The download was the wrong size.").into());
    }
    if sha256(&tmp).as_deref() != Some(t.sha256.to_lowercase().as_str()) {
        return fail(crate::i18n::t("The download didn't match its checksum.").into());
    }
    if let Err(e) = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)) {
        return fail(crate::i18n::not_executable(&e.to_string()));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        return fail(crate::i18n::couldnt_replace(
            &path.display().to_string(),
            &e.to_string(),
        ));
    }
    Ok(())
}

fn marker() -> PathBuf {
    crate::style::state_dir().join("updated")
}

fn mark() {
    let _ = std::fs::create_dir_all(crate::style::state_dir());
    let _ = std::fs::write(marker(), "");
}

/// After an update: one card saying so, at the new build's start.
pub fn greeting() -> Option<Note> {
    std::fs::remove_file(marker()).ok()?;
    let mut n = note(
        crate::i18n::t("Hokum updated"),
        crate::i18n::now_running(VERSION),
        None,
    );
    n.timeout = Timeout::Default;
    Some(n)
}

/// The systemd unit this process runs in, from its cgroup.
fn unit() -> Option<String> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    path.rsplit('/')
        .find(|s| s.ends_with(".service"))
        .map(str::to_owned)
}

/// Into the new executable: systemd restarts the unit, otherwise exec it with the same args.
fn restart() -> Result<(), String> {
    if std::env::var_os("INVOCATION_ID").is_some()
        && let Some(unit) = unit()
    {
        let ok = Command::new("systemctl")
            .args(["--user", "--no-block", "restart", &unit])
            .status()
            .is_ok_and(|s| s.success());
        return if ok {
            Ok(())
        } else {
            Err(crate::i18n::restart_failed(&unit))
        };
    }
    use std::os::unix::process::CommandExt;
    let e = Command::new(exe()).args(std::env::args_os().skip(1)).exec();
    Err(crate::i18n::no_restart(&e.to_string()))
}

/// `hokum update [--install]`.
pub fn cli(args: &[String]) -> i32 {
    let ch = channel(VERSION);
    let how = install();
    println!(
        "Hokum {VERSION} ({} channel) at {}",
        ch.name(),
        exe().display()
    );
    if *how == Install::Source {
        println!("This is a cargo build; update it from source.");
        return 0;
    }
    let offer = match check(ch) {
        Ok(Some(o)) => o,
        Ok(None) => {
            println!("Up to date.");
            return 0;
        }
        Err(e) => {
            eprintln!("hokum: {e}");
            return 1;
        }
    };
    println!("Hokum {} is available.", offer.version);
    let install_now = args.iter().any(|a| a == "--install");
    match how {
        Install::Direct(path) if install_now => {
            if let Err(e) = replace(path, &offer.target) {
                eprintln!("hokum: {e}");
                return 1;
            }
            println!("Installed Hokum {} at {}.", offer.version, path.display());
            let active = Command::new("systemctl")
                .args(["--user", "is-active", "--quiet", "hokum.service"])
                .status()
                .is_ok_and(|s| s.success());
            if active {
                mark();
                let ok = Command::new("systemctl")
                    .args(["--user", "restart", "hokum.service"])
                    .status()
                    .is_ok_and(|s| s.success());
                println!(
                    "{}",
                    if ok {
                        "Restarted hokum.service."
                    } else {
                        "Restart hokum.service to use it."
                    }
                );
            } else {
                println!("Restart Hokum to use it.");
            }
            0
        }
        Install::Direct(_) => {
            println!("Run `hokum update --install` (or click the card) to install it.");
            0
        }
        Install::Package { package, .. } => {
            match &offer.command {
                Some(cmd) => println!("Update it with your package manager: {cmd}"),
                None => println!("Update the {package} package with your package manager."),
            }
            i32::from(install_now)
        }
        _ => {
            println!(
                "{} isn't writable; update Hokum the way you installed it ({SITE}).",
                exe().display()
            );
            i32::from(install_now)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(sha: &str) -> Release {
        let json = format!(
            r#"{{"version":"0.0.0-dev-23","targets":{{"linux-x64":{{"url":"/x","size":1,"sha256":"{sha}"}},"linux-arm64":{{"url":"/y","size":1,"sha256":"{sha}"}}}},
            "distributions":[{{"kind":"direct"}},{{"kind":"aur","command":"yay -Syu hokum-bin","package":"hokum-bin"}},
            {{"kind":"deb-{a}","command":"sudo apt install /tmp/hokum.deb","package":"hokum"}}]}}"#,
            a = arch()
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn the_channel_comes_from_the_version() {
        assert_eq!(channel("0.0.0-dev-22"), Channel::Dev);
        assert_eq!(channel("0.1.1"), Channel::Stable);
        assert_eq!(channel("0.2.0-beta.1"), Channel::Beta);
    }

    #[test]
    fn a_build_is_current_when_its_sha_matches() {
        let r = release("ABC");
        assert!(newer(&r, "abc").is_none());
        assert!(newer(&r, "def").is_some());
    }

    #[test]
    fn the_install_decides_what_updating_does() {
        let home = Path::new("/home/d/.local/bin/hokum");
        assert_eq!(classify(home, None, true), Install::Direct(home.to_owned()));
        assert_eq!(classify(home, None, false), Install::Unknown);
        assert_eq!(
            classify(Path::new("/usr/bin/hokum"), None, true),
            Install::Unknown
        );
        assert_eq!(
            classify(
                Path::new("/usr/bin/hokum"),
                Some(("pacman", "hokum-bin".into())),
                false
            ),
            Install::Package {
                manager: "pacman",
                package: "hokum-bin".into()
            }
        );
        assert_eq!(
            classify(Path::new("/home/d/hokum/target/release/hokum"), None, true),
            Install::Source
        );
    }

    #[test]
    fn package_commands_come_from_the_manifest_with_the_installed_helper() {
        let r = release("abc");
        assert_eq!(
            command(&r, "pacman", "hokum-bin", |h| h == "paru").as_deref(),
            Some("paru -Syu hokum-bin")
        );
        assert_eq!(
            command(&r, "pacman", "hokum-bin", |_| true).as_deref(),
            Some("yay -Syu hokum-bin")
        );
        assert_eq!(command(&r, "pacman", "hokum-git", |_| true), None);
        assert_eq!(
            command(&r, "dpkg", "hokum", |_| true).as_deref(),
            Some("sudo apt install /tmp/hokum.deb")
        );
    }

    #[test]
    fn a_dismissed_build_stays_quiet_for_a_day() {
        let now = Instant::now();
        let dismissed = ("abc".to_owned(), now);
        assert!(due("abc", None, now));
        assert!(!due(
            "abc",
            Some(&dismissed),
            now + Duration::from_secs(3600)
        ));
        assert!(due("def", Some(&dismissed), now));
        assert!(due("abc", Some(&dismissed), now + QUIET));
    }
}
