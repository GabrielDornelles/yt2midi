import { useEffect, useMemo, useRef, useState } from 'react'
import { get, pollJob, post, qs } from '../lib/api'
import type { Song } from '../types'
import { usePlayer } from '../lib/player'
import { saveTiming } from '../lib/timing'
import type { NoteRec } from '../types'
import { Alert, ChevL, ChevR, Download, Gear, Pause, Play } from './icons'

interface Prefs { size: string; kindleSize: string; rotate: 'cw' | 'ccw' | 'none'; mpp: number; mpr: number; spp: number; dot: number; flat: boolean; octave: boolean; ticks: boolean; chords: boolean; holds: boolean; link: boolean; lineSpace: number; beats: number }
const DEFAULTS: Prefs = { size: '1680x1264', kindleSize: '1680x1264', rotate: 'cw', mpp: 4, mpr: 4, spp: 1, dot: 56, flat: false, octave: false, ticks: true, chords: false, holds: true, link: true, lineSpace: 1, beats: 3 }
const SIZES: [string, string][] = [
  ['1680x1264', 'Kindle Paperwhite 7″'],
  ['1648x1236', 'Kindle Paperwhite 6.8″'],
  ['1448x1072', 'Kindle (basic)'],
  ['2480x1860', 'Kindle Scribe'],
  ['a4print', 'A4 for printing (portrait)'],
]

type Fmt = 'pdf' | 'epub'
interface ExportState { phase: 'idle' | 'running' | 'done' | 'error'; progress: number; status: string; key?: string; file?: string; name?: string; pages?: number; mb?: number; error?: string }
const IDLE: ExportState = { phase: 'idle', progress: 0, status: '' }

interface Auto { bpm: number; t_start: number; t_downbeat: number; measures: number; pages: number; notes: number }

function usePrefs(): [Prefs, (p: Partial<Prefs>) => void] {
  const [prefs, set] = useState<Prefs>(() => {
    try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem('sheetPrefs') ?? '{}') } } catch { return DEFAULTS }
  })
  const update = (p: Partial<Prefs>) => set((cur) => {
    const next = { ...cur, ...p }
    try { localStorage.setItem('sheetPrefs', JSON.stringify(next)) } catch { /* ignore */ }
    return next
  })
  return [prefs, update]
}

/** One measure on the page: song time it starts and ends, and its rectangle in image pixels. */
type Box = [number, number, number, number, number, number]
/** One note dot: when it starts and ends sounding (ms) and where its centre is, in image pixels. */
type Dot = [number, number, number, number]

/** Fetches the page image; keeps showing the previous one (dimmed) until the next is ready. */
function useStableImage(url: string) {
  const [shown, setShown] = useState('')
  const [scale, setScale] = useState(1)
  const [boxes, setBoxes] = useState<Box[]>([])
  const [dots, setDots] = useState<Dot[]>([])
  const [dotR, setDotR] = useState(0)
  const [loading, setLoading] = useState(true)
  const [failed, setFailed] = useState(false)
  const current = useRef('')
  useEffect(() => {
    if (!url) return
    const ac = new AbortController()
    setLoading(true); setFailed(false)
    fetch(url, { signal: ac.signal })
      .then(async (r) => {
        if (!r.ok) throw new Error(String(r.status))
        const sc = Number(r.headers.get('X-Sheet-Scale') ?? '1')
        let bx: Box[] = []
        try { bx = JSON.parse(r.headers.get('X-Sheet-Boxes') ?? '[]') } catch { /* no boxes: playback from a click is just unavailable */ }
        let dt: Dot[] = []
        try { dt = JSON.parse(r.headers.get('X-Sheet-Notes') ?? '[]') } catch { /* no dots: notes just won't light up */ }
        const dr = Number(r.headers.get('X-Sheet-Dot') ?? '0')
        const obj = URL.createObjectURL(await r.blob())
        if (current.current) URL.revokeObjectURL(current.current)
        current.current = obj
        setShown(obj); setScale(sc); setBoxes(bx); setDots(dt); setDotR(dr); setLoading(false)
      })
      .catch(() => { if (!ac.signal.aborted) { setFailed(true); setLoading(false) } })
    return () => ac.abort()
  }, [url])
  useEffect(() => () => { if (current.current) URL.revokeObjectURL(current.current) }, [])
  return { shown, loading, failed, scale, boxes, dots, dotR }
}

