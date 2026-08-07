use std::path::Path;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use wobu_core::Id;
use wobu_sync::{Disposition, Projects, Session, Sessions, Ticket};

use super::*;
use crate::diag;
use crate::error::CommandResult;
use crate::sync::SyncPhase;
use crate::sync::round;

/// The accept side, holding a [`Weak`] so that the router does not keep the
/// manager — and therefore the router — alive. See the module documentation.
pub(super) struct Accepts(pub(super) Weak<SyncManager>);

impl Projects for Accepts {
    /// Cheap, synchronous, and a bool. The signature is the security boundary:
    /// there is nowhere to put a list, so the accept path cannot disclose one.
    fn admits(&self, project: &Id, grant: Option<&wobu_sync::Grant>) -> bool {
        self.0.upgrade().is_some_and(|m| !m.stopping() && m.admits(project, grant))
    }
}

#[async_trait]
impl Sessions for Accepts {
    /// This future *is* the session's lifetime — iroh drops the connection as
    /// soon as it returns — so the round runs inside it rather than being sent
    /// somewhere. That is also why it is bounded: an accept handler that never
    /// returns is an accept loop that cannot be wound down, which is the
    /// shutdown hang [`SyncManager::shutdown`] is written against.
    async fn opened(&self, session: Session) {
        let Some(manager) = self.0.upgrade() else { return };
        let Some(replica) = manager.replica(session.project()) else {
            session.close();
            return;
        };
        // Refused rather than queued. See the module documentation: making a
        // peer wait would stall it inside a manifest exchange whose idle timeout
        // would then fire, and "busy, try again" is both truer and faster.
        let Ok(gate) = replica.round.try_lock() else {
            session.close();
            return;
        };

        let project = session.project();
        let endpoint_id = session.peer().to_string();
        let alias = wobu_core::peer::alias(session.peer().as_bytes());
        manager.set_peer(
            project,
            endpoint_id.clone(),
            alias.clone(),
            true,
            false,
            SyncPhase::Syncing,
        );

        let outcome =
            tokio::time::timeout(SESSION_BUDGET, round::run(&manager, &replica, &session));
        let converged = match outcome.await {
            Ok(Ok(outcome)) => outcome.converged(),
            Ok(Err(e)) => {
                diag::error(format!("sync: inbound round failed: {}", e.message));
                false
            }
            Err(_elapsed) => {
                diag::error("sync: inbound round ran out of time");
                false
            }
        };
        drop(gate);
        session.close();
        manager.set_peer(project, endpoint_id, alias, false, converged, SyncPhase::Idle);
    }
}

/// The project ULIDs this machine has a folder for, as a value.
///
/// Only [`Ticket::disposition`] consults this, and it is **not** the accept
/// path's answer — see [`SyncManager::present`] for the two questions and why
/// they are different sets.
///
/// A value rather than a borrow for two reasons. [`Projects`] is `'static` — it
/// is stored behind an `Arc<dyn _>` on the accept path, so it has to be — and
/// the manager's own implementation lives on [`Accepts`] behind a `Weak` that a
/// `&self` method cannot produce. And a snapshot cannot see the registry mutate
/// underneath it, which matters because "which worlds do I have" is precisely
/// the question nothing should be able to answer twice differently within one
/// decision.
struct Present(std::collections::BTreeSet<Id>);

impl Projects for Present {
    fn admits(&self, project: &Id, _grant: Option<&wobu_sync::Grant>) -> bool {
        self.0.contains(project)
    }
}

impl SyncManager {
    /// Somebody pressed "Share".
    ///
    /// Returns the ticket to hand out. **Await [`SyncEndpoint::online`] first**
    /// under [`Reach::Internet`] or the address in it has no relay and the
    /// string works on this machine's LAN and nowhere else — which fails at the
    /// far end as a dial timeout and reads to a user as "they are offline". The
    /// caller does that awaiting, because it is the caller that knows whether
    /// somebody is standing in front of a progress spinner.
    pub fn share(self: &Arc<SyncManager>, project: Id, root: &Path) -> Ticket {
        let grant = {
            let mut shares = self.shares.lock();
            let grant = shares.share(project, root).grant;
            report(shares.save());
            grant
        };
        self.register(project, root, self.state.open_id() == Some(project));
        self.announce_project(project);
        if self.poll {
            self.spawn_poller(project);
        }
        self.endpoint().ticket(project, grant)
    }

