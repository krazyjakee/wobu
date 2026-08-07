use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use wobu_core::Id;
use wobu_sync::Ticket;

use super::*;
use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::sync::SyncPhase;
use crate::sync::round;

impl SyncManager {
    /// One explicit outbound round, with no timer. The integration harness uses
    /// this to make every network step deterministic and observable.
    #[cfg(test)]
    pub(in crate::sync) async fn run_once(self: &Arc<SyncManager>, project: Id) -> bool {
        self.dial_round(project).await
    }

    pub(in crate::sync) async fn run_ticket(
        self: &Arc<SyncManager>,
        project: Id,
        ticket: &Ticket,
    ) -> CommandResult<()> {
        let Some(replica) = self.replica(project) else {
            return Err(WobuError::new(Code::Internal, "The clone was not registered."));
        };
        self.set_phase(project, SyncPhase::Connecting);
        let session = self.endpoint().connect_ticket(ticket).await.map_err(|error| {
            self.set_phase(project, SyncPhase::Offline);
            WobuError::from(error)
        })?;
        let gate = replica.round.lock().await;
        let endpoint_id = ticket.peer().to_string();
        let alias = ticket.alias();
        self.set_peer(project, endpoint_id.clone(), alias.clone(), true, false, SyncPhase::Syncing);
        let outcome = round::run(self, &replica, &session).await;
        drop(gate);
        session.close();
        let converged = outcome.as_ref().is_ok_and(|outcome| outcome.converged());
        self.set_peer(project, endpoint_id, alias, false, converged, SyncPhase::Idle);
        outcome.map(|_| ())
    }

    pub(in crate::sync) fn start_poller(self: &Arc<SyncManager>, project: Id) {
        if self.poll {
            self.spawn_poller(project);
        }
    }

    /// One outbound round against every peer a share names.
    ///
    /// Returns whether anything actually moved, which is what [`BACKOFF`] walks.
    async fn dial_round(self: &Arc<SyncManager>, project: Id) -> bool {
        let Some(replica) = self.replica(project) else { return false };
        let Some(share) = self.shares.lock().get(project).cloned() else { return false };
        if share.peers.is_empty() {
            return false;
        }

        let mut worked = false;
        let mut answered = false;
        for ticket in &share.peers {
            if self.stopping() {
                return worked;
            }
            self.set_phase(project, SyncPhase::Connecting);
            let session = match self.endpoint().connect_ticket(ticket).await {
                Ok(session) => session,
                Err(e) => {
                    // A peer that is not online is the ordinary state of a
                    // peer-to-peer share with no seed node, so this is a debug
                    // line and not an error: raising a toast every time a
                    // collaborator's laptop is shut would make the app unusable.
                    diag::record(
                        diag::Level::Debug,
                        format!("sync: {} is not answering: {e}", ticket.alias()),
                    );
                    continue;
                }
            };
            answered = true;

            let gate = replica.round.lock().await;
            let endpoint_id = ticket.peer().to_string();
            let alias = ticket.alias();
            self.set_peer(
                project,
                endpoint_id.clone(),
                alias.clone(),
                true,
                false,
                SyncPhase::Syncing,
            );
            let outcome = round::run(self, &replica, &session).await;
            drop(gate);
            session.close();

            let converged = outcome.as_ref().is_ok_and(|outcome| outcome.converged());
            self.set_peer(project, endpoint_id, alias, false, converged, SyncPhase::Idle);

            match outcome {
                Ok(outcome) => worked |= outcome.did_something(),
                Err(e) => diag::error(format!("sync: round with a peer failed: {}", e.message)),
            }
        }
        self.set_phase(project, if answered { SyncPhase::Idle } else { SyncPhase::Offline });
        worked
    }

    /// The outbound half: dial this share's peers, back off when nothing
    /// happens.
    ///
    /// One task per share rather than one task with a schedule, because the
    /// backoff is per share — a world in step with a collaborator who is asleep
    /// must not slow down the one they are both editing right now.
    pub(super) fn spawn_poller(self: &Arc<SyncManager>, project: Id) {
        let manager = Arc::downgrade(self);
        let handle = tauri::async_runtime::spawn(async move {
            loop {
                let wait = {
                    let Some(manager) = manager.upgrade() else { return };
                    if manager.stopping() {
                        return;
                    }
                    let Some(replica) = manager.replica(project) else { return };
                    let step = replica.idle.load(Ordering::Relaxed) as usize;
                    Duration::from_secs(BACKOFF[step.min(BACKOFF.len() - 1)])
                    // `manager` dropped here, deliberately: holding a strong
                    // reference across the sleep would keep the endpoint alive
                    // through a shutdown for up to two minutes.
                };
                tokio::time::sleep(wait).await;

                let Some(manager) = manager.upgrade() else { return };
                if manager.stopping() {
                    return;
                }
                let worked = manager.dial_round(project).await;
                if let Some(replica) = manager.replica(project) {
                    if worked {
                        replica.idle.store(0, Ordering::Relaxed);
                    } else {
                        replica.idle.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        self.pollers.lock().push(handle);
    }
}
