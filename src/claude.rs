//! Everything that comes from Claude: the login and usage cache Claude Code keeps on disk,
//! its per-session busy/idle status, and the plan usage endpoint.

use chrono::{DateTime, Local, TimeDelta, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Network blips and server hiccups usually clear up within seconds, so a request that
/// failed quickly is retried after these pauses before the poller's longer wait. A slow
/// failure (a timeout) isn't retried, since that would hold up the poller.
const QUICK_RETRIES: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
const QUICK_FAILURE: Duration = Duration::from_secs(5);
/// How often local files are checked. Cheap: a handful of small files and timestamps.
const TICK: Duration = Duration::from_secs(2);
/// Usage only moves when Claude is used, so without local activity we check occasionally
/// (which also catches use on claude.ai or other devices).
const IDLE_INTERVAL: TimeDelta = TimeDelta::minutes(10);
/// Activity this close before a fetch may not be counted yet, so it earns one more fetch.
const SETTLE: TimeDelta = TimeDelta::seconds(15);
/// A busy session that hasn't written anything for this long has probably crashed.
const STALE_BUSY: TimeDelta = TimeDelta::minutes(10);
const MAX_BACKOFF: TimeDelta = TimeDelta::minutes(15);
/// Re-reading the credentials file is cheap, so an expired login is re-checked often
/// to pick up the new token as soon as Claude Code refreshes it.
const EXPIRED_RECHECK: TimeDelta = TimeDelta::seconds(15);

#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub label: String,
    pub percent: f32,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default)]
pub struct Account {
    pub email: Option<String>,
    pub organization: Option<String>,
    pub plan: Option<String>,
}

/// The poller publishes a fresh snapshot only when something changed, so the UI's
/// per-frame read is just a pointer copy.
pub type Shared = Arc<Mutex<Arc<State>>>;

#[derive(Clone, Debug, Default)]
pub struct State {
    pub limits: Vec<Limit>,
    pub account: Account,
    pub updated: Option<DateTime<Local>>,
    pub error: Option<String>,
    /// Some Claude Code session on this machine is busy.
    pub working: bool,
}

#[derive(Debug)]
pub enum Error {
    NotLoggedIn(String),
    Expired,
    Unauthorized,
    RateLimited(Option<TimeDelta>),
    Http(u16),
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotLoggedIn(why) => write!(f, "No Claude login found ({why}). Run `claude` and log in."),
            Error::Expired => write!(f, "Login token expired. Open Claude Code once to refresh it."),
            Error::Unauthorized => write!(f, "Login rejected. Run `claude` and /login again."),
            Error::RateLimited(_) => write!(f, "Rate limited by the API, retrying later."),
            Error::Http(code) => write!(f, "Usage API returned HTTP {code}."),
            Error::Network(why) => write!(f, "Network error: {why}"),
        }
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Claude Code honours `CLAUDE_CONFIG_DIR`, so we do too.
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)
}

fn claude_dir() -> PathBuf {
    config_dir().unwrap_or_else(|| home_dir().join(".claude"))
}

fn claude_json_path() -> PathBuf {
    config_dir().unwrap_or_else(home_dir).join(".claude.json")
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn modified(path: &Path) -> Option<DateTime<Utc>> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.into())
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<OAuth>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuth {
    access_token: String,
    expires_at: Option<i64>,
    subscription_type: Option<String>,
}

fn read_credentials() -> Result<OAuth, Error> {
    let path = claude_dir().join(".credentials.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::NotLoggedIn(format!("{}: {e}", path.display())))?;
    let file: CredentialsFile = serde_json::from_str(&text)
        .map_err(|e| Error::NotLoggedIn(format!("unreadable {}: {e}", path.display())))?;
    file.oauth
        .ok_or_else(|| Error::NotLoggedIn("no claude.ai subscription login".into()))
}

/// The login to send. Claude Code lets `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`)
/// override its saved login, so we do too.
fn login() -> Result<OAuth, Error> {
    match std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        Ok(token) if !token.trim().is_empty() => Ok(OAuth {
            access_token: token.trim().to_owned(),
            expires_at: None,
            subscription_type: None,
        }),
        _ => read_credentials(),
    }
}

