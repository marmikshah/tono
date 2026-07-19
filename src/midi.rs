//! MIDI export: write a document's `seq` compositions to a Standard MIDI File,
//! so a melody/drum pattern written in tono can round-trip into a DAW.
//! Read-only and additive — it never touches the audio render.
//!
//! Each `seq` becomes one MIDI track; notes map by their `(step, len)` on a
//! 480-PPQ grid (`steps_per_beat` steps to the quarter). A single global tempo
//! (the first seq's `bpm`) is written — multi-tempo documents are retimed to it.

use std::path::Path;

use anyhow::Result;
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};
use tono_core::dsl::{Modulator, Node, SeqNote, SeqWave, SoundDoc, Value, note_to_hz};

const PPQ: u16 = 480;

/// What [`export_midi`] wrote.
pub struct MidiSummary {
    /// MIDI tracks written (one per seq).
    pub tracks: usize,
    /// Total notes written.
    pub notes: usize,
}

struct SeqRef<'a> {
    bpm: f32,
    spb: u32,
    notes: &'a [SeqNote],
    /// Kit seqs land on MIDI channel 10 (the GM percussion channel), so a DAW
    /// plays them as drums instead of pitched notes.
    drums: bool,
}

/// Write every `seq` in `doc` as a MIDI track to `dest`.
pub fn export_midi(doc: &SoundDoc, dest: &Path) -> Result<MidiSummary> {
    let mut seqs = Vec::new();
    collect_seqs(&doc.root, &mut seqs);
    if seqs.is_empty() {
        anyhow::bail!(
            "no seq nodes to export — MIDI export needs at least one seq (a melody or drum pattern)"
        );
    }
    let mut smf = Smf::new(Header::new(Format::Parallel, Timing::Metrical(PPQ.into())));
    let us_per_qn = (60_000_000.0 / seqs[0].bpm.max(1.0)) as u32;
    let mut total = 0usize;
    for (i, s) in seqs.iter().enumerate() {
        let (track, n) = seq_track(s, (i == 0).then_some(us_per_qn));
        total += n;
        smf.tracks.push(track);
    }
    smf.save(dest)?;
    Ok(MidiSummary {
        tracks: seqs.len(),
        notes: total,
    })
}

fn collect_seqs<'a>(node: &'a Node, out: &mut Vec<SeqRef<'a>>) {
    if let Node::Seq {
        bpm,
        steps_per_beat,
        notes,
        wave,
        sf2,
        ..
    } = node
    {
        out.push(SeqRef {
            bpm: *bpm,
            spb: (*steps_per_beat).max(1),
            notes,
            drums: *wave == SeqWave::Kit || (*wave == SeqWave::Sampler && sf2.sf2_bank == 128),
        });
    }
    node.children().for_each(|c| collect_seqs(c, out));
}

/// Build one MIDI track from a seq. `tempo` (if `Some`) writes the global tempo.
fn seq_track(s: &SeqRef, tempo: Option<u32>) -> (Track<'static>, usize) {
    // Ticks from the absolute step, rounded per event — a truncated per-step
    // tick count would drift cumulatively for steps_per_beat values that do
    // not divide the PPQ (e.g. septuplets). Clamped to the MIDI u28 max: a
    // pathological step would otherwise wrap the wire format's delta times.
    let tick = |step: u32| {
        ((step as u64 * PPQ as u64 + s.spb as u64 / 2) / s.spb as u64).min(0x0FFF_FFFF) as u32
    };
    // (absolute tick, is_note_on, key, velocity). Note-offs sort before
    // note-ons at the same tick so a zero-length gap re-strikes cleanly.
    let mut events: Vec<(u32, bool, u8, u8)> = Vec::with_capacity(s.notes.len() * 2);
    for note in s.notes {
        let key = pitch_to_midi(&note.pitch);
        // MIDI velocity is the lossless carrier for the note's gain.
        let vel = (note.gain * 127.0).round().clamp(1.0, 127.0) as u8;
        events.push((tick(note.step), true, key, vel));
        events.push((
            tick(note.step.saturating_add(note.len.max(1))),
            false,
            key,
            0,
        ));
    }
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let channel = if s.drums { 9 } else { 0 };
    let mut track = Track::new();
    if let Some(us) = tempo {
        track.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(us.into())),
        });
    }
    let (mut last, mut n) = (0u32, 0usize);
    for (tick, is_on, key, vel) in events {
        let delta = tick - last;
        last = tick;
        let message = if is_on {
            n += 1;
            MidiMessage::NoteOn {
                key: key.into(),
                vel: vel.into(),
            }
        } else {
            MidiMessage::NoteOff {
                key: key.into(),
                vel: 0.into(),
            }
        };
        track.push(TrackEvent {
            delta: delta.into(),
            kind: TrackEventKind::Midi {
                channel: channel.into(),
                message,
            },
        });
    }
    track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    (track, n)
}

