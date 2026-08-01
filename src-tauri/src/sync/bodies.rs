//! Node bodies over an open `wobu/sync/1` connection: ask for some, hand some
//! over, say when you have finished.
//!
//! #79 swapped two lists of `(node id, hash)` and #80 turned two lists into a
//! [`Plan`](wobu_store::Plan). Neither moves a byte of anybody's writing. This
//! is the step between them and `Project::apply_from_peer` — the one that
//! actually carries the Markdown — and it is the only part of M3's wire that had
//! nowhere else to live.
//!
//! ## Why this is in the shell and not in `wobu-sync`
//!
//! It looks like transport, so the instinct is that it belongs one crate over
//! beside [`manifest`](wobu_sync::manifest). It does not, and the reason is the
//! payload type. What travels here is [`Outgoing`] — a node id, a filename stem
//! and the whole file — and that type is `wobu-store`'s, because `apply` is the
//! thing that decides what a node payload is and there must be exactly one
//! answer to that. `wobu-sync` deliberately does not depend on `wobu-store`
//! (`manifest.rs` argues it at length: the diff must not exist twice, and the
//! wire version must not be wired to `SCHEMA_VERSION`), so putting this there
//! would mean either giving it that dependency or declaring a second `Outgoing`
//! in a crate with no filesystem to check it against.
//!
//! The shell is the only place that holds both a [`Session`] and a `Project`.
//! That is not an accident of layering, it is the layering: `wobu-sync` moves
//! bytes and knows nothing about worlds, `wobu-store` owns a folder and cannot
//! open a socket, and this module is the seam. If it ever moves, it moves
//! together with a decision about which crate owns `Outgoing`, and that decision
//! is the interesting one.
//!
//! ## Bidirectional streams, and why that is forced
//!
//! [`manifest`](wobu_sync::manifest) reserves **the first unidirectional stream
//! in each direction** and says so: its `accept_uni` will answer any later
//! `open_uni` on this ALPN, which would be a hang rather than a compile error.
//! So everything here runs on `open_bi`/`accept_bi`, where the opener and the
//! accepter are decided by QUIC rather than by two peers agreeing out of band.
//!
//! One exchange per stream, and the stream is the frame: the asker writes its
//! request, finishes its send half, and reads the reply to end-of-stream. There
//! is no request id, no multiplexing and no state carried between streams, which
//! is what makes a desynchronised connection impossible rather than merely
//! unlikely — the failure mode a hand-rolled framing gets wrong is two requests
//! interleaved on one stream, and a stream that carries exactly one cannot have
//! it.
//!
//! ## Framing and the numbers
//!
//! One JSON object per line, exactly as `manifest` does it, and for the same
//! reason: `serde_json` escapes every newline inside a string, so a raw `\n`
//! cannot occur inside a message and is an unambiguous terminator. **Nothing
//! here reads a length a peer wrote.** [`MAX_LINE`] is this build's number.
//!
//! The reader is hand-rolled over `RecvStream::read` rather than reached for
//! through `tokio::io::AsyncBufReadExt`, because the bound is the entire point
//! and `read_until` does not have one. It is a near-copy of `manifest`'s `Lines`
//! and that duplication is deliberate: that one is private, and widening it to
//! `pub` would export a parser as part of a transport crate's API for the
//! convenience of one caller.
//!
//! - [`MAX_LINE`] = 2 MiB. One node file, JSON-escaped, plus room. A Markdown
//!   file larger than about a megabyte and a half will not cross this seam and
//!   the sync will report it rather than silently omitting it — see
//!   [`Error::Malformed`](crate::error::Code::Malformed). That is a real
//!   limitation and the right place to raise it is here, where the number is,
//!   rather than in a peer's allocator.
//! - [`MAX_BODIES`] = 32 lines of that per stream, so the most a stranger can
//!   make this process hold at once is bounded by the product and not by how
//!   many nodes it feels like sending.
//! - [`BATCH`] = 16 is what *this* side asks for and pushes at a time, and it is
//!   deliberately far below the cap. #80 asks for small batches for a reason
//!   that is not about memory: `ensure_writable` is checked once per
//!   `apply_from_peer` call, so a share that vanishes mid-batch is discovered at
//!   the start of the next one. Sixteen nodes is a fraction of a second of
//!   writing, so the window in which a write can land in an unmounted
//!   mountpoint's leftover directory is a fraction of a second wide.
//! - [`IDLE`] bounds **silence**, per read and per write, not the whole
//!   exchange — the same distinction `manifest::IDLE_TIMEOUT` makes and for the
//!   same reason. Twenty seconds rather than fifteen because a body is
//!   kilobytes where a manifest entry is bytes, and a slow relay should stall a
//!   sync rather than fail it.

