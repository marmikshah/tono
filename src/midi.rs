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
use tono_core::dsl::{Modulator, Node, SeqNote, SoundDoc, Value, note_to_hz};

const PPQ: u16 = 480;

/// What [`export_midi`] wrote.
pub struct MidiSummary {
    pub tracks: usize,
    pub notes: usize,
}

struct SeqRef<'a> {
    bpm: f32,
    spb: u32,
    notes: &'a [SeqNote],
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
    match node {
        Node::Seq {
            bpm,
            steps_per_beat,
            notes,
            ..
        } => out.push(SeqRef {
            bpm: *bpm,
            spb: (*steps_per_beat).max(1),
            notes,
        }),
        Node::Tracks { tracks, .. } => {
            for t in tracks {
                collect_seqs(&t.node, out);
            }
        }
        Node::Mix { inputs } | Node::Mul { inputs } => {
            for inp in inputs {
                collect_seqs(inp, out);
            }
        }
        Node::Chain { stages } => {
            for st in stages {
                collect_seqs(st, out);
            }
        }
        _ => {}
    }
}

/// Build one MIDI track from a seq. `tempo` (if `Some`) writes the global tempo.
fn seq_track(s: &SeqRef, tempo: Option<u32>) -> (Track<'static>, usize) {
    let step_ticks = (PPQ as u32 / s.spb).max(1);
    // (absolute tick, is_note_on, key). Note-offs sort before note-ons at the
    // same tick so a zero-length gap re-strikes cleanly.
    let mut events: Vec<(u32, bool, u8)> = Vec::with_capacity(s.notes.len() * 2);
    for note in s.notes {
        let key = pitch_to_midi(&note.pitch);
        let on = note.step * step_ticks;
        let off = (note.step + note.len.max(1)) * step_ticks;
        events.push((on, true, key));
        events.push((off, false, key));
    }
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut track = Track::new();
    if let Some(us) = tempo {
        track.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(us.into())),
        });
    }
    let (mut last, mut n) = (0u32, 0usize);
    for (tick, is_on, key) in events {
        let delta = tick - last;
        last = tick;
        let message = if is_on {
            n += 1;
            MidiMessage::NoteOn {
                key: key.into(),
                vel: 96.into(),
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
                channel: 0.into(),
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
    (69.0 + 12.0 * (hz / 440.0).log2())
        .round()
        .clamp(0.0, 127.0) as u8
}

fn representative_hz(m: &Modulator) -> f32 {
    match m {
        Modulator::Slide { from, .. } => *from,
        Modulator::Lfo { center, .. } => *center,
        Modulator::Arp { steps, .. } => steps.first().copied().unwrap_or(440.0),
        Modulator::EnvMod { from, .. } => *from,
        Modulator::Rand { from, to, .. } => 0.5 * (from + to),
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
    fn no_seq_is_an_error() {
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"x", "duration":0.2, "root":{"type":"sine","freq":440} }"#,
        )
        .unwrap();
        assert!(export_midi(&doc, std::path::Path::new("/tmp/none.mid")).is_err());
    }
}
