import { call } from './call'
import type { Condition, SceneFile, VariableDecl } from './narrative'
import type { NarrativeBuild } from './narrativeBuild'
export interface AnalysisPolicy {
  version: number
  target: { scene: string; beat: string }
  initial: Record<string, unknown>[]
  invariants: Condition[]
  quests: { quest: string; variable: string }[]
  events: { event: string; effects: unknown[] }[]
  external: boolean
  limits: { states: number; variants: number; milliseconds: number; witness_steps?: number }
}
interface Count {
  value: string
  exact: boolean
}
export interface AnalysisRow {
  id: string
  values: Record<string, unknown>
  when: Condition
  classification: 'included' | 'excluded' | 'unknown'
  reason: string
  witness: null | {
    state: Record<string, unknown>
    transitions: string[]
    initial: number
    target: { scene: string; beat: string }
    runtime_route_verified: boolean
  }
  coverage: { slot: string; matching: string[]; selected: string | null; fallback: string | null }[]
}
export interface AnalysisReport {
  id: string
  source_guard: string
  world_guard: string
  policies: { policies: AnalysisPolicy[]; guard: string }
  report: {
    target: { scene: string; beat: string }
    domains: { name: string; ty: VariableDecl['type'] }[]
    potential: Count
    included: Count
    excluded: Count
    unknown: Count
    rows: AnalysisRow[]
    complete: boolean
    explored_states: number
    limits: AnalysisPolicy['limits']
    reasons: string[]
    assumptions: string[]
  }
}
export interface AnalysisView {
  policy: AnalysisPolicy
  capture: { policies: AnalysisPolicy[]; guard: string }
  latest: AnalysisReport | null
  stale: boolean
}
export const narrativeVariantsGet = (scene: string, beat: string) =>
  call<AnalysisView>('narrative_variants_get', { scene, beat })
export const narrativeVariantsPlan = (source: string, guard: string) =>
  call<AnalysisReport>('narrative_variants_plan', { source, guard })
export const narrativeVariantsMaterialize = (report: string, slot: string, rows: string[]) =>
  call<{ before: SceneFile; after: SceneFile }>('narrative_variants_materialize', {
    report,
    slot,
    rows,
  })
export const narrativeVariantsBuild = (report: string, slot: string, rows: string[]) =>
  call<NarrativeBuild>('narrative_variants_build', { report, slot, rows })
