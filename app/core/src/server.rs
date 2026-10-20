//! HTTP layer: the JSON API the web page talks to, plus the built page itself.
use crate::app::{ApiError, ApiRes, Calib, SheetReq, State};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State as St};
use axum::http::{header, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

#[derive(RustEmbed)]
#[folder = "../../frontend/dist"]
struct Assets;

/// A file of the embedded web page (used by the native player to find the piano samples).
pub fn asset(path: &str) -> Option<Vec<u8>> {
    Assets::get(path).map(|f| f.data.into_owned())
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::from_u16(self.0).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(json!({"detail": self.1}))).into_response()
    }
}

type S = St<Arc<State>>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> ApiRes<T> + Send + 'static) -> ApiRes<T> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ApiError(500, "Something went wrong inside the app".into()))?
}

fn de_bool_lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let s = String::deserialize(d)?.to_lowercase();
    Ok(matches!(s.as_str(), "1" | "true" | "yes" | "on" | "t" | "y"))
}

/// Query string of the sheet endpoints (booleans arrive as "true"/"false").
#[derive(Deserialize, Default)]
struct SheetQuery {
    video_id: String,
    midi: Option<String>,
    beats: Option<usize>,
    bpm: Option<f64>,
    t_start: Option<f64>,
    t_downbeat: Option<f64>,
    mpp: Option<usize>,
    spp: Option<i64>,
    w: Option<u32>,
    h: Option<u32>,
    dot: Option<f64>,
    acc: Option<String>,
    #[serde(default, deserialize_with = "opt_bool")]
    octave: Option<bool>,
    #[serde(default, deserialize_with = "opt_bool")]
    ticks: Option<bool>,
    rotate: Option<String>,
    #[serde(default, deserialize_with = "opt_bool")]
    print: Option<bool>,
    mpr: Option<i64>,
    #[serde(default, deserialize_with = "opt_bool")]
    chords: Option<bool>,
    lsp: Option<f64>,
    #[serde(default, deserialize_with = "opt_bool")]
    holds: Option<bool>,
    #[serde(default, deserialize_with = "opt_bool")]
    link: Option<bool>,
    page: Option<usize>,
    #[serde(default, deserialize_with = "opt_bool")]
    download: Option<bool>,
    title: Option<String>,
    subtitle: Option<String>,
    composer: Option<String>,
    arranger: Option<String>,
    hold: Option<f64>,
    bars: Option<String>,
    key: Option<String>,
}

fn opt_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(Some(de_bool_lenient(d)?))
}

impl SheetQuery {
    fn req(self) -> SheetReq {
        let d = SheetReq::default();
        SheetReq {
            video_id: self.video_id, midi: self.midi, beats: self.beats.unwrap_or(d.beats), bpm: self.bpm, t_start: self.t_start,
            t_downbeat: self.t_downbeat, mpp: self.mpp.unwrap_or(d.mpp), spp: self.spp.unwrap_or(d.spp), w: self.w.unwrap_or(d.w),
            h: self.h.unwrap_or(d.h), dot: self.dot.unwrap_or(d.dot), acc: self.acc.unwrap_or(d.acc), octave: self.octave.unwrap_or(d.octave),
            ticks: self.ticks.unwrap_or(d.ticks), rotate: self.rotate.unwrap_or(d.rotate), print: self.print.unwrap_or(d.print),
            mpr: self.mpr.unwrap_or(d.mpr), chords: self.chords.unwrap_or(d.chords), lsp: self.lsp.unwrap_or(d.lsp),
            holds: self.holds.unwrap_or(d.holds), link: self.link.unwrap_or(d.link),
            title: self.title, subtitle: self.subtitle, composer: self.composer, arranger: self.arranger, hold: self.hold, bars: self.bars, key: self.key,
        }
    }
}

#[derive(Deserialize)]
struct UrlReq {
    url: String,
    #[serde(default)]
    quality: Option<String>,
}

