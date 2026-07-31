import { create } from 'zustand'
import * as api from './api'
import type { WobuNode } from './api'

/**
 * The workspace undo stack.
 *
 * Every world mutation the app performs is recorded here as a pair of
 * primitive command sequences — one that puts the world back, one that redoes
 * it — rather than as a diff of the UI state. The recording happens at the
 * mutation hooks in `lib/queries.ts` and nowhere else: that is the single choke
 * point every call site already goes through, so a new button that renames a
 * node is undoable the day it is written, without its author knowing this file
 * exists.
 *
 * Three primitives are enough for the whole surface, which is why there are
 * only three. A rename, a notes edit, a link added or reweighted, a cover
 * assigned — all of them are `upsert`, because that is how they reach disk.
 * Create inverts to `delete`, delete inverts to `upsert`, move inverts to
 * `move`. Adding a fourth command means the backend grew a write path that
 * `queries.ts` does not own, which is the thing to fix instead.
 */
export type WorldCommand =
  | { type: 'upsert'; node: WobuNode }
  | { type: 'delete'; id: string }
  | { type: 'move'; id: string; parentId: string | null }

export interface UndoEntry {
  /**
   * The node the entry is about. Only used to decide whether two consecutive
   * edits belong to the same run of typing.
   */
  nodeId: string
  /** Verb phrase for the toast and the palette: "rename Ashfall". */
  label: string
  /** Applied in order. More than one only for a delete, which restores children. */
  undo: WorldCommand[]
  redo: WorldCommand[]
  /**
   * Whether a later entry for the same node may absorb this one. True for the
   * debounced content edits, false for the structural commands — nobody wants
   * three separate deletes to collapse into one ⌘Z.
   */
  coalesce: boolean
  /** `Date.now()` at the push, so the coalescing window can be measured. */
  at: number
  /** Something the inverse cannot restore, said out loud when it runs. */
  caveat?: string
}

/** An entry before it is pushed; `at` is stamped by `push` unless a test sets it. */
export type NewEntry = Omit<UndoEntry, 'at'> & { at?: number }

/** Runs one command against the world. Injected so the store stays testable. */
export type Runner = (cmd: WorldCommand) => Promise<void>

/**
 * How long after an edit a further edit to the same node still counts as the
 * same action.
 *
 * Notes and description editing reach `node_upsert` through `useAutosaveNode`,
 * which debounces at 500ms by default. Logging each of those writes verbatim
 * would make ⌘Z rewind half a second of typing at a time, which is not undo —
 * it is a very slow backspace. This window has to be comfortably longer than
 * that debounce or consecutive keystrokes never merge; it is deliberately not
 * derived from the autosave setting, because someone who raises the debounce to
 * three seconds has already chosen three-second save granularity and coarser
 * undo is the honest consequence of that.
 *
 * The window slides: each absorbed edit refreshes it, so an unbroken run of
 * typing is one entry however long it lasts, and a pause of more than a second
 * starts a new one. That matches where a person would expect the boundary to
 * be, which is the only defensible place to put it.
 */
export const COALESCE_MS = 1200

/**
 * Entries hold whole node snapshots, so the stack is bounded by memory rather
 * than by patience. A hundred is far past where anyone navigates by ⌘Z, and
 * cheap enough that a world of large notes cannot make the app fat.
 */
export const MAX_ENTRIES = 100

interface UndoState {
  /**
   * Which project the stack belongs to. Not persisted, and neither is anything
   * else here: an undo entry is a promise to restore an exact previous state,
   * and a stack that outlived the app cannot make that promise. Between quit
   * and relaunch the folder may have been edited in Obsidian, pulled, synced by
   * a collaborator or restored from a backup, and replaying a week-old inverse
   * over that would not be undo, it would be a silent overwrite of somebody
   * else's work. The stack is only trustworthy for the session that recorded it.
   */
  projectId: string | null
  past: UndoEntry[]
  future: UndoEntry[]
  /** An undo is in flight. Holding ⌘Z must not start a second one. */
  busy: boolean

  /** Point the stack at a project, discarding another project's history. */
  setProject: (id: string | null) => void
  push: (entry: NewEntry) => void
  undo: (run: Runner) => Promise<UndoEntry | null>
  redo: (run: Runner) => Promise<UndoEntry | null>
  clear: () => void
}

