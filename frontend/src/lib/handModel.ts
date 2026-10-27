import type { Pose } from './hands'

/*
 * A hand built the way a real one is: bones with real lengths (in centimetres), a wrist held level with the keys, a palm
 * arched at the knuckles and fingers that bend in their three joints. For every fingertip that has somewhere to be, the
 * joints are solved so that the tip really lands there (inverse kinematics). The last joint of a finger follows the
 * middle one at about two thirds of its angle, as in a real hand. Everything is then drawn in perspective from above and
 * behind the player.
 *
 * Measures are those of an average adult hand (about 18 cm long, 8 cm wide); a white key is 2.35 cm wide.
 */

type V = [number, number, number]
const lerp = (a: V, b: V, k: number): V => [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k]

export const KEY_CM = 2.35 // width of a white key
const BLACK = new Set([1, 3, 6, 8, 10])
const isBlack = (m: number) => BLACK.has(m % 12)

interface Digit { L: number[]; r: number[]; root: V }
// right hand, x towards the little finger, y towards the fingertips, z up; origin: the middle of the wrist
const DIGIT: Digit[] = [
  { L: [], r: [], root: [0, 0, 0] },
  { L: [3.6, 3.0, 2.3], r: [1.05, 0.98, 0.86], root: [-2.6, 2.4, 0.1] }, // thumb: its first bone is the metacarpal
  { L: [4.2, 2.4, 1.9], r: [0.98, 0.88, 0.78], root: [-3.0, 9.3, 2.7] },
  { L: [4.7, 2.9, 2.1], r: [1.0, 0.9, 0.8], root: [-1.0, 9.8, 3.0] },
  { L: [4.3, 2.7, 2.0], r: [0.94, 0.84, 0.74], root: [1.0, 9.3, 2.8] },
  { L: [3.4, 1.9, 1.8], r: [0.88, 0.77, 0.7], root: [2.8, 8.0, 2.1] },
]

const SKIN: V = [233, 190, 154], LIGHT: V = [252, 222, 196], SHADE: V = [168, 112, 84], EDGE: V = [128, 80, 58]
const rgb = (c: V, k = 1) => `rgb(${Math.round(c[0] * k)},${Math.round(c[1] * k)},${Math.round(c[2] * k)})`
const tone = (a: V, b: V, k: number) => rgb(lerp(a, b, Math.max(0, Math.min(1, k))))

/**
 * Bend a finger so that its tip reaches `to` from `root`. The finger bends in the vertical plane through both: `a` is
 * the first joint's angle below the horizontal, `b` the second's; the third follows the second. Of all the bends that
 * reach, the one a relaxed hand would take (second joint a little more bent than the first) is chosen.
 */
