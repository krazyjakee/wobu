import { ariaChord, formatChord } from '../lib/keys'
import { bindingOf, useKeybindings, type CommandId } from '../store/keybindings'
import { useUI, type Mode } from '../store/ui'
import { GuideRailButton } from './GuideLink'
import { Icon } from './Icon'
import { IconButton } from './Tooltip'

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
   *
   * The rail used to own the only tooltip in Wobu — a `data-tip` attribute and
   * a `::after` in `shell.css` — which is why #129 exists: nothing else could
   * have one without copying that pair. It now uses the shared primitive, so
   * the tooltip reaches the keyboard, dismisses on Escape, and is not clipped
   * by the rail it is drawn inside.
   */
  const button = (def: (typeof MODES)[number]) => {
    const chord = bindingOf(overrides, def.command)
    return (
      <IconButton
        key={def.mode}
        className={mode === def.mode ? 'rbtn is-active' : 'rbtn'}
        label={def.tip}
        tip={chord ? `${def.tip} · ${formatChord(chord)}` : def.tip}
        placement="right"
        onClick={() => setMode(def.mode)}
        aria-keyshortcuts={ariaChord(chord)}
        aria-current={mode === def.mode ? 'page' : undefined}
      >
        <Icon name={def.icon} />
      </IconButton>
    )
  }

  return (
    <nav className="rail" aria-label="Workspace modes">
      {MODES.map(button)}
      <div className="rspace" />
      <GuideRailButton />
      {button(SETTINGS)}
    </nav>
  )
}
