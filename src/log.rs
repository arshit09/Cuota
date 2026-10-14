//! A log of what Cuota ran into, such as rate limits, failed requests and crashes, to read
//! later: `%LOCALAPPDATA%\Cuota\Logs\cuota.log`. Past [`MAX_SIZE`] it moves to `cuota.old.log`,
//! replacing the one before, so the two never take much more than 2 MB.
//!
//! Log with [`info!`], [`warn!`] and [`error!`], as `log::warn!("…")`. Until [`start`], they
//! write nowhere (debug builds still print them), which keeps tests out of the real log.
//! Leave out the account's email and what sessions are about, so a log can be shared to report
//! a problem.

use chrono::Local;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const MAX_SIZE: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

macro_rules! log_info {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Info, format_args!($($arg)*)) };
}
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Warn, format_args!($($arg)*)) };
}
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::log::write($crate::log::Level::Error, format_args!($($arg)*)) };
}
// Named apart at first, since a `warn` of our own would clash with the #[warn] attribute.
pub(crate) use {log_error as error, log_info as info, log_warn as warn};

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Where the log is kept. Local, not Roaming like the settings, since it's about this PC.
pub fn path() -> Option<PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(dir).join("Cuota").join("Logs").join("cuota.log"))
}

/// Starts writing to the log at [`path`], and logs crashes there too, since a release build
/// has no console to show them in.
pub fn start() {
    let Some(log) = path().and_then(Log::open) else { return };
    *LOG.lock().unwrap_or_else(PoisonError::into_inner) = Some(log);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        log_error!("Crashed on the {} thread: {info}", thread.name().unwrap_or("unnamed"));
        default_hook(info);
    }));
}

/// Adds a line to the log, stamped with the time. Use [`info!`], [`warn!`] or [`error!`].
pub fn write(level: Level, message: fmt::Arguments) {
    let line = line(&Local::now().format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string(), level, message);
    // Tests would show it among their results, which don't capture writes to stderr.
    if cfg!(debug_assertions) && !cfg!(test) {
        // Not eprint!, which panics when there's no console to write to.
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
    // Nothing panics while this is held, so the panic hook can't find it locked by its own thread.
    if let Some(log) = LOG.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        log.append(&line);
    }
}

/// A line of the log. A message over several lines, like a crash's, has the rest indented.
fn line(time: &str, level: Level, message: fmt::Arguments) -> String {
    let message = message.to_string();
    format!("{time} {:<5} {}\n", level.name(), message.trim_end().replace('\n', "\n    "))
}

/// The open log file, and how big it has grown.
struct Log {
    path: PathBuf,
    file: File,
    size: u64,
}

impl Log {
    fn open(path: PathBuf) -> Option<Self> {
        std::fs::create_dir_all(path.parent()?).ok()?;
        let file = append_to(&path)?;
        let size = file.metadata().map_or(0, |m| m.len());
        Some(Self { path, file, size })
    }

    fn append(&mut self, line: &str) {
        if self.size > 0 && self.size + line.len() as u64 > MAX_SIZE {
            self.start_over();
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.size += line.len() as u64;
        }
    }

    /// Keeps the log so far as the old log, replacing the one before, and starts a new one.
    /// Rust opens files sharing delete access, so this works with the log open, here or in a
    /// second Cuota. If it doesn't, writing carries on, and it's tried again after another
    /// `MAX_SIZE`.
    fn start_over(&mut self) {
        self.size = 0;
        if std::fs::rename(&self.path, old_path(&self.path)).is_ok()
            && let Some(file) = append_to(&self.path)
        {
            self.file = file;
        }
    }
}

fn append_to(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// `cuota.old.log` beside `cuota.log`.
fn old_path(path: &Path) -> PathBuf {
    path.with_extension("old.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_has_the_time_and_level_and_indents_the_rest_of_a_long_message() {
        let line = line("2026-10-14 09:30:00.000 +05:30", Level::Warn, format_args!("Crashed:\nat main.rs\n"));
        assert_eq!(line, "2026-10-14 09:30:00.000 +05:30 WARN  Crashed:\n    at main.rs\n");
    }

    #[test]
    fn a_full_log_becomes_the_old_log() {
        let dir = std::env::temp_dir().join(format!("cuota-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("cuota.log");
        let mut log = Log::open(path.clone()).unwrap();
        let line = format!("{}\n", "x".repeat(1023));
        for _ in 0..MAX_SIZE / 1024 {
            log.append(&line);
        }
        assert!(!old_path(&path).exists(), "the log was moved before it was full");
        log.append("next\n");
        assert_eq!(std::fs::metadata(old_path(&path)).unwrap().len(), MAX_SIZE);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "next\n");

        // Starting again carries on from where the log is.
        drop(log);
        let mut log = Log::open(path.clone()).unwrap();
        log.append("again\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "next\nagain\n");
        drop(log);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
