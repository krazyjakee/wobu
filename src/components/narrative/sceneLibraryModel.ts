import type { GenerationPolicy, ReviewState, Freshness, Scene, SceneSummary } from '../../lib/api'
import type { Quest } from '../../lib/api/narrativeWorld'
import type { NarrativeTarget } from '../../store/ui'

export type SceneQuest = Pick<Quest, 'id' | 'name' | 'scene_ids'>

export interface LibraryView {
  query: string
  participant: string
  quest: string
  policy: GenerationPolicy | ''
  review: ReviewState | ''
  freshness: Freshness | ''
  missing: boolean
  includeDrafts: boolean
  sort: 'name' | 'nameDescending'
}
export const DEFAULT_LIBRARY_VIEW: LibraryView = {
  query: '',
  participant: '',
  quest: '',
  policy: '',
  review: '',
  freshness: '',
  missing: false,
  includeDrafts: false,
  sort: 'name',
}
export interface SceneMatch extends NarrativeTarget {
  snippet: string
  variantId?: string
  draft?: boolean
}
export interface LibraryRow {
  summary: SceneSummary
  scene?: Scene
  error?: string
}
export interface SceneResult extends LibraryRow {
  quests: SceneQuest[]
  matches: SceneMatch[]
  slots: number
  filled: number
}

/** Search only canonical source. Generated drafts are an explicit opt-in. */
export function findScenes(
  rows: LibraryRow[],
  view: LibraryView,
  quests: SceneQuest[] = [],
): SceneResult[] {
  const memberships = new Map<string, SceneQuest[]>()
  for (const quest of quests) {
    for (const sceneId of new Set(quest.scene_ids)) {
      const linked = memberships.get(sceneId) ?? []
      linked.push(quest)
      memberships.set(sceneId, linked)
    }
  }
  const query = view.query.trim().toLocaleLowerCase()
  return rows
    .flatMap((row): SceneResult[] => {
      const linkedQuests = memberships.get(row.summary.id) ?? []
      if (view.quest && !linkedQuests.some((quest) => quest.id === view.quest)) return []
      const scene = row.scene
      if (view.participant && !scene?.participants?.some((p) => p.entity === view.participant))
        return []
      const slots = scene?.beats?.flatMap((b) => b.dialogue ?? []) ?? []
      const filled = slots.filter((s) => s.variants?.some((v) => v.text.body.trim())).length
      if (view.missing && (!scene || slots.length === filled)) return []
      // Independent facets must describe the same variant, rather than borrowing
      // approval from one line and freshness from another.
      if (view.policy || view.review || view.freshness) {
        const matchesLifecycle = slots.some((slot) =>
          (slot.variants ?? []).some(({ text }) => {
            const lifecycle = text.lifecycle
            return (
              (!view.policy ||
                (slot.policy === 'locked'
                  ? 'locked'
                  : (lifecycle?.policy ?? slot.policy ?? 'edited')) === view.policy) &&
              (!view.review || (lifecycle?.review ?? 'draft') === view.review) &&
              (!view.freshness || (lifecycle?.freshness ?? 'current') === view.freshness)
            )
          }),
        )
        if (!matchesLifecycle) return []
      }
      const matches: SceneMatch[] = []
      const add = (
        text: string | undefined,
        target: NarrativeTarget,
        variantId?: string,
        draft?: boolean,
      ) => {
        if (!query || !text) return
        const offset = text.toLocaleLowerCase().indexOf(query)
        if (offset < 0) return
        const start = Math.max(0, offset - 45)
        const end = Math.min(text.length, offset + query.length + 90)
        matches.push({
          ...target,
          snippet: `${start ? '…' : ''}${text.slice(start, end)}${end < text.length ? '…' : ''}`,
          variantId,
          draft,
        })
      }
      const target = { sceneId: row.summary.id }
      add(row.summary.name, target)
      add(scene?.summary, target)
      for (const beat of scene?.beats ?? []) {
        const beatTarget = { ...target, beatId: beat.id }
        add(beat.title, beatTarget)
        for (const intent of beat.intents ?? []) add(intent.intent, beatTarget)
        for (const text of beat.must_convey ?? []) add(text, beatTarget)
        for (const slot of beat.dialogue ?? []) {
          for (const variant of slot.variants ?? []) {
            const generated =
              typeof variant.text.provenance === 'object' && 'generated' in variant.text.provenance
            const draft = generated && variant.text.lifecycle?.review !== 'approved'
            if (!draft || view.includeDrafts)
              add(variant.text.body, { ...beatTarget, lineId: slot.id }, variant.id, draft)
          }
        }
      }
      if (query && !matches.length) return []
      return [{ ...row, quests: linkedQuests, matches, slots: slots.length, filled }]
    })
    .sort((a, b) => {
      const byName =
        a.summary.name.localeCompare(b.summary.name) || a.summary.id.localeCompare(b.summary.id)
      return view.sort === 'name' ? byName : -byName
    })
}
