import { call } from './call'
import type { StateValue } from './narrative'

export interface ContextSelection {
  scene: string
  beat: string
  slot: string
  variant: string | null
}
export interface ContextOptions {
  selection: ContextSelection
  state: Record<string, StateValue>
  token_budget: number
}
export interface ContextFragment {
  kind: string
  source: string
  required: boolean
  data: unknown
}
export interface FrozenContext {
  version: number
  options: ContextOptions
  fragments: ContextFragment[]
  omitted: string[]
  diagnostics: { code: string; source: string; message: string; blocking: boolean }[]
  dependencies: Record<string, string>
  queries: { name: string; parameters: unknown; members: Record<string, string> }[]
  request: string
  estimated_tokens: number
  ready: boolean
  hash: string
}
export const narrativeContextCapture = (
  options: ContextOptions,
  stateJson: string,
): Promise<FrozenContext> =>
  call('narrative_context_capture', {
    selection: options.selection,
    stateJson,
    tokenBudget: options.token_budget,
  })
export const narrativeContextFreshness = (
  options: ContextOptions,
  expectedHash: string,
): Promise<{ current: boolean; hash: string }> =>
  call('narrative_context_freshness', { options, expectedHash })
