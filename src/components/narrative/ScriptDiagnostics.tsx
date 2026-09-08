import { useState } from 'react'
import type { NarrativeDiagnostic, Scene } from '../../lib/api'
import { useDiagnoseScene } from '../../lib/queries'

export function ScriptDiagnostics({
  scene,
  onSelect,
}: {
  scene: Scene
  onSelect: (diagnostic: NarrativeDiagnostic) => void
}) {
  const diagnose = useDiagnoseScene()
  const [checked, setChecked] = useState<Scene | null>(null)
  const current = checked === scene
  return (
    <div className="nrt-script-diagnostics">
      <button
        className="btn"
        disabled={diagnose.isPending}
        onClick={() => {
          setChecked(scene)
          diagnose.mutate({ sceneId: scene.id, scene })
        }}
      >
        {diagnose.isPending ? 'Checking script…' : 'Check script'}
      </button>
      {diagnose.isError && <p role="alert">Could not check script: {String(diagnose.error)}</p>}
      {checked && !current && (
        <p className="nrt-note">Script changed. Check again for current problems.</p>
      )}
      {current && diagnose.isSuccess && (
        <>
          <p role="status">
            {diagnose.data.length
              ? `${diagnose.data.length} script problems`
              : 'No script problems found.'}
          </p>
          <ul aria-label="Script problems">
            {diagnose.data.map((diagnostic, index) => (
              <li key={index}>
                <button className="btn" onClick={() => onSelect(diagnostic)}>
                  {diagnostic.message}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  )
}
