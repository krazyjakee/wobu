import type { ReactNode } from 'react'
import { Icon } from '../../Icon'
import { useFlowLevel } from './flowStore'
import {
  PreviewOverlayContext,
  ROUTE_MARKS,
  usePreviewOverlay,
  usePreviewRouteFocus,
  type PreviewOverlayValue,
  type RouteMark,
  type RouteNode,
} from './overlay'
import './overlay.css'

/**
 * The Preview route, drawn over the Flow canvas.
 *
 * ── why a context and not a prop ─────────────────────────────────────────────
 *
 * The overlay has to reach every node face, and the node faces are given their
 * `FlowGraphNode` once, in React Flow's `data`. Pushing a mark through that
 * object would rebuild all three hundred node objects on every Preview step —
 * the exact cost `flowStore` exists to avoid for selection, measured at 83 ms a
 * click on 1,000 nodes. So a node *subscribes* here, the same way it subscribes
 * to the selection, and the `nodes` array is untouched by a step.
 *
 * It is a React context rather than a store because, unlike selection, an
 * overlay is not the canvas's own state: it is a reading of a Preview session
 * that lives in `previewStore`. A store here would be a second copy of the
 * route, free to disagree with the trace it came from. The context carries a
 * value derived on render and nothing else — there is no setter.
 *
 * ── what it is allowed to touch ──────────────────────────────────────────────
 *
 * Nothing. No component below writes a scene, a position, a layout sidecar or
 * a project file, and none of them is handed a callback that could: the
 * provider takes a `FlowScene` and returns marks. That is the guarantee #188
 * asks for — "the overlay never modifies layout, source or canon" — held by
 * construction rather than by care.
 */

export function PreviewOverlayProvider({
  value,
  children,
}: {
  value: PreviewOverlayValue | null
  children: ReactNode
}) {
  return <PreviewOverlayContext.Provider value={value}>{children}</PreviewOverlayContext.Provider>
}

/* ── on the node face ─────────────────────────────────────────────────────── */

/**
 * The cap a marked node wears, and the button back to the transcript.
 *
 * A word, a glyph and — on the played route — a number, so the mark survives
 * being read in greyscale and the *order* survives a layout that put the third
 * beat above the second. The button is the cap itself: caps do not take pointer
 * events, so the one thing on a node that has somewhere to go has to be the
 * thing a pointer can reach.
 */
export function RouteCap({ node, onOpen }: { node: RouteNode; onOpen?: () => void }) {
  const mark = ROUTE_MARKS[node.mark]
  const order = node.order === null ? '' : ` ${node.order}`
  const label = `${mark.label}${order}${node.visits > 1 ? ` · ${node.visits} times` : ''}`
  if (!onOpen) {
    return (
      <span className={`nrt-node-cap nrt-route-cap is-route-${node.mark}`} title={mark.means}>
        <Icon name={mark.icon} size="sm" />
        {label}
      </span>
    )
  }
  return (
    <button
      type="button"
      className={`nrt-node-cap nrt-route-cap is-route-${node.mark}`}
      title={`${mark.means} Opens the Preview transcript at this step.`}
      onClick={(event) => {
        // The click is the cap's, not the canvas's: letting it through would
        // also move the canvas cursor, and a designer reading the route would
        // find their authoring selection had moved with it.
        event.stopPropagation()
        onOpen()
      }}
    >
      <Icon name={mark.icon} size="sm" />
      {label}
    </button>
  )
}

/**
 * The cap for a node, wired to the shared cursor. Rendered by every node face.
 *
 * Returns null unmarked, which is every node in a project with no Preview run —
 * so the ordinary authoring canvas is unchanged, down to the DOM.
 */
export function RouteMarkCap({ nodeId }: { nodeId: string }) {
  const value = usePreviewOverlay()
  const node = value?.overlay.nodes.get(nodeId)
  if (!value || !node) return null
  return (
    <RouteCap
      node={node}
      onOpen={
        value.overlay.origin === 'arc'
          ? undefined
          : () => usePreviewRouteFocus.getState().showStep(value.key, node.step)
      }
    />
  )
}

/* ── the legend ───────────────────────────────────────────────────────────── */

/**
 * What the marks mean, for the marks that are actually on this canvas.
 *
 * Only the ones in play: a legend row for a state no box is wearing is a
 * control answering a question nobody asked, and it is also a claim — "there
 * are unavailable branches here" — that would be false.
 */
