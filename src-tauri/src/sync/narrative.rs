//! Versioned mutable narrative exchange. Only our own acknowledged hashes may
//! permit replacement; wire hashes identify bytes, never authorize overwrites.
use std::collections::{BTreeMap, BTreeSet};

use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};
use wobu_store::{NarrativeApplied, NarrativeIncoming, NarrativeSyncEntry};
use wobu_sync::Session;

use super::{
    bodies,
    manager::{Replica, SyncManager},
    round::Outcome,
};
use crate::error::{Code, CommandResult, WobuError};

const MAX_FILES: usize = 10_000;
const MAX_FRAME: usize = 6 * wobu_store::MAX_NARRATIVE_FILE_BYTES + 4096;
const MAX_TOTAL: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Manifest { version: u32, entries: Vec<NarrativeSyncEntry> },
    Want { entries: Vec<NarrativeSyncEntry> },
    Body { incoming: NarrativeIncoming },
    Missing { entry: NarrativeSyncEntry },
    Ack { rel: String, accepted: bool },
    End { count: usize },
    Finished,
}

pub(super) fn require_capability(supported: bool) -> CommandResult<()> {
    if supported {
        return Ok(());
    }
    Err(WobuError::new(
        Code::Malformed,
        "This peer cannot sync narrative records. Update Wobu on both machines before syncing this project.",
    ))
}

fn permitted(manager: &SyncManager, replica: &Replica) -> CommandResult<()> {
    if !manager.stopping()
        && manager
            .replica(replica.project())
            .is_some_and(|current| std::ptr::eq(current.as_ref(), replica))
    {
        return Ok(());
    }
    Err(WobuError::new(Code::ReadOnly, "Sharing this project was stopped during narrative sync."))
}

pub(super) async fn exchange(
    manager: &SyncManager,
    replica: &Replica,
    session: &Session,
) -> CommandResult<Outcome> {
    permitted(manager, replica)?;
    let peer = session.peer().to_string();
    let entries = replica.with(|p| Ok(p.narrative_manifest()?))?;
    validate_entries(&entries)?;
    let (sent, received) = tokio::try_join!(
        Box::pin(offer(manager, replica, &peer, session.connection(), entries)),
        Box::pin(receive(manager, replica, &peer, session.connection())),
    )?;
    permitted(manager, replica)?;
    let recovered = replica.with(|p| Ok(p.apply_narrative_deletions()?))?;
    Ok(Outcome {
        pushed: sent.pushed,
        served: sent.served,
        applied: received.applied,
        parked: received.parked,
        refused: sent.refused + received.refused,
        changed: received.changed || recovered,
        whole: true,
        ..Outcome::default()
    })
}

async fn offer(
    manager: &SyncManager,
    replica: &Replica,
    peer: &str,
    connection: &Connection,
    entries: Vec<NarrativeSyncEntry>,
) -> CommandResult<Outcome> {
    let (mut send, mut recv) = bodies::open(connection).await?;
    let mut remaining = MAX_TOTAL;
    let offered: BTreeMap<_, _> =
        entries.iter().map(|entry| (entry.rel.clone(), entry.hash.clone())).collect();
    bodies::write_bounded(
        &mut send,
        &Frame::Manifest { version: 1, entries },
        MAX_FRAME,
        &mut remaining,
    )
    .await?;
    let mut lines = bodies::Lines::with_limits(&mut recv, MAX_FRAME, MAX_TOTAL);
    let Some(Frame::Want { entries }) = lines.next().await? else {
        return Err(invalid("expected a wanted-record list"));
    };
    validate_entries(&entries)?;
    if entries.iter().any(|entry| offered.get(&entry.rel) != Some(&entry.hash)) {
        return Err(invalid("requested an unannounced record"));
    }
    let mut outcome = Outcome::default();
    for entry in &entries {
        permitted(manager, replica)?;
        let incoming = replica.with(|p| Ok(p.narrative_outgoing(entry)?))?;
        let present = incoming.is_some();
        let frame = incoming.map_or_else(
            || Frame::Missing { entry: entry.clone() },
            |incoming| Frame::Body { incoming },
        );
        bodies::write_bounded(&mut send, &frame, MAX_FRAME, &mut remaining).await?;
        let Some(Frame::Ack { rel, accepted }) = lines.next().await? else {
            return Err(invalid("expected a record acknowledgement"));
        };
        if rel != entry.rel || (accepted && !present) {
            return Err(invalid("acknowledged a different or missing record"));
        }
        permitted(manager, replica)?;
        if accepted {
            // The hash is paired with this side's announcement, never echoed by
            // the peer. Dropped transfers and conflicts cannot advance the base.
            replica.with(|p| Ok(p.record_narrative_agreed(peer, entry)?))?;
            outcome.pushed += 1;
        } else {
            outcome.refused += 1;
        }
        outcome.served += usize::from(present);
    }
    bodies::write_bounded(
        &mut send,
        &Frame::End { count: entries.len() },
        MAX_FRAME,
        &mut remaining,
    )
    .await?;
    bodies::finish(&mut send)?;
    if !matches!(lines.next::<Frame>().await?, Some(Frame::Finished))
        || lines.next::<Frame>().await?.is_some()
    {
        return Err(invalid("the record exchange did not finish"));
    }
    Ok(outcome)
}

