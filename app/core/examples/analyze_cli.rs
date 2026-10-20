use std::time::Instant;
use yt2midi_core::{analyze, detect, keyboard, midi, video};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = std::path::PathBuf::from(&a[1]);
    let out = std::path::PathBuf::from(&a[2]);
    std::fs::create_dir_all(&out).unwrap();
    let t = Instant::now();
    let info = video::probe(&path).unwrap();
    println!("{info:?}");
    let auto = detect::autodetect(&path, &info).unwrap().expect("no keyboard");
    println!("{:?} times {:?}  ({:.1}s)", auto.det, auto.times, t.elapsed().as_secs_f32());
    let d = &auto.det;
    let keys = keyboard::build_keys(d.x0, d.x1, d.y0, d.y1, &d.first_note, d.n_white).unwrap();
    let med = detect::full_median(&path, &info).unwrap();
    let (y0, _) = analyze::Sampler::strip(&keys, info.width, info.height);
    let _ = y0;
    let base = analyze::Sampler::new(&keys, info.width, info.height, 0).sample(&med.rgb);
    let t = Instant::now();
    let (notes, cents, fps) = analyze::run(&path, &info, &keys, &base, 40.0, &mut |p, s| { if (p * 100.0) as i32 % 20 == 0 { eprintln!("{p:.2} {s}") } }, None, false).unwrap();
    println!("{} notes, hands {:?}, fps {} in {:.1}s", notes.len(), cents, fps, t.elapsed().as_secs_f32());
    midi::write(&notes, &out.join("result.mid")).unwrap();
}
