import { call } from './call'
import type { InfluenceLayer, LinkRole, NodeKind } from './model'
/* ── domain types ─────────────────────────────────────────────────────────── */

/**
 * Where a fragment is routed once a generation is compiled.
 *
 * `moodboard_only` is the one that matters: it is shown to the human and never
 * sent anywhere. It appears on a layer card and in nothing `promptCompile`
 * returns — see `InfluenceFragment.sendable`.
 */
export type FragmentTarget =
  'prompt' | 'negative' | 'style_ref' | 'structure_ref' | 'palette' | 'moodboard_only'

/**
 * How a source got into the stack, and therefore what the layer card says about
 * why it is there. A card that cannot answer that is unarguable — the user sees
 * a culture they did not expect and has nowhere to go.
 */
export type Reached =
  | 'subject'
  /** A project singleton seeded into every stack: the Style Guide, the World Bible. */
  | 'root'
  | { link: LinkRole }
  /** Followed `parentId`, the implicit link of weight 1.0. */
  | 'parent'
  /** The shot controls, which are not part of the world. */
  | 'shot'

/** Why a fragment the user wrote is not in the compiled prompt. */
export type DropReason =
  /** Turned down to nothing — a slider at the bottom, or a link weighted to 0. */
  | 'silenced'
  /** It did not fit. The lightest go first, so this one was among the least weighted. */
  | 'budget'

/** How much an output preset cares about one description section. 1.0 is no opinion. */
export interface SectionPriority {
  section: string
  weight: number
}

export interface PresetView {
  /** Provider-ready tag recorded on the generation and sent with the mesh input. */
  viewType: string
  /** Camera instruction appended to this generation's Shot fragments. */
  framing: string
}

export interface ImageConstraints {
  mimeTypes: string[]
  minSide: number
  maxSide: number
  /** Whole named-view batch, before base64 encoding. */
  maxBatchBytes: number
}

/**
 * The recipe that turns one description into a particular kind of sheet.
 *
 * Returned whole by both commands rather than as an id: the panel needs the
 * aspect and image count to describe what Generate would do, and a round trip
 * for a static table would be a round trip for nothing.
 */
export interface Preset {
  id: string
  label: string
  kinds: NodeKind[]
  defaultFor: NodeKind[]
  priorities: SectionPriority[]
  /** The Shot layer's own text — pose, lighting, background, distance. */
  framing: string
  aspect: string
  images: number
  /** Tagged, framed views in emission order; empty when the batch just varies. */
  views: PresetView[]
  imageConstraints: ImageConstraints | null
}

/** Presets applicable to one kind, in registry order. */
export const presetList = (kind: NodeKind) => call<Preset[]>('preset_list', { kind })

/**
 * One thing one layer contributes.
 *
 * The same shape appears three times — a card's contributions, the spans in the
 * compiled prompt, and the drop report — because they are the same fragments
 * seen from three angles.
 */
export interface InfluenceFragment {
  layer: InfluenceLayer
  /** Null for the Shot layer, whose framing text comes from the preset. */
  nodeId: string | null
  sourceName: string
  /** A description section key, or a reference's role. */
  section: string
  /** Prose. Null for a reference image, which carries `assetId` instead. */
  text: string | null
  assetId: string | null
  /** `link.weight × section_priority × user_slider`, already multiplied out. */
  weight: number
  target: FragmentTarget
  /**
   * Whether this may be put in front of a provider. False only for
   * `moodboard_only`. Read it rather than re-deriving it from `target` — one
   * list of what is private is the whole point, and the direction a second one
   * fails in is somebody's mood board on a third party's servers.
   */
  sendable: boolean
}

/** One layer card. */
export interface LayerCard {
  layer: InfluenceLayer
  nodeId: string | null
  /** What the card is titled — the node's name, or the shot's label. */
  name: string
  kind: NodeKind | null
  reached: Reached
  /** Hops from whichever root reached this. The subject and the singletons are 0. */
  distance: number
  /** The product of the link weights along the path. Not the user's slider. */
  weight: number
  /** Where this card's slider sits, 0.0–1.0. */
  slider: number
  fragments: InfluenceFragment[]
}

