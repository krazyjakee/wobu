import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { councilHearing } from './fixture'
import { buildGraph } from './graph'
import { layoutRequest } from './layout'

/** The real upstream classic worker script, isolated in a worker-like JS realm.
 * This exercises its message protocol and actual layout, not a fake position reply.
 * Native browser execution is separately recorded in docs/17-flow-canvas.md. */
const fs = (await import('node:' + 'fs')) as {
  readFileSync(path: string, encoding: 'utf8'): string
}
const vm = (await import('node:' + 'vm')) as {
  runInNewContext(source: string, scope: object): unknown
}
const script = fs.readFileSync('node_modules/elkjs/lib/elk-worker.min.js', 'utf8')
class WorkerHarness extends EventTarget {
  static created: WorkerHarness[] = []
  onmessage: ((event: MessageEvent) => void) | null = null
  stopped = false
  receive: (json: string) => void
  constructor(readonly url: string) {
    super()
    WorkerHarness.created.push(this)
    const scope = {
      onmessage: undefined as ((event: { data: unknown }) => void) | undefined,
      postMessage: (data: unknown) =>
        queueMicrotask(() => {
          if (!this.stopped) this.onmessage?.(new MessageEvent('message', { data }))
        }),
      console,
      setTimeout,
      clearTimeout,
    }
    this.receive = vm.runInNewContext(
      'self = globalThis;\n' + script + '\n(json) => self.onmessage({data: JSON.parse(json)})',
      scope,
    ) as (json: string) => void
  }
  postMessage(data: unknown) {
    queueMicrotask(() => {
      if (!this.stopped) this.receive(JSON.stringify(data))
    })
  }
  terminate() {
    this.stopped = true
  }
}

beforeEach(() => {
  vi.resetModules()
  WorkerHarness.created = []
  vi.stubGlobal('Worker', WorkerHarness)
})
afterEach(() => vi.unstubAllGlobals())
it('uses the upstream worker asset and resolves concurrent grouped layouts with finite positions', async () => {
  const { elkLayout } = await import('./elkRunner')
  const request = layoutRequest(buildGraph(councilHearing()))
  const [first, second] = await Promise.all([elkLayout(request), elkLayout(request)])
  expect(WorkerHarness.created).toHaveLength(1)
  expect(WorkerHarness.created[0]!.url).toContain('elk-worker.min.js')
  expect(first).toEqual(second)
  for (const node of request.nodes) {
    expect(first.positions[node.id]).toEqual({ x: expect.any(Number), y: expect.any(Number) })
    expect(Number.isFinite(first.positions[node.id]!.x)).toBe(true)
  }
  expect(first.positions['beat.2']!.x).toBeGreaterThan(first.positions['group.evidence']!.x)
})
it('rejects pending layouts when the worker fails and starts a fresh worker on retry', async () => {
  const { elkLayout } = await import('./elkRunner')
  const request = layoutRequest(buildGraph(councilHearing()))
  const first = elkLayout(request),
    second = elkLayout(request)
  const rejected = Promise.all([
    expect(first).rejects.toThrow('worker stopped'),
    expect(second).rejects.toThrow('worker stopped'),
  ])
  WorkerHarness.created[0]!.dispatchEvent(new Event('error'))
  await rejected
  expect(WorkerHarness.created[0]!.stopped).toBe(true)
  const retry = elkLayout(request)
  WorkerHarness.created[0]!.dispatchEvent(new Event('error'))
  const result = await retry
  expect(WorkerHarness.created).toHaveLength(2)
  expect(result.positions['beat.1']).toBeDefined()
})
