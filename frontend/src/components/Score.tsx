import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { TimemapEntry, VerovioToolkit } from 'verovio/esm'
import { get, qs } from '../lib/api'
import { usePlayer } from '../lib/player'
import { readTiming } from '../lib/timing'
import type { NoteRec, Song } from '../types'
import { Download, Pause, Play } from './icons'

interface ScoreData { xml: string; beat_times: number[]; fifths: number; minor: boolean; measures: number; bpm: number; beats: number; t_downbeat: number; detected: { fifths: number; minor: boolean } }

const MAJOR = ['C♭', 'G♭', 'D♭', 'A♭', 'E♭', 'B♭', 'F', 'C', 'G', 'D', 'A', 'E', 'B', 'F♯', 'C♯']
const MINOR = ['A♭', 'E♭', 'B♭', 'F', 'C', 'G', 'D', 'A', 'E', 'B', 'F♯', 'C♯', 'G♯', 'D♯', 'A♯']
const keyName = (f: number, minor: boolean) => `${(minor ? MINOR : MAJOR)[Math.max(-7, Math.min(7, f)) + 7]} ${minor ? 'minor' : 'major'}`

/** Verovio (the engraver) is a few megabytes of WebAssembly: it is fetched the first time a score is opened. */
let toolkit: Promise<VerovioToolkit> | null = null
function getToolkit(): Promise<VerovioToolkit> {
  toolkit ??= Promise.all([import('verovio/wasm'), import('verovio/esm')]).then(async ([wasm, esm]) => new esm.VerovioToolkit(await wasm.default()))
  return toolkit
}

/** A video title makes a poor sheet title: drop the [tags], the "| channel" tail and the word tutorial. */
const cleanTitle = (t: string) => {
  const noise = /\b(sheet music|piano tutorial|tutorial|synthesia|midi|piano cover|cover|lesson)\b/i
  const c = t.replace(/\[[^\]]*\]/g, ' ').split('|')[0]
    .split(/\s+[-–—]\s+/).filter((part) => !noise.test(part)).join(' - ') // "… - with Sheet Music" and the like are not part of the title
    .replace(/\((synthesia|piano cover|sheet music)[^)]*\)/gi, ' ').replace(/\s+/g, ' ').replace(/^[\s\-–—:]+|[\s\-–—:]+$/g, '')
  return c || t.trim()
}

/** What is written at the top of the first page. A null subtitle means "Key of …"; an empty one means none. */
interface Meta { title: string; subtitle: string | null; composer: string; arranger: string }

/** Each mark that can be hidden, said in plain words: what it looks like and what it asks of the player. */
const MARKS: { key: string; label: string; what: string }[] = [
  { key: 'rests', label: 'Rests', what: 'The little squiggles and bars sitting in the staff. They mean: stay silent for that long.' },
  { key: 'ties', label: 'Ties', what: 'A curved line joining two notes of the same pitch. Play the first and hold it through the second; do not play it again.' },
  { key: 'dots', label: 'Dots after notes', what: 'A dot right after a note makes it half as long again.' },
  { key: 'staccato', label: 'Staccato dots', what: 'A dot above or below a note (not after it). Play it short and detached, then let go quickly.' },
  { key: 'arpeg', label: 'Rolled-chord squiggles', what: 'A wavy vertical line beside a chord: play its notes quickly one after another, from the bottom up (or top down), not all at once.' },
  { key: 'triplets', label: 'Triplet numbers', what: 'A small 3 over a group of notes: three notes squeezed into the time of two.' },
  { key: 'accidentals', label: 'Sharps, flats and naturals', what: 'The ♯ ♭ ♮ signs before a note. ♯ is the next key up, ♭ the next key down, ♮ cancels one.' },
  { key: 'barnums', label: 'Bar numbers', what: 'The small numbers above the staff. They match the numbers on the easy sheet.' },
]
type Show = Record<string, boolean>
const SHOW_ALL: Show = Object.fromEntries([...MARKS.map((m) => [m.key, true]), ['follow', false]])

