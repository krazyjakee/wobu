import { beforeEach, describe, expect, it } from 'vitest'
import {
  COMMANDS,
  COMMAND_GROUPS,
  bindingOf,
  commandDef,
  findConflicts,
  reservedFor,
  resolveCommand,
  useKeybindings,
} from './keybindings'

beforeEach(() => {
  useKeybindings.setState({ overrides: {} })
  localStorage.clear()
})

describe('what this build ships with', () => {
  it('binds no chord to two commands', () => {
    // The whole point of the registry is that this is checkable at all. A
    // default map that shadowed one of its own commands would ship a key that
    // silently does nothing.
    expect(findConflicts({})).toEqual([])
  })

  it('puts every command in a group the reference will render', () => {
    for (const def of COMMANDS) expect(COMMAND_GROUPS).toContain(def.group)
  })

  it('gives every command a chord and a distinct id', () => {
    expect(new Set(COMMANDS.map((c) => c.id)).size).toBe(COMMANDS.length)
    for (const def of COMMANDS) expect(def.chord.length).toBeGreaterThan(0)
  })
})

describe('resolving a keystroke to exactly one command', () => {
  it('answers with the command that owns the chord', () => {
    expect(resolveCommand('Mod+K', {})?.id).toBe('palette.toggle')
    expect(resolveCommand('[', {})?.id).toBe('panel.navigator')
    expect(resolveCommand('Mod+Q', {})).toBeNull()
  })

  it('honours the other platform’s redo, until redo is rebound', () => {
    expect(resolveCommand('Mod+Y', {})?.id).toBe('edit.redo')
    expect(resolveCommand('Mod+Y', { 'edit.redo': 'Mod+R' })).toBeNull()
  })

  it('follows an override rather than the default', () => {
    const overrides = { 'palette.toggle': 'Mod+P' }
    expect(resolveCommand('Mod+P', overrides)?.id).toBe('palette.toggle')
    expect(resolveCommand('Mod+K', overrides)).toBeNull()
  })

  it('gives a contested chord to the earlier command, everywhere', () => {
    // Registry order decides, not listener registration order — otherwise the
    // global handler and a surface handler would disagree about the same key.
    const overrides = { enhance: 'Mod+K' }
    expect(resolveCommand('Mod+K', overrides)?.id).toBe('palette.toggle')
  })

  it('finds nothing for a command the user has unbound', () => {
    expect(bindingOf({ 'panel.navigator': null }, 'panel.navigator')).toBeNull()
    expect(resolveCommand('[', { 'panel.navigator': null })).toBeNull()
  })
})

describe('reporting conflicts rather than absorbing them', () => {
  it('names the winner and everything it shadows', () => {
    const conflicts = findConflicts({ 'node.new': 'Mod+K' })
    expect(conflicts).toHaveLength(1)
    expect(conflicts[0]!.chord).toBe('Mod+K')
    expect(conflicts[0]!.winner.id).toBe('palette.toggle')
    expect(conflicts[0]!.shadowed.map((c) => c.id)).toEqual(['node.new'])
  })

  it('does not count an unbound command as clashing with another', () => {
    expect(findConflicts({ 'node.new': null, 'edit.undo': null })).toEqual([])
  })

  it('says when a chord belongs to the system as well', () => {
    expect(reservedFor('Mod+C')).toBe('copy')
    expect(reservedFor('Mod+K')).toBeNull()
  })
})

describe('the stored overrides', () => {
  it('changes, resets one, and resets all', () => {
    const store = useKeybindings.getState()
    store.setBinding('palette.toggle', 'Mod+P')
    store.setBinding('node.new', null)
    expect(bindingOf(useKeybindings.getState().overrides, 'palette.toggle')).toBe('Mod+P')
    expect(bindingOf(useKeybindings.getState().overrides, 'node.new')).toBeNull()

    useKeybindings.getState().resetBinding('palette.toggle')
    expect(bindingOf(useKeybindings.getState().overrides, 'palette.toggle')).toBe(
      commandDef('palette.toggle').chord,
    )

    useKeybindings.getState().resetAll()
    expect(useKeybindings.getState().overrides).toEqual({})
  })

  it('drops anything stored that is not a binding this build has', () => {
    // A map left behind by another build, or edited by hand, must not be able
    // to make the application deaf to the keyboard.
    localStorage.setItem(
      'wobu.keybindings',
      JSON.stringify({
        state: { overrides: { 'palette.toggle': 'Mod+P', 'gone.command': 'Mod+G', node: 7 } },
        version: 0,
      }),
    )
    useKeybindings.persist.rehydrate()

    expect(useKeybindings.getState().overrides).toEqual({ 'palette.toggle': 'Mod+P' })
  })

  it('survives a rehydrate with nothing stored at all', () => {
    localStorage.removeItem('wobu.keybindings')
    useKeybindings.persist.rehydrate()
    expect(useKeybindings.getState().overrides).toEqual({})
    expect(resolveCommand('Mod+K', useKeybindings.getState().overrides)?.id).toBe('palette.toggle')
  })
})
