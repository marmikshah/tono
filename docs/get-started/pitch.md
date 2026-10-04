# Pitch: choose how high a note sounds

**Pitch** is how high or low a sound feels. A bass note is low; a whistle is
high. Volume is a separate choice: making a note louder does not raise its pitch.

Start with [the sound basics](/get-started/basics). To run the examples,
[install Tono](/get-started/), then save Rust code as `src/main.rs` and run
`cargo run`, or save Python code as `pitch.py` and run `python pitch.py`.

## Hz, note names, and octaves

**Hertz (Hz)** counts vibrations per second. More Hz means a higher pitch.
A **note name** gives that pitch a musical label: `A4` is 440 Hz.
The letter names the note; the number names its **octave**, or pitch register.

| Note | Frequency | What you hear |
| --- | ---: | --- |
| `A3` | 220 Hz | Lower A |
| `A4` | 440 Hz | The reference A |
| `A#4` | About 466.16 Hz | One small step above A4 |
| `A5` | 880 Hz | A4 raised one octave |

An octave contains **12 semitones**. A semitone is the small step from `A4`
to `A#4`; moving up 12 semitones doubles the frequency. Moving down an octave
halves it. The note keeps its letter, but sounds in a higher or lower register.

## Create a note and move it

Create `A4`, then move it up one semitone and one octave. Both examples print:

```text
A4: MIDI 69, 440.00 Hz
A#4: MIDI 70, 466.16 Hz
A5: MIDI 81, 880.00 Hz
```

MIDI here is simply a numbered note: adding 1 moves up one semitone.
It does not require a keyboard or a MIDI device.

::: code-group

```rust [Rust]
use tono_core::music::Pitch;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a = Pitch::from_name("A4")?;
    let notes = [a, a.add_semitones(1)?, a.add_semitones(12)?];
    for pitch in notes {
        println!("{pitch}: MIDI {}, {:.2} Hz", pitch.to_midi(), pitch.to_hz());
    }
    Ok(())
}
```

```python [Python]
import tono

a = tono.Pitch("A4")
notes = [a, a.transpose(1), a.transpose(12)]
for pitch in notes:
    # Python exposes MIDI; this converts that number to Hz.
    hz = 440 * 2 ** ((pitch.midi - 69) / 12)
    print(f"{pitch.name}: MIDI {pitch.midi}, {hz:.2f} Hz")
```

:::

For note spelling, use a letter `A`–`G`, an optional `#` (sharp) or `b`
(flat), and an octave: `C4`, `F#3`, `Gb5`. A sharp raises the plain note one
semitone; a flat lowers it one. `F#4` and `Gb4` name the same pitch; Tono displays
it as `F#4`. `s` also works for sharp (`Fs4`), and `midi:60` names `C4`.

The `Pitch` API requires the octave and accepts only one accidental.
`C`, `H4`, and `C##4` fail with an error; it does not guess what you meant.
Use plain `#` and `b`, rather than the musical symbols `♯` and `♭`.

## Turn four notes into a phrase

A **phrase** is a short musical idea. This one plays `C4`, `E4`, `G4`, `C5`
in order on a piano, one note per beat. At 120 beats per minute, each beat
lasts half a second.

::: code-group

```rust [Rust]
use tono_core::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let octave_shift = 0; // Try 12 to raise the whole phrase one octave.
    let mut notes = Vec::new();
    for (beat, name) in ["C4", "E4", "G4", "C5"].iter().enumerate() {
        let pitch = Pitch::from_name(name)?.add_semitones(octave_shift)?;
        println!("{pitch}");
        notes.push(note(beat as u32 * 4, 4, &pitch.to_string()));
    }
    let mut song = Song::new("climb", 120.0);
    song.add_voice("piano", &GrandPiano::grand());
    song.add_pattern("climb", 1, notes);
    song.arrange("piano", "climb", 0);
    let program = song.compile(&CompileOptions {
        sample_rate: Some(48_000), ..Default::default()
    })?;
    let (left, _right) = program.render_stereo();
    println!("Rendered {} stereo frames", left.len());
    Ok(())
}
```

```python [Python]
import tono

octave_shift = 0  # Try 12 to raise the whole phrase one octave.
phrase = tono.Pattern(bars=1)
for beat, name in enumerate(["C4", "E4", "G4", "C5"]):
    pitch = tono.Pitch(name).transpose(octave_shift)
    print(pitch.name)
    phrase.note(pitch.name, at=beat, duration=1)
song = tono.Song("climb", tempo=120)
piano = song.track("piano", tono.instruments.piano())
song.arrange(piano, phrase, bars=0)
program = song.compile(sample_rate=48_000)
audio = program.render()
print(f"Rendered {len(audio)} stereo frames")
```

:::

Both render 192,000 stereo frames in memory: two seconds of notes plus the
compiler’s two-second tail allowance. They do not open speakers. Follow
[the WAV example](/guides/songs#render-a-melody-to-wav) to save a file you can hear.

Change `octave_shift` to `12`: the phrase becomes `C5`, `E5`, `G5`, `C6`.
Every pitch doubles in frequency; the rhythm and chosen instrument stay the same.
Change it to `-12` for a lower version. To change the character of the sound,
choose a different [instrument](/get-started/instruments).

Next: [tempo and rhythm](/get-started/rhythm) — decide when notes play and how
long they last.
