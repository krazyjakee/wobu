import { useEffect, useMemo, useState } from 'react'
import type { TransferOutcome, TransferPreview } from '../lib/api'
import { errorMessage, styleTransferPreview } from '../lib/api'
import { useApplyStyleTransfer } from '../lib/queries'
import { labelFor, type KindIndex } from '../lib/kinds'
import { toast } from '../store/ui'

export function StyleTransferSheet({
  sourcePath,
  kinds,
  onClose,
  onImported,
}: {
  sourcePath: string
  kinds: KindIndex
  onClose: () => void
  onImported: (id: string) => void
}) {
  const [preview, setPreview] = useState<TransferPreview | null>(null)
  const [rootId, setRootId] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [partial, setPartial] = useState<TransferOutcome | null>(null)
  const apply = useApplyStyleTransfer()

  useEffect(() => {
    let active = true
    void styleTransferPreview(sourcePath).then(
      (value) => {
        if (!active) return
        setPreview(value)
        setRootId(value.defaultRootId ?? value.candidates[0]?.rootId ?? '')
      },
      (reason) => active && setError(errorMessage(reason)),
    )
    return () => {
      active = false
    }
  }, [sourcePath])

  const candidate = useMemo(
    () => preview?.candidates.find((item) => item.rootId === rootId) ?? null,
    [preview, rootId],
  )

  function submit() {
    if (!candidate || candidate.missingAssetCount > 0 || candidate.missingLoraCount > 0) return
    setError(null)
    setPartial(null)
    apply.mutate(
      { sourcePath, rootId: candidate.rootId },
      {
        onError: (reason) => setError(errorMessage(reason)),
        onSuccess: (outcome) => {
          if (!outcome.completed) {
            setPartial(outcome)
            return
          }
          const links = outcome.droppedExternalLinkCount
            ? ` ${outcome.droppedExternalLinkCount} link${outcome.droppedExternalLinkCount === 1 ? '' : 's'} outside the subtree were left behind.`
            : ''
          toast(
            `Imported ${outcome.plannedNodeCount} node${outcome.plannedNodeCount === 1 ? '' : 's'}.${links}`,
          )
          onImported(outcome.importedRootId)
        },
      },
    )
  }

  return (
    <div
      className="scrim"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <div className="sheet transfer-sheet" role="dialog" aria-label="Import style or subtree">
        <h2>Import style or subtree</h2>
        {!preview && !error && <p>Reading the source project…</p>}
        {preview && (
          <>
            <p>
              Choose one root from <b>{preview.sourceProjectName}</b>. Its nested descendants and
              referenced images come with it; links to everything else stay behind.
            </p>
            <div className="field">
              <label htmlFor="transfer-root">Root</label>
              <select
                id="transfer-root"
                value={rootId}
                onChange={(event) => setRootId(event.target.value)}
              >
                {preview.candidates.map((item) => (
                  <option key={item.rootId} value={item.rootId}>
                    {labelFor(kinds.get(item.kind), item.kind)} — {item.name}
                  </option>
                ))}
              </select>
            </div>
            {candidate && (
              <div className="transfer-summary">
                <p>
                  {candidate.nodeCount} node{candidate.nodeCount === 1 ? '' : 's'} ·{' '}
                  {candidate.referenceCount} reference{candidate.referenceCount === 1 ? '' : 's'}
                  {candidate.loraCount > 0 &&
                    ` · ${candidate.loraCount} LoRA${candidate.loraCount === 1 ? '' : 's'}`}
                  {candidate.externalLinkCount > 0 &&
                    ` · ${candidate.externalLinkCount} outside link${candidate.externalLinkCount === 1 ? '' : 's'} dropped`}
                </p>
                {candidate.replacesSingleton && (
                  <p className="sheet-warning">
                    This replaces the destination{' '}
                    {labelFor(kinds.get(candidate.kind), candidate.kind)} content. Its destination
                    identity and incoming links are preserved.
                  </p>
                )}
                {candidate.missingAssetCount > 0 && (
                  <p className="sheet-err">
                    {candidate.missingAssetCount} referenced image
                    {candidate.missingAssetCount === 1 ? ' is' : 's are'} missing from the source.
                    Restore them before importing.
                  </p>
                )}
                {candidate.missingLoraCount > 0 && (
                  <p className="sheet-err">
                    {candidate.missingLoraCount} pinned LoRA weight file
                    {candidate.missingLoraCount === 1 ? ' is' : 's are'} missing from the source.
                    Restore them before importing.
                  </p>
                )}
              </div>
            )}
            <p className="transfer-lora">{preview.loraNote}</p>
          </>
        )}
        {error && <div className="sheet-err">{error}</div>}
        {partial && (
          <div className="sheet-err" role="status">
            Transfer stopped after {partial.appliedNodeIds.length} of {partial.plannedNodeCount}{' '}
            nodes. {partial.failure} {partial.pendingNodeIds.length} node
            {partial.pendingNodeIds.length === 1 ? '' : 's'} remain unapplied.
            {partial.conflictPaths.length > 0 &&
              ` Incoming content is recoverable at ${partial.conflictPaths.join(', ')}.`}
          </div>
        )}
        <div className="sheet-actions">
          <button className="btn btn-ghost" onClick={onClose}>
            Close
          </button>
          <button
            className="btn btn-primary"
            onClick={submit}
            disabled={
              !candidate ||
              candidate.missingAssetCount > 0 ||
              candidate.missingLoraCount > 0 ||
              apply.isPending ||
              !!partial
            }
          >
            {apply.isPending
              ? 'Importing…'
              : candidate?.replacesSingleton
                ? 'Replace and import'
                : 'Import'}
          </button>
        </div>
      </div>
    </div>
  )
}
