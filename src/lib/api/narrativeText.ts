import { call } from './call'
import type {
  Condition,
  DialogueSlot,
  GenerationPolicy,
  NarrativeDiagnostic,
  Participant,
  Precondition,
  SceneId,
  Stamp,
  TextAssetId,
  TextEntryId,
} from './narrative'

export type { TextAssetId, TextEntryId } from './narrative'

/**
 * The Text library's command surface: barks, ambient exchanges, companion
 * reactions, codex entries, quest summaries and journals (#167).
 *
 * The same two spellings meet here as in `narrative.ts` and for the same
 * reason: `TextCatalog` and `TextFile` are envelopes and are camelCase, while
 * `TextAsset` and everything under it is the YAML in `narrative/texts/*.yaml`
 * verbatim and is snake_case. A camelCase mirror would give the form editor and
 * the Source view two vocabularies for one file.
 *
 * The load-bearing reuse is `DialogueSlot`: a bark's line is the *same* type as
 * a scene line, so the review, freshness and policy controls that already know
 * how to render one need no supporting-text special case. If this file ever
 * grows its own line type, that stops being true for a quarter of the words in
 * the game.
 */

/* ── domain ───────────────────────────────────────────────────────────────── */

/** Which of the six supporting kinds an asset is. */
export type TextKind = 'bark' | 'ambient' | 'reaction' | 'codex' | 'quest_summary' | 'journal'

/**
 * How the runtime picks between eligible entries, and what the next trigger
 * does. Authored rather than left to the host: two hosts choosing differently
 * would make the same project play differently.
 */
export type RepeatPolicy =
  /** Always the first eligible entry — prose whose wording is a function of state. */
  | 'first'
  /** Each eligible entry once, in author order, then nothing. */
  | 'once'
  /** Round-robin in author order. */
  | 'cycle'
  /** A seeded bag: each eligible entry once per round, never repeating across the join. */
  | 'shuffle'

/** An authored record this text is about, and a dependency edge for #168. */
export type SourceLink =
  | { scene: SceneId }
  | { quest: string }
  | { fact: string }
  | { event: string }
  | { character: string }

/** The host-side moment an asset belongs to. */
export interface Trigger {
  /** A declared identifier, like a host command name: `[a-z][a-z0-9_]*`. */
  event: string
  /** Absent is unconditional, which is not the same as `never`. */
  when?: Condition
}

/** One alternative delivery: a single bark, a version of an exchange, a page. */
export interface TextEntry {
  id: TextEntryId
  label?: string
  when?: Condition
  /** Ordered. More than one only for an exchange or a passage of prose. */
  lines?: DialogueSlot[]
}

export interface TextAsset {
  id: TextAssetId
  kind: TextKind
  name: string
  summary?: string
  trigger: Trigger
  sources?: SourceLink[]
  /** Empty for standalone prose; every entity that speaks has to be listed. */
  participants?: Participant[]
  must_convey?: string[]
  must_not_reveal?: string[]
  repeat?: RepeatPolicy
  /** Whether generation may write into this asset at all. */
  policy?: GenerationPolicy
  entries?: TextEntry[]
  tombstones?: unknown[]
}

/* ── envelopes ────────────────────────────────────────────────────────────── */

export interface TextSummary {
  id: TextAssetId
  kind: TextKind
  name: string
  /** The file name stem, minted at create and stable across a rename. */
  slug: string
  /** Project-relative, `/`-separated. */
  rel: string
}

export interface TextCatalog {
  assets: TextSummary[]
  /** Files in the directory that could not be identified, listed not dropped. */
  unreadable: { rel: string; reason: string }[]
}

/** One asset, whole, with the precondition for writing it back. */
export interface TextFile {
  asset: TextAsset
  slug: string
  rel: string
  /** `null` means "we believe there is no file". */
  stamp: Stamp | null
}

/* ── commands ─────────────────────────────────────────────────────────────── */

export const narrativeTexts = () => call<TextCatalog>('narrative_texts')

export const narrativeTextGet = (assetId: TextAssetId) =>
  call<TextFile>('narrative_text_get', { assetId })

/**
 * Create an asset from a kind template.
 *
 * `event` is required rather than defaulted: an asset with no trigger is one
 * the host can never ask for, and a placeholder name would be a
 * working-looking binding to a moment that does not exist.
 */
export const narrativeTextCreate = (kind: TextKind, name: string, event: string) =>
  call<TextFile>('narrative_text_create', { kind, name, event })

/**
 * Write a whole supporting text document.
 *
 * One write path, guarded by the same precondition a scene uses, so a stale
 * editor tab cannot overwrite a colleague's newer file. `slug` is only
 * consulted for an asset the project does not currently have.
 */
export const narrativeTextSave = (asset: TextAsset, expected: Precondition, slug?: string | null) =>
  call<TextFile>('narrative_text_save', { asset, expected, slug: slug ?? null })

export const narrativeTextDelete = (assetId: TextAssetId) =>
  call<void>('narrative_text_delete', { assetId })

/**
 * What is wrong with one asset. Pass `asset` to diagnose the document as the
 * writer currently has it, unsaved edits and all.
 */
export const narrativeTextDiagnostics = (assetId: TextAssetId, asset?: TextAsset | null) =>
  call<NarrativeDiagnostic[]>('narrative_text_diagnostics', { assetId, asset: asset ?? null })