/**
 * Verovio draws every line of the title block at one size. A sheet has a big title, a smaller line under it and
 * smaller names at the side, so the sizes are set here, and the lines below the title are moved down to make room.
 */
const PRINT_ROOM = 250 // the same for the printed page, which has less to spare
const HEADER_ROOM = 440 // how far the music is moved down to make room for the larger title (page units)
function sizeHeader(root: Element, room = HEADER_ROOM) {
  const lines = Array.from(root.querySelectorAll('g.pgHead > text'))
  if (!lines.length) return
  lines.forEach((t, i) => {
    const f = i === 0 ? 1.55 : i === 1 ? 1.0 : 0.8
    const rend = t.querySelector('tspan.rend')
    const inner = t.querySelector('tspan.rend tspan.text > tspan')
    if (!rend || !inner) return
    const size = parseFloat(inner.getAttribute('font-size') ?? '405')
    inner.setAttribute('font-size', `${Math.round(size * f)}px`)
    if (i === 0) {
      // a long title is made smaller until it fits between the margins (the composer sits to its right)
      const vb = (root.querySelector('svg.definition-scale') ?? root.querySelector('svg'))?.getAttribute('viewBox')?.split(/\s+/).map(Number)
      const room = (vb?.[2] ?? 22440) * 0.7
      let px = Math.round(size * f)
      for (let k = 0; k < 14 && (inner as unknown as SVGTSpanElement).getComputedTextLength?.() > room; k++) {
        px = Math.round(px * 0.92)
        inner.setAttribute('font-size', `${px}px`)
      }
    }
    const y = parseFloat(rend.getAttribute('y') ?? '0')
    rend.setAttribute('y', String(Math.round(i === 0 ? y + size * (f - 1) * 0.55 : i === 1 ? y + 250 : y)))
  })
  root.querySelectorAll('g.system').forEach((g) => g.setAttribute('transform', `translate(0,${room})`))
}

