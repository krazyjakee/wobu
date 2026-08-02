import { useEffect, useRef } from 'react'
import { isTauri } from '../lib/api'
import { closeProjectAfterEditorWrites, reportProjectCloseFailure } from '../lib/projectClose'
import { closeWindow, destroyWindow, onCloseRequested } from '../lib/window'

/**
 * A normal OS/titlebar close is asynchronous: refuse the first request, settle
 * every registered editor write, close the backend project, then repeat the
 * window close with a one-shot permit. A failed save leaves the renderer and
 * its controlled inputs alive so the user can recover.
 */
export function useSafeWindowClose(projectOpen: boolean): void {
  const projectOpenRef = useRef(projectOpen)

  useEffect(() => {
    projectOpenRef.current = projectOpen
  }, [projectOpen])

  useEffect(() => {
    if (!isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    let closing = false
    let permitClose = false

    const requestAgain = () => void closeWindow()
    void onCloseRequested(async (event) => {
      if (permitClose || !projectOpenRef.current) return
      event.preventDefault()
      if (closing) return
      closing = true
      try {
        await closeProjectAfterEditorWrites()
        permitClose = true
        // We are already inside Tauri's async close-request callback. Calling
        // `close()` here emits a second close request and can leave the first
        // request waiting on itself on some webview/runtime combinations.
        // `destroy()` is safe only after the editor and backend gates above.
        await destroyWindow()
      } catch (error) {
        reportProjectCloseFailure(error, requestAgain)
      } finally {
        closing = false
      }
    })
      .then((stop) => {
        if (disposed) stop()
        else unlisten = stop
      })
      .catch((error) => reportProjectCloseFailure(error, requestAgain))

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])
}