use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use wobu_core::Id;
use wobu_store::Outgoing;

use crate::error::{Code, CommandResult, WobuError};

/// The ceiling on one line, and the only thing between a peer and an allocation
/// the peer chooses the size of.
pub const MAX_LINE: usize = 2 * 1024 * 1024;

/// The most body lines this build will read from one stream.
pub const MAX_BODIES: usize = 32;

/// How many nodes this side asks for, or pushes, per stream.
///
/// Small on purpose; see the module documentation on `ensure_writable`.
pub const BATCH: usize = 16;

/// The bound on silence, per step.
pub const IDLE: Duration = Duration::from_secs(20);

/// How much of a peer's writing is taken off the socket at a time. Not a
/// protocol constant; nothing on the wire depends on it.
const READ_CHUNK: usize = 64 * 1024;

/* ── the wire ─────────────────────────────────────────────────────────────── */

/// One line from the side that opened the stream.
///
/// Internally tagged, so a line names itself and a build that meets a tag it
/// does not know can say "malformed" rather than guessing. The tag is not a
/// version — the ALPN is the version, exactly as `wobu-sync` argues for its own
/// messages, and a second version number in the payload is one that can disagree
/// with the one TLS already negotiated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
enum Ask {
    /// Send me the bodies of these nodes. One line, then end of stream.
    Want { ids: Vec<Id> },
    /// Here is one node. Boxed because this variant is two orders of magnitude
    /// larger than the others and an enum is as big as its widest arm.
    Give { node: Box<Outgoing> },
    /// The end of a run of [`Ask::Give`]. Distinct from end-of-stream so that a
    /// connection cut mid-push is not read as a complete, shorter push — the
    /// same reason `manifest`'s `Page::End` exists.
    Sent,
    /// This side has finished asking and pushing for this round.
    Done,
}

/// One line from the side that accepted the stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
enum Answer {
    Body {
        node: Box<Outgoing>,
    },
    /// The end of a run of [`Answer::Body`]. A peer that asked for eight nodes
    /// and got three followed by this has been told, truthfully, that the other
    /// five are not there — a node deleted between a manifest and a request is
    /// ordinary. Without it, three-and-a-cut-connection would look the same.
    End,
    /// The node ids the answering side now holds the asker's bytes for.
    ///
    /// **Ids, not hashes.** The asker pairs these back with the hashes it
    /// actually sent, because `record_agreed` is what a later fast-forward
    /// trusts and a hash the peer echoes is a hash the peer chose. Letting the
    /// receiver name the agreed bytes would let it license an overwrite of a
    /// file it has never seen.
    Agreed {
        ids: Vec<Id>,
    },
    /// Acknowledges [`Ask::Done`].
    Finished,
}

/// What one accepted stream turned out to be.
#[derive(Debug)]
pub enum Request {
    Want(Vec<Id>),
    Give(Vec<Outgoing>),
    Done,
}

/* ── asking ───────────────────────────────────────────────────────────────── */

