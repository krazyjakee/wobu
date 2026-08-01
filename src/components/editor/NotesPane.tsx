import { useEffect, useRef, useState } from 'react'
import type { DescriptionState, KindDef, SectionValue, WobuNode } from '../../lib/api'
import { saveLabel, type useAutosaveNode } from '../../hooks/useAutosaveNode'
import { AttributesEditor } from './AttributesEditor'

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
}: {
  node: WobuNode
  def: KindDef | undefined
  readOnly: boolean
  autosave: ReturnType<typeof useAutosaveNode>
}) {
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
          <h2>Enhanced description</h2>
          <span className="col-tag col-tag-ai">{STATE_LABEL[node.descriptionState]}</span>
        </div>
        <div className="desc">
          <Description node={node} def={def} />
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
  const [text, setText] = useState(node.notesRaw)
  const typing = useRef(false)

  // A fresh node always wins; an incoming refetch only wins when we are idle,
  // so an external edit never eats what is being typed.
  useEffect(() => {
    typing.current = false
    setText(node.notesRaw)
  }, [node.id])
  useEffect(() => {
    if (!typing.current) setText(node.notesRaw)
  }, [node.notesRaw])

  return (
    <textarea
      className="notes"
      value={text}
      readOnly={readOnly}
      spellCheck={false}
      placeholder={
        readOnly
          ? 'This project folder is read-only.'
          : 'Messy is fine. This half is yours and nothing ever writes over it.'
      }
      onChange={(e) => {
        typing.current = true
        setText(e.target.value)
        autosave.queue({ notesRaw: e.target.value })
      }}
      onBlur={() => {
        typing.current = false
        autosave.flush()
      }}
    />
  )
}

function Description({ node, def }: { node: WobuNode; def: KindDef | undefined }) {
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
        <span className="milestone">M4 — Enhance (first BYOK providers)</span>
      </div>
    )
  }

  // Registry order first, then anything extra the file happens to carry.
  const ordered: { key: string; label: string; value: SectionValue }[] = []
  const seen = new Set<string>()
  for (const s of def?.sections ?? []) {
    const v = sections[s.key]
    if (v) {
      ordered.push({ key: s.key, label: s.label, value: v })
      seen.add(s.key)
    }
  }
  for (const [key, value] of Object.entries(sections)) {
    if (!seen.has(key)) ordered.push({ key, label: key.replace(/_/g, ' '), value })
  }

  return (
    <>
      {ordered.map((s) => (
        <section key={s.key} className={s.key === 'never' ? 'sec never' : 'sec'}>
          <h3>{s.label}</h3>
          {s.value.type === 'list' ? (
            <ul>
              {s.value.value.map((item, i) => (
                <li key={`${s.key}-${i}`}>{item}</li>
              ))}
            </ul>
          ) : (
            <p>{s.value.value}</p>
          )}
        </section>
      ))}
    </>
  )
}
