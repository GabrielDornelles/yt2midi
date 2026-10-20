//! Automatic keyboard detection from a handful of video frames.
//! Works on a per-pixel median of frames sampled across the video, which removes falling notes and pressed-key
//! colours and leaves the resting keyboard.
use crate::analyze::{self, Sampler};
use crate::color::*;
use crate::keyboard::{build_keys, has_sharp, midi_number, WHITE};
use crate::tools::Res;
use crate::video::{self, Frame, VideoInfo};
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;

const BOTTOM_FRAC: f64 = 0.45; // keyboard is searched for in the bottom part of the frame
const N_SAMPLES: usize = 48;

pub struct Bottom {
    pub y_start: usize,
    /// y of the first row actually held (even-aligned, <= y_start)
    pub y_off: usize,
    pub frames: Vec<Frame>,
    pub times: Vec<f64>,
}

/// Evenly spaced frames across the video, bottom part only.
pub fn sample_bottom(path: &Path, info: &VideoInfo) -> Res<Bottom> {
    let y_start = (info.height as f64 * (1.0 - BOTTOM_FRAC)) as usize;
    let y_off = y_start & !1;
    let h = (info.height - y_off) & !1;
    let total = info.frames as f64;
    let lo = (total * 0.01) as i64;
    let hi = (((total * 0.99) as i64) - 1).max(1);
    let idxs: Vec<i64> = (0..N_SAMPLES).map(|i| (lo as f64 + (hi - lo) as f64 * i as f64 / (N_SAMPLES - 1) as f64) as i64).collect();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(6).build().map_err(|e| e.to_string())?;
    let got: Vec<(f64, Option<Frame>)> = pool.install(|| {
        idxs.par_iter()
            .map(|&i| {
                let t = i as f64 / info.fps;
                (t, video::frame_at(path, t, Some((y_off, h)), info).ok())
            })
            .collect()
    });
    let (mut frames, mut times) = (vec![], vec![]);
    for (t, f) in got {
        if let Some(f) = f {
            frames.push(f);
            times.push(t);
        }
    }
    if frames.is_empty() {
        return Err("Could not read frames from the video".into());
    }
    Ok(Bottom { y_start, y_off, frames, times })
}

/// Per-pixel median over the frames (mean of the two middle values when the count is even, like numpy).
pub fn median_stack(frames: &[Frame]) -> Frame {
    let (w, h) = (frames[0].w, frames[0].h);
    let n = frames.len();
    let mut out = vec![0u8; w * h * 3];
    out.par_chunks_mut(3 * w).enumerate().for_each(|(row, line)| {
        let mut vals = vec![0u8; n];
        for (i, px) in line.iter_mut().enumerate() {
            let o = row * w * 3 + i;
            for (j, f) in frames.iter().enumerate() {
                vals[j] = f.rgb[o];
            }
            vals.sort_unstable();
            *px = if n % 2 == 1 { vals[n / 2] } else { ((vals[n / 2 - 1] as u16 + vals[n / 2] as u16) / 2) as u8 };
        }
    });
    Frame { w, h, rgb: out }
}

/// Full-size frame whose bottom part holds the per-pixel median (the resting keyboard).
pub fn full_median(path: &Path, info: &VideoInfo) -> Res<Frame> {
    let b = sample_bottom(path, info)?;
    Ok(embed(&median_stack(&b.frames), b.y_off, info))
}

fn embed(strip: &Frame, y_off: usize, info: &VideoInfo) -> Frame {
    let mut rgb = vec![0u8; info.width * info.height * 3];
    let o = y_off * info.width * 3;
    rgb[o..o + strip.rgb.len()].copy_from_slice(&strip.rgb);
    Frame { w: info.width, h: info.height, rgb }
}

fn longest_run(mask: &[bool]) -> (usize, usize) {
    let mut best = (0, 0);
    let mut start: Option<usize> = None;
    for i in 0..=mask.len() {
        let v = i < mask.len() && mask[i];
        if v && start.is_none() {
            start = Some(i);
        } else if !v {
            if let Some(s) = start.take() {
                if i - s > best.1 - best.0 {
                    best = (s, i);
                }
            }
        }
    }
    best
}

