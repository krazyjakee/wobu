import { useCallback, useMemo, useState } from 'react'
import type { CSSProperties, ReactNode } from 'react'
import type {
  CompiledPrompt,
  DroppedFragment,
  InfluenceFragment,
  NodeSummary,
  ProjectSummary,
} from '../../lib/api'
import { errorMessage } from '../../lib/api'
import { useCompiledPrompt } from '../../lib/queries'
import type { PromptOptions } from '../../lib/queries'
import { layerColor, layerLabel } from '../../lib/kinds'
import {
  dropGroups,
  formatWeight,
  fragmentLabel,
  promptEmptiness,
  promptSegments,
  sectionLabel,
} from '../../lib/prompt'
import type { PromptChannel } from '../../lib/prompt'
import { report, toast } from '../../store/ui'
import { Icon } from '../Icon'

/**
 * The compiled prompt, and the account of what did not make it into one.
 *
 * The differentiating surface of the whole product, and the rule it exists to
 * keep is *nothing is hidden*: the string is always the string the backend
 * compiled, every run of it names the layer and the node it came from, and
 * everything that was left out is on screen with the reason it was left out.
 * A box that showed only the survivors would be exactly the invisible
 * prompt-magic Wobu is meant not to have.
 *
 * Three things follow from that and are load-bearing:
 *
 * - **The text is real text.** Runs are inline `<span>`s in one paragraph with
 *   the separators as ordinary text nodes, so a selection crosses fragment
 *   boundaries and a copy yields the prompt rather than a soup of markup.
 *   Nothing here is generated content, absolutely positioned, or a control that
 *   a browser would refuse to select through.
 * - **Colour is never the only carrier.** Layer colours sit close together by
 *   design (docs/03-ui-layout.md) and some readers cannot tell them apart at
 *   all, so every run also carries its attribution as a `title`, the hover
 *   readout says it in words, and "Show sources" turns the whole thing into a
 *   labelled list.
 * - **`moodboard_only` is not in here.** It is not in the prompt, so it is not
 *   in the box, and it is not in the drop report either — it was not dropped
 *   from anything. See `promptSegments` and `dropGroups`.
 *
 * Everything it renders is a pure function of `useCompiledPrompt(subject,
 * options)`, so the weight sliders (#47) need only put their values in
 * `options`: a new value is a new query key, `keepPreviousData` holds the last
 * prompt on screen while the next one compiles, and the box redraws as the drag
 * moves. It owns no copy of the prompt to go stale.
 */
/**
 * A stable default so an omitted `options` does not produce a new query key —
 * and therefore a new compile — on every render of the Inspector.
 */
const EMPTY_OPTIONS: PromptOptions = {}

