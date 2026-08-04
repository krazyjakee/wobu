import type { JobState, SyncPhase } from './api'

/**
 * The identifiers the core switches on, in the words the app speaks — #127.
 *
 * Several surfaces printed a backend state straight onto the screen: the Forge
 * job list rendered `job.state`, the status bar rendered `sync.state`, and the
 * LoRA card rendered `application_state` with its underscores swapped for
 * spaces, which is a code identifier wearing a hat. A user waiting for a
 * picture read "retrying"; a user with a shared project read "sync · idle" and
 * had to guess whether that was good news.
 *
 * They live together rather than beside each caller because the same state now
 * appears on more than one surface, and two copies of a word is how a queue
 * comes to be "working" in one pane and "running" in the next.
 *
 * Every function here falls back to the raw identifier rather than to nothing:
 * a state this build has never heard of is still better shown badly than
 * silently dropped.
 */

/** Where a job is. */
export function jobStateLabel(state: JobState['state']): string {
  switch (state) {
    case 'queued':
      return 'waiting'
    case 'running':
      return 'working'
    case 'retrying':
      return 'trying again'
    case 'done':
      return 'finished'
    case 'cancelled':
      return 'stopped'
    case 'failed':
      return 'failed'
  }
}

/** Where peer-to-peer sync has got to. */
export function syncStateLabel(state: SyncPhase): string {
  switch (state) {
    case 'idle':
      return 'up to date'
    case 'connecting':
      return 'connecting'
    case 'syncing':
      return 'catching up'
    case 'offline':
      return 'offline'
  }
}

/**
 * Whether a trained style is actually being used, and if not, why not.
 *
 * `wobu`'s `lora.rs` sends these as snake_case strings rather than as a typed
 * enum, so the fallback below is load-bearing rather than defensive.
 */
const LORA_STATE_LABELS: Record<string, string> = {
  none: 'not trained yet',
  ready: 'in use',
  not_installed: 'trained, not installed',
  model_mismatch: 'trained for a different model',
  provider_unsupported: 'this provider cannot use it',
  weight_missing: 'its file is missing',
  weight_corrupt: 'its file is damaged',
}

export function loraStateLabel(state: string): string {
  return LORA_STATE_LABELS[state] ?? state.replaceAll('_', ' ')
}

/** Whether the local trainer can be reached, and whether it is the right one. */
const TRAINER_STATE_LABELS: Record<string, string> = {
  available: 'ready',
  unavailable: 'not reachable',
  incompatible: 'wrong version',
}

export function trainerStateLabel(state: string): string {
  return TRAINER_STATE_LABELS[state] ?? state.replaceAll('_', ' ')
}
