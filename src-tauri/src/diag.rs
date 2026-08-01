//! The diagnostic log.
//!
//! Wobu collects no telemetry and is not going to, so when something goes wrong
//! the only account of it is on the user's own disk. This module is that
//! account: a small rolling file they can look at, and then hand over.
//!
//! Three properties are load-bearing, in descending order of how bad it is to
//! get them wrong.
//!
//! **It is never written inside a project folder.** A project is a folder on a
//! share that other people can read, and one of the things this file exists to
//! record is provider failures. Putting it next to the Markdown would publish a
//! user's diagnostics to everyone with the path, and sync it to every machine
//! that mounts the share. It goes in local app data beside the index, which is
//! already the convention for everything derived and machine-local.
//!
//! **Every line is redacted on the way out.** [`write_line`] is the only place
//! that touches the file, and it calls [`redact::scrub`] unconditionally — the
//! same function `WobuError::new` uses. So "remember to redact before logging"
//! is not a rule anyone has to keep; there is no code path that can skip it.
//! This is the whole reason logging is a module and not a `writeln!` at each
//! call site.
//!
//! **It is bounded.** Two files, 2 MiB each, and it is the *newest* 2 MiB that
//! survives. A log that grows without limit gets deleted by the user long
//! before the bug they want to report happens.
//!
//! No `tracing`, no `log`: this needs a level filter, a size cap and one
//! rename, all of which are below the cost of the dependency, and the shell
//! deliberately has a small tree.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::redact;

/// Per file. Two of them, so the worst case on disk is twice this.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

const CURRENT: &str = "wobu.log";
const PREVIOUS: &str = "wobu.1.log";

/// Where the chosen level is remembered between runs. Deliberately one bare
/// word in a text file rather than JSON: the most likely reason to change it is
/// someone being talked through a bug report, and "open this file and type
/// debug" survives that conversation better than a settings schema.
const LEVEL_FILE: &str = "level";

/// Overrides the stored level for one run, without persisting. For reproducing
/// something once at `debug` without leaving the app noisy afterwards.
const LEVEL_ENV: &str = "WOBU_LOG";

/// How much noise reaches the file.
///
/// Ordered least to most, so a record is written when the configured level is
/// at least as verbose as the record's own. `Off` sorts below everything and so
/// admits nothing — including errors, which is the point of having it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Off,
    Error,
    Warn,
    /// The default. A log that has to be switched on before it records anything
    /// is empty exactly when it is needed — the user has already hit the bug by
    /// the time they think to look.
    #[default]
    Info,
    Debug,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Off => "OFF",
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }

    fn as_u8(self) -> u8 {
        self as u8
    }

    fn from_u8(v: u8) -> Level {
        match v {
            0 => Level::Off,
            1 => Level::Error,
            2 => Level::Warn,
            4 => Level::Debug,
            _ => Level::Info,
        }
    }
}

impl FromStr for Level {
    type Err = ();

    fn from_str(s: &str) -> Result<Level, ()> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "silent" => Ok(Level::Off),
            "error" => Ok(Level::Error),
            "warn" | "warning" => Ok(Level::Warn),
            "info" => Ok(Level::Info),
            "debug" | "trace" => Ok(Level::Debug),
            _ => Err(()),
        }
    }
}

struct Sink {
    file: File,
    written: u64,
}

/// One log. Owns its directory, its level and its open handle.
///
/// Constructible standalone rather than only as a global, so the rotation and
/// redaction behaviour can be tested against a temporary directory instead of
/// whatever the developer's real app data happens to contain.
pub struct Diagnostics {
    dir: PathBuf,
    level: AtomicU8,
    /// `None` until the first record. A session that logs nothing leaves no
    /// file behind, so an empty `wobu.log` always means "logging is off or
    /// broken" rather than "nothing happened".
    sink: Mutex<Option<Sink>>,
}

impl Diagnostics {
    pub fn new(dir: PathBuf) -> Diagnostics {
        let level = read_level(&dir);
        Diagnostics { dir, level: AtomicU8::new(level.as_u8()), sink: Mutex::new(None) }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(CURRENT)
    }

    pub fn level(&self) -> Level {
        Level::from_u8(self.level.load(Ordering::Relaxed))
    }

