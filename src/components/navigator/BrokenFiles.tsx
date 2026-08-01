import { useState } from 'react'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import type { CorruptFile } from '../../lib/api'
import { projectReload } from '../../lib/api'
import { report, toast } from '../../store/ui'
import { Icon } from '../Icon'

/**
 * Files that are on disk and could not be parsed.
 *
 * Sits above the tree rather than inside it, because a broken file is not a
 * node — it may never have had one — and burying it in whichever kind folder
 * it happens to live under is how it goes unnoticed. Dropbox and OneDrive can
 * copy a half-written file, so this is a normal thing to happen to a synced
 * project, not an exotic one.
 *
 * The two actions are deliberately the only two: **reveal**, because fixing a
 * truncated YAML block is a text-editor job and Wobu should not pretend
 * otherwise, and **reload**, because after fixing it the user needs to know
 * whether it worked without guessing at the watcher's debounce. There is no
 * "repair" button — inventing content to replace what a sync client mangled is
 * exactly the thing this feature exists to prevent.
 */
export function BrokenFiles({ files, projectPath }: { files: CorruptFile[]; projectPath: string }) {
  if (!files.length) return null
  return (
    <div className="broken">
      <div className="broken-h">
        <Icon name="lock" size="sm" />
        {files.length === 1
          ? '1 file could not be read'
          : `${files.length} files could not be read`}
      </div>
      {files.map((f) => (
        <BrokenRow key={f.relPath} file={f} projectPath={projectPath} />
      ))}
      <p className="broken-note">
        Wobu has not touched these, and will not write over them. Anything they used to contain is
        still listed below, from the last time it read cleanly.
      </p>
    </div>
  )
}

function BrokenRow({ file, projectPath }: { file: CorruptFile; projectPath: string }) {
  const [busy, setBusy] = useState(false)

  // Forward slashes throughout: `relPath` is stored `/`-separated so the same
  // project opens from `/Volumes/art` and `Z:\art`, and Windows accepts them.
  const absolute = `${projectPath}/${file.relPath}`

  async function reveal() {
    try {
      await revealItemInDir(absolute)
    } catch (e) {
      report(e, 'Could not open the file manager')
    }
  }

  async function reload() {
    setBusy(true)
    try {
      await projectReload()
      toast('Folder re-read.')
    } catch (e) {
      report(e, 'Could not re-read the folder')
    } finally {
      setBusy(false)
    }
  }

  // Most parse errors name the file themselves, and printing the path twice
  // reads like a bug. The standalone line is kept for the ones that don't —
  // a raw YAML error is "expected a map at line 3" and nothing else.
  const errorNamesTheFile = file.error.includes(file.relPath)

  return (
    <div className="broken-row">
      {!errorNamesTheFile && <code className="broken-path">{file.relPath}</code>}
      {/* The parser's own words. Terse and technical, and the only thing that
          says which line to look at. */}
      <span className="broken-why">{file.error}</span>
      <div className="broken-acts">
        <button className="btn-mini" onClick={reveal}>
          <Icon name="folder" size="sm" />
          Reveal
        </button>
        <button className="btn-mini" onClick={reload} disabled={busy}>
          <Icon name="refresh" size="sm" />
          {busy ? 'Reading…' : 'Reload'}
        </button>
      </div>
    </div>
  )
}
