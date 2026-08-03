import { useState } from 'react'
import type { DescriptionState, KindDef, WobuNode } from '../../lib/api'
import { saveLabel, type useAutosaveNode } from '../../hooks/useAutosaveNode'
import { AttributesEditor } from './AttributesEditor'
import { DescriptionEditor } from './DescriptionEditor'
import { EnhanceReview } from './EnhanceReview'
import type { EnhanceSession } from './useEnhanceSession'

const STATE_LABEL: Record<DescriptionState, string> = {
  none: 'not generated',
  enhancing: 'enhancing…',
  fresh: 'fresh',
  edited: 'edited by you',
  stale: 'stale',
}

export function NotesPane({
  node,
  def,
  readOnly,
  autosave,
  enhance,
}: {
  node: WobuNode
  def: KindDef | undefined
  readOnly: boolean
  autosave: ReturnType<typeof useAutosaveNode>
  enhance?: EnhanceSession
}) {
  return (
    <NotesPaneSession
      key={node.id}
      node={node}
      def={def}
      readOnly={readOnly}
      autosave={autosave}
      enhance={enhance}
    />
  )
}

function NotesPaneSession({
  node,
  def,
  readOnly,
  autosave,
  enhance,
}: {
  node: WobuNode
  def: KindDef | undefined
  readOnly: boolean
  autosave: ReturnType<typeof useAutosaveNode>
  enhance?: EnhanceSession
}) {
  const [descriptionEdit, setDescriptionEdit] = useState<{
    description: WobuNode['description']
    state: DescriptionState
  } | null>(null)
  const localDescriptionApplies =
    descriptionEdit?.description === node.description &&
    descriptionEdit.state === node.descriptionState
  const shownDescriptionState = localDescriptionApplies ? 'edited' : node.descriptionState

  return (
    <div className="split">
      <div className="col">
        <div className="col-head">
          <h2>Raw notes</h2>
          <span className="col-tag">yours</span>
          <span className="col-tag-save">{saveLabel(autosave.status)}</span>
        </div>
        <NotesField node={node} readOnly={readOnly} autosave={autosave} />
        <AttributesEditor
          node={node}
          definitions={def?.attributes ?? []}
          readOnly={readOnly}
          autosave={autosave}
        />
      </div>

      <div className="col col-ai">
        <div className="col-head">
          <h2>{enhance?.active ? 'Enhance review' : 'Enhanced description'}</h2>
          <span className="col-tag col-tag-ai">machine side</span>
          <span className="col-tag">
            {enhance?.active
              ? enhance.complete
                ? 'awaiting decision'
                : enhance.running
                  ? 'streaming'
                  : 'local draft'
              : STATE_LABEL[shownDescriptionState]}
          </span>
        </div>
        <div className="desc">
          {enhance?.active ? (
            <EnhanceReview
              current={node.description}
              definitions={def?.sections ?? []}
              session={enhance}
            />
          ) : (
            <Description
              node={node}
              def={def}
              readOnly={readOnly}
              autosave={autosave}
              onEdit={() =>
                setDescriptionEdit({
                  description: node.description,
                  state: node.descriptionState,
                })
              }
            />
          )}
        </div>
      </div>
    </div>
  )
}

function NotesField({
  node,
  readOnly,
  autosave,
}: {
  node: WobuNode
  readOnly: boolean
  autosave: ReturnType<typeof useAutosaveNode>
}) {
  const [draft, setDraft] = useState<{
    source: string
    value: string
    editing: boolean
  } | null>(null)
  const text =
    draft && (draft.editing || draft.source === node.notesRaw) ? draft.value : node.notesRaw

  return (
    <textarea
      className="notes"
      value={text}
      readOnly={readOnly}
      spellCheck={false}
      placeholder={readOnly ? 'This project folder is read-only.' : 'Write your notes here…'}
      onChange={(e) => {
        setDraft({ source: node.notesRaw, value: e.target.value, editing: true })
        autosave.queue({ notesRaw: e.target.value })
      }}
      onBlur={() => {
        setDraft((current) =>
          current ? { ...current, source: node.notesRaw, editing: false } : current,
        )
        autosave.flush()
      }}
    />
  )
}

function Description({
  node,
  def,
  readOnly,
  autosave,
  onEdit,
}: {
  node: WobuNode
  def: KindDef | undefined
  readOnly: boolean
  autosave: ReturnType<typeof useAutosaveNode>
  onEdit: () => void
}) {
  const sections = node.description?.sections
  if (!sections || Object.keys(sections).length === 0) {
    return (
      <div className="desc-empty">
        <b>Nothing enhanced yet.</b>
        <span>
          This half is the machine&apos;s. Enhance turns the notes on the left into structured
          sections{def ? ` — ${def.sections.map((s) => s.label).join(' · ')}` : ''} — which is what
          the influence compiler reads.
        </span>
      </div>
    )
  }

  return (
    <DescriptionEditor
      node={node}
      definitions={def?.sections ?? []}
      readOnly={readOnly}
      autosave={autosave}
      onEdit={onEdit}
    />
  )
}
