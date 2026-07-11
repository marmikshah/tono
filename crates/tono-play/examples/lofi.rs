//! A full multi-instrument song, composed with the catalog + fluent builder.
//!
//!     make play EXAMPLE=lofi                               # hear it live
//!     cargo run -p tono-play --example lofi -- song.json  # write the SoundDoc
//!         # then: tono render song.json -o out/            # → audio + images
//!
//! A warm Am–F–C–G loop at 84 bpm: mellow-piano chords, a string pad, a fingered
//! bass, a steel-guitar arpeggio, a laid-back acoustic beat, and a bright-piano
//! melody that comes in for the second half. Everything is one `Song::add` per
//! instrument on the shared beat timeline, compiled to a deterministic SoundDoc.

use tono_core::catalog::{Bass, Drums, GrandPiano, Guitar, Strings};
use tono_core::dsl::SoundDoc;
use tono_core::song::Song;
use tono_play::play_doc;

fn build() -> SoundDoc {
    // The progression, one entry per bar (looped over 8 bars).
    let chords = [
        ["A3", "C4", "E4"], // Am
        ["F3", "A3", "C4"], // F
        ["C4", "E4", "G4"], // C
        ["G3", "B3", "D4"], // G
    ];
    let roots = ["A1", "F1", "C2", "G1"];
    let chord = |bar: i32| &chords[(bar % 4) as usize][..];

    // A simple melody over the second pass (bars 4..8): (beat, pitch, beats).
    let melody = [
        (16.0, "E4", 1.0),
        (17.0, "G4", 1.0),
        (18.0, "A4", 2.0),
        (20.0, "C5", 1.0),
        (21.0, "A4", 1.0),
        (22.0, "G4", 2.0),
        (24.0, "E4", 1.0),
        (25.0, "G4", 1.0),
        (26.0, "E4", 1.0),
        (27.0, "D4", 1.0),
        (28.0, "E4", 4.0),
    ];

    Song::new("lofi loop", 84.0)
        // Held piano chords — the harmonic bed.
        .add(GrandPiano::mellow(), |t| {
            for bar in 0..8 {
                t.at(bar as f32 * 4.0).chord(chord(bar), 4.0);
            }
        })
        // A soft string pad underneath, panned left, entering slightly early.
        .add(Strings::warm().gain(0.5).pan(-0.3), |t| {
            for bar in 0..8 {
                t.at(bar as f32 * 4.0 - 0.1).chord(chord(bar), 4.0);
            }
        })
        // Fingered bass on the root, two hits a bar.
        .add(Bass::finger(), |t| {
            for bar in 0..8 {
                let (b, r) = (bar as f32 * 4.0, roots[(bar % 4) as usize]);
                t.at(b).note(r, 2.0).at(b + 2.0).note(r, 2.0);
            }
        })
        // Steel-guitar arpeggio, quiet and panned right.
        .add(Guitar::steel().gain(0.55).pan(0.35), |t| {
            for bar in 0..8 {
                t.at(bar as f32 * 4.0);
                for _ in 0..2 {
                    for p in chord(bar) {
                        t.play(p, 0.5);
                    }
                }
            }
        })
        // A laid-back acoustic beat.
        .add(Drums::acoustic(), |t| {
            for bar in 0..8 {
                let b = bar as f32 * 4.0;
                t.at(b).kick().at(b + 2.5).kick();
                t.at(b + 1.0).snare().at(b + 3.0).snare();
                for eighth in 0..8 {
                    t.at(b + eighth as f32 * 0.5).vel(0.5).hat();
                }
            }
        })
        // A bright-piano melody for the second half.
        .add(GrandPiano::bright().gain(0.8), |t| {
            for &(beat, pitch, dur) in &melody {
                t.at(beat).vel(0.85).note(pitch, dur);
            }
        })
        .to_doc()
        .expect("a valid song")
}

fn main() -> anyhow::Result<()> {
    let doc = build();
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
        println!("wrote {path} — render it with: tono render {path} -o out/");
    } else {
        println!(
            "playing '{}' ({:.0}s) — Ctrl-C to stop",
            doc.name, doc.duration
        );
        play_doc(&doc, doc.duration)?;
    }
    Ok(())
}
