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

// Bones, in white-key widths: [first, second, third] phalanx of each finger (the thumb has two)
const BONES: number[][] = [[], [1.25, 1.0], [1.75, 0.95, 0.8], [1.9, 1.1, 0.85], [1.75, 1.0, 0.8], [1.4, 0.7, 0.7]]
const KNUCKLE: [number, number][] = [[0, 0], [-2.0, 1.5], [-0.88, 0.12], [0, -0.12], [0.88, 0.1], [1.72, 0.52]] // across, down (before mirroring)
const sum = (a: number[]) => a.reduce((x, y) => x + y, 0)

/**
 * A finger curls in the vertical plane, so from above all its joints lie on one line and the more it curls the
 * shorter it looks, while its middle joint rises towards the viewer. `r` is how far the tip must be from the knuckle.
 */
function curl(bones: number[], r: number, thumb: boolean) {
  const angles = (c: number) => {
    const a = thumb ? [0.05 + 0.5 * c, 0.1 + 0.7 * c] : [0.1 + 0.55 * c, 0.15 + 1.0 * c, 0.1 + 0.5 * c]
    let th = 0
    return a.map((x) => (th += x))
  }
  const reach = (c: number) => angles(c).reduce((s, th, i) => s + bones[i] * Math.cos(th), 0)
  const CMAX = 0.8
  const CMIN = 0.2 // a relaxed finger is never dead straight
  let c = CMIN
  if (r <= reach(CMAX)) c = CMAX
  else if (r < reach(CMIN)) { let lo = CMIN, hi = CMAX; for (let i = 0; i < 24; i++) { c = (lo + hi) / 2; if (reach(c) > r) lo = c; else hi = c } }
  const th = angles(c)
  const d = [0], z = [0]
  th.forEach((a, i) => { d.push(d[i] + bones[i] * Math.cos(a)); z.push(z[i] + bones[i] * Math.sin(a)) })
  return { d, z }
}

const mix = (a: number[], b: number[], k: number) => `rgb(${a.map((x, i) => Math.round(x + (b[i] - x) * k)).join(',')})`
const SKIN = [236, 196, 160], LIGHT = [250, 218, 190], EDGE = [150, 98, 70]

/**
 * Draw one hand. `kw` is the width of a white key in pixels; the knuckles sit at `knuckleY`, and fingertips that
 * press a key reach up to `pressY` (or `pressBlackY` on a black key).
 */
