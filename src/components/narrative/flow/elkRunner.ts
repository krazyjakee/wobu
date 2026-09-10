import ELK, { type ELK as ElkEngine } from 'elkjs/lib/elk-api.js'
import workerUrl from 'elkjs/lib/elk-worker.min.js?url'
import { elkGraph, elkPositions } from './elkGraph'
import type { LayoutRunner } from './layout'

let engine: ElkEngine | null = null
let activeWorker: Worker | null = null
const pending = new Set<(error: Error) => void>()

/** Use ELK's upstream worker directly. Its bundled in-process Worker shim cannot
 * be constructed inside a real browser Worker (the worker entry exports no shim).
 * Vite copies the unmodified classic script as an asset, keeping layout off-thread. */
export const elkLayout: LayoutRunner = (request) => {
  if (!engine) {
    engine = new ELK({
      workerFactory: () => {
        const worker = new Worker(workerUrl)
        activeWorker = worker
        const failed = () => {
          if (activeWorker !== worker) return
          activeWorker = null
          engine = null
          worker.terminate()
          for (const reject of pending)
            reject(new Error('The layout worker stopped. Try Auto layout again.'))
          pending.clear()
        }
        worker.addEventListener('error', failed)
        worker.addEventListener('messageerror', failed)
        return worker
      },
    })
  }
  const current = engine
  return new Promise((resolve, reject) => {
    pending.add(reject)
    current.layout(elkGraph(request)).then(
      (graph) => {
        pending.delete(reject)
        resolve({ positions: elkPositions(graph) })
      },
      (error: unknown) => {
        pending.delete(reject)
        reject(error)
      },
    )
  })
}