/// A seq note's pitch → MIDI note number. Modulated pitches use a representative
/// value (a slide's start, an arp's first step, …).
fn pitch_to_midi(v: &Value) -> u8 {
    let hz = match v {
        Value::Const(c) => *c,
        Value::Note(s) => note_to_hz(s).unwrap_or(440.0),
        Value::Modulated(m) => representative_hz(m),
    };
    if hz <= 0.0 {
        return 60;
    }
    tono_core::dsp::hz_to_midi(hz).round().clamp(0.0, 127.0) as u8
}

fn representative_hz(m: &Modulator) -> f32 {
    match m {
        Modulator::Slide { from, .. } => *from,
        Modulator::Lfo { center, .. } => *center,
        Modulator::Arp { steps, .. } => steps.first().copied().unwrap_or(440.0),
        Modulator::EnvMod { from, .. } => *from,
        Modulator::Rand { from, to, .. } => 0.5 * (from + to),
        // Modulator is non_exhaustive: a future modulator exports as A4 until
        // a representative is chosen for it.
        _ => 440.0,
    }
}

/// What [`import_midi`] read.
pub struct ImportSummary {
    /// tono tracks produced (one per MIDI track with notes).
    pub tracks: usize,
    /// Total notes imported.
    pub notes: usize,
    /// Tempo used, beats per minute.
    pub bpm: f32,
}

/// One decoded MIDI note, in absolute ticks.
struct RawNote {
    tick_on: u64,
    tick_off: u64,
    key: u8,
    velocity: u8,
    /// GM percussion channel (10) — becomes the `kit` voice.
    drums: bool,
    /// GM program active at note-on, for voice mapping.
    program: u8,
}

