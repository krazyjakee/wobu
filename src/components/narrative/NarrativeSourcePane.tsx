import { useEffect, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { errorCode, errorMessage } from '../../lib/api'
import { setNarrativeDraftGuard } from '../../lib/narrativeDraftGuard'
import {
  narrativeSourceCheck,
  narrativeSourceGet,
  narrativeSourceOpen,
  narrativeSourceRepair,
  type NarrativeSource,
  type SourceCheck,
} from '../../lib/api/narrativeSource'
import { qk, invalidateNarrative } from '../../lib/queries/keys'
import { sourceRange, syntaxOffset, type SourceRange } from './sourceRanges'
import { useSaveScene } from '../../lib/queries'
import { useUI } from '../../store/ui'
import './narrativeSource.css'

let draftSerial = 0

interface Draft {
  revision: number
  base: NarrativeSource
  yaml: string
}

// Project-scoped drafts retain their original stamp across view changes.
const draftKey = (project: string, id: string) => ['narrative_source_draft', project, id]

export function NarrativeSourcePane({
  readOnly = false,
  projectKey = '',
  rel,
}: {
  rel?: string
  readOnly?: boolean
  projectKey?: string
}) {
  const sceneId = useUI((s) => s.narrative.sceneId)
  const source = useQuery({
    queryKey: ['narrative_source', projectKey, rel ?? sceneId],
    queryFn: () => (rel ? narrativeSourceOpen(rel) : narrativeSourceGet(sceneId!)),
    enabled: !!rel || !!sceneId,
    retry: false,
    staleTime: 0,
  })
  if (!sceneId && !rel) return <p className="nrt-note">Choose a scene to edit its YAML source.</p>
  if (!source.data) {
    return (
      <p role="status">{source.error ? errorMessage(source.error) : 'Loading scene source…'}</p>
    )
  }
  return (
    <SourceEditor
      key={`${projectKey}:${rel ?? sceneId}:${source.data.stamp.hash}`}
      target={rel ?? sceneId!}
      initial={source.data}
      readOnly={readOnly}
      projectKey={projectKey}
    />
  )
}

function SourceEditor({
  initial,
  target,
  readOnly,
  projectKey,
}: {
  target: string
  initial: NarrativeSource
  readOnly: boolean
  projectKey: string
}) {
  const qc = useQueryClient()
  const sceneId = initial.sceneId
  const sourceKey = ['narrative_source', projectKey, target]
  const [draft, setDraft] = useState<Draft>(
    () =>
      ((cached) => (cached?.yaml !== cached?.base.yaml ? cached : undefined))(
        qc.getQueryData<Draft>(draftKey(projectKey, target)),
      ) ?? {
        revision: ++draftSerial,
        base: initial,
        yaml: initial.yaml,
      },
  )
  const [check, setCheck] = useState<SourceCheck | null>(
    initial.problem
      ? { scene: null, formatted: null, problem: initial.problem, diagnostics: [] }
      : null,
  )
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)
  const [confirmReload, setConfirmReload] = useState(false)
  const editor = useRef<HTMLTextAreaElement>(null)
  const save = useSaveScene()
  const dirty = draft.yaml !== draft.base.yaml
  const guardKey = `${projectKey}:${target}:source`
  const latestDraft = useRef(draft)
  useEffect(() => {
    setNarrativeDraftGuard(guardKey, dirty)
    qc.setQueryData(draftKey(projectKey, target), draft)
  }, [guardKey, dirty, qc, projectKey, target, draft])
  const update = (value: Omit<Draft, 'revision'>) => {
    const next = { ...value, revision: ++draftSerial }
    setNarrativeDraftGuard(guardKey, next.yaml !== next.base.yaml)
    qc.setQueryDefaults(draftKey(projectKey, target), { gcTime: Infinity })
    qc.setQueryData(draftKey(projectKey, target), next)
    latestDraft.current = next
    setDraft(next)
    setCheck(null)
    setMessage('')
    setConfirmReload(false)
  }

  const run = async (action: 'validate' | 'format' | 'save') => {
    setBusy(true)
    setMessage('')
    try {
      const result = await narrativeSourceCheck(sceneId, draft.yaml)
      const stillCurrent = () =>
        latestDraft.current.revision === draft.revision &&
        qc.getQueryData<Draft>(draftKey(projectKey, target))?.revision === draft.revision
      if (!stillCurrent()) return
      setCheck(result)
      if (!result.scene || result.formatted === null) return
      if (action === 'format') {
        const latest = qc.getQueryData<Draft>(draftKey(projectKey, target))
        if (latest?.revision !== draft.revision || latestDraft.current.revision !== draft.revision)
          return
        update({ ...draft, yaml: result.formatted })
        setCheck(result)
        setMessage('Formatted draft. Save to update the scene.')
      } else if (action === 'save') {
        let base: NarrativeSource
        let recovery = ''
        if (draft.base.file) {
          const file = await save.mutateAsync({ file: draft.base.file, scene: result.scene })
          base = {
            file,
            yaml: result.formatted,
            rel: file.rel,
            stamp: file.stamp!,
            sceneId: file.scene.id,
            problem: null,
            repairBlocked: false,
          }
        } else {
          const repaired = await narrativeSourceRepair(draft.base, draft.yaml)
          base = repaired.source
          recovery = repaired.recoveryRel
          if (base.file) qc.setQueryData(qk.narrativeScene(base.file.scene.id), base.file)
          invalidateNarrative(qc)
        }
        qc.setQueryData(sourceKey, base)
        // A tab can unmount and reopen while this write is in flight. A newer
        // buffer must keep its old stamp so the next save detects the race.
        const latest = qc.getQueryData<Draft>(draftKey(projectKey, target))
        if (latest?.revision !== draft.revision || latestDraft.current.revision !== draft.revision)
          return
        update({ base, yaml: base.yaml })
        setCheck(result)
        setMessage(
          recovery
            ? `Repair saved. Original bytes retained at ${recovery}.`
            : 'Scene saved. Flow and Script now use this source.',
        )
      } else {
        setMessage(
          result.diagnostics.length
            ? 'Source parsed; review the diagnostics below.'
            : 'Source is valid.',
        )
      }
    } catch (error) {
      if (errorCode(error) === 'write.conflict') invalidateNarrative(qc)
      setMessage(errorMessage(error))
    } finally {
      setBusy(false)
    }
  }

  const reload = async () => {
    if (dirty && !confirmReload) {
      setConfirmReload(true)
      return
    }
    setBusy(true)
    try {
      const base = await narrativeSourceOpen(draft.base.rel)
      const latest = qc.getQueryData<Draft>(draftKey(projectKey, target))
      if (latest?.revision !== draft.revision || latestDraft.current.revision !== draft.revision)
        return
      update({ base, yaml: base.yaml })
      qc.setQueryData(sourceKey, base)
      setMessage('Loaded the latest saved scene.')
    } catch (error) {
      if (errorCode(error) === 'write.conflict') invalidateNarrative(qc)
      setMessage(errorMessage(error))
    } finally {
      setBusy(false)
    }
  }

  const locateProblem = () => {
    const location = check?.problem?.location
    if (!location || !editor.current) return
    const offset = syntaxOffset(draft.yaml, location.line, location.column)
    editor.current.focus()
    editor.current.setSelectionRange(offset, offset)
  }

  const locateSource = useMemo(
    () => (check?.diagnostics.length ? sourceRange(draft.yaml) : () => null),
    [check, draft.yaml],
  )

  const locateRange = (range: SourceRange) => {
    editor.current?.focus()
    editor.current?.setSelectionRange(range.start, range.end)
  }

  return (
    <section className="nrt-source" aria-label="Scene source editor">
      <div className="nrt-source-toolbar">
        <code>{draft.base.rel}</code>
        <span>{dirty ? 'Unsaved draft' : 'Saved source'}</span>
        <button className="btn" type="button" disabled={busy} onClick={() => void run('validate')}>
          Validate
        </button>
        <button
          className="btn"
          type="button"
          disabled={busy || readOnly || draft.base.repairBlocked}
          onClick={() => void run('format')}
        >
          Format
        </button>
        <button
          className="btn is-primary"
          type="button"
          disabled={busy || readOnly || draft.base.repairBlocked || !dirty}
          onClick={() => void run('save')}
        >
          Save source
        </button>
        <button className="btn" type="button" disabled={busy} onClick={() => void reload()}>
          Reload
        </button>
      </div>
      <p className="nrt-note" id="nrt-source-policy">
        Save and Format normalise YAML and remove comments. IDs and dialogue remain in the shared
        scene model. Unsaved drafts stay in this session when you change views. Layout is stored
        separately.
      </p>
      {!draft.base.file && !draft.base.repairBlocked && (
        <p role="alert">
          This saved file is malformed. Repair the source below; a successful repair keeps an exact
          recovery copy. Invalid drafts cannot replace it.
        </p>
      )}
      {draft.base.stamp.hash !== initial.stamp.hash && (
        <p role="alert">
          Source changed outside this draft. Saving checks the original stamp and keeps conflicts
          separately.
        </p>
      )}
      {draft.base.repairBlocked && (
        <p role="alert">
          This source schema is unsupported. Open it with a compatible Wobu; this editor will not
          downgrade it.
        </p>
      )}
      {readOnly && <p className="nrt-note">This project is read-only.</p>}
      {confirmReload && (
        <div className="nrt-source-reload" role="alert">
          Reloading discards this unsaved draft.
          <button className="btn" type="button" onClick={() => void reload()}>
            Discard draft and reload
          </button>
          <button className="btn" type="button" onClick={() => setConfirmReload(false)}>
            Keep editing
          </button>
        </div>
      )}
      <textarea
        ref={editor}
        aria-label="Scene YAML"
        aria-describedby="nrt-source-policy"
        spellCheck={false}
        readOnly={readOnly || busy || draft.base.repairBlocked}
        value={draft.yaml}
        onChange={(event) => update({ ...draft, yaml: event.target.value })}
      />
      {message && <p role="status">{message}</p>}
      {check?.problem && (
        <div role="alert">
          <p>{check.problem.message}</p>
          {check.problem.location && (
            <button className="btn" type="button" onClick={locateProblem}>
              Go to error
            </button>
          )}
        </div>
      )}
      {!!check?.diagnostics.length && (
        <ul aria-label="Source diagnostics">
          {check.diagnostics.map((diagnostic, index) => {
            const range = diagnostic.sourcePath ? locateSource(diagnostic.sourcePath) : null
            return (
              <li key={`${diagnostic.code}-${index}`}>
                {diagnostic.message}{' '}
                {range && (
                  <button className="btn" type="button" onClick={() => locateRange(range)}>
                    Source {range.line}:{range.column}
                    {range.alias ? ` (alias *${range.alias})` : ''}
                  </button>
                )}
                {sceneId &&
                  diagnostic.beatId &&
                  (['flow', 'script'] as const).map((tab) => (
                    <button
                      className="btn"
                      type="button"
                      key={tab}
                      onClick={() => {
                        useUI.getState().selectNarrative(
                          {
                            sceneId,
                            beatId: diagnostic.beatId!,
                            lineId: diagnostic.slotId ?? null,
                          },
                          'diagnostic',
                        )
                        useUI.getState().setNarrativeTab(tab)
                      }}
                    >
                      Open {tab === 'flow' ? 'Flow' : 'Script'}
                    </button>
                  ))}
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
