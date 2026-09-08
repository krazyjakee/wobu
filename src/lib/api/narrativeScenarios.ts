import { call } from './call'
import type { Stamp, VarType, StateValue } from './narrative'
import type {
  CompileDiagnostic,
  PreviewState,
  PreviewHostResult,
  PreviewTraceSite,
  ExecutionTrace,
} from './narrativePreview'

export type ScenarioAction =
  | { kind: 'advance' }
  | { kind: 'choose'; choice: string }
  | { kind: 'complete_command'; result: PreviewHostResult }
  | { kind: 'save_checkpoint' }
  | { kind: 'restore_checkpoint' }
export type ScenarioBoundary =
  | { kind: 'line'; scene?: string; beat?: string; slot?: string; variant?: string }
  | { kind: 'choices'; scene?: string; beat?: string; ids?: string[] }
  | { kind: 'command'; name?: string; args?: StateValue[] }
  | { kind: 'end'; scene?: string; beat?: string }
export interface ScenarioStep {
  action: ScenarioAction | null
  expect: {
    boundary?: ScenarioBoundary | null
    state?: PreviewState
    error?: 'command_failed' | 'command_cancelled' | null
  }
}
export interface Scenario {
  version: 1
  scene: string
  initial_state: PreviewState
  seed: number
  commands: Record<string, VarType[]>
  steps: ScenarioStep[]
}
export interface ScenarioFile {
  id: string
  name: string
  scenario: Scenario
  stamp: Stamp | null
}
export interface ScenarioResult {
  id: string
  report: {
    passed: boolean
    checked_steps: number
    divergence: {
      step: number
      field: string
      expected: unknown
      actual: unknown
      site: PreviewTraceSite
      trace: ExecutionTrace
    } | null
  } | null
  diagnostics: CompileDiagnostic[]
}
export const narrativeScenariosList = (): Promise<ScenarioFile[]> =>
  call('narrative_scenarios_list')
export const narrativeScenarioSave = (
  name: string,
  scenario: Scenario | string,
  file?: ScenarioFile,
): Promise<ScenarioFile> =>
  call('narrative_scenario_save', {
    name,
    source: typeof scenario === 'string' ? scenario : JSON.stringify(scenario),
    id: file?.id ?? null,
    expected: file?.stamp ?? null,
  })
export const narrativeScenarioRun = (id: string): Promise<ScenarioResult> =>
  call('narrative_scenario_run', { id })
