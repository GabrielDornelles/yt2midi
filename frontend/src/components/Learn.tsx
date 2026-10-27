import { useEffect, useMemo, useState } from 'react'
import { get } from '../lib/api'
import { buildSteps, pickSection, rungs, type First, type Step, type Window, type WindowKind } from '../lib/learn'
import type { LearnPlan, NoteRec, Rgb } from '../types'

const mmss = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`
const KIND_LABEL: Record<WindowKind, string> = { phrase: 'phrases', seconds: 'seconds', notes: 'notes', chords: 'chords' }

/** Which colour (hand index) plays the right and the left hand. */
export function handIndexes(notes: NoteRec[]): { right: number; left: number } {
  const count = [[0, 0], [0, 0]]
  for (const n of notes) count[n.right === false ? 1 : 0][n.hand === 1 ? 1 : 0]++
  return { right: count[0][1] > count[0][0] ? 1 : 0, left: count[1][1] > count[1][0] ? 1 : 0 }
}

/** The state of "help me learn a section". The player is driven from outside (see Overview). */
export function useLesson(videoId: string, notes: NoteRec[], pos: number, aim: number) {
  const [plan, setPlan] = useState<LearnPlan | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    const ac = new AbortController()
    setPlan(null); setFailed(false)
    get<LearnPlan>(`/api/song/${videoId}/chunks`, ac.signal).then(setPlan).catch(() => { if (!ac.signal.aborted) setFailed(true) })
    return () => ac.abort()
  }, [videoId, notes])

  const [win, setWinState] = useState<Window>(() => { try { return JSON.parse(localStorage.getItem('learnWin') ?? '') } catch { return { kind: 'phrase', n: 1 } } })
  const [first, setFirstState] = useState<First>(() => { try { return (localStorage.getItem('learnFirst') as First) || 'auto' } catch { return 'auto' } })
  const setWin = (w: Window) => { setWinState(w); try { localStorage.setItem('learnWin', JSON.stringify(w)) } catch { /* not remembered */ } }
  const setFirst = (f: First) => { setFirstState(f); try { localStorage.setItem('learnFirst', f) } catch { /* not remembered */ } }

  const [steps, setSteps] = useState<Step[] | null>(null)
  const [cur, setCur] = useState(0)
  const [rung, setRung] = useState(0)

  const section = useMemo(() => (plan ? pickSection(plan, pos, win) : null), [plan, pos, win])
  const preview = useMemo(() => (section ? buildSteps(section, first) : []), [section, first])
  const step = steps ? steps[Math.min(cur, steps.length - 1)] : null
  const speeds = step ? rungs(step, aim) : []

  const start = () => { if (preview.length) { setSteps(preview); setCur(0); setRung(0) } }
  const exit = () => { setSteps(null); setCur(0); setRung(0) }
  const go = (i: number) => { if (steps) { setCur(Math.max(0, Math.min(steps.length - 1, i))); setRung(0) } }
  /** "I've got it": a notch faster, and on to the next step after the last notch. */
  const gotIt = () => { if (!step) return; if (rung + 1 < speeds.length) setRung(rung + 1); else if (cur + 1 < steps!.length) go(cur + 1); else exit() }
  const again = () => setRung(Math.max(0, rung - 1))

  return { plan, failed, win, setWin, first, setFirst, section, preview, steps, step, cur, rung, speeds, running: !!step, start, exit, go, gotIt, again }
}
export type Lesson = ReturnType<typeof useLesson>

function Fingers({ s }: { s: Step }) {
  if (s.kind === 'together') return <div className="fingers">{s.fingers.map((f, i) => <div key={i}>{f}</div>)}</div>
  const shown = s.fingers.length > 40 ? [...s.fingers.slice(0, 40), '…'] : s.fingers
  return (
    <div className="fingers">
      {shown.map((f, i) => <span key={i} className={`fing ${f.includes('-') ? 'chord' : ''} ${s.gap === i ? 'turn' : ''}`}>{f}</span>)}
    </div>
  )
}

/** Over the piano: what to practise now, how fast, and the buttons to move on. */
export function LessonBar({ lesson, passes, playing, onToggle }: { lesson: Lesson; passes: number; playing: boolean; onToggle: () => void }) {
  const { step, steps, cur, rung, speeds } = lesson
  if (!step || !steps) return null
  return (
    <div className="lesson-bar" role="group" aria-label="Lesson">
      <div className="lesson-head">
        <b>{step.title}</b>
        <span className="hint">{step.tag} · step {cur + 1} of {steps.length}</span>
        <span className="grow" />
        <button className="btn small" onClick={lesson.exit}>Stop lesson</button>
      </div>
      <p className="hint lesson-sub">{step.sub}</p>
      <Fingers s={step} />
      <div className="lesson-ctl">
        <button className="btn small icon-text" onClick={onToggle}>{playing ? 'Pause' : 'Play'}</button>
        <span className="rungs" aria-label="Speed steps">
          {speeds.map((v, i) => <span key={i} className={`rung ${i === rung ? 'on' : i < rung ? 'done' : ''}`}>{v}%</span>)}
        </span>
        <span className="hint">played {passes} {passes === 1 ? 'time' : 'times'}</span>
        <span className="grow" />
        <button className="btn small" onClick={lesson.again} disabled={rung === 0}>Slower</button>
        <button className="btn small primary" onClick={lesson.gotIt}>{rung + 1 < speeds.length ? 'Got it, faster' : cur + 1 < steps.length ? 'Got it, next' : 'Got it, finish'}</button>
        <button className="btn small" onClick={() => lesson.go(cur - 1)} disabled={cur === 0}>Back</button>
        <button className="btn small" onClick={() => lesson.go(cur + 1)} disabled={cur + 1 >= steps.length}>Skip</button>
      </div>
    </div>
  )
}

/** In the side column: set up a lesson, then see where you are in it. */
export function LearnCard({ lesson, pos, colors, idx }: { lesson: Lesson; pos: number; colors: Rgb[]; idx: { right: number; left: number } }) {
  const { section, preview, win, steps } = lesson
  const dot = (right: boolean) => { const c = colors[right ? idx.right : idx.left]; return c ? <span className="dot" style={{ background: `rgb(${c})` }} /> : null }
  if (steps) {
    return (
      <div className="card pad learn">
        <h2>Your lesson</h2>
        <p className="hint">{steps.length} steps. Tap one to jump to it.</p>
        <ol className="outline">
          {steps.map((s, i) => (
            <li key={s.id}><button className={`step ${i === lesson.cur ? 'on' : i < lesson.cur ? 'done' : ''}`} onClick={() => lesson.go(i)}>{s.title}<small>{s.kind === 'seam' ? 'the turn' : s.kind === 'join' ? 'join' : s.kind === 'cell' ? 'pattern' : ''}</small></button></li>
          ))}
        </ol>
      </div>
    )
  }
  return (
    <div className="card pad learn stack">
      <div>
        <h2>Learn a section</h2>
        <p className="hint">Pick where to start by clicking the music bar. We cut what follows into small parts, with the fingers, one hand at a time.</p>
      </div>
      {lesson.failed ? <p className="hint">Could not prepare the lesson for this song.</p> : !lesson.plan ? <p className="hint">Preparing…</p> : (
        <>
          <div className="learn-row">
            <label>How much</label>
            <input type="number" min={1} max={99} value={win.n} aria-label="Amount" onChange={(e) => lesson.setWin({ ...win, n: Math.max(1, Math.min(99, Number(e.target.value) || 1)) })} />
            <select value={win.kind} aria-label="Unit" onChange={(e) => lesson.setWin({ kind: e.target.value as WindowKind, n: e.target.value === 'seconds' ? 10 : e.target.value === 'notes' ? 12 : e.target.value === 'chords' ? 3 : 1 })}>
              {(Object.keys(KIND_LABEL) as WindowKind[]).map((k) => <option key={k} value={k}>{KIND_LABEL[k]}</option>)}
            </select>
          </div>
          <div className="learn-row">
            <label>Start with</label>
            <select value={lesson.first} onChange={(e) => lesson.setFirst(e.target.value as First)}>
              <option value="auto">the harder hand</option>
              <option value="right">the right hand</option>
              <option value="left">the left hand</option>
            </select>
          </div>
          {section ? (
            <>
              <p className="hint">From <b>{mmss(section.from)}</b> to <b>{mmss(section.to)}</b>: {section.right.length + section.left.length} parts, {preview.length} steps. {pos > 0 && section.from > pos + 0.3 ? `It starts at the first part after ${mmss(pos)}.` : ''}</p>
              <div className="parts">
                {([true, false] as const).map((right) => {
                  const cs = right ? section.right : section.left
                  return cs.length ? (
                    <div key={String(right)}>
                      <h3>{dot(right)} {right ? 'Right hand' : 'Left hand'}</h3>
                      {cs.map((c) => <div className="part" key={c.id}><b>{c.label}</b><span>{c.fingers.slice(0, 14).join(' ')}{c.fingers.length > 14 ? ' …' : ''}</span></div>)}
                    </div>
                  ) : null
                })}
              </div>
              <button className="btn primary big" onClick={lesson.start}>Start learning</button>
            </>
          ) : <p className="hint">There is nothing left to learn after this point.</p>}
        </>
      )}
    </div>
  )
}
