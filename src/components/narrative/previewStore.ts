import { create } from 'zustand'
import type { Scenario } from '../../lib/api/narrativeScenarios'
import type { ScenarioTape } from './scenarioTape'
import type {
  CompileDiagnostic,
  PreviewFrame,
  PreviewGraph,
  ExecutionTrace,
} from '../../lib/api/narrativePreview'

export interface PreviewSession {
  tape?: ScenarioTape
  graph: PreviewGraph
  frame: PreviewFrame
  trace: { label: string; execution: ExecutionTrace }[]
  bookmark?: PreviewFrame
}
/**
 * A saved scenario opened as an overlay rather than played (#162, #188).
 *
 * The tape and nothing else. Opening one draws a route on the Flow canvas and
 * starts no runtime at all, which is what makes "opened without replaying it"
 * a property of the data rather than a promise about the code: there is no
 * snapshot here for anything to advance.
 */
export interface OpenedScenario {
  name: string
  scenario: Scenario
}
export const usePreviewSessions = create<{
  busy: Record<string, boolean>
  setBusy: (key: string, busy: boolean) => void
  sessions: Record<string, PreviewSession>
  diagnostics: Record<string, CompileDiagnostic[]>
  /** Scenarios opened as overlays, per scene. Null clears one. */
  opened: Record<string, OpenedScenario>
  put: (key: string, session: PreviewSession) => void
  report: (key: string, diagnostics: CompileDiagnostic[]) => void
  open: (key: string, scenario: OpenedScenario | null) => void
}>((set) => ({
  busy: {},
  setBusy: (key, busy) => set((state) => ({ busy: { ...state.busy, [key]: busy } })),
  sessions: {},
  diagnostics: {},
  opened: {},
  put: (key, session) => set((state) => ({ sessions: { ...state.sessions, [key]: session } })),
  report: (key, diagnostics) =>
    set((state) => ({ diagnostics: { ...state.diagnostics, [key]: diagnostics } })),
  open: (key, scenario) =>
    set((state) => {
      const opened = { ...state.opened }
      if (scenario) opened[key] = scenario
      else delete opened[key]
      return { opened }
    }),
}))
