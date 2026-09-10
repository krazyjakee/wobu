import { projectClose, projectOpen } from './api/project'
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }))
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  COALESCE_MS,
  MAX_ENTRIES,
  birthEntry,
  deletionEntry,
  editEntry,
  editLabel,
  moveEntry,
  sceneBirthEntry,
  sceneDeletionEntry,
  sceneEditEntry,
  sceneEditLabel,
  useUndoStack,
  type NewEntry,
  type UndoEntry,
  type WorldCommand,
} from './undo'
import type { Beat, Scene, SceneFile } from './api'
import { node } from '../test/fixtures'

/* ── narrative fixtures ───────────────────────────────────────────────────── */

function beat(over: Partial<Beat> & { id: string }): Beat {
  return { title: over.id, ...over }
}

function scene(over: Partial<Scene> & { id: string }): Scene {
  return { name: 'Council hearing', ...over }
}

function file(over: Partial<SceneFile> & { scene: Scene }): SceneFile {
  return {
    slug: 'council-hearing',
    rel: 'narrative/scenes/council-hearing.yaml',
    stamp: { mtime_ms: 1, size: 2, hash: 'h' },
    ...over,
  }
}

function typingScene(title: string): SceneFile {
  return file({ scene: scene({ id: 's1', beats: [beat({ id: 'b1', title })] }) })
}

/** An entry with only the fields a given test cares about spelled out. */
function entry(over: Partial<UndoEntry> & { subjectId: string }): NewEntry {
  return {
    label: `edit ${over.subjectId}`,
    undo: [{ type: 'delete', id: over.subjectId }],
    redo: [{ type: 'delete', id: over.subjectId }],
    coalesce: false,
    ...over,
  }
}

/** An edit of `id`, carrying the two states it moved between. */
function edit(id: string, before: string, after: string, at?: number) {
  return entry({
    subjectId: id,
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
    push(entry({ subjectId: 'kell', coalesce: false, at: 1000 }))
    push(entry({ subjectId: 'kell', coalesce: false, at: 1001 }))
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
        subjectId: 'kell',
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
    push(entry({ subjectId: 'first' }))
    push(entry({ subjectId: 'second' }))
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
    useUndoStack.getState().push(entry({ subjectId: 'kell' }))
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
    push(entry({ subjectId: 'a' }))
    push(entry({ subjectId: 'b' }))
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
    for (let i = 0; i < MAX_ENTRIES + 10; i++) push(entry({ subjectId: `n${i}` }))
    const { past } = useUndoStack.getState()
    expect(past).toHaveLength(MAX_ENTRIES)
    expect(past[0]!.subjectId).toBe('n10')
  })
})

