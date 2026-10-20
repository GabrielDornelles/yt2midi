//! The application core behind the HTTP API: songs on disk, the video -> MIDI pipeline, jobs, and sheet rendering.
//! Every function here is independent of the web layer so it can be tested and reused.
use crate::analyze::{self, Note, Sampler};
use crate::color::{self, hue_angle};
use crate::keyboard::{build_keys, Key};
use crate::video::{self, Frame, VideoInfo};
use crate::{detect, downloader, export, midi, sheet, tools};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const VIDEO_EXTS: [&str; 11] = ["mp4", "mkv", "webm", "mov", "avi", "m4v", "mpg", "mpeg", "ts", "wmv", "flv"];

fn default_note() -> String { "C4".into() }
fn default_white() -> usize { 28 }
fn default_thr() -> f64 { 40.0 }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Calib {
    pub video_id: String,
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
    #[serde(default = "default_note")]
    pub first_note: String,
    #[serde(default = "default_white")]
    pub n_white: usize,
    #[serde(default)]
    pub baseline_t: Option<f64>,
    #[serde(default = "default_thr")]
    pub threshold: f64,
    #[serde(default)]
    pub hand_t: Option<f64>,
    #[serde(default)]
    pub swap_hands: bool,
}

#[derive(Clone, Debug)]
pub struct ApiError(pub u16, pub String);
pub type ApiRes<T> = Result<T, ApiError>;
impl From<String> for ApiError {
    fn from(s: String) -> Self { ApiError(500, s) }
}
fn bad(msg: impl Into<String>) -> ApiError { ApiError(400, msg.into()) }
fn not_found(msg: impl Into<String>) -> ApiError { ApiError(404, msg.into()) }

