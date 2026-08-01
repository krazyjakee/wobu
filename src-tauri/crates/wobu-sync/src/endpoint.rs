//! Binding, dialling, and the accept loop.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iroh::endpoint::{Connection, VarInt, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode};
use wobu_core::Id;

use crate::blobs::Blobs;
use crate::error::{Error, Result};
use crate::identity::Identity;
use crate::ticket::{Grant, Ticket};
use crate::{ALPN, opening};

/// QUIC application close codes. They are what the *peer* is told; the reason
/// strings beside them are fixed, so nothing a peer reads back from a refusal
/// depends on what this machine holds.
mod close {
    pub(super) const NOT_HELD: u32 = 1;
    pub(super) const MALFORMED: u32 = 2;
    pub(super) const TIMED_OUT: u32 = 3;
    pub(super) const INTERRUPTED: u32 = 4;
    /// The session ended normally. Distinct from the rest so that #83's status
    /// UI can tell "the peer hung up" from "the peer went away".
    pub(super) const DONE: u32 = 0;
}

/// How far this endpoint can be reached from.
///
/// Not a quality setting. It decides which transports exist at all, and the
/// difference between the two is the difference between a program and a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// n0's relays and n0's pkarr/DNS address lookup: the configuration the app
    /// ships with, and the only one that can reach a peer behind a NAT.
    ///
    /// It depends on infrastructure we do not operate. `docs/10-sync-spike.md`
    /// records that as an open posture decision rather than a settled one.
    Internet,
    /// One socket on `127.0.0.1`, no relays, no address lookup.
    ///
    /// This is what makes the crate testable without a network, and it is a
    /// first-class variant rather than a test helper so that "no test in this
    /// crate touches the internet" is a property of the code and not of
    /// everybody remembering. What it therefore cannot exercise is real: NAT
    /// traversal, holepunching, relay selection, and a relay-blocked network are
    /// all invisible to a loopback pair. Those need two machines.
    Loopback,
}

/// How a sync endpoint is set up.
#[derive(Debug, Clone)]
pub struct Config {
    /// The identity this endpoint presents.
    ///
    /// `None` leaves iroh to mint a fresh key on every bind, which is right for
    /// a test that only cares that two endpoints can find each other and wrong
    /// for the app: an endpoint id is a peer's *name*, and a name that changes
    /// on restart is not a name. The app passes [`Identity::load`], which is
    /// keychain-backed and per installation.
    ///
    /// An `Identity` rather than a bare `SecretKey`, and that is the point of
    /// the type: the key is `pub(crate)` behind it, so the only thing in this
    /// workspace that can reach an ed25519 secret is [`Config::bind`] below,
    /// handing it to iroh. A `pub secret_key` field would put one in every
    /// struct that ever holds a `Config`.
    pub identity: Option<Identity>,
    pub reach: Reach,
    /// The project's blob store, if this endpoint is to move file content.
    ///
    /// `None` binds `wobu/sync/1` alone: the opening exchange and the manifest
    /// swap work, and a peer that tries to fetch a blob is refused by TLS for
    /// speaking an ALPN this endpoint does not offer. That is the right shape
    /// for a test of the node half and wrong for the app.
    ///
    /// It is here rather than a parameter of [`SyncEndpoint::bind`] because
    /// iroh's `RouterBuilder` is consumed by `spawn`, so **every ALPN an endpoint
    /// will ever speak has to be known at bind time**. A `SyncEndpoint::with_blobs`
    /// added afterwards is not expressible, and an `Option` that has to be
    /// supplied before the router exists is exactly what a config field is.
    ///
    /// One store per project, and the same one for the whole life of the
    /// endpoint: it is what serves peers *and* what receives from them, and two
    /// would be two answers to "do I have this hash".
    pub blobs: Option<Blobs>,
    /// How long a peer gets to complete the opening exchange.
    ///
    /// Generous enough to survive a relayed round trip on a bad link — the spike
    /// measured ~65 ms RTT over an n0 relay from the UK, and a first-packet
    /// connection setup costs several of those — and short enough that a peer
    /// which connects and says nothing is gone within seconds rather than
    /// holding an accept task until the process ends.
    pub open_timeout: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            identity: None,
            reach: Reach::Internet,
            open_timeout: Duration::from_secs(10),
            blobs: None,
        }
    }
}

