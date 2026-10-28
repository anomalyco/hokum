//! The update check: the service fetches its channel's release manifest at startup and then
//! hourly, as `hokum/<channel>/<version>`, and notes in its log when a newer version is out.
//! It never updates anything (package managers do) and fails silently. `check-for-updates
//! #false` in config.kdl's `behaviour` turns it off.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const EVERY: Duration = Duration::from_secs(60 * 60);

/// The release channel a version belongs to: `0.0.0-dev-<n>` is dev, `<v>-beta.<n>` beta.
fn channel(version: &str) -> &'static str {
    if version.starts_with("0.0.0-dev-") {
        "dev"
    } else if version.contains("-beta.") {
        "beta"
    } else {
        "stable"
    }
}

/// The version's numbers in order, so `0.0.0-dev-22` > `0.0.0-dev-9` and `0.2.0` > `0.1.9`.
fn numbers(version: &str) -> Vec<u64> {
    version.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect()
}

fn newer(remote: &str, local: &str) -> bool {
    numbers(remote) > numbers(local)
}

#[derive(serde::Deserialize)]
struct Manifest {
    release: Release,
}

#[derive(serde::Deserialize)]
struct Release {
    version: String,
}

fn fetch(agent: &ureq::Agent, url: &str) -> Option<String> {
    let body = agent.get(url).call().ok()?.body_mut().read_to_string().ok()?;
    Some(serde_json::from_str::<Manifest>(&body).ok()?.release.version)
}

pub fn spawn() {
    std::thread::spawn(|| {
        let channel = channel(VERSION);
        let url = format!("https://anoma.ly/release/hokum/desktop/{channel}.json");
        let agent: ureq::Agent =
            ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).user_agent(format!("hokum/{channel}/{VERSION}")).build().into();
        let mut told = String::new();
        loop {
            if crate::config::load().behaviour.check_for_updates
                && let Some(v) = fetch(&agent, &url)
                && newer(&v, VERSION)
                && v != told
            {
                eprintln!("hokum: {v} is available (running {VERSION}); update through your package manager");
                told = v;
            }
            let jitter = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_millis() as u64) * 300;
            std::thread::sleep(EVERY + Duration::from_millis(jitter));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels() {
        assert_eq!(channel("0.0.0-dev-19"), "dev");
        assert_eq!(channel("0.2.0-beta.3"), "beta");
        assert_eq!(channel("0.1.0"), "stable");
    }

    #[test]
    fn newer_versions() {
        assert!(newer("0.0.0-dev-22", "0.0.0-dev-9"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(!newer("0.1.0", "0.1.1"));
        assert!(!newer("0.1.1", "0.1.1"));
    }
}
