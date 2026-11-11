use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::Deserialize;
use thiserror::Error;

pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/xahon/kdiff4/releases/latest";
/// Replaces `LATEST_RELEASE_URL`, to try the window against a local server.
pub const URL_OVERRIDE_ENV: &str = "KDIFF4_UPDATE_URL";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("cannot reach the update server: {0}")]
    Request(String),
    #[error("the update server sent an unreadable answer: {0}")]
    Response(String),
    #[error("the release tag {tag:?} is not a version number")]
    Version { tag: String },
    #[error("the update check stopped before it finished")]
    Stopped,
    #[error("cannot download the update: {0}")]
    Download(String),
    #[error("cannot install the update: {0}")]
    Install(String),
    #[error("the download was canceled")]
    Canceled,
}

/// The newest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: Version,
    pub tag: String,
    /// Markdown written on the release page.
    pub notes: String,
    pub page_url: String,
    pub assets: Vec<ReleaseAsset>,
}

impl ReleaseInfo {
    /// The file to download for this platform, if the release has one.
    pub fn asset_for_this_platform(&self) -> Option<&ReleaseAsset> {
        let name = platform_asset_name();
        self.assets.iter().find(|asset| asset.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub size: u64,
    pub url: String,
}

/// `kdiff4-<os>-<arch>`, plus `.exe` on Windows: the name the release workflow gives each platform's single executable.
pub fn platform_asset_name() -> String {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let extension = if cfg!(windows) { ".exe" } else { "" };
    format!("kdiff4-{os}-{}{extension}", std::env::consts::ARCH)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    Available(ReleaseInfo),
    UpToDate,
    /// A newer release the user chose to skip; only an automatic check stays quiet about it.
    Skipped,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("App: the package version is not a semantic version")
}

pub fn parse_release(json: &str) -> Result<ReleaseInfo, UpdateError> {
    let raw: RawRelease = serde_json::from_str(json).map_err(|error| UpdateError::Response(error.to_string()))?;
    let version = Version::parse(raw.tag_name.trim().trim_start_matches(['v', 'V'])).map_err(|_| UpdateError::Version { tag: raw.tag_name.clone() })?;
    Ok(ReleaseInfo {
        version,
        tag: raw.tag_name,
        notes: raw.body.unwrap_or_default().replace("\r\n", "\n"),
        page_url: raw.html_url,
        assets: raw
            .assets
            .into_iter()
            .map(|asset| ReleaseAsset {
                name: asset.name,
                size: asset.size,
                url: asset.browser_download_url,
            })
            .collect(),
    })
}

pub fn evaluate(current: &Version, release: ReleaseInfo, skipped: Option<&str>, manual: bool) -> CheckOutcome {
    if release.version <= *current {
        return CheckOutcome::UpToDate;
    }
    if !manual && skipped.is_some_and(|skipped| skipped == release.version.to_string()) {
        return CheckOutcome::Skipped;
    }
    CheckOutcome::Available(release)
}

/// An automatic check is due once a day, or when none ran yet (0). A last check in the future (the clock was set back) counts as due.
pub fn is_due(last_check_unix: u64, now_unix: u64) -> bool {
    last_check_unix == 0 || now_unix < last_check_unix || now_unix - last_check_unix >= CHECK_INTERVAL.as_secs()
}

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

pub fn release_url() -> String {
    std::env::var(URL_OVERRIDE_ENV).ok().filter(|url| !url.trim().is_empty()).unwrap_or_else(|| LATEST_RELEASE_URL.to_string())
}

pub fn fetch_latest(url: &str) -> Result<ReleaseInfo, UpdateError> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .user_agent(format!("kdiff4/{}", current_version()))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| UpdateError::Request(error.to_string()))?;
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .map_err(|error| UpdateError::Response(error.to_string()))?;
    parse_release(&text)
}

/// A check running on its own thread, so the window never waits for the network.
pub struct UpdateCheck {
    receiver: Receiver<Result<ReleaseInfo, UpdateError>>,
    /// Asked for by the user rather than by the daily timer.
    pub manual: bool,
}

impl UpdateCheck {
    /// `on_done` runs on the worker thread when the result is ready, to wake the window.
    pub fn start(url: String, manual: bool, on_done: impl FnOnce() + Send + 'static) -> UpdateCheck {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(fetch_latest(&url));
            on_done();
        });
        UpdateCheck { receiver, manual }
    }

    pub fn poll(&self) -> Option<Result<ReleaseInfo, UpdateError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(UpdateError::Stopped)),
        }
    }
}


