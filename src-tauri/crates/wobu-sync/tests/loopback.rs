//! Two endpoints, one process, one loopback interface.
//!
//! Everything here runs against real iroh: real QUIC, real TLS, real ALPN
//! negotiation, real streams. The only thing faked is which projects a machine
//! holds and where accepted sessions go, because those are the two seams the app
//! fills in.
//!
//! What these tests therefore do *not* cover, and what a green run must not be
//! read as covering: NAT traversal, holepunching, relay selection, or a network
//! where the relay is blocked. `Reach::Loopback` has no relay and no address
//! lookup precisely so that no test here can quietly start depending on n0's
//! infrastructure — `a_loopback_endpoint_cannot_look_a_peer_up_at_all` is the
//! test that keeps that true.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use wobu_core::{Id, new_id};
use wobu_sync::{ALPN, Config, Error, Identity, Projects, Session, Sessions, SyncEndpoint};

/// The projects one machine holds.
struct Held(Vec<Id>);

impl Projects for Held {
    fn admits(&self, project: &Id, _grant: Option<&wobu_sync::Grant>) -> bool {
        self.0.contains(project)
    }
}

/// Where a peer's accepted sessions arrive.
struct Sink(mpsc::UnboundedSender<Session>);

#[async_trait]
impl Sessions for Sink {
    async fn opened(&self, session: Session) {
        // Sending moves the session, and with it the connection handle, out of
        // the accept future. That is what keeps the connection alive after this
        // returns; an implementation that dropped the session would close it.
        let _ = self.0.send(session);
    }
}

/// One peer: an endpoint holding `held`, and the queue its inbound sessions
/// land in.
async fn peer(held: Vec<Id>) -> (SyncEndpoint, mpsc::UnboundedReceiver<Session>) {
    bind(held, None).await
}

/// `peer`, with the identity chosen. `None` leaves iroh to mint one per bind,
/// which is what every test but the identity one wants: they need two endpoints
/// that are not each other, and do not care which.
async fn bind(
    held: Vec<Id>,
    identity: Option<Identity>,
) -> (SyncEndpoint, mpsc::UnboundedReceiver<Session>) {
    let (sessions, inbox) = mpsc::unbounded_channel();
    let config = Config {
        identity,
        // Short, because every wait in these tests is a loopback round trip and
        // one test deliberately waits the whole timeout out.
        open_timeout: Duration::from_millis(500),
        ..Config::loopback()
    };
    let endpoint = SyncEndpoint::bind(config, Arc::new(Held(held)), Arc::new(Sink(sessions)))
        .await
        .expect("a loopback endpoint binds without a network");
    (endpoint, inbox)
}

/// The happy path end to end, and the two facts a session is allowed to assert:
/// which project it is about, and who is on the other end. The peer ids are
/// checked in both directions because they are TLS identities rather than
/// anything either side told the other, and a session reporting the wrong one
/// would make every later authorisation decision meaningless.
#[tokio::test]
async fn a_dial_naming_a_project_both_sides_hold_reaches_a_session() {
    let project = new_id();
    let (accepting, mut inbox) = peer(vec![project]).await;
    let (dialling, _dialling_inbox) = peer(vec![project]).await;

    let outbound = dialling.connect(accepting.addr(), project).await.unwrap();
    let inbound = inbox.recv().await.unwrap();

    assert_eq!(outbound.project(), project);
    assert_eq!(inbound.project(), project);
    assert_eq!(outbound.peer(), accepting.id());
    assert_eq!(inbound.peer(), dialling.id());
    assert!(!outbound.is_relayed(), "a loopback pair has no relay path to select");
}

/// Guards the seam #79 and #81 are built on. The opening exchange finishes its
/// stream, so the next stream either side opens carries nothing left over from
/// it — if the opening message were ever framed into a stream meant to be
/// reused, this is what would break first.
#[tokio::test]
async fn a_stream_opened_after_the_exchange_carries_only_what_is_written_to_it() {
    let project = new_id();
    let (accepting, mut inbox) = peer(vec![project]).await;
    let (dialling, _dialling_inbox) = peer(vec![project]).await;

    let outbound = dialling.connect(accepting.addr(), project).await.unwrap();
    let inbound = inbox.recv().await.unwrap();

    let (mut send, _recv) = outbound.connection().open_bi().await.unwrap();
    send.write_all(b"manifest").await.unwrap();
    send.finish().unwrap();

    let (_send, mut recv) = inbound.connection().accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"manifest");
}

/// The refusal path. A peer that names a project we have never seen is turned
/// away, and nothing reaches the application: a session handed on for a project
/// this machine does not hold would be a connection with nothing to sync and no
/// rule about what it may read.
#[tokio::test]
async fn a_project_the_peer_does_not_hold_is_refused_and_opens_no_session() {
    let theirs = new_id();
    let mine = new_id();
    let (accepting, mut inbox) = peer(vec![theirs]).await;
    let (dialling, _dialling_inbox) = peer(vec![mine]).await;

    let refused = dialling.connect(accepting.addr(), mine).await;

    assert!(matches!(refused, Err(Error::ProjectNotHeld)), "{refused:?}");
    assert!(inbox.try_recv().is_err(), "a refused dial reached the application anyway");
}

