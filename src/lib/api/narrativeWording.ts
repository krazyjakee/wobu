import { call } from './call'

/** Which document a wording lives in; the two open different editors. */
export type WordingContainer = { scene: string } | { text_asset: string }

export interface WordingSite {
  container: WordingContainer
  containerName: string
  slot: string
  variant: string
  /** Keyed finely enough to select this copy and no other. */
  site: unknown
}
export interface DuplicatedWording {
  revision: string
  /** The shared words, so a reader sees what was duplicated without opening a site. */
  body: string
  /** Every copy. At least two, by construction. */
  sites: WordingSite[]
}
export interface WordingSuppression {
  revision: string
  rationale: string
}
export interface WordingReport {
  duplicated: DuplicatedWording[]
  suppressions: WordingSuppression[]
  /** Opaque. Carry it, hand it back, never build one. */
  guard: string
  unreadable: string[]
}

export const narrativeWordingReport = (): Promise<WordingReport> => call('narrative_wording_report')
export const narrativeWordingSuppress = (
  suppressions: WordingSuppression[],
  expected: string,
): Promise<WordingReport> => call('narrative_wording_suppress', { suppressions, expected })
