use std::time::Duration;

use iroh::EndpointId;
use iroh::endpoint::{BindError, ConnectError};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// What can go wrong between wanting to sync a project with a peer and having a
/// connection we are willing to use.
///
/// Everything here is about *reaching* a peer and agreeing which project is
/// being talked about. Nothing here is about trust, because nothing in this
/// crate decides trust — see the crate documentation for why there is no
/// `BadSignature` or `AuthFailed` variant and why adding one would be a
/// mistake rather than an omission.
#[derive(Debug, Error)]
pub enum Error {
    #[error("could not bind a sync endpoint on this machine")]
    Bind {
        #[source]
        source: BindError,
    },

    /// The peer could not be reached at all: no route, no relay, wrong ALPN,
    /// or a local endpoint that has already been shut down.
    ///
    /// Carries the peer's id because a desktop app syncing with several peers
    /// has to be able to say *which one* went quiet, and the id is public — it
    /// is the peer's TLS identity, exchanged in the clear as part of every
    /// connection attempt.
    #[error("could not open a sync connection to {peer}")]
    Dial {
        peer: EndpointId,
        #[source]
        source: ConnectError,
    },

    /// The other side does not hold the project this connection asked about.
    ///
    /// **This variant carries nothing, deliberately.** It is the answer both to
    /// "you asked for a project I have never seen" and to "you asked for a
    /// project I have never seen, though I hold four others", and those two must
    /// be indistinguishable. A field here — a count, a list, a nearest match —
    /// would turn a dial with a guessed ULID into a way to enumerate somebody's
    /// worlds. See [`crate::Projects`], whose signature is what keeps this
    /// honest at the type level.
    #[error("the peer does not hold this project")]
    ProjectNotHeld,

    /// The opening exchange did not finish in time.
    ///
    /// A peer that connects and then says nothing must not be able to hold an
    /// accept task, a connection and a file descriptor open indefinitely, and a
    /// dial to a peer that accepts and then stalls must not hang the caller. The
    /// same bound covers both directions.
    #[error("the peer did not finish opening the connection within {within:?}")]
    OpeningTimedOut { within: Duration },

    /// Bytes arrived on the opening stream that are not a `wobu/sync/1` opening
    /// message.
    ///
    /// No detail, and not because detail is unavailable: an error message
    /// quoting what the peer sent is a log line an unauthenticated dialler
    /// controls the contents of.
    #[error("the peer's opening message is not wobu/sync/1")]
    Malformed,

    /// The connection or the opening stream failed underneath us.
    ///
    /// The source is boxed because the five calls that make up the opening
    /// exchange fail with five unrelated iroh error types — opening a stream,
    /// writing, finishing, reading, and waiting for the read to be
    /// acknowledged. Five variants for "the connection died, at one of five
    /// points, all of which mean try again later" would be five ways to say the
    /// same thing to the same caller.
    #[error("the connection failed during the opening exchange")]
    Interrupted {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The accept loop did not wind down cleanly, which in practice means a
    /// protocol handler panicked and the panic is being carried out here.
    #[error("the sync endpoint did not shut down cleanly")]
    Shutdown {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl Error {
    pub(crate) fn interrupted<E: std::error::Error + Send + Sync + 'static>(source: E) -> Error {
        Error::Interrupted { source: Box::new(source) }
    }
}
