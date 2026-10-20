//! Beginner-friendly "sheet" pages from a MIDI file: staff lines for pitch height, big dots with the note name
//! inside, bar lines and beat ticks. No rhythm notation: the player follows the video/audio.
use crate::canvas::{self, downscale_lanczos, Canvas, Font};
use crate::midi::SheetNote as Note;
use std::collections::HashMap;
use std::sync::Mutex;

pub const A4_SIZE: (u32, u32) = (2480, 3508); // A4 portrait at 300 dpi
const REF_WIDTH: f64 = 1680.0; // note size is defined for a 1680 px wide page; wider pages scale with it
const LETTERS: [char; 7] = ['C', 'D', 'E', 'F', 'G', 'A', 'B'];

fn step_of_pc(pc: i32) -> (usize, i32) {
    [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (3, 0), (3, 1), (4, 0), (4, 1), (5, 0), (5, 1), (6, 0)][pc as usize]
}
fn flat_step(pc: i32) -> Option<(usize, i32)> {
    match pc {
        1 => Some((1, -1)),
        3 => Some((2, -1)),
        6 => Some((4, -1)),
        8 => Some((5, -1)),
        10 => Some((6, -1)),
        _ => None,
    }
}

type Cluster = (f64, Vec<usize>);
type ByMeasure = HashMap<usize, Vec<Cluster>>;

// ------------------------------------------------------------------ grouping
const LOOSE_GAP: f64 = 0.09;
const LOOSE_SPAN: f64 = 0.12;
const LOOSE_MAX_NOTES: usize = 6;

fn loosen(clusters: Vec<Cluster>) -> Vec<Cluster> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < clusters.len() {
        let mut j = i;
        while j + 1 < clusters.len() && clusters[j + 1].0 - clusters[j].0 <= LOOSE_GAP {
            j += 1;
        }
        let chain = &clusters[i..=j];
        let notes: Vec<usize> = chain.iter().flat_map(|(_, g)| g.iter().copied()).collect();
        if chain.len() > 1 && chain[chain.len() - 1].0 - chain[0].0 <= LOOSE_SPAN && notes.len() <= LOOSE_MAX_NOTES {
            out.push((chain[0].0, notes));
        } else {
            out.extend(chain.iter().cloned());
        }
        i = j + 1;
    }
    out
}

/// Group notes whose onsets are within 25 ms: detection jitter turns chords into 1-frame spreads.
pub fn cluster(notes: &[Note], loose: bool) -> Vec<Cluster> {
    let mut out: Vec<(f64, Vec<usize>)> = Vec::new();
    for (i, n) in notes.iter().enumerate() {
        match out.last_mut() {
            Some(l) if n.start - l.0 <= 0.025 => l.1.push(i),
            _ => out.push((n.start, vec![i])),
        }
    }
    let strict: Vec<Cluster> = out
        .into_iter()
        .map(|(_, g)| (g.iter().map(|&i| notes[i].start).sum::<f64>() / g.len() as f64, g))
        .collect();
    if loose { loosen(strict) } else { strict }
}

fn percentile(v: &mut Vec<f64>, q: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pos = q / 100.0 * (v.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    v[lo] + (v[hi] - v[lo]) * (pos - lo as f64)
}

/// (beat period s, downbeat time). Beat = strongest periodicity of onsets (mild preference for 60-160 bpm); the downbeat
/// is the beat phase where the low notes fall most often (oom-pah-pah style).
pub fn estimate_beats(notes: &[Note], bpm: usize, period: Option<f64>) -> (f64, f64) {
    let cl = cluster(notes, false);
    let t: Vec<f64> = cl.iter().map(|c| c.0).collect();
    if t.len() < 8 {
        return (0.5, t.first().copied().unwrap_or(0.0));
    }
    let z = |p: f64| -> (f64, f64) {
        let (mut re, mut im) = (0.0, 0.0);
        for &x in &t {
            let a = -2.0 * std::f64::consts::PI * x / p;
            re += a.cos();
            im += a.sin();
        }
        ((re * re + im * im).sqrt(), im.atan2(re))
    };
    let (mut best_s, mut p, mut phi) = (0.0, 0.5, 0.0);
    let cands: Vec<f64> = match period {
        Some(p) => vec![p],
        None => (0..625).map(|i| 0.25 + i as f64 * 0.002).collect(),
    };
    for c in cands {
        let (m, ang) = z(c);
        let prior = (-0.5 * ((c / 0.55).log2() / 0.7).powi(2)).exp();
        let score = m / t.len() as f64 * prior;
        if score > best_s {
            best_s = score;
            p = c;
            phi = ang;
        }
    }
    if period.is_none() {
        // fine-tune on the real onsets: local search maximising the resultant length
        let (start, stop) = (p - 0.004, p + 0.004); // numpy.arange: ceil((stop - start) / step) values
        for i in 0..(((stop - start) / 0.0002).ceil() as usize) {
            let q = start + i as f64 * 0.0002;
            let (mq, aq) = z(q);
            if mq > z(p).0 {
                p = q;
                phi = aq;
            }
        }
    }
    let t0 = ((phi / (2.0 * std::f64::consts::PI)) * p).rem_euclid(p);
    let mut lows: Vec<f64> = cl.iter().map(|(_, g)| g.iter().map(|&i| notes[i].midi).min().unwrap() as f64).collect();
    let low = percentile(&mut lows, 35.0);
    let mut weights = vec![0usize; bpm];
    for (tt, g) in &cl {
        let k = (tt - t0) / p;
        let ki = k.round_ties_even();
        if (k - ki).abs() < 0.18 && g.iter().map(|&i| notes[i].midi).min().unwrap() as f64 <= low {
            weights[(ki as i64).rem_euclid(bpm as i64) as usize] += 1;
        }
    }
    let down = weights.iter().enumerate().fold((0, 0), |b, (i, &w)| if w > b.1 { (i, w) } else { b }).0;
    let mut td = t0 + down as f64 * p;
    let mlen = p * bpm as f64;
    td += ((t[0] - td + 1e-6) / mlen).floor() * mlen;
    (p, td)
}

// ------------------------------------------------------------------ parameters
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rotate {
    None,
    Cw,
    Ccw,
}

#[derive(Clone)]
pub struct Plan {
    pub pages: Vec<Vec<Vec<usize>>>,
    pub scale: f64,
}

#[derive(Clone)]
struct Hold {
    x: f64,
    y: f64,
    xe: f64,
    clipped: bool,
    again: bool,
    up: bool,
    letter: usize,
    acc: i32,
    octave: i32,
    filled: bool,
}

#[derive(Clone)]
pub struct Params {
    pub beats_per_measure: usize,
    pub beat_period: Option<f64>,
    pub t_downbeat: Option<f64>,
    pub measures_per_page: usize,
    pub systems_per_page: i64,
    pub size: (u32, u32),
    pub dot: f64,
    pub flat: bool,
    pub show_octave: bool,
    pub show_beats: bool,
    pub title: String,
    pub t_start: Option<f64>,
    pub window: f64,
    pub min_scale: f64,
    pub used_scale: f64,
    pub rotate: Rotate,
    pub print_mode: bool,
    pub measures_per_row: usize,
    pub first_page_rows: usize,
    pub dot_mult: f64,
    pub page_index: usize,
    /// filled by render(): (start, end, x, y) for every drawn note, in final image pixels
    pub dots: Vec<(f64, f64, f64, f64)>,
    pub dot_r: f64,
    /// filled by render(): (t0, t1, x0, y0, x1, y1) per drawn measure, in final image pixels
    pub boxes: Vec<(f64, f64, f64, f64, f64, f64)>,
    pub group_chords: bool,
    pub line_space: f64,
    pub print_sp: f64,
    pub show_holds: bool,
    pub show_link: bool,
    pub plan: Option<Plan>,
    pub print_pad: f64,
    pending: Vec<Hold>,
    restrike: HashMap<usize, f64>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            beats_per_measure: 3, beat_period: None, t_downbeat: None, measures_per_page: 4, systems_per_page: 1, size: (1680, 1264),
            dot: 56.0, flat: false, show_octave: false, show_beats: true, title: String::new(), t_start: None, window: 16.0,
            min_scale: 0.03, used_scale: 1.0, rotate: Rotate::None, print_mode: false, measures_per_row: 0, first_page_rows: 0,
            dot_mult: 1.0, page_index: 0, dots: vec![], dot_r: 0.0, boxes: vec![], group_chords: false, line_space: 1.0,
            print_sp: 0.72, show_holds: true, show_link: true, plan: None, print_pad: 0.8, pending: vec![], restrike: HashMap::new(),
        }
    }
}