    /// Somebody pasted a ticket.
    ///
    /// [`Disposition::Clone`] is returned rather than acted on: cloning means
    /// creating a folder somewhere the user picks, and a background task must
    /// not choose where somebody's world lives. The caller surfaces it.
    ///
    /// [`Disposition::Join`] is the case this method actually does something in,
    /// and it is the one the issue names: one project ULID is one world however
    /// many folders it is sitting in, so a ticket for a project already here
    /// starts syncing *this* replica rather than making a second one.
    pub fn accept(self: &Arc<SyncManager>, ticket: &Ticket) -> Disposition {
        let project = ticket.project();
        if ticket.disposition(&self.present()) == Disposition::Clone {
            return Disposition::Clone;
        }
        let Some(root) = self.root_of(project).or_else(|| self.open_root(project)) else {
            // `present` said yes a line ago, so this is unreachable in practice.
            // Reporting `Clone` rather than panicking is the honest answer: this
            // machine cannot name a folder for the project, which is what
            // `Clone` means.
            return Disposition::Clone;
        };

        let mut shares = self.shares.lock();
        shares.invite(project, &root, ticket.clone());
        report(shares.save());
        drop(shares);

        self.register(project, &root, self.state.open_id() == Some(project));
        self.announce_project(project);
        if self.poll {
            self.spawn_poller(project);
        }
        Disposition::Join
    }

    /// Register the fresh folder the foreground Accept flow just created.
    /// Polling begins only after its explicit first round succeeds, so that
    /// round and a background dial cannot race to initialise the clone.
    pub(in crate::sync) fn accept_clone(self: &Arc<SyncManager>, ticket: &Ticket, root: &Path) {
        let project = ticket.project();
        let mut shares = self.shares.lock();
        shares.invite(project, root, ticket.clone());
        report(shares.save());
        drop(shares);
        self.register(project, root, false);
        self.announce_project(project);
    }

    /// Stop syncing a project, and forget everything agreed with everybody about
    /// it.
    ///
    /// `forget_peer` for each peer as well as dropping the share, and the reason
    /// is `wobu-store`'s: a base is a claim that a specific machine holds
    /// specific bytes and the next sync fast-forwards on it without asking. A
    /// base left behind for a collaborator who has been un-shared is a licence to
    /// overwrite, sitting in a database, waiting for somebody to be re-added.
    /// Forgetting too much costs a re-compare; forgetting too little costs
    /// somebody's writing.
    pub fn unshare(&self, project: Id) -> CommandResult<()> {
        let peers: Vec<String> = {
            let mut shares = self.shares.lock();
            let peers = shares
                .get(project)
                .map(|s| s.peers.iter().map(|t| t.peer().to_string()).collect())
                .unwrap_or_default();
            shares.forget(project);
            report(shares.save());
            peers
        };

        // Taken out of the map before the loop, so the registry lock is not held
        // across a store step. The lock order everywhere else is replicas → held
        // → project slot, and this is the one place it would have been tempting
        // to hold all three.
        let replica = self.replicas.lock().remove(&project);
        if let Some(replica) = replica {
            for peer in &peers {
                // Reported and not propagated: a share that has been removed
                // from the list is not shared any more whether or not the index
                // could be tidied, and failing the command would leave the two
                // halves disagreeing.
                if let Err(e) = replica.with(|p| Ok(p.forget_peer(peer)?)) {
                    diag::error(format!(
                        "sync: could not forget peer for {project}: {}",
                        e.message
                    ));
                }
            }
        }
        self.runtime.lock().remove(&project);
        Ok(())
    }

    /// Whether this machine holds a project and the dialler presented the grant
    /// this installation minted for it. Both failures deliberately collapse to
    /// one bool before the transport constructs its refusal.
    fn admits(&self, project: &Id, grant: Option<&wobu_sync::Grant>) -> bool {
        if !self.replicas.lock().contains_key(project) {
            return false;
        }
        self.shares
            .lock()
            .get(*project)
            .is_some_and(|share| grant.is_some_and(|grant| grant == &share.grant))
    }

    /// Every world on this machine, shared or merely open. See [`Present`].
    ///
    /// Deliberately a wider set than [`Self::admits`], and the gap between them is
    /// the point. "Do I have this world anywhere" is the question a pasted ticket
    /// asks, and the open project counts — a friend sending a ticket for the
    /// world already on screen is joining it, not cloning a second copy of it
    /// next to itself. "May a stranger who dialled me sync this" is a different
    /// question and its answer is [`Self::admits`], which is shares only, because
    /// merely opening a folder is not consent to serve it to anybody who can
    /// guess its ULID off a `project.json` on a NAS.
    fn present(&self) -> Present {
        let mut ids: std::collections::BTreeSet<Id> =
            self.replicas.lock().keys().copied().collect();
        ids.extend(self.state.open_id());
        Present(ids)
    }
}
