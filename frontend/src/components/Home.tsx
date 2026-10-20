import { useEffect, useRef, useState } from 'react'
import { del, get, post } from '../lib/api'
import { go } from '../lib/router'
import type { LibraryItem } from '../types'
import { Trash, Wand } from './icons'

const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`

export function Home() {
  const [url, setUrl] = useState('')
  const [quality, setQuality] = useState('1080p')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState('')
  const [songs, setSongs] = useState<LibraryItem[] | null>(null)
  const [armed, setArmed] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    const ac = new AbortController()
    get<LibraryItem[]>('/api/library', ac.signal).then(setSongs).catch(() => setSongs([]))
    return () => ac.abort()
  }, [])

  async function start(e: React.FormEvent) {
    e.preventDefault()
    if (!url.trim()) return
    setBusy(true); setErr('')
    try {
      const { job } = await post<{ job: string }>('/api/process', { url: url.trim(), quality })
      go(`/new/${job}`)
    } catch (ex) {
      setErr(ex instanceof Error ? ex.message : 'That link did not work')
      setBusy(false)
    }
  }

  /** A video from this computer instead of a link: sent to the app, then handled like a downloaded one. */
  async function sendFile(file: File | undefined) {
    if (!file || busy) return
    setBusy(true); setErr('')
    try {
      const r = await fetch(`/api/upload?name=${encodeURIComponent(file.name)}`, { method: 'POST', body: file })
      const j = await r.json().catch(() => ({}))
      if (!r.ok) throw new Error(typeof j.detail === 'string' ? j.detail : 'That file did not work')
      go(`/new/${j.job}`)
    } catch (ex) {
      setErr(ex instanceof Error ? ex.message : 'That file did not work')
      setBusy(false)
    }
  }

  // a video can be dropped anywhere on the window
  const sendRef = useRef(sendFile)
  sendRef.current = sendFile
  useEffect(() => {
    const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files')
    const over = (e: DragEvent) => { if (hasFiles(e)) { e.preventDefault(); setDragging(true) } }
    const leave = (e: DragEvent) => { if (!e.relatedTarget) setDragging(false) }
    const drop = (e: DragEvent) => {
      if (!hasFiles(e)) return
      e.preventDefault(); setDragging(false)
      void sendRef.current(e.dataTransfer?.files?.[0])
    }
    window.addEventListener('dragover', over)
    window.addEventListener('dragleave', leave)
    window.addEventListener('drop', drop)
    return () => { window.removeEventListener('dragover', over); window.removeEventListener('dragleave', leave); window.removeEventListener('drop', drop) }
  }, [])

  async function remove(id: string) {
    if (armed !== id) { setArmed(id); window.setTimeout(() => setArmed((a) => (a === id ? null : a)), 3000); return }
    await del(`/api/song/${id}`).catch(() => undefined)
    setSongs((s) => (s ? s.filter((x) => x.video_id !== id) : s))
    setArmed(null)
  }

  return (
    <>
      <section className="newsong" aria-labelledby="new-title">
        <div className="newsong-head">
          <h1 id="new-title">New song</h1>
          <p>Paste the link to a falling-notes piano video. Keys, notes and both hands are found for you.</p>
        </div>
        <form className="paste" onSubmit={start}>
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="Paste a YouTube link…"
            aria-label="YouTube link"
            autoFocus
            inputMode="url"
            spellCheck={false}
          />
          <button className="btn primary big" disabled={busy || !url.trim()}>
            <Wand /> {busy ? 'Starting…' : 'Make my sheet'}
          </button>
        </form>
        {err && <p className="err" role="alert">{err}</p>}
        <div className="newsong-foot">
          <label className="quality">
            Video quality
            <select className="select" value={quality} onChange={(e) => setQuality(e.target.value)} aria-label="Video quality">
              <option>1080p</option><option>720p</option><option>480p</option>
            </select>
          </label>
          <span className="or">
            or drop a video file here, or{' '}
            <button type="button" className="textlink" onClick={() => input.current?.click()}>choose one</button>
          </span>
        </div>
        <input ref={input} type="file" accept="video/*,.mkv,.mp4,.webm,.mov" hidden onChange={(e) => { void sendFile(e.target.files?.[0]); e.target.value = '' }} />
      </section>

      {songs === null && <div className="empty">Loading…</div>}
      {songs && songs.length === 0 && (
        <section className="firstrun" aria-label="How it works">
          <div><i>1</i><b>Paste a link</b><span>Any YouTube piano tutorial where the notes fall onto a keyboard, or a video file of your own.</span></div>
          <div><i>2</i><b>We read the keys</b><span>The keyboard, every note and which hand plays it are found automatically.</span></div>
          <div><i>3</i><b>Learn it</b><span>Slow the piano down, switch a hand off, or read it as an easy big-dot sheet or real sheet music. The MIDI file is yours too.</span></div>
        </section>
      )}
      {songs && songs.length > 0 && (
        <>
          <div className="section-title"><h2>Your songs</h2><span className="count">{songs.length}</span></div>
          <div className="library">
            {songs.map((s) => (
              <div key={s.video_id} className="card song-card">
                <a href={`#/song/${s.video_id}`} style={{ color: 'inherit', textDecoration: 'none' }}>
                  <img className="thumb" src={`/api/thumb/${s.video_id}`} alt="" loading="lazy" />
                  <div className="body">
                    <h3>{s.title}</h3>
                    <div className="meta">
                      {s.ready ? (
                        <>
                          {s.hand_colors.map((c, i) => <span key={i} className="dot" style={{ background: `rgb(${c})` }} />)}
                          <span>{s.notes.toLocaleString()} notes · {mmss(s.duration)}</span>
                        </>
                      ) : <span className="chip warn">Needs setup</span>}
                    </div>
                  </div>
                </a>
                <button
                  className={`btn small del ${armed === s.video_id ? 'armed' : ''}`}
                  onClick={() => remove(s.video_id)}
                  aria-label={armed === s.video_id ? 'Click again to delete' : 'Delete song'}
                >
                  <Trash /> {armed === s.video_id ? 'Delete?' : ''}
                </button>
              </div>
            ))}
          </div>
        </>
      )}
      {dragging && <div className="dropveil" aria-hidden="true"><div>Drop the video to make a sheet</div></div>}
    </>
  )
}
