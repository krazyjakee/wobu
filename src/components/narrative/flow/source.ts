import type {
  Beat,
  Condition,
  Destination,
  Effect,
  Layout,
  Operand,
  Scene,
  Speaker,
  Tombstone,
} from '../../../lib/api'
import type {
  FlowEffect,
  FlowElement,
  FlowGroup,
  FlowLevel,
  FlowPort,
  FlowScene,
  FlowWorkCounts,
} from './model'
import type { NarrativeStatus } from '../narrativeModel'

/**
 * The seam between the narrative source document and the canvas view model.
 *
 * ── the one rule this file exists to keep ────────────────────────────────────
 *
 * **The canvas view model is lossy, so the way back is a patch and never a
 * rebuild.** A `FlowBeat` holds `lines: 12`, not twelve lines. It has no
 * dialogue, no variants, no revisions, no provenance, no intents, no
 * `must_not_reveal`, no typed conditions and no typed effects — because a
 * canvas that carried those would be a second copy of the story, and a scene
 * with 300 boxes would carry all of it into React Flow's store.
 *
 * So [`patchScene`] never constructs a `Scene` from a `FlowLevel`. It takes the
 * document that was loaded, works out what the canvas *did* by comparing the
 * level against [`sceneToFlow`] of that same document, and applies exactly
 * those operations to the loaded document. Everything the canvas does not model
 * is not preserved by care; it is preserved because nothing here ever reads or
 * writes it.
 *
 * If that ever becomes a reconstruction, the first drag a writer makes deletes
 * every line of dialogue in the scene. `source.test.ts` holds it to a
 * deep-equality round trip so the day it happens is the day the suite goes red.
 *
 * ── what the canvas can and cannot say ───────────────────────────────────────
 *
 * The source model's [`Destination`] has three cases — a beat, a scene, an
 * ending — and deliberately no fourth. There is no "nowhere". The canvas has
 * one: a `FlowPort` with `to: null`. That asymmetry is not a bug on either
 * side, and it is the reason a port on a source-backed scene cannot be cleared:
 * doing so would have to mean *deleting the choice*, which would throw away its
 * label, its condition and its effects. `flowAuthoring` says so out loud and
 * the edit is refused with that sentence.
 *
 * Likewise the canvas's `choice` and `outcome` boxes are not free-standing
 * elements: a `Choice` lives inside a `Beat`, and creating one requires a
 * destination the writer has not chosen yet. So they are created by *connecting
 * from a beat's spare handle*, where the gesture itself supplies the
 * destination, rather than by an Add button that would have to invent one.
 *
 * ── ids are layout keys ──────────────────────────────────────────────────────
 *
 * A node's id is the layout file's own `NodeKey` spelling — `beat:01J…`,
 * `choice:01J…`, `outcome:01J…` — so #185's stored coordinates need no
 * translation table to line up with the boxes they describe. `beatId` is the
 * bare narrative id, because that is what the shared selection, Script and the
 * diagnostics all key on.
 *
 * An `end` or `sceneLink` box is *derived*: it is a picture of a field on the
 * choice or outcome that leads to it, not an element of its own. Its id names
 * that holder (`end:choice:01J…`), it has no layout key, and it cannot be
 * deleted or re-pointed — see `derived` on `FlowElementBase`.
 */

/* ── ids ──────────────────────────────────────────────────────────────────── */

/** A ULID, as the Rust side writes and parses one. Nothing else is an id. */
const ULID = /^[0-9A-HJKMNP-TV-Z]{26}$/

export const nodeId = {
  beat: (id: string) => `beat:${id}`,
  choice: (id: string) => `choice:${id}`,
  outcome: (id: string) => `outcome:${id}`,
  /** The ending a choice or an outcome leads to, named after that holder. */
  end: (holder: string) => `end:${holder}`,
  /** The way out of the scene a choice or an outcome states. */
  link: (holder: string) => `link:${holder}`,
}

/** The narrative id inside a node id, or null when the node is derived. */
export function narrativeIdOf(node: string): { kind: string; id: string } | null {
  const at = node.indexOf(':')
  if (at < 0) return null
  const kind = node.slice(0, at)
  const id = node.slice(at + 1)
  if (!ULID.test(id)) return null
  return { kind, id }
}