/// Everything about the song the layout needs, computed once.
struct Ctx<'a> {
    notes: &'a [Note],
    lanes: Vec<i32>,
    med: HashMap<i32, f64>,
}

fn lanes_of(notes: &[Note]) -> Ctx<'_> {
    let mut hands: Vec<i32> = notes.iter().map(|n| n.hand).collect();
    hands.sort_unstable();
    hands.dedup();
    let med: HashMap<i32, f64> = hands
        .iter()
        .map(|&h| {
            let mut v: Vec<f64> = notes.iter().filter(|n| n.hand == h).map(|n| n.midi as f64).collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = v.len();
            (h, if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
        })
        .collect();
    let mut order = hands;
    order.sort_by(|a, b| med[b].partial_cmp(&med[a]).unwrap()); // upper lane first = higher hand
    Ctx { notes, lanes: order, med }
}

/// Fill in t_start / beat_period / t_downbeat (all editable) so measure 0 is where the page starts.
pub fn resolve(notes: &[Note], p: &mut Params) {
    let t_start = *p.t_start.get_or_insert(notes[0].start);
    if p.beat_period.is_none() || p.t_downbeat.is_none() {
        let mut win: Vec<Note> = notes.iter().filter(|n| t_start - 0.05 <= n.start && n.start <= t_start + p.window).cloned().collect();
        if win.is_empty() {
            win = notes.to_vec();
        }
        let (bp, td) = estimate_beats(&win, p.beats_per_measure, p.beat_period);
        let period = *p.beat_period.get_or_insert(bp);
        if p.t_downbeat.is_none() {
            let mlen = period * p.beats_per_measure as f64;
            p.t_downbeat = Some(td + ((t_start - td + 0.15 * period) / mlen).floor() * mlen);
        }
    }
    if p.print_mode && p.plan.is_none() {
        plan_print(notes, p);
    }
}

// ------------------------------------------------------------------ names and staff positions
fn name_of(midi: i32, flat: bool) -> (usize, i32, i32) {
    let pc = midi.rem_euclid(12);
    let octave = midi.div_euclid(12) - 1;
    match (flat, flat_step(pc)) {
        (true, Some((li, acc))) => (li, acc, octave),
        _ => {
            let (li, acc) = step_of_pc(pc);
            (li, acc, octave)
        }
    }
}

fn step_of(li: usize, octave: i32) -> i32 {
    octave * 7 + li as i32
}

struct Page {
    d: f64,
    margin: f64,
    base: f64,
    sp: f64,
    gap: f64,
    s: f64,
}

impl Page {
    fn new(p: &Params, scale: f64, gap: f64) -> Page {
        let d = p.dot * p.dot_mult * scale;
        let margin = (if p.print_mode { 90.0 } else { 40.0 }) * p.dot_mult * scale;
        let base = d * if p.print_mode { p.print_sp } else { 0.92 };
        Page { d, margin, base, sp: base * p.line_space, gap, s: scale }
    }
}

fn lane_geometry(n_lanes: usize, med_pitch: f64) -> (i32, bool) {
    let treble = if n_lanes > 1 { med_pitch >= 60.0 } else { med_pitch >= 57.0 };
    (if treble { step_of(2, 4) } else { step_of(4, 2) }, treble)
}

/// Horizontal offsets (in dot sizes) for the notes of one hand's chord, given their staff steps (any order).
/// A note nothing touches stays on the chord's axis; touching notes go into side-by-side columns.
fn chord_columns(steps: &[i32], pg: &Page) -> Vec<f64> {
    let need = 0.9 * pg.d;
    let n = steps.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| steps[i]);
    let touching: Vec<bool> = (0..n).map(|i| (0..n).any(|j| j != i && ((steps[j] - steps[i]).abs() as f64) * pg.sp / 2.0 < need)).collect();
    let mut offs = vec![0.0; n];
    const COLS: [f64; 6] = [-0.51, 0.51, -1.53, 1.53, -2.55, 2.55];
    let mut last: [Option<i32>; 6] = [None; 6];
    for &i in &order {
        if !touching[i] {
            continue;
        }
        let mut placed = false;
        for (ci, &c) in COLS.iter().enumerate() {
            if last[ci].map_or(true, |l| ((steps[i] - l).abs() as f64) * pg.sp / 2.0 >= need) {
                offs[i] = c;
                last[ci] = Some(steps[i]);
                placed = true;
                break;
            }
        }
        if !placed {
            offs[i] = COLS[5];
        }
    }
    offs
}

fn hand_steps(cx: &Ctx, g: &[usize], hand: i32, flat: bool) -> Vec<i32> {
    g.iter().filter(|&&i| cx.notes[i].hand == hand).map(|&i| {
        let nm = name_of(cx.notes[i].midi, flat);
        step_of(nm.0, nm.2)
    }).collect()
}

