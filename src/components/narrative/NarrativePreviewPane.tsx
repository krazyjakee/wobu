import { useState } from 'react'
import { errorMessage, type VarType } from '../../lib/api'
import {
  narrativeCompile,
  narrativePreviewStart,
  narrativePreviewStep,
  type CompileDiagnostic,
  type PreviewAction,
  type PreviewFrame,
  type PreviewState,
} from '../../lib/api/narrativePreview'
import { useNarrativeState, useNodes } from '../../lib/queries'
import { useUI } from '../../store/ui'
import { TypedOperand } from './TypedOperand'
import { PreviewCommands } from './PreviewCommands'
import { PreviewCommandResult } from './PreviewCommandResult'
import { PreviewTrace } from './PreviewTrace'
import { usePreviewSessions } from './previewStore'
import { PreviewScenarios } from './PreviewScenarios'
import { nodeForSite, usePreviewRouteFocus } from './flow/overlay'
import { assertFrame, appendTape } from './scenarioTape'
import type { Scenario, ScenarioAction } from '../../lib/api/narrativeScenarios'
import './preview.css'

export function NarrativePreviewPane({
  projectKey = '',
  readOnly = false,
}: {
  projectKey?: string
  readOnly?: boolean
}) {
  const sceneId = useUI((state) => state.narrative.sceneId)
  if (!sceneId) return <p className="nrt-note">Choose a scene to preview.</p>
  return (
    <PreviewEditor
      key={`${projectKey}:${sceneId}`}
      projectKey={projectKey}
      sceneId={sceneId}
      readOnly={readOnly}
    />
  )
}

