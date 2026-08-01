import type { Mode } from '../store/ui'
import type { QueueSnapshot } from '../lib/api'
import { elapsedText } from '../lib/jobs'
import { useUI } from '../store/ui'
import { Icon } from './Icon'

const COPY: Record<
  Exclude<Mode, 'library' | 'assets'>,
  { icon: string; title: string; ms: string; body: string }
> = {
  board: {
    icon: 'board',
    title: 'Board',
    ms: 'M3 — References',
    body: 'A freeform pan/zoom canvas holding every image in the project, where dragging a picture onto an entity turns loose inspiration into a weighted reference. It needs the asset pipeline, so it arrives with References.',
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

export function MilestoneMode({
  mode,
  queue,
}: {
  mode: Exclude<Mode, 'library'> | Mode
  queue: QueueSnapshot
}) {
  const setMode = useUI((s) => s.setMode)
  if (mode === 'library' || mode === 'assets') return null
  if (mode === 'forge') return <QueueView queue={queue} onBack={() => setMode('library')} />
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

function QueueView({ queue, onBack }: { queue: QueueSnapshot; onBack: () => void }) {
  return (
    <div className="mode-empty queue-view">
      <div className="queue-panel">
        <div className="queue-head">
          <div>
            <h3>Job queue</h3>
            <p>Live backend work and the recent outcomes retained by Wobu.</p>
          </div>
          <button className="btn" onClick={onBack}>Back to Library</button>
        </div>
        {queue.jobs.length === 0 ? (
          <p className="queue-empty">Queue 0 · no recent jobs</p>
        ) : (
          <ol className="queue-jobs">
            {queue.jobs.map((job) => (
              <li key={job.id}>
                <span>{job.label}</span>
                <code>{job.state}</code>
                <span>{job.elapsedMs > 0 ? elapsedText(job.elapsedMs) : 'not started'}</span>
              </li>
            ))}
          </ol>
        )}
      </div>
    </div>
  )
}
