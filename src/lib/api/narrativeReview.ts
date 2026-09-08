import { call } from './call'
import type {
  Freshness,
  GenerationPolicy,
  ReviewState,
  SceneFile,
  Speaker,
  Stamp,
  StateValue,
  Text,
} from './narrative'
import type { GeneratedCandidate } from './narrativeGeneration'

export interface ReviewTarget {
  scene: string
  beat: string
  slot: string
  variant: string | null
}
export interface ReviewGuard {
  stamp: Stamp | null
  head: string | null
}
export interface ReviewProposal {
  id: string
  hash: string
  request_id: string
  receipt_id: string
  candidate: GeneratedCandidate
  base_revision: string | null
  base_wording: string | null
  status: 'pending' | 'accepted' | 'rejected'
  reason: string
}
export interface ReviewLine {
  target: ReviewTarget
  speaker: Speaker
  text: Text | null
  slot_policy: GenerationPolicy
  review: ReviewState
  freshness: Freshness
  approval_valid: boolean
  reason: string
  context_revision: string
  proposals: ReviewProposal[]
}
export interface ReviewHistory {
  id: string
  action: string
  target: ReviewTarget | null
  actor: string
  context_revision: string | null
}
export interface ReviewSceneView {
  scene_id: string
  guard: ReviewGuard
  state_json: string
  lines: ReviewLine[]
  history: ReviewHistory[]
  context_summary: string
}
export interface ReviewContext {
  version: 1
  revision: string
  state: Record<string, StateValue>
  inputs: unknown
}
export type ReviewAction =
  | { kind: 'policy'; scope: 'slot' | 'variant'; policy: GenerationPolicy }
  | { kind: 'approve' }
  | { kind: 'attest' }
  | { kind: 'accept'; proposal_id: string; proposal_hash: string; reviewed_text: string | null }
  | { kind: 'reject'; proposal_id: string; proposal_hash: string }
  | { kind: 'edit'; body: string }
export interface ReviewRequest {
  guard: ReviewGuard
  target: ReviewTarget
  context_revision: string
  state_json: string
  action: ReviewAction
}
export const narrativeReviewGet = (sceneId: string, stateJson: string | null = null) =>
  call<ReviewSceneView>('narrative_review_get', { sceneId, stateJson })
export const narrativeReviewContext = (target: ReviewTarget, stateJson: string | null = null) =>
  call<ReviewContext>('narrative_review_context', { target, stateJson })
export const narrativeReviewApply = (request: ReviewRequest) =>
  call<{ file: SceneFile; review: ReviewSceneView }>('narrative_review_apply', { request })

export interface ReviewList {
  scenes: ReviewSceneView[]
  errors: { scene_id: string | null; reason: string }[]
  next_offset: number | null
  total_scenes: number
  catalog_revision: string
}
export interface ReviewBatchItem {
  index: number
  target: ReviewTarget
  status: 'eligible' | 'applied' | 'skipped' | 'conflicting'
  reason: string
}
export interface ReviewBatchResult {
  items: ReviewBatchItem[]
}
export const narrativeReviewList = (
  stateJson: string | null = null,
  offset = 0,
  expectedCatalog: string | null = null,
) => call<ReviewList>('narrative_review_list', { stateJson, offset, expectedCatalog })
export const narrativeReviewBatch = (requests: ReviewRequest[], commit: boolean) =>
  call<ReviewBatchResult>('narrative_review_batch', { requests, commit })
