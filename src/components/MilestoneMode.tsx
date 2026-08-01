import type { Mode } from '../store/ui'
import { useUI } from '../store/ui'
import { Icon } from './Icon'

const COPY: Record<
  Exclude<Mode, 'library'>,
  { icon: string; title: string; ms: string; body: string }
> = {
  board: {
    icon: 'board',
    title: 'Board',
    ms: 'M3 — References',
    body: 'A freeform pan/zoom canvas holding every image in the project, where dragging a picture onto an entity turns loose inspiration into a weighted reference. It needs the asset pipeline, so it arrives with References.',
  },
  assets: {
    icon: 'assets',
    title: 'Assets',
    ms: 'M3 — References',
    body: 'Every file under assets/, content-addressed and filterable by role. Nothing has been imported yet because image import itself is not built.',
  },
  forge: {
    icon: 'forge',
    title: 'Forge',
    ms: 'M6 — Iteration and consistency',
    body: "The Inspector's controls promoted to full width with a large result grid, for hammering one subject until it is right. It depends on generation, which depends on the Influence Engine.",
  },
  settings: {
    icon: 'settings',
    title: 'Settings',
    ms: 'M4 — Enhance (first BYOK providers)',
    body: 'Provider keys, model defaults and appearance. Keys live in the OS keychain and never in the project folder, so this screen lands with the first provider that needs one.',
  },
}

export function MilestoneMode({ mode }: { mode: Exclude<Mode, 'library'> | Mode }) {
  const setMode = useUI((s) => s.setMode)
  if (mode === 'library') return null
  const c = COPY[mode]
  return (
    <div className="mode-empty">
      <div className="empty">
        <Icon name={c.icon} size="xl" />
        <h3>{c.title}</h3>
        <span className="milestone">{c.ms}</span>
        <p>{c.body}</p>
        <button className="btn" onClick={() => setMode('library')}>
          <Icon name="library" size="sm" />
          Back to Library
        </button>
      </div>
    </div>
  )
}
