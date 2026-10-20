//! Video access through ffmpeg: one-frame grabs, and a stream of a thin horizontal strip (the keyboard) for analysis.
use crate::tools::{self, Res};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub width: usize,
    pub height: usize,
    pub fps: f64,
    pub frames: usize,
    pub duration: f64,
}

#[derive(Clone)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<PStream>,
    format: Option<PFormat>,
}
#[derive(Deserialize)]
struct PStream {
    width: Option<usize>,
    height: Option<usize>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    nb_frames: Option<String>,
    duration: Option<String>,
}
#[derive(Deserialize)]
struct PFormat {
    duration: Option<String>,
}

fn rate(s: &str) -> Option<f64> {
    let (a, b) = s.split_once('/')?;
    let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    (b > 0.0 && a > 0.0).then_some(a / b)
}

pub fn probe(path: &Path) -> Res<VideoInfo> {
    let out = tools::command(tools::ffprobe()?)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=width,height,avg_frame_rate,r_frame_rate,nb_frames,duration:format=duration")
        .args(["-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| format!("could not run ffprobe: {e}"))?;
    let p: Probe = serde_json::from_slice(&out.stdout).map_err(|_| "Could not read that video file".to_string())?;
    let s = p.streams.first().ok_or("That file has no video")?;
    let (w, h) = (s.width.unwrap_or(0), s.height.unwrap_or(0));
    if w == 0 || h == 0 {
        return Err("Could not read that video file".into());
    }
    let fps = s.avg_frame_rate.as_deref().and_then(rate).or(s.r_frame_rate.as_deref().and_then(rate)).unwrap_or(30.0);
    let dur: f64 = s
        .duration
        .as_deref()
        .or(p.format.as_ref().and_then(|f| f.duration.as_deref()))
        .and_then(|d| d.parse().ok())
        .unwrap_or(0.0);
    let frames = s.nb_frames.as_deref().and_then(|n| n.parse().ok()).unwrap_or((dur * fps).round() as usize).max(1);
    Ok(VideoInfo { width: w, height: h, fps, frames, duration: frames as f64 / fps })
}

fn read_exact_or_eof(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut got = 0;
    while got < buf.len() {
        let n = r.read(&mut buf[got..])?;
        if n == 0 {
            return Ok(false);
        }
        got += n;
    }
    Ok(true)
}

fn passthrough_args() -> Vec<&'static str> {
    // ffmpeg 5.1+ calls it -fps_mode; older versions only know -vsync. Both mean "do not duplicate or drop frames".
    use std::sync::OnceLock;
    static NEW: OnceLock<bool> = OnceLock::new();
    let new = *NEW.get_or_init(|| {
        tools::ffmpeg()
            .ok()
            .and_then(|f| tools::command(f).arg("-version").output().ok())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.split_whitespace().nth(2).map(str::to_string))
            .and_then(|v| {
                // a development build ("N-127790-g56dbd4cd37-...", as the ready-made downloads are) is newer than any release
                if v.starts_with("N-") || v.starts_with("git-") {
                    return Some(true);
                }
                let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u32>().ok();
                let mut it = v.trim_start_matches(|c: char| !c.is_ascii_digit()).split('.');
                let major = digits(it.next()?)?;
                let minor = it.next().and_then(digits).unwrap_or(0);
                Some(major > 5 || (major == 5 && minor >= 1))
            })
            .unwrap_or(false)
    });
    if new { vec!["-fps_mode", "passthrough"] } else { vec!["-vsync", "0"] }
}

/// The frame at time `t` (seconds), optionally only the rows `y0..y0+h` (y0 and h even).
pub fn frame_at(path: &Path, t: f64, crop: Option<(usize, usize)>, info: &VideoInfo) -> Res<Frame> {
    let mut cmd = tools::command(tools::ffmpeg()?);
    cmd.args(["-v", "error", "-ss", &format!("{:.3}", t.max(0.0)), "-i"]).arg(path).args(["-an", "-frames:v", "1"]);
    let (y0, h) = crop.unwrap_or((0, info.height));
    if crop.is_some() {
        cmd.args(["-vf", &format!("crop={}:{}:0:{}", info.width, h, y0)]);
    }
    let out = cmd.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]).stdin(Stdio::null()).stderr(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.stdout.len() != info.width * h * 3 {
        return Err("Could not read a frame at that time".into());
    }
    Ok(Frame { w: info.width, h, rgb: out.stdout })
}

/// Decode the whole video and hand every frame's rows `y0..y0+h` to `on_frame` (RGB, full width).
/// Returns the number of frames seen.
pub fn stream_strip(path: &Path, info: &VideoInfo, y0: usize, h: usize, mut on_frame: impl FnMut(usize, &[u8])) -> Res<usize> {
    let mut cmd = tools::command(tools::ffmpeg()?);
    cmd.args(["-v", "error", "-i"]).arg(path).args(["-an", "-sn", "-vf", &format!("crop={}:{}:0:{}", info.width, h, y0)]);
    cmd.args(passthrough_args()).args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("could not run ffmpeg: {e}"))?;
    let mut out = child.stdout.take().unwrap();
    let mut buf = vec![0u8; info.width * h * 3];
    let mut i = 0;
    while read_exact_or_eof(&mut out, &mut buf).map_err(|e| e.to_string())? {
        on_frame(i, &buf);
        i += 1;
    }
    let _ = child.wait();
    if i == 0 {
        return Err("Could not read any frames from the video".into());
    }
    Ok(i)
}
