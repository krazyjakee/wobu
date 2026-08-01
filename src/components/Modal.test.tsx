import { fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { ConfirmSheet } from './ConfirmSheet'
import { Modal } from './Modal'

function BasicHarness({ busy = false }: { busy?: boolean }) {
  const [open, setOpen] = useState(false)
  return (
    <main data-testid="background">
      <button type="button" onClick={() => setOpen(true)}>
        Open sheet
      </button>
      {open && (
        <Modal
          titleId="basic-title"
          descriptionId="basic-description"
          onClose={() => setOpen(false)}
          busy={busy}
          busyMessage={busy ? 'Saving. This operation cannot be interrupted.' : undefined}
        >
          <h2 id="basic-title">Edit detail</h2>
          <p id="basic-description">Change the selected detail.</p>
          <button type="button" data-modal-initial-focus>
            First
          </button>
          <button type="button">Last</button>
        </Modal>
      )}
    </main>
  )
}

describe('Modal', () => {
  it('sets complete dialog semantics, makes the background inert, and restores focus', () => {
    render(<BasicHarness />)
    const opener = screen.getByRole('button', { name: 'Open sheet' })
    opener.focus()
    fireEvent.click(opener)

    const dialog = screen.getByRole('dialog', { name: 'Edit detail' })
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleDescription('Change the selected detail.')
    expect(screen.getByRole('button', { name: 'First' })).toHaveFocus()
    expect(screen.getByTestId('background').parentElement).toHaveAttribute('inert')

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(opener).toHaveFocus()
    expect(screen.getByTestId('background').parentElement).not.toHaveAttribute('inert')
  })

  it('wraps Tab and Shift+Tab within the topmost dialog', () => {
    render(<BasicHarness />)
    fireEvent.click(screen.getByRole('button', { name: 'Open sheet' }))
    const first = screen.getByRole('button', { name: 'First' })
    const last = screen.getByRole('button', { name: 'Last' })

    fireEvent.keyDown(document, { key: 'Tab', shiftKey: true })
    expect(last).toHaveFocus()
    fireEvent.keyDown(document, { key: 'Tab' })
    expect(first).toHaveFocus()
  })

  it('keeps a non-interruptible async operation open with a visible explanation', () => {
    render(<BasicHarness busy />)
    fireEvent.click(screen.getByRole('button', { name: 'Open sheet' }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveAttribute('aria-busy', 'true')
    expect(screen.getByText(/Saving\. This operation cannot be interrupted/)).toBeVisible()

    fireEvent.keyDown(document, { key: 'Escape' })
    fireEvent.mouseDown(dialog.parentElement!)
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('restores focus through nested dialogs without exposing the parent', () => {
    function Nested() {
      const [parent, setParent] = useState(false)
      const [child, setChild] = useState(false)
      return (
        <>
          <button type="button" onClick={() => setParent(true)}>
            Open parent
          </button>
          {parent && (
            <Modal
              titleId="parent-title"
              descriptionId="parent-description"
              onClose={() => setParent(false)}
            >
              <h2 id="parent-title">Parent sheet</h2>
              <p id="parent-description">Parent description.</p>
              <button type="button" data-modal-initial-focus onClick={() => setChild(true)}>
                Open child
              </button>
              {child && (
                <Modal
                  titleId="child-title"
                  descriptionId="child-description"
                  onClose={() => setChild(false)}
                >
                  <h2 id="child-title">Child sheet</h2>
                  <p id="child-description">Child description.</p>
                  <button type="button" data-modal-initial-focus>
                    Child action
                  </button>
                </Modal>
              )}
            </Modal>
          )}
        </>
      )
    }

    render(<Nested />)
    const outside = screen.getByRole('button', { name: 'Open parent' })
    outside.focus()
    fireEvent.click(outside)
    const childOpener = screen.getByRole('button', { name: 'Open child' })
    fireEvent.click(childOpener)

    expect(screen.getByRole('button', { name: 'Child action' })).toHaveFocus()
    expect(
      screen
        .getByRole('dialog', { name: 'Parent sheet', hidden: true })
        .closest('[data-modal-host]'),
    ).toHaveAttribute('inert')

    fireEvent.keyDown(document, { key: 'Escape' })
    expect(childOpener).toHaveFocus()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(outside).toHaveFocus()
  })
})

describe('ConfirmSheet', () => {
  it('initially focuses Cancel for a destructive confirmation', () => {
    render(
      <ConfirmSheet
        title="Delete node?"
        body="This cannot be undone."
        confirmLabel="Delete"
        danger
        onCancel={vi.fn()}
        onConfirm={vi.fn()}
      />,
    )

    expect(screen.getByRole('alertdialog', { name: 'Delete node?' })).toHaveAccessibleDescription(
      'This cannot be undone.',
    )
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus()
  })
})