function reach(root: V, to: V, L: number[], thumb: boolean, forward: number): V[] {
  const rx = to[0] - root[0], ry = to[1] - root[1]
  // a finger can only swing so far to the side of the hand (the thumb much further)
  let rel = Math.atan2(ry, rx) - forward
  rel = Math.atan2(Math.sin(rel), Math.cos(rel))
  const lim = thumb ? 1.1 : 0.42
  rel = Math.max(-lim, Math.min(lim, rel))
  const az = forward + rel
  const dx = Math.cos(az), dy = Math.sin(az)
  const d = Math.max(0.5, rx * dx + ry * dy), h = root[2] - to[2]
  const follow = thumb ? 0.7 : 0.66
  const tip = (a: number, b: number) => {
    const t1 = a, t2 = a + b, t3 = a + b + b * follow
    return [L[0] * Math.cos(t1) + L[1] * Math.cos(t2) + L[2] * Math.cos(t3), L[0] * Math.sin(t1) + L[1] * Math.sin(t2) + L[2] * Math.sin(t3)]
  }
  const cost = (a: number, b: number) => {
    const [x, z] = tip(a, b)
    const want = thumb ? 0.5 + 0.4 * a : 0.35 + 1.2 * a
    return (x - d) ** 2 + (z - h) ** 2 + 0.03 * (b - want) ** 2
  }
  const lo = thumb ? -0.6 : -0.4, hi = thumb ? 1.3 : 1.5, bmax = thumb ? 1.2 : 2.0
  let ba = 0.3, bb = 0.6, bc = Infinity
  for (let a = lo; a <= hi; a += 0.1) for (let b = 0; b <= bmax; b += 0.1) { const c = cost(a, b); if (c < bc) { bc = c; ba = a; bb = b } }
  for (const step of [0.03, 0.01]) {
    const a0 = ba, b0 = bb
    for (let a = a0 - step * 3; a <= a0 + step * 3; a += step) for (let b = Math.max(0, b0 - step * 3); b <= b0 + step * 3; b += step) { const c = cost(a, b); if (c < bc) { bc = c; ba = a; bb = b } }
  }
  const cx = dx, cy = dy
  const ang = [ba, ba + bb, ba + bb + bb * follow]
  const pts: V[] = [root]
  ang.forEach((t, i) => { const p = pts[i]; pts.push([p[0] + cx * L[i] * Math.cos(t), p[1] + cy * L[i] * Math.cos(t), p[2] - L[i] * Math.sin(t)]) })
  return pts
}

interface Frame { s: number; frontY: number; cx: number; cam: V; H: number }

