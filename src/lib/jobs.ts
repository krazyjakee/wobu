import type { JobSnapshot, QueueSnapshot } from './api'

/** Last successful image generation, not the last arbitrary or failed job. */
export function lastGeneration(snapshot: QueueSnapshot): JobSnapshot | null {
  for (let index = snapshot.jobs.length - 1; index >= 0; index -= 1) {
    const job = snapshot.jobs[index]
    if (job?.kind === 'generate' && job.state === 'done') return job
  }
  return null
}

export function elapsedText(elapsedMs: number): string {
  const seconds = Math.max(0, elapsedMs) / 1_000
  if (seconds < 10) return `${seconds.toFixed(1)}s`
  if (seconds < 60) return `${Math.round(seconds)}s`
  const minutes = Math.floor(seconds / 60)
  const rest = Math.round(seconds % 60)
  return rest === 0 ? `${minutes}m` : `${minutes}m ${rest}s`
}
