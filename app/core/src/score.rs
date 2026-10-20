//! MIDI to readable sheet music, as MusicXML (which MuseScore, Finale, Sibelius and Verovio all open).
//!
//! The notes come from a video of someone playing, so their times are a performance, not a score. The aim is a clean
//! draft that can be read and learned, not a faithful copy of every wobble:
//!   * beats are tracked (so a slowly drifting tempo still lands on the grid) and everything is snapped to a grid of
//!     sixteenths, or triplet eighths in the beats that clearly are triplets;
//!   * notes struck within a few hundredths of a second of each other are one chord (a rolled chord, or a finger a
//!     little early);
//!   * a note is held until the next one starts unless there is a real gap, because keys are never released at
//!     exactly the right moment; a long note held while others move becomes a second voice;
//!   * each hand gets its own staff (treble and bass), the key signature is estimated and notes are spelled to it.
//!
//! Time is counted in twelfths of a beat (the lowest common multiple of sixteenths and triplet eighths), starting at
//! beat 0 = the first bar line of the easy sheet, so bar numbers match it.
use crate::midi::SheetNote;
use std::fmt::Write;

const U: i64 = 12; // units per beat
/// Notes this close together are one chord.
const CHORD_S: f64 = 0.07;
/// A note that ends this long (in beats) before the next one starts is followed by a rest; closer than that, it is held.
const REST_GAP: f64 = 0.45;
/// A note that is still sounding this long (in beats) after the next one starts is sustained: it gets its own voice.
const SUSTAIN: f64 = 0.6;

pub struct Grid {
    pub t_downbeat: f64,
    pub beat: f64,
    pub beats: usize,
}

pub struct Score {
    pub xml: String,
    /// Real time of every beat from beat 0 on, for lining the score up with playback.
    pub beat_times: Vec<f64>,
    pub fifths: i32,
    pub minor: bool,
    pub measures: usize,
    pub bpm: f64,
    pub beats: usize,
    pub t_downbeat: f64,
    /// the key the notes suggest, whatever key was asked for
    pub detected: (i32, bool),
}

// ------------------------------------------------------------------ beats

struct Beats {
    t: Vec<f64>,
}

impl Beats {
    /// Position in beats (0 = first bar line) of a moment in the music.
    fn pos(&self, t: f64) -> f64 {
        let n = self.t.len();
        if t <= self.t[0] {
            return (t - self.t[0]) / (self.t[1] - self.t[0]);
        }
        let i = self.t.partition_point(|&x| x <= t) - 1;
        if i >= n - 1 {
            return i as f64 + (t - self.t[i]) / (self.t[n - 1] - self.t[n - 2]);
        }
        i as f64 + (t - self.t[i]) / (self.t[i + 1] - self.t[i])
    }
}

/// Beat times that follow the playing: each beat is placed by where the onsets near it fall, gently.
fn follow_beats(onsets: &[f64], g: &Grid, n: usize) -> Vec<f64> {
    let mut t = vec![g.t_downbeat];
    let (mut pos, mut per) = (g.t_downbeat, g.beat);
    for _ in 0..n {
        let exp = pos + per;
        let win = 0.22 * per;
        let a = onsets.partition_point(|&o| o < exp - win);
        let b = onsets.partition_point(|&o| o <= exp + win);
        let mut next = exp;
        if b > a {
            let err = onsets[a..b].iter().map(|o| o - exp).sum::<f64>() / (b - a) as f64;
            next += 0.5 * err;
            per = (per + 0.1 * err).clamp(g.beat * 0.85, g.beat * 1.15);
        }
        pos = next;
        t.push(pos);
    }
    t
}

fn grid_cost(beats: &Beats, onsets: &[f64]) -> f64 {
    onsets
        .iter()
        .map(|&o| {
            let b = beats.pos(o);
            let f = b - b.floor();
            let ds = (f * 4.0 - (f * 4.0).round()).abs() / 4.0;
            let dt = (f * 3.0 - (f * 3.0).round()).abs() / 3.0;
            ds.min(dt).powi(2)
        })
        .sum()
}

// ------------------------------------------------------------------ chords and snapping

#[derive(Clone, Debug)]
struct Cluster {
    t: f64,
    notes: Vec<(i32, f64)>, // midi, end time
    /// 0 = struck together, 1 = rolled upward, -1 = rolled downward, 2 = rolled without a clear direction
    arp: i8,
    /// the notes themselves (midi, start, end)
    members: Vec<(i32, f64, f64)>,
}

/// Notes of a rolled chord: this far apart at most, over this long in all, and every one still held when the next arrives.
const ROLL_GAP: f64 = 0.10;
const ROLL_SPAN: f64 = 0.40;
/// ... and the notes of a rolled chord stay down at least this long after the last one is struck.
const ROLL_HOLD: f64 = 0.30;
/// A chord spread over more than this is drawn with the rolled-chord sign.
const ROLL_MARK: f64 = 0.035;

fn cluster(mut notes: Vec<(i32, f64, f64)>) -> Vec<Cluster> {
    notes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.cmp(&b.0)));
    // (midi, start, end) of the notes of each chord
    let mut out: Vec<Vec<(i32, f64, f64)>> = Vec::new();
    for (m, s, e) in notes {
        let join = out.last().is_some_and(|g| {
            let (gap, span) = (s - g[g.len() - 1].1, s - g[0].1);
            let together = gap <= CHORD_S && span <= 2.0 * CHORD_S;
            // a rolled chord: the notes come a little further apart, but each is still down when the next arrives
            // (a quick run played legato also overlaps itself, so the notes must be held a good while after the last one lands)
            let rolled = gap <= ROLL_GAP && span <= ROLL_SPAN && e >= s + ROLL_HOLD && g.iter().all(|x| x.2 >= s + ROLL_HOLD);
            together || rolled
        });
        if join {
            let g = out.last_mut().unwrap();
            if let Some(x) = g.iter_mut().find(|x| x.0 == m) {
                x.2 = x.2.max(e);
            } else {
                g.push((m, s, e));
            }
        } else {
            out.push(vec![(m, s, e)]);
        }
    }
    out.into_iter()
        .map(|g| {
            let mut starts: Vec<f64> = g.iter().map(|x| x.1).collect();
            starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let spread = starts[starts.len() - 1] - starts[0];
            let arp = if spread < ROLL_MARK || g.len() < 2 {
                0
            } else {
                let (mut up, mut down) = (0, 0);
                for w in g.windows(2) {
                    if w[1].1 - w[0].1 > 0.01 {
                        if w[1].0 > w[0].0 { up += 1 } else if w[1].0 < w[0].0 { down += 1 }
                    }
                }
                let total = up + down;
                if total > 0 && up as f64 >= 0.8 * total as f64 { 1 } else if total > 0 && down as f64 >= 0.8 * total as f64 { -1 } else { 2 }
            };
            // a chord struck together is where most of it lands; a rolled one is where its roll begins
            Cluster { t: if arp != 0 { starts[0] } else { starts[starts.len() / 2] }, notes: g.iter().map(|x| (x.0, x.2)).collect(), arp, members: g.clone() }
        })
        .collect()
}

/// Which beats are triplets (three equal notes in the beat) rather than straight sixteenths.
fn triplet_beats(cl: &[Cluster], beats: &Beats, n: usize) -> Vec<bool> {
    let mut by_beat: Vec<Vec<f64>> = vec![Vec::new(); n + 2];
    for c in cl {
        let b = beats.pos(c.t);
        let k = (b + 0.125).floor();
        if k >= 0.0 && (k as usize) < by_beat.len() {
            by_beat[k as usize].push((b - k) * U as f64);
        }
    }
    let near = |f: f64, step: f64| (f - (f / step).round() * step).abs();
    let mut trip = vec![false; n + 2];
    for (k, fs) in by_beat.iter().enumerate() {
        if fs.is_empty() {
            continue;
        }
        let err_s: f64 = fs.iter().map(|&f| near(f, 3.0).powi(2)).sum();
        let err_t: f64 = fs.iter().map(|&f| near(f, 4.0).powi(2)).sum();
        let has4 = fs.iter().any(|&f| (f - 4.0).abs() < 1.3);
        let has8 = fs.iter().any(|&f| (f - 8.0).abs() < 1.3);
        trip[k] = (has4 && has8 && err_t <= err_s + 0.5) || ((has4 || has8) && fs.len() >= 3 && err_t < 0.25 * err_s);
    }
    // one clear triplet beat next to a lone ambiguous one: the music is almost surely in triplets there too
    let strong = trip.clone();
    for k in 0..trip.len() {
        if strong[k] || by_beat[k].is_empty() {
            continue;
        }
        let nb = (k > 0 && strong[k - 1]) || strong.get(k + 1).copied().unwrap_or(false);
        let fs = &by_beat[k];
        let err_s: f64 = fs.iter().map(|&f| near(f, 3.0).powi(2)).sum();
        let err_t: f64 = fs.iter().map(|&f| near(f, 4.0).powi(2)).sum();
        let off = fs.iter().any(|&f| (f - 4.0).abs() < 1.3 || (f - 8.0).abs() < 1.3);
        if nb && off && err_t <= err_s {
            trip[k] = true;
        }
    }
    trip
}