    /// Changes the level for this run and remembers it for the next one.
    ///
    /// Dropping to a quieter level closes nothing and truncates nothing: what
    /// has already been recorded is what the user is about to send, and
    /// silently shortening it would be the opposite of a diagnostic.
    pub fn set_level(&self, level: Level) {
        self.level.store(level.as_u8(), Ordering::Relaxed);
        let _ = fs::create_dir_all(&self.dir);
        let _ = fs::write(self.dir.join(LEVEL_FILE), format!("{}\n", level.label().to_lowercase()));
    }

    pub fn record(&self, level: Level, message: &str) {
        if level == Level::Off || level > self.level() {
            return;
        }
        let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        self.write_line(&format!("{stamp} {:<5} {message}", level.label()));
    }

    /// The only path to the file. Redaction and newline-escaping live here so
    /// that no caller can be the one that forgot.
    fn write_line(&self, line: &str) {
        // A message carrying a newline would otherwise forge log records — one
        // entry becomes two, the second with no timestamp and no level. Folding
        // it to a literal `\n` keeps one record per line, which is what makes
        // the file greppable and the tail below correct.
        let safe = redact::scrub(line).replace('\r', "\\r").replace('\n', "\\n");

        let Ok(mut guard) = self.sink.lock() else {
            return;
        };
        if guard.is_none() {
            *guard = self.open();
        }
        let Some(sink) = guard.as_mut() else {
            return;
        };

        let bytes = safe.len() as u64 + 1;
        if sink.written + bytes > MAX_BYTES {
            // Rotate *before* writing, so a record is never split across files.
            if let Some(fresh) = self.rotate() {
                *sink = fresh;
            }
        }

        // A failed write is dropped on purpose. A full disk or a revoked
        // permission is not a reason to fail the user's actual work, and there
        // is nowhere to report it to that would not have the same problem.
        if writeln!(sink.file, "{safe}").is_ok() {
            sink.written += bytes;
        }
    }

    fn open(&self) -> Option<Sink> {
        fs::create_dir_all(&self.dir).ok()?;
        let path = self.path();
        let file = OpenOptions::new().create(true).append(true).open(&path).ok()?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        Some(Sink { file, written })
    }

    /// Current becomes previous, previous is discarded.
    fn rotate(&self) -> Option<Sink> {
        let _ = fs::rename(self.path(), self.dir.join(PREVIOUS));
        self.open()
    }

    /// The last `max_lines` records, oldest first.
    ///
    /// This exists so the user can see what they are about to hand over before
    /// they hand it over. "Trust us, it's redacted" is not something a person
    /// pasting a file into a public issue tracker should have to take on faith.
    pub fn tail(&self, max_lines: usize) -> String {
        let Ok(text) = fs::read_to_string(self.path()) else {
            return String::new();
        };
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(max_lines);
        lines[start..].join("\n")
    }
}

fn read_level(dir: &Path) -> Level {
    if let Ok(raw) = std::env::var(LEVEL_ENV)
        && let Ok(level) = raw.parse()
    {
        return level;
    }
    fs::read_to_string(dir.join(LEVEL_FILE)).ok().and_then(|s| s.parse().ok()).unwrap_or_default()
}

/* ── the process-wide log ─────────────────────────────────────────────────── */

static DIAG: OnceLock<Diagnostics> = OnceLock::new();

/// Installed once at startup. Records made before this are dropped rather than
/// buffered — there is nothing worth keeping from before the log has a home.
pub fn init(dir: PathBuf) {
    let _ = DIAG.set(Diagnostics::new(dir));
}

pub fn global() -> Option<&'static Diagnostics> {
    DIAG.get()
}

pub fn record(level: Level, message: impl AsRef<str>) {
    if let Some(d) = DIAG.get() {
        d.record(level, message.as_ref());
    }
}

pub fn error(message: impl AsRef<str>) {
    record(Level::Error, message);
}

pub fn info(message: impl AsRef<str>) {
    record(Level::Info, message);
}

