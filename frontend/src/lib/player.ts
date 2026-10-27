import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { NoteRec } from '../types'
import { get, post } from './api'

/*
 * In-browser player. Notes are played from recorded grand-piano samples (Salamander Grand Piano by Alexander Holm,
 * CC BY 3.0, fetched by scripts/fetch-piano.sh). Samples exist every minor third, so the nearest one is pitch-shifted
 * by at most 1-2 semitones. If the samples cannot load, a small synth takes over so playback never fails.
 * Notes are scheduled ~1s ahead so thousands of notes stay cheap.
 */

interface Sample { midi: number; buf: AudioBuffer }

// A0, then A / C / D# / F# in octaves 1-7, then C8: the files in /samples/piano
const SAMPLE_FILES: [number, string][] = (() => {
  const out: [number, string][] = [[21, 'A0']]
  for (let o = 1; o <= 7; o++) {
    const c = 12 * (o + 1)
    out.push([c, `C${o}`], [c + 3, `Ds${o}`], [c + 6, `Fs${o}`], [c + 9, `A${o}`])
  }
  out.push([108, 'C8'])
  return out
})()

const KEEP_SECONDS = 7 // notes rarely sustain longer; trimming keeps memory small

/** Decode, mix to mono, trim to KEEP_SECONDS and fade the tail so the cut is inaudible. */
async function loadBank(ctx: AudioContext): Promise<Sample[]> {
  const bank = await Promise.all(SAMPLE_FILES.map(async ([midi, name]) => {
    const res = await fetch(`/samples/piano/${name}.mp3`)
    if (!res.ok) throw new Error(`missing sample ${name}`)
    const full = await ctx.decodeAudioData(await res.arrayBuffer())
    const len = Math.min(full.length, Math.floor(KEEP_SECONDS * full.sampleRate))
    const buf = ctx.createBuffer(1, len, full.sampleRate)
    const out = buf.getChannelData(0)
    const chans = Array.from({ length: full.numberOfChannels }, (_, c) => full.getChannelData(c))
    for (let i = 0; i < len; i++) {
      let v = 0
      for (const ch of chans) v += ch[i]
      out[i] = v / chans.length
    }
    const fade = Math.floor(0.6 * full.sampleRate)
    for (let i = 0; i < Math.min(fade, len); i++) out[len - 1 - i] *= i / fade
    return { midi, buf } as Sample
  }))
  return bank.sort((a, b) => a.midi - b.midi)
}

/** A section played again and again: the notes in [from, until) are heard, and it starts over at `restart`. */
export interface Region { from: number; until: number; restart: number }
const LEAD = 1.2 // seconds (of the clock) the notes fall before a section starts

/** How to play: how fast, and how loud each hand is (0 = silent). */
export interface Playback { speed: number; gains: [number, number] }
export const PLAIN: Playback = { speed: 1, gains: [1, 1] }

