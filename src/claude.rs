//! Everything that comes from Claude: the login and usage cache Claude Code keeps on disk,
//! its per-session busy/idle/waiting status and the steps its transcripts show, and the plan
//! usage endpoint.

use crate::log;
use chrono::{DateTime, Local, TimeDelta, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
/// After a rate limit the API is left alone this long, doubling while it keeps refusing.
/// Its `Retry-After` is often 0, and taking that at its word retried every tick, which kept
/// the limit from ever lifting.
const RATE_LIMIT_WAIT: TimeDelta = TimeDelta::minutes(5);
const MAX_BACKOFF: TimeDelta = TimeDelta::minutes(15);
/// Re-reading the credentials file is cheap, so an expired login is re-checked often
/// to pick up the new token as soon as Claude Code refreshes it.
const EXPIRED_RECHECK: TimeDelta = TimeDelta::seconds(15);

#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub label: String,
    pub percent: f32,
    pub resets_at: Option<DateTime<Utc>>,
    /// How long the window runs (5 hours, 7 days), to tell how much of it has passed.
    pub window: Option<TimeDelta>,
    /// For a model's own weekly limit: how much of the overall weekly limit it has used, and
    /// how much of that limit it may use, in percent.
    pub share: Option<(f32, f32)>,
}

/// Where a limit is headed if usage keeps up the pace it has had since the window opened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pace {
    Reached,
    /// Ends the window at about this percentage.
    Finish(f32),
    /// Reaches 100% at this time, before the window resets.
    RunsOut(DateTime<Utc>),
}

/// Before this much of a window has passed, the pace so far says little about where it ends.
const PACE_AFTER: f32 = 0.1;

impl Limit {
    /// How much of the window has passed, from 0 to 1.
    pub fn elapsed(&self, now: DateTime<Utc>) -> Option<f32> {
        let (resets_at, window) = (self.resets_at?, self.window?);
        let left = (resets_at - now).num_seconds() as f32 / window.num_seconds() as f32;
        Some((1.0 - left).clamp(0.0, 1.0))
    }

    pub fn pace(&self, now: DateTime<Utc>) -> Option<Pace> {
        if self.percent >= 100.0 {
            return Some(Pace::Reached);
        }
        let elapsed = self.elapsed(now).filter(|&e| e >= PACE_AFTER)?;
        let projected = self.percent / elapsed;
        if projected <= 100.0 {
            return Some(Pace::Finish(projected));
        }
        let window = self.window?;
        let to_full = window.num_seconds() as f32 * elapsed * 100.0 / self.percent;
        Some(Pace::RunsOut(self.resets_at? - window + TimeDelta::seconds(to_full as i64)))
    }
}

/// What the week's usage went to (Claude Code, chats, Cowork...), in percent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Breakdown {
    pub since: Option<DateTime<Utc>>,
    /// Key, name and percent, in the order the API lists them.
    pub parts: Vec<(String, String, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Money {
    pub amount: f64,
    pub currency: String,
}

impl std::fmt::Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.currency.as_str() {
            "USD" => write!(f, "${:.2}", self.amount),
            "EUR" => write!(f, "\u{20AC}{:.2}", self.amount),
            "GBP" => write!(f, "\u{A3}{:.2}", self.amount),
            code => write!(f, "{:.2} {code}", self.amount),
        }
    }
}

/// Extra usage: paid credits that keep Claude going past the plan's limits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extra {
    pub enabled: bool,
    pub used: Option<Money>,
    pub limit: Option<Money>,
    /// The monthly spend limit has been reached.
    pub capped: bool,
}

/// Everything the usage endpoint reports.
#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub limits: Vec<Limit>,
    pub breakdown: Option<Breakdown>,
    pub extra: Option<Extra>,
}

#[derive(Clone, Debug, Default)]
pub struct Account {
    pub email: Option<String>,
    pub organization: Option<String>,
    pub plan: Option<String>,
    /// When the subscription started.
    pub since: Option<DateTime<Utc>>,
    /// Who bills it, e.g. "Stripe" or "Apple".
    pub billing: Option<String>,
    /// The role in the organization, e.g. "Admin".
    pub role: Option<String>,
}

/// A Claude Code session running on this PC.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub name: String,
    /// What the conversation is called: Claude Code's AI title, or what it was renamed to.
    pub title: Option<String>,
    /// The folder it works in, unless the Claude app made that folder for it.
    pub project: Option<String>,
    /// Where it runs: "VS Code", "Terminal", "Claude app"...
    pub app: String,
    pub busy: bool,
    /// Waiting on you: a permission prompt, a question or another dialog.
    pub waiting: bool,
    pub started: Option<DateTime<Utc>>,
    /// How many steps its transcript has shown since Cuota began watching it, and the latest.
    pub steps: u32,
    pub step: Option<Step>,
}

