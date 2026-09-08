import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'
import { ReviewDiff } from './ReviewDiff'

it('compares wording with explicit change labels and distinguishes the base revision', () => {
  render(
    <ReviewDiff
      current="I saw the beacon fail."
      candidate="They say the beacon failed."
      currentRevision="new-revision"
      baseRevision="old-revision"
      baseWording="The beacon failed."
    />,
  )
  expect(screen.getByRole('table')).toHaveAccessibleName(
    'Current wording compared with proposed wording',
  )
  expect(screen.getByRole('rowheader', { name: 'Changed' })).toBeInTheDocument()
  expect(screen.getByRole('cell', { name: 'I saw the beacon fail.' })).toBeInTheDocument()
  expect(screen.getByRole('cell', { name: 'They say the beacon failed.' })).toBeInTheDocument()
  expect(screen.getByText(/Current wording has changed since/)).toBeInTheDocument()
  expect(screen.getByText('The beacon failed.')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Show full wording' }))
  expect(screen.getByRole('button', { name: 'Show changes only' })).toHaveAttribute(
    'aria-pressed',
    'true',
  )
})

it('does not invent missing base wording or an empty-slot text revision', () => {
  const { rerender } = render(
    <ReviewDiff current="" candidate="A rumour." currentRevision={null} baseRevision={null} />,
  )
  expect(screen.getByText('Originally empty slot')).toBeInTheDocument()
  expect(screen.getByRole('rowheader', { name: 'Added' })).toBeInTheDocument()
  rerender(
    <ReviewDiff
      current="A rumour."
      candidate="A rumour."
      currentRevision="revision"
      baseRevision="revision"
    />,
  )
  expect(screen.getByText('The current and proposed wording are identical.')).toBeInTheDocument()
  expect(screen.getByText(/its wording was not included/)).toBeInTheDocument()
})
