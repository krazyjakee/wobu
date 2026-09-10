import { call } from './call'
import type { NarrativeDiagnostic, Scene, SceneFile, Stamp } from './narrative'

export interface NarrativeSource {
  yaml: string
  file: SceneFile | null
  rel: string
  stamp: Stamp
  sceneId: string | null
  repairBlocked: boolean
  problem: SourceCheck['problem']
}

export interface SourceCheck {
  scene: Scene | null
  formatted: string | null
  problem: { message: string; location: { line: number; column: number } | null } | null
  diagnostics: (NarrativeDiagnostic & { sourcePath?: (string | number)[] })[]
}

export const narrativeSourceGet = (sceneId: string): Promise<NarrativeSource> =>
  call('narrative_source_get', { sceneId })

export const narrativeSourceCheck = (sceneId: string | null, yaml: string): Promise<SourceCheck> =>
  call('narrative_source_check', { sceneId, yaml })

export const narrativeSourceOpen = (rel: string): Promise<NarrativeSource> =>
  call('narrative_source_open', { rel })

export const narrativeSourceRepair = (
  source: NarrativeSource,
  yaml: string,
): Promise<{ source: NarrativeSource; recoveryRel: string }> =>
  call('narrative_source_repair', {
    rel: source.rel,
    yaml,
    expected: source.stamp,
    sceneId: source.sceneId,
  })
