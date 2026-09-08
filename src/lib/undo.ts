import { create } from 'zustand'
import { narrativeWorldRestore, type WorldDocument } from './api/narrativeWorld'
import * as api from './api'
import type { Scene, SceneFile, WobuNode } from './api'

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
 * Three primitives cover every node edit, and for a long time three was the
 * whole list. A rename, a notes edit, a link added or reweighted, a cover
 * assigned — all of them are `upsert`, because that is how they reach disk.
 * Create inverts to `delete`, delete inverts to `upsert`, move inverts to
 * `move`.
 *
 * ## Why there are now five
 *
 * Narrative source is the write path that earned the extension. A scene is not
 * a `WobuNode`: it is a structured YAML document in its own tree, saved through
 * its own guarded path against its own precondition, and there is no spelling
 * of `upsert` that reaches one. So `sceneSave` and `sceneDelete` sit beside the
 * node three, mirroring them exactly — create inverts to `sceneDelete`, delete
 * inverts to `sceneSave`, and every structural or textual edit inverts to
 * `sceneSave`, because that is how all of them reach disk.
 *
 * The alternative was a second undo stack for the Narrative workspace, and it
 * is worse in precisely the way that matters. #186 requires a structural edit
 * made on the canvas and the same edit made in a form to be indistinguishable
 * in undo history; two stacks would make ⌘Z mean different things in different
 * tabs, and in a workspace showing both a scene and a node inspector it would
 * mean whichever one happened to have focus.
 *
 * The bar for a sixth is unchanged: a new command here means the backend grew a
 * write path the mutation hooks in `lib/queries/` do not own, which is the
 * thing to fix instead of the thing to add.
 *
 * ## Canvas layout is deliberately absent, and must stay absent
 *
 * There is no command here that can carry a coordinate, and that is the
 * command-layer half of #185. Moving a box is not a story change: the file
 * layer keeps that true by putting arrangements in `narrative/layout/`, where
 * no fingerprint, compiler or export can reach them, and this file keeps it
 * true by having nothing that could put one on the world stack.
 *
 * The consequence is chosen rather than conceded: **arranging the canvas gets
 * no undo affordance at this layer at all.** A ⌘Z that rewound a drag would, on
 * the very next press, rewind a paragraph, and nothing on screen tells a writer
 * which press they are about to make. If the canvas ever wants to take back a
 * drag, that history belongs to the canvas, is scoped to the canvas, and is
 * never mixed into the history of the world.
 */
export type WorldCommand =
  | { type: 'upsert'; node: WobuNode }
  | { type: 'delete'; id: string }
  | { type: 'move'; id: string; parentId: string | null }
  /**
   * Write a whole scene document. `slug` is only consulted when the project no
   * longer has the scene — restoring one that was deleted — and is made unique
   * on the far side, so a restore can never land on top of a scene that took
   * the name in the meantime.
   */
  | { type: 'sceneSave'; scene: Scene; slug: string }
  | { type: 'sceneDelete'; id: string }
  | { type: 'worldRestore'; document: WorldDocument; expected: WorldDocument }

export interface UndoEntry {
  /**
   * The node or scene the entry is about. Only used to decide whether two
   * consecutive edits belong to the same run of typing — which is why it is a
   * subject rather than a node: a run of typing into a beat's dialogue has to
   * coalesce on the same terms a run of typing into a node's notes does.
   */
  subjectId: string
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
        top.subjectId === entry.subjectId &&
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
    // `current` rather than the stamp the entry was recorded against, and it is
    // the same guarantee `node_upsert` gives rather than a weaker one: that
    // command reads its precondition out of the index — this session's own view
    // of disk — and so does this. An entry recorded before three later saves
    // would otherwise present a precondition three versions stale and park the
    // undo as a conflict, which is a ⌘Z that fails on every press but the first.
    case 'sceneSave':
      return api.narrativeSceneSave(cmd.scene, { kind: 'current' }, cmd.slug).then(() => undefined)
    case 'sceneDelete':
      return api.narrativeSceneDelete(cmd.id)
    case 'worldRestore':
      return narrativeWorldRestore(cmd.document, cmd.expected).then(() => undefined)
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
  // Beside `links` because it is the same kind of change: an edge off this node
  // that the user placed deliberately and would expect ⌘Z to take back. Missing
  // it does not produce a wrong label, it produces no entry at all — `editEntry`
  // reads the `null` as "nothing happened" — so attaching or reweighting a
  // reference image would be the one edit in the app that undo cannot see.
  if (JSON.stringify(before.assetLinks) !== JSON.stringify(after.assetLinks))
    return 'reference change'
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
    subjectId: node.id,
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
    subjectId: node.id,
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
    subjectId: node.id,
    label: `move “${node.name}”`,
    undo: [{ type: 'move', id: node.id, parentId: node.parentId }],
    redo: [{ type: 'move', id: node.id, parentId }],
    coalesce: false,
  }
}

