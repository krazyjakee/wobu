import { call } from './call'
/* ── domain types ─────────────────────────────────────────────────────────── */

/**
 * The narrative command surface: scenes, declared state, diagnostics, layout.
 *
 * Two spellings meet in this file and the split is deliberate. Every *envelope*
 * — `SceneFile`, `SceneCatalog`, `NarrativeDiagnostic`, `LayoutLoad` — is
 * camelCase, like the rest of `lib/api`. Every *source document* inside one —
 * `Scene`, `Beat`, `StateDocument` — is snake_case, because it is the YAML in
 * `narrative/scenes/*.yaml` verbatim rather than a rendering of it. The Source
 * tab edits that YAML directly, so a camelCase mirror here would give the
 * Script tab and the Source tab two vocabularies for one file, and every save
 * would have to translate between them exactly right or lose a field.
 *
 * The Rust side of the same argument is in `src-tauri/src/commands/narrative.rs`,
 * and a Rust test pins the spelling so nobody tidies it away.
 *
 * Nothing here is generated. `commands/narrative.rs` and this file are kept in
 * step by tests on both sides, so a change to one is a change to both.
 */

/* ── identities ───────────────────────────────────────────────────────────── */

/**
 * Stable identities. Each is a ULID string, and each names a *slot* that
 * survives renaming, reordering and rewriting; duplicating mints a new one.
 * Distinct aliases rather than a shared `string` so a signature says which one
 * it wants, even though TypeScript will not enforce it.
 */
export type SceneId = string
export type BeatId = string
export type ChoiceId = string
export type OutcomeId = string
export type DialogueSlotId = string
export type VariantId = string
export type TextAssetId = string
export type TextEntryId = string

/**
 * A content hash over some wording, not an identity: it changes on every edit.
 * A translation, a recording and an approval are all keyed to one, so nothing
 * on this side ever constructs or edits one.
 */
export type Revision = string

/** A declared identifier — a variable, an enum member, a host command. */
export type StateName = string

/* ── declared state ───────────────────────────────────────────────────────── */

/** A literal. Untagged in the source file: `40`, `true`, `investigating`. */
export type StateValue = boolean | number | StateName

export type VarType =
  'bool' | { enum: { members: StateName[] } } | { int: { min: number; max: number } }

/** Who may write a variable. `host` variables are readable and never assigned. */
export type StateOwner = 'narrative' | 'host'

export interface VariableDecl {
  name: StateName
  type: VarType
  /** Required, not defaulted: "what is `trust` at the start" has one answer. */
  default: StateValue
  owner?: StateOwner
  description?: string
}

export interface StateDocument {
  schema_version: number
  variables: VariableDecl[]
}

/* ── conditions and effects ───────────────────────────────────────────────── */

export type CompareOp = 'eq' | 'ne' | 'lt' | 'le' | 'gt' | 'ge'

/** Explicitly tagged: `{ var: 'trust' }` is never confused with a member name. */
export type Operand = { literal: StateValue } | { var: StateName }

export interface Comparison {
  var: StateName
  op: CompareOp
  value: Operand
}

export type Condition =
  | 'always'
  /** A branch the author closed off deliberately — not an unsatisfiable compare. */
  | 'never'
  | { not: Condition }
  | { all: Condition[] }
  | { any: Condition[] }
  | { compare: Comparison }

export interface Assignment {
  var: StateName
  value: Operand
}

/** `support +10`. Deliberately not `set x = x + 10`: operands have no arithmetic. */
export interface Increment {
  var: StateName
  by: number
}

export interface HostCommand {
  name: StateName
  args?: Operand[]
}

export type Effect = { set: Assignment } | { add: Increment } | { command: HostCommand }

/* ── the scene document ───────────────────────────────────────────────────── */

/** The player, unattributed narration, or a character already in the project. */
export type Speaker = 'player' | 'narrator' | { entity: string }

export interface Participant {
  entity: string
  /** "accuser", "witness". Free text; nothing branches on it. */
  role?: string
}

/** Prose, and inert: nothing evaluates or matches an intent. */
export interface Intent {
  subject: Speaker
  intent: string
}

export type Provenance =
  'human' | { generated: { fingerprint: string } } | { imported: { source: string } }

export type GenerationPolicy = 'generated' | 'edited' | 'locked'
export type ReviewState = 'draft' | 'approved'
export type Freshness = 'current' | 'out_of_date'

export interface ContentLifecycle {
  policy?: GenerationPolicy
  review?: ReviewState
  freshness?: Freshness
}

