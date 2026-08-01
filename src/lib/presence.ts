/**
 * Who else has this project open, in one place.
 *
 * Everything here is **advisory and only advisory**. Nothing in this module,
 * and nothing that reads it, disables a control, delays a save or refuses one.
 * Hard locks over a share strand files whenever a laptop sleeps or a VPN drops
 * and the recovery is worse than the collision, so presence warns and never
 * blocks — see `docs/07-file-shares.md` and `wobu-store/src/presence.rs`.
 *
 * The wording lives here rather than in the components for the same reason
 * `readOnly.ts` exists: four surfaces say versions of one sentence — the
 * greeting on open, the navigator dot, the editor banner, the session count —
 * and they have to agree about who is here.
 */
import type { Peer } from './api'

/**
 * Seconds after which a session that has stopped writing its heartbeat is
 * treated as gone.
 *
 * The same sixty seconds as `presence::STALE_AFTER`, deliberately, and not a
 * second rule: the backend drops stale sessions when it answers, and this side
 * applies the identical bound to the answer it is holding. Two numbers here
 * would mean a peer the backend had reaped still lighting up a row, or a peer
 * who is sitting there flickering out of one.
 */
export const STALE_AFTER_SECS = 60

/**
 * How often the peer list is re-read.
 *
 * Half the heartbeat interval, so someone who arrives is on screen within about
 * a beat rather than two. Polled rather than pushed because the answer only
 * changes at human speed — see `presence_peers` for why there is no event.
 *
 * What this does not cover, honestly: polling pauses while the window is in the
 * background, so a list can be arbitrarily old on return. A refetch on focus is
 * what corrects it, and the filter below is what stops the held answer being
 * believed in the meantime.
 */
export const PRESENCE_POLL_MS = 10_000

/** One banner, keyed by code, exactly as `project.read_only` is. */
export const PRESENCE_BANNER = 'presence.editing_elsewhere'

/**
 * The peers still beating, out of whatever the last poll returned.
 *
 * `seenSecsAgo` is measured on the backend against its own clock, never against
 * the timestamp inside a peer's file — two desktops on a LAN routinely sit a
 * minute apart, which is the whole tolerance being spent here.
 */
export function livePeers(peers: Peer[] | undefined): Peer[] {
  return (peers ?? []).filter((p) => p.seenSecsAgo <= STALE_AFTER_SECS)
}

/** The live peers with `nodeId` open, which is what raises a dot and a banner. */
export function editorsOf(nodeId: string | null, peers: Peer[]): Peer[] {
  if (!nodeId) return []
  return peers.filter((p) => p.editing.includes(nodeId))
}

/**
 * Node id → who has it open, for the navigator.
 *
 * Built once per poll rather than searched per row: a world of several hundred
 * nodes would otherwise scan the peer list once for each of them on every
 * render, to answer a question that is almost always "nobody".
 */
export function editorsByNode(peers: Peer[]): Map<string, string> {
  const byNode = new Map<string, Peer[]>()
  for (const p of peers) {
    for (const id of p.editing) {
      const at = byNode.get(id)
      if (at) at.push(p)
      else byNode.set(id, [p])
    }
  }
  return new Map([...byNode].map(([id, who]) => [id, names(who)]))
}

/**
 * "Nadia", "Nadia and Tomas", "Nadia, Tomas and 2 others".
 *
 * Deduplicated, because the same person with the project open on a desktop and
 * a laptop is two sessions and one name, and "Nadia and Nadia" reads as a bug.
 * The session count in the status bar is the number that stays honest about the
 * difference.
 */
function names(peers: Peer[]): string {
  const unique = [...new Set(peers.map((p) => p.user))]
  const [first, second] = unique
  if (!first) return ''
  if (!second) return first
  if (unique.length === 2) return `${first} and ${second}`
  const rest = unique.length - 2
  return `${first}, ${second} and ${rest} other${rest === 1 ? '' : 's'}`
}

/** Singular against the deduplicated names, not against the session count. */
function verb(peers: Peer[]): string {
  return new Set(peers.map((p) => p.user)).size === 1 ? 'has' : 'have'
}

/**
 * The greeting on open, or `null` when nobody else is here.
 *
 * `null` rather than "nobody else has this open": an empty folder is the
 * ordinary case, and announcing it every time would train people to dismiss the
 * one message that matters.
 */
export function openedText(peers: Peer[]): string | null {
  if (peers.length === 0) return null
  return `${names(peers)} ${verb(peers)} this project open.`
}

/**
 * The passive banner over the editor.
 *
 * It says what happens if both of you save, because that is the only thing the
 * user can act on — and what happens is a conflict file rather than a loss,
 * which is the reassurance that makes not blocking defensible.
 */
export function editingText(peers: Peer[], nodeName: string): string {
  return (
    `${names(peers)} ${verb(peers)} “${nodeName}” open in another session. Nothing is locked ` +
    'and nothing here is switched off — if you both save, the later save is parked beside the ' +
    'file as a conflict for you to resolve rather than overwriting anything.'
  )
}

/** The dot's tooltip. A dot with no explanation is just a mark on a row. */
export function editingTitle(who: string): string {
  return `${who} has this node open in another session — nothing is locked`
}

/**
 * The status bar's session count, or `null` when this is the only session.
 *
 * Sessions rather than people, and ours is one of them: the number answers "how
 * many copies of Wobu are in this folder", which is what the heartbeat files
 * actually say and what makes a second dot on a row explicable.
 */
export function sessionsText(peers: Peer[]): string | null {
  if (peers.length === 0) return null
  return `${peers.length + 1} sessions`
}

/** Names and hosts behind the count, for the status bar's tooltip. */
export function sessionsTitle(peers: Peer[]): string {
  return peers.map((p) => `${p.user} (${p.host})`).join(', ')
}
