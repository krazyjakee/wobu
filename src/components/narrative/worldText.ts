import type { WorldDocument, QuestStage } from '../../lib/api/narrativeWorld'
import { stageName, stageObjective } from '../../lib/api/narrativeWorld'
import { narrativeTextWritten } from '../../lib/api'
import { mintId } from './sceneIdentity'

/**
 * Seal the quest stage objectives whose words changed, and give a new one an id.
 *
 * The counterpart of `prepareScriptText`, and it exists for the same reason: a
 * revision is a digest of the words and their provenance, so a form that minted
 * one per keystroke would produce a new identity for every character typed —
 * and everything keyed to a revision, a translation included, would churn with
 * it. Unchanged wording keeps its revision and its provenance untouched.
 *
 * An id is minted only for an objective that has none, so editing the words of
 * an existing objective keeps the identity its locale row is filed under.
 */
export async function prepareWorldText(
  before: WorldDocument,
  draft: WorldDocument,
): Promise<WorldDocument> {
  const next = structuredClone(draft)
  if (!next.quests?.length) return next
  const previous = new Map<string, string>()
  for (const quest of before.quests ?? []) {
    for (const stage of quest.stages) {
      const objective = stageObjective(stage)
      if (objective) previous.set(`${quest.id}/${stageName(stage)}`, objective.text.body)
    }
  }
  for (const quest of next.quests) {
    const stages: QuestStage[] = []
    for (const stage of quest.stages) {
      const objective = stageObjective(stage)
      if (!objective) {
        stages.push(stage)
        continue
      }
      const unchanged =
        objective.id &&
        objective.text.revision &&
        previous.get(`${quest.id}/${stageName(stage)}`) === objective.text.body
      if (unchanged) {
        stages.push(stage)
        continue
      }
      const text = await narrativeTextWritten(
        objective.text.body,
        objective.text.lifecycle?.policy === 'locked',
      )
      stages.push({
        name: stageName(stage),
        objective: { id: objective.id || mintId(), text },
      })
    }
    quest.stages = stages
  }
  return next
}
