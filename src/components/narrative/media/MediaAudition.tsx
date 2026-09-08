import { useEffect, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import type { MediaAudition as Audition } from '../../../lib/api/narrativeMedia'
import { localeDirection } from '../../../lib/api/narrativeLocale'
export function MediaAudition({ audition }: { audition: Audition }) {
  return <Player key={`${audition.path}/${audition.take.audio.hash}`} audition={audition} />
}
function Player({ audition }: { audition: Audition }) {
  const [source, setSource] = useState('')
  const [position, setPosition] = useState(0)
  const [error, setError] = useState('')
  useEffect(() => {
    const controller = new AbortController()
    let objectUrl = ''
    // WebKit can fetch the scoped asset protocol but cannot stream it as media.
    // Only the explicitly requested, backend-validated take is held in memory.
    void (async () => {
      try {
        const response = await fetch(convertFileSrc(audition.path), {
          signal: controller.signal,
          cache: 'no-store',
        })
        if (!response.ok) throw new Error('The recording file could not be read.')
        const expected = audition.take.audio.bytes
        if (expected < 1 || expected > 32 * 1024 * 1024 || !response.body)
          throw new Error('The recording file size is invalid.')
        const bytes = new Uint8Array(expected)
        const reader = response.body.getReader()
        let offset = 0
        try {
          while (true) {
            const { value, done } = await reader.read()
            if (done) break
            if (offset + value.byteLength > expected)
              throw new Error(
                'The recording file size changed. Audition it again after checking the file.',
              )
            bytes.set(value, offset)
            offset += value.byteLength
          }
          if (offset !== expected)
            throw new Error(
              'The recording file size changed. Audition it again after checking the file.',
            )
        } finally {
          await reader.cancel()
          reader.releaseLock()
        }
        if (controller.signal.aborted) return
        objectUrl = URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' }))
        setSource(objectUrl)
      } catch (cause) {
        if (!controller.signal.aborted)
          setError(cause instanceof Error ? cause.message : 'The recording file could not be read.')
      }
    })()
    return () => {
      controller.abort()
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [audition.path, audition.take.audio.bytes])
  const active =
    audition.timing?.cues.filter((cue) => cue.start_ms <= position && position < cue.end_ms) ?? []
  return (
    <section aria-label="Recording audition">
      <p>
        {audition.current ? 'Current take' : 'Historical or outdated take'} ·{' '}
        {audition.take.info.duration_ms} ms · {audition.take.info.sample_rate} Hz
      </p>
      <p dir={localeDirection(audition.take.row.key.locale)}>{audition.take.spoken_text}</p>
      <audio
        controls
        preload="metadata"
        src={source || undefined}
        onTimeUpdate={(e) => setPosition(Math.floor(e.currentTarget.currentTime * 1000))}
        onError={() =>
          setError(
            'Audio playback failed. The file is retained; inspect its format and device playback support.',
          )
        }
      />
      <p>Playback: {position} ms</p>
      {audition.timing ? (
        <>
          <p>{audition.timing.cues.length} prepared timing cues</p>
          <ul aria-label="Active timing cues">
            {active.map((cue, i) => (
              <li key={i}>
                {cue.kind}: {cue.value} ({cue.start_ms}–{cue.end_ms} ms)
              </li>
            ))}
          </ul>
        </>
      ) : (
        <p>No timing sidecar in this take.</p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  )
}
