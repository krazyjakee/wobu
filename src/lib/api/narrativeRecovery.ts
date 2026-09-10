import { call } from './call'

export interface RetainedNarrativeDeletion {
  id: string
  name: string
  target: string
  hash: string
  /** An explicit restoration was requested; a newer current file may still have won. */
  restored: boolean
}
export type NarrativeRestoration =
  { status: 'saved'; id: string } | { status: 'conflict'; id: string; conflictPath: string }
export const narrativeRecoveryList = (): Promise<RetainedNarrativeDeletion[]> =>
  call('narrative_recovery_list')
export const narrativeRecoveryRestore = (id: string): Promise<NarrativeRestoration> =>
  call('narrative_recovery_restore', { id })