/**
 * The layout file's key for a canvas node, or null when it has none.
 *
 * Null is not an oversight: an `end` box is a picture of a destination field
 * and a dangling arc box is a picture of an id that does not resolve, so
 * neither has an identity a coordinate could be filed under. Sending one anyway
 * would be rejected by `NodeKey`'s own parser at the bridge, and the layout
 * save that #185 promises can never fail would start failing.
 */
export function layoutKeyOf(node: string, level: 'scene' | 'arc'): string | null {
  if (level === 'arc') return ULID.test(node) ? `scene:${node}` : null
  const parsed = narrativeIdOf(node)
  if (parsed === null) return null
  return ['beat', 'choice', 'outcome'].includes(parsed.kind) ? node : null
}

/** The canvas node a stored coordinate belongs to, or null for a key we do not draw. */
export function nodeOfLayoutKey(key: string, level: 'scene' | 'arc'): string | null {
  const parsed = narrativeIdOf(key)
  if (parsed === null) return null
  if (level === 'arc') return parsed.kind === 'scene' ? parsed.id : null
  return ['beat', 'choice', 'outcome'].includes(parsed.kind) ? key : null
}

/**
 * The port a beat offers for a way out it does not have yet.
 *
 * Fixed id, because it is the same offer every time and `connectPort` appends a
 * port under exactly this name — which is how [`patchScene`] recognises the
 * gesture as "author an outcome leading there" rather than as an edit to a port
 * that already exists.
 */
export const NEW_OUTCOME_PORT = 'outcome:new'

/**
 * The coordinates the canvas should open with, from a loaded sidecar.
 *
 * Keys that name something this level does not draw are dropped rather than
 * carried: the backend already reconciles a scene's sidecar against the scene,
 * so anything left over is a key from a *newer* Wobu, and drawing a box at it
 * is not something this build can do.
 */
export function positionsFromLayout(
  layout: Layout | null | undefined,
  level: 'scene' | 'arc',
): Record<string, { x: number; y: number }> {
  const out: Record<string, { x: number; y: number }> = {}
  for (const [key, node] of Object.entries(layout?.nodes ?? {})) {
    const id = nodeOfLayoutKey(key, level)
    if (id !== null) out[id] = { x: node.x, y: node.y }
  }
  return out
}

/**
 * The layout to save after a drag, from the one that was loaded.
 *
 * Two things it deliberately does:
 *
 * - **Drops every box with no layout key.** An ending and a scene link are
 *   pictures of a destination field, so they have no identity to file a
 *   coordinate under. Sending one would fail `NodeKey`'s own parser at the
 *   bridge — and #185's promise is that a layout save can never fail, so the
 *   filter is what keeps that true rather than a `catch` that hides it.
 * - **Stamps only what moved.** `updatedAt` is the whole of the merge rule on
 *   the far side, so re-stamping a box nobody touched would let this machine
 *   win an argument it was not having with a collaborator.
 */
export function layoutWithPositions(
  layout: Layout,
  positions: Readonly<Record<string, { x: number; y: number }>>,
  level: 'scene' | 'arc',
  now = new Date().toISOString(),
): Layout {
  const nodes = { ...layout.nodes }
  for (const [id, at] of Object.entries(positions)) {
    const key = layoutKeyOf(id, level)
    if (key === null) continue
    const existing = nodes[key]
    if (existing && existing.x === at.x && existing.y === at.y) continue
    nodes[key] = { ...existing, x: at.x, y: at.y, updatedAt: now }
  }
  return { ...layout, nodes }
}

/* ── minting ──────────────────────────────────────────────────────────────── */

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'

/**
 * A fresh ULID, in the spelling `wobu_core::Id` parses.
 *
 * Minted here rather than asked for, because the command surface is coarse on
 * purpose: a scene is saved as a whole document, so there is no
 * `narrative_beat_add` to hand back an id, and adding one would be the first
 * step towards two implementations of every structural edit. The identity is
 * short-lived either way — the save returns the document the backend wrote, and
 * the canvas redraws from that.
 */
