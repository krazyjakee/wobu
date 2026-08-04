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
            ? ` ${outcome.droppedExternalLinkCount} link${outcome.droppedExternalLinkCount === 1 ? '' : 's'} pointing outside the branch were left behind.`
            : ''
          toast(
            `Imported ${outcome.plannedNodeCount} entit${outcome.plannedNodeCount === 1 ? 'y' : 'ies'}.${links}`,
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
        apply.isPending
          ? 'Copying the branch across. This cannot be stopped once it has started.'
          : undefined
      }
    >
      <h2 id="style-transfer-title">Import from another project</h2>
      <p id="style-transfer-description">
        {preview
          ? `Choose one entity from ${preview.sourceProjectName} to bring across. Everything nested inside it, and the images it uses, come with it; links to anything else stay behind.`
          : error
            ? 'The source project preview could not be read.'
            : 'Reading the source project…'}
      </p>
      <div>
        {preview && (
          <>
            <div className="field">
              <label htmlFor="transfer-root">Top of the branch</label>
              <Combobox
                id="transfer-root"
                value={rootId}
                options={rootOptions}
                sort="title"
                placeholder="Choose where the branch starts"
                onChange={setRootId}
              />
            </div>
            {candidate && (
              <div className="transfer-summary">
                <p>
                  {candidate.nodeCount} entit{candidate.nodeCount === 1 ? 'y' : 'ies'} ·{' '}
                  {candidate.referenceCount} reference{candidate.referenceCount === 1 ? '' : 's'}
                  {candidate.loraCount > 0 &&
                    ` · ${candidate.loraCount} LoRA${candidate.loraCount === 1 ? '' : 's'}`}
                  {candidate.externalLinkCount > 0 &&
                    ` · ${candidate.externalLinkCount} link${candidate.externalLinkCount === 1 ? '' : 's'} out of the branch, left behind`}
                </p>
                {candidate.replacesSingleton && (
                  <p className="sheet-warning">
                    This replaces the {labelFor(kinds.get(candidate.kind), candidate.kind)} already
                    in this project. The one here keeps its own identity, and everything that links
                    to it goes on pointing at it.
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
                    {candidate.missingLoraCount} trained style file
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
          The import stopped after {partial.appliedNodeIds.length} of {partial.plannedNodeCount}{' '}
          entities. {partial.failure} {partial.pendingNodeIds.length}{' '}
          {partial.pendingNodeIds.length === 1 ? 'entity was' : 'entities were'} not brought across.
          {partial.conflictPaths.length > 0 &&
            ` What was coming in is still on disk at ${partial.conflictPaths.join(', ')}, if you want it.`}
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
