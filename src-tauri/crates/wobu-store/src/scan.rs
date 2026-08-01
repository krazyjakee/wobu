//! Watching a long scan, and stopping one.
//!
//! Opening a project on a NAS means several hundred small reads, and SMB
//! round-trips make that take seconds to minutes rather than milliseconds. Two
//! things follow, and they are the difference between "slow" and "broken":
//!
//! The user has to be able to see it moving. A progress count is not decoration
//! here — a stalled mount and a large world look identical from outside, and
//! only a number that stops advancing tells them apart.
//!
//! And the user has to be able to stop it. A share that has gone unresponsive
//! can block a single `read` for the mount's timeout, which on a default SMB
//! mount is minutes. Without a way out, that is an app that has to be killed
//! from a terminal.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

/// How far through a scan we are.
///
/// `total` is the file count from walking the directory, which is one cheap
/// listing even over SMB — it is re-reading the files that is expensive. It can
/// still be wrong by the time the scan finishes if someone else is writing, so
/// treat it as an estimate rather than a promise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub done: usize,
    pub total: usize,
}

impl ScanProgress {
    /// 0–100, saturating. `total == 0` reads as complete rather than as a
    /// division by zero.
    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            return 100;
        }
        ((self.done.min(self.total) * 100) / self.total) as u8
    }
}

/// A shared "stop what you are doing" flag.
///
/// Cloneable and cheap on purpose: the copy that gets cancelled lives on the
/// UI thread while the copy being checked is deep inside a blocking scan on
/// another one.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// `Err(Error::Cancelled)` if someone asked us to stop.
    ///
    /// Checked between files rather than inside a read: a scan is interruptible
    /// at file boundaries, which bounds the wait by one file's latency rather
    /// than by the whole remaining folder.
    pub fn check(&self) -> crate::error::Result<()> {
        if self.is_cancelled() { Err(crate::error::Error::Cancelled) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_saturating_and_never_divides_by_zero() {
        assert_eq!(ScanProgress { done: 0, total: 0 }.percent(), 100);
        assert_eq!(ScanProgress { done: 0, total: 4 }.percent(), 0);
        assert_eq!(ScanProgress { done: 2, total: 4 }.percent(), 50);
        assert_eq!(ScanProgress { done: 4, total: 4 }.percent(), 100);
        // The walk's count can be stale if someone else is writing; reporting
        // 150% would look like a bug to the user and to us.
        assert_eq!(ScanProgress { done: 9, total: 4 }.percent(), 100);
    }

    #[test]
    fn a_clone_sees_the_cancellation() {
        // The whole point: the flag is set on one thread and read on another.
        let a = Cancel::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
        assert!(b.check().is_err());
    }

    #[test]
    fn a_fresh_token_permits_everything() {
        let c = Cancel::new();
        assert!(c.check().is_ok());
        assert!(!c.is_cancelled());
    }
}