export function mintId(now = Date.now(), random = crypto.getRandomValues.bind(crypto)): string {
  let time = ''
  let remaining = now
  for (let index = 0; index < 10; index++) {
    time = CROCKFORD[remaining % 32] + time
    remaining = Math.floor(remaining / 32)
  }
  const bytes = random(new Uint8Array(16))
  let tail = ''
  for (const byte of bytes) tail += CROCKFORD[byte % 32]
  return time + tail
}

/* ── conditions and effects, in words ─────────────────────────────────────── */

const COMPARE: Record<string, string> = {
  eq: '=',
  ne: '≠',
  lt: '<',
  le: '≤',
  gt: '>',
  ge: '≥',
}

function operandText(operand: Operand): string {
  return 'literal' in operand ? String(operand.literal) : operand.var
}

/**
 * A typed condition as a writer's sentence.
 *
 * One direction only. Nothing here parses a sentence back into a condition, and
 * nothing should: prose that could author an executable branch is precisely
 * what #151 forbids, and a round trip through a string is how a `never` a
 * writer set deliberately turns into an unsatisfiable comparison.
 */
export function conditionText(condition: Condition | null | undefined): string | null {
  if (condition === null || condition === undefined) return null
  if (condition === 'always') return 'always'
  if (condition === 'never') return 'never'
  if ('not' in condition) return `not (${conditionText(condition.not)})`
  if ('all' in condition) return condition.all.map(conditionText).join(' and ')
  if ('any' in condition) return condition.any.map(conditionText).join(' or ')
  const { var: name, op, value } = condition.compare
  return `${name} ${COMPARE[op] ?? op} ${operandText(value)}`
}

/** The rows an outcome or a choice shows: variable, operator, value. */
export function effectRows(effects: readonly Effect[] | undefined): FlowEffect[] {
  return (effects ?? []).map((effect) => {
    if ('set' in effect) {
      return {
        variable: effect.set.var,
        operator: '=',
        value: operandText(effect.set.value),
      }
    }
    if ('add' in effect) {
      // The minus is U+2212, matching the fixture and the stylesheet: a hyphen
      // at this size is indistinguishable from the plus beside it.
      return {
        variable: effect.add.var,
        operator: effect.add.by < 0 ? '−' : '+',
        value: String(Math.abs(effect.add.by)),
      }
    }
    return {
      variable: effect.command.name,
      operator: 'call',
      value: (effect.command.args ?? []).map(operandText).join(', '),
    }
  })
}

/* ── the lifecycle, as three counts that are never collapsed ──────────────── */

/**
 * What a beat's text is waiting for, counted per dimension.
 *
 * Three independent answers, exactly as `lifecycle.rs` keeps three independent
 * fields. A locked line that no longer fits its context is *both* locked and
 * out of date, and a summary that had to pick one would hide whichever it did
 * not pick — which is the specific mistake US-06 is about ("freshness is never
 * reset merely because a line was locked").
 */
export function beatCounts(beat: Beat): FlowWorkCounts {
  const counts: FlowWorkCounts = { needsText: 0, needsReview: 0, outOfDate: 0, locked: 0 }
  for (const slot of beat.dialogue ?? []) {
    // A slot with no wording is one piece of missing text, not zero: the task
    // is the slot, and counting its (absent) variants would count nothing.
    if ((slot.variants ?? []).length === 0) counts.needsText += 1
    if (slot.policy === 'locked') counts.locked += 1
    for (const variant of slot.variants ?? []) {
      const lifecycle = variant.text.lifecycle
      if ((lifecycle?.review ?? 'draft') === 'draft') counts.needsReview += 1
      if (lifecycle?.freshness === 'out_of_date') counts.outOfDate += 1
      if (slot.policy !== 'locked' && lifecycle?.policy === 'locked') counts.locked += 1
    }
  }
  return counts
}

