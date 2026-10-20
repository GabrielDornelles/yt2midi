use yt2midi_core::midi::SheetNote as Note;
use yt2midi_core::sheet::*;

fn n(midi: i32, start: f64, hand: i32) -> Note {
    Note { midi, start, hand, end: start }
}

/// A simple oom-pah-pah piece: bass on beat 1, chord on 2 and 3, melody note on every beat.
fn synth(measures: usize) -> Vec<Note> {
    let (beat, beats) = (0.5, 3);
    let mut out = vec![];
    for m in 0..measures {
        let t0 = 1.0 + m as f64 * beat * beats as f64;
        out.push(n(45, t0, 0));
        for b in 0..beats {
            out.push(n(69 + (b as i32 % 2) * 2, t0 + b as f64 * beat, 1));
            if b > 0 {
                out.push(n(57, t0 + b as f64 * beat, 0));
            }
        }
    }
    out.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    out
}

fn base() -> Params {
    let mut p = Params::default();
    p.beat_period = Some(0.5);
    p.t_downbeat = Some(1.0);
    p.t_start = Some(1.0);
    p
}

fn print_params() -> Params {
    let mut p = base();
    p.print_mode = true;
    p.systems_per_page = 5;
    p.first_page_rows = 5;
    p.measures_per_row = 4;
    p
}

#[test]
fn kindle_pagination() {
    let p = Params::default();
    assert_eq!(measure_range(&p, 0), (0, 1, 4));
    assert_eq!(measure_range(&p, 3), (12, 1, 4));
    assert_eq!(pages_needed(&p, 13), 4);
}

#[test]
fn print_pagination_is_5_rows_on_every_page() {
    let mut p = Params::default();
    p.print_mode = true;
    p.systems_per_page = 5;
    p.first_page_rows = 5;
    p.measures_per_row = 4;
    assert_eq!(measure_range(&p, 0), (0, 5, 4));
    assert_eq!(measure_range(&p, 1), (20, 5, 4));
    assert_eq!(pages_needed(&p, 20), 1);
    assert_eq!(pages_needed(&p, 21), 2);
    assert_eq!(pages_needed(&p, 41), 3);
}

#[test]
fn print_pages_are_a4_and_never_clip() {
    let notes = synth(60);
    for i in 0..2 {
        let mut p = print_params();
        p.title = "A song with a rather long name to check that the title wraps neatly across lines".into();
        let img = render_ss(&notes, &mut p, i, 1);
        assert_eq!((img.w as u32, img.h as u32), A4_SIZE);
        assert!((0.2..=1.0).contains(&p.used_scale), "{}", p.used_scale);
    }
}

#[test]
fn rolled_chord_groups_only_when_asked() {
    let notes = [n(64, 1.00, 0), n(73, 1.05, 0), n(57, 1.06, 0)];
    assert_eq!(cluster(&notes, false).iter().map(|c| c.1.len()).collect::<Vec<_>>(), [1, 2]);
    let loose = cluster(&notes, true);
    assert_eq!(loose.len(), 1);
    assert_eq!(loose[0].1.len(), 3);
    assert!((loose[0].0 - 1.0).abs() < 1e-9, "sits at the first note");
}

#[test]
fn fast_run_is_not_merged_and_separate_chords_stay_separate() {
    let run: Vec<Note> = (0..8).map(|i| n(60 + i, 1.0 + i as f64 * 0.05, 0)).collect();
    assert_eq!(cluster(&run, true).len(), 8);
    let two = [n(60, 1.0, 0), n(64, 1.04, 0), n(62, 1.5, 0), n(65, 1.54, 0)];
    assert_eq!(cluster(&two, true).iter().map(|c| c.1.len()).collect::<Vec<_>>(), [2, 2]);
}

#[test]
fn group_chords_changes_the_page_not_the_notes() {
    let notes = synth(60);
    // roll every melody note: the right hand arrives 50 ms after the left-hand chord
    let rolled: Vec<Note> = notes.iter().map(|x| Note { start: x.start + if x.hand == 1 { 0.05 } else { 0.0 }, end: x.start, ..x.clone() }).collect();
    let a = render_ss(&rolled, &mut base(), 0, 1);
    let mut p = base();
    p.group_chords = true;
    let b = render_ss(&rolled, &mut p, 0, 1);
    assert_eq!((a.w, a.h), (b.w, b.h));
    assert!(a.gray != b.gray);
}

fn boxes(rot: Rotate) -> (Rendered, Vec<(f64, f64, f64, f64, f64, f64)>) {
    let mut p = base();
    p.rotate = rot;
    let img = render_ss(&synth(60), &mut p, 1, 1);
    (img, p.boxes.clone())
}

#[test]
fn every_measure_gets_a_box_with_its_time_and_boxes_follow_a_turned_page() {
    let (img, plain) = boxes(Rotate::None);
    assert_eq!(plain.len(), 4);
    for (i, b) in plain.iter().enumerate() {
        assert!((b.0 - (1.0 + (4 + i) as f64 * 1.5)).abs() < 1e-6, "page 2 = measures 5-8");
        assert!((b.1 - b.0 - 1.5).abs() < 1e-6);
        assert!(0.0 <= b.2 && b.2 < b.4 && b.4 <= img.w as f64 && 0.0 <= b.3 && b.3 < b.5 && b.5 <= img.h as f64);
    }
    let (w, h) = (1680.0, 1264.0);
    let (img_cw, cw) = boxes(Rotate::Cw);
    assert_eq!((img_cw.w, img_cw.h), (1264, 1680));
    for (a, b) in plain.iter().zip(&cw) {
        assert_eq!((b.2, b.3, b.4, b.5), (h - a.5, a.2, h - a.3, a.4)); // (x, y) -> (H - y, x)
    }
    let (_, ccw) = boxes(Rotate::Ccw);
    for (a, b) in plain.iter().zip(&ccw) {
        assert_eq!((b.2, b.3, b.4, b.5), (a.3, w - a.4, a.5, w - a.2)); // (x, y) -> (y, W - x)
    }
}

