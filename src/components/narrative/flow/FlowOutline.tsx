import { PresentationTools, PinnedNotes } from './PresentationTools'
import { useFlowGroupPresentation } from './useFlowGroupPresentation'
import type { FlowPresentation } from './useFlowPresentation'
import { ParticipantFilter, FlowStatusFilters } from './ParticipantFilter'
import { FlowBadgeFilters } from './FlowBadgeFilters'
import { isFlowElementMuted } from './flowFilters'
import { badgeShown } from './badges'
import { useUI } from '../../../store/ui'
import type { CanonicalFlowActions } from './canonicalFlow'
import { useMemo, useRef } from 'react'
import { useFlowReveal } from './useFlowReveal'
import { Icon } from '../../Icon'
import { NARRATIVE_STATUS } from '../narrativeModel'
import { useFlowLevel } from './flowStore'
import { useSceneEdits, type FlowAuthoring } from './useSceneEdits'
import {
  FLOW_KIND_ICON,
  FLOW_KIND_LABEL,
  type FlowElement,
  type FlowKind,
  type FlowLevel,
  type FlowPort,
} from './model'

/**
 * The same scene as an ordered list, and a full alternative to the canvas.
 *
 * #151 requires this, and for two different readers. A screen-reader user gets
 * a document rather than a viewport — every element, every way out, every
 * destination, in reading order, with ordinary buttons and selects instead of
 * a zoomable plane. A user in a narrow window gets something that works at 320
 * pixels wide.
 *
 * "Full alternative" is enforced by construction: every operation here goes
 * through `useSceneEdits`, the same hook the canvas calls, so an operation
 * cannot exist on one and not the other. Selection is the same store too, so
 * switching between the two lands on the element you were reading.
 *
 * Order is the reading order of the story — depth first from the scene's entry,
 * following each element's ports in authored order — and then anything the
 * entry cannot reach, which is a finding in itself.
 *
 * Both levels use it (#187). An arc read as a list is scenes in the order the
 * story reaches them, with each one's exits as selects and any scene the start
 * cannot reach called out — which is exactly the finding US-08 asks for, and
 * the one thing a canvas is worst at showing.
 */

const SCENE_CREATABLE: FlowKind[] = ['beat', 'choice', 'condition', 'outcome', 'end']

function readingOrder(scene: FlowLevel): { element: FlowElement; reachable: boolean }[] {
  const byId = new Map(scene.elements.map((element) => [element.id, element]))
  const seen = new Set<string>()
  const ordered: FlowElement[] = []

  const walk = (id: string | null) => {
    if (id === null || seen.has(id)) return
    const element = byId.get(id)
    if (!element) return
    seen.add(id)
    ordered.push(element)
    for (const port of element.out) walk(port.to)
  }

  walk(scene.entryId)
  const unreachable = scene.elements.filter((element) => !seen.has(element.id))
  return [
    ...ordered.map((element) => ({ element, reachable: true })),
    ...unreachable.map((element) => ({ element, reachable: false })),
  ]
}

