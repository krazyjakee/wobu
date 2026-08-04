import { useQuery } from '@tanstack/react-query'
import { useCallback, useMemo, useState } from 'react'
import type { ProjectSummary, SharedTicket } from '../lib/api'
import { errorMessage, syncShare, syncStatus, syncUnshare } from '../lib/api'
import { qk } from '../lib/queries'
import { toast } from '../store/ui'
import { ConfirmSheet } from './ConfirmSheet'
import { syncStateLabel } from '../lib/stateLabels'
import { Modal } from './Modal'

export function SharingSheet({
  project,
  onClose,
}: {
  project: ProjectSummary
  onClose: () => void
}) {
  const [ticket, setTicket] = useState<SharedTicket | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [working, setWorking] = useState(false)
  const [confirmUnshare, setConfirmUnshare] = useState(false)

  // The same key `useProjectSync` reads, so sharing from here and the live sync
  // state elsewhere cannot disagree about who this installation is shared with.
  const {
    data: status = null,
    isFetching: loading,
    error: readError,
    refetch,
  } = useQuery({ queryKey: qk.syncStatus, queryFn: syncStatus, retry: false })

  const refresh = useCallback(async () => {
    setActionError(null)
    await refetch()
  }, [refetch])

  // A failed share is the more specific answer, so it wins over a stale read.
  const error = actionError ?? (readError ? errorMessage(readError) : null)

  const share = status?.shares.find((entry) => entry.project === project.id) ?? null
  const runtime = status?.projects.find((entry) => entry.project === project.id) ?? null
  const knownPeers = useMemo(() => runtime?.peers.map((peer) => peer.alias) ?? [], [runtime])

  async function makeTicket(copy: boolean) {
    setWorking(true)
    setActionError(null)
    try {
      const created = await syncShare()
      setTicket(created)
      await refresh()
      if (copy) {
        if (navigator.clipboard?.writeText) {
          await navigator.clipboard.writeText(created.token)
          toast('Share ticket copied')
        } else {
          setActionError('The ticket was created and is shown above, but it could not be copied.')
        }
      }
    } catch (reason) {
      setActionError(errorMessage(reason))
    } finally {
      setWorking(false)
    }
  }

  async function stopSharing() {
    setWorking(true)
    setActionError(null)
    try {
      await syncUnshare(project.id)
      setTicket(null)
      setConfirmUnshare(false)
      await refresh()
      toast('Project is no longer shared')
    } catch (reason) {
      setActionError(errorMessage(reason))
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
        busyMessage={working ? 'Saving the change to sharing…' : undefined}
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
            Wobu could not store this machine&rsquo;s identity securely, so tickets may stop
            recognising it once Wobu has restarted.
          </div>
        )}

        {share && (
          <div className="share-summary">
            <strong>Shared from this folder</strong>
            <code>{share.root}</code>
            <span>
              {share.peers} known {share.peers === 1 ? 'peer' : 'peers'}
              {runtime ? ` · ${syncStateLabel(runtime.state)}` : ''}
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
                Wobu could not reach a public relay, so this ticket is only expected to work between
                machines on the same local network. Make another one when this machine is online, if
                you need to share further afield.
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
