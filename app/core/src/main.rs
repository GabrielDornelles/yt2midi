//! Headless server: the same engine as the desktop app, served at http://127.0.0.1:<port> for a browser.
use yt2midi_core::{app::State, server, tools};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let port: u16 = get("--port").and_then(|p| p.parse().ok()).unwrap_or(8000);
    let work = get("--work").map(Into::into).unwrap_or_else(tools::work_dir);
    println!("songs live in {}", work.display());
    let st = State::new(work);
    if std::env::var_os("YT2MIDI_NATIVE_AUDIO").is_some() {
        // for testing the desktop sound engine without the desktop window
        st.native_audio.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let port = server::spawn(st, port).expect("could not start the server");
    println!("yt2midi is running at http://127.0.0.1:{port}");
    if args.iter().any(|a| a == "--open") {
        let _ = std::process::Command::new("xdg-open").arg(format!("http://127.0.0.1:{port}")).spawn();
    }
    loop {
        std::thread::park();
    }
}
