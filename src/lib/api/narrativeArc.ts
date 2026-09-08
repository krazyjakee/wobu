import { call } from './call'
import type { SceneCatalog, SceneSummary } from './narrative'

export interface ArcExit {
  beatId: string
  routeId: string
  choice: boolean
  label: string
  to: string | null
}
export interface ArcScene {
  summary: SceneSummary
  actId: string | null
  arcId: string | null
  tagIds: string[]
  participants: string[]
  slots: number
  filled: number
  beats: number
  counts: {
    generated: number
    edited: number
    locked: number
    needsReview: number
    outOfDate: number
  }
  arc: { exits: ArcExit[]; needsText: number }
}
export interface ProjectArc {
  revision: string
  scenes: ArcScene[]
  unreadable: SceneCatalog['unreadable']
}
export const narrativeArc = () => call<ProjectArc>('narrative_arc')
