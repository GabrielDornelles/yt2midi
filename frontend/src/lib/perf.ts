/** Developer probe (only when the app starts with YT2MIDI_PERF=<file>): reports frame times while you scroll. */
export async function startPerfProbe() {
  const dev = await fetch('/api/dev').then((r) => r.json()).catch(() => ({ perf: false }))
  if (!dev.perf) return
  let last = performance.now(), frames: number[] = [], long = 0
  const loop = (t: number) => { frames.push(t - last); last = t; requestAnimationFrame(loop) }
  requestAnimationFrame(loop)
  try { new PerformanceObserver((l) => { long += l.getEntries().length }).observe({ entryTypes: ['longtask'] }) } catch { /* not supported */ }
  const wheels: number[][] = []
  window.addEventListener('wheel', (e) => { wheels.push([e.deltaMode, Math.round(e.deltaY * 10) / 10, Math.round(performance.now())]) }, { passive: true })
  let sy = window.scrollY, scrolled = 0, trace: number[][] = []
  window.addEventListener('scroll', () => { scrolled += Math.abs(window.scrollY - sy); sy = window.scrollY; trace.push([Math.round(performance.now()), Math.round(sy)]) }, { passive: true })
  setInterval(() => {
    if (frames.length < 3) return
    const f = [...frames].sort((a, b) => a - b)
    const q = (p: number) => f[Math.min(f.length - 1, Math.floor(p * f.length))]
    const body = JSON.stringify({ route: location.hash, n: frames.length, fps: +(1000 / (f.reduce((a, b) => a + b, 0) / f.length)).toFixed(1),
      p50: +q(0.5).toFixed(1), p95: +q(0.95).toFixed(1), max: +f[f.length - 1].toFixed(0), longtasks: long, scrolledPx: scrolled, trace: scrolled ? trace : [], wheels: wheels.splice(0, 12), dpr: devicePixelRatio, w: innerWidth, h: innerHeight })
    frames = []; long = 0; scrolled = 0; trace = []
    void fetch('/api/perf', { method: 'POST', body })
  }, 2000)
}