/// A kind of step Claude Code takes, as its transcript tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Thought it over.
    Think,
    /// Read or searched files.
    Look,
    /// Wrote or changed a file.
    Edit,
    /// Ran a command.
    Run,
    /// Went online, or out to another app through MCP.
    Web,
    /// Handed work to another agent.
    Delegate,
    /// Planned, or worked through its to-do list.
    Plan,
    /// A tool failed, or was turned down.
    Fail,
    /// Summed up the conversation to make room in its context.
    Compact,
}

impl Step {
    /// The step a call to the tool `name` is. Asking you a question isn't one: the session
    /// waits on you then.
    fn of_tool(name: &str) -> Option<Self> {
        Some(match name {
            "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" | "ToolSearch" => Self::Look,
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Self::Edit,
            "Bash" | "BashOutput" | "KillBash" | "KillShell" | "PowerShell" | "Monitor" => Self::Run,
            "WebFetch" | "WebSearch" | "web_fetch" | "web_search" => Self::Web,
            "Task" | "Agent" | "Workflow" | "SendMessage" => Self::Delegate,
            "TodoWrite" | "TaskCreate" | "TaskUpdate" | "EnterPlanMode" | "ExitPlanMode" | "Skill" => Self::Plan,
            name if name.starts_with("mcp__") => Self::Web,
            _ => return None,
        })
    }

    /// The step a transcript line records, if any. Only assistant lines are parsed; the rest
    /// are told apart by text that can't appear unescaped inside a message.
    fn of_line(line: &str) -> Option<Self> {
        if line.contains(r#""is_error":true"#) {
            return Some(Self::Fail);
        }
        if line.contains(r#""subtype":"compact_boundary""#) {
            return Some(Self::Compact);
        }
        if !line.contains(r#""type":"assistant""#) {
            return None;
        }
        let entry: StepLine = serde_json::from_str(line).ok()?;
        if entry.kind != "assistant" {
            return None;
        }
        entry.message.content.iter().find_map(|block| match block.kind.as_str() {
            "thinking" | "redacted_thinking" => Some(Self::Think),
            "tool_use" | "server_tool_use" => block.name.as_deref().and_then(Self::of_tool),
            _ => None,
        })
    }
}

#[derive(Deserialize)]
struct StepLine {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: StepMessage,
}

#[derive(Deserialize, Default)]
struct StepMessage {
    #[serde(default)]
    content: Vec<StepBlock>,
}

#[derive(Deserialize)]
struct StepBlock {
    #[serde(rename = "type")]
    kind: String,
    name: Option<String>,
}