function PreviewEditor({
  projectKey,
  sceneId,
  readOnly,
}: {
  projectKey: string
  sceneId: string
  readOnly: boolean
}) {
  const key = `${projectKey}:${sceneId}`
  const session = usePreviewSessions((state) => state.sessions[key])
  const diagnostics = usePreviewSessions((state) => state.diagnostics[key])
  const declared = useNarrativeState()
  const nodes = useNodes(true)
  const variables = declared.data?.document.variables ?? []
  const [initial, setInitial] = useState<PreviewState>({})
  const [commands, setCommands] = useState<Record<string, VarType[]> | null>({})
  const busy = usePreviewSessions((state) => state.busy[key] ?? false)
  const setBusy = (value: boolean) => usePreviewSessions.getState().setBusy(key, value)
  const [error, setError] = useState('')
  const select = useUI((state) => state.selectNarrative)
  const openScript = (selection: { sceneId: string; beatId: string | null; lineId?: string }) => {
    select(selection, 'script')
    useUI.getState().setNarrativeTab('script')
  }
  const put = usePreviewSessions((state) => state.put)
  const opened = usePreviewSessions((state) => state.opened[key])
  /*
   * The Flow overlay's cursor, honoured on this side (#188).
   *
   * A click on a highlighted node latches a step here rather than calling into
   * this pane, because only one narrative tab is mounted at a time — so the
   * request has to survive the trip. Filtered by key so a run in another scene
   * cannot scroll this one.
   */
  const focus = usePreviewRouteFocus((state) => state.step)
  const focusStep = focus && focus.key === key ? focus.step : null
  const current = session?.frame.current
  const speaker = current && 'line' in current ? current.line.speaker : null
  const speakerName =
    speaker && typeof speaker === 'object'
      ? (nodes.data?.find((one) => one.id === speaker.entity)?.name ?? speaker.entity)
      : speaker
  const [loaded, setLoaded] = useState<Scenario | null>(null)
  const start = async (scenario?: Scenario) => {
    if (busy || (!commands && !scenario)) return
    setBusy(true)
    setError('')
    try {
      const frozenCommands = scenario?.commands ?? commands!
      const report = await narrativeCompile(frozenCommands)
      usePreviewSessions.getState().report(key, report.diagnostics)
      if (!report.graph) {
        const failure = new Error('Compilation needs corrections before this preview can start')
        if (scenario) throw failure
        setError(failure.message)
        return
      }
      const inputs =
        scenario?.initial_state ??
        Object.fromEntries(variables.map((one) => [one.name, initial[one.name] ?? one.default]))
      const startScene = scenario?.scene ?? sceneId
      const frame = await narrativePreviewStart(
        report.graph,
        startScene,
        inputs,
        scenario?.seed ?? 0,
      )
      put(key, {
        graph: report.graph,
        tape: {
          incomplete: false,
          scenario: {
            version: 1,
            scene: startScene,
            initial_state: structuredClone(inputs),
            seed: scenario?.seed ?? 0,
            commands: structuredClone(frozenCommands),
            steps: [assertFrame(frame, null)],
          },
        },
        frame,
        trace: [{ label: 'Started', execution: frame.trace }],
      })
    } catch (failure) {
      setError(errorMessage(failure))
      if (scenario) throw failure
    } finally {
      setBusy(false)
    }
  }
  const step = async (action: PreviewAction, label: string, restore?: PreviewFrame) => {
    if (!session || busy) return
    setBusy(true)
    setError('')
    try {
      const frame = await narrativePreviewStep(
        session.graph,
        restore?.snapshot ?? session.frame.snapshot,
        action,
      )
      put(key, {
        ...session,
        frame,
        trace: [...session.trace, { label, execution: frame.trace }].slice(-100),
        tape: appendTape(session.tape, frame, scenarioAction(action)),
      })
      if (frame.trace.error) setError(errorMessage(frame.trace.error))
    } catch (failure) {
      setError(errorMessage(failure))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="nrt-preview nrt-script-editor">
      <p className="nrt-note">
        Preview compiles saved scenes and declared state. Save Script, Source and World drafts
        before restarting. This session runs locally without generation.
      </p>
      <details open={!session}>
        <summary>Starting state</summary>
        <fieldset disabled={busy}>
          {variables.map((variable) => (
            <TypedOperand
              key={variable.name}
              label={`Initial ${variable.name}`}
              variable={variable}
              variables={[]}
              value={{ literal: initial[variable.name] ?? variable.default }}
              onChange={(value) => {
                if ('literal' in value) setInitial({ ...initial, [variable.name]: value.literal })
              }}
            />
          ))}
          {declared.isPending && <p>Loading state declarations…</p>}
          {declared.isError && (
            <p role="alert">Could not load state declarations: {String(declared.error)}</p>
          )}
        </fieldset>
      </details>
      <PreviewCommands
        key={loaded ? JSON.stringify(loaded.commands) : 'manual'}
        initial={loaded?.commands}
        disabled={busy}
        onChange={setCommands}
      />
      <PreviewScenarios
        sceneId={sceneId}
        tape={session?.tape}
        busy={busy}
        readOnly={readOnly}
        onLoad={async (scenario) => {
          setLoaded(scenario)
          setInitial(scenario.initial_state)
          setCommands(scenario.commands)
          await start(scenario)
        }}
        onOverlay={(name, scenario) => usePreviewSessions.getState().open(key, { name, scenario })}
      />
      {opened && (
        <p className="nrt-note" role="status">
          “{opened.name}” is drawn on the Flow canvas as an overlay. It is not being played, and
          nothing about the scene, its arrangement or the project is changed by it.{' '}
          <button className="btn" onClick={() => usePreviewSessions.getState().open(key, null)}>
            Close scenario overlay
          </button>
        </p>
      )}
      <div className="nrt-script-actions">
        <button
          className="btn is-primary"
          disabled={busy || !declared.data || !commands}
          onClick={() => void start()}
        >
          {busy ? 'Running…' : session ? 'Restart preview' : 'Start preview'}
        </button>
        <button
          className="btn"
          disabled={busy || !session}
          onClick={() =>
            session &&
            put(key, {
              ...session,
              bookmark: structuredClone(session.frame),
              tape: appendTape(session.tape, session.frame, { kind: 'save_checkpoint' }),
            })
          }
        >
          Save snapshot
        </button>
        <button
          className="btn"
          disabled={busy || !session?.bookmark}
          onClick={() => void step({ kind: 'restore' }, 'Restored snapshot', session?.bookmark)}
        >
          Restore snapshot
        </button>
      </div>
      {error && <p role="alert">Preview stopped: {error} The previous step is retained.</p>}
      {!!diagnostics?.length && (
        <ul aria-label="Compilation diagnostics">
          {diagnostics.map((diagnostic, index) => (
            <li key={index}>
              <button
                className="btn"
                onClick={() =>
                  openScript({ sceneId: diagnostic.scene, beatId: diagnosticBeat(diagnostic) })
                }
              >
                {diagnostic.severity}: {diagnostic.message}
              </button>
            </li>
          ))}
        </ul>
      )}
      {current && (
        <section aria-label="Current preview step" className="nrt-preview-stage">
          {'line' in current && (
            <>
              <h3>{speakerName}</h3>
              <p className="nrt-preview-line">{current.line.text}</p>
              <button
                className="btn"
                onClick={() =>
                  openScript({
                    sceneId: current.line.scene,
                    beatId: current.line.beat,
                    lineId: current.line.slot,
                  })
                }
              >
                Open this line in Script
              </button>
              <button
                className="btn is-primary"
                disabled={busy}
                onClick={() => void step({ kind: 'advance' }, `Read ${current.line.slot}`)}
              >
                Continue
              </button>
            </>
          )}
          {'choices' in current && (
            <>
              <h3>Choose a response</h3>
              {current.choices.choices.map((choice) => (
                <button
                  className="btn"
                  disabled={busy}
                  key={choice.id}
                  onClick={() =>
                    void step({ kind: 'choose', choiceId: choice.id }, `Chose ${choice.label}`)
                  }
                >
                  {choice.label}
                </button>
              ))}
            </>
          )}
          {'game_command' in current && session && (
            <PreviewCommandResult
              key={current.game_command.token}
              name={current.game_command.name}
              args={current.game_command.args}
              variables={Object.entries(session.graph.state ?? {})
                .filter(([, variable]) => variable.owner === 'host')
                .map(([name, variable]) => ({
                  name,
                  type: variable.ty,
                  default: variable.default,
                  owner: variable.owner,
                }))}
              state={session.frame.state}
              disabled={busy}
              onResult={(result) =>
                void step(
                  { kind: 'completeCommand', token: current.game_command.token, result },
                  `Host result: ${current.game_command.name}`,
                )
              }
            />
          )}
          {'end' in current && (
            <p role="status">Scene ended{current.end.label ? `: ${current.end.label}` : '.'}</p>
          )}
        </section>
      )}
      {session && (
        <>
          <table aria-label="Current preview state">
            <thead>
              <tr>
                <th>Variable</th>
                <th>Value</th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(session.frame.state).map(([name, value]) => (
                <tr key={name}>
                  <td>{name}</td>
                  <td>{String(value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <PreviewTrace
            entries={session.trace}
            focusStep={focusStep}
            openSource={(site) => {
              openScript({
                sceneId: site.scene,
                beatId: site.beat,
                ...(site.slot ? { lineId: site.slot } : {}),
              })
            }}
            onCentre={(site) => {
              const node = nodeForSite(site)
              if (!node) return
              usePreviewRouteFocus.getState().showNode(key, node)
              useUI.getState().setNarrativeTab('flow')
            }}
          />
          <p className="nrt-note">
            Snapshots and playback history are kept for this scene and project in the current app
            session. Restart compiles current saved source.
          </p>
        </>
      )}
    </div>
  )
}

function diagnosticBeat(diagnostic: CompileDiagnostic): string | null {
  const find = (value: unknown): string | null => {
    if (!value || typeof value !== 'object') return null
    if ('beat' in value && typeof value.beat === 'string') return value.beat
    for (const child of Object.values(value)) {
      const found = find(child)
      if (found) return found
    }
    return null
  }
  return find(diagnostic.site)
}

function scenarioAction(action: PreviewAction): ScenarioAction {
  switch (action.kind) {
    case 'advance':
      return { kind: 'advance' }
    case 'choose':
      return { kind: 'choose', choice: action.choiceId }
    case 'completeCommand':
      return { kind: 'complete_command', result: action.result }
    case 'restore':
      return { kind: 'restore_checkpoint' }
  }
}
