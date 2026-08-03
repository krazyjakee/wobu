import { isMac } from './platform'

/**
 * Chords, in one notation.
 *
 * A binding is stored as a normalised string — `Mod+Shift+Z`, `[`, `Mod+Enter`
 * — rather than as a `KeyboardEvent` predicate, because the registry has to be
 * able to compare two bindings for equality (that is what conflict detection
 * is) and to print one (that is what the reference is). A function cannot be
 * compared or printed; a string can be both, and can also survive a round trip
 * through local storage.
 *
 * `Mod` is the platform's command modifier. It matches Command *or* Control
 * rather than resolving to one of them, which is deliberate: Wobu runs the same
 * build on three platforms, users arrive with muscle memory from all of them,
 * and there is no key that means one thing under Command and another under
 * Control anywhere in this app. Only the *display* is platform-specific.
 */

/** Key names that are only ever a modifier, so never a chord on their own. */
const MODIFIER_KEYS = new Set([
  'Shift',
  'Control',
  'Alt',
  'Meta',
  'AltGraph',
  'CapsLock',
  'OS',
  'Dead',
  'Unidentified',
])

/** Named keys, lowercased, to their canonical spelling. */
const CANONICAL = new Map<string, string>(
  [
    'Enter',
    'Escape',
    'Tab',
    'Backspace',
    'Delete',
    'Insert',
    'Home',
    'End',
    'PageUp',
    'PageDown',
    'ArrowUp',
    'ArrowDown',
    'ArrowLeft',
    'ArrowRight',
    'F1',
    'F2',
    'F3',
    'F4',
    'F5',
    'F6',
    'F7',
    'F8',
    'F9',
    'F10',
    'F11',
    'F12',
    'Space',
  ].map((name) => [name.toLowerCase(), name]),
)

const MAC_MODIFIER: Record<string, string> = { Alt: '⌥', Shift: '⇧', Mod: '⌘' }
const PC_MODIFIER: Record<string, string> = { Mod: 'Ctrl', Alt: 'Alt', Shift: 'Shift' }

/** Modifiers in the order each platform writes them. */
const MAC_ORDER = ['Alt', 'Shift', 'Mod']
const PC_ORDER = ['Mod', 'Alt', 'Shift']

const MAC_KEY: Record<string, string> = {
  Enter: '↵',
  Escape: '⎋',
  Tab: '⇥',
  Backspace: '⌫',
  Delete: '⌦',
  Space: '␣',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
}

const PC_KEY: Record<string, string> = {
  Escape: 'Esc',
  Delete: 'Del',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
}

function normalizeKey(raw: string): string | null {
  if (raw === ' ') return 'Space'
  if (raw.length === 1) return raw.toUpperCase()
  return CANONICAL.get(raw.toLowerCase()) ?? raw
}

/**
 * Whether Shift is part of the chord or already inside the character.
 *
 * `Shift+2` produces `"` on one layout and `@` on another, so recording it as
 * "Shift plus the 2 key" would make a binding that only works where it was
 * recorded. The produced character is what the user pressed and what they will
 * press again, so for printable non-letters the modifier is folded away. A
 * letter keeps its Shift, because `Z` is `Z` either way and `⇧⌘Z` has to stay
 * distinguishable from `⌘Z`.
 */
function shiftIsSeparate(key: string): boolean {
  return key.length > 1 || /^[A-Z]$/.test(key)
}

export interface ChordSource {
  key: string
  metaKey: boolean
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
}

/** The chord this keystroke is, or `null` if it is only a modifier being held. */
export function chordFromEvent(event: ChordSource): string | null {
  const key = normalizeKey(event.key)
  if (!key || MODIFIER_KEYS.has(key) || MODIFIER_KEYS.has(event.key)) return null
  const parts: string[] = []
  if (event.metaKey || event.ctrlKey) parts.push('Mod')
  if (event.altKey) parts.push('Alt')
  if (event.shiftKey && shiftIsSeparate(key)) parts.push('Shift')
  parts.push(key)
  return parts.join('+')
}

/** The chord as this platform writes it, one `<kbd>`-sized token per entry. */
export function chordParts(chord: string): string[] {
  const mac = isMac()
  const tokens = chord.split('+')
  const key = tokens[tokens.length - 1] ?? ''
  const held = new Set(tokens.slice(0, -1))
  const order = mac ? MAC_ORDER : PC_ORDER
  const modifiers = order
    .filter((m) => held.has(m))
    .map((m) => (mac ? MAC_MODIFIER : PC_MODIFIER)[m]!)
  return [...modifiers, (mac ? MAC_KEY : PC_KEY)[key] ?? key]
}

/**
 * The chord as one string.
 *
 * Joined without a separator on macOS and with `+` elsewhere, because that is
 * how each platform's own menus print them — `⇧⌘Z` and `Ctrl+Shift+Z` are the
 * same chord and neither reads as correct in the other's notation.
 */
export function formatChord(chord: string): string {
  return chordParts(chord).join(isMac() ? '' : '+')
}

/**
 * The chord in the notation `aria-keyshortcuts` specifies, which is a third one
 * — assistive technology announces the token names itself, so it wants
 * `Meta+K`, not `⌘K` and not `Ctrl+K`.
 */
export function ariaChord(chord: string | null): string | undefined {
  if (!chord) return undefined
  return chord.replace(/\bMod\b/, isMac() ? 'Meta' : 'Control')
}

/**
 * Whether this event came out of somewhere the user is composing text.
 *
 * The gate every unmodified — and most modified — bindings sit behind. A
 * shortcut that fires mid-sentence is worse than no shortcut, because the user
 * cannot tell what they did.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el || typeof el.tagName !== 'string') return false
  const tag = el.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable === true
}