/**
 * The single-valued status the *participant and work filters* switch on.
 *
 * Deliberately a lossy summary, and deliberately not what the badges show. The
 * badges read `counts` and show each dimension separately; this exists because
 * `NarrativeStatus` is one value and a chip row is one row. Nothing derived
 * from it is ever shown as "the state of this beat".
 */
export function summaryStatus(counts: FlowWorkCounts): NarrativeStatus {
  if (counts.needsText > 0) return 'needsText'
  if (counts.outOfDate > 0) return 'outOfDate'
  if (counts.needsReview > 0) return 'needsReview'
  if (counts.locked > 0) return 'locked'
  return 'ready'
}

/* ── speakers ─────────────────────────────────────────────────────────────── */

function speakerName(speaker: Speaker, nameOf?: (entity: string) => string | undefined): string {
  if (speaker === 'player') return 'Player'
  if (speaker === 'narrator') return 'Narrator'
  // The entity id, when nothing can resolve it. Not a guessed name: a plausible
  // one would be indistinguishable from a real one and wrong.
  return nameOf?.(speaker.entity) ?? speaker.entity
}

/** Who is heard in this beat: the people who speak, and whose intent is stated. */
function participantsOf(beat: Beat, nameOf?: (entity: string) => string | undefined): string[] {
  const names = new Set<string>()
  for (const slot of beat.dialogue ?? []) names.add(speakerName(slot.speaker, nameOf))
  for (const intent of beat.intents ?? []) names.add(speakerName(intent.subject, nameOf))
  return [...names].sort()
}

/* ── forward: document to canvas ──────────────────────────────────────────── */

export interface SceneToFlowOptions {
  /** #185's sidecar, for the containers a scene's boxes are grouped into. */
  layout?: Layout | null
  /** Resolves a world entity id to its name. Absent leaves the id showing. */
  nameOf?: (entity: string) => string | undefined
  /** Resolves a scene id to its name, for the boxes that leave this scene. */
  sceneName?: (scene: string) => string | undefined
}

/**
 * One scene document, as the boxes and wires that describe it.
 *
 * Pure, total and independent of React: this is the function the round-trip
 * tests pin, so it must be possible to call it with nothing but a document.
 */
export function sceneToFlow(scene: Scene, options: SceneToFlowOptions = {}): FlowScene {
  const elements: FlowElement[] = []
  const groupOf = layoutGroups(options.layout)

  for (const beat of scene.beats ?? []) {
    const beatNode = nodeId.beat(beat.id)
    const out: FlowPort[] = []
    const counts = beatCounts(beat)
    const lines = (beat.dialogue ?? []).length
    const variants = (beat.dialogue ?? []).reduce((sum, s) => sum + (s.variants ?? []).length, 0)

    for (const choice of beat.choices ?? []) {
      const node = nodeId.choice(choice.id)
      out.push({
        id: node,
        label: choice.label,
        // On the beat's own port row, where the doc wants it: the requirement
        // stays beside the option it gates however the graph is laid out.
        requires: conditionText(choice.requires),
        to: node,
        fixed: true,
      })
      elements.push({
        kind: 'choice',
        id: node,
        title: choice.label,
        beatId: beat.id,
        groupId: groupOf(node),
        effects: effectRows(choice.effects),
        out: [destinationPort(node, choice.to)],
      })
      pushDestinationBox(elements, node, choice.to, beat.id, groupOf, options.sceneName)
    }

    for (const outcome of beat.outcomes ?? []) {
      const node = nodeId.outcome(outcome.id)
      const when = conditionText(outcome.when)
      out.push({
        id: node,
        label: when === null ? 'Automatically' : `When ${when}`,
        requires: when,
        to: node,
        fixed: true,
      })
      elements.push({
        kind: 'outcome',
        id: node,
        // An outcome has no name in the source model and inventing one would be
        // inventing content, so it is named by the thing that decides it.
        title: when === null ? 'Automatically' : `When ${when}`,
        beatId: beat.id,
        groupId: groupOf(node),
        effects: effectRows(outcome.effects),
        out: [destinationPort(node, outcome.to)],
      })
      pushDestinationBox(elements, node, outcome.to, beat.id, groupOf, options.sceneName)
    }

    elements.push({
      kind: 'beat',
      id: beatNode,
      title: beat.title,
      beatId: beat.id,
      groupId: groupOf(beatNode),
      participants: participantsOf(beat, options.nameOf),
      lines,
      variants,
      counts,
      // A beat with no dialogue at all is waiting for its text as much as one
      // with an empty slot in it, and neither is "ready".
      status: lines === 0 ? 'needsText' : summaryStatus(counts),
      out,
    })
  }

  const first = (scene.beats ?? [])[0]
  return {
    id: scene.id,
    name: scene.name,
    groups: layoutGroupList(options.layout),
    elements,
    // The scene starts at its first beat. There is no `entry beat` field to
    // disagree with the order, exactly as there is no `order` field on a beat.
    entryId: first ? nodeId.beat(first.id) : null,
  }
}