export function RouteLegend() {
  const value = usePreviewOverlay()
  if (!value) return null
  const present = (['played', 'current', 'open', 'blocked'] as RouteMark[]).filter((mark) =>
    [...value.overlay.nodes.values()].some((node) => node.mark === mark),
  )
  if (present.length === 0) return null
  return (
    <ul className="nrt-route-legend" aria-label="Preview route legend">
      {present.map((mark) => (
        <li key={mark} className={`nrt-badge is-route-${mark}`} title={ROUTE_MARKS[mark].means}>
          <Icon name={ROUTE_MARKS[mark].icon} size="sm" />
          {ROUTE_MARKS[mark].label}
        </li>
      ))}
    </ul>
  )
}

/* ── what the overlay is, and what it cannot say ──────────────────────────── */

/**
 * The banner above the canvas: which run, which build, and what changed since.
 *
 * `role="status"` and never `role="alert"`. An overlay is information about a
 * run; nothing about it puts a writer's words at risk, and drift is a fact to
 * know rather than a problem to answer.
 */
export function RouteBanner({ onClose }: { onClose?: () => void }) {
  const value = usePreviewOverlay()
  if (!value) return null
  const { overlay, drift } = value
  return (
    <div className="nrt-route-note">
      <p className="nrt-note" role="status">
        <Icon name="spark" size="sm" />
        {overlay.origin === 'arc'
          ? `${overlay.nodes.size} scene${
              overlay.nodes.size === 1 ? ' has' : 's have'
            } been played in Preview during this session, and are badged below.`
          : overlay.origin === 'scenario'
            ? `Showing the saved scenario “${overlay.name}” as an overlay. It is not being replayed, and nothing about the scene is changed by drawing it.`
            : `Showing the route Preview played, over ${overlay.route.length} element${
                overlay.route.length === 1 ? '' : 's'
              }. Pinned to build ${(overlay.build ?? '').slice(0, 12) || 'unknown'}.`}
        {onClose && (
          <button type="button" className="btn btn-sm" onClick={onClose}>
            Clear overlay
          </button>
        )}
      </p>
      {drift.length > 0 && (
        <p className="nrt-note" role="status">
          <Icon name="clock" size="sm" />
          {drift.join(' ')}
        </p>
      )}
      <ul className="nrt-route-limits" aria-label="What this overlay does not show">
        {overlay.limits.map((limit) => (
          <li key={limit}>
            <Icon name="minus" size="sm" />
            <span>{limit}</span>
          </li>
        ))}
      </ul>
    </div>
  )
}

/* ── why a branch was closed ──────────────────────────────────────────────── */

/**
 * The selected node's route detail: the gate that closed it, or what it wrote.
 *
 * The failed condition is named as the writer wrote it and the values are the
 * ones the runtime actually read — `has_logbook = false`, at
 * `choice:01J….requires`. Nothing here names a compiled artefact, an
 * expression index or an internal path: those exist in the trace and are the
 * runtime's business, and a designer asked to fix "records[3].path [0,1]" has
 * been handed a bug report about Wobu rather than about their scene.
 */
export function RouteDetail() {
  const value = usePreviewOverlay()
  const selectedId = useFlowLevel((state) => state.selectedId)
  const node = selectedId === null ? undefined : value?.overlay.nodes.get(selectedId)
  if (!value || !node) return null
  const block = node.block
  return (
    <div className="nrt-route-block">
      <p>
        <span className={`nrt-badge is-route-${node.mark}`}>
          <Icon name={ROUTE_MARKS[node.mark].icon} size="sm" />
          {ROUTE_MARKS[node.mark].label}
        </span>{' '}
        {ROUTE_MARKS[node.mark].means}
      </p>
      {block && !block.explained && (
        <p>
          This came from a saved scenario, which records what was offered and not why. Start the
          preview from the scenario to see the values.
        </p>
      )}
      {block && block.explained && (
        <>
          <p>
            <code>{block.field}</code> did not hold
            {block.failed ? (
              <>
                {' '}
                at <code>{block.failed}</code>
              </>
            ) : null}
            {block.gate && block.gate !== block.failed ? (
              <>
                , within <code>{block.gate}</code>
              </>
            ) : null}
            .
          </p>
          {block.values.length > 0 && (
            <table aria-label="Values when this branch was evaluated">
              <thead>
                <tr>
                  <th>Variable</th>
                  <th>Value</th>
                </tr>
              </thead>
              <tbody>
                {block.values.map((value) => (
                  <tr key={value.name}>
                    <td>
                      <code>{value.name}</code>
                    </td>
                    <td>{value.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
      {node.effects?.length ? (
        <ul className="nrt-route-effects" aria-label="What the route applied here">
          {node.effects.map((effect, index) => (
            <li key={index}>
              <code>{effect.wrote}</code>
              {effect.values.map((value) => (
                <span key={value.name}>
                  {' '}
                  {value.name}: {value.before} → {value.after}
                </span>
              ))}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}
