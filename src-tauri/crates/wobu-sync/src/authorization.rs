//! The ticket grant, on a stream of its own.
//!
//! The project offer in [`crate::opening`] is already part of the
//! `wobu/sync/1` wire format and must not grow a field. A grant therefore
//! travels on a second, unidirectional QUIC stream. The acceptor reads the
//! project from the original opening stream first, reads this presentation
//! second, and answers on the original stream only after
//! [`crate::Projects::admits`] has reduced both inputs to one bool.

use iroh::endpoint::Connection;

use crate::error::{Error, Result};
use crate::ticket::Grant;

const NONE: u8 = 0;
const SOME: u8 = 1;
const GRANT_BYTES: usize = 32;
const MAX_PRESENTATION_BYTES: usize = 1 + GRANT_BYTES;

/// Present a grant, or state explicitly that this dial did not come from a
/// ticket. One tag byte keeps `None` different from a truncated `Some`.
pub(super) async fn present(connection: &Connection, grant: Option<&Grant>) -> Result<()> {
    let mut send = connection.open_uni().await.map_err(Error::interrupted)?;
    match grant {
        None => send.write_all(&[NONE]).await.map_err(Error::interrupted)?,
        Some(grant) => {
            send.write_all(&[SOME]).await.map_err(Error::interrupted)?;
            send.write_all(grant.as_bytes()).await.map_err(Error::interrupted)?;
        }
    }
    send.finish().map_err(Error::interrupted)?;
    Ok(())
}

/// Receive the presentation from the stream which follows the opening offer.
/// Every malformed shape is one undifferentiated malformed opening; no supplied
/// credential is ever copied into an error.
pub(super) async fn receive(connection: &Connection) -> Result<Option<Grant>> {
    let mut recv = connection.accept_uni().await.map_err(Error::interrupted)?;
    let bytes = recv.read_to_end(MAX_PRESENTATION_BYTES).await.map_err(Error::interrupted)?;
    match bytes.as_slice() {
        [NONE] => Ok(None),
        [SOME, bytes @ ..] if bytes.len() == GRANT_BYTES => {
            let mut grant = [0; GRANT_BYTES];
            grant.copy_from_slice(bytes);
            Ok(Some(Grant::from_bytes(grant)))
        }
        _ => Err(Error::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_presentation_has_one_and_only_one_shape_for_each_case() {
        assert_eq!(MAX_PRESENTATION_BYTES, 33);
        assert_ne!(NONE, SOME);

        let grant = Grant::from_bytes([7; GRANT_BYTES]);
        let mut some = vec![SOME];
        some.extend_from_slice(grant.as_bytes());
        assert_eq!(some.len(), MAX_PRESENTATION_BYTES);
    }
}