export function drawHand(g: CanvasRenderingContext2D, p: Pose, kw: number, knuckleY: number, pressY: number, pressBlackY: number, dark: boolean) {
  const dir = p.right ? 1 : -1
  const hx = p.at * kw
  const wx = hx, wy = knuckleY + 5.2 * kw // the wrist, which the hand turns around
  const cos = Math.cos(p.yaw), sin = Math.sin(p.yaw)
  const local = (x: number, y: number): [number, number] => { const dx = x - wx, dy = y - wy; return [wx + dx * cos + dy * sin, wy - dx * sin + dy * cos] }
  const lift = 1 + p.speed * 0.5 // a hand that is travelling is held higher over the keys
  const base = dark ? SKIN : SKIN
  const edge = `rgb(${EDGE})`

  g.save()
  g.lineCap = 'round'
  g.lineJoin = 'round'
  g.translate(wx, wy); g.rotate(p.yaw); g.translate(-wx, -wy)

  // ----- palm and forearm
  const X = (k: number) => hx + dir * k * kw
  const Y = (k: number) => knuckleY + k * kw
  const palm = new Path2D()
  palm.moveTo(X(-2.3), Y(1.9))
  palm.bezierCurveTo(X(-2.5), Y(3.4), X(-1.9), Y(4.4), X(-1.55), Y(5.2)
  )
  palm.lineTo(X(-1.6), Y(9)); palm.lineTo(X(1.6), Y(9)); palm.lineTo(X(1.5), Y(5.2))
  palm.bezierCurveTo(X(1.85), Y(3.9), X(2.3), Y(2.2), X(2.2), Y(0.55))
  palm.bezierCurveTo(X(1.5), Y(-0.45), X(-0.9), Y(-0.55), X(-1.9), Y(0.1))
  palm.bezierCurveTo(X(-2.5), Y(0.5), X(-2.5), Y(1.3), X(-2.3), Y(1.9))
  g.globalAlpha = 0.22; g.fillStyle = '#000'
  g.save(); g.translate(2 + 2 * lift, 4 + 4 * lift); g.fill(palm); g.restore()
  g.globalAlpha = 1
  const grad = g.createLinearGradient(0, Y(0), 0, Y(6))
  grad.addColorStop(0, mix(base, LIGHT, 0.35)); grad.addColorStop(1, mix(base, EDGE, 0.18))
  g.fillStyle = grad; g.fill(palm)
  g.strokeStyle = edge; g.lineWidth = 1.6; g.stroke(palm)
  // the pad of the thumb, and a soft crease across the palm
  g.save(); g.clip(palm)
  g.fillStyle = 'rgba(255,235,215,.28)'
  g.beginPath(); g.ellipse(X(-1.55), Y(2.5), kw * 0.85, kw * 1.5, dir * 0.35, 0, Math.PI * 2); g.fill()
  g.strokeStyle = 'rgba(150,98,70,.3)'; g.lineWidth = 1.2
  g.beginPath(); g.moveTo(X(-1.7), Y(1.6)); g.quadraticCurveTo(X(0), Y(2.6), X(1.9), Y(1.2)); g.stroke()
  g.restore()

  // ----- fingers, the thumb last so it lies over the others
  for (const f of [5, 4, 2, 3, 1]) {
    const bones = BONES[f], thumb = f === 1
    const [kx, ky] = KNUCKLE[f]
    const bx = X(kx), by = Y(ky)
    const total = sum(bones)
    const tip = p.fingers[f]
    // relaxed: a loosely curled finger, a little fanned out; the thumb rests beside the hand
    const rx = X(kx + (thumb ? -0.55 : (f - 3) * 0.14)), ry = by - total * kw * (thumb ? 0.6 : 0.64)
    let tx = rx, ty = ry, press = 0
    if (tip) {
      press = tip.press
      const kyKey = isBlack(tip.midi) ? pressBlackY : pressY
      tx = rx + (tip.x * kw - rx) * press
      ty = ry + (kyKey - ry) * press
    }
    const [lx, ly] = local(tx, ty)
    let vx = lx - bx, vy = ly - by
    const r0 = Math.hypot(vx, vy) || 1
    vx /= r0; vy /= r0
    const { d, z } = curl(bones, Math.max(r0 / kw, total * 0.3), thumb)
    const w0 = (thumb ? 0.82 : f === 5 ? 0.56 : 0.66) * kw
    const air = (1 - press) * 0.35 + Math.sin(press * Math.PI) * 0.5 // a finger on its way is held above the keys
    const pts = d.map((di, i) => ({ x: bx + vx * di * kw, y: by + vy * di * kw, h: z[i] + air * (i / Math.max(1, d.length - 1)) }))
    const wd = pts.map((q, i) => w0 * (1 - 0.1 * i) * (1 + 0.1 * q.h))
    const nx = -vy, ny = vx
    // shadow on the keys
    g.globalAlpha = 0.13; g.fillStyle = '#000'
    pts.forEach((q, i) => { if (i === 0) return; const o = q.h * kw * 0.16 + 2; g.beginPath(); g.ellipse(q.x + o * 0.7, q.y + o * 1.5, wd[i] * 0.46, wd[i] * 0.34, 0, 0, Math.PI * 2); g.fill() })
    g.globalAlpha = 1
    const n = bones.length
    // each bone is a tapered body with a round joint at each end; every outline goes down first, then every fill,
    // so the joints inside a finger leave no lines
    const pieces: Path2D[] = []
    for (let i = 0; i < n; i++) {
      const a = pts[i], b = pts[i + 1], pa = wd[i] / 2, pb = wd[i + 1] / 2
      const body = new Path2D()
      body.moveTo(a.x + nx * pa, a.y + ny * pa); body.lineTo(b.x + nx * pb, b.y + ny * pb)
      body.lineTo(b.x - nx * pb, b.y - ny * pb); body.lineTo(a.x - nx * pa, a.y - ny * pa); body.closePath()
      const ja = new Path2D(); ja.arc(a.x, a.y, pa, 0, Math.PI * 2)
      const jb = new Path2D(); jb.arc(b.x, b.y, pb, 0, Math.PI * 2)
      pieces.push(body, ja, jb)
    }
    g.strokeStyle = edge; g.lineWidth = 3
    pieces.forEach((pa) => g.stroke(pa))
    pieces.forEach((pa, k) => {
      const i = Math.floor(k / 3)
      // a part that is raised towards the viewer is lighter
      g.fillStyle = mix(base, LIGHT, Math.min(0.7, 0.12 + 0.28 * (pts[i].h + pts[i + 1].h) / 2))
      g.fill(pa)
    })
    // roundness: a soft highlight along the finger, and a crease at each joint
    g.strokeStyle = 'rgba(255,240,225,.35)'; g.lineWidth = Math.max(2, w0 * 0.28)
    g.beginPath(); g.moveTo(pts[0].x + nx * w0 * 0.12, pts[0].y + ny * w0 * 0.12); g.lineTo(pts[n].x + nx * w0 * 0.1, pts[n].y + ny * w0 * 0.1); g.stroke()
    g.strokeStyle = 'rgba(140,90,62,.55)'; g.lineWidth = 1.3
    for (let i = 1; i < n; i++) {
      const q = pts[i], hw = wd[i] * 0.34
      g.beginPath(); g.moveTo(q.x + nx * hw, q.y + ny * hw); g.quadraticCurveTo(q.x + vx * 2.5, q.y + vy * 2.5, q.x - nx * hw, q.y - ny * hw); g.stroke()
    }
    // the nail
    const e = pts[n], s0 = pts[n - 1]
    const nl = Math.hypot(e.x - s0.x, e.y - s0.y) * 0.5 + wd[n] * 0.25
    g.save(); g.translate(e.x - vx * wd[n] * 0.1, e.y - vy * wd[n] * 0.1); g.rotate(Math.atan2(vy, vx) - Math.PI / 2)
    g.beginPath(); g.ellipse(0, -nl * 0.15, wd[n] * 0.3, Math.max(wd[n] * 0.22, nl * 0.55), 0, 0, Math.PI * 2)
    g.fillStyle = press > 0.9 ? 'rgba(235,150,130,.7)' : 'rgba(255,238,228,.78)'; g.fill()
    g.strokeStyle = 'rgba(150,98,70,.5)'; g.lineWidth = 1; g.stroke()
    g.restore()
    // the knuckle
    if (!thumb) { g.beginPath(); g.arc(bx, by, w0 * 0.42, 0, Math.PI * 2); g.fillStyle = 'rgba(250,225,200,.35)'; g.fill() }
  }
  g.restore()
}
