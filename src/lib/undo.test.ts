import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  COALESCE_MS,
  MAX_ENTRIES,
  birthEntry,
  deletionEntry,
  editEntry,
  editLabel,
  moveEntry,
  undoIntent,
  useUndoStack,
  type NewEntry,
  type UndoEntry,
  type WorldCommand,
} from './undo'
import { node } from '../test/fixtures'

/** An entry with only the fields a given test cares about spelled out. */
function entry(over: Partial<UndoEntry> & { nodeId: string }): NewEntry {
  return {
    label: `edit ${over.nodeId}`,
    undo: [{ type: 'delete', id: over.nodeId }],
    redo: [{ type: 'delete', id: over.nodeId }],
    coalesce: false,
    ...over,
  }
}

/** An edit of `id`, carrying the two states it moved between. */
function edit(id: string, before: string, after: string, at?: number) {
  return entry({
    nodeId: id,
    label: `notes edit ${after}`,
    undo: [{ type: 'upsert', node: node({ id, notesRaw: before }) }],
    redo: [{ type: 'upsert', node: node({ id, notesRaw: after }) }],
    coalesce: true,
    at,
  })
}

/** Records what reached the backend, in order, and never fails. */
function recorder() {
  const seen: WorldCommand[] = []
  return {
    seen,
    run: async (cmd: WorldCommand) => {
      seen.push(cmd)
    },
  }
}

/** The notes text an `upsert` command would write, for readable assertions. */
function notesOf(cmd: WorldCommand | undefined): string | undefined {
  return cmd && cmd.type === 'upsert' ? cmd.node.notesRaw : undefined
}

beforeEach(() => {
  useUndoStack.setState({ projectId: 'proj', past: [], future: [], busy: false })
})

describe('coalescing a run of typing', () => {
  /*
   * Notes editing reaches node_upsert once per autosave debounce, so without
   * this every ⌘Z would rewind half a second of typing and the user would have
   * to press it thirty times to take back a sentence.
   */

  it('merges consecutive edits to one node into a single entry', () => {
    const push = useUndoStack.getState().push
    push(edit('kell', '', 'a', 1000))
    push(edit('kell', 'a', 'ab', 1400))
    push(edit('kell', 'ab', 'abc', 1800))
    expect(useUndoStack.getState().past).toHaveLength(1)
  })

  it('keeps the state from before the *first* edit of the run as the inverse', () => {
    // The regression this guards is the obvious implementation: overwriting the
    // top entry wholesale, which leaves the inverse pointing at the text from
    // one debounce ago, so undo rewinds a keystroke and calls it done.
    const push = useUndoStack.getState().push
    push(edit('kell', 'original', 'o', 1000))
    push(edit('kell', 'o', 'ov', 1400))
    const top = useUndoStack.getState().past[0]!
    expect(notesOf(top.undo[0])).toBe('original')
    expect(notesOf(top.redo[0])).toBe('ov')
  })

  it('starts a new entry once the window has passed', () => {
    const push = useUndoStack.getState().push
    push(edit('kell', '', 'a', 1000))
    push(edit('kell', 'a', 'ab', 1000 + COALESCE_MS + 1))
    expect(useUndoStack.getState().past).toHaveLength(2)
  })

  it('slides the window, so an unbroken run stays one entry however long it is', () => {
    const push = useUndoStack.getState().push
    let t = 1000
    for (let i = 0; i < 20; i++) {
      push(edit('kell', String(i), String(i + 1), t))
      t += COALESCE_MS - 100
    }
    expect(useUndoStack.getState().past).toHaveLength(1)
  })

  it('never merges edits to different nodes, however fast they arrive', () => {
    // Tabbing between two nodes and typing in both is two separate actions, and
    // merging them would make one ⌘Z revert a node the user is not looking at.
    const push = useUndoStack.getState().push
    push(edit('kell', '', 'a', 1000))
    push(edit('vashk', '', 'a', 1010))
    expect(useUndoStack.getState().past).toHaveLength(2)
  })

  it('never merges structural commands, even on the same node in the same instant', () => {
    // Three deletes collapsing into one ⌘Z would leave two nodes gone with no
    // way back.
    const push = useUndoStack.getState().push
    push(entry({ nodeId: 'kell', coalesce: false, at: 1000 }))
    push(entry({ nodeId: 'kell', coalesce: false, at: 1001 }))
    expect(useUndoStack.getState().past).toHaveLength(2)
  })
})