#[derive(Deserialize)]
struct PreviewReq {
    #[serde(flatten)]
    c: Calib,
    #[serde(default)]
    t: f64,
}

fn one() -> f64 {
    1.0
}

fn both_loud() -> [f32; 2] {
    [1.0, 1.0]
}

/// How the song should be played (see audio::PlayOpts); every part is optional.
#[derive(Deserialize)]
struct AudioOpts {
    #[serde(default = "one")]
    speed: f64,
    #[serde(default = "both_loud")]
    gains: [f32; 2],
}

impl AudioOpts {
    fn play(&self, from: f64) -> crate::audio::PlayOpts {
        crate::audio::PlayOpts { from, speed: self.speed, gains: self.gains }
    }
    fn live(&self) -> crate::audio::Live {
        crate::audio::Live { speed: self.speed, gains: self.gains }
    }
}

#[derive(Deserialize)]
struct PlayReq {
    video_id: String,
    #[serde(default)]
    from: f64,
    #[serde(flatten)]
    o: AudioOpts,
}

#[derive(Deserialize)]
struct MatchReq {
    #[serde(flatten)]
    c: Calib,
    #[serde(default)]
    t: f64,
    notes: Vec<String>,
}

#[derive(Deserialize)]
struct ExportReq {
    video_id: String,
    format: String,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

fn file_response(bytes: Vec<u8>, mime: &str, extra: &[(HeaderName, String)]) -> Response {
    let mut r = Response::new(Body::from(bytes));
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_str(mime).unwrap());
    for (k, v) in extra {
        if let Ok(v) = HeaderValue::from_str(v) { r.headers_mut().insert(k.clone(), v); }
    }
    r
}

async fn read_file(path: std::path::PathBuf) -> ApiRes<Vec<u8>> {
    tokio::fs::read(path).await.map_err(|_| ApiError(404, "Not found".into()))
}

