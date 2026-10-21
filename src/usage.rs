//! How much of each limit Claude Code's tasks use: the limits are noted when a session starts
//! working, and again once usage fetched after it's done comes in. The tasks that finished are
//! kept, newest first, in `usage.json` beside the settings, with the plan they ran on, since
//! the same work takes a smaller share of a bigger plan.

use crate::claude::{Limit, Session, State};
use crate::log;
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Usage fetched this soon after a task ended may not count all of it yet. The poller fetches
/// once more for activity this close before a fetch, so it waits as long.
const SETTLE: TimeDelta = TimeDelta::seconds(15);
/// A task whose closing usage hasn't come in by then is let go: the API kept failing.
const GIVE_UP: TimeDelta = TimeDelta::hours(6);
/// Reset times that differ by less than this are the same window, give or take rounding.
const SAME_RESET: TimeDelta = TimeDelta::minutes(1);
/// The most tasks kept. The oldest go first.
const KEPT: usize = 1000;

/// One limit, before a task and after it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub label: String,
    pub before: f32,
    pub after: f32,
    /// The window reset while the task ran, so `after` counts from zero.
    #[serde(default)]
    pub reset: bool,
}

impl Change {
    /// How much of the limit the task used, in percent. Across a reset, only what it used
    /// since then is known.
    pub fn used(&self) -> f32 {
        if self.reset { self.after } else { (self.after - self.before).max(0.0) }
    }
}

/// A task a Claude Code session worked through: from when it started working to when it was
/// done, counting time spent waiting on you.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// The conversation's title, or the session's name.
    pub session: String,
    pub project: Option<String>,
    pub app: String,
    /// The plan it ran on, like "Claude Max 5x".
    pub plan: Option<String>,
    pub started: DateTime<Utc>,
    pub ended: DateTime<Utc>,
    pub changes: Vec<Change>,
    /// Other sessions worked at the same time, so the changes count their usage too.
    #[serde(default)]
    pub shared: bool,
}

impl Task {
    /// The first limit's change, which is the session's: what the task took of it.
    pub fn headline(&self) -> Option<&Change> {
        self.changes.first()
    }
}

/// What tasks on one plan used on average, leaving out those that shared their time with
/// other sessions.
#[derive(Clone, Debug, PartialEq)]
pub struct Average {
    pub plan: String,
    pub tasks: usize,
    pub duration: TimeDelta,
    /// Each limit, in the order the API lists them, and its average use in percent.
    pub used: Vec<(String, f32)>,
}

/// A limit as a task began.
#[derive(Clone, Debug)]
struct Mark {
    label: String,
    percent: f32,
    resets_at: Option<DateTime<Utc>>,
}

impl Mark {
    fn of(limit: &Limit) -> Self {
        Self { label: limit.label.clone(), percent: limit.percent, resets_at: limit.resets_at }
    }
}

/// Sessions are told apart by name and start, as the sounds do.
type Key = (String, Option<DateTime<Utc>>);

fn key(session: &Session) -> Key {
    (session.name.clone(), session.started)
}

/// A task under way, or done and waiting for usage fetched after it.
#[derive(Clone, Debug)]
struct Running {
    key: Key,
    started: DateTime<Utc>,
    before: Vec<Mark>,
    plan: Option<String>,
    shared: bool,
    ended: Option<DateTime<Utc>>,
    /// The latest of how the session was called and where it ran.
    session: Session,
}

/// The tasks kept, and those under way.
pub struct History {
    pub tasks: Vec<Task>,
    running: Vec<Running>,
    /// Sessions already working when Cuota first looked, whose start it missed. Their next
    /// task counts.
    missed: Vec<Key>,
    looked: bool,
    /// Where they're saved. Nowhere in the demo.
    path: Option<PathBuf>,
}

