import { useEffect, useRef, useState } from 'react'
import type { ReviewTarget } from '../../lib/api/narrativeReview'
import type { DialogueSlot, Participant } from '../../lib/api'
import { useNarrativeState } from '../../lib/queries'
import { TypedCondition } from './TypedCondition'
import { speakerFromKey, speakerKey } from './scriptModel'
import { mintId } from './sceneIdentity'

export function TextLineFields({
  slot,
  target,
  label,
  cast,
  participants,
  disabled,
  nameOf,
  onChange,
}: {
  slot: DialogueSlot
  target?: ReviewTarget
  label: string
  cast: boolean
  participants: Participant[]
  disabled: boolean
  nameOf: (id: string) => string | undefined
  onChange: (slot: DialogueSlot) => void
}) {
  const schema = useNarrativeState()
  const [variantId, setVariantId] = useState(target?.variant ?? '')
  const [lastTarget, setLastTarget] = useState(target)
  if (lastTarget !== target) {
    setLastTarget(target)
    setVariantId(target?.variant ?? '')
  }
  const field = useRef<HTMLTextAreaElement>(null)
  useEffect(() => {
    if (!target) return
    const frame = requestAnimationFrame(() => {
      field.current?.scrollIntoView?.({ block: 'center' })
      field.current?.focus()
    })
    return () => cancelAnimationFrame(frame)
  }, [target, slot.variants?.length])
  const variant = slot.variants?.find((variant) => variant.id === variantId) ?? slot.variants?.[0]
  const locked =
    disabled || slot.policy === 'locked' || variant?.text.lifecycle?.policy === 'locked'
  const change = (patch: Partial<NonNullable<DialogueSlot['variants']>[number]>) => {
    onChange({
      ...slot,
      variants: slot.variants?.map((one) => (one.id === variant?.id ? { ...one, ...patch } : one)),
    })
  }
  return (
    <div className="ntl-line">
      <label>
        Speaker for {label}
        <select
          value={speakerKey(slot.speaker)}
          disabled={locked}
          onChange={(event) => onChange({ ...slot, speaker: speakerFromKey(event.target.value) })}
        >
          <option value="narrator">Narrator</option>
          <option value="player">Player</option>
          {cast &&
            participants.map((participant) => (
              <option key={participant.entity} value={participant.entity}>
                {nameOf(participant.entity) ?? participant.entity}
              </option>
            ))}
          {typeof slot.speaker === 'object' &&
            !participants.some(
              (participant) => participant.entity === (slot.speaker as { entity: string }).entity,
            ) && (
              <option value={slot.speaker.entity}>
                {nameOf(slot.speaker.entity) ?? slot.speaker.entity} (missing cast member)
              </option>
            )}
        </select>
      </label>
      <label>
        Wording for {label}
        <select value={variant?.id ?? ''} onChange={(event) => setVariantId(event.target.value)}>
          {(slot.variants ?? []).map((variant, index) => (
            <option key={variant.id} value={variant.id}>
              Variant {index + 1} · {variant.text.lifecycle?.review ?? 'draft'}
            </option>
          ))}
        </select>
      </label>
      {variant && (
        <>
          <textarea
            ref={field}
            rows={2}
            aria-label={label}
            value={variant.text.body}
            disabled={locked}
            onChange={(event) => change({ text: { ...variant.text, body: event.target.value } })}
          />
          <TypedCondition
            label={`${label} variant condition`}
            value={variant.when}
            variables={schema.data?.document.variables ?? []}
            disabled={locked}
            onChange={(when) => change({ when })}
          />
        </>
      )}
      <button
        className="btn"
        disabled={disabled || slot.policy === 'locked'}
        onClick={() => {
          const id = mintId()
          onChange({
            ...slot,
            variants: [...(slot.variants ?? []), { id, text: { body: '', revision: '' } }],
          })
          setVariantId(id)
        }}
      >
        Add wording variant
      </button>
    </div>
  )
}
