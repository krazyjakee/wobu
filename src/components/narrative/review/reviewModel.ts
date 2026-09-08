import type { GenerationPolicy, Freshness } from '../../../lib/api'
import type { ReviewLine, ReviewSceneView, ReviewTarget } from '../../../lib/api/narrativeReview'

export interface ReviewRow {
  key: string
  scene: ReviewSceneView
  line: ReviewLine
  sceneName: string
  speakerName: string
  speakerKey: string
}
export interface ReviewFilters {
  query: string
  scene: string
  speaker: string
  policy: GenerationPolicy | ''
  approval: 'draft' | 'approved' | 'invalid' | ''
  freshness: Freshness | ''
}
export const EMPTY_REVIEW_FILTERS: ReviewFilters = {
  query: '',
  scene: '',
  speaker: '',
  policy: '',
  approval: '',
  freshness: '',
}
export const reviewTargetKey = (target: ReviewTarget) =>
  `${target.scene}/${target.beat}/${target.slot}/${target.variant ?? 'empty'}`
export const reviewPolicy = (line: ReviewLine): GenerationPolicy =>
  line.slot_policy === 'locked' ? 'locked' : (line.text?.lifecycle?.policy ?? line.slot_policy)

/** These are display filters, never eligibility checks for writes. */
export function filterReviewRows(rows: ReviewRow[], filters: ReviewFilters): ReviewRow[] {
  const query = filters.query.trim().toLocaleLowerCase()
  return rows.filter(({ sceneName, speakerName, speakerKey, line, scene }) => {
    if (filters.scene && scene.scene_id !== filters.scene) return false
    if (filters.speaker && speakerKey !== filters.speaker) return false
    if (filters.policy && reviewPolicy(line) !== filters.policy) return false
    if (filters.freshness && line.freshness !== filters.freshness) return false
    if (filters.approval === 'approved' && !line.approval_valid) return false
    if (filters.approval === 'draft' && line.review !== 'draft') return false
    if (
      filters.approval === 'invalid' &&
      (line.approval_valid || line.text?.lifecycle?.review !== 'approved')
    )
      return false
    return (
      !query ||
      [
        sceneName,
        speakerName,
        line.text?.body ?? '',
        line.reason,
        ...line.proposals.map((p) => p.candidate.text),
      ]
        .join('\n')
        .toLocaleLowerCase()
        .includes(query)
    )
  })
}
