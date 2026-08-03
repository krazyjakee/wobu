import { invoke } from '@tauri-apps/api/core'
import { create } from 'zustand'
import privacyPolicy from '../../docs/legal/privacy-policy.md?raw'
import termsOfUse from '../../docs/legal/terms.md?raw'
import { isTauri } from '../lib/api'

/**
 * First run, from install to first concept — and the legal gate in front of it.
 *
 * Deliberately *not* persisted with zustand's `persist` middleware, unlike
 * `store/settings.ts` beside it. Local storage is the right home for a
 * preference; it is the wrong home for a record that the terms of use and the
 * privacy policy were accepted. Clearing site data is a thing a webview does on
 * its own, and an agreement that a cache eviction can silently undo is not a
 * record of anything. Both facts therefore live in the Rust process, in the
 * application-data `settings.json` that `machine.rs` already owns and already
 * writes at mode 0600 — beside the ComfyUI endpoint, for the same reason: they
 * belong to this installation, never to a project folder that gets shared.
 *
 * What this store holds is the *session*: which step is on screen, and the last
 * answer the core gave. Nothing here is the source of truth.
 */

/**
 * The revision that was on screen, derived rather than typed.
 *
 * `LegalSection` explains why the documents themselves are read from
 * `docs/legal/` at build time instead of being retyped in TSX: a legal pane
 * that has drifted from the document it claims to be is worse than none. A
 * hand-maintained version constant is the same hazard one step removed — it
 * would be the number that got forgotten in the commit that rewrote a clause.
 * So the version is the `**Last updated:**` line out of each document, which is
 * the one thing in them that a substantive edit is actually expected to change.
 *
 * A build whose documents lost that line falls back to a fixed string rather
 * than to something unstable: a digest of the full text would re-prompt every
 * reader for a typo fix, which teaches people to click through the gate.
 */
function revisionOf(document: string, fallback: string): string {
  const found = /\*\*Last updated:\*\*\s*([^·\n]+)/.exec(document)?.[1]
  return found ? found.trim() : fallback
}

export const LEGAL_VERSION = `terms ${revisionOf(termsOfUse, 'unversioned')}; privacy ${revisionOf(
  privacyPolicy,
  'unversioned',
)}`

/**
 * The steps, in order.
 *
 * `legal` is first and is not one of the skippable ones — see `OnboardingState`
 * below. The rest are a path rather than a form: each says what to do next on
 * the real surfaces, and none of them holds state the app needs.
 */
export type OnboardingStep = 'legal' | 'welcome' | 'project' | 'providers' | 'concept'

export const ONBOARDING_STEPS: OnboardingStep[] = [
  'legal',
  'welcome',
  'project',
  'providers',
  'concept',
]

/** Exactly `machine.rs`'s `OnboardingState`. */
export interface OnboardingRecord {
  legalAcceptedAt: string | null
  legalVersion: string | null
  completedAt: string | null
}

const NOTHING_RECORDED: OnboardingRecord = {
  legalAcceptedAt: null,
  legalVersion: null,
  completedAt: null,
}

/**
 * Outside the webview there is no `settings.json` and no core to ask. `App`
 * already refuses to render anything but an explanation in that case, so
 * answering "nothing recorded" here keeps the store honest without inventing a
 * second no-backend surface.
 */
function ask<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return Promise.reject(new Error(`No Wobu core: ${command} is unavailable.`))
  return invoke<T>(command, args)
}

/** Whether the documents on screen are the ones this record agreed to. */
function agreed(record: OnboardingRecord): boolean {
  return record.legalAcceptedAt !== null && record.legalVersion === LEGAL_VERSION
}

interface OnboardingState {
  /** `null` until the core has answered — the overlay draws nothing before then. */
  record: OnboardingRecord | null
  /** Whether the overlay is on screen at all. */
  open: boolean
  step: OnboardingStep
  /** Set while a write to `settings.json` is in flight. */
  saving: boolean

  /**
   * Ask the core what has been settled, and open the overlay if anything is
   * outstanding. Called once, from `App`.
   */
  load: () => Promise<void>
  go: (step: OnboardingStep) => void
  /**
   * Record the acceptance, then continue.
   *
   * Rejections are returned as `false` rather than thrown: the caller shows the
   * reason beside the button that caused it, and a keychain-style failure to
   * write must not be able to advance the gate.
   */
  acceptLegal: () => Promise<boolean>
  /** Finish or skip — both record completion so the overlay stops opening itself. */
  dismiss: () => Promise<void>
  /** Re-run, from Settings or from the launcher. Never un-accepts anything. */
  restart: () => void
}

export const useOnboarding = create<OnboardingState>((set, get) => ({
  record: null,
  open: false,
  step: 'legal',
  saving: false,

  load: async () => {
    const record = await ask<OnboardingRecord>('onboarding_state').catch(() => NOTHING_RECORDED)
    // Two independent reasons to open, and the order matters: an installation
    // that finished the tour under an older revision of the documents gets the
    // gate again and nothing else, rather than the whole tour a second time.
    const gated = !agreed(record)
    set({
      record,
      open: gated || record.completedAt === null,
      step: gated ? 'legal' : 'welcome',
    })
  },

  go: (step) => set({ step }),

  acceptLegal: async () => {
    set({ saving: true })
    try {
      const record = await ask<OnboardingRecord>('onboarding_accept_legal', {
        version: LEGAL_VERSION,
      })
      // A record that has already been through the tour has nothing left to
      // show once the documents are agreed to again.
      set({ record, step: 'welcome', open: record.completedAt === null })
      return true
    } catch {
      return false
    } finally {
      set({ saving: false })
    }
  },

  dismiss: async () => {
    set({ open: false })
    const record = await ask<OnboardingRecord>('onboarding_finish').catch(() => null)
    if (record) set({ record })
  },

  restart: () => {
    // Re-running never re-opens the gate: the documents were accepted, and
    // asking again would imply the earlier answer had expired. If the *text*
    // has changed, `load` has already reopened the gate at launch.
    const record = get().record
    set({ open: true, step: record && !agreed(record) ? 'legal' : 'welcome' })
  },
}))