impl Config {
    /// The configuration the tests use, and the one to reach for when writing
    /// new ones.
    pub fn loopback() -> Config {
        Config { reach: Reach::Loopback, ..Config::default() }
    }

    async fn bind(&self) -> Result<Endpoint> {
        let builder = match self.reach {
            Reach::Internet => Endpoint::builder(presets::N0),
            // `Minimal` sets the crypto provider and nothing else. The relay
            // mode and the transport clearing are then redundant with it today
            // and stated anyway, because "this endpoint cannot leave the
            // machine" is the property under test and it should not be able to
            // change when a preset's contents change.
            Reach::Loopback => Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .clear_address_lookup()
                .clear_ip_transports()
                .bind_addr("127.0.0.1:0")
                .expect("127.0.0.1:0 is a valid socket address"),
        };
        // The only place in the workspace an ed25519 secret is read out of an
        // `Identity`, and it goes straight into iroh's builder without being
        // stored, formatted or logged on the way.
        let builder = match &self.identity {
            Some(identity) => builder.secret_key(identity.secret_key().clone()),
            None => builder,
        };
        builder.bind().await.map_err(|source| Error::Bind { source })
    }
}

/// Which projects this machine holds.
///
/// The signature is the security boundary, so it is worth reading as one: it
/// takes the project the peer asked about and returns a bool. An implementation
/// is not given a way to return a list, a count, or a near miss, so the accept
/// path physically cannot disclose one — the difference between "I do not have
/// that" and "I do not have that, but I have three others" is not a discipline
/// this crate has to keep, it is a sentence it cannot form.
///
/// Called on the accept path with a project id a stranger chose, so it must be
/// cheap and must not block: a membership test against something already in
/// memory. It is deliberately not `async` for that reason — an implementation
/// that needs to open a database to answer this is an implementation that has
/// handed a stranger a disk seek per dial.
pub trait Projects: Send + Sync + 'static {
    fn holds(&self, project: &Id) -> bool;
}

/// Where accepted sessions go.
///
/// The future returned by [`Sessions::opened`] is the session's life. iroh drops
/// the accepting side of the connection as soon as the accept handler returns,
/// so an implementation either does its work inside this future or moves the
/// [`Session`] somewhere that outlives it — sending it down a channel is enough,
/// because [`Session`] owns a connection handle.
///
/// A trait rather than a channel because #82's `SyncManager` is the real
/// implementation and a channel would force it to exist before anything else
/// could be tested. This is the shape `wobu_jobs::Notify` already has, for the
/// same reason.
#[async_trait]
pub trait Sessions: Send + Sync + 'static {
    async fn opened(&self, session: Session);
}

/// A connection that has said which project it is about.
///
/// Holding one means two things and no more: the peer's endpoint id is
/// cryptographically theirs (QUIC/TLS 1.3 did that, not us), and both sides hold
/// the project named by [`Session::project`]. It does *not* mean the peer is
/// allowed to read that project — see the crate documentation.
#[derive(Debug)]
pub struct Session {
    project: Id,
    connection: Connection,
}

impl Session {
    /// The project both sides agreed this connection is about.
    pub fn project(&self) -> Id {
        self.project
    }

    /// The peer's endpoint id, which is its TLS identity: iroh will not hand
    /// over a connection whose remote id is not the key that signed the
    /// handshake, so this is authenticated without anything here checking it.
    pub fn peer(&self) -> EndpointId {
        self.connection.remote_id()
    }

