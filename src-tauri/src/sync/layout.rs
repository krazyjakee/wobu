//! Optional cosmetic stage after core sync. A layout problem cannot turn a
//! successfully transferred story into a failed source/asset exchange.
use super::{
    bodies,
    manager::{Replica, SyncManager},
};
use crate::error::{Code, CommandResult, WobuError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use wobu_store::{LayoutManifest, LayoutOffer, LayoutSave};
use wobu_sync::Session;

const MAX_FILES: usize = 2000;
const FRAME: usize = 6 * wobu_store::narrative::layout::MAX_LAYOUT_BYTES + 4096;
const TOTAL: usize = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(tag = "layout", rename_all = "snake_case", deny_unknown_fields)]
enum Message {
    Manifest { version: u32, manifest: LayoutManifest },
    Wanted { entries: Vec<LayoutOffer> },
    Content { entry: LayoutOffer, text: Option<String> },
    Applied { rel: String, notice: Option<String> },
    Complete { count: usize },
    Finished,
}
fn bad() -> WobuError {
    WobuError::new(
        Code::Malformed,
        "A peer sent an unsupported or incomplete Flow arrangement exchange.",
    )
}
fn permission(manager: &SyncManager, replica: &Replica) -> CommandResult<()> {
    if manager.stopping()
        || !manager
            .replica(replica.project())
            .is_some_and(|current| std::ptr::eq(current.as_ref(), replica))
    {
        return Err(WobuError::new(
            Code::ReadOnly,
            "Sharing stopped before arrangements could be transferred.",
        ));
    }
    Ok(())
}
fn validate(manifest: &LayoutManifest) -> CommandResult<()> {
    if manifest.entries.len() > MAX_FILES
        || manifest.notices.len() > 20
        || manifest.notices.iter().any(|n| n.len() > 4096)
    {
        return Err(bad());
    }
    let mut found = BTreeSet::new();
    for entry in &manifest.entries {
        entry.validate()?;
        if !found.insert(&entry.rel) {
            return Err(bad());
        }
    }
    Ok(())
}

/// Freeze exact offered bytes before either direction can merge into local files.
/// The independent wire budget still accounts for JSON escaping and framing.
pub(super) struct Snapshot {
    manifest: LayoutManifest,
    bodies: BTreeMap<String, String>,
}
impl Snapshot {
    pub(super) fn capture(project: &wobu_store::Project) -> CommandResult<Self> {
        Self::capture_with_budget(project, TOTAL)
    }
    fn capture_with_budget(
        project: &wobu_store::Project,
        mut remaining: usize,
    ) -> CommandResult<Self> {
        let mut manifest = project.layout_manifest();
        validate(&manifest)?;
        let mut bodies = BTreeMap::new();
        let offered = std::mem::take(&mut manifest.entries);
        for entry in offered {
            if let Some(text) = project.layout_outgoing(&entry)? {
                remaining = remaining.checked_sub(text.len()).ok_or_else(|| {
                    WobuError::new(
                        Code::Malformed,
                        "Flow arrangements exceed the snapshot size limit.",
                    )
                })?;
                bodies.insert(entry.rel.clone(), text);
                manifest.entries.push(entry);
            } else if manifest.notices.len() < 20 {
                manifest.notices.push(
                    "An arrangement changed while preparing the transfer; sync again to share it."
                        .into(),
                );
            }
        }
        Ok(Self { manifest, bodies })
    }
}

