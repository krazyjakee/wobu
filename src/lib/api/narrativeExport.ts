import { call } from './call'
import type { VarType } from './narrative'
import type { CompileDiagnostic } from './narrativePreview'

export interface ExportOptions {
  profile: 'development' | 'release'
  commands: Record<string, VarType[]>
  debug: boolean
}
export interface ExportCheck {
  diagnostics: CompileDiagnostic[]
  localeDiagnostics?: import('./narrativeLocale').LocaleDiagnostic[]
  mediaDiagnostics?: import('./narrativeMedia').MediaDiagnostic[]
  payloadHash: string | null
  scenes: number
  strings: number
  bytes: number
}
export interface NarrativeExportReport {
  destination: string
  payloadHash: string
  scenes: number
  strings: number
  bytes: number
}
export const narrativeExportCheck = (options: ExportOptions): Promise<ExportCheck> =>
  call('narrative_export_check', { ...options })
export const narrativeExport = (
  options: ExportOptions,
  destination: string,
  expectedHash: string,
): Promise<NarrativeExportReport> =>
  call('narrative_export', { ...options, destination, expectedHash })
