//! Whether a newer NeoAmp has been released. At startup, at most once a day, NeoAmp asks
//! GitHub's API for the latest release, on a thread of its own, and tells of one newer than
//! itself once a build for this system and processor is attached to it: the release workflow
//! attaches the builds a while after the release is published, and not every platform has
//! one.
//!
//! The check fails silently. No network, an error from GitHub or an answer this version cannot
//! read is logged at debug level and nothing more, and the check is due again at the next
//! start: only an answer that was read counts as the day's check.
//!
//! The request says who asks (`User-Agent: NeoAmp/<version>`, from [`crate::http`]) and
//! nothing else. Releases are tagged with a `v` and the version (`v0.2.0`), and their builds
//! named `neoamp-<tag>-<system>-<processor>` and a `.` or `-` (`neoamp-v0.2.0-macos-arm64.dmg`,
//! `neoamp-v0.2.0-windows-x86_64-setup.exe`; see `.github/workflows/release.yml`).

use std::fmt;
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::{Context as _, Result, bail};
use jiff::civil::Date;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::http::{self, Timeouts, Url};

/// NeoAmp's releases on GitHub.
pub const RELEASES: &str = "https://github.com/BlakeGardner/NeoAmp/releases";
/// GitHub's API for the latest release, which is never a draft or a pre-release.
const LATEST: &str = "https://api.github.com/repos/BlakeGardner/NeoAmp/releases/latest";
/// The most an answer may take. One is a few kilobytes, more with long release notes.
const MAX_ANSWER_BYTES: usize = 1 << 20;

/// The system this NeoAmp was built for, as the builds' names give it; `None` for one that
/// releases have no builds for.
const SYSTEM: Option<&str> = if cfg!(target_os = "linux") {
    Some("linux")
} else if cfg!(target_os = "macos") {
    Some("macos")
} else if cfg!(target_os = "windows") {
    Some("windows")
} else {
    None
};

/// The processor this NeoAmp was built for, by the names builds may give it: 64-bit ARM is
/// `arm64`, as on macOS, or `aarch64`, as Linux calls it. None for one that releases have no
/// builds for.
const PROCESSOR: &[&str] = if cfg!(target_arch = "x86_64") {
    &["x86_64"]
} else if cfg!(target_arch = "aarch64") {
    &["arm64", "aarch64"]
} else {
    &[]
};

/// A release's version: three numbers, as in `0.2.0`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// This NeoAmp's version.
    pub const CURRENT: Version = Version::new(
        digits(env!("CARGO_PKG_VERSION_MAJOR")),
        digits(env!("CARGO_PKG_VERSION_MINOR")),
        digits(env!("CARGO_PKG_VERSION_PATCH")),
    );

    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }
}

