import { create } from 'zustand'
import type {
  CompileDiagnostic,
  PreviewFrame,
  PreviewGraph,
  ExecutionTrace,
} from '../../lib/api/narrativePreview'

export interface PreviewSession {
  graph: PreviewGraph
  frame: PreviewFrame
  trace: { label: string; execution: ExecutionTrace }[]
  bookmark?: PreviewFrame
}
export const usePreviewSessions = create<{
  busy: Record<string, boolean>
  setBusy: (key: string, busy: boolean) => void
  sessions: Record<string, PreviewSession>
  diagnostics: Record<string, CompileDiagnostic[]>
  put: (key: string, session: PreviewSession) => void
  report: (key: string, diagnostics: CompileDiagnostic[]) => void
}>((set) => ({
  busy: {},
  setBusy: (key, busy) => set((state) => ({ busy: { ...state.busy, [key]: busy } })),
  sessions: {},
  diagnostics: {},
  put: (key, session) => set((state) => ({ sessions: { ...state.sessions, [key]: session } })),
  report: (key, diagnostics) =>
    set((state) => ({ diagnostics: { ...state.diagnostics, [key]: diagnostics } })),
}))