fn is_trip(trip: &[bool], beat: i64) -> bool {
    beat >= 0 && trip.get(beat as usize).copied().unwrap_or(false)
}

/// Snap a position (in units) to the nearest place on the grid of its beat.
fn snap_u(u: f64, trip: &[bool]) -> i64 {
    let k = (u / U as f64).floor().max(0.0) as i64;
    let step = if is_trip(trip, k) { 4.0 } else { 3.0 };
    let r = u - (k * U) as f64;
    k * U + ((r / step).round() * step) as i64
}

/// Snap the start of a note: a hair early belongs to the beat it is leading into.
fn snap_start(b: f64, trip: &[bool]) -> i64 {
    let k = (b + 0.125).floor().max(0.0) as i64;
    let step = if is_trip(trip, k) { 4.0 } else { 3.0 };
    let f = ((b - k as f64) * U as f64).max(0.0);
    k * U + ((f / step).round() * step) as i64
}

// ------------------------------------------------------------------ one staff: chords, lengths, voices

#[derive(Clone, Debug)]
struct Ev {
    pos: i64,
    dur: i64,
    pitches: Vec<i32>, // ascending; empty = rest
    staccato: bool,
    arp: i8,
}

fn staff_voices(notes: Vec<(i32, f64, f64)>, beats: &Beats, trip: &[bool], hold: f64) -> (Vec<Vec<Ev>>, f64) {
    let cl = cluster(notes);
    if cl.is_empty() {
        return (vec![], 0.0);
    }
    // where each chord starts; two chords never share a place
    let mut pos: Vec<i64> = cl.iter().map(|c| snap_start(beats.pos(c.t), trip)).collect();
    for i in 1..pos.len() {
        if pos[i] <= pos[i - 1] {
            pos[i] = pos[i - 1] + if is_trip(trip, pos[i - 1] / U) { 4 } else { 3 };
        }
    }
    // how far (in beats) the chords had to be moved to reach the grid: a measure of how well the grid fits
    let err: f64 = cl.iter().zip(&pos).map(|(c, &p)| (beats.pos(c.t) - p as f64 / U as f64).powi(2)).sum();
    let mut v1: Vec<Ev> = Vec::new();
    let mut v2: Vec<Ev> = Vec::new();
    for (i, c) in cl.iter().enumerate() {
        let next = pos.get(i + 1).copied();
        let mut ordinary: Vec<(i32, f64)> = Vec::new();
        let mut sustained: Vec<(i32, f64)> = Vec::new();
        for &(m, e) in &c.notes {
            let ue = beats.pos(e) * U as f64;
            match next {
                Some(n) if ue - n as f64 > SUSTAIN * U as f64 && ue - pos[i] as f64 >= hold * U as f64 => sustained.push((m, ue)),
                _ => ordinary.push((m, ue)),
            }
        }
        let min_dur = if is_trip(trip, pos[i] / U) { 4 } else { 3 };
        if !ordinary.is_empty() {
            let emax = ordinary.iter().map(|x| x.1).fold(0.0, f64::max);
            let len = (emax - pos[i] as f64).max(0.0);
            // keys are never let go at the exact moment: a note lasts until the next one unless there is a real gap, and a
            // short detached note is written as the beat it stands for (with a staccato dot) rather than as a tiny note and a rest
            let (end, stac) = match next {
                Some(n) => {
                    let ioi = (n - pos[i]) as f64;
                    let detached = ioi >= 6.0 && len < 0.4 * ioi;
                    if len >= 0.75 * ioi || ioi - len < 3.0 {
                        (n, false)
                    } else if ioi <= U as f64 {
                        (n, detached)
                    } else if ioi - len < REST_GAP * U as f64 {
                        (n, false)
                    } else {
                        let rounded = ((len / 6.0).ceil() * 6.0).max(6.0);
                        let e = snap_u(pos[i] as f64 + rounded, trip).clamp(pos[i] + min_dur, n);
                        (e, len < 0.6 * rounded)
                    }
                }
                None => {
                    // the last chord: the whole beat it stands on
                    let rounded = ((len / U as f64).ceil() * U as f64).max(U as f64);
                    (pos[i] + rounded as i64, len < 0.4 * rounded)
                }
            };
            let mut p: Vec<i32> = ordinary.iter().map(|x| x.0).collect();
            p.sort_unstable();
            v1.push(Ev { pos: pos[i], dur: end - pos[i], pitches: p, staccato: stac, arp: c.arp });
        }
        if !sustained.is_empty() {
            let mut ends: Vec<f64> = sustained.iter().map(|x| x.1).collect();
            ends.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let end = snap_u(ends[ends.len() / 2], trip).max(pos[i] + min_dur);
            let mut p: Vec<i32> = sustained.iter().map(|x| x.0).collect();
            p.sort_unstable();
            v2.push(Ev { pos: pos[i], dur: end - pos[i], pitches: p, staccato: false, arp: 0 });
        }
    }
    for v in [&mut v1, &mut v2] {
        for i in 1..v.len() {
            let start = v[i].pos;
            let prev = &mut v[i - 1];
            if prev.pos + prev.dur > start {
                prev.dur = start - prev.pos;
            }
        }
    }
    // a staff whose second voice has nothing worth a voice (a single stray note) stays one voice
    let mut out = vec![v1];
    if !v2.is_empty() {
        let mean = |v: &Vec<Ev>| v.iter().flat_map(|e| e.pitches.iter().copied()).sum::<i32>() as f64 / v.iter().map(|e| e.pitches.len()).sum::<usize>().max(1) as f64;
        // the lower line is voice 2 (stems down), whichever holds
        if mean(&v2) <= mean(&out[0]) || out[0].is_empty() {
            out.push(v2);
        } else {
            out.insert(0, v2);
        }
    }
    out.retain(|v| !v.is_empty());
    (out, err)
}

// ------------------------------------------------------------------ writing durations as note values

/// Pieces a span may be written as. `s` is the start within the measure, in units.
fn allowed(s: i64, l: i64, trip: &[bool], bar: i64) -> bool {
    if s + l > bar {
        return false;
    }
    let valid = |p: i64| p % U == 0 || if is_trip(trip, p / U) { p % 4 == 0 } else { p % 3 == 0 };
    if !valid(s) || !valid(s + l) {
        return false;
    }
    // a piece that only part-fills a beat must agree with how that beat is divided
    let triplet_value = l == 4 || l == 8;
    for b in (s / U)..=((s + l - 1) / U) {
        let covers = s <= b * U && s + l >= (b + 1) * U;
        if !covers && is_trip(trip, b) != triplet_value {
            return false;
        }
    }
    if triplet_value && s / U != (s + l - 1) / U {
        return false;
    }
    match l {
        48 => s == 0,
        36 => s == 0,
        24 => s % 24 == 0,
        18 | 12 | 6 => s % 6 == 0,
        9 => s % U == 0 || s % U == 3,
        8 => s % U == 0 || s % U == 4,
        4 | 3 => true,
        _ => false,
    }
}

fn decompose(s: i64, len: i64, trip: &[bool], bar: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let (mut s, mut rem) = (s, len);
    while rem > 0 {
        let l = [48, 36, 24, 18, 12, 9, 8, 6, 4, 3].into_iter().find(|&l| l <= rem && allowed(s, l, trip, bar)).unwrap_or_else(|| rem.min(3));
        out.push((s, l));
        s += l;
        rem -= l;
    }
    out
}