export const useUndoStack = create<UndoState>((set, get) => ({
  projectId: null,
  past: [],
  future: [],
  busy: false,

  setProject: (id) =>
    set((s) => (s.projectId === id ? {} : { projectId: id, past: [], future: [] })),

  push: (entry) =>
    set((s) => {
      // No project, no stack. Every command below names a node in a world that
      // is not open, so keeping them would mean an undo that either fails or —
      // far worse — lands in whichever project is opened next.
      if (!s.projectId) return {}

      const at = entry.at ?? Date.now()
      const top = s.past[s.past.length - 1]

      if (
        top &&
        entry.coalesce &&
        top.coalesce &&
        top.nodeId === entry.nodeId &&
        at - top.at <= COALESCE_MS
      ) {
        // The absorbed entry keeps the *older* inverse. That is the whole point
        // of coalescing: the state to go back to is the one before the first
        // keystroke of the run, not the one from 500ms ago. Only the redo and
        // the label move forward, to the newest text.
        const merged: UndoEntry = {
          ...top,
          label: entry.label,
          redo: entry.redo,
          at,
        }
        return { past: [...s.past.slice(0, -1), merged], future: [] }
      }

      // Doing something new after an undo abandons the redo branch. Keeping it
      // would mean a redo that replays an edit onto text that has since
      // diverged, which is a different world, not a later one.
      const past = [...s.past, { ...entry, at }]
      return { past: past.length > MAX_ENTRIES ? past.slice(-MAX_ENTRIES) : past, future: [] }
    }),

  undo: async (run) => {
    const { past, busy } = get()
    const entry = past[past.length - 1]
    if (busy || !entry) return null
    set({ past: past.slice(0, -1), busy: true })
    try {
      for (const cmd of entry.undo) await run(cmd)
      set((s) => ({ future: [...s.future, entry], busy: false }))
      return entry
    } catch (e) {
      // Put it back rather than swallowing it. The write was refused — a
      // conflict, a read-only folder, a share that went away — and all of those
      // are conditions the user can resolve and try again through. Losing the
      // entry would make the failure permanent.
      set((s) => ({ past: [...s.past, entry], busy: false }))
      throw e
    }
  },

  redo: async (run) => {
    const { future, busy } = get()
    const entry = future[future.length - 1]
    if (busy || !entry) return null
    set({ future: future.slice(0, -1), busy: true })
    try {
      for (const cmd of entry.redo) await run(cmd)
      set((s) => ({ past: [...s.past, entry], busy: false }))
      return entry
    } catch (e) {
      set((s) => ({ future: [...s.future, entry], busy: false }))
      throw e
    }
  },

  clear: () => set({ past: [], future: [] }),
}))

/**
 * Send one command to the backend.
 *
 * Deliberately the same guarded write path as an ordinary edit: `node_upsert`
 * compares against the stamp the index holds, so an undo that would clobber a
 * change made since — by a collaborator on the share, or by the user in
 * Obsidian — raises `write.conflict` exactly as a normal save would. An undo
 * that bypassed that check to "restore" a value would be the one operation in
 * the app licensed to destroy someone's work.
 *
 * `upsert` rather than `node_create` is also what makes undoing a delete
 * correct: `node_create` mints a fresh ULID, so a recreated node would be a
 * different entity and every link pointing at the original would resolve to
 * nothing. `save_node` looks up the stamp by id, finds none for a node that is
 * gone, and writes a new file under the original id — the node comes back as
 * itself.
 */
export function applyCommand(cmd: WorldCommand): Promise<void> {
  switch (cmd.type) {
    case 'upsert':
      return api.nodeUpsert(cmd.node).then(() => undefined)
    case 'delete':
      return api.nodeDelete(cmd.id)
    case 'move':
      return api.nodeMove(cmd.id, cmd.parentId)
  }
}

/**
 * What an edit did, in the words the toast will use — or `null` when it did
 * nothing worth remembering.
 *
 * The `null` case is not a nicety. Every save re-stamps `updated_at`, so two
 * consecutive upserts of identical content are different objects; logging one
 * would put an entry on the stack whose inverse restores the state it is
 * already in, and a ⌘Z that visibly does nothing reads as a broken feature.
 * `updated_at` is the one field deliberately not compared here for that reason.
 */
export function editLabel(before: WobuNode, after: WobuNode): string | null {
  if (before.name !== after.name) return 'rename'
  if (before.notesRaw !== after.notesRaw) return 'notes edit'
  if (before.summary !== after.summary) return 'summary edit'
  if (JSON.stringify(before.links) !== JSON.stringify(after.links)) return 'link change'
  if (JSON.stringify(before.description) !== JSON.stringify(after.description))
    return 'description edit'
  if (JSON.stringify(before.tags) !== JSON.stringify(after.tags)) return 'tag change'
  if (JSON.stringify(before.attributes) !== JSON.stringify(after.attributes))
    return 'attribute edit'
  if (before.coverAssetId !== after.coverAssetId) return 'cover change'
  if (before.parentId !== after.parentId) return 'move'
  if (before.descriptionState !== after.descriptionState || before.slug !== after.slug) {
    return 'edit'
  }
  return null
}

