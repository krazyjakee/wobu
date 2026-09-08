import type { WorldDocument } from '../../lib/api/narrativeWorld'
import { WORLD_ENTITY_COLLECTIONS, type WorldItem } from './worldModel'

/** Only typed identity fields count as links; prose and rumour descriptions never do. */
export function worldRecordEntities(record: WorldItem): string[] {
  const ids: string[] = []
  if ('entity_ids' in record) ids.push(...(record.entity_ids ?? []))
  if ('character' in record) ids.push(record.character)
  if (
    'provenance' in record &&
    typeof record.provenance === 'object' &&
    'told' in record.provenance
  )
    ids.push(record.provenance.told.by)
  if ('from' in record) ids.push(record.from, record.to)
  if ('characters' in record) ids.push(...record.characters)
  return [...new Set(ids.filter(Boolean))]
}
export function narrativeBacklinks(document: WorldDocument, entityId: string, character: boolean) {
  return WORLD_ENTITY_COLLECTIONS.flatMap(({ key, label }) =>
    document[key]
      .filter(
        (record) =>
          worldRecordEntities(record).includes(entityId) ||
          (character && 'characters' in record && !record.characters.length),
      )
      .map((record) => ({ collection: key, category: label, record })),
  )
}
