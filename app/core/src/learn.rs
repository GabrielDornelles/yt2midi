//! Cuts a song into pieces a person can learn, the way a piano teacher would.
//!
//! The unit of learning is a hand position and a repeated idea, not a number of seconds. So for each hand:
//!  1. notes that start together become events, and each event gets a "how much does a phrase end here" score
//!     (a rest, a hand that has to move, a leap, a change of direction) with a penalty for cutting a finger crossing;
//!  2. repeating cells are found ("the same four notes, moved up a step, four times"): a repeat is learned once and
//!     is never cut in the middle;
//!  3. the rest is cut by dynamic programming into chunks of a size that fits the speed of the music, preferring to
//!     cut where the score is high;
//!  4. phrases (for both hands together) are cut at places where neither hand is in the middle of a repeat.
//!
//! Everything is plain rules; the numbers are for a hobbyist learning a piece, not measurements.
use crate::fingering::{HOME, TOGETHER};
use serde::Serialize;

/// A note with the finger the fingering engine gave it.
#[derive(Clone, Copy, Debug)]
pub struct Nt {
    pub midi: i32,
    pub start: f64,
    pub end: f64,
    pub finger: u8,
    pub right: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cell {
    /// events in one repeat
    pub events: usize,
    pub reps: usize,
    /// the repeat starts on the same keys each time (otherwise it moves along)
    pub same: bool,
    /// time the first repeat ends
    pub first_end: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Chunk {
    pub id: usize,
    pub right: bool,
    pub phrase: usize,
    pub start: f64,
    pub end: f64,
    /// start time of every event (a note or chord)
    pub onsets: Vec<f64>,
    pub notes: usize,
    /// the fingers of every event, lowest note first ("3" or "5-3-1")
    pub fingers: Vec<String>,
    pub kind: &'static str,
    pub label: String,
    pub detail: String,
    pub cell: Option<Cell>,
    /// why joining the previous chunk is a hard moment, if it is
    pub seam: Option<String>,
    pub difficulty: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Phrase {
    pub id: usize,
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Plan {
    pub chunks: Vec<Chunk>,
    pub phrases: Vec<Phrase>,
}

struct Ev {
    t: f64,
    end: f64,
    midi: Vec<i32>, // rising
    fing: Vec<u8>,
    at: f64, // where the hand's home sits (semitones)
}

impl Ev {
    fn low(&self) -> i32 {
        self.midi[0]
    }
    fn top(&self) -> i32 {
        *self.midi.last().unwrap()
    }
}

/// (first event, one past the last, and for a repeat: events per cell and repeats)
type Unit = (usize, usize, Option<(usize, usize)>);

struct Hand {
    right: bool,
    ev: Vec<Ev>,
    med: f64,
    /// strength of the boundary before event i
    b: Vec<f64>,
    /// units: [start, end) event ranges that are not cut; a repeating run is one unit
    units: Vec<Unit>,
}

fn white_keys(a: i32, b: i32) -> i32 {
    let (lo, hi) = (a.min(b), a.max(b));
    (lo..hi).filter(|m| !matches!(m.rem_euclid(12), 1 | 3 | 6 | 8 | 10)).count() as i32
}

fn events(notes: &[Nt], right: bool) -> Vec<Ev> {
    let mut v: Vec<Nt> = notes.iter().copied().filter(|n| n.right == right && n.finger > 0).collect();
    v.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.midi.cmp(&b.midi)));
    let mut out: Vec<Ev> = Vec::new();
    let mut t0 = f64::MIN;
    for n in v {
        if n.start - t0 > TOGETHER || out.is_empty() {
            t0 = n.start;
            out.push(Ev { t: n.start, end: n.end, midi: vec![], fing: vec![], at: 0.0 });
        }
        let e = out.last_mut().unwrap();
        e.end = e.end.max(n.end);
        e.midi.push(n.midi);
        e.fing.push(n.finger);
    }
    for e in &mut out {
        let homes: f64 = e.midi.iter().zip(&e.fing).map(|(&m, &f)| if right { m as f64 - HOME[f as usize] } else { m as f64 + HOME[f as usize] }).sum();
        e.at = homes / e.midi.len() as f64;
    }
    out
}

fn median_gap(ev: &[Ev]) -> f64 {
    let mut g: Vec<f64> = ev.windows(2).map(|w| w[1].t - w[0].t).filter(|&d| d > 0.0).collect();
    if g.is_empty() {
        return 0.5;
    }
    g.sort_by(f64::total_cmp);
    g[g.len() / 2].max(0.08)
}

/// Is going from event a to event b a finger crossing (thumb under or a finger over)?
fn crossing(a: &Ev, b: &Ev, right: bool) -> bool {
    if a.midi.len() != 1 || b.midi.len() != 1 {
        return false;
    }
    let dm = b.midi[0] - a.midi[0];
    let df = b.fing[0] as i32 - a.fing[0] as i32;
    // far apart notes are reached by moving the hand, not by crossing fingers
    if dm == 0 || df == 0 || dm.abs() > 6 {
        return false;
    }
    let normal = if right { dm * df > 0 } else { dm * df < 0 };
    !normal && (a.fing[0] == 1 || b.fing[0] == 1)
}

/// Why going into event b from a is a hard moment, if it is.
fn hard_move(a: &Ev, b: &Ev, right: bool) -> Option<String> {
    if crossing(a, b, right) {
        return Some(if b.fing[0] == 1 { "thumb goes under".into() } else { "fingers go over the thumb".into() });
    }
    let leap = (b.low() - a.low()).abs().max((b.top() - a.top()).abs());
    if leap >= 10 {
        let keys = white_keys(a.low(), b.low()).max(white_keys(a.top(), b.top()));
        return Some(format!("jump of {keys} keys"));
    }
    let shift = (b.at - a.at).abs();
    if shift >= 5.0 {
        return Some(format!("hand moves {} keys", ((shift * 7.0 / 12.0).round() as i32).max(1)));
    }
    None
}

fn boundaries(h: &mut Hand) {
    let n = h.ev.len();
    h.b = vec![0.0; n + 1];
    for i in 1..n {
        let (a, b) = (&h.ev[i - 1], &h.ev[i]);
        let r = (b.t - a.t) / h.med;
        let mut s = 0.0;
        if r > 1.7 {
            s += (r - 1.0).min(3.0) * 0.9;
        }
        if b.t - a.end > 0.25 {
            s += 0.8;
        }
        let shift = (b.at - a.at).abs();
        if shift >= 4.0 {
            s += (shift / 6.0).min(1.6);
        }
        if (b.low() - a.low()).abs().max((b.top() - a.top()).abs()) >= 10 {
            s += 0.6;
        }
        if i >= 2 {
            let (d1, d2) = (a.top() - h.ev[i - 2].top(), b.top() - a.top());
            if d1 * d2 < 0 {
                s += 0.4;
            }
        }
        if a.midi.len() != b.midi.len() {
            s += 0.3;
        }
        if crossing(a, b, h.right) {
            s -= 1.5;
        }
        h.b[i] = s;
    }
}

/// Do the cells starting at events s and t (p events each) say the same thing? Same shape inside, same rhythm.
fn same_cell(ev: &[Ev], s: usize, t: usize, p: usize) -> bool {
    for k in 0..p {
        let (a, b) = (&ev[s + k], &ev[t + k]);
        if a.midi.len() != b.midi.len() || (p == 1 && a.midi != b.midi) {
            return false;
        }
        if k > 0 {
            let (pa, pb) = (&ev[s + k - 1], &ev[t + k - 1]);
            if a.low() - pa.low() != b.low() - pb.low() || a.top() - pa.top() != b.top() - pb.top() {
                return false;
            }
        }
        // chords inside a cell must have the same shape
        for j in 1..a.midi.len() {
            if a.midi[j] - a.midi[0] != b.midi[j] - b.midi[0] {
                return false;
            }
        }
        if k + 1 < p {
            let (da, db) = (ev[s + k + 1].t - a.t, ev[t + k + 1].t - b.t);
            if (da - db).abs() > (0.06f64).max(0.25 * da.max(db)) {
                return false;
            }
        }
    }
    true
}

/// Repeats of a cell, as (start event, cell events, reps), longest first. Plain scales and runs are not patterns.
fn repeats(ev: &[Ev]) -> Vec<(usize, usize, usize)> {
    let n = ev.len();
    let mut found = Vec::new();
    for p in 1..=8usize {
        let mut s = 0;
        while s + 2 * p <= n {
            if s >= p && same_cell(ev, s - p, s, p) {
                s += 1;
                continue; // an earlier start covers this
            }
            let mut reps = 1;
            while s + (reps + 1) * p <= n && same_cell(ev, s, s + reps * p, p) {
                reps += 1;
            }
            // the steps of one period: if they are all equal it is a scale (or one note again and again)
            let ok_len = match p {
                1 => reps >= 4,
                2 => reps >= 3,
                _ => reps >= 2,
            };
            if ok_len && reps * p >= 6 {
                let step = |i: usize| (ev[s + i + 1].top() - ev[s + i].top(), ev[s + i + 1].low() - ev[s + i].low());
                let steps: Vec<(i32, i32)> = (0..p).map(|i| if s + i + 1 < n { step(i) } else { (0, 0) }).collect();
                let uniform = steps.iter().all(|&x| x == steps[0]);
                let stepwise = steps.iter().all(|&(t, l)| t != 0 && t.signum() == steps[0].0.signum() && t.abs() <= 2 && l.abs() <= 2 && l.signum() == steps[0].1.signum());
                if stepwise {
                    s += 1;
                    continue;
                }
                if !uniform || (steps[0] == (0, 0) && reps >= 4) {
                    found.push((s, p, reps));
                    s += reps * p;
                    continue;
                }
            }
            s += 1;
        }
    }
    found.sort_by(|a, b| (b.1 * b.2).cmp(&(a.1 * a.2)).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0)));
    found
}

