//! Native piano playback for the desktop app. The web view's own audio engine is too weak for a busy piece, so the
//! desktop app plays the notes itself, straight to the sound card, and the page only says play / stop / seek.
//! The sound is the same as in the browser: Salamander grand piano samples, pitch-shifted by at most a couple of
//! semitones, held while the key is down and then damped.
use std::io::Cursor;
use std::sync::{Arc, Mutex, OnceLock};

const KEEP_SECONDS: f64 = 7.0; // notes rarely sustain longer; trimming keeps memory small

#[derive(Clone, Debug)]
pub struct NoteEv {
    pub midi: i32,
    pub start: f64,
    pub end: f64,
    pub hand: i32,
}

/// How to play: where from, how fast, and how loud each hand is (0 = silent).
#[derive(Clone, Debug)]
pub struct PlayOpts {
    pub from: f64,
    pub speed: f64,
    pub gains: [f32; 2],
    /// notes that start before this are not played (a lead-in that is only seen)
    pub silent_until: f64,
    /// notes that start at or after this are not played (the end of a section that is looped)
    pub until: f64,
}

impl Default for PlayOpts {
    fn default() -> Self {
        PlayOpts { from: 0.0, speed: 1.0, gains: [1.0, 1.0], silent_until: f64::NEG_INFINITY, until: f64::INFINITY }
    }
}