export function FlowOutline({
  scene,
  onChange,
  readOnly = false,
  creatable = SCENE_CREATABLE,
  spare,
  onActivate,
  activateLabel = 'Open',
  targetOf,
  authoring,
  actions,
  presentation,
}: {
  scene: FlowLevel
  onChange: (scene: FlowLevel) => void
  readOnly?: boolean
  /** What this level allows, and the sentence for each refusal. See `useSceneEdits`. */
  authoring?: FlowAuthoring
  actions?: CanonicalFlowActions
  presentation?: FlowPresentation
  /** Which kinds this level can create. The arc creates scenes and nothing else. */
  creatable?: readonly FlowKind[]
  /** The unauthored way out this level offers, if any. See `FlowGraphNode.spare`. */
  spare?: (element: FlowElement) => FlowPort | null
  /** Go into this element — the arc's drill-down, as a plain button. */
  onActivate?: (id: string) => void
  activateLabel?: string
  targetOf?: Parameters<typeof useSceneEdits>[0]['targetOf']
}) {
  const container = useRef<HTMLOListElement>(null)
  useFlowGroupPresentation(presentation, readOnly)
  const closedGroups = useFlowLevel((state) => state.closedGroups)
  const setClosedGroups = useFlowLevel((state) => state.setClosedGroups)
  const participant = useFlowLevel((state) => state.participant)
  const badges = useFlowLevel((state) => state.badges)
  const statuses = useUI((state) => state.narrativeFilters)
  const level = presentation?.layout.graph.kind === 'scene' ? 'scene' : 'arc'
  useFlowReveal({ scene, container, projectKey: actions?.projectKey })
  const sourceDisabled = readOnly || !!actions?.disabled
  const selectedId = useFlowLevel((s) => s.selectedId)
  const announcement = useFlowLevel((s) => s.announcement)
  const { select, connect, remove, add } = useSceneEdits({
    scene,
    onChange,
    readOnly: sourceDisabled,
    targetOf,
    authoring,
    actions,
  })
  const rows = useMemo(() => readingOrder(scene), [scene])
  const destinations = actions
    ? scene.elements.filter((element) =>
        ['beat', 'end', 'sceneLink', 'missing'].includes(element.kind),
      )
    : scene.elements

  return (
    <>
      <div className="nrt-flow-bar" role="toolbar" aria-label="Flow outline actions">
        {presentation && (
          <PresentationTools presentation={presentation} level={level} readOnly={readOnly} />
        )}
        <button
          type="button"
          className="btn btn-sm"
          disabled={!scene.groups.length}
          onClick={() =>
            setClosedGroups(closedGroups.length ? [] : scene.groups.map((group) => group.id))
          }
        >
          {closedGroups.length ? 'Open all groups' : 'Close all groups'}
        </button>
        <ParticipantFilter scene={scene} />
        <FlowStatusFilters />
        {scene.elements.some((element) => element.diagnostics || element.counts) && (
          <FlowBadgeFilters />
        )}
      </div>
      {presentation && (
        <PinnedNotes
          presentation={presentation}
          positions={presentation.layout.nodes}
          readOnly={readOnly}
          outline
        />
      )}
      <p className="sr-only" role="status" aria-live="polite">
        <span key={announcement.seq}>{announcement.text}</span>
      </p>
      <ol ref={container} className="nrt-outline" aria-label={`${scene.name} outline`}>
        {scene.groups.map((group) => (
          <li className="nrt-outline-group" key={`group:${group.id}`}>
            <button
              type="button"
              className="btn"
              aria-expanded={!closedGroups.includes(group.id)}
              onClick={() =>
                setClosedGroups(
                  closedGroups.includes(group.id)
                    ? closedGroups.filter((id) => id !== group.id)
                    : [...closedGroups, group.id],
                )
              }
            >
              {closedGroups.includes(group.id) ? 'Open' : 'Close'} group {group.name}
            </button>
            <span>
              {scene.elements.filter((element) => element.groupId === group.id).length} elements
            </span>
          </li>
        ))}
        {rows
          .filter(({ element }) => !element.groupId || !closedGroups.includes(element.groupId))
          .map(({ element, reachable }) => {
            const status = element.status ? NARRATIVE_STATUS[element.status] : null
            const selected = element.id === selectedId
            const blocking = (element.diagnostics ?? []).some((found) => found.severity === 'error')
            const muted = isFlowElementMuted(element, blocking, participant, statuses)
            const diagnostics = (element.diagnostics ?? []).filter((found) =>
              badgeShown(found, badges),
            )
            return (
              <li
                key={element.id}
                className={`nrt-outline-row${selected ? ' is-selected' : ''}${muted ? ' is-muted' : ''}`}
              >
                <div className="nrt-outline-head">
                  <button
                    type="button"
                    className="nrt-outline-name"
                    data-flow-id={element.id}
                    aria-current={selected ? 'true' : undefined}
                    onClick={() => select(element.id)}
                  >
                    <Icon name={FLOW_KIND_ICON[element.kind]} size="sm" />
                    <span className="nrt-node-kind">{FLOW_KIND_LABEL[element.kind]}</span>
                    {element.title}
                  </button>
                  {element.kind === 'beat' && (
                    <span className="nrt-badge">
                      {element.lines} lines · {element.variants} variants
                    </span>
                  )}
                  {muted && <span className="nrt-badge">Filtered</span>}
                  {status && (
                    <span className="nrt-badge">
                      <Icon name={status.icon} size="sm" />
                      {status.label}
                    </span>
                  )}
                  {!reachable && (
                    <span className="nrt-badge is-conflict">
                      <Icon name="x" size="sm" />
                      Not reachable from the start
                    </span>
                  )}
                  {onActivate && (
                    /* The drill-down, as an ordinary button. Double-click is not
                   an operation a keyboard has, so the outline cannot leave
                   entering a scene to the canvas's gesture. */
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => onActivate(element.id)}
                    >
                      {activateLabel}
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={sourceDisabled}
                    /* Not `disabled` for a derived box: `remove` refuses it out
                   loud, in the same live region every other refusal uses, and a
                   dead button explains nothing. */
                    onClick={() => remove(element.id)}
                  >
                    Delete
                  </button>
                </div>

                {(element.out.length > 0 || spare?.(element)) && (
                  <ul className="nrt-outline-ports">
                    {portsOf(element, spare).map((port) => (
                      <li key={port.id}>
                        <label>
                          <span className="nrt-port-label">
                            {port.label}
                            {port.requires ? ` — requires ${port.requires}` : ''}
                          </span>
                          {/* The connection, as a control rather than a gesture.
                          Choosing "Nothing yet" is how a destination is
                          disconnected without a pointer or a canvas. */}
                          <select
                            value={port.to ?? ''}
                            /* A structural port is read here rather than edited: a
                           beat leads to its own choices because they belong to
                           it, so there is no destination on it to choose. */
                            disabled={sourceDisabled || (port.fixed && !!authoring?.fixedPort)}
                            aria-label={`${element.title} — ${port.label} leads to`}
                            onChange={(event) =>
                              connect(
                                { elementId: element.id, portId: port.id, label: port.label },
                                event.target.value || null,
                              )
                            }
                          >
                            {/* Left out when the source model has no "nowhere":
                            offering a choice that will be refused is worse than
                            not offering it, and the reason is on the row. The
                            unauthored row keeps it, because that *is* its
                            current value — nobody has asked for a way out yet. */}
                            {(!authoring?.destinationRequired || port.unauthored) && (
                              <option value="">Nothing yet</option>
                            )}
                            {destinations
                              .filter((candidate) => candidate.id !== element.id)
                              .map((candidate) => (
                                <option key={candidate.id} value={candidate.id}>
                                  {candidate.title}
                                </option>
                              ))}
                          </select>
                        </label>
                        {port.to === null && !port.unauthored && (
                          <span className="nrt-chip is-bad">
                            <Icon name="x" size="sm" />
                            no destination
                          </span>
                        )}
                      </li>
                    ))}
                  </ul>
                )}

                {diagnostics.length > 0 && (
                  /* The full wording, which a fixed-size box on the canvas cannot
                 carry. #151 requires the outline to be a complete alternative
                 to the canvas, and a badge that only counts problems is not. */
                  <ul
                    className="nrt-outline-diagnostics"
                    aria-label={`Problems with ${element.title}`}
                  >
                    {diagnostics.map((found) => (
                      <li key={found.id} className={found.severity === 'error' ? 'is-bad' : ''}>
                        <Icon name={found.severity === 'error' ? 'x' : 'clock'} size="sm" />
                        <span className="nrt-badge">
                          {found.severity === 'error' ? 'Error' : 'Warning'}
                        </span>
                        <span>{found.message}</span>
                      </li>
                    ))}
                  </ul>
                )}

                {selected && (
                  <div
                    className="nrt-outline-add"
                    role="group"
                    aria-label={`Add after ${element.title}`}
                  >
                    {creatable.map((kind) => (
                      <button
                        key={kind}
                        type="button"
                        className="btn btn-sm"
                        disabled={sourceDisabled}
                        onClick={() => add(kind, element.id)}
                      >
                        Add {FLOW_KIND_LABEL[kind].toLocaleLowerCase()} after
                      </button>
                    ))}
                  </div>
                )}
              </li>
            )
          })}
      </ol>
    </>
  )
}

/**
 * The rows of ways out, with the unauthored one last.
 *
 * `unauthored` is what stops the spare wearing a "no destination" chip: it has
 * no destination because nobody has asked for one yet, which is not the same
 * problem as a route a writer drew and left hanging.
 */
function portsOf(
  element: FlowElement,
  spare?: (element: FlowElement) => FlowPort | null,
): (FlowPort & { unauthored?: boolean })[] {
  const extra = spare?.(element) ?? null
  return extra ? [...element.out, { ...extra, unauthored: true }] : element.out
}