impl Session {
    /// The conversation's title, or the session's name before it has one.
    pub fn display_name(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// The poller publishes a fresh snapshot only when something changed, so the UI's
/// per-frame read is just a pointer copy.
pub type Shared = Arc<Mutex<Arc<State>>>;

#[derive(Clone, Debug, Default)]
pub struct State {
    pub limits: Vec<Limit>,
    pub breakdown: Option<Breakdown>,
    pub extra: Option<Extra>,
    pub account: Account,
    pub updated: Option<DateTime<Local>>,
    pub error: Option<String>,
    /// Some Claude Code session on this machine is busy.
    pub working: bool,
    /// Busy ones first.
    pub sessions: Vec<Session>,
}

impl State {
    /// When the current weekly window opened, or six days before today before usage has loaded.
    pub fn week_start(&self) -> DateTime<Utc> {
        let weekly = self.limits.iter().find(|l| l.window == Some(TimeDelta::days(7)) && l.share.is_none());
        let from_weekly = weekly.and_then(|l| Some(l.resets_at? - l.window?));
        self.breakdown.as_ref().and_then(|b| b.since).or(from_weekly).unwrap_or_else(|| {
            // From midnight, so the start holds still through the day.
            let today = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap_or_default();
            let today = today.and_local_timezone(Local).earliest().map_or_else(Utc::now, |t| t.to_utc());
            today - TimeDelta::days(6)
        })
    }
}

#[derive(Debug)]
pub enum Error {
    NotLoggedIn(String),
    Expired,
    Unauthorized,
    RateLimited(Option<TimeDelta>),
    /// The status, and the start of what the server said, for the log.
    Http(u16, String),
    Network(String),
}

impl Error {
    /// What the log says: the message, with the server's reply or how long it asked us to wait.
    fn logged(&self) -> String {
        match self {
            Error::RateLimited(Some(after)) => {
                format!("Rate limited by the usage API, which asked to wait {}s", after.num_seconds())
            }
            Error::RateLimited(None) => "Rate limited by the usage API".into(),
            Error::Http(code, reply) if !reply.is_empty() => format!("Usage API returned HTTP {code}: {reply}"),
            e => e.to_string().trim_end_matches('.').to_owned(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotLoggedIn(why) => write!(f, "No Claude login found ({why}). Run `claude` and log in."),
            Error::Expired => write!(f, "Login token expired. Open Claude Code once to refresh it."),
            Error::Unauthorized => write!(f, "Login rejected. Run `claude` and /login again."),
            Error::RateLimited(_) => write!(f, "Rate limited by the API, retrying later."),
            Error::Http(code, _) => write!(f, "Usage API returned HTTP {code}."),
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

pub fn claude_dir() -> PathBuf {
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

/// Hands each whole line of a file from `from` on to `each`, and returns where the last one
/// ended. A line still being written is left for next time.
pub fn read_lines(path: &Path, from: u64, mut each: impl FnMut(&[u8])) -> u64 {
    let Ok(mut file) = std::fs::File::open(path) else { return from };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return from;
    }
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let (mut line, mut to) = (Vec::new(), from);
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(n) if n > 0 && line.ends_with(b"\n") => {
                to += n as u64;
                each(&line);
            }
            _ => return to,
        }
    }
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
    cached_usage: Option<(Usage, DateTime<Utc>)>,
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
        .filter(|(usage, _)| !usage.limits.is_empty());
    // Max comes as 5x or 20x, which ends the rate limit tier: "default_claude_max_20x".
    let multiplier = account["organizationRateLimitTier"]
        .as_str()
        .and_then(|tier| tier.rsplit('_').next())
        .filter(|m| m.ends_with('x') && m.starts_with(|c: char| c.is_ascii_digit()));
    let plan = read_credentials().ok().and_then(|c| c.subscription_type).map(|s| match multiplier {
        Some(m) => format!("Claude {} {m}", title_case(&s)),
        None => format!("Claude {}", title_case(&s)),
    });
    ClaudeJson {
        account: Account {
            email: text("emailAddress"),
            organization: text("organizationName"),
            plan,
            since: parse_time(&account["subscriptionCreatedAt"]),
            billing: account["billingType"]
                .as_str()
                .map(|b| title_case(b.trim_end_matches("_subscription")))
                .filter(|b| !b.is_empty()),
            role: account["organizationRole"].as_str().map(title_case),
        },
        cached_usage,
    }
}

pub fn local_account() -> Account {
    read_claude_json().account
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SessionFile {
    pid: Option<u32>,
    session_id: String,
    name: String,
    cwd: String,
    entrypoint: String,
    status: String,
    started_at: Option<i64>,
    status_updated_at: Option<i64>,
}

impl SessionFile {
    fn session(&self, busy: bool, read: &Transcript) -> Session {
        let folder = Path::new(&self.cwd).file_name().map(|name| name.to_string_lossy().into_owned());
        // The Claude app runs its sessions in scratch folders under %APPDATA%.
        let app_data = std::env::var("APPDATA").unwrap_or_default().to_lowercase();
        let made_by_app = !app_data.is_empty() && self.cwd.to_lowercase().starts_with(&app_data);
        let project = folder.filter(|_| !made_by_app);
        let app = match self.entrypoint.as_str() {
            "cli" => "Terminal".to_owned(),
            "claude-vscode" => "VS Code".to_owned(),
            "claude-desktop" => "Claude app".to_owned(),
            other => title_case(&other.trim_start_matches("claude-").replace('-', "_")),
        };
        Session {
            name: Some(self.name.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| project.clone())
                .unwrap_or_else(|| "Session".into()),
            title: read.title(),
            project,
            app,
            busy,
            waiting: self.status == "waiting",
            started: self.started_at.and_then(DateTime::<Utc>::from_timestamp_millis),
            steps: read.steps,
            step: read.step,
        }
    }
}

/// What a transcript says its conversation is called, and the steps it records. Claude Code
/// appends an `ai-title` line once it has named the conversation and a `custom-title` one
/// when it's renamed, and repeats them as the transcript grows, so the latest of each counts.
#[derive(Default)]
struct Transcript {
    /// How far the transcript has been read: the end of its last whole line.
    read: u64,
    ai: Option<String>,
    custom: Option<String>,
    /// Steps added since it was first read, and the latest.
    steps: u32,
    step: Option<Step>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TitleLine {
    #[serde(rename = "type")]
    kind: String,
    ai_title: Option<String>,
    custom_title: Option<String>,
}

impl Transcript {
    /// A rename wins over the AI's title.
    fn title(&self) -> Option<String> {
        self.custom.clone().or_else(|| self.ai.clone())
    }

    /// Reads what was added to the transcript since last time. Transcripts run to megabytes,
    /// so each is read whole only once.
    fn update(&mut self, path: &Path) {
        let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else { return };
        if len < self.read {
            // Rewritten from scratch.
            *self = Self::default();
        }
        if len > self.read {
            let from = self.read;
            // What was done before Cuota looked isn't news.
            let count_steps = from > 0;
            self.read = read_lines(path, from, |line| self.take(line, count_steps));
        }
    }

    fn take(&mut self, line: &[u8], count_steps: bool) {
        let Ok(text) = std::str::from_utf8(line) else { return };
        if count_steps && let Some(step) = Step::of_line(text) {
            self.steps = self.steps.wrapping_add(1);
            self.step = Some(step);
        }
        // Most lines aren't titles; skip them without parsing. In a message's text the quote
        // after `-title` would be escaped.
        if !text.contains(r#"-title""#) {
            return;
        }
        let Ok(entry) = serde_json::from_str::<TitleLine>(text) else { return };
        let title = |t: Option<String>| t.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        match entry.kind.as_str() {
            "ai-title" => self.ai = title(entry.ai_title),
            "custom-title" => self.custom = title(entry.custom_title),
            _ => {}
        }
    }
}

/// What Claude Code is doing on this machine.
struct Activity {
    working: bool,
    /// When any session last did something.
    last_active: Option<DateTime<Utc>>,
    sessions: Vec<Session>,
}

/// Claude Code keeps a `sessions/<pid>.json` per running session (CLI, VS Code, desktop)
/// with a `busy`/`idle`/`waiting` status. `transcripts` holds what was read of each running
/// session's transcript, by its path.
fn session_activity(now: DateTime<Utc>, transcripts: &mut HashMap<PathBuf, Transcript>) -> Activity {
    let dir = claude_dir();
    let mut activity = Activity { working: false, last_active: None, sessions: Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir.join("sessions")) else { return activity };
    let mut running = HashMap::new();
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(session) = read_json::<SessionFile>(&path) else { continue };
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
        activity.last_active = activity.last_active.max(active_at);
        let busy = session.status == "busy" && active_at.is_some_and(|t| now - t < STALE_BUSY);
        activity.working |= busy;
        // A session that crashed leaves its file behind.
        if session.pid.is_none_or(crate::win::process_alive) {
            let mut read = transcripts.remove(&transcript).unwrap_or_default();
            read.update(&transcript);
            activity.sessions.push(session.session(busy, &read));
            running.insert(transcript, read);
        }
    }
    // Sessions that ended are forgotten.
    *transcripts = running;
    activity
        .sessions
        .sort_by(|a, b| b.busy.cmp(&a.busy).then_with(|| a.display_name().cmp(b.display_name())));
    activity
}

pub fn fetch() -> Result<Usage, Error> {
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
            Err(Error::Network(_) | Error::Http(500..=599, _)) if started.elapsed() < QUICK_FAILURE => {
                std::thread::sleep(pause);
            }
            result => return result,
        }
    }
    request(&login.access_token)
}

fn request(token: &str) -> Result<Usage, Error> {
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
        Err(ureq::Error::Status(code, r)) => return Err(Error::Http(code, reply(r))),
        Err(e) => return Err(Error::Network(e.to_string())),
    };
    let usage = parse_usage(&body);
    // Most likely the API changed. Once is enough to tell.
    static TOLD: AtomicBool = AtomicBool::new(false);
    if usage.limits.is_empty() && !TOLD.swap(true, Ordering::Relaxed) {
        let fields = body.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>().join(", "));
        log::warn!("The usage API's reply had no limits Cuota knows. Its fields: {}", fields.unwrap_or_default());
    }
    Ok(usage)
}

/// The start of an error reply, on one line, for the log.
fn reply(response: ureq::Response) -> String {
    let text = response.into_string().unwrap_or_default();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(300) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

/// Turns the usage response into display rows. Prefers the `limits` list that Claude Code's
/// own usage panel shows (it includes per-model weekly limits such as Fable),
/// falling back to the older fixed `five_hour` / `seven_day_<model>` fields.
pub fn parse_usage(body: &Value) -> Usage {
    let mut limits = match body["limits"].as_array() {
        Some(list) => list.iter().enumerate().filter_map(|(i, entry)| listed_limit(entry, share(body, i))).collect(),
        None => legacy_limits(body),
    };

    let extra = &body["extra_usage"];
    if extra["is_enabled"].as_bool() == Some(true)
        && let Some(l) = legacy_window("Extra".into(), extra, None)
    {
        limits.push(l);
    }
    Usage { limits, breakdown: breakdown(&body["seven_day_breakdown"]), extra: extra_usage(body) }
}

/// The share of the overall weekly limit that the `index`th listed limit has used and may use.
fn share(body: &Value, index: usize) -> Option<(f32, f32)> {
    let share = body["weekly_scoped_shares"]
        .as_array()?
        .iter()
        .find(|s| s["limit_index"].as_u64() == Some(index as u64))?;
    Some((
        share["used_percent_of_weekly"].as_f64()? as f32,
        share["allowance_percent_of_weekly"].as_f64()? as f32,
    ))
}

fn breakdown(value: &Value) -> Option<Breakdown> {
    let parts = value["rows"]
        .as_array()?
        .iter()
        .filter_map(|row| {
            let name = row["display_name"].as_str()?;
            let key = row["key"].as_str().unwrap_or(name);
            Some((key.to_owned(), name.to_owned(), row["percent"].as_f64()? as f32))
        })
        .collect();
    Some(Breakdown { since: parse_time(&value["window_started_at"]), parts })
}

fn extra_usage(body: &Value) -> Option<Extra> {
    let (spend, extra) = (&body["spend"], &body["extra_usage"]);
    if !spend.is_object() && !extra.is_object() {
        return None;
    }
    Some(Extra {
        enabled: spend["enabled"].as_bool().or(extra["is_enabled"].as_bool()).unwrap_or(false),
        used: money(&spend["used"]),
        limit: money(&spend["limit"]).or_else(|| money(&spend["cap"])),
        capped: extra["spend_limit_reached"].as_bool().unwrap_or(false),
    })
}

/// `{"amount_minor": 1230, "currency": "USD", "exponent": 2}` is $12.30.
fn money(value: &Value) -> Option<Money> {
    let minor = value["amount_minor"].as_f64()?;
    let exponent = value["exponent"].as_i64().unwrap_or(2) as i32;
    Some(Money {
        amount: minor / 10f64.powi(exponent),
        currency: value["currency"].as_str().unwrap_or("USD").to_owned(),
    })
}

/// How long a limit's window runs, from its group ("session", "weekly") or kind.
fn window_length(group: &str) -> Option<TimeDelta> {
    match group {
        "session" | "five_hour" => Some(TimeDelta::hours(5)),
        g if g.starts_with("weekly") || g.starts_with("seven_day") => Some(TimeDelta::days(7)),
        _ => None,
    }
}

fn listed_limit(entry: &Value, share: Option<(f32, f32)>) -> Option<Limit> {
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
    let group = entry["group"].as_str().or(entry["kind"].as_str()).unwrap_or_default();
    Some(Limit {
        label,
        percent: entry["percent"].as_f64()? as f32,
        resets_at: parse_time(&entry["resets_at"]),
        window: window_length(group),
        share,
    })
}

fn legacy_limits(body: &Value) -> Vec<Limit> {
    let mut limits: Vec<Limit> = [("five_hour", "Session"), ("seven_day", "Weekly")]
        .into_iter()
        .filter_map(|(key, label)| legacy_window(label.into(), &body[key], window_length(key)))
        .collect();

    for (key, window) in body.as_object().into_iter().flatten() {
        let Some(model) = key.strip_prefix("seven_day_") else { continue };
        // Windows for models that were never used come back as 0% with no reset time.
        if let Some(l) = legacy_window(title_case(model), window, window_length(key))
            && (l.percent > 0.0 || l.resets_at.is_some())
        {
            limits.push(l);
        }
    }
    limits
}

fn legacy_window(label: String, window: &Value, length: Option<TimeDelta>) -> Option<Limit> {
    Some(Limit {
        label,
        percent: window["utilization"].as_f64()? as f32,
        resets_at: parse_time(&window["resets_at"]),
        window: length,
        share: None,
    })
}

fn parse_time(value: &Value) -> Option<DateTime<Utc>> {
    let text = value.as_str()?;
    DateTime::parse_from_rfc3339(text).ok().map(|t| t.with_timezone(&Utc))
}

pub fn title_case(s: &str) -> String {
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
    /// The wait after the last rate limit, or zero when the last fetch wasn't limited.
    backoff: TimeDelta,
    /// What's been read of each running session's transcript.
    transcripts: HashMap<PathBuf, Transcript>,
    /// Whether the API has answered yet, so the first answer is logged.
    fetched: bool,
    failing: Option<Failing>,
}

/// Fetches failing in a row, as the log tells of them.
struct Failing {
    since: DateTime<Utc>,
    tries: u32,
    /// The last failure logged, and when.
    logged: String,
    logged_at: DateTime<Utc>,
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
            backoff: TimeDelta::zero(),
            transcripts: HashMap::new(),
            fetched: false,
            failing: None,
        }
    }

