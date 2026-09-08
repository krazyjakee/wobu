import { narrativeTextWritten } from '../../lib/api'
import type { RepeatPolicy, TextAsset, TextKind } from '../../lib/api/narrativeText'

/**
 * The six templates the Text library offers, and the rules a form has to
 * respect (#167).
 *
 * A table rather than six components, because the kinds differ in exactly two
 * ways — who may voice them and how many lines one delivery is — and the Rust
 * model already enforces both. Restating those rules as six bespoke forms would
 * give the UI a second, drifting opinion about what a codex entry is; keeping
 * one table means the form can only be wrong in the same way the model is.
 *
 * The rules here shape the *controls*: a codex form does not offer a cast
 * picker, and a bark form does not offer "add another line". They are not the
 * enforcement — that is `TextAsset::diagnostics` on the Rust side, which the
 * editor renders — because a caller reaching the same document through Source
 * or through a sync peer never sees this file at all.
 */
export interface TextTemplate {
  kind: TextKind
  label: string
  /** What the writer is being asked for, in one sentence. */
  hint: string
  /** A trigger name that reads like the moment, offered as a starting point. */
  event: string
  /** Whether entries may hold more than one line. */
  sequence: boolean
  /** Whether characters may voice it, and therefore whether it has a cast. */
  cast: boolean
}

export const TEXT_TEMPLATES: TextTemplate[] = [
  {
    kind: 'bark',
    label: 'Bark',
    hint: 'One line thrown at the player in passing.',
    event: 'player_passes',
    sequence: false,
    cast: true,
  },
  {
    kind: 'ambient',
    label: 'Ambient exchange',
    hint: 'A short conversation the player overhears rather than joins.',
    event: 'area_idle',
    sequence: true,
    cast: true,
  },
  {
    kind: 'reaction',
    label: 'Companion reaction',
    hint: 'One line answering something that just happened.',
    event: 'event_happened',
    sequence: false,
    cast: true,
  },
  {
    kind: 'codex',
    label: 'Codex entry',
    hint: 'An encyclopaedia page. Prose, with no speaker.',
    event: 'codex_opened',
    sequence: true,
    cast: false,
  },
  {
    kind: 'quest_summary',
    label: 'Quest summary',
    hint: 'What the quest log shows about where the player is up to.',
    event: 'quest_log_opened',
    sequence: true,
    cast: false,
  },
  {
    kind: 'journal',
    label: 'Journal entry',
    hint: 'A first-person record, in the player’s own voice.',
    event: 'day_ends',
    sequence: true,
    cast: false,
  },
]

/**
 * The template for a kind.
 *
 * The list is exhaustive over `TextKind`, so the fallback is unreachable rather
 * than a policy — but it is a bark rather than a throw, because a document
 * written by a newer build with a seventh kind should render with the most
 * cautious controls instead of taking the whole pane down.
 */
export function templateOf(kind: TextKind): TextTemplate {
  return TEXT_TEMPLATES.find((template) => template.kind === kind) ?? BARK
}

const BARK: TextTemplate = TEXT_TEMPLATES[0]!

/** What each selection policy does, in the words a writer would use. */
export const REPEAT_LABELS: Record<RepeatPolicy, string> = {
  first: 'First match — always the first eligible entry',
  once: 'Once — each entry once, then nothing',
  cycle: 'Cycle — round robin in author order',
  shuffle: 'Shuffle — each entry once per round, seeded, never twice running',
}

/**
 * Re-seal only the wording that actually changed.
 *
 * The supporting-text counterpart of `prepareScriptText`, and it exists for the
 * same reason: a revision is what a translation, a recording and an approval
 * are keyed to, so re-deriving one for a line nobody touched would invalidate
 * all three for nothing. Sealing goes through the backend rather than being
 * hashed here, because the hash recipe is the model's and a second
 * implementation in TypeScript is a second thing to keep exactly right.
 */
export async function prepareTextAsset(before: TextAsset, draft: TextAsset): Promise<TextAsset> {
  const next = structuredClone(draft)
  const original = new Map(
    (before.entries ?? [])
      .flatMap((entry) => entry.lines ?? [])
      .flatMap((slot) => slot.variants ?? [])
      .map((variant) => [variant.id, variant.text]),
  )
  for (const entry of next.entries ?? []) {
    for (const slot of entry.lines ?? []) {
      for (const variant of slot.variants ?? []) {
        const was = original.get(variant.id)
        if (was?.body === variant.text.body) continue
        const text = await narrativeTextWritten(
          variant.text.body,
          variant.text.lifecycle?.policy === 'locked',
        )
        variant.text = {
          ...text,
          // Rewriting the words does not answer the question freshness asks, so
          // the previous answer is carried rather than reset to "current".
          lifecycle: { ...text.lifecycle, freshness: was?.lifecycle?.freshness ?? 'current' },
        }
      }
    }
  }
  return next
}
