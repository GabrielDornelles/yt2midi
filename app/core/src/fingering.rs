//! Which finger plays which note, found with plain rules and maths (no learned model).
//!
//! Each hand's notes become a row of events (a note, or a chord). Every event has a few possible assignments of
//! fingers 1 (thumb) to 5 (little finger), in the order the hand has them. A Viterbi search then picks the cheapest
//! path through the song. The cost mixes the ideas of Parncutt et al. (1997) and Al Kasimi et al. (2007):
//!  - how far the hand has to travel, which is measured by where the hand's "home" would be for each finger,
//!  - how far two fingers can stretch apart (comfortable and practical limits),
//!  - thumb under and finger over, which cost more the weaker the finger,
//!  - thumb and little finger on black keys, and weak fingers 4 and 5 generally,
//!  - chords that fit the natural spread of the hand.
//! The numbers are for a normal adult hand; they are rules of thumb, not measurements.

/// A note as the search sees it.
#[derive(Clone, Copy, Debug)]
pub struct In {
    pub midi: i32,
    pub start: f64,
    /// when the key comes up again; a note still down while the next one starts ties its finger up
    pub end: f64,
    pub hand: i32,
}

/// Notes within this many seconds start together (rolled chords are close enough to count as one).
pub const TOGETHER: f64 = 0.06;

/// Semitones from the thumb's key to each finger's key in a relaxed hand.
pub const HOME: [f64; 6] = [0.0, 0.0, 1.9, 3.8, 5.7, 7.6];

/// Furthest apart two fingers (a < b) reach in a chord: (comfortable, practical), in semitones.
fn reach(a: usize, b: usize) -> (f64, f64) {
    match (a, b) {
        (1, 2) => (5.0, 8.0),
        (1, 3) => (7.0, 10.0),
        (1, 4) => (9.0, 12.0),
        (1, 5) => (10.0, 13.0),
        (2, 3) => (2.0, 3.0),
        (2, 4) => (4.0, 5.0),
        (2, 5) => (6.0, 8.0),
        (3, 4) => (2.0, 3.0),
        (3, 5) => (4.0, 5.0),
        _ => (2.0, 3.0), // 4, 5
    }
}

/// Furthest a finger can lie "behind" the thumb (thumb under, finger over) comfortably, in semitones, for the pair (1, b).
fn behind(b: usize) -> f64 {
    match b {
        2 => 3.0,
        3 => 2.0,
        4 => 1.0,
        _ => 0.0,
    }
}

