import { useEffect, useRef } from 'react'
import { buildKeys, pretty } from '../lib/keyboard'
import type { Pressed } from '../types'

export type Box = [number, number, number, number]
interface Props {
  img: HTMLImageElement | null
  box: Box
  nWhite: number
  first: string
  pressed: Pressed[]
  onBox: (b: Box) => void
}

type Drag = { mode: 'l' | 'r' | 't' | 'b' | 'move' | 'new'; start: [number, number]; box: Box }

export function KeyboardCanvas({ img, box, nWhite, first, pressed, onBox }: Props) {
  const cv = useRef<HTMLCanvasElement>(null)
  const drag = useRef<Drag | null>(null)

  useEffect(() => {
    const c = cv.current
    if (!c || !img) return
    c.width = img.naturalWidth; c.height = img.naturalHeight
    const g = c.getContext('2d')!
    g.drawImage(img, 0, 0)
    const sc = c.width / 1920
    const [x0, y0, x1, y1] = box
    if (x1 - x0 > 10 && y1 - y0 > 5 && nWhite > 0) {
      const keys = buildKeys(box, first, nWhite)
      g.lineWidth = Math.max(1, 2 * sc)
      for (const k of keys) if (!k.black) {
        const [a, b, cc, d] = k.box
        g.fillStyle = 'rgba(70,200,255,.20)'; g.fillRect(a, b, cc - a, d - b)
        g.strokeStyle = 'rgba(70,200,255,.9)'; g.strokeRect(a, b, cc - a, d - b)
      }
      for (const k of keys) if (k.black) {
        const [a, b, cc, d] = k.box
        g.fillStyle = 'rgba(90,40,170,.55)'; g.fillRect(a, b, cc - a, d - b)
        g.strokeStyle = 'rgba(190,140,255,.95)'; g.strokeRect(a, b, cc - a, d - b)
      }
      const pitch = (x1 - x0) / nWhite
      const fs = Math.max(9, Math.min(24, pitch * 0.42))
      g.textAlign = 'center'; g.textBaseline = 'bottom'
      for (const k of keys) {
        const [a, b, cc, d] = k.box
        const mid = (a + cc) / 2
        if (k.black) {
          if (fs < 12) continue
          g.fillStyle = '#efe2ff'; g.font = `${fs * 0.62}px system-ui`
          g.fillText(pretty(k.name.replace(/\d/, '')), mid, b + (d - b) * 0.9)
          continue
        }
        const isC = k.name[0] === 'C'
        if (!isC && pitch < fs * 1.1) continue
        g.font = `${isC ? 'bold ' : ''}${fs}px system-ui`
        g.lineWidth = 3; g.strokeStyle = '#000'; g.fillStyle = '#fff'
        const t = isC ? k.name : k.name.replace(/\d/, '')
        const ly = d - (d - b) * 0.2
        g.strokeText(t, mid, ly); g.fillText(t, mid, ly)
      }
      for (const p of pressed) {
        const k = keys.find((q) => q.name === p.name)
        if (!k) continue
        const [a, b, cc, d] = k.box
        g.fillStyle = `rgba(${p.rgb[0]},${p.rgb[1]},${p.rgb[2]},.55)`; g.fillRect(a, b, cc - a, d - b)
        g.lineWidth = Math.max(2, 3 * sc); g.strokeStyle = '#fff'; g.strokeRect(a, b, cc - a, d - b)
      }
    }
    // the box itself, with grab handles
    g.setLineDash([10 * sc, 6 * sc]); g.lineWidth = Math.max(2, 3 * sc); g.strokeStyle = '#ffd24a'
    g.strokeRect(x0, y0, x1 - x0, y1 - y0); g.setLineDash([])
    g.fillStyle = '#ffd24a'
    const hs = 8 * sc
    for (const [hx, hy] of [[x0, (y0 + y1) / 2], [x1, (y0 + y1) / 2], [(x0 + x1) / 2, y0], [(x0 + x1) / 2, y1]]) g.fillRect(hx - hs, hy - hs, hs * 2, hs * 2)
  }, [img, box, nWhite, first, pressed])

  const point = (e: React.PointerEvent): [number, number] => {
    const c = cv.current!, r = c.getBoundingClientRect()
    return [((e.clientX - r.left) * c.width) / r.width, ((e.clientY - r.top) * c.height) / r.height]
  }
  const hit = (p: [number, number]): Drag['mode'] | null => {
    const c = cv.current!, m = (14 * c.width) / c.getBoundingClientRect().width
    const [x0, y0, x1, y1] = box, [x, y] = p
    const inX = x > x0 - m && x < x1 + m, inY = y > y0 - m && y < y1 + m
    if (inY && Math.abs(x - x0) < m) return 'l'
    if (inY && Math.abs(x - x1) < m) return 'r'
    if (inX && Math.abs(y - y0) < m) return 't'
    if (inX && Math.abs(y - y1) < m) return 'b'
    if (x > x0 && x < x1 && y > y0 && y < y1) return 'move'
    return null
  }
  const cursors = { l: 'ew-resize', r: 'ew-resize', t: 'ns-resize', b: 'ns-resize', move: 'move' } as const

  return (
    <canvas
      ref={cv}
      aria-label="Video frame with the keyboard overlay. Drag the dashed box edges to fit the keyboard."
      style={{ cursor: 'crosshair', aspectRatio: img ? `${img.naturalWidth} / ${img.naturalHeight}` : '16 / 9' }}
      onPointerDown={(e) => {
        const p = point(e), m = hit(p)
        e.currentTarget.setPointerCapture(e.pointerId)
        drag.current = { mode: m ?? 'new', start: p, box }
      }}
      onPointerMove={(e) => {
        const p = point(e), d = drag.current
        if (!d) { const m = hit(p); e.currentTarget.style.cursor = m ? cursors[m as keyof typeof cursors] : 'crosshair'; return }
        const dx = p[0] - d.start[0], dy = p[1] - d.start[1]
        let b: Box = [...d.box]
        if (d.mode === 'new') b = [Math.min(d.start[0], p[0]), Math.min(d.start[1], p[1]), Math.max(d.start[0], p[0]), Math.max(d.start[1], p[1])]
        else if (d.mode === 'move') b = [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy]
        else { if (d.mode === 'l') b[0] += dx; if (d.mode === 'r') b[2] += dx; if (d.mode === 't') b[1] += dy; if (d.mode === 'b') b[3] += dy }
        if (b[2] - b[0] > 20 && b[3] - b[1] > 10) onBox(b)
      }}
      onPointerUp={() => { drag.current = null }}
    />
  )
}