/// Settings that can change while a song plays: playback carries on from where the music is.
#[derive(Clone, Debug)]
pub struct Live {
    pub speed: f64,
    pub gains: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct Pos {
    pub pos: f64,
    pub speed: f64,
    pub playing: bool,
}

struct Sample {
    midi: i32,
    data: Arc<Vec<f32>>,
    rate: f64,
}

struct Voice {
    data: Arc<Vec<f32>>,
    pos: f64,
    step: f64,
    age: f64,      // seconds since the voice began
    hold: f64,     // key held until this age, then damped
    tau: f64,      // damping time constant
    end: f64,      // voice is over at this age
    gain: f32,
}

#[derive(Default)]
struct Shared {
    events: Vec<NoteEv>,
    next: usize,
    /// Song seconds played since `from`.
    t: f64,
    from: f64,
    playing: bool,
    voices: Vec<Voice>,
    sr: f64,
    bank: Vec<Sample>,
    speed: f64,
    gains: [f32; 2],
    silent_until: f64,
    until: f64,
}

pub struct Audio {
    sh: Arc<Mutex<Shared>>,
    ready: Arc<std::sync::atomic::AtomicBool>,
}

fn sample_names() -> Vec<(i32, String)> {
    let mut v = vec![(21, "A0".to_string())];
    for o in 1..=7 {
        let c = 12 * (o + 1);
        v.extend([(c, format!("C{o}")), (c + 3, format!("Ds{o}")), (c + 6, format!("Fs{o}")), (c + 9, format!("A{o}"))]);
    }
    v.push((108, "C8".into()));
    v
}

/// Decode an mp3 to mono, trim to KEEP_SECONDS and fade the tail so the cut is inaudible.
fn decode(bytes: Vec<u8>) -> Option<(Vec<f32>, f64)> {
    use symphonia::core::{audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut probed = symphonia::default::get_probe().format(Hint::new().with_extension("mp3"), mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let track = probed.format.default_track()?.clone();
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
    let (mut out, mut sr) = (Vec::<f32>::new(), 44100.0);
    loop {
        let pkt = match probed.format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(_)) | Err(Error::ResetRequired) => break,
            Err(_) => break,
        };
        if pkt.track_id() != track.id {
            continue;
        }
        let Ok(buf) = dec.decode(&pkt) else { continue };
        sr = buf.spec().rate as f64;
        let ch = buf.spec().channels.count().max(1);
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, *buf.spec());
        sb.copy_interleaved_ref(buf);
        out.extend(sb.samples().chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
    }
    let len = out.len().min((KEEP_SECONDS * sr) as usize);
    out.truncate(len);
    let fade = ((0.6 * sr) as usize).min(len);
    for i in 0..fade {
        out[len - 1 - i] *= i as f32 / fade as f32;
    }
    Some((out, sr))
}

fn damp_all(sh: &mut Shared, tau: f64) {
    for v in sh.voices.iter_mut() {
        v.hold = v.age;
        v.tau = tau;
        v.end = v.age + 5.0 * tau;
    }
}

/// Starts the notes that are due at the current moment of the song.
fn tick(sh: &mut Shared) {
    let now = sh.from + sh.t;
    while sh.next < sh.events.len() && sh.events[sh.next].start <= now {
        let ev = sh.events[sh.next].clone();
        sh.next += 1;
        let gain = sh.gains.get(ev.hand.max(0) as usize).copied().unwrap_or(1.0);
        if sh.bank.is_empty() || ev.start < now - 0.05 || gain <= 0.0 || ev.start < sh.silent_until || ev.start >= sh.until {
            continue;
        }
        let best = sh.bank.iter().min_by_key(|s| (s.midi - ev.midi).abs()).unwrap();
        let hold = (ev.end - ev.start).max(0.3).min(KEEP_SECONDS - 1.0) / sh.speed;
        let v = Voice { data: best.data.clone(), pos: 0.0, step: 2f64.powf((ev.midi - best.midi) as f64 / 12.0) * best.rate / sh.sr, age: 0.0, hold, tau: 0.14, end: hold + 1.2, gain };
        if sh.voices.len() >= 96 {
            sh.voices.remove(0);
        }
        sh.voices.push(v);
    }
}

fn fill(sh: &mut Shared, out: &mut [f32], channels: usize) {
    let sr = sh.sr;
    let dt = 1.0 / sr;
    for frame in out.chunks_mut(channels) {
        if sh.playing {
            tick(sh);
            sh.t += dt * sh.speed;
        }
        let mut sum = 0.0f32;
        for v in sh.voices.iter_mut() {
            let i = v.pos as usize;
            if i + 1 >= v.data.len() || v.age >= v.end {
                v.age = f64::MAX;
                continue;
            }
            let f = (v.pos - i as f64) as f32;
            let s = v.data[i] * (1.0 - f) + v.data[i + 1] * f;
            let g = if v.age < v.hold { 0.9 } else { 0.9 * (-(v.age - v.hold) / v.tau).exp() };
            sum += s * g as f32 * v.gain;
            v.pos += v.step;
            v.age += dt;
        }
        let x = sum * 0.8;
        let y = if x.abs() > 0.8 { x.signum() * (0.8 + 0.2 * ((x.abs() - 0.8) / 0.2).tanh()) } else { x };
        frame.iter_mut().for_each(|s| *s = y);
    }
    sh.voices.retain(|v| v.age < f64::MAX);
}

impl Audio {
    /// Start the sound card and load the piano on a background thread. Safe to call more than once.
    pub fn get() -> &'static Audio {
        static A: OnceLock<Audio> = OnceLock::new();
        A.get_or_init(|| {
            let sh = Arc::new(Mutex::new(Shared::default()));
            let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (sh2, ready2) = (sh.clone(), ready.clone());
            std::thread::spawn(move || {
                use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
                let host = cpal::default_host();
                let Some(dev) = host.default_output_device() else { log::warn!("no sound device"); return };
                let Ok(cfg) = dev.default_output_config() else { return };
                let channels = cfg.channels() as usize;
                sh2.lock().unwrap().sr = cfg.sample_rate().0 as f64;
                let bank: Vec<Sample> = sample_names()
                    .into_iter()
                    .filter_map(|(midi, name)| {
                        let bytes = crate::server::asset(&format!("samples/piano/{name}.mp3"))?;
                        let (data, rate) = decode(bytes)?;
                        Some(Sample { midi, data: Arc::new(data), rate })
                    })
                    .collect();
                if bank.is_empty() {
                    log::warn!("piano samples missing: native playback off");
                    return;
                }
                sh2.lock().unwrap().bank = bank;
                let shared = sh2.clone();
                let stream = dev.build_output_stream(
                    &cfg.config(),
                    move |data: &mut [f32], _| {
                        if let Ok(mut s) = shared.try_lock() {
                            fill(&mut s, data, channels);
                        } else {
                            data.fill(0.0);
                        }
                    },
                    |e| log::warn!("audio: {e}"),
                    None,
                );
                match stream {
                    Ok(st) if st.play().is_ok() => {
                        ready2.store(true, std::sync::atomic::Ordering::SeqCst);
                        loop {
                            std::thread::park();
                        }
                    }
                    _ => log::warn!("could not open the sound device"),
                }
            });
            Audio { sh, ready }
        })
    }

    pub fn ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Play `notes` as `o` says; anything already sounding is damped quickly.
    pub fn play(&self, mut notes: Vec<NoteEv>, o: PlayOpts) {
        notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        let mut s = self.sh.lock().unwrap();
        start(&mut s, notes, &o);
    }

    pub fn stop(&self) {
        let mut s = self.sh.lock().unwrap();
        s.playing = false;
        damp_all(&mut s, 0.03);
    }

    /// Change settings while playing (see `Live`).
    pub fn set(&self, l: &Live) {
        let mut s = self.sh.lock().unwrap();
        apply(&mut s, l);
    }

    /// Where playback is.
    pub fn pos(&self) -> Pos {
        let s = self.sh.lock().unwrap();
        position(&s)
    }
}

fn clamp_speed(v: f64) -> f64 {
    v.clamp(0.1, 2.0)
}

fn start(s: &mut Shared, notes: Vec<NoteEv>, o: &PlayOpts) {
    damp_all(s, 0.02);
    s.next = notes.partition_point(|n| n.start < o.from - 0.02);
    s.events = notes;
    s.from = o.from;
    s.t = 0.0;
    s.speed = clamp_speed(o.speed);
    s.gains = o.gains;
    s.silent_until = o.silent_until;
    s.until = o.until;
    s.playing = true;
}

