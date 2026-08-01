import { useState } from 'react'
import { save } from '@tauri-apps/plugin-dialog'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import type { ProjectSummary, WikiExport } from '../lib/api'
import { errorMessage, projectExportWiki } from '../lib/api'

export function WikiExportSection({ project }: { project: ProjectSummary }) {
  const [exporting, setExporting] = useState(false)
  const [result, setResult] = useState<WikiExport | null>(null)
  const [error, setError] = useState<string | null>(null)

  async function start() {
    setError(null)
    let destination: string | null
    try {
      destination = await save({
        title: 'Export static world wiki',
        defaultPath: `${safeFilename(project.name)}-wiki`,
      })
    } catch (reason) {
      setError(errorMessage(reason))
      return
    }
    if (!destination) return

    setExporting(true)
    setResult(null)
    try {
      setResult(await projectExportWiki(destination))
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setExporting(false)
    }
  }

  async function reveal() {
    if (!result) return
    try {
      await revealItemInDir(result.destination)
    } catch (reason) {
      setError(errorMessage(reason))
    }
  }

  return (
    <section className="set-sec" aria-labelledby="wiki-export-title">
      <h3 id="wiki-export-title">Static world wiki</h3>
      <p className="set-note">
        Make a browsable, self-contained site with node pages, image galleries, concepts, and the
        influence graph. The project is only read; this also works when it is read-only.
      </p>
      <p className="set-note">
        Choose a new folder path outside the project. Wobu will not overwrite an existing export.
      </p>
      <div className="set-acts">
        <button className="btn-mini" onClick={() => void start()} disabled={exporting}>
          {exporting ? 'Exporting…' : 'Export static wiki…'}
        </button>
        {result && (
          <button className="btn-mini" onClick={() => void reveal()}>
            Reveal exported folder
          </button>
        )}
      </div>
      {result && (
        <div className="wiki-export-result" role="status">
          <span>
            Exported {result.nodeCount} {result.nodeCount === 1 ? 'node' : 'nodes'} and{' '}
            {result.imageCount} {result.imageCount === 1 ? 'image' : 'images'}.
          </span>
          {result.missingImages > 0 && (
            <span className="wiki-export-warning">
              {result.missingImages} missing{' '}
              {result.missingImages === 1 ? 'image was' : 'images were'} replaced with placeholders.
            </span>
          )}
          <span className="set-path">{result.destination}</span>
        </div>
      )}
      {error && (
        <p className="wiki-export-error" role="alert">
          {error}
        </p>
      )}
    </section>
  )
}

function safeFilename(value: string): string {
  const safe = value
    .trim()
    .replace(/[\\/:*?"<>|]+/g, '-')
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-')
  return safe.replace(/^[.-]+|[.-]+$/g, '') || 'world'
}
