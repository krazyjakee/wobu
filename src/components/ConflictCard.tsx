import { useMemo, useState } from 'react'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import type { Conflict } from '../lib/api'
import { useResolveConflict } from '../lib/queries'
import { collapse, diffLines, hasChanges } from '../lib/diff'
import type { DiffRow } from '../lib/diff'
import { report } from '../store/ui'
import { Icon } from './Icon'

/**
 * Two versions of one node, and the decision between them.
 *
 * Raised when a save lost a race and Wobu parked the losing version beside the
 * winner. The card offers exactly three things and no fourth: **keep mine**,
 * **keep theirs**, and **open both**. There is no merge button, and there must
 * never be one — three sentences two people rewrote in different directions
 * have no correct interleaving, and a machine guessing at one produces text
 * neither of them wrote. See `docs/07-file-shares.md`.
 *
 * "Open both" is what it says: it stops folding the identical parts and shows
 * the two documents whole, side by side, so the reader can judge them rather
 * than judge a diff of them. It deletes nothing and writes nothing — the
 * conflict stays open, because leaving both files on disk for a human, or for
 * git, is a legitimate answer and the card should not pretend otherwise.
 *
 * Keeping one version deletes the other, and that is the only place in Wobu
 * where a conflict sibling is deleted at all.
 */
export function ConflictCard({
  conflict,
  projectPath,
}: {
  conflict: Conflict
  projectPath: string
}) {
  const [both, setBoth] = useState(false)
  const resolve = useResolveConflict()
  const current = conflict.current
  const parked = conflict.parked

  const rows = useMemo(() => diffLines(current, parked), [current, parked])
  const shown = useMemo(() => (both ? rows : collapse(rows)), [both, rows])
  const differs = hasChanges(rows)

  // "Keep mine / keep theirs" only reads correctly for the person who lost the
  // race. Anyone else opening this card — a collaborator on the share, or the
  // same person tomorrow — needs the two sides named after who wrote them, so
  // the wording follows `mine` rather than assuming it.
  const owner = conflict.mine ? 'Your' : ownerLabel(conflict.user)
  const keepParked = conflict.mine ? 'Keep mine' : `Keep ${owner}`
  const keepCurrent = conflict.mine ? 'Keep theirs' : 'Keep what is on disk'
  const when = formatWhen(conflict.savedAt)

  const busy = resolve.isPending
  const keep = (k: 'parked' | 'current') =>
    resolve.mutate({
      relPath: conflict.relPath,
      keep: k,
      expectedHash: conflict.currentHash,
    })

  // Forward slashes throughout: `relPath` is stored `/`-separated so the same
  // project opens from `/Volumes/art` and `Z:\art`, and Windows accepts them.
  async function reveal() {
    try {
      await revealItemInDir(`${projectPath}/${conflict.relPath}`)
    } catch (e) {
      report(e, 'Could not open the file manager')
    }
  }

  return (
    <section className="conflict" aria-label="Unresolved conflict">
      <header className="conflict-h">
        <Icon name="layers" size="sm" />
        <span className="conflict-title">
          {conflict.nodeName ?? conflict.nodeRelPath} — two versions
        </span>
        <span className="conflict-when">
          {owner} {when}
        </span>
      </header>

      <p className="conflict-note">
        {/* Said plainly, because the fear this card has to answer immediately is
            "have I lost what I just typed". Nothing has been thrown away yet. */}
        Both versions are on disk. Nothing has been deleted, and nothing will be until you choose —
        the one you do not keep is removed then.
      </p>

      {differs ? (
        <div className="conflict-diff" role="table">
          <div className="conflict-cols" role="row">
            <span role="columnheader">On disk now</span>
            <span role="columnheader">{owner} version</span>
          </div>
          {shown.map((row, i) => (
            <DiffLine key={i} row={row} />
          ))}
        </div>
      ) : (
        <p className="conflict-same">
          The two files are line for line identical, so keeping either loses nothing.
        </p>
      )}

      <div className="conflict-acts">
        <button className="btn-mini" onClick={() => keep('parked')} disabled={busy}>
          <Icon name="check" size="sm" />
          {keepParked}
        </button>
        <button className="btn-mini" onClick={() => keep('current')} disabled={busy}>
          <Icon name="check" size="sm" />
          {keepCurrent}
        </button>
        <button
          className={both ? 'btn-mini is-on' : 'btn-mini'}
          onClick={() => setBoth((v) => !v)}
          aria-pressed={both}
        >
          <Icon name="copy" size="sm" />
          {both ? 'Show changes only' : 'Open both'}
        </button>
        <button className="btn-mini" onClick={reveal}>
          <Icon name="folder" size="sm" />
          Reveal
        </button>
      </div>
    </section>
  )
}

/**
 * One line naming conflicts on nodes other than the one being edited.
 *
 * Without it a conflict on a node the user never happens to open is invisible
 * for as long as they never open it — the same silent loss the conflict card
 * exists to prevent, only slower. Deliberately not a card: it offers no
 * decision, because the decision needs the diff, and the diff needs the node.
 */
export function ConflictsElsewhere({
  conflicts,
  onJump,
}: {
  conflicts: Conflict[]
  onJump: (id: string) => void
}) {
  if (!conflicts.length) return null
  return (
    <p className="conflict-else">
      <Icon name="layers" size="sm" />
      {conflicts.length === 1
        ? 'Another node has two versions waiting: '
        : `${conflicts.length} other nodes have two versions waiting: `}
      {conflicts.map((c, i) => (
        <span key={c.relPath}>
          {i > 0 && ', '}
          {c.nodeId ? (
            <button className="conflict-jump" onClick={() => onJump(c.nodeId as string)}>
              {c.nodeName ?? c.nodeRelPath}
            </button>
          ) : (
            // No node to jump to — the file it was parked beside is gone, so
            // the path is the only handle there is.
            <code>{c.nodeRelPath}</code>
          )}
        </span>
      ))}
    </p>
  )
}

function DiffLine({ row }: { row: DiffRow }) {
  if (row.kind === 'gap') {
    return (
      <div className="conflict-row is-gap" role="row">
        <span className="conflict-gap">{row.count} identical lines</span>
      </div>
    )
  }
  return (
    <div className={`conflict-row is-${row.kind}`} role="row">
      <span className="conflict-no">{row.leftNo ?? ''}</span>
      <span className="conflict-cell conflict-l" role="cell">
        {row.left}
      </span>
      <span className="conflict-no">{row.rightNo ?? ''}</span>
      <span className="conflict-cell conflict-r" role="cell">
        {row.right}
      </span>
    </div>
  )
}

/** `nadia-okonkwo` reads better as `Nadia Okonkwo's`; the filename only has a slug. */
function ownerLabel(user: string | null): string {
  if (!user) return "Someone's"
  const name = user
    .split('-')
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ')
  return name.endsWith('s') ? `${name}'` : `${name}'s`
}

/**
 * "at 14:22 today" — the two things that identify a save to the person who made
 * it. Never a bare date: a conflict is almost always minutes old, and "31 July"
 * does not tell anybody which of their afternoon's saves this was.
 */
function formatWhen(iso: string | null): string {
  if (!iso) return 'version'
  const at = new Date(iso)
  if (Number.isNaN(at.getTime())) return 'version'

  const time = at.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
  const midnight = new Date()
  midnight.setHours(0, 0, 0, 0)
  const days = Math.floor((midnight.getTime() - at.getTime()) / 86_400_000) + 1

  if (days <= 0) return `version, ${time} today`
  if (days === 1) return `version, ${time} yesterday`
  return `version, ${at.toLocaleDateString()} ${time}`
}