/// Read a Standard MIDI File into a renderable `tracks` [`SoundDoc`] of `seq`
/// nodes (plus an [`ImportSummary`]) — the inverse of [`export_midi`],
/// quantized onto the seq grid.
///
/// Mapping: the first tempo event sets one global `bpm` (later tempo changes
/// are retimed to it); channel 10 becomes the `kit` voice; melodic tracks map
/// their GM program family onto the closest built-in voice (piano / epiano /
/// organ / strings / bass / pluck, anything else `square`); velocities become
/// note `gain`s. Timing quantizes to `steps_per_beat` grid steps.
pub fn import_midi(src: &Path, steps_per_beat: u32) -> Result<(SoundDoc, ImportSummary)> {
    let bytes = std::fs::read(src)?;
    let smf = Smf::parse(&bytes)?;
    let ppq = match smf.header.timing {
        Timing::Metrical(t) => u16::from(t) as u64,
        Timing::Timecode(..) => {
            anyhow::bail!(
                "SMPTE-timecode MIDI files are not supported — re-export with metrical (PPQ) timing"
            )
        }
    };
    let spb = steps_per_beat.max(1);

    // First tempo event anywhere in the file wins (format-1 files keep the
    // tempo map on track 0); default 120 bpm per the MIDI spec.
    let mut us_per_qn = 500_000u32;
    'tempo: for track in &smf.tracks {
        let mut at = 0u64;
        for ev in track {
            at += u32::from(ev.delta) as u64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(us)) = ev.kind {
                us_per_qn = u32::from(us);
                break 'tempo;
            }
            // Only accept the tempo that is in force from the top.
            if at > 0 {
                break;
            }
        }
    }
    let bpm = 60_000_000.0 / us_per_qn.max(1) as f32;

    // Decode each MIDI track's notes (running program changes per channel).
    let mut song_tracks: Vec<Vec<RawNote>> = Vec::new();
    for track in &smf.tracks {
        let mut at = 0u64;
        let mut program = [0u8; 16];
        // (channel, key) → (tick_on, velocity, program) for open notes.
        let mut open: std::collections::HashMap<(u8, u8), (u64, u8, u8)> =
            std::collections::HashMap::new();
        let mut notes: Vec<RawNote> = Vec::new();
        for ev in track {
            at += u32::from(ev.delta) as u64;
            let TrackEventKind::Midi { channel, message } = ev.kind else {
                continue;
            };
            let ch = u8::from(channel);
            match message {
                MidiMessage::ProgramChange { program: p } => program[ch as usize] = u8::from(p),
                MidiMessage::NoteOn { key, vel } if u8::from(vel) > 0 => {
                    open.insert(
                        (ch, u8::from(key)),
                        (at, u8::from(vel), program[ch as usize]),
                    );
                }
                // NoteOn vel 0 is the wire-efficient NoteOff.
                MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                    if let Some((tick_on, velocity, prog)) = open.remove(&(ch, u8::from(key))) {
                        notes.push(RawNote {
                            tick_on,
                            tick_off: at,
                            key: u8::from(key),
                            velocity,
                            drums: ch == 9,
                            program: prog,
                        });
                    }
                }
                _ => {}
            }
        }
        if !notes.is_empty() {
            song_tracks.push(notes);
        }
    }
    if song_tracks.is_empty() {
        anyhow::bail!("no notes found in {}", src.display());
    }

    // Quantize onto the seq grid and build one seq node per MIDI track.
    let tick_to_step = |tick: u64| -> u32 {
        ((tick * spb as u64 + ppq / 2) / ppq.max(1)).min(u32::MAX as u64) as u32
    };
    let mut tracks_json = Vec::new();
    let mut total_notes = 0usize;
    let mut end_step = 0u32;
    for (i, notes) in song_tracks.iter().enumerate() {
        let drums = notes.iter().any(|n| n.drums);
        let wave = if drums {
            "kit"
        } else {
            voice_for_program(notes[0].program)
        };
        let mut seq_notes = Vec::with_capacity(notes.len());
        for n in notes {
            let step = tick_to_step(n.tick_on);
            let len = (tick_to_step(n.tick_off).saturating_sub(step)).max(1);
            end_step = end_step.max(step.saturating_add(len));
            total_notes += 1;
            seq_notes.push(serde_json::json!({
                "step": step,
                "len": len,
                "pitch": format!("midi:{}", n.key),
                "gain": (n.velocity as f32 / 127.0).clamp(0.05, 1.0),
            }));
        }
        // Sustained-friendly default envelope; the kit ignores pitch/holds.
        let env = if drums {
            serde_json::json!({ "s": 1.0 })
        } else {
            serde_json::json!({ "a": 0.005, "s": 0.8, "r": 0.15 })
        };
        tracks_json.push(serde_json::json!({
            "id": format!("track_{i}"),
            "node": {
                "type": "seq",
                "bpm": bpm,
                "steps_per_beat": spb,
                "wave": wave,
                "env": env,
                "notes": seq_notes,
            }
        }));
    }

    let tracks_json_len = tracks_json.len();
    let sec_per_step = 60.0 / (bpm.max(1.0) * spb as f32);
    let duration = end_step as f32 * sec_per_step + 2.0; // release/reverb tail
    let name = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported")
        .to_string();
    let doc: SoundDoc = serde_json::from_value(serde_json::json!({
        "name": name,
        "duration": duration,
        "engine": tono_core::dsl::ENGINE_VERSION,
        "root": { "type": "tracks", "tracks": tracks_json },
    }))?;
    doc.validate().map_err(|e| anyhow::anyhow!(e))?;
    Ok((
        doc,
        ImportSummary {
            tracks: tracks_json_len,
            notes: total_notes,
            bpm,
        },
    ))
}

