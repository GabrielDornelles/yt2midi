// The same key geometry as the engine (app/core/src/keyboard.rs), so the overlay follows the pointer with no round-trips.
const WHITE = 'CDEFGAB'
const SEMITONE: Record<string, number> = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 }
const SHARP_AFTER: Record<string, string> = { C: 'C#', D: 'D#', F: 'F#', G: 'G#', A: 'A#' }

export interface Key { name: string; black: boolean; box: [number, number, number, number] }

export function midiNumber(name: string): number {
  const octave = parseInt(name.replace(/^[A-G]#?/, ''), 10)
  return 12 * (octave + 1) + SEMITONE[name[0]] + (name.includes('#') ? 1 : 0)
}

export function whiteNames(first: string, n: number): string[] {
  let i = WHITE.indexOf(first[0])
  let octave = parseInt(first.slice(1), 10)
  const out: string[] = []
  for (let k = 0; k < n; k++) {
    out.push(`${WHITE[i]}${octave}`)
    if (++i === 7) { i = 0; octave++ }
  }
  return out
}

export function buildKeys(box: [number, number, number, number], first: string, nWhite: number): Key[] {
  const [x0, y0, x1, y1] = box
  const w = (x1 - x0) / nWhite
  const h = y1 - y0
  const keys: Key[] = []
  whiteNames(first, nWhite).forEach((nm, i) => {
    const kx = x0 + i * w
    keys.push({ name: nm, black: false, box: [kx, y0, kx + w, y1] })
    if (SHARP_AFTER[nm[0]] && i < nWhite - 1) {
      const bx = kx + w
      keys.push({ name: SHARP_AFTER[nm[0]] + nm.slice(1), black: true, box: [bx - w * 0.3, y0, bx + w * 0.3, y0 + h * 0.6] })
    }
  })
  return keys.sort((a, b) => midiNumber(a.name) - midiNumber(b.name))
}

export const shiftFirst = (first: string, whiteSteps: number): string => {
  let i = WHITE.indexOf(first[0]) + whiteSteps
  let o = parseInt(first.slice(1), 10)
  while (i < 0) { i += 7; o-- }
  while (i > 6) { i -= 7; o++ }
  return `${WHITE[i]}${Math.max(0, Math.min(8, o))}`
}

export const pretty = (name: string) => name.replace('#', '♯')
