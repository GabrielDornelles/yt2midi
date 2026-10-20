import { useEffect, useState } from 'react'
import { pollJob } from '../lib/api'
import { go } from '../lib/router'
import type { Job } from '../types'
import { Alert, Check } from './icons'

const STAGES = [
  { key: 'download', title: 'Getting the video', sub: 'This is usually the longest part.' },
  { key: 'detect', title: 'Finding the keyboard', sub: 'Looking for the piano at the bottom of the screen.' },
  { key: 'read', title: 'Listening to the keys', sub: 'Following every key that lights up.' },
  { key: 'finish', title: 'Writing your MIDI', sub: 'Almost there.' },
]

interface Done { video_id: string }

export function Processing({ job }: { job: string }) {
  const [state, setState] = useState<Job<Done>>({ state: 'running', progress: 0, stage: 'download', status: 'Starting…' })
  const [error, setError] = useState<{ msg: string; code?: string; vid?: string } | null>(null)

  useEffect(() => {
    const ac = new AbortController()
    pollJob<Done>(job, setState, ac.signal)
      .then((r) => go(`/song/${r.video_id}`))
      .catch((e: Error & { job?: Job }) => {
        if (ac.signal.aborted) return
        const gone = /not found/i.test(e.message)
        setError({ msg: gone ? 'This job is no longer running (the server was restarted).' : e.message, code: e.job?.code, vid: e.job?.video_id })
      })
    return () => ac.abort()
  }, [job])

  const active = Math.max(0, STAGES.findIndex((s) => s.key === state.stage))

  if (error) {
    return (
      <div className="processing">
        <div className="notice bad" role="alert">
          <Alert />
          <div className="grow">
            <b>Something got in the way</b>
            {error.msg}
          </div>
        </div>
        <div className="row" style={{ marginTop: 18, justifyContent: 'center' }}>
          <button className="btn primary" onClick={() => go('/')}>Try another video</button>
          {error.code === 'no_keyboard' && error.vid && (
            <button className="btn" onClick={() => go(`/song/${error.vid}/adjust`)}>Show me the keyboard myself</button>
          )}
        </div>
      </div>
    )
  }

  return (
    <div className="processing">
      <h1>Making your sheet…</h1>
      <p className="sub">You can leave this tab open. Nothing to click.</p>
      <div className="card pad">
        <ol className="steps">
          {STAGES.map((s, i) => (
            <li key={s.key} className={i < active ? 'done' : i === active ? 'active' : ''}>
              <span className="mark">{i < active ? <Check /> : i === active ? <span className="spin" /> : i + 1}</span>
              <span>
                {s.title}
                {i === active && <small>{state.status && state.status !== s.title ? state.status : s.sub}</small>}
              </span>
            </li>
          ))}
        </ol>
        <div className="progress" role="progressbar" aria-valuenow={Math.round(state.progress * 100)} aria-valuemin={0} aria-valuemax={100}>
          <i style={{ width: `${Math.max(3, state.progress * 100)}%` }} />
        </div>
      </div>
    </div>
  )
}
