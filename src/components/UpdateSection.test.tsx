import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { UpdateSection } from './UpdateSection'

/*
 * The update pane. Two things matter here and neither is markup: that nothing
 * reaches the network until the button is pressed — a local-first app that
 * phoned home on mount would contradict what the Legal pane promises — and that
 * a refusal is *shown*, because the refusal is the security feature. A verified
 * signature failing quietly and a successful install look identical to someone
 * who has already looked away.
 */

const h = vi.hoisted(() => ({ check: vi.fn(), relaunch: vi.fn(), openUrl: vi.fn() }))

vi.mock('@tauri-apps/plugin-updater', () => ({ check: h.check }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: h.relaunch }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: h.openUrl }))

beforeEach(() => {
  h.check.mockReset()
  h.relaunch.mockReset().mockResolvedValue(undefined)
  h.openUrl.mockReset().mockResolvedValue(undefined)
})

function press(label: string | RegExp) {
  fireEvent.click(screen.getByRole('button', { name: label }))
}

it('contacts nothing until the user asks', () => {
  render(<UpdateSection />)

  expect(h.check).not.toHaveBeenCalled()
  expect(screen.getByText('Not checked this session.')).toBeTruthy()
})

it('says so plainly when this is already the latest release', async () => {
  h.check.mockResolvedValue(null)
  render(<UpdateSection />)

  press(/check for updates/i)

  await waitFor(() => expect(screen.getByText('You are on the latest release.')).toBeTruthy())
})

it('installs only after a second, explicit press, then offers the restart', async () => {
  const downloadAndInstall = vi.fn(async (onEvent: (e: unknown) => void) => {
    onEvent({ event: 'Started', data: { contentLength: 200 } })
    onEvent({ event: 'Progress', data: { chunkLength: 100 } })
    onEvent({ event: 'Finished' })
  })
  h.check.mockResolvedValue({ version: '9.9.9', body: 'Fixes everything.', downloadAndInstall })
  render(<UpdateSection />)

  press(/check for updates/i)
  await waitFor(() => expect(screen.getByText('Wobu 9.9.9 is available.')).toBeTruthy())
  expect(screen.getByText('Fixes everything.')).toBeTruthy()
  // Found, not fetched: the bytes wait for the user.
  expect(downloadAndInstall).not.toHaveBeenCalled()

  press(/download and install 9\.9\.9/i)
  await waitFor(() => expect(screen.getByText(/9\.9\.9 is installed/)).toBeTruthy())

  press(/restart now/i)
  expect(h.relaunch).toHaveBeenCalled()
})

it('shows a rejected payload rather than swallowing it', async () => {
  h.check.mockRejectedValue(new Error('signature verification failed'))
  render(<UpdateSection />)

  press(/check for updates/i)

  await waitFor(() => expect(screen.getByText(/signature verification failed/)).toBeTruthy())
  // Still offerable: a failed check is not a dead pane.
  expect(screen.getByRole('button', { name: /check for updates/i })).toBeTruthy()
})
