//! Sharing a project the way a person does it: mint a string, paste it, dial it.
//!
//! `loopback.rs` proves two endpoints can talk. This proves the *only* thing a
//! user ever holds — a token out of a chat window — is enough to get from nothing
//! to a session. Every test here round-trips the ticket through its string form
//! even where the value is right there, because the string is what travels and a
//! test that passed a `Ticket` by reference would be testing a struct rather than
//! a share.
//!
//! Same caveat as `loopback.rs`, and it bites harder here: [`Reach::Loopback`]
//! has no relay, so every ticket these tests mint carries a direct address and
//! nothing else. A green run says a ticket is sufficient to dial *on one host*.
//! Whether the address inside a real ticket is reachable from another network is
//! a question about relays and holepunching that no single-process test can ask.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iroh::{EndpointAddr, SecretKey};
use tokio::sync::mpsc;
use wobu_core::{Id, new_id};
use wobu_sync::{
    Config, Disposition, Error, Grant, Projects, Session, Sessions, SyncEndpoint, Ticket,
};

struct Held(Vec<Id>);

impl Projects for Held {
    fn holds(&self, project: &Id) -> bool {
        self.0.contains(project)
    }
}

struct Sink(mpsc::UnboundedSender<Session>);

#[async_trait]
impl Sessions for Sink {
    async fn opened(&self, session: Session) {
        let _ = self.0.send(session);
    }
}

async fn peer(held: Vec<Id>) -> (SyncEndpoint, mpsc::UnboundedReceiver<Session>) {
    let (sessions, inbox) = mpsc::unbounded_channel();
    let config = Config { open_timeout: Duration::from_millis(500), ..Config::loopback() };
    let endpoint = SyncEndpoint::bind(config, Arc::new(Held(held)), Arc::new(Sink(sessions)))
        .await
        .expect("a loopback endpoint binds without a network");
    (endpoint, inbox)
}

/// A ticket as it actually arrives: through a string, from somewhere else.
fn pasted(ticket: &Ticket) -> Ticket {
    ticket.to_string().parse().expect("a minted ticket parses back")
}

/// The whole of #77 in one test: a string is a share.
///
/// The sharing side mints, the accepting side is handed nothing but characters,
/// and a session comes out with both sides agreeing which project it is about and
/// who is on the other end. Everything else in this file is a way this can fail.
#[tokio::test]
async fn a_pasted_ticket_is_enough_to_reach_a_session() {
    let project = new_id();
    let (sharing, mut inbox) = peer(vec![project]).await;
    let (accepting, _accepting_inbox) = peer(vec![project]).await;

    let token = sharing.ticket(project, Grant::generate()).to_string();

    let ticket: Ticket = token.parse().unwrap();
    let outbound = accepting.connect_ticket(&ticket).await.unwrap();
    let inbound = inbox.recv().await.unwrap();

    assert_eq!(outbound.project(), project);
    assert_eq!(inbound.project(), project);
    assert_eq!(outbound.peer(), sharing.id(), "the ticket dialled somebody else");
    assert_eq!(ticket.peer(), sharing.id());
    assert_eq!(ticket.alias(), sharing.alias());
}

/// The reason a ticket exists rather than a bare endpoint id.
///
/// `loopback.rs` has the other half of this: on a loopback endpoint there is no
/// address lookup, so an id alone cannot be resolved and the dial fails. The same
/// peer, dialled by ticket, connects — because the ticket brought the address
/// with it. That difference *is* the feature; without it, sharing would need an
/// address lookup service and a round trip to one.
#[tokio::test]
async fn a_ticket_carries_the_address_a_bare_endpoint_id_would_have_to_be_looked_up() {
    let project = new_id();
    let (sharing, _inbox) = peer(vec![project]).await;
    let (accepting, _accepting_inbox) = peer(vec![project]).await;
    let ticket = pasted(&sharing.ticket(project, Grant::generate()));

    let by_id = accepting.connect(sharing.id(), project).await;
    let by_ticket = accepting.connect_ticket(&ticket).await;

    assert!(matches!(by_id, Err(Error::Dial { .. })), "{by_id:?}");
    assert!(by_ticket.is_ok(), "{by_ticket:?}");
    assert!(!ticket.addr().addrs.is_empty(), "a minted ticket named no route at all");
}

/// A loopback ticket has no relay in it, and the type says so.
///
/// Not a loopback quirk — it is what every ticket minted before
/// [`SyncEndpoint::online`] resolves looks like under `Reach::Internet` too, and
/// it is the failure a share dialog has to catch: a string that works perfectly
/// on the LAN it was made on and times out for the person it was sent to, which
/// reads to both of them as "they are offline".
#[tokio::test]
async fn a_ticket_minted_without_a_relay_reports_that_it_has_none() {
    let project = new_id();
    let (sharing, _inbox) = peer(vec![project]).await;

    assert!(!pasted(&sharing.ticket(project, Grant::generate())).is_relayed());
}

/// The only way a share ever stops working, since there is no revocation: the
/// other side stops holding the project. The refusal is the ordinary one, so the
/// UI has one story for "that share is over" rather than a ticket-shaped variant
/// of it.
#[tokio::test]
async fn a_ticket_for_a_project_the_peer_has_dropped_is_refused_like_any_other_dial() {
    let project = new_id();
    let (sharing, mut inbox) = peer(vec![]).await;
    let (accepting, _accepting_inbox) = peer(vec![project]).await;

    let ticket = pasted(&sharing.ticket(project, Grant::generate()));
    let refused = accepting.connect_ticket(&ticket).await;

    assert!(matches!(refused, Err(Error::ProjectNotHeld)), "{refused:?}");
    assert!(inbox.try_recv().is_err(), "a refused ticket reached the application anyway");
}

