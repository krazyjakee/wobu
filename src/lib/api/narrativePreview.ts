import { call } from './call'
import type { Condition, Effect, Destination, Speaker, StateValue, VarType } from './narrative'

export type PreviewState = Record<string, StateValue>
/** Opaque Rust-owned values; the webview carries them without rewriting. */
export type PreviewGraph = Record<string, unknown> & {
  state?: Record<string, { ty: VarType; default: StateValue; owner: 'host' | 'narrative' }>
}
export type PreviewSnapshot = Record<string, unknown>
export interface CompileDiagnostic {
  scene: string
  site: unknown
  severity: 'error' | 'warning'
  code: string
  message: string
}
export interface CompileReport {
  graph: PreviewGraph | null
  diagnostics: CompileDiagnostic[]
}
export type PreviewYield =
  | {
      line: {
        scene: string
        beat: string
        slot: string
        variant: string
        speaker: Speaker
        text: string
        revision: string
      }
    }
  | { choices: { scene: string; beat: string; choices: { id: string; label: string }[] } }
  | { game_command: { token: string; name: string; args: StateValue[] } }
  | { end: { label: string } }
export interface PreviewTraceSite {
  scene: string
  beat: string | null
  choice: string | null
  outcome: string | null
  slot: string | null
  variant: string | null
}
export type PreviewHostResult =
  { success: { host_inputs: PreviewState } } | { failed: { message: string } } | 'cancelled'
export type PreviewTraceEvent =
  | {
      kind: 'condition'
      expression: Condition
      path: number[]
      passed: boolean
      inputs: PreviewState
    }
  | { kind: 'transition'; to: Destination }
  | { kind: 'effect'; index: number; effect: Effect; before: PreviewState; after: PreviewState }
  | {
      kind: 'command_result'
      token: string
      result: PreviewHostResult
      before: PreviewState
      after: PreviewState
      repeated: boolean
    }
export interface ExecutionTrace {
  committed: boolean
  error: string | null
  omitted: number
  records: { site: PreviewTraceSite; event: PreviewTraceEvent }[]
}
/**
 * One choice at the branch a run is stopped at, taken or not.
 *
 * `PreviewYield`'s `choices` lists what may be picked and deliberately nothing
 * else — a game is not told which options it was refused. Authoring is the
 * other case, so the closed routes and the recorded evaluation of each gate
 * arrive here instead, in the trace's own vocabulary.
 *
 * Empty away from a branch, and empty is a fact rather than an absence: a run
 * stopped on a line has no availability to report yet.
 */
export interface PreviewChoiceStatus {
  id: string
  label: string
  available: boolean
  records: { site: PreviewTraceSite; event: PreviewTraceEvent }[]
}
export interface PreviewFrame {
  site?: PreviewTraceSite
  snapshot: PreviewSnapshot
  current: PreviewYield
  state: PreviewState
  trace: ExecutionTrace
  /** Every choice at this position. See `PreviewChoiceStatus`. */
  branch: PreviewChoiceStatus[]
  /**
   * The compiled content this run is pinned to.
   *
   * Preview compiles *saved* source, so a frame outlives the source it
   * describes the moment somebody edits a scene. Anything derived from a frame
   * carries this so it can say which build it draws.
   */
  build: string
}
export type PreviewAction =
  | { kind: 'advance' }
  | { kind: 'choose'; choiceId: string }
  | { kind: 'completeCommand'; token: string; result: PreviewHostResult }
  | { kind: 'restore' }
export const narrativeCompile = (
  commands: Record<string, VarType[]> = {},
): Promise<CompileReport> => call('narrative_compile', { commands })
export const narrativePreviewStart = (
  graph: PreviewGraph,
  sceneId: string,
  initialState: PreviewState,
  seed = 0,
): Promise<PreviewFrame> => call('narrative_preview_start', { graph, sceneId, initialState, seed })
export const narrativePreviewStep = (
  graph: PreviewGraph,
  snapshot: PreviewSnapshot,
  action: PreviewAction,
): Promise<PreviewFrame> => call('narrative_preview_step', { graph, snapshot, action })
