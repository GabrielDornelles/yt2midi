//! Pressed-key detection: compare each key's colour to its colour in an all-released frame.
use crate::color::*;
use crate::keyboard::Key;
use crate::tools::Res;
use crate::video::{self, VideoInfo};
use serde::Serialize;
use std::path::Path;

pub const MIN_NOTE_S: f64 = 0.015;
pub const MERGE_GAP_S: f64 = 0.03;
/// a held key counts as released once it falls below this fraction of its peak colour distance
pub const RELEASE_FRAC: f64 = 0.25;
/// a press this short (in frames) counts as a flick and must prove itself ...
pub const FLASH_FRAMES: usize = 2;
/// ... by peaking at this many times the sensitivity threshold
pub const FLASH_STRENGTH: f64 = 1.5;
/// with the keyboard visible and few keys down, the median white key is at rest; title cards and fades push it far above
pub const VISIBLE_MEDIAN: f64 = 12.0;
pub const SETTLE_FRAMES: usize = 3;
/// pale hand colours sit around 20; grey flicker is < 3
pub const MIN_CHROMA: f64 = 10.0;
/// more keys than this at once is a scene change, not music
pub const MAX_SIMULTANEOUS: usize = 14;
const GRID: usize = 5;

/// Pixel positions (inside the frame buffer being read) of the sample grid for every key.
pub struct Sampler {
    pts: Vec<[(u32, u32); GRID * GRID]>,
    w: usize,
    /// first row of the full frame that the buffer holds
    pub y_off: usize,
}

fn linspace(a: f64, b: f64, n: usize) -> impl Iterator<Item = f64> {
    (0..n).map(move |i| a + (b - a) * i as f64 / (n - 1) as f64)
}

impl Sampler {
    /// `w`/`h` are the full frame size; the buffer later passed in holds rows `y_off..`.
    pub fn new(keys: &[Key], w: usize, h: usize, y_off: usize) -> Self {
        let pts = keys
            .iter()
            .map(|k| {
                let [x0, y0, x1, y1] = k.patch;
                let mut a = [(0u32, 0u32); GRID * GRID];
                let mut n = 0;
                for y in linspace(y0, y1, GRID) {
                    for x in linspace(x0, x1, GRID) {
                        let xi = (x.round_ties_even() as i64).clamp(0, w as i64 - 1) as u32;
                        let yi = (y.round_ties_even() as i64).clamp(0, h as i64 - 1) as u32;
                        a[n] = (xi, yi - y_off as u32);
                        n += 1;
                    }
                }
                a
            })
            .collect();
        Sampler { pts, w, y_off }
    }

    /// Smallest strip of rows that covers every sample point: (first row, height), both even.
    pub fn strip(keys: &[Key], w: usize, h: usize) -> (usize, usize) {
        let s = Sampler::new(keys, w, h, 0);
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        for k in &s.pts {
            for &(_, y) in k {
                lo = lo.min(y as usize);
                hi = hi.max(y as usize);
            }
        }
        let y0 = lo & !1;
        let mut hh = hi + 1 - y0;
        hh += hh & 1;
        if y0 + hh > h {
            hh = h - y0;
        }
        (y0, hh)
    }

    /// Lab colour per key: the median over its sample grid (ignores small specks such as sparkles).
    pub fn sample(&self, rgb: &[u8]) -> Vec<Lab> {
        self.pts
            .iter()
            .map(|pts| {
                let mut l = [0f32; GRID * GRID];
                let mut a = l;
                let mut b = l;
                for (i, &(x, y)) in pts.iter().enumerate() {
                    let o = (y as usize * self.w + x as usize) * 3;
                    let c = rgb_to_lab(rgb[o], rgb[o + 1], rgb[o + 2]);
                    l[i] = c[0];
                    a[i] = c[1];
                    b[i] = c[2];
                }
                let med = |v: &mut [f32; GRID * GRID]| {
                    let (_, m, _) = v.select_nth_unstable_by(GRID * GRID / 2, |p, q| p.partial_cmp(q).unwrap());
                    *m
                };
                [med(&mut l), med(&mut a), med(&mut b)]
            })
            .collect()
    }
}