#[derive(Serialize, Clone, Default)]
pub struct Job {
    pub state: &'static str,
    pub progress: f64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

pub struct JobFail {
    pub msg: String,
    pub code: Option<String>,
}
impl From<String> for JobFail {
    fn from(msg: String) -> Self { JobFail { msg, code: None } }
}

pub struct State {
    pub work: PathBuf,
    jobs: Mutex<HashMap<String, Job>>,
    frames: Mutex<Vec<((String, i64), Arc<Frame>)>>,
    medians: Mutex<HashMap<String, Arc<Frame>>>,
    infos: Mutex<HashMap<String, VideoInfo>>,
    /// the desktop app plays the piano itself (see audio.rs); a browser uses its own
    pub native_audio: std::sync::atomic::AtomicBool,
}

/// Handle a running job uses to report where it is.
/// ffmpeg is what reads the video. Systems that do not have it (a fresh Windows, for one) get it fetched here, once.
fn ensure_ffmpeg(ctl: &JobCtl) -> Result<(), JobFail> {
    if tools::have_ffmpeg() {
        return Ok(());
    }
    ctl.stage("download", 0.0, "Getting ffmpeg, one time only");
    tools::install_ffmpeg(&mut |done, total| {
        let mb = done as f64 / 1e6;
        let status = match total {
            Some(t) => format!("Getting ffmpeg, one time only: {mb:.0} of {:.0} MB", t as f64 / 1e6),
            None => format!("Getting ffmpeg, one time only: {mb:.0} MB"),
        };
        ctl.stage("download", 0.4 * total.map_or(0.0, |t| (done as f64 / t as f64).min(1.0)), status);
    })
    .map_err(|msg| JobFail { msg, code: None })
}

#[derive(Clone)]
pub struct JobCtl {
    st: Arc<State>,
    id: String,
}
impl JobCtl {
    pub fn progress(&self, p: f64, status: impl Into<String>) {
        if let Some(j) = self.st.jobs.lock().unwrap().get_mut(&self.id) {
            j.progress = p;
            j.status = status.into();
        }
    }
    pub fn stage(&self, name: &str, p: f64, status: impl Into<String>) {
        if let Some(j) = self.st.jobs.lock().unwrap().get_mut(&self.id) {
            j.stage = Some(name.into());
            j.progress = p;
            j.status = status.into();
        }
    }
    fn video_id(&self, v: &str) {
        if let Some(j) = self.st.jobs.lock().unwrap().get_mut(&self.id) {
            j.video_id = Some(v.into());
        }
    }
}

fn r(v: f64, d: i32) -> f64 {
    let m = 10f64.powi(d);
    (v * m).round_ties_even() / m
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn hand_colors(hues: &[f64]) -> Vec<[u8; 3]> {
    hues.iter().map(|a| color::lab_to_rgb([150.0, (128.0 + 70.0 * a.to_radians().cos()) as f32, (128.0 + 70.0 * a.to_radians().sin()) as f32])).collect()
}

impl State {
    pub fn new(work: PathBuf) -> Arc<State> {
        let _ = std::fs::create_dir_all(&work);
        Arc::new(State { work, jobs: Default::default(), frames: Default::default(), medians: Default::default(), infos: Default::default(), native_audio: Default::default() })
    }

    // ------------------------------------------------------------ jobs
    pub fn job(&self, id: &str) -> Option<Job> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    pub fn run_job(self: &Arc<Self>, f: impl FnOnce(&JobCtl) -> Result<Value, JobFail> + Send + 'static) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        self.jobs.lock().unwrap().insert(id.clone(), Job { state: "running", status: "Starting".into(), ..Default::default() });
        let ctl = JobCtl { st: self.clone(), id: id.clone() };
        std::thread::spawn(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&ctl)));
            let mut jobs = ctl.st.jobs.lock().unwrap();
            if let Some(j) = jobs.get_mut(&ctl.id) {
                match res {
                    Ok(Ok(v)) => { j.state = "done"; j.result = Some(v); }
                    Ok(Err(e)) => { log::warn!("job failed: {}", e.msg); j.state = "error"; j.error = Some(e.msg); j.code = e.code; }
                    Err(_) => { j.state = "error"; j.error = Some("Something went wrong inside the app".into()); }
                }
            }
        });
        id
    }

    // ------------------------------------------------------------ files
    pub fn song_dir(&self, vid: &str) -> ApiRes<PathBuf> {
        let d = self.work.join(vid);
        if !valid_id(vid) || !d.is_dir() { return Err(not_found("Unknown song")); }
        Ok(d)
    }

    pub fn video_path(&self, vid: &str) -> ApiRes<PathBuf> {
        if valid_id(vid) {
            if let Ok(rd) = std::fs::read_dir(self.work.join(vid)) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.file_stem().is_some_and(|s| s == "video") && p.extension().and_then(|x| x.to_str()).is_some_and(|x| VIDEO_EXTS.contains(&x.to_lowercase().as_str())) {
                        return Ok(p);
                    }
                }
            }
        }
        Err(not_found("Video not downloaded"))
    }

    pub fn info(&self, vid: &str) -> ApiRes<VideoInfo> {
        if let Some(i) = self.infos.lock().unwrap().get(vid) { return Ok(i.clone()); }
        let i = video::probe(&self.video_path(vid)?)?;
        self.infos.lock().unwrap().insert(vid.to_string(), i.clone());
        Ok(i)
    }

    /// The frame at time t. A small cache makes scrubbing and re-previewing instant.
    pub fn read_frame(&self, vid: &str, t: f64) -> ApiRes<Arc<Frame>> {
        let key = (vid.to_string(), (t.max(0.0) * 100.0).round_ties_even() as i64);
        {
            let mut c = self.frames.lock().unwrap();
            if let Some(i) = c.iter().position(|(k, _)| *k == key) {
                let e = c.remove(i);
                let f = e.1.clone();
                c.push(e);
                return Ok(f);
            }
        }
        let info = self.info(vid)?;
        let f = Arc::new(video::frame_at(&self.video_path(vid)?, key.1 as f64 / 100.0, None, &info).map_err(|_| bad("Could not read frame at that time"))?);
        let mut c = self.frames.lock().unwrap();
        c.push((key, f.clone()));
        if c.len() > 16 { c.remove(0); }
        Ok(f)
    }

    /// Resting keyboard: per-pixel median over frames spread across the video (cached).
    pub fn median_frame(&self, vid: &str) -> ApiRes<Arc<Frame>> {
        if let Some(m) = self.medians.lock().unwrap().get(vid) { return Ok(m.clone()); }
        let m = Arc::new(detect::full_median(&self.video_path(vid)?, &self.info(vid)?)?);
        self.medians.lock().unwrap().insert(vid.to_string(), m.clone());
        Ok(m)
    }

    pub fn forget(&self, vid: &str) {
        self.medians.lock().unwrap().remove(vid);
        self.infos.lock().unwrap().remove(vid);
        self.frames.lock().unwrap().retain(|(k, _)| k.0 != vid);
    }

    /// The notes of a song as the player needs them.
    pub fn song_notes(&self, vid: &str) -> ApiRes<Vec<crate::audio::NoteEv>> {
        let d = self.song_dir(vid)?;
        let v = read_json(&d.join("result.json")).and_then(|v| v.as_array().cloned()).ok_or_else(|| not_found("No notes yet"))?;
        Ok(v.iter().map(|n| crate::audio::NoteEv { midi: n["midi"].as_i64().unwrap_or(60) as i32, start: n["start"].as_f64().unwrap_or(0.0), end: n["end"].as_f64().unwrap_or(0.0), hand: n["hand"].as_i64().unwrap_or(0) as i32 }).collect())
    }

    pub fn title(&self, vid: &str) -> String {
        read_json(&self.work.join(vid).join("meta.json")).and_then(|m| m["title"].as_str().map(str::to_string)).unwrap_or_else(|| vid.to_string())
    }

    pub fn midis(&self, vid: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.work.join(vid)).into_iter().flatten().flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string)).filter(|n| n.ends_with(".mid")).collect();
        v.sort_by(|a, b| (a != "result.mid", a).cmp(&(b != "result.mid", b)));
        v
    }

    pub fn is_ready(&self, vid: &str) -> bool {
        self.work.join(vid).join("result.json").exists() && !self.midis(vid).is_empty()
    }

    // ------------------------------------------------------------ analysis
    pub fn keys_for(&self, c: &Calib) -> ApiRes<Vec<Key>> {
        if c.x1 - c.x0 < 10.0 || c.y1 - c.y0 < 5.0 { return Err(bad("Draw the keyboard box first")); }
        build_keys(c.x0, c.x1, c.y0, c.y1, &c.first_note, c.n_white).ok_or_else(|| bad(format!("Bad first note: {}", c.first_note)))
    }

    fn sample_full(&self, f: &Frame, keys: &[Key]) -> Vec<color::Lab> {
        Sampler::new(keys, f.w, f.h, 0).sample(&f.rgb)
    }

    pub fn baseline_for(&self, c: &Calib, keys: &[Key]) -> ApiRes<Vec<color::Lab>> {
        let f = match c.baseline_t { Some(t) => self.read_frame(&c.video_id, t)?, None => self.median_frame(&c.video_id)? };
        Ok(self.sample_full(&f, keys))
    }

    /// Run the full analysis for a calibration, write MIDI + song metadata, return the result payload.
    pub fn analyze_and_save(&self, c: &Calib, progress: &mut dyn FnMut(f64, String), extra: Option<Value>) -> ApiRes<Value> {
        let keys = self.keys_for(c)?;
        let base = self.baseline_for(c, &keys)?;
        let vp = self.video_path(&c.video_id)?;
        let info = self.info(&c.video_id)?;
        let dir = self.work.join(&c.video_id);
        std::fs::write(dir.join("calib.json"), serde_json::to_string(c).unwrap()).map_err(|e| e.to_string())?;
        let mut centroids = None;
        if let Some(t) = c.hand_t {
            let fr = self.read_frame(&c.video_id, t)?;
            let lab = self.sample_full(&fr, &keys);
            let m = analyze::pressed_mask(&lab, &base, c.threshold);
            let hues: Vec<f64> = lab.iter().zip(&m).filter(|(_, &p)| p).map(|(l, _)| hue_angle(l)).collect();
            let cl = analyze::cluster_hands(&hues);
            centroids = if cl.is_empty() { None } else { Some(cl) };
        }
        let (notes, cents, _) = analyze::run(&vp, &info, &keys, &base, c.threshold, progress, centroids, c.swap_hands)?;
        midi::write(&notes, &dir.join("result.mid"))?;
        let meta = self.song_meta(&c.video_id, true, Some(&cents), Some(c), extra);
        Ok(json!({"video_id": c.video_id, "notes": notes.len(), "hand_colors": meta["hand_colors"], "song": meta}))
    }

    pub fn song_meta(&self, vid: &str, rebuild: bool, hand_hues: Option<&[f64]>, calib: Option<&Calib>, extra: Option<Value>) -> Value {
        let d = self.work.join(vid);
        let f = d.join("song.json");
        let old = read_json(&f).unwrap_or_else(|| json!({}));
        if f.exists() && !rebuild && old.is_object() { return old; }
        let notes = read_json(&d.join("result.json")).and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let hues: Vec<f64> = match hand_hues {
            Some(h) => h.to_vec(),
            None => analyze::cluster_hands(&notes.iter().filter_map(|n| n["hue"].as_f64()).collect::<Vec<_>>()),
        };
        let cal = calib.map(|c| serde_json::to_value(c).unwrap()).or_else(|| read_json(&d.join("calib.json")));
        let mut meta = old.as_object().cloned().unwrap_or_default();
        if let Some(Value::Object(e)) = extra { meta.extend(e); }
        let dur = notes.iter().filter_map(|n| n["end"].as_f64()).fold(0.0, f64::max);
        meta.extend([
            ("video_id".to_string(), json!(vid)), ("title".into(), json!(self.title(vid))), ("notes".into(), json!(notes.len())),
            ("duration".into(), json!(r(dur, 1))), ("hand_hues".into(), json!(hues)), ("hand_colors".into(), json!(hand_colors(&hues))),
            ("first_note".into(), cal.as_ref().map_or(Value::Null, |c| c["first_note"].clone())),
            ("n_white".into(), cal.as_ref().map_or(Value::Null, |c| c["n_white"].clone())),
            ("updated".into(), json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()))),
        ]);
        let v = Value::Object(meta);
        let _ = std::fs::write(&f, v.to_string());
        v
    }

    pub fn make_thumb(&self, vid: &str) -> ApiRes<PathBuf> {
        let d = self.song_dir(vid)?;
        let out = d.join("thumb.jpg");
        if out.exists() { return Ok(out); }
        let t = read_json(&d.join("song.json")).and_then(|m| m["thumb_t"].as_f64()).unwrap_or_else(|| self.info(vid).map_or(0.0, |i| i.duration * 0.3));
        let f = self.read_frame(vid, t)?;
        let img = image::RgbImage::from_raw(f.w as u32, f.h as u32, f.rgb.clone()).ok_or_else(|| "bad frame".to_string())?;
        let nh = (640.0 * f.h as f64 / f.w as f64) as u32;
        let small = image::imageops::resize(&img, 640, nh, image::imageops::FilterType::Triangle);
        let file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 80).encode_image(&small).map_err(|e| e.to_string())?;
        Ok(out)
    }

    pub fn jpeg(&self, vid: &str, t: f64) -> ApiRes<Vec<u8>> {
        let f = self.read_frame(vid, t)?;
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
            .encode(&f.rgb, f.w as u32, f.h as u32, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
        Ok(out)
    }

    // ------------------------------------------------------------ pipeline (URL or file in, MIDI out)
    pub fn process_video(self: &Arc<Self>, vid: String, ctl: &JobCtl) -> Result<Value, JobFail> {
        ctl.video_id(&vid);
        ctl.stage("detect", 0.40, "Looking for the keyboard");
        let path = self.video_path(&vid).map_err(|e| e.1)?;
        let info = self.info(&vid).map_err(|e| e.1)?;
        let Some(auto) = detect::autodetect(&path, &info)? else {
            return Err(JobFail { msg: "We couldn't find a piano keyboard in this video.".into(), code: Some("no_keyboard".into()) });
        };
        self.medians.lock().unwrap().insert(vid.clone(), Arc::new(auto.median.clone()));
        let d = &auto.det;
        let c = Calib { video_id: vid.clone(), x0: d.x0, x1: d.x1, y0: d.y0, y1: d.y1, first_note: d.first_note.clone(), n_white: d.n_white,
            baseline_t: None, threshold: 40.0, hand_t: None, swap_hands: false };
        let extra = json!({"auto": true, "letter_certain": d.letter_certain,
            "octave_certain": d.n_white >= 51 && d.first_letter == "A", "thumb_t": auto.times.first()});
        ctl.stage("read", 0.50, "Reading the notes");
        let c2 = ctl.clone();
        let result = self.analyze_and_save(&c, &mut |p, st| c2.stage("read", 0.50 + 0.45 * p, st), Some(extra)).map_err(|e| e.1)?;
        ctl.stage("finish", 0.97, "Saving");
        self.make_thumb(&vid).map_err(|e| e.1)?;
        ctl.stage("finish", 1.0, "Done");
        Ok(result)
    }

    pub fn start_url(self: &Arc<Self>, url: String, quality: String) -> ApiRes<String> {
        let vid = downloader::extract_video_id(&url).map_err(bad)?;
        let st = self.clone();
        Ok(self.run_job(move |ctl| {
            ensure_ffmpeg(ctl)?;
            ctl.stage("download", 0.0, "Starting download");
            let c2 = ctl.clone();
            let (_, title) = downloader::download(&url, &st.work, &quality, &mut |p, s| c2.stage("download", 0.40 * p, s))?;
            let _ = std::fs::write(st.work.join(&vid).join("meta.json"), json!({"title": title}).to_string());
            st.process_video(vid, ctl)
        }))
    }

    /// Save an uploaded video (already written to `tmp`) as a new song and process it.
    pub fn start_file(self: &Arc<Self>, tmp: PathBuf, file_name: &str) -> ApiRes<String> {
        let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("video").to_string();
        let ext = Path::new(file_name).extension().and_then(|s| s.to_str()).map(str::to_lowercase).filter(|e| VIDEO_EXTS.contains(&e.as_str())).unwrap_or_else(|| "mp4".into());
        let slug: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
        let slug: String = slug.chars().take(18).collect();
        let vid = format!("f-{}-{}", if slug.is_empty() { "video" } else { &slug }, &uuid::Uuid::new_v4().simple().to_string()[..5]);
        let dir = self.work.join(&vid);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join(format!("video.{ext}"))).or_else(|_| std::fs::copy(&tmp, dir.join(format!("video.{ext}"))).map(|_| ())).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&tmp);
        let title = stem.replace(['_', '-'], " ").trim().to_string();
        let _ = std::fs::write(dir.join("meta.json"), json!({"title": if title.is_empty() { vid.clone() } else { title }}).to_string());
        // without ffmpeg the file cannot be looked at yet: that check waits until the job has fetched it
        let looked_at = tools::have_ffmpeg();
        if looked_at {
            if let Err(e) = self.info(&vid) {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(bad(if e.1.is_empty() { "That file does not look like a video".to_string() } else { e.1 }));
            }
        }
        let st = self.clone();
        Ok(self.run_job(move |ctl| {
            ensure_ffmpeg(ctl)?;
            if !looked_at {
                if let Err(e) = st.info(&vid) {
                    let _ = std::fs::remove_dir_all(st.work.join(&vid));
                    return Err(JobFail { msg: if e.1.is_empty() { "That file does not look like a video".to_string() } else { e.1 }, code: None });
                }
            }
            ctl.stage("download", 0.40, "Got your file");
            st.process_video(vid, ctl)
        }))
    }

    // ------------------------------------------------------------ library
    pub fn library(&self) -> Value {
        let mut dirs: Vec<_> = std::fs::read_dir(&self.work).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).collect();
        dirs.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).ok()));
        let mut out = vec![];
        for e in dirs {
            let name = e.file_name().to_string_lossy().to_string();
            if !valid_id(&name) || self.video_path(&name).is_err() { continue; }
            if self.is_ready(&name) {
                let m = self.song_meta(&name, false, None, None, None);
                out.push(json!({"video_id": name, "title": m["title"], "notes": m["notes"], "duration": m["duration"], "hand_colors": m["hand_colors"], "ready": true}));
            } else {
                out.push(json!({"video_id": name, "title": self.title(&name), "notes": 0, "duration": 0, "hand_colors": [], "ready": false}));
            }
        }
        Value::Array(out)
    }

    pub fn song(&self, vid: &str) -> ApiRes<Value> {
        let d = self.song_dir(vid)?;
        let meta = if d.join("result.json").exists() { self.song_meta(vid, false, None, None, None) } else { json!({"video_id": vid, "title": self.title(vid)}) };
        let info = self.info(vid)?;
        let mut m = meta.as_object().cloned().unwrap_or_default();
        m.extend([("width".to_string(), json!(info.width)), ("height".into(), json!(info.height)), ("fps".into(), json!(info.fps)),
            ("duration".into(), json!(info.duration)), ("calib".into(), read_json(&d.join("calib.json")).unwrap_or(Value::Null)),
            ("ready".into(), json!(self.is_ready(vid))), ("midis".into(), json!(self.midis(vid)))]);
        Ok(Value::Object(m))
    }

    pub fn delete_song(&self, vid: &str) -> ApiRes<()> {
        let d = self.song_dir(vid)?;
        std::fs::remove_dir_all(d).map_err(|e| e.to_string())?;
        self.forget(vid);
        Ok(())
    }

    pub fn swap_hands(&self, vid: &str) -> ApiRes<()> {
        let d = self.song_dir(vid)?;
        let notes = read_json(&d.join("result.json")).and_then(|v| v.as_array().cloned()).ok_or_else(|| not_found("No notes yet"))?;
        let swapped: Vec<Note> = notes.iter().map(|n| {
            let hand = n["hand"].as_i64().unwrap_or(0) as i32;
            Note { name: n["name"].as_str().unwrap_or("").into(), midi: n["midi"].as_i64().unwrap_or(0) as i32, start: n["start"].as_f64().unwrap_or(0.0),
                end: n["end"].as_f64().unwrap_or(0.0), angle: n["hue"].as_f64(), hand: if hand == 0 || hand == 1 { 1 - hand } else { hand }, chroma: 0.0 }
        }).collect();
        midi::write(&swapped, &d.join("result.mid"))?;
        let mut meta = self.song_meta(vid, false, None, None, None);
        for k in ["hand_hues", "hand_colors"] {
            if let Some(a) = meta[k].as_array_mut() { a.reverse(); }
        }
        let _ = std::fs::write(d.join("song.json"), meta.to_string());
        if let Some(mut c) = read_json(&d.join("calib.json")) {
            c["swap_hands"] = json!(!c["swap_hands"].as_bool().unwrap_or(false));
            let _ = std::fs::write(d.join("calib.json"), c.to_string());
        }
        Ok(())
    }

    // ------------------------------------------------------------ keyboard adjusting
    pub fn autodetect(&self, vid: &str) -> ApiRes<Value> {
        let auto = detect::autodetect(&self.video_path(vid)?, &self.info(vid)?)?;
        Ok(match auto {
            None => { self.medians.lock().unwrap().remove(vid); json!({"found": false}) }
            Some(a) => {
                self.medians.lock().unwrap().insert(vid.to_string(), Arc::new(a.median.clone()));
                let mut v = serde_json::to_value(&a.det).unwrap();
                v["found"] = json!(true);
                v["suggest_times"] = json!(a.times);
                v
            }
        })
    }

    pub fn preview(&self, c: &Calib, t: f64) -> ApiRes<Value> {
        let keys = self.keys_for(c)?;
        let base = self.baseline_for(c, &keys)?;
        let fr = self.read_frame(&c.video_id, t)?;
        let lab = self.sample_full(&fr, &keys);
        let mask = analyze::pressed_mask(&lab, &base, c.threshold);
        let hues: Vec<f64> = lab.iter().zip(&mask).filter(|(l, &m)| m && color::chroma(l) > 12.0).map(|(l, _)| hue_angle(l)).collect();
        let cents = analyze::cluster_hands(&hues);
        let pressed: Vec<Value> = (0..keys.len()).filter(|&i| mask[i]).map(|i| {
            let l = &lab[i];
            json!({"name": keys[i].name, "hue": hue_angle(l), "rgb": color::lab_to_rgb(*l), "delta": color::dist(l, &base[i])})
        }).collect();
        let _ = std::fs::write(self.song_dir(&c.video_id)?.join("calib.json"), serde_json::to_string(c).unwrap());
        Ok(json!({"pressed": pressed, "hand_hues": cents, "hand_colors": hand_colors(&cents)}))
    }

    /// Given notes the user says are sounding at time t, find the first-key name that makes the detected pressed keys
    /// carry exactly those names.
    pub fn match_first_note(&self, c: &Calib, t: f64, notes: &[String]) -> ApiRes<Value> {
        let want: std::collections::HashSet<String> = notes.iter().map(|n| n.trim().to_uppercase().replace('♯', "#")).filter(|n| !n.is_empty()).map(|w| w.replace("B#", "C")).collect();
        if want.is_empty() { return Err(bad("Type at least one note, e.g. A2 or C#4")); }
        let frame = self.read_frame(&c.video_id, t)?;
        let med = self.median_frame(&c.video_id)?;
        let mut hits = vec![];
        for letter in "CDEFGAB".chars() {
            for octave in 0..8 {
                let first = format!("{letter}{octave}");
                let Some(keys) = build_keys(c.x0, c.x1, c.y0, c.y1, &first, c.n_white) else { continue };
                let sampler = Sampler::new(&keys, frame.w, frame.h, 0);
                let mask = analyze::pressed_mask(&sampler.sample(&frame.rgb), &sampler.sample(&med.rgb), c.threshold);
                let names: std::collections::HashSet<String> = (0..keys.len()).filter(|&i| mask[i]).map(|i| keys[i].name.clone()).collect();
                if !names.is_empty() && names == want { hits.push(first); }
            }
        }
        Ok(json!({"matches": hits}))
    }
}

