import { useState } from 'react'
import type { KeyboardEvent as ReactKeyboardEvent } from 'react'
import { chordFromEvent, chordParts, formatChord } from '../lib/keys'
import {
  COMMANDS,
  COMMAND_GROUPS,
  bindingOf,
  commandDef,
  findConflicts,
  reservedFor,
  useKeybindings,
  type CommandDef,
  type CommandId,
} from '../store/keybindings'
import { useUI } from '../store/ui'
import { Icon } from './Icon'

/**
 * Rebinding, in Settings.
 *
 * The rule this pane is built around: a conflict is *reported*, never silently
 * absorbed. Giving `⌘E` to a second command is allowed — people have their
 * reasons, and refusing would only push them into a config file we do not have
 * — but the moment it happens both rows say which command wins and which one
 * has stopped working, and the pane grows a line at the top offering to undo
 * it. The failure this replaces is a key that quietly does nothing, which the
 * user experiences as the application being broken.
 *
 * Bindings are stored on this machine, like the interface scale beside them and
 * unlike anything in `project.json`.
 */
export function KeybindingsSection() {
  const overrides = useKeybindings((s) => s.overrides)
  const setBinding = useKeybindings((s) => s.setBinding)
  const resetBinding = useKeybindings((s) => s.resetBinding)
  const resetAll = useKeybindings((s) => s.resetAll)
  const setShortcutsOpen = useUI((s) => s.setShortcutsOpen)
  const [recording, setRecording] = useState<CommandId | null>(null)

  const conflicts = findConflicts(overrides)
  const shadowed = new Map<CommandId, CommandDef>()
  for (const conflict of conflicts) {
    for (const loser of conflict.shadowed) shadowed.set(loser.id, conflict.winner)
  }

  const changed = COMMANDS.some((def) => overrides[def.id] !== undefined)

  return (
    <section className="set-sec">
      <h3>Keyboard</h3>
      <p className="set-note">
        Every shortcut Wobu listens for, and what it currently runs. Changes are kept on this
        computer — like your keys and the interface scale, and unlike anything in the project
        folder, because a remapped key is a fact about your hands rather than about the world.
      </p>

      {conflicts.length > 0 && (
        <div className="keys-conflicts" role="alert">
          <p>
            <b>
              {conflicts.length === 1
                ? 'One chord runs two commands.'
                : `${conflicts.length} chords run more than one command.`}
            </b>{' '}
            The first of them wins and the rest do nothing at all, which is why they are named here
            rather than left to be discovered by pressing the key.
          </p>
          <ul>
            {conflicts.map((conflict) => (
              <li key={conflict.chord}>
                <span className="keys-chord">
                  {chordParts(conflict.chord).map((part, i) => (
                    <kbd key={i}>{part}</kbd>
                  ))}
                </span>
                runs <b>{conflict.winner.label}</b>; {listNames(conflict.shadowed)}{' '}
                {conflict.shadowed.length === 1 ? 'does' : 'do'} nothing.
                <button
                  className="btn-mini"
                  onClick={() => {
                    for (const loser of conflict.shadowed) resetBinding(loser.id)
                  }}
                >
                  Restore the default
                  {conflict.shadowed.length === 1 ? '' : 's'}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="set-acts">
        <button className="btn-mini" onClick={() => setShortcutsOpen(true)}>
          <Icon name="search" size="sm" />
          Show the printable list
        </button>
        {changed && (
          <button
            className="btn-mini"
            onClick={() => {
              setRecording(null)
              resetAll()
            }}
          >
            <Icon name="refresh" size="sm" />
            Reset every shortcut
          </button>
        )}
      </div>

      {COMMAND_GROUPS.map((group) => (
        <div className="keys-group" key={group}>
          <span className="set-label">{group}</span>
          {COMMANDS.filter((def) => def.group === group).map((def) => (
            <BindingRow
              key={def.id}
              def={def}
              chord={bindingOf(overrides, def.id)}
              customised={overrides[def.id] !== undefined}
              shadowedBy={shadowed.get(def.id) ?? null}
              recording={recording === def.id}
              onRecord={() => setRecording(def.id)}
              onCancel={() => setRecording(null)}
              onSet={(chord) => {
                setBinding(def.id, chord)
                setRecording(null)
              }}
              onReset={() => {
                resetBinding(def.id)
                setRecording(null)
              }}
            />
          ))}
        </div>
      ))}
    </section>
  )
}

function listNames(defs: CommandDef[]): string {
  const names = defs.map((def) => def.label)
  if (names.length <= 1) return names[0] ?? ''
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
}

function BindingRow({
  def,
  chord,
  customised,
  shadowedBy,
  recording,
  onRecord,
  onCancel,
  onSet,
  onReset,
}: {
  def: CommandDef
  chord: string | null
  customised: boolean
  shadowedBy: CommandDef | null
  recording: boolean
  onRecord: () => void
  onCancel: () => void
  onSet: (chord: string | null) => void
  onReset: () => void
}) {
  const reserved = chord ? reservedFor(chord) : null

  /**
   * The recording itself.
   *
   * `stopPropagation` matters more than it looks: without it the keystroke
   * being recorded would also reach the global dispatcher on `window` and *run*
   * the command it is being assigned to, so binding something to `⌘\` would
   * throw the user into Forge on the way past.
   */
  const capture = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (event.key === 'Tab') {
      onCancel()
      return
    }
    event.preventDefault()
    event.stopPropagation()
    if (event.key === 'Escape') {
      onCancel()
      return
    }
    if (event.key === 'Backspace' || event.key === 'Delete') {
      onSet(null)
      return
    }
    const next = chordFromEvent(event)
    if (next) onSet(next)
  }

  return (
    <div className="keys-edit">
      <span className="keys-edit-label">
        {def.label}
        {def.surface && <span className="badge">in context</span>}
      </span>

      <button
        className={recording ? 'btn-mini is-on' : 'btn-mini'}
        aria-label={
          recording
            ? `Press the new shortcut for ${def.label}`
            : chord
              ? `${def.label} — ${formatChord(chord)}. Change it.`
              : `${def.label} — no shortcut. Add one.`
        }
        onClick={() => (recording ? onCancel() : onRecord())}
        onKeyDown={recording ? capture : undefined}
        onBlur={recording ? onCancel : undefined}
      >
        {recording ? (
          'Press a key…'
        ) : chord ? (
          <span className="keys-chord">
            {chordParts(chord).map((part, i) => (
              <kbd key={i}>{part}</kbd>
            ))}
          </span>
        ) : (
          'not bound'
        )}
      </button>

      {customised && (
        <button className="btn-mini" onClick={onReset}>
          Default ({formatChord(commandDef(def.id).chord)})
        </button>
      )}

      {recording && <span className="keys-hint">Escape cancels; Backspace leaves it unbound.</span>}

      {shadowedBy && (
        <span className="keys-clash">
          Shadowed by <b>{shadowedBy.label}</b>, which claims the same chord and runs first.
        </span>
      )}

      {reserved && !shadowedBy && (
        <span className="keys-hint">
          This is also the system&rsquo;s {reserved}, which will happen as well.
        </span>
      )}
    </div>
  )
}
