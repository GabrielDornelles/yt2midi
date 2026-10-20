//! Tries the on-demand ffmpeg download into the folder YT2MIDI_HOME points at (a temporary one when testing).
use yt2midi_core::tools;

fn main() {
    let t = std::time::Instant::now();
    let mut last = 0;
    let r = tools::install_ffmpeg(&mut |done, total| {
        let mb = done / 1_000_000;
        if mb >= last + 20 {
            last = mb;
            println!("  {mb} of {} MB", total.map_or("?".to_string(), |t| (t / 1_000_000).to_string()));
        }
    });
    println!("{r:?} in {:.0} s", t.elapsed().as_secs_f64());
    for n in ["ffmpeg", "ffprobe"] {
        let p = tools::bin_dir().join(n);
        println!("{}: {} bytes", p.display(), std::fs::metadata(&p).map_or(0, |m| m.len()));
        let out = std::process::Command::new(&p).arg("-version").output();
        println!("  {}", out.map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string()).unwrap_or_else(|e| format!("could not run: {e}")));
    }
}
