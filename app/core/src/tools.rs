//! Where the helper programs live (ffmpeg, yt-dlp, a JavaScript runtime) and how to fetch the ones we can.
//! Order of lookup: environment override, next to the app, our own `bin` folder, then the system.
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub type Res<T> = Result<T, String>;

pub fn data_dir() -> PathBuf {
    std::env::var_os("YT2MIDI_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("yt2midi"))
}

pub fn work_dir() -> PathBuf {
    std::env::var_os("YT2MIDI_WORK").map(PathBuf::from).unwrap_or_else(|| data_dir().join("songs"))
}

pub fn bin_dir() -> PathBuf {
    data_dir().join("bin")
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn is_exec(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return fs::metadata(p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false);
    }
    #[allow(unreachable_code)]
    true
}

fn in_path(name: &str) -> Option<PathBuf> {
    let file = exe_name(name);
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // a GUI app started from a launcher often has a short PATH
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join(".local/bin"));
        // nvm keeps node out of the default PATH
        if let Ok(rd) = fs::read_dir(h.join(".nvm/versions/node")) {
            let mut v: Vec<_> = rd.flatten().map(|e| e.path().join("bin")).collect();
            v.sort();
            v.reverse();
            dirs.extend(v);
        }
    }
    dirs.extend(["/usr/local/bin", "/usr/bin", "/opt/homebrew/bin", "/snap/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(&file)).find(|c| is_exec(c))
}

fn beside_app(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let file = exe_name(name);
    [dir.join(&file), dir.join("bin").join(&file), dir.join("../lib/yt2midi").join(&file), dir.join("../lib/yt2midi/bin").join(&file)]
        .into_iter()
        .find(|c| is_exec(c))
}

/// Find a tool: `YT2MIDI_<NAME>` env override, next to the app, our bin folder, then PATH.
pub fn find(name: &str) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os(format!("YT2MIDI_{}", name.to_uppercase().replace('-', "_"))) {
        let p = PathBuf::from(v);
        if is_exec(&p) {
            return Some(p);
        }
    }
    beside_app(name).or_else(|| Some(bin_dir().join(exe_name(name))).filter(|p| is_exec(p))).or_else(|| in_path(name))
}

pub fn ffmpeg() -> Res<PathBuf> {
    find("ffmpeg").ok_or_else(|| "ffmpeg was not found. Install it (for example: sudo apt install ffmpeg) and try again.".to_string())
}

pub fn ffprobe() -> Res<PathBuf> {
    find("ffprobe").ok_or_else(|| "ffprobe was not found. It comes with ffmpeg (sudo apt install ffmpeg).".to_string())
}

/// A helper program to run: on Windows without a console window flashing up for every call.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn have_ffmpeg() -> bool {
    find("ffmpeg").is_some() && find("ffprobe").is_some()
}

// ------------------------------------------------------------------ downloads on demand
fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" }
}

fn http_get(url: &str) -> Res<Box<dyn Read + Send + Sync>> {
    http_get_len(url).map(|x| x.0)
}

/// The body of a download and, when the server says, how long it is.
fn http_get_len(url: &str) -> Res<(Box<dyn Read + Send + Sync>, Option<u64>)> {
    let agent = ureq::AgentBuilder::new().user_agent("yt2midi").build();
    let r = agent.get(url).call().map_err(|e| format!("Could not download {url}: {e}"))?;
    let len = r.header("Content-Length").and_then(|v| v.parse().ok());
    Ok((Box::new(r.into_reader()), len))
}