/// What Claude Code keeps in `.claude.json`: the signed-in account and, whenever its usage
/// panel or `/usage` loads, a copy of the usage response.
struct ClaudeJson {
    account: Account,
    cached_usage: Option<(Vec<Limit>, DateTime<Utc>)>,
}

fn read_claude_json() -> ClaudeJson {
    let json: Value = read_json(&claude_json_path()).unwrap_or_default();
    let account = &json["oauthAccount"];
    let text = |key: &str| account[key].as_str().map(str::to_owned);
    let cache = &json["cachedUsageUtilization"];
    let cached_usage = cache["fetchedAtMs"]
        .as_i64()
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .filter(|_| cache["accountUuid"] == account["accountUuid"])
        .map(|fetched_at| (parse_usage(&cache["utilization"]), fetched_at))
        .filter(|(limits, _)| !limits.is_empty());
    ClaudeJson {
        account: Account {
            email: text("emailAddress"),
            organization: text("organizationName"),
            plan: read_credentials()
                .ok()
                .and_then(|c| c.subscription_type)
                .map(|s| format!("Claude {}", title_case(&s))),
        },
        cached_usage,
    }
}

pub fn local_account() -> Account {
    read_claude_json().account
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Session {
    session_id: String,
    cwd: String,
    status: String,
    status_updated_at: Option<i64>,
}

/// Claude Code keeps a `sessions/<pid>.json` per running session (CLI, VS Code, desktop)
/// with a `busy`/`idle` status. Returns whether any session is working, and when any
/// session last did something.
fn session_activity(now: DateTime<Utc>) -> (bool, Option<DateTime<Utc>>) {
    let dir = claude_dir();
    let Ok(entries) = std::fs::read_dir(dir.join("sessions")) else { return (false, None) };
    let (mut working, mut last_active) = (false, None);
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(session) = read_json::<Session>(&path) else { continue };
        // Transcripts are appended while a session works, so they are the freshest sign of life.
        let project: String = session
            .cwd
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let transcript = dir
            .join("projects")
            .join(project)
            .join(format!("{}.jsonl", session.session_id));
        let active_at = session
            .status_updated_at
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .max(modified(&transcript));
        last_active = last_active.max(active_at);
        working |= session.status == "busy" && active_at.is_some_and(|t| now - t < STALE_BUSY);
    }
    (working, last_active)
}

pub fn fetch() -> Result<Vec<Limit>, Error> {
    let login = login()?;
    if login
        .expires_at
        .is_some_and(|ms| ms <= Utc::now().timestamp_millis())
    {
        return Err(Error::Expired);
    }

    for pause in QUICK_RETRIES {
        let started = Instant::now();
        match request(&login.access_token) {
            Err(Error::Network(_) | Error::Http(500..=599)) if started.elapsed() < QUICK_FAILURE => {
                std::thread::sleep(pause);
            }
            result => return result,
        }
    }
    request(&login.access_token)
}

fn request(token: &str) -> Result<Vec<Limit>, Error> {
    let response = ureq::get(USAGE_URL)
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", concat!("cuota/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .call();
    let body: Value = match response {
        Ok(r) => r.into_json().map_err(|e| Error::Network(e.to_string()))?,
        Err(ureq::Error::Status(401 | 403, _)) => return Err(Error::Unauthorized),
        Err(ureq::Error::Status(429, r)) => {
            let after = r.header("retry-after").and_then(|s| s.parse().ok());
            return Err(Error::RateLimited(after.map(TimeDelta::seconds)));
        }
        Err(ureq::Error::Status(code, _)) => return Err(Error::Http(code)),
        Err(e) => return Err(Error::Network(e.to_string())),
    };
    Ok(parse_usage(&body))
}

/// Turns the usage response into display rows. Prefers the `limits` list that Claude Code's
/// own usage panel shows (it includes per-model weekly limits such as Fable),
/// falling back to the older fixed `five_hour` / `seven_day_<model>` fields.
pub fn parse_usage(body: &Value) -> Vec<Limit> {
    let mut limits = match body["limits"].as_array() {
        Some(list) => list.iter().filter_map(listed_limit).collect(),
        None => legacy_limits(body),
    };

    let extra = &body["extra_usage"];
    if extra["is_enabled"].as_bool() == Some(true)
        && let Some(l) = legacy_window("Extra".into(), extra)
    {
        limits.push(l);
    }
    limits
}