#[test]
fn every_drawn_note_reports_where_and_when_it_sounds() {
    let mut notes = synth(12);
    notes.iter_mut().for_each(|x| x.end = x.start + 0.4);
    let mut p = base();
    let img = render_ss(&notes, &mut p, 0, 1);
    let on_page = notes.iter().filter(|x| 1.0 <= x.start && x.start < 1.0 + 4.0 * 1.5 - 1e-6).count();
    assert_eq!(p.dots.len(), on_page);
    assert!(p.dot_r > 5.0);
    assert!(p.dots.iter().all(|d| 0.0 <= d.2 && d.2 <= img.w as f64 && 0.0 <= d.3 && d.3 <= img.h as f64 && d.1 > d.0));
}

fn dense(measures: usize, per: usize) -> Vec<Note> {
    let step = 1.5 / per as f64;
    (0..measures * per).map(|i| Note { midi: 40 + ((i * 7) % 44) as i32, start: 1.0 + i as f64 * step, hand: (i % 2) as i32, end: 1.0 + i as f64 * step + 0.2 }).collect()
}

#[test]
fn overcrowded_page_shrinks_to_fit_and_never_leaves_the_page() {
    let notes = dense(30, 48);
    let mut p = print_params();
    p.measures_per_row = 5;
    p.dot = 90.0;
    p.line_space = 2.2;
    let img = render_ss(&notes, &mut p, 0, 1);
    assert!(p.used_scale < 0.2, "{}", p.used_scale);
    assert!(!p.dots.is_empty() && p.dots.iter().all(|d| 0.0 <= d.2 && d.2 <= img.w as f64 && 0.0 <= d.3 && d.3 <= img.h as f64));
    assert!(p.boxes.iter().all(|b| b.2 >= 0.0 && b.4 <= img.w as f64 && b.3 >= 0.0 && b.5 <= img.h as f64));
    assert_eq!(p.boxes.len(), p.plan.as_ref().unwrap().pages[0].iter().map(|r| r.len()).sum::<usize>());
}

#[test]
fn print_pages_share_one_note_size_and_hold_several_rows() {
    let notes = dense(30, 18);
    let mut p = print_params();
    p.dot = 90.0;
    resolve(&notes, &mut p);
    let plan = p.plan.clone().unwrap();
    assert!(plan.pages[..plan.pages.len() - 1].iter().all(|pg| pg.len() >= 5));
    let mut used = std::collections::HashSet::new();
    for i in 0..plan.pages.len() {
        render_ss(&notes, &mut p, i, 1);
        used.insert((p.used_scale * 1e4).round() as i64);
    }
    assert_eq!(used.len(), 1, "identical size on every page");
}

#[test]
fn hold_arcs_and_hand_links_are_drawn_and_optional() {
    let notes = [Note { midi: 60, start: 1.0, hand: 0, end: 3.0 }, Note { midi: 40, start: 1.0, hand: 1, end: 1.4 }, Note { midi: 64, start: 3.0, hand: 0, end: 3.3 }];
    let mk = |holds: bool, link: bool| {
        let mut p = base();
        p.title = "t".into();
        p.show_holds = holds;
        p.show_link = link;
        let r = render_ss(&notes, &mut p, 0, 1);
        (r.gray, p.dots.len())
    };
    let (on, _) = mk(true, true);
    let (off, _) = mk(false, false);
    assert!(on != off, "something extra is drawn");
    let (h_on, dots) = mk(true, false);
    let (h_off, _) = mk(false, false);
    assert!(h_on != h_off);
    assert_eq!(dots, 3, "the release dot is not a played note");
}

#[test]
fn no_two_dots_overlap_even_with_close_chords_in_a_row() {
    let mut notes = vec![];
    for i in 0..24 {
        // a chord of neighbouring notes every 60 ms, then a gap
        let t = 1.0 + (i / 3) as f64 * 0.6 + (i % 3) as f64 * 0.06;
        for m in [61, 62, 64, 66] {
            notes.push(Note { midi: m + (i as i32 % 2), start: t, hand: 0, end: t + 0.1 });
        }
        notes.push(Note { midi: 45 + (i as i32 % 3), start: t, hand: 1, end: t + 0.1 });
    }
    notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let mut p = base();
    p.dot = 56.0;
    render_ss(&notes, &mut p, 0, 1);
    assert!(p.dots.len() > 20);
    let mut min = f64::MAX;
    for (i, a) in p.dots.iter().enumerate() {
        for b in &p.dots[i + 1..] {
            min = min.min(((a.2 - b.2).powi(2) + (a.3 - b.3).powi(2)).sqrt());
        }
    }
    assert!(min >= 1.7 * p.dot_r, "closest dots {min:.1} apart, radius {}", p.dot_r);
}
