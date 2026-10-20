//! Keyboard geometry: map a strip + first note + white-key count to per-key sample patches.
use serde::Serialize;

pub const WHITE: [char; 7] = ['C', 'D', 'E', 'F', 'G', 'A', 'B'];

pub fn semitone(c: char) -> Option<i32> {
    Some(match c {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    })
}

/// Black key exists after these white keys.
pub fn has_sharp(c: char) -> bool {
    matches!(c, 'C' | 'D' | 'F' | 'G' | 'A')
}

pub fn midi_number(name: &str) -> Option<i32> {
    let mut ch = name.chars();
    let letter = ch.next()?;
    let base = semitone(letter)? + i32::from(name.contains('#'));
    let octave: i32 = name.trim_start_matches(|c: char| "ABCDEFG#".contains(c)).parse().ok()?;
    Some(12 * (octave + 1) + base)
}

pub fn white_names(first: &str, n: usize) -> Option<Vec<String>> {
    let letter = first.chars().next()?;
    let mut octave: i32 = first.get(1..)?.parse().ok()?;
    let mut i = WHITE.iter().position(|&c| c == letter)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(format!("{}{}", WHITE[i], octave));
        i += 1;
        if i == 7 {
            i = 0;
            octave += 1;
        }
    }
    Some(out)
}

#[derive(Clone, Debug, Serialize)]
pub struct Key {
    pub name: String,
    pub midi: i32,
    pub black: bool,
    /// x0, y0, x1, y1 - the visible key (for the overlay)
    pub box_: [f64; 4],
    /// x0, y0, x1, y1 - pixels sampled for colour
    pub patch: [f64; 4],
}

pub fn build_keys(x0: f64, x1: f64, y0: f64, y1: f64, first: &str, n_white: usize) -> Option<Vec<Key>> {
    let black_frac = 0.6;
    let names = white_names(first, n_white)?;
    let w = (x1 - x0) / n_white as f64;
    let h = y1 - y0;
    let mut keys = Vec::new();
    for (i, nm) in names.iter().enumerate() {
        let kx0 = x0 + i as f64 * w;
        let py = y0 + h * 0.78;
        let ph = (h * 0.04).max(2.0);
        let pw = w * 0.22;
        let cx = kx0 + w / 2.0;
        keys.push(Key {
            name: nm.clone(),
            midi: midi_number(nm)?,
            black: false,
            box_: [kx0, y0, kx0 + w, y1],
            patch: [cx - pw, py - ph, cx + pw, py + ph],
        });
        let letter = nm.chars().next()?;
        if has_sharp(letter) && i < n_white - 1 {
            let sharp = format!("{}#{}", letter, &nm[1..]);
            let bw = w * 0.6;
            let bx = kx0 + w;
            let by1 = y0 + h * black_frac;
            let cy = y0 + h * 0.3;
            let bpw = bw * 0.25;
            let bph = (h * 0.05).max(2.0);
            keys.push(Key {
                midi: midi_number(&sharp)?,
                name: sharp,
                black: true,
                box_: [bx - bw / 2.0, y0, bx + bw / 2.0, by1],
                patch: [bx - bpw, cy - bph, bx + bpw, cy + bph],
            });
        }
    }
    keys.sort_by_key(|k| k.midi);
    Some(keys)
}