fn listed_limit(entry: &Value) -> Option<Limit> {
    let label = match entry["kind"].as_str()? {
        "session" => "Session".to_owned(),
        "weekly_all" => "Weekly".to_owned(),
        kind => {
            let scope = &entry["scope"];
            scope["model"]["display_name"]
                .as_str()
                .or_else(|| scope["surface"]["display_name"].as_str())
                .map_or_else(|| title_case(kind), str::to_owned)
        }
    };
    Some(Limit {
        label,
        percent: entry["percent"].as_f64()? as f32,
        resets_at: parse_time(&entry["resets_at"]),
    })
}

fn legacy_limits(body: &Value) -> Vec<Limit> {
    let mut limits: Vec<Limit> = [("five_hour", "Session"), ("seven_day", "Weekly")]
        .into_iter()
        .filter_map(|(key, label)| legacy_window(label.into(), &body[key]))
        .collect();

    for (key, window) in body.as_object().into_iter().flatten() {
        let Some(model) = key.strip_prefix("seven_day_") else { continue };
        // Windows for models that were never used come back as 0% with no reset time.
        if let Some(l) = legacy_window(title_case(model), window)
            && (l.percent > 0.0 || l.resets_at.is_some())
        {
            limits.push(l);
        }
    }
    limits
}

fn legacy_window(label: String, window: &Value) -> Option<Limit> {
    Some(Limit {
        label,
        percent: window["utilization"].as_f64()? as f32,
        resets_at: parse_time(&window["resets_at"]),
    })
}

fn parse_time(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?;
    DateTime::parse_from_rfc3339(text).ok().map(|t| t.with_timezone(&Utc))
}