/** `videoId` lets the desktop app play the song itself (see /api/audio/*); a browser plays it with Web Audio. */
export function usePlayer(notes: NoteRec[], videoId?: string, playback: Playback = PLAIN) {
  const sorted = useMemo(() => [...notes].sort((a, b) => a.start - b.start), [notes])
  const duration = useMemo(() => notes.reduce((m, n) => Math.max(m, n.end), 0), [notes])
  const [playing, setPlaying] = useState(false)
  const [pos, setPos] = useState(0)
  const [sound, setSound] = useState<'loading' | 'piano' | 'synth'>('loading')
  const ctx = useRef<AudioContext | null>(null)
  const bank = useRef<Sample[] | null>(null)
  const bankReady = useRef<Promise<void> | null>(null)
  const master = useRef<GainNode | null>(null)
  const native = useRef(false)
  const nativeChecked = useRef<Promise<void> | null>(null)
  const lastSync = useRef(0)
  const pb = useRef(playback)
  pb.current = playback
  const region = useRef<Region | null>(null)
  const playRef = useRef<((t: number) => Promise<void>) | null>(null)
  const gen = useRef(0) // a newer start of playback cancels an older one that is still waiting
  const [passes, setPasses] = useState(0)
  // t0: the clock time at which the song would have been at 0 at the current speed, so song time = (clock - t0) * speed
  const st = useRef({ t0: 0, speed: 1, idx: 0, raf: 0, playing: false, offset: 0, shown: 0 })

  const getCtx = () => {
    if (!ctx.current) {
      const AC = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
      ctx.current = new AC()
    }
    return ctx.current
  }

  // start fetching the piano as soon as the player appears, so the first press of Play is instant
  useEffect(() => {
    let alive = true
    nativeChecked.current = get<{ native: boolean }>('/api/audio/status')
      .then((j) => { native.current = !!j.native })
      .catch(() => { native.current = false })
      .then(() => {
        if (native.current) { if (alive) setSound('piano'); return }
        const c = getCtx()
        bankReady.current = loadBank(c)
          .then((b) => { bank.current = b; if (alive) setSound('piano') })
          .catch(() => { if (alive) setSound('synth') })
      })
    return () => { alive = false }
  }, [])

  const killMaster = () => {
    const c = ctx.current, m = master.current
    if (c && m) { m.gain.setTargetAtTime(0, c.currentTime, 0.03); window.setTimeout(() => m.disconnect(), 250) }
    master.current = null
  }

  const clock = () => (native.current ? performance.now() / 1000 : ctx.current!.currentTime)
  const songTime = () => (clock() - st.current.t0) * st.current.speed

  /** `hold` is how long the key stays down, in clock seconds. */
  const sampled = (c: AudioContext, out: AudioNode, n: NoteRec, when: number, b: Sample[], gain: number) => {
    let best = b[0]
    for (const s of b) if (Math.abs(s.midi - n.midi) < Math.abs(best.midi - n.midi)) best = s
    const src = c.createBufferSource()
    src.buffer = best.buf
    src.playbackRate.value = 2 ** ((n.midi - best.midi) / 12)
    // hold while the key is down (a little longer than a staccato blip, like a touch of pedal), then damp
    const hold = Math.min(Math.max(n.end - n.start, 0.3), KEEP_SECONDS - 1) / st.current.speed
    const g = c.createGain()
    g.gain.setValueAtTime(0.9 * gain, when)
    g.gain.setValueAtTime(0.9 * gain, when + hold)
    g.gain.setTargetAtTime(0, when + hold, 0.14)
    src.connect(g).connect(out)
    src.start(when)
    src.stop(when + hold + 1.2)
  }

  const synth = (c: AudioContext, out: AudioNode, n: NoteRec, when: number, gain: number) => {
    const f = 440 * 2 ** ((n.midi - 69) / 12)
    const len = Math.min(Math.max(n.end - n.start, 0.15), 3) / st.current.speed
    const g = c.createGain()
    g.gain.setValueAtTime(0.0001, when)
    g.gain.exponentialRampToValueAtTime(0.16 * gain, when + 0.006)
    g.gain.exponentialRampToValueAtTime(0.05 * gain, when + Math.min(len, 0.6))
    g.gain.exponentialRampToValueAtTime(0.0001, when + len + 0.35)
    for (const [mult, type, amp] of [[1, 'triangle', 1], [2, 'sine', 0.25]] as const) {
      const o = c.createOscillator()
      o.type = type
      o.frequency.value = f * mult
      const a = c.createGain()
      a.gain.value = amp
      o.connect(a).connect(g)
      o.start(when)
      o.stop(when + len + 0.4)
    }
    g.connect(out)
  }

  const tick = useCallback(() => {
    const c = ctx.current, s = st.current
    if (!s.playing || (!c && !native.current)) return
    const now = songTime()
    if (native.current) {
      // the desktop app keeps the time; re-align our clock with it now and then
      const t = performance.now()
      if (t - lastSync.current > 1000) {
        lastSync.current = t
        get<{ pos: number; playing: boolean }>('/api/audio/pos').then((j) => { if (st.current.playing && j.playing) st.current.t0 = performance.now() / 1000 - j.pos / st.current.speed }).catch(() => undefined)
      }
    }
    // notes are scheduled about a second (of song time) ahead on the audio clock, so a busy page never makes them late
    const r = region.current
    while (!native.current && c && s.idx < sorted.length && sorted[s.idx].start < now + 1) {
      const n = sorted[s.idx++]
      const gain = pb.current.gains[n.hand] ?? 1
      const when = s.t0 + n.start / s.speed
      if (r && (n.start < r.from - 0.02 || n.start >= r.until)) continue
      if (gain > 0 && when > c.currentTime - 0.05 && master.current) {
        const at = Math.max(when, c.currentTime)
        if (bank.current) sampled(c, master.current, n, at, bank.current, gain)
        else synth(c, master.current, n, at, gain)
      }
    }
    if (r && now >= r.restart) { setPasses((p) => p + 1); void playRef.current?.(Math.max(0, r.from - LEAD * s.speed)); return }
    // the screen follows at ~20 fps
    const t = performance.now()
    if (t - s.shown >= 50) { s.shown = t; setPos(now) }
    if (now > duration + 1) { s.playing = false; setPlaying(false); setPos(0); s.offset = 0; killMaster(); if (native.current) void post('/api/audio/stop', {}).catch(() => undefined); return }
    s.raf = requestAnimationFrame(tick)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sorted, duration])

  const newMaster = (c: AudioContext) => {
    killMaster()
    const m = c.createGain()
    m.gain.value = 0.8
    const comp = c.createDynamicsCompressor()
    m.connect(comp).connect(c.destination)
    master.current = m
  }

  const play = useCallback(async (wanted: number) => {
    // the video's silent introduction is not worth waiting for: begin just before the first note
    const first = sorted.length ? sorted[0].start : 0
    const from = wanted < first - 0.6 ? Math.max(0, first - 0.6) : wanted
    if (nativeChecked.current) await nativeChecked.current
    const my = ++gen.current
    const s = st.current
    cancelAnimationFrame(s.raf)
    const { speed, gains } = pb.current
    s.speed = speed
    const r = region.current
    if (native.current) {
      let at = from
      try { at = (await post<{ pos: number }>('/api/audio/play', { video_id: videoId, from, speed, gains, ...(r ? { silent_until: r.from - 0.02, until: r.until } : {}) })).pos } catch { /* keep going silently */ }
      if (my !== gen.current) return
      s.t0 = performance.now() / 1000 - at / speed
      lastSync.current = performance.now()
    } else {
      const c = getCtx()
      await c.resume()
      if (bankReady.current) await bankReady.current // a no-op once loaded; first press waits for the piano
      if (my !== gen.current) return
      newMaster(c)
      s.t0 = c.currentTime - from / speed
      s.idx = sorted.findIndex((n) => n.start >= from - 0.02)
      if (s.idx < 0) s.idx = sorted.length
    }
    s.shown = 0
    s.playing = true
    setPlaying(true)
    s.raf = requestAnimationFrame(tick)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sorted, tick, videoId])

  playRef.current = play

  const pause = useCallback(() => {
    gen.current++
    const s = st.current
    s.offset = songTime()
    s.playing = false
    cancelAnimationFrame(s.raf)
    setPos(s.offset)
    setPlaying(false)
    killMaster()
    if (native.current) void post('/api/audio/stop', {}).catch(() => undefined)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const toggle = useCallback(() => {
    if (playing) pause()
    else if (region.current) void play(Math.max(0, region.current.from - LEAD * pb.current.speed))
    else void play(st.current.offset)
  }, [playing, pause, play])

  /** Loop a section (null goes back to playing the whole song). With `go`, it starts right away. */
  const setRegion = useCallback((r: Region | null, go = false) => {
    region.current = r
    setPasses(0)
    if (r) { st.current.offset = Math.max(0, r.from - LEAD * pb.current.speed); setPos(st.current.offset) }
    if (go && r) void play(Math.max(0, r.from - LEAD * pb.current.speed))
    else if (!r && st.current.playing) pause()
  }, [play, pause])
  const seek = useCallback((t: number) => {
    const clamped = Math.max(0, Math.min(duration, t))
    st.current.offset = clamped
    setPos(clamped)
    if (st.current.playing) void play(clamped)
  }, [duration, play])

  // speed or hand volume changed while playing: carry on from the same place. The desktop app takes it live; the
  // browser schedules ahead, so it starts again from here to be heard at once.
  const key = `${playback.speed}|${playback.gains.join(',')}`
  useEffect(() => {
    const s = st.current
    if (!s.playing) return
    const at = songTime()
    if (native.current) {
      void post('/api/audio/set', { speed: pb.current.speed, gains: pb.current.gains }).catch(() => undefined)
      s.speed = pb.current.speed
      s.t0 = clock() - at / s.speed
    } else void play(at)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  useEffect(() => () => {
    cancelAnimationFrame(st.current.raf)
    st.current.playing = false
    killMaster()
    if (native.current) void post('/api/audio/stop', {}).catch(() => undefined)
    void ctx.current?.close()
    ctx.current = null
  }, [])

  /** Start playing from a time in the song (used by "click a measure to play from there"). */
  const playFrom = useCallback((t: number) => {
    const clamped = Math.max(0, Math.min(duration, t))
    st.current.offset = clamped
    setPos(clamped)
    void play(clamped)
  }, [duration, play])

  /** Where the music is right now: smooth, unlike `pos`, which updates about 20 times a second. */
  const getPos = useCallback(() => (st.current.playing ? songTime() : st.current.offset), [])

  return { playing, pos, getPos, duration, toggle, seek, playFrom, sound, setRegion, passes }
}
