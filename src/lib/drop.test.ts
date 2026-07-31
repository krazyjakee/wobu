import { describe, expect, it } from 'vitest'
import { canDrop, type DropContext } from './drop'
import { descendantsOf, indexNodes } from './tree'
import { kindDef, kindIndex, summary } from '../test/fixtures'

/*
 * A world shaped to exercise every rule:
 *
 *   species   vashk
 *               └── ashborn
 *                     └── deepwalker
 *             aurelian
 *   character kell
 */
const nodes = [
  summary({ id: 'vashk', kind: 'species' }),
  summary({ id: 'ashborn', kind: 'species', parentId: 'vashk' }),
  summary({ id: 'deepwalker', kind: 'species', parentId: 'ashborn' }),
  summary({ id: 'aurelian', kind: 'species' }),
  summary({ id: 'kell', kind: 'character' }),
]

const byId = indexNodes(nodes)

function dragging(id: string, over: Partial<DropContext> = {}): DropContext {
  return {
    dragId: id,
    byId,
    forbidden: descendantsOf(id, nodes),
    kinds: kindIndex([kindDef('species'), kindDef('character')]),
    readOnly: false,
    ...over,
  }
}

describe('canDrop', () => {
  it('allows a plain reparent within one kind', () => {
    expect(canDrop(dragging('aurelian'), 'vashk', 'species')).toBe(true)
  })

  it('refuses a drop across kinds', () => {
    // Nesting is same-kind by design; a character under a species has no
    // meaning in the data model and no folder to live in.
    expect(canDrop(dragging('kell'), 'vashk', 'species')).toBe(false)
  })

  it('refuses a drop onto itself', () => {
    expect(canDrop(dragging('vashk'), 'vashk', 'species')).toBe(false)
  })

  it('refuses a drop onto a direct child', () => {
    // This is the one that detaches a subtree from the roots: vashk's parent
    // becomes ashborn, whose parent is vashk, and neither is reachable again.
    expect(canDrop(dragging('vashk'), 'ashborn', 'species')).toBe(false)
  })

  it('refuses a drop onto a deeper descendant', () => {
    expect(canDrop(dragging('vashk'), 'deepwalker', 'species')).toBe(false)
  })

  it('refuses a no-op reparent onto the current parent', () => {
    // It would write the file, bump updatedAt and wake every other client on
    // the share to tell them nothing changed.
    expect(canDrop(dragging('ashborn'), 'vashk', 'species')).toBe(false)
  })

  it('refuses everything while there is no drag in progress', () => {
    expect(canDrop(dragging('vashk', { dragId: null }), 'aurelian', 'species')).toBe(false)
  })

  it('refuses when the dragged node is not in the index', () => {
    expect(canDrop(dragging('ghost', { forbidden: new Set() }), 'vashk', 'species')).toBe(false)
  })

  it('refuses every drop on a read-only project', () => {
    expect(canDrop(dragging('aurelian', { readOnly: true }), 'vashk', 'species')).toBe(false)
  })

  it('refuses nesting into a kind whose registry entry says it does not nest', () => {
    const flat = kindIndex([kindDef('species', { nests: false }), kindDef('character')])
    expect(canDrop(dragging('aurelian', { kinds: flat }), 'vashk', 'species')).toBe(false)
  })

  it('allows an unknown kind to nest rather than blocking the drag outright', () => {
    // The registry arrives asynchronously. Between open and first paint the
    // index is empty, and a UI that refuses every drag in that window reads as
    // broken. Rust re-checks the move regardless.
    expect(canDrop(dragging('aurelian', { kinds: new Map() }), 'vashk', 'species')).toBe(true)
  })

  describe('the group header — a drop target meaning "top level"', () => {
    it('accepts a nested node', () => {
      expect(canDrop(dragging('ashborn'), null, 'species')).toBe(true)
    })

    it('refuses a node that is already at the top level', () => {
      expect(canDrop(dragging('aurelian'), null, 'species')).toBe(false)
    })

    it('refuses a header of another kind', () => {
      expect(canDrop(dragging('ashborn'), null, 'character')).toBe(false)
    })

    it('accepts even when the kind does not nest — there must be a way out', () => {
      // A node parented before the registry said `nests: false`, or by a hand
      // edit. If the escape hatch closed too, it would be stuck forever.
      const flat = kindIndex([kindDef('species', { nests: false })])
      expect(canDrop(dragging('ashborn', { kinds: flat }), null, 'species')).toBe(true)
    })
  })
})