/// Half the width, in dot sizes, that a chord (all hands) takes up, counted from its axis.
fn chord_halfwidth(cx: &Ctx, g: &[usize], p: &Params, pg: &Page) -> f64 {
    let mut hw: f64 = 0.5;
    let mut hands: Vec<i32> = g.iter().map(|&i| cx.notes[i].hand).collect();
    hands.sort_unstable();
    hands.dedup();
    for h in hands {
        let st = hand_steps(cx, g, h, p.flat);
        if st.len() > 1 {
            let m = chord_columns(&st, pg).iter().fold(0.0f64, |a, o| a.max(o.abs()));
            hw = hw.max(m + 0.5);
        }
    }
    hw
}

// ------------------------------------------------------------------ title
fn title_lines(p: &Params, pg: &Page, maxw: f64) -> (f64, Vec<String>) {
    let mut last = (0.0, vec![]);
    for size in [pg.d * 1.05, pg.d * 0.9, pg.d * 0.75, pg.d * 0.6] {
        let size = size.trunc();
        let (mut lines, mut cur) = (Vec::<String>::new(), String::new());
        for w in p.title.split_whitespace() {
            let t = if cur.is_empty() { w.to_string() } else { format!("{cur} {w}") };
            if canvas::text_len(&t, Font::Bold, size) <= maxw || cur.is_empty() {
                cur = t;
            } else {
                lines.push(std::mem::replace(&mut cur, w.to_string()));
            }
        }
        lines.push(cur);
        let fits = lines.len() <= 2 && lines.iter().all(|l| canvas::text_len(l, Font::Bold, size) <= maxw);
        last = (size, lines);
        if fits {
            return last;
        }
    }
    last.1.truncate(2);
    last
}

fn title_height(p: &Params, pg: &Page, maxw: f64) -> f64 {
    let (size, lines) = title_lines(p, pg, maxw);
    lines.len() as f64 * size * 1.25 + pg.d * 1.4
}

// ------------------------------------------------------------------ layout
#[derive(Clone)]
struct Sys {
    ms: Vec<usize>,
    widths: Vec<f64>,
    final_: Vec<(f64, Vec<f64>)>,
    geos: Vec<(i32, bool)>,
    tops: Vec<f64>,
    y0: f64,
    y1: f64,
}

const MIN_ROWS: usize = 5; // the note size is chosen so that pages hold at least this many rows
const MAX_ROWS: usize = 9; // a page never holds more rows than this, however small the notes
const GAPS: [f64; 3] = [1.08, 0.95, 0.85];

fn try_layout(cx: &Ctx, p: &Params, scale: f64, systems: &[Vec<usize>], bm: &ByMeasure, mlen: f64, w: f64, h: f64, gap: f64) -> Option<Vec<Sys>> {
    let pg = Page::new(p, scale, gap);
    let margin = pg.margin;
    let left_label = pg.d * 0.9;
    let avail_w = w - 2.0 * margin - left_label;
    let header = if p.print_mode {
        if p.page_index == 0 { title_height(p, &pg, avail_w) } else { pg.d * 0.5 }
    } else {
        pg.d * 1.4
    };
    let padv = if p.print_mode { p.print_pad } else { 1.6 };
    let td = p.t_downbeat.unwrap();
    let mut y = margin + header;
    let mut out = Vec::new();
    for ms in systems {
        let (padl, padr) = (pg.d * 0.8, pg.d * 0.5);
        let mut widths = Vec::new();
        for &m in ms {
            let items = &bm[&m];
            let base_w = avail_w / ms.len() as f64;
            let inner = base_w - padl - padr;
            let (mut lastx, mut lasthw) = (-1e9f64, 0.5f64);
            let mut last_pos = None;
            for (t, g) in items {
                let hw = chord_halfwidth(cx, g, p, &pg);
                let x = ((t - (td + m as f64 * mlen)) / mlen) * inner;
                let x = x.max(lastx + pg.d * pg.gap.max(lasthw + hw + 0.08));
                last_pos = Some(x);
                lastx = x;
                lasthw = hw;
            }
            let mut need = match last_pos { Some(lp) => lp + padl + padr + pg.d * lasthw.max(0.5), None => base_w };
            if let Some(lp) = last_pos {
                if lp <= inner + pg.d * 0.5 {
                    need = base_w; // a note right at the bar line needs no extra room
                }
            }
            widths.push(need.max(pg.d * 4.0));
        }
        if widths.iter().sum::<f64>() > avail_w + 1.0 {
            return None;
        }
        // water-filling: measures needing more than an equal share keep what they need, the rest split the remainder
        let needs = widths.clone();
        let share = avail_w / widths.len() as f64;
        let big: Vec<usize> = (0..needs.len()).filter(|&i| needs[i] > share).collect();
        let rest: Vec<usize> = (0..needs.len()).filter(|i| !big.contains(i)).collect();
        if !rest.is_empty() {
            let each = (avail_w - big.iter().map(|&i| needs[i]).sum::<f64>()) / rest.len() as f64;
            for &i in &rest {
                widths[i] = needs[i].max(each);
            }
            let tot: f64 = widths.iter().sum();
            widths.iter_mut().for_each(|w| *w = *w * avail_w / tot);
        }
        // recompute proportional positions with final widths, keep the minimum-gap guarantee
        let mut final_ = Vec::new();
        for (&m, &wd) in ms.iter().zip(&widths) {
            let inner = wd - padl - padr;
            let (mut pos, mut lastx, mut lasthw) = (Vec::new(), -1e9f64, 0.5f64);
            for (t, g) in &bm[&m] {
                let hw = chord_halfwidth(cx, g, p, &pg);
                let x = ((t - (td + m as f64 * mlen)) / mlen) * inner;
                let x = x.max(lastx + pg.d * pg.gap.max(lasthw + hw + 0.08));
                pos.push(x);
                lastx = x;
                lasthw = hw;
            }
            if let Some(&lp) = pos.last() {
                if lp + padl + pg.d * lasthw > wd - pg.d * 0.1 {
                    // last chord would cross the bar line: squeeze a little
                    let kk = (wd - padl - pg.d * (lasthw + 0.1)) / lp;
                    pos.iter_mut().for_each(|x| *x *= kk);
                }
            }
            final_.push((padl, pos));
        }
        // vertical extents per lane for this system
        let (mut geos, mut ext) = (Vec::new(), Vec::new());
        for (li, &hand) in cx.lanes.iter().enumerate() {
            let _ = li;
            let (bottom_step, treble) = lane_geometry(cx.lanes.len(), cx.med[&hand]);
            let mut lo = bottom_step;
            let mut hi = bottom_step + 8;
            for &m in ms {
                for (_, g) in &bm[&m] {
                    for &i in g {
                        if cx.notes[i].hand == hand {
                            let nm = name_of(cx.notes[i].midi, p.flat);
                            let st = step_of(nm.0, nm.2);
                            lo = lo.min(st);
                            hi = hi.max(st);
                        }
                    }
                }
            }
            let up = (hi - (bottom_step + 8)) as f64 * pg.sp / 2.0 + pg.base * padv;
            let dn = (bottom_step - lo) as f64 * pg.sp / 2.0 + pg.base * padv;
            geos.push((bottom_step, treble));
            ext.push((up, dn));
        }
        let lane_gap = pg.base * if p.print_mode { 0.25 } else { 0.6 };
        let (mut height, mut tops) = (0.0, Vec::new());
        for (li, (up, dn)) in ext.iter().enumerate() {
            tops.push(y + height + up);
            height += up + 4.0 * pg.sp + dn + if li < cx.lanes.len() - 1 { lane_gap } else { 0.0 };
        }
        out.push(Sys { ms: ms.clone(), widths, final_, geos, tops, y0: y, y1: y + height });
        y += height + pg.d * if p.print_mode { 0.5 } else { 0.9 };
    }
    let foot = if p.print_mode { margin } else { margin * 0.6 };
    if y - pg.d * (if p.print_mode { 0.5 } else { 0.9 }) > h - foot {
        return None;
    }
    Some(out)
}