/// The disclosure regression. A dialler guessing ULIDs must learn whether *its*
/// guess was right and nothing else — not how many other projects are here, not
/// what they are called. Everything the refused peer can see is checked against
/// everything it must not: the error text, its debug form, and the ids
/// themselves.
#[tokio::test]
async fn a_refusal_names_nothing_else_the_accepting_side_holds() {
    let mine = vec![new_id(), new_id(), new_id()];
    let guess = new_id();
    let (accepting, _inbox) = peer(mine.clone()).await;
    let (dialling, _dialling_inbox) = peer(vec![guess]).await;

    let refused = dialling.connect(accepting.addr(), guess).await.unwrap_err();

    let told = format!("{refused} {refused:?}");
    for project in &mine {
        assert!(!told.contains(&project.to_string()), "the refusal named {project}");
    }
    assert_eq!(refused.to_string(), "the peer does not hold this project");
}

/// A peer that negotiates the ALPN and then says nothing must be hung up on. It
/// is the shape of a stalled peer and of a port scan, and without the deadline
/// each one costs an accept task and a connection until the process ends.
#[tokio::test]
async fn a_peer_that_connects_and_never_states_a_project_is_hung_up_on() {
    let project = new_id();
    let (accepting, mut inbox) = peer(vec![project]).await;

    let silent = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_address_lookup()
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .bind()
        .await
        .unwrap();
    let connection = silent.connect(accepting.addr(), ALPN).await.unwrap();

    let closed = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;

    assert!(closed.is_ok(), "the accepting side held a silent connection open indefinitely");
    assert!(inbox.try_recv().is_err());
}

/// Shutdown has to be a stop, not a detach. After it, the endpoint is closed and
/// a dial fails immediately rather than waiting out a connection timeout on a
/// socket nobody is listening to — a hang at quit is the failure mode iroh's
/// abort-on-drop `Router` makes easy to write.
#[tokio::test]
async fn a_shut_down_endpoint_stops_accepting_and_refuses_to_dial() {
    let project = new_id();
    let (accepting, _inbox) = peer(vec![project]).await;
    let (dialling, _dialling_inbox) = peer(vec![project]).await;
    let addr = accepting.addr();

    dialling.connect(addr.clone(), project).await.unwrap().close();
    dialling.shutdown().await.unwrap();
    dialling.shutdown().await.expect("shutdown is idempotent");

    let after = dialling.connect(addr, project).await;

    assert!(matches!(after, Err(Error::Dial { .. })), "{after:?}");
}

/// Keeps the test rig honest. `Reach::Loopback` has no address lookup, so a bare
/// endpoint id — which carries no network path — cannot be resolved. If this
/// ever starts succeeding, the suite has acquired a dependency on n0's DNS and
/// relays and is no longer a test that runs without a network.
#[tokio::test]
async fn a_loopback_endpoint_cannot_look_a_peer_up_at_all() {
    let project = new_id();
    let (accepting, _inbox) = peer(vec![project]).await;
    let (dialling, _dialling_inbox) = peer(vec![project]).await;

    let bare = dialling.connect(accepting.id(), project).await;

    assert!(matches!(bare, Err(Error::Dial { .. })), "{bare:?}");
}

/// The whole of #76, against real TLS rather than against our own struct.
///
/// The claim is not that `Identity` remembers a key — a unit test covers that —
/// but that the key it remembers is the one the *peer* ends up looking at. A
/// bind that quietly ignored `Config::identity`, or an iroh that derived its
/// endpoint id from something else, would pass every test in `identity.rs` and
/// still leave a collaborator seeing a different person after every restart.
#[tokio::test]
async fn an_identity_is_the_same_peer_across_binds_and_is_what_the_other_side_sees() {
    let project = new_id();
    let identity = Identity::ephemeral();

    let (first, _first_inbox) = bind(vec![project], Some(identity.clone())).await;
    let addr = first.addr();
    assert_eq!(first.id(), identity.id(), "the bound endpoint is not the identity's key");
    assert_eq!(first.alias(), identity.alias());

    // The other side's view, which is the one that matters: `Session::peer` is
    // iroh's output from the TLS handshake, not anything this side claimed.
    let (dialling, _dialling_inbox) = peer(vec![project]).await;
    let session = dialling.connect(addr, project).await.expect("both sides hold the project");
    assert_eq!(session.peer(), identity.id());
    session.close();
    first.shutdown().await.unwrap();

    // And again, on a fresh endpoint — the restart this exists to survive.
    let (second, _second_inbox) = bind(vec![project], Some(identity.clone())).await;
    assert_eq!(second.id(), identity.id(), "the same identity bound as a different peer");
    assert_eq!(second.alias(), identity.alias());
}

/// The behaviour #76 replaces, stated so it cannot come back by accident: with
/// no identity, iroh mints a key per bind and the endpoint is a stranger every
/// time. Fine for the tests above, which only need two endpoints that are not
/// each other; fatal for an app, where an endpoint id is a person's name.
#[tokio::test]
async fn no_identity_still_means_a_new_peer_on_every_bind() {
    let project = new_id();

    let (first, _first_inbox) = bind(vec![project], None).await;
    let (second, _second_inbox) = bind(vec![project], None).await;

    assert_ne!(first.id(), second.id());
}
