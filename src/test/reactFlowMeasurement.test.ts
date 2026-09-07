import { describe, expect, it } from 'vitest'

/**
 * The guard on the two stubs in `setup.ts`.
 *
 * Without a synchronously-firing `ResizeObserver`, React Flow considers every
 * node unmeasured and draws **no edges at all** — and a canvas test that
 * asserts on edges then passes against an empty list. The failure mode is a
 * suite that looks green and checks nothing, which is worse than a red one.
 *
 * So the stubs get tests of their own, here, next to where they are installed.
 * Delete either stub and these fail immediately and by name, rather than
 * leaving somebody to work out why `FlowCanvas.test.tsx` went quiet.
 */

describe('the jsdom stubs React Flow needs', () => {
  it('fires the resize observer synchronously, inside observe', () => {
    // The whole point. A stub that queued a microtask would leave React Flow
    // unmeasured for the rest of the test, and it would look identical here
    // unless the assertion runs before any await.
    const element = document.createElement('div')
    element.style.width = '224px'
    element.style.height = '104px'

    let entries: ResizeObserverEntry[] = []
    const observer = new ResizeObserver((seen) => {
      entries = seen
    })
    observer.observe(element)

    expect(entries).toHaveLength(1)
    expect(entries[0]?.contentRect.width).toBe(224)
    expect(entries[0]?.contentRect.height).toBe(104)
    observer.disconnect()
  })

  it('measures elements inside a React Flow plane and leaves everything else alone', () => {
    const plane = document.createElement('div')
    plane.className = 'react-flow'
    const node = document.createElement('div')
    node.style.width = '224px'
    node.style.height = '104px'
    plane.append(node)
    const outside = document.createElement('div')
    outside.style.width = '224px'
    document.body.append(plane, outside)

    expect(node.getBoundingClientRect().width).toBe(224)
    expect(plane.getBoundingClientRect().width).toBeGreaterThan(0)
    // Scoped on purpose: every other suite keeps jsdom's own zero-sized boxes,
    // so this stub cannot quietly change the geometry the whole run sees.
    expect(outside.getBoundingClientRect().width).toBe(0)

    plane.remove()
    outside.remove()
  })
})