/**
 * A content edit — rename, notes, description, links, tags, references, cover —
 * which all reach disk the same way and so all invert the same way. `null` when the save
 * changed nothing but the timestamp.
 */
export function editEntry(before: WobuNode, after: WobuNode): NewEntry | null {
  const verb = editLabel(before, after)
  if (!verb) return null
  return {
    subjectId: after.id,
    label: `${verb} “${after.name}”`,
    undo: [{ type: 'upsert', node: before }],
    redo: [{ type: 'upsert', node: after }],
    // The only coalescing case: these are the writes that arrive one per
    // autosave debounce while somebody types.
    coalesce: true,
  }
}

/* ── inverses: narrative source ───────────────────────────────────────────── */

/**
 * The narrative builders, beside the node ones and answering the same question.
 *
 * Every one of them inverts to `sceneSave`, because every narrative edit
 * reaches disk that way — a beat added on the canvas, a beat added in the
 * outline, a line typed in Script and a hand edit in the Source tab are one
 * write, so they are one inverse. That is what makes #186's "indistinguishable
 * in undo history" a property of the design rather than something to test for.
 */

/** What an edit did, and whether a run of them should collapse into one ⌘Z. */
export interface SceneEdit {
  verb: string
  coalesce: boolean
}

/** The wiring: which branches exist, what gates them, where they go. */
function wiringOf(scene: Scene): string {
  return JSON.stringify(
    (scene.beats ?? []).map((beat) => ({
      choices: (beat.choices ?? []).map((c) => ({
        id: c.id,
        requires: c.requires ?? null,
        effects: c.effects ?? [],
        to: c.to,
      })),
      outcomes: (beat.outcomes ?? []).map((o) => ({
        id: o.id,
        when: o.when ?? null,
        effects: o.effects ?? [],
        to: o.to,
      })),
    })),
  )
}

/** Which slots exist and who speaks them, without their words. */
function slotsOf(scene: Scene): string {
  return JSON.stringify(
    (scene.beats ?? []).map((beat) =>
      (beat.dialogue ?? []).map((slot) => ({
        id: slot.id,
        speaker: slot.speaker,
        policy: slot.policy ?? null,
        variants: (slot.variants ?? []).map((v) => ({ id: v.id, when: v.when ?? null })),
      })),
    ),
  )
}

/** Everything a person typed: titles, labels, briefs, intents, lines. */
function proseOf(scene: Scene): string {
  return JSON.stringify(
    (scene.beats ?? []).map((beat) => ({
      title: beat.title,
      intents: beat.intents ?? [],
      mustConvey: beat.must_convey ?? [],
      mustNotReveal: beat.must_not_reveal ?? [],
      labels: (beat.choices ?? []).map((c) => c.label),
      lines: (beat.dialogue ?? []).flatMap((slot) =>
        (slot.variants ?? []).map((v) => [v.text.body, v.text.lifecycle ?? null]),
      ),
    })),
  )
}

