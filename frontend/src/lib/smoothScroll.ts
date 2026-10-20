/*
 * The Linux system web view (WebKitGTK, used by the desktop app) scrolls a mouse wheel in big jumps with no easing,
 * where Chrome and Firefox take small steps and glide. This gives it the same feel: each wheel notch moves about
 * 50 px and eases in. Touchpads (small deltas), keyboard, scrollbar, zoom and scrollable inner boxes are left alone.
 */
const STEP = 0.55        // wheel delta -> scroll distance (a WebKitGTK notch reports 93)
const GLIDE_MS = 60      // how quickly the page catches up with where the wheel says it should be

function needed() {
  const ua = navigator.userAgent
  return /Linux/.test(ua) && /AppleWebKit/.test(ua) && !/Chrome|Chromium|Firefox|Android/.test(ua)
}

/** Is there an inner scrollable box under the pointer that can still move in this direction? */
function innerScroller(el: Element | null, dy: number): boolean {
  for (let e = el; e && e !== document.body && e !== document.documentElement; e = e.parentElement) {
    const o = getComputedStyle(e).overflowY
    if ((o === 'auto' || o === 'scroll') && e.scrollHeight > e.clientHeight + 1) {
      if (dy > 0 ? e.scrollTop + e.clientHeight < e.scrollHeight - 1 : e.scrollTop > 0) return true
    }
  }
  return false
}

export function installSmoothScroll() {
  if (!needed()) return
  let pos = 0, target = 0, raf = 0, last = 0, active = false
  const max = () => Math.max(0, document.documentElement.scrollHeight - window.innerHeight)

  const step = (t: number) => {
    const dt = Math.min(64, t - last)
    last = t
    const diff = target - pos
    if (Math.abs(diff) < 1.2) { pos = target; window.scrollTo(0, pos); active = false; raf = 0; return }
    pos += diff * (1 - Math.exp(-dt / GLIDE_MS))
    window.scrollTo(0, pos)
    raf = requestAnimationFrame(step)
  }

  window.addEventListener('wheel', (e) => {
    // only real wheel notches: pixel mode, big whole deltas; not zoom, not horizontal, not a touchpad
    if (e.ctrlKey || e.deltaMode !== 0 || Math.abs(e.deltaY) < 40 || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return
    if (innerScroller(e.target as Element, e.deltaY)) return
    e.preventDefault()
    if (!active) { pos = target = window.scrollY; active = true; last = performance.now() }
    target = Math.min(max(), Math.max(0, target + e.deltaY * STEP))
    if (!raf) { last = performance.now(); raf = requestAnimationFrame(step) }
  }, { passive: false })

  // something else moved the page (scrollbar, keyboard, a link): follow it instead of fighting it
  window.addEventListener('scroll', () => {
    if (active && Math.abs(window.scrollY - pos) > 3) { active = false; cancelAnimationFrame(raf); raf = 0 }
  }, { passive: true })
}