/// Map a GM program number onto the closest built-in seq voice.
fn voice_for_program(program: u8) -> &'static str {
    match program {
        4..=5 => "epiano",    // electric pianos
        0..=7 => "piano",     // the acoustic rest of the piano family
        8..=15 => "fm",       // chromatic percussion → FM mallets
        16..=23 => "organ",   // organs
        24..=31 => "pluck",   // guitars
        32..=39 => "bass",    // basses
        40..=55 => "strings", // strings / ensemble / choir
        _ => "square",        // honest chiptune fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_seq_notes_to_a_parsable_midi_file() {
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"m", "duration":2.0, "root":{ "type":"seq", "bpm":120,
              "steps_per_beat":4, "wave":"square", "env":{"a":0.005,"d":0.1,"s":0.3,"r":0.05},
              "notes":[ {"step":0,"len":2,"pitch":"C4"}, {"step":2,"len":2,"pitch":"E4"},
                        {"step":4,"len":4,"pitch":"G4"} ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.mid");
        let s = export_midi(&doc, &path).unwrap();
        assert_eq!(s.tracks, 1);
        assert_eq!(s.notes, 3, "three notes written");

        // Re-parse: the file is a valid SMF with three note-ons.
        let bytes = std::fs::read(&path).unwrap();
        let smf = Smf::parse(&bytes).unwrap();
        assert_eq!(smf.tracks.len(), 1);
        let note_ons = smf.tracks[0]
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    TrackEventKind::Midi {
                        message: MidiMessage::NoteOn { vel, .. },
                        ..
                    } if vel > 0
                )
            })
            .count();
        assert_eq!(note_ons, 3, "round-trips to three note-ons");
    }

    #[test]
    fn velocity_channel_and_ticks_are_faithful() {
        // gain → velocity (the lossless carrier), kit → channel 10, and
        // non-divisor steps_per_beat must not drift: at 7 steps per beat,
        // step 7 is exactly one quarter note = 480 ticks.
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"d", "duration":2.0, "root":{ "type":"seq", "bpm":120,
              "steps_per_beat":7, "wave":"kit", "env":{"a":0.001,"d":0.1,"s":0.0,"r":0.05},
              "notes":[ {"step":0,"len":1,"pitch":"midi:36","gain":0.5},
                        {"step":7,"len":1,"pitch":"midi:38"} ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("drums.mid");
        export_midi(&doc, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let smf = Smf::parse(&bytes).unwrap();
        let mut ons = Vec::new();
        let mut at = 0u32;
        for e in &smf.tracks[0] {
            at += u32::from(e.delta);
            if let TrackEventKind::Midi { channel, message } = e.kind
                && let MidiMessage::NoteOn { vel, .. } = message
            {
                ons.push((at, u8::from(channel), u8::from(vel)));
            }
        }
        assert_eq!(ons.len(), 2);
        assert_eq!(ons[0], (0, 9, 64), "gain 0.5 → vel 64 on the drum channel");
        assert_eq!(ons[1].0, 480, "step 7 of 7/beat lands exactly on the beat");
    }

    #[test]
    fn no_seq_is_an_error() {
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"x", "duration":0.2, "root":{"type":"sine","freq":440} }"#,
        )
        .unwrap();
        assert!(export_midi(&doc, std::path::Path::new("/tmp/none.mid")).is_err());
    }

    #[test]
    fn import_round_trips_an_exported_file() {
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"rt", "duration":2.0, "root":{ "type":"seq", "bpm":120,
              "steps_per_beat":4, "wave":"square", "env":{"a":0.005,"d":0.1,"s":0.3,"r":0.05},
              "notes":[ {"step":0,"len":2,"pitch":"C4","gain":0.9},
                        {"step":2,"len":2,"pitch":"E4","gain":0.5},
                        {"step":4,"len":4,"pitch":"G4","gain":1.0} ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rt.mid");
        export_midi(&doc, &path).unwrap();

        let (imported, summary) = import_midi(&path, 4).unwrap();
        assert_eq!(summary.tracks, 1);
        assert_eq!(summary.notes, 3);
        assert!((summary.bpm - 120.0).abs() < 0.01, "tempo survives");
        imported.validate().expect("imported doc is renderable");

        // The notes come back on the same grid with the same pitches.
        let Node::Tracks { tracks, .. } = &imported.root else {
            panic!("tracks root");
        };
        let Node::Seq { notes, .. } = &tracks[0].node else {
            panic!("seq node");
        };
        let got: Vec<(u32, u32, String)> = notes
            .iter()
            .map(|n| {
                let Value::Note(p) = &n.pitch else { panic!() };
                (n.step, n.len, p.clone())
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (0, 2, "midi:60".into()),
                (2, 2, "midi:64".into()),
                (4, 4, "midi:67".into()),
            ]
        );
    }

    #[test]
    fn import_maps_gm_percussion_to_the_kit() {
        // A one-track file on channel 10 must come back as the kit voice.
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"drums", "duration":2.0, "root":{ "type":"seq", "bpm":100,
              "steps_per_beat":4, "wave":"kit", "env":{"s":1},
              "notes":[ {"step":0,"len":2,"pitch":"midi:36"},
                        {"step":4,"len":2,"pitch":"midi:38"} ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gm_kit.mid");
        export_midi(&doc, &path).unwrap();

        let (imported, _) = import_midi(&path, 4).unwrap();
        let Node::Tracks { tracks, .. } = &imported.root else {
            panic!("tracks root");
        };
        assert!(
            matches!(
                &tracks[0].node,
                Node::Seq {
                    wave: SeqWave::Kit,
                    ..
                }
            ),
            "channel 10 → kit"
        );
    }
}

#[cfg(test)]
mod duck_and_bounds_tests {
    use super::*;

    #[test]
    fn exports_a_seq_inside_a_duck_trigger() {
        // The duck trigger is where the kick pattern lives — a doc whose only
        // seq is a trigger must not export a note-less file.
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"pump", "duration":2.0, "root":{ "type":"chain", "stages":[
              { "type":"sawtooth", "freq":110 },
              { "type":"duck", "amount":0.8,
                "trigger": { "type":"seq", "bpm":120, "steps_per_beat":4, "wave":"kit",
                  "env":{"s":1},
                  "notes":[ {"step":0,"len":2,"pitch":"midi:36"} ] } } ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pump.mid");
        let s = export_midi(&doc, &path).unwrap();
        assert_eq!(s.notes, 1, "the duck trigger's seq exported");
    }

    #[test]
    fn pathological_step_values_never_panic() {
        // A near-u32::MAX step must saturate, not overflow or wrap the export.
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"big", "duration":2.0, "root":{ "type":"seq", "bpm":120,
              "steps_per_beat":4, "wave":"square", "env":{"s":1},
              "notes":[ {"step":4294967290,"len":10,"pitch":"C4"} ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.mid");
        export_midi(&doc, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        Smf::parse(&bytes).unwrap(); // a valid file comes back out
    }
}
