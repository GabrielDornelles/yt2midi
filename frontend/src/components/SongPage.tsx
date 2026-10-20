import { useCallback, useEffect, useState } from 'react'
import { get } from '../lib/api'
import { go } from '../lib/router'
import type { Song } from '../types'
import { Adjust } from './Adjust'
import { Overview } from './Overview'
import { Score } from './Score'
import { Sheet } from './Sheet'

const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`

export function SongPage({ id, tab }: { id: string; tab: 'overview' | 'sheet' | 'score' | 'adjust' }) {
  const [song, setSong] = useState<Song | null>(null)
  const [err, setErr] = useState('')

  const reload = useCallback(() => {
    get<Song>(`/api/song/${id}`).then(setSong).catch((e: Error) => setErr(e.message))
  }, [id])
  useEffect(reload, [reload])

  // a song without notes yet can only be set up in Adjust
  useEffect(() => { if (song && !song.ready && tab !== 'adjust') go(`/song/${id}/adjust`) }, [song, tab, id])

  if (err) return <div className="empty">We couldn't open this song ({err}). <a href="#/">Back to your songs</a></div>
  if (!song) return <div className="empty">Opening…</div>

  const link = (t: string, label: string, disabled = false) => (
    <a href={`#/song/${id}${t ? '/' + t : ''}`} className={(tab === (t || 'overview') ? 'on' : '')} aria-disabled={disabled} aria-current={tab === (t || 'overview') ? 'page' : undefined}>
      {label}
    </a>
  )

  return (
    <>
      <div className="song-head">
        <div className="thumb" style={{ backgroundImage: `url(/api/thumb/${id})` }} />
        <div className="titlebox">
          <h1>{song.title}</h1>
          <div className="meta">
            {song.ready ? (
              <>
                <span className="chip">{song.notes?.toLocaleString()} notes</span>
                {!!song.duration && <span className="chip">{mmss(song.duration)}</span>}
                {song.first_note && <span className="chip">{song.n_white} white keys from {song.first_note}</span>}
                {song.hand_colors?.map((c, i) => <span key={i} className="dot" style={{ background: `rgb(${c})` }} />)}
              </>
            ) : <span className="chip warn">Keyboard needs to be set up</span>}
          </div>
        </div>
      </div>
      <div className="tabsbar">
        <nav className="seg" aria-label="Song sections">
          {link('', 'Overview', !song.ready)}
          {link('sheet', 'Easy sheet', !song.ready)}
          {link('score', 'Sheet music', !song.ready)}
          {link('adjust', 'Adjust')}
        </nav>
      </div>
      {tab === 'overview' && song.ready && <Overview song={song} reload={reload} />}
      {tab === 'sheet' && song.ready && <Sheet song={song} />}
      {tab === 'score' && song.ready && <Score song={song} />}
      {tab === 'adjust' && <Adjust song={song} reload={reload} />}
    </>
  )
}