impl History {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Cuota").join("usage.json"))
    }

    pub fn load() -> Self {
        let path = Self::path();
        let tasks = path.as_ref().map_or_else(Vec::new, |path| match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("Couldn't read the usage history in {}: {e}", path.display());
                Vec::new()
            }),
            Err(_) => Vec::new(),
        });
        Self::with(tasks, path)
    }

    fn with(tasks: Vec<Task>, path: Option<PathBuf>) -> Self {
        Self { tasks, running: Vec::new(), missed: Vec::new(), looked: false, path }
    }

    /// Made-up tasks for `--demo`, never saved.
    pub fn demo() -> Self {
        let now = Utc::now();
        let task = |session: &str, project: &str, mins_ago: i64, mins: i64, changes: [(f32, f32); 3], shared| Task {
            session: session.into(),
            project: Some(project.into()),
            app: "VS Code".into(),
            plan: Some("Claude Max 5x".into()),
            started: now - TimeDelta::minutes(mins_ago + mins),
            ended: now - TimeDelta::minutes(mins_ago),
            changes: ["Session", "Weekly", "Fable"]
                .into_iter()
                .zip(changes)
                .map(|(label, (before, after))| Change { label: label.into(), before, after, reset: false })
                .collect(),
            shared,
        };
        Self::with(
            vec![
                task("Show each session's title in the details", "Cuota", 12, 18, [(9.0, 15.0), (47.0, 49.0), (90.0, 92.0)], false),
                task("Fix the pricing page layout", "Website", 95, 7, [(6.0, 9.0), (46.0, 47.0), (89.0, 90.0)], true),
                task("Add a usage history page", "Cuota", 240, 35, [(1.0, 6.0), (44.0, 46.0), (88.0, 89.0)], false),
            ],
            None,
        )
    }

    /// Follows Claude's sessions: notes the limits as one starts working, and keeps the task
    /// once usage fetched after it's done comes in. Off, it forgets the tasks under way.
    /// Returns whether a task was kept.
    pub fn update(&mut self, state: &State, on: bool, now: DateTime<Utc>) -> bool {
        if !on {
            self.running.clear();
            self.missed.clear();
            self.looked = false;
            return false;
        }
        // Waiting on you is still part of the task, as it is for the sounds.
        let engaged: Vec<&Session> = state.sessions.iter().filter(|s| s.busy || s.waiting).collect();
        if !self.looked {
            self.looked = true;
            self.missed = engaged.iter().map(|s| key(s)).collect();
        }
        self.missed.retain(|k| engaged.iter().any(|s| key(s) == *k));

        for session in &engaged {
            let k = key(session);
            if let Some(running) = self.running.iter_mut().find(|r| r.key == k) {
                // Back at it before its usage came in: the same task, still going.
                running.ended = None;
                running.session = (*session).clone();
            } else if !self.missed.contains(&k) && !state.limits.is_empty() {
                self.running.push(Running {
                    key: k,
                    started: now,
                    before: state.limits.iter().map(Mark::of).collect(),
                    plan: state.account.plan.clone(),
                    shared: false,
                    ended: None,
                    session: (*session).clone(),
                });
            }
        }
        for running in &mut self.running {
            let mine = engaged.iter().any(|s| key(s) == running.key);
            if !mine && running.ended.is_none() {
                running.ended = Some(now);
            }
            if engaged.iter().any(|s| key(s) != running.key) {
                running.shared = true;
            }
        }

        let updated = state.updated.map(|t| t.to_utc());
        let mut kept = false;
        let mut i = 0;
        while i < self.running.len() {
            let Some(ended) = self.running[i].ended else {
                i += 1;
                continue;
            };
            if updated.is_some_and(|t| t >= ended + SETTLE) && !state.limits.is_empty() {
                let running = self.running.remove(i);
                self.keep(finish(running, ended, state));
                kept = true;
            } else if now - ended > GIVE_UP {
                self.running.remove(i);
            } else {
                i += 1;
            }
        }
        if kept {
            self.save();
        }
        kept
    }

    fn keep(&mut self, task: Task) {
        self.tasks.insert(0, task);
        self.tasks.truncate(KEPT);
    }

    pub fn clear(&mut self) {
        self.tasks.clear();
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let save = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, serde_json::to_string_pretty(&self.tasks)?)
        };
        if let Err(e) = save() {
            log::error!("Couldn't save the usage history to {}: {e}", path.display());
        }
    }

    /// What tasks used on average on each plan, the most used plan first. Tasks that shared
    /// their time with other sessions are left out, since their changes count those too.
    pub fn averages(&self) -> Vec<Average> {
        let mut plans: BTreeMap<&str, Vec<&Task>> = BTreeMap::new();
        for task in self.tasks.iter().filter(|t| !t.shared) {
            plans.entry(task.plan.as_deref().unwrap_or("Unknown plan")).or_default().push(task);
        }
        let mut averages: Vec<Average> = plans
            .into_iter()
            .map(|(plan, tasks)| {
                let count = tasks.len();
                let duration = tasks.iter().map(|t| t.ended - t.started).sum::<TimeDelta>() / count as i32;
                let mut used: Vec<(String, f32, usize)> = Vec::new();
                for change in tasks.iter().flat_map(|t| &t.changes) {
                    match used.iter_mut().find(|(label, ..)| *label == change.label) {
                        Some((_, sum, n)) => {
                            *sum += change.used();
                            *n += 1;
                        }
                        None => used.push((change.label.clone(), change.used(), 1)),
                    }
                }
                let used = used.into_iter().map(|(label, sum, n)| (label, sum / n as f32)).collect();
                Average { plan: plan.to_owned(), tasks: count, duration, used }
            })
            .collect();
        averages.sort_by(|a, b| b.tasks.cmp(&a.tasks));
        averages
    }
}

