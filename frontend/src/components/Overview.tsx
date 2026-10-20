import { useEffect, useMemo, useRef, useState } from 'react'
import { get, post } from '../lib/api'
import { go } from '../lib/router'
import { usePlayer } from '../lib/player'
import { FallingKeys } from './FallingKeys'
import type { NoteRec, Song } from '../types'
import { Alert, Download, Pause, Play, Swap } from './icons'
import { PianoRoll } from './PianoRoll'

const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`

export function Overview({ song, reload }: { song: Song; reload: () => void }) {
  const [notes, setNotes] = useState<NoteRec[] | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    const ac = new AbortController()
    get<NoteRec[]>(`/api/song/${song.video_id}/notes`, ac.signal).then(setNotes).catch(() => setNotes([]))
    return () => ac.abort()
  }, [song.video_id, song.notes, song.hand_colors])

  const list = useMemo(() => notes ?? [], [notes])
  // how the music is played: slower, and with a hand switched off (remembered per song)
  const [speed, setSpeed] = useState(() => { try { return Number(localStorage.getItem(`speed:${song.video_id}`)) || 100 } catch { return 100 } })
  const [handOn, setHandOn] = useState<[boolean, boolean]>(() => { try { return JSON.parse(localStorage.getItem(`hands:${song.video_id}`) ?? '[true,true]') } catch { return [true, true] } })
  useEffect(() => { try { localStorage.setItem(`speed:${song.video_id}`, String(speed)); localStorage.setItem(`hands:${song.video_id}`, JSON.stringify(handOn)) } catch { /* not remembered */ } }, [speed, handOn, song.video_id])
  const playback = useMemo(() => ({ speed: speed / 100, gains: [handOn[0] ? 1 : 0, handOn[1] ? 1 : 0] as [number, number] }), [speed, handOn])
  const player = usePlayer(list, song.video_id, playback)
  const colors = useMemo(() => song.hand_colors ?? [], [song.hand_colors])

  // label hands by the pitch they play, which is clearer than "hand 1 / hand 2"
  const labels = useMemo(() => {
    const med = [0, 1].map((h) => {
      const m = list.filter((n) => n.hand === h).map((n) => n.midi).sort((a, b) => a - b)
      return m.length ? m[Math.floor(m.length / 2)] : null
    })
    return colors.map((_, i) => (colors.length < 2 ? 'All notes' : med[i] !== null && med[1 - i] !== null && med[i]! < med[1 - i]! ? 'Lower notes' : 'Higher notes'))
  }, [list, colors])

  // full screen: the piano takes the whole screen, and the controls sit over it and fade away while the music plays
  const studio = useRef<HTMLDivElement>(null)
  const [full, setFull] = useState(false)
  const [awake, setAwake] = useState(true)
  const idle = useRef(0)
  const wake = () => { setAwake(true); window.clearTimeout(idle.current); idle.current = window.setTimeout(() => setAwake(false), 2800) }
  const openFull = () => { setFull(true); wake(); void studio.current?.requestFullscreen?.().catch(() => undefined) }
  const closeFull = () => { setFull(false); window.clearTimeout(idle.current); if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined) }
  useEffect(() => {
    const onFs = () => { if (!document.fullscreenElement) setFull(false) }
    document.addEventListener('fullscreenchange', onFs)
    return () => { document.removeEventListener('fullscreenchange', onFs); window.clearTimeout(idle.current) }
  }, [])
  useEffect(() => {
    if (!full) return
    const on = (e: KeyboardEvent) => {
      if (e.key === 'Escape') closeFull()
      else if (e.key === ' ' && !(e.target instanceof HTMLButtonElement || e.target instanceof HTMLInputElement)) { e.preventDefault(); player.toggle() }
    }
    window.addEventListener('keydown', on)
    document.body.style.overflow = 'hidden'
    return () => { window.removeEventListener('keydown', on); document.body.style.overflow = '' }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [full, player.toggle])
  const quiet = full && player.playing && !awake

  async function swap() {
    setBusy(true)
    try { await post(`/api/song/${song.video_id}/swap-hands`, {}); reload() } finally { setBusy(false) }
  }

  return (
    <div className="overview">
      <div className="stack">
        <div className="card roll-wrap">
          {notes === null ? <div className="empty">Loading notes…</div> : list.length === 0 ? (
            <div className="empty">No notes were found. Try adjusting the keyboard.</div>
          ) : (
            <div ref={studio} className={`studio ${full ? 'fs' : ''} ${quiet ? 'quiet' : ''}`} onMouseMove={full ? wake : undefined} onPointerDown={full ? wake : undefined}>
              <div className="fs-top">
              <div className="player">
                <button className="btn primary icon" onClick={player.toggle} aria-label={player.playing ? 'Pause' : 'Play'}>
                  {player.playing ? <Pause /> : <Play />}
                </button>
                <span className="time">{mmss(player.pos)} / {mmss(player.duration)}</span>
                <span className="grow" />
                <span className="speedbox">
                  <label htmlFor="speed">Speed {speed}%</label>
                  <input id="speed" type="range" min={25} max={100} step={5} value={speed} onChange={(e) => setSpeed(Number(e.target.value))} />
                  {[50, 75, 100].map((v) => <button key={v} className={`btn small ${speed === v ? 'primary' : ''}`} onClick={() => setSpeed(v)}>{v}</button>)}
                </span>
                <button className="btn small" onClick={full ? closeFull : openFull}>{full ? 'Exit full screen' : 'Full screen'}</button>
              </div>
              <div className="player handrow" role="group" aria-label="Hands">
                {colors.map((c, i) => (
                  <button key={i} className={`btn small hand-toggle ${handOn[i === 1 ? 1 : 0] ? '' : 'off'}`} aria-pressed={handOn[i === 1 ? 1 : 0]}
                    onClick={() => setHandOn((h) => (i === 1 ? [h[0], !h[1]] : [!h[0], h[1]]))}>
                    <span className="dot" style={{ background: `rgb(${c})` }} /> {labels[i]}: {handOn[i === 1 ? 1 : 0] ? 'on' : 'off'}
                  </button>
                ))}
                <span className="hint grow">
                  {player.sound === 'loading' ? 'Loading the piano sound…' : player.sound === 'synth' ? 'Piano samples unavailable, using a simple tone.' : 'Slow it down or switch a hand off to learn a hard part.'}
                </span>
              </div>
              <div className="slim"><PianoRoll notes={list} colors={colors} duration={player.duration} pos={player.pos} onSeek={player.seek} /></div>
              </div>
              <FallingKeys notes={list} colors={colors} on={handOn} playing={player.playing} pos={player.pos} getPos={player.getPos} />
            </div>
          )}
        </div>
        {song.auto && song.octave_certain === false && (
          <div className="notice">
            <Alert />
            <div className="grow">
              <b>Worth a quick check: the octave</b>
              The video shows only part of a piano, so the note names depend on which key is on the far left. We guessed <b style={{ display: 'inline' }}>{song.first_note}</b>.
            </div>
            <button className="btn small" onClick={() => go(`/song/${song.video_id}/adjust`)}>Check keyboard</button>
          </div>
        )}
      </div>

      <div className="side">
        <div className="card pad stack">
          <div>
            <h2>Your music</h2>
            <p className="hint">{song.notes?.toLocaleString()} notes found</p>
          </div>
          <a className="btn primary big" href={`/api/result/${song.video_id}.mid`} download>
            <Download /> Download MIDI
          </a>
          <button className="btn big" onClick={() => go(`/song/${song.video_id}/sheet`)}>Open the easy sheet</button>
        </div>
        <div className="card pad">
          <h2>Hands</h2>
          <p className="hint">Told apart by the color of the lit keys.</p>
          <div style={{ marginTop: 8 }}>
            {colors.map((c, i) => (
              <div className="hand" key={i}>
                <span className="dot" style={{ background: `rgb(${c})` }} />
                <span className="grow">{labels[i]}</span>
              </div>
            ))}
          </div>
          {colors.length >= 2 && (
            <button className="btn small" style={{ marginTop: 8 }} disabled={busy} onClick={swap}>
              <Swap /> {busy ? 'Swapping…' : 'Swap hands'}
            </button>
          )}
        </div>
        <div className="card pad">
          <h2>Something off?</h2>
          <p className="hint" style={{ marginBottom: 10 }}>Missing notes, wrong names or a keyboard that was not found correctly.</p>
          <button className="btn" onClick={() => go(`/song/${song.video_id}/adjust`)}>Adjust the keyboard</button>
        </div>
      </div>
    </div>
  )
}
