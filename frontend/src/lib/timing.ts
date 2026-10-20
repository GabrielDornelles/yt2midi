/*
 * The bar lines of a song. The easy sheet works them out (and lets you correct them); the sheet music uses the very
 * same ones, so "bar 5" means the same on both. The easy sheet leaves what it settled on in local storage.
 */
export interface Timing { beats: number; bpm: number | null; tdown: number | null }

export function readTiming(videoId: string): Timing {
  let beats = 3
  try { beats = Number(JSON.parse(localStorage.getItem('sheetPrefs') ?? '{}').beats) || 3 } catch { /* default */ }
  try {
    const t = JSON.parse(localStorage.getItem(`timing:${videoId}`) ?? 'null') as Timing | null
    if (t && t.beats === beats) return t
  } catch { /* none saved */ }
  return { beats, bpm: null, tdown: null }
}

export function saveTiming(videoId: string, t: Timing) {
  try { localStorage.setItem(`timing:${videoId}`, JSON.stringify(t)) } catch { /* private window: the score just works the bars out itself */ }
}