    /// The live connection, for the protocols that come after this one.
    ///
    /// The opening exchange finished its stream, so every stream opened from
    /// here belongs to whoever opens it, with no framing left over to trip on.
    /// #79's manifest exchange is the caller today.
    ///
    /// #81's blob transfer is deliberately *not*, and the reason is TLS rather
    /// than framing: a connection negotiates one ALPN, and blobs speak
    /// `iroh-blobs`'. [`Session::addr`] is what a second connection to the same
    /// peer is dialled at.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Where this peer is reachable *right now*, as an address that can be
    /// dialled again.
    ///
    /// #81's blob transfer runs on a second ALPN, which means a second
    /// connection, which means an [`EndpointAddr`] — and a bare [`EndpointId`]
    /// is not one: it carries no network path and leaves the address-lookup
    /// service to find one, which under [`Reach::Loopback`] does not exist and
    /// on a bad network is a round trip we already have the answer to.
    ///
    /// Assembled from the connection's live paths, so it says where the peer is
    /// as observed rather than where it said it was. Two things follow. It is a
    /// **snapshot**: a connection that has since upgraded from relay to direct
    /// has different paths, and one taken before holepunching finished may hold
    /// only a relay. And it is only as good as the connection it came from — a
    /// [`Session`] that has died yields an address that no longer resolves, which
    /// is a dial failure and not a wrong answer.
    ///
    /// The id half is not a snapshot of anything: it is the peer's TLS identity,
    /// so a second connection dialled at this address is the same peer or it is
    /// no connection at all.
    pub fn addr(&self) -> EndpointAddr {
        EndpointAddr::from_parts(
            self.peer(),
            self.connection.paths().iter().map(|path| path.remote_addr().clone()),
        )
    }

    /// Whether application data is currently travelling through a relay rather
    /// than directly.
    ///
    /// A snapshot, not a state: a connection holds several paths at once and the
    /// selected one can change mid-session, upgrading from relay to direct once
    /// holepunching lands. #83 wants `Connection::path_events` for that; this is
    /// the cheap answer for a status badge that has to say something now.
    pub fn is_relayed(&self) -> bool {
        self.connection.paths().iter().any(|path| path.is_selected() && path.is_relay())
    }

    /// End the session, telling the peer it was a normal ending.
    pub fn close(self) {
        self.connection.close(VarInt::from_u32(close::DONE), b"done");
    }
}

/// A bound endpoint speaking `wobu/sync/1`.
///
/// Dropping this aborts the accept loop: iroh's `Router` is `#[must_use]` and
/// abandons its task on drop, so a `SyncEndpoint` that goes out of scope takes
/// every inbound connection with it, immediately and without telling the peers.
/// That is the right default — an endpoint outliving the project it syncs is
/// worse — but it means the app has to hold this for as long as sync is meant to
/// work, and should call [`SyncEndpoint::shutdown`] rather than dropping it.
#[derive(Debug, Clone)]
pub struct SyncEndpoint {
    router: Router,
    open_timeout: Duration,
    /// Kept beside the router rather than only inside it, because the handler
    /// iroh holds serves peers and the caller still needs the same store to
    /// *fetch* with. One store, two directions.
    blobs: Option<Blobs>,
}

impl SyncEndpoint {
    /// Bind, and start accepting.
    ///
    /// Runs on the caller's tokio runtime — the one Tauri already started. There
    /// is no runtime built here and there must not be: iroh asks for `tokio ^1`
    /// and the workspace's 1.53.1 satisfies it, so the accept loop, the job
    /// queue and the webview's commands are all tasks on the same executor and
    /// all stop when it does.
    pub async fn bind(
        config: Config,
        projects: Arc<dyn Projects>,
        sessions: Arc<dyn Sessions>,
    ) -> Result<SyncEndpoint> {
        let endpoint = config.bind().await?;
        let open_timeout = config.open_timeout;
        // `spawn` sets the endpoint's ALPN list from the handlers registered
        // here, which is why the builder is not also told about `ALPN`.
        let mut builder =
            Router::builder(endpoint).accept(ALPN, Inbound { projects, sessions, open_timeout });
        // #81's second ALPN, on the same router, the same endpoint, the same key
        // and the same socket. It is a *separate connection* rather than a
        // second protocol on the sync connection, because TLS negotiates one
        // ALPN per connection — see [`crate::blobs`].
        //
        // Note what is not registered beside it: no gate, no authorisation hook,
        // no per-peer filter. A peer that reaches this router has proved its key
        // and nothing else, which is why `Blobs::protocol` passes `None` for
        // `iroh-blobs`' event sender rather than an empty one that looks like a
        // place to put a rule. #90 asked whether there should be a rule and closed
        // `wontfix`; this ALPN is also half of why, because a gate on
        // `wobu/sync/1` would never have covered the connection the bytes actually
        // move on. See `ticket::Grant`.
        if let Some(blobs) = &config.blobs {
            builder = builder.accept(crate::blobs::ALPN, blobs.protocol());
        }
        let router = builder.spawn();
        Ok(SyncEndpoint { router, open_timeout, blobs: config.blobs })
    }