/** The port a choice or an outcome states its destination through. */
function destinationPort(holder: string, to: Destination): FlowPort {
  return { id: 'then', label: 'Then', to: destinationNode(holder, to) }
}

/**
 * The node a destination points at.
 *
 * A beat destination points at the beat's box whether or not that beat is in
 * the scene: a dangling one draws no edge and the backend's `dangling_beat`
 * diagnostic is what names it, which is better than this file inventing a
 * second opinion about what is missing.
 */
function destinationNode(holder: string, to: Destination): string {
  if ('beat' in to) return nodeId.beat(to.beat)
  if ('scene' in to) return nodeId.link(holder)
  return nodeId.end(holder)
}

/** The derived box an ending or a way out of the scene is drawn as. */
function pushDestinationBox(
  elements: FlowElement[],
  holder: string,
  to: Destination,
  beatId: string,
  groupOf: (node: string) => string | null,
  sceneName?: (scene: string) => string | undefined,
): void {
  if ('beat' in to) return
  if ('scene' in to) {
    elements.push({
      kind: 'sceneLink',
      id: nodeId.link(holder),
      title: sceneName?.(to.scene) ?? to.scene,
      beatId,
      groupId: groupOf(holder),
      derived: true,
      targetSceneId: to.scene,
      out: [],
    })
    return
  }
  elements.push({
    kind: 'end',
    id: nodeId.end(holder),
    // An ending with no label is still an ending, and saying "End" is a
    // rendering of the empty label rather than a name being written into it —
    // `patchScene` never reads a title back.
    title: to.end.label || 'End',
    beatId,
    groupId: groupOf(holder),
    derived: true,
    out: [],
  })
}

/* ── groups come from the arrangement, never from the source ─────────────── */

function layoutGroupList(layout: Layout | null | undefined): FlowGroup[] {
  return Object.values(layout?.groups ?? {}).map((group) => ({
    id: group.id,
    name: group.label ?? group.id,
  }))
}

/**
 * Which container each box sits in, read out of the layout sidecar.
 *
 * Grouping is presentation and lives in `narrative/layout/`, so a scene
 * document has no `groups` and never gains one: a container is a thing a reader
 * drew around some boxes, and putting it in the source would send it to the
 * compiler, the build fingerprint and the exported package.
 */
function layoutGroups(layout: Layout | null | undefined): (node: string) => string | null {
  if (!layout) return () => null
  const owner = new Map<string, string>()
  for (const group of Object.values(layout.groups ?? {})) {
    for (const member of group.members ?? []) owner.set(member, group.id)
  }
  return (node) => owner.get(node) ?? null
}

/* ── back: the patch ──────────────────────────────────────────────────────── */

/**
 * What a canvas edit is allowed to be, said in the words the refusal uses.
 *
 * Exported so the pane can hand the same sentences to `useSceneEdits` and to
 * the reader, rather than the rule living in one place and its explanation in
 * another.
 */
