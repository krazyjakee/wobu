import type { ReactNode } from 'react'
import type { Beat, DialogueSlot, Speaker, VariableDecl } from '../../lib/api'
import { ScriptOrderControls } from './ScriptOrderControls'
import { TypedCondition } from './TypedCondition'
import { mintId } from './flow/source'
import { speakerFromKey, speakerKey } from './scriptModel'

export function ScriptDialogue({
  beat,
  disabled,
  selectedLineId,
  onSelectSlot,
  changeBeat,
  speakerOptions,
  variables,
  onDeleteVariant,
  onDeleteSlot,
  reviewControls,
}: {
  variables: VariableDecl[]
  reviewControls?: (slotId: string, variantId: string | null) => ReactNode
  onDeleteVariant: (slotId: string, variantId: string) => void
  onDeleteSlot: (slotId: string) => void
  beat: Beat
  disabled: boolean
  selectedLineId: string | null
  onSelectSlot: (id: string) => void
  changeBeat: (beat: Beat) => void
  speakerOptions: (speaker: Speaker) => ReactNode
}) {
  const changeSlot = (next: DialogueSlot) =>
    changeBeat({
      ...beat,
      dialogue: beat.dialogue?.map((slot) => (slot.id === next.id ? next : slot)),
    })
  return (
    <fieldset disabled={disabled}>
      <legend>Dialogue</legend>
      {(beat.dialogue ?? []).map((slot, slotIndex) => {
        const locked =
          slot.policy === 'locked' ||
          slot.variants?.some((v) => v.text.lifecycle?.policy === 'locked')
        return (
          <div
            key={slot.id}
            data-slot-id={slot.id}
            className={`nrt-script-line${selectedLineId === slot.id ? ' is-selected' : ''}`}
            onFocus={() => onSelectSlot(slot.id)}
          >
            <label>
              Speaker {slotIndex + 1}
              <select
                disabled={locked}
                data-narrative-field={`slot:${slot.id}`}
                value={speakerKey(slot.speaker)}
                onChange={(e) => changeSlot({ ...slot, speaker: speakerFromKey(e.target.value) })}
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
            {(slot.variants ?? []).map((variant, index) => (
              <div key={variant.id}>
                <label>
                  Dialogue {slotIndex + 1}, variant {index + 1}
                  <span className="nrt-script-status">
                    {variant.when && variant.when !== 'always'
                      ? 'Conditional variant'
                      : 'Unconditional variant'}{' '}
                  </span>
                  <textarea
                    disabled={locked}
                    data-narrative-field={`variant:${variant.id}`}
                    data-variant-id={variant.id}
                    value={variant.text.body}
                    onChange={(e) =>
                      changeSlot({
                        ...slot,
                        variants: slot.variants?.map((v) =>
                          v.id === variant.id
                            ? {
                                ...v,
                                text: {
                                  ...v.text,
                                  body: e.target.value,
                                  lifecycle: {
                                    ...v.text.lifecycle,
                                    policy: 'edited',
                                    review: 'draft',
                                  },
                                },
                              }
                            : v,
                        ),
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
                    onChange={(when) =>
                      changeSlot({
                        ...slot,
                        variants: slot.variants?.map((one) =>
                          one.id === variant.id
                            ? {
                                ...one,
                                when,
                                text: {
                                  ...one.text,
                                  lifecycle: { ...one.text.lifecycle, review: 'draft' },
                                },
                              }
                            : one,
                        ),
                      })
                    }
                  />
                  <ScriptOrderControls
                    label={`dialogue ${slotIndex + 1} variant ${index + 1}`}
                    index={index}
                    items={slot.variants ?? []}
                    onChange={(variants) => changeSlot({ ...slot, variants })}
                  />
                  <button className="btn" onClick={() => onDeleteVariant(slot.id, variant.id)}>
                    Delete dialogue {slotIndex + 1} variant {index + 1}
                  </button>
                </fieldset>
              </div>
            ))}
            <button
              className="btn"
              disabled={locked}
              onClick={() =>
                changeSlot({
                  ...slot,
                  variants: [
                    ...(slot.variants ?? []),
                    {
                      id: mintId(),
                      text: {
                        revision: '',
                        body: '',
                        provenance: 'human',
                        lifecycle: {
                          review: 'draft',
                          freshness: 'current',
                        },
                      },
                    },
                  ],
                })
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
              onChange={(dialogue) => changeBeat({ ...beat, dialogue })}
            />
            <button className="btn" disabled={locked} onClick={() => onDeleteSlot(slot.id)}>
              Delete dialogue {slotIndex + 1} slot
            </button>
          </div>
        )
      })}
      <button
        className="btn"
        onClick={() => {
          const slot: DialogueSlot = { id: mintId(), speaker: 'narrator', policy: 'edited' }
          changeBeat({ ...beat, dialogue: [...(beat.dialogue ?? []), slot] })
          onSelectSlot(slot.id)
        }}
      >
        Add dialogue slot
      </button>
    </fieldset>
  )
}
