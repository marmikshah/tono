//! The full loop, end to end: compose a song, compile it to a Program,
//! render the mix and stems, then run it live through a Performance — all
//! deterministic. Run with `cargo run -p tono-core --example compose`.

use std::sync::Arc;

use tono_core::catalog::{Bass, Drums, ElectricPiano};
use tono_core::prelude::*;
use tono_core::runtime::{At, Command, Performance};

fn main() {
    // Compose: tracks from the catalog, patterns, an arrangement, a bus for
    // the shared reverb, and a beat-addressed fade-in on the keys.
    let mut song = Song::new("compose-demo", 112.0);
    song.add_voice("drums", &Drums::tr808());
    song.add_voice("bass", &Bass::finger());
    song.add_voice("keys", &ElectricPiano::rhodes());
    song.buses.push(tono_core::dsl::Bus {
        id: "verb".into(),
        gain: 0.7,
        effects: vec![Node::Reverb {
            room: 0.5,
            mix: 0.6,
        }],
    });
    song.tracks[2].sends.push(tono_core::dsl::Send {
        bus: "verb".into(),
        amount: 0.4,
    });
    song.tracks[2].automation.push(tono_core::song::SongLane {
        target: tono_core::dsl::AutoTarget::Gain,
        curve: tono_core::dsl::AutoCurve::Exp,
        points: vec![
            tono_core::song::SongPoint { at: 0.0, v: 0.1 },
            tono_core::song::SongPoint { at: 4.0, v: 0.9 },
        ],
    });
    let mut beat = tono_core::song::Phrase::new(4);
    beat.at(0.0)
        .kick()
        .at(0.5)
        .hat()
        .at(1.0)
        .snare()
        .at(1.5)
        .hat();
    song.add_pattern("beat", 1, beat.into_notes());
    song.add_pattern(
        "riff",
        1,
        vec![note(0, 2, "C2"), note(4, 2, "D#2"), note(8, 2, "G2")],
    );
    song.add_pattern(
        "chords",
        1,
        vec![note(0, 8, "C4"), note(0, 8, "D#4"), note(0, 8, "G4")],
    );
    song.arrange_repeat("drums", "beat", 0, 4);
    song.arrange_repeat("bass", "riff", 0, 4);
    song.arrange_repeat("keys", "chords", 0, 4);
    song.sections.push(tono_core::song::Section {
        name: "outro".into(),
        bar: 2,
        bars: 2,
    });

    // Compile once: the artifact carries the hash, metadata, and estimates.
    let program = song.compile(&CompileOptions::default()).expect("compiles");
    println!(
        "compiled '{}': hash {:#018x}, {:.1}s, {} tracks, {} events, streamable={}",
        program.doc.name,
        program.hash,
        program.doc.duration,
        program.meta.tracks.len(),
        program.estimates.events,
        program.is_streamable(),
    );

    // Render the mix and the stems (pre-master: they feed an external mixer).
    let (l, r) = program.render_stereo();
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, x| m.max(x.abs()));
    println!("rendered the mix: {} frames, peak {:.3}", l.len(), peak);
    for stem in program.render_stems() {
        let p = stem.left.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        println!("  stem {:<10} peak {:.3}", stem.id, p);
    }

    // Run it live: scheduled commands land on exact frames — the host never
    // wakes on a musical boundary.
    let mut perf = Performance::new(Arc::new(program));
    perf.schedule(Command::Play, At::Immediate).unwrap();
    perf.schedule(Command::SetGain(0.6), At::Section("outro".into()))
        .unwrap();
    let mut out = vec![0.0f32; 8192 * 2];
    perf.fill(&mut out);
    let metrics = perf.metrics();
    println!(
        "ran a performance: {} frames, {} commands executed, {} dropped",
        metrics.frames_rendered, metrics.commands_executed, metrics.commands_dropped,
    );
}
