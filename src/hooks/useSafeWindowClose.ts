import { useEffect, useRef } from 'react'
import { isTauri } from '../lib/api'
import { closeForWindowExit, reportProjectCloseFailure } from '../lib/projectClose'
import { closeWindow, destroyWindow, onCloseRequested } from '../lib/window'

/**
 * The renderer's half of the exit policy — see `docs/15-exit-policy.md`.
 *
 * A normal OS/titlebar close is asynchronous: refuse the first request, settle
 * every registered editor write, warn about anything still generating, close
 * the backend project, then repeat the window close with a one-shot permit. A
 * failed save leaves the renderer and its controlled inputs alive so the user
 * can recover.
 *
 * Every close arrives here, and that is the point of doing it this way. The
 * window is frameless, so the titlebar's own button is `close()` rather than a
 * native control — but the OS menu, a window-manager keybinding, `Alt+F4` and a
 * `quit` from the dock all raise the same `CloseRequested`, so there is one
 * gate rather than one per affordance.
 *
 * `projectOpen` decides only whether the backend project needs closing. The
 * gate itself runs either way, because the job queue outlives the project it
 * was started for: quitting from the launcher can still destroy a generation.
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
    // Set only by the user answering the warning, and never reset: the answer
    // is about this quit, and there is no later in which it could go stale.
    let discardJobs = false

    const retry = () => void closeWindow()
    const quitAnyway = () => {
      discardJobs = true
      void closeWindow()
    }

    void onCloseRequested(async (event) => {
      if (permitClose) return
      event.preventDefault()
      if (closing) return
      closing = true
      try {
        await closeForWindowExit({ projectOpen: projectOpenRef.current, discardJobs })
        permitClose = true
        // We are already inside Tauri's async close-request callback. Calling
        // `close()` here emits a second close request and can leave the first
        // request waiting on itself on some webview/runtime combinations.
        // `destroy()` is safe only after the editor, job and backend gates
        // above.
        await destroyWindow()
      } catch (error) {
        reportProjectCloseFailure(error, { retry, quitAnyway })
      } finally {
        closing = false
      }
    })
      .then((stop) => {
        if (disposed) stop()
        else unlisten = stop
      })
      .catch((error) => reportProjectCloseFailure(error, { retry, quitAnyway }))

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])
}
