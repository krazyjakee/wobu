//! Test-only helpers shared across the adapters.

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

/// A one-thread executor, so an adapter's async surface is exercised without a
/// runtime. `wobu-imagine` names none — it runs on Tauri's — and pulling tokio
/// in to prove that would undo the claim.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    struct Unparker(std::thread::Thread);
    impl Wake for Unparker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Unparker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park();
    }
}
