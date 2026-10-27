import type { Chunk, LearnPlan, Phrase } from '../types'
import type { Region } from './player'

/*
 * Turns the chunks of a song into a lesson: which section to work on, and the steps to learn it in.
 * The order is what a teacher does: each hand on its own (chunk by chunk, with the hard joins between chunks drilled
 * on their own), then both hands one phrase at a time. Every step is played slowly first and gets faster as it works.
 */

export type WindowKind = 'phrase' | 'seconds' | 'notes' | 'chords'
export interface Window { kind: WindowKind; n: number }
export type First = 'auto' | 'right' | 'left'

export interface Section {
  from: number
  to: number
  right: Chunk[]
  left: Chunk[]
  phrases: Phrase[]
}

const noteCount = (f: string) => f.split('-').length

/** The section to work on, starting at the next note at or after `pos`. It always ends on a whole chunk. */
export function pickSection(plan: LearnPlan, pos: number, win: Window): Section | null {
  const all = plan.chunks
  if (!all.length) return null
  // the next note
  let next = Infinity
  for (const c of all) for (const t of c.onsets) if (t >= pos - 0.05 && t < next) next = t
  if (!isFinite(next)) return null
  // each hand starts at the chunk that holds the next note (or the first one after it)
  const begin = (right: boolean) => {
    const cs = all.filter((c) => c.right === right)
    return cs.find((c) => c.onsets[c.onsets.length - 1] >= next - 0.05)
  }
  const starts = [begin(true), begin(false)].filter((c): c is Chunk => !!c)
  // a chunk that began a while ago and only just reaches the note is not what the learner meant
  const t0 = Math.min(...starts.map((c) => (c.start < next - 2.5 ? Infinity : c.start)).filter(isFinite).concat([next]))
  const tail = (right: boolean) => all.filter((c) => c.right === right && c.start >= t0 - 0.05)
  const heard: { t: number; n: number }[] = []
  for (const c of [...tail(true), ...tail(false)]) c.onsets.forEach((t, i) => heard.push({ t, n: noteCount(c.fingers[i]) }))
  heard.sort((a, b) => a.t - b.t)
  let target = t0
  if (win.kind === 'seconds') target = t0 + win.n
  else if (win.kind === 'notes') { let k = 0; for (const h of heard) { k += h.n; target = h.t; if (k >= win.n) break } }
  else if (win.kind === 'chords') { let k = 0; for (const h of heard) { if (h.n >= 2) { k++; target = h.t; if (k >= win.n) break } } }
  else {
    const ph = plan.phrases.filter((p) => p.end > t0)
    const last = ph[Math.min(win.n, ph.length) - 1]
    target = last ? last.end - 0.01 : t0
  }
  const pick = (right: boolean) => tail(right).filter((c) => c.start <= target)
  const right = pick(true), left = pick(false)
  const chunks = [...right, ...left]
  if (!chunks.length) return null
  const to = Math.max(...chunks.map((c) => c.end))
  const phrases = plan.phrases.filter((p) => p.end > t0 && p.start < to)
  return { from: t0, to, right, left, phrases }
}

export type StepKind = 'cell' | 'chunk' | 'seam' | 'join' | 'whole' | 'together'

export interface Step {
  id: string
  kind: StepKind
  /** which hands play (the other is switched off) */
  hands: { right: boolean; left: boolean }
  title: string
  sub: string
  /** "chunk 2 of 5" */
  tag: string
  region: Region
  /** the fingers to read, in order, and where a join falls between them */
  fingers: string[]
  gap: number
  /** speeds to climb, as a fraction of the speed the learner is aiming for */
  ladder: number[]
}

const TAIL = 0.35 // seconds of ring after the last note before a loop starts over

function region(from: number, until: number, lastEnd: number): Region {
  return { from, until, restart: Math.max(from + 0.5, lastEnd) + TAIL }
}

const handName = (right: boolean) => (right ? 'Right hand' : 'Left hand')

/** How hard a hand's part is, so the harder one can be learned first. */
export function hardness(cs: Chunk[]): number {
  const w = cs.reduce((a, c) => a + c.onsets.length, 0)
  return w ? cs.reduce((a, c) => a + c.difficulty * c.onsets.length, 0) / w : 0
}