export function PromptBox({
  project,
  subject,
  options,
  onJump,
}: {
  /** Null on the Launcher, where there is no world to compile against. */
  project: ProjectSummary | null
  subject: NodeSummary | null
  /** Preset, sliders, shot and budget — whatever the Inspector's controls hold. */
  options?: PromptOptions
  onJump: (id: string) => void
}) {
  // Gated on the project as well as the selection: a subject left over from the
  // project that was just closed would compile against whatever the backend has
  // open now, or fail — either way it is a round trip that can only mislead.
  const compiled = useCompiledPrompt(
    project && subject ? subject.id : null,
    options ?? EMPTY_OPTIONS,
  )
  const data = compiled.data

  const [hovered, setHovered] = useState<InfluenceFragment | null>(null)
  const [asList, setAsList] = useState(false)

  // A click that ends a drag-selection is indistinguishable from a click on the
  // word, and this box has to be selectable. Selecting a phrase and landing on
  // another node is the more annoying of the two failures, so a live selection
  // wins and the jump is dropped.
  const jump = useCallback(
    (nodeId: string | null) => {
      if (!nodeId) return
      const selection = window.getSelection()
      if (selection && !selection.isCollapsed) return
      onJump(nodeId)
    },
    [onJump],
  )

  const drops = useMemo(() => dropGroups(data?.dropped ?? []), [data?.dropped])
  const emptiness = data ? promptEmptiness(data) : null

  return (
    <section className="prompt" aria-label="Compiled prompt">
      <div className="prompt-head">
        <h2>Compiled prompt</h2>
        {data && <span className="hint">{data.preset.label}</span>}
        {data && emptiness !== 'nothing-at-all' && (
          <button
            className={asList ? 'btn-mini is-on' : 'btn-mini'}
            onClick={() => setAsList((v) => !v)}
            aria-pressed={asList}
          >
            <Icon name="layers" size="sm" />
            {asList ? 'Show prose' : 'Show sources'}
          </button>
        )}
      </div>

      <div className="prompt-body">
        {!project ? (
          <Empty
            title="No project open."
            body="Open a world and the prompt it would send for the selected entity is compiled here."
          />
        ) : !subject ? (
          <Empty
            title="No node selected."
            body="Select an entity and this shows the exact string a generation would send for it, fragment by fragment, with what was left out and why."
          />
        ) : compiled.isError ? (
          <Empty title="The prompt could not be compiled." body={errorMessage(compiled.error)} />
        ) : !data ? (
          // Not a spinner: the compile does no file I/O, so this is on screen
          // for a frame or two and only ever persists if something is wrong,
          // in which case the error branch above replaces it.
          <p className="prompt-wait">Compiling…</p>
        ) : emptiness === 'nothing-at-all' ? (
          <Empty
            title="Nothing to compile yet."
            body={`${subject.name} has no described sections, and nothing above it contributes any. Write notes and enhance them, or link this to a style, a culture or a place.`}
          />
        ) : (
          <Compiled
            compiled={data}
            drops={drops}
            emptiness={emptiness}
            asList={asList}
            hovered={hovered}
            onHover={setHovered}
            onJump={jump}
          />
        )}
      </div>
    </section>
  )
}

function Compiled({
  compiled,
  drops,
  emptiness,
  asList,
  hovered,
  onHover,
  onJump,
}: {
  compiled: CompiledPrompt
  drops: { silenced: DroppedFragment[]; budget: DroppedFragment[] }
  emptiness: 'all-cut' | null
  asList: boolean
  hovered: InfluenceFragment | null
  onHover: (f: InfluenceFragment | null) => void
  onJump: (nodeId: string | null) => void
}) {
  // The hovered fragment is looked up again in the compile that is on screen
  // now, so the weight in the readout is the weight now. Held as the object the
  // pointer arrived on, it would keep reading 0.80 through a drag that has taken
  // it to 0.20 — the pointer does not move, so no second hover corrects it. A
  // fragment that this compile no longer emits falls back to the hint: it is not
  // in the prompt any more, and the readout should not claim otherwise.
  const live = hovered
    ? (compiled.spans.find(
        (s) =>
          s.layer === hovered.layer &&
          s.nodeId === hovered.nodeId &&
          s.section === hovered.section &&
          s.target === hovered.target,
      ) ?? null)
    : null

  return (
    <>
      <Channel
        label="Prompt"
        channel="prompt"
        text={compiled.prompt}
        spans={compiled.spans}
        asList={asList}
        onHover={onHover}
        onJump={onJump}
        empty={
          emptiness === 'all-cut'
            ? 'Everything was left out. Nothing reached the prompt, so a generation from here would have nothing to draw — the report below says which control brings it back.'
            : null
        }
      />

      {/* The negative prompt gets the same treatment rather than a plain
          string: a word nobody remembers writing is exactly as puzzling here as
          in the positive prompt, and the layer it came from is the answer. */}
      <Channel
        label="Negative"
        channel="negative"
        text={compiled.negative}
        spans={compiled.spans}
        asList={asList}
        onHover={onHover}
        onJump={onJump}
        empty={compiled.negative.length === 0 ? 'Nothing is being excluded.' : null}
      />

      <p className="prompt-read" aria-live="polite">
        {live
          ? `${fragmentLabel(live)} — weight ${formatWeight(live.weight)}${
              live.nodeId ? '. Click to open it.' : '. From the output preset, not the world.'
            }`
          : 'Hover any fragment to see where it came from. Click it to open that node.'}
      </p>

      {compiled.overflow !== null && (
        <p className="prompt-over">
          <Icon name="minus" size="sm" />
          Over the character budget by {compiled.overflow.toLocaleString()}. Everything but the
          heaviest fragment was cut and it still does not fit — an empty prompt is not a smaller
          picture, so this one is sent long rather than sent blank.
        </p>
      )}

      <Drops
        title="Turned down"
        note="A slider or a link weight is at zero. Raise it and these come straight back — nothing has been edited."
        rows={drops.silenced}
        onJump={onJump}
      />
      <Drops
        title="Did not fit"
        note="These were the lightest fragments when the prompt ran over budget. Write leaner notes upstream, or weight these sources higher so something else goes first."
        rows={drops.budget}
        onJump={onJump}
      />
    </>
  )
}

