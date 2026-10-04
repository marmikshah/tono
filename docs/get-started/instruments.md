# Instruments and sound

Play the same notes on a piano and a bass: the melody stays the same, but the sound changes. That difference in character is called **timbre**. An instrument gives your notes their timbre.

If these terms are new, start with [the basics](/get-started/basics), [pitch and notes](/get-started/pitch), and [rhythm and timing](/get-started/rhythm).

## What belongs to the instrument?

- A **note** says which pitch to play, when it starts, how long it lasts, and how strongly it is played.
- A **pattern** collects notes into a reusable phrase.
- An **instrument** decides how those notes sound: its source, envelope, and tone settings.
- A **track** gives an instrument a place in the song, with its own volume and stereo position. You arrange patterns onto tracks.

In Tono's song API, an instrument configuration is called a **Voice**. Adding a voice creates a track; it does not add any notes yet.

## A waveform is part of an instrument

An **oscillator** generates a repeating signal. Its frequency sets the pitch; its **waveform** sets the shape of each repetition. A sine wave sounds smooth, while a square or sawtooth wave has extra high-frequency content and sounds buzzier.

An instrument can combine sources and change them over time. Tono's piano, for example, combines many ringing tones with a sharp strike and a fading body. Choosing a piano is more than choosing one simple waveform.

Two other source ideas appear in sound design: **noise** is an irregular signal useful for hiss and drum attacks; **FM**, or frequency modulation, uses one oscillator to alter another, producing brighter or bell-like tones.

Start with the catalog rather than building these sources yourself:

| Instrument | Rust constructor | Python constructor |
|---|---|---|
| Grand piano | `GrandPiano::grand()` | `tono.instruments.piano("grand")` |
| Electric piano | `ElectricPiano::rhodes()` | `tono.instruments.electric_piano("rhodes")` |
| Finger bass | `Bass::finger()` | `tono.instruments.bass("finger")` |
| Nylon guitar | `Guitar::nylon()` | `tono.instruments.guitar("nylon")` |
| Tonewheel organ | `Organ::tonewheel()` | `tono.instruments.organ("tonewheel")` |
| Classic drum kit | `Drums::classic()` | `tono.instruments.drums("classic")` |

These catalog instruments synthesize their audio; you do not need sample files. With a drum kit, the note selects a drum: `midi:36` is a kick, `midi:38` a snare, and `midi:42` a closed hi-hat.

## An envelope shapes the level over time

**Amplitude** is the size of the audio signal. An amplitude **envelope** changes that level during a note: a quick rise gives a sharp attack; a slow rise gives a swell.

The common envelope is **ADSR**:

| Stage | Tono field | Meaning | Example |
|---|---|---|---|
| Attack | `a` | Time to rise from silence to the peak | `0.01`: a 10 ms rise |
| Decay | `d` | Time to fall from the peak to the sustain level | `0.1`: a 100 ms fall |
| Sustain | `s` | Level after the decay, while the note continues | `0.6`: 60% of the envelope's peak |
| Release | `r` | Time for the final fade to silence | `0.2`: a 200 ms fade |

Attack, decay, and release are in **seconds**. Sustain is a **level from 0 to 1**, not a duration. The note's length determines how much time is available for these stages.

For Tono's synthesized `Song`/`seq` notes, release fits **inside the written note duration**. A one-second note with `r: 0.2` begins its final fade at 0.8 seconds. Increasing release makes that fade start earlier; it does not extend the note. Live held instruments instead start their release when you send `note_off`.

The source can also decay on its own. A piano gets quieter as its modeled strings lose energy, even with sustain set to 1. Catalog voices already contain tuned envelopes; you can use those defaults while learning. Custom envelopes belong to the [SoundDoc graph](/reference/sounddoc).

## Keep the notes; change the instrument

[Install Tono](/get-started/) first. Save the Rust example as `src/main.rs` in `tono-demo` and run `cargo run`, or save the Python example as `instruments.py` and run `python instruments.py`.

This writes `instruments.wav`: the same four-note pattern on piano in the first bar and bass in the second. Each bar lasts two seconds at 120 BPM. The stereo file is 48 kHz and includes the compiler's two-second end buffer. No speaker or audio device is opened.

::: code-group

```rust [Rust]
use tono_core::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let piano = GrandPiano::grand().gain(0.65).pan(-0.4);
    let bass = Bass::finger().gain(0.6).pan(0.4);
    let mut song = Song::new("instruments", 120.0).with_seed(7);
    song.add_voice("piano", &piano);
    song.add_voice("bass", &bass);
    song.add_pattern("phrase", 1, vec![
        note(0, 4, "C3"), note(4, 4, "E3"),
        note(8, 4, "G3"), note(12, 4, "C4"),
    ]);
    song.arrange("piano", "phrase", 0);
    song.arrange("bass", "phrase", 1);
    let program = song.compile(&CompileOptions {
        sample_rate: Some(48_000), ..Default::default()
    })?;
    let (left, right) = program.render_stereo();
    let spec = hound::WavSpec {
        channels: 2, sample_rate: 48_000, bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create("instruments.wav", spec)?;
    for (&l, &r) in left.iter().zip(&right) {
        wav.write_sample((l.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        wav.write_sample((r.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    wav.finalize()?;
    println!("Wrote instruments.wav ({} stereo frames)", left.len());
    Ok(())
}
```

```python [Python]
import wave
import tono

piano_voice = tono.instruments.piano("grand").gain(0.65).pan(-0.4)
bass_voice = tono.instruments.bass("finger").gain(0.6).pan(0.4)
song = tono.Song("instruments", tempo=120, seed=7)
piano = song.track("piano", piano_voice)
bass = song.track("bass", bass_voice)
phrase = tono.Pattern(bars=1)
phrase.notes(["C3", "E3", "G3", "C4"], durations=1)
song.arrange(piano, phrase, bars=0)
song.arrange(bass, phrase, bars=1)
program = song.compile(sample_rate=48_000)
audio = program.render()  # Float32 samples: (frames, 2).
pcm = (audio.clip(-1, 1) * 32767).astype("<i2")
with wave.open("instruments.wav", "wb") as wav:
    wav.setnchannels(2)
    wav.setsampwidth(2)
    wav.setframerate(program.sample_rate)
    wav.writeframes(pcm.tobytes())
print(f"Wrote instruments.wav ({len(audio)} stereo frames)")
```

:::

Open the WAV in an audio player, then change one voice setting and run the code again:

| Change | What you hear | What stays the same |
|---|---|---|
| Change piano `.gain(0.65)` to `.gain(0.3)` | A quieter piano | Its notes and timing |
| Change piano `.pan(-0.4)` to `.pan(0.4)` | The piano moves from left toward right | Its pitch and rhythm |
| Use `GrandPiano::bright()` / `piano("bright")` | A sharper piano tone | The four-note phrase |
| Add `.reverb(0.2)` to a voice before adding its track | A sense of space and a lingering tail | The scheduled note starts |

**Gain** is a multiplier: 0 is silent, 1 leaves the level unchanged, and 0.5 halves the signal's amplitude. The voice's `.gain()` becomes the track fader, so it affects every note on that track. Individual note strength is separate: it can change both level and attack character on instruments that respond to it.

**Pan** places a track in stereo: −1 is left, 0 is center, and 1 is right. Set both pans to 0 to compare the two timbres without their different positions.

Next, [compose a song](/guides/songs) with different patterns for each instrument. To change the actual melody or rhythm, return to [pitch](/get-started/pitch) and [timing](/get-started/rhythm).