/** The resolved stack for one subject, outermost layer first. */
export interface InfluenceStack {
  subjectId: string
  preset: Preset
  layers: LayerCard[]
}

export interface DroppedFragment {
  fragment: InfluenceFragment
  reason: DropReason
}

export interface CompiledPrompt {
  subjectId: string
  preset: Preset
  prompt: string
  negative: string
  /**
   * The fragments the two strings are made of, in emission order — what lets the
   * prompt box tint each span by where it came from. That attribution is the
   * main feedback loop for learning to write good upstream notes, not a debug
   * feature.
   */
  spans: InfluenceFragment[]
  /**
   * Everything left out, in reading order, so it can be walked alongside the
   * layer cards. The Inspector reports what was dropped rather than truncating
   * silently; a user who cannot see what was cut cannot learn to fix it.
   */
  dropped: DroppedFragment[]
  /**
   * Characters over budget, or null when it fits. Only ever set when the budget
   * could not fit even one fragment: the compiler keeps the heaviest and says
   * so, because an empty prompt is not a smaller picture, it is a different one
   * that still costs money.
   */
  overflow: number | null
}

/** Where one layer card's weight slider sits. */
export interface SliderSetting {
  nodeId: string
  /** 0.0–1.0. Clamped on the far side rather than refused. */
  value: number
  /** Removes the card for this run while preserving `value` for unmute. */
  muted?: boolean
}

/** The Shot layer — layer 7, the one the Inspector's own controls own. */
export interface ShotControls {
  /** What the Shot card is titled. Defaults to the preset's label. */
  label?: string
  weight?: number
  /** Extra framing typed for this run; unlike `label`, this is sent. */
  prompt?: string
}

/**
 * What one compilation may spend on text, in characters rather than tokens.
 *
 * Omit either and that pool is unlimited, which is the right answer until a
 * backend has been chosen — a limit invented here would drop fragments to fit a
 * number nobody measured. The two are metered separately because a request with
 * no negative prompt is ordinary and one with no positive prompt is a picture of
 * nothing that still costs money.
 */
export interface PromptBudget {
  promptChars?: number
  negativeChars?: number
}

/**
 * The resolved stack for a subject, with the per-layer detail the layer cards
 * read.
 *
 * Answers from the local index and never touches the project folder, so it is
 * as fast on a share that has just been unplugged as on a local disk. A project
 * with no Style Guide, or with none of the links the stack walks, resolves to a
 * short list rather than an error — that is the state every project is in on day
 * one. Rejects with `node.not_found` for a subject that is not there, which is
 * usually a panel still pointing at something a collaborator deleted.
 *
 * `shot` is optional and omitting it means there is no shot yet: no Shot card
 * appears, because nothing has been framed. `promptCompile` always has one.
 */
export const influenceResolve = (
  subjectId: string,
  options: {
    preset?: string
    sliders?: SliderSetting[]
    shot?: ShotControls
  } = {},
) => call<InfluenceStack>('influence_resolve', { subjectId, ...options })

/**
 * The compiled positive and negative prompt, the spans they are made of, and the
 * account of what did not make it.
 *
 * Called on every Inspector interaction — every slider drag, every preset
 * change — and does no file I/O at all, so it is cheap enough to run on each
 * one rather than behind a debounce.
 *
 * A preset the backend has never heard of falls back to the kind's default
 * rather than failing, because a generation record naming a preset since renamed
 * still has to open.
 */
export const promptCompile = (
  subjectId: string,
  options: {
    preset?: string
    sliders?: SliderSetting[]
    shot?: ShotControls
    budget?: PromptBudget
  } = {},
) => call<CompiledPrompt>('prompt_compile', { subjectId, ...options })
