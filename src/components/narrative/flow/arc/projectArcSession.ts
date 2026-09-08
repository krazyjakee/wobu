import { createFlowStore } from '../flowStore'
import type { ArcView } from './ArcFlow'

const sessions = new Map<
  string,
  {
    scope: string
    views: Map<string, ArcView>
    stores: Map<string, ReturnType<typeof createFlowStore>>
    scroll: number
  }
>()
export function projectArcSession(project: string) {
  let session = sessions.get(project)
  if (!session) {
    session = { scope: '', views: new Map(), stores: new Map(), scroll: 0 }
    sessions.set(project, session)
  }
  return session
}