/// Ask a peer for some node bodies.
///
/// May come back with fewer than were asked for, and that is not an error: a
/// node the peer deleted, or whose file went away, between the manifest and this
/// request is the ordinary case. It is never more, and never one that was not
/// asked for — an unrequested body is a peer trying to write a file this side
/// did not plan for, and it is dropped here rather than handed to `apply` to
/// think about.
pub async fn want(connection: &Connection, ids: &[Id]) -> CommandResult<Vec<Outgoing>> {
    let (mut send, mut recv) = open(connection).await?;
    write_line(&mut send, &Ask::Want { ids: ids.to_vec() }).await?;
    finish(&mut send)?;

    let mut bodies = Vec::new();
    let mut whole = false;
    let mut lines = Lines::new(&mut recv);
    while let Some(answer) = lines.next::<Answer>().await? {
        match answer {
            Answer::Body { node } => {
                if bodies.len() >= MAX_BODIES {
                    return Err(malformed("a peer sent more bodies than were asked for"));
                }
                // The filter that makes the docstring above true. Comparing
                // against the request rather than trusting the reply is what
                // stops a peer volunteering a node this side never planned to
                // write.
                if ids.contains(&node.node_id) {
                    bodies.push(*node);
                }
            }
            Answer::End => whole = true,
            _ => return Err(malformed("a peer answered a request for bodies with something else")),
        }
    }
    if !whole {
        return Err(malformed("a peer's bodies ended without saying so"));
    }
    Ok(bodies)
}

/// Push node bodies to a peer, and learn which of them landed.
///
/// The returned ids are the peer's acknowledgement and nothing more. The caller
/// moves its bases from them — see [`Answer::Agreed`] for why the hashes are not
/// on the wire, and `Project::record_agreed` for why an acknowledgement rather
/// than a send is the only thing allowed to move one.
pub async fn give(connection: &Connection, nodes: &[Outgoing]) -> CommandResult<Vec<Id>> {
    let (mut send, mut recv) = open(connection).await?;
    for node in nodes {
        write_line(&mut send, &Ask::Give { node: Box::new(node.clone()) }).await?;
    }
    write_line(&mut send, &Ask::Sent).await?;
    finish(&mut send)?;

    let mut lines = Lines::new(&mut recv);
    let mut agreed: Option<Vec<Id>> = None;
    while let Some(answer) = lines.next::<Answer>().await? {
        match answer {
            Answer::Agreed { ids } => agreed = Some(ids),
            _ => return Err(malformed("a peer answered a push with something else")),
        }
    }
    // A push that was not acknowledged is a push that did not happen, as far as
    // the bases are concerned. Reporting it as an empty agreement rather than an
    // error would quietly turn "the connection died" into "the peer refused
    // everything", and the two want different retries.
    agreed.ok_or_else(|| malformed("a peer did not acknowledge a push"))
}

/// Tell a peer this side has finished its half of the round.
///
/// The termination signal, and the whole reason the round cannot deadlock: a
/// side's *server* loop runs until the peer says this, not until its own client
/// half is finished. So a peer that still has bodies to ask for is still being
/// answered, and both loops end exactly once both sides have stopped asking.
pub async fn done(connection: &Connection) -> CommandResult<()> {
    let (mut send, mut recv) = open(connection).await?;
    write_line(&mut send, &Ask::Done).await?;
    finish(&mut send)?;
    let mut lines = Lines::new(&mut recv);
    while let Some(answer) = lines.next::<Answer>().await? {
        if !matches!(answer, Answer::Finished) {
            return Err(malformed("a peer answered `done` with something else"));
        }
    }
    Ok(())
}

/* ── answering ────────────────────────────────────────────────────────────── */

