//! YouTube download through yt-dlp. The program is fetched on first use and refreshed when a download fails,
//! because YouTube changes often and an old copy is the usual reason for trouble.
use crate::tools::{self, Res};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub fn extract_video_id(url: &str) -> Res<String> {
    let url = url.trim();
    let rest = url.split("://").last().unwrap_or(url);
    let (host_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = host_path.split_once('/').unwrap_or((host_path, ""));
    let path = path.split('#').next().unwrap_or("");
    let vid = if host.ends_with("youtu.be") {
        path.trim_matches('/').to_string()
    } else {
        let v = query.split('&').find_map(|kv| kv.strip_prefix("v=")).unwrap_or("");
        if v.is_empty() { path.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string() } else { v.to_string() }
    };
    let vid: String = vid.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    if vid.is_empty() { Err("Could not find a video id in that URL".into()) } else { Ok(vid) }
}

fn needs_cookies(err: &str) -> bool {
    let t = err.to_lowercase();
    ["sign in", "cookies", "confirm you", "not a bot", "age"].iter().any(|k| t.contains(k))
}

/// Last useful line of yt-dlp's complaint, without its prefix.
fn short_error(stderr: &str) -> String {
    let line = stderr.lines().rev().find(|l| l.contains("ERROR")).or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty())).unwrap_or("unknown error");
    line.trim().trim_start_matches("ERROR:").trim().to_string()
}

fn run_once(exe: &Path, url: &str, out_dir: &Path, height: u32, browser: Option<&str>, progress: &mut dyn FnMut(f64, String)) -> Result<(), String> {
    let (rt_name, rt_path) = tools::js_runtime()?;
    let mut cmd = tools::command(exe);
    let fmt = format!(
        "bestvideo[height<={h}][vcodec^=avc1]/bestvideo[height<={h}][ext=mp4]/bestvideo[height<={h}]/best[height<={h}]/best",
        h = height
    );
    cmd.args(["-f", &fmt, "--merge-output-format", "mp4", "--no-playlist", "--no-warnings", "--newline", "--force-overwrites", "--write-info-json"])
        .args(["--progress-template", "download:PROGRESS %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s"])
        .args(["--js-runtimes", &format!("{rt_name}:{}", rt_path.display()), "--remote-components", "ejs:github"])
        .arg("-o")
        .arg(out_dir.join("video.%(ext)s"));
    if let Ok(ff) = tools::ffmpeg() {
        if let Some(dir) = ff.parent() {
            cmd.arg("--ffmpeg-location").arg(dir);
        }
    }
    if let Some(b) = browser {
        cmd.args(["--cookies-from-browser", b]);
    }
    cmd.arg(url).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not start yt-dlp: {e}"))?;
    let stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut s);
        s
    });
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        if let Some(rest) = line.strip_prefix("download:PROGRESS ") {
            let v: Vec<f64> = rest.split_whitespace().map(|x| x.parse().unwrap_or(0.0)).collect();
            let total = if v.get(1).copied().unwrap_or(0.0) > 0.0 { v[1] } else { v.get(2).copied().unwrap_or(0.0) };
            let frac = if total > 0.0 { v.first().copied().unwrap_or(0.0) / total } else { 0.0 };
            progress(frac.min(1.0) * 0.95, format!("Downloading {:.0}%", frac.min(1.0) * 100.0));
        } else if line.contains("[Merger]") || line.contains("[VideoRemuxer]") {
            progress(0.96, "Merging".into());
        }
    }
    let ok = child.wait().map_err(|e| e.to_string())?.success();
    let err = err_thread.join().unwrap_or_default();
    if ok { Ok(()) } else { Err(short_error(&err)) }
}

/// Download the video (no audio needed) into `work/<id>/video.*`; returns the file and the video's title.
pub fn download(url: &str, work: &Path, quality: &str, progress: &mut dyn FnMut(f64, String)) -> Res<(PathBuf, String)> {
    let vid = extract_video_id(url)?;
    let out_dir = work.join(&vid);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let height = match quality { "720p" => 720, "480p" => 480, _ => 1080 };
    let watch = format!("https://www.youtube.com/watch?v={vid}");
    let mut refreshed = false; // true once we hold a freshly downloaded yt-dlp
    let mut exe = match tools::ytdlp() {
        Some(p) => p,
        None => {
            progress(0.0, "Getting the downloader (first time only)".into());
            refreshed = true;
            tools::install_ytdlp()?
        }
    };
    let mut browsers = ["chrome", "firefox"].into_iter();
    let mut browser: Option<&str> = None;
    let mut last = String::new();
    loop {
        match run_once(&exe, &watch, &out_dir, height, browser, progress) {
            Ok(()) => break,
            Err(e) => {
                last = e;
                if needs_cookies(&last) {
                    // YouTube wants a signed-in browser: borrow its cookies
                    match browsers.next() {
                        Some(b) => {
                            progress(0.0, format!("YouTube asked us to sign in: trying your {b} session"));
                            browser = Some(b);
                            continue;
                        }
                        None => break,
                    }
                }
                if !refreshed {
                    // most failures are an out-of-date yt-dlp: fetch the newest one and try again
                    refreshed = true;
                    progress(0.0, "Updating the downloader and trying again".into());
                    match tools::install_ytdlp() {
                        Ok(p) => {
                            exe = p;
                            continue;
                        }
                        Err(_) => break,
                    }
                }
                break;
            }
        }
    }
    let file = std::fs::read_dir(&out_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_stem().is_some_and(|s| s == "video") && p.extension().is_some_and(|e| !["json", "part"].contains(&e.to_str().unwrap_or(""))));
    let Some(file) = file else {
        return Err(format!("Download failed: {last}"));
    };
    // title from the info file yt-dlp wrote; then remove it
    let info = out_dir.join("video.info.json");
    let title = std::fs::read_to_string(&info)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["title"].as_str().map(str::to_string))
        .unwrap_or_else(|| vid.clone());
    let _ = std::fs::remove_file(&info);
    progress(1.0, "Done".into());
    Ok((file, title))
}
