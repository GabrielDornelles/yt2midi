import { useCallback, useEffect, useRef, useState } from 'react'
import { get, pollJob, post, qs } from '../lib/api'
import { go } from '../lib/router'
import { shiftFirst } from '../lib/keyboard'
import type { Calib, Detection, Pressed, Song } from '../types'
import { Alert, Refresh } from './icons'
import { KeyboardCanvas, type Box } from './KeyboardCanvas'

const LETTERS = ['C', 'D', 'E', 'F', 'G', 'A', 'B']
const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`

export function Adjust({ song, reload }: { song: Song; reload: () => void }) {
  const id = song.video_id
  const [box, setBox] = useState<Box | null>(song.calib ? [song.calib.x0, song.calib.y0, song.calib.x1, song.calib.y1] : null)
  const [first, setFirst] = useState(song.calib?.first_note ?? 'C4')
  const [nWhite, setNWhite] = useState(song.calib?.n_white ?? 28)
  const [thr, setThr] = useState(song.calib?.threshold ?? 40)
  const [t, setT] = useState(() => song.thumb_t ?? Math.round((song.duration ?? 60) * 0.3))
  const [img, setImg] = useState<HTMLImageElement | null>(null)
  const [pressed, setPressed] = useState<Pressed[]>([])
  const [detecting, setDetecting] = useState(false)
  const [detMsg, setDetMsg] = useState('')
  const [suggest, setSuggest] = useState<number[]>(song.thumb_t != null ? [song.thumb_t] : [])
  const sIdx = useRef(0)
  const [match, setMatch] = useState('')
  const [matchMsg, setMatchMsg] = useState('')
  const [job, setJob] = useState<{ p: number; s: string } | null>(null)
  const [err, setErr] = useState('')

  const autodetect = useCallback(async () => {
    setDetecting(true); setDetMsg('')
    try {
      const d = await get<Detection>(`/api/autodetect/${id}`)
      if (!d.found) { setDetMsg("We couldn't find the keyboard on its own. Drag a box around it on the picture."); return }
      setBox([d.x0, d.y0, d.x1, d.y1]); setFirst(d.first_note); setNWhite(d.n_white)
      setSuggest(d.suggest_times); sIdx.current = 0
      if (d.suggest_times[0] !== undefined) setT(d.suggest_times[0])
      setDetMsg(`Found ${d.n_white} white keys.`)
    } catch (e) { setDetMsg(e instanceof Error ? e.message : 'Detection failed') } finally { setDetecting(false) }
  }, [id])

  // brand-new songs have no settings yet: find the keyboard right away
  useEffect(() => { if (!song.calib) void autodetect() }, [song.calib, autodetect])

  // detection found nothing (or hasn't run): start from a box over the bottom of the picture so there is something to drag
  useEffect(() => { if (!box && img && !detecting) setBox([0, img.naturalHeight * 0.8, img.naturalWidth, img.naturalHeight]) }, [box, img, detecting])

  // frame under the scrubber
  useEffect(() => {
    const timer = window.setTimeout(() => {
      const im = new Image()
      im.onload = () => setImg(im)
      im.src = `/api/frame/${id}?${qs({ t })}`
    }, 70)
    return () => window.clearTimeout(timer)
  }, [id, t])

  const calib = useCallback((): Calib | null => (box ? {
    video_id: id, x0: box[0], y0: box[1], x1: box[2], y1: box[3], first_note: first, n_white: nWhite,
    baseline_t: song.calib?.baseline_t ?? null, threshold: thr, hand_t: song.calib?.hand_t ?? null, swap_hands: song.calib?.swap_hands ?? false,
  } : null), [box, id, first, nWhite, thr, song.calib])

  // which keys look pressed in this frame (the server does the color math)
  useEffect(() => {
    const c = calib()
    if (!c || c.x1 - c.x0 < 20) return
    const ac = new AbortController()
    const timer = window.setTimeout(() => {
      post<{ pressed: Pressed[] }>('/api/preview', { ...c, t }, ac.signal).then((r) => setPressed(r.pressed)).catch(() => undefined)
    }, 280)
    return () => { window.clearTimeout(timer); ac.abort() }
  }, [calib, t])

  const pitch = box ? (box[2] - box[0]) / nWhite : 0
  const addRight = (d: number) => { if (!box || nWhite + d < 5) return; setNWhite(nWhite + d); setBox([box[0], box[1], box[2] + d * pitch, box[3]]) }
  const addLeft = (d: number) => { if (!box || nWhite + d < 5) return; setNWhite(nWhite + d); setBox([box[0] - d * pitch, box[1], box[2], box[3]]); setFirst((f) => shiftFirst(f, -d)) }

  async function jump() {
    if (!suggest.length) { await autodetect(); return }
    setT(suggest[sIdx.current++ % suggest.length])
  }

  async function runMatch() {
    const c = calib()
    const notes = match.split(/[\s,]+/).filter(Boolean)
    if (!c || !notes.length) { setMatchMsg('Type the notes you see lit in the frame, like A2 E3.'); return }
    setMatchMsg('Trying every starting note…')
    try {
      const r = await post<{ matches: string[] }>('/api/match', { ...c, t, notes })
      if (!r.matches.length) setMatchMsg('Nothing matched. Check that the box fits the keys and that those keys are lit in this frame.')
      else { setFirst(r.matches[0]); setMatchMsg(r.matches.length > 1 ? `Several fit (${r.matches.join(', ')}). Using ${r.matches[0]}. A frame with more notes lit helps.` : `Matched. The lowest key is ${r.matches[0]}.`) }
    } catch (e) { setMatchMsg(e instanceof Error ? e.message : 'Could not match') }
  }

  async function save() {
    const c = calib()
    if (!c) return
    setErr(''); setJob({ p: 0, s: 'Starting' })
    try {
      const { job: jid } = await post<{ job: string }>('/api/analyze', c)
      await pollJob(jid, (j) => setJob({ p: j.progress, s: j.status ?? '' }))
      reload()
      go(`/song/${id}`)
    } catch (e) { setErr(e instanceof Error ? e.message : 'Something went wrong'); setJob(null) }
  }

  const letter = first[0]
  const octave = parseInt(first.slice(1), 10)
  const setNum = (i: number, v: number) => setBox((b) => { if (!b) return b; const n: Box = [...b]; n[i] = v; return n })

  return (
    <div className="adjust">
      <div className="card stage">
        {!song.ready && (
          <div className="notice" style={{ marginBottom: 12 }}>
            <Alert />
            <div className="grow"><b>Let's set up the keyboard</b>Drag the dashed box so it hugs the piano keys, then press “Make the notes”.</div>
          </div>
        )}
        {box ? <KeyboardCanvas img={img} box={box} nWhite={nWhite} first={first} pressed={pressed} onBox={setBox} />
          : <div className="empty">{detecting ? 'Looking for the keyboard…' : 'Loading the picture…'}</div>}
        <div className="scrub">
          <input type="range" min={0} max={Math.max(1, Math.floor(song.duration ?? 100))} step={0.5} value={t} onChange={(e) => setT(Number(e.target.value))} aria-label="Video time" />
          <span className="time">{mmss(t)}</span>
        </div>
        <div className="row" style={{ marginTop: 8 }}>
          <button className="btn small" onClick={jump} disabled={detecting}>Jump to a frame with notes</button>
          <button className="btn small ghost" onClick={() => setT((x) => Math.max(0, x - 1))}>−1s</button>
          <button className="btn small ghost" onClick={() => setT((x) => x + 1)}>+1s</button>
        </div>
        <div className="chips" aria-live="polite">
          {pressed.length ? <><span className="hint">Lit here:</span>{pressed.map((p) => <span className="chip" key={p.name}><i className="dot" style={{ background: `rgb(${p.rgb})` }} />{p.name.replace('#', '♯')}</span>)}</>
            : <span className="hint">No lit keys in this frame. Use “Jump to a frame with notes”.</span>}
        </div>
        <div className="legend"><span><i style={{ background: 'rgba(70,200,255,.7)' }} />white keys</span><span><i style={{ background: 'rgba(120,60,200,.8)' }} />black keys</span><span>Labels show the note names we will write. They should match the notes you hear.</span></div>
      </div>

      <div className="card pad stack">
        <div>
          <h2>Keyboard</h2>
          <p className="hint">{detMsg || 'Fit the dashed box to the keys, then check the names.'}</p>
          <button className="btn small" style={{ marginTop: 8 }} onClick={autodetect} disabled={detecting}><Refresh /> {detecting ? 'Looking…' : 'Find it again'}</button>
        </div>

        <div className="field">
          <label>Lowest key on screen (far left)</label>
          <div className="row">
            <select className="select" style={{ width: 80 }} value={letter} onChange={(e) => setFirst(e.target.value + octave)} aria-label="Note letter">
              {LETTERS.map((l) => <option key={l}>{l}</option>)}
            </select>
            <div className="stepper-btn" role="group" aria-label="Octave">
              <button onClick={() => setFirst(letter + Math.max(0, octave - 1))} aria-label="Octave down">−</button>
              <output>oct {octave}</output>
              <button onClick={() => setFirst(letter + Math.min(8, octave + 1))} aria-label="Octave up">+</button>
            </div>
          </div>
          <div className="row tight">
            <button className="btn small ghost" onClick={() => setFirst(shiftFirst(first, -1))}>◀ shift names</button>
            <button className="btn small ghost" onClick={() => setFirst(shiftFirst(first, 1))}>shift names ▶</button>
          </div>
        </div>

        <div className="field">
          <label>White keys on screen: {nWhite}</label>
          <div className="row">
            <div className="stepper-btn"><button onClick={() => addLeft(-1)} aria-label="Remove a key on the left">−</button><output>Left</output><button onClick={() => addLeft(1)} aria-label="Add a key on the left">+</button></div>
            <div className="stepper-btn"><button onClick={() => addRight(-1)} aria-label="Remove a key on the right">−</button><output>Right</output><button onClick={() => addRight(1)} aria-label="Add a key on the right">+</button></div>
          </div>
          <p className="hint">Add or remove a key at either end. Key width stays the same.</p>
        </div>

        <details className="more">
          <summary>Tell me the notes you hear</summary>
          <div className="stack">
            <p className="hint">Not sure which octave? Pick a frame with lit keys, type their names (like <b>A2 E3</b>) and we'll fit the labels.</p>
            <div className="row"><input className="input grow" value={match} placeholder="A2 E3" onChange={(e) => setMatch(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && runMatch()} /><button className="btn" onClick={runMatch}>Match</button></div>
            {matchMsg && <p className="hint" role="status">{matchMsg}</p>}
          </div>
        </details>

        <details className="more">
          <summary>Fine-tune</summary>
          <div className="stack">
            {box && (
              <div className="numgrid">
                {(['Left', 'Top', 'Right', 'Bottom'] as const).map((l, i) => (
                  <div className="field" key={l}><label>{l}</label>
                    <input className="input" type="number" value={Math.round(box[i])} onChange={(e) => setNum(i, Number(e.target.value))} /></div>
                ))}
              </div>
            )}
            <div className="field"><label>Sensitivity: {thr}</label>
              <input type="range" min={10} max={90} value={thr} onChange={(e) => setThr(Number(e.target.value))} />
              <p className="hint">Lower catches fainter keys. Higher ignores flicker.</p></div>
          </div>
        </details>

        {err && <div className="notice bad" role="alert"><Alert /><div className="grow">{err}</div></div>}
        <div className="sticky-actions stack">
          {job && <div><div className="progress" style={{ marginTop: 0 }}><i style={{ width: `${Math.max(3, job.p * 100)}%` }} /></div><p className="hint" style={{ marginTop: 6 }}>{job.s}</p></div>}
          <button className="btn primary big" onClick={save} disabled={!box || !!job}>{song.ready ? 'Save & redo the notes' : 'Make the notes'}</button>
        </div>
      </div>
    </div>
  )
}
