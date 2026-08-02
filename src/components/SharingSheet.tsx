import { useCallback, useEffect, useMemo, useState } from 'react'
import type { ProjectSummary, SharedTicket, SyncStatus } from '../lib/api'
import { errorMessage, syncShare, syncStatus, syncUnshare } from '../lib/api'
import { toast } from '../store/ui'
import { ConfirmSheet } from './ConfirmSheet'
import { Modal } from './Modal'

export function SharingSheet({
  project,
  onClose,
}: {
  project: ProjectSummary
  onClose: () => void
}) {
  const [status, setStatus] = useState<SyncStatus | null>(null)
  const [ticket, setTicket] = useState<SharedTicket | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [working, setWorking] = useState(false)
  const [confirmUnshare, setConfirmUnshare] = useState(false)

  const refresh = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      setStatus(await syncStatus())
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const share = status?.shares.find((entry) => entry.project === project.id) ?? null
  const runtime = status?.projects.find((entry) => entry.project === project.id) ?? null
  const knownPeers = useMemo(() => runtime?.peers.map((peer) => peer.alias) ?? [], [runtime])

  async function makeTicket(copy: boolean) {
    setWorking(true)
    setError(null)
    try {
      const created = await syncShare()
      setTicket(created)
      await refresh()
      if (copy) {
        if (navigator.clipboard?.writeText) {
          await navigator.clipboard.writeText(created.token)
          toast('Share ticket copied')
        } else {
          setError('The ticket was created and is shown above, but it could not be copied.')
        }
      }
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setWorking(false)
    }
  }

  async function stopSharing() {
    setWorking(true)
    setError(null)
    try {
      await syncUnshare(project.id)
      setTicket(null)
      setConfirmUnshare(false)
      await refresh()
      toast('Project is no longer shared')
    } catch (reason) {
      setError(errorMessage(reason))
      setConfirmUnshare(false)
    } finally {
      setWorking(false)
    }
  }

  return (
    <>
      <Modal
        titleId="sharing-title"
        descriptionId="sharing-description"
        onClose={onClose}
        busy={working}
        busyMessage={working ? 'Updating this project’s sharing state…' : undefined}
      >
        <h2 id="sharing-title">{share ? 'Manage sharing' : 'Share this project'}</h2>
        <p id="sharing-description">
          Wobu syncs peer to peer. Both people must have Wobu running at the same time; there is no
          cloud copy or always-on server.
        </p>

        <div className="share-warning" role="note">
          <strong>A ticket is an access credential.</strong>
          <p>
            Send it privately. Tickets do not expire and one ticket cannot be revoked by itself.
            Stopping sharing revokes every ticket this installation issued for this project, but it
            does not delete copies collaborators already downloaded.
          </p>
        </div>

        {loading && <p role="status">Reading sharing status…</p>}
        {!loading && status && !status.running && (
          <div className="sheet-err" role="alert">
            Sync is still starting. Wait a moment, then try again.
          </div>
        )}
        {status?.running && !status.persistent && (
          <div className="sheet-err" role="alert">
            This session could not store its identity securely. Tickets may stop identifying this
            machine after Wobu restarts.
          </div>
        )}

        {share && (
          <div className="share-summary">
            <strong>Shared from this folder</strong>
            <code>{share.root}</code>
            <span>
              {share.peers} known {share.peers === 1 ? 'peer' : 'peers'}
              {runtime ? ` · ${runtime.state}` : ''}
            </span>
            {knownPeers.length > 0 && <span>Joined with {knownPeers.join(', ')}</span>}
          </div>
        )}

        {ticket && (
          <div className="field">
            <label htmlFor="share-ticket">Ticket</label>
            <textarea id="share-ticket" rows={4} readOnly value={ticket.token} />
            {!ticket.relayed && (
              <small className="share-relay-warning">
                No relay was available. This ticket is expected to work only on the same local
                network. Try creating it again when this machine is online for remote sharing.
              </small>
            )}
          </div>
        )}

        {error && (
          <div className="sheet-err" role="alert">
            {error}{' '}
            <button className="btn-mini" onClick={() => void refresh()}>
              Refresh status
            </button>
          </div>
        )}

        <div className="sheet-actions share-actions">
          {share && (
            <button className="btn" onClick={() => setConfirmUnshare(true)} disabled={working}>
              Stop sharing…
            </button>
          )}
          <span className="tspace" />
          <button className="btn btn-ghost" onClick={onClose} disabled={working}>
            Close
          </button>
          <button
            className="btn btn-primary"
            onClick={() => void makeTicket(true)}
            disabled={working || !status?.running}
            data-modal-initial-focus
          >
            {working ? 'Preparing…' : share ? 'Create and copy ticket' : 'Share and copy ticket'}
          </button>
        </div>
      </Modal>

      {confirmUnshare && (
        <ConfirmSheet
          title="Stop sharing this project?"
          body="This revokes every ticket this installation issued for this project and forgets sync history with its peers. It does not delete any project folder or collaborator copy."
          confirmLabel={working ? 'Stopping…' : 'Stop sharing'}
          danger
          busy={working}
          onCancel={() => setConfirmUnshare(false)}
          onConfirm={() => void stopSharing()}
        />
      )}
    </>
  )
}