export const flowAuthoring = {
  /**
   * Why a destination cannot be cleared on a source-backed scene.
   *
   * This is the source model speaking, not a limitation of this build:
   * `Destination` has a beat, a scene and an ending, and no fourth case,
   * because an implicit "nowhere" would make an unfinished branch and a
   * finished one look the same.
   */
  destinationRequired:
    'Every choice and outcome has to lead somewhere — a beat, another scene, or an ending. ' +
    'Point it somewhere else, or delete it.',
  /** Why a beat's own ways out cannot be re-wired directly. */
  fixedPort:
    'A beat leads to its own choices and outcomes. Move the wire that leaves the choice or the ' +
    'outcome instead.',
  /** Why an ending or a scene link is not a box you can delete. */
  derived:
    'This box is a picture of where a choice or an outcome leads. Change it on that choice or ' +
    'outcome, or delete that instead.',
  /** Why the arc cannot be rewired from the arc. */
  arcReadOnly:
    'A scene’s exits are authored on a beat inside it. Open the scene to change where it leads.',
} as const

/** What [`patchScene`] did, for a caller that has to follow the new element. */
export interface ScenePatch {
  scene: Scene
  /** Canvas ids of elements this patch brought into existence, freshly minted. */
  created: string[]
  /** True when the patch changed nothing, so the save can be skipped. */
  unchanged: boolean
}

/**
 * Apply a canvas edit to the document it was made against.
 *
 * **Read the file header before changing anything here.** This is the function
 * that could silently delete a project's dialogue, and the only thing that
 * stops it is that it never builds a `Scene` — it clones the one it was given
 * and writes exactly six things:
 *
 *   1. removes a beat from `beats`, leaving tombstones (as `Scene::remove_beat`
 *      does) and leaving destinations that named it dangling;
 *   2. appends a beat to `beats`;
 *   3. renames a beat;
 *   4. removes a choice from `beat.choices` or an outcome from `beat.outcomes`;
 *   5. appends an outcome, when the writer connected a beat's spare handle;
 *   6. sets `choice.to` or `outcome.to`.
 *
 * Nothing else in the document is read, and nothing else is written. Dialogue,
 * variants, revisions, provenance, lifecycle, intents, `must_convey`,
 * `must_not_reveal`, participants, the entry condition, the summary, choice
 * labels, conditions, effects and existing tombstones are all carried by the
 * clone without this function knowing they exist.
 */
