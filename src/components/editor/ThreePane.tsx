import { lazy, Suspense, useCallback, useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { save } from '@tauri-apps/plugin-dialog'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import * as api from '../../lib/api'
import type { MeshConcept, QueueSnapshot, TurnaroundView, WobuNode } from '../../lib/api'
import { useAssetThumb, useMeshAssetPath, useMeshConcepts } from '../../lib/queries'
import { TurnaroundReview } from './TurnaroundReview'

const MeshViewport = lazy(() => import('./MeshViewport'))

const EMPTY_QUEUE: QueueSnapshot = { jobs: [], queued: 0, running: 0, retrying: 0 }

export function ThreePane({
  node,
  queue = EMPTY_QUEUE,
  readOnly = false,
}: {
  node: WobuNode
  queue?: QueueSnapshot
  readOnly?: boolean
}) {
  return <ThreePaneSession key={node.id} node={node} queue={queue} readOnly={readOnly} />
}

function ThreePaneSession({
  node,
  queue,
  readOnly,
}: {
  node: WobuNode
  queue: QueueSnapshot
  readOnly: boolean
}) {
  const concepts = useMeshConcepts(node.id)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const selected =
    concepts.data?.find((concept) => concept.asset.id === selectedId) ?? concepts.data?.[0] ?? null
  const path = useMeshAssetPath(selected?.asset.id ?? null)
  // Defaults on, except for someone who has asked the system for less motion.
  // This is the largest and most continuous movement in the app, and being WebGL
  // it is the one animation `prefers-reduced-motion` cannot reach from CSS. The
  // toggle still works either way; only the starting position changes.
  const [turntable, setTurntable] = useState(
    () =>
      typeof window.matchMedia !== 'function' ||
      !window.matchMedia('(prefers-reduced-motion: reduce)').matches,
  )
  const [wireframe, setWireframe] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [exporting, setExporting] = useState(false)
  const onViewerError = useCallback((message: string) => setError(message), [])

  const meshUrl = useMemo(() => (path.data ? convertFileSrc(path.data) : null), [path.data])

  async function reveal() {
    if (!selected) return
    try {
      const source = await api.meshSourcePath(selected.asset.id)
      if (!source) setError('The canonical GLB is unavailable.')
      else await revealItemInDir(source)
    } catch (reason) {
      setError(api.errorMessage(reason))
    }
  }

  async function exportMesh() {
    if (!selected) return
    setError(null)
    let destination: string | null
    try {
      destination = await save({
        defaultPath: `${safeFilename(node.name)}.glb`,
        filters: [{ name: 'Binary glTF', extensions: ['glb'] }],
      })
    } catch (reason) {
      setError(api.errorMessage(reason))
      return
    }
    if (!destination) return
    setExporting(true)
    try {
      await api.meshExport(selected.asset.id, destination)
    } catch (reason) {
      setError(api.errorMessage(reason))
    } finally {
      setExporting(false)
    }
  }

  if (concepts.isPending) return <div className="mesh-empty">Reading 3D history…</div>
  if (concepts.isError) {
    return (
      <div className="mesh-empty">
        Could not read 3D history: {api.errorMessage(concepts.error)}
      </div>
    )
  }
  // An entity with no mesh yet is the state this tab exists to get *out* of, so
  // the reconstruction panel is what fills it rather than a sentence about
  // meshes that will appear from somewhere unspecified (#110).
  if (!selected) {
    return (
      <section className="mesh-pane is-empty" aria-label={`3D concepts for ${node.name}`}>
        <div className="mesh-workbench">
          <div className="mesh-empty">
            <h3>No meshes yet</h3>
            <p>
              Reconstruct one from a turnaround and it opens here, in the viewer, ready to export.
            </p>
          </div>
          <TurnaroundReview node={node} queue={queue} readOnly={readOnly} />
        </div>
      </section>
    )
  }

  return (
    <section className="mesh-pane" aria-label={`3D concepts for ${node.name}`}>
      {(concepts.data?.length ?? 0) > 1 && (
        <nav className="mesh-history" aria-label="Mesh history">
          {concepts.data?.map((concept) => (
            <button
              key={concept.asset.id}
              className={concept.asset.id === selected.asset.id ? 'is-active' : ''}
              aria-current={concept.asset.id === selected.asset.id ? 'true' : undefined}
              onClick={() => setSelectedId(concept.asset.id)}
            >
              <b>{new Date(concept.createdAt).toLocaleDateString()}</b>
              <span>{concept.model}</span>
            </button>
          ))}
        </nav>
      )}
      <div className="mesh-workbench">
        <div className="mesh-viewer">
          <div className="mesh-toolbar">
            <button
              className={turntable ? 'btn-mini is-on' : 'btn-mini'}
              aria-pressed={turntable}
              onClick={() => setTurntable((value) => !value)}
            >
              Turntable
            </button>
            <button
              className={wireframe ? 'btn-mini is-on' : 'btn-mini'}
              aria-pressed={wireframe}
              onClick={() => setWireframe((value) => !value)}
            >
              Wireframe
            </button>
            <span className="mesh-scale">Orange marker = 1 m</span>
            <span className="mesh-size">{formatBytes(selected.asset.bytes)}</span>
            <button className="btn-mini" onClick={() => void reveal()}>
              Reveal GLB
            </button>
            <button className="btn-mini" disabled={exporting} onClick={() => void exportMesh()}>
              {exporting ? 'Exporting…' : 'Export copy…'}
            </button>
          </div>
          <div className="mesh-stage">
            {path.isPending && <span>Loading and validating GLB…</span>}
            {path.isError && <span>{api.errorMessage(path.error)}</span>}
            {!path.isPending && !path.isError && !meshUrl && <span>The GLB is unavailable.</span>}
            {meshUrl && (
              <Suspense fallback={<span>Starting 3D viewer…</span>}>
                <MeshViewport
                  url={meshUrl}
                  turntable={turntable}
                  wireframe={wireframe}
                  onError={onViewerError}
                />
              </Suspense>
            )}
          </div>
          {error && <p className="mesh-error">{error}</p>}
        </div>
        <TurnaroundSheet concept={selected} />
      </div>
      <TurnaroundReview node={node} queue={queue} readOnly={readOnly} />
    </section>
  )
}

const VIEW_ORDER = [
  'front',
  'left',
  'right',
  'back',
  'top',
  'bottom',
  'left_front',
  'right_front',
] as const

function TurnaroundSheet({ concept }: { concept: MeshConcept }) {
  const views = [...concept.turnaround].sort(
    (a, b) =>
      VIEW_ORDER.indexOf(a.viewType as (typeof VIEW_ORDER)[number]) -
      VIEW_ORDER.indexOf(b.viewType as (typeof VIEW_ORDER)[number]),
  )
  return (
    <aside className="mesh-sheet" aria-label="Source turnaround sheet">
      <div>
        <h3>Source turnaround</h3>
        <span>{concept.backend}</span>
      </div>
      {views.length === 0 ? (
        <p>The immutable mesh receipt did not record a complete source sheet.</p>
      ) : (
        <div className="mesh-sheet-grid">
          {views.map((view) => (
            <TurnaroundTile key={view.generationId} view={view} />
          ))}
        </div>
      )}
    </aside>
  )
}

function TurnaroundTile({ view }: { view: TurnaroundView }) {
  const thumb = useAssetThumb(view.assetId)
  return (
    <figure>
      {thumb.data ? (
        <img src={convertFileSrc(thumb.data)} alt={`${view.viewType} turnaround view`} />
      ) : (
        <span />
      )}
      <figcaption>{view.viewType.replace('_', ' ')}</figcaption>
    </figure>
  )
}

function safeFilename(name: string): string {
  return (
    name
      .trim()
      .replace(/[^a-z0-9._-]+/gi, '-')
      .replace(/^-+|-+$/g, '') || 'wobu-mesh'
  )
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
