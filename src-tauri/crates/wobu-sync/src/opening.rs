//! The opening exchange: which project is this connection about.
//!
//! One message each way on the original bidirectional stream. The dialler states
//! a project ULID; the accepting side answers whether it admits that project.
//! #90 adds no field to either message: the optional ticket grant travels on a
//! second, unidirectional stream between the offer and the answer.
//!
//! ## Why the stream is finished rather than framed
//!
//! Neither side sends a length prefix. The dialler writes its offer and calls
//! `finish()`, so end-of-stream *is* the frame, and the reader uses
//! [`MAX_OPENING_BYTES`] as its ceiling rather than trusting a length the peer
//! wrote. A length prefix would add a second thing to disagree about and a
//! number an unauthenticated dialler chooses; QUIC already delimits streams for
//! free. The cost is that this stream is single-use, which is the intent —
//! see the note on later streams in [`crate`].
//!
//! ## Why the answer waits to be acknowledged
//!
//! [`answer`] does not return until the peer has acknowledged the reply.
//! `Connection::close` is allowed to discard stream data the remote QUIC stack
//! has received but not yet delivered, so closing straight after writing a
//! refusal can turn "I do not hold that project" into a bare connection close —
//! which is indistinguishable from the network dropping, and which a peer would
//! reasonably retry forever.

use std::future::Future;
use std::time::Duration;

use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};
use wobu_core::Id;

use crate::Projects;
use crate::authorization;
use crate::error::{Error, Result};
use crate::ticket::Grant;

/// The ceiling on either message of the opening exchange.
///
/// An offer is one ULID and an answer is one word, so this is roughly ten times
/// what the exchange needs. It exists because `read_to_end` on a stream a
/// stranger is writing needs a bound, and the bound is the only thing standing
/// between a dial and an allocation the dialler picks the size of.
pub(crate) const MAX_OPENING_BYTES: usize = 256;

/// What the dialler says.
///
/// The ALPN is the version. `wobu/sync/1` is negotiated in the TLS handshake
/// before a byte of this is written, so a second version number in the payload
/// would be a version that can disagree with the one already agreed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Offer {
    pub project: Id,
}

/// What the accepting side says back.
///
/// Two variants and no fields, and the emptiness is the point: there is nothing
/// in this type for a refusal to disclose. [`Answer::NotHeld`] serialises to the
/// same bytes on a machine holding no projects and on a machine holding a
/// hundred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub(crate) enum Answer {
    Held,
    NotHeld,
}

/// The dialling half: state a project, present the optional grant separately,
/// and find out whether the peer admits the pair.
pub(crate) async fn offer(
    connection: &Connection,
    project: Id,
    grant: Option<&Grant>,
) -> Result<()> {
    let (mut send, mut recv) = connection.open_bi().await.map_err(Error::interrupted)?;

    let offer = serde_json::to_vec(&Offer { project })
        .expect("an Offer is one ULID and serde_json cannot fail on it");
    send.write_all(&offer).await.map_err(Error::interrupted)?;
    send.finish().map_err(Error::interrupted)?;

    // A second stream, deliberately. The bytes above are the complete and
    // unchanged `wobu/sync/1` opening message; a grant must never become a
    // field an older peer would read as part of it.
    authorization::present(connection, grant).await?;

    let answer = recv.read_to_end(MAX_OPENING_BYTES).await.map_err(Error::interrupted)?;
    match serde_json::from_slice::<Answer>(&answer) {
        Ok(Answer::Held) => Ok(()),
        Ok(Answer::NotHeld) => Err(Error::ProjectNotHeld),
        Err(_) => Err(Error::Malformed),
    }
}

/// The accepting half: read the project the peer wants and answer for it.
///
/// `Err(`[`Error::ProjectNotHeld`]`)` here means *we* refused, where the same
/// variant out of [`offer`] means the peer refused us. One variant rather than
/// two because it is one fact — this pair does not have a project in common —
/// and the caller already knows which side of the connection it is on.
pub(crate) async fn answer(connection: &Connection, projects: &dyn Projects) -> Result<Id> {
    let (mut send, mut recv) = connection.accept_bi().await.map_err(Error::interrupted)?;

    let offer = recv.read_to_end(MAX_OPENING_BYTES).await.map_err(Error::interrupted)?;
    let offer: Offer = serde_json::from_slice(&offer).map_err(|_| Error::Malformed)?;
    let grant = authorization::receive(connection).await?;

    // Asked once, for one project and the one optional grant presented beside
    // it. Everything after this line sees only the bool, so an unknown project
    // and a wrong grant necessarily take the same refusal path.
    let admitted = projects.admits(&offer.project, grant.as_ref());

    let answer = if admitted { Answer::Held } else { Answer::NotHeld };
    let answer = serde_json::to_vec(&answer).expect("an Answer is a unit variant");
    send.write_all(&answer).await.map_err(Error::interrupted)?;
    send.finish().map_err(Error::interrupted)?;
    send.stopped().await.map_err(Error::interrupted)?;

    if admitted { Ok(offer.project) } else { Err(Error::ProjectNotHeld) }
}

/// Put a deadline on one half of the exchange.
///
/// Both directions get the same bound, from the same config value, because both
/// are waiting on a peer that may simply never speak — and an unbounded await
/// on a stranger is how a shutdown turns into a hang.
pub(crate) async fn within<T>(
    within: Duration,
    exchange: impl Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(within, exchange).await {
        Ok(result) => result,
        Err(_elapsed) => Err(Error::OpeningTimedOut { within }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the disclosure this crate exists to avoid: if a refusal ever grows
    /// a field, the bytes change and this fails. "I do not have that" and "I do
    /// not have that, but here are four I do" must be the same answer on the
    /// wire, and the wire is what a peer sees.
    #[test]
    fn a_refusal_is_the_same_bytes_no_matter_what_else_is_held() {
        let refusal = serde_json::to_vec(&Answer::NotHeld).unwrap();
        assert_eq!(refusal, br#"{"answer":"not_held"}"#);
    }

    /// The offer is the only place a project id crosses the wire before a
    /// session exists, so its encoding is load-bearing: a ULID that round-trips
    /// as anything but itself would make every project a peer names a project
    /// we do not hold.
    #[test]
    fn an_offer_round_trips_through_its_wire_form() {
        let project = Id::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let bytes = serde_json::to_vec(&Offer { project }).unwrap();
        assert_eq!(bytes, br#"{"project":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}"#);
        assert_eq!(serde_json::from_slice::<Offer>(&bytes).unwrap().project, project);
    }
}
