import type { ReactNode } from 'react'
import type { Beat, Speaker, VariableDecl } from '../../lib/api'
import type { SceneEditOperation } from './sceneEdits'
import { ScriptOrderControls } from './ScriptOrderControls'
import { TypedCondition } from './TypedCondition'
import { speakerFromKey, speakerKey } from './scriptModel'

export function ScriptDialogue({
  beat,
  disabled,
  selectedLineId,
  onSelectSlot,
  speakerOptions,
  variables,
  reviewControls,
  onEditOperation,
}: {
  variables: VariableDecl[]
  reviewControls?: (slotId: string, variantId: string | null) => ReactNode
  beat: Beat
  disabled: boolean
  selectedLineId: string | null
  onSelectSlot: (id: string, variantId?: string) => void
  speakerOptions: (speaker: Speaker) => ReactNode
  onEditOperation: (operation: SceneEditOperation) => void
}) {
  return (
    <fieldset disabled={disabled}>
      <legend>Dialogue</legend>
      {(beat.dialogue ?? []).map((slot, slotIndex) => {
        const locked =
          slot.policy === 'locked' ||
          slot.variants?.some((variant) => variant.text.lifecycle?.policy === 'locked')
        return (
          <div
            key={slot.id}
            data-slot-id={slot.id}
            className={`nrt-script-line${selectedLineId === slot.id ? ' is-selected' : ''}`}
            onFocus={(event) =>
              onSelectSlot(
                slot.id,
                (event.target as HTMLElement).closest<HTMLElement>('[data-variant-id]')?.dataset
                  .variantId,
              )
            }
          >
            <label>
              Speaker {slotIndex + 1}
              <select
                disabled={locked}
                data-narrative-field={`slot:${slot.id}`}
                value={speakerKey(slot.speaker)}
                onChange={(event) =>
                  onEditOperation({
                    kind: 'setSpeaker',
                    beatId: beat.id,
                    slotId: slot.id,
                    value: speakerFromKey(event.target.value),
                  })
                }
              >
                {speakerOptions(slot.speaker)}
              </select>
            </label>
            <span className="nrt-script-status">
              {locked ? 'Locked' : (slot.policy ?? 'edited')} · {slot.variants?.length ?? 0}{' '}
              variants
            </span>
            {reviewControls?.(slot.id, null)}
            {!slot.variants?.length && (
              <p className="nrt-note">Missing text — this slot is intentionally empty.</p>
            )}
            {(slot.variants ?? []).map((variant, index) => {
              const owner = {
                kind: 'variant' as const,
                beatId: beat.id,
                slotId: slot.id,
                id: variant.id,
              }
              return (
                <div key={variant.id} data-variant-id={variant.id}>
                  <label>
                    Dialogue {slotIndex + 1}, variant {index + 1}
                    <span className="nrt-script-status">
                      {variant.when && variant.when !== 'always'
                        ? 'Conditional variant'
                        : 'Unconditional variant'}
                    </span>
                    <textarea
                      disabled={locked}
                      data-narrative-field={`variant:${variant.id}`}
                      data-variant-id={variant.id}
                      value={variant.text.body}
                      onChange={(event) =>
                        onEditOperation({
                          kind: 'setVariantBody',
                          owner,
                          value: event.target.value,
                        })
                      }
                    />
                  </label>
                  {reviewControls?.(slot.id, variant.id)}
                  <fieldset disabled={locked}>
                    <TypedCondition
                      label={`Dialogue ${slotIndex + 1} variant ${index + 1}`}
                      value={variant.when}
                      variables={variables}
                      onChange={(value) => onEditOperation({ kind: 'setCondition', owner, value })}
                    />
                    <ScriptOrderControls
                      label={`dialogue ${slotIndex + 1} variant ${index + 1}`}
                      index={index}
                      items={slot.variants ?? []}
                      onChange={(next) =>
                        onEditOperation({
                          kind: 'moveVariant',
                          owner,
                          index: next.findIndex((one) => one.id === variant.id),
                        })
                      }
                    />
                    <button
                      className="btn"
                      onClick={() =>
                        onEditOperation({
                          kind: 'removeVariant',
                          beatId: beat.id,
                          slotId: slot.id,
                          variantId: variant.id,
                        })
                      }
                    >
                      Delete dialogue {slotIndex + 1} variant {index + 1}
                    </button>
                  </fieldset>
                </div>
              )
            })}
            <button
              className="btn"
              disabled={locked}
              onClick={() =>
                onEditOperation({ kind: 'addVariant', beatId: beat.id, slotId: slot.id })
              }
            >
              {slot.variants?.length
                ? `Add dialogue ${slotIndex + 1} variant`
                : `Write dialogue ${slotIndex + 1}`}
            </button>
            <ScriptOrderControls
              label={`dialogue ${slotIndex + 1} slot`}
              index={slotIndex}
              items={beat.dialogue ?? []}
              onChange={(next) =>
                onEditOperation({
                  kind: 'moveSlot',
                  beatId: beat.id,
                  slotId: slot.id,
                  index: next.findIndex((one) => one.id === slot.id),
                })
              }
            />
            <button
              className="btn"
              disabled={locked}
              onClick={() =>
                onEditOperation({ kind: 'removeSlot', beatId: beat.id, slotId: slot.id })
              }
            >
              Delete dialogue {slotIndex + 1} slot
            </button>
          </div>
        )
      })}
      <button
        className="btn"
        onClick={() => onEditOperation({ kind: 'addSlot', beatId: beat.id, speaker: 'narrator' })}
      >
        Add dialogue slot
      </button>
    </fieldset>
  )
}
