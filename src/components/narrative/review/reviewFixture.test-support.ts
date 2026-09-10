import type { ReviewSceneView } from '../../../lib/api/narrativeReview'
import { reviewTargetKey, type ReviewRow } from './reviewModel'

export function reviewFixture(): ReviewSceneView {
  return {
    scene_id: 'scene',
    guard: { stamp: null, head: 'original-head' },
    state_json: '{}',
    context_summary: 'Mara witnessed the beacon failing. Trust is high.',
    history: [],
    lines: [
      {
        target: { scene: 'scene', beat: 'arrival', slot: 'warning', variant: 'warning-main' },
        speaker: { entity: 'mara' },
        text: {
          revision: 'original-revision',
          body: 'The beacon failed.',
          lifecycle: { policy: 'generated', review: 'draft', freshness: 'current' },
        },
        slot_policy: 'generated',
        review: 'draft',
        freshness: 'current',
        approval_valid: false,
        reason: 'A new generated proposal is ready for review.',
        context_revision: 'original-context',
        proposals: [
          {
            id: 'proposal',
            hash: 'proposal-hash',
            request_id: 'request',
            receipt_id: 'receipt',
            candidate: {
              slot_id: 'warning',
              variant_id: 'warning-main',
              speaker: { entity: 'mara' },
              text: 'I watched the beacon go dark.',
            },
            base_revision: 'original-revision',
            base_wording: 'The beacon failed.',
            status: 'pending',
            reason: 'Generated for the beacon failure beat.',
          },
        ],
      },
    ],
  }
}
export function fixtureRow(scene = reviewFixture()): ReviewRow {
  return {
    key: reviewTargetKey(scene.lines[0]!.target),
    scene,
    line: scene.lines[0]!,
    sceneName: 'Beacon aftermath',
    speakerName: 'Mara',
    speakerKey: 'mara',
  }
}