pub fn router(st: Arc<State>) -> Router {
    Router::new()
        .route("/api/process", post(|s: S, Json(r): Json<UrlReq>| async move {
            let job = blocking(move || s.0.start_url(r.url, r.quality.unwrap_or_else(|| "1080p".into()))).await?;
            Ok::<_, ApiError>(Json(json!({"job": job})))
        }))
        .route("/api/upload", post(upload).layer(DefaultBodyLimit::disable()))
        .route("/api/job/{id}", get(|s: S, Path(id): Path<String>| async move {
            s.0.job(&id).map(Json).ok_or(ApiError(404, "Not Found".into()))
        }))
        .route("/api/library", get(|s: S| async move { Json(blocking(move || Ok(s.0.library())).await.unwrap_or(json!([]))) }))
        .route("/api/song/{vid}", get(|s: S, Path(v): Path<String>| async move { blocking(move || s.0.song(&v)).await.map(Json) })
            .delete(|s: S, Path(v): Path<String>| async move { blocking(move || s.0.delete_song(&v).map(|_| json!({"deleted": v}))).await.map(Json) }))
        .route("/api/song/{vid}/notes", get(|s: S, Path(v): Path<String>| async move {
            let d = s.0.song_dir(&v)?;
            let b = read_file(d.join("result.json")).await.map_err(|_| ApiError(404, "No notes yet".into()))?;
            Ok::<_, ApiError>(file_response(b, "application/json", &[(header::CACHE_CONTROL, "no-cache".into())]))
        }))
        .route("/api/song/{vid}/swap-hands", post(|s: S, Path(v): Path<String>| async move {
            blocking(move || s.0.swap_hands(&v).map(|_| json!({"ok": true}))).await.map(Json)
        }))
        .route("/api/thumb/{vid}", get(|s: S, Path(v): Path<String>| async move {
            let p = blocking(move || s.0.make_thumb(&v)).await?;
            Ok::<_, ApiError>(file_response(read_file(p).await?, "image/jpeg", &[(header::CACHE_CONTROL, "private, max-age=86400".into())]))
        }))
        .route("/api/frame/{vid}", get(|s: S, Path(v): Path<String>, Query(q): Query<std::collections::HashMap<String, String>>| async move {
            let t: f64 = q.get("t").and_then(|t| t.parse().ok()).unwrap_or(0.0);
            let b = blocking(move || s.0.jpeg(&v, t)).await?;
            Ok::<_, ApiError>(file_response(b, "image/jpeg", &[(header::CACHE_CONTROL, "private, max-age=3600".into())]))
        }))
        .route("/api/autodetect/{vid}", get(|s: S, Path(v): Path<String>| async move { blocking(move || s.0.autodetect(&v)).await.map(Json) }))
        .route("/api/preview", post(|s: S, Json(r): Json<PreviewReq>| async move { blocking(move || s.0.preview(&r.c, r.t)).await.map(Json) }))
        .route("/api/match", post(|s: S, Json(r): Json<MatchReq>| async move { blocking(move || s.0.match_first_note(&r.c, r.t, &r.notes)).await.map(Json) }))
        .route("/api/analyze", post(|s: S, Json(c): Json<Calib>| async move {
            let st = s.0.clone();
            blocking(move || {
                st.keys_for(&c)?; // validate early
                st.video_path(&c.video_id)?;
                let st2 = st.clone();
                Ok(st.run_job(move |ctl| {
                    let ctl2 = ctl.clone();
                    st2.analyze_and_save(&c, &mut |p, s| ctl2.progress(p, s), None).map_err(|e| e.1.into())
                }))
            }).await.map(|job| Json(json!({"job": job})))
        }))
        .route("/api/result/{file}", get(|s: S, Path(file): Path<String>| async move {
            let vid = file.strip_suffix(".mid").unwrap_or(&file).to_string();
            let d = s.0.song_dir(&vid)?;
            let name = s.0.midis(&vid).into_iter().next().ok_or(ApiError(404, "Not Found".into()))?;
            let b = read_file(d.join(name)).await?;
            Ok::<_, ApiError>(file_response(b, "audio/midi", &[(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{vid}.mid\""))]))
        }))
        .route("/api/dev", get(|| async { Json(json!({"perf": std::env::var_os("YT2MIDI_PERF").is_some()})) }))
        .route("/api/perf", post(|body: String| async move {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::var("YT2MIDI_PERF").unwrap_or_default()) { let _ = writeln!(f, "{body}"); }
            Json(json!({"ok": true}))
        }))
        .route("/api/audio/status", get(|s: S| async move {
            let on = s.0.native_audio.load(std::sync::atomic::Ordering::SeqCst);
            Json(json!({"native": on, "ready": on && crate::audio::Audio::get().ready()}))
        }))
        .route("/api/audio/play", post(|s: S, Json(r): Json<PlayReq>| async move {
            if !s.0.native_audio.load(std::sync::atomic::Ordering::SeqCst) { return Err(ApiError(400, "Native audio is off".into())); }
            let notes = blocking(move || s.0.song_notes(&r.video_id)).await?;
            let a = crate::audio::Audio::get();
            a.play(notes, r.o.play(r.from.max(0.0)));
            Ok(Json(json!({"pos": a.pos().pos})))
        }))
        .route("/api/audio/stop", post(|s: S| async move {
            if s.0.native_audio.load(std::sync::atomic::Ordering::SeqCst) { crate::audio::Audio::get().stop(); }
            Json(json!({"ok": true}))
        }))
        .route("/api/audio/set", post(|s: S, Json(o): Json<AudioOpts>| async move {
            if s.0.native_audio.load(std::sync::atomic::Ordering::SeqCst) { crate::audio::Audio::get().set(&o.live()); }
            Json(json!({"ok": true}))
        }))
        .route("/api/audio/pos", get(|| async move {
            let p = crate::audio::Audio::get().pos();
            Json(json!({"pos": p.pos, "speed": p.speed, "playing": p.playing}))
        }))
        .route("/api/sheet/auto", get(|s: S, Query(q): Query<SheetQuery>| async move { blocking(move || s.0.sheet_auto(&q.req())).await.map(Json) }))
        .route("/api/score", get(|s: S, Query(q): Query<SheetQuery>| async move { blocking(move || s.0.score(&q.req())).await.map(Json) }))
        .route("/api/sheet/page.png", get(|s: S, Query(q): Query<SheetQuery>| async move {
            let (page, dl) = (q.page.unwrap_or(0), q.download.unwrap_or(false));
            let req = q.req();
            let vid = req.video_id.clone();
            let (png, hdrs) = blocking(move || s.0.sheet_page(&req, page)).await?;
            let mut extra: Vec<(HeaderName, String)> = hdrs.into_iter().map(|(k, v)| (HeaderName::from_static(match k { "X-Sheet-Scale" => "x-sheet-scale", "X-Sheet-Boxes" => "x-sheet-boxes", "X-Sheet-Notes" => "x-sheet-notes", _ => "x-sheet-dot" }), v)).collect();
            if dl { extra.push((header::CONTENT_DISPOSITION, format!("attachment; filename=\"{vid}-page{:03}.png\"", page + 1))); }
            Ok::<_, ApiError>(file_response(png, "image/png", &extra))
        }))
        .route("/api/sheet/export", post(|s: S, Json(r): Json<ExportReq>| async move {
            let q: SheetReq = serde_json::from_value(Value::Object(r.rest.into_iter().chain([("video_id".to_string(), json!(r.video_id))]).collect())).map_err(|e| ApiError(400, e.to_string()))?;
            let job = blocking(move || s.0.start_export(q, r.format)).await?;
            Ok::<_, ApiError>(Json(json!({"job": job})))
        }))
        .route("/api/exports/{vid}/{name}", get(|s: S, Path((vid, name)): Path<(String, String)>| async move {
            let ok = regex::Regex::new(r"^[\w\-]+\.(pdf|epub)$").unwrap().is_match(&name);
            let f = s.0.song_dir(&vid)?.join("exports").join(&name);
            if !ok || !f.is_file() { return Err(ApiError(404, "Not Found".into())); }
            let mime = if name.ends_with(".pdf") { "application/pdf" } else { "application/epub+zip" };
            Ok(file_response(read_file(f).await?, mime, &[(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))]))
        }))
        .fallback(static_file)
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(st)
}

