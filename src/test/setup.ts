import { afterEach } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'

// React Testing Library only auto-cleans when Vitest globals are on, and they
// are not (see vitest.config.ts). Unmounting between tests matters more here
// than usual: useAutosaveNode flushes a pending save on unmount, so a component
// left mounted would fire its write into the *next* test's mock.
afterEach(cleanup)

/* ── making React Flow measurable in jsdom ────────────────────────────────────

   READ THIS BEFORE DELETING ANY OF IT.

   React Flow measures every node with a `ResizeObserver` and refuses to draw an
   edge between two nodes it believes are unmeasured. jsdom has no
   `ResizeObserver` at all and gives every element a zero-sized bounding box, so
   without the two stubs below a canvas test renders **nodes and no edges** — and
   a test that asserts on edges then passes while asserting on an empty list.
   That is the specific failure this block exists to prevent: not a broken test,
   a *vacuous* one.

   `src/test/reactFlowMeasurement.test.ts` guards both stubs directly, and
   `src/components/narrative/flow/FlowCanvas.test.tsx` asserts a non-zero edge
   count, so removing either of these turns at least two tests red rather than
   quietly turning a suite into a no-op.

   Both stubs are deliberately narrow. The observer fires synchronously on
   `observe`, because React Flow needs the measurement in the same commit; and
   the rect override only answers for elements inside a `.react-flow` root,
   leaving every other suite with jsdom's own zero-sized boxes rather than
   silently changing the geometry the whole test run sees. */

/**
 * The nominal size of a React Flow pane in a test.
 *
 * Deliberately large. `onlyRenderVisibleElements` is on in the product, and it
 * culls by intersecting node boxes with the visible viewport — which, against a
 * stubbed viewport, culls most of a scene and makes every count in a canvas
 * test a fact about this constant rather than about the canvas. A pane big
 * enough to hold the whole fixture takes that variable out of the tests without
 * turning the option off and testing a configuration the app never runs.
 *
 * The spike raised the same flag from the other side: its "nodes in DOM" column
 * under `onlyRenderVisibleElements` read 1 at every size, for exactly this
 * reason, and was dropped from the note as not a real measurement.
 */
const PANE = { width: 4096, height: 4096 }

/**
 * A length in `px`, or null.
 *
 * Strict about the unit for a reason: React Flow's own root carries
 * `width: 100%`, and a lenient `parseFloat` reads that as a hundred-pixel pane
 * — which then culls most of the canvas under `onlyRenderVisibleElements` and
 * makes every node count in a test a fact about that mistake.
 */
function pixels(value: string | undefined): number | null {
  if (!value || !value.endsWith('px')) return null
  const parsed = Number.parseFloat(value)
  return Number.isFinite(parsed) ? parsed : null
}

/** A port, which React Flow measures separately to route an edge to it. */
const HANDLE = { width: 8, height: 8 }

function measured(element: Element): { width: number; height: number } {
  // A node wrapper carries its size as an inline style, because the canvas
  // gives layout fixed node sizes rather than measuring the DOM. Anything else
  // inside the plane is a handle or the pane itself.
  const style = element instanceof HTMLElement ? element.style : null
  const width = pixels(style?.width)
  const height = pixels(style?.height)
  if (width !== null && height !== null) return { width, height }
  if (element.classList?.contains('react-flow__handle')) return HANDLE
  return PANE
}

class SynchronousResizeObserver implements ResizeObserver {
  private targets = new Set<Element>()

  constructor(private readonly callback: ResizeObserverCallback) {}

  observe(target: Element) {
    this.targets.add(target)
    const { width, height } = measured(target)
    const rect = { x: 0, y: 0, top: 0, left: 0, right: width, bottom: height, width, height }
    // Synchronously, and in this call. An observer that queued a microtask
    // would leave React Flow unmeasured for the rest of the test.
    this.callback(
      [
        {
          target,
          contentRect: rect as DOMRectReadOnly,
          borderBoxSize: [{ inlineSize: width, blockSize: height }],
          contentBoxSize: [{ inlineSize: width, blockSize: height }],
          devicePixelContentBoxSize: [{ inlineSize: width, blockSize: height }],
        },
      ],
      this,
    )
  }

  unobserve(target: Element) {
    this.targets.delete(target)
  }

  disconnect() {
    this.targets.clear()
  }
}

globalThis.ResizeObserver = SynchronousResizeObserver

/*
 * The offset properties, which are the ones that actually decide it.
 *
 * React Flow measures a node with `node.offsetWidth` / `node.offsetHeight`, and
 * treats a node of zero size as unmeasured: it renders it `visibility: hidden`
 * and routes no edge to or from it. jsdom hard-codes both to 0, so without this
 * the canvas mounts, draws every box invisibly, and draws no edges at all.
 */
for (const property of ['offsetWidth', 'offsetHeight'] as const) {
  Object.defineProperty(HTMLElement.prototype, property, {
    configurable: true,
    get(this: HTMLElement) {
      if (!this.closest?.('.react-flow')) return 0
      return property === 'offsetWidth' ? measured(this).width : measured(this).height
    },
  })
}

/*
 * `getBBox`, for the label on a back edge.
 *
 * jsdom implements no SVG geometry at all, and React Flow's `EdgeText` measures
 * its own text to size the plate behind it. Without this, rendering a single
 * labelled edge throws and takes the whole canvas down with it.
 */
if (!('getBBox' in SVGElement.prototype)) {
  Object.defineProperty(SVGElement.prototype, 'getBBox', {
    configurable: true,
    value: () => ({ x: 0, y: 0, width: 0, height: 0 }),
  })
}

const jsdomRect = Element.prototype.getBoundingClientRect
Element.prototype.getBoundingClientRect = function getBoundingClientRect(this: Element) {
  if (!this.closest?.('.react-flow')) return jsdomRect.call(this)
  const { width, height } = measured(this)
  return {
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: width,
    bottom: height,
    width,
    height,
    toJSON: () => ({}),
  } as DOMRect
}

/**
 * The third thing jsdom does not have.
 *
 * React Flow reads the pane's zoom straight off its computed transform with
 * `new window.DOMMatrixReadOnly(style.transform).m22`, and jsdom has no
 * `DOMMatrix` of any kind, so mounting the canvas throws before a single node
 * is measured. Only `m11` and `m22` are ever read, so this parses the two
 * transform forms that reach it and answers 1 for anything else — which is
 * exactly what `transform: none` means.
 */
class MatrixReadOnly {
  m11 = 1
  m22 = 1

  constructor(transform?: string) {
    const matrix = /matrix\(\s*([-\d.]+)\s*,\s*[-\d.]+\s*,\s*[-\d.]+\s*,\s*([-\d.]+)/.exec(
      transform ?? '',
    )
    if (matrix) {
      this.m11 = Number(matrix[1])
      this.m22 = Number(matrix[2])
      return
    }
    const scale = /scale\(\s*([-\d.]+)/.exec(transform ?? '')
    if (scale) {
      this.m11 = Number(scale[1])
      this.m22 = Number(scale[1])
    }
  }
}

if (!('DOMMatrixReadOnly' in globalThis)) {
  ;(globalThis as unknown as { DOMMatrixReadOnly: unknown }).DOMMatrixReadOnly = MatrixReadOnly
}
