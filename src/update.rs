//! Updates from Cuota's GitHub releases: a look at the latest one a little after Cuota starts and
//! once a day after that, or whenever asked, and installing it, by itself when found by those
//! looks. The settings, kept in `%APPDATA%`, stay put.
//!
//! Releases carry only CuotaSetup.exe, so updating downloads that to `%TEMP%` and runs it with
//! `--update` and this exe's path. Setup closes this Cuota, installs the new one (moving a copy
//! run from anywhere else into its folder, and deleting it there), and starts it with
//! `--updated`, which clears away what's left.

use crate::log;
use chrono::{DateTime, Local};
use eframe::egui;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const AGENT: &str = concat!("cuota/", env!("CARGO_PKG_VERSION"));
/// The release's file to download, the same name every time.
const ASSET: &str = "CuotaSetup.exe";
/// Setup's flag for updating without being asked, followed by the exe it replaces.
const SETUP_UPDATE: &str = "--update";
/// The first look waits a little, so it stays out of the way of starting up.
const FIRST_CHECK: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// Far more than setup takes, so a wrong download can't fill the memory.
const MAX_SIZE: usize = 64 * 1024 * 1024;
/// The flag the new copy starts with.
pub const UPDATED: &str = "--updated";
/// How long the new copy waits for the old one to quit.
pub const HANDOVER: Duration = Duration::from_secs(20);

/// A release on GitHub.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Its version, like "1.2.0".
    pub version: String,
    /// Its page, which says what's new.
    pub page: String,
    /// Where its CuotaSetup.exe downloads from, if it has one.
    pub download: Option<String>,
}

