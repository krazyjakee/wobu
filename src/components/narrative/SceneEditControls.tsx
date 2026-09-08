import type { useSceneEditSession } from './useSceneEditSession'

export function SceneEditControls({
  session,
}: {
  session: ReturnType<typeof useSceneEditSession>
}) {
  const { draft, disabled } = session
  return (
    <>
      <div className="nrt-script-toolbar" aria-label="Scene editing">
        <button
          type="button"
          className="btn is-primary"
          disabled={disabled || !draft}
          onClick={() => void session.save()}
        >
          {draft?.pending ? 'Saving…' : 'Save scene'}
        </button>
        <button
          type="button"
          className="btn"
          disabled={!!draft?.pending || !draft}
          onClick={session.discard}
        >
          Discard changes
        </button>
        <button
          type="button"
          className="btn"
          disabled={disabled || !draft?.past.length}
          onClick={session.undo}
        >
          Undo draft
        </button>
        <button
          type="button"
          className="btn"
          disabled={disabled || !draft?.future.length}
          onClick={session.redo}
        >
          Redo draft
        </button>
        <span role="status">
          {draft ? 'Unsaved scene — shared by Script and Flow.' : 'Saved scene'}
        </span>
      </div>
      {draft?.incoming && (
        <p role="alert">
          The scene changed since this draft began. Saving will check for a conflict; discard to
          load the latest version.
        </p>
      )}
      {draft?.error && (
        <p className="inline-error" role="alert">
          Could not save: {draft.error}. Your draft is kept.
          {draft.conflictPath && <> Retained conflict: {draft.conflictPath}</>}
        </p>
      )}
      {draft?.historyTruncated && (
        <p role="status">
          Earlier draft undo steps exceeded the local history limit. Your current scene is kept;
          Discard still returns to saved source.
        </p>
      )}
    </>
  )
}