    /// Returns true when `state` changed.
    fn tick(&mut self, state: &Mutex<Arc<State>>, force: bool) -> bool {
        let now = Utc::now();
        let activity = session_activity(now, &mut self.transcripts);
        self.last_activity = self.last_activity.max(activity.last_active);
        let mut changed = false;
        {
            let mut shared = state.lock().unwrap();
            if shared.working != activity.working || shared.sessions != activity.sessions {
                let s = Arc::make_mut(&mut shared);
                s.working = activity.working;
                s.sessions = activity.sessions;
                changed = true;
            }
        }

        let modified = modified(&claude_json_path());
        if self.claude_json_modified != Some(modified) {
            self.claude_json_modified = Some(modified);
            let local = read_claude_json();
            let mut shared = state.lock().unwrap();
            let s = Arc::make_mut(&mut shared);
            if s.account.plan != local.account.plan {
                log::info!("Claude plan: {}", local.account.plan.as_deref().unwrap_or("none found"));
            }
            s.account = local.account;
            if let Some((usage, fetched_at)) = local.cached_usage
                && self.data_time < Some(fetched_at)
            {
                self.show(s, usage, fetched_at);
            }
            changed = true;
        }

        // Refresh skips the usual wait, but not a rate limit's: asking again early only
        // makes it last longer.
        if (force && !self.rate_limited(now)) || self.api_due(now) {
            let result = fetch();
            let mut shared = state.lock().unwrap();
            let s = Arc::make_mut(&mut shared);
            match result {
                Ok(usage) => {
                    self.log_success(&usage, now);
                    self.show(s, usage, now);
                    self.retry_at = None;
                    self.backoff = TimeDelta::zero();
                }
                Err(e) => {
                    let wait = match &e {
                        Error::Expired => EXPIRED_RECHECK,
                        Error::RateLimited(after) => self.rate_limit_wait(*after),
                        _ => self.active_interval,
                    };
                    self.retry_at = Some(now + wait);
                    self.log_failure(&e, now, wait);
                    s.error = Some(e.to_string());
                }
            }
            changed = true;
        }
        changed |= roll_over(&mut state.lock().unwrap(), now);
        changed
    }