/* ── inverses ─────────────────────────────────────────────────────────────── */

/**
 * The four entry builders, kept here rather than at the mutation hooks that
 * call them.
 *
 * Every one of them is the answer to "what puts this back", which is the only
 * hard question in the whole feature and the one worth having in one file,
 * beside the primitives it is expressed in and under test. `queries.ts` is left
 * with what it is actually good for: knowing which cached state to hand over.
 */

/** A node that has just come into existence — created, duplicated, imported. */
export function birthEntry(node: WobuNode, verb: string): NewEntry {
  return {
    nodeId: node.id,
    label: `${verb} “${node.name}”`,
    undo: [{ type: 'delete', id: node.id }],
    // Redo goes back through `upsert`, not `node_create`: `node_create` mints a
    // fresh ULID, so a redone create would be a *different* node, and every
    // entry recorded after it — the ones about to be redone next — names the id
    // that no longer exists.
    redo: [{ type: 'upsert', node }],
    coalesce: false,
  }
}

/**
 * A delete, with the ids `node_delete` promoted to the deleted node's parent.
 *
 * Restoring the node alone would be half the job: the subtree would stay
 * flattened and the user would have to rebuild a hierarchy they never chose to
 * change. The moves come after the upsert because a child cannot be reparented
 * onto a node that is not there yet.
 *
 * The one thing no inverse can cover is the inbound links. The delete strips
 * every edge that pointed at the node, out of files anywhere in the world, and
 * finding them again would mean reading every node in the project — over what
 * may be a network share — on the off-chance the delete is undone. That is said
 * out loud when the undo runs rather than being quietly dropped.
 */
export function deletionEntry(node: WobuNode, childIds: string[]): NewEntry {
  return {
    nodeId: node.id,
    label: `delete “${node.name}”`,
    undo: [
      { type: 'upsert', node },
      ...childIds.map<WorldCommand>((id) => ({ type: 'move', id, parentId: node.id })),
    ],
    redo: [{ type: 'delete', id: node.id }],
    coalesce: false,
    caveat: 'Any links that pointed at it were removed by the delete and do not come back.',
  }
}

/**
 * A reparent. `null` when it did not move anything: `node_move` returns early
 * on a no-op, and an entry for it would undo to where the node already is.
 */
export function moveEntry(
  node: { id: string; name: string; parentId: string | null },
  parentId: string | null,
): NewEntry | null {
  if (node.parentId === parentId) return null
  return {
    nodeId: node.id,
    label: `move “${node.name}”`,
    undo: [{ type: 'move', id: node.id, parentId: node.parentId }],
    redo: [{ type: 'move', id: node.id, parentId }],
    coalesce: false,
  }
}

/**
 * A content edit — rename, notes, description, links, tags, cover — which all
 * reach disk the same way and so all invert the same way. `null` when the save
 * changed nothing but the timestamp.
 */
export function editEntry(before: WobuNode, after: WobuNode): NewEntry | null {
  const verb = editLabel(before, after)
  if (!verb) return null
  return {
    nodeId: after.id,
    label: `${verb} “${after.name}”`,
    undo: [{ type: 'upsert', node: before }],
    redo: [{ type: 'upsert', node: after }],
    // The only coalescing case: these are the writes that arrive one per
    // autosave debounce while somebody types.
    coalesce: true,
  }
}

/**
 * What a keydown means for the undo stack, if anything.
 *
 * `typing` is the caller's answer to "is a text field focused", and when it is
 * true this returns `null` on purpose: while the caret is in a textarea, ⌘Z
 * belongs to the field, and stealing it would make the workspace rewind a
 * whole save every time someone tried to take back a word.
 *
 * The cost of that rule, stated plainly because it is a real one: once the
 * field's own undo stack is exhausted the browser does nothing at all, and
 * workspace undo never gets its turn without the user first clicking away. The
 * alternative — taking ⌘Z once the native stack looks empty — cannot be
 * implemented, because the DOM does not expose whether it is. Between a key
 * that occasionally does nothing and a key that occasionally discards a
 * paragraph the user was still editing, this is the safe direction.
 */
export function undoIntent(
  e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey'>,
  typing: boolean,
): 'undo' | 'redo' | null {
  if (!(e.metaKey || e.ctrlKey)) return null
  const key = e.key.toLowerCase()
  // ⇧⌘Z is the mac redo; ⌃Y is what a Windows keyboard reaches for, and both
  // land here because the same build runs on both.
  if (key === 'z' && e.shiftKey) return typing ? null : 'redo'
  if (key === 'z') return typing ? null : 'undo'
  if (key === 'y' && !e.shiftKey) return typing ? null : 'redo'
  return null
}