/// Largest number of events one learning unit of a repeat may have before it is split into pieces.
const RUN_MAX: usize = 20;

fn make_units(h: &mut Hand) {
    let n = h.ev.len();
    let mut taken = vec![false; n];
    let mut runs: Vec<(usize, usize, usize)> = Vec::new();
    for (s, p, reps) in repeats(&h.ev) {
        // a cell starts on its lowest note (the downbeat of a waltz bass), when that still leaves whole repeats
        let low = (0..p).min_by_key(|&k| (h.ev[s + k].low(), k)).unwrap_or(0);
        let (s, reps) = if p >= 2 && low > 0 && reps >= 3 { (s + low, reps - 1) } else { (s, reps) };
        if taken[s..s + p * reps].iter().any(|&x| x) {
            continue;
        }
        taken[s..s + p * reps].iter_mut().for_each(|x| *x = true);
        let per = (RUN_MAX / p).max(1); // cells per piece
        let pieces = reps.div_ceil(per);
        let base = reps / pieces;
        let extra = reps % pieces;
        let mut at = s;
        for k in 0..pieces {
            let r = base + usize::from(k < extra);
            runs.push((at, p, r));
            at += r * p;
        }
    }
    runs.sort_unstable();
    let mut units = Vec::new();
    let mut i = 0;
    let mut r = 0;
    while i < n {
        if r < runs.len() && runs[r].0 == i {
            let (s, p, reps) = runs[r];
            units.push((s, s + p * reps, Some((p, reps))));
            i = s + p * reps;
            r += 1;
        } else {
            units.push((i, i + 1, None));
            i += 1;
        }
    }
    // a repeat ends an idea at both of its edges
    for &(a, b, run) in &units {
        if run.is_some() {
            h.b[a] += 1.2;
            if b < n {
                h.b[b] += 1.2;
            }
        }
    }
    h.units = units;
}