export function Score({ song }: { song: Song }) {
  const id = song.video_id
  const [data, setData] = useState<ScoreData | null>(null)
  const [err, setErr] = useState('')
  const [meta, setMeta] = useState<Meta>(() => {
    try { return { title: cleanTitle(song.title), subtitle: null, composer: '', arranger: '', ...JSON.parse(localStorage.getItem(`scoreMeta:${id}`) ?? '{}') } } catch { return { title: cleanTitle(song.title), subtitle: null, composer: '', arranger: '' } }
  })
  const [show, setShow] = useState<Show>(() => { try { return { ...SHOW_ALL, ...JSON.parse(localStorage.getItem('scoreShow') ?? '{}') } } catch { return SHOW_ALL } })
  useEffect(() => { try { localStorage.setItem(`scoreMeta:${id}`, JSON.stringify(meta)) } catch { /* not remembered */ } }, [meta, id])
  useEffect(() => { try { localStorage.setItem('scoreShow', JSON.stringify(show)) } catch { /* not remembered */ } }, [show])
  // the shortest hold (in beats) that is still written as a hold; shorter notes end where the next one starts
  const [hold, setHold] = useState(() => { try { const v = localStorage.getItem('scoreHold'); return v === null ? 2 : Number(v) } catch { return 2 } })
  useEffect(() => { try { localStorage.setItem('scoreHold', String(hold)) } catch { /* not remembered */ } }, [hold])
  // how the bars are found: automatically, with a fixed number of beats, or the same as the easy sheet
  const [bars, setBars] = useState<string>(() => { try { return localStorage.getItem('scoreBars') ?? 'auto' } catch { return 'auto' } })
  useEffect(() => { try { localStorage.setItem('scoreBars', bars) } catch { /* not remembered */ } }, [bars])
  // the key the sheet is written in: 'auto' (what the notes suggest) or e.g. '-4:major' (A flat major)
  const [key, setKey] = useState<string>(() => { try { return localStorage.getItem(`scoreKey:${id}`) ?? 'auto' } catch { return 'auto' } })
  useEffect(() => { try { localStorage.setItem(`scoreKey:${id}`, key) } catch { /* not remembered */ } }, [key, id])
  const found = useRef<{ mode: string; beats: number; bpm: number; t_downbeat: number } | null>(null) // the bars the search settled on
  const [notes, setNotes] = useState<NoteRec[]>([])
  const [scale, setScale] = useState(() => { try { return Number(localStorage.getItem('scoreScale')) || 45 } catch { return 45 } })
  const [drawn, setDrawn] = useState(false)
  const [failed, setFailed] = useState('')
  const box = useRef<HTMLDivElement>(null)
  const timemap = useRef<TimemapEntry[]>([])
  const lit = useRef({ idx: -1, on: new Set<string>() })

  useEffect(() => {
    const ac = new AbortController()
    get<NoteRec[]>(`/api/song/${id}/notes`, ac.signal).then(setNotes).catch(() => setNotes([]))
    return () => ac.abort()
  }, [id])

  // the score is written again when the title page changes (typing waits a moment)
  useEffect(() => {
    const ac = new AbortController()
    const timer = window.setTimeout(() => {
      const t = readTiming(id)
      // the bars are searched for once; editing the title or the holds afterwards keeps the bars that were found
      const keep = found.current?.mode === bars ? found.current : null
      const timing = bars === 'sheet'
        ? { bars: 'sheet', beats: t.beats, bpm: t.bpm, t_downbeat: t.tdown }
        : keep ? { bars: 'sheet', beats: keep.beats, bpm: keep.bpm, t_downbeat: keep.t_downbeat }
        : bars === 'auto' ? { bars: 'auto', beats: t.beats, bpm: t.bpm, t_downbeat: t.tdown }
        : { bars: 'fixed', beats: Number(bars), bpm: t.bpm, t_downbeat: t.tdown }
      get<ScoreData>(`/api/score?${qs({ video_id: id, ...timing, title: meta.title, subtitle: meta.subtitle === null ? undefined : meta.subtitle || ' ', composer: meta.composer, arranger: meta.arranger, hold, key: key === 'auto' ? undefined : key.replace(':', ',') })}`, ac.signal)
        .then((d) => { found.current = { mode: bars, beats: d.beats, bpm: d.bpm, t_downbeat: d.t_downbeat }; setData(d); setErr('') }).catch((e: Error) => { if (!ac.signal.aborted) setErr(e.message) })
    }, data ? 600 : 0)
    return () => { window.clearTimeout(timer); ac.abort() }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, meta, hold, bars, key])

  const player = usePlayer(notes, id)

  // quarter-note position <-> time in the video, through the beat times worked out for the score
  const qAt = useCallback((t: number) => {
    const b = data?.beat_times
    if (!b || b.length < 2) return 0
    if (t <= b[0]) return (t - b[0]) / (b[1] - b[0])
    let lo = 0, hi = b.length
    while (lo < hi) { const m = (lo + hi) >> 1; if (b[m] <= t) lo = m + 1; else hi = m }
    const i = Math.min(lo - 1, b.length - 2)
    return i + (t - b[i]) / (b[i + 1] - b[i])
  }, [data])
  const timeAt = useCallback((q: number) => {
    const b = data?.beat_times
    if (!b || b.length < 2) return 0
    const i = Math.max(0, Math.min(Math.floor(q), b.length - 2))
    return b[i] + (q - i) * (b[i + 1] - b[i])
  }, [data])

  const draw = useCallback(async (width: number) => {
    if (!data || !box.current) return
    try {
      const tk = await getToolkit()
      tk.setOptions({
        scale, pageWidth: Math.round((width * 100) / scale), pageHeight: 60000, adjustPageHeight: true, svgViewBox: true,
        header: 'auto', footer: 'none', breaks: 'auto', mnumInterval: 4,
        pageMarginLeft: 40, pageMarginRight: 40, pageMarginTop: 60, pageMarginBottom: Math.min(500, 40 + HEADER_ROOM), spacingStaff: 9, spacingSystem: 10,
      })
      if (!tk.loadData(data.xml)) throw new Error(tk.getLog().slice(0, 300) || 'The score could not be read')
      box.current.innerHTML = tk.renderToSVG(1)
      sizeHeader(box.current)
      timemap.current = tk.renderToTimemap({ includeRests: false })
      lit.current = { idx: -1, on: new Set() }
      setFailed('')
      setDrawn(true)
    } catch (e) {
      setFailed(e instanceof Error ? e.message : 'The score could not be drawn')
    }
  }, [data, scale])

  // draw when the score arrives, and again when the window or the size changes
  useEffect(() => {
    const el = box.current
    if (!el || !data) return
    let timer = 0
    let last = 0
    const run = () => { const w = el.clientWidth; if (w > 0 && Math.abs(w - last) > 2) { last = w; void draw(w) } }
    run()
    const ro = new ResizeObserver(() => { window.clearTimeout(timer); timer = window.setTimeout(run, 250) })
    ro.observe(el)
    return () => { ro.disconnect(); window.clearTimeout(timer) }
  }, [data, draw])
  useEffect(() => { try { localStorage.setItem('scoreScale', String(scale)) } catch { /* not remembered */ } }, [scale])

  // light the notes that are sounding
  const toggle = (ids: Iterable<string>, on: boolean) => { for (const i of ids) document.getElementById(i)?.classList.toggle('playing', on) }
  useEffect(() => {
    const tm = timemap.current
    if (!drawn || !tm.length) return
    const q = qAt(player.pos)
    let lo = 0, hi = tm.length
    while (lo < hi) { const m = (lo + hi) >> 1; if (tm[m].qstamp <= q + 1e-6) lo = m + 1; else hi = m }
    const idx = lo - 1
    const st = lit.current
    if (idx === st.idx) return
    if (idx < st.idx) { toggle(st.on, false); st.on.clear(); st.idx = -1 }
    let first: string | null = null
    for (let i = st.idx + 1; i <= idx; i++) {
      for (const n of tm[i].off ?? []) if (st.on.delete(n)) toggle([n], false)
      for (const n of tm[i].on ?? []) { st.on.add(n); toggle([n], true); first ??= n }
    }
    st.idx = idx
    // following the music down the page is only when asked for, and never while the reader is scrolling by hand
    if (first && player.playing && followRef.current && performance.now() - lastUser.current > 4000) {
      const r = document.getElementById(first)?.getBoundingClientRect()
      if (r && (r.top < 120 || r.bottom > window.innerHeight - 120)) document.getElementById(first)?.scrollIntoView({ block: 'center', behavior: 'smooth' })
    }
  }, [player.pos, player.playing, drawn, qAt])
  const followRef = useRef(false)
  followRef.current = !!show.follow
  const lastUser = useRef(0)
  useEffect(() => {
    const on = () => { lastUser.current = performance.now() }
    const evs = ['wheel', 'touchmove', 'keydown', 'pointerdown'] as const
    evs.forEach((e) => window.addEventListener(e, on, { passive: true }))
    return () => evs.forEach((e) => window.removeEventListener(e, on))
  }, [])
  const hidden = MARKS.filter((m) => !show[m.key]).map((m) => `hide-${m.key}`).join(' ')

  // click a note to play from there
  const onClick = (e: React.MouseEvent) => {
    const g = (e.target as Element).closest?.('g.note, g.rest, g.chord')
    if (!g?.id) return
    const entry = timemap.current.find((t) => t.on?.includes(g.id))
      ?? timemap.current.find((t) => g.querySelector?.(`[id="${t.on?.[0]}"]`))
    if (entry) player.playFrom(Math.max(0, timeAt(entry.qstamp)))
  }

  const name = useMemo(() => song.title.replace(/[\\/:*?"<>|]+/g, ' ').trim().slice(0, 80) || 'score', [song.title])
  const download = () => {
    if (!data) return
    const url = URL.createObjectURL(new Blob([data.xml], { type: 'application/vnd.recordare.musicxml+xml' }))
    const a = document.createElement('a')
    a.href = url; a.download = `${name}.musicxml`; a.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 5000)
  }

  // print or save as PDF: A4 pages drawn fresh, then the screen version again
  const [printing, setPrinting] = useState(false)
  const print = async () => {
    if (!data) return
    setPrinting(true)
    try {
      const tk = await getToolkit()
      tk.setOptions({ scale: 85, pageWidth: Math.round(2100 * 100 / 85), pageHeight: Math.round(2970 * 100 / 85), adjustPageHeight: false, svgViewBox: true, header: 'auto', footer: 'none', breaks: 'auto', mnumInterval: 4,
        pageMarginLeft: 90, pageMarginRight: 90, pageMarginTop: 110, pageMarginBottom: 110 + PRINT_ROOM })
      tk.loadData(data.xml)
      const pages = Array.from({ length: tk.getPageCount() }, (_, i) => tk.renderToSVG(i + 1))
      const host = document.createElement('div')
      host.className = `score-print ${hidden}`
      host.innerHTML = pages.map((p) => `<div class="score-page">${p}</div>`).join('')
      sizeHeader(host.firstElementChild ?? host, PRINT_ROOM) // only the first page has a title
      document.body.appendChild(host)
      document.body.classList.add('printing')
      const done = () => { document.body.classList.remove('printing'); host.remove(); window.removeEventListener('afterprint', done); void draw(box.current?.clientWidth ?? 800) }
      window.addEventListener('afterprint', done)
      window.print()
    } finally { setPrinting(false) }
  }

  if (err) return <div className="notice bad">{err}</div>
  if (!data) return <div className="empty">Writing out the sheet music…</div>
  const beginner = !show.rests && !show.dots && !show.staccato
  return (
    <div className="score-layout">
      <div className="card">
        <div className="sheet-bar">
          <button className="btn primary" onClick={player.toggle} aria-pressed={player.playing}>{player.playing ? <Pause /> : <Play />} {player.playing ? 'Pause' : 'Play'}</button>
          <span className="chip">{keyName(data.fifths, data.minor)}</span>
          <span className="chip">{data.beats}/4</span>
          <span className="chip">♩ = {Math.round(data.bpm)}</span>
          <span className="chip">{data.measures} bars</span>
          <span className="grow" />
          <span className="zoom" role="group" aria-label="Size">
            <button className="btn small icon" onClick={() => setScale((s) => Math.max(25, s - 5))} aria-label="Smaller">−</button>
            <button className="btn small icon" onClick={() => setScale((s) => Math.min(90, s + 5))} aria-label="Bigger">+</button>
          </span>
          <button className="btn" onClick={download}><Download /> MusicXML</button>
          <button className="btn" onClick={() => void print()} disabled={printing}>Print or PDF</button>
        </div>
        {failed && <div className="notice bad" style={{ margin: 14 }}>{failed}</div>}
        <div className="score-stage">
          {!drawn && !failed && <div className="hint" style={{ padding: 20 }}>Drawing the score…</div>}
          <div className={`score-paper ${hidden}`} ref={box} onClick={onClick} />
        </div>
      </div>

      <aside className="card panel stack score-side" aria-label="Sheet options">
        <p className="hint">
          Written automatically from the notes in the video: rhythms are tidied to the nearest sixteenth (or triplet), and a chord that was rolled or came in a little early still reads as one chord.
          It is a clean draft to read and learn from. Click any note to hear the song from there.
        </p>
        <button className={`btn ${beginner ? 'primary' : ''}`} aria-pressed={beginner} onClick={() => setShow({ ...show, rests: beginner, dots: beginner, staccato: beginner })}>
          Beginner mode: {beginner ? 'on' : 'off'}
        </button>
        <p className="hint" style={{ marginTop: -6 }}>Hides rests, dots after notes and staccato dots, so only the notes and the beat remain.</p>

        <h3>Bars</h3>
        <div className="field">
          <label htmlFor="bars">Where the bar lines go</label>
          <select id="bars" className="select" value={bars} onChange={(e) => { found.current = null; setBars(e.target.value) }}>
            <option value="auto">Automatic (the simplest page)</option>
            <option value="2">2 beats to a bar</option>
            <option value="3">3 beats to a bar (waltz)</option>
            <option value="4">4 beats to a bar</option>
            <option value="sheet">Same as the easy sheet</option>
          </select>
          <p className="hint">If the notes sit on a regular pulse, Automatic picks the tempo and meter that make the cleanest page. If a bar looks wrong, choose how many beats it has.</p>
        </div>

        <h3>Key</h3>
        <div className="field">
          <label htmlFor="key">Write the sheet in</label>
          <select id="key" className="select" value={key} onChange={(e) => setKey(e.target.value)}>
            <option value="auto">Automatic{data ? ` (${keyName(data.detected.fifths, data.detected.minor)})` : ''}</option>
            {(['major', 'minor'] as const).map((mode) => (
              <optgroup key={mode} label={mode === 'major' ? 'Major keys' : 'Minor keys'}>
                {[0, 1, 2, 3, 4, 5, 6, 7, -1, -2, -3, -4, -5, -6, -7].map((f) => (
                  <option key={`${f}:${mode}`} value={`${f}:${mode}`}>{keyName(f, mode === 'minor')} ({f === 0 ? 'no sharps or flats' : `${Math.abs(f)} ${f > 0 ? '♯' : '♭'}`})</option>
                ))}
              </optgroup>
            ))}
          </select>
          <p className="hint">The notes are the same; only how they are spelled changes, and the signature at the start of each line. Pick the key you know the piece is in if the automatic one looks odd (for example A♭ major instead of C♯ major).</p>
        </div>

        <h3>Holds</h3>
        <div className="field">
          <label htmlFor="hold">{hold === 0 ? 'Show every hold' : `Only show holds of ${hold} beat${hold === 1 ? '' : 's'} or more`}</label>
          <input id="hold" type="range" min={0} max={4} step={0.5} value={hold} onChange={(e) => setHold(Number(e.target.value))} />
          <p className="hint">A hold is a note kept down while the music moves on: it shows as a tied note on the other side of a bar line, or as a long note under moving ones. Holds shorter than this are cut where the next note comes, so the page shows only the notes you play. Slide to 0 to see every hold.</p>
        </div>

        <h3>Title page</h3>
        <div className="field"><label htmlFor="st-title">Title</label>
          <input id="st-title" className="input" value={meta.title} onChange={(e) => setMeta({ ...meta, title: e.target.value })} /></div>
        <div className="field"><label htmlFor="st-sub">Under the title</label>
          <input id="st-sub" className="input" value={meta.subtitle ?? `Key of ${keyName(data.fifths, data.minor)}`} onChange={(e) => setMeta({ ...meta, subtitle: e.target.value })} />
          {meta.subtitle !== null && <button className="btn small ghost" style={{ justifySelf: 'start' }} onClick={() => setMeta({ ...meta, subtitle: null })}>Use the key again</button>}</div>
        <div className="field"><label htmlFor="st-comp">Composer</label>
          <input id="st-comp" className="input" placeholder="e.g. Tchaikovsky" value={meta.composer} onChange={(e) => setMeta({ ...meta, composer: e.target.value })} /></div>
        <div className="field"><label htmlFor="st-arr">Arranged by</label>
          <input id="st-arr" className="input" placeholder="your name" value={meta.arranger} onChange={(e) => setMeta({ ...meta, arranger: e.target.value })} /></div>

        <h3>What to show</h3>
        <p className="hint" style={{ marginTop: -6 }}>Untick anything you would rather not see. The notes stay the same; only the marks are hidden.</p>
        {MARKS.map((m) => (
          <label className="check mark" key={m.key}>
            <input type="checkbox" checked={!!show[m.key]} onChange={(e) => setShow({ ...show, [m.key]: e.target.checked })} />
            <span><b>{m.label}</b><small>{m.what}</small></span>
          </label>
        ))}
        <label className="check mark">
          <input type="checkbox" checked={!!show.follow} onChange={(e) => setShow({ ...show, follow: e.target.checked })} />
          <span><b>Scroll along while it plays</b><small>The page moves down to keep the playing notes in view. It stops whenever you scroll yourself.</small></span>
        </label>
      </aside>
    </div>
  )
}
