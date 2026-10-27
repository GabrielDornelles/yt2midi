export type Rgb = [number, number, number]

export interface Calib {
  video_id: string
  x0: number
  x1: number
  y0: number
  y1: number
  first_note: string
  n_white: number
  baseline_t: number | null
  threshold: number
  hand_t: number | null
  swap_hands: boolean
}

export interface LibraryItem {
  video_id: string
  title: string
  notes: number
  duration: number
  hand_colors: Rgb[]
  ready: boolean
}

export interface Song {
  video_id: string
  title: string
  ready: boolean
  notes?: number
  duration?: number
  hand_colors?: Rgb[]
  first_note?: string | null
  n_white?: number | null
  auto?: boolean
  thumb_t?: number | null
  letter_certain?: boolean
  octave_certain?: boolean
  width: number
  height: number
  fps: number
  calib: Calib | null
  midis: string[]
}

export interface Job<T = unknown> {
  state: 'running' | 'done' | 'error'
  progress: number
  status?: string
  stage?: string
  error?: string
  code?: string
  video_id?: string
  result?: T
}

export interface NoteRec {
  name: string
  midi: number
  start: number
  end: number
  hand: number
  hue: number | null
  /** 1 (thumb) to 5, and whether the right hand plays it; worked out by the engine */
  finger?: number
  right?: boolean
}

export interface Pressed {
  name: string
  rgb: Rgb
}

export interface Detection {
  found: boolean
  x0: number
  x1: number
  y0: number
  y1: number
  n_white: number
  first_note: string
  letter_certain: boolean
  suggest_times: number[]
}

/** A repeating cell inside a chunk ("the same three steps, four times"). */
export interface Cell { events: number; reps: number; same: boolean; first_end: number }

/** A piece of one hand's part that is learned as a unit (see app/core/src/learn.rs). */
export interface Chunk {
  id: number
  right: boolean
  phrase: number
  start: number
  end: number
  onsets: number[]
  notes: number
  fingers: string[]
  kind: 'pattern' | 'run' | 'chords' | 'leaps' | 'melody'
  label: string
  detail: string
  cell: Cell | null
  seam: string | null
  difficulty: number
}

export interface Phrase { id: number; start: number; end: number }
export interface LearnPlan { chunks: Chunk[]; phrases: Phrase[] }
