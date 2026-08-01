import * as api from './api'
import { editorWrites, isEditorWritesBlocked } from './editorWrites'
import { report, useUI } from '../store/ui'

export const EDITOR_CLOSE_BLOCKED = 'editor.close_blocked'

/** The only safe ordering for either a project close or a normal window exit. */
export async function closeProjectAfterEditorWrites(): Promise<void> {
  await editorWrites.flushAll()
  await api.projectClose()
  useUI.getState().clearBanner(EDITOR_CLOSE_BLOCKED)
}

/** Keep the workspace mounted and give the user a deliberate retry path. */
export function reportProjectCloseFailure(error: unknown, retry: () => void): void {
  if (!isEditorWritesBlocked(error)) {
    report(error, 'Could not close project')
    return
  }

  const count = Math.max(1, error.writes.length)
  useUI.getState().raiseBanner({
    code: EDITOR_CLOSE_BLOCKED,
    text:
      `Wobu kept this project open because ${count} editor ${count === 1 ? 'write has' : 'writes have'} ` +
      'not saved. Resolve any save error or conflict, then retry.',
    retryable: true,
    sticky: true,
    action: { label: 'Retry save and close', run: retry },
  })
}
