import { useUI, type Mode } from '../store/ui'
import { Icon } from './Icon'

const MODES: { mode: Mode; icon: string; tip: string }[] = [
  { mode: 'library', icon: 'library', tip: 'Library' },
  { mode: 'board', icon: 'board', tip: 'Board' },
  { mode: 'forge', icon: 'forge', tip: 'Forge — M6' },
  { mode: 'assets', icon: 'assets', tip: 'Assets' },
  { mode: 'history', icon: 'history', tip: 'Generation history' },
]

export function ModeRail() {
  const mode = useUI((s) => s.mode)
  const setMode = useUI((s) => s.setMode)

  return (
    <nav className="rail">
      {MODES.map((m) => (
        <button
          key={m.mode}
          className={mode === m.mode ? 'rbtn is-active' : 'rbtn'}
          data-tip={m.tip}
          onClick={() => setMode(m.mode)}
          aria-label={m.tip}
        >
          <Icon name={m.icon} />
        </button>
      ))}
      <div className="rspace" />
      <button
        className={mode === 'settings' ? 'rbtn is-active' : 'rbtn'}
        data-tip="Settings"
        onClick={() => setMode('settings')}
        aria-label="Settings"
      >
        <Icon name="settings" />
      </button>
    </nav>
  )
}