fn title_case(s: &str) -> String {
    s.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// "2d 3h", "4h 12m", "9m", "<1m".
pub fn until(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (t - now).num_seconds().max(0);
    let (days, hours, mins) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    match (days, hours, mins) {
        (0, 0, 0) => "<1m".into(),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// Decides when the API is worth calling. Usage only changes when Claude is used, so it
/// fetches about every `active_interval` while sessions are working, once more after they
/// stop, every `IDLE_INTERVAL` otherwise, and right after a limit window resets. Usage that
/// Claude Code fetched itself (its `/usage` panel) is picked up from disk for free.
struct Poller {
    active_interval: TimeDelta,
    /// When the usage on screen was fetched, by us or by Claude Code.
    data_time: Option<DateTime<Utc>>,
    next_reset: Option<DateTime<Utc>>,
    last_activity: Option<DateTime<Utc>>,
    /// `None` until `.claude.json` has been read once.
    claude_json_modified: Option<Option<DateTime<Utc>>>,
    retry_at: Option<DateTime<Utc>>,
    backoff: TimeDelta,
}

impl Poller {
    fn new(active_interval: TimeDelta) -> Self {
        Self {
            active_interval,
            data_time: None,
            next_reset: None,
            last_activity: None,
            claude_json_modified: None,
            retry_at: None,
            backoff: active_interval,
        }
    }

    /// Returns true when `state` changed.
    fn tick(&mut self, state: &Mutex<Arc<State>>, force: bool) -> bool {
        let now = Utc::now();
        let (working, last_active) = session_activity(now);
        self.last_activity = self.last_activity.max(last_active);
        let mut changed = false;
        {
            let mut shared = state.lock().unwrap();
            if shared.working != working {
                Arc::make_mut(&mut shared).working = working;
                changed = true;
            }
        }

        let modified = modified(&claude_json_path());
        if self.claude_json_modified != Some(modified) {
            self.claude_json_modified = Some(modified);
            let local = read_claude_json();
            let mut shared = state.lock().unwrap();
            let s = Arc::make_mut(&mut shared);
            s.account = local.account;
            if let Some((limits, fetched_at)) = local.cached_usage
                && self.data_time < Some(fetched_at)
            {
                self.show(s, limits, fetched_at);
            }
            changed = true;
        }

        if force || self.api_due(now) {
            let result = fetch();
            let mut shared = state.lock().unwrap();
            let s = Arc::make_mut(&mut shared);
            match result {
                Ok(limits) => {
                    self.show(s, limits, now);
                    self.retry_at = None;
                    self.backoff = self.active_interval;
                }
                Err(e) => {
                    let wait = match &e {
                        Error::Expired => EXPIRED_RECHECK,
                        Error::RateLimited(after) => {
                            self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
                            after.unwrap_or(self.backoff)
                        }
                        _ => self.active_interval,
                    };
                    self.retry_at = Some(now + wait);
                    s.error = Some(e.to_string());
                }
            }
            changed = true;
        }
        changed |= roll_over(&mut state.lock().unwrap(), now);
        changed
    }

    fn api_due(&self, now: DateTime<Utc>) -> bool {
        if self.retry_at.is_some_and(|t| now < t) {
            return false;
        }
        let Some(data_time) = self.data_time else { return true };
        if self.next_reset.is_some_and(|reset| data_time < reset && now > reset + SETTLE) {
            return true;
        }
        let used_since = self.last_activity.is_some_and(|t| t + SETTLE > data_time);
        now - data_time >= if used_since { self.active_interval } else { IDLE_INTERVAL }
    }

    fn show(&mut self, s: &mut State, limits: Vec<Limit>, fetched_at: DateTime<Utc>) {
        self.data_time = Some(fetched_at);
        self.next_reset = limits.iter().filter_map(|l| l.resets_at).min();
        s.limits = limits;
        s.updated = Some(fetched_at.with_timezone(&Local));
        s.error = None;
    }
}

/// Zeroes windows whose reset time has passed: they have rolled over, even while the fetch
/// that would confirm it is pending or failing. Returns true when anything changed.
fn roll_over(state: &mut Arc<State>, now: DateTime<Utc>) -> bool {
    let has_reset = |l: &Limit| l.resets_at.is_some_and(|t| t <= now);
    if !state.limits.iter().any(has_reset) {
        return false;
    }
    for l in Arc::make_mut(state).limits.iter_mut().filter(|l| has_reset(l)) {
        l.percent = 0.0;
        l.resets_at = None;
    }
    true
}

/// Runs the [`Poller`] in a background thread. A message on `wake` forces an API refresh;
/// `on_update` runs whenever `state` changed so the UI can repaint.
pub fn spawn_poller(
    state: Shared,
    wake: Receiver<()>,
    active_interval: TimeDelta,
    on_update: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut poller = Poller::new(active_interval);
        let mut force = false;
        loop {
            if poller.tick(&state, force) {
                on_update();
            }
            force = match wake.recv_timeout(TICK) {
                Ok(()) => true,
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => return,
            };
        }
    });
}

/// Fake data for previewing the overlay without a login (`--demo`).
pub fn demo_state() -> State {
    let now = Utc::now();
    let limit = |label: &str, percent, mins| Limit {
        label: label.into(),
        percent,
        resets_at: Some(now + TimeDelta::minutes(mins)),
    };
    State {
        limits: vec![
            limit("Session", 15.0, 134),
            limit("Weekly", 49.0, 1_630),
            limit("Fable", 92.0, 1_630),
        ],
        account: Account {
            email: Some("you@example.com".into()),
            organization: Some("Example Organization".into()),
            plan: Some("Claude Max".into()),
        },
        updated: Some(Local::now()),
        error: None,
        working: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_limits_list() {
        let body: Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 17.0, "resets_at": "2026-10-13T19:00:00+00:00"},
                "limits": [
                    {"kind": "session", "percent": 17, "resets_at": "2026-10-13T19:00:00.113858+00:00", "scope": null},
                    {"kind": "weekly_all", "percent": 50, "resets_at": "2026-10-15T03:00:00+00:00", "scope": null},
                    {"kind": "weekly_scoped", "percent": 8, "resets_at": "2026-10-15T03:00:00+00:00",
                     "scope": {"model": {"display_name": "Fable", "id": null}, "surface": null}}
                ],
                "extra_usage": {"is_enabled": false, "utilization": null}
            }"#,
        )
        .unwrap();
        let limits = parse_usage(&body);
        let labels: Vec<_> = limits.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["Session", "Weekly", "Fable"]);
        assert_eq!(limits[2].percent, 8.0);
        assert_eq!(
            limits[0].resets_at.unwrap().to_rfc3339(),
            "2026-10-13T19:00:00.113858+00:00"
        );
    }

    #[test]
    fn falls_back_to_legacy_fields() {
        let body: Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 15.0, "resets_at": "2026-10-13T14:00:00.123456+00:00"},
                "seven_day": {"utilization": 49.0, "resets_at": "2026-10-14T10:00:00+00:00"},
                "seven_day_oauth_apps": null,
                "seven_day_opus": {"utilization": 0.0, "resets_at": null},
                "seven_day_fable": {"utilization": 8.0, "resets_at": "2026-10-14T10:00:00+00:00"},
                "extra_usage": {"is_enabled": false, "utilization": null}
            }"#,
        )
        .unwrap();
        let limits = parse_usage(&body);
        let labels: Vec<_> = limits.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["Session", "Weekly", "Fable"]);
        assert_eq!(limits[0].percent, 15.0);
    }

    #[test]
    fn shows_enabled_extra_usage_and_unused_session() {
        let body: Value = serde_json::from_str(
            r#"{
                "five_hour": {"utilization": 0.0, "resets_at": null},
                "extra_usage": {"is_enabled": true, "utilization": 12.5}
            }"#,
        )
        .unwrap();
        let limits = parse_usage(&body);
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].resets_at, None);
        assert_eq!(limits[1].label, "Extra");
    }

    #[test]
    fn formats_time_until_reset() {
        let now = Utc::now();
        let after = |secs| until(now + TimeDelta::seconds(secs), now);
        assert_eq!(after(30), "<1m");
        assert_eq!(after(9 * 60), "9m");
        assert_eq!(after(2 * 3600 + 14 * 60), "2h 14m");
        assert_eq!(after(27 * 3600), "1d 3h");
        assert_eq!(after(-5), "<1m");
    }

    #[test]
    fn title_cases_model_keys() {
        assert_eq!(title_case("fable"), "Fable");
        assert_eq!(title_case("oauth_apps"), "Oauth Apps");
    }

    fn poller_with_data(fetched_mins_ago: i64) -> (Poller, DateTime<Utc>) {
        let now = Utc::now();
        let mut poller = Poller::new(TimeDelta::seconds(60));
        poller.data_time = Some(now - TimeDelta::minutes(fetched_mins_ago));
        (poller, now)
    }

    #[test]
    fn fetches_often_only_after_activity() {
        let (mut poller, now) = poller_with_data(2);
        assert!(!poller.api_due(now), "idle: 2 minute old data is fine");
        poller.last_activity = Some(now - TimeDelta::seconds(30));
        assert!(poller.api_due(now), "used since the fetch");
        let (poller, now) = poller_with_data(11);
        assert!(poller.api_due(now), "idle interval elapsed");
    }

    #[test]
    fn fetches_after_reset_and_respects_retry() {
        let (mut poller, now) = poller_with_data(2);
        poller.next_reset = Some(now - TimeDelta::minutes(1));
        assert!(poller.api_due(now), "window reset since the fetch");
        poller.retry_at = Some(now + TimeDelta::seconds(30));
        assert!(!poller.api_due(now), "backing off");
    }

    #[test]
    fn zeroes_windows_past_their_reset() {
        let now = Utc::now();
        let mut state = Arc::new(demo_state());
        Arc::make_mut(&mut state).limits[0].resets_at = Some(now - TimeDelta::minutes(1));
        assert!(roll_over(&mut state, now));
        assert_eq!(state.limits[0].percent, 0.0);
        assert_eq!(state.limits[0].resets_at, None);
        assert_eq!(state.limits[1].percent, 49.0, "windows still running are kept");
        assert!(!roll_over(&mut state, now), "nothing left to roll over");
    }
}