export interface Text {
  /** Stored, not recomputed: it is what an approval and a locale row are keyed to. */
  revision: Revision
  body: string
  provenance?: Provenance
  lifecycle?: ContentLifecycle
}

export interface Variant {
  id: VariantId
  /** Absent is the unconditional variant — the one used when nothing else applies. */
  when?: Condition
  text: Text
}

export interface DialogueSlot {
  id: DialogueSlotId
  speaker: Speaker
  policy?: GenerationPolicy
  /** Empty is a slot with no text yet, shown as a task rather than filled in. */
  variants?: Variant[]
}

/**
 * Where the story goes next. Three cases and no fourth — there is no "fall
 * through to the next beat", because an implicit destination would mean
 * reordering beats silently rewired the story.
 */
export type Destination =
  | { beat: BeatId }
  | { scene: SceneId }
  | { end: { label?: string } }
  | { unresolved: Record<string, never> }

export interface Choice {
  id: ChoiceId
  label: string
  /** Absent is the "always" row of the choice table. */
  requires?: Condition
  effects?: Effect[]
  to: Destination
}

/** An automatic transition: what happens when the player is not being asked. */
export interface Outcome {
  id: OutcomeId
  when?: Condition
  effects?: Effect[]
  to: Destination
}

export interface Beat {
  id: BeatId
  /** Display only. No id anywhere is derived from it. */
  title: string
  intents?: Intent[]
  must_convey?: string[]
  must_not_reveal?: string[]
  dialogue?: DialogueSlot[]
  choices?: Choice[]
  outcomes?: Outcome[]
}

export type TombstoneTarget =
  { beat: BeatId } | { dialogue_slot: DialogueSlotId } | { variant: VariantId }

/**
 * What a deleted element was, so a reference to it can still be explained.
 * Without one, a destination naming a removed beat can only say "unknown beat
 * 01J8…", which tells a writer nothing about what to do.
 */
export interface Tombstone {
  target: TombstoneTarget
  label: string
  deleted_at: string
  reason?: string
}

export interface Scene {
  act_id?: string
  arc_id?: string
  tag_ids?: string[]
  editorial_head?: string | null
  id: SceneId
  name: string
  summary?: string
  participants?: Participant[]
  /** Absent means unconditional, which is a different claim from `'never'`. */
  entry?: Condition
  /** The array *is* the order. There is no index field to disagree with it. */
  beats?: Beat[]
  tombstones?: Tombstone[]
}

/* ── the guarded write ────────────────────────────────────────────────────── */

/**
 * What a file looked like when it was read.
 *
 * Opaque. Carry it, hand it back, never inspect it and never build one — the
 * fields exist so it can cross the bridge, not so anything here can reason
 * about them.
 */
export interface Stamp {
  mtime_ms: number
  size: number
  hash: string
}

/**
 * Which version of a file a write expects to be replacing.
 *
 * Three named cases rather than an optional stamp, because "I hold no stamp"
 * and "whatever is there now" are opposite intentions and one of them silently
 * overwrites somebody's work.
 */
export type Precondition =
  /** The file as this caller last read it. Anything since is a conflict. */
  | { kind: 'stamp'; stamp: Stamp }
  /** The caller believes there is no file. Anything there is a conflict. */
  | { kind: 'new' }
  /**
   * Whatever is on disk at the moment of the write. The undo path, and the
   * exact analogue of `node_upsert` reading the stamp out of the index —
   * see `lib/undo.ts`, which is the only caller that should send it.
   */
  | { kind: 'current' }

/** The precondition for saving a file back, derived from how it was read. */
export function preconditionOf(stamp: Stamp | null | undefined): Precondition {
  return stamp ? { kind: 'stamp', stamp } : { kind: 'new' }
}

/* ── scenes ───────────────────────────────────────────────────────────────── */

export interface SceneSummary {
  id: SceneId
  name: string
  /** The file name stem, minted at create and stable across a rename. */
  slug: string
  /** Project-relative, `/`-separated. */
  rel: string
}

/**
 * A scene file that is on disk and could not be identified — a sync client
 * copied it half-written, most likely. Listed rather than dropped: a catalog
 * that omitted it would present somebody's file as deleted.
 */
export interface UnreadableScene {
  rel: string
  reason: string
}

export interface SceneCatalog {
  scenes: SceneSummary[]
  unreadable: UnreadableScene[]
}

/** One scene, whole, with the precondition for writing it back. */
export interface SceneFile {
  scene: Scene
  slug: string
  rel: string
  /** `null` means "we believe there is no file". */
  stamp: Stamp | null
}

