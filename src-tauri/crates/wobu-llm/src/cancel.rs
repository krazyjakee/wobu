//! Stopping a request that is already costing the user money.
//!
//! This is deliberately the same shape as `wobu_store::Cancel` — a cloneable
//! flag, checked by whoever is doing the work — because the two are meant to be
//! interchangeable in a reader's head, and because the job queue (#49) will hand
//! one of these to every provider call. It is a separate type rather than a
//! dependency on `wobu-store`: this crate talks to the network and that one
//! talks to a filesystem, and pointing one at the other for a flag would be the
//! first edge in a cycle we would then have to argue about.
//!
//! It has one thing the store's version does not: [`Cancel::cancelled`], a
//! future that resolves when the flag is set. A scan can poll a flag between
//! files because a file read finishes on its own. A streaming provider call
//! cannot — between two tokens the socket may be quiet for tens of seconds, and
//! a request we have stopped wanting is still generating tokens the user is
//! billed for. So an adapter races its next read against this future and drops
//! the request the moment it resolves. Polling alone would mean paying for
//! however long the provider decides to think.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::error::{Error, Result};

/// A shared "stop what you are doing" flag.
///
/// Cloneable and cheap on purpose: the copy that gets cancelled lives on the
/// UI thread while the copy being checked is inside a read loop on another one.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<Inner>);

#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    /// Everyone currently parked in [`Cancel::cancelled`], each under the slot
    /// id its future was given. Guarded by a mutex rather than being lock-free
    /// because the flag and this list have to move together: a waiter that
    /// checks the flag, sees `false`, and registers after `cancel` has already
    /// drained the list would sleep forever.
    waiters: Mutex<Vec<(u64, Waker)>>,
    next_slot: AtomicU64,
}

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// Ask everyone holding a clone to stop. Idempotent — the job queue may
    /// cancel a job that has already failed.
    pub fn cancel(&self) {
        let woken = {
            let mut waiters = self.lock_waiters();
            self.0.cancelled.store(true, Ordering::SeqCst);
            std::mem::take(&mut *waiters)
        };
        // Woken outside the lock: a waker is allowed to poll the future again
        // straight away, on this thread, which would deadlock on a re-entrant
        // lock.
        for (_, waker) in woken {
            waker.wake();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// `Err(Error::Cancelled)` if someone asked us to stop.
    ///
    /// For the boundaries an adapter reaches anyway — after a chunk, before the
    /// next request in a retry. [`Cancel::cancelled`] is for the wait in
    /// between, which is where the time actually goes.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(Error::Cancelled) } else { Ok(()) }
    }

    /// A future that resolves once cancelled, and never resolves otherwise.
    ///
    /// Meant to be raced against the next socket read. Resolving with `()`
    /// rather than an error because the adapter's answer to it is not always
    /// `Error::Cancelled` — a provider that has already reported usage still
    /// owes us that usage on the way out.
    pub fn cancelled(&self) -> Cancelled<'_> {
        Cancelled { cancel: self, slot: None }
    }

    /// A poisoned lock means some waiter's `wake` panicked. That is a bug
    /// somewhere else, and refusing to cancel because of it would strand the
    /// user with a paid request they asked to stop, so the poison is stepped
    /// over rather than propagated.
    fn lock_waiters(&self) -> std::sync::MutexGuard<'_, Vec<(u64, Waker)>> {
        self.0.waiters.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The future returned by [`Cancel::cancelled`].
///
/// It holds a slot id rather than relying on `Waker::will_wake` to recognise its
/// own registration. `will_wake` is allowed to answer `false` for two wakers
/// that would in fact wake the same task — the comparison is over pointers that
/// can be duplicated per codegen unit — so using it to decide whether to
/// register would leave one waker per poll behind on some builds and not others.
#[derive(Debug)]
pub struct Cancelled<'a> {
    cancel: &'a Cancel,
    /// Assigned on the first poll. `None` means nothing to clean up, which is
    /// the common case for a future that was built and dropped unpolled.
    slot: Option<u64>,
}

