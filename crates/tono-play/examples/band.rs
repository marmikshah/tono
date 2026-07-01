//! A four-piece band, written with the catalog + fluent song builder, then heard.
//!
//!     cargo run -p tono-play --example band
//!
//! `Song::add(instrument, |t| ...)` adds a catalog instrument and writes its
//! notes on the shared beat timeline — piano chords, a walking bass, a steel
//! guitar arpeggio, and drums, all layered at the same beats. It compiles to an
//! ordinary deterministic `SoundDoc` and plays through the same engine as
//! everything else.

use tono_core::catalog::{Bass, Drums, GrandPiano, Guitar};
use tono_core::song::Song;
use tono_play::play_doc;

fn main() -> anyhow::Result<()> {
    // A four-bar loop over C – Am – F – G (4/4 at 96 bpm).
    let chords = [
        ["C4", "E4", "G4"],
        ["A3", "C4", "E4"],
        ["F3", "A3", "C4"],
        ["G3", "B3", "D4"],
    ];
    let roots = ["C2", "A1", "F1", "G1"];

    let song = Song::new("band demo", 96.0)
        .add(GrandPiano::grand(), |t| {
            for (bar, chord) in chords.iter().enumerate() {
                t.at(bar as f32 * 4.0).chord(&chord[..], 4.0);
            }
        })
        .add(Bass::finger().gain(0.9), |t| {
            for (bar, root) in roots.iter().enumerate() {
                let b = bar as f32 * 4.0;
                t.at(b).note(root, 2.0).at(b + 2.0).note(root, 2.0);
            }
        })
        .add(Guitar::steel().gain(0.7).pan(0.3), |t| {
            for (bar, chord) in chords.iter().enumerate() {
                // Arpeggiate the chord up as eighth notes across the bar.
                t.at(bar as f32 * 4.0);
                for _ in 0..2 {
                    for pitch in chord {
                        t.play(pitch, 0.5);
                    }
                }
            }
        })
        .add(Drums::acoustic(), |t| {
            for bar in 0..4 {
                let b = bar as f32 * 4.0;
                t.at(b).kick().at(b + 2.0).kick();
                t.at(b + 1.0).snare().at(b + 3.0).snare();
                for eighth in 0..8 {
                    t.at(b + eighth as f32 * 0.5).vel(0.6).hat();
                }
            }
        });

    let doc = song.to_doc().map_err(anyhow::Error::msg)?;
    println!(
        "playing '{}' — a four-piece band on one timeline",
        song.name
    );
    play_doc(&doc, doc.duration)?;
    Ok(())
}
