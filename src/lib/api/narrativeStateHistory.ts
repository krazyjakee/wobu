import { call } from './call'
import type { StateDocument, StateFile } from './narrative'
/** Undo compares the authored document before using its current guarded source stamp. */
export const narrativeStateRestore = (document: StateDocument, expected: StateDocument) =>
  call<StateFile>('narrative_state_restore', { document, expected })