describe('undo and redo', () => {
  it('runs the entry inverse and moves it onto the redo stack', async () => {
    const { seen, run } = recorder()
    useUndoStack.getState().push(edit('kell', 'before', 'after', 1000))
    await useUndoStack.getState().undo(run)
    expect(notesOf(seen[0])).toBe('before')
    expect(useUndoStack.getState().past).toEqual([])
    expect(useUndoStack.getState().future).toHaveLength(1)
  })

  it('replays the forward command on redo and puts the entry back', async () => {
    const { seen, run } = recorder()
    useUndoStack.getState().push(edit('kell', 'before', 'after', 1000))
    await useUndoStack.getState().undo(run)
    await useUndoStack.getState().redo(run)
    expect(notesOf(seen[1])).toBe('after')
    expect(useUndoStack.getState().past).toHaveLength(1)
    expect(useUndoStack.getState().future).toEqual([])
  })

  it('restores a deleted node under its original id, then reparents its children', async () => {
    // node_create would mint a fresh ULID, so a "restored" node would be a
    // different entity and every link pointing at the original would resolve to
    // nothing. The child moves matter for the same reason the delete does them:
    // the delete promoted them, and undo has to put the shape back.
    const { seen, run } = recorder()
    useUndoStack.getState().push(
      entry({
        nodeId: 'kell',
        undo: [
          { type: 'upsert', node: node({ id: 'kell' }) },
          { type: 'move', id: 'child', parentId: 'kell' },
        ],
        redo: [{ type: 'delete', id: 'kell' }],
      }),
    )
    await useUndoStack.getState().undo(run)
    expect(seen).toEqual([
      { type: 'upsert', node: node({ id: 'kell' }) },
      { type: 'move', id: 'child', parentId: 'kell' },
    ])
  })

  it('undoes in reverse order of the pushes', async () => {
    const { seen, run } = recorder()
    const push = useUndoStack.getState().push
    push(entry({ nodeId: 'first' }))
    push(entry({ nodeId: 'second' }))
    await useUndoStack.getState().undo(run)
    await useUndoStack.getState().undo(run)
    expect(seen).toEqual([
      { type: 'delete', id: 'second' },
      { type: 'delete', id: 'first' },
    ])
  })

  it('keeps the entry when the write is refused, so the user can resolve and retry', async () => {
    // An undo goes through the same guarded write path as any save and can
    // raise write.conflict. Dropping the entry would make a recoverable refusal
    // permanent.
    useUndoStack.getState().push(entry({ nodeId: 'kell' }))
    const run = vi.fn().mockRejectedValue({ code: 'write.conflict', message: 'someone else wrote' })
    await expect(useUndoStack.getState().undo(run)).rejects.toMatchObject({
      code: 'write.conflict',
    })
    expect(useUndoStack.getState().past).toHaveLength(1)
    expect(useUndoStack.getState().future).toEqual([])
    expect(useUndoStack.getState().busy).toBe(false)
  })

  it('does nothing, quietly, when there is nothing to undo', async () => {
    const { seen, run } = recorder()
    expect(await useUndoStack.getState().undo(run)).toBeNull()
    expect(seen).toEqual([])
  })

  it('abandons the redo branch as soon as something new is done', async () => {
    // Redoing onto text that has since diverged would produce a third world
    // that the user never edited their way into.
    const { run } = recorder()
    const push = useUndoStack.getState().push
    push(edit('kell', '', 'a', 1000))
    await useUndoStack.getState().undo(run)
    expect(useUndoStack.getState().future).toHaveLength(1)
    push(edit('kell', '', 'z', 9000))
    expect(useUndoStack.getState().future).toEqual([])
  })

  it('refuses a second undo while one is still in flight', async () => {
    // ⌘Z held down repeats, and two overlapping undos would interleave writes
    // to the same file.
    const push = useUndoStack.getState().push
    push(entry({ nodeId: 'a' }))
    push(entry({ nodeId: 'b' }))
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => (release = r))
    const first = useUndoStack.getState().undo(() => gate)
    expect(await useUndoStack.getState().undo(async () => {})).toBeNull()
    release()
    await first
    expect(useUndoStack.getState().past).toHaveLength(1)
  })

  it('drops the oldest entries rather than growing without bound', () => {
    const push = useUndoStack.getState().push
    for (let i = 0; i < MAX_ENTRIES + 10; i++) push(entry({ nodeId: `n${i}` }))
    const { past } = useUndoStack.getState()
    expect(past).toHaveLength(MAX_ENTRIES)
    expect(past[0]!.nodeId).toBe('n10')
  })
})

describe('project scope', () => {
  it('throws the stack away when a different project is opened', () => {
    // Every command names a node by id in one world. Replaying it against
    // another project would either fail or, worse, hit an unrelated node.
    useUndoStack.getState().push(entry({ nodeId: 'kell' }))
    useUndoStack.getState().setProject('other')
    expect(useUndoStack.getState().past).toEqual([])
    expect(useUndoStack.getState().future).toEqual([])
  })

  it('leaves the stack alone when told the same project again', () => {
    // The scope is synced from a query result that re-renders freely; clearing
    // on every report would empty the stack at random moments.
    useUndoStack.getState().push(entry({ nodeId: 'kell' }))
    useUndoStack.getState().setProject('proj')
    expect(useUndoStack.getState().past).toHaveLength(1)
  })

  it('clears when the project closes and ignores pushes after it', () => {
    useUndoStack.getState().push(entry({ nodeId: 'kell' }))
    useUndoStack.getState().setProject(null)
    useUndoStack.getState().push(entry({ nodeId: 'kell' }))
    expect(useUndoStack.getState().past).toEqual([])
  })
})

