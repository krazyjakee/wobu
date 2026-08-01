import { useEffect, useRef } from 'react'
import { isTauri } from '../lib/api'
import { closeProjectAfterEditorWrites, reportProjectCloseFailure } from '../lib/projectClose'
import { closeWindow, onCloseRequested } from '../lib/window'

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
        await closeWindow()
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