fn save_exec(mut src: impl Read, dest: &Path) -> Res<()> {
    fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("part");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = src.read(&mut buf).map_err(|e| format!("download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

/// Download (or refresh) our own copy of yt-dlp. YouTube changes often, so a fresh copy is the cure for most failures.
pub fn install_ytdlp() -> Res<PathBuf> {
    let asset = if cfg!(windows) {
        "yt-dlp.exe".to_string()
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos".to_string()
    } else if arch() == "aarch64" {
        "yt-dlp_linux_aarch64".to_string()
    } else {
        "yt-dlp_linux".to_string()
    };
    let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{asset}");
    let dest = bin_dir().join(exe_name("yt-dlp"));
    save_exec(http_get(&url)?, &dest)?;
    Ok(dest)
}

pub fn ytdlp() -> Option<PathBuf> {
    find("yt-dlp")
}

/// YouTube needs a JavaScript runtime for yt-dlp. Use what is installed, else fetch Deno.
pub fn js_runtime() -> Res<(String, PathBuf)> {
    for name in ["deno", "node", "bun"] {
        if let Some(p) = find(name) {
            return Ok((name.to_string(), p));
        }
    }
    install_deno().map(|p| ("deno".to_string(), p))
}

pub fn install_deno() -> Res<PathBuf> {
    let triple = match (std::env::consts::OS, arch()) {
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", _) => "x86_64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", _) => "x86_64-apple-darwin",
        _ => "x86_64-pc-windows-msvc",
    };
    let url = format!("https://github.com/denoland/deno/releases/latest/download/deno-{triple}.zip");
    let mut body = Vec::new();
    http_get(&url)?.read_to_end(&mut body).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(body)).map_err(|e| e.to_string())?;
    let file = exe_name("deno");
    let entry = z.by_name(&file).map_err(|e| e.to_string())?;
    let dest = bin_dir().join(&file);
    save_exec(entry, &dest)?;
    Ok(dest)
}

// ------------------------------------------------------------------ ffmpeg on demand
/// Where to get a ready-made ffmpeg for this system, and whether it is a tar.xz (else a zip). These are the LGPL builds of
/// BtbN's FFmpeg-Builds: we only fetch them onto the user's own machine, we do not pass them on.
fn ffmpeg_source() -> Option<(String, bool)> {
    const BASE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/";
    match (std::env::consts::OS, arch()) {
        ("windows", "x86_64") => Some(("ffmpeg-master-latest-win64-lgpl.zip", false)),
        ("linux", "x86_64") => Some(("ffmpeg-master-latest-linux64-lgpl.tar.xz", true)),
        ("linux", "aarch64") => Some(("ffmpeg-master-latest-linuxarm64-lgpl.tar.xz", true)),
        _ => None,
    }
    .map(|(f, xz)| (format!("{BASE}{f}"), xz))
}

/// Get ffmpeg and ffprobe into our own `bin` folder. `progress(bytes so far, total if known)` is called as it downloads.
/// Used when the system has none (Windows, or a Linux machine without the package).
pub fn install_ffmpeg(progress: &mut dyn FnMut(u64, Option<u64>)) -> Res<()> {
    let Some((url, xz)) = ffmpeg_source() else {
        return Err("ffmpeg was not found, and there is no automatic download for this system. Install it (macOS: brew install ffmpeg) and try again.".into());
    };
    let dir = bin_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join(if xz { "ffmpeg-download.tar.xz" } else { "ffmpeg-download.zip" });
    {
        let (mut src, len) = http_get_len(&url)?;
        let mut f = fs::File::create(&archive).map_err(|e| e.to_string())?;
        let (mut buf, mut done) = (vec![0u8; 1 << 16], 0u64);
        let mut last = 0u64;
        loop {
            let n = src.read(&mut buf).map_err(|e| format!("The ffmpeg download was interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if done - last >= 1 << 20 {
                last = done;
                progress(done, len);
            }
        }
        progress(done, len);
    }
    let wanted = [exe_name("ffmpeg"), exe_name("ffprobe")];
    let res = if xz { unpack_tar_xz(&archive, &dir, &wanted) } else { unpack_zip(&archive, &dir, &wanted) };
    let _ = fs::remove_file(&archive);
    res?;
    if have_ffmpeg() { Ok(()) } else { Err("ffmpeg was downloaded but could not be set up".into()) }
}

fn unpack_zip(archive: &Path, dir: &Path, wanted: &[String]) -> Res<()> {
    let mut z = zip::ZipArchive::new(fs::File::open(archive).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let entry = z.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if let Some(w) = wanted.iter().find(|w| name.ends_with(&format!("/bin/{w}"))) {
            save_exec(entry, &dir.join(w))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn unpack_tar_xz(archive: &Path, dir: &Path, wanted: &[String]) -> Res<()> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(xz2::read::XzDecoder::new(std::io::BufReader::new(file)));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        if let Some(w) = wanted.iter().find(|w| path.ends_with(&format!("/bin/{w}"))) {
            save_exec(entry, &dir.join(w))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn unpack_tar_xz(_: &Path, _: &Path, _: &[String]) -> Res<()> {
    Err("this download format is not supported here".into())
}
