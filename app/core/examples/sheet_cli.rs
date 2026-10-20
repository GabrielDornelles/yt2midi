use std::time::Instant;
use yt2midi_core::{export, midi, sheet};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let has = |k: &str| a.iter().any(|x| x == k);
    let notes = midi::load(std::path::Path::new(&a[1])).unwrap();
    let out = get("--out").unwrap();
    let page: usize = get("--page").map_or(0, |v| v.parse().unwrap());
    let mut p = sheet::Params::default();
    p.title = get("--title").unwrap_or_default();
    if has("--print") {
        p.print_mode = true;
        p.systems_per_page = 5;
        p.first_page_rows = 5;
        p.measures_per_row = get("--mpr").map_or(4, |v| v.parse().unwrap());
    } else {
        p.size = (1680, 1264);
        p.rotate = sheet::Rotate::Cw;
        p.systems_per_page = get("--spp").map_or(1, |v| v.parse().unwrap());
        p.measures_per_page = get("--mpp").map_or(4, |v| v.parse().unwrap());
    }
    p.group_chords = has("--chords");
    p.show_holds = !has("--noholds");
    p.show_link = !has("--nolink");
    let t = Instant::now();
    sheet::resolve(&notes, &mut p);
    let r = sheet::render(&notes, &mut p, page);
    eprintln!("page {page}: {}x{} in {:.2}s  bpm {:.2} t_down {:.3} scale {:.3} pages {}", r.w, r.h, t.elapsed().as_secs_f32(),
        60.0 / p.beat_period.unwrap(), p.t_downbeat.unwrap(), p.used_scale, p.plan.as_ref().map_or(0, |x| x.pages.len()));
    std::fs::write(format!("{out}.png"), export::png_bytes(&r.gray, r.w, r.h, 300)).unwrap();
    let boxes: Vec<_> = p.boxes.iter().map(|b| [b.0, b.1, b.2, b.3, b.4, b.5]).collect();
    let dots: Vec<_> = p.dots.iter().map(|d| [d.0, d.1, d.2, d.3]).collect();
    std::fs::write(format!("{out}.json"), serde_json::json!({"bpm": 60.0 / p.beat_period.unwrap(), "td": p.t_downbeat, "scale": p.used_scale, "boxes": boxes, "dots": dots}).to_string()).unwrap();
}
