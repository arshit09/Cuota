//! What Claude Code did this week on this PC, from the transcripts it keeps in `projects/`:
//! every reply there names its model and counts its tokens. They add up to gigabytes, so they're
//! read on a thread of their own, only while the panel shows some of what they tell, and each
//! file only from where the last read stopped.

use crate::claude::{self, title_case};
use chrono::{DateTime, Local, NaiveDate, TimeDelta, Timelike, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

impl std::ops::AddAssign for Tokens {
    fn add_assign(&mut self, other: Self) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tally {
    pub replies: u64,
    pub tool_calls: u64,
    pub tokens: Tokens,
}

impl Tally {
    fn add(&mut self, reply: &Reply) {
        self.replies += 1;
        self.tool_calls += u64::from(reply.tool_calls);
        self.tokens += reply.tokens;
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub total: Tally,
    /// Each local day from `since` through today, oldest first.
    pub days: Vec<(NaiveDate, Tally)>,
    /// Most tokens first.
    pub models: Vec<(String, Tally)>,
    /// Folders Claude Code worked in, most tokens first.
    pub projects: Vec<(String, Tally)>,
    /// The local hour with the most replies.
    pub busiest_hour: Option<u32>,
}

/// Reads transcripts on its own thread when asked, and shares what it found.
#[derive(Clone)]
pub struct Transcripts {
    ask: Option<Sender<DateTime<Utc>>>,
    latest: Arc<Mutex<Option<Arc<Stats>>>>,
}

impl Transcripts {
    /// `on_update` runs after each read, so the UI can repaint.
    pub fn spawn(on_update: impl Fn() + Send + 'static) -> Self {
        let (ask, asked) = mpsc::channel::<DateTime<Utc>>();
        let latest = Arc::new(Mutex::new(None));
        let shared = latest.clone();
        std::thread::spawn(move || {
            let mut reader = Reader::new(claude::claude_dir().join("projects"));
            while let Ok(since) = asked.recv() {
                // Only the newest ask counts.
                let since = asked.try_iter().last().unwrap_or(since);
                reader.read(since);
                *shared.lock().unwrap() = Some(Arc::new(reader.stats(Local::now().date_naive())));
                on_update();
            }
        });
        Self { ask: Some(ask), latest }
    }

    /// Fake numbers for `--demo`.
    pub fn demo(since: DateTime<Utc>) -> Self {
        let mut reader = Reader::new(PathBuf::new());
        reader.since = since;
        let today = Local::now().date_naive();
        let days = (today - since.with_timezone(&Local).date_naive()).num_days();
        for day in 0..=days {
            let at = since + TimeDelta::days(day);
            for (model, project, replies) in [("claude-opus-5-5", "Cuota", 120), ("claude-fable-5-1", "Website", 40)] {
                for i in 0..(replies * (day + 2) % 170) {
                    let tokens = Tokens { input: 4, output: 600, cache_read: 60_000, cache_write: 3_000 };
                    let line = Line { at: at + TimeDelta::minutes(i * 3), model, project, tokens, tool_calls: 1 };
                    reader.add(&line, None);
                }
            }
        }
        let stats = reader.stats(today);
        Self { ask: None, latest: Arc::new(Mutex::new(Some(Arc::new(stats)))) }
    }

    /// Reads what was written since the last time, counting replies from `since` on.
    pub fn refresh(&self, since: DateTime<Utc>) {
        if let Some(ask) = &self.ask {
            let _ = ask.send(since);
        }
    }

    pub fn latest(&self) -> Option<Arc<Stats>> {
        self.latest.lock().unwrap().clone()
    }
}

/// One reply, as kept between reads.
struct Reply {
    at: DateTime<Utc>,
    model: usize,
    project: usize,
    tokens: Tokens,
    tool_calls: u32,
}

/// What one transcript line says about a reply.
struct Line<'a> {
    at: DateTime<Utc>,
    model: &'a str,
    project: &'a str,
    tokens: Tokens,
    tool_calls: u32,
}

struct Reader {
    root: PathBuf,
    since: DateTime<Utc>,
    /// How far each transcript has been read, in bytes.
    read_to: HashMap<PathBuf, u64>,
    replies: Vec<Reply>,
    /// Claude Code writes a reply a line per part (thinking, text, each tool call), every line
    /// repeating its usage, so replies are told apart by message and request id.
    seen: HashMap<String, usize>,
    models: Vec<String>,
    projects: Vec<String>,
}

impl Reader {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            since: DateTime::<Utc>::MIN_UTC,
            read_to: HashMap::new(),
            replies: Vec::new(),
            seen: HashMap::new(),
            models: Vec::new(),
            projects: Vec::new(),
        }
    }

    fn read(&mut self, since: DateTime<Utc>) {
        if since != self.since {
            // A new week, or the first one: start over.
            *self = Self { since, ..Self::new(std::mem::take(&mut self.root)) };
        }
        let mut files = Vec::new();
        find_transcripts(&self.root, since, &mut files);
        for (path, len) in files {
            let done = self.read_to.get(&path).copied().unwrap_or(0);
            if len == done {
                continue;
            }
            // Shorter than before: rewritten, so read it again. Replies seen before don't count twice.
            let from = if len < done { 0 } else { done };
            let to = claude::read_lines(&path, from, |line| self.read_line(line));
            self.read_to.insert(path, to);
        }
    }

    fn read_line(&mut self, line: &[u8]) {
        let Ok(text) = std::str::from_utf8(line) else { return };
        // Most lines aren't replies; skip them without parsing.
        if !text.contains(r#""type":"assistant""#) {
            return;
        }
        let Ok(entry) = serde_json::from_str::<Entry>(text) else { return };
        let Some(message) = entry.message.filter(|_| entry.kind == "assistant") else { return };
        let (Some(usage), Some(model)) = (message.usage, message.model) else { return };
        let Some(at) = entry.timestamp.as_deref().and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else { return };
        // "<synthetic>" marks messages Claude Code wrote itself, without a model.
        if model.starts_with('<') {
            return;
        }
        let cwd = entry.cwd.unwrap_or_default();
        let project = Path::new(&cwd).file_name().and_then(|n| n.to_str()).unwrap_or("Other");
        let key = message.id.map(|id| format!("{id}:{}", entry.request_id.unwrap_or_default()));
        let count = |n: Option<u64>| n.unwrap_or(0);
        self.add(
            &Line {
                at: at.to_utc(),
                model: &model,
                project,
                tokens: Tokens {
                    input: count(usage.input_tokens),
                    output: count(usage.output_tokens),
                    cache_read: count(usage.cache_read_input_tokens),
                    cache_write: count(usage.cache_creation_input_tokens),
                },
                // Each tool call is its own content block; compact JSON makes this exact.
                tool_calls: text.matches(r#"{"type":"tool_use","#).count() as u32,
            },
            key,
        );
    }

    fn add(&mut self, line: &Line, key: Option<String>) {
        if line.at < self.since {
            return;
        }
        if let Some(&i) = key.as_ref().and_then(|k| self.seen.get(k)) {
            self.replies[i].tool_calls += line.tool_calls;
            return;
        }
        if let Some(key) = key {
            self.seen.insert(key, self.replies.len());
        }
        let model = intern(&mut self.models, &model_name(line.model));
        let project = intern(&mut self.projects, line.project);
        self.replies.push(Reply { at: line.at, model, project, tokens: line.tokens, tool_calls: line.tool_calls });
    }

    fn stats(&self, today: NaiveDate) -> Stats {
        let first = self.since.with_timezone(&Local).date_naive();
        let mut days: Vec<(NaiveDate, Tally)> =
            first.iter_days().take_while(|d| *d <= today).map(|d| (d, Tally::default())).collect();
        let mut models = vec![Tally::default(); self.models.len()];
        let mut projects = vec![Tally::default(); self.projects.len()];
        let (mut total, mut hours) = (Tally::default(), [0u64; 24]);
        for reply in &self.replies {
            let local = reply.at.with_timezone(&Local);
            if let Some((_, day)) = days.get_mut((local.date_naive() - first).num_days() as usize) {
                day.add(reply);
            }
            hours[local.hour() as usize] += 1;
            models[reply.model].add(reply);
            projects[reply.project].add(reply);
            total.add(reply);
        }
        let ranked = |names: &[String], tallies: Vec<Tally>| {
            let mut ranked: Vec<(String, Tally)> = names.iter().cloned().zip(tallies).collect();
            ranked.sort_by(|a, b| b.1.tokens.total().cmp(&a.1.tokens.total()));
            ranked
        };
        let busiest_hour = (0..24u32).filter(|&h| hours[h as usize] > 0).max_by_key(|&h| hours[h as usize]);
        Stats {
            total,
            days,
            models: ranked(&self.models, models),
            projects: ranked(&self.projects, projects),
            busiest_hour,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    cwd: Option<String>,
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

/// Every `.jsonl` under `dir` changed since `since`, with its length. Subagents keep theirs in
/// folders under their session's.
fn find_transcripts(dir: &Path, since: DateTime<Utc>, found: &mut Vec<(PathBuf, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let path = entry.path();
        if meta.is_dir() {
            find_transcripts(&path, since, found);
        } else if path.extension().is_some_and(|e| e == "jsonl")
            && meta.modified().is_ok_and(|t| DateTime::<Utc>::from(t) >= since)
        {
            found.push((path, meta.len()));
        }
    }
}

fn intern(names: &mut Vec<String>, name: &str) -> usize {
    names.iter().position(|n| n == name).unwrap_or_else(|| {
        names.push(name.to_owned());
        names.len() - 1
    })
}

/// "claude-opus-5-5" is "Opus 5.5", "claude-sonnet-4-5-20250929" is "Sonnet 4.5", and
/// "claude-3-5-haiku-20241022" is "Haiku 3.5".
fn model_name(id: &str) -> String {
    let id = id.split('[').next().unwrap_or(id);
    let id = id.strip_prefix("claude-").unwrap_or(id);
    let (mut family, mut version) = (Vec::new(), Vec::new());
    for part in id.split('-') {
        if part.chars().all(|c| c.is_ascii_digit()) {
            // Longer numbers are release dates.
            if part.len() <= 2 {
                version.push(part);
            }
        } else {
            family.push(title_case(part));
        }
    }
    match (family.is_empty(), version.is_empty()) {
        (true, _) => id.to_owned(),
        (false, true) => family.join(" "),
        (false, false) => format!("{} {}", family.join(" "), version.join(".")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_models() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-fable-5-1"), "Fable 5.1");
        assert_eq!(model_name("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(model_name("claude-3-5-haiku-20241022"), "Haiku 3.5");
        assert_eq!(model_name("claude-opus-4-6[1m]"), "Opus 4.6");
    }

    fn line(id: &str, at: &str, content: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{at}","cwd":"g:\\Data\\Git\\Cuota","requestId":"req_1","message":{{"model":"claude-opus-5-5","id":"{id}","type":"message","role":"assistant","content":[{content}],"usage":{{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":50}}}}}}"#
        )
    }

    #[test]
    fn counts_each_reply_once_with_all_its_tool_calls() {
        let at = "2026-10-13T10:00:00Z";
        let mut reader = Reader::new(PathBuf::new());
        reader.since = DateTime::parse_from_rfc3339("2026-10-08T03:00:00Z").unwrap().to_utc();
        let thinking = r#"{"type":"thinking","thinking":"{\"type\":\"tool_use\",\"id\":\"x\"}"}"#;
        let tool = r#"{"type":"tool_use","id":"toolu_1","name":"Read","input":{}}"#;
        for content in [thinking, tool, tool] {
            reader.read_line(line("msg_1", at, content).as_bytes());
        }
        reader.read_line(line("msg_2", at, tool).as_bytes());
        reader.read_line(line("msg_old", "2026-10-01T10:00:00Z", tool).as_bytes());
        reader.read_line(br#"{"type":"user","message":{"role":"user","content":"hi"}}"#);

        let stats = reader.stats(NaiveDate::from_ymd_opt(2026, 10, 13).unwrap());
        assert_eq!(stats.total.replies, 2, "repeated lines and last week's reply don't count");
        assert_eq!(stats.total.tool_calls, 3, "tool calls quoted in thinking don't count");
        assert_eq!(stats.total.tokens.total(), 2 * 1152);
        assert_eq!(stats.models[0].0, "Opus 5.5");
        assert_eq!(stats.projects[0].0, "Cuota");
        assert_eq!(stats.days.last().unwrap().1.replies, 2);
    }

    #[test]
    fn reads_only_whole_lines_and_picks_up_where_it_stopped() {
        let dir = std::env::temp_dir().join(format!("cuota-stats-{}", std::process::id()));
        let sub = dir.join("project").join("session").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let path = sub.join("agent.jsonl");
        let now = Utc::now().to_rfc3339();
        let tool = r#"{"type":"tool_use","id":"toolu_1","name":"Read","input":{}}"#;
        let first = line("msg_1", &now, tool);
        std::fs::write(&path, format!("{first}\n{}", &line("msg_2", &now, tool)[..40])).unwrap();

        let mut reader = Reader::new(dir.clone());
        let since = Utc::now() - TimeDelta::days(1);
        reader.read(since);
        assert_eq!(reader.replies.len(), 1, "the unfinished line waits");
        assert_eq!(reader.read_to[&path], first.len() as u64 + 1);

        std::fs::write(&path, format!("{first}\n{}\n", line("msg_2", &now, tool))).unwrap();
        reader.read(since);
        assert_eq!(reader.replies.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
