import { call } from './call'
import type { ContextSelection } from './narrativeContext'
import type { GenerationHistory } from './narrativeGeneration'
import type { AffectedExplanation } from './narrativeDeps'

export type BuildScope = 'missing' | 'affected' | 'all_selected'
export type BuildAction = 'generate' | 'propose' | 'locked' | 'blocked'
export interface BuildItem {
  id: string
  target: ContextSelection
  asset: boolean
  label: string
  action: BuildAction
  reasons: AffectedExplanation[]
  diagnostics: string[]
  request_id: string | null
  reusable: boolean
  candidate_variant_id: string | null
  state: Record<string, unknown>
}
export interface NarrativeBuild {
  version: number
  id: string
  scope: BuildScope
  provider: string
  model: string
  items: BuildItem[]
  diagnostics: string[]
}
export interface BuildStatus {
  build: NarrativeBuild
  history: GenerationHistory[]
  dispatched: string[]
  decided: string[]
}
export interface BuildSummary {
  id: string
  scope: BuildScope
  items: number
  provider: string
  model: string
}
export const narrativeBuildPlan = (source: string) =>
  call<NarrativeBuild>('narrative_build_plan', { source })
export const narrativeBuildStatus = (buildId: string) =>
  call<BuildStatus>('narrative_build_status', { buildId })
export const narrativeBuildList = () => call<BuildSummary[]>('narrative_build_list')
export const narrativeBuildStart = (buildId: string, items: string[]) =>
  call<{ request_id: string; job_id: string }[]>('narrative_build_start', { buildId, items })
