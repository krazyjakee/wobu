import { useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { errorMessage } from '../../lib/api'
import { setNarrativeDraftGuard } from '../../lib/narrativeDraftGuard'
import {
  narrativeSourceCheck,
  narrativeSourceGet,
  type NarrativeSource,
  type SourceCheck,
} from '../../lib/api/narrativeSource'
import { useSaveScene } from '../../lib/queries'
import { useUI } from '../../store/ui'
import './narrativeSource.css'

interface Draft {
  base: NarrativeSource
  yaml: string
}

// Project-scoped drafts retain their original stamp across view changes.
const draftKey = (project: string, id: string) => ['narrative_source_draft', project, id]

export function NarrativeSourcePane({
  readOnly = false,
  projectKey = '',
}: {
  readOnly?: boolean
  projectKey?: string
}) {
  const sceneId = useUI((s) => s.narrative.sceneId)
  const source = useQuery({
    queryKey: ['narrative_source', projectKey, sceneId],
    queryFn: () => narrativeSourceGet(sceneId!),
    enabled: !!sceneId,
    retry: false,
    staleTime: 0,
  })
  if (!sceneId) return <p className="nrt-note">Choose a scene to edit its YAML source.</p>
  if (!source.data) {
    return (
      <p role="status">{source.error ? errorMessage(source.error) : 'Loading scene source…'}</p>
    )
  }
  return (
    <SourceEditor
      key={`${projectKey}:${sceneId}:${source.data.file.stamp?.hash ?? ''}`}
      initial={source.data}
      readOnly={readOnly}
      projectKey={projectKey}
    />
  )
}

function SourceEditor({
  initial,
  readOnly,
  projectKey,
}: {
  initial: NarrativeSource
  readOnly: boolean
  projectKey: string
}) {
  const qc = useQueryClient()
  const sceneId = initial.file.scene.id
  const [draft, setDraft] = useState<Draft>(
    () =>
      qc.getQueryData<Draft>(draftKey(projectKey, sceneId)) ?? {
        base: initial,
        yaml: initial.yaml,
      },
  )
  const [check, setCheck] = useState<SourceCheck | null>(null)
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)
  const [confirmReload, setConfirmReload] = useState(false)
  const editor = useRef<HTMLTextAreaElement>(null)
  const save = useSaveScene()
  const dirty = draft.yaml !== draft.base.yaml
  const guardKey = `${projectKey}:${sceneId}:source`
  useEffect(() => {
    setNarrativeDraftGuard(guardKey, dirty)
  }, [guardKey, dirty])
  const update = (next: Draft) => {
    setNarrativeDraftGuard(guardKey, next.yaml !== next.base.yaml)
    if (next.yaml === next.base.yaml) {
      qc.removeQueries({ queryKey: draftKey(projectKey, sceneId), exact: true })
    } else {
      qc.setQueryDefaults(draftKey(projectKey, sceneId), { gcTime: Infinity })
      qc.setQueryData(draftKey(projectKey, sceneId), next)
    }
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
      setCheck(result)
      if (!result.scene || result.formatted === null) return
      if (action === 'format') {
        const latest = qc.getQueryData<Draft>(draftKey(projectKey, sceneId))
        if (latest && latest !== draft) return
        update({ ...draft, yaml: result.formatted })
        setCheck(result)
        setMessage('Formatted draft. Save to update the scene.')
      } else if (action === 'save') {
        const file = await save.mutateAsync({ file: draft.base.file, scene: result.scene })
        const base = { file, yaml: result.formatted }
        qc.setQueryData(['narrative_source', projectKey, sceneId], base)
        // A tab can unmount and reopen while this write is in flight. A newer
        // buffer must keep its old stamp so the next save detects the race.
        const latest = qc.getQueryData<Draft>(draftKey(projectKey, sceneId))
        if (latest && latest !== draft) return
        update({ base, yaml: base.yaml })
        setCheck(result)
        setMessage('Scene saved. Flow and Script now use this source.')
      } else {
        setMessage(
          result.diagnostics.length
            ? 'Source parsed; review the diagnostics below.'
            : 'Source is valid.',
        )
      }
    } catch (error) {
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
      const base = await narrativeSourceGet(sceneId)
      const latest = qc.getQueryData<Draft>(draftKey(projectKey, sceneId))
      if (latest && latest !== draft) return
      update({ base, yaml: base.yaml })
      qc.setQueryData(['narrative_source', projectKey, sceneId], base)
      setMessage('Loaded the latest saved scene.')
    } catch (error) {
      setMessage(errorMessage(error))
    } finally {
      setBusy(false)
    }
  }

  const locateProblem = () => {
    const location = check?.problem?.location
    if (!location || !editor.current) return
    const lines = draft.yaml.split('\n')
    const offset =
      lines.slice(0, location.line - 1).reduce((n, line) => n + line.length + 1, 0) +
      location.column -
      1
    editor.current.focus()
    editor.current.setSelectionRange(offset, offset)
  }

  return (
    <section className="nrt-source" aria-label="Scene source editor">
      <div className="nrt-source-toolbar">
        <code>{draft.base.file.rel}</code>
        <span>{dirty ? 'Unsaved draft' : 'Saved source'}</span>
        <button className="btn" type="button" disabled={busy} onClick={() => void run('validate')}>
          Validate
        </button>
        <button
          className="btn"
          type="button"
          disabled={busy || readOnly}
          onClick={() => void run('format')}
        >
          Format
        </button>
        <button
          className="btn is-primary"
          type="button"
          disabled={busy || readOnly || !dirty}
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
        readOnly={readOnly || busy}
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
          {check.diagnostics.map((diagnostic, index) => (
            <li key={`${diagnostic.code}-${index}`}>
              {diagnostic.message}{' '}
              {diagnostic.beatId &&
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
          ))}
        </ul>
      )}
    </section>
  )
}
