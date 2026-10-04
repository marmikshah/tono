# Compose songs

Make a melody, arrange a band, and save the compiled song. Start with the
[Rust or Python installation](/get-started/).

Save each Rust example as `src/main.rs` and run `cargo run`. Save each Python
example as `song.py` and run `python song.py`.

<a id="write-music-with-seq"></a>
<a id="music-with-seq"></a>

## Render a melody to WAV

This writes `melody.wav`: two seconds of piano notes at 120 BPM in 48 kHz
stereo, plus the compiler’s two-second tail allowance for lingering effects.

::: code-group

```rust [Rust]
use tono_core::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut song = Song::new("melody", 120.0);
    song.add_voice("piano", &GrandPiano::grand());
    song.add_pattern("melody", 1, vec![
        note(0, 4, "C4"), note(4, 4, "E4"),
        note(8, 4, "G4"), note(12, 4, "C5"),
    ]);
    song.arrange("piano", "melody", 0);
    let program = song.compile(&CompileOptions {
        sample_rate: Some(48_000), ..Default::default()
    })?;
    let (left, right) = program.render_stereo();
    let spec = hound::WavSpec {
        channels: 2, sample_rate: 48_000, bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create("melody.wav", spec)?;
    for (&l, &r) in left.iter().zip(&right) {
        wav.write_sample((l.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        wav.write_sample((r.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    wav.finalize()?;
    println!("Wrote melody.wav ({} frames)", left.len());
    Ok(())
}
```

```python [Python]
import wave
import tono

song = tono.Song("melody", tempo=120)
piano = song.track("piano", tono.instruments.piano())
melody = tono.Pattern(bars=1)
melody.notes(["C4", "E4", "G4", "C5"], durations=1)
song.arrange(piano, melody, bars=0)
program = song.compile(sample_rate=48_000)
mix = program.render()  # Float32 array: (frames, 2), left/right.
pcm = (mix.clip(-1, 1) * 32767).astype("<i2")
with wave.open("melody.wav", "wb") as wav:
    wav.setnchannels(2)
    wav.setsampwidth(2)
    wav.setframerate(program.sample_rate)
    wav.writeframes(pcm.tobytes())
print(f"Wrote melody.wav ({len(mix)} frames)")
```

:::

The default grid has four steps per beat: Rust `note(4, 4, "E4")` starts at
beat 1 and lasts one beat. Python pattern times use beats directly.

<a id="compile-a-song-to-a-program"></a>
<a id="songs--from-a-composition-to-a-program"></a>

## Arrange four bars and save the song

Reuse one-bar drum and bass patterns. This writes `groove.program.json` and
prints the frame count of each stereo stem.

::: code-group

```rust [Rust]
use tono_core::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut song = Song::new("groove", 110.0).with_seed(42);
    song.add_voice("drums", &Drums::classic().gain(0.65));
    song.add_voice("bass", &Bass::finger().gain(0.6));
    song.add_pattern("beat", 1, vec![
        note(0, 2, "midi:36"), note(4, 2, "midi:38"),
        note(8, 2, "midi:36"), note(12, 2, "midi:38"),
    ]);
    song.add_pattern("riff", 1, vec![
        note(0, 4, "C2"), note(4, 4, "C2"),
        note(8, 4, "G2"), note(12, 4, "A2"),
    ]);
    song.arrange_repeat("drums", "beat", 0, 4);
    song.arrange_repeat("bass", "riff", 0, 4);
    let program = song.compile(&CompileOptions::default())?;
    std::fs::write("groove.program.json", program.to_json())?;
    for stem in program.render_stems() {
        println!("{}: {} frames", stem.id, stem.left.len());
    }
    Ok(())
}
```

```python [Python]
import tono

song = tono.Song("groove", tempo=110, seed=42)
drums = song.track("drums", tono.instruments.drums("classic").gain(0.65))
bass = song.track("bass", tono.instruments.bass().gain(0.6))
beat = tono.Pattern(bars=1)
for at, drum in [(0, "midi:36"), (1, "midi:38"),
                 (2, "midi:36"), (3, "midi:38")]:
    beat.note(drum, at=at, duration=0.5)
riff = tono.Pattern(bars=1)
riff.notes(["C2", "C2", "G2", "A2"], durations=1)
song.arrange(drums, beat, bars=range(4))
song.arrange(bass, riff, bars=range(4))
program = song.compile()
program.save("groove.program.json")
for name, stem in program.render_stems().items():
    print(f"{name}: {len(stem)} frames")
```

:::

Reload the saved bundle with Rust `Program::from_json` or Python
`tono.Program.load`. Save the editable score separately with
`serde_json::to_string(&song)` or `song.to_json()`.

A normal song render plays once. For a prepared seamless loop, use the
[background music library](/bgm); for runtime looping, see [live playback](/guides/live).
Stems are before the master effects; bus-routed tracks are already included
in their bus return.

More: [instrument and note reference](/reference/sounddoc),
[Rust Song API](https://docs.rs/tono-core/latest/tono_core/song/struct.Song.html),
and [complete Python compositions](https://github.com/marmikshah/tono/tree/master/crates/tono-py/examples).