export const narrativeScenes = () => call<SceneCatalog>('narrative_scenes')

export const narrativeSceneGet = (sceneId: SceneId) =>
  call<SceneFile>('narrative_scene_get', { sceneId })

export const narrativeSceneCreate = (name: string) =>
  call<SceneFile>('narrative_scene_create', { name })

/**
 * Change a scene's display name and nothing else.
 *
 * No precondition argument, and that is not an omission: the Library row that
 * offers Rename holds a name and an id, never a version of the file, so
 * whatever it could send would be stale by definition. The backend reads the
 * precondition in the same critical section as the write.
 */
export const narrativeSceneRename = (sceneId: SceneId, name: string) =>
  call<SceneFile>('narrative_scene_rename', { sceneId, name })

export const narrativeSceneDelete = (sceneId: SceneId) =>
  call<void>('narrative_scene_delete', { sceneId })

/**
 * Write a whole scene document.
 *
 * The one write path for narrative source, which is what makes a canvas edit
 * and the equivalent form edit produce identical source rather than merely
 * similar source. `slug` is only consulted for a scene the project does not
 * currently have — undo restoring one that was deleted — and is re-slugified
 * and made unique on the far side, so it can never land on top of a different
 * scene that took the name.
 */
export const narrativeSceneSave = (scene: Scene, expected: Precondition, slug?: string | null) =>
  call<SceneFile>('narrative_scene_save', { scene, expected, slug: slug ?? null })

/* ── declared state ───────────────────────────────────────────────────────── */

export interface StateFile {
  document: StateDocument
  /** `null` in a project that has never declared any state. */
  stamp: Stamp | null
}

export const narrativeStateGet = () => call<StateFile>('narrative_state_get')

/**
 * Write the declared variables. Refused if they do not hold together — a name
 * declared twice, a default outside its own range — because a broken schema
 * makes every scene's diagnostics wrong at once and there is nowhere to say so.
 */
export const narrativeStateSave = (document: StateDocument, expected: Precondition) =>
  call<StateFile>('narrative_state_save', { document, expected })

/* ── diagnostics ──────────────────────────────────────────────────────────── */

/** Which element a diagnostic is about, and therefore which id below to select. */
export type DiagnosticKind =
  | 'scene'
  | 'entry'
  | 'participant'
  | 'choice'
  | 'outcome'
  | 'beat'
  | 'intent'
  | 'dialogueSlot'
  | 'variant'
  /** Supporting text assets (#167) — see `lib/api/narrativeText.ts`. */
  | 'textAsset'
  | 'textTrigger'
  | 'textEntry'
  | 'textLine'
  | 'textVariant'

/**
 * A stable machine-readable name for a problem, distinct from its `message`,
 * which is prose and may be reworded. Filters and badges switch on this.
 *
 * Left open with `(string & {})` for the same reason `ErrorCode` is: a build
 * meeting a code it has never heard of has to fall back to showing the message
 * rather than failing to type-check.
 */
export type DiagnosticCode =
  | 'dangling_beat'
  | 'deleted_beat'
  | 'unknown_scene'
  | 'no_destination'
  | 'no_beats'
  | 'type_error'
  | 'not_a_participant'
  | 'missing_text'
  | 'revision_mismatch'
  | 'duplicate_id'
  | 'no_text_entries'
  | 'prose_has_cast'
  | 'voice_not_allowed'
  | 'wrong_line_count'
  | (string & {})

/**
 * One problem, flattened onto the ids that identify the thing responsible.
 *
 * Flat on purpose: the canvas keys nodes by beat/choice/outcome id and the
 * Validation list keys rows by the same, so they join on identical fields
 * rather than on two careful readings of a nested union.
 */
export interface NarrativeDiagnostic {
  kind: DiagnosticKind
  code: DiagnosticCode
  /** The backend's own wording. Not restated on this side. */
  message: string
  /** One of the destination problems, so the canvas can draw broken wiring. */
  destination: boolean
  beatId?: BeatId
  choiceId?: ChoiceId
  outcomeId?: OutcomeId
  slotId?: DialogueSlotId
  variantId?: VariantId
  entityId?: string
  /** Intents have no id; their position in the beat's list addresses them. */
  intentIndex?: number
  /**
   * The supporting text entry a problem belongs to (#167). A separate field
   * from `beatId` because the two open different editors, and one field holding
   * either would make choosing between them a guess.
   */
  entryId?: TextEntryId
}