export function Sheet({ song }: { song: Song }) {
  const [prefs, setPrefs] = usePrefs()
  const [bpm, setBpm] = useState('')
  const [start, setStart] = useState('')
  const [tdown, setTdown] = useState<number | null>(null)
  const [info, setInfo] = useState<Auto | null>(null)
  const [page, setPage] = useState(0)
  const [open, setOpen] = useState(false)
  const [showExport, setShowExport] = useState(false)
  const [err, setErr] = useState('')
  const tdownRef = useRef(tdown)
  tdownRef.current = tdown
  const [exp, setExp] = useState<Record<Fmt, ExportState>>({ pdf: IDLE, epub: IDLE })
  const [notes, setNotes] = useState<NoteRec[]>([])
  const [nat, setNat] = useState<{ w: number; h: number } | null>(null)
  const lastTurn = useRef(0)

  const print = prefs.size === 'a4print'
  const [w, h] = print ? [2480, 3508] : prefs.size.split('x').map(Number)
  const params = (extra: Record<string, string | number | boolean | null> = {}) => qs({
    video_id: song.video_id, beats: prefs.beats, mpp: prefs.mpp, spp: prefs.spp, w, h, dot: prefs.dot,
    acc: prefs.flat ? 'flat' : 'sharp', octave: prefs.octave, ticks: prefs.ticks, rotate: prefs.rotate, print, mpr: prefs.mpr, chords: prefs.chords, lsp: prefs.lineSpace, holds: prefs.holds, link: prefs.link,
    bpm, t_start: start, t_downbeat: tdown, ...extra,
  })

  // estimate tempo and bar lines whenever the timing inputs change (debounced)
  useEffect(() => {
    const ac = new AbortController()
    const id = window.setTimeout(() => {
      const q = qs({ video_id: song.video_id, beats: prefs.beats, mpp: prefs.mpp, spp: prefs.spp, w, h, dot: prefs.dot, print, mpr: prefs.mpr,
        bpm, t_start: start, t_downbeat: tdownRef.current })
      get<Auto>(`/api/sheet/auto?${q}`, ac.signal)
        .then((a) => {
          setInfo(a); setErr('')
          saveTiming(song.video_id, { beats: prefs.beats, bpm: a.bpm, tdown: tdownRef.current ?? a.t_downbeat })
          setTdown((cur) => (cur === null ? a.t_downbeat : cur))
          setBpm((cur) => (cur === '' ? String(a.bpm) : cur))
          setPage((p) => Math.min(p, a.pages - 1))
        })
        .catch((e: Error) => { if (!ac.signal.aborted) setErr(e.message) })
    }, 350)
    return () => { window.clearTimeout(id); ac.abort() }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [song.video_id, prefs.beats, prefs.mpp, prefs.mpr, prefs.spp, print, bpm, start])

  const url = useMemo(() => `/api/sheet/page.png?${params({ page })}`,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [song.video_id, prefs, bpm, start, tdown, page])
  const { shown, loading, failed, scale, boxes, dots, dotR } = useStableImage(tdown === null ? '' : url)
  const pages = info?.pages ?? 1

  // listening: play from this page, or from any measure you click
  useEffect(() => {
    const ac = new AbortController()
    get<NoteRec[]>(`/api/song/${song.video_id}/notes`, ac.signal).then(setNotes).catch(() => setNotes([]))
    return () => ac.abort()
  }, [song.video_id])
  const player = usePlayer(notes, song.video_id)
  const pageStart = boxes.length ? Math.min(...boxes.map((b) => b[0])) : null
  const pageEnd = boxes.length ? Math.max(...boxes.map((b) => b[1])) : null
  const heard = player.playing || player.pos > 0
  // notes sounding right now (a very short note still flashes for a moment so it can be followed)
  const lit = heard && nat ? dots.filter(([a, b]) => player.pos * 1000 >= a && player.pos * 1000 < Math.max(b, a + 160)) : []
  const current = heard ? boxes.find((b) => player.pos >= b[0] && player.pos < b[1]) : undefined
  const clickPaper = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!nat) return
    const r = e.currentTarget.getBoundingClientRect()
    const x = ((e.clientX - r.left) / r.width) * nat.w, y = ((e.clientY - r.top) / r.height) * nat.h
    const hit = boxes.find((b) => x >= b[2] && x <= b[4] && y >= b[3] && y <= b[5])
    if (hit) player.playFrom(hit[0])
  }
  // while playing, turn to the next page when the music reaches the end of this one
  useEffect(() => {
    if (player.playing && pageEnd !== null && player.pos >= pageEnd - 0.02 && pageEnd !== lastTurn.current) {
      lastTurn.current = pageEnd
      setPage((p) => Math.min(pages - 1, p + 1))
    }
  }, [player.pos, player.playing, pageEnd, pages])

  // everything an export depends on; if it changes after an export, that export is out of date
  const exportBody = (format: Fmt) => {
    const [kw, kh] = prefs.kindleSize.split('x').map(Number)
    return {
      video_id: song.video_id, format, beats: prefs.beats, bpm: Number(bpm) || null,
      t_start: start === '' ? null : Number(start), t_downbeat: tdown, mpp: prefs.mpp, spp: prefs.spp,
      w: kw, h: kh, dot: prefs.dot, acc: prefs.flat ? 'flat' : 'sharp', octave: prefs.octave, ticks: prefs.ticks,
      rotate: prefs.rotate === 'ccw' ? 'ccw' : 'cw', mpr: prefs.mpr, chords: prefs.chords, lsp: prefs.lineSpace, holds: prefs.holds, link: prefs.link,
    }
  }
  async function startExport(format: Fmt) {
    const body = exportBody(format)
    const key = JSON.stringify(body)
    setExp((e) => ({ ...e, [format]: { phase: 'running', progress: 0, status: 'Starting…' } }))
    try {
      const { job } = await post<{ job: string }>('/api/sheet/export', body)
      const r = await pollJob<{ file: string; name: string; pages: number; mb: number }>(job, (j) =>
        setExp((e) => ({ ...e, [format]: { phase: 'running', progress: j.progress, status: j.status ?? '' } })))
      setExp((e) => ({ ...e, [format]: { phase: 'done', progress: 1, status: '', key, ...r } }))
    } catch (ex) {
      setExp((e) => ({ ...e, [format]: { phase: 'error', progress: 0, status: '', error: ex instanceof Error ? ex.message : 'Export failed' } }))
    }
  }
  const beatSec = 60 / (Number(bpm) || 100)

  const retime = (patch: { bpm?: string; start?: string; beats?: number }) => {
    setTdown(null); setPage(0)
    if (patch.bpm !== undefined) setBpm(patch.bpm)
    if (patch.start !== undefined) { setStart(patch.start); setBpm('') }
    if (patch.beats !== undefined) { setPrefs({ beats: patch.beats }); setBpm('') }
  }

  return (
    <div className={`sheet-layout ${open ? '' : 'nopanel'}`}>
      <div className="card" style={{ overflow: 'hidden' }}>
        <div className="sheet-bar">
          <button className="btn icon" onClick={() => setPage((p) => Math.max(0, p - 1))} disabled={page === 0} aria-label="Previous page"><ChevL /></button>
          <span className="page-no">Page {page + 1} of {pages}</span>
          <button className="btn icon" onClick={() => setPage((p) => Math.min(pages - 1, p + 1))} disabled={page >= pages - 1} aria-label="Next page"><ChevR /></button>
          <button className="btn" onClick={() => (player.playing ? player.toggle() : pageStart !== null && player.playFrom(pageStart))} disabled={pageStart === null && !player.playing} aria-pressed={player.playing}>
            {player.playing ? <Pause /> : <Play />} {player.playing ? 'Pause' : 'Play this page'}
          </button>
          <span className="grow" />
          <button className="btn" onClick={() => setShowExport((o) => !o)} aria-expanded={showExport}><Download /> Export whole song</button>
          <button className="btn" onClick={() => setOpen((o) => !o)} aria-expanded={open}><Gear /> Page settings</button>
          <a className="btn primary" href={`/api/sheet/page.png?${params({ page, download: true })}`} download>
            <Download /> Save page
          </a>
        </div>
        {!loading && scale < (print ? 0.3 : 0.5) && (
          <div className="notice" style={{ margin: '12px 14px 0' }}>
            <div className="grow">
              <b>{print ? "This song is busy" : "This page is crowded"}</b>
              The notes were shrunk to {Math.round(scale * 100)}% to fit{print ? " (the same on every page)" : ""}. For bigger notes, show fewer measures per {print ? "row" : "page"} or lower the note size in Page settings.
            </div>
            <button className="btn small" onClick={() => setOpen(true)}>Page settings</button>
          </div>
        )}
        {(showExport || exp.epub.phase === 'running' || exp.pdf.phase === 'running') && (
        <div className="export-panel">
        <div className="exports">
          {(['epub', 'pdf'] as Fmt[]).map((f) => {
            const st = exp[f]
            const fresh = st.phase === 'done' && st.key === JSON.stringify(exportBody(f))
            const kindleName = SIZES.find(([v]) => v === prefs.kindleSize)?.[1] ?? 'Kindle'
            return (
              <div className="export" key={f}>
                <h3>{f === 'epub' ? 'Kindle book (EPUB)' : 'PDF to print (A4)'}</h3>
                <p className="hint">
                  {f === 'epub'
                    ? `Made for the ${kindleName}. Pages are turned so you hold the Kindle sideways, turned to the left. Send it with Amazon's Send to Kindle, or copy it over USB.`
                    : 'Portrait A4, 300 dpi: one note size for the whole song, as many rows per page as fit.'}
                </p>
                {st.phase === 'running' ? (
                  <div>
                    <div className="progress" style={{ marginTop: 0 }}><i style={{ width: `${Math.max(3, st.progress * 100)}%` }} /></div>
                    <p className="hint" style={{ marginTop: 6 }}>{st.status}</p>
                  </div>
                ) : fresh ? (
                  <div className="row">
                    <a className="btn primary" href={st.file} download={st.name}><Download /> Download {f === 'epub' ? 'EPUB' : 'PDF'}</a>
                    <span className="hint">{st.pages} pages · {st.mb} MB</span>
                  </div>
                ) : (
                  <div className="row">
                    <button className="btn" onClick={() => startExport(f)} disabled={tdown === null}>
                      {st.phase === 'done' ? 'Settings changed: make it again' : f === 'epub' ? 'Make the Kindle book' : 'Make the PDF'}
                    </button>
                  </div>
                )}
                {st.phase === 'error' && <div className="notice bad" role="alert"><Alert /><div className="grow">{st.error}</div></div>}
              </div>
            )
          })}
        </div>
        <p className="hint" style={{ marginTop: 12 }}>
          Bar lines follow the tempo near your start time, so songs that change tempo can drift in later pages. We'll improve this.
        </p>
        </div>
        )}
        {boxes.length > 0 && <p className="hint" style={{ margin: '10px 14px 0' }}>Tip: click any measure on the page to hear it from there.</p>}
        <div className="paper-stage">
          {err ? <div className="notice bad">{err}</div> : failed ? <div className="notice bad">This page could not be drawn.</div> : shown ? (
            <div className="paper-wrap" onClick={clickPaper} style={{ cursor: boxes.length ? 'pointer' : 'default' }}>
              <img className={`paper ${loading ? 'loading' : ''}`} src={shown} alt={`Sheet page ${page + 1}`}
                onLoad={(e) => setNat({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })} />
              {nat && dotR > 0 && lit.map(([a, , x, y]) => (
                <div key={`${a}-${x}-${y}`} className="note-lit" style={{
                  left: `${((x - dotR) / nat.w) * 100}%`, top: `${((y - dotR) / nat.h) * 100}%`,
                  width: `${((dotR * 2) / nat.w) * 100}%`, height: `${((dotR * 2) / nat.h) * 100}%`,
                }} />
              ))}
              {current && nat && (
                <div className="measure-hl" style={{
                  left: `${(current[2] / nat.w) * 100}%`, top: `${(current[3] / nat.h) * 100}%`,
                  width: `${((current[4] - current[2]) / nat.w) * 100}%`, height: `${((current[5] - current[3]) / nat.h) * 100}%`,
                }} />
              )}
            </div>
          ) : <div className="hint">Drawing page…</div>}
        </div>
      </div>

      {open && (
        <aside className="card panel stack" aria-label="Page settings">
          <div className="field">
            <label htmlFor="size">Your reader</label>
            <select id="size" className="select" value={prefs.size} onChange={(e) => setPrefs(e.target.value === 'a4print' ? { size: e.target.value } : { size: e.target.value, kindleSize: e.target.value })}>
              {SIZES.map(([v, n]) => <option key={v} value={v}>{n}</option>)}
            </select>
          </div>
          {!print && <div className="field">
            <label htmlFor="rot">Turn the page</label>
            <select id="rot" className="select" value={prefs.rotate} onChange={(e) => setPrefs({ rotate: e.target.value as Prefs['rotate'] })}>
              <option value="cw">Rotated ⟳ (hold the Kindle sideways, turn it left)</option>
              <option value="ccw">Rotated ⟲ (hold the Kindle sideways, turn it right)</option>
              <option value="none">Not rotated</option>
            </select>
            <p className="kindle-note">Rotated pages fill a Kindle's screen when you hold it in landscape.</p>
          </div>}
          <div className="field">
            <label htmlFor="dot">Note size: {prefs.dot}</label>
            <input id="dot" type="range" min={30} max={90} value={prefs.dot} onChange={(e) => setPrefs({ dot: Number(e.target.value) })} />
          </div>
          <div className="field">
            <label htmlFor="lsp">Space between lines: {prefs.lineSpace <= 1.05 ? 'normal' : prefs.lineSpace >= 1.8 ? 'wide' : 'a bit wider'}</label>
            <input id="lsp" type="range" min={1} max={2.2} step={0.1} value={prefs.lineSpace} onChange={(e) => setPrefs({ lineSpace: Number(e.target.value) })} />
            <p className="kindle-note">Wider lines give neighbouring notes room, so chords like D♭ + E stack in one column instead of sitting side by side. Note size does not change.</p>
          </div>
          {print ? (
            <div className="field">
              <label htmlFor="mpr">Measures per row</label>
              <div className="stepper-btn">
                <button onClick={() => { setPrefs({ mpr: Math.max(2, prefs.mpr - 1) }); setPage(0) }} aria-label="Fewer measures per row">−</button>
                <output id="mpr">{prefs.mpr}</output>
                <button onClick={() => { setPrefs({ mpr: Math.min(8, prefs.mpr + 1) }); setPage(0) }} aria-label="More measures per row">+</button>
              </div>
              <p className="kindle-note">Notes are the same size on every page, and each row holds as many measures as fit (up to this number). If your notes look small, the song is busy: pick fewer measures per row. Saved at 300 dpi, so it prints at true A4 size.</p>
            </div>
          ) : (
            <div className="field">
              <label htmlFor="mpp">Measures per page</label>
              <div className="row">
                <div className="stepper-btn">
                  <button onClick={() => { setPrefs({ mpp: Math.max(1, prefs.mpp - 1) }); setPage(0) }} aria-label="Fewer measures">−</button>
                  <output id="mpp">{prefs.mpp}</output>
                  <button onClick={() => { setPrefs({ mpp: Math.min(8, prefs.mpp + 1) }); setPage(0) }} aria-label="More measures">+</button>
                </div>
                <span className="hint">in {prefs.spp} row{prefs.spp > 1 ? 's' : ''}</span>
                <button className="btn small ghost" onClick={() => { setPrefs({ spp: prefs.spp === 1 ? 2 : 1 }); setPage(0) }}>{prefs.spp === 1 ? 'Use 2 rows' : 'Use 1 row'}</button>
              </div>
            </div>
          )}
          <label className="check"><input type="checkbox" checked={prefs.flat} onChange={(e) => setPrefs({ flat: e.target.checked })} /> Write black keys as flats (E♭ instead of D♯)</label>
          <label className="check"><input type="checkbox" checked={prefs.octave} onChange={(e) => setPrefs({ octave: e.target.checked })} /> Show octave numbers</label>
          <label className="check"><input type="checkbox" checked={prefs.ticks} onChange={(e) => setPrefs({ ticks: e.target.checked })} /> Show beat marks</label>
          <label className="check"><input type="checkbox" checked={prefs.holds} onChange={(e) => setPrefs({ holds: e.target.checked })} /> Show long notes as a curve (hold)</label>
          <p className="kindle-note">A long note gets a curved line to a pale (not filled) copy of the same note: keep the key down until that pale note, then let go. If the curve ends in an arrow, keep holding into the next row.</p>
          <label className="check"><input type="checkbox" checked={prefs.link} onChange={(e) => setPrefs({ link: e.target.checked })} /> Join notes both hands play together</label>
          <div className="field">
            <label className="check"><input type="checkbox" checked={prefs.chords} onChange={(e) => setPrefs({ chords: e.target.checked })} /> Tidy up rolled chords</label>
            <p className="kindle-note">When the player hits one note of a chord a hair early, as it really happens in the video, the sheet shows a staggered chord. Tick this to draw such notes as one chord. If two neighbouring notes have to sit side by side, they share a grey capsule and are centred on the beat, so none looks like it comes later. The MIDI keeps the real timing.</p>
          </div>

          <details className="more">
            <summary>Timing</summary>
            <div className="stack">
              <p className="hint">Bars are placed from the tempo near the start time. If chords don't land at the start of a bar, nudge the bars or fix the tempo.</p>
              <div className="field">
                <label htmlFor="beats">Beats per bar</label>
                <select id="beats" className="select" value={prefs.beats} onChange={(e) => retime({ beats: Number(e.target.value) })}>
                  <option value={2}>2 (march)</option><option value={3}>3 (waltz)</option><option value={4}>4 (most songs)</option><option value={6}>6 (slow compound)</option>
                </select>
              </div>
              <div className="row">
                <div className="field grow"><label htmlFor="bpm">Tempo (BPM)</label>
                  <input id="bpm" className="input" inputMode="decimal" value={bpm} onChange={(e) => retime({ bpm: e.target.value })} /></div>
                <div className="field grow"><label htmlFor="start">Start at (seconds)</label>
                  <input id="start" className="input" inputMode="decimal" placeholder={info ? String(info.t_start) : ''} value={start} onChange={(e) => retime({ start: e.target.value })} /></div>
              </div>
              <div className="row">
                <button className="btn small" onClick={() => setTdown((t) => (t === null ? t : t - beatSec))}>Bars one beat earlier</button>
                <button className="btn small" onClick={() => setTdown((t) => (t === null ? t : t + beatSec))}>Bars one beat later</button>
              </div>
              <p className="hint">Songs that change tempo: start again from where the steady part begins.</p>
            </div>
          </details>
        </aside>
      )}
    </div>
  )
}
