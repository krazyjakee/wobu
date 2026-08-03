import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * Every keyboard command in Wobu, declared once.
 *
 * Before this existed the map was spread across `useKeyboard`, the Editor, the
 * Inspector and half a dozen local handlers, and there was no place that knew
 * the whole of it — so nothing could show it, nothing could check it for
 * collisions, and a second command bound to `⌘E` would simply have run
 * alongside the first with no way for anyone to find out. The registry is that
 * place. Surfaces still *do* the work (only the Editor knows whether Enhance is
 * eligible right now), but they no longer decide which keystroke means what.
 *
 * Overrides live in local storage, next to `store/settings.ts` and
 * `lib/favourites.ts` and for the same reason: a binding is a fact about this
 * person's hands, not about the world. Writing it into the project folder would
 * push one author's remapped keys onto everybody who opens the world.
 */

export type CommandId =
  | 'palette.toggle'
  | 'shortcuts.show'
  | 'nav.filter'
  | 'mode.library'
  | 'mode.forge'
  | 'mode.assets'
  | 'mode.settings'
  | 'tab.notes'
  | 'tab.refs'
  | 'tab.concepts'
  | 'tab.three'
  | 'tab.relations'
  | 'panel.navigator'
  | 'panel.inspector'
  | 'nav.toggleAll'
  | 'node.new'
  | 'edit.undo'
  | 'edit.redo'
  | 'enhance'
  | 'generate'

export type CommandGroup = 'Getting around' | 'Panels' | 'The editor' | 'Writing'

/** Section order in the reference and in Settings. */
export const COMMAND_GROUPS: CommandGroup[] = ['Getting around', 'Panels', 'The editor', 'Writing']

export interface CommandDef {
  id: CommandId
  label: string
  group: CommandGroup
  /** What this build ships. Never mutated; overrides live in the store. */
  chord: string
  /**
   * Extra chords that also run it, and which the user cannot edit. Only for
   * the other platform's muscle memory — `⌃Y` is redo on a Windows keyboard.
   */
  aliases?: string[]
  /** Runs even when the caret is in a text field. The exception, not the rule. */
  whileTyping?: boolean
  /** Runs even when a dialog is on top of the workspace. */
  whileModal?: boolean
  /** Handled by the surface that owns the action, which knows its eligibility. */
  surface?: boolean
  /** One line of why, for the reference. */
  note?: string
}

/**
 * Declaration order is precedence order.
 *
 * If a user binds two commands to the same chord the earlier one wins, and
 * both rows say so — an assignment that quietly did nothing is the failure this
 * registry exists to prevent, so the shadowed command is *named* rather than
 * left to be discovered by pressing it.
 */
export const COMMANDS: CommandDef[] = [
  {
    id: 'palette.toggle',
    label: 'Command palette',
    group: 'Getting around',
    chord: 'Mod+K',
    whileTyping: true,
    whileModal: true,
    note: 'Deliberately reachable mid-sentence: it is how you leave where you are.',
  },
  {
    id: 'shortcuts.show',
    label: 'Keyboard shortcuts',
    group: 'Getting around',
    chord: 'Mod+/',
    whileTyping: true,
    whileModal: true,
    note: 'This list.',
  },
  {
    id: 'nav.filter',
    label: 'Filter the navigator',
    group: 'Getting around',
    chord: 'Mod+F',
    whileTyping: true,
    note: 'Names and summaries only — the palette searches notes as well.',
  },
  { id: 'mode.library', label: 'Library', group: 'Getting around', chord: 'Mod+Shift+L' },
  {
    id: 'mode.forge',
    label: 'Forge, and back',
    group: 'Getting around',
    chord: 'Mod+\\',
    note: 'A toggle: it returns you to the Library from Forge.',
  },
  { id: 'mode.assets', label: 'Assets', group: 'Getting around', chord: 'Mod+Shift+A' },
  { id: 'mode.settings', label: 'Settings', group: 'Getting around', chord: 'Mod+,' },

  { id: 'panel.navigator', label: 'Toggle the navigator', group: 'Panels', chord: '[' },
  { id: 'panel.inspector', label: 'Toggle the inspector', group: 'Panels', chord: ']' },
  {
    id: 'nav.toggleAll',
    label: 'Collapse or expand everything',
    group: 'Panels',
    chord: 'Mod+Shift+C',
  },

  { id: 'tab.notes', label: 'Notes', group: 'The editor', chord: 'Mod+1' },
  { id: 'tab.refs', label: 'References', group: 'The editor', chord: 'Mod+2' },
  { id: 'tab.concepts', label: 'Concepts', group: 'The editor', chord: 'Mod+3' },
  { id: 'tab.three', label: '3D', group: 'The editor', chord: 'Mod+4' },
  { id: 'tab.relations', label: 'Relations', group: 'The editor', chord: 'Mod+5' },

  { id: 'node.new', label: 'New entity', group: 'Writing', chord: 'Mod+N' },
  /*
   * Undo and redo are `whileTyping: false`, and the cost of that is real enough
   * to write down. While the caret is in a textarea, ⌘Z belongs to the field —
   * stealing it would rewind a whole save every time somebody tried to take
   * back a word. The price is that once the field's own undo stack is empty the
   * key does nothing at all, and workspace undo never gets its turn without the
   * user clicking away first. Taking ⌘Z once the native stack looks empty
   * cannot be implemented, because the DOM does not expose whether it is.
   * Between a key that occasionally does nothing and a key that occasionally
   * discards a paragraph somebody was still editing, this is the safe
   * direction.
   */
  {
    id: 'edit.undo',
    label: 'Undo',
    group: 'Writing',
    chord: 'Mod+Z',
    note: 'Inside a text box this is the box’s own undo, not the world’s.',
  },
  { id: 'edit.redo', label: 'Redo', group: 'Writing', chord: 'Mod+Shift+Z', aliases: ['Mod+Y'] },
  {
    id: 'enhance',
    label: 'Enhance the open entity',
    group: 'Writing',
    chord: 'Mod+E',
    surface: true,
  },
  {
    id: 'generate',
    label: 'Generate',
    group: 'Writing',
    chord: 'Mod+Enter',
    surface: true,
    note: 'From the inspector, on whatever the prompt currently compiles to.',
  },
]

