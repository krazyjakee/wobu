import type { NodeSummary } from '../lib/api'
import { colorFor, labelFor, type KindIndex } from '../lib/kinds'
import { Icon } from './Icon'

/**
 * The Influence Stack shell. Resolution, weights, muting and the compiled
 * prompt are `wobu-influence`'s job and arrive with M5 — so this panel shows
 * its own shape and says so, rather than inventing layers.
 */
export function Inspector({ selected, kinds }: { selected: NodeSummary | null; kinds: KindIndex }) {
  const def = selected ? kinds.get(selected.kind) : undefined
  const color = selected ? colorFor(def, selected.kind) : 'var(--border-str)'

  return (
    <aside className="insp">
      <div className="insp-head">
        <h2>Influence stack</h2>
        <span className="hint">this generation only</span>
      </div>

      <div className="stack">
        {selected ? (
          <div className="layer" style={{ ['--lc' as string]: color }}>
            <div className="layer-h">
              <span className="txt">
                <span className="layer-l">{labelFor(def, selected.kind)}</span>
                <span className="layer-n">{selected.name}</span>
              </span>
            </div>
          </div>
        ) : (
          <div className="insp-empty">
            <b>No node selected.</b>
            <span>The stack is resolved per node, outermost layer first.</span>
          </div>
        )}

        <div className="insp-empty">
          <Icon name="layers" size="xl" />
          <b>Upstream layers arrive in M5.</b>
          <span>
            Art Style, World Canon, species, culture and place resolve into an ordered stack, each
            card carrying a weight slider, a mute toggle and the exact fragments it contributes.
            None of that is computed yet, so none of it is shown.
          </span>
          <span className="milestone">M5 — Influence Engine + first images</span>
        </div>
      </div>

      <div className="insp-foot">
        <b>Compiled prompt</b> and the shot controls sit here, tinted per layer so attribution is
        visible at a glance. Muting or reweighting affects one generation only — it never edits the
        world.
      </div>
    </aside>
  )
}