// ---------------------------------------------------------------------- sheets
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SheetReq {
    pub video_id: String,
    pub midi: Option<String>,
    pub beats: usize,
    pub bpm: Option<f64>,
    pub t_start: Option<f64>,
    pub t_downbeat: Option<f64>,
    pub mpp: usize,
    pub spp: i64,
    pub w: u32,
    pub h: u32,
    pub dot: f64,
    pub acc: String,
    pub octave: bool,
    pub ticks: bool,
    pub rotate: String,
    pub print: bool,
    pub mpr: i64,
    pub chords: bool,
    pub lsp: f64,
    pub holds: bool,
    pub link: bool,
    // the title page of the sheet music (None = what the song gives)
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub composer: Option<String>,
    pub arranger: Option<String>,
    pub hold: Option<f64>,
    /// How the score finds its bars: None or "auto" (best fit), "fixed" (the given beats per bar), "sheet" (the easy sheet's bars)
    pub bars: Option<String>,
    /// A key to write the score in, as "fifths,major" or "fifths,minor" (for example "-4,major" is A flat major); None = detected
    pub key: Option<String>,
}
impl Default for SheetReq {
    fn default() -> Self {
        SheetReq { video_id: String::new(), midi: None, beats: 3, bpm: None, t_start: None, t_downbeat: None, mpp: 4, spp: 1, w: 1680, h: 1264,
            dot: 56.0, acc: "sharp".into(), octave: false, ticks: true, rotate: "cw".into(), print: false, mpr: 4, chords: false, lsp: 1.0, holds: true, link: true,
            title: None, subtitle: None, composer: None, arranger: None, hold: None, bars: None, key: None }
    }
}