/// Where the log lives, whether or not it has been created yet.
pub fn dir() -> PathBuf {
    wobu_store::paths::app_data_dir().join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A private directory per test. `tempfile` is not a dependency and adding
    /// one to a shared manifest for four assertions is not worth it.
    fn scratch(name: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("wobu-diag-{}-{name}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn body(d: &Diagnostics) -> String {
        fs::read_to_string(d.path()).unwrap_or_default()
    }

    #[test]
    fn nothing_is_created_until_something_is_recorded() {
        let dir = scratch("lazy");
        let d = Diagnostics::new(dir.clone());
        assert!(!d.path().exists(), "opening the log must not create it");

        d.record(Level::Error, "now there is something to say");
        assert!(d.path().exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_quieter_level_drops_the_noisier_records() {
        let dir = scratch("levels");
        let d = Diagnostics::new(dir.clone());
        d.set_level(Level::Warn);

        d.record(Level::Error, "kept-error");
        d.record(Level::Warn, "kept-warn");
        d.record(Level::Info, "dropped-info");
        d.record(Level::Debug, "dropped-debug");

        let text = body(&d);
        assert!(text.contains("kept-error"));
        assert!(text.contains("kept-warn"));
        assert!(!text.contains("dropped-info"));
        assert!(!text.contains("dropped-debug"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn off_admits_nothing_at_all_including_errors() {
        let dir = scratch("off");
        let d = Diagnostics::new(dir.clone());
        d.set_level(Level::Off);
        d.record(Level::Error, "should not appear");
        assert_eq!(body(&d), "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_level_survives_a_restart() {
        let dir = scratch("persist");
        Diagnostics::new(dir.clone()).set_level(Level::Debug);
        assert_eq!(Diagnostics::new(dir.clone()).level(), Level::Debug);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_never_reaches_the_disk() {
        // The property the whole module exists for. If this ever fails, the log
        // is a credential leak with a filename.
        let dir = scratch("redact");
        let d = Diagnostics::new(dir.clone());
        d.record(Level::Error, "POST failed, Authorization: Bearer sk-ant-abcdef0123456789");
        d.record(Level::Error, r#"config was {"api_key": "wobbly-secret-value-here"}"#);

        let text = body(&d);
        assert!(!text.contains("sk-ant-abcdef0123456789"), "issuer-prefixed key on disk: {text}");
        assert!(!text.contains("wobbly-secret-value-here"), "assigned secret on disk: {text}");
        assert!(text.contains(redact::MASK));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_message_cannot_forge_extra_records() {
        let dir = scratch("newline");
        let d = Diagnostics::new(dir.clone());
        d.record(Level::Error, "first line\nnot actually a second record");

        assert_eq!(body(&d).lines().count(), 1);
        assert!(body(&d).contains("\\n"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_is_capped_and_it_is_the_newest_that_survives() {
        let dir = scratch("rotate");
        let d = Diagnostics::new(dir.clone());

        // ~1 KiB per record, so this comfortably crosses 2 MiB.
        let filler = "x".repeat(1000);
        for i in 0..3000 {
            d.record(Level::Info, &format!("record-{i} {filler}"));
        }

        let current = fs::metadata(d.path()).unwrap().len();
        assert!(current <= MAX_BYTES, "current file exceeded the cap: {current}");
        assert!(dir.join(PREVIOUS).exists(), "the previous file should have been kept");

        // The most recent record must be in the live file, not rotated away.
        assert!(body(&d).contains("record-2999"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_returns_the_end_of_the_file() {
        let dir = scratch("tail");
        let d = Diagnostics::new(dir.clone());
        for i in 0..50 {
            d.record(Level::Info, &format!("line-{i}"));
        }

        let tail = d.tail(5);
        assert_eq!(tail.lines().count(), 5);
        assert!(tail.contains("line-49"));
        assert!(!tail.contains("line-44"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_of_a_log_that_does_not_exist_is_empty_not_an_error() {
        let dir = scratch("tail-missing");
        assert_eq!(Diagnostics::new(dir).tail(10), "");
    }

    #[test]
    fn the_log_is_never_inside_a_project_folder() {
        // Stated as a test because it is the one mistake that would publish a
        // user's diagnostics to everyone who can mount the share.
        let d = dir();
        assert!(d.starts_with(wobu_store::paths::app_data_dir()));
        assert!(d.ends_with("logs"));
    }

    #[test]
    fn levels_parse_from_the_spellings_a_person_would_type() {
        assert_eq!("Debug".parse(), Ok(Level::Debug));
        assert_eq!("  warning ".parse(), Ok(Level::Warn));
        assert_eq!("off".parse(), Ok(Level::Off));
        assert_eq!("".parse::<Level>(), Err(()));
    }
}