/// How many events a chunk should have so it can be held in mind at the music's speed.
fn target(med: f64) -> f64 {
    (2.6 / med).round().clamp(4.0, 8.0)
}

fn cut_chunks(h: &Hand, from: usize, to: usize) -> Vec<(usize, usize)> {
    // units fully inside [from, to)
    let us: Vec<&Unit> = h.units.iter().filter(|u| u.0 >= from && u.1 <= to).collect();
    let m = us.len();
    if m == 0 {
        return vec![];
    }
    let t = target(h.med);
    let cost = |i: usize, j: usize| -> f64 {
        let l = (us[j].1 - us[i].0) as f64;
        let mut c = if l > t { ((l - t) / t).powi(2) } else { 0.6 * ((t - l) / t).powi(2) };
        if l < 3.0 && m > 1 {
            c += 1.5;
        }
        if l > 2.2 * t && j > i {
            c += 3.0 + (l - 2.2 * t) * 0.3;
        }
        c
    };
    let mut f = vec![f64::MAX; m + 1];
    let mut back = vec![0usize; m + 1];
    f[0] = 0.0;
    for j in 1..=m {
        for i in (0..j).rev() {
            let l = us[j - 1].1 - us[i].0;
            if l as f64 > 3.0 * t && j - 1 > i {
                break;
            }
            let bonus = if i > 0 { h.b[us[i].0] } else { 0.0 };
            let c = f[i] + cost(i, j - 1) - bonus;
            if c < f[j] {
                f[j] = c;
                back[j] = i;
            }
        }
    }
    let mut cuts = Vec::new();
    let mut j = m;
    while j > 0 {
        let i = back[j];
        cuts.push((us[i].0, us[j - 1].1));
        j = i;
    }
    cuts.reverse();
    cuts
}