#[derive(Clone, Debug)]
struct Item {
    pos: i64, // within the measure
    dur: i64,
    pitches: Vec<i32>,
    tie_start: bool,
    tie_stop: bool,
    tuplet: bool, // a triplet piece (4 or 8 units inside a triplet beat)
    whole_rest: bool,
    staccato: bool,
    arp: i8,
}

/// One voice laid out in measures of notatable pieces, rests filling every gap.
fn measures_of(events: &[Ev], total: i64, bar: i64, trip: &[bool], hold_units: i64) -> Vec<Vec<Item>> {
    let n = (total / bar) as usize;
    let mut filled: Vec<Ev> = Vec::new();
    let mut cur = 0;
    for e in events {
        // a note held for less than the shortest hold that is worth showing is not tied on: it ends with its first piece
        let mut e = e.clone();
        if !e.pitches.is_empty() && e.dur < hold_units {
            let m = (e.pos / bar) as usize;
            let stop = (e.pos + e.dur).min((m as i64 + 1) * bar);
            let local: &[bool] = trip.get((m as i64 * bar / U) as usize..).unwrap_or(&[]);
            let pieces = decompose(e.pos - m as i64 * bar, stop - e.pos, local, bar);
            if pieces.len() > 1 || stop < e.pos + e.dur {
                e.dur = pieces[0].1;
            }
        }
        let e = &e;
        if e.pos > cur {
            filled.push(Ev { pos: cur, dur: e.pos - cur, pitches: vec![], staccato: false, arp: 0 });
        }
        filled.push(e.clone());
        cur = e.pos + e.dur;
    }
    if cur < total {
        filled.push(Ev { pos: cur, dur: total - cur, pitches: vec![], staccato: false, arp: 0 });
    }
    let mut bars: Vec<Vec<Item>> = vec![Vec::new(); n];
    for e in &filled {
        let mut pieces: Vec<(usize, i64, i64)> = Vec::new(); // measure, start within it, length
        let (mut s, end) = (e.pos, e.pos + e.dur);
        while s < end {
            let m = (s / bar) as usize;
            let stop = end.min((m as i64 + 1) * bar);
            let local: &[bool] = trip.get((m as i64 * bar / U) as usize..).unwrap_or(&[]);
            if e.pitches.is_empty() && s % bar == 0 && stop - s == bar {
                bars[m].push(Item { pos: 0, dur: bar, pitches: vec![], tie_start: false, tie_stop: false, tuplet: false, whole_rest: true, staccato: false, arp: 0 });
            } else {
                for (ps, pl) in decompose(s - m as i64 * bar, stop - s, local, bar) {
                    pieces.push((m, ps, pl));
                }
            }
            s = stop;
        }
        let count = pieces.len();
        for (k, (m, ps, pl)) in pieces.into_iter().enumerate() {
            let local: &[bool] = trip.get((m as i64 * bar / U) as usize..).unwrap_or(&[]);
            let tuplet = (pl == 4 || pl == 8) && is_trip(local, ps / U);
            let note = !e.pitches.is_empty();
            bars[m].push(Item { pos: ps, dur: pl, pitches: e.pitches.clone(), tie_start: note && k + 1 < count, tie_stop: note && k > 0, tuplet, whole_rest: false, staccato: e.staccato && k == 0, arp: if k == 0 { e.arp } else { 0 } });
        }
    }
    bars
}

// ------------------------------------------------------------------ key and spelling

/// Krumhansl-Schmuckler: the key whose profile best matches how long each pitch class sounds.
fn estimate_key(notes: &[SheetNote]) -> (i32, bool) {
    const MAJ: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
    const MIN: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
    let mut h = [0.0f64; 12];
    for n in notes {
        h[n.midi.rem_euclid(12) as usize] += (n.end - n.start).clamp(0.05, 2.0);
    }
    let corr = |prof: &[f64; 12], tonic: usize| {
        let x: Vec<f64> = (0..12).map(|i| h[(i + tonic) % 12]).collect();
        let (mx, mp) = (x.iter().sum::<f64>() / 12.0, prof.iter().sum::<f64>() / 12.0);
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for i in 0..12 {
            sxy += (x[i] - mx) * (prof[i] - mp);
            sxx += (x[i] - mx).powi(2);
            syy += (prof[i] - mp).powi(2);
        }
        sxy / (sxx * syy).sqrt().max(1e-9)
    };
    let mut best = (f64::MIN, 0usize, false);
    for t in 0..12 {
        for (minor, prof) in [(false, &MAJ), (true, &MIN)] {
            let c = corr(prof, t);
            if c > best.0 {
                best = (c, t, minor);
            }
        }
    }
    // sharps or flats counted from the tonic of the major key (a minor key shares the signature of the major a third up)
    const FIFTHS: [i32; 12] = [0, -5, 2, -3, 4, -1, 6, 1, -4, 3, -2, 5];
    let major_tonic = if best.2 { (best.1 + 3) % 12 } else { best.1 };
    (FIFTHS[major_tonic], best.2)
}

const SHARP_ORDER: [char; 7] = ['F', 'C', 'G', 'D', 'A', 'E', 'B'];
const NATURAL_PC: [(char, i32); 7] = [('C', 0), ('D', 2), ('E', 4), ('F', 5), ('G', 7), ('A', 9), ('B', 11)];

/// What each letter is in the key signature: +1 sharp, -1 flat.
fn key_alter(fifths: i32, step: char) -> i32 {
    let i = SHARP_ORDER.iter().position(|&c| c == step).unwrap() as i32;
    if fifths > 0 && i < fifths {
        1
    } else if fifths < 0 && 6 - i < -fifths {
        -1
    } else {
        0
    }
}

/// (step, alter, octave) for a MIDI note, spelled to suit the key.
fn spell(midi: i32, fifths: i32) -> (char, i32, i32) {
    let pc = midi.rem_euclid(12);
    let mut pick: Option<(char, i32)> = None;
    // a letter of the key
    for &(step, npc) in &NATURAL_PC {
        let a = key_alter(fifths, step);
        if (npc + a).rem_euclid(12) == pc {
            pick = Some((step, a));
        }
    }
    if pick.is_none() {
        // outside the key: a natural, else a sharp or flat according to the side of the key
        if let Some(&(step, _)) = NATURAL_PC.iter().find(|&&(_, npc)| npc == pc) {
            pick = Some((step, 0));
        } else {
            let flat_side = fifths < 0 || (fifths == 0 && matches!(pc, 3 | 8 | 10));
            pick = NATURAL_PC.iter().find_map(|&(step, npc)| {
                if flat_side && (npc - 1).rem_euclid(12) == pc { Some((step, -1)) } else if !flat_side && (npc + 1).rem_euclid(12) == pc { Some((step, 1)) } else { None }
            });
        }
    }
    let (step, alter) = pick.unwrap();
    (step, alter, (midi - alter).div_euclid(12) - 1)
}

// ------------------------------------------------------------------ MusicXML

fn type_of(dur: i64) -> (&'static str, bool) {
    match dur {
        48 => ("whole", false),
        36 => ("half", true),
        24 => ("half", false),
        18 => ("quarter", true),
        12 | 8 => ("quarter", false),
        9 => ("eighth", true),
        6 | 4 => ("eighth", false),
        _ => ("16th", false),
    }
}

/// Which clef each staff wears in each measure: the upper staff goes to bass clef when it dives low, the lower to treble when it climbs,
/// so notes stay on the staff instead of piling up ledger lines. 'G' or 'F'.
fn clefs_for(staves: &[Vec<Vec<Vec<Item>>>], n_meas: usize) -> Vec<Vec<char>> {
    staves
        .iter()
        .enumerate()
        .map(|(si, voices)| {
            let mut cur = if si == 0 { 'G' } else { 'F' };
            (0..n_meas)
                .map(|m| {
                    let mut ps: Vec<i32> = voices.iter().flat_map(|v| v[m].iter().flat_map(|it| it.pitches.iter().copied())).collect();
                    if !ps.is_empty() {
                        ps.sort_unstable();
                        let med = ps[ps.len() / 2];
                        cur = match (si == 0, cur) {
                            (true, 'G') if med < 52 => 'F',
                            (true, 'F') if med > 59 => 'G',
                            (false, 'F') if med > 67 => 'G',
                            (false, 'G') if med < 60 => 'F',
                            (_, c) => c,
                        };
                    }
                    cur
                })
                .collect()
        })
        .collect()
}

