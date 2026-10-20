use yt2midi_core::{midi, score, sheet};

/// score_cli <song.mid> <out.musicxml> [beats | auto]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(3).map(String::as_str).unwrap_or("auto");
    let notes = midi::load(std::path::Path::new(&a[1])).unwrap();
    let mut p = sheet::Params::default();
    p.beats_per_measure = 4;
    sheet::resolve(&notes, &mut p);
    let base = score::Grid { t_downbeat: p.t_downbeat.unwrap(), beat: p.beat_period.unwrap(), beats: 4 };
    let t = std::time::Instant::now();
    let g = if mode == "auto" { score::auto_grid(&notes, &base, None, 2.0) } else { score::auto_grid(&notes, &base, Some(mode.parse().unwrap()), 2.0) };
    let found = t.elapsed().as_secs_f64();
    let meta = score::Meta { title: "Test".into(), subtitle: None, composer: String::new(), arranger: String::new(), hold: 2.0, key: None };
    let s = score::build_with(&notes, &g, &meta);
    eprintln!("{} notes -> {} measures in {}/4 at {:.1} bpm (base guess {:.1}), key {} {}; search {:.0} ms", notes.len(), s.measures, g.beats, 60.0 / g.beat, 60.0 / base.beat, s.fifths, if s.minor { "minor" } else { "major" }, found * 1000.0);
    std::fs::write(&a[2], s.xml).unwrap();
}
