//! Render a synthetic Synthesia-like video with known notes, run the analysis, compare.
use std::io::Write;
use std::process::{Command, Stdio};
use yt2midi_core::{analyze, color, keyboard, midi, video};

const W: usize = 640;
const H: usize = 360;
const FPS: usize = 30;
const BOX: [f64; 4] = [20.0, 280.0, 620.0, 350.0]; // x0, y0, x1, y1 of the keyboard
const HANDS: [[u8; 3]; 2] = [[40, 140, 255], [90, 200, 60]]; // RGB: blue-ish, green-ish (the code does not know them)
const TRUTH: [(&str, f64, f64, usize); 5] = [("C4", 0.5, 1.0, 0), ("E4", 0.5, 1.0, 0), ("C#4", 1.5, 2.2, 0), ("G3", 1.0, 2.0, 1), ("A4", 2.5, 3.0, 1)];

fn fill(img: &mut [u8], b: [f64; 4], c: [u8; 3]) {
    for y in b[1] as usize..(b[3] as usize).min(H) {
        for x in b[0] as usize..(b[2] as usize).min(W) {
            img[(y * W + x) * 3..][..3].copy_from_slice(&c);
        }
    }
}

fn make_video(path: &std::path::Path, intro: usize) -> Vec<keyboard::Key> {
    let keys = keyboard::build_keys(BOX[0], BOX[2], BOX[1], BOX[3], "C3", 14).unwrap();
    // the sample video is encoded with the system ffmpeg (it needs x264); the app under test may use another build
    let mut ff = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{W}x{H}"), "-r", &FPS.to_string(), "-i", "-"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "14"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = ff.stdin.take().unwrap();
    for f in 0..(4 * FPS + intro) {
        let t = (f as f64 - intro as f64) / FPS as f64;
        let mut img = vec![30u8; W * H * 3];
        if f < intro {
            // title card: the keyboard is not on screen yet
            img.chunks_mut(3).for_each(|p| p.copy_from_slice(&[20, 40, 60]));
            stdin.write_all(&img).unwrap();
            continue;
        }
        let active = |name: &str| TRUTH.iter().find(|(n, s, e, _)| *n == name && *s <= t && t < *e).map(|x| x.3);
        for k in keys.iter().filter(|k| !k.black) {
            let col = active(&k.name).map_or([240, 240, 240], |h| HANDS[h]);
            fill(&mut img, [k.box_[0], k.box_[1], k.box_[2] - 2.0, k.box_[3]], col);
        }
        for k in keys.iter().filter(|k| k.black) {
            let col = active(&k.name).map_or([20, 20, 20], |h| HANDS[h].map(|v| (v as f64 * 0.7) as u8));
            fill(&mut img, k.box_, col);
        }
        stdin.write_all(&img).unwrap();
    }
    drop(stdin);
    assert!(ff.wait().unwrap().success());
    keys
}

fn run(intro: usize) -> Vec<analyze::Note> {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("v.mp4");
    let keys = make_video(&p, intro);
    let info = video::probe(&p).unwrap();
    let f0 = video::frame_at(&p, intro as f64 / FPS as f64, None, &info).unwrap();
    let base = analyze::Sampler::new(&keys, f0.w, f0.h, 0).sample(&f0.rgb);
    let (notes, cents, _) = analyze::run(&p, &info, &keys, &base, 18.0, &mut |_, _| {}, None, false).unwrap();
    assert_eq!(cents.len(), 2, "two hand colours learned");
    notes
}

#[test]
fn recovers_notes_and_hands() {
    let notes = run(0);
    assert_eq!(notes.len(), TRUTH.len(), "{notes:?}");
    let mut by_true_hand: [Vec<i32>; 2] = Default::default();
    for (name, s, e, h) in TRUTH {
        let n = notes.iter().find(|x| x.name == name).unwrap();
        assert!((n.start - s).abs() < 2.0 / FPS as f64 && (n.end - e).abs() < 2.0 / FPS as f64, "{name}: {n:?}");
        by_true_hand[h].push(n.hand);
    }
    // same true hand -> same cluster, different true hands -> different clusters
    assert!(by_true_hand.iter().all(|v| v.iter().all(|&x| x == v[0])));
    assert_ne!(by_true_hand[0][0], by_true_hand[1][0]);
}

#[test]
fn title_card_before_the_keyboard_is_not_music() {
    let intro = 15;
    let notes = run(intro);
    assert_eq!(notes.len(), TRUTH.len(), "{notes:?}");
    assert!(notes.iter().map(|n| n.start).fold(f64::MAX, f64::min) >= intro as f64 / FPS as f64 + 0.45);
}

#[test]
fn sparkles_and_glows_are_dropped_by_hand_colors() {
    let mk = |name: &str, midi: i32, hue: f64, chroma: f64, i: usize| analyze::Note {
        name: name.into(), midi, start: i as f64 * 0.2, end: i as f64 * 0.2 + 0.15, angle: Some(hue), hand: 0, chroma,
    };
    let mut real: Vec<_> = (0..40).map(|i| mk("A2", 45, 128.0 + (i % 3) as f64, 80.0, i)).collect();
    real.extend((0..40).map(|i| mk("A4", 69, 66.0 + (i % 3) as f64, 70.0, 40 + i)));
    let junk = [mk("D#6", 87, 300.0, 16.0, 90), mk("F#4", 66, 254.0, 17.0, 91), mk("C4", 60, 128.0, 9.0, 92)]; // wrong hue / far too pale
    let all: Vec<_> = real.iter().cloned().chain(junk).collect();
    let cents = analyze::cluster_hands(&all.iter().filter_map(|n| n.angle).collect::<Vec<_>>());
    assert_eq!(cents.len(), 2);
    let mut r: Vec<i32> = cents.iter().map(|c| (c / 10.0).round() as i32).collect();
    r.sort();
    assert_eq!(r, [7, 13]);
    assert_eq!(analyze::drop_unlike_hands(all, &cents).len(), real.len());
}

#[test]
fn midi_round_trip_keeps_when_notes_end() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("t.mid");
    let note = |name: &str, midi, s, e, hand| analyze::Note { name: name.into(), midi, start: s, end: e, angle: Some(0.0), hand, chroma: 0.0 };
    midi::write(&[note("C4", 60, 1.0, 1.5, 0), note("E4", 64, 2.0, 2.25, 1)], &out).unwrap();
    let got: Vec<_> = midi::load(&out).unwrap().iter().map(|n| (n.midi, (n.start * 100.0).round() / 100.0, (n.end * 100.0).round() / 100.0, n.hand)).collect();
    assert_eq!(got, [(60, 1.0, 1.5, 0), (64, 2.0, 2.25, 1)]);
}

#[test]
fn lab_conversion_round_trips_and_hand_colors_are_distinct() {
    for rgb in [[0u8, 150, 255], [90, 160, 25], [250, 250, 250], [20, 20, 20]] {
        let back = color::lab_to_rgb(color::rgb_to_lab(rgb[0], rgb[1], rgb[2]));
        assert!(rgb.iter().zip(back).all(|(a, b)| (*a as i32 - b as i32).abs() <= 6), "{rgb:?} -> {back:?}");
    }
}