impl Release {
    /// Whether it can be installed from inside Cuota. Debug builds leave cargo's exe alone,
    /// so they only link to the page.
    pub fn installable(&self) -> bool {
        self.download.is_some() && !cfg!(debug_assertions)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Unchecked,
    Checking,
    /// Nothing newer was out when it last looked.
    UpToDate(DateTime<Local>),
    Available(Release),
    /// Fetching it, with how much has come, from 0 to 1.
    Downloading(Release, f32),
    /// Setup is installing it, so this one quits.
    Restarting(Release),
    /// The look or the update didn't work. Holds the release when it was the update.
    Failed { error: String, release: Option<Release> },
}

pub struct Updater {
    status: Arc<Mutex<Status>>,
    /// Looks by itself, once `FIRST_CHECK` has passed and then each `CHECK_EVERY`.
    automatic: bool,
    started: Instant,
    /// When it last started looking.
    asked: Option<Instant>,
    /// Installs what its own look finds, without waiting for Update now.
    install_found: bool,
}

impl Updater {
    /// `automatic` is off for the demo, whose panels are for screenshots.
    pub fn new(automatic: bool) -> Self {
        Self { status: Arc::default(), automatic, started: Instant::now(), asked: None, install_found: false }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Looks for a new release when one's due and `on`, and installs what that finds. Returns
    /// how long until the next look is due.
    pub fn tick(&mut self, ctx: &egui::Context, on: bool) -> Option<Duration> {
        if !on || !self.automatic {
            return None;
        }
        if self.install_found
            && let Status::Available(release) = self.status()
        {
            // Once: if it fails, Try again is the user's.
            self.install_found = false;
            if release.installable() {
                log::info!("Installing Cuota {} by itself", release.version);
                self.install(ctx);
            }
        }
        let due = match self.asked {
            Some(asked) => asked + CHECK_EVERY,
            None => self.started + FIRST_CHECK,
        };
        let now = Instant::now();
        if now < due {
            return Some(due - now);
        }
        // Found already, or on its way in: no need to look again.
        let status = self.status();
        if !matches!(status, Status::Available(_) | Status::Downloading(..) | Status::Restarting(_)) {
            self.check(ctx);
            self.install_found = true;
        }
        self.asked = Some(now);
        Some(CHECK_EVERY)
    }

    /// Looks on GitHub for a new release now, unless it's already looking or updating.
    pub fn check(&mut self, ctx: &egui::Context) {
        if !self.idle() {
            return;
        }
        self.asked = Some(Instant::now());
        self.set(Status::Checking);
        let (status, ctx) = (self.status.clone(), ctx.clone());
        std::thread::spawn(move || {
            let found = match latest() {
                Ok(Some(release)) if newer(&release.version, VERSION) => {
                    log::info!("Cuota {} is out, and this is {VERSION}", release.version);
                    Status::Available(release)
                }
                Ok(_) => Status::UpToDate(Local::now()),
                Err(error) => {
                    log::warn!("Couldn't check for updates: {error}");
                    Status::Failed { error, release: None }
                }
            };
            *status.lock().unwrap() = found;
            ctx.request_repaint();
        });
    }

    /// Downloads the release found and starts its setup, which closes this Cuota.
    pub fn install(&mut self, ctx: &egui::Context) {
        let release = match self.status() {
            Status::Available(release) | Status::Failed { release: Some(release), .. } => release,
            _ => return,
        };
        let Some(url) = release.download.clone().filter(|_| release.installable()) else { return };
        self.set(Status::Downloading(release.clone(), 0.0));
        let (status, ctx) = (self.status.clone(), ctx.clone());
        std::thread::spawn(move || {
            let progress = |done: f32| {
                *status.lock().unwrap() = Status::Downloading(release.clone(), done);
                ctx.request_repaint();
            };
            let done = download(&url, progress).and_then(|bytes| run_setup(&bytes));
            *status.lock().unwrap() = match done {
                Ok(()) => {
                    log::info!("Setup is installing Cuota {}, which starts as this one quits", release.version);
                    Status::Restarting(release)
                }
                Err(error) => {
                    log::error!("Couldn't update to Cuota {}: {error}", release.version);
                    Status::Failed { error, release: Some(release) }
                }
            };
            ctx.request_repaint();
        });
    }

    /// Not looking or updating.
    fn idle(&self) -> bool {
        !matches!(self.status(), Status::Checking | Status::Downloading(..) | Status::Restarting(_))
    }

    fn set(&self, status: Status) {
        *self.status.lock().unwrap() = status;
    }

    /// Puts it where it would be after looking, without looking.
    #[cfg(test)]
    pub fn set_status(&self, status: Status) {
        self.set(status);
    }
}

/// Clears away what an update left: beside the exe, the copy it replaced and a download that
/// never took its place, as earlier versions updated, and the setup it ran. The new copy does
/// this once the old one has quit.
pub fn clean_up() {
    let Ok(exe) = std::env::current_exe() else { return };
    for leftover in [beside(&exe, "old"), beside(&exe, "new")] {
        if leftover.exists()
            && let Err(e) = std::fs::remove_file(&leftover)
        {
            log::warn!("Couldn't remove {}: {e}", leftover.display());
        }
    }
    // Setup started this copy as its last step, so it takes a moment to let go of its file.
    std::thread::spawn(|| {
        let setup = downloaded_setup();
        let until = Instant::now() + Duration::from_secs(60);
        while setup.exists() && std::fs::remove_file(&setup).is_err() && Instant::now() < until {
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Where the release's setup downloads to.
fn downloaded_setup() -> PathBuf {
    std::env::temp_dir().join(ASSET)
}

/// Saves `bytes`, a release's setup, and runs it to update this exe.
fn run_setup(bytes: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Couldn't tell where Cuota is: {e}"))?;
    let setup = downloaded_setup();
    std::fs::write(&setup, bytes).map_err(|e| format!("Couldn't save the update in {}: {e}", setup.display()))?;
    std::process::Command::new(&setup)
        .arg(SETUP_UPDATE)
        .arg(&exe)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Couldn't start the update: {e}"))
}

/// The latest release, or `None` when there's none yet.
fn latest() -> Result<Option<Release>, String> {
    let repository = REPOSITORY.trim_start_matches("https://github.com/");
    let url = format!("https://api.github.com/repos/{repository}/releases/latest");
    let response = ureq::get(&url)
        .set("User-Agent", AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .call();
    let body: Value = match response {
        Ok(r) => r.into_json().map_err(|e| format!("GitHub's reply didn't make sense: {e}"))?,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        // GitHub lets each address ask 60 times an hour.
        Err(ureq::Error::Status(403 | 429, _)) => return Err("GitHub asked to wait a while. Try again later.".into()),
        Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub answered with error {code}")),
        Err(e) => return Err(format!("Couldn't reach GitHub: {e}")),
    };
    Ok(parse_release(&body))
}

fn parse_release(body: &Value) -> Option<Release> {
    let tag = body["tag_name"].as_str()?;
    number(tag)?;
    let page = body["html_url"].as_str().map_or_else(|| format!("{REPOSITORY}/releases/latest"), str::to_owned);
    let assets = body["assets"].as_array().into_iter().flatten();
    let exe = assets.filter(|asset| asset["name"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(ASSET)));
    let download = exe.filter_map(|asset| asset["browser_download_url"].as_str()).next().map(str::to_owned);
    Some(Release { version: tag.trim_start_matches(['v', 'V']).to_owned(), page, download })
}

/// A version like "v1.2" or "1.2.0" as its three numbers, leaving out anything after a - or +.
fn number(version: &str) -> Option<[u64; 3]> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some([major, minor, patch])
}

/// Whether `version` comes after `current`.
fn newer(version: &str, current: &str) -> bool {
    match (number(version), number(current)) {
        (Some(version), Some(current)) => version > current,
        _ => false,
    }
}

/// The file at `url`, telling `progress` how much of it has come.
fn download(url: &str, progress: impl Fn(f32)) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .set("User-Agent", AGENT)
        .timeout(Duration::from_secs(300))
        .call()
        .map_err(|e| format!("Couldn't download it: {e}"))?;
    let size = response.header("content-length").and_then(|size| size.parse::<usize>().ok());
    if size.is_some_and(|size| size > MAX_SIZE) {
        return Err("The download is far bigger than Cuota".into());
    }
    let mut reader = response.into_reader().take(MAX_SIZE as u64 + 1);
    let mut bytes = Vec::with_capacity(size.unwrap_or(0));
    let mut chunk = vec![0; 64 * 1024];
    let mut shown = 0;
    loop {
        let read = reader.read(&mut chunk).map_err(|e| format!("The download stopped: {e}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        // Each whole percent, not each chunk.
        if let Some(size) = size {
            let percent = bytes.len() * 100 / size.max(1);
            if percent > shown {
                shown = percent;
                progress(percent.min(100) as f32 / 100.0);
            }
        }
    }
    if bytes.len() > MAX_SIZE {
        return Err("The download is far bigger than Cuota".into());
    }
    if size.is_some_and(|size| bytes.len() != size) {
        return Err("The download was cut short".into());
    }
    // Every Windows program starts so.
    if !bytes.starts_with(b"MZ") {
        return Err("The download isn't a program".into());
    }
    Ok(bytes)
}

/// `exe` with `.extension` added, like Cuota.exe.old.
fn beside(exe: &Path, extension: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    name.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_versions_with_or_without_a_v_and_missing_parts() {
        assert_eq!(number("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(number("1.10"), Some([1, 10, 0]));
        assert_eq!(number("2"), Some([2, 0, 0]));
        assert_eq!(number("1.2.0-beta.1"), Some([1, 2, 0]));
        assert_eq!(number("latest"), None);
    }

    #[test]
    fn compares_versions_by_number_not_by_text() {
        assert!(newer("1.10.0", "1.9.0"));
        assert!(newer("v2.0.0", "1.1.0"));
        assert!(!newer("1.1.0", "1.1.0"));
        assert!(!newer("1.0.9", "1.1.0"));
        assert!(!newer("nightly", "1.1.0"));
    }

    #[test]
    fn finds_the_setup_among_a_releases_files() {
        let body = json!({
            "tag_name": "v1.2.0",
            "html_url": "https://github.com/arshit09/Cuota/releases/tag/v1.2.0",
            "assets": [
                { "name": "Cuota.exe", "browser_download_url": "https://example.com/Cuota.exe" },
                { "name": "CuotaSetup.exe", "browser_download_url": "https://example.com/CuotaSetup.exe" },
            ],
        });
        let release = parse_release(&body).unwrap();
        assert_eq!(release.version, "1.2.0");
        assert_eq!(release.page, "https://github.com/arshit09/Cuota/releases/tag/v1.2.0");
        assert_eq!(release.download.as_deref(), Some("https://example.com/CuotaSetup.exe"));

        let no_exe = parse_release(&json!({ "tag_name": "1.3.0", "assets": [] })).unwrap();
        assert_eq!(no_exe.download, None);
        assert_eq!(no_exe.page, format!("{REPOSITORY}/releases/latest"));
        assert_eq!(parse_release(&json!({ "tag_name": "nightly" })), None);
    }

    /// Asks GitHub for real, so it's left out of the usual run: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_the_latest_release_from_github() {
        let release = latest().unwrap().expect("a release on GitHub");
        let url = release.download.expect("a CuotaSetup.exe in the latest release");
        let shown = std::cell::Cell::new(0.0);
        let bytes = download(&url, |done| shown.set(done)).unwrap();
        assert!(bytes.len() > 1024 * 1024);
        assert_eq!(shown.get(), 1.0);
    }
}