/// Layout at `scale`, loosening the note spacing a little if needed.
fn fit(cx: &Ctx, p: &Params, scale: f64, systems: &[Vec<usize>], bm: &ByMeasure, mlen: f64, w: f64, h: f64) -> (Option<Vec<Sys>>, f64) {
    let gaps: &[f64] = if p.print_mode { &GAPS } else { &GAPS[..1] };
    for &g in gaps {
        if let Some(r) = try_layout(cx, p, scale, systems, bm, mlen, w, h, g) {
            return (Some(r), g);
        }
    }
    (None, GAPS[2])
}

static PLANS: Mutex<Option<HashMap<String, Plan>>> = Mutex::new(None);

pub fn plan_print(notes: &[Note], p: &mut Params) -> Plan {
    let key = format!(
        "{}|{:.3}|{:.3}|{}|{}|{:.5}|{:.4}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        notes.len(), notes[0].start, notes[notes.len() - 1].end, notes.iter().map(|n| n.midi as i64).sum::<i64>(), p.beats_per_measure,
        p.beat_period.unwrap(), p.t_downbeat.unwrap(), p.dot, p.measures_per_row, p.line_space, p.flat, p.group_chords,
        p.systems_per_page, p.print_sp, p.print_pad, p.title
    );
    if let Some(pl) = PLANS.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        p.plan = Some(pl.clone());
        return pl.clone();
    }
    let plan = plan_print_inner(notes, p);
    let mut g = PLANS.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 40 {
        m.clear();
    }
    m.insert(key, plan.clone());
    p.plan = Some(plan.clone());
    plan
}