/** One compiled string — the positive prompt or the negative one. */
function Channel({
  label,
  channel,
  text,
  spans,
  asList,
  onHover,
  onJump,
  empty,
}: {
  label: string
  channel: PromptChannel
  text: string
  spans: InfluenceFragment[]
  asList: boolean
  onHover: (f: InfluenceFragment | null) => void
  onJump: (nodeId: string | null) => void
  empty: string | null
}) {
  const segments = useMemo(() => promptSegments(text, spans, channel), [text, spans, channel])
  const fragments = useMemo(
    () => segments.map((s) => s.fragment).filter((f): f is InfluenceFragment => f !== null),
    [segments],
  )

  // The string as the backend sent it, never the rendered DOM. A copy that went
  // through the markup could differ from the prompt by a stray space and nobody
  // would ever find out where the picture went wrong.
  const copy = () =>
    void navigator.clipboard.writeText(text).then(
      () => toast(`${label} copied.`),
      (e: unknown) => report(e, `Could not copy the ${label.toLowerCase()}`),
    )

  return (
    <div className="prompt-chan">
      <div className="prompt-chan-h">
        <b>{label}</b>
        <span className="prompt-count">
          {text.length > 0
            ? `${fragments.length} ${fragments.length === 1 ? 'fragment' : 'fragments'} · ${text.length.toLocaleString()} characters`
            : ''}
        </span>
        {text.length > 0 && (
          <button
            className="btn-mini"
            onClick={copy}
            aria-label={`Copy the ${label.toLowerCase()}`}
          >
            <Icon name="copy" size="sm" />
            Copy
          </button>
        )}
      </div>

      {empty ? (
        <p className="prompt-none">{empty}</p>
      ) : asList ? (
        <ol className="prompt-list">
          {fragments.map((f, i) => (
            <li key={i}>
              <FragmentRow fragment={f} onJump={onJump}>
                <span className="prompt-list-t">{f.text}</span>
              </FragmentRow>
            </li>
          ))}
        </ol>
      ) : (
        /* One paragraph, inline spans, separators as bare text. Anything else
           here — a wrapper per run, a flex row, generated commas — breaks
           selection across a boundary or puts characters in the clipboard that
           are not in the prompt. */
        <p className="prompt-text">
          {segments.map((seg, i) =>
            seg.fragment ? (
              <Span key={i} fragment={seg.fragment} onHover={onHover} onJump={onJump}>
                {seg.text}
              </Span>
            ) : (
              seg.text
            ),
          )}
        </p>
      )}
    </div>
  )
}