/// The task, with each limit as it began and as it is now.
fn finish(running: Running, ended: DateTime<Utc>, state: &State) -> Task {
    let changes = state
        .limits
        .iter()
        .filter_map(|limit| {
            let before = running.before.iter().find(|m| m.label == limit.label)?;
            let reset = match (before.resets_at, limit.resets_at) {
                (Some(then), Some(now)) => (now - then).abs() > SAME_RESET,
                _ => false,
            };
            Some(Change { label: limit.label.clone(), before: before.percent, after: limit.percent, reset })
        })
        .collect();
    let session = running.session;
    Task {
        session: session.display_name().to_owned(),
        project: session.project,
        app: session.app,
        plan: running.plan.or_else(|| state.account.plan.clone()),
        started: running.started,
        ended,
        changes,
        shared: running.shared,
    }
}

/// "<1%" for less than a percent, "+3%" otherwise.
pub fn used_text(used: f32) -> String {
    if used < 0.5 { "<1%".into() } else { format!("+{used:.0}%") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::Account;
    use chrono::Local;

    fn limit(label: &str, percent: f32, resets_in_hours: i64, now: DateTime<Utc>) -> Limit {
        Limit {
            label: label.into(),
            percent,
            resets_at: Some(now + TimeDelta::hours(resets_in_hours)),
            window: None,
            share: None,
        }
    }

    fn session(name: &str, busy: bool, waiting: bool) -> Session {
        Session { name: name.into(), app: "Terminal".into(), busy, waiting, ..Session::default() }
    }

    struct World {
        now: DateTime<Utc>,
        state: State,
        history: History,
    }

    impl World {
        fn new() -> Self {
            let now = Utc::now();
            let state = State {
                limits: vec![limit("Session", 6.0, 3, now), limit("Weekly", 88.0, 100, now), limit("Fable", 84.0, 100, now)],
                account: Account { plan: Some("Claude Pro".into()), ..Account::default() },
                updated: Some(now.with_timezone(&Local)),
                sessions: vec![session("a", false, false)],
                ..State::default()
            };
            let mut world = Self { now, state, history: History::with(Vec::new(), None) };
            world.look();
            world
        }

        fn look(&mut self) -> bool {
            self.history.update(&self.state, true, self.now)
        }

        fn wait(&mut self, secs: i64) {
            self.now += TimeDelta::seconds(secs);
        }

        fn set(&mut self, name: &str, busy: bool, waiting: bool) {
            let s = self.state.sessions.iter_mut().find(|s| s.name == name).unwrap();
            s.busy = busy;
            s.waiting = waiting;
        }

        fn fetch(&mut self, percents: [f32; 3]) {
            for (limit, percent) in self.state.limits.iter_mut().zip(percents) {
                limit.percent = percent;
            }
            self.state.updated = Some(self.now.with_timezone(&Local));
        }
    }

    fn used(task: &Task) -> Vec<(&str, f32, f32, f32)> {
        task.changes.iter().map(|c| (c.label.as_str(), c.before, c.after, c.used())).collect()
    }

    #[test]
    fn a_task_is_kept_once_usage_fetched_after_it_ended_comes_in() {
        let mut w = World::new();
        w.set("a", true, false);
        w.look();
        w.wait(60);
        w.set("a", false, true);
        w.look();
        w.wait(60);
        w.fetch([8.0, 89.0, 84.0]);
        w.set("a", false, false);
        assert!(!w.look(), "still waiting on you, so not done yet");
        // Fetched too soon after it ended to count all of it.
        w.wait(5);
        w.fetch([9.0, 89.0, 84.0]);
        assert!(!w.look());
        w.wait(60);
        w.fetch([10.0, 89.0, 84.0]);
        assert!(w.look());
        let task = &w.history.tasks[0];
        assert_eq!(used(task), [("Session", 6.0, 10.0, 4.0), ("Weekly", 88.0, 89.0, 1.0), ("Fable", 84.0, 84.0, 0.0)]);
        assert_eq!(task.plan.as_deref(), Some("Claude Pro"));
        assert_eq!((task.ended - task.started).num_seconds(), 120);
        assert!(!task.shared);
        assert!(!w.look(), "kept once");
    }

    #[test]
    fn going_back_to_work_before_the_usage_came_in_carries_on_the_same_task() {
        let mut w = World::new();
        w.set("a", true, false);
        w.look();
        w.wait(30);
        w.set("a", false, false);
        w.look();
        w.wait(10);
        w.set("a", true, false);
        w.look();
        w.wait(30);
        w.set("a", false, false);
        w.look();
        w.wait(30);
        w.fetch([12.0, 90.0, 85.0]);
        assert!(w.look());
        assert_eq!(w.history.tasks.len(), 1);
        assert_eq!((w.history.tasks[0].ended - w.history.tasks[0].started).num_seconds(), 70);
    }

    #[test]
    fn work_under_way_when_cuota_first_looks_is_left_out_but_the_next_task_counts() {
        let mut w = World::new();
        let mut history = History::with(Vec::new(), None);
        w.set("a", true, false);
        history.update(&w.state, true, w.now);
        w.set("a", false, false);
        w.wait(60);
        w.fetch([9.0, 89.0, 84.0]);
        assert!(!history.update(&w.state, true, w.now));
        w.set("a", true, false);
        history.update(&w.state, true, w.now);
        w.set("a", false, false);
        history.update(&w.state, true, w.now);
        w.wait(60);
        w.fetch([11.0, 89.0, 84.0]);
        assert!(history.update(&w.state, true, w.now));
        assert_eq!(used(&history.tasks[0])[0], ("Session", 9.0, 11.0, 2.0));
    }

    #[test]
    fn a_task_alongside_another_session_is_marked_shared_and_left_out_of_the_averages() {
        let mut w = World::new();
        w.state.sessions.push(session("b", false, false));
        w.set("a", true, false);
        w.look();
        w.set("b", true, false);
        w.look();
        w.set("a", false, false);
        w.set("b", false, false);
        w.look();
        w.wait(60);
        w.fetch([14.0, 90.0, 84.0]);
        assert!(w.look());
        assert_eq!(w.history.tasks.len(), 2);
        assert!(w.history.tasks.iter().all(|t| t.shared));
        assert!(w.history.averages().is_empty());
    }

    #[test]
    fn a_window_that_reset_during_a_task_counts_from_zero() {
        let mut w = World::new();
        w.set("a", true, false);
        w.look();
        w.wait(60);
        w.set("a", false, false);
        w.look();
        w.wait(60);
        w.fetch([3.0, 89.0, 84.0]);
        w.state.limits[0].resets_at = Some(w.now + TimeDelta::hours(5));
        assert!(w.look());
        let session = &w.history.tasks[0].changes[0];
        assert!(session.reset);
        assert_eq!(session.used(), 3.0);
    }

    #[test]
    fn nothing_is_tracked_while_turned_off() {
        let mut w = World::new();
        w.set("a", true, false);
        w.look();
        w.history.update(&w.state, false, w.now);
        w.set("a", false, false);
        w.wait(60);
        w.fetch([9.0, 89.0, 84.0]);
        assert!(!w.look());
        assert!(w.history.tasks.is_empty());
    }

    #[test]
    fn averages_go_by_plan() {
        let mut history = History::demo();
        let mut pro = history.tasks[0].clone();
        pro.plan = Some("Claude Pro".into());
        pro.changes[0] = Change { label: "Session".into(), before: 10.0, after: 40.0, reset: false };
        history.tasks.push(pro);
        let averages = history.averages();
        let plans: Vec<(&str, usize)> = averages.iter().map(|a| (a.plan.as_str(), a.tasks)).collect();
        assert_eq!(plans, [("Claude Max 5x", 2), ("Claude Pro", 1)]);
        assert_eq!(averages[0].used[0], ("Session".to_owned(), 5.5));
        assert_eq!(averages[1].used[0], ("Session".to_owned(), 30.0));
    }

    #[test]
    fn tasks_survive_a_save_and_older_files_without_new_fields_still_load() {
        let history = History::demo();
        let text = serde_json::to_string(&history.tasks).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Task>>(&text).unwrap(), history.tasks);
        let old = r#"[{"session":"x","project":null,"app":"Terminal","plan":null,
            "started":"2026-10-21T10:00:00Z","ended":"2026-10-21T10:05:00Z",
            "changes":[{"label":"Session","before":1.0,"after":2.0}]}]"#;
        let tasks: Vec<Task> = serde_json::from_str(old).unwrap();
        assert!(!tasks[0].shared && !tasks[0].changes[0].reset);
    }

    #[test]
    fn under_a_percent_reads_as_less_than_one() {
        assert_eq!(used_text(0.0), "<1%");
        assert_eq!(used_text(3.2), "+3%");
    }
}
