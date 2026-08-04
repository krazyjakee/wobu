import { chordParts, formatChord } from '../lib/keys'
import {
  COMMANDS,
  COMMAND_GROUPS,
  bindingOf,
  findConflicts,
  useKeybindings,
  type CommandDef,
} from '../store/keybindings'
import { useUI } from '../store/ui'
import { Modal } from './Modal'

/**
 * The whole keyboard map, on screen.
 *
 * Half of these keys were previously findable only by reading the source: `[`,
 * `]`, `⌘\` and `⌘↵` appeared in no tooltip, no menu and no help of any kind.
 * A shortcut nobody can discover is not a feature, so this list is not an
 * appendix to the bindings editor — it is the thing the bindings editor exists
 * to describe, and it is one keystroke away from anywhere.
 *
 * It reads from the same registry the dispatcher does, so a rebound key is
 * right here the moment it changes, and a conflict is named rather than left to
 * be found by pressing the key and watching nothing happen.
 */
export function ShortcutsSheet() {
  const open = useUI((s) => s.shortcutsOpen)
  if (!open) return null
  return <OpenShortcutsSheet />
}

/** One row's chord, or the fact that it has none. */
function Chord({ chord }: { chord: string | null }) {
  if (!chord) return <span className="keys-unbound">no shortcut</span>
  return (
    <span className="keys-chord">
      {chordParts(chord).map((part, i) => (
        <kbd key={i}>{part}</kbd>
      ))}
    </span>
  )
}

function OpenShortcutsSheet() {
  const setOpen = useUI((s) => s.setShortcutsOpen)
  const setMode = useUI((s) => s.setMode)
  const overrides = useKeybindings((s) => s.overrides)
  const conflicts = findConflicts(overrides)

  /** Command ids that lose their chord to an earlier command. */
  const shadowed = new Map<string, CommandDef>()
  for (const conflict of conflicts) {
    for (const loser of conflict.shadowed) shadowed.set(loser.id, conflict.winner)
  }

  return (
    <Modal
      className="pal keys-sheet"
      scrimClassName="scrim-top"
      titleId="shortcuts-title"
      descriptionId="shortcuts-description"
      onClose={() => setOpen(false)}
    >
      <div className="keys-head">
        <h2 id="shortcuts-title">Keyboard shortcuts</h2>
        <p id="shortcuts-description">
          Every key Wobu listens for. Change any of them in Settings; they are kept on this computer
          rather than in the project folder.
        </p>
        <button
          className="btn-mini"
          data-modal-initial-focus
          onClick={() => {
            setOpen(false)
            setMode('settings')
          }}
        >
          Change these
        </button>
      </div>

      <div className="keys-list">
        {COMMAND_GROUPS.map((group) => {
          const rows = COMMANDS.filter((c) => c.group === group)
          if (rows.length === 0) return null
          return (
            <section key={group}>
              <h3>{group}</h3>
              <dl>
                {rows.map((def) => {
                  const chord = bindingOf(overrides, def.id)
                  const winner = shadowed.get(def.id)
                  return (
                    <div className="keys-row" key={def.id}>
                      <dt>
                        {def.label}
                        {def.note && <span className="keys-note">{def.note}</span>}
                        {winner && (
                          <span className="keys-clash">
                            {formatChord(chord ?? '')} runs <b>{winner.label}</b> instead — two
                            commands share it.
                          </span>
                        )}
                      </dt>
                      <dd>
                        <Chord chord={winner ? null : chord} />
                        {def.aliases?.map((alias) => (
                          <Chord key={alias} chord={alias} />
                        ))}
                      </dd>
                    </div>
                  )
                })}
              </dl>
            </section>
          )
        })}

        <section>
          <h3>Everywhere</h3>
          <dl>
            {/* Not in the registry and not rebindable. Escape means dismiss in
                every dialog on every platform, and a build where it did not
                would be broken rather than configurable. */}
            <div className="keys-row">
              <dt>
                Close a dialog
                <span className="keys-note">Fixed. Escape dismisses whatever is on top.</span>
              </dt>
              <dd>
                <Chord chord="Escape" />
              </dd>
            </div>
            <div className="keys-row">
              <dt>
                Move through a dialog
                <span className="keys-note">Focus stays inside it until it closes.</span>
              </dt>
              <dd>
                <Chord chord="Tab" />
                <Chord chord="Shift+Tab" />
              </dd>
            </div>
            <div className="keys-row">
              <dt>
                Choose in the palette or a menu
                <span className="keys-note">Arrows move, Enter takes the highlighted row.</span>
              </dt>
              <dd>
                <Chord chord="ArrowUp" />
                <Chord chord="ArrowDown" />
                <Chord chord="Enter" />
              </dd>
            </div>
          </dl>
        </section>
      </div>
    </Modal>
  )
}