#[cfg(test)]
pub(crate) mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    const RELEASE_JSON: &str = r###"{"tag_name":"v0.2.0","html_url":"https://github.com/xahon/kdiff4/releases/tag/v0.2.0","body":"## Changes\r\n- Faster diff\r\n","draft":false,"assets":[{"name":"kdiff4-windows-x86_64.exe","size":1234,"browser_download_url":"https://github.com/xahon/kdiff4/releases/download/v0.2.0/kdiff4-windows-x86_64.exe"},{"name":"kdiff4-linux-x86_64","size":99,"browser_download_url":"https://example.com/linux"}]}"###;

    fn release(version: &str) -> ReleaseInfo {
        ReleaseInfo {
            version: Version::parse(version).unwrap(),
            tag: format!("v{version}"),
            notes: String::new(),
            page_url: String::new(),
            assets: Vec::new(),
        }
    }

    pub(crate) fn serve_once(status_line: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/latest", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let response = format!("HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(response.as_bytes());
        });
        url
    }

    #[test]
    fn parses_tag_notes_and_page_url() {
        let parsed = parse_release(RELEASE_JSON).unwrap();
        assert_eq!(parsed.version, Version::new(0, 2, 0));
        assert_eq!(parsed.tag, "v0.2.0");
        assert_eq!(parsed.notes, "## Changes\n- Faster diff\n");
        assert_eq!(parsed.page_url, "https://github.com/xahon/kdiff4/releases/tag/v0.2.0");
    }

    #[test]
    fn parses_assets_and_picks_one_by_name() {
        let parsed = parse_release(RELEASE_JSON).unwrap();
        assert_eq!(parsed.assets.len(), 2);
        assert_eq!(parsed.assets[0].name, "kdiff4-windows-x86_64.exe");
        assert_eq!(parsed.assets[0].size, 1234);
        assert_eq!(parsed.assets[0].url, "https://github.com/xahon/kdiff4/releases/download/v0.2.0/kdiff4-windows-x86_64.exe");
    }

    #[test]
    fn release_without_a_file_for_this_platform_has_no_asset() {
        assert!(release("0.2.0").asset_for_this_platform().is_none());
        let mut with_asset = release("0.2.0");
        with_asset.assets.push(ReleaseAsset {
            name: platform_asset_name(),
            size: 5,
            url: "u".to_string(),
        });
        assert_eq!(with_asset.asset_for_this_platform().unwrap().size, 5);
    }

    #[test]
    fn platform_asset_name_follows_the_release_naming() {
        let name = platform_asset_name();
        assert!(name.starts_with("kdiff4-"));
        assert_eq!(name.ends_with(".exe"), cfg!(windows));
        assert!(name.contains(std::env::consts::ARCH));
    }

    #[test]
    fn tag_without_prefix_and_missing_notes_are_accepted() {
        let parsed = parse_release(r#"{"tag_name":"1.4.0-rc.1","html_url":"u","body":null}"#).unwrap();
        assert_eq!(parsed.version, Version::parse("1.4.0-rc.1").unwrap());
        assert_eq!(parsed.notes, "");
    }

    #[test]
    fn tag_that_is_not_a_version_is_an_error() {
        assert!(matches!(parse_release(r#"{"tag_name":"nightly","html_url":"u"}"#), Err(UpdateError::Version { tag }) if tag == "nightly"));
    }

    #[test]
    fn json_without_a_tag_is_an_error() {
        assert!(matches!(parse_release(r#"{"html_url":"u"}"#), Err(UpdateError::Response(_))));
        assert!(matches!(parse_release("not json"), Err(UpdateError::Response(_))));
    }

    #[test]
    fn newer_release_is_available() {
        let current = Version::new(0, 1, 0);
        assert_eq!(evaluate(&current, release("0.2.0"), None, false), CheckOutcome::Available(release("0.2.0")));
        assert_eq!(evaluate(&current, release("0.1.1"), None, false), CheckOutcome::Available(release("0.1.1")));
    }

    #[test]
    fn same_or_older_release_is_up_to_date() {
        let current = Version::new(0, 2, 0);
        assert_eq!(evaluate(&current, release("0.2.0"), None, true), CheckOutcome::UpToDate);
        assert_eq!(evaluate(&current, release("0.1.9"), None, true), CheckOutcome::UpToDate);
    }

    #[test]
    fn pre_release_of_the_current_version_is_older() {
        let current = Version::new(0, 2, 0);
        assert_eq!(evaluate(&current, release("0.2.0-rc.1"), None, false), CheckOutcome::UpToDate);
    }

    #[test]
    fn skipped_version_stays_quiet_only_for_automatic_checks() {
        let current = Version::new(0, 1, 0);
        assert_eq!(evaluate(&current, release("0.2.0"), Some("0.2.0"), false), CheckOutcome::Skipped);
        assert_eq!(evaluate(&current, release("0.2.0"), Some("0.2.0"), true), CheckOutcome::Available(release("0.2.0")));
        assert_eq!(evaluate(&current, release("0.3.0"), Some("0.2.0"), false), CheckOutcome::Available(release("0.3.0")));
    }

    #[test]
    fn check_is_due_after_a_day() {
        let day = CHECK_INTERVAL.as_secs();
        assert!(is_due(0, 5));
        assert!(!is_due(1000, 1000 + day - 1));
        assert!(is_due(1000, 1000 + day));
    }

    #[test]
    fn check_is_due_when_the_clock_went_back() {
        assert!(is_due(2000, 1000));
    }

    #[test]
    fn current_version_is_a_semantic_version() {
        assert_eq!(current_version().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn fetch_reads_the_release_from_a_server() {
        let url = serve_once("200 OK", RELEASE_JSON);
        assert_eq!(fetch_latest(&url).unwrap().version, Version::new(0, 2, 0));
    }

    #[test]
    fn fetch_reports_an_error_status() {
        let url = serve_once("403 Forbidden", r#"{"message":"rate limit exceeded"}"#);
        assert!(matches!(fetch_latest(&url), Err(UpdateError::Request(_))));
    }

    #[test]
    fn fetch_reports_an_unreachable_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/latest", listener.local_addr().unwrap());
        drop(listener);
        assert!(matches!(fetch_latest(&url), Err(UpdateError::Request(_))));
    }

    #[test]
    fn background_check_delivers_its_result_and_wakes_the_window() {
        let url = serve_once("200 OK", RELEASE_JSON);
        let (woken_sender, woken) = mpsc::channel();
        let check = UpdateCheck::start(url, true, move || woken_sender.send(()).unwrap());
        woken.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(check.manual);
        assert_eq!(check.poll().unwrap().unwrap().version, Version::new(0, 2, 0));
    }
}
