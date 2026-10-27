import { useEffect, useRef } from 'react'
import type { NoteRec, Rgb } from '../types'
import type { Region } from '../lib/player'
import { buildSides, poseAt, type Side } from '../lib/hands'
import { drawHand } from '../lib/handModel'

/*
 * The whole piano (88 keys, always the same size) with the music falling onto it from above, the way the original
 * video shows it. Notes are in their hand's colour; a hand that is switched off is hidden. How fast the notes fall
 * follows the speed the music is played at.
 */

const FALLBACK: Rgb[] = [[224, 142, 90], [110, 160, 200]]
const LOW = 21, HIGH = 108 // A0 to C8
const LOOK = 3.2 // seconds of music shown above the keys
const BLACK = new Set([1, 3, 6, 8, 10])
const isBlack = (m: number) => BLACK.has(m % 12)
/** White keys below this note, counting from A0. */
const whiteIndex = (m: number) => { let n = 0; for (let k = LOW; k < m; k++) if (!isBlack(k)) n++; return n }
const N_WHITE = whiteIndex(HIGH) + 1
const LETTER: Record<number, string> = { 0: 'C', 2: 'D', 4: 'E', 5: 'F', 7: 'G', 9: 'A', 11: 'B' }
const rgba = (c: Rgb, a: number) => `rgba(${c[0]},${c[1]},${c[2]},${a})`

interface Props {
  notes: NoteRec[]
  colors: Rgb[]
  /** Which hands are shown (index = hand). */
  on: [boolean, boolean]
  /** draw the hands and the finger numbers */
  hands: boolean
  /** a section being practised: the rest of the music is dimmed */
  focus?: Region | null
  playing: boolean
  pos: number
  getPos: () => number
}

