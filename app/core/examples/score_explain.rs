use yt2midi_core::{midi, score, sheet};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let notes = midi::load(std::path::Path::new(&a[1])).unwrap();
    let mut p = sheet::Params::default();
    p.beats_per_measure = 4;
    sheet::resolve(&notes, &mut p);
    let base = score::Grid { t_downbeat: p.t_downbeat.unwrap(), beat: p.beat_period.unwrap(), beats: 4 };
    let (pp, pr) = score::strongest_pulse(&notes);
    println!("strongest pulse: beat {:.4} s ({:.1} bpm) pull {:.2}", pp, 60.0 / pp, pr);
    let mut c = score::candidates(&notes, &[pp, pp * 2.0, pp * 4.0], &[4, 3, 2], 2.0);
    c.sort_by(|x, y| x.cost.partial_cmp(&y.cost).unwrap());
    println!("{} candidates; base {:.1} bpm; pull at base {:.2}, at x2 {:.2}, x0.5 {:.2}, x1.5 {:.2}", c.len(), 60.0 / base.beat, score::beat_pull(&notes, base.beat), score::beat_pull(&notes, base.beat * 2.0), score::beat_pull(&notes, base.beat * 0.5), score::beat_pull(&notes, base.beat * 1.5));
    // the best reading at each tempo
    let mut seen: Vec<i64> = vec![];
    for x in c.iter() {
        let k = (60.0 / x.grid.beat * 2.0).round() as i64;
        if seen.contains(&k) { continue; }
        seen.push(k);
        println!("{:8.1}  {:?}  {}/4 at {:6.1} bpm (x{:.2} of the base)  td {:.2}", x.cost, x.parts.map(|v| v.round()), x.grid.beats, 60.0 / x.grid.beat, base.beat / x.grid.beat, x.grid.t_downbeat);
    }
    for x in c.iter().take(0) {
        println!("cost {:8.1}  {:?}  {}/4 at {:6.1} bpm  td {:.2}", x.cost, x.parts.map(|v| v.round()), x.grid.beats, 60.0 / x.grid.beat, x.grid.t_downbeat);
    }
    for (td, p, b) in [(2.811, 60.0 / 104.64, 3), (2.811, 60.0 / 104.64, 4), (2.811, 60.0 / 104.0, 3), (2.811, 60.0 / 105.2, 3)] {
        let x = score::evaluate(&notes, &score::Grid { t_downbeat: td, beat: p, beats: b }, 2.0);
        println!("by hand td {td} beat {p} {b}/4: cost {:.1} notes/ties/rests/short/trip/voice2/err = {:?}", x.cost, x.parts.map(|v| v.round()));
    }
    // the reading with the beat of 0.75 s in 4/4 for comparison
    for x in c.iter().filter(|x| (x.grid.beat - 0.75).abs() < 0.02 && x.grid.beats == 4).take(2) {
        println!("[0.75 s, 4/4] cost {:8.1}  err {:6.2}  notes {:5}  td {:.2}", x.cost, x.err, x.items, x.grid.t_downbeat);
    }
}