async fn receive(
    manager: &SyncManager,
    replica: &Replica,
    peer: &str,
    connection: &Connection,
) -> CommandResult<Outcome> {
    let (mut send, mut recv) = bodies::within(async {
        connection.accept_bi().await.map_err(|e| {
            WobuError::new(Code::Io, "Narrative sync was interrupted.").with_detail(e.to_string())
        })
    })
    .await?;
    let mut lines = bodies::Lines::with_limits(&mut recv, MAX_FRAME, MAX_TOTAL);
    let Some(Frame::Manifest { version: 1, entries }) = lines.next().await? else {
        return Err(invalid("unsupported or missing narrative manifest"));
    };
    validate_entries(&entries)?;
    permitted(manager, replica)?;
    let wanted = replica.with(|p| {
        let mut wanted = Vec::new();
        for entry in entries {
            if p.narrative_wants(peer, &entry)? {
                wanted.push(entry);
            }
        }
        Ok(wanted)
    })?;
    let mut remaining = MAX_TOTAL;
    bodies::write_bounded(
        &mut send,
        &Frame::Want { entries: wanted.clone() },
        MAX_FRAME,
        &mut remaining,
    )
    .await?;
    let mut outcome = Outcome::default();
    for expected in &wanted {
        let frame = lines
            .next::<Frame>()
            .await?
            .ok_or_else(|| invalid("stopped before all requested records arrived"))?;
        permitted(manager, replica)?;
        let accepted = match frame {
            Frame::Body { incoming }
                if incoming.rel == expected.rel && incoming.hash == expected.hash =>
            {
                match replica.with(|p| Ok(p.apply_narrative_from_peer(peer, &incoming)?))? {
                    NarrativeApplied::Agreed { changed } => {
                        outcome.applied += usize::from(changed);
                        outcome.changed |= changed;
                        true
                    }
                    NarrativeApplied::Conflict { .. } => {
                        outcome.parked += 1;
                        outcome.changed = true;
                        false
                    }
                    NarrativeApplied::Deleted => {
                        outcome.refused += 1;
                        false
                    }
                }
            }
            Frame::Missing { entry } if entry == *expected => {
                outcome.refused += 1;
                false
            }
            _ => return Err(invalid("sent an unexpected record")),
        };
        bodies::write_bounded(
            &mut send,
            &Frame::Ack { rel: expected.rel.clone(), accepted },
            MAX_FRAME,
            &mut remaining,
        )
        .await?;
    }
    if !matches!(lines.next::<Frame>().await?, Some(Frame::End {count}) if count == wanted.len())
        || lines.next::<Frame>().await?.is_some()
    {
        return Err(invalid("missing the complete record count"));
    }
    bodies::write_bounded(&mut send, &Frame::Finished, MAX_FRAME, &mut remaining).await?;
    bodies::finish(&mut send)?;
    Ok(outcome)
}

fn validate_entries(entries: &[NarrativeSyncEntry]) -> CommandResult<()> {
    if entries.len() > MAX_FILES {
        return Err(invalid("too many narrative records"));
    }
    let mut paths = BTreeSet::new();
    for entry in entries {
        wobu_store::project::narrative_sync::validate_entry(entry)?;
        if !paths.insert(&entry.rel) {
            return Err(invalid("duplicate narrative record path"));
        }
    }
    Ok(())
}
fn invalid(reason: &str) -> WobuError {
    WobuError::new(Code::Malformed, format!("Narrative sync could not continue: {reason}."))
}