fn apply(s: &mut Shared, l: &Live) {
    s.gains = l.gains;
    let speed = clamp_speed(l.speed);
    if (speed - s.speed).abs() > 1e-9 {
        // carry on from where the music is
        s.from += s.t;
        s.t = 0.0;
        s.speed = speed;
    }
}

fn position(s: &Shared) -> Pos {
    Pos { pos: s.from + s.t, speed: s.speed, playing: s.playing }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_a_chord_without_clipping_and_goes_quiet_after() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/public/samples/piano");
        let bank: Vec<Sample> = sample_names()
            .into_iter()
            .filter_map(|(midi, name)| {
                let (data, rate) = decode(std::fs::read(dir.join(format!("{name}.mp3"))).ok()?)?;
                Some(Sample { midi, data: Arc::new(data), rate })
            })
            .collect();
        assert!(bank.len() > 20, "piano samples must be fetched (scripts/fetch-piano.sh)");
        let mut sh = Shared { sr: 48000.0, bank, playing: true, speed: 1.0, gains: [1.0, 1.0], ..Default::default() };
        sh.events = (0..12).map(|i| NoteEv { midi: 40 + i * 4, start: 0.0, end: 0.5, hand: 0 }).collect();
        let mut out = vec![0f32; 48000 * 2];
        fill(&mut sh, &mut out, 1);
        let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
        let early: f32 = out[..24000].iter().map(|v| v * v).sum::<f32>() / 24000.0;
        assert!(peak > 0.05 && peak <= 1.0, "peak {peak}");
        assert!(early > 1e-4, "audible at the start");
        let mut tail = vec![0f32; 48000 * 2];
        fill(&mut sh, &mut tail, 1);
        assert!(tail[48000..].iter().all(|v| v.abs() < 1e-3), "damped after the notes end");
    }

    // ---- speed and hands, with a one-note piano so no sample files are needed

    fn rig(notes: &[(f64, i32)], o: PlayOpts) -> Shared {
        let bank = vec![Sample { midi: 60, data: Arc::new(vec![0.5; 96000]), rate: 48000.0 }];
        let mut sh = Shared { sr: 48000.0, bank, ..Default::default() };
        let evs: Vec<NoteEv> = notes.iter().map(|&(t, hand)| NoteEv { midi: 60, start: t, end: t + 0.2, hand }).collect();
        start(&mut sh, evs, &o);
        sh
    }

    fn run(sh: &mut Shared, secs: f64) {
        let mut buf = vec![0f32; 48];
        for _ in 0..(secs * 1000.0) as usize {
            fill(sh, &mut buf, 1);
        }
    }

    #[test]
    fn slower_speed_stretches_time() {
        let mut sh = rig(&[(0.0, 0), (1.0, 0)], PlayOpts { speed: 0.5, ..Default::default() });
        run(&mut sh, 1.0);
        assert!((position(&sh).pos - 0.5).abs() < 0.01, "{:?}", position(&sh));
        assert_eq!(sh.next, 1, "the second note waits for its time");
        run(&mut sh, 1.05);
        assert!((position(&sh).pos - 1.025).abs() < 0.01);
        assert_eq!(sh.next, 2);
    }

    #[test]
    fn a_muted_hand_is_not_played() {
        let notes = [(0.0, 0), (0.25, 1), (0.5, 0), (0.75, 1)];
        let mut sh = rig(&notes, PlayOpts { gains: [1.0, 0.0], ..Default::default() });
        run(&mut sh, 0.1);
        assert_eq!(sh.voices.len(), 1, "only the first hand's note at 0.0 sounds");
        run(&mut sh, 1.0);
        assert_eq!(sh.next, 4, "the muted hand's notes were skipped, not stuck");
    }

    #[test]
    fn a_section_can_have_a_silent_lead_in_and_an_end() {
        let notes = [(0.0, 0), (1.0, 0), (2.0, 0), (3.0, 0)];
        let mut sh = rig(&notes, PlayOpts { silent_until: 1.0, until: 3.0, ..Default::default() });
        let mut started = 0;
        for _ in 0..(4.0 * 48000.0) as usize {
            let before = sh.voices.len();
            let mut out = [0.0f32; 1];
            fill(&mut sh, &mut out, 1);
            if sh.voices.len() > before { started += 1 }
        }
        assert_eq!(started, 2, "only the notes at 1 s and 2 s sound");
    }

    #[test]
    fn changing_speed_while_playing_carries_on_from_where_it_is() {
        let notes: Vec<(f64, i32)> = (0..8).map(|i| (i as f64 * 0.25, 0)).collect();
        let mut sh = rig(&notes, PlayOpts::default());
        run(&mut sh, 0.6);
        let before = position(&sh).pos;
        apply(&mut sh, &Live { speed: 0.5, gains: [1.0, 1.0] });
        assert!((position(&sh).pos - before).abs() < 1e-9, "no jump");
        run(&mut sh, 1.0);
        assert!((position(&sh).pos - (before + 0.5)).abs() < 0.01, "now at half speed");
    }
}