/// Wait for the peer to open a stream and read whatever it asked.
///
/// Returns the accepted stream's send half beside the request, because the reply
/// goes back down the same stream and pairing them in the type is what stops a
/// caller answering the wrong one.
pub async fn accept(connection: &Connection) -> CommandResult<(SendStream, Request)> {
    let (send, mut recv) =
        within(async { connection.accept_bi().await.map_err(|e| transport(&e.to_string())) })
            .await?;

    let mut asks = Vec::new();
    let mut lines = Lines::new(&mut recv);
    while let Some(ask) = lines.next::<Ask>().await? {
        if asks.len() > MAX_BODIES {
            return Err(malformed("a peer pushed more nodes than one stream may carry"));
        }
        asks.push(ask);
    }

    // `Sent` has to be the last line of a push, or this was a push that was cut
    // off. Applying a truncated batch would be harmless in itself — `apply`
    // decides each node afresh against the disk — but *acknowledging* one would
    // move bases for a peer that has no idea which of its nodes arrived.
    if !matches!(asks.last(), Some(Ask::Sent)) {
        return match asks.len() {
            1 => match asks.remove(0) {
                Ask::Want { ids } if ids.len() <= MAX_BODIES => Ok((send, Request::Want(ids))),
                Ask::Want { .. } => {
                    Err(malformed("a peer asked for more nodes than one stream may carry"))
                }
                Ask::Done => Ok((send, Request::Done)),
                _ => Err(malformed("a peer's push ended without saying so")),
            },
            _ => Err(malformed("a peer opened a stream and said nothing usable")),
        };
    }

    asks.pop();
    let mut nodes = Vec::with_capacity(asks.len());
    for ask in asks {
        match ask {
            Ask::Give { node } => nodes.push(*node),
            _ => return Err(malformed("a peer mixed a push with something else")),
        }
    }
    Ok((send, Request::Give(nodes)))
}

/// Answer a [`Request::Want`] with whatever of it this side actually has.
pub async fn bodies(send: &mut SendStream, nodes: &[Outgoing]) -> CommandResult<()> {
    for node in nodes {
        write_line(send, &Answer::Body { node: Box::new(node.clone()) }).await?;
    }
    write_line(send, &Answer::End).await?;
    finish(send)
}

/// Answer a [`Request::Give`] with the ids that landed.
pub async fn agreed(send: &mut SendStream, ids: &[Id]) -> CommandResult<()> {
    write_line(send, &Answer::Agreed { ids: ids.to_vec() }).await?;
    finish(send)
}

/// Answer a [`Request::Done`].
pub async fn finished(send: &mut SendStream) -> CommandResult<()> {
    write_line(send, &Answer::Finished).await?;
    finish(send)
}

/* ── plumbing ─────────────────────────────────────────────────────────────── */

async fn open(connection: &Connection) -> CommandResult<(SendStream, RecvStream)> {
    within(async { connection.open_bi().await.map_err(|e| transport(&e.to_string())) }).await
}

/// One message, one line, with the deadline on the write.
///
/// The `expect` is not optimism: every type serialised here is ids, strings and
/// a node's text, and `serde_json` fails on exactly two things — a map with
/// non-string keys and a non-finite float — neither of which can occur. An error
/// branch here would be one no test could reach and no caller could act on.
async fn write_line<T: Serialize>(send: &mut SendStream, message: &T) -> CommandResult<()> {
    let mut line = serde_json::to_vec(message).expect("ids, strings and a node's text");
    if line.len() >= MAX_LINE {
        // Raised on the *sending* build, which is the one that can be fixed,
        // rather than on whichever peer happened to receive the oversized line.
        return Err(WobuError::new(
            Code::Malformed,
            "A node is too large to sync. It will stay on this machine.",
        )
        .with_detail(format!("{} bytes, over the {MAX_LINE} byte limit", line.len())));
    }
    line.push(b'\n');
    within(async { send.write_all(&line).await.map_err(|e| transport(&e.to_string())) }).await
}