describe('undoIntent — who owns ⌘Z', () => {
  const key = (over: Partial<KeyboardEvent> = {}) =>
    ({ key: 'z', metaKey: true, ctrlKey: false, shiftKey: false, ...over }) as KeyboardEvent

  it('claims ⌘Z and ⌃Z for the workspace', () => {
    expect(undoIntent(key(), false)).toBe('undo')
    expect(undoIntent(key({ metaKey: false, ctrlKey: true }), false)).toBe('undo')
  })

  it('reads ⇧⌘Z and ⌃Y as redo', () => {
    expect(undoIntent(key({ shiftKey: true }), false)).toBe('redo')
    expect(undoIntent(key({ key: 'y', metaKey: false, ctrlKey: true }), false)).toBe('redo')
  })

  it('yields to a focused text field', () => {
    // While the caret is in a textarea the field's own undo owns the key.
    // Stealing it would rewind a whole save every time someone tried to take
    // back a word.
    expect(undoIntent(key(), true)).toBeNull()
    expect(undoIntent(key({ shiftKey: true }), true)).toBeNull()
  })

  it('ignores an unmodified z, which is just typing', () => {
    expect(undoIntent(key({ metaKey: false }), false)).toBeNull()
  })

  it('ignores the other modified keys the workspace binds', () => {
    expect(undoIntent(key({ key: 'k' }), false)).toBeNull()
    expect(undoIntent(key({ key: 'n' }), false)).toBeNull()
  })
})

describe('the inverse of each command', () => {
  it('inverts a create by deleting, and redoes it without minting a new id', () => {
    // node_create would hand back a different ULID on redo, orphaning every
    // link made to the original and every later entry that names it.
    const made = node({ id: 'kell', name: 'Kell' })
    const e = birthEntry(made, 'create')
    expect(e.undo).toEqual([{ type: 'delete', id: 'kell' }])
    expect(e.redo).toEqual([{ type: 'upsert', node: made }])
    expect(e.coalesce).toBe(false)
  })

  it('inverts a delete by upserting the original node, then putting its children back', () => {
    // node_delete promotes children to the deleted node's parent. Restoring the
    // node alone leaves the subtree flattened, which the user never asked for.
    const gone = node({ id: 'kell', name: 'Kell' })
    const e = deletionEntry(gone, ['a', 'b'])
    expect(e.undo).toEqual([
      { type: 'upsert', node: gone },
      { type: 'move', id: 'a', parentId: 'kell' },
      { type: 'move', id: 'b', parentId: 'kell' },
    ])
    expect(e.redo).toEqual([{ type: 'delete', id: 'kell' }])
    // Inbound links cannot be restored without reading the whole world, so the
    // entry says so rather than letting the user assume otherwise.
    expect(e.caveat).toBeTruthy()
  })

  it('inverts a move back to the parent it came from', () => {
    const e = moveEntry({ id: 'kell', name: 'Kell', parentId: 'old' }, 'new')
    expect(e?.undo).toEqual([{ type: 'move', id: 'kell', parentId: 'old' }])
    expect(e?.redo).toEqual([{ type: 'move', id: 'kell', parentId: 'new' }])
  })

  it('records nothing for a move onto the parent the node already has', () => {
    // The backend returns early on that, so an entry would undo to where the
    // node is and read as a ⌘Z that did nothing.
    expect(moveEntry({ id: 'kell', name: 'Kell', parentId: 'old' }, 'old')).toBeNull()
  })

  it('inverts an edit with the state that preceded it, and marks it coalescable', () => {
    const before = node({ id: 'kell', notesRaw: 'was' })
    const after = { ...before, notesRaw: 'is' }
    const e = editEntry(before, after)
    expect(e?.undo).toEqual([{ type: 'upsert', node: before }])
    expect(e?.redo).toEqual([{ type: 'upsert', node: after }])
    expect(e?.label).toContain('notes edit')
    expect(e?.coalesce).toBe(true)
  })

  it('records nothing for a save that only moved the timestamp', () => {
    const before = node({ id: 'kell' })
    expect(editEntry(before, { ...before, updatedAt: '2030-01-01T00:00:00Z' })).toBeNull()
  })
})

describe('editLabel', () => {
  it('names what changed, so the toast can say it', () => {
    const before = node({ id: 'kell' })
    expect(editLabel(before, { ...before, name: 'Kell the Grey' })).toBe('rename')
    expect(editLabel(before, { ...before, notesRaw: 'x' })).toBe('notes edit')
    const link = { toId: 'a', role: 'related_to' as const, weight: 1, enabled: true }
    expect(editLabel(before, { ...before, links: [link] })).toBe('link change')
  })

  it('returns null for a save that changed nothing but the timestamp', () => {
    // Every save re-stamps updatedAt, so identical content still arrives as a
    // different object. Logging it would put an entry on the stack whose undo
    // restores the state it is already in — a ⌘Z that visibly does nothing.
    const before = node({ id: 'kell' })
    expect(editLabel(before, { ...before, updatedAt: '2030-01-01T00:00:00Z' })).toBeNull()
  })
})