/// A4 print: one note size for the WHOLE song and rows packed with as many measures as fit (up to measures_per_row), so
/// every page looks alike.
fn plan_print_inner(notes: &[Note], p: &mut Params) -> Plan {
    p.plan = None;
    let keep = p.page_index;
    p.dot_mult = A4_SIZE.0 as f64 / REF_WIDTH;
    let (w, h) = (A4_SIZE.0 as f64, A4_SIZE.1 as f64);
    let mlen = p.beat_period.unwrap() * p.beats_per_measure as f64;
    let td = p.t_downbeat.unwrap();
    let last = notes.iter().map(|n| n.start).fold(f64::MIN, f64::max) + 1e-3;
    let total = ((((last - td) / mlen - 1e-6).ceil()) as i64).max(1) as usize;
    let cx = lanes_of(notes);
    let cl = cluster(notes, p.group_chords);
    let bm: ByMeasure = (0..total)
        .map(|m| (m, cl.iter().filter(|(t, _)| td + m as f64 * mlen - 1e-6 <= *t && *t < td + (m + 1) as f64 * mlen - 1e-6).cloned().collect()))
        .collect();
    let per = (if p.measures_per_row == 0 { 4 } else { p.measures_per_row }).max(1);
    // width of a measure in dot sizes when its notes sit at the tighter spacing: padding + one column per note
    let mut units: Vec<f64> = (0..total).map(|m| 4.0f64.max(1.3 + 0.5 + (bm[&m].len().saturating_sub(1)) as f64 * GAPS[1])).collect();
    let margin_u = 2.0 * 90.0 / p.dot + 0.9;
    units.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pos = 0.9 * (units.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    let busy = units[lo] + (units[hi] - units[lo]) * (pos - lo as f64); // the busiest tenth of the measures may squeeze more
    let mut scale = (w / (busy + margin_u) / (p.dot * p.dot_mult)).min(1.0);
    scale = scale.max(p.min_scale);

    let pack = |p: &mut Params, scale: f64| -> Option<Vec<Vec<usize>>> {
        let (mut rows, mut cur): (Vec<Vec<usize>>, Vec<usize>) = (vec![], vec![]);
        let keep_page = p.page_index;
        p.page_index = 1;
        for m in 0..total {
            if !cur.is_empty() {
                let mut trial = cur.clone();
                trial.push(m);
                if cur.len() >= per || fit(&cx, p, scale, &[trial], &bm, mlen, w, h).0.is_none() {
                    rows.push(std::mem::take(&mut cur));
                }
            }
            if cur.is_empty() && fit(&cx, p, scale, &[vec![m]], &bm, mlen, w, h).0.is_none() {
                p.page_index = keep_page;
                return None;
            }
            cur.push(m);
        }
        if !cur.is_empty() {
            rows.push(cur);
        }
        p.page_index = keep_page;
        Some(rows)
    };
    let paginate = |p: &mut Params, scale: f64| -> Option<Vec<Vec<Vec<usize>>>> {
        let rows = pack(p, scale)?;
        let (mut pages, mut i) = (Vec::new(), 0usize);
        while i < rows.len() {
            p.page_index = pages.len();
            let mut n = 0;
            while i + n < rows.len() && (p.systems_per_page <= 0 || n < MAX_ROWS) && fit(&cx, p, scale, &rows[i..i + n + 1], &bm, mlen, w, h).0.is_some() {
                n += 1;
            }
            if n == 0 {
                return None;
            }
            pages.push(rows[i..i + n].to_vec());
            i += n;
        }
        Some(pages)
    };
    // largest size at which every page (the last one aside) holds at least MIN_ROWS rows: a normal-looking sheet
    let mut pages = None;
    while scale > p.min_scale {
        pages = paginate(p, scale);
        if let Some(pg) = &pages {
            let n_rows = pack(p, scale).map_or(0, |r| r.len());
            if pg[..pg.len() - 1].iter().all(|x| x.len() >= MIN_ROWS) && pg[0].len() >= MIN_ROWS.min(n_rows) {
                break;
            }
        }
        scale = p.min_scale.max(scale * 0.95);
    }
    let pages = pages.unwrap_or_else(|| vec![(0..total).map(|m| vec![m]).collect::<Vec<_>>()]);
    p.page_index = keep;
    let plan = Plan { pages, scale };
    p.plan = Some(plan.clone());
    plan
}

/// (first measure index, number of rows, measures per row) shown on a page.
pub fn measure_range(p: &Params, page: usize) -> (usize, usize, usize) {
    if let Some(pl) = &p.plan {
        return (0, pl.pages[page.min(pl.pages.len() - 1)].len(), 0);
    }
    if p.print_mode || p.measures_per_row > 0 {
        let per = if p.measures_per_row > 0 { p.measures_per_row } else { 4 };
        let sp = p.systems_per_page as usize;
        let rows = if page == 0 && p.first_page_rows > 0 { p.first_page_rows } else { sp };
        let first = if p.first_page_rows > 0 { p.first_page_rows } else { sp };
        let start = if page == 0 { 0 } else { first * per + (page - 1) * sp * per };
        return (start, rows, per);
    }
    let per = (p.measures_per_page as f64 / p.systems_per_page as f64).ceil() as usize;
    (page * p.measures_per_page, p.systems_per_page as usize, per)
}

pub fn pages_needed(p: &Params, total_measures: usize) -> usize {
    if let Some(pl) = &p.plan {
        return pl.pages.len();
    }
    let mut n = 1;
    loop {
        let (start, rows, per) = measure_range(p, n - 1);
        if start + rows * per >= total_measures {
            return n;
        }
        n += 1;
    }
}

// ------------------------------------------------------------------ rendering
pub struct Rendered {
    pub gray: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

pub fn render(notes: &[Note], p: &mut Params, page_index: usize) -> Rendered {
    render_ss(notes, p, page_index, 2)
}

/// `ss` is the supersampling factor: the page is drawn `ss` times larger and scaled down for smooth edges.
pub fn render_ss(notes: &[Note], p: &mut Params, page_index: usize, ss: usize) -> Rendered {
    if p.print_mode {
        p.size = A4_SIZE;
        p.rotate = Rotate::None;
        p.dot_mult = A4_SIZE.0 as f64 / REF_WIDTH;
    }
    let (w, h) = (p.size.0 as f64, p.size.1 as f64);
    p.page_index = page_index;
    p.boxes.clear();
    p.dots.clear();
    p.pending.clear();
    resolve(notes, p);
    p.restrike.clear();
    let mut by_key: HashMap<i32, usize> = HashMap::new();
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|&a, &b| notes[a].start.partial_cmp(&notes[b].start).unwrap());
    for i in order {
        if let Some(&prev) = by_key.get(&notes[i].midi) {
            p.restrike.insert(prev, notes[i].start);
        }
        by_key.insert(notes[i].midi, i);
    }
    let mlen = p.beat_period.unwrap() * p.beats_per_measure as f64;
    let td = p.t_downbeat.unwrap();
    let cx = lanes_of(notes);
    let cl = cluster(notes, p.group_chords);
    let systems: Vec<Vec<usize>> = if let Some(pl) = &p.plan {
        pl.pages[page_index.min(pl.pages.len() - 1)].clone()
    } else {
        let (mi0, rows, per) = measure_range(p, page_index);
        let ms: Vec<usize> = (mi0..mi0 + rows * per).collect();
        ms.chunks(per).map(|c| c.to_vec()).collect()
    };
    let bm: ByMeasure = systems
        .iter()
        .flatten()
        .map(|&m| (m, cl.iter().filter(|(t, _)| td + m as f64 * mlen - 1e-6 <= *t && *t < td + (m + 1) as f64 * mlen - 1e-6).cloned().collect()))
        .collect();
    // largest size at which the page fits; keeps shrinking in small steps so a dense page never runs off the edge
    let mut scale = p.plan.as_ref().map_or(1.0, |pl| pl.scale);
    let (layout, gap) = loop {
        let (r, gap) = fit(&cx, p, scale, &systems, &bm, mlen, w, h);
        if r.is_some() || scale <= p.min_scale {
            break (r, gap);
        }
        scale = p.min_scale.max(scale * 0.95);
    };
    p.used_scale = scale;
    let img = draw(&cx, p, scale, &systems, &bm, mlen, layout, w, h, ss, gap);
    let (mut gray, mut gw, mut gh) = img;
    let (ow, oh) = (p.size.0 as usize, p.size.1 as usize);
    match p.rotate {
        Rotate::Cw => {
            // pixel (x, y) -> (H - 1 - y, x)
            let mut out = vec![0u8; gw * gh];
            for y in 0..gh {
                for x in 0..gw {
                    out[x * gh + (gh - 1 - y)] = gray[y * gw + x];
                }
            }
            gray = out;
            std::mem::swap(&mut gw, &mut gh);
        }
        Rotate::Ccw => {
            // pixel (x, y) -> (y, W - 1 - x)
            let mut out = vec![0u8; gw * gh];
            for y in 0..gh {
                for x in 0..gw {
                    out[(gw - 1 - x) * gh + y] = gray[y * gw + x];
                }
            }
            gray = out;
            std::mem::swap(&mut gw, &mut gh);
        }
        Rotate::None => {}
    }
    let (rw, rh) = (ow as f64, oh as f64);
    let rot = p.rotate;
    p.boxes = p.boxes.iter().map(|b| rotate_box(*b, rot, rw, rh)).collect();
    p.dots = p.dots.iter().map(|&(a, b, x, y)| { let (nx, ny) = rotate_point(x, y, rot, rw, rh); (a, b, nx, ny) }).collect();
    // never report anything outside the saved page: the web view would glow on empty space
    let (fw, fh) = (gw as f64, gh as f64);
    p.dots.retain(|d| 0.0 <= d.2 && d.2 <= fw && 0.0 <= d.3 && d.3 <= fh);
    p.boxes.retain(|b| b.4 - b.2 > 1.0 && b.5 - b.3 > 1.0);
    Rendered { gray, w: gw, h: gh }
}

fn rotate_point(x: f64, y: f64, rot: Rotate, w: f64, h: f64) -> (f64, f64) {
    match rot {
        Rotate::Cw => (h - y, x),
        Rotate::Ccw => (y, w - x),
        Rotate::None => (x, y),
    }
}

fn rotate_box(b: (f64, f64, f64, f64, f64, f64), rot: Rotate, w: f64, h: f64) -> (f64, f64, f64, f64, f64, f64) {
    let (t0, t1, x0, y0, x1, y1) = b;
    let (x0, x1) = (x0.max(0.0), x1.min(w));
    let (y0, y1) = (y0.max(0.0), y1.min(h));
    match rot {
        Rotate::Cw => (t0, t1, h - y1, x0, h - y0, x1),
        Rotate::Ccw => (t0, t1, y0, w - x1, y1, w - x0),
        Rotate::None => (t0, t1, x0, y0, x1, y1),
    }
}

fn dotted(cv: &mut Canvas, x: f64, y0: f64, y1: f64, s: f64) {
    let mut y = y0;
    while y < y1 {
        cv.line(&[(x, y), (x, (y + 5.0 * s).min(y1))], 170, s, false);
        y += 11.0 * s;
    }
}

/// Fonts and sizes of the dot labels at one page scale (sizes in supersampled pixels, whole numbers like PIL's).
struct Labels {
    fb: f64,
    fs: f64,
    fl: f64,
    fa: f64,
    ring: f64,
    s: f64,
    octave: bool,
}

/// One note dot (or the pale "let go" dot of a held note) with its letter.
fn put(cv: &mut Canvas, lb: &Labels, d: f64, x: f64, y: f64, letter: usize, acc: i32, octave: i32, release: bool, filled: bool) {
    let k = |v: f64| v * lb.s;
    let r = d / 2.0;
    let bx = [k(x - r), k(y - r), k(x + r), k(y + r)];
    let col;
    if release {
        // end of a held note: not filled, same letter, so it reads as "let go here"
        cv.ellipse(bx, Some(if filled { 255 } else { 225 }), Some(0), lb.ring);
        col = 0;
    } else if filled {
        cv.ellipse(bx, Some(0), None, 0.0);
        col = 255;
    } else {
        cv.ellipse(bx, Some(255), Some(0), lb.ring);
        col = 0;
    }
    let l = LETTERS[letter].to_string();
    let lw;
    if acc != 0 {
        // letter + small accidental as one group, centred in the dot (e.g. E♭)
        let sym = if acc == 1 { "♯" } else { "♭" };
        lw = canvas::text_len(&l, Font::Bold, lb.fl);
        let sw = canvas::text_len(sym, Font::Bold, lb.fa);
        let gw = lw + sw * 0.85;
        let tx = k(x) - gw / 2.0;
        cv.text(tx, k(y) - lb.fl * 0.62, &l, Font::Bold, lb.fl, col, 0.0);
        cv.text(tx + lw - sw * 0.05, k(y) - lb.fl * 0.62 + lb.fl * 0.02, sym, Font::Bold, lb.fa, col, ((lb.s as i64) / 2) as f64);
    } else {
        lw = canvas::text_len(&l, Font::Bold, lb.fb);
        cv.text(k(x) - lw / 2.0, k(y) - lb.fb * 0.62, &l, Font::Bold, lb.fb, col, 0.0);
    }
    if lb.octave {
        cv.text(k(x) + lw * 0.1, k(y) + lb.fb * 0.1, &octave.to_string(), Font::Bold, lb.fs, col, 0.0);
    }
}

fn draw_title(cv: &mut Canvas, p: &Params, pg: &Page, maxw: f64, s: f64) {
    let (size, lines) = title_lines(p, pg, maxw);
    let mut y = pg.margin;
    for line in lines {
        cv.text(pg.margin * s, y * s, &line, Font::Bold, (size * s).trunc(), 0, 0.0);
        y += size * 1.25;
    }
    cv.line(&[(pg.margin * s, (y + pg.d * 0.15) * s), ((pg.margin + maxw + pg.d * 0.9) * s, (y + pg.d * 0.15) * s)], 150, s.max(1.0), false);
}

fn draw(cx: &Ctx, p: &mut Params, scale: f64, systems: &[Vec<usize>], bm: &ByMeasure, mlen: f64, layout: Option<Vec<Sys>>, w: f64, h: f64, ss: usize, gap: f64) -> (Vec<u8>, usize, usize) {
    let mut layout = layout.unwrap_or_else(|| try_layout(cx, p, p.min_scale, systems, bm, mlen, w * 3.0, h * 3.0, gap).unwrap_or_default());
    let pg = Page::new(p, scale, gap);
    let n_lay = layout.len();
    if n_lay > 0 && p.print_mode {
        // spread the rows evenly down the sheet (capped, so a short last page stays tidy)
        let bottom0 = layout[n_lay - 1].y1 + pg.d * 0.6;
        let extra = (h - pg.margin * 1.3 - bottom0).max(0.0);
        let per = if n_lay > 1 { (extra / (n_lay - 1) as f64).min(pg.d * 3.0) } else { 0.0 };
        for (i, sl) in layout.iter_mut().enumerate() {
            sl.tops.iter_mut().for_each(|t| *t += i as f64 * per);
            sl.y0 += i as f64 * per;
            sl.y1 += i as f64 * per;
        }
    } else if n_lay > 0 {
        // centre the staff block vertically in the space under the title
        let (top0, bottom0) = (layout[0].y0 - pg.base * 1.6, layout[n_lay - 1].y1 + pg.d * 0.6);
        let dy = ((h - pg.margin * 0.6 - top0 - (bottom0 - top0)) / 2.0).max(0.0);
        for sl in layout.iter_mut() {
            sl.tops.iter_mut().for_each(|t| *t += dy);
            sl.y0 += dy;
            sl.y1 += dy;
        }
    }
    let s = ss as f64;
    let k = |v: f64| v * s;
    let mut cv = Canvas::new((w as usize * ss) as u32, (h as usize * ss) as u32);
    let ft = (12.0f64.max(pg.d * 0.32) * s).trunc();
    let ftt = (14.0f64.max(pg.d * 0.42) * s).trunc();
    let lb = Labels {
        fb: (pg.d * 0.58 * s).trunc(),
        fs: (pg.d * 0.34 * s).trunc(),
        fl: (pg.d * 0.52 * s).trunc(),
        fa: (pg.d * 0.44 * s).trunc(),
        ring: 2.0f64.max((2.6 * s * pg.s).trunc()),
        s,
        octave: p.show_octave,
    };
    let margin = pg.margin;
    let left_label = pg.d * 0.9;
    if p.print_mode {
        if p.page_index == 0 {
            draw_title(&mut cv, p, &pg, w - 2.0 * margin - left_label, s);
        }
        cv.text_mm(k(w / 2.0), k(h - margin * 0.55), &(p.page_index + 1).to_string(), Font::Regular, ft, 90);
    } else {
        cv.text(k(margin), k(margin * 0.55), &p.title, Font::Bold, ftt, 0, 0.0);
    }
    let td = p.t_downbeat.unwrap();
    let nl = cx.lanes.len();
    for sl in &layout {
        let mut xcur = margin + left_label;
        let total_w: f64 = sl.widths.iter().sum();
        for li in 0..nl {
            let top = sl.tops[li];
            for i in 0..5 {
                let yy = top + i as f64 * pg.sp;
                cv.line(&[(k(xcur), k(yy)), (k(xcur + total_w), k(yy))], 0, 1.0f64.max((1.6 * s).trunc()), false);
            }
            let lab = if li == 0 && nl > 1 { "R" } else if nl > 1 { "L" } else { "" };
            if !lab.is_empty() {
                cv.text(k(margin + 4.0), k(top + 2.0 * pg.sp - pg.d * 0.25), lab, Font::Regular, ft, 90, 0.0);
            }
        }
        // grand-staff brace line at the left
        if nl > 1 {
            cv.line(&[(k(xcur), k(sl.tops[0])), (k(xcur), k(sl.tops[nl - 1] + 4.0 * pg.sp))], 0, (2.5 * s).trunc(), false);
        }
        // time -> x along this row (measure starts, note columns, measure ends), so a held note can be drawn out to its release
        let (mut at, mut ax, mut xx) = (Vec::<f64>::new(), Vec::<f64>::new(), xcur);
        for ((&m, &wd), (padl, pos)) in sl.ms.iter().zip(&sl.widths).zip(&sl.final_) {
            let t_m = td + m as f64 * mlen;
            at.push(t_m);
            ax.push(xx + padl);
            for ((t, _), x) in bm[&m].iter().zip(pos) {
                if *t > *at.last().unwrap() {
                    at.push(*t);
                    ax.push(xx + padl + x);
                }
            }
            at.push(t_m + mlen);
            let last = *ax.last().unwrap();
            ax.push((xx + wd - pg.d * 0.5).max(last + 1e-3));
            xx += wd;
        }
        let row_end = xcur + total_w - pg.d * 0.2;
        let xmap = |t: f64| -> (f64, bool) {
            if t >= *at.last().unwrap() {
                return (row_end, true);
            }
            let j = at.partition_point(|&a| a <= t);
            if j == 0 {
                return (ax[0], false);
            }
            let j = j - 1;
            (ax[j] + (ax[j + 1] - ax[j]) * (t - at[j]) / (at[j + 1] - at[j]), false)
        };
        for ((&m, &wd), (padl, pos)) in sl.ms.iter().zip(&sl.widths).zip(&sl.final_) {
            let t_m = td + m as f64 * mlen; // where this measure starts in the song, so a click on it can start playback
            p.boxes.push((t_m, t_m + mlen, xcur, sl.tops[0] - pg.base * 1.2, xcur + wd, sl.tops[nl - 1] + 4.0 * pg.sp + pg.base));
            for b in 0..p.beats_per_measure {
                let bx = xcur + padl + (b as f64 / p.beats_per_measure as f64) * (wd - padl - pg.d * 0.5);
                if p.show_beats && b > 0 {
                    dotted(&mut cv, k(bx), k(sl.tops[0] - pg.base * 0.3), k(sl.tops[nl - 1] + 4.0 * pg.sp + pg.base * 0.3), s);
                }
                if p.show_beats {
                    cv.text(k(bx - pg.d * 0.1), k(sl.y1 - pg.base * 0.1), &(b + 1).to_string(), Font::Regular, ft, 120, 0.0);
                }
            }
            let xnext = xcur + wd;
            cv.line(&[(k(xnext), k(sl.tops[0])), (k(xnext), k(sl.tops[nl - 1] + 4.0 * pg.sp))], 0, (2.2 * s).trunc(), false);
            cv.text(k(xcur + 6.0), k(sl.tops[0] - pg.base * 1.55), &(m + 1).to_string(), Font::Regular, ft, 90, 0.0);
            for ((_, g), x) in bm[&m].iter().zip(pos) {
                chord(cx, &mut cv, p, &pg, &lb, g, sl, xcur + padl + x, &xmap, mlen);
            }
            xcur = xnext;
        }
    }
    draw_holds(&mut cv, p, &pg, &lb);
    let (gray, gw, gh) = cv.into_gray();
    if ss == 1 {
        return (gray, gw, gh);
    }
    downscale_lanczos(&gray, gw, gh, ss)
}

/// Tie arcs from long notes to a pale release dot. If the release spot is taken by another note, the arc simply runs
/// into that note instead, so nothing is drawn on top of anything.
fn draw_holds(cv: &mut Canvas, p: &mut Params, pg: &Page, lb: &Labels) {
    let (s, d) = (lb.s, pg.d);
    let k = |v: f64| v * s;
    let r = d / 2.0;
    let mut rels: Vec<(f64, f64)> = vec![];
    let pending = std::mem::take(&mut p.pending);
    for hd in &pending {
        let (x, y) = (hd.x, hd.y);
        let mut xe = hd.xe;
        if !hd.clipped {
            for &(rx, ry) in &rels {
                // two notes let go together on neighbouring steps: the second sits beside the first
                if (ry - y).abs() < d * 0.9 && (rx - xe).abs() < d * 0.95 {
                    xe = rx + d * 1.02;
                }
            }
        }
        let sgn = if hd.up { -1.0 } else { 1.0 };
        let mut taken = None;
        if !hd.clipped {
            for &(_, _, ox, oy) in &p.dots {
                if (oy - y).abs() < d * 0.9 && (ox - xe).abs() < d * 0.95 && (ox - x).abs() > d * 0.5 {
                    taken = Some(ox);
                    break;
                }
            }
        }
        let x0 = x + r * 0.7;
        let x1 = match taken {
            Some(t) => t - r * 0.7,
            None if !hd.clipped => xe - r * 0.7,
            None => xe,
        };
        if x1 - x0 < d * 0.5 {
            continue;
        }
        let hh = (d * 0.55).min((x1 - x0) * 0.22);
        let pts: Vec<(f64, f64)> = (0..25)
            .map(|i| {
                let u = i as f64 / 24.0;
                (k(x0 + (x1 - x0) * u), k(y + sgn * (r * 0.78 + hh * 4.0 * u * (1.0 - u))))
            })
            .collect();
        cv.line(&pts, 0, 2.0f64.max((d * 0.07 * s).trunc()), true);
        if hd.clipped {
            let a = d * 0.2;
            let ya = y + sgn * r * 0.78;
            cv.polygon(&[(k(xe + a), k(ya)), (k(xe - a * 0.5), k(ya - a * 0.8)), (k(xe - a * 0.5), k(ya + a * 0.8))], 0);
        } else if taken.is_none() && !hd.again {
            put(cv, lb, d, xe, y, hd.letter, hd.acc, hd.octave, true, hd.filled);
            rels.push((xe, y));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn chord(cx: &Ctx, cv: &mut Canvas, p: &mut Params, pg: &Page, lb: &Labels, g: &[usize], sl: &Sys, cxp: f64, xmap: &dyn Fn(f64) -> (f64, bool), mlen: f64) {
    let (s, d) = (lb.s, pg.d);
    let k = |v: f64| v * s;
    let mut spans: Vec<(f64, f64)> = vec![];
    for (li, &hand) in cx.lanes.iter().enumerate() {
        let mut grp: Vec<usize> = g.iter().copied().filter(|&i| cx.notes[i].hand == hand).collect();
        if grp.is_empty() {
            continue;
        }
        grp.sort_by_key(|&i| cx.notes[i].midi);
        let (bottom_step, _) = sl.geos[li];
        let top = sl.tops[li];
        let bottom_y = top + 4.0 * pg.sp;
        let names: Vec<_> = grp.iter().map(|&i| name_of(cx.notes[i].midi, p.flat)).collect();
        let steps: Vec<i32> = names.iter().map(|nm| step_of(nm.0, nm.2)).collect();
        // neighbouring notes that would overlap form a cluster: drawn side by side, centred on the chord's moment, with a
        // soft capsule behind them so the whole thing reads as ONE chord
        let offs = chord_columns(&steps, pg);
        let has_cluster = offs.iter().any(|&o| o != 0.0);
        let placed: Vec<(usize, i32, f64, f64, usize, i32, i32)> = grp
            .iter()
            .enumerate()
            .map(|(q, &i)| {
                let (letter, acc, octave) = names[q];
                let y = bottom_y - (steps[q] - bottom_step) as f64 * pg.sp / 2.0;
                (i, steps[q], cxp + offs[q] * d, y, letter, acc, octave)
            })
            .collect();
        if has_cluster {
            // one capsule around EVERY note this hand plays at this moment, touching or not
            let ys = placed.iter().map(|q| q.3);
            let xs = placed.iter().map(|q| q.2);
            let pad = d * 0.08;
            let (ymin, ymax) = (ys.clone().fold(f64::MAX, f64::min), ys.fold(f64::MIN, f64::max));
            let (xmin, xmax) = (xs.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max));
            cv.rounded_rect([k(xmin - d / 2.0 - pad), k(ymin - d / 2.0 - pad), k(xmax + d / 2.0 + pad), k(ymax + d / 2.0 + pad)], k(d * 0.5 + pad), Some(226), Some(120), 1.0f64.max((1.4 * s).trunc()));
        }
        // ledger lines
        for q in &placed {
            let (st, x) = (q.1, q.2);
            let (lo_line, hi_line) = (bottom_step, bottom_step + 8);
            if st < lo_line - 1 || st > hi_line + 1 {
                let rng: Vec<i32> = if st < lo_line { (st..=lo_line - 2).rev().filter(|v| (lo_line - 2 - v) % 2 == 0).collect() } else { (hi_line + 2..=st).filter(|v| (v - (hi_line + 2)) % 2 == 0).collect() };
                for ls in rng {
                    let ly = bottom_y - (ls - bottom_step) as f64 * pg.sp / 2.0;
                    cv.line(&[(k(x - d * 0.75), k(ly)), (k(x + d * 0.75), k(ly))], 0, (1.6 * s).trunc(), false);
                }
            }
        }
        let (ymin, ymax) = placed.iter().fold((f64::MAX, f64::MIN), |a, q| (a.0.min(q.3), a.1.max(q.3)));
        spans.push((ymin, ymax));
        let filled = li == 0 || cx.lanes.len() == 1;
        if p.show_holds {
            // tied notes are drawn once the whole row is placed (see draw_holds)
            let beat = mlen / p.beats_per_measure as f64;
            let mid = top + 2.0 * pg.sp;
            for q in &placed {
                let (i, x, y) = (q.0, q.2, q.3);
                let n = &cx.notes[i];
                let nxt = p.restrike.get(&i).copied();
                let again = nxt.map_or(false, |nx| nx - n.end < 0.15);
                let (xe, clipped) = xmap(if again { nxt.unwrap() } else { n.end });
                if (n.end - n.start) / beat < 1.25 || xe - x < d * 2.4 {
                    continue;
                }
                p.pending.push(Hold { x, y, xe, clipped, again, up: y <= mid, letter: q.4, acc: q.5, octave: q.6, filled });
            }
        }
        for q in &placed {
            let n = &cx.notes[q.0];
            p.dots.push((n.start, n.end, q.2, q.3));
            p.dot_r = d / 2.0;
            put(cv, lb, d, q.2, q.3, q.4, q.5, q.6, false, filled);
        }
    }
    // both hands strike together: a dotted line across the gap between the two staves
    if p.show_link && spans.len() == 2 {
        let (y0, y1) = (spans[0].1 + d * 0.55, spans[1].0 - d * 0.55);
        if y1 - y0 > d * 0.3 {
            dotted(cv, k(cxp), k(y0), k(y1), s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(sp: f64) -> Page {
        let mut p = Params::default();
        p.line_space = sp;
        Page::new(&p, 1.0, 1.08)
    }

    #[test]
    fn untouched_chord_notes_stay_on_the_axis() {
        // A, E, A: no neighbours, nothing is shifted
        let steps = [step_of(5, 3), step_of(2, 4), step_of(5, 4)];
        assert!(chord_columns(&steps, &page(1.0)).iter().all(|&o| o == 0.0));
    }

    #[test]
    fn touching_notes_go_side_by_side_and_wider_lines_stack_them() {
        let steps = [step_of(1, 4), step_of(2, 4)]; // D and E
        let offs = chord_columns(&steps, &page(1.0));
        assert!(offs[0] != 0.0 && offs[1] != 0.0 && offs[0] != offs[1]);
        assert!(chord_columns(&steps, &page(2.0)).iter().all(|&o| o == 0.0));
    }

    #[test]
    fn note_names() {
        assert_eq!(name_of(61, false), (0, 1, 4)); // C#4
        assert_eq!(name_of(61, true), (1, -1, 4)); // Db4
        assert_eq!(name_of(60, false), (0, 0, 4));
    }
}