fn fingers_of(e: &Ev) -> String {
    let v: Vec<String> = e.fing.iter().map(|f| f.to_string()).collect();
    v.join("-")
}

fn describe(h: &Hand, a: usize, b: usize, cell: &Option<Cell>) -> (&'static str, String, String) {
    let ev = &h.ev[a..b];
    let l = ev.len();
    let chords = ev.iter().filter(|e| e.midi.len() >= 3).count();
    let tops: Vec<i32> = ev.iter().map(|e| e.top()).collect();
    let steps: Vec<i32> = tops.windows(2).map(|w| w[1] - w[0]).collect();
    let stepwise = steps.iter().filter(|s| s.abs() <= 2 && **s != 0).count();
    let net = tops.last().unwrap() - tops[0];
    let jump = ev.windows(2).map(|w| (w[1].low() - w[0].low()).abs().max((w[1].top() - w[0].top()).abs())).max().unwrap_or(0);
    if let Some(c) = cell {
        let how = if c.same { "the same keys each time" } else { "moving along the keyboard" };
        return ("pattern", format!("Repeating pattern x{}", c.reps), format!("{} step{} played {} times, {}. Learn the pattern once, then repeat it.", c.events, if c.events == 1 { "" } else { "s" }, c.reps, how));
    }
    if chords * 10 >= l * 6 && l >= 2 {
        return ("chords", if l == 2 { "Chord change".into() } else { format!("{l} chords") }, "Find the shape of the first chord, then see which fingers move to the next.".into());
    }
    let one_way = steps.iter().filter(|s| **s > 0).count().max(steps.iter().filter(|s| **s < 0).count());
    if l >= 5 && !steps.is_empty() && one_way * 10 >= steps.len() * 9 && stepwise * 10 < steps.len() * 7 && net.abs() >= 12 {
        let dir = if net > 0 { "Sweep up" } else { "Sweep down" };
        return ("run", dir.into(), "The hand travels in one direction. Let the arm carry it and keep the fingers relaxed.".into());
    }
    if l >= 5 && !steps.is_empty() && stepwise * 10 >= steps.len() * 7 {
        let dir = if net >= 3 { "Run up" } else if net <= -3 { "Run down" } else { "Run" };
        return ("run", dir.into(), "Move in small steps. Keep the fingers close to the keys and the hand quiet.".into());
    }
    if jump >= 12 {
        let keys = ev.windows(2).map(|w| white_keys(w[0].low(), w[1].low()).max(white_keys(w[0].top(), w[1].top()))).max().unwrap_or(0);
        return ("leaps", "Leaps".into(), format!("Biggest jump is {keys} keys. Look at where you land before you move."));
    }
    ("melody", if ev.iter().all(|e| e.midi.len() == 1) { "Melody".into() } else { "Phrase".into() }, "Learn the fingers first, then the rhythm.".into())
}