/**
 * What an edit did, in the words the toast will use — or `null` when the save
 * changed nothing.
 *
 * Ordered from the most structural to the most textual, because a beat deletion
 * that also changed some wording is a deletion; describing it as a "text edit"
 * would put a label on the stack that badly understates what ⌘Z is about to
 * reverse.
 *
 * Only the textual cases coalesce. Nobody wants three deleted beats to collapse
 * into one ⌘Z, and everybody wants a run of typing to.
 */
export function sceneEditLabel(before: Scene, after: Scene): SceneEdit | null {
  // The `null` case matters for the same reason it does for nodes: an entry
  // whose inverse restores the state it is already in is a ⌘Z that visibly
  // does nothing, which reads as a broken feature.
  if (JSON.stringify(before) === JSON.stringify(after)) return null

  if (before.name !== after.name) return { verb: 'rename', coalesce: false }

  const was = (before.beats ?? []).map((b) => b.id)
  const now = (after.beats ?? []).map((b) => b.id)
  if (now.length > was.length) return { verb: 'add beat', coalesce: false }
  if (now.length < was.length) return { verb: 'delete beat', coalesce: false }
  if (was.join() !== now.join()) return { verb: 'reorder beats', coalesce: false }

  if (wiringOf(before) !== wiringOf(after)) return { verb: 'branch change', coalesce: false }
  if (slotsOf(before) !== slotsOf(after)) return { verb: 'dialogue change', coalesce: false }
  if (proseOf(before) !== proseOf(after)) return { verb: 'text edit', coalesce: true }

  if (JSON.stringify(before.participants ?? []) !== JSON.stringify(after.participants ?? [])) {
    return { verb: 'participant change', coalesce: false }
  }
  if (JSON.stringify(before.entry ?? null) !== JSON.stringify(after.entry ?? null)) {
    return { verb: 'condition change', coalesce: false }
  }
  if ((before.summary ?? '') !== (after.summary ?? '')) {
    return { verb: 'summary edit', coalesce: true }
  }
  return { verb: 'edit', coalesce: false }
}

/** A scene that has just come into existence — created, duplicated, imported. */
export function sceneBirthEntry(file: SceneFile, verb: string): NewEntry {
  return {
    subjectId: file.scene.id,
    label: `${verb} “${file.scene.name}”`,
    undo: [{ type: 'sceneDelete', id: file.scene.id }],
    // Redo through `sceneSave`, not `narrative_scene_create`: create mints a
    // fresh scene id, so a redone create would be a *different* scene, and
    // every entry recorded after it names ids that no longer exist.
    redo: [{ type: 'sceneSave', scene: file.scene, slug: file.slug }],
    coalesce: false,
  }
}

/**
 * A deleted scene.
 *
 * The inverse restores the document — the scene id, and every beat, choice,
 * slot and line id under it — so a destination elsewhere that pointed at it
 * resolves again and a recording filed against one of its lines still matches.
 *
 * The one thing no inverse covers is the arrangement. The delete takes the
 * layout sidecar with it, deliberately and in that order, and a restore has
 * nothing to put back; the scene reopens laid out automatically. That is said
 * out loud when the undo runs rather than quietly discovered.
 */
export function sceneDeletionEntry(file: SceneFile): NewEntry {
  return {
    subjectId: file.scene.id,
    label: `delete “${file.scene.name}”`,
    undo: [{ type: 'sceneSave', scene: file.scene, slug: file.slug }],
    redo: [{ type: 'sceneDelete', id: file.scene.id }],
    coalesce: false,
    caveat: 'Its canvas arrangement went with it and does not come back.',
  }
}

/**
 * Any edit to a scene document, however it was made. `null` when the save
 * changed nothing.
 */
export function sceneEditEntry(before: SceneFile, after: SceneFile): NewEntry | null {
  const edit = sceneEditLabel(before.scene, after.scene)
  if (!edit) return null
  return {
    subjectId: after.scene.id,
    label: `${edit.verb} “${after.scene.name}”`,
    undo: [{ type: 'sceneSave', scene: before.scene, slug: before.slug }],
    redo: [{ type: 'sceneSave', scene: after.scene, slug: after.slug }],
    coalesce: edit.coalesce,
  }
}