export function FallingKeys({ notes, colors, on, hands, focus = null, playing, pos, getPos }: Props) {
  const box = useRef<HTMLDivElement>(null)
  const cv = useRef<HTMLCanvasElement>(null)
  const size = useRef({ w: 0, h: 0, dpr: 1 })
  const live = useRef({ notes, colors, on, hands, focus, playing, pos, getPos })
  live.current = { notes, colors, on, hands, focus, playing, pos, getPos }
  const shape = useRef({ sorted: [] as NoteRec[], maxDur: 0, sides: [] as Side[] })
  const dark = useRef({ v: true, frame: 0, bg: '#1d1915', line: '#3d352e', accent: '#e08e5a' })

  useEffect(() => {
    const sorted = [...notes].sort((a, b) => a.start - b.start)
    shape.current = { sorted, maxDur: sorted.reduce((m, n) => Math.max(m, n.end - n.start), 0), sides: buildSides(sorted) }
  }, [notes])

  const draw = () => {
    const c = cv.current
    const { w, h, dpr } = size.current
    if (!c || !w) return
    const L = live.current, { sorted, maxDur, sides } = shape.current
    const th = dark.current
    if (th.frame++ % 60 === 0) {
      const cs = getComputedStyle(document.documentElement)
      th.v = cs.colorScheme === 'dark'
      th.bg = cs.getPropertyValue('--bg-2').trim() || th.bg
      th.line = cs.getPropertyValue('--line-2').trim() || th.line
      th.accent = cs.getPropertyValue('--accent').trim() || th.accent
    }
    const g = c.getContext('2d')!
    g.setTransform(dpr, 0, 0, dpr, 0, 0)
    g.clearRect(0, 0, w, h)
    const now = L.playing ? L.getPos() : L.pos
    const kw = w / N_WHITE
    const handH = L.hands ? kw * 5.4 : 0 // room under the keys for the palms
    const keysH = Math.max(64, Math.min((h - handH) * 0.3, kw * 7))
    const top = h - handH - keysH
    const pal = L.colors.length ? L.colors : FALLBACK
    const hue = (hand: number) => pal[hand] ?? pal[0]
    const shown = (hand: number) => L.on[hand === 1 ? 1 : 0]
    const keyX = (m: number) => (isBlack(m) ? whiteIndex(m) * kw - kw * 0.3 : whiteIndex(m) * kw)
    const keyW = (m: number) => (isBlack(m) ? kw * 0.6 : kw)

    g.fillStyle = th.bg
    g.fillRect(0, 0, w, top)
    // a faint line at each C, and a stronger one where the pianist's middle is, so the keys can be followed upward
    g.fillStyle = 'rgba(128,110,90,.16)'
    for (let m = LOW; m <= HIGH; m++) if (m % 12 === 0) g.fillRect(Math.round(keyX(m)), 0, 1, top)

    // notes falling: the top of a note is its end, the bottom is its start, which reaches the keys when the note begins
    const down = new Map<number, number>()
    const sec = top / LOOK
    for (let i = lowerBound(sorted, now - maxDur); i < sorted.length && sorted[i].start < now + LOOK; i++) {
      const n = sorted[i]
      if (n.end < now || !shown(n.hand) || n.midi < LOW || n.midi > HIGH) continue
      const active = n.start <= now
      const out = !!L.focus && (n.start < L.focus.from - 0.02 || n.start >= L.focus.until)
      if (active && !out) down.set(n.midi, n.hand)
      const yb = top - (n.start - now) * sec, yt = top - (n.end - now) * sec
      const x = keyX(n.midi), wd = keyW(n.midi)
      const y0 = Math.max(-2, yt), y1 = Math.min(top, yb)
      if (y1 <= y0) continue
      g.fillStyle = rgba(hue(n.hand), out ? 0.22 : active ? 1 : 0.9)
      g.beginPath(); g.roundRect(x + 1, y0, wd - 2, Math.max(4, y1 - y0 - 1), Math.min(5, wd / 3)); g.fill()
      g.strokeStyle = 'rgba(0,0,0,.28)'; g.lineWidth = 1; g.stroke()
      if (L.hands && n.finger && y1 - y0 > kw * 0.9) {
        g.fillStyle = 'rgba(0,0,0,.72)'; g.font = `800 ${Math.max(9, Math.min(22, kw * 0.6))}px ui-sans-serif, system-ui, sans-serif`; g.textAlign = 'center'
        g.fillText(String(n.finger), x + wd / 2, y1 - Math.max(4, kw * 0.22))
      }
    }

    // keys
    g.fillStyle = th.v ? '#0f0c0a' : '#3a3028'
    g.fillRect(0, top - 3, w, 3)
    for (let m = LOW; m <= HIGH; m++) {
      if (isBlack(m)) continue
      const x = keyX(m)
      g.fillStyle = th.v ? '#ece5d8' : '#fffdf9'
      g.fillRect(x + 0.5, top, kw - 1, keysH)
      const hand = down.get(m)
      if (hand !== undefined) { g.fillStyle = rgba(hue(hand), 0.95); g.fillRect(x + 0.5, top, kw - 1, keysH) }
      // the name of every white key, at the bottom of the key; C also says which C it is
      const size = Math.max(8, Math.min(30, kw * 0.58))
      g.fillStyle = hand !== undefined ? 'rgba(0,0,0,.7)' : m % 12 === 0 ? '#5d5348' : '#8a7e71'
      g.font = `${m % 12 === 0 || hand !== undefined ? 800 : 600} ${size}px ui-sans-serif, system-ui, sans-serif`
      g.textAlign = 'center'
      g.fillText(m % 12 === 0 ? `C${m / 12 - 1}` : LETTER[m % 12], x + kw / 2, top + keysH - Math.max(5, size * 0.4))
    }
    for (let m = LOW; m <= HIGH; m++) {
      if (!isBlack(m)) continue
      const x = keyX(m), hand = down.get(m)
      g.fillStyle = '#1a1511'
      g.beginPath(); g.roundRect(x, top, keyW(m), keysH * 0.62, [0, 0, 3, 3]); g.fill()
      if (hand !== undefined) { g.fillStyle = rgba(hue(hand), 0.95); g.beginPath(); g.roundRect(x, top, keyW(m), keysH * 0.62, [0, 0, 3, 3]); g.fill() }
    }
    g.fillStyle = th.accent
    g.fillRect(0, top - 3, w, 2)
    if (L.hands) {
      g.fillStyle = th.bg
      g.fillRect(0, top + keysH, w, handH)
      g.fillStyle = th.v ? '#0f0c0a' : '#3a3028'
      g.fillRect(0, top + keysH, w, 3)
      for (const s of sides) {
        if (!shown(s.hand)) continue
        const p = poseAt(s, now)
        if (p) drawHand(g, p, kw, top + keysH, keysH, w / 2, th.v)
      }
    }
  }

  useEffect(() => {
    const el = box.current, c = cv.current
    if (!el || !c) return
    const fit = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 2)
      const w = el.clientWidth, h = el.clientHeight
      size.current = { w, h, dpr }
      c.width = Math.round(w * dpr); c.height = Math.round(h * dpr)
      draw()
    }
    fit()
    const ro = new ResizeObserver(fit)
    ro.observe(el)
    return () => ro.disconnect()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // paused: redraw when something changes; playing: every frame
  useEffect(() => {
    if (!playing) { draw(); return }
    let raf = 0
    const loop = () => { draw(); raf = requestAnimationFrame(loop) }
    raf = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(raf)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, pos, on[0], on[1], hands, notes, colors, focus])

  return <div className="falling" ref={box} role="img" aria-label="Piano keyboard with the notes falling onto it"><canvas ref={cv} /></div>
}

function lowerBound(a: NoteRec[], t: number): number {
  let lo = 0, hi = a.length
  while (lo < hi) { const m = (lo + hi) >> 1; if (a[m].start < t) lo = m + 1; else hi = m }
  return lo
}