    /// This project's blob store, if one was configured.
    ///
    /// The fetching half of #81. The serving half needs no method — it is inside
    /// the router, answering peers, from the moment [`SyncEndpoint::bind`]
    /// returns.
    pub fn blobs(&self) -> Option<&Blobs> {
        self.blobs.as_ref()
    }

    /// This endpoint's id — its public key, and its name to every peer.
    pub fn id(&self) -> EndpointId {
        self.router.endpoint().id()
    }

    /// The short name a person reads for this endpoint: `amber-heron-4f1a`.
    ///
    /// Taken from the bound endpoint rather than from [`Config::identity`] so
    /// that it is right in the `None` case too — an endpoint whose key iroh
    /// minted has an alias like any other, it just has a different one next
    /// time. Anything *deciding* something still compares [`Self::id`]; see
    /// [`wobu_core::peer`] for why an alias may only ever be displayed.
    pub fn alias(&self) -> String {
        wobu_core::peer::alias(self.id().as_bytes())
    }

    /// Where this endpoint can currently be reached.
    ///
    /// Under [`Reach::Internet`] this is worth nothing until
    /// [`SyncEndpoint::online`] has resolved: before a relay is picked there is
    /// no relay URL in here, and a ticket minted from it (#77) is undialable
    /// from outside the LAN.
    pub fn addr(&self) -> EndpointAddr {
        self.router.endpoint().addr()
    }

    /// Resolves once a relay has been reached, which is when [`Self::addr`]
    /// becomes worth publishing.
    pub async fn online(&self) {
        self.router.endpoint().online().await;
    }

    /// The underlying iroh endpoint.
    ///
    /// Exposed because the protocols stacked on top of this one need it by
    /// reference rather than by wrapper: `iroh-blobs`' downloader takes an
    /// `&Endpoint` (#81), and a second ALPN is registered against the same one.
    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    /// Dial a peer and state which project this connection is about.
    ///
    /// `peer` is anything that becomes an [`EndpointAddr`], including a bare
    /// [`EndpointId`] — which produces an address with no network paths and
    /// leaves the configured address-lookup service to find one. That is the
    /// dial-by-id case, and under [`Reach::Loopback`] there is no lookup
    /// service, so it fails rather than reaching the internet to try.
    ///
    /// Fails with [`Error::ProjectNotHeld`] if the peer does not have the
    /// project. The connection is closed on the way out of every failure here;
    /// a dial that returns an error leaves nothing behind.
    pub async fn connect(&self, peer: impl Into<EndpointAddr>, project: Id) -> Result<Session> {
        let peer = peer.into();
        let id = peer.id;
        let connection = self
            .endpoint()
            .connect(peer, ALPN)
            .await
            .map_err(|source| Error::Dial { peer: id, source })?;

        match opening::within(self.open_timeout, opening::offer(&connection, project)).await {
            Ok(()) => Ok(Session { project, connection }),
            Err(error) => {
                hang_up(&connection, &error);
                Err(error)
            }
        }
    }

    /// Mint a ticket for a project: the string "share this project" copies.
    ///
    /// **Await [`Self::online`] first** under [`Reach::Internet`], and the
    /// ordering is the whole hazard. [`Self::addr`] before a relay has been
    /// picked contains direct socket addresses and nothing else, so the ticket
    /// works on the minting machine's LAN and nowhere on earth beyond it — and
    /// fails as a dial timeout, which reads to a user as "the other person is
    /// offline". [`Ticket::is_relayed`] is the check a share dialog should make
    /// before it lets the string be copied.
    ///
    /// The grant is a parameter, not something minted here: a project shared a
    /// second time must be shared with the *same* grant, or the ticket somebody
    /// pasted into their notes last month is not the ticket this machine would
    /// honour. Keep it with the share; [`Grant::generate`] is for the first time
    /// only. See [`crate::ticket`] for what a grant is and, more importantly,
    /// what it is not.
    pub fn ticket(&self, project: Id, grant: Grant) -> Ticket {
        Ticket::new(project, self.addr(), grant)
    }

