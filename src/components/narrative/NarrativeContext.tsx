import { useState } from 'react'
import { errorMessage } from '../../lib/api'
import type { DialogueSlot, StateValue } from '../../lib/api/narrative'
import {
  narrativeContextCapture,
  narrativeContextFreshness,
  type ContextSelection,
  type FrozenContext,
} from '../../lib/api/narrativeContext'
import { useNarrativeState } from '../../lib/queries'
import { hasUnsafeInteger, parseSafeInteger } from './integerInput'
import './context.css'
import { ContextFragmentView } from './ContextFragmentView'

/** A frozen request is kept intact when source queries refetch. Explicit recapture replaces it. */
export function NarrativeContext({
  selection,
  slot,
}: {
  selection: Omit<ContextSelection, 'variant'>
  slot: DialogueSlot
}) {
  const schema = useNarrativeState()
  const [variant, setVariant] = useState('')
  const [scenario, setScenario] = useState<string | null>(null)
  const [budget, setBudget] = useState('4000')
  const [snapshot, setSnapshot] = useState<FrozenContext | null>(null)
  const [fresh, setFresh] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const defaults = Object.fromEntries(
    (schema.data?.document.variables ?? []).map((v) => [v.name, v.default]),
  )
  const stateText = scenario ?? JSON.stringify(defaults, null, 2)
  const inspect = async () => {
    setError(null)
    try {
      const state: unknown = JSON.parse(stateText)
      const tokens = parseSafeInteger(budget)
      if (!state || Array.isArray(state) || typeof state !== 'object' || hasUnsafeInteger(state)) {
        throw new Error('Enter a state object using booleans, enum strings and safe integers.')
      }
      if (!tokens || tokens < 1 || tokens > 100000) {
        throw new Error('Choose an estimated token budget between 1 and 100000.')
      }
      setBusy(true)
      const result = await narrativeContextCapture(
        {
          selection: { ...selection, variant: variant || null },
          state: state as Record<string, StateValue>,
          token_budget: tokens,
        },
        stateText,
      )
      setSnapshot(result)
      setFresh(true)
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }
  const check = async () => {
    if (!snapshot) return
    setBusy(true)
    setError(null)
    try {
      const result = await narrativeContextFreshness(snapshot.options, snapshot.hash)
      setFresh(result.current)
    } catch (e) {
      setFresh(null)
      setError(errorMessage(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <section className="nrt-context nrt-resolved-context">
      <h3>Generation context</h3>
      <p className="nrt-note">
        Saved sources only. Knowledge belongs to this line’s speaker. Time is explicit scenario
        state; event attendance does not grant knowledge.
      </p>
      <label>
        Variant
        <select value={variant} onChange={(e) => setVariant(e.target.value)} disabled={busy}>
          <option value="">New wording for this slot</option>
          {slot.variants?.map((item) => (
            <option key={item.id} value={item.id}>
              Existing: {item.text.body.slice(0, 55) || item.id}
            </option>
          ))}
        </select>
      </label>
      <label>
        Scenario state (JSON)
        <textarea
          rows={5}
          value={stateText}
          onChange={(e) => setScenario(e.target.value)}
          disabled={busy}
          spellCheck={false}
        />
      </label>
      <label>
        Estimated input token budget
        <input value={budget} onChange={(e) => setBudget(e.target.value)} disabled={busy} />
      </label>
      <p className="nrt-note">
        Estimate: three characters per token. Required constraints are retained even when too large.
        No provider is called.
      </p>
      {schema.isError && <p role="alert">{errorMessage(schema.error)}</p>}
      {error && <p role="alert">{error}</p>}
      <button className="btn" disabled={busy || !schema.data} onClick={() => void inspect()}>
        {busy ? 'Inspecting…' : snapshot ? 'Capture again' : 'Inspect generation request'}
      </button>
      {snapshot && (
        <>
          <p role="status">
            {snapshot.ready ? 'Context ready for review' : 'Context blocked'} ·{' '}
            {snapshot.estimated_tokens} estimated tokens
          </p>
          <p className="nrt-note">
            {fresh === false
              ? 'Out of date: saved dependencies or query membership changed. This frozen request is retained for comparison.'
              : fresh === true
                ? 'Sources matched at the last check. Later edits do not change this frozen request.'
                : 'Source freshness could not be checked.'}
          </p>
          <button className="btn" disabled={busy} onClick={() => void check()}>
            Check source freshness
          </button>
          {snapshot.diagnostics.map((diagnostic, index) => (
            <p key={index} className="nrt-context-diagnostic">
              <strong>{diagnostic.blocking ? 'Blocker' : 'Notice'}:</strong> {diagnostic.message}
              <code>{diagnostic.source}</code>
            </p>
          ))}
          {snapshot.fragments.map((fragment, index) => (
            <ContextFragmentView
              key={index}
              fragment={fragment}
              nameOf={(id) => {
                const voice = snapshot.fragments.find(
                  (item) => item.kind === 'voice' && (item.data as { id: string }).id === id,
                )
                return voice ? (voice.data as { name: string }).name : id
              }}
            />
          ))}
          <details>
            <summary>Omitted context ({snapshot.omitted.length})</summary>
            <ul>
              {snapshot.omitted.map((source) => (
                <li key={source}>{source}</li>
              ))}
            </ul>
          </details>
          <details>
            <summary>Dependencies and query membership</summary>
            <pre>
              {JSON.stringify(
                { dependencies: snapshot.dependencies, queries: snapshot.queries },
                null,
                2,
              )}
            </pre>
          </details>
          <details>
            <summary>Exact generation input</summary>
            <pre>{snapshot.request}</pre>
          </details>
          <p className="nrt-note">
            Context hash: <code>{snapshot.hash}</code>
          </p>
          <p className="nrt-note">
            These checks cannot prove that generated prose respects the story.
          </p>
        </>
      )}
    </section>
  )
}