fn clef_xml(number: usize, c: char) -> String {
    if c == 'G' {
        format!("<clef number=\"{number}\"><sign>G</sign><line>2</line></clef>")
    } else {
        format!("<clef number=\"{number}\"><sign>F</sign><line>4</line></clef>")
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// For the notes of one measure of one voice: beam marks (number, kind) for each item.
fn beams(items: &[Item]) -> Vec<Vec<(u8, &'static str)>> {
    let mut out: Vec<Vec<(u8, &'static str)>> = vec![vec![]; items.len()];
    let beamable = |it: &Item| !it.pitches.is_empty() && matches!(it.dur, 3 | 4 | 6 | 9);
    let mut i = 0;
    while i < items.len() {
        if !beamable(&items[i]) {
            i += 1;
            continue;
        }
        let beat = items[i].pos / U;
        let mut j = i;
        while j < items.len() && beamable(&items[j]) && items[j].pos / U == beat {
            j += 1;
        }
        if j - i >= 2 {
            for k in i..j {
                out[k].push((1, if k == i { "begin" } else if k == j - 1 { "end" } else { "continue" }));
            }
            // second level for sixteenths
            let sixteenth = |it: &Item| it.dur == 3;
            let mut k = i;
            while k < j {
                if !sixteenth(&items[k]) {
                    k += 1;
                    continue;
                }
                let mut m = k;
                while m < j && sixteenth(&items[m]) {
                    m += 1;
                }
                if m - k >= 2 {
                    for q in k..m {
                        out[q].push((2, if q == k { "begin" } else if q == m - 1 { "end" } else { "continue" }));
                    }
                } else {
                    out[k].push((2, if k == i { "forward hook" } else { "backward hook" }));
                }
                k = m;
            }
        }
        i = j;
    }
    out
}

struct Writer<'a> {
    xml: String,
    fifths: i32,
    bar: i64,
    staves: &'a [Vec<Vec<Vec<Item>>>], // staff, voice, measure
    clefs: &'a [Vec<char>],            // staff, measure
}

impl Writer<'_> {
    fn measure(&mut self, m: usize, bpm: f64, beats: usize, last: bool) {
        let mut x = String::new();
        let _ = write!(x, "<measure number=\"{}\">", m + 1);
        if m == 0 {
            let _ = write!(x, "<attributes><divisions>{U}</divisions><key><fifths>{}</fifths></key><time><beats>{beats}</beats><beat-type>4</beat-type></time><staves>2</staves>{}{}</attributes>", self.fifths, clef_xml(1, self.clefs[0][0]), clef_xml(2, self.clefs[1][0]));
            let _ = write!(x, "<direction placement=\"above\"><direction-type><metronome><beat-unit>quarter</beat-unit><per-minute>{}</per-minute></metronome></direction-type><sound tempo=\"{}\"/></direction>", bpm.round() as i64, bpm.round() as i64);
        }
        if m > 0 {
            let changed: String = (0..2).filter(|&i| self.clefs[i][m] != self.clefs[i][m - 1]).map(|i| clef_xml(i + 1, self.clefs[i][m])).collect();
            if !changed.is_empty() {
                let _ = write!(x, "<attributes>{changed}</attributes>");
            }
        }
        let mut first = true;
        for (si, voices) in self.staves.iter().enumerate() {
            // accidentals already shown in this measure on this staff, by (step, octave)
            let mut seen: std::collections::HashMap<(char, i32), i32> = Default::default();
            let two = voices.iter().filter(|v| v.get(m).is_some_and(|b| b.iter().any(|it| !it.pitches.is_empty()))).count() > 1;
            for (vi, voice) in voices.iter().enumerate() {
                let items = &voice[m];
                if vi > 0 && !items.iter().any(|it| !it.pitches.is_empty()) {
                    continue; // a second voice with nothing in this measure
                }
                if !first {
                    let _ = write!(x, "<backup><duration>{}</duration></backup>", self.bar);
                }
                first = false;
                let vnum = si * 2 + vi + 1;
                let bm = beams(items);
                let mut tup_open = false;
                for (k, it) in items.iter().enumerate() {
                    let (ty, dot) = type_of(it.dur);
                    let next_same_beat_tuplet = items.get(k + 1).is_some_and(|n| n.tuplet && n.pos / U == it.pos / U);
                    let tuplet_start = it.tuplet && !tup_open;
                    let tuplet_stop = it.tuplet && !next_same_beat_tuplet;
                    tup_open = it.tuplet && !tuplet_stop;
                    let stem = if two { if vi == 0 { "<stem>up</stem>" } else { "<stem>down</stem>" } } else { "" };
                    if it.pitches.is_empty() {
                        let _ = write!(x, "<note><rest{}/><duration>{}</duration><voice>{vnum}</voice>", if it.whole_rest { " measure=\"yes\"" } else { "" }, it.dur);
                        if !it.whole_rest {
                            let _ = write!(x, "<type>{ty}</type>{}", if dot { "<dot/>" } else { "" });
                        }
                        if it.tuplet {
                            x.push_str("<time-modification><actual-notes>3</actual-notes><normal-notes>2</normal-notes></time-modification>");
                        }
                        let _ = write!(x, "<staff>{}</staff>", si + 1);
                        if it.tuplet && (tuplet_start || tuplet_stop) {
                            let _ = write!(x, "<notations><tuplet type=\"{}\"/></notations>", if tuplet_start { "start" } else { "stop" });
                        }
                        x.push_str("</note>");
                        continue;
                    }
                    for (pi, &midi) in it.pitches.iter().enumerate() {
                        let (step, alter, oct) = spell(midi, self.fifths);
                        let current = seen.get(&(step, oct)).copied().unwrap_or_else(|| key_alter(self.fifths, step));
                        let show = if alter != current && !it.tie_stop { Some(alter) } else { None };
                        seen.insert((step, oct), alter);
                        x.push_str("<note>");
                        if pi > 0 {
                            x.push_str("<chord/>");
                        }
                        let _ = write!(x, "<pitch><step>{step}</step>{}<octave>{oct}</octave></pitch><duration>{}</duration>", if alter != 0 { format!("<alter>{alter}</alter>") } else { String::new() }, it.dur);
                        if it.tie_stop {
                            x.push_str("<tie type=\"stop\"/>");
                        }
                        if it.tie_start {
                            x.push_str("<tie type=\"start\"/>");
                        }
                        let _ = write!(x, "<voice>{vnum}</voice><type>{ty}</type>{}", if dot { "<dot/>" } else { "" });
                        if let Some(a) = show {
                            let _ = write!(x, "<accidental>{}</accidental>", match a { 1 => "sharp", -1 => "flat", _ => "natural" });
                        }
                        if it.tuplet {
                            x.push_str("<time-modification><actual-notes>3</actual-notes><normal-notes>2</normal-notes></time-modification>");
                        }
                        let _ = write!(x, "{stem}<staff>{}</staff>", si + 1);
                        if pi == 0 {
                            for (n, kind) in &bm[k] {
                                let _ = write!(x, "<beam number=\"{n}\">{kind}</beam>");
                            }
                        }
                        let tied = it.tie_stop || it.tie_start;
                        let stac = pi == 0 && it.staccato;
                        let arp = it.arp != 0;
                        if tied || stac || arp || (pi == 0 && it.tuplet && (tuplet_start || tuplet_stop)) {
                            x.push_str("<notations>");
                            if arp {
                                x.push_str(match it.arp { 1 => "<arpeggiate direction=\"up\"/>", -1 => "<arpeggiate direction=\"down\"/>", _ => "<arpeggiate/>" });
                            }
                            if stac {
                                x.push_str("<articulations><staccato/></articulations>");
                            }
                            if it.tie_stop {
                                x.push_str("<tied type=\"stop\"/>");
                            }
                            if it.tie_start {
                                x.push_str("<tied type=\"start\"/>");
                            }
                            if pi == 0 && it.tuplet && (tuplet_start || tuplet_stop) {
                                let _ = write!(x, "<tuplet type=\"{}\"/>", if tuplet_start { "start" } else { "stop" });
                            }
                            x.push_str("</notations>");
                        }
                        x.push_str("</note>");
                    }
                }
            }
        }
        if last {
            x.push_str("<barline location=\"right\"><bar-style>light-heavy</bar-style></barline>");
        }
        x.push_str("</measure>");
        self.xml.push_str(&x);
    }
}

// ------------------------------------------------------------------ everything

/// What is written at the top of the first page. `subtitle` None means the key ("Key of D major"); Some("") means none.
pub struct Meta {
    pub title: String,
    pub subtitle: Option<String>,
    pub composer: String,
    pub arranger: String,
    /// Shortest hold, in beats, that is still written as a hold (tied over a bar line, or kept under moving notes). Shorter
    /// notes are cut where the next one comes. 0 keeps every hold.
    pub hold: f64,
    /// The key the reader says it is in (sharps or flats, minor or not), instead of the one the notes suggest
    pub key: Option<(i32, bool)>,
}

pub fn key_name(fifths: i32, minor: bool) -> String {
    const MAJOR: [&str; 15] = ["C♭", "G♭", "D♭", "A♭", "E♭", "B♭", "F", "C", "G", "D", "A", "E", "B", "F♯", "C♯"];
    const MINOR: [&str; 15] = ["A♭", "E♭", "B♭", "F", "C", "G", "D", "A", "E", "B", "F♯", "C♯", "G♯", "D♯", "A♯"];
    let i = (fifths.clamp(-7, 7) + 7) as usize;
    format!("{} {}", if minor { MINOR[i] } else { MAJOR[i] }, if minor { "minor" } else { "major" })
}

pub fn build(notes: &[SheetNote], g: &Grid, title: &str) -> Score {
    build_with(notes, g, &Meta { title: title.to_string(), subtitle: None, composer: String::new(), arranger: String::new(), hold: 0.0, key: None })
}

/// Everything between the notes and the page: beats, staves of measures of note values, and how well the grid fits.
struct Analysis {
    beats: Beats,
    staves: Vec<Vec<Vec<Vec<Item>>>>, // staff, voice, measure
    n_meas: usize,
    bar: i64,
    err: f64,
    /// how many bars begin with a note or chord (a good meter puts a new sound on most bar lines)
    bars_started: usize,
    /// how many bars lay out their notes like the bar before (or the one before that): a right meter makes the pattern repeat
    bars_repeating: usize,
}

fn analyse(notes: &[SheetNote], g: &Grid, hold: f64) -> Analysis {
    let last_end = notes.iter().map(|n| n.end).fold(0.0, f64::max);
    let n_beats = (((last_end - g.t_downbeat) / g.beat).ceil().max(1.0) as usize) + 2;

    // every chord of every hand, for tracking the beat
    let all: Vec<Cluster> = cluster(notes.iter().map(|n| (n.midi, n.start, n.end)).collect());
    let onsets: Vec<f64> = all.iter().map(|c| c.t).collect();
    let plain = Beats { t: (0..=n_beats).map(|k| g.t_downbeat + k as f64 * g.beat).collect() };
    let followed = Beats { t: follow_beats(&onsets, g, n_beats) };
    let beats = if grid_cost(&followed, &onsets) < 0.85 * grid_cost(&plain, &onsets) { followed } else { plain };

    // which staff each note is on: the two hands, else by pitch
    let mut hands: Vec<i32> = notes.iter().map(|n| n.hand).collect();
    hands.sort_unstable();
    hands.dedup();
    let med = |h: i32| {
        let mut m: Vec<i32> = notes.iter().filter(|n| n.hand == h).map(|n| n.midi).collect();
        m.sort_unstable();
        m[m.len() / 2]
    };
    let staff_notes: Vec<Vec<(i32, f64, f64)>> = if hands.len() == 2 {
        let (hi, lo) = if med(hands[0]) >= med(hands[1]) { (hands[0], hands[1]) } else { (hands[1], hands[0]) };
        vec![notes.iter().filter(|n| n.hand == hi).map(|n| (n.midi, n.start, n.end)).collect(), notes.iter().filter(|n| n.hand == lo).map(|n| (n.midi, n.start, n.end)).collect()]
    } else {
        // one hand (or more than two): split by pitch, keeping chords together
        let mut up = Vec::new();
        let mut down = Vec::new();
        for c in &all {
            let mut ms: Vec<i32> = c.notes.iter().map(|x| x.0).collect();
            ms.sort_unstable();
            let upper = ms[ms.len() / 2] >= 60;
            for &m in &c.members {
                (if upper { &mut up } else { &mut down }).push(m);
            }
        }
        vec![up, down]
    };

    let bar = g.beats as i64 * U;
    let mut staff_events: Vec<Vec<Vec<Ev>>> = Vec::new();
    let mut err = 0.0;
    let mut staff_trip: Vec<Vec<bool>> = Vec::new();
    for sn in staff_notes {
        let cl = cluster(sn.clone());
        let trip = triplet_beats(&cl, &beats, n_beats);
        let (voices, e) = staff_voices(sn, &beats, &trip, hold);
        err += e;
        staff_events.push(voices);
        staff_trip.push(trip);
    }
    let end_u = staff_events.iter().flatten().flatten().map(|e| e.pos + e.dur).max().unwrap_or(bar);
    let n_meas = ((end_u + bar - 1) / bar).max(1) as usize;
    let total = n_meas as i64 * bar;
    let hold_u = (hold.max(0.0) * U as f64).round() as i64;

    let staves: Vec<Vec<Vec<Vec<Item>>>> = staff_events
        .iter()
        .zip(&staff_trip)
        .map(|(voices, trip)| {
            let mut v: Vec<Vec<Vec<Item>>> = voices.iter().map(|ev| measures_of(ev, total, bar, trip, hold_u)).collect();
            if v.is_empty() {
                v.push(measures_of(&[], total, bar, trip, hold_u));
            }
            v
        })
        .collect();
    let mut started = vec![false; n_meas];
    for e in staff_events.iter().flatten().flatten() {
        if e.pos % bar == 0 && !e.pitches.is_empty() {
            if let Some(b) = started.get_mut((e.pos / bar) as usize) {
                *b = true;
            }
        }
    }
    let bars_started = started.iter().filter(|&&b| b).count();
    let mut sigs: Vec<Vec<i64>> = vec![Vec::new(); n_meas];
    for e in staff_events.iter().flatten().flatten().filter(|e| !e.pitches.is_empty()) {
        if let Some(v) = sigs.get_mut((e.pos / bar) as usize) {
            v.push(e.pos % bar);
        }
    }
    for v in &mut sigs {
        v.sort_unstable();
        v.dedup();
    }
    let bars_repeating = (1..n_meas).filter(|&m| !sigs[m].is_empty() && (sigs[m] == sigs[m - 1] || (m >= 2 && sigs[m] == sigs[m - 2]))).count();
    Analysis { beats, staves, n_meas, bar, err, bars_started, bars_repeating }
}

/// How busy a reading of the music is: notes, ties, rests, short values and triplets all make a page harder to read, and a
/// grid that does not fit the playing costs most. The simplest page that stays faithful to the playing wins.
fn busyness(an: &Analysis, bpm: f64) -> f64 {
    busyness_parts(an, bpm).0
}

/// The total, and what it is made of: notes, ties, rests, short values, triplets, second voices, grid misfit.
fn busyness_parts(an: &Analysis, bpm: f64) -> (f64, [f64; 9]) {
    let mut parts = [0.0f64; 9];
    let mut cost = 0.0;
    for staff in &an.staves {
        for (vi, voice) in staff.iter().enumerate() {
            for bar in voice {
                for it in bar {
                    if it.pitches.is_empty() {
                        if !it.whole_rest {
                            parts[2] += 0.6;
                        }
                        continue;
                    }
                    parts[0] += 1.0;
                    if it.tie_stop {
                        parts[1] += 2.5;
                    }
                    if it.dur <= 6 {
                        parts[3] += 0.3;
                    }
                    if it.dur == 3 {
                        parts[3] += 0.7;
                    }
                    if it.tuplet {
                        parts[4] += 1.2;
                    }
                    if vi > 0 {
                        parts[5] += 1.5;
                    }
                }
            }
        }
    }
    parts[6] = 40.0 * an.err;
    // a bar with nothing starting on its first beat means the bar lines are in the wrong place (or the bars are too short)
    parts[7] = 3.0 * (an.n_meas - an.bars_started) as f64;
    parts[8] = 2.0 * (an.n_meas - an.bars_repeating) as f64;
    cost += parts.iter().sum::<f64>();
    // a preference for ordinary tempos: the same music reads as 80 beats a minute or 160, and 80 is the more natural page
    (cost * (1.0 + 0.2 * (bpm / 90.0).log2().abs()), parts)
}

/// One reading of the music that was tried: how busy its page is, how far the playing sat from its grid, and the grid itself.
pub struct Candidate {
    pub cost: f64,
    pub parts: [f64; 9],
    pub err: f64,
    pub items: usize,
    pub grid: Grid,
}

/// Every one of these beat lengths, in each of these meters and with a bar line at each plausible place, with its cost.
pub fn candidates(notes: &[SheetNote], periods: &[f64], meters: &[usize], hold: f64) -> Vec<Candidate> {
    let cl = cluster(notes.iter().map(|n| (n.midi, n.start, n.end)).collect());
    let onsets: Vec<f64> = cl.iter().map(|c| c.t).collect();
    let first = onsets.first().copied().unwrap_or(0.0);
    let mut out = Vec::new();
    for &p in periods {
        let bpm = 60.0 / p;
        if !(30.0..=240.0).contains(&bpm) {
            continue;
        }
        // where the beats fall, from the onsets themselves
        let (mut re, mut im) = (0.0, 0.0);
        for &t in &onsets {
            let a = -2.0 * std::f64::consts::PI * t / p;
            re += a.cos();
            im += a.sin();
        }
        let phase = (im.atan2(re) / (2.0 * std::f64::consts::PI) * p).rem_euclid(p);
        for &b in meters {
            let bar = p * b as f64;
            let mut starts: Vec<f64> = (0..b).map(|k| phase + k as f64 * p).collect();
            starts.push(first); // music that begins on a bar line
            for st in starts {
                // the last bar line at or before the first note (a hair after is fine: a note a little early belongs to the next beat)
                let td = st + ((first + 0.15 * p - st) / bar).floor() * bar;
                let g = Grid { t_downbeat: td, beat: p, beats: b };
                let an = analyse(notes, &g, hold);
                let items = an.staves.iter().flatten().flatten().flatten().filter(|it| !it.pitches.is_empty()).count();
                let (cost, parts) = busyness_parts(&an, bpm);
                out.push(Candidate { cost, parts, err: an.err, items, grid: g });
            }
        }
    }
    out
}

/// A piece whose chords sit on a perfectly regular pulse (a machine-made MIDI) is read from that pulse: the bar length and the
/// meter are then a matter of which reading gives the simplest page. A human-played piece has no such pulse, and the tempo from the
/// easy sheet is kept, with the meter `beats` (or the base's own).
pub fn auto_grid(notes: &[SheetNote], base: &Grid, beats: Option<usize>, hold: f64) -> Grid {
    let keep = Grid { t_downbeat: base.t_downbeat, beat: base.beat, beats: beats.unwrap_or(base.beats).max(1) };
    let (pulse, pull) = strongest_pulse(notes);
    if pull < 0.5 {
        return keep;
    }
    let cl = cluster(notes.iter().map(|n| (n.midi, n.start, n.end)).collect());
    let onsets: Vec<f64> = cl.iter().map(|c| c.t).collect();
    let pull_at = |p: f64| {
        let (mut re, mut im) = (0.0, 0.0);
        for &t in &onsets {
            let a = -2.0 * std::f64::consts::PI * t / p;
            re += a.cos();
            im += a.sin();
        }
        re.hypot(im)
    };
    // tune the pulse to the notes (it is exact to a thousandth already), then take it as it is, doubled and quadrupled
    let (mut p, mut best) = (pulse, pull_at(pulse));
    for i in -20..=20 {
        let q = pulse * (1.0 + i as f64 * 0.00005);
        let r = pull_at(q);
        if r > best {
            best = r;
            p = q;
        }
    }
    let meters: Vec<usize> = beats.map_or(vec![4, 3, 2], |b| vec![b.max(1)]);
    candidates(notes, &[p, p * 2.0, p * 4.0], &meters, hold)
        .into_iter()
        .min_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap())
        .map(|c| c.grid)
        .unwrap_or(keep)
}

