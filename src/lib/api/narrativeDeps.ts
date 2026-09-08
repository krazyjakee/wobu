import { call } from './call'

/**
 * One row of the Why affected inspector.
 *
 * `scene` and `asset` are exclusive: a scene line names the first, a supporting
 * text line the second. They are two fields rather than one because the two
 * containers are different documents with different editors, and a single
 * `container` string would put the job of telling them apart back on every
 * reader.
 */
export interface AffectedLine {
  scene: string | null
  asset: string | null
  slot: string
  variant: string
  /**
   * `changed` — what this line was written against moved.
   * `untracked` — nothing has ever been recorded for it, so nothing can have
   * gone stale.
   * `absent` — a recorded line the project no longer contains.
   */
  kind: 'changed' | 'untracked' | 'absent'
  /** The fingerprint the line's existing results were produced under. */
  before: string | null
  after: string | null
  explanations: AffectedExplanation[]
}

/** Source field → context or variant → line, in the backend's own words. */
export interface AffectedExplanation {
  source: string
  context: string
  line: string
  message: string
}

/** Every line whose recorded dependencies no longer match the project (#168). */
export const narrativeAffected = () => call<AffectedLine[]>('narrative_affected', {})
