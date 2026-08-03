import { ariaChord, formatChord } from '../lib/keys'
import { bindingOf, useKeybindings, type CommandId } from '../store/keybindings'
import { useUI, type Mode } from '../store/ui'
import { Icon } from './Icon'

const MODES: { mode: Mode; icon: string; tip: string; command: CommandId }[] = [
  { mode: 'library', icon: 'library', tip: 'Library', command: 'mode.library' },
  { mode: 'forge', icon: 'forge', tip: 'Forge', command: 'mode.forge' },
  { mode: 'assets', icon: 'assets', tip: 'Assets', command: 'mode.assets' },
]

const SETTINGS: { mode: Mode; icon: string; tip: string; command: CommandId } = {
  mode: 'settings',
  icon: 'settings',
  tip: 'Settings',
  command: 'mode.settings',
}

export function ModeRail() {
  const mode = useUI((s) => s.mode)
  const setMode = useUI((s) => s.setMode)
  const overrides = useKeybindings((s) => s.overrides)

  /*
   * The tooltip carries the key.
   *
   * These buttons were the only visible route to a mode, while `⌘\` existed and
   * said so nowhere — so the rail is where a reader is most likely to be
   * looking at the moment the shortcut would have helped. It is read from the
   * registry rather than written into the label, so a rebound key moves the
   * tooltip with it.
   */
  const button = (def: (typeof MODES)[number]) => {
    const chord = bindingOf(overrides, def.command)
    return (
      <button
        key={def.mode}
        className={mode === def.mode ? 'rbtn is-active' : 'rbtn'}
        data-tip={chord ? `${def.tip} · ${formatChord(chord)}` : def.tip}
        onClick={() => setMode(def.mode)}
        aria-label={def.tip}
        aria-keyshortcuts={ariaChord(chord)}
        aria-current={mode === def.mode ? 'page' : undefined}
      >
        <Icon name={def.icon} />
      </button>
    )
  }

  return (
    <nav className="rail" aria-label="Workspace modes">
      {MODES.map(button)}
      <div className="rspace" />
      {button(SETTINGS)}
    </nav>
  )
}