export function buildSteps(sec: Section, first: First): Step[] {
  const steps: Step[] = []
  const hands: boolean[] = []
  if (sec.right.length) hands.push(true)
  if (sec.left.length) hands.push(false)
  const solo = hands.length === 1
  if (hands.length === 2) {
    const rightFirst = first === 'right' || (first === 'auto' && hardness(sec.right) >= hardness(sec.left))
    if (!rightFirst) hands.reverse()
  }
  const one = (right: boolean) => ({ right, left: !right })
  for (const right of hands) {
    const cs = right ? sec.right : sec.left
    const who = handName(right)
    cs.forEach((c, i) => {
      const tag = `${who}, part ${i + 1} of ${cs.length}`
      if (c.cell && c.cell.reps > 1) {
        const k = c.cell.events
        const endOfCell = c.onsets[k] ?? c.end
        steps.push({
          id: `c${c.id}-cell`, kind: 'cell', hands: one(right), title: `${who}: the pattern`, sub: `Just the ${k} steps that repeat. When they feel easy, play them again and again.`, tag,
          region: region(c.start, endOfCell - 0.01, c.onsets[k - 1] + 0.4), fingers: c.fingers.slice(0, k), gap: -1, ladder: [0.5, 0.7, 0.85, 1],
        })
      }
      steps.push({
        id: `c${c.id}`, kind: 'chunk', hands: one(right), title: `${who}: ${c.label}`, sub: c.detail, tag,
        region: region(c.start, c.end + 0.05, c.end), fingers: c.fingers, gap: -1, ladder: [0.5, 0.7, 0.85, 1],
      })
      const prev = cs[i - 1]
      if (prev && c.seam) {
        const a = Math.max(0, prev.onsets.length - 2)
        const j = Math.min(2, c.onsets.length - 1)
        const stop = c.onsets[j + 1] ?? c.end + 0.05
        const lastOn = c.onsets[j]
        steps.push({
          id: `c${c.id}-seam`, kind: 'seam', hands: one(right), title: `${who}: the turn`, sub: `${c.seam[0].toUpperCase()}${c.seam.slice(1)}. Practise only the moment where one part leads into the next.`, tag: `${who}, between ${i} and ${i + 1}`,
          region: region(prev.onsets[a], stop - 0.01, lastOn + 0.4), fingers: [...prev.fingers.slice(a), ...c.fingers.slice(0, j + 1)], gap: prev.fingers.length - a, ladder: [0.6, 0.8, 1],
        })
      }
      if (prev) {
        steps.push({
          id: `c${c.id}-join`, kind: 'join', hands: one(right), title: `${who}: parts ${i} and ${i + 1} together`, sub: 'Now join them without stopping.', tag: `${who}, ${i} + ${i + 1}`,
          region: region(prev.start, c.end + 0.05, c.end), fingers: [...prev.fingers, ...c.fingers], gap: prev.fingers.length, ladder: [0.6, 0.8, 1],
        })
      }
    })
    if (cs.length > 2) {
      const all = cs.flatMap((c) => c.fingers)
      steps.push({
        id: `${right ? 'r' : 'l'}-whole`, kind: 'whole', hands: one(right), title: `${who}: all of it`, sub: 'The whole section in one go.', tag: who,
        region: region(cs[0].start, cs[cs.length - 1].end + 0.05, cs[cs.length - 1].end), fingers: all, gap: -1, ladder: [0.6, 0.8, 1],
      })
    }
  }
  if (!solo) {
    const ph = sec.phrases
    ph.forEach((p, i) => {
      const from = Math.max(p.start, sec.from), until = Math.min(p.end, sec.to + 0.05)
      const fr = sec.right.filter((c) => c.start >= from - 0.07 && c.start < until).flatMap((c) => c.fingers)
      const fl = sec.left.filter((c) => c.start >= from - 0.07 && c.start < until).flatMap((c) => c.fingers)
      steps.push({
        id: `p${p.id}`, kind: 'together', hands: { right: true, left: true }, title: 'Both hands', sub: i === 0 ? 'Start slow. Put the hands together only after each is comfortable alone.' : 'The next stretch with both hands.', tag: `Both hands, part ${i + 1} of ${ph.length}`,
        region: region(from, until, until - TAIL), fingers: [fr.length ? `R ${fr.slice(0, 12).join(' ')}` : '', fl.length ? `L ${fl.slice(0, 12).join(' ')}` : ''].filter(Boolean), gap: -1, ladder: [0.5, 0.7, 0.85, 1],
      })
    })
    if (ph.length > 1) {
      steps.push({
        id: 'together-whole', kind: 'whole', hands: { right: true, left: true }, title: 'Both hands: all of it', sub: 'The whole section with both hands.', tag: 'Both hands',
        region: region(sec.from, sec.to + 0.05, sec.to), fingers: [], gap: -1, ladder: [0.6, 0.8, 1],
      })
    }
  }
  return steps
}

/** The speeds of a step in percent, for the speed the learner is aiming for. */
export function rungs(step: Step, target: number): number[] {
  const out: number[] = []
  for (const f of step.ladder) {
    const v = Math.max(25, Math.min(100, Math.round((target * f) / 5) * 5))
    if (out[out.length - 1] !== v) out.push(v)
  }
  return out
}