/**
 * What is wrong with one scene.
 *
 * Pass `scene` to diagnose the document as the writer currently has it, unsaved
 * edits and all. Without it a destination the writer connected thirty seconds
 * ago would still be reported as broken, and they would learn to ignore the
 * list. The scene catalog and the state schema always come from the project, so
 * a cross-scene link is checked against the scenes that really exist.
 */
export const narrativeDiagnostics = (sceneId: SceneId, scene?: Scene | null) =>
  call<NarrativeDiagnostic[]>('narrative_diagnostics', { sceneId, scene: scene ?? null })

/* ── layout ───────────────────────────────────────────────────────────────── */

/**
 * Which graph an arrangement is for. An arc is keyed by a slug because arcs
 * have no source document yet, so there is no stable id to key one to.
 */
export type GraphKey =
  | { kind: 'scene'; scene: SceneId }
  | { kind: 'arc'; arc: string }
  | { kind: 'quest'; quest: string }

/** `beat:01J…`, `choice:01J…`. A tagged id, so a beat cannot be read as a scene. */
export type NodeKey = string

export type LayoutMode = 'automatic' | 'manual'

export interface NodeLayout {
  x: number
  y: number
  collapsed?: boolean
  group?: string
  /** By the clock of the machine that set it. The whole of the merge rule. */
  updatedAt: string
}

export interface LayoutGroup {
  id: string
  label?: string
  collapsed?: boolean
  members?: NodeKey[]
  updatedAt: string
}

export interface LayoutAnnotation {
  id: string
  body?: string
  x: number
  y: number
  width?: number
  height?: number
  attachedTo?: NodeKey
  updatedAt: string
}

/**
 * One layout file. Nothing in here is story: it is where the boxes are, and it
 * lives in `narrative/layout/` precisely so that everything which must never
 * see a coordinate — the compiler, the build fingerprint, an export — excludes
 * one directory rather than remembering a filename rule.
 */
export interface Layout {
  schemaVersion: number
  graph: GraphKey
  mode: LayoutMode
  /** Stamped apart from the nodes: switching to manual and dragging are two edits. */
  modeUpdatedAt: string
  nodes: Record<NodeKey, NodeLayout>
  groups: Record<string, LayoutGroup>
  annotations: Record<string, LayoutAnnotation>
  removedGroups?: Record<string, string>
  removedAnnotations?: Record<string, string>
}

/**
 * Something the canvas should know and must not be stopped by.
 *
 * `blocking` is the backend's own answer rather than a constant here. It is
 * false for every notice today; carrying it means that if a blocking case is
 * ever added, this side reports it instead of continuing to promise it cannot
 * happen.
 */
export interface LayoutNotice {
  kind: 'missing' | 'unreadable' | 'newerSchema' | 'wrongGraph' | 'unplaced' | 'stale'
  blocking: boolean
  rel?: string
  reason?: string
  found?: number
  supported?: number
  /** Node keys, in the canvas's own `beat:01J…` vocabulary. */
  nodes?: NodeKey[]
}

export interface LayoutLoad {
  layout: Layout
  notices: LayoutNotice[]
}

/**
 * What happened to an arrangement.
 *
 * An outcome, never a rejection. The canvas autosaves one save per drag, so a
 * failure that arrived as an error would put a toast on screen per rectangle —
 * and none of them would be about anything the writer could lose.
 */
export type LayoutSave =
  | { outcome: 'written' }
  /** A sidecar written by a newer Wobu. Left alone rather than downgraded. */
  | { outcome: 'deferred'; rel: string; found: number; supported: number }
  /** Nothing reached disk; the arrangement is only in this session. */
  | { outcome: 'unwritable'; reason: string }

export const narrativeLayoutGet = (graph: GraphKey) =>
  call<LayoutLoad>('narrative_layout_get', { graph })

export const narrativeLayoutSave = (layout: Layout) =>
  call<LayoutSave>('narrative_layout_save', { layout })

/** The cache-key form of a graph: one string per canvas, stable across renders. */
export function graphKeyId(graph: GraphKey): string {
  return graph.kind === 'scene'
    ? `scene:${graph.scene}`
    : graph.kind === 'quest'
      ? `quest:${graph.quest}`
      : `arc:${graph.arc}`
}

/** Prepare handwritten text with the backend's canonical content revision. */
export const narrativeTextWritten = (body: string, locked = false) =>
  call<Text>('narrative_text_written', { body, locked })

export const narrativeSceneRestore = (scene: Scene, expected: Scene | null, slug: string) =>
  call<SceneFile>('narrative_scene_restore', { scene, expected, slug })
