import { call } from './call'
import type { Speaker, StateValue, VarType } from './narrative'

export type PreviewState = Record<string, StateValue>
/** Opaque Rust-owned values; the webview carries them without rewriting. */
export type PreviewGraph = Record<string, unknown>
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
export interface PreviewFrame {
  snapshot: PreviewSnapshot
  current: PreviewYield
  state: PreviewState
}
export type PreviewAction =
  | { kind: 'advance' }
  | { kind: 'choose'; choiceId: string }
  | { kind: 'completeCommand'; token: string }
  | { kind: 'restore' }
export const narrativeCompile = (
  commands: Record<string, VarType[]> = {},
): Promise<CompileReport> => call('narrative_compile', { commands })
export const narrativePreviewStart = (
  graph: PreviewGraph,
  sceneId: string,
  initialState: PreviewState,
): Promise<PreviewFrame> => call('narrative_preview_start', { graph, sceneId, initialState })
export const narrativePreviewStep = (
  graph: PreviewGraph,
  snapshot: PreviewSnapshot,
  action: PreviewAction,
): Promise<PreviewFrame> => call('narrative_preview_step', { graph, snapshot, action })