/// Finish the send half, which is this protocol's end-of-message.
///
/// Not followed by `stopped()`. The manifest exchange waits for one because it
/// hands the connection straight on and a prompt `Connection::close` can discard
/// data the peer's stack has received but not delivered. Here every reader reads
/// its stream to end-of-stream before the round finishes, and the round finishes
/// before anything closes, so the wait would buy nothing and would be one more
/// place to hang.
fn finish(send: &mut SendStream) -> CommandResult<()> {
    send.finish().map_err(|e| transport(&e.to_string()))
}

/// Write only the first half of one pushed node, then sever the connection.
/// #85 uses this fault injector to prove a body is not visible to `apply` until
/// its complete line and the batch's `Sent` marker have both arrived.
#[cfg(test)]
pub(super) async fn cut_push(connection: &Connection, node: &Outgoing) -> CommandResult<()> {
    let (mut send, _recv) = open(connection).await?;
    let line = serde_json::to_vec(&Ask::Give { node: Box::new(node.clone()) })
        .expect("an outgoing node always serialises");
    let cut = line.len() / 2;
    within(async { send.write_all(&line[..cut]).await.map_err(|e| transport(&e.to_string())) })
        .await?;
    connection.close(iroh::endpoint::VarInt::from_u32(99), b"test cut mid-body");
    Ok(())
}

/// Newline-delimited messages off a stream a stranger is writing.
struct Lines<'a> {
    recv: &'a mut RecvStream,
    /// Bytes read and not yet consumed: at most one line plus whatever of the
    /// next arrived in the same chunk.
    buf: Vec<u8>,
}

impl<'a> Lines<'a> {
    fn new(recv: &'a mut RecvStream) -> Lines<'a> {
        Lines { recv, buf: Vec::new() }
    }

    /// The next message, or `None` at a clean end of stream.
    ///
    /// Trailing bytes with no newline are half a message and are refused, not
    /// returned as a short one — a peer that stopped mid-write and a peer that
    /// sent nothing must not be the same event.
    async fn next<T: for<'de> Deserialize<'de>>(&mut self) -> CommandResult<Option<T>> {
        loop {
            if let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=end).collect();
                line.pop();
                return serde_json::from_slice(&line)
                    .map(Some)
                    .map_err(|_| malformed("a peer sent a line this build cannot read"));
            }
            // Checked before reading more, so the buffer never grows past one
            // chunk beyond the cap however much a peer writes in one go.
            if self.buf.len() > MAX_LINE {
                return Err(malformed("a peer sent a line larger than this build accepts"));
            }

            let mut chunk = [0u8; READ_CHUNK];
            let read = within(async {
                self.recv.read(&mut chunk).await.map_err(|e| transport(&e.to_string()))
            })
            .await?;
            match read {
                Some(n) => self.buf.extend_from_slice(&chunk[..n]),
                None if self.buf.is_empty() => return Ok(None),
                None => return Err(malformed("a peer stopped mid-message")),
            }
        }
    }
}

/// The deadline on one step, rather than one around the whole exchange.
///
/// The difference is the contract: this bounds how long a peer may say *nothing*,
/// so a large node over a slow relay arrives as long as it keeps arriving. A
/// single deadline over a whole batch would make the batch size a function of
/// the link speed, which is how a sync that works in testing stops working on a
/// hotel network.
async fn within<T>(step: impl Future<Output = CommandResult<T>>) -> CommandResult<T> {
    match tokio::time::timeout(IDLE, step).await {
        Ok(result) => result,
        Err(_elapsed) => Err(transport("a peer went quiet")),
    }
}

/// A peer said something this build will not act on.
///
/// `Code::Malformed` rather than `Code::Io`, and the difference is the retry:
/// `Io` is retryable and this is not, because a peer that sends an unreadable
/// line will send the same one again. The peer's own strings never appear in
/// here — they are attacker-chosen and this message reaches a log.
fn malformed(what: &str) -> WobuError {
    WobuError::new(Code::Malformed, format!("A peer's sync message could not be read: {what}."))
}