const BY_ID = new Map<CommandId, CommandDef>(COMMANDS.map((c) => [c.id, c]))

export function commandDef(id: CommandId): CommandDef {
  const def = BY_ID.get(id)
  if (!def) throw new Error(`unknown command ${id}`)
  return def
}

/**
 * Chords the operating system, the webview or the text field under the caret
 * has a prior claim on. Not refused — a user who wants `⌘S` for something can
 * have it — but said out loud at the moment of binding, because the failure
 * otherwise happens later and looks like Wobu ignoring the key.
 */
const RESERVED: Record<string, string> = {
  'Mod+C': 'copy',
  'Mod+V': 'paste',
  'Mod+X': 'cut',
  'Mod+A': 'select all',
  'Mod+Q': 'quit',
  'Mod+W': 'close the window',
  'Mod+R': 'reload',
  'Mod+Shift+I': 'the developer tools',
}

export function reservedFor(chord: string): string | null {
  return RESERVED[chord] ?? null
}

/** `undefined` means "the default"; `null` means the user unbound it. */
type Overrides = Partial<Record<CommandId, string | null>>

interface KeybindingsState {
  overrides: Overrides
  setBinding: (id: CommandId, chord: string | null) => void
  resetBinding: (id: CommandId) => void
  resetAll: () => void
}

export const useKeybindings = create<KeybindingsState>()(
  persist(
    (set) => ({
      overrides: {},
      setBinding: (id, chord) => set((s) => ({ overrides: { ...s.overrides, [id]: chord } })),
      resetBinding: (id) =>
        set((s) => {
          const next = { ...s.overrides }
          delete next[id]
          return { overrides: next }
        }),
      resetAll: () => set({ overrides: {} }),
    }),
    {
      name: 'wobu.keybindings',
      // A stored map written by a build with different command ids — or edited
      // by hand — must not be able to make the app unresponsive to the
      // keyboard. Unknown ids and non-string chords are dropped rather than
      // carried into the dispatcher, where a bad entry would shadow a default.
      merge: (stored, current) => {
        const raw = (stored as { overrides?: unknown } | null)?.overrides
        const overrides: Overrides = {}
        if (raw && typeof raw === 'object') {
          for (const [id, chord] of Object.entries(raw as Record<string, unknown>)) {
            if (!BY_ID.has(id as CommandId)) continue
            if (chord === null) overrides[id as CommandId] = null
            else if (typeof chord === 'string' && chord.length > 0) {
              overrides[id as CommandId] = chord
            }
          }
        }
        return { ...current, overrides }
      },
    },
  ),
)

/** The chord in force for a command, or `null` if it has none. */
export function bindingOf(overrides: Overrides, id: CommandId): string | null {
  const override = overrides[id]
  return override === undefined ? commandDef(id).chord : override
}

/**
 * Which command a keystroke runs — exactly one, or none.
 *
 * Every listener in the app resolves through here rather than testing its own
 * chord, so a chord bound to two commands runs the *first* one everywhere
 * rather than running one command in the global handler and a different one in
 * a surface handler depending on which listener was registered last.
 */
export function resolveCommand(chord: string, overrides: Overrides): CommandDef | null {
  for (const def of COMMANDS) {
    if (bindingOf(overrides, def.id) === chord) return def
    if (def.aliases?.includes(chord) && overrides[def.id] === undefined) return def
  }
  return null
}

export interface Conflict {
  chord: string
  /** The command that actually runs. */
  winner: CommandDef
  /** The ones that do not, in registry order. */
  shadowed: CommandDef[]
}

/** Every chord that more than one command claims. */
export function findConflicts(overrides: Overrides): Conflict[] {
  const claims = new Map<string, CommandDef[]>()
  for (const def of COMMANDS) {
    const chord = bindingOf(overrides, def.id)
    if (!chord) continue
    const existing = claims.get(chord)
    if (existing) existing.push(def)
    else claims.set(chord, [def])
  }
  const out: Conflict[] = []
  for (const [chord, defs] of claims) {
    if (defs.length < 2) continue
    out.push({ chord, winner: defs[0]!, shadowed: defs.slice(1) })
  }
  return out
}