/// How strongly the chords line up on a beat of this length: 1 when every chord lands exactly on a beat (or on the same place in
/// every beat), near 0 when they fall anywhere. A steady piece scores high at its real beat.
pub fn beat_pull(notes: &[SheetNote], period: f64) -> f64 {
    let cl = cluster(notes.iter().map(|n| (n.midi, n.start, n.end)).collect());
    let (mut re, mut im) = (0.0, 0.0);
    for c in &cl {
        let a = -2.0 * std::f64::consts::PI * c.t / period;
        re += a.cos();
        im += a.sin();
    }
    re.hypot(im) / cl.len().max(1) as f64
}

/// The beat length (0.25 s to 1.6 s) that the chords line up on best, and how well (see `beat_pull`).
pub fn strongest_pulse(notes: &[SheetNote]) -> (f64, f64) {
    let cl = cluster(notes.iter().map(|n| (n.midi, n.start, n.end)).collect());
    let times: Vec<f64> = cl.iter().map(|c| c.t).collect();
    let (mut best, mut p) = ((0.0, 0.5), 0.25);
    while p < 1.6 {
        let (mut re, mut im) = (0.0, 0.0);
        for &t in &times {
            let a = -2.0 * std::f64::consts::PI * t / p;
            re += a.cos();
            im += a.sin();
        }
        let r = re.hypot(im) / times.len().max(1) as f64;
        if r > best.0 {
            best = (r, p);
        }
        p *= 1.0005;
    }
    (best.1, best.0)
}