impl State {
    pub fn sheet_params(&self, q: &SheetReq) -> ApiRes<(Vec<midi::SheetNote>, sheet::Params)> {
        let vid = &q.video_id;
        self.song_dir(vid)?;
        let midis = self.midis(vid);
        if midis.is_empty() { return Err(not_found("No MIDI yet for this video. Generate it in the first tab.")); }
        let name = q.midi.as_ref().filter(|m| midis.contains(m)).unwrap_or(&midis[0]);
        let notes = midi::load(&self.work.join(vid).join(name))?;
        if notes.is_empty() { return Err(bad("That MIDI has no notes")); }
        let mut p = sheet::Params::default();
        p.beats_per_measure = q.beats.max(1);
        p.beat_period = q.bpm.filter(|b| *b > 0.0).map(|b| 60.0 / b);
        p.t_start = q.t_start;
        p.t_downbeat = q.t_downbeat;
        p.measures_per_page = q.mpp.max(1);
        p.systems_per_page = q.spp.min(q.mpp as i64).max(1);
        p.size = (q.w, q.h);
        p.dot = q.dot;
        p.flat = q.acc == "flat";
        p.show_octave = q.octave;
        p.show_beats = q.ticks;
        p.title = self.title(vid);
        p.rotate = match q.rotate.as_str() { "cw" => sheet::Rotate::Cw, "ccw" => sheet::Rotate::Ccw, _ => sheet::Rotate::None };
        if q.print {
            // A4 portrait: 5 rows on every page (the first page also carries the song title)
            p.print_mode = true;
            p.size = sheet::A4_SIZE;
            p.rotate = sheet::Rotate::None;
            p.systems_per_page = 5;
            p.first_page_rows = 5;
            p.measures_per_row = q.mpr.clamp(1, 8) as usize;
        }
        p.group_chords = q.chords;
        p.show_holds = q.holds;
        p.show_link = q.link;
        p.line_space = q.lsp.clamp(0.8, 3.0);
        Ok((notes, p))
    }

