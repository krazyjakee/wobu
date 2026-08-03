import { render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  NODE_THUMB_BATCH_LIMIT,
  refreshNodeThumbs,
  resetNodeThumbs,
  useNodeThumbs,
} from './nodeThumbs'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: h.invoke,
  convertFileSrc: (path: string) => `asset://${path}`,
}))

/** Every call the backend saw, as `[command, nodeIds]` pairs. */
function calls(): [string, string[]][] {
  return h.invoke.mock.calls.map(([cmd, args]) => [
    cmd as string,
    ((args as { nodeIds?: string[] } | undefined)?.nodeIds ?? []) as string[],
  ])
}

/** One list of rows, exactly as the navigator and the palette use the hook. */
function Rows({ ids }: { ids: string[] }) {
  const thumbs = useNodeThumbs(ids)
  return (
    <ul>
      {ids.map((id) => (
        <li key={id} data-testid={id}>
          {thumbs.get(id) ?? 'none'}
        </li>
      ))}
    </ul>
  )
}

beforeEach(() => {
  resetNodeThumbs()
  h.invoke.mockReset()
  h.invoke.mockResolvedValue({})
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

afterEach(() => {
  resetNodeThumbs()
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('batched node thumbnails', () => {
  it('asks for a whole list of rows in one call, not one call per row', async () => {
    const ids = Array.from({ length: 30 }, (_, index) => `node-${index}`)
    h.invoke.mockResolvedValue({ 'node-3': '/thumbs/three.webp' })

    render(<Rows ids={ids} />)
    await waitFor(() => expect(screen.getByTestId('node-3')).toHaveTextContent('three.webp'))

    expect(calls()).toEqual([['node_thumb_batch', ids]])
  })

  it('coalesces separate lists mounted together into the same call', async () => {
    // The navigator, the palette and an inspector list are three components
    // that know nothing about each other. One IPC is still the right number.
    render(
      <>
        <Rows ids={['a', 'b']} />
        <Rows ids={['c']} />
      </>,
    )

    await waitFor(() => expect(h.invoke).toHaveBeenCalled())
    expect(calls()).toEqual([['node_thumb_batch', ['a', 'b', 'c']]])
  })

  it('never re-asks for an id it already knows, however often the window moves', async () => {
    h.invoke.mockResolvedValue({ a: '/thumbs/a.webp' })
    const view = render(<Rows ids={['a', 'b']} />)
    await waitFor(() => expect(screen.getByTestId('a')).toHaveTextContent('/thumbs/a.webp'))

    // Scrolled on: one new row, two already resolved.
    view.rerender(<Rows ids={['b', 'c']} />)
    await waitFor(() => expect(calls()).toHaveLength(2))
    expect(calls()[1]).toEqual(['node_thumb_batch', ['c']])

    // And back again: nothing new to ask for at all.
    view.rerender(<Rows ids={['a', 'b']} />)
    await Promise.resolve()
    expect(calls()).toHaveLength(2)
    expect(screen.getByTestId('a')).toHaveTextContent('/thumbs/a.webp')
  })

  it('splits a list larger than the backend bound rather than being rejected', async () => {
    const ids = Array.from({ length: NODE_THUMB_BATCH_LIMIT + 5 }, (_, index) => `n-${index}`)
    render(<Rows ids={ids} />)

    await waitFor(() => expect(calls()).toHaveLength(2))
    expect(calls()[0]?.[1]).toHaveLength(NODE_THUMB_BATCH_LIMIT)
    expect(calls()[1]?.[1]).toHaveLength(5)
  })

  it('settles rows on their fallback when the backend refuses, without retrying', async () => {
    h.invoke.mockRejectedValue({ code: 'project.none_open', message: 'no project' })
    render(<Rows ids={['a']} />)

    await waitFor(() => expect(h.invoke).toHaveBeenCalledTimes(1))
    expect(screen.getByTestId('a')).toHaveTextContent('none')

    // A failed lookup is remembered as "no picture"; a re-render must not
    // turn every scroll tick into another rejected round trip.
    await Promise.resolve()
    expect(h.invoke).toHaveBeenCalledTimes(1)
  })

  it('makes no call at all outside Tauri', async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
    render(<Rows ids={['a']} />)

    await waitFor(() => expect(screen.getByTestId('a')).toHaveTextContent('none'))
    expect(h.invoke).not.toHaveBeenCalled()
  })

  it('re-asks for watched rows when the world changes, and forgets the rest', async () => {
    h.invoke.mockResolvedValue({ a: '/thumbs/old.webp', gone: '/thumbs/gone.webp' })
    const view = render(<Rows ids={['a', 'gone']} />)
    await waitFor(() => expect(screen.getByTestId('a')).toHaveTextContent('/thumbs/old.webp'))

    // `gone` scrolls out of view, so nothing is watching it any more.
    view.rerender(<Rows ids={['a']} />)
    h.invoke.mockResolvedValue({ a: '/thumbs/new.webp' })
    refreshNodeThumbs()

    await waitFor(() => expect(screen.getByTestId('a')).toHaveTextContent('/thumbs/new.webp'))
    expect(calls().at(-1)).toEqual(['node_thumb_batch', ['a']])

    // The unwatched id was dropped rather than kept, so scrolling back to it
    // asks again instead of drawing a cover that has since been replaced.
    view.rerender(<Rows ids={['a', 'gone']} />)
    await waitFor(() => expect(calls().at(-1)).toEqual(['node_thumb_batch', ['gone']]))
  })

  it('leaves a picture on screen while its replacement is being fetched', async () => {
    h.invoke.mockResolvedValue({ a: '/thumbs/old.webp' })
    render(<Rows ids={['a']} />)
    await waitFor(() => expect(screen.getByTestId('a')).toHaveTextContent('/thumbs/old.webp'))

    let release: (value: Record<string, string>) => void = () => {}
    h.invoke.mockReturnValue(
      new Promise<Record<string, string>>((resolve) => {
        release = resolve
      }),
    )
    refreshNodeThumbs()
    await waitFor(() => expect(h.invoke).toHaveBeenCalledTimes(2))

    // Mid-flight: the old picture is still there rather than blanked.
    expect(screen.getByTestId('a')).toHaveTextContent('/thumbs/old.webp')
    release({ a: '/thumbs/new.webp' })
    await waitFor(() => expect(screen.getByTestId('a')).toHaveTextContent('/thumbs/new.webp'))
  })
})