export function drawHand(g: CanvasRenderingContext2D, p: Pose, kw: number, frontY: number, keysH: number, cx: number, dark: boolean) {
  const dir = p.right ? 1 : -1
  const s = kw / KEY_CM // pixels per centimetre
  const keysCm = keysH / s
  const H = 70 // the camera: 70 cm above the keys, 38 cm behind their front edge
  const fr: Frame = { s, frontY, cx, cam: [cx / s, -38, H], H }
  // a point in the world (cm) to the screen, and how much bigger it looks for being closer to the camera
  const proj = (v: V): [number, number, number] => {
    const k = H / Math.max(10, H - v[2])
    const gx = v[0] * s, gy = frontY - v[1] * s
    const ox = cx, oy = frontY + 38 * s
    return [ox + (gx - ox) * k, oy + (gy - oy) * k, k * s]
  }
  const dist = (v: V) => Math.hypot(v[0] - fr.cam[0], v[1] - fr.cam[1], v[2] - fr.cam[2])

  // ---- the wrist: level with the keys and just behind them, so the knuckles sit above the front of the keys
  const wx = p.at * KEY_CM + dir * 1.0
  const wy = -10.4
  const wz = 2.7 + 1.6 * p.speed
  const yaw = p.yaw * dir
  const cs = Math.cos(yaw), sn = Math.sin(yaw)
  const W: V = [wx, wy, wz]
  const toWorld = (l: V): V => { const x = l[0] * dir; return [W[0] + x * cs - l[1] * sn, W[1] + x * sn + l[1] * cs, W[2] + l[2]] }
  const turn = (x: number, y: number): [number, number] => [x * cs - y * sn, x * sn + y * cs]

  // ---- where each fingertip wants to be
  const whiteY = 4.6, blackY = keysCm * 0.38 + 2.4
  const digits: { pts: V[]; thumb: boolean; r: number[]; press: number }[] = []
  for (let f = 1; f <= 5; f++) {
    const d = DIGIT[f], thumb = f === 1
    const root = toWorld(d.root)
    // resting: a loosely curled finger just above the keys, the thumb lower and near the front edge
    let rest: V
    if (thumb) { const [ox, oy] = turn(dir * -2.4, 4.6); rest = [root[0] + ox, root[1] + oy, 1.0] }
    else { const [ox, oy] = turn(dir * (f - 3) * 0.5, 6.1 - (f === 5 ? 0.6 : 0)); rest = [root[0] + ox, root[1] + oy, 1.7 + (f === 5 ? 0.2 : 0)] }
    const tip = p.fingers[f]
    let target = rest, press = 0
    if (tip) {
      press = tip.press
      const black = isBlack(tip.midi)
      const key: V = [tip.x * KEY_CM, black ? blackY : whiteY, black ? 2.1 : 0.9]
      target = lerp(rest, key, press)
      target[2] += 1.3 * Math.sin(Math.PI * press) * (press < 1 ? 1 : 0) // a finger comes down onto its key in an arc
    }
    digits.push({ pts: reach(root, target, d.L, thumb, Math.atan2(cs, -sn)), thumb, r: d.r, press })
  }

  // ---- everything that is drawn, far to near
  const prims: { depth: number; draw: () => void }[] = []

  // shadows of the bones on the keys
  const shadow = () => {
    g.save()
    g.fillStyle = '#000'
    for (const dg of digits) {
      for (let i = 0; i < 3; i++) {
        const a = dg.pts[i], b = dg.pts[i + 1]
        for (const [grow, alpha] of [[1.1, 0.045]] as [number, number][]) {
          g.globalAlpha = alpha
          const oa = proj([a[0] + a[2] * 0.25, a[1] - a[2] * 0.12, 0]), ob = proj([b[0] + b[2] * 0.25, b[1] - b[2] * 0.12, 0])
          capsule(g, oa[0], oa[1], dg.r[i] * s * grow, ob[0], ob[1], dg.r[Math.min(2, i)] * s * grow)
          g.fill()
        }
      }
    }
    g.restore()
  }
  prims.push({ depth: 1e9, draw: shadow })

  // forearm
  prims.push({
    depth: 9e8, draw: () => {
      const back = (len: number, z: number): V => toWorld([0, -len, z])
      const parts: [number, number, number, number, boolean][] = [[0, 17, 2.6, 3.5, false], [15, 40, 3.9, 5.2, true]]
      for (const [a0, a1, ra, rb, sleeve] of parts) {
        const A = proj(back(a0, -0.3 * a0 / 10)), B = proj(back(a1, -0.3 * a1 / 10))
        capsule(g, A[0], A[1], ra * A[2], B[0], B[1], rb * B[2])
        const gr = g.createLinearGradient(A[0] - ra * A[2], 0, A[0] + ra * A[2], 0)
        if (sleeve) { gr.addColorStop(0, dark ? '#59627a' : '#6b7590'); gr.addColorStop(0.5, dark ? '#444c60' : '#555e77'); gr.addColorStop(1, dark ? '#303648' : '#434a60') }
        else { gr.addColorStop(0, tone(SKIN, LIGHT, 0.45)); gr.addColorStop(0.55, rgb(SKIN)); gr.addColorStop(1, tone(SKIN, SHADE, 0.45)) }
        g.fillStyle = gr; g.fill()
        g.strokeStyle = sleeve ? 'rgba(0,0,0,.45)' : rgb(EDGE); g.lineWidth = 1.2; g.stroke()
      }
    },
  })

  // the back of the hand: an arch from the wrist up to the knuckles, with tendons running to each finger
  const outline: V[] = [[2.7, -0.4, 0.2], [3.5, 4.5, 0.9], [3.5, 7.2, 1.6], [2.9, 8.6, 2.2], [1.0, 9.9, 3.1], [-1.0, 10.3, 3.2], [-3.0, 9.7, 2.8], [-3.7, 8.2, 2.1], [-4.6, 6.0, 1.0], [-4.9, 4.2, 0.6], [-4.0, 2.0, 0.1], [-2.7, -0.4, 0.2]]
  prims.push({
    depth: 8e8, draw: () => {
      const pts = outline.map((o) => proj(toWorld(o)))
      const mid = (a: number[], b: number[]): [number, number] => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]
      const edge = new Path2D()
      const m0 = mid(pts[0], pts[1])
      edge.moveTo(m0[0], m0[1])
      for (let i = 1; i < pts.length - 1; i++) { const m = mid(pts[i], pts[i + 1]); edge.quadraticCurveTo(pts[i][0], pts[i][1], m[0], m[1]) }
      const body = new Path2D(edge)
      body.lineTo(pts[pts.length - 1][0], pts[pts.length - 1][1]); body.lineTo(pts[0][0], pts[0][1]); body.closePath()
      const w0 = proj(toWorld([0, 0, 0.2])), k0 = proj(toWorld([0, 10, 3]))
      const gr = g.createLinearGradient(w0[0], w0[1], k0[0], k0[1])
      gr.addColorStop(0, tone(SKIN, SHADE, 0.28)); gr.addColorStop(0.7, rgb(SKIN)); gr.addColorStop(1, tone(SKIN, LIGHT, 0.4))
      g.lineJoin = 'round'
      g.fillStyle = gr; g.fill(body)
      g.strokeStyle = rgb(EDGE); g.lineWidth = 1.4; g.stroke(edge) // the wrist edge is left open, it continues into the forearm
      // tendons
      g.lineCap = 'round'
      for (let f = 2; f <= 5; f++) {
        const a = proj(toWorld([DIGIT[f].root[0] * 0.55, 0.5, 0.9])), m = proj(toWorld([DIGIT[f].root[0] * 0.85, 5.2, 2.4])), b = proj(toWorld([DIGIT[f].root[0], DIGIT[f].root[1] - 0.2, DIGIT[f].root[2] + 0.6]))
        g.strokeStyle = 'rgba(150,98,72,.22)'; g.lineWidth = Math.max(1, 0.55 * s)
        g.beginPath(); g.moveTo(a[0], a[1]); g.quadraticCurveTo(m[0], m[1], b[0], b[1]); g.stroke()
        g.strokeStyle = 'rgba(255,238,220,.2)'; g.lineWidth = Math.max(1, 0.3 * s)
        g.beginPath(); g.moveTo(a[0] + 1, a[1]); g.quadraticCurveTo(m[0] + 1, m[1], b[0] + 1, b[1]); g.stroke()
      }
      // the knuckles
      for (let f = 2; f <= 5; f++) {
        const q = proj(toWorld([DIGIT[f].root[0], DIGIT[f].root[1] - 0.3, DIGIT[f].root[2] + 0.5]))
        const rg = g.createRadialGradient(q[0] - q[2] * 0.3, q[1] - q[2] * 0.3, 0, q[0], q[1], q[2] * 1.1)
        rg.addColorStop(0, 'rgba(255,238,220,.5)'); rg.addColorStop(1, 'rgba(255,238,220,0)')
        g.fillStyle = rg; g.beginPath(); g.arc(q[0], q[1], q[2] * 1.1, 0, Math.PI * 2); g.fill()
      }
    },
  })

  // fingers
  digits.forEach((dg, i) => {
    const mid = proj(dg.pts[2])
    prims.push({
      depth: dist(dg.pts[2]) + i * 0.001 + (dg.thumb ? 0.5 : 0), draw: () => {
        const q = dg.pts.map(proj)
        const rad = dg.r
        // outline first, then the fill, so the joints inside a finger leave no lines
        g.lineCap = 'round'; g.lineJoin = 'round'
        for (let b = dg.thumb ? 1 : 0; b < 3; b++) { capsule(g, q[b][0], q[b][1], rad[b] * q[b][2], q[b + 1][0], q[b + 1][1], rad[b] * 0.92 * q[b + 1][2]); g.strokeStyle = rgb(EDGE); g.lineWidth = 2.4; g.stroke() }
        for (let b = dg.thumb ? 1 : 0; b < 3; b++) {
          const ax = q[b + 1][0] - q[b][0], ay = q[b + 1][1] - q[b][1], l = Math.hypot(ax, ay) || 1
          const nx = -ay / l, ny = ax / l
          // light comes from above and to the left; the lit side of a bone is the one facing it
          const side = nx * -0.6 + ny * -0.8 >= 0 ? 1 : -1
          const r = rad[b] * q[b][2]
          const gr = g.createLinearGradient(q[b][0] + nx * r * side, q[b][1] + ny * r * side, q[b][0] - nx * r * side, q[b][1] - ny * r * side)
          const up = Math.max(0, Math.min(1, (dg.pts[b][2] + dg.pts[b + 1][2]) / 12)) // higher parts catch more light
          gr.addColorStop(0, tone(SKIN, LIGHT, 0.55 + up * 0.5)); gr.addColorStop(0.45, tone(SKIN, LIGHT, 0.1 + up * 0.3)); gr.addColorStop(1, tone(SKIN, SHADE, 0.45))
          capsule(g, q[b][0], q[b][1], rad[b] * q[b][2], q[b + 1][0], q[b + 1][1], rad[b] * 0.92 * q[b + 1][2])
          g.fillStyle = gr; g.fill()
        }
        // creases over the joints, which show more the more the finger bends
        g.strokeStyle = 'rgba(120,72,52,.5)'; g.lineWidth = 1.1
        for (let b = dg.thumb ? 2 : 1; b < 3; b++) {
          const ax = q[b + 1][0] - q[b - 1][0], ay = q[b + 1][1] - q[b - 1][1], l = Math.hypot(ax, ay) || 1
          const nx = -ay / l, ny = ax / l, r = rad[b] * q[b][2] * 0.6
          g.beginPath(); g.moveTo(q[b][0] + nx * r, q[b][1] + ny * r); g.quadraticCurveTo(q[b][0] + (ax / l) * r * 0.5, q[b][1] + (ay / l) * r * 0.5, q[b][0] - nx * r, q[b][1] - ny * r); g.stroke()
        }
        // the nail, on the back of the last bone
        const a = q[2], e = q[3]
        const ax = e[0] - a[0], ay = e[1] - a[1], l = Math.hypot(ax, ay) || 1
        const r = rad[2] * e[2]
        const c0: [number, number] = [a[0] + ax * 0.62, a[1] + ay * 0.62]
        g.save(); g.translate(c0[0], c0[1]); g.rotate(Math.atan2(ay, ax))
        g.beginPath(); g.ellipse(0, 0, Math.max(r * 0.42, l * 0.24), r * 0.46, 0, 0, Math.PI * 2)
        g.fillStyle = dg.press > 0.9 ? 'rgba(236,150,132,.85)' : 'rgba(255,240,230,.72)'; g.fill()
        g.strokeStyle = 'rgba(140,88,66,.55)'; g.lineWidth = 1; g.stroke()
        g.restore()
        void mid
      },
    })
  })

  prims.sort((a, b) => b.depth - a.depth)
  g.save()
  for (const pr of prims) pr.draw()
  g.restore()
}

/** The outline of a tapered round-ended bar from (ax, ay) to (bx, by) as the current path. */
function capsule(g: CanvasRenderingContext2D, ax: number, ay: number, ra: number, bx: number, by: number, rb: number) {
  const dx = bx - ax, dy = by - ay, l = Math.hypot(dx, dy)
  g.beginPath()
  if (l < 1e-3) { g.arc(ax, ay, Math.max(ra, rb), 0, Math.PI * 2); return }
  const ang = Math.atan2(dy, dx)
  // tangent lines of two circles of different size meet the circles at an angle off the perpendicular
  const t = Math.asin(Math.max(-1, Math.min(1, (ra - rb) / l)))
  g.arc(ax, ay, ra, ang + Math.PI / 2 + t, ang - Math.PI / 2 - t, false)
  g.arc(bx, by, rb, ang - Math.PI / 2 - t, ang + Math.PI / 2 + t, false)
  g.closePath()
}