    pub fn sheet_auto(&self, q: &SheetReq) -> ApiRes<Value> {
        let (notes, mut p) = self.sheet_params(q)?;
        sheet::resolve(&notes, &mut p);
        let mlen = p.beat_period.unwrap() * q.beats as f64;
        let total = ((notes.last().unwrap().start - p.t_downbeat.unwrap()) / mlen).ceil() as i64 + 1;
        Ok(json!({"midis": self.midis(&q.video_id), "bpm": r(60.0 / p.beat_period.unwrap(), 2), "t_start": r(p.t_start.unwrap(), 3),
            "t_downbeat": r(p.t_downbeat.unwrap(), 3), "measures": total, "pages": sheet::pages_needed(&p, total.max(0) as usize),
            "notes": notes.len(), "end": r(notes.last().unwrap().start, 1)}))
    }

    /// The song as standard sheet music (MusicXML), with bar lines from the same tracking the easy sheet uses.
    pub fn score(&self, q: &SheetReq) -> ApiRes<Value> {
        let (notes, mut p) = self.sheet_params(q)?;
        sheet::resolve(&notes, &mut p);
        let base = crate::score::Grid { t_downbeat: p.t_downbeat.unwrap(), beat: p.beat_period.unwrap(), beats: q.beats.max(1) };
        let hold = q.hold.unwrap_or(2.0).clamp(0.0, 8.0);
        let g = match q.bars.as_deref() {
            Some("sheet") => base,
            Some("fixed") => crate::score::auto_grid(&notes, &base, Some(q.beats.max(1)), hold),
            _ => crate::score::auto_grid(&notes, &base, None, hold),
        };
        let meta = crate::score::Meta { title: q.title.clone().unwrap_or_else(|| self.title(&q.video_id)), subtitle: q.subtitle.clone(), composer: q.composer.clone().unwrap_or_default(), arranger: q.arranger.clone().unwrap_or_default(), hold,
            key: q.key.as_deref().and_then(|k| { let (f, m) = k.split_once(',')?; Some((f.trim().parse::<i32>().ok()?, m.trim() == "minor")) }) };
        let s = crate::score::build_with(&notes, &g, &meta);
        Ok(json!({"xml": s.xml, "beat_times": s.beat_times, "fifths": s.fifths, "minor": s.minor, "measures": s.measures, "bpm": r(s.bpm, 4), "beats": s.beats, "t_downbeat": r(s.t_downbeat, 3), "detected": {"fifths": s.detected.0, "minor": s.detected.1}}))
    }