fn is_black(m: i32) -> bool {
    matches!(m.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

#[derive(Clone)]
struct State {
    /// (midi, finger) in rising pitch
    n: Vec<(i32, usize)>,
    /// where the hand's home sits for this state (in semitones)
    pos: f64,
    own: f64,
}

/// All ways to give `k` fingers to `pitches` (rising), in the order of the hand.
fn states(pitches: &[i32], right: bool) -> Vec<State> {
    let k = pitches.len();
    let mut out = Vec::new();
    let mut pick = Vec::new();
    fn go(start: usize, k: usize, pick: &mut Vec<usize>, all: &mut Vec<Vec<usize>>) {
        if pick.len() == k {
            all.push(pick.clone());
            return;
        }
        for f in start..=5 {
            pick.push(f);
            go(f + 1, k, pick, all);
            pick.pop();
        }
    }
    let mut all = Vec::new();
    go(1, k, &mut pick, &mut all);
    for fingers in all {
        // rising pitch goes with rising finger in the right hand, falling finger in the left
        let n: Vec<(i32, usize)> = if right {
            pitches.iter().copied().zip(fingers.iter().copied()).collect()
        } else {
            pitches.iter().copied().zip(fingers.iter().rev().copied()).collect()
        };
        let homes: Vec<f64> = n.iter().map(|&(m, f)| if right { m as f64 - HOME[f] } else { m as f64 + HOME[f] }).collect();
        let pos = homes.iter().sum::<f64>() / homes.len() as f64;
        let mut own = homes.iter().map(|h| (h - pos).abs()).sum::<f64>() * 0.12;
        for (i, &(m, f)) in n.iter().enumerate() {
            own += match (f, is_black(m)) {
                (1, true) => 2.0,
                (5, true) => 1.0,
                (2, true) => 0.3,
                (4, false) => 0.3,
                (5, false) => 0.2,
                _ => 0.0,
            };
            for &(m2, f2) in &n[i + 1..] {
                let (lo, hi) = if f < f2 { (f, f2) } else { (f2, f) };
                let span = (m2 - m).abs() as f64;
                let (c, p) = reach(lo, hi);
                if span > c { own += 1.5 * (span - c); }
                if span > p { own += 8.0 * (span - p); }
            }
        }
        out.push(State { n, pos, own });
    }
    out
}

/// `held` says which of a's notes are still down when b starts, by pitch.
fn step(a: &State, b: &State, gap: f64, right: bool, held: &[i32]) -> f64 {
    let free = (1.0 + 6.0 * gap).min(12.0);
    let mut c = 0.5 * ((b.pos - a.pos).abs() - free).max(0.0);
    let melodic = a.n.len() == 1 && b.n.len() == 1;
    for &(m1, f1) in &a.n {
        let down = held.contains(&m1);
        for &(m2, f2) in &b.n {
            if f1 == f2 {
                if m1 != m2 { c += 5.0 + 0.5 * (m2 - m1).abs() as f64 * (1.0 - gap.min(0.5)); }
                else if gap < 0.25 { c += 0.5 }
                if down && m1 != m2 { c += 40.0 } // a finger cannot play while it holds another key
            } else if down && m1 != m2 {
                // both keys are down at once: the fingers keep the order of the keys, as in a chord. Only the thumb may
                // be held while another finger goes over it (or goes under a held finger), and even that is awkward.
                let d = (m2 - m1) as f64;
                let span = if f2 > f1 { if right { d } else { -d } } else if right { -d } else { d };
                let (lo, hi) = (f1.min(f2), f1.max(f2));
                if span < 1.0 {
                    c += if lo == 1 { 10.0 + 3.0 * (1.0 - span) } else { 40.0 + 5.0 * (1.0 - span) };
                } else {
                    let (comf, prac) = reach(lo, hi);
                    if span > comf { c += 1.5 * (span - comf) }
                    if span > prac { c += 8.0 * (span - prac) }
                }
            } else if !melodic && gap < 0.4 && (a.n.len() == 1 || b.n.len() == 1) && f1.min(f2) > 1 && {
                let d = (m2 - m1) as f64;
                (if f2 > f1 { if right { d } else { -d } } else if right { -d } else { d }) < 1.0
            } {
                // next to a chord, one finger still cannot pass over another finger
                c += 30.0 + 20.0 * (0.4 - gap);
            } else if melodic {
                let d = (m2 - m1) as f64;
                // how far the second finger's key lies from the first's, counted outward (towards the little finger)
                let span = if f2 > f1 { if right { d } else { -d } } else if right { -d } else { d };
                let (lo, hi) = (f1.min(f2), f1.max(f2));
                if span < 1.0 {
                    // one finger crosses another: thumb under or finger over
                    let ok = lo == 1 && -span <= behind(hi);
                    // only the thumb passes under (or a finger over it); two other fingers cannot cross, least of all quickly
                    c += if ok { 2.0 + 0.8 * (hi as f64 - 2.0) } else if lo == 1 { 6.0 + 3.0 * (1.0 - span) } else { 30.0 + 10.0 * (1.0 - span) + 20.0 * (0.5 - gap).max(0.0) };
                    if f2 == 1 && is_black(m2) { c += 1.0 }
                } else {
                    let (comf, prac) = reach(lo, hi);
                    if span > comf { c += 1.5 * (span - comf) }
                    if span > prac { c += 8.0 * (span - prac) }
                }
            }
        }
    }
    c
}

/// A finger (1 to 5) for every note, and whether it is the right hand. The right hand is the one with the higher
/// notes; if the song has one hand only, it is the right hand when its notes are mostly above middle C.
pub fn assign(notes: &[In]) -> Vec<(u8, bool)> {
    let mut out = vec![(0u8, true); notes.len()];
    let mut hands: Vec<i32> = notes.iter().map(|n| n.hand).collect();
    hands.sort_unstable();
    hands.dedup();
    let mean = |h: i32| {
        let v: Vec<f64> = notes.iter().filter(|n| n.hand == h).map(|n| n.midi as f64).collect();
        v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    for &h in &hands {
        let right = match hands.iter().find(|&&o| o != h) {
            Some(&o) => mean(h) >= mean(o),
            None => mean(h) >= 60.0,
        };
        let idx: Vec<usize> = {
            let mut v: Vec<usize> = (0..notes.len()).filter(|&i| notes[i].hand == h).collect();
            v.sort_by(|&a, &b| notes[a].start.total_cmp(&notes[b].start).then(notes[a].midi.cmp(&notes[b].midi)));
            v
        };
        let fingers = hand(notes, &idx, right);
        for (&i, f) in idx.iter().zip(fingers) {
            out[i] = (f, right);
        }
    }
    out
}

/// The fingers for one hand's notes, which are in time order.
fn hand(notes: &[In], idx: &[usize], right: bool) -> Vec<u8> {
    // events: runs of notes that start together
    let mut events: Vec<(f64, Vec<usize>)> = Vec::new(); // (start, positions in idx)
    for (p, &i) in idx.iter().enumerate() {
        match events.last_mut() {
            Some((t, v)) if notes[i].start - *t <= TOGETHER => v.push(p),
            _ => events.push((notes[i].start, vec![p])),
        }
    }
    let pitches: Vec<Vec<i32>> = events
        .iter()
        .map(|(_, v)| {
            let mut m: Vec<i32> = v.iter().map(|&p| notes[idx[p]].midi).collect();
            m.sort_unstable();
            m.dedup();
            if m.len() > 5 {
                // a hand has five fingers: finger the outer notes and spread the rest
                m = (0..5).map(|i| m[(i * (m.len() - 1) + 2) / 4]).collect();
                m.dedup();
            }
            m
        })
        .collect();
    // which keys of each event are still down at a given time
    let ends: Vec<Vec<(i32, f64)>> = events.iter().map(|(_, v)| v.iter().map(|&p| (notes[idx[p]].midi, notes[idx[p]].end)).collect()).collect();
    let all: Vec<Vec<State>> = pitches.iter().map(|p| states(p, right)).collect();
    // Viterbi
    let mut cost: Vec<Vec<f64>> = Vec::with_capacity(all.len());
    let mut from: Vec<Vec<usize>> = Vec::with_capacity(all.len());
    for (e, ss) in all.iter().enumerate() {
        if e == 0 {
            cost.push(ss.iter().map(|s| s.own).collect());
            from.push(vec![0; ss.len()]);
            continue;
        }
        let gap = events[e].0 - events[e - 1].0;
        let held: Vec<i32> = ends[e - 1].iter().filter(|&&(_, end)| end > events[e].0 + 0.12).map(|&(m, _)| m).collect();
        let mut c = Vec::with_capacity(ss.len());
        let mut f = Vec::with_capacity(ss.len());
        for s in ss {
            let (bi, bc) = all[e - 1].iter().enumerate().map(|(i, p)| (i, cost[e - 1][i] + step(p, s, gap, right, &held))).fold((0, f64::MAX), |a, b| if b.1 < a.1 { b } else { a });
            c.push(bc + s.own);
            f.push(bi);
        }
        cost.push(c);
        from.push(f);
    }
    let mut out = vec![0u8; idx.len()];
    if all.is_empty() {
        return out;
    }
    let last = all.len() - 1;
    let mut s = cost[last].iter().enumerate().fold((0, f64::MAX), |a, (i, &c)| if c < a.1 { (i, c) } else { a }).0;
    for e in (0..all.len()).rev() {
        for &p in &events[e].1 {
            let m = notes[idx[p]].midi;
            // a note that was left out of a big chord takes the finger of the nearest one that was kept
            let f = all[e][s].n.iter().min_by_key(|&&(mm, _)| (mm - m).abs()).map(|&(_, f)| f).unwrap_or(0);
            out[p] = f as u8;
        }
        s = from[e][s];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(pitches: &[i32], gap: f64, hand: i32) -> Vec<In> {
        pitches.iter().enumerate().map(|(i, &m)| In { midi: m, start: i as f64 * gap, end: i as f64 * gap + gap * 0.9, hand }).collect()
    }
    fn fingers(n: &[In]) -> Vec<u8> {
        assign(n).iter().map(|x| x.0).collect()
    }

    #[test]
    fn c_major_right_hand() {
        let n = line(&[60, 62, 64, 65, 67, 69, 71, 72], 0.25, 0);
        assert_eq!(fingers(&n), vec![1, 2, 3, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn c_major_left_hand_going_up() {
        let n = line(&[48, 50, 52, 53, 55, 57, 59, 60], 0.25, 0);
        assert_eq!(fingers(&n), vec![5, 4, 3, 2, 1, 3, 2, 1]);
    }

    #[test]
    fn c_major_right_hand_going_down() {
        let n = line(&[72, 71, 69, 67, 65, 64, 62, 60], 0.25, 0);
        assert_eq!(fingers(&n), vec![5, 4, 3, 2, 1, 3, 2, 1]);
    }

    #[test]
    fn chords_follow_the_hand() {
        let c = |a: &[i32], hand| a.iter().map(|&m| In { midi: m, start: 0.0, end: 1.0, hand }).collect::<Vec<_>>();
        assert_eq!(fingers(&c(&[60, 64, 67], 0)), vec![1, 3, 5]);
        assert_eq!(fingers(&c(&[36, 40, 43], 0)), vec![5, 3, 1]);
    }

    #[test]
    fn arpeggio_uses_the_usual_fingers() {
        let n = line(&[60, 64, 67, 72], 0.2, 0);
        assert_eq!(fingers(&n), vec![1, 2, 3, 5]);
    }

    #[test]
    fn the_hand_with_higher_notes_is_the_right_one() {
        let mut n = line(&[40, 43], 0.3, 0);
        n.extend(line(&[72, 76], 0.3, 1));
        let a = assign(&n);
        assert!(!a[0].1 && a[2].1);
        assert!(a.iter().all(|x| (1..=5).contains(&x.0)));
    }

    #[test]
    fn a_held_key_keeps_its_finger_in_order() {
        // the key an octave up is held while the next, higher one is played: the finger for it cannot be the little finger
        for right in [true, false] {
            let up = |m: i32| if right { m } else { 120 - m };
            let n = vec![
                In { midi: up(64), start: 0.0, end: 0.25, hand: 0 },
                In { midi: up(72), start: 0.3, end: 1.5, hand: 0 },
                In { midi: up(74), start: 0.6, end: 0.8, hand: 0 },
                In { midi: up(72), start: 1.0, end: 1.2, hand: 0 },
            ];
            let f = fingers(&n);
            assert_ne!(f[1], 5, "{f:?}");
            assert!(f[2] != f[1], "{f:?}");
        }
    }

    #[test]
    fn fingers_do_not_cross_each_other_quickly() {
        // thumb and little finger together, then the key above the little finger's: the 4th finger cannot reach over it
        let n = vec![
            In { midi: 60, start: 0.0, end: 1.0, hand: 0 },
            In { midi: 67, start: 0.0, end: 0.2, hand: 0 },
            In { midi: 67, start: 0.25, end: 0.4, hand: 0 },
            In { midi: 68, start: 0.4, end: 0.6, hand: 0 },
        ];
        let f = fingers(&n);
        assert!(!(f[1] == 5 && f[3] == 4), "{f:?}");
    }

    #[test]
    fn big_chords_and_repeats_do_not_break() {
        let mut n = vec![];
        for (i, m) in [48, 52, 55, 60, 64, 67, 72].iter().enumerate() {
            n.push(In { midi: *m, start: 0.0, end: 0.5, hand: 0 });
            n.push(In { midi: *m, start: 1.0 + i as f64 * 0.1, end: 1.5, hand: 0 });
        }
        assert!(fingers(&n).iter().all(|f| (1..=5).contains(f)));
    }
}