pub(super) async fn exchange(
    manager: &SyncManager,
    replica: &Replica,
    session: &Session,
) -> CommandResult<(bool, Option<String>)> {
    permission(manager, replica)?;
    let local = replica.with(|p| Snapshot::capture(p))?;
    let (sent, received) = tokio::try_join!(
        Box::pin(offer(manager, replica, session, local)),
        Box::pin(receive(manager, replica, session))
    )?;
    let notice = sent.or(received.1);
    Ok((received.0, notice))
}
pub(super) async fn offer(
    manager: &SyncManager,
    replica: &Replica,
    session: &Session,
    snapshot: Snapshot,
) -> CommandResult<Option<String>> {
    let Snapshot { manifest, mut bodies } = snapshot;
    let announced = manifest.entries.clone();
    let mut notice = manifest.notices.first().cloned();
    let (mut send, mut recv) = bodies::open(session.connection()).await?;
    let mut budget = TOTAL;
    bodies::write_bounded(
        &mut send,
        &Message::Manifest { version: 1, manifest },
        FRAME,
        &mut budget,
    )
    .await?;
    let mut lines = bodies::Lines::with_limits(&mut recv, FRAME, TOTAL);
    let Some(Message::Wanted { entries }) = lines.next().await? else { return Err(bad()) };
    validate(&LayoutManifest { entries: entries.clone(), notices: vec![] })?;
    if entries.iter().any(|e| !announced.contains(e)) {
        return Err(bad());
    }
    for entry in &entries {
        permission(manager, replica)?;
        let text = bodies.remove(&entry.rel).ok_or_else(bad)?;
        bodies::write_bounded(
            &mut send,
            &Message::Content { entry: entry.clone(), text: Some(text) },
            FRAME,
            &mut budget,
        )
        .await?;
        let Some(Message::Applied { rel, notice: response }) = lines.next().await? else {
            return Err(bad());
        };
        if rel != entry.rel || response.as_ref().is_some_and(|n| n.len() > 4096) {
            return Err(bad());
        }
        if notice.is_none() {
            notice = response;
        }
    }
    bodies::write_bounded(
        &mut send,
        &Message::Complete { count: entries.len() },
        FRAME,
        &mut budget,
    )
    .await?;
    bodies::finish(&mut send)?;
    if !matches!(lines.next::<Message>().await?, Some(Message::Finished))
        || lines.next::<Message>().await?.is_some()
    {
        return Err(bad());
    }
    Ok(notice)
}
pub(super) async fn receive(
    manager: &SyncManager,
    replica: &Replica,
    session: &Session,
) -> CommandResult<(bool, Option<String>)> {
    let (mut send, mut recv) = bodies::within(async {
        session.connection().accept_bi().await.map_err(|e| WobuError::new(Code::Io, e.to_string()))
    })
    .await?;
    let mut lines = bodies::Lines::with_limits(&mut recv, FRAME, TOTAL);
    let Some(Message::Manifest { version: 1, manifest }) = lines.next().await? else {
        return Err(bad());
    };
    validate(&manifest)?;
    permission(manager, replica)?;
    let entries = replica.with(|p| {
        Ok(manifest
            .entries
            .into_iter()
            .filter(|entry| p.layout_outgoing(entry).ok().flatten().is_none())
            .collect::<Vec<_>>())
    })?;
    let mut budget = TOTAL;
    let mut changed = false;
    let mut notice = manifest.notices.into_iter().next();
    bodies::write_bounded(
        &mut send,
        &Message::Wanted { entries: entries.clone() },
        FRAME,
        &mut budget,
    )
    .await?;
    for expected in &entries {
        let Some(Message::Content { entry, text }) = lines.next().await? else { return Err(bad()) };
        if &entry != expected {
            return Err(bad());
        }
        permission(manager, replica)?;
        let diagnostic = match text {
            None => {
                Some("An arrangement changed during transfer; sync again to receive it.".into())
            }
            Some(text) => match replica.with(|p| {
                let before = wobu_store::narrative::layout::read_raw(p.root(), &entry.rel)
                    .ok()
                    .flatten()
                    .map(|(_, stamp)| stamp.hash);
                let result = p.apply_layout_from_peer(&entry, &text)?;
                Ok((result, before))
            }) {
                Ok((LayoutSave::Written(stamp), before)) => {
                    changed |= before.as_ref() != Some(&stamp.hash);
                    None
                }
                Ok((LayoutSave::Deferred { .. }, _)) => Some(
                    "A newer arrangement format was preserved; update Wobu to share it.".into(),
                ),
                Err(error) => Some(format!("An arrangement was kept locally: {}", error.message)),
            },
        };
        if notice.is_none() {
            notice = diagnostic.clone()
        }
        bodies::write_bounded(
            &mut send,
            &Message::Applied { rel: entry.rel, notice: diagnostic },
            FRAME,
            &mut budget,
        )
        .await?;
    }
    if !matches!(lines.next::<Message>().await?,Some(Message::Complete{count}) if count==entries.len())
        || lines.next::<Message>().await?.is_some()
    {
        return Err(bad());
    }
    bodies::write_bounded(&mut send, &Message::Finished, FRAME, &mut budget).await?;
    bodies::finish(&mut send)?;
    // This is the final stream in the round; ensure the acknowledgement was
    // consumed before either side can close its session.
    bodies::within(async {
        send.stopped().await.map(|_| ()).map_err(|e| WobuError::new(Code::Io, e.to_string()))
    })
    .await?;
    Ok((changed, notice))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_bounds_total_bytes_without_changing_layout_files() {
        let home =
            std::env::temp_dir().join(format!("wobu-layout-snapshot-{}", wobu_core::new_id()));
        std::fs::create_dir_all(&home).unwrap();
        let project = wobu_store::Project::create(&home, "Ashfall").unwrap();
        for arc in ["first", "second"] {
            let layout = wobu_store::Layout::empty(wobu_store::GraphKey::Arc { arc: arc.into() });
            wobu_store::narrative::layout::save(project.root(), "test", &layout, None).unwrap();
        }
        let before = project.layout_manifest().entries;
        let total =
            before.iter().map(|entry| project.layout_outgoing(entry).unwrap().unwrap().len()).sum();
        let exact = Snapshot::capture_with_budget(&project, total).unwrap();
        assert_eq!(exact.manifest.entries, before);
        assert_eq!(exact.bodies.values().map(String::len).sum::<usize>(), total);
        assert!(Snapshot::capture_with_budget(&project, total - 1).is_err());
        assert_eq!(project.layout_manifest().entries, before);
        let index = project.index_path();
        drop(project);
        std::fs::remove_dir_all(home).unwrap();
        let _ = std::fs::remove_file(index);
    }
}