    /// One page as a PNG plus the data the web page uses to light the notes while they play.
    pub fn sheet_page(&self, q: &SheetReq, page: usize) -> ApiRes<(Vec<u8>, Vec<(&'static str, String)>)> {
        let (notes, mut p) = self.sheet_params(q)?;
        let img = sheet::render(&notes, &mut p, page);
        let png = export::png_bytes(&img.gray, img.w, img.h, 300);
        let boxes: Vec<Value> = p.boxes.iter().map(|b| json!([r(b.0, 3), r(b.1, 3), b.2 as i64, b.3 as i64, b.4 as i64, b.5 as i64])).collect();
        let dots: Vec<Value> = p.dots.iter().map(|d| json!([(d.0 * 1000.0) as i64, (d.1.max(d.0) * 1000.0) as i64, d.2 as i64, d.3 as i64])).collect();
        Ok((png, vec![("X-Sheet-Scale", format!("{:.2}", p.used_scale)), ("X-Sheet-Boxes", Value::Array(boxes).to_string()),
            ("X-Sheet-Notes", Value::Array(dots).to_string()), ("X-Sheet-Dot", (p.dot_r as i64).to_string())]))
    }

    /// Whole-song export as a job: "pdf" (A4 for printing) or "epub" (Kindle).
    pub fn start_export(self: &Arc<Self>, mut q: SheetReq, format: String) -> ApiRes<String> {
        if format != "pdf" && format != "epub" { return Err(bad("format must be pdf or epub")); }
        q.print = format == "pdf";
        if !matches!(q.rotate.as_str(), "cw" | "ccw") { q.rotate = "cw".into(); }
        let (notes, mut p) = self.sheet_params(&q)?;
        if format == "epub" && p.rotate == sheet::Rotate::None { p.rotate = sheet::Rotate::Cw; }
        let st = self.clone();
        Ok(self.run_job(move |ctl| {
            sheet::resolve(&notes, &mut p);
            let mlen = p.beat_period.unwrap() * p.beats_per_measure as f64;
            let total = ((notes.last().unwrap().start - p.t_downbeat.unwrap()) / mlen).ceil() as i64 + 1;
            let n = sheet::pages_needed(&p, total.max(0) as usize);
            let mut pages = Vec::new();
            for i in 0..n {
                ctl.progress(0.85 * i as f64 / n as f64, format!("Drawing page {} of {}", i + 1, n));
                let img = sheet::render(&notes, &mut p, i);
                pages.push((img.gray, img.w, img.h));
            }
            let title = st.title(&q.video_id);
            let out_dir = st.work.join(&q.video_id).join("exports");
            std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let name = format!("{}-{}", export::safe_name(&title, 60), if format == "pdf" { "print-A4.pdf" } else { "kindle.epub" });
            ctl.progress(0.88, if format == "pdf" { "Building the PDF" } else { "Packing the book for Kindle" });
            let bytes = if format == "pdf" {
                export::pdf_from_pages(&pages, &title, 300.0)
            } else {
                let pngs: Vec<Vec<u8>> = pages.iter().map(|(g, w, h)| export::png_bytes(g, *w, *h, 300)).collect();
                export::epub_from_pages(&pngs, pages[0].1, pages[0].2, &title)?
            };
            std::fs::write(out_dir.join(&name), &bytes).map_err(|e| e.to_string())?;
            ctl.progress(1.0, "Done");
            Ok(json!({"file": format!("/api/exports/{}/{}", q.video_id, name), "name": name, "pages": n, "mb": r(bytes.len() as f64 / 1e6, 1)}))
        }))
    }
}