describe('project scope', () => {
  it('throws the stack away when a different project is opened', () => {
    // Every command names a node by id in one world. Replaying it against
    // another project would either fail or, worse, hit an unrelated node.
    useUndoStack.getState().push(entry({ subjectId: 'kell' }))
    useUndoStack.getState().setProject('other')
    expect(useUndoStack.getState().past).toEqual([])
    expect(useUndoStack.getState().future).toEqual([])
  })

  it('leaves the stack alone when told the same project again', () => {
    // The scope is synced from a query result that re-renders freely; clearing
    // on every report would empty the stack at random moments.
    useUndoStack.getState().push(entry({ subjectId: 'kell' }))
    useUndoStack.getState().setProject('proj')
    expect(useUndoStack.getState().past).toHaveLength(1)
  })

  it('clears when the project closes and ignores pushes after it', () => {
    useUndoStack.getState().push(entry({ subjectId: 'kell' }))
    useUndoStack.getState().setProject(null)
    useUndoStack.getState().push(entry({ subjectId: 'kell' }))
    expect(useUndoStack.getState().past).toEqual([])
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

  it('notices a reference image attached, reweighted or muted', () => {
    // `assetLinks` was added to WobuNode after this comparison was written and
    // went unlisted, so an upsert carrying only a reference change labelled as
    // nothing and `editEntry` dropped it: the one edit ⌘Z could not take back.
    const before = node({ id: 'kell' })
    const ref = { assetId: 'img', role: 'full_ref' as const, weight: 1, enabled: true }
    expect(editLabel(before, { ...before, assetLinks: [ref] })).toBe('reference change')
    const attached = { ...before, assetLinks: [ref] }
    expect(editLabel(attached, { ...attached, assetLinks: [{ ...ref, weight: 0.4 }] })).toBe(
      'reference change',
    )
    expect(editLabel(attached, { ...attached, assetLinks: [{ ...ref, enabled: false }] })).toBe(
      'reference change',
    )
  })

  it('returns null for a save that changed nothing but the timestamp', () => {
    // Every save re-stamps updatedAt, so identical content still arrives as a
    // different object. Logging it would put an entry on the stack whose undo
    // restores the state it is already in — a ⌘Z that visibly does nothing.
    const before = node({ id: 'kell' })
    expect(editLabel(before, { ...before, updatedAt: '2030-01-01T00:00:00Z' })).toBeNull()
  })
})

describe('narrative source on the same stack as nodes', () => {
  it('inverts a scene create by deleting, and redoes it without minting a new id', () => {
    // narrative_scene_create mints a fresh scene id, so a redone create would
    // be a different scene and every entry recorded after it would name beats
    // that no longer exist.
    const made = file({ scene: scene({ id: 's1' }) })
    const e = sceneBirthEntry(made, 'create')
    expect(e.undo).toEqual([{ type: 'sceneDelete', id: 's1' }])
    expect(e.redo).toEqual([{ type: 'sceneSave', scene: made.scene, slug: made.slug }])
    expect(e.coalesce).toBe(false)
  })

  it('restores a deleted scene as itself, and says what it cannot bring back', () => {
    const gone = file({ scene: scene({ id: 's1', beats: [beat({ id: 'b1' })] }) })
    const e = sceneDeletionEntry(gone)
    expect(e.undo).toEqual([{ type: 'sceneSave', scene: gone.scene, slug: gone.slug }])
    expect(e.redo).toEqual([{ type: 'sceneDelete', id: 's1' }])
    // The layout sidecar goes with the scene and no inverse can put it back.
    expect(e.caveat).toBeTruthy()
  })

  it('inverts every edit the same way, so a canvas edit and a form edit match', () => {
    // #186: a structural edit made on the canvas and the same edit made in a
    // form have to be indistinguishable in history. They are, because both
    // arrive here as one document replacing another.
    const before = file({ scene: scene({ id: 's1', beats: [beat({ id: 'b1' })] }) })
    const wired = {
      ...before.scene,
      beats: [beat({ id: 'b1', choices: [{ id: 'c1', label: 'Go', to: { end: {} } }] })],
    }
    const fromCanvas = sceneEditEntry(before, { ...before, scene: wired })
    const fromForm = sceneEditEntry(before, { ...before, scene: structuredClone(wired) })

    expect(fromCanvas).toEqual(fromForm)
    expect(fromCanvas?.undo).toEqual([
      { type: 'sceneSave', scene: before.scene, slug: before.slug, expected: wired },
    ])
    expect(fromCanvas?.label).toContain('branch change')
  })

  it('records nothing for a save that changed nothing', () => {
    // An entry whose inverse restores the state it is already in reads as a
    // broken ⌘Z, exactly as it does for nodes.
    const before = file({ scene: scene({ id: 's1' }) })
    expect(
      sceneEditEntry(before, { ...before, stamp: { mtime_ms: 9, size: 9, hash: 'x' } }),
    ).toBeNull()
  })

  it('coalesces a run of typing and never a run of structural edits', () => {
    const s = scene({ id: 's1', beats: [beat({ id: 'b1', title: 'Presen' })] })
    const typed = { ...s, beats: [beat({ id: 'b1', title: 'Present' })] }
    expect(sceneEditLabel(s, typed)).toEqual({ verb: 'text edit', coalesce: true })

    const deleted = { ...s, beats: [] }
    expect(sceneEditLabel(s, deleted)).toEqual({ verb: 'delete beat', coalesce: false })
  })

  it('names the most structural thing that changed, not the most textual', () => {
    // A deletion that also touched some wording is a deletion. Labelling it a
    // text edit would badly understate what ⌘Z is about to reverse.
    const s = scene({ id: 's1', beats: [beat({ id: 'b1' }), beat({ id: 'b2' })] })
    const both = { ...s, beats: [beat({ id: 'b1', title: 'renamed' })] }
    expect(sceneEditLabel(s, both)?.verb).toBe('delete beat')
  })

  it('tells reordering apart from adding and deleting', () => {
    const s = scene({ id: 's1', beats: [beat({ id: 'b1' }), beat({ id: 'b2' })] })
    const moved = { ...s, beats: [beat({ id: 'b2' }), beat({ id: 'b1' })] }
    expect(sceneEditLabel(s, moved)?.verb).toBe('reorder beats')
    expect(sceneEditLabel(s, { ...s, beats: [...(s.beats ?? []), beat({ id: 'b3' })] })?.verb).toBe(
      'add beat',
    )
  })

  it('notices a destination connected or disconnected', () => {
    // The canvas edit #186 is built around. Missing it would not produce a
    // wrong label, it would produce no entry at all.
    const linked = scene({
      id: 's1',
      beats: [beat({ id: 'b1', choices: [{ id: 'c1', label: 'Go', to: { beat: 'b2' } }] })],
    })
    const cut = scene({
      id: 's1',
      beats: [beat({ id: 'b1', choices: [{ id: 'c1', label: 'Go', to: { end: {} } }] })],
    })
    expect(sceneEditLabel(linked, cut)?.verb).toBe('branch change')
  })

  it('names the scene-level edits that are not about a beat', () => {
    const s = scene({ id: 's1' })
    expect(sceneEditLabel(s, { ...s, participants: [{ entity: 'kael' }] })?.verb).toBe(
      'participant change',
    )
    expect(sceneEditLabel(s, { ...s, entry: 'never' })?.verb).toBe('condition change')
    expect(sceneEditLabel(s, { ...s, summary: 'the captain answers' })).toEqual({
      verb: 'summary edit',
      coalesce: true,
    })
    // A field this comparison has never heard of still produces an entry. A
    // label that is merely vague is recoverable; no entry at all is the one
    // edit ⌘Z cannot take back.
    expect(sceneEditLabel(s, { ...s, tombstones: [] })?.verb).toBe('edit')
  })

  it('separates a scene rename from anything inside it', () => {
    const s = scene({ id: 's1' })
    expect(sceneEditLabel(s, { ...s, name: 'The hearing' })).toEqual({
      verb: 'rename',
      coalesce: false,
    })
  })

  it('carries no command that could put a coordinate on the world stack', () => {
    // #185, kept as a property of the primitives rather than as a convention.
    // Every command names a node or a scene; none of them has an x or a y, so
    // no caller — including one written after this test — can make ⌘Z undo a
    // drag and then, on the very next press, undo a paragraph.
    const commands: WorldCommand[] = [
      { type: 'upsert', node: node({ id: 'kell' }) },
      { type: 'delete', id: 'kell' },
      { type: 'move', id: 'kell', parentId: null },
      { type: 'sceneSave', scene: scene({ id: 's1' }), slug: 'council-hearing' },
      { type: 'sceneDelete', id: 's1' },
    ]
    for (const command of commands) {
      expect(JSON.stringify(command)).not.toMatch(/"(x|y|collapsed|graph|annotations)"/)
    }
  })

  it('coalesces a scene typing run and a node typing run separately', () => {
    // The subject key is what keeps the two apart on one stack: typing into a
    // scene must not absorb the node edit that happened a moment earlier.
    const { push } = useUndoStack.getState()
    push(entry({ subjectId: 'kell', coalesce: true, at: 1000 }))
    push(entry({ subjectId: 's1', coalesce: true, at: 1100 }))
    expect(useUndoStack.getState().past).toHaveLength(2)
  })

  it('undoes and redoes coalesced scene typing with guards for the whole run', async () => {
    const original = typingScene('C')
    const middle = typingScene('Co')
    const final = typingScene('Council')
    const { push, undo, redo } = useUndoStack.getState()
    push({ ...sceneEditEntry(original, middle)!, at: 1000 })
    push({ ...sceneEditEntry(middle, final)!, at: 1100 })
    expect(useUndoStack.getState().past).toHaveLength(1)
    let saved = final.scene
    const guardedSave = async (cmd: WorldCommand) => {
      if (cmd.type !== 'sceneSave') throw new Error('Expected a scene restore')
      if (JSON.stringify(saved) !== JSON.stringify(cmd.expected)) throw new Error('Conflict')
      saved = cmd.scene
    }
    await undo(guardedSave)
    expect(saved).toEqual(original.scene)
    await redo(guardedSave)
    expect(saved).toEqual(final.scene)
    saved = { ...saved, summary: 'A collaborator added this' }
    await expect(undo(guardedSave)).rejects.toThrow('Conflict')
    expect(saved.summary).toBe('A collaborator added this')
    expect(useUndoStack.getState().past).toHaveLength(1)
  })

  it('does not coalesce scene typing across a collaborator change', () => {
    const original = typingScene('C')
    const first = typingScene('Co')
    const peer = { ...first, scene: { ...first.scene, summary: 'Peer notes' } }
    const last = { ...peer, scene: { ...peer.scene, beats: typingScene('Council').scene.beats } }
    const { push } = useUndoStack.getState()
    push({ ...sceneEditEntry(original, first)!, at: 1000 })
    push({ ...sceneEditEntry(peer, last)!, at: 1100 })
    expect(useUndoStack.getState().past).toHaveLength(2)
  })
})

it.each(['undo', 'redo'] as const)(
  'stops a multi-command %s after same-project reopen without clearing a newer busy operation',
  async (direction) => {
    ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
    const history: UndoEntry = {
      ...entry({
        subjectId: 'first',
        undo: [
          { type: 'delete', id: 'first' },
          { type: 'delete', id: 'second' },
        ],
        redo: [
          { type: 'delete', id: 'first' },
          { type: 'delete', id: 'second' },
        ],
      }),
      at: 1,
    }
    useUndoStack.setState({
      projectId: 'same-id',
      past: direction === 'undo' ? [history] : [],
      future: direction === 'redo' ? [history] : [],
      busy: false,
    })
    let finishOld!: () => void
    const oldRunner = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finishOld = resolve
        }),
    )
    const old = useUndoStack
      .getState()
      [direction](oldRunner)
      .catch((error: unknown) => error)
    await projectClose()
    useUndoStack.getState().setProject(null)
    await projectOpen('/same')
    useUndoStack.getState().setProject('same-id')
    useUndoStack.setState({
      past: direction === 'undo' ? [history] : [],
      future: direction === 'redo' ? [history] : [],
    })
    let finishNew!: () => void
    const newRunner = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            finishNew = resolve
          }),
      )
      .mockResolvedValue(undefined)
    const newer = useUndoStack.getState()[direction](newRunner)
    finishOld()
    expect(await old).toMatchObject({ message: expect.stringContaining('session changed') })
    expect(oldRunner).toHaveBeenCalledTimes(1)
    expect(useUndoStack.getState().busy).toBe(true)
    finishNew()
    await newer
    expect(useUndoStack.getState().busy).toBe(false)
    expect(newRunner).toHaveBeenCalledTimes(2)
  },
)