/// A video dropped on the page: streamed to disk, then processed like a downloaded one.
async fn upload(St(st): S, Query(q): Query<std::collections::HashMap<String, String>>, body: Body) -> Result<Json<Value>, ApiError> {
    let name = q.get("name").cloned().unwrap_or_else(|| "video.mp4".into());
    let tmp = st.work.join(format!(".upload-{}", uuid::Uuid::new_v4().simple()));
    let mut f = tokio::fs::File::create(&tmp).await.map_err(|e| ApiError(500, e.to_string()))?;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError(400, format!("Upload interrupted: {e}")))?;
        f.write_all(&chunk).await.map_err(|e| ApiError(500, e.to_string()))?;
    }
    f.flush().await.map_err(|e| ApiError(500, e.to_string()))?;
    drop(f);
    let job = blocking(move || st.start_file(tmp, &name)).await?;
    Ok(Json(json!({"job": job})))
}

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(f) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
            file_response(f.data.into_owned(), mime.as_ref(), &[(header::CACHE_CONTROL, cache.into())])
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

/// Start serving on a background thread. Tries `preferred` first so the page keeps the same address (and so its saved
/// settings) between runs, then any free port. Returns the port.
pub fn spawn(st: Arc<State>, preferred: u16) -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", preferred)).or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0)))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
        rt.block_on(async move {
            let l = tokio::net::TcpListener::from_std(listener).expect("listener");
            axum::serve(l, router(st)).await.ok();
        });
    });
    Ok(port)
}