impl Future for Cancelled<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // `Cancelled` holds a reference and an integer, so there is nothing for
        // pinning to protect and nothing that moving it would invalidate.
        let this = self.get_mut();
        let mut waiters = this.cancel.lock_waiters();
        if this.cancel.is_cancelled() {
            return Poll::Ready(());
        }

        match this.slot {
            // A `select!` loop polls this once per streamed chunk, so a second
            // registration per poll would grow the list by one waker per chunk
            // and hold them all until the call ended.
            Some(slot) => match waiters.iter_mut().find(|(id, _)| *id == slot) {
                Some((_, waker)) => {
                    // The task can move between executor threads mid-call, which
                    // is what makes this a refresh rather than a no-op.
                    if !waker.will_wake(cx.waker()) {
                        *waker = cx.waker().clone();
                    }
                }
                None => waiters.push((slot, cx.waker().clone())),
            },
            None => {
                let slot = this.cancel.0.next_slot.fetch_add(1, Ordering::Relaxed);
                waiters.push((slot, cx.waker().clone()));
                this.slot = Some(slot);
            }
        }
        Poll::Pending
    }
}

impl Drop for Cancelled<'_> {
    /// A `select!` that loses the race drops this future and builds a new one on
    /// the next pass. Without this, a `Cancel` that outlives a call — and it
    /// does, the job queue holds one per job — would accumulate a dead waker per
    /// abandoned wait, all held until a cancellation that may never come.
    fn drop(&mut self) {
        if let Some(slot) = self.slot {
            self.cancel.lock_waiters().retain(|(id, _)| *id != slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::stream::testing::block_on;
    #[test]
    fn a_clone_sees_the_cancellation() {
        // The whole point: the flag is set on the UI thread and read on the one
        // holding the socket.
        let a = Cancel::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
        assert!(b.check().is_err());
        assert!(matches!(b.check(), Err(Error::Cancelled)));
    }

    #[test]
    fn a_fresh_token_permits_everything() {
        let c = Cancel::new();
        assert!(c.check().is_ok());
        assert!(!c.is_cancelled());
    }

    #[test]
    fn a_waiter_parked_before_the_cancellation_is_woken_by_it() {
        // The regression this guards is the expensive one: an adapter awaiting
        // the next token of a response nobody wants any more, still being
        // billed, because nothing woke it.
        let cancel = Cancel::new();
        let from_another_thread = cancel.clone();
        std::thread::spawn(move || {
            // Long enough that the main thread is parked inside `poll`, so this
            // exercises the wake path rather than the already-cancelled one.
            std::thread::sleep(std::time::Duration::from_millis(50));
            from_another_thread.cancel();
        });
        block_on(cancel.cancelled());
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn cancelling_first_resolves_immediately_rather_than_parking_forever() {
        // A job cancelled between being queued and being started would otherwise
        // hang on its first await with nobody left to wake it.
        let cancel = Cancel::new();
        cancel.cancel();
        block_on(cancel.cancelled());
    }

    #[test]
    fn re_polling_a_waiter_does_not_grow_the_waker_list() {
        // A streaming call polls this once per chunk. One waker per chunk held
        // until the call ends is a leak that scales with the length of the
        // response — worst on exactly the calls that take longest.
        let cancel = Cancel::new();
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut waiting = std::pin::pin!(cancel.cancelled());
        for _ in 0..8 {
            assert!(waiting.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(cancel.lock_waiters().len(), 1);
    }

    #[test]
    fn abandoning_a_waiter_unregisters_it() {
        // A `select!` that loses the race drops the loser each pass. The token
        // outlives the call, so anything left behind here is held for the life
        // of the job.
        let cancel = Cancel::new();
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        for _ in 0..4 {
            let mut waiting = std::pin::pin!(cancel.cancelled());
            assert!(waiting.as_mut().poll(&mut cx).is_pending());
        }
        assert!(cancel.lock_waiters().is_empty());
    }
}
