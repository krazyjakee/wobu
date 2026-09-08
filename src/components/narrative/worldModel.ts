import type { WorldDocument, WorldRecord } from '../../lib/api/narrativeWorld'
import { hasUnsafeInteger } from './integerInput'
import { mintId } from './flow/source'

export type WorldCollection = Exclude<keyof WorldDocument, 'schema_version'>
export type WorldItem = NonNullable<WorldDocument[WorldCollection]>[number]
export const WORLD_ENTITY_COLLECTIONS = [
  { key: 'facts', label: 'Facts' },
  { key: 'knowledge', label: 'Knowledge' },
  { key: 'relationships', label: 'Relationships' },
  { key: 'events', label: 'Events' },
  { key: 'quests', label: 'Quests' },
  { key: 'restrictions', label: 'Future knowledge' },
] as const
export const WORLD_COLLECTIONS: { key: WorldCollection; label: string }[] = [
  ...WORLD_ENTITY_COLLECTIONS,
  { key: 'acts', label: 'Acts' },
  { key: 'arcs', label: 'Arcs' },
  { key: 'tags', label: 'Tags' },
]
export function createWorldRecord(collection: WorldCollection): WorldItem {
  const common = { id: mintId(), name: 'New record' }
  switch (collection) {
    case 'acts':
    case 'arcs':
    case 'tags':
      return common
    case 'facts':
      return { ...common, assertion: '', sources: [] }
    case 'knowledge':
      return {
        ...common,
        character: '',
        fact: '',
        belief: 'unknown',
        provenance: 'witnessed',
        when: 'always',
      }
    case 'relationships':
      return { ...common, from: '', to: '', kind: '', value: 0, when: 'always' }
    case 'events':
      return { ...common, summary: '', fact_ids: [], when: 'always' }
    case 'quests':
      return {
        ...common,
        summary: '',
        stages: ['started', 'completed'],
        initial: 'started',
        transitions: [],
        scene_ids: [],
      }
    case 'restrictions':
      return { ...common, fact: '', characters: [], until: 'never' }
  }
}
export function recordUsers(
  document: WorldDocument,
  id: string,
): { collection: WorldCollection; record: WorldRecord }[] {
  const users: { collection: WorldCollection; record: WorldRecord }[] = []
  for (const record of document.knowledge)
    if (record.fact === id) users.push({ collection: 'knowledge', record })
  for (const record of document.events)
    if (record.fact_ids.includes(id)) users.push({ collection: 'events', record })
  for (const record of document.restrictions)
    if (record.fact === id) users.push({ collection: 'restrictions', record })
  return users
}

/** Missing required selectors are local drafts, not valid YAML identities. */
export function requiredWorldFields(
  document: WorldDocument,
): { collection: WorldCollection; id: string; message: string }[] {
  const problems: { collection: WorldCollection; id: string; message: string }[] = []
  for (const { key } of WORLD_COLLECTIONS) {
    for (const record of document[key] ?? []) {
      if (hasUnsafeInteger(record))
        problems.push({
          collection: key,
          id: record.id,
          message: `${record.name}: a value is outside the editor’s exact integer range. Correct it before saving.`,
        })
    }
  }
  for (const record of document.knowledge) {
    if (!record.character || !record.fact)
      problems.push({
        collection: 'knowledge',
        id: record.id,
        message: `${record.name}: choose a character and fact.`,
      })
    if (
      typeof record.provenance === 'object' &&
      'told' in record.provenance &&
      !record.provenance.told.by
    )
      problems.push({
        collection: 'knowledge',
        id: record.id,
        message: `${record.name}: choose who told them.`,
      })
  }
  for (const record of document.relationships) {
    if (!record.from || !record.to || !record.kind.trim())
      problems.push({
        collection: 'relationships',
        id: record.id,
        message: `${record.name}: choose both characters and a relationship kind.`,
      })
    if (typeof record.value === 'number' && !Number.isSafeInteger(record.value))
      problems.push({
        collection: 'relationships',
        id: record.id,
        message: `${record.name}: use a whole number within the supported range.`,
      })
  }
  for (const record of document.restrictions)
    if (!record.fact)
      problems.push({
        collection: 'restrictions',
        id: record.id,
        message: `${record.name}: choose the restricted fact.`,
      })
  return problems
}
