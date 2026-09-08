import { useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import type { MediaAudition as Audition } from '../../../lib/api/narrativeMedia'
import { localeDirection } from '../../../lib/api/narrativeLocale'
export function MediaAudition({ audition }: { audition: Audition }) {
  const [position, setPosition] = useState(0)
  const [error, setError] = useState('')
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
        src={convertFileSrc(audition.path)}
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
