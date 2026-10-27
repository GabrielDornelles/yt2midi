import type { NoteRec } from '../types'

/*
 * The two hands drawn over the keys. Where each hand is, and which fingers are down, is a plain function of the
 * song time and of the fingering the engine worked out, so seeking and slowing down just work.
 */

const BLACK = new Set([1, 3, 6, 8, 10])
const LOW = 21
const isBlack = (m: number) => BLACK.has(m % 12)
export const whiteIndex = (m: number) => { let n = 0; for (let k = LOW; k < m; k++) if (!isBlack(k)) n++; return n }
/** Centre of a key, in white-key widths from the left edge. */
const centre = (m: number) => (isBlack(m) ? whiteIndex(m) : whiteIndex(m) + 0.5)

const SPREAD = 0.95 // white keys between neighbouring fingers of a relaxed hand
const TOGETHER = 0.06
const REACH = 0.28 // seconds before a note that its finger starts moving over

interface Ev { t: number; at: number } // when, and where the hand's middle finger rests (in white keys)

export interface Side {
  right: boolean
  hand: number // which of the song's hands (colours) plays here
  notes: NoteRec[] // with a finger, in time order
  events: Ev[]
  maxDur: number
}

export function buildSides(notes: NoteRec[]): Side[] {
  return [true, false].map((right) => {
    const list = notes.filter((n) => !!n.finger && (n.right ?? true) === right).sort((a, b) => a.start - b.start || a.midi - b.midi)
    const dir = right ? 1 : -1
    const events: Ev[] = []
    let i = 0
    while (i < list.length) {
      let j = i
      let sum = 0
      while (j < list.length && list[j].start - list[i].start <= TOGETHER) { sum += centre(list[j].midi) - dir * (list[j].finger! - 3) * SPREAD; j++ }
      events.push({ t: list[i].start, at: sum / (j - i) })
      i = j
    }
    return { right, hand: list[0]?.hand ?? 0, notes: list, events, maxDur: list.reduce((m, n) => Math.max(m, n.end - n.start), 0) }
  })
}

const smooth = (x: number) => { const c = Math.max(0, Math.min(1, x)); return c * c * (3 - 2 * c) }

/** Where the hand rests at time t (white keys). It slides to the next position just before it is needed. */
function restAt(s: Side, t: number): number | null {
  const e = s.events
  if (!e.length) return null
  let lo = 0, hi = e.length
  while (lo < hi) { const m = (lo + hi) >> 1; if (e[m].t <= t) lo = m + 1; else hi = m }
  const k = lo - 1
  if (k < 0) return e[0].at
  if (k + 1 >= e.length) return e[k].at
  const gap = e[k + 1].t - e[k].t
  const move = Math.min(0.4, gap * 0.9)
  return e[k].at + (e[k + 1].at - e[k].at) * smooth((t - (e[k + 1].t - move)) / move)
}

export interface Tip { x: number; pressed: boolean; midi: number } // x in white keys

export interface Pose { at: number; right: boolean; fingers: (Tip | null)[] } // fingers[1..5]

export function poseAt(s: Side, t: number): Pose | null {
  const at = restAt(s, t)
  if (at === null) return null
  const dir = s.right ? 1 : -1
  const fingers: (Tip | null)[] = [null, null, null, null, null, null]
  const from = lowerBound(s.notes, t - s.maxDur)
  for (let i = from; i < s.notes.length && s.notes[i].start < t + REACH; i++) {
    const n = s.notes[i]
    if (n.end < t) continue
    const f = n.finger!
    if (n.start <= t) { fingers[f] = { x: centre(n.midi), pressed: true, midi: n.midi }; continue } // the latest note on the finger wins
    if (fingers[f]) continue // already down, or already reaching for an earlier note
    // reaching: the fingertip travels from its resting place to the key
    const u = smooth(1 - (n.start - t) / REACH)
    const rest = at + dir * (f - 3) * SPREAD
    fingers[f] = { x: rest + (centre(n.midi) - rest) * u, pressed: false, midi: n.midi }
  }
  return { at, right: s.right, fingers }
}

function lowerBound(a: NoteRec[], t: number): number {
  let lo = 0, hi = a.length
  while (lo < hi) { const m = (lo + hi) >> 1; if (a[m].start < t) lo = m + 1; else hi = m }
  return lo
}

