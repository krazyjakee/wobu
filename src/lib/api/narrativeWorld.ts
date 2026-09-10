import { call } from './call'
import type { Condition, Precondition, SceneId, Stamp, StateValue, Text } from './narrative'

export interface WorldRecord {
  id: string
  name: string
}

export interface Fact extends WorldRecord {
  assertion: string
  sources: string[]
  entity_ids?: string[]
}

export type KnowledgeProvenance =
  | 'witnessed'
  | { told: { by: string } }
  | { rumour: { source: string } }
  | { inferred: { reason: string } }

export interface KnowledgeClaim extends WorldRecord {
  character: string
  fact: string
  belief: 'true' | 'false' | 'unknown'
  provenance: KnowledgeProvenance
  when: Condition
}

export interface Relationship extends WorldRecord {
  from: string
  to: string
  kind: string
  value: StateValue
  when: Condition
}

export interface WorldEvent extends WorldRecord {
  summary: string
  fact_ids: string[]
  entity_ids?: string[]
  when: Condition
}

export interface QuestTransition {
  from: string
  to: string
  when: Condition
}

/** Player-facing objective wording for one stage, with the identity a locale row
 * and an approval are filed under. */
export interface QuestObjective {
  id: string
  text: Text
}

/**
 * A quest stage: a bare name, or a name with an objective beside it.
 *
 * Two shapes because the file has two: a stage nobody has written an objective
 * for stays a bare name, so opening an existing World file does not rewrite it.
 * Read one with {@link stageName} rather than branching at each use.
 */
export type QuestStage = string | { name: string; objective?: QuestObjective | null }

/** Which stage this is, whichever shape the file used. */
export const stageName = (stage: QuestStage): string =>
  typeof stage === 'string' ? stage : stage.name

/** The objective wording for a stage, or null when none is authored. */
export const stageObjective = (stage: QuestStage): QuestObjective | null =>
  typeof stage === 'string' ? null : (stage.objective ?? null)

/**
 * The stage with its objective's words replaced.
 *
 * An empty body removes the objective entirely, so clearing the field leaves the
 * stage in the bare shape it started in rather than storing a blank one. The id
 * and the revision are left as they are: sealing changed words is the save path's
 * job ({@link ../../components/narrative/worldText.prepareWorldText}), because a
 * revision is a digest and a keystroke must not mint one.
 */
export function setStageObjective(stage: QuestStage, body: string): QuestStage {
  const name = stageName(stage)
  if (!body.trim()) return name
  const existing = stageObjective(stage)
  return {
    name,
    objective: {
      id: existing?.id ?? '',
      text: { ...(existing?.text ?? { revision: '', body: '' }), body },
    },
  }
}

export interface Quest extends WorldRecord {
  summary: string
  stages: QuestStage[]
  initial: string
  transitions: QuestTransition[]
  scene_ids: SceneId[]
}

export interface FutureRestriction extends WorldRecord {
  fact: string
  characters: string[]
  until: Condition
}

/** Canonical narrative/world.yaml; unresolved references remain visible draft diagnostics. */
export type NamedClassification = WorldRecord

export interface WorldDocument {
  acts?: NamedClassification[]
  arcs?: NamedClassification[]
  tags?: NamedClassification[]
  schema_version: number
  facts: Fact[]
  knowledge: KnowledgeClaim[]
  relationships: Relationship[]
  events: WorldEvent[]
  quests: Quest[]
  restrictions: FutureRestriction[]
}

export interface WorldDiagnostic {
  recordId: string | null
  field: string
  message: string
}

export interface WorldFile {
  document: WorldDocument
  stamp: Stamp | null
  diagnostics: WorldDiagnostic[]
}

export const narrativeWorldGet = (): Promise<WorldFile> => call('narrative_world_get')

export const narrativeWorldSave = (
  document: WorldDocument,
  expected: Precondition,
): Promise<WorldFile> => call('narrative_world_save', { document, expected })

/** Undo/redo compares the expected document under the project lock before a guarded write. */
export const narrativeWorldRestore = (
  document: WorldDocument,
  expected: WorldDocument,
): Promise<WorldFile> => call('narrative_world_restore', { document, expected })
