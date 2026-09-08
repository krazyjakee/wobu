import { call } from './call'
import type { Condition, Precondition, SceneId, Stamp, StateValue } from './narrative'

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

export interface Quest extends WorldRecord {
  summary: string
  stages: string[]
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
