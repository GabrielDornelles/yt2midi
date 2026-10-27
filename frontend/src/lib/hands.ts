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
  const move = Math.min(0.5, gap * 0.95)
  return e[k].at + (e[k + 1].at - e[k].at) * smooth((t - (e[k + 1].t - 0.04 - move)) / move)
}

/** A fingertip: where it is (white keys), how far it has come down onto its key (0 resting, 1 pressed), and the key. */
export interface Tip { x: number; press: number; midi: number }

export interface Pose { at: number; right: boolean; fingers: (Tip | null)[]; yaw: number; speed: number } // fingers[1..5]

const RELEASE = 0.14 // seconds a finger takes to come back up after its note

export function poseAt(s: Side, t: number): Pose | null {
  const at = restAt(s, t)
  if (at === null) return null
  const dir = s.right ? 1 : -1
  const fingers: (Tip | null)[] = [null, null, null, null, null, null]
  const rank = [0, 0, 0, 0, 0, 0] // 3 down, 2 reaching, 1 letting go
  const from = lowerBound(s.notes, t - s.maxDur - RELEASE)
  for (let i = from; i < s.notes.length && s.notes[i].start < t + REACH; i++) {
    const n = s.notes[i]
    const f = n.finger!
    const rest = at + dir * (f - 3) * SPREAD
    if (n.start <= t && n.end >= t) { fingers[f] = { x: centre(n.midi), press: 1, midi: n.midi }; rank[f] = 3; continue }
    if (rank[f] === 3) continue
    if (n.start > t) {
      if (rank[f] === 2) continue
      const u = smooth(1 - (n.start - t) / REACH)
      fingers[f] = { x: rest + (centre(n.midi) - rest) * u, press: u, midi: n.midi }
      rank[f] = 2
    } else if (t - n.end < RELEASE && rank[f] < 2) {
      const r = smooth((t - n.end) / RELEASE)
      fingers[f] = { x: centre(n.midi) + (rest - centre(n.midi)) * r, press: 1 - r, midi: n.midi }
      rank[f] = 1
    }
  }
  // the hand leans into a move
  const v = ((restAt(s, t + 0.06) ?? at) - (restAt(s, t - 0.06) ?? at)) / 0.12
  return { at, right: s.right, fingers, yaw: Math.max(-0.22, Math.min(0.22, v * 0.012)), speed: Math.min(1, Math.abs(v) / 14) }
}

function lowerBound(a: NoteRec[], t: number): number {
  let lo = 0, hi = a.length
  while (lo < hi) { const m = (lo + hi) >> 1; if (a[m].start < t) lo = m + 1; else hi = m }
  return lo
}