pub fn pressed(lab: &Lab, base: &Lab, thr: f64) -> bool {
    dist(lab, base) > thr && chroma(lab) > MIN_CHROMA
}

pub fn pressed_mask(lab: &[Lab], base: &[Lab], thr: f64) -> Vec<bool> {
    lab.iter().zip(base).map(|(l, b)| pressed(l, b, thr)).collect()
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Find the one or two hand colours as hue peaks (nothing is hard-coded). Seeds come from the two biggest histogram
/// peaks, then a trimmed circular k-means refines them. Most populated first.
pub fn cluster_hands(angles: &[f64]) -> Vec<f64> {
    const MIN_SEP: f64 = 35.0;
    const MIN_FRAC: f64 = 0.08;
    if angles.is_empty() {
        return vec![];
    }
    let bins = 72usize;
    let mut hist = vec![0f64; bins];
    for &a in angles {
        let b = ((a / 360.0 * bins as f64) as usize).min(bins - 1);
        hist[b] += 1.0;
    }
    let kern = [1.0, 2.0, 3.0, 2.0, 1.0];
    let sm: Vec<f64> = (0..bins)
        .map(|i| (0..5).map(|j| hist[(i + bins + j - 2) % bins] * kern[j]).sum())
        .collect();
    let centers: Vec<f64> = (0..bins).map(|i| (i as f64 + 0.5) * 360.0 / bins as f64).collect();
    let argmax = |v: &[f64]| v.iter().enumerate().fold((0, f64::MIN), |b, (i, &x)| if x > b.1 { (i, x) } else { b }).0;
    let p1 = argmax(&sm);
    let sm2: Vec<f64> = (0..bins).map(|i| if circ_dist(centers[i], centers[p1]) >= MIN_SEP { sm[i] } else { 0.0 }).collect();
    let p2 = argmax(&sm2);
    let mut cents = vec![centers[p1]];
    if sm2[p2] >= MIN_FRAC * sm[p1] {
        cents.push(centers[p2]);
    }
    for _ in 0..20 {
        let mut new = Vec::new();
        let near: Vec<usize> = angles
            .iter()
            .map(|&a| {
                let d: Vec<f64> = cents.iter().map(|&c| circ_dist(a, c)).collect();
                d.iter().enumerate().fold((0, f64::MAX), |b, (i, &x)| if x < b.1 { (i, x) } else { b }).0
            })
            .collect();
        for (j, &c) in cents.iter().enumerate() {
            let sel: Vec<f64> = angles.iter().zip(&near).filter(|(&a, &n)| n == j && circ_dist(a, c) <= 45.0).map(|(&a, _)| a).collect();
            new.push(if sel.is_empty() { c } else { circ_mean(&sel) });
        }
        let same = new.iter().zip(&cents).all(|(a, b)| (a - b).abs() <= 0.05 + 1e-5 * b.abs());
        cents = new;
        if same {
            break;
        }
    }
    if cents.len() == 2 && circ_dist(cents[0], cents[1]) < MIN_SEP {
        cents.truncate(1);
    }
    let mut counts = vec![0usize; cents.len()];
    for &a in angles {
        let j = cents.iter().enumerate().fold((0, f64::MAX), |b, (i, &c)| { let d = circ_dist(a, c); if d < b.1 { (i, d) } else { b } }).0;
        counts[j] += 1;
    }
    let mut order: Vec<usize> = (0..cents.len()).collect();
    order.sort_by_key(|&j| counts[j]);
    order.reverse();
    order.into_iter().map(|j| cents[j]).collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct Note {
    pub name: String,
    pub midi: i32,
    pub start: f64,
    pub end: f64,
    #[serde(rename = "hue")]
    pub angle: Option<f64>,
    pub hand: i32,
    #[serde(skip)]
    pub chroma: f64,
}

impl Note {
    /// Rounded the way the saved JSON has always been (so songs made by earlier versions load the same).
    pub fn to_json(&self) -> serde_json::Value {
        let r = |v: f64, d: i32| { let m = 10f64.powi(d); (v * m).round_ties_even() / m };
        serde_json::json!({ "name": self.name, "midi": self.midi, "start": r(self.start, 4), "end": r(self.end, 4),
            "hand": self.hand, "hue": self.angle.map(|h| r(h, 1)) })
    }
}

struct Raw {
    key: usize,
    start: usize,
    end: usize,
    angles: Vec<f64>,
    peak: f64,
    chromas: Vec<f64>,
}

pub fn run(
    video_path: &Path,
    info: &VideoInfo,
    keys: &[Key],
    baseline: &[Lab],
    thr: f64,
    progress: &mut dyn FnMut(f64, String),
    hand_centroids: Option<Vec<f64>>,
    swap_hands: bool,
) -> Res<(Vec<Note>, Vec<f64>, f64)> {
    let fps = info.fps;
    let total = info.frames.max(1);
    let (y0, sh) = Sampler::strip(keys, info.width, info.height);
    let sampler = Sampler::new(keys, info.width, info.height, y0);
    let n = keys.len();
    let white: Vec<usize> = (0..n).filter(|&i| !keys[i].black).collect();
    let off_thr = thr * 0.6;
    let mut vis_run = 0usize;
    let mut on = vec![false; n];
    let mut start = vec![0usize; n];
    let mut peak = vec![0f64; n];
    let mut angles: Vec<Vec<f64>> = vec![vec![]; n];
    let mut chromas: Vec<Vec<f64>> = vec![vec![]; n];
    let mut raw: Vec<Raw> = Vec::new();
    let mut new_on = vec![false; n];
    let mut dists = vec![0f64; n];
    let seen = video::stream_strip(video_path, info, y0, sh, |i, buf| {
        let lab = sampler.sample(buf);
        for k in 0..n {
            dists[k] = dist(&lab[k], &baseline[k]);
        }
        let mut wd: Vec<f64> = white.iter().map(|&k| dists[k]).collect();
        vis_run = if median(&mut wd) <= VISIBLE_MEDIAN { vis_run + 1 } else { 0 };
        let mut count = 0;
        for k in 0..n {
            let col = chroma(&lab[k]) > MIN_CHROMA; // a pressed key is a saturated colour; grey flicker isn't a press
            // released when the colour fades well below this press's own peak, so a key resting a little off its
            // baseline cannot keep the note alive and swallow the next press
            let v = if on[k] { dists[k] > off_thr.max(RELEASE_FRAC * peak[k]) } else { dists[k] > thr } && col;
            new_on[k] = v;
            count += v as usize;
        }
        if vis_run < SETTLE_FRAMES || count > MAX_SIMULTANEOUS {
            new_on.iter_mut().for_each(|v| *v = false); // keyboard not on screen, or a fade/cut: nothing here is music
        }
        for k in 0..n {
            if new_on[k] && !on[k] {
                start[k] = i;
                angles[k].clear();
                chromas[k].clear();
                peak[k] = 0.0;
            }
            if new_on[k] {
                peak[k] = peak[k].max(dists[k]);
                let ck = chroma(&lab[k]);
                if ck > 12.0 && dists[k] > thr {
                    angles[k].push(hue_angle(&lab[k]));
                    chromas[k].push(ck);
                }
            }
        }
        for k in 0..n {
            if on[k] && !new_on[k] {
                raw.push(Raw { key: k, start: start[k], end: i, angles: angles[k].clone(), peak: peak[k], chromas: chromas[k].clone() });
            }
        }
        on.copy_from_slice(&new_on);
        if (i + 1) % 60 == 0 {
            progress(((i + 1) as f64 / total as f64).min(0.98), format!("Analyzing {}/{} frames", i + 1, total));
        }
    })?;
    for k in 0..n {
        if on[k] {
            raw.push(Raw { key: k, start: start[k], end: seen, angles: angles[k].clone(), peak: peak[k], chromas: chromas[k].clone() });
        }
    }

    // merge tiny gaps, drop tiny notes
    raw.sort_by_key(|r| (r.key, r.start));
    let mut merged: Vec<Raw> = Vec::new();
    for r in raw {
        match merged.last_mut() {
            Some(l) if l.key == r.key && (r.start as f64 - l.end as f64) / fps <= MERGE_GAP_S => {
                l.end = r.end;
                l.angles.extend(r.angles);
                l.peak = l.peak.max(r.peak);
                l.chromas.extend(r.chromas);
            }
            _ => merged.push(r),
        }
    }
    let mut notes: Vec<Note> = Vec::new();
    for r in merged {
        let len = r.end - r.start;
        if len as f64 / fps < MIN_NOTE_S {
            continue;
        }
        // a 1-2 frame flash is real only if it is clearly strong; weak ones are flicker
        if len <= FLASH_FRAMES && r.peak < FLASH_STRENGTH * thr {
            continue;
        }
        let ch = if r.chromas.is_empty() { 0.0 } else { median(&mut r.chromas.clone()) };
        notes.push(Note {
            name: keys[r.key].name.clone(),
            midi: keys[r.key].midi,
            start: r.start as f64 / fps,
            end: r.end as f64 / fps,
            angle: if r.angles.is_empty() { None } else { Some(circ_mean(&r.angles)) },
            hand: 0,
            chroma: ch,
        });
    }
    notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap().then(a.midi.cmp(&b.midi)));

    // a frame that only showed one hand is not enough: learn the colours from the whole video instead
    let learned: Vec<f64> = notes.iter().filter_map(|n| n.angle).collect();
    let cents = match hand_centroids {
        Some(c) if c.len() >= 2 => c,
        _ => cluster_hands(&learned),
    };
    let mut notes = drop_unlike_hands(notes, &cents);
    for n in notes.iter_mut() {
        n.hand = match (n.angle, cents.len()) {
            (Some(a), 2..) => cents.iter().enumerate().fold((0, f64::MAX), |b, (i, &c)| { let d = circ_dist(a, c); if d < b.1 { (i, d) } else { b } }).0 as i32,
            _ => 0,
        };
        if swap_hands && cents.len() >= 2 {
            n.hand = 1 - n.hand;
        }
    }
    progress(1.0, format!("Found {} notes", notes.len()));
    Ok((notes, cents, fps))
}

/// Sparkles, glows and other effects light a key dimly or in the wrong colour. Real notes share the video's hand
/// colours, so drop notes much paler than their hand's typical colour or far from both hand hues.
pub fn drop_unlike_hands(notes: Vec<Note>, cents: &[f64]) -> Vec<Note> {
    const MIN_REL_CHROMA: f64 = 0.4;
    const MAX_HUE_GAP: f64 = 50.0;
    if notes.len() < 30 || cents.is_empty() {
        return notes;
    }
    let near_gap = |a: f64| cents.iter().enumerate().fold((0, f64::MAX), |b, (i, &c)| { let d = circ_dist(a, c); if d < b.1 { (i, d) } else { b } });
    let info: Vec<Option<(usize, f64)>> = notes.iter().map(|n| n.angle.map(near_gap)).collect();
    let refs: Vec<f64> = (0..cents.len())
        .map(|j| {
            let mut v: Vec<f64> = notes.iter().zip(&info).filter(|(_, i)| matches!(i, Some((nj, _)) if *nj == j)).map(|(n, _)| n.chroma).collect();
            if v.is_empty() { 0.0 } else { median(&mut v) }
        })
        .collect();
    notes
        .into_iter()
        .zip(info)
        .filter(|(n, i)| matches!(i, Some((j, gap)) if *gap <= MAX_HUE_GAP && n.chroma >= MIN_REL_CHROMA * refs[*j]))
        .map(|(n, _)| n)
        .collect()
}
