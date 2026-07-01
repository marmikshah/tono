//! Compose a little song by adding instruments and arranging parts, then hear it.
//!
//!     cargo run -p tono-play --example song
//!
//! A [`Song`] is add-a-track, define-a-pattern, arrange-it — and it compiles to
//! an ordinary deterministic `SoundDoc`, so it plays through the same engine as
//! everything else.

use tono_core::dsl::{Adsr, SeqWave};
use tono_core::song::{Song, note};
use tono_play::play_doc;

fn main() -> anyhow::Result<()> {
    let short = Adsr {
        a: 0.001,
        d: 0.08,
        s: 0.0,
        r: 0.05,
        punch: 0.0,
    };
    let pluck = Adsr {
        a: 0.002,
        d: 0.15,
        s: 0.0,
        r: 0.25,
        punch: 0.0,
    };
    let sustain = Adsr {
        a: 0.01,
        d: 0.1,
        s: 0.85,
        r: 0.25,
        punch: 0.0,
    };

    // Three instruments.
    let mut song = Song::new("code-song", 110.0);
    song.add_track("drums", SeqWave::Kit, short);
    song.add_track("bass", SeqWave::Bass, sustain);
    song.add_track("keys", SeqWave::Epiano, pluck);

    // One-bar backing patterns (16 steps to a bar) + a two-bar keys hook.
    song.add_pattern(
        "beat",
        1,
        vec![
            note(0, 2, "midi:36"),  // kick
            note(4, 2, "midi:38"),  // snare
            note(8, 2, "midi:36"),  // kick
            note(12, 2, "midi:38"), // snare
        ],
    );
    song.add_pattern(
        "bassline",
        1,
        vec![
            note(0, 4, "C2"),
            note(4, 4, "C2"),
            note(8, 4, "G2"),
            note(12, 4, "A2"),
        ],
    );
    song.add_pattern(
        "hook",
        2,
        vec![
            note(0, 4, "C4"),
            note(4, 4, "E4"),
            note(8, 4, "G4"),
            note(16, 8, "C5"),
        ],
    );

    // Arrange four bars: drums + bass throughout, the keys hook twice on top.
    song.arrange_repeat("drums", "beat", 0, 4);
    song.arrange_repeat("bass", "bassline", 0, 4);
    song.arrange_repeat("keys", "hook", 0, 2);

    let doc = song.to_doc().map_err(anyhow::Error::msg)?;
    println!("playing '{}' — {} bars", song.name, song.length_bars());
    play_doc(&doc, doc.duration)?;
    Ok(())
}
