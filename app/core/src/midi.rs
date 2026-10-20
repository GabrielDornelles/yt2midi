//! Standard MIDI files in and out. Written files use 960 ticks per second so a tick is real time.
use crate::analyze::Note;
use midly::{num::*, Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use std::path::Path;

const TICKS_PER_SEC: f64 = 960.0;

pub fn write(notes: &[Note], path: &Path) -> Result<(), String> {
    let mut hands: Vec<i32> = notes.iter().map(|n| n.hand).collect();
    hands.sort_unstable();
    hands.dedup();
    if hands.is_empty() {
        hands.push(0);
    }
    let mut smf = Smf::new(Header::new(Format::Parallel, Timing::Metrical(u15::new(480))));
    let names: Vec<String> = hands.iter().map(|h| format!("hand {}", h + 1)).collect();
    for (hi, &h) in hands.iter().enumerate() {
        let mut ev: Vec<(i64, u8, u8)> = vec![]; // (tick, on?, key)
        for n in notes.iter().filter(|n| n.hand == h) {
            ev.push(((n.start * TICKS_PER_SEC).round_ties_even() as i64, 1, n.midi as u8));
            ev.push(((n.end * TICKS_PER_SEC).round_ties_even() as i64, 0, n.midi as u8));
        }
        ev.sort(); // note-offs sort before note-ons at the same tick
        let mut tr: Vec<TrackEvent> = vec![
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500_000))) },
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(names[hi].as_bytes())) },
        ];
        let mut last = 0i64;
        for (t, on, key) in ev {
            tr.push(TrackEvent {
                delta: u28::new((t - last).max(0) as u32),
                kind: TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: if on == 1 { MidiMessage::NoteOn { key: u7::new(key), vel: u7::new(80) } } else { MidiMessage::NoteOff { key: u7::new(key), vel: u7::new(0) } },
                },
            });
            last = t;
        }
        tr.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
        smf.tracks.push(tr);
    }
    let mut buf = Vec::new();
    smf.write_std(&mut buf).map_err(|e| e.to_string())?;
    std::fs::write(path, buf).map_err(|e| e.to_string())?;
    let json: Vec<_> = notes.iter().map(|n| n.to_json()).collect();
    std::fs::write(path.with_extension("json"), serde_json::to_string(&json).unwrap()).map_err(|e| e.to_string())
}

/// A note as the sheet needs it.
#[derive(Clone, Debug)]
pub struct SheetNote {
    pub midi: i32,
    pub start: f64,
    pub hand: i32,
    pub end: f64,
}

pub fn load(path: &Path) -> Result<Vec<SheetNote>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let smf = Smf::parse(&data).map_err(|e| format!("That is not a valid MIDI file: {e}"))?;
    let tpb = match smf.header.timing {
        Timing::Metrical(t) => t.as_int() as f64,
        Timing::Timecode(fps, sub) => fps.as_f32() as f64 * sub as f64 * 0.5, // rare; treat 2 beats per second
    };
    let tempo = smf
        .tracks
        .iter()
        .flatten()
        .find_map(|e| if let TrackEventKind::Meta(MetaMessage::Tempo(t)) = e.kind { Some(t.as_int() as f64) } else { None })
        .unwrap_or(500_000.0);
    let sec = |ticks: u64| ticks as f64 * tempo / 1e6 / tpb;
    let mut notes: Vec<SheetNote> = Vec::new();
    let mut hand = 0;
    for t in &smf.tracks {
        let (mut ticks, mut found) = (0u64, false);
        let mut held: std::collections::HashMap<u8, usize> = Default::default();
        for e in t {
            ticks += e.delta.as_int() as u64;
            if let TrackEventKind::Midi { message, .. } = e.kind {
                match message {
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        notes.push(SheetNote { midi: key.as_int() as i32, start: sec(ticks), hand, end: sec(ticks) });
                        held.insert(key.as_int(), notes.len() - 1);
                        found = true;
                    }
                    MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                        if let Some(i) = held.remove(&key.as_int()) {
                            notes[i].end = sec(ticks);
                        }
                    }
                    _ => {}
                }
            }
        }
        hand += found as i32;
    }
    notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap().then(a.midi.cmp(&b.midi)));
    Ok(notes)
}