/// White-key pitch in pixels from the autocorrelation of a brightness profile.
fn pitch(profile: &[f64], lo: usize, hi: usize) -> f64 {
    let mean = profile.iter().sum::<f64>() / profile.len() as f64;
    let p: Vec<f64> = profile.iter().map(|v| v - mean).collect();
    let hi = hi.min(p.len().saturating_sub(2));
    let mut ac: Vec<f64> = (0..=hi + 1).map(|lag| (0..p.len() - lag).map(|i| p[i] * p[i + lag]).sum()).collect();
    let a0 = ac[0];
    ac.iter_mut().for_each(|v| *v /= a0 + 1e-9);
    let peaks: Vec<usize> = (lo..hi).filter(|&l| ac[l] >= ac[l - 1] && ac[l] >= ac[l + 1] && ac[l] > 0.25).collect();
    if peaks.is_empty() {
        let mut best = lo;
        for l in lo..hi {
            if ac[l] > ac[best] {
                best = l;
            }
        }
        return best as f64;
    }
    let best = peaks.iter().map(|&l| ac[l]).fold(f64::MIN, f64::max);
    let l = *peaks.iter().find(|&&l| ac[l] > 0.75 * best).unwrap();
    let (a, b, c) = (ac[l - 1], ac[l], ac[l + 1]);
    l as f64 + 0.5 * (a - c) / (a - 2.0 * b + c + 1e-9)
}

#[derive(Clone, Debug, Serialize)]
pub struct Detection {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
    pub n_white: usize,
    pub first_letter: String,
    pub first_note: String,
    pub confidence: f64,
    pub letter_certain: bool,
}

/// `med` is the median of the bottom part of the frame (rows start at `y_start`).
pub fn detect_keyboard(med: &Frame, y_start: usize) -> Option<Detection> {
    let (w, h) = (med.w, med.h);
    let px = |x: usize, y: usize, c: usize| med.rgb[(y * w + x) * 3 + c];
    let bright = |x: usize, y: usize| px(x, y, 0).min(px(x, y, 1)).min(px(x, y, 2)) > 170;
    let rows: Vec<bool> = (0..h).map(|y| (0..w).filter(|&x| bright(x, y)).count() as f64 / w as f64 > 0.35).collect();
    let (r0, r1) = longest_run(&rows);
    if r1 - r0 < 20 {
        return None;
    }
    let kh = r1 - r0;
    let band0 = r0 + (kh as f64 * 0.6) as usize;
    let band_rows = (r1 - band0) as f64;
    let cols_raw: Vec<bool> = (0..w).map(|x| (band0..r1).filter(|&y| bright(x, y)).count() as f64 / band_rows > 0.5).collect();
    // morphological close with a 1x25 kernel: dilate then erode (the borders do not count)
    let dil: Vec<bool> = (0..w).map(|i| (i.saturating_sub(12)..=(i + 12).min(w - 1)).any(|j| cols_raw[j])).collect();
    let cols: Vec<bool> = (0..w).map(|i| (i.saturating_sub(12)..=(i + 12).min(w - 1)).all(|j| dil[j])).collect();
    let (c0, c1) = longest_run(&cols);
    if ((c1 - c0) as f64) < w as f64 * 0.2 {
        return None;
    }
    let gray = |x: usize, y: usize| (px(x, y, 0) as f64 + px(x, y, 1) as f64 + px(x, y, 2) as f64) / 3.0;
    let width = c1 - c0;
    let (pa, pb) = (r0 + (kh as f64 * 0.72) as usize, r0 + (kh as f64 * 0.85) as usize);
    let prof: Vec<f64> = (c0..c1).map(|x| (pa..pb).map(|y| gray(x, y)).sum::<f64>() / (pb - pa) as f64).collect();
    let p0 = pitch(&prof, 8, 12.max(width / 6));
    let n0 = 5.max((width as f64 / p0).round_ties_even() as usize);
    let at = |v: f64| (v.round_ties_even() as i64).clamp(0, width as i64 - 1) as usize;
    let mut best = (-1e9f64, n0);
    for n in 5.max(n0.saturating_sub(3))..n0 + 4 {
        let wk = width as f64 / n as f64;
        let seps: f64 = (1..n).map(|i| prof[at(i as f64 * wk)]).sum::<f64>() / (n - 1) as f64;
        let cent: f64 = (0..n).map(|i| prof[at((i as f64 + 0.5) * wk)]).sum::<f64>() / n as f64;
        let score = cent - seps;
        if score > best.0 + 1e-6 || ((score - best.0).abs() < 1.0 && n > best.1) {
            best = (score, n);
        }
    }
    let n = best.1;
    let wk = width as f64 / n as f64;
    // black keys: a dark window around each boundary in the upper part
    let (uy0, uy1) = (r0 + (kh as f64 * 0.15) as usize, r0 + (kh as f64 * 0.45) as usize);
    let has_black: Vec<bool> = (1..n)
        .map(|i| {
            let xb = c0 as f64 + i as f64 * wk;
            let xa = ((xb - wk * 0.12) as i64).max(0) as usize;
            let xz = (((xb + wk * 0.12) as i64) + 1).min(w as i64) as usize;
            let mut s = 0.0;
            let mut cnt = 0.0;
            for y in uy0..uy1.min(h) {
                for x in xa..xz {
                    s += gray(x, y);
                    cnt += 1.0;
                }
            }
            s / cnt < 110.0
        })
        .collect();
    let scores: Vec<f64> = (0..7)
        .map(|l0| (0..n - 1).filter(|&j| has_black[j] == has_sharp(WHITE[(l0 + j) % 7])).count() as f64 / ((n - 1).max(1)) as f64)
        .collect();
    let l0 = scores.iter().enumerate().fold((0, f64::MIN), |b, (i, &s)| if s > b.1 { (i, s) } else { b }).0;
    let mut ordered = scores.clone();
    ordered.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let certain = ordered[0] - ordered[1] > 0.1;
    let letter = WHITE[l0];
    let span = n as f64 * 12.0 / 7.0;
    let first = if n >= 51 && letter == 'A' {
        "A0".to_string()
    } else {
        // centre the keyboard around the middle of a typical piano range
        let target = 66.0 - span / 2.0;
        (0..7)
            .map(|o| { let nm = format!("{letter}{o}"); ((midi_number(&nm).unwrap() as f64 - target).abs(), nm) })
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap()
            .1
    };
    let r2 = |v: f64| (v * 100.0).round_ties_even() / 100.0;
    Some(Detection {
        x0: c0 as f64,
        x1: c1 as f64,
        y0: (y_start + r0) as f64,
        y1: (y_start + r1) as f64,
        n_white: n,
        first_letter: letter.to_string(),
        first_note: first,
        confidence: r2(ordered[0]),
        letter_certain: certain,
    })
}

