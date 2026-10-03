//! MIDI export: write a document's `seq` compositions to a Standard MIDI File,
//! so a melody/drum pattern written in tono can round-trip into a DAW.
//! Read-only and additive — it never touches the audio render.
//!
//! Each `seq` becomes one MIDI track; notes map by their `(step, len)` on a
//! 480-PPQ grid (`steps_per_beat` steps to the quarter). A single global tempo
//! (the first seq's `bpm`) is written — multi-tempo documents are retimed to it.
//!
//! Songs round-trip through the same machinery: [`import_midi_song`] reads an
//! SMF straight into a [`Song`] (notes land directly on the tracks) and
//! [`export_song_midi`] lowers a `Song` through [`Song::to_doc`] and exports
//! the resulting document.

use std::path::Path;

use anyhow::Result;
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};
use tono_core::dsl::{Adsr, Modulator, Node, SeqNote, SeqWave, SoundDoc, Value, note_to_hz};
use tono_core::song::Song;

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
    let global_bpm = seqs[0].bpm.max(1.0);
    let us_per_qn = (60_000_000.0 / global_bpm) as u32;
    let mut total = 0usize;
    for (i, s) in seqs.iter().enumerate() {
        let (track, n) = seq_track(s, (i == 0).then_some(us_per_qn), global_bpm);
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
/// `global_bpm` is the file's single tempo: a seq at any other bpm is retimed
/// so its notes keep their ABSOLUTE time (tick = step × PPQ × global/seq bpm
/// ÷ steps-per-beat), not their grid position.
fn seq_track(s: &SeqRef, tempo: Option<u32>, global_bpm: f32) -> (Track<'static>, usize) {
    // Ticks from the absolute step, rounded per event — a truncated per-step
    // tick count would drift cumulatively for steps_per_beat values that do
    // not divide the PPQ (e.g. septuplets). Clamped to the MIDI u28 max: a
    // pathological step would otherwise wrap the wire format's delta times.
    let retime = (global_bpm / s.bpm.max(1e-6)) as f64;
    let tick = |step: u32| {
        ((step as f64 * PPQ as f64 * retime / s.spb as f64 + 0.5).floor() as u64).min(0x0FFF_FFFF)
            as u32
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

/// Write a [`Song`] as a Standard MIDI File: the song lowers through
/// [`Song::to_doc`] and the resulting document's seqs export exactly like
/// [`export_midi`] — one MIDI track per song track, kits on channel 10, the
/// song's `bpm` as the single global tempo.
pub fn export_song_midi(song: &Song, out: &Path) -> Result<MidiSummary> {
    let doc = song
        .to_doc()
        .map_err(|e| anyhow::anyhow!("compiling song '{}' for MIDI export: {e}", song.name))?;
    export_midi(&doc, out)
}

/// What [`import_midi`] read.
pub struct ImportSummary {
    /// tono tracks produced (one per MIDI track/channel/program group).
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
    /// MIDI channel. Channel 10 becomes the `kit` voice; other channels stay
    /// separate so Format-0 files do not collapse a whole arrangement.
    channel: u8,
    /// GM program active at note-on, for voice mapping.
    program: u8,
}

struct OpenNote {
    tick_on: u64,
    velocity: u8,
    program: u8,
}

/// One imported MIDI track, quantized onto the step grid with its voice
/// already mapped — the shared output of both importers.
struct ImportedTrack {
    /// The mapped voice: `kit` for a channel-10 track, else the GM program
    /// family of the track's first note.
    wave: SeqWave,
    /// A sustained-friendly default envelope; the kit ignores pitch/holds.
    env: Adsr,
    /// The quantized notes, pitch carried as `"midi:N"`.
    notes: Vec<SeqNote>,
}

/// Read a Standard MIDI File and quantize every track/channel/program group
/// that has notes onto the `steps_per_beat` grid — the machinery [`import_midi`]
/// (which wraps the groups as a renderable [`SoundDoc`]) and
/// [`import_midi_song`] (which wraps them as a [`Song`]) share.
///
/// Mapping: the first tempo event anywhere in the file sets one global `bpm`
/// (later tempo changes are retimed to it); channel 10 becomes the `kit`
/// voice; melodic tracks map their GM program family onto the closest built-in
/// voice (piano / epiano / organ / strings / bass / pluck, anything else
/// `square`); velocities become note `gain`s.
fn read_midi_tracks(src: &Path, steps_per_beat: u32) -> Result<(f32, Vec<ImportedTrack>)> {
    let bytes = std::fs::read(src)?;
    // Reject unsupported timing before midly decodes its signed frame byte.
    if bytes.starts_with(b"MThd") && bytes.get(12).is_some_and(|division| division & 0x80 != 0) {
        anyhow::bail!(
            "SMPTE-timecode MIDI files are not supported — re-export with metrical (PPQ) timing"
        );
    }
    // Contain malformed input rejected by the parser, including RIFF-wrapped MIDI.
    let smf = std::panic::catch_unwind(|| Smf::parse(&bytes))
        .map_err(|_| anyhow::anyhow!("invalid MIDI file"))??;
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

    // Decode each MIDI track's notes (running program changes per channel),
    // then split Format-0/multi-channel tracks into single-voice groups.
    let mut song_tracks: Vec<Vec<RawNote>> = Vec::new();
    for track in &smf.tracks {
        let mut at = 0u64;
        let mut program = [0u8; 16];
        // MIDI permits repeated NoteOn messages for the same channel/key
        // before their corresponding NoteOffs. Keep every open voice FIFO.
        let mut open: std::collections::HashMap<(u8, u8), std::collections::VecDeque<OpenNote>> =
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
                    open.entry((ch, u8::from(key)))
                        .or_default()
                        .push_back(OpenNote {
                            tick_on: at,
                            velocity: u8::from(vel),
                            program: program[ch as usize],
                        });
                }
                // NoteOn vel 0 is the wire-efficient NoteOff.
                MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                    let open_key = (ch, u8::from(key));
                    let closed = open
                        .get_mut(&open_key)
                        .and_then(|voices| voices.pop_front());
                    if open.get(&open_key).is_some_and(|voices| voices.is_empty()) {
                        open.remove(&open_key);
                    }
                    if let Some(closed) = closed {
                        notes.push(RawNote {
                            tick_on: closed.tick_on,
                            tick_off: at,
                            key: u8::from(key),
                            velocity: closed.velocity,
                            channel: ch,
                            program: closed.program,
                        });
                    }
                }
                _ => {}
            }
        }
        let mut groups: std::collections::BTreeMap<(u8, u8), Vec<RawNote>> =
            std::collections::BTreeMap::new();
        for note in notes {
            // A drum channel has one fixed voice; melodic program changes are
            // separate groups because a tono track has one fixed voice too.
            let program_key = if note.channel == 9 { 0 } else { note.program };
            groups
                .entry((note.channel, program_key))
                .or_default()
                .push(note);
        }
        song_tracks.extend(groups.into_values());
    }
    if song_tracks.is_empty() {
        anyhow::bail!("no notes found in {}", src.display());
    }

    // Quantize onto the step grid and map each track's voice.
    let tick_to_step = |tick: u64| -> u32 {
        (tick.saturating_mul(spb as u64).saturating_add(ppq / 2) / ppq.max(1)).min(u32::MAX as u64)
            as u32
    };
    let mut tracks = Vec::with_capacity(song_tracks.len());
    for notes in &song_tracks {
        let drums = notes[0].channel == 9;
        let wave = if drums {
            SeqWave::Kit
        } else {
            voice_for_program(notes[0].program)
        };
        // Sustained-friendly default envelope; the kit ignores pitch/holds.
        let env = if drums {
            Adsr {
                s: 1.0,
                ..Adsr::default()
            }
        } else {
            Adsr {
                a: 0.005,
                s: 0.8,
                r: 0.15,
                ..Adsr::default()
            }
        };
        let mut seq_notes = Vec::with_capacity(notes.len());
        for n in notes {
            let step = tick_to_step(n.tick_on);
            let len = (tick_to_step(n.tick_off).saturating_sub(step)).max(1);
            seq_notes.push(SeqNote {
                step,
                len,
                pitch: Value::Note(format!("midi:{}", n.key)),
                gain: (n.velocity as f32 / 127.0).clamp(0.05, 1.0),
            });
        }
        tracks.push(ImportedTrack {
            wave,
            env,
            notes: seq_notes,
        });
    }
    Ok((bpm, tracks))
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
    let spb = steps_per_beat.max(1);
    let (bpm, tracks) = read_midi_tracks(src, spb)?;

    // Build one seq node per imported track.
    let mut tracks_json = Vec::new();
    let mut total_notes = 0usize;
    let mut end_step = 0u32;
    for (i, t) in tracks.iter().enumerate() {
        total_notes += t.notes.len();
        for n in &t.notes {
            end_step = end_step.max(n.step.saturating_add(n.len.max(1)));
        }
        tracks_json.push(serde_json::json!({
            "id": format!("track_{i}"),
            "node": {
                "type": "seq",
                "bpm": bpm,
                "steps_per_beat": spb,
                "wave": t.wave,
                "env": t.env,
                "notes": t.notes,
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

/// Read a Standard MIDI File into a [`Song`] — one song track per MIDI
/// track/channel/program group, with notes written directly onto the track (no
/// pattern recovery: patterns are an authoring convenience, not something to
/// reverse-engineer out of a flat file). The inverse of [`export_song_midi`].
///
/// The mapping is [`import_midi`]'s exactly (shared machinery): the file's
/// first tempo event sets `song.bpm` (later tempo changes are retimed to
/// it), channel 10 becomes a `kit` track, melodic tracks map their GM program
/// family onto the closest built-in voice, velocities become note `gain`s,
/// and timing quantizes onto the `steps_per_beat` grid. Tracks are named
/// `track_0`, `track_1`, … in file order; the song takes the file's stem.
pub fn import_midi_song(src: &Path, steps_per_beat: u32) -> Result<Song> {
    let spb = steps_per_beat.max(1);
    let (bpm, tracks) = read_midi_tracks(src, spb)?;
    let name = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported")
        .to_string();
    let mut song = Song::new(name, bpm);
    song.steps_per_beat = spb;
    for (i, t) in tracks.into_iter().enumerate() {
        song.add_track(format!("track_{i}"), t.wave, t.env);
        song.tracks[i].notes = t.notes;
    }
    Ok(song)
}

/// Map a GM program number onto the closest built-in seq voice.
fn voice_for_program(program: u8) -> SeqWave {
    match program {
        4..=5 => SeqWave::Epiano,    // electric pianos
        0..=7 => SeqWave::Piano,     // the acoustic rest of the piano family
        8..=15 => SeqWave::Fm,       // chromatic percussion → FM mallets
        16..=23 => SeqWave::Organ,   // organs
        24..=31 => SeqWave::Pluck,   // guitars
        32..=39 => SeqWave::Bass,    // basses
        40..=55 => SeqWave::Strings, // strings / ensemble / choir
        _ => SeqWave::Square,        // honest chiptune fallback
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
    fn multi_tempo_seqs_are_retimed_to_the_global_tempo() {
        // One file, one tempo (the first seq's 120 bpm): the 60 bpm seq's
        // notes must keep their ABSOLUTE time — its beat (step 4) is one full
        // second, i.e. two quarter notes at the file tempo = tick 960, not 480.
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"mt", "duration":4.0, "version":2, "root":{ "type":"tracks",
              "tracks":[
                { "id":"a", "node": { "type":"seq", "bpm":120, "steps_per_beat":4,
                    "wave":"square", "env":{"s":1},
                    "notes":[ {"step":4,"len":1,"pitch":"C4"} ] } },
                { "id":"b", "node": { "type":"seq", "bpm":60, "steps_per_beat":4,
                    "wave":"square", "env":{"s":1},
                    "notes":[ {"step":4,"len":1,"pitch":"E4"} ] } }
              ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mt.mid");
        export_midi(&doc, &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let smf = Smf::parse(&bytes).unwrap();
        let first_on_at = |ti: usize| {
            let mut at = 0u32;
            for e in &smf.tracks[ti] {
                at += u32::from(e.delta);
                if matches!(
                    e.kind,
                    TrackEventKind::Midi {
                        message: MidiMessage::NoteOn { .. },
                        ..
                    }
                ) {
                    return at;
                }
            }
            panic!("track {ti} has no note-on");
        };
        assert_eq!(first_on_at(0), 480, "the 120 bpm seq keeps the grid");
        assert_eq!(first_on_at(1), 960, "the 60 bpm seq is retimed, not slowed");
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

#[cfg(test)]
mod song_tests {
    use super::*;
    use tono_core::song::{note, note_vel};

    fn amp() -> Adsr {
        Adsr {
            a: 0.005,
            d: 0.1,
            s: 0.8,
            r: 0.2,
            punch: 0.0,
        }
    }

    /// A two-track song with notes written directly onto the tracks.
    fn demo_song() -> Song {
        let mut song = Song::new("demo", 120.0);
        song.add_track("keys", SeqWave::Square, amp());
        song.tracks[0].notes = vec![
            note_vel(0, 2, "midi:60", 1.0),
            note_vel(2, 2, "midi:64", 1.0),
            note_vel(4, 4, "midi:67", 1.0),
        ];
        song.add_track("drums", SeqWave::Kit, amp());
        song.tracks[1].notes = vec![note(0, 2, "midi:36"), note(4, 2, "midi:38")];
        song
    }

    fn steps_lens_pitches(t: &tono_core::song::SongTrack) -> Vec<(u32, u32, String)> {
        t.notes
            .iter()
            .map(|n| {
                let Value::Note(p) = &n.pitch else { panic!() };
                (n.step, n.len, p.clone())
            })
            .collect()
    }

    #[test]
    fn song_round_trips_through_midi() {
        let song = demo_song();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song_rt.mid");
        let s = export_song_midi(&song, &path).unwrap();
        assert_eq!(s.tracks, 2);
        assert_eq!(s.notes, 5);

        let back = import_midi_song(&path, 4).unwrap();
        assert_eq!(back.tracks.len(), 2);
        assert!((back.bpm - 120.0).abs() < 0.01, "tempo survives");
        assert_eq!(back.steps_per_beat, 4);
        // The notes come back on the same grid with the same pitches.
        assert_eq!(
            steps_lens_pitches(&back.tracks[0]),
            vec![
                (0, 2, "midi:60".into()),
                (2, 2, "midi:64".into()),
                (4, 4, "midi:67".into()),
            ]
        );
        assert_eq!(
            steps_lens_pitches(&back.tracks[1]),
            vec![(0, 2, "midi:36".into()), (4, 2, "midi:38".into())]
        );
        // The wave/program mapping survives: a kit track stays a kit.
        assert_eq!(back.tracks[1].wave, SeqWave::Kit, "a kit stays a kit");
        // The reimported song still compiles and renders through to_doc.
        back.to_doc().expect("the round-tripped song compiles");
    }

    #[test]
    fn import_maps_channels_and_programs_to_song_voices() {
        // Hand-build a file (export writes no program changes): a melodic
        // track on GM program 32 (a bass) and a channel-10 drum track.
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("voices.mid");
        let mut smf = Smf::new(Header::new(Format::Parallel, Timing::Metrical(PPQ.into())));
        let midi = |delta: u32, channel: u8, message: MidiMessage| -> TrackEvent<'static> {
            TrackEvent {
                delta: delta.into(),
                kind: TrackEventKind::Midi {
                    channel: channel.into(),
                    message,
                },
            }
        };
        let meta = |delta: u32, m: MetaMessage<'static>| -> TrackEvent<'static> {
            TrackEvent {
                delta: delta.into(),
                kind: TrackEventKind::Meta(m),
            }
        };
        let bass = vec![
            meta(0, MetaMessage::Tempo(500_000u32.into())),
            midi(0, 0, MidiMessage::ProgramChange { program: 32.into() }),
            midi(
                0,
                0,
                MidiMessage::NoteOn {
                    key: 36.into(),
                    vel: 100.into(),
                },
            ),
            midi(
                480,
                0,
                MidiMessage::NoteOff {
                    key: 36.into(),
                    vel: 0.into(),
                },
            ),
            meta(0, MetaMessage::EndOfTrack),
        ];
        let drums = vec![
            midi(
                0,
                9,
                MidiMessage::NoteOn {
                    key: 38.into(),
                    vel: 100.into(),
                },
            ),
            midi(
                480,
                9,
                MidiMessage::NoteOff {
                    key: 38.into(),
                    vel: 0.into(),
                },
            ),
            meta(0, MetaMessage::EndOfTrack),
        ];
        smf.tracks.push(bass);
        smf.tracks.push(drums);
        smf.save(&path).unwrap();

        let song = import_midi_song(&path, 4).unwrap();
        assert_eq!(song.tracks.len(), 2);
        assert_eq!(song.tracks[0].name, "track_0");
        assert_eq!(song.tracks[0].wave, SeqWave::Bass, "GM program 32 → bass");
        assert_eq!(song.tracks[1].name, "track_1");
        assert_eq!(song.tracks[1].wave, SeqWave::Kit, "channel 10 → kit");
        assert!((song.bpm - 120.0).abs() < 0.01);
        // 480 ticks at 480 PPQ = one beat = 4 steps at 4 steps/beat.
        assert_eq!(song.tracks[0].notes[0].step, 0);
        assert_eq!(song.tracks[0].notes[0].len, 4);
        assert_eq!(song.tracks[1].notes[0].step, 0);
        assert_eq!(song.tracks[1].notes[0].len, 4);
    }

    #[test]
    fn format_zero_splits_channels_and_preserves_overlapping_notes() {
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("format_zero.mid");
        let midi = |delta: u32, channel: u8, message: MidiMessage| -> TrackEvent<'static> {
            TrackEvent {
                delta: delta.into(),
                kind: TrackEventKind::Midi {
                    channel: channel.into(),
                    message,
                },
            }
        };
        let track = vec![
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MetaMessage::Tempo(500_000u32.into())),
            },
            midi(0, 0, MidiMessage::ProgramChange { program: 0.into() }),
            midi(
                0,
                0,
                MidiMessage::NoteOn {
                    key: 60.into(),
                    vel: 100.into(),
                },
            ),
            midi(
                120,
                0,
                MidiMessage::NoteOn {
                    key: 60.into(),
                    vel: 90.into(),
                },
            ),
            midi(
                0,
                9,
                MidiMessage::NoteOn {
                    key: 36.into(),
                    vel: 110.into(),
                },
            ),
            midi(
                120,
                0,
                MidiMessage::NoteOff {
                    key: 60.into(),
                    vel: 0.into(),
                },
            ),
            midi(
                0,
                9,
                MidiMessage::NoteOff {
                    key: 36.into(),
                    vel: 0.into(),
                },
            ),
            midi(
                120,
                0,
                MidiMessage::NoteOff {
                    key: 60.into(),
                    vel: 0.into(),
                },
            ),
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            },
        ];
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical(PPQ.into()),
        ));
        smf.tracks.push(track);
        smf.save(&path).unwrap();

        let song = import_midi_song(&path, 4).unwrap();
        assert_eq!(song.tracks.len(), 2);
        assert_eq!(song.tracks[0].wave, SeqWave::Piano);
        assert_eq!(song.tracks[0].notes.len(), 2);
        assert_eq!(song.tracks[0].notes[0].step, 0);
        assert_eq!(song.tracks[0].notes[0].len, 2);
        assert_eq!(song.tracks[0].notes[1].step, 1);
        assert_eq!(song.tracks[0].notes[1].len, 2);
        assert_eq!(song.tracks[1].wave, SeqWave::Kit);
        assert_eq!(song.tracks[1].notes.len(), 1);
    }

    #[test]
    fn exported_song_is_a_parsable_midi_file() {
        let song = demo_song();
        let dir = std::env::temp_dir().join("tono-midi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.mid");
        let s = export_song_midi(&song, &path).unwrap();
        assert_eq!(s.tracks, 2);

        // Re-parse: the file is a valid SMF with five note-ons, the kit on
        // channel 10.
        let bytes = std::fs::read(&path).unwrap();
        let smf = Smf::parse(&bytes).unwrap();
        assert_eq!(smf.tracks.len(), 2);
        let note_ons = smf
            .tracks
            .iter()
            .flat_map(|t| t.iter())
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
        assert_eq!(note_ons, 5, "round-trips to five note-ons");
        let kit_channel = smf.tracks[1].iter().any(|e| {
            matches!(
                e.kind,
                TrackEventKind::Midi { channel, .. } if u8::from(channel) == 9
            )
        });
        assert!(kit_channel, "the kit track plays on channel 10");
    }

    #[test]
    fn export_of_an_empty_song_names_the_song() {
        let song = Song::new("empty_one", 120.0);
        let err = export_song_midi(&song, std::path::Path::new("/tmp/none.mid"))
            .err()
            .expect("an empty song has no tracks to export");
        assert!(
            err.to_string().contains("empty_one"),
            "the error carries the song context: {err}"
        );
    }
}