    /// Accept a ticket: dial the peer it names about the project it names.
    ///
    /// Exactly [`Self::connect`] with the ticket's two dialable fields pulled
    /// out, and it is a method rather than a line at the call site so that the
    /// next sentence has somewhere to live: **the grant is not sent.** Nothing in
    /// `wobu/sync/1` presents or checks one, so accepting a ticket proves no more
    /// to the other side than dialling with the peer's endpoint id and the project
    /// ULID would. #90 asked whether it should and closed `wontfix` — the pair a
    /// dialler needs is only ever handed out inside a ticket, so a grant check
    /// would refuse nobody who could have got this far. [`crate::ticket::Grant`]
    /// is where that is argued out.
    ///
    /// Fails with [`Error::ProjectNotHeld`] if the peer no longer has the project
    /// — which, since there is no revocation, is the only way a share ever stops
    /// working.
    pub async fn connect_ticket(&self, ticket: &Ticket) -> Result<Session> {
        self.connect(ticket.addr().clone(), ticket.project()).await
    }

    /// Stop accepting, wind the handlers down, and close the endpoint.
    ///
    /// Idempotent, and the thing to call instead of dropping: it stops the
    /// accept loop, winds the handler down, closes the endpoint, and waits for
    /// all of that rather than abandoning it.
    ///
    /// It is not a drain. iroh aborts the in-flight accept futures once the
    /// handler's own shutdown returns, and this handler has nothing to wind
    /// down, so a session mid-transfer is cut. Draining is #82's problem,
    /// because only the thing that owns the sessions knows which of them is
    /// worth waiting for.
    pub async fn shutdown(&self) -> Result<()> {
        self.router.shutdown().await.map_err(|source| Error::Shutdown { source: Box::new(source) })
    }
}

/// The accept side of `wobu/sync/1`.
struct Inbound {
    projects: Arc<dyn Projects>,
    sessions: Arc<dyn Sessions>,
    open_timeout: Duration,
}

/// `ProtocolHandler` requires `Debug`, and this is why it is written by hand
/// rather than derived: deriving it would require `Projects` to be `Debug`, and
/// the obvious implementation of that prints the set of projects this machine
/// holds. iroh logs the handler on some paths. A crate whose one security
/// property is not disclosing the project list should not put the project list
/// one `tracing` filter away from a log file.
impl fmt::Debug for Inbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inbound").field("open_timeout", &self.open_timeout).finish_non_exhaustive()
    }
}

impl ProtocolHandler for Inbound {
    async fn accept(&self, connection: Connection) -> std::result::Result<(), AcceptError> {
        let exchange = opening::answer(&connection, self.projects.as_ref());
        let project = match opening::within(self.open_timeout, exchange).await {
            Ok(project) => project,
            Err(error) => {
                // Closed here with a code rather than returned as an error:
                // returning `Err` drops the connection without sending the peer
                // anything, so a refused peer would see a dead socket and could
                // not tell a rejection from a crash. `Ok` afterwards because
                // refusing a stranger is this handler working, not failing.
                hang_up(&connection, &error);
                return Ok(());
            }
        };

        // Awaited, not spawned: this future *is* the connection's lifetime, and
        // returning early would drop iroh's handle to it.
        self.sessions.opened(Session { project, connection }).await;
        Ok(())
    }
}

/// Close a connection with a code and a fixed reason.
///
/// The reason strings are constants and say only what went wrong with *this*
/// connection. None of them is derived from state, which is what keeps a
/// refusal from being a probe.
fn hang_up(connection: &Connection, error: &Error) {
    let (code, reason): (u32, &[u8]) = match error {
        Error::ProjectNotHeld => (close::NOT_HELD, b"project not held"),
        Error::Malformed => (close::MALFORMED, b"malformed opening message"),
        Error::OpeningTimedOut { .. } => (close::TIMED_OUT, b"opening exchange timed out"),
        _ => (close::INTERRUPTED, b"opening exchange interrupted"),
    };
    connection.close(VarInt::from_u32(code), reason);
}
