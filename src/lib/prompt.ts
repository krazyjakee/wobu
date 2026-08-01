import type { CompiledPrompt, DroppedFragment, InfluenceFragment } from './api'
import { layerLabel } from './kinds'

/**
 * Turning what `prompt_compile` returns into something the prompt box can render
 * without ever showing a string the backend did not send.
 *
 * The compiler is Rust's (`wobu-influence`'s `compile.rs`) and there is only one
 * of it. Nothing here re-implements any part of it: the spans are matched
 * *against* the emitted string rather than joined into one, so a disagreement
 * between the two shows up as lost tinting and never as a prompt that reads
 * differently from the prompt a generation would send.
 */

/**
 * What joins two fragments in a compiled prompt.
 *
 * The same `", "` `compile.rs` emits with, and charged to the budget there. It
 * appears here only as something to recognise while walking the string — see
 * `promptSegments`, which falls back to plain text rather than trusting it.
 */
const SEPARATOR = ', '

/** Which of the two compiled strings a fragment belongs to. */
export type PromptChannel = 'prompt' | 'negative'

/** A run of the compiled string, and the fragment that put it there. */
export interface PromptSegment {
  text: string
  /** Null for the separator between two fragments, which belongs to neither. */
  fragment: InfluenceFragment | null
}

/**
 * Split one compiled string into tinted runs, matched against the spans.
 *
 * `spans` covers both prompts interleaved in reading order, which is also
 * emission order within each pool, so filtering to one channel gives that
 * channel's fragments in the order they were joined.
 *
 * Two filters, not one. `target` is what routes a fragment into a string;
 * `sendable` is what says it may leave the machine at all, and it is false for
 * exactly one thing — `moodboard_only`. That has already been kept out at the
 * link, the fragment, the budget and the bridge, and it is restated here because
 * this is the last surface before a human reads the prompt, and the direction
 * this fails in is somebody's mood board appearing in a string they then paste
 * into a provider's box by hand.
 *
 * **The walk verifies rather than assembles.** If the runs do not account for
 * the string exactly — a routing change, a separator change, a fragment whose
 * text was rewritten between compiling and emitting — the whole string comes
 * back as one untinted segment. Attribution is lost, which is visible; the
 * alternative is a box that reads plausibly and is not the prompt, which is the
 * one failure this surface must not have.
 */
export function promptSegments(
  text: string,
  spans: InfluenceFragment[],
  channel: PromptChannel,
): PromptSegment[] {
  const mine = spans.filter((s) => s.sendable && s.target === channel && s.text !== null)

  const out: PromptSegment[] = []
  let cursor = 0
  for (const fragment of mine) {
    const body = fragment.text as string
    if (out.length > 0) {
      if (!text.startsWith(SEPARATOR, cursor)) return plain(text)
      out.push({ text: SEPARATOR, fragment: null })
      cursor += SEPARATOR.length
    }
    if (!text.startsWith(body, cursor)) return plain(text)
    out.push({ text: body, fragment })
    cursor += body.length
  }
  if (cursor !== text.length) return plain(text)
  return out
}

function plain(text: string): PromptSegment[] {
  return text.length > 0 ? [{ text, fragment: null }] : []
}

/**
 * The drop report, split by what the user would have to do about it.
 *
 * The two reasons send someone to two different controls: `silenced` means a
 * slider is down and the fragment is one drag away from coming back, `budget`
 * means it was among the lightest and the notes upstream are too long. Merging
 * them into one "dropped" list would be advice that is wrong half the time.
 *
 * `sendable` is filtered for the same reason `promptSegments` filters it: the
 * drop report is part of the prompt box, and a mood board reference has not been
 * dropped from anything — it is doing exactly what it was attached to do.
 * `compile.rs` never reports one; this is the belt to its braces.
 */
export interface DropGroups {
  silenced: DroppedFragment[]
  budget: DroppedFragment[]
}

export function dropGroups(dropped: DroppedFragment[]): DropGroups {
  const groups: DropGroups = { silenced: [], budget: [] }
  for (const d of dropped) {
    if (!d.fragment.sendable || d.fragment.text === null) continue
    groups[d.reason === 'silenced' ? 'silenced' : 'budget'].push(d)
  }
  return groups
}

/**
 * Why the box has no prompt to show, when it has none.
 *
 * - `nothing-at-all` — nothing was written, upstream or here. The user has to go
 *   and describe something.
 * - `all-cut` — fragments existed and none of them reached the positive prompt.
 *   The drop report is the whole answer, so it stays on screen.
 *
 * An empty positive prompt is never silent truncation: `compile.rs` keeps the
 * heaviest fragment rather than emptying the pool, so `all-cut` means every
 * fragment was turned down or routed elsewhere, and it is worth saying so.
 */
export type PromptEmptiness = 'nothing-at-all' | 'all-cut' | null

export function promptEmptiness(compiled: CompiledPrompt): PromptEmptiness {
  if (compiled.prompt.length > 0) return null
  const nothing =
    compiled.negative.length === 0 && compiled.spans.length === 0 && compiled.dropped.length === 0
  return nothing ? 'nothing-at-all' : 'all-cut'
}

/** "Style · Ashfall Style Guide · silhouette" — attribution in words, not colour. */
export function fragmentLabel(f: InfluenceFragment): string {
  return `${layerLabel(f.layer)} · ${f.sourceName} · ${sectionLabel(f.section)}`
}

/**
 * A description section key or a reference role as prose. The registry's own
 * labels are per kind and a span only carries the key, so the underscore is all
 * there is to fix — and `full_ref` reading as "full ref" is still better than a
 * key nobody typed.
 */
export function sectionLabel(section: string): string {
  return section.replace(/_/g, ' ')
}

/** Two decimals, because a slider at 0.05 and one at 0.5 must not read alike. */
export function formatWeight(weight: number): string {
  return weight.toFixed(2)
}