/// How one particular reading (a grid) fares.
pub fn evaluate(notes: &[SheetNote], g: &Grid, hold: f64) -> Candidate {
    let an = analyse(notes, g, hold);
    let items = an.staves.iter().flatten().flatten().flatten().filter(|it| !it.pitches.is_empty()).count();
    Candidate { cost: busyness(&an, 60.0 / g.beat), parts: busyness_parts(&an, 60.0 / g.beat).1, err: an.err, items, grid: Grid { t_downbeat: g.t_downbeat, beat: g.beat, beats: g.beats } }
}

pub fn build_with(notes: &[SheetNote], g: &Grid, meta: &Meta) -> Score {
    let title = meta.title.as_str();
    let Analysis { beats, staves, n_meas, bar, .. } = analyse(notes, g, meta.hold);
    let clefs = clefs_for(&staves, n_meas);
    let detected = estimate_key(notes);
    let (fifths, minor) = meta.key.map_or(detected, |(f, m)| (f.clamp(-7, 7), m));
    let bpm = 60.0 / g.beat;
    let mut w = Writer { xml: String::new(), fifths, bar, staves: &staves, clefs: &clefs };
    let subtitle = match &meta.subtitle {
        Some(s) => s.trim().to_string(),
        None => format!("Key of {}", key_name(fifths, minor)),
    };
    let arranger = meta.arranger.trim();
    let arranger = if arranger.is_empty() || arranger.to_lowercase().starts_with("arr") { arranger.to_string() } else { format!("arr.: {arranger}") };
    let composer = meta.composer.trim();
    let mut ident = String::new();
    if !composer.is_empty() {
        let _ = write!(ident, "<creator type=\"composer\">{}</creator>", esc(composer));
    }
    if !arranger.is_empty() {
        let _ = write!(ident, "<creator type=\"arranger\">{}</creator>", esc(&arranger));
    }
    // the title page: the title and what the key is in the middle, composer and arranger on the right
    let mut credits = String::new();
    let mut credit = |kind: &str, text: &str, x: i32, y: i32, justify: &str, size: &str| {
        if !text.trim().is_empty() {
            let _ = write!(credits, "<credit page=\"1\"><credit-type>{kind}</credit-type><credit-words default-x=\"{x}\" default-y=\"{y}\" justify=\"{justify}\" valign=\"top\" font-size=\"{size}\">{}</credit-words></credit>", esc(text));
        }
    };
    credit("title", title, 616, 1500, "center", "xx-large");
    credit("subtitle", &subtitle, 616, 1440, "center", "large");
    credit("composer", composer, 1150, 1440, "right", "medium");
    credit("arranger", &arranger, 1150, 1415, "right", "medium");
    let _ = write!(w.xml, "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 3.1 Partwise//EN\" \"http://www.musicxml.org/dtds/partwise.dtd\"><score-partwise version=\"3.1\"><work><work-title>{}</work-title></work><identification>{ident}</identification><defaults><page-layout><page-height>1600</page-height><page-width>1233</page-width></page-layout></defaults>{credits}<part-list><score-part id=\"P1\"><part-name print-object=\"no\">Piano</part-name></score-part></part-list><part id=\"P1\">", esc(title));
    for m in 0..n_meas {
        w.measure(m, bpm, g.beats, m + 1 == n_meas);
    }
    w.xml.push_str("</part></score-partwise>");
    Score { xml: w.xml, beat_times: beats.t.clone(), fifths, minor, measures: n_meas, bpm, beats: g.beats, t_downbeat: g.t_downbeat, detected }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(midi: i32, start: f64, end: f64, hand: i32) -> SheetNote {
        SheetNote { midi, start, end, hand }
    }
    fn grid(beats: usize) -> Grid {
        Grid { t_downbeat: 0.0, beat: 0.5, beats }
    }

    #[test]
    fn every_piece_is_a_note_value_and_pieces_add_up() {
        let bar = 48;
        for trip_beat in 0..4usize {
            let trip: Vec<bool> = (0..4).map(|b| b == trip_beat).collect();
            for s in 0..bar {
                for l in 1..=(bar - s) {
                    let ok = |p: i64| p % 12 == 0 || if trip[(p / 12) as usize % 4] { p % 4 == 0 } else { p % 3 == 0 };
                    if !ok(s) || !ok(s + l) {
                        continue;
                    }
                    let pieces = decompose(s, l, &trip, bar);
                    assert_eq!(pieces.iter().map(|p| p.1).sum::<i64>(), l, "s={s} l={l} trip={trip_beat}");
                    let mut at = s;
                    for (ps, pl) in pieces {
                        assert_eq!(ps, at);
                        assert!([48, 36, 24, 18, 12, 9, 8, 6, 4, 3].contains(&pl), "odd piece {pl} for s={s} l={l} trip={trip_beat}");
                        at += pl;
                    }
                }
            }
        }
    }

    #[test]
    fn a_scale_in_quarter_notes_is_written_as_quarters_in_c_major() {
        let notes: Vec<SheetNote> = [60, 62, 64, 65, 67, 69, 71, 72].iter().enumerate().map(|(i, &m)| n(m, i as f64 * 0.5 + 0.01, i as f64 * 0.5 + 0.45, 0)).collect();
        let s = build(&notes, &grid(4), "Scale");
        assert_eq!((s.fifths, s.measures), (0, 2));
        assert_eq!(s.xml.matches("<type>quarter</type>").count() >= 8, true, "{}", s.xml);
        assert!(s.xml.contains("<fifths>0</fifths>") && s.xml.contains("<beats>4</beats>"));
        assert!(!s.xml.contains("<alter>"));
    }

    #[test]
    fn a_rolled_chord_is_one_chord() {
        // three notes within 50 ms of each other, then a note on the next beat
        let notes = vec![n(60, 0.000, 0.5, 0), n(64, 0.035, 0.5, 0), n(67, 0.06, 0.5, 0), n(72, 0.5, 1.0, 0)];
        let s = build(&notes, &grid(4), "Chord");
        assert_eq!(s.xml.matches("<chord/>").count(), 2, "{}", s.xml);
    }

    #[test]
    fn three_notes_to_the_beat_are_a_triplet() {
        let mut notes = Vec::new();
        for beat in 0..4 {
            for k in 0..3 {
                let t = beat as f64 * 0.5 + k as f64 * 0.5 / 3.0;
                notes.push(n(60 + k * 2, t, t + 0.15, 0));
            }
        }
        let s = build(&notes, &grid(4), "Triplets");
        assert!(s.xml.contains("<time-modification>"), "{}", s.xml);
        assert!(s.xml.contains("<tuplet type=\"start\"/>") && s.xml.contains("<tuplet type=\"stop\"/>"));
        assert_eq!(s.xml.matches("<type>eighth</type>").count() >= 12, true);
    }

    #[test]
    fn short_detached_chords_are_quarters_with_dots_not_tiny_notes_and_rests() {
        // oom-pah-pah: a bass note, then two short chords, in 3/4, twice
        let mut notes = Vec::new();
        for bar in 0..2 {
            let t = bar as f64 * 1.5;
            notes.push(n(38, t, t + 0.4, 1));
            for k in 1..3 {
                let s = t + k as f64 * 0.5;
                notes.extend([n(54, s, s + 0.12, 1), n(57, s, s + 0.12, 1)]);
            }
        }
        let s = build(&notes, &grid(3), "Waltz");
        assert!(!s.xml.contains("<type>16th</type>") && !s.xml.contains("<type>eighth</type>"), "{}", s.xml);
        assert!(s.xml.contains("<staccato/>"));
        assert_eq!(s.xml.matches("<rest").count(), s.xml.matches("<rest measure=\"yes\"/>").count(), "only the empty staff has rests");
    }

    #[test]
    fn short_holds_lose_their_tails_and_long_ones_keep_them() {
        // 4/4 at 120 bpm: a chord on the last beat of the bar held for 2 beats, over the bar line, then a closing note
        let notes = vec![n(48, 0.0, 0.5, 0), n(60, 1.5, 2.5, 0), n(64, 1.5, 2.5, 0), n(72, 4.0, 4.5, 0)];
        let with = |hold: f64| {
            let g = grid(4);
            build_with(&notes, &g, &Meta { title: "H".into(), subtitle: None, composer: String::new(), arranger: String::new(), hold, key: None })
        };
        let all = with(0.0);
        assert!(all.xml.contains("<tie type=\"start\"/>"), "{}", all.xml);
        let some = with(3.0);
        assert!(!some.xml.contains("<tie type=\"start\"/>"), "{}", some.xml);
        // either way each measure still adds up
        for m in some.xml.split("<measure number=").skip(1) {
            let total: i64 = m.split("<note>").skip(1).filter(|x| !x.contains("<chord/>")).map(|x| x.split("<duration>").nth(1).unwrap().split('<').next().unwrap().parse::<i64>().unwrap()).sum();
            assert_eq!(total % 48, 0, "{m}");
        }
    }

    #[test]
    fn a_rolled_chord_held_together_is_one_chord_with_the_roll_sign() {
        // six notes rolled upward (0.067 s apart) and all held for three seconds, twice
        let mut notes = Vec::new();
        for c in 0..2 {
            let t = c as f64 * 3.0;
            for (k, m) in [53, 60, 65, 67, 69, 76].iter().enumerate() {
                notes.push(n(*m, t + k as f64 * 0.067, t + 2.9, 0));
            }
        }
        let s = build(&notes, &Grid { t_downbeat: 0.0, beat: 0.75, beats: 4 }, "Roll");
        assert_eq!(s.xml.matches("<chord/>").count(), 10, "two chords of six notes");
        assert!(s.xml.contains("<arpeggiate direction=\"up\"/>"));
        assert!(!s.xml.contains("<type>16th</type>") && !s.xml.contains("<type>eighth</type>"), "{}", s.xml);
    }

    #[test]
    fn a_quick_legato_run_is_not_turned_into_a_chord() {
        // six notes 0.09 s apart that each last 0.2 s: they overlap a little, but it is a run
        let notes: Vec<SheetNote> = (0..6).map(|k| n(60 + k * 2, k as f64 * 0.09, k as f64 * 0.09 + 0.2, 0)).collect();
        let s = build(&notes, &grid(4), "Run");
        assert_eq!(s.xml.matches("<chord/>").count(), 0, "{}", s.xml);
        assert!(!s.xml.contains("<arpeggiate"));
    }

    #[test]
    fn a_staff_that_climbs_or_dives_changes_clef() {
        // the left hand (lower staff) plays up in the treble for four bars, the right hand dives into the bass
        let mut notes = Vec::new();
        for bar in 0..6 {
            for beat in 0..4 {
                let t = (bar * 4 + beat) as f64 * 0.5;
                notes.push(n(76 + beat, t, t + 0.4, 1)); // upper hand (higher median)
                notes.push(n(if bar < 3 { 72 } else { 40 } + beat, t, t + 0.4, 0)); // hand 0 starts high, then drops low
            }
        }
        let s = build(&notes, &grid(4), "Clefs");
        // the staff that starts in bass clef moves to treble, and the one that dives moves to bass
        assert!(s.xml.contains("<clef number=\"2\"><sign>G</sign>"), "{}", &s.xml[..1500]);
        assert!(s.xml.contains("<clef number=\"1\"><sign>F</sign>") || s.xml.contains("<clef number=\"2\"><sign>F</sign>"));
    }

    #[test]
    fn a_regular_pulse_gets_bars_that_hold_its_chords() {
        // a chord on every third second that lasts 2.9 s, a melody of notes every 0.375 s: a machine-exact piece
        let mut notes = Vec::new();
        for c in 0..24 {
            let t = c as f64 * 3.0;
            for m in [48, 55, 64] {
                notes.push(n(m, t, t + 2.9, 1));
            }
            for k in 0..8 {
                notes.push(n(72 + (k * 3) % 7, t + k as f64 * 0.375, t + k as f64 * 0.375 + 0.3, 0));
            }
        }
        // the easy sheet's first guess is a beat that does not fit
        let base = Grid { t_downbeat: 0.0, beat: 0.4926, beats: 3 };
        let g = auto_grid(&notes, &base, None, 2.0);
        let bar = g.beat * g.beats as f64;
        assert!((bar - 3.0).abs() < 0.03 || (bar - 1.5).abs() < 0.03, "bar of {bar} s, {}/4 at {} bpm", g.beats, 60.0 / g.beat);
        assert!((g.t_downbeat.rem_euclid(1.5)).min(1.5 - g.t_downbeat.rem_euclid(1.5)) < 0.1, "bars start on the chords: {}", g.t_downbeat);
        // a human-played piece (no regular pulse) keeps the first guess, with the meter asked for
        let mut at = 0.0;
        let sloppy: Vec<SheetNote> = (0..80).map(|k| { at += 0.2 + ((k * 7919) % 100) as f64 / 100.0 * 0.7; n(60 + (k % 5), at, at + 0.15, 0) }).collect();
        let g2 = auto_grid(&sloppy, &base, Some(3), 2.0);
        assert_eq!((g2.beats, g2.beat), (3, 0.4926));
    }

    #[test]
    fn a_key_chosen_by_the_reader_respells_the_notes_and_the_signature() {
        // a phrase in G sharp / A flat: the notes suggest sharps, the reader says A flat major
        let notes: Vec<SheetNote> = [68, 71, 73, 68, 72, 75, 68, 73, 71, 68].iter().enumerate().map(|(i, &m)| n(m, i as f64 * 0.5, i as f64 * 0.5 + 0.4, 0)).collect();
        let meta = |key| Meta { title: "K".into(), subtitle: None, composer: String::new(), arranger: String::new(), hold: 0.0, key };
        let flats = build_with(&notes, &grid(4), &meta(Some((-4, false))));
        assert_eq!(flats.fifths, -4);
        assert!(flats.xml.contains("<fifths>-4</fifths>") && flats.xml.contains("<step>A</step><alter>-1</alter>"), "{}", &flats.xml[..900]);
        assert!(!flats.xml.contains("<step>G</step><alter>1</alter>"));
        assert!(flats.xml.contains("Key of A♭ major"));
        let auto = build_with(&notes, &grid(4), &meta(None));
        assert_eq!(auto.detected, (auto.fifths, auto.minor), "with no choice the detected key is used");
        assert_eq!(flats.detected, auto.detected, "the detected key is reported either way");
    }

    #[test]
    fn sixteenths_stay_sixteenths() {
        let notes: Vec<SheetNote> = (0..16).map(|k| n(60 + (k % 4), k as f64 * 0.125, k as f64 * 0.125 + 0.1, 0)).collect();
        let s = build(&notes, &grid(4), "Sixteenths");
        assert!(!s.xml.contains("<time-modification>"));
        assert!(s.xml.matches("<type>16th</type>").count() >= 16, "{}", s.xml);
        assert!(s.xml.contains("<beam number=\"2\">"));
    }

    #[test]
    fn each_hand_gets_a_staff_and_a_held_note_becomes_a_second_voice() {
        // right hand: a whole-bar note with a tune moving above it; left hand: bass notes
        let mut notes = vec![n(55, 0.0, 2.0, 0), n(72, 0.0, 0.5, 0), n(74, 0.5, 1.0, 0), n(76, 1.0, 1.5, 0), n(77, 1.5, 2.0, 0)];
        notes.push(n(48, 0.0, 1.0, 1));
        notes.push(n(43, 1.0, 2.0, 1));
        let s = build(&notes, &grid(4), "Two hands");
        assert!(s.xml.contains("<staff>2</staff>"));
        assert!(s.xml.contains("<voice>2</voice>"), "the held note is its own voice: {}", s.xml);
        assert!(s.xml.contains("<backup>"));
    }

    #[test]
    fn the_key_and_the_spelling_follow_the_music() {
        // G major: F sharp, no accidental printed; a stray F natural gets a natural sign
        let notes = vec![n(67, 0.0, 0.5, 0), n(69, 0.5, 1.0, 0), n(71, 1.0, 1.5, 0), n(72, 1.5, 2.0, 0), n(74, 2.0, 2.5, 0), n(78, 2.5, 3.0, 0), n(79, 3.0, 3.5, 0), n(74, 3.5, 4.0, 0), n(79, 4.0, 4.5, 0), n(78, 4.5, 5.0, 0), n(74, 5.0, 5.5, 0), n(67, 5.5, 6.0, 0), n(77, 6.0, 6.5, 0), n(67, 6.5, 8.0, 0)];
        let s = build(&notes, &grid(4), "G");
        assert_eq!(s.fifths, 1, "{}", s.xml);
        assert!(s.xml.contains("<step>F</step><alter>1</alter>"));
        assert!(s.xml.contains("<accidental>natural</accidental>"), "{}", s.xml);
        assert!(!s.xml.contains("<accidental>sharp</accidental>"));
        assert_eq!(spell(61, 0), ('C', 1, 4));
        assert_eq!(spell(70, -2), ('B', -1, 4));
        assert_eq!(spell(59, 0), ('B', 0, 3));
    }

    #[test]
    fn a_slowing_tempo_still_lands_on_the_beats() {
        // the player slows down 12% over 20 beats; notes on every beat
        let mut t = 0.0;
        let mut per = 0.5;
        let mut notes = Vec::new();
        for k in 0..24 {
            notes.push(n(60 + (k % 5), t, t + per * 0.9, 0));
            t += per;
            per *= 1.006;
        }
        let s = build(&notes, &grid(4), "Rit");
        // every note is a quarter and none was pushed into a rest or a short value
        assert!(s.xml.matches("<type>quarter</type>").count() >= 24, "{}", s.xml);
        assert!(!s.xml.contains("<type>eighth</type>") && !s.xml.contains("<type>16th</type>"), "{}", s.xml);
    }

    #[test]
    fn every_measure_adds_up() {
        let mut notes = Vec::new();
        for k in 0..40 {
            notes.push(n(60 + (k * 7) % 12, k as f64 * 0.37, k as f64 * 0.37 + 0.2 + (k % 3) as f64 * 0.2, k % 2));
        }
        let s = build(&notes, &grid(3), "Odd");
        // every <measure> has voice 1 (and staff 2 voice 3) filling 36 units: sum of <duration> per voice
        for m in s.xml.split("<measure number=").skip(1) {
            let mut per_voice: std::collections::HashMap<String, i64> = Default::default();
            for note in m.split("<note>").skip(1) {
                if note.contains("<chord/>") {
                    continue;
                }
                let d: i64 = note.split("<duration>").nth(1).unwrap().split('<').next().unwrap().parse().unwrap();
                let v = note.split("<voice>").nth(1).unwrap().split('<').next().unwrap().to_string();
                *per_voice.entry(v).or_default() += d;
            }
            for (v, total) in per_voice {
                assert_eq!(total, 36, "voice {v} in {}", &m[..60]);
            }
        }
    }
}
