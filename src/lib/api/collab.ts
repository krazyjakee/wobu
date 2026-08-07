import { call } from './call'
/* ── domain types ─────────────────────────────────────────────────────────── */

/**
 * A version of a node that lost a save race and was parked beside the winner.
 *
 * Both documents arrive whole rather than as a diff: how much context to show
 * and whether to fold the unchanged parts is a rendering decision, and `Conflict`
 * is a few kilobytes. `src/lib/diff.ts` does the aligning on this side.
 */
export interface Conflict {
  /** The sibling file, project-relative. Also the handle `conflictResolve` takes. */
  relPath: string
  /** The node file it was parked beside. */
  nodeRelPath: string
  /** Null when the node file has since been deleted or cannot be parsed. */
  nodeId: string | null
  nodeName: string | null
  /** Whose version was set aside, read back out of the filename. */
  user: string | null
  /** ISO 8601. Null for a sibling whose name nothing could parse. */
  savedAt: string | null
  /**
   * Whether the parked version carries *this* session's user name — the
   * difference between a card that says "keep mine" and one that says "keep
   * Nadia's". Only the wording depends on it.
   */
  mine: boolean
  /** The text that was set aside. */
  parked: string
  /** What is at `nodeRelPath` right now. Empty if the node file is gone. */
  current: string
  /**
   * Hash of `current` as the card rendered it. Handed straight back to
   * `conflictResolve`, which refuses if it no longer matches — that is what
   * stops a decision being applied to a version the user never read.
   */
  currentHash: string
}

/** Which version the user chose. There is deliberately no "merge". */
export type ConflictKeep = 'parked' | 'current'

/**
 * What came of a resolution. Only `done` changed anything on disk; `stale` and
 * `conflict` both left the two files exactly as they were.
 */
export type ConflictResolved =
  | { outcome: 'done' }
  /** A third writer moved the node file while the user was reading the diff. */
  | { outcome: 'stale' }
  /** The write lost a race of its own; our pick is parked as a further sibling. */
  | { outcome: 'conflict'; conflictPath: string }

export const conflicts = () => call<Conflict[]>('conflicts')

/**
 * Apply the user's decision, and delete the version they rejected.
 *
 * The only call in the app that removes a conflict sibling, which is why it is
 * only ever reached from a button. `expectedHash` is `Conflict.currentHash`
 * unchanged — passing anything else defeats the staleness check that stops a
 * third writer's version being discarded unseen.
 */
export const conflictResolve = (relPath: string, keep: ConflictKeep, expectedHash: string) =>
  call<ConflictResolved>('conflict_resolve', { relPath, keep, expectedHash })

/* ── presence ─────────────────────────────────────────────────────────────── */

/**
 * Someone else with this project open.
 *
 * Advisory, and only advisory. Nothing here reserves a node or refuses a save —
 * hard locks over a share strand files whenever a laptop sleeps or a VPN drops,
 * and the recovery is worse than the collision. A peer editing the node you are
 * in is a banner, never a block. See `docs/07-file-shares.md`.
 */
export interface Peer {
  /** ULID of their session, not of a node. Stable for as long as they stay. */
  sessionId: string
  user: string
  /** Best effort — reads `unknown` on platforms that do not hand it over. */
  host: string
  /**
   * Seconds since their heartbeat file was last written, measured by *this*
   * machine's clock against the file's mtime.
   *
   * Deliberately not a timestamp. Two machines on a LAN routinely sit an hour
   * apart, so a duration computed on this side from a time the other side wrote
   * would be wrong by exactly that much, and nothing here could tell.
   */
  seenSecsAgo: number
  /** Node ids they have open. Render it; do not act on it. */
  editing: string[]
}

/**
 * Who else has this project open.
 *
 * Poll it — there is no event, because the answer only changes at human speed
 * and pushing one per heartbeat per peer would be traffic on the same share the
 * presence is describing. Empty when no project is open, rather than an error.
 */
export const presencePeers = () => call<Peer[]>('presence_peers')

/**
 * Tell everyone else which nodes this session has open.
 *
 * The whole list every time, not a delta: this side already knows exactly which
 * nodes are open, and a delta would drift the first time one closed while the
 * share was away.
 */
export const presenceEditing = (nodeIds: string[]) => call<void>('presence_editing', { nodeIds })

/* ── provider keys ────────────────────────────────────────────────────────── */