const LEN = [0, 2.6, 3.3, 3.7, 3.4, 2.8] // finger lengths in white-key widths
const SKIN = '#efc7a3', SKIN_DARK = '#a9714b'

/**
 * Draw one hand. `kw` is the width of a white key in pixels; the knuckles sit at `knuckleY`, and fingertips that
 * press a key reach up to `pressY` (or `pressBlackY` on a black key).
 */
export function drawHand(g: CanvasRenderingContext2D, p: Pose, kw: number, knuckleY: number, pressY: number, pressBlackY: number, dark: boolean) {
  const dir = p.right ? 1 : -1
  const hx = p.at * kw
  const base = (f: number) => ({
    x: hx + dir * (f - 3) * 0.86 * kw,
    y: knuckleY + [0, 1.1, 0.1, -0.15, 0.1, 0.65][f] * kw,
  })
  const tips = [1, 2, 3, 4, 5].map((f) => {
    const b = base(f), tip = p.fingers[f]
    // resting finger: slightly curled and fanned out
    const rest = { x: b.x + dir * (f === 1 ? -0.75 : (f - 3) * 0.16) * kw, y: b.y - LEN[f] * kw * (f === 1 ? 0.55 : 0.6) }
    if (!tip) return { ...b, tx: rest.x, ty: rest.y, down: false }
    const ty = tip.pressed ? (isBlack(tip.midi) ? pressBlackY : pressY) : rest.y
    const tx = tip.x * kw
    // never reach further than the finger is long
    const dx = tx - b.x, dy = ty - b.y, d = Math.hypot(dx, dy), max = LEN[f] * kw * 1.1
    const s = d > max ? max / d : 1
    return { ...b, tx: b.x + dx * s, ty: b.y + dy * s, down: tip.pressed }
  })
  const fw = kw * 0.74
  const skin = dark ? '#e6bf9b' : SKIN
  g.save()
  g.lineCap = 'round'
  g.lineJoin = 'round'
  // shadow on the keys
  g.globalAlpha = 0.2
  g.strokeStyle = '#000'
  g.lineWidth = fw
  for (const t of tips) { g.beginPath(); g.moveTo(t.x + 3, t.y + 4); g.lineTo(t.tx + 3, t.ty + 4); g.stroke() }
  g.globalAlpha = 0.96
  // forearm and palm
  const palm = () => {
    g.beginPath()
    g.moveTo(hx - dir * 2.0 * kw, knuckleY + 0.9 * kw)
    g.quadraticCurveTo(hx - dir * 2.3 * kw, knuckleY + 3.3 * kw, hx - dir * 1.2 * kw, knuckleY + 8 * kw)
    g.lineTo(hx + dir * 1.6 * kw, knuckleY + 8 * kw)
    g.quadraticCurveTo(hx + dir * 2.3 * kw, knuckleY + 3.0 * kw, hx + dir * 2.15 * kw, knuckleY + 0.3 * kw)
    g.quadraticCurveTo(hx, knuckleY - 0.55 * kw, hx - dir * 2.0 * kw, knuckleY + 0.9 * kw)
    g.closePath()
  }
  palm(); g.strokeStyle = SKIN_DARK; g.lineWidth = 2; g.stroke()
  // fingers: outline first, then the fill, so neighbours blend into one hand
  for (const t of tips) { g.beginPath(); g.moveTo(t.x, t.y); g.lineTo(t.tx, t.ty); g.strokeStyle = SKIN_DARK; g.lineWidth = fw + 3; g.stroke() }
  palm(); g.fillStyle = skin; g.fill()
  for (const t of tips) { g.beginPath(); g.moveTo(t.x, t.y); g.lineTo(t.tx, t.ty); g.strokeStyle = skin; g.lineWidth = fw; g.stroke() }
  // a nail on each tip, and a slightly warmer tip where a key is pressed
  for (const t of tips) {
    g.beginPath(); g.arc(t.tx, t.ty - fw * 0.1, fw * 0.28, 0, Math.PI * 2)
    g.fillStyle = t.down ? 'rgba(210,120,100,.55)' : 'rgba(255,255,255,.45)'; g.fill()
  }
  g.restore()
}