/// Exercise a malformed sender against the production receive path. This does
/// not bypass any framing, grant or store validation.
#[cfg(test)]
pub(super) async fn fault_exchange(
    manager: &SyncManager,
    replica: &Replica,
    outbound: &Session,
    inbound: &Session,
    incoming: NarrativeIncoming,
    fault: &str,
) {
    let sender = async {
        let (mut send, mut recv) = bodies::open(outbound.connection()).await.unwrap();
        let entry = NarrativeSyncEntry { rel: incoming.rel.clone(), hash: incoming.hash.clone() };
        let mut remaining = MAX_TOTAL;
        bodies::write_bounded(
            &mut send,
            &Frame::Manifest { version: 1, entries: vec![entry] },
            MAX_FRAME,
            &mut remaining,
        )
        .await
        .unwrap();
        let mut lines = bodies::Lines::with_limits(&mut recv, MAX_FRAME, MAX_TOTAL);
        assert!(matches!(lines.next::<Frame>().await.unwrap(), Some(Frame::Want { .. })));
        let mut incoming = incoming;
        if fault == "revoke" {
            manager.unshare(replica.project()).unwrap();
        }
        if fault == "hash" {
            incoming.text.push(' ');
        }
        if fault == "cut" {
            let frame = serde_json::to_vec(&Frame::Body { incoming }).unwrap();
            send.write_all(&frame[..frame.len() / 2]).await.unwrap();
        } else {
            bodies::write_bounded(&mut send, &Frame::Body { incoming }, MAX_FRAME, &mut remaining)
                .await
                .unwrap();
        }
        bodies::finish(&mut send).unwrap();
    };
    let peer = inbound.peer().to_string();
    let receiver = receive(manager, replica, &peer, inbound.connection());
    let (result, ()) = tokio::join!(receiver, sender);
    assert!(result.is_err(), "{fault} unexpectedly completed");
}

/// The hand-driven art-only peer in the node-round regression advertises this
/// capability too. It must complete its empty narrative stage before node IO.
#[cfg(test)]
pub(super) async fn empty_exchange(connection: &Connection) -> CommandResult<()> {
    let send_empty = async {
        let (mut send, mut recv) = bodies::open(connection).await?;
        let mut budget = MAX_TOTAL;
        bodies::write_bounded(
            &mut send,
            &Frame::Manifest { version: 1, entries: vec![] },
            MAX_FRAME,
            &mut budget,
        )
        .await?;
        let mut lines = bodies::Lines::with_limits(&mut recv, MAX_FRAME, MAX_TOTAL);
        assert!(
            matches!(lines.next::<Frame>().await?,Some(Frame::Want{entries}) if entries.is_empty())
        );
        bodies::write_bounded(&mut send, &Frame::End { count: 0 }, MAX_FRAME, &mut budget).await?;
        bodies::finish(&mut send)?;
        assert!(matches!(lines.next::<Frame>().await?, Some(Frame::Finished)));
        assert!(lines.next::<Frame>().await?.is_none());
        CommandResult::Ok(())
    };
    let receive_empty = async {
        let (mut send, mut recv) =
            connection.accept_bi().await.map_err(|e| WobuError::new(Code::Io, e.to_string()))?;
        let mut lines = bodies::Lines::with_limits(&mut recv, MAX_FRAME, MAX_TOTAL);
        assert!(
            matches!(lines.next::<Frame>().await?,Some(Frame::Manifest{version:1,entries}) if entries.is_empty())
        );
        let mut budget = MAX_TOTAL;
        bodies::write_bounded(&mut send, &Frame::Want { entries: vec![] }, MAX_FRAME, &mut budget)
            .await?;
        assert!(matches!(lines.next::<Frame>().await?, Some(Frame::End { count: 0 })));
        assert!(lines.next::<Frame>().await?.is_none());
        bodies::write_bounded(&mut send, &Frame::Finished, MAX_FRAME, &mut budget).await?;
        bodies::finish(&mut send)?;
        CommandResult::Ok(())
    };
    tokio::try_join!(Box::pin(send_empty), Box::pin(receive_empty))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_capability_fails_before_record_stream_and_manifest_is_bounded() {
        assert!(require_capability(false).unwrap_err().message.contains("Update Wobu"));
        require_capability(true).unwrap();
        let valid = NarrativeSyncEntry { rel: "narrative/state.yaml".into(), hash: "a".repeat(64) };
        validate_entries(std::slice::from_ref(&valid)).unwrap();
        assert!(validate_entries(&[valid.clone(), valid.clone()]).is_err());
        assert!(validate_entries(&vec![valid; MAX_FILES + 1]).is_err());
        assert!(
            validate_entries(&[NarrativeSyncEntry {
                rel: "../secrets".into(),
                hash: "a".repeat(64)
            }])
            .is_err()
        );
    }
}