fn difficulty(h: &Hand, a: usize, b: usize) -> f64 {
    let ev = &h.ev[a..b];
    let l = ev.len() as f64;
    let dur = (ev.last().unwrap().t - ev[0].t).max(0.3);
    let rate = (l / dur).min(12.0);
    let moves = ev.windows(2).filter(|w| hard_move(&w[0], &w[1], h.right).is_some()).count() as f64;
    let size = ev.iter().map(|e| e.midi.len() as f64).sum::<f64>() / l;
    let black = ev.iter().flat_map(|e| e.midi.iter()).filter(|m| matches!(m.rem_euclid(12), 1 | 3 | 6 | 8 | 10)).count() as f64 / ev.iter().map(|e| e.midi.len() as f64).sum::<f64>();
    rate / 3.0 + 1.5 * moves / l + 0.4 * (size - 1.0) + 0.3 * black
}

pub fn plan(notes: &[Nt]) -> Plan {
    let mut hands: Vec<Hand> = [true, false]
        .into_iter()
        .map(|right| {
            let ev = events(notes, right);
            let med = median_gap(&ev);
            Hand { right, ev, med, b: vec![], units: vec![] }
        })
        .filter(|h| !h.ev.is_empty())
        .collect();
    for h in &mut hands {
        boundaries(h);
        make_units(h);
    }
    if hands.is_empty() {
        return Plan::default();
    }
    // ---- phrases: the places both hands can start afresh
    let mut times: Vec<f64> = hands.iter().flat_map(|h| h.ev.iter().map(|e| e.t)).collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() <= TOGETHER);
    let first_idx = |h: &Hand, t: f64| h.ev.partition_point(|e| e.t < t - TOGETHER);
    // strength and legality of a cut at times[k]
    let cut: Vec<Option<f64>> = times
        .iter()
        .map(|&t| {
            let mut s: f64 = 0.0;
            for h in &hands {
                let i = first_idx(h, t);
                if i > 0 && i < h.ev.len() {
                    if !h.units.iter().any(|u| u.0 == i) {
                        return None;
                    }
                    if (h.ev[i].t - t).abs() <= TOGETHER {
                        s = s.max(h.b[i]);
                    }
                }
            }
            let before = hands.iter().flat_map(|h| h.ev.iter().filter(|e| e.t < t - TOGETHER).map(|e| e.end)).fold(f64::MIN, f64::max);
            if t - before > 0.2 {
                s += 1.5;
            }
            Some(s)
        })
        .collect();
    let mut bounds: Vec<usize> = vec![0]; // indexes into times where a phrase starts
    {
        let m = times.len();
        let end_t = hands.iter().flat_map(|h| h.ev.last().map(|e| e.end)).fold(0.0, f64::max);
        let tm = |k: usize| if k < m { times[k] } else { end_t };
        let target_s = 6.0;
        let cost = |i: usize, j: usize| {
            let d = tm(j) - tm(i);
            let mut c = ((d - target_s) / target_s).powi(2) * if d < target_s { 0.6 } else { 1.0 };
            if d < 2.0 {
                c += 2.0;
            }
            c
        };
        let mut f = vec![f64::MAX; m + 1];
        let mut back = vec![0usize; m + 1];
        f[0] = 0.0;
        for j in 1..=m {
            for i in (0..j).rev() {
                let d = tm(j) - tm(i);
                if d > 16.0 && i + 1 < j {
                    break;
                }
                if d < 2.5 && j < m {
                    continue; // a phrase is long enough to be worth a name
                }
                if i > 0 && cut[i].is_none() {
                    continue;
                }
                if j < m && cut[j].is_none() {
                    continue;
                }
                if f[i] == f64::MAX {
                    continue;
                }
                let c = f[i] + cost(i, j) - if i > 0 { cut[i].unwrap_or(0.0) } else { 0.0 };
                if c < f[j] {
                    f[j] = c;
                    back[j] = i;
                }
            }
        }
        if m > 0 && f[m] < f64::MAX {
            let mut j = m;
            bounds.clear();
            while j > 0 {
                let i = back[j];
                bounds.push(i);
                j = i;
            }
            bounds.reverse();
        }
    }
    let mut out = Plan::default();
    let end_t = hands.iter().flat_map(|h| h.ev.last().map(|e| e.end)).fold(0.0, f64::max);
    for (pi, &k) in bounds.iter().enumerate() {
        let a = times[k];
        let b = bounds.get(pi + 1).map(|&k2| times[k2]).unwrap_or(end_t + 1.0);
        out.phrases.push(Phrase { id: pi, start: a, end: b });
    }
    // ---- chunks inside each phrase, for each hand
    for h in &hands {
        let mut prev_last: Option<usize> = None;
        for ph in &out.phrases {
            let (from, to) = (first_idx(h, ph.start), if ph.id + 1 == out.phrases.len() { h.ev.len() } else { first_idx(h, ph.end) });
            if from >= to {
                continue;
            }
            for (a, b) in cut_chunks(h, from, to) {
                let cell = h.units.iter().find(|u| u.0 == a && u.1 == b && u.2.is_some()).and_then(|u| u.2).map(|(p, reps)| Cell {
                    events: p,
                    reps,
                    same: h.ev[a].low() == h.ev[a + p].low(),
                    first_end: h.ev[a + p - 1].end.min(h.ev[a + p].t + 0.0),
                });
                let (kind, label, detail) = describe(h, a, b, &cell);
                let last = &h.ev[b - 1];
                let end = last.end.min(last.t + (0.5f64).max(2.0 * h.med));
                let seam = prev_last.and_then(|p| if p + 1 == a { hard_move(&h.ev[p], &h.ev[a], h.right) } else { None });
                out.chunks.push(Chunk {
                    id: 0,
                    right: h.right,
                    phrase: ph.id,
                    start: h.ev[a].t,
                    end,
                    onsets: h.ev[a..b].iter().map(|e| e.t).collect(),
                    notes: h.ev[a..b].iter().map(|e| e.midi.len()).sum(),
                    fingers: h.ev[a..b].iter().map(fingers_of).collect(),
                    kind,
                    label,
                    detail,
                    cell,
                    seam,
                    difficulty: difficulty(h, a, b),
                });
                prev_last = Some(b - 1);
            }
        }
    }
    out.chunks.sort_by(|x, y| x.start.total_cmp(&y.start).then(y.right.cmp(&x.right)));
    for (i, c) in out.chunks.iter_mut().enumerate() {
        c.id = i;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(midi: i32, start: f64, finger: u8, right: bool) -> Nt {
        Nt { midi, start, end: start + 0.2, finger, right }
    }

    /// bass, chord, chord (left hand) four times on the same keys, then something else
    fn waltz() -> Vec<Nt> {
        let mut v = Vec::new();
        for bar in 0..4 {
            let t = bar as f64 * 1.5;
            v.push(n(43, t, 5, false));
            for (k, off) in [0.5, 1.0].iter().enumerate() {
                let _ = k;
                v.push(n(52, t + off, 3, false));
                v.push(n(55, t + off, 2, false));
                v.push(n(59, t + off, 1, false));
            }
        }
        v.push(n(48, 6.0, 5, false));
        v.push(n(55, 6.5, 2, false));
        v
    }

    #[test]
    fn finds_a_repeating_cell_and_keeps_it_whole() {
        let p = plan(&waltz());
        let cells: Vec<&Chunk> = p.chunks.iter().filter(|c| c.cell.is_some()).collect();
        assert_eq!(cells.len(), 1, "{:#?}", p.chunks.iter().map(|c| (&c.label, c.onsets.len())).collect::<Vec<_>>());
        let c = cells[0].cell.as_ref().unwrap();
        assert_eq!((c.events, c.reps, c.same), (3, 4, true));
        assert_eq!(cells[0].onsets.len(), 12);
        assert_eq!(cells[0].fingers[..3], ["5".to_string(), "3-2-1".to_string(), "3-2-1".to_string()]);
    }

    #[test]
    fn a_scale_is_a_run_not_a_pattern() {
        let notes: Vec<Nt> = [60, 62, 64, 65, 67, 69, 71, 72].iter().enumerate().map(|(i, &m)| n(m, i as f64 * 0.25, [1, 2, 3, 1, 2, 3, 4, 5][i], true)).collect();
        let p = plan(&notes);
        assert!(p.chunks.iter().all(|c| c.cell.is_none()));
        assert!(p.chunks.iter().any(|c| c.kind == "run"));
    }

    #[test]
    fn long_music_is_cut_into_small_chunks_that_cover_everything() {
        // a melody of 60 different notes
        let notes: Vec<Nt> = (0..60).map(|i| n(60 + (i * 7) % 11 + (i / 5) % 3, i as f64 * 0.4, 1 + (i % 5) as u8, true)).collect();
        let p = plan(&notes);
        let total: usize = p.chunks.iter().map(|c| c.onsets.len()).sum();
        assert_eq!(total, 60);
        assert!(p.chunks.iter().all(|c| c.onsets.len() <= 14), "{:?}", p.chunks.iter().map(|c| c.onsets.len()).collect::<Vec<_>>());
        assert!(p.chunks.len() >= 6);
        // chunks are in order and do not overlap
        for w in p.chunks.windows(2) {
            assert!(w[0].onsets.last().unwrap() < &w[1].start);
        }
    }

    #[test]
    fn a_rest_ends_a_chunk() {
        let mut notes: Vec<Nt> = (0..5).map(|i| n(60 + i * 2, i as f64 * 0.4, 1 + i as u8, true)).collect();
        notes.extend((0..5).map(|i| n(72 - i * 2, 6.0 + i as f64 * 0.4, 5 - i as u8, true)));
        let p = plan(&notes);
        assert_eq!(p.chunks.len(), 2);
        assert!(p.chunks[0].end < p.chunks[1].start);
    }

    #[test]
    fn both_hands_share_phrases() {
        let mut notes = waltz();
        notes.extend((0..24).map(|i| n(72 + (i % 5) * 2, i as f64 * 0.3, 1 + (i % 5) as u8, true)));
        let p = plan(&notes);
        assert!(!p.phrases.is_empty());
        for c in &p.chunks {
            let ph = &p.phrases[c.phrase];
            assert!(c.start >= ph.start - 0.07 && c.start < ph.end, "chunk {} outside its phrase", c.id);
        }
    }

    #[test]
    fn a_hard_join_is_named() {
        // thumb under between two groups
        let notes = vec![n(60, 0.0, 1, true), n(62, 0.3, 2, true), n(64, 0.6, 3, true), n(65, 0.9, 1, true), n(67, 1.2, 2, true)];
        let a = &events(&notes, true);
        assert_eq!(hard_move(&a[2], &a[3], true).as_deref(), Some("thumb goes under"));
    }

    #[test]
    fn empty_and_tiny_songs_do_not_panic() {
        assert!(plan(&[]).chunks.is_empty());
        let p = plan(&[n(60, 0.0, 1, true)]);
        assert_eq!(p.chunks.len(), 1);
    }
}