export function patchScene(doc: Scene, after: FlowLevel): ScenePatch {
  const scene: Scene = structuredClone(doc)
  const before = sceneToFlow(doc)
  const beforeById = new Map(before.elements.map((element) => [element.id, element]))
  const afterById = new Map(after.elements.map((element) => [element.id, element]))
  const created: string[] = []
  let changed = false

  /* 1 — beats the canvas no longer draws. */
  for (const element of before.elements) {
    if (afterById.has(element.id) || element.kind !== 'beat') continue
    const index = (scene.beats ?? []).findIndex((beat) => beat.id === element.beatId)
    if (index < 0) continue
    const [gone] = scene.beats!.splice(index, 1)
    if (gone) {
      scene.tombstones = [...(scene.tombstones ?? []), ...tombstonesFor(gone)]
      changed = true
    }
  }

  /* 4 — choices and outcomes the canvas no longer draws. */
  for (const element of before.elements) {
    if (afterById.has(element.id)) continue
    const parsed = narrativeIdOf(element.id)
    if (!parsed || (parsed.kind !== 'choice' && parsed.kind !== 'outcome')) continue
    for (const beat of scene.beats ?? []) {
      const list = parsed.kind === 'choice' ? beat.choices : beat.outcomes
      const index = (list ?? []).findIndex((one) => one.id === parsed.id)
      if (index >= 0) {
        list!.splice(index, 1)
        changed = true
      }
    }
  }

  /* 2 — beats the canvas has drawn and the document does not have. */
  const minted = new Map<string, string>()
  for (const element of after.elements) {
    if (beforeById.has(element.id) || element.kind !== 'beat') continue
    const id = mintId()
    minted.set(element.id, id)
    scene.beats = [...(scene.beats ?? []), { id, title: element.title }]
    created.push(nodeId.beat(id))
    changed = true
  }

  /** Where a port now leads, in the document's own vocabulary. */
  const destination = (node: string | null): Destination | null => {
    if (node === null) return null
    const parsed = narrativeIdOf(node)
    if (parsed?.kind === 'beat') return { beat: parsed.id }
    const fresh = minted.get(node)
    if (fresh !== undefined) return { beat: fresh }
    /*
     * An ending or a scene link, copied from the element that owns it rather
     * than rebuilt from the box on screen. The box shows `End` for an ending
     * whose label is empty, so reading a destination back out of a title would
     * quietly write the word "End" into the file — and a scene link's box shows
     * the *name* of the scene it leads to, not its id.
     */
    const holder = node.startsWith('end:')
      ? node.slice('end:'.length)
      : node.startsWith('link:')
        ? node.slice('link:'.length)
        : null
    if (holder === null) return null
    const owner = narrativeIdOf(holder)
    if (!owner) return null
    for (const beat of doc.beats ?? []) {
      const found =
        owner.kind === 'choice'
          ? (beat.choices ?? []).find((choice) => choice.id === owner.id)
          : (beat.outcomes ?? []).find((outcome) => outcome.id === owner.id)
      if (found) return structuredClone(found.to)
    }
    return null
  }

  for (const element of after.elements) {
    const parsed = narrativeIdOf(element.id)

    /* 3 and 5 — a beat's title, and a way out authored by connecting. */
    if (element.kind === 'beat' && parsed?.kind === 'beat') {
      const beat = (scene.beats ?? []).find((one) => one.id === parsed.id)
      if (!beat) continue
      if (beat.title !== element.title) {
        beat.title = element.title
        changed = true
      }
      const known = new Set((beforeById.get(element.id)?.out ?? []).map((port) => port.id))
      for (const port of element.out) {
        if (known.has(port.id)) continue
        const to = destination(port.to)
        // A spare handle dragged nowhere is not an outcome. The destination is
        // the whole of what the gesture supplies, so without one there is
        // nothing to author.
        if (to === null) continue
        beat.outcomes = [...(beat.outcomes ?? []), { id: mintId(), to }]
        changed = true
      }
      continue
    }

    /* 6 — a destination moved. */
    if (parsed?.kind !== 'choice' && parsed?.kind !== 'outcome') continue
    const to = destination(element.out[0]?.to ?? null)
    if (to === null) continue
    for (const beat of scene.beats ?? []) {
      const holder =
        parsed.kind === 'choice'
          ? (beat.choices ?? []).find((choice) => choice.id === parsed.id)
          : (beat.outcomes ?? []).find((outcome) => outcome.id === parsed.id)
      if (!holder) continue
      if (!sameDestination(holder.to, to)) {
        holder.to = to
        changed = true
      }
    }
  }

  return { scene, created, unchanged: !changed }
}

function sameDestination(a: Destination, b: Destination): boolean {
  if ('beat' in a && 'beat' in b) return a.beat === b.beat
  if ('scene' in a && 'scene' in b) return a.scene === b.scene
  if ('end' in a && 'end' in b) return (a.end.label ?? '') === (b.end.label ?? '')
  return false
}

/**
 * The records a deleted beat leaves behind.
 *
 * The same set `Scene::remove_beat` writes, and for the same reason: a locale
 * row or a recording script filed against a variant id has to stay explainable
 * after the line is gone (#178, #179), and a destination naming a removed beat
 * should say "Verdict, deleted today" rather than "unknown beat 01J8…".
 *
 * Choices and outcomes get none, exactly as they get none in Rust: nothing
 * outside the scene file refers to one by id.
 */
function tombstonesFor(beat: Beat): Tombstone[] {
  const deletedAt = new Date().toISOString()
  const out: Tombstone[] = [{ target: { beat: beat.id }, label: beat.title, deleted_at: deletedAt }]
  for (const slot of beat.dialogue ?? []) {
    out.push({ target: { dialogue_slot: slot.id }, label: beat.title, deleted_at: deletedAt })
    for (const variant of slot.variants ?? []) {
      // The words, because that is what identifies a line to whoever is holding
      // a recording script for it.
      out.push({
        target: { variant: variant.id },
        label: variant.text.body,
        deleted_at: deletedAt,
      })
    }
  }
  return out
}