/// **The grant is not checked, this is the test that says so out loud, and it is
/// a decision rather than an unfinished job.**
///
/// Two peers reach a session while holding completely different grants for the
/// same project. That is the truth and it must not be assumed away: nothing in
/// `wobu/sync/1` presents a grant, because checking one is authorisation and
/// `Projects::holds` takes one project and returns one bool, with nowhere to put
/// a second input.
///
/// #90 asked whether that should change and closed `wontfix`, so this test is not
/// a placeholder waiting for a feature to invert it. `wobu_sync::ticket::Grant`
/// carries the whole argument; the part that matters when reading *this* file is
/// that the forgery below is not an attack anybody could actually mount. It needs
/// `issued.addr()` — the sharing peer's ed25519 endpoint id — and the project
/// ULID, and the only artefact in the system that puts those two together is a
/// ticket, which carries the real grant beside them. The test can forge one
/// because it minted the endpoint in the line above; a stranger cannot.
///
/// So a ticket grants exactly what knowing a project ULID and a peer's endpoint
/// id grants, the UI must not imply more, and a change that made this test fail
/// would be reopening a closed question rather than finishing an open one.
#[tokio::test]
async fn a_grant_is_not_checked_and_that_is_deliberate() {
    let project = new_id();
    let (sharing, _inbox) = peer(vec![project]).await;
    let (accepting, _accepting_inbox) = peer(vec![project]).await;
    let issued = sharing.ticket(project, Grant::generate());

    // A ticket for the same peer and the same project, with a grant nobody ever
    // issued. If a grant meant anything on the wire, this would be refused.
    let forged = Ticket::new(project, issued.addr().clone(), Grant::generate());
    assert_ne!(forged.grant(), issued.grant());

    let session = accepting.connect_ticket(&pasted(&forged)).await;

    assert!(session.is_ok(), "{session:?}");
}

/// The other half of the same decision, and the reason the forgery above is not
/// an attack: **a grant is not the unguessable thing in a ticket, the pair of the
/// other two fields is.**
///
/// A peer who reads a project ULID off a `project.json` on a NAS learns one of
/// the two things a dial needs and not the other. Give them the *harder* half as
/// well — the sharing machine's socket, which is what an address is — and the
/// dial still fails, because iroh will not hand over a connection to a machine
/// presenting a different key than the one asked for. It fails at TLS, before
/// `wobu/sync/1` exists to have an opinion, which is where a grant check would
/// have sat.
///
/// Pinned here because the argument in `ticket::Grant` rests on it, and prose is
/// not a test.
#[tokio::test]
async fn a_project_ulid_and_a_route_without_the_peers_endpoint_id_reach_nothing() {
    let project = new_id();
    let (sharing, mut inbox) = peer(vec![project]).await;
    let (guesser, _guesser_inbox) = peer(vec![project]).await;

    // The sharing peer's real socket, with an endpoint id that is not theirs.
    // That is the most a project folder could ever disclose: the ULID is in
    // `project.json` and a machine's address is observable, but the ed25519 key
    // lives in a keychain and only a ticket carries it beside the ULID.
    let stranger = EndpointAddr::from_parts(
        SecretKey::generate().public(),
        sharing.addr().addrs.iter().cloned(),
    );
    let dialled = guesser.connect(stranger, project).await;

    assert!(dialled.is_err(), "a dial with the wrong endpoint id succeeded: {dialled:?}");
    assert!(inbox.try_recv().is_err(), "a guessed ULID reached the sharing peer anyway");

    // And the same peer, dialled by ticket, connects — so the refusal above is
    // about the identity and not about the endpoint being unreachable.
    let honest = pasted(&sharing.ticket(project, Grant::generate()));
    assert!(guesser.connect_ticket(&honest).await.is_ok());
}

/// Accepting a ticket for a project already on this machine joins the replica
/// that is here. Two folders holding one ULID would be two replicas syncing
/// against each other on one disk, which is not a share.
#[tokio::test]
async fn a_ticket_for_a_project_this_machine_already_holds_joins_rather_than_cloning() {
    let project = new_id();
    let (sharing, _inbox) = peer(vec![project]).await;
    let ticket = pasted(&sharing.ticket(project, Grant::generate()));

    assert_eq!(ticket.disposition(&Held(vec![project])), Disposition::Join);
    assert_eq!(ticket.disposition(&Held(vec![new_id()])), Disposition::Clone);
    assert_eq!(ticket.disposition(&Held(vec![])), Disposition::Clone);
}

/// A ticket is worth keeping, so the shell has to be able to write one down. The
/// serialised form is the token and nothing but the token — the same characters
/// that were pasted — so a file in app data and a message in a chat client hold
/// the same string, and a user can copy one out of the other.
#[tokio::test]
async fn a_ticket_written_down_and_read_back_is_the_same_share() {
    let project = new_id();
    let (sharing, _inbox) = peer(vec![project]).await;
    let (accepting, _accepting_inbox) = peer(vec![project]).await;
    let ticket = sharing.ticket(project, Grant::generate());

    // What the shell would put in app data: a list of tickets as JSON.
    let kept = serde_json::to_string(&vec![ticket.clone()]).unwrap();
    assert_eq!(kept, format!("[\"{ticket}\"]"), "the stored form is not the pasted form");

    let restored: Vec<Ticket> = serde_json::from_str(&kept).unwrap();
    let session = accepting.connect_ticket(&restored[0]).await;

    assert!(session.is_ok(), "{session:?}");
}
