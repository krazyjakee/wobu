use chrono::{SecondsFormat, Utc};
use wobu_core::Id;
use wobu_sync::Ticket;

use super::*;
use crate::sync::clone;
use crate::sync::{ProjectSyncStatus, SyncPeerStatus, SyncPhase};

impl SyncManager {
    /// Catch-up snapshots for a webview that mounted after an event fired.
    pub fn project_statuses(&self) -> Vec<ProjectSyncStatus> {
        let shares = self.shares();
        let runtime = self.runtime.lock();
        shares
            .into_iter()
            .map(|share| {
                let mut snapshot = runtime.get(&share.project).map_or_else(
                    || ProjectSyncStatus {
                        project: share.project,
                        state: SyncPhase::Idle,
                        peers: Vec::new(),
                        arriving: false,
                    },
                    |status| status.snapshot(share.project),
                );

                snapshot.arriving = clone::still_arriving(&share.root);
                Self::add_known_peers(&mut snapshot, share.peers);
                snapshot
            })
            .collect()
    }

    /// Announce the catch-up shape after the manager is installed in
    /// `SyncState`. In particular this emits `idle`: a webview may have queried
    /// while endpoint binding was still in flight, and silence would leave that
    /// truthful-but-temporary "not running" answer on screen forever.
    pub fn announce(&self) {
        for status in self.project_statuses() {
            self.wake.sync_state(status);
        }
    }

    pub(super) fn announce_project(&self, project: Id) {
        if let Some(status) = self.project_statuses().into_iter().find(|s| s.project == project) {
            self.wake.sync_state(status);
        }
    }

    fn add_known_peers(snapshot: &mut ProjectSyncStatus, tickets: Vec<Ticket>) {
        // A joined project knows its outbound peers before the first dial. Put
        // them in status as disconnected so "offline" can name who was tried
        // rather than looking like no setup.
        for ticket in tickets {
            let endpoint_id = ticket.peer().to_string();
            if !snapshot.peers.iter().any(|peer| peer.endpoint_id == endpoint_id) {
                snapshot.peers.push(SyncPeerStatus {
                    endpoint_id,
                    alias: ticket.alias(),
                    connected: false,
                    last_converged_at: None,
                });
            }
        }
        snapshot.peers.sort_by(|a, b| a.alias.cmp(&b.alias));
    }

    pub(super) fn with_known_peers(&self, mut snapshot: ProjectSyncStatus) -> ProjectSyncStatus {
        if let Some(share) = self.shares.lock().get(snapshot.project).cloned() {
            // One `is_file` on a local path, per status emission rather than per
            // node, and it is read here rather than cached because the thing it
            // describes is a file another part of this process deletes.
            snapshot.arriving = clone::still_arriving(&share.root);
            Self::add_known_peers(&mut snapshot, share.peers);
        }
        snapshot
    }

    /// The first round finished: this replica is no longer half a world.
    ///
    /// Removing the marker is what makes that durable, and announcing is what
    /// takes "still arriving" off the status bar without waiting for the next
    /// phase change — a round that converges and then finds nothing to do would
    /// otherwise leave the sentence up until somebody edited something.
    pub(in crate::sync) fn mark_arrived(&self, project: Id, root: &std::path::Path) {
        let marker = clone::clone_marker(root);
        if !marker.is_file() {
            return;
        }
        match std::fs::remove_file(&marker) {
            Ok(()) => diag::info(format!("sync: {project} finished arriving")),
            // Left for the next converged round rather than propagated: the
            // world is complete either way, and failing a round that succeeded
            // over a note this machine keeps for itself would be the tail
            // wagging the dog.
            Err(error) => {
                diag::error(format!("sync: could not clear the clone marker: {error}"));
                return;
            }
        }
        self.announce_project(project);
    }

    pub(super) fn set_phase(&self, project: Id, state: SyncPhase) {
        let snapshot = {
            let mut runtime = self.runtime.lock();
            let status = runtime.entry(project).or_default();
            if status.state == state {
                return;
            }
            status.state = state;
            status.snapshot(project)
        };
        self.wake.sync_state(self.with_known_peers(snapshot));
    }

    pub(super) fn set_peer(
        &self,
        project: Id,
        endpoint_id: String,
        alias: String,
        connected: bool,
        converged: bool,
        state: SyncPhase,
    ) {
        let snapshot = {
            let mut runtime = self.runtime.lock();
            let status = runtime.entry(project).or_default();
            status.state = state;
            let peer = status.peers.entry(endpoint_id.clone()).or_insert(SyncPeerStatus {
                endpoint_id,
                alias: alias.clone(),
                connected,
                last_converged_at: None,
            });
            peer.alias = alias;
            peer.connected = connected;
            if converged {
                peer.last_converged_at =
                    Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
            }
            status.snapshot(project)
        };
        let snapshot = self.with_known_peers(snapshot);
        self.wake.sync_peer(snapshot.clone());
        self.wake.sync_state(snapshot);
    }
}
