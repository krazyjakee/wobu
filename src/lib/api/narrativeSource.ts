import { call } from './call'
import type { NarrativeDiagnostic, Scene, SceneFile } from './narrative'

export interface NarrativeSource {
  yaml: string
  file: SceneFile
}

export interface SourceCheck {
  scene: Scene | null
  formatted: string | null
  problem: { message: string; location: { line: number; column: number } | null } | null
  diagnostics: NarrativeDiagnostic[]
}

export const narrativeSourceGet = (sceneId: string): Promise<NarrativeSource> =>
  call('narrative_source_get', { sceneId })

export const narrativeSourceCheck = (sceneId: string, yaml: string): Promise<SourceCheck> =>
  call('narrative_source_check', { sceneId, yaml })