pub struct Auto {
    pub det: Detection,
    pub times: Vec<f64>,
    /// the resting keyboard (full frame, only the bottom part filled in), a by-product worth keeping
    pub median: Frame,
}

/// Detect the keyboard and suggest frames where several keys are pressed.
pub fn autodetect(path: &Path, info: &VideoInfo) -> Res<Option<Auto>> {
    let b = sample_bottom(path, info)?;
    let med = median_stack(&b.frames);
    // detection looks at rows from y_start on; our strip may begin one row earlier (even alignment)
    let skip = (b.y_start - b.y_off) * med.w * 3;
    let sub = Frame { w: med.w, h: med.h - (b.y_start - b.y_off), rgb: med.rgb[skip..].to_vec() };
    let Some(det) = detect_keyboard(&sub, b.y_start) else { return Ok(None) };
    let Some(keys) = build_keys(det.x0, det.x1, det.y0, det.y1, &det.first_note, det.n_white) else { return Ok(None) };
    let sampler = Sampler::new(&keys, info.width, info.height, b.y_off);
    let base = sampler.sample(&med.rgb);
    let mut scored: Vec<(i64, f64)> = vec![];
    for (f, &t) in b.frames.iter().zip(&b.times) {
        let lab = sampler.sample(&f.rgb);
        let mask = analyze::pressed_mask(&lab, &base, 40.0);
        let k = mask.iter().filter(|&&m| m).count();
        if (2..=analyze::MAX_SIMULTANEOUS).contains(&k) {
            let hues: Vec<f64> = lab.iter().zip(&mask).filter(|(_, &m)| m).map(|(l, _)| hue_angle(l)).collect();
            let spread = analyze::cluster_hands(&hues).len() as i64;
            scored.push((spread * 100 + k as i64, t));
        }
    }
    scored.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let times = scored.iter().take(8).map(|&(_, t)| (t * 100.0).round_ties_even() / 100.0).collect();
    Ok(Some(Auto { det, times, median: embed(&med, b.y_off, info) }))
}