    /// Logs the first usage fetched, and usage fetched again after failing.
    fn log_success(&mut self, usage: &Usage, now: DateTime<Utc>) {
        let limits: Vec<String> = usage.limits.iter().map(|l| format!("{} {:.0}%", l.label, l.percent)).collect();
        if let Some(failing) = self.failing.take() {
            let since = failing.since.with_timezone(&Local).format("%H:%M");
            log::info!(
                "Usage fetched again, after {} failed tries since {since} ({} ago): {}",
                failing.tries,
                until(now, failing.since),
                limits.join(", ")
            );
        } else if !self.fetched {
            log::info!("Usage fetched: {}", limits.join(", "));
        }
        self.fetched = true;
    }

    /// Logs a failed fetch and when the next try is. The same failure again is logged only
    /// once an hour, so a night offline doesn't fill the log, except a rate limit, since each
    /// one waits longer.
    fn log_failure(&mut self, e: &Error, now: DateTime<Utc>, wait: TimeDelta) {
        let message = e.logged();
        let failing = self.failing.get_or_insert_with(|| Failing {
            since: now,
            tries: 0,
            logged: String::new(),
            logged_at: now,
        });
        failing.tries += 1;
        let again = failing.logged == message;
        if again && now - failing.logged_at < TimeDelta::hours(1) && !matches!(e, Error::RateLimited(_)) {
            return;
        }
        let tries = match failing.tries {
            1 => String::new(),
            tries => format!(" ({tries} failed tries in a row)"),
        };
        log::warn!("{message}{tries}. Trying again in {}.", until(now + wait, now));
        failing.logged = message;
        failing.logged_at = now;
    }

