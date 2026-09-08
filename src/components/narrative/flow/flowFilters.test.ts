import { expect, it } from 'vitest'
import type { NarrativeFilter } from '../../../store/ui'
import { isFlowElementMuted } from './flowFilters'
import { sceneToFlow } from './source'
import type { FlowBeat } from './model'
const filters = (selected: NarrativeFilter) => ({
  needsText: selected === 'needsText',
  needsReview: selected === 'needsReview',
  outOfDate: selected === 'outOfDate',
})
const beat = sceneToFlow({
  id: 'scene',
  name: 'Mixed',
  beats: [
    {
      id: 'beat',
      title: 'Mixed work',
      dialogue: [
        { id: 'empty', speaker: 'player' },
        {
          id: 'review',
          speaker: 'narrator',
          variants: [
            {
              id: 'variant',
              text: {
                revision: 'revision',
                body: 'Wording',
                lifecycle: { review: 'draft', freshness: 'out_of_date' },
              },
            },
          ],
        },
      ],
    },
  ],
}).elements.find((element) => element.kind === 'beat')! as FlowBeat
it.each(['needsText', 'needsReview', 'outOfDate'] as const)(
  'matches every populated %s dimension of a mixed beat',
  (key) => {
    expect(beat.status).toBe('needsText')
    expect(isFlowElementMuted(beat, false, null, filters(key))).toBe(false)
  },
)
it('uses authoritative zero counts rather than a stale summary, while preserving empty-beat needs-text semantics', () => {
  const zero = { ...beat, counts: { needsText: 0, needsReview: 0, outOfDate: 0, locked: 0 } }
  expect(isFlowElementMuted(zero, false, null, filters('needsText'))).toBe(true)
  expect(isFlowElementMuted({ ...zero, lines: 0 }, false, null, filters('needsText'))).toBe(false)
  expect(isFlowElementMuted({ ...zero, lines: 0 }, false, null, filters('needsReview'))).toBe(true)
})
it('retains summary-only fixtures, participant filters and the release-error override', () => {
  const legacy = { ...beat, counts: undefined, status: 'needsReview' as const }
  expect(isFlowElementMuted(legacy, false, null, filters('needsReview'))).toBe(false)
  expect(isFlowElementMuted(beat, false, 'Someone absent', filters('needsReview'))).toBe(true)
  expect(isFlowElementMuted(beat, true, 'Someone absent', filters('needsReview'))).toBe(false)
})
