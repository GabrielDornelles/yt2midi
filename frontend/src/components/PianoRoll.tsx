import { useEffect, useRef } from 'react'
import type { NoteRec, Rgb } from '../types'

const FALLBACK: Rgb[] = [[224, 142, 90], [110, 160, 200]]

interface Props {
  notes: NoteRec[]
  colors: Rgb[]
  duration: number
  pos: number
  onSeek: (t: number) => void
}

/** Static notes are drawn once to an offscreen canvas; each frame only blits it and draws the playhead. */
export function PianoRoll({ notes, colors, duration, pos, onSeek }: Props) {
  const box = useRef<HTMLDivElement>(null)
  const cv = useRef<HTMLCanvasElement>(null)
  const layer = useRef<HTMLCanvasElement | null>(null)
  const size = useRef({ w: 0, h: 0, dpr: 1 })
  const accent = useRef('#b85c2e')
  const posRef = useRef(pos)
  posRef.current = pos

  const paint = () => {
    const c = cv.current, l = layer.current
    if (!c || !l) return
    const { w, h, dpr } = size.current
    const g = c.getContext('2d')!
    g.setTransform(1, 0, 0, 1, 0, 0)
    g.clearRect(0, 0, c.width, c.height)
    g.drawImage(l, 0, 0)
    const x = (posRef.current / Math.max(duration, 0.001)) * w * dpr
    g.fillStyle = 'rgba(255,255,255,.12)'
    g.fillRect(0, 0, x, h * dpr)
    g.fillStyle = accent.current
    g.fillRect(x - 1 * dpr, 0, 2 * dpr, h * dpr)
  }

  const build = () => {
    const el = box.current, c = cv.current
    if (!el || !c || notes.length === 0) return
    accent.current = getComputedStyle(document.documentElement).getPropertyValue('--accent') || '#b85c2e'
    const dpr = Math.min(window.devicePixelRatio || 1, 2)
    const w = el.clientWidth, h = el.clientHeight
    size.current = { w, h, dpr }
    c.width = w * dpr; c.height = h * dpr
    const l = layer.current ?? (layer.current = document.createElement('canvas'))
    l.width = c.width; l.height = c.height
    const g = l.getContext('2d')!
    const lo = Math.min(...notes.map((n) => n.midi)) - 1, hi = Math.max(...notes.map((n) => n.midi)) + 1
    const kh = (h * dpr) / (hi - lo + 1)
    // faint lines on every C so the pitch range is easy to read
    g.strokeStyle = 'rgba(128,110,90,.25)'
    g.lineWidth = 1
    for (let m = lo; m <= hi; m++) if (m % 12 === 0) {
      const y = (h * dpr) - (m - lo + 0.5) * kh
      g.beginPath(); g.moveTo(0, y); g.lineTo(l.width, y); g.stroke()
    }
    const pal = colors.length ? colors : FALLBACK
    for (const n of notes) {
      const col = pal[n.hand] ?? pal[0]
      g.fillStyle = `rgb(${col[0]},${col[1]},${col[2]})`
      const x = (n.start / duration) * l.width
      const wd = Math.max(2 * dpr, ((n.end - n.start) / duration) * l.width)
      g.fillRect(x, (h * dpr) - (n.midi - lo + 1) * kh + 0.5, wd, Math.max(2, kh - 1))
    }
    paint()
  }

  useEffect(() => {
    build()
    const ro = new ResizeObserver(build)
    if (box.current) ro.observe(box.current)
    return () => ro.disconnect()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [notes, colors, duration])

  useEffect(paint)

  const seek = (e: React.PointerEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
    onSeek(((e.clientX - r.left) / r.width) * duration)
  }
  return (
    <div
      className="roll"
      ref={box}
      onPointerDown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); seek(e) }}
      onPointerMove={(e) => { if (e.buttons) seek(e) }}
      role="slider"
      aria-label="Song position"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(pos)}
    >
      <canvas ref={cv} />
    </div>
  )
}