    /// How long to leave the API alone after it rate limited us, given its `Retry-After`.
    fn rate_limit_wait(&mut self, after: Option<TimeDelta>) -> TimeDelta {
        self.backoff = (self.backoff * 2).clamp(RATE_LIMIT_WAIT, MAX_BACKOFF);
        after.map_or(self.backoff, |after| after.max(self.backoff))
    }

    fn rate_limited(&self, now: DateTime<Utc>) -> bool {
        self.backoff > TimeDelta::zero() && self.retry_at.is_some_and(|t| now < t)
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

    fn show(&mut self, s: &mut State, usage: Usage, fetched_at: DateTime<Utc>) {
        self.data_time = Some(fetched_at);
        self.next_reset = usage.limits.iter().filter_map(|l| l.resets_at).min();
        s.limits = usage.limits;
        s.breakdown = usage.breakdown;
        s.extra = usage.extra;
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
    let limit = |label: &str, percent, mins, window| Limit {
        label: label.into(),
        percent,
        resets_at: Some(now + TimeDelta::minutes(mins)),
        window: Some(window),
        share: None,
    };
    let (five_hours, week) = (TimeDelta::hours(5), TimeDelta::days(7));
    let mut fable = limit("Fable", 92.0, 1_630, week);
    fable.share = Some((46.0, 50.0));
    let part = |key: &str, name: &str, percent| (key.to_owned(), name.to_owned(), percent);
    let session = |name: &str, title: &str, project: &str, app: &str, busy, mins| Session {
        name: name.into(),
        title: Some(title.into()),
        project: Some(project.into()),
        app: app.into(),
        busy,
        waiting: false,
        started: Some(now - TimeDelta::minutes(mins)),
        steps: 0,
        step: None,
    };
    State {
        limits: vec![limit("Session", 15.0, 134, five_hours), limit("Weekly", 49.0, 1_630, week), fable],
        breakdown: Some(Breakdown {
            since: Some(now + TimeDelta::minutes(1_630) - week),
            parts: vec![part("claude_code", "Claude Code", 81.0), part("chat", "Chats", 15.0), part("cowork", "Cowork", 4.0)],
        }),
        extra: Some(Extra::default()),
        account: Account {
            email: Some("you@example.com".into()),
            organization: Some("Example Organization".into()),
            plan: Some("Claude Max 5x".into()),
            since: DateTime::parse_from_rfc3339("2025-08-25T15:00:00Z").ok().map(|t| t.to_utc()),
            billing: Some("Stripe".into()),
            role: Some("Admin".into()),
        },
        updated: Some(Local::now()),
        error: None,
        working: true,
        sessions: vec![
            session("cuota-94", "Show each session's title in the details", "Cuota", "VS Code", true, 95),
            session("website-12", "Fix the pricing page layout", "Website", "Terminal", false, 300),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_failure_is_logged_once_an_hour_but_every_rate_limit_is() {
        let mut poller = Poller::new(TimeDelta::minutes(1));
        let now = Utc::now();
        let at = |mins| now + TimeDelta::minutes(mins);
        let logged_at = |poller: &Poller| poller.failing.as_ref().unwrap().logged_at;
        let offline = Error::Network("offline".into());

        poller.log_failure(&offline, now, TimeDelta::minutes(1));
        poller.log_failure(&offline, at(30), TimeDelta::minutes(1));
        assert_eq!(logged_at(&poller), now);
        poller.log_failure(&offline, at(61), TimeDelta::minutes(1));
        assert_eq!(logged_at(&poller), at(61));

        let limited = Error::RateLimited(Some(TimeDelta::minutes(2)));
        poller.log_failure(&limited, at(62), TimeDelta::minutes(2));
        poller.log_failure(&limited, at(64), TimeDelta::minutes(4));
        assert_eq!(logged_at(&poller), at(64));
        assert_eq!(poller.failing.as_ref().unwrap().tries, 5);

        poller.log_success(&Usage { limits: Vec::new(), breakdown: None, extra: None }, at(68));
        assert!(poller.failing.is_none());
    }

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
        let limits = parse_usage(&body).limits;
        let labels: Vec<_> = limits.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["Session", "Weekly", "Fable"]);
        assert_eq!(limits[2].percent, 8.0);
        assert_eq!(
            limits[0].resets_at.unwrap().to_rfc3339(),
            "2026-10-13T19:00:00.113858+00:00"
        );
        assert_eq!(limits[0].window, Some(TimeDelta::hours(5)));
        assert_eq!(limits[2].window, Some(TimeDelta::days(7)));
    }

    #[test]
    fn parses_details() {
        let body: Value = serde_json::from_str(
            r#"{
                "limits": [
                    {"kind": "session", "group": "session", "percent": 23, "resets_at": "2026-10-13T19:00:00+00:00"},
                    {"kind": "weekly_all", "group": "weekly", "percent": 52, "resets_at": "2026-10-15T03:00:00+00:00"},
                    {"kind": "weekly_scoped", "group": "weekly", "percent": 11, "resets_at": "2026-10-15T03:00:00+00:00",
                     "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}}
                ],
                "extra_usage": {"is_enabled": true, "utilization": 24.6, "spend_limit_reached": false},
                "spend": {"used": {"amount_minor": 1230, "currency": "USD", "exponent": 2},
                          "limit": {"amount_minor": 5000, "currency": "USD", "exponent": 2}, "enabled": true},
                "seven_day_breakdown": {"window_started_at": "2026-10-08T03:00:00+00:00", "rows": [
                    {"key": "claude_code", "display_name": "Claude Code", "percent": 96},
                    {"key": "chat", "display_name": "Chats", "percent": 3}
                ]},
                "weekly_scoped_shares": [{"limit_index": 2, "used_percent_of_weekly": 6, "allowance_percent_of_weekly": 50}]
            }"#,
        )
        .unwrap();
        let usage = parse_usage(&body);
        assert_eq!(usage.limits[2].share, Some((6.0, 50.0)));
        assert_eq!(usage.limits[1].share, None);
        let breakdown = usage.breakdown.unwrap();
        assert_eq!(breakdown.since.unwrap().to_rfc3339(), "2026-10-08T03:00:00+00:00");
        assert_eq!(breakdown.parts[0], ("claude_code".into(), "Claude Code".into(), 96.0));
        let extra = usage.extra.unwrap();
        assert!(extra.enabled);
        assert_eq!(extra.used.unwrap().to_string(), "$12.30");
        assert_eq!(extra.limit.unwrap().to_string(), "$50.00");
        let mut state = State { limits: usage.limits, ..State::default() };
        assert_eq!(state.week_start().to_rfc3339(), "2026-10-08T03:00:00+00:00", "from the weekly limit");
        state.limits.clear();
        assert!(Utc::now() - state.week_start() < TimeDelta::days(7), "no usage yet: about a week back");
    }

    #[test]
    fn projects_where_a_limit_is_headed() {
        let now = Utc::now();
        let weekly = |percent, hours_left| Limit {
            label: "Weekly".into(),
            percent,
            resets_at: Some(now + TimeDelta::hours(hours_left)),
            window: Some(TimeDelta::days(7)),
            share: None,
        };
        // 4 of 7 days gone.
        let Some(Pace::Finish(end)) = weekly(40.0, 72).pace(now) else { panic!("should finish") };
        assert!((end - 70.0).abs() < 0.5, "{end}");
        // 80% used halfway: 100% after 5/8 of the week.
        let Some(Pace::RunsOut(at)) = weekly(80.0, 84).pace(now) else { panic!("should run out") };
        let start = now - TimeDelta::hours(84);
        assert!((at - (start + TimeDelta::hours(105))).num_minutes().abs() <= 1, "{at}");
        assert_eq!(weekly(100.0, 84).pace(now), Some(Pace::Reached));
        assert_eq!(weekly(5.0, 7 * 24 - 2).pace(now), None, "too early to tell");
        assert_eq!(Limit { window: None, ..weekly(40.0, 72) }.pace(now), None);
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
        let limits = parse_usage(&body).limits;
        let labels: Vec<_> = limits.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["Session", "Weekly", "Fable"]);
        assert_eq!(limits[0].percent, 15.0);
        assert_eq!(limits[2].window, Some(TimeDelta::days(7)));
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
        let limits = parse_usage(&body).limits;
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

    #[test]
    fn reads_steps_as_the_transcript_grows_but_not_what_came_before() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("cuota-steps-{}.jsonl", std::process::id()));
        let append = |lines: &[&str]| {
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
            for line in lines {
                writeln!(file, "{line}").unwrap();
            }
        };
        let tool = |name: &str| {
            format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"t","name":"{name}","input":{{}}}}]}}}}"#)
        };
        let _ = std::fs::remove_file(&path);
        let mut read = Transcript::default();
        append(&[&tool("Edit"), r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Hm"}]}}"#]);
        read.update(&path);
        assert_eq!((read.steps, read.step), (0, None), "steps from before Cuota looked");

        append(&[r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Hm"}]}}"#]);
        read.update(&path);
        assert_eq!((read.steps, read.step), (1, Some(Step::Think)));
        for (name, step) in [
            ("Grep", Step::Look),
            ("Write", Step::Edit),
            ("Bash", Step::Run),
            ("WebFetch", Step::Web),
            ("mcp__claude-in-chrome__navigate", Step::Web),
            ("Agent", Step::Delegate),
            ("TodoWrite", Step::Plan),
        ] {
            append(&[&tool(name)]);
            read.update(&path);
            assert_eq!(read.step, Some(step), "{name}");
        }
        assert_eq!(read.steps, 8);

        append(&[
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","is_error":true,"content":"No"}]}}"#,
        ]);
        read.update(&path);
        assert_eq!((read.steps, read.step), (9, Some(Step::Fail)));
        append(&[r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}"#]);
        read.update(&path);
        assert_eq!((read.steps, read.step), (10, Some(Step::Compact)));

        // Text and results, questions for you, tools it doesn't know, and steps quoted in a
        // message aren't steps.
        append(&[
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}"#,
            &tool("AskUserQuestion"),
            &tool("SomethingNew"),
            r#"{"type":"user","message":{"content":"grep '{\"type\":\"assistant\",\"is_error\":true}'"}}"#,
        ]);
        read.update(&path);
        assert_eq!((read.steps, read.step), (10, Some(Step::Compact)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reads_titles_as_the_transcript_grows() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("cuota-titles-{}.jsonl", std::process::id()));
        let append = |text: &str| {
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
            file.write_all(text.as_bytes()).unwrap();
        };
        let _ = std::fs::remove_file(&path);
        let mut titles = Transcript::default();
        titles.update(&path);
        assert_eq!(titles.title(), None, "no transcript yet");

        append(concat!(
            r#"{"type":"user","message":{"content":"grep '{\"type\":\"ai-title\",\"aiTitle\":\"Quoted\"}'"}}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"First title","sessionId":"x"}"#,
            "\n",
        ));
        titles.update(&path);
        assert_eq!(titles.title().as_deref(), Some("First title"), "titles quoted in messages don't count");

        append(r#"{"type":"ai-title","aiTitle":"Half"#);
        titles.update(&path);
        assert_eq!(titles.title().as_deref(), Some("First title"), "waits for the whole line");
        append(concat!(r#" written"}"#, "\n"));
        titles.update(&path);
        assert_eq!(titles.title().as_deref(), Some("Half written"));

        append(concat!(r#"{"type":"custom-title","customTitle":"Renamed"}"#, "\n", r#"{"type":"ai-title","aiTitle":"Later"}"#, "\n"));
        titles.update(&path);
        assert_eq!(titles.title().as_deref(), Some("Renamed"), "a rename wins");

        std::fs::write(&path, concat!(r#"{"type":"ai-title","aiTitle":"Fresh"}"#, "\n")).unwrap();
        titles.update(&path);
        assert_eq!(titles.title().as_deref(), Some("Fresh"), "starts over when rewritten");
        let _ = std::fs::remove_file(&path);
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
    fn rate_limits_wait_minutes_even_when_told_zero() {
        let (mut poller, now) = poller_with_data(2);
        let zero = Some(TimeDelta::zero());
        assert_eq!(poller.rate_limit_wait(zero), TimeDelta::minutes(5));
        assert_eq!(poller.rate_limit_wait(zero), TimeDelta::minutes(10));
        assert_eq!(poller.rate_limit_wait(None), TimeDelta::minutes(15));
        assert_eq!(poller.rate_limit_wait(None), TimeDelta::minutes(15), "capped");
        assert_eq!(poller.rate_limit_wait(Some(TimeDelta::hours(1))), TimeDelta::hours(1), "a longer ask is kept");

        poller.retry_at = Some(now + TimeDelta::minutes(5));
        assert!(poller.rate_limited(now), "Refresh waits too");
        poller.backoff = TimeDelta::zero();
        assert!(!poller.rate_limited(now), "other failures can be refreshed past");
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