/// A number Cargo gives in decimal digits, read at compile time.
const fn digits(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut n = 0;
    let mut i = 0;
    while i < bytes.len() {
        assert!(bytes[i].is_ascii_digit(), "not a version number");
        n = n * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    n
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for Version {
    type Err = anyhow::Error;

    /// `0.2.0`, or a release's tag, `v0.2.0`. A pre-release's (`0.3.0-rc.1`) is not a version
    /// here.
    fn from_str(s: &str) -> Result<Version> {
        // Digits only: `u32::from_str` would take a leading `+` too.
        let number = |part: &str| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let parts: Vec<&str> = s.strip_prefix('v').unwrap_or(s).split('.').collect();
        if let [major, minor, patch] = parts[..]
            && let (Some(major), Some(minor), Some(patch)) =
                (number(major), number(minor), number(patch))
        {
            return Ok(Version::new(major, minor, patch));
        }
        bail!("{s:?} is not a version such as 0.2.0")
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The page of the release of `version`, with its downloads and what is new in it.
#[must_use]
pub fn release_page(version: Version) -> String {
    format!("{RELEASES}/tag/v{version}")
}

/// Today in the local time zone, or in UTC where the time zone cannot be found.
#[must_use]
pub fn today() -> Date {
    jiff::Zoned::now().date()
}

/// Whether a check is due `today`, the last one having been made on `last`: one a day. A last
/// check dated after today, because the clock was put back, does not hold the next one up.
#[must_use]
pub fn is_due(last: Option<Date>, today: Date) -> bool {
    last != Some(today)
}

/// Whether the latest release is worth telling of: newer than this NeoAmp, `current`, and
/// than the release the user chose to skip.
fn is_news(latest: Version, current: Version, skipped: Option<Version>) -> bool {
    latest > current && skipped.is_none_or(|s| latest > s)
}

/// The parts of GitHub's answer about a release that matter here.
#[derive(Deserialize)]
struct Answer {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    /// The files attached to the release.
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

/// Whether the file `name` is a build of the release tagged `tag` for `system` and the
/// processor `processor`: `neoamp-<tag>-<system>-<processor>`, then a `.` or `-`. The last
/// keeps `x86_64` from matching another processor's name that starts the same way.
fn is_build_for(name: &str, tag: &str, system: &str, processor: &str) -> bool {
    name.strip_prefix(&format!("neoamp-{tag}-{system}-{processor}"))
        .is_some_and(|rest| rest.starts_with(['.', '-']))
}

/// Whether `assets`, the files of the release tagged `tag`, hold a build for `system` and a
/// processor of one of the names `processor` gives. Where releases have no builds for the
/// system or the processor (`None`, or no names), any file will do: NeoAmp was built from
/// source there, and can be again.
fn has_build(assets: &[Asset], tag: &str, system: Option<&str>, processor: &[&str]) -> bool {
    match system {
        Some(system) if !processor.is_empty() => assets.iter().any(|a| {
            processor
                .iter()
                .any(|p| is_build_for(&a.name, tag, system, p))
        }),
        _ => !assets.is_empty(),
    }
}

/// The version of the latest release in GitHub's `answer`; `None` while no build for this
/// system and processor is attached to it.
///
/// # Errors
/// Fails if the answer is not about a release, or the release's tag is not a version.
fn latest(answer: &[u8]) -> Result<Option<Version>> {
    let release: Answer = serde_json::from_slice(answer).context("reading GitHub's answer")?;
    let version: Version = release.tag_name.parse()?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let built = has_build(&release.assets, &release.tag_name, SYSTEM, PROCESSOR);
    Ok(built.then_some(version))
}

/// Ask `api` for the latest release.
///
/// # Errors
/// Fails if it cannot be reached, or its answer cannot be read.
fn fetch_from(api: &str) -> Result<Option<Version>> {
    let never = || false;
    let mut response = http::get(&Url::parse(api)?, Timeouts::default(), &never)?;
    latest(&response.read_to_end_limited(MAX_ANSWER_BYTES, &never)?)
}

/// How a check is going.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Progress {
    /// No answer yet.
    Waiting,
    /// GitHub could not be asked, or its answer could not be read.
    Failed,
    /// GitHub answered, with a release worth telling of or none.
    Answered(Option<Version>),
}

/// A check under way, on a thread of its own. Dropping it forgets the answer.
#[derive(Debug)]
pub struct Check {
    answer: Receiver<Option<Version>>,
}

impl Check {
    /// Ask GitHub for the latest release, to tell of it if it is newer than this NeoAmp and
    /// than the release the user chose to skip. `None` if the thread cannot be started.
    #[must_use]
    pub fn start(skipped: Option<Version>) -> Option<Check> {
        Check::start_from(LATEST.to_owned(), skipped)
    }

    fn start_from(api: String, skipped: Option<Version>) -> Option<Check> {
        let (tx, answer) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("update check".into())
            .spawn(move || {
                let latest = match fetch_from(&api) {
                    Ok(latest) => latest,
                    Err(e) => {
                        log::debug!("cannot check for a new release: {e:#}");
                        return;
                    }
                };
                let news = latest.filter(|&v| is_news(v, Version::CURRENT, skipped));
                match (latest, news) {
                    (_, Some(v)) => log::info!("NeoAmp {v} is out; this is {}", Version::CURRENT),
                    (Some(v), None) if skipped == Some(v) => {
                        log::debug!("NeoAmp {v} is out, and was skipped");
                    }
                    _ => log::debug!("no newer release than NeoAmp {}", Version::CURRENT),
                }
                // The receiver is gone if the checks were turned off meanwhile.
                let _ = tx.send(news);
            });
        match spawned {
            Ok(_) => Some(Check { answer }),
            Err(e) => {
                log::debug!("cannot start checking for a new release: {e}");
                None
            }
        }
    }

    /// How the check is going. The answer is given once; after it the check reports
    /// [`Progress::Failed`], so take it then.
    #[must_use]
    pub fn poll(&self) -> Progress {
        match self.answer.try_recv() {
            Ok(news) => Progress::Answered(news),
            Err(TryRecvError::Empty) => Progress::Waiting,
            Err(TryRecvError::Disconnected) => Progress::Failed,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::http::tests::{listen, serve};
    use std::time::{Duration, Instant};

    fn v(major: u32, minor: u32, patch: u32) -> Version {
        Version::new(major, minor, patch)
    }

    fn ok(body: &str) -> Vec<Vec<u8>> {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        vec![head.into_bytes(), body.as_bytes().to_vec()]
    }

    /// GitHub's answer about release `tag` with these files, cut down to what is read and a
    /// little more.
    fn answer(tag: &str, assets: &[&str]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|name| format!(r#"{{"name":"{name}","size":1234}}"#))
            .collect();
        format!(
            r#"{{"tag_name":"{tag}","name":"NeoAmp {tag}","draft":false,"prerelease":false,"body":"What is new.","assets":[{}]}}"#,
            assets.join(",")
        )
    }

    /// The names [`PROCESSOR`] gives each processor.
    const X86_64: &[&str] = &["x86_64"];
    const ARM64: &[&str] = &["arm64", "aarch64"];

    /// The files of release `tag`, named as the release workflow names them, from `files`
    /// like `macos-arm64.dmg`.
    fn files(tag: &str, files: &[&str]) -> Vec<Asset> {
        files
            .iter()
            .map(|file| Asset {
                name: format!("neoamp-{tag}-{file}"),
            })
            .collect()
    }

    /// The files a release has today.
    const TODAYS_FILES: [&str; 6] = [
        "linux-x86_64.flatpak",
        "linux-x86_64.tar.gz",
        "macos-arm64.dmg",
        "macos-x86_64.dmg",
        "windows-x86_64-setup.exe",
        "windows-x86_64.zip",
    ];

    /// A build of release `tag` for the system and processor the tests run on, or any file
    /// where releases have no builds for them.
    fn this_build(tag: &str) -> String {
        let system = SYSTEM.unwrap_or("other");
        let processor = PROCESSOR.first().copied().unwrap_or("other");
        format!("neoamp-{tag}-{system}-{processor}.zip")
    }

    fn wait(check: &Check) -> Progress {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let progress = check.poll();
            if progress != Progress::Waiting || Instant::now() > deadline {
                return progress;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn versions_are_read_written_and_ordered() {
        assert_eq!("0.2.0".parse::<Version>().unwrap(), v(0, 2, 0));
        assert_eq!("v1.10.3".parse::<Version>().unwrap(), v(1, 10, 3));
        assert_eq!(v(1, 10, 3).to_string(), "1.10.3");
        for bad in [
            "",
            "v",
            "1.2",
            "1.2.3.4",
            "0.3.0-rc.1",
            "+1.2.3",
            "1..3",
            "V1.2.3",
            "x",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?}");
        }
        assert!(v(0, 10, 0) > v(0, 9, 9), "numbers, not text");
        assert!(v(1, 0, 0) > v(0, 99, 99));
        assert!(v(0, 2, 1) > v(0, 2, 0));
    }

    #[test]
    fn this_version_is_cargos() {
        assert_eq!(
            Version::CURRENT.to_string(),
            env!("CARGO_PKG_VERSION"),
            "a release's version has no pre-release part"
        );
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let today = jiff::civil::date(2026, 10, 23);
        assert!(is_due(None, today), "never checked");
        assert!(!is_due(Some(today), today), "checked today");
        assert!(is_due(Some(jiff::civil::date(2026, 10, 22)), today));
        assert!(
            is_due(Some(jiff::civil::date(2026, 10, 24)), today),
            "the clock was put back"
        );
    }

    #[test]
    fn only_a_release_newer_than_this_and_the_skipped_one_is_news() {
        let current = v(0, 2, 0);
        assert!(is_news(v(0, 3, 0), current, None));
        assert!(!is_news(v(0, 2, 0), current, None), "this one");
        assert!(!is_news(v(0, 1, 0), current, None), "an older one");
        assert!(!is_news(v(0, 3, 0), current, Some(v(0, 3, 0))), "skipped");
        assert!(is_news(v(0, 4, 0), current, Some(v(0, 3, 0))), "after it");
    }

    #[test]
    fn a_build_is_named_for_its_system_and_processor() {
        let is = |name, system, processor| is_build_for(name, "v0.3.0", system, processor);
        assert!(is("neoamp-v0.3.0-macos-arm64.dmg", "macos", "arm64"));
        assert!(!is("neoamp-v0.3.0-macos-arm64.dmg", "macos", "x86_64"));
        assert!(!is("neoamp-v0.3.0-macos-arm64.dmg", "linux", "arm64"));
        assert!(is(
            "neoamp-v0.3.0-windows-x86_64-setup.exe",
            "windows",
            "x86_64"
        ));
        assert!(is("neoamp-v0.3.0-linux-aarch64.tar.gz", "linux", "aarch64"));
        // Another processor whose name starts the same way.
        assert!(!is(
            "neoamp-v0.3.0-linux-x86_64_v3.tar.gz",
            "linux",
            "x86_64"
        ));
        assert!(!is("neoamp-v0.3.0-macos-arm64e.dmg", "macos", "arm64"));
        // Another release's, and not a build at all.
        assert!(!is("neoamp-v0.2.0-macos-arm64.dmg", "macos", "arm64"));
        assert!(!is("neoamp-v0.3.0-macos-arm64", "macos", "arm64"));
        assert!(!is("checksums.txt", "macos", "arm64"));
    }

    #[test]
    fn a_release_counts_once_a_build_for_this_system_and_processor_is_attached() {
        let today = files("v0.3.0", &TODAYS_FILES);
        let built = |assets: &[Asset], system, processor| {
            has_build(assets, "v0.3.0", Some(system), processor)
        };
        assert!(built(&today, "linux", X86_64));
        assert!(built(&today, "macos", ARM64));
        assert!(built(&today, "macos", X86_64));
        assert!(built(&today, "windows", X86_64));
        assert!(!built(&today, "linux", ARM64), "no ARM Linux build yet");
        assert!(!built(&today, "windows", ARM64));
        for arm_linux in ["linux-arm64.tar.gz", "linux-aarch64.flatpak"] {
            let mut later = files("v0.3.0", &TODAYS_FILES);
            later.extend(files("v0.3.0", &[arm_linux]));
            assert!(built(&later, "linux", ARM64), "{arm_linux}");
        }
        // Published, but nothing built yet.
        assert!(!built(&[], "macos", ARM64));
        assert!(!has_build(&[], "v0.3.0", None, &[]));
        // Built for an earlier release only.
        assert!(!built(&files("v0.2.0", &TODAYS_FILES), "macos", ARM64));
        // Where releases have no builds, any file will do.
        assert!(has_build(&today, "v0.3.0", None, X86_64));
        assert!(has_build(&today, "v0.3.0", Some("linux"), &[]));
    }

    #[test]
    fn the_latest_release_counts_once_built_for_here() {
        let tag = "v0.3.0";
        assert_eq!(
            latest(answer(tag, &[&this_build(tag)]).as_bytes()).unwrap(),
            Some(v(0, 3, 0))
        );
        assert_eq!(
            latest(answer(tag, &[]).as_bytes()).unwrap(),
            None,
            "published, not built yet"
        );
        let pre = answer(tag, &[&this_build(tag)])
            .replace(r#""prerelease":false"#, r#""prerelease":true"#);
        assert_eq!(latest(pre.as_bytes()).unwrap(), None);
    }

    #[test]
    fn an_answer_that_cannot_be_read_is_an_error() {
        assert!(latest(b"<html>rate limited</html>").is_err());
        assert!(latest(br#"{"message":"Not Found"}"#).is_err(), "no tag");
        assert!(latest(answer("nightly", &[]).as_bytes()).is_err());
        assert!(latest(br#"{"tag_name":7}"#).is_err());
    }

    #[test]
    fn the_release_page_is_on_github() {
        assert_eq!(
            release_page(v(0, 3, 0)),
            "https://github.com/BlakeGardner/NeoAmp/releases/tag/v0.3.0"
        );
    }

    #[test]
    fn a_check_asks_the_api_and_tells_of_a_newer_release() {
        let tag = "v999.0.0";
        let (address, server) = serve(vec![ok(&answer(tag, &[&this_build(tag)]))], Duration::ZERO);
        let check = Check::start_from(format!("{address}/releases/latest"), None).unwrap();
        assert_eq!(wait(&check), Progress::Answered(Some(v(999, 0, 0))));
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /releases/latest HTTP/1.1\r\n"));
        assert!(requests[0].contains("User-Agent: NeoAmp/"));
        assert_eq!(check.poll(), Progress::Failed, "the answer is given once");
    }

    #[test]
    fn a_check_of_an_older_or_skipped_release_answers_none() {
        for (tag, skipped) in [("v0.0.1", None), ("v999.0.0", Some(v(999, 0, 0)))] {
            let reply = ok(&answer(tag, &[&this_build(tag)]));
            let (address, _server) = serve(vec![reply], Duration::ZERO);
            let check = Check::start_from(address, skipped).unwrap();
            assert_eq!(wait(&check), Progress::Answered(None), "{tag}");
        }
    }

    #[test]
    fn a_check_fails_quietly() {
        // Nobody listening.
        let (listener, address) = listen();
        drop(listener);
        let check = Check::start_from(address, None).unwrap();
        assert_eq!(wait(&check), Progress::Failed);
        // An error from GitHub, and an answer that is not a release.
        let refused =
            vec![b"HTTP/1.1 403 rate limit exceeded\r\nContent-Length: 0\r\n\r\n".to_vec()];
        for reply in [refused, ok("{}")] {
            let (address, _server) = serve(vec![reply], Duration::ZERO);
            let check = Check::start_from(address, None).unwrap();
            assert_eq!(wait(&check), Progress::Failed);
        }
    }
}
