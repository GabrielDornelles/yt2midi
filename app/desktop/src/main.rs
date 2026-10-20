//! Desktop shell: starts the engine on a local port and shows the web page in a native window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use tauri::webview::DownloadEvent;
use tauri::{WebviewUrl, WebviewWindowBuilder};
use yt2midi_core::{app::State, server, tools};

/// Same address every run, so the page keeps its saved settings (theme, sheet options).
const PREFERRED_PORT: u16 = 47831;

/// A path in `dir` for `name` that does not overwrite an existing file ("song.pdf", "song (1).pdf", ...).
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("download");
    let ext = p.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    let mut c = dir.join(name);
    let mut i = 1;
    while c.exists() {
        c = dir.join(format!("{stem} ({i}){ext}"));
        i += 1;
    }
    c
}

/// A Chromium-based browser, if one is installed (its engine is smoother than the system web view on some machines).
fn find_chromium() -> Option<PathBuf> {
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "brave-browser", "microsoft-edge"].iter().find_map(|n| tools::find(n))
}

static ICON: &[u8] = include_bytes!("../icons/icon.png");

/// A launcher entry (app menu, search) with our icon. The running window still shows as the browser in the dock: a
/// Chromium app window always reports the browser's own window class, which cannot be changed.
fn install_launcher() {
    let (Some(apps), Ok(exe)) = (dirs::data_dir().map(|d| d.join("applications")), std::env::current_exe()) else { return };
    let icon = tools::data_dir().join("icon.png");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=yt2midi\nComment=Piano videos to MIDI and an easy sheet\nExec={}\nIcon={}\nCategories=AudioVideo;Music;\nTerminal=false\n",
        exe.display(), icon.display()
    );
    let file = apps.join("yt2midi.desktop");
    if std::fs::read_to_string(&file).ok().as_deref() == Some(entry.as_str()) {
        return;
    }
    let _ = std::fs::create_dir_all(&apps);
    let _ = std::fs::create_dir_all(tools::data_dir());
    let _ = std::fs::write(&icon, ICON);
    let _ = std::fs::write(&file, entry);
}

/// Show the page in a browser "app window" (no tabs, no address bar) and return when it is closed.
/// Returns false if the browser could not show it (so the caller can use the system web view instead).
fn run_in_chromium(bin: &Path, port: u16) -> bool {
    let profile = tools::data_dir().join("window-profile");
    let hash = std::env::var("YT2MIDI_HASH").unwrap_or_default();
    let started = std::time::Instant::now();
    install_launcher();
    let status = std::process::Command::new(bin)
        // the browser's background features (on-device ML models and so on) are not needed for our window
        .arg("--disable-features=OptimizationGuideModelExecution,OptimizationHints,OptimizationGuideOnDeviceModel,Translate")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .arg(format!("--app=http://127.0.0.1:{port}/{hash}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(["--no-first-run", "--no-default-browser-check", "--window-size=1180,900"])
        .status();
    match status {
        Ok(s) => s.success() || started.elapsed().as_secs() >= 3, // quitting at once with an error means it did not work
        Err(_) => false,
    }
}

fn main() {
    let state = State::new(tools::work_dir());
    let port = server::spawn(state.clone(), PREFERRED_PORT).expect("could not start the local server");
    // On Linux the system web view (WebKitGTK) is limited to 60 fps and scrolls roughly, so a Chromium-based browser is
    // used for the window when one is installed. YT2MIDI_WINDOW=webview always uses the system web view instead.
    let want_chromium = cfg!(target_os = "linux") && std::env::var("YT2MIDI_WINDOW").as_deref() != Ok("webview");
    if want_chromium {
        if let Some(bin) = find_chromium() {
            if run_in_chromium(&bin, port) {
                return;
            }
        }
    }
    // the system web view is weak at audio, so the desktop app plays the piano itself
    state.native_audio.store(true, std::sync::atomic::Ordering::SeqCst);
    tauri::Builder::default()
        .setup(move |app| {
            let hash = std::env::var("YT2MIDI_HASH").unwrap_or_default(); // for testing: open straight on a page
            let url = format!("http://127.0.0.1:{port}/{hash}").parse().expect("url");
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("yt2midi")
                .inner_size(1180.0, 900.0)
                .min_inner_size(480.0, 560.0)
                // let the page handle dropped files itself (it sends them to the engine)
                .disable_drag_drop_handler()
                // files the page offers for download (MIDI, PDF, EPUB, PNG) go to the Downloads folder
                .on_download(|webview, event| {
                    match event {
                        DownloadEvent::Requested { destination, .. } => {
                            let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."));
                            let name = destination.file_name().and_then(|n| n.to_str()).unwrap_or("download").to_string();
                            *destination = free_name(&dir, &name);
                        }
                        DownloadEvent::Finished { path, success, .. } => {
                            let msg = match (&path, success) {
                                (Some(p), true) => format!("{{\"ok\":true,\"path\":{:?}}}", p.display().to_string()),
                                _ => "{\"ok\":false}".to_string(),
                            };
                            let _ = webview.eval(format!("window.dispatchEvent(new CustomEvent('yt2midi:saved',{{detail:{msg}}}))"));
                        }
                        _ => {}
                    }
                    true
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running yt2midi");
}