/**
 * One tinted run of the prompt.
 *
 * `role="link"` and a tab stop, because that is what it is: activating it opens
 * the node the words came from. The accessible name is the text itself, which is
 * the truth — the `title` becomes its description and carries the layer, the
 * source and the section for anyone who cannot use the tint.
 *
 * A Shot fragment has no node behind it — its text is the output preset's
 * framing — so it is tinted and described but not focusable and not a link.
 * Offering a jump that goes nowhere is worse than offering none.
 */
function Span({
  fragment,
  onHover,
  onJump,
  children,
}: {
  fragment: InfluenceFragment
  onHover: (f: InfluenceFragment | null) => void
  onJump: (nodeId: string | null) => void
  children: string
}) {
  const linked = fragment.nodeId !== null
  return (
    <span
      className={linked ? 'pfrag is-linked' : 'pfrag'}
      style={{ ['--lc' as string]: layerColor(fragment.layer) } as CSSProperties}
      title={`${fragmentLabel(fragment)} — weight ${formatWeight(fragment.weight)}`}
      role={linked ? 'link' : undefined}
      tabIndex={linked ? 0 : undefined}
      onMouseEnter={() => onHover(fragment)}
      onMouseLeave={() => onHover(null)}
      onFocus={() => onHover(fragment)}
      onBlur={() => onHover(null)}
      onClick={linked ? () => onJump(fragment.nodeId) : undefined}
      onKeyDown={
        linked
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onJump(fragment.nodeId)
              }
            }
          : undefined
      }
    >
      {children}
    </span>
  )
}

/**
 * A fragment named in words: the layer, the node and the section.
 *
 * The same row serves the sources list and the drop report, because they are the
 * same fragments seen from either side of the budget, and a reader who has
 * learned to read one has learned to read the other.
 */
function FragmentRow({
  fragment,
  onJump,
  children,
}: {
  fragment: InfluenceFragment
  onJump: (nodeId: string | null) => void
  children?: ReactNode
}) {
  const head = (
    <>
      <span className="lchip" style={{ ['--lc' as string]: layerColor(fragment.layer) }}>
        {layerLabel(fragment.layer)}
      </span>
      <span className="prompt-src">{fragment.sourceName}</span>
      <span className="prompt-sec">{sectionLabel(fragment.section)}</span>
      <span className="prompt-w">{formatWeight(fragment.weight)}</span>
    </>
  )
  return (
    <div className="prompt-row">
      {fragment.nodeId ? (
        <button className="prompt-rowh" onClick={() => onJump(fragment.nodeId)}>
          {head}
        </button>
      ) : (
        <span className="prompt-rowh">{head}</span>
      )}
      {children}
    </div>
  )
}

/**
 * One reason's worth of the drop report.
 *
 * Kept apart by reason and not merged, because "you turned this down" and "this
 * did not fit" send the reader to two different controls — one is a slider they
 * moved, the other is prose they have to shorten. Advice that is wrong half the
 * time is worse than none.
 */
function Drops({
  title,
  note,
  rows,
  onJump,
}: {
  title: string
  note: string
  rows: DroppedFragment[]
  onJump: (nodeId: string | null) => void
}) {
  if (rows.length === 0) return null
  return (
    // A section rather than a div: named, it is a landmark somebody can jump to,
    // and "what was left out" is the second thing anyone asks this panel.
    <section className="prompt-drops" aria-label={`${title} — left out of the prompt`}>
      <div className="prompt-drops-h">
        <b>{title}</b>
        <span className="prompt-count">{rows.length}</span>
      </div>
      <p className="prompt-note">{note}</p>
      {rows.map((d, i) => (
        <FragmentRow key={i} fragment={d.fragment} onJump={onJump}>
          <span className="prompt-cut">
            {d.fragment.text}
            {d.fragment.target === 'negative' && <em> — negative prompt</em>}
          </span>
        </FragmentRow>
      ))}
    </section>
  )
}

function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="insp-empty">
      <b>{title}</b>
      <span>{body}</span>
    </div>
  )
}
