import { call } from './call'
import type { Speaker } from './narrative'
import type { ContextSelection, FrozenContext } from './narrativeContext'

export interface GeneratedCandidate {
  slot_id: string
  variant_id: string
  speaker: Speaker
  text: string
}
export interface GenerationPlan {
  id: string
  provider: string
  model: string
  requests: {
    request_id: string
    target: ContextSelection
    candidate_variant_id: string
    expected_policy: string | null
    context: FrozenContext
    prompt: string
  }[]
  skipped: { target: ContextSelection; reason: string }[]
}
export interface GenerationHistory {
  request_id: string
  batch_id: string
  target: ContextSelection
  provider: string
  model: string
  attempts: number
  status: string
  receipt_id: string | null
  candidate: GeneratedCandidate | null
  usage: { input: number; cached_input: number; output: number }
  billing_unknown: boolean
  error_code: string | null
  proposal_published: boolean
  proposal_current_at_publication: boolean | null
}
export const narrativeGenerationPlan = (source: string) =>
  call<GenerationPlan>('narrative_generation_plan', { source })
export const narrativeGenerationStart = (planId: string) =>
  call<{ request_id: string; job_id: string }[]>('narrative_generation_start', { planId })
export const narrativeGenerationHistory = () =>
  call<GenerationHistory[]>('narrative_generation_history')
export const narrativeGenerationRetry = (requestId: string) =>
  call<{ request_id: string; job_id: string }>('narrative_generation_retry', { requestId })
export const narrativeGenerationRecover = (requestId: string) =>
  call<void>('narrative_generation_recover', { requestId })
