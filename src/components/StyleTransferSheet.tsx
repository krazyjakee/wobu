import { useEffect, useMemo, useState } from 'react'
import type { TransferOutcome, TransferPreview } from '../lib/api'
import { errorMessage, styleTransferPreview } from '../lib/api'
import { useApplyStyleTransfer } from '../lib/queries'
import { labelFor, type KindIndex } from '../lib/kinds'
import { toast } from '../store/ui'
import { Combobox } from './Combobox'
import { Modal } from './Modal'

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

  /*
   * Grouped by kind, and named by the entity alone.
   *
   * The row used to read "Style guide — Ashgate" because a native `<option>`
   * had nowhere else to put the kind. With a heading above each run of rows the
   * kind is said once, the name is what the user filters on, and typing
   * "ashgate" is no longer competing with the kind's own letters.
   */
  const rootOptions = useMemo(
    () =>
      (preview?.candidates ?? []).map((item) => ({
        value: item.rootId,
        label: item.name,
        keywords: item.kind,
        group: labelFor(kinds.get(item.kind), item.kind),
      })),
    [kinds, preview?.candidates],
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
    <Modal
      className="sheet transfer-sheet"
      titleId="style-transfer-title"
      descriptionId="style-transfer-description"
      onClose={onClose}
      busy={apply.isPending}
      busyMessage={
        apply.isPending ? 'Importing the subtree. This operation cannot be interrupted.' : undefined
      }
    >
      <h2 id="style-transfer-title">Import style or subtree</h2>
      <p id="style-transfer-description">
        {preview
          ? `Choose one root from ${preview.sourceProjectName}. Its nested descendants and referenced images come with it; links to everything else stay behind.`
          : error
            ? 'The source project preview could not be read.'
            : 'Reading the source project…'}
      </p>
      <div>
        {preview && (
          <>
            <div className="field">
              <label htmlFor="transfer-root">Root</label>
              <Combobox
                id="transfer-root"
                value={rootId}
                options={rootOptions}
                sort="title"
                placeholder="Choose a root"
                onChange={setRootId}
              />
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
      </div>
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
        <button
          className="btn btn-ghost"
          onClick={onClose}
          disabled={apply.isPending}
          data-modal-initial-focus
        >
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
    </Modal>
  )
}