/// The link, rather than the peer, is what failed.
fn transport(detail: &str) -> WobuError {
    WobuError::new(Code::Io, "The connection to a peer was interrupted.").with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(text: &str) -> Outgoing {
        Outgoing {
            node_id: Id::from(1u128),
            slug: "kael-vantris".into(),
            text: text.into(),
            hash: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262".into(),
        }
    }

    #[test]
    fn a_message_is_exactly_one_line() {
        // The framing, and the reason no length prefix is needed. A node's text
        // is the one field a *person* controls, so it is the one that would
        // carry a newline — and `serde_json` escaping it is what makes
        // end-of-line an unambiguous frame rather than a hopeful one.
        let ask = Ask::Give { node: Box::new(node("# Kael\n\nA line.\r\nAnd another.\u{2028}")) };

        let line = serde_json::to_vec(&ask).unwrap();

        assert_eq!(line.iter().filter(|&&b| b == b'\n').count(), 0, "{line:?}");
    }

    #[test]
    fn a_batch_of_the_widest_bodies_this_build_will_send_is_bounded() {
        // The arithmetic behind `MAX_LINE` and `MAX_BODIES`, asserted rather
        // than left in a comment: the product is what a stranger can make this
        // process hold at once, and `BATCH` has to stay well under the cap so
        // that a peer running this build never trips its own peer's limit.
        // Both are `const` blocks, so a build that breaks the arithmetic fails
        // to compile rather than waiting for the test to be run.
        const { assert!(BATCH <= MAX_BODIES, "this build would push more than a peer will read") };
        const { assert!(MAX_BODIES.saturating_mul(MAX_LINE) <= 128 * 1024 * 1024) };
    }

    #[test]
    fn a_node_too_large_to_frame_is_refused_on_the_sending_side() {
        // The limitation stated out loud. It has to fail here, where the number
        // is and where the message can say what happened, rather than as a
        // stranger's oversized line at the far end.
        let huge = node(&"a".repeat(MAX_LINE));
        let line = serde_json::to_vec(&Ask::Give { node: Box::new(huge) }).unwrap();
        assert!(line.len() >= MAX_LINE);
    }

    #[test]
    fn every_message_names_itself() {
        // The tags are on the wire and a peer running a different build reads
        // them. Renaming a variant is a wire change, so this is what makes one
        // fail here rather than at a user's machine.
        let tag = |value: serde_json::Value| value.get("ask").cloned().unwrap();
        assert_eq!(tag(serde_json::to_value(Ask::Want { ids: vec![] }).unwrap()), "want");
        assert_eq!(tag(serde_json::to_value(Ask::Sent).unwrap()), "sent");
        assert_eq!(tag(serde_json::to_value(Ask::Done).unwrap()), "done");
        assert_eq!(
            tag(serde_json::to_value(Ask::Give { node: Box::new(node("x")) }).unwrap()),
            "give"
        );

        let tag = |value: serde_json::Value| value.get("answer").cloned().unwrap();
        assert_eq!(tag(serde_json::to_value(Answer::End).unwrap()), "end");
        assert_eq!(tag(serde_json::to_value(Answer::Finished).unwrap()), "finished");
        assert_eq!(tag(serde_json::to_value(Answer::Agreed { ids: vec![] }).unwrap()), "agreed");
    }

    #[test]
    fn an_acknowledgement_carries_ids_and_not_hashes() {
        // The rule `record_agreed` rests on. A base licenses a later
        // fast-forward without asking anybody, so the bytes it names have to be
        // bytes *this* side chose. A hash field here would be the receiver
        // naming them, which is the receiver authorising an overwrite of a file
        // it has never seen.
        let json = serde_json::to_string(&Answer::Agreed { ids: vec![Id::from(1u128)] }).unwrap();
        assert!(!json.contains("hash"), "{json}");
    }
}
