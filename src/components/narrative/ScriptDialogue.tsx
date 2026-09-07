import type { ReactNode } from 'react'
import type { Beat, DialogueSlot, Speaker } from '../../lib/api'
import { mintId } from './flow/source'
import { speakerFromKey, speakerKey } from './scriptModel'

export function ScriptDialogue({
  beat,
  disabled,
  selectedLineId,
  onSelectSlot,
  changeBeat,
  speakerOptions,
}: {
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
            {locked ? (
              <button
                className="btn"
                onClick={() =>
                  changeSlot({
                    ...slot,
                    policy: 'edited',
                    variants: slot.variants?.map((v) => ({
                      ...v,
                      text: {
                        ...v.text,
                        lifecycle: { ...v.text.lifecycle, policy: 'edited' },
                      },
                    })),
                  })
                }
              >
                Unlock dialogue {slotIndex + 1}
              </button>
            ) : (
              <button
                className="btn"
                onClick={() =>
                  changeSlot({
                    ...slot,
                    policy: 'locked',
                    variants: slot.variants?.map((v) => ({
                      ...v,
                      text: {
                        ...v.text,
                        lifecycle: { ...v.text.lifecycle, policy: 'locked' },
                      },
                    })),
                  })
                }
              >
                Lock dialogue {slotIndex + 1}
              </button>
            )}
            {!slot.variants?.length && (
              <p className="nrt-note">Missing text — this slot is intentionally empty.</p>
            )}
            {(slot.variants ?? []).map((variant, index) => (
              <label key={variant.id}>
                Dialogue {slotIndex + 1}, variant {index + 1}
                <span className="nrt-script-status">
                  {variant.when && variant.when !== 'always'
                    ? 'Conditional variant'
                    : 'Unconditional variant'}{' '}
                  · {variant.text.lifecycle?.review ?? 'Draft'} ·{' '}
                  {variant.text.lifecycle?.freshness ?? 'current'}
                </span>
                <textarea
                  disabled={locked}
                  data-variant-id={variant.id}
                  value={variant.text.body}
                  onChange={(e) =>
                    changeSlot({
                      ...slot,
                      policy: 'edited',
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
            ))}
            {!slot.variants?.length && (
              <button
                className="btn"
                disabled={locked}
                onClick={() =>
                  changeSlot({
                    ...slot,
                    policy: 'edited',
                    variants: [
                      {
                        id: mintId(),
                        text: {
                          revision: '',
                          body: '',
                          provenance: 'human',
                          lifecycle: {
                            policy: 'edited',
                            review: 'draft',
                            freshness: 'current',
                          },
                        },
                      },
                    ],
                  })
                }
              >
                Write dialogue {slotIndex + 1}
              </button>
            )}
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
