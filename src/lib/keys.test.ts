import { afterEach, describe, expect, it, vi } from 'vitest'
import { ariaChord, chordFromEvent, chordParts, formatChord, isTypingTarget } from './keys'

/** jsdom reports a Linux platform; a mac run has to be asked for explicitly. */
function onMac(mac: boolean) {
  vi.spyOn(navigator, 'platform', 'get').mockReturnValue(mac ? 'MacIntel' : 'Linux x86_64')
}

function press(key: string, mods: Partial<Record<'meta' | 'ctrl' | 'alt' | 'shift', true>> = {}) {
  return chordFromEvent({
    key,
    metaKey: mods.meta ?? false,
    ctrlKey: mods.ctrl ?? false,
    altKey: mods.alt ?? false,
    shiftKey: mods.shift ?? false,
  })
}

afterEach(() => vi.restoreAllMocks())

describe('reading a chord off a keystroke', () => {
  it('treats Command and Control as the same modifier', () => {
    // The same build runs on three platforms and users arrive with muscle
    // memory from all of them. No key in Wobu means one thing under Command
    // and another under Control, so accepting either is free.
    expect(press('k', { meta: true })).toBe('Mod+K')
    expect(press('k', { ctrl: true })).toBe('Mod+K')
  })

  it('keeps Shift on a letter and folds it into a punctuation key', () => {
    // ⇧⌘Z has to stay distinguishable from ⌘Z. But `Shift+2` is `"` on one
    // layout and `@` on another, so recording the modifier there would make a
    // binding that only works on the keyboard it was recorded on.
    expect(press('Z', { meta: true, shift: true })).toBe('Mod+Shift+Z')
    expect(press('z', { meta: true })).toBe('Mod+Z')
    expect(press('@', { shift: true })).toBe('@')
  })

  it('is nothing at all while only a modifier is held', () => {
    expect(press('Shift', { shift: true })).toBeNull()
    expect(press('Meta', { meta: true })).toBeNull()
    expect(press('Control', { ctrl: true })).toBeNull()
  })

  it('canonicalises named keys and the space bar', () => {
    expect(press('Enter', { meta: true })).toBe('Mod+Enter')
    expect(press(' ')).toBe('Space')
    expect(press('ArrowUp')).toBe('ArrowUp')
  })

  it('orders modifiers the same way whichever order they were pressed in', () => {
    // Two spellings of one chord would read as two different bindings, and
    // conflict detection is string equality.
    expect(press('e', { meta: true, alt: true, shift: true })).toBe('Mod+Alt+Shift+E')
    expect(press('E', { ctrl: true, shift: true, alt: true })).toBe('Mod+Alt+Shift+E')
  })
})

describe('printing a chord', () => {
  it('uses each platform’s own notation', () => {
    onMac(true)
    expect(formatChord('Mod+Shift+Z')).toBe('⇧⌘Z')
    expect(formatChord('Mod+Enter')).toBe('⌘↵')
    onMac(false)
    expect(formatChord('Mod+Shift+Z')).toBe('Ctrl+Shift+Z')
    expect(formatChord('Mod+Enter')).toBe('Ctrl+Enter')
  })

  it('splits into one token per key, for one <kbd> each', () => {
    onMac(false)
    expect(chordParts('Mod+K')).toEqual(['Ctrl', 'K'])
    expect(chordParts('[')).toEqual(['['])
  })

  it('names the modifier the way assistive technology expects', () => {
    onMac(false)
    expect(ariaChord('Mod+K')).toBe('Control+K')
    onMac(true)
    expect(ariaChord('Mod+K')).toBe('Meta+K')
    expect(ariaChord(null)).toBeUndefined()
  })
})

describe('knowing when somebody is typing', () => {
  it('recognises the controls a caret can be in', () => {
    const editable = document.createElement('div')
    editable.contentEditable = 'true'
    // jsdom does not implement isContentEditable from the attribute.
    Object.defineProperty(editable, 'isContentEditable', { value: true })

    expect(isTypingTarget(document.createElement('input'))).toBe(true)
    expect(isTypingTarget(document.createElement('textarea'))).toBe(true)
    expect(isTypingTarget(document.createElement('select'))).toBe(true)
    expect(isTypingTarget(editable)).toBe(true)
    expect(isTypingTarget(document.createElement('button'))).toBe(false)
    expect(isTypingTarget(window)).toBe(false)
    expect(isTypingTarget(null)).toBe(false)
  })
})
