# Rhythm: when notes play

[Pitch](/get-started/pitch) chooses the note. **Rhythm** chooses when it starts and how long it lasts. You can tap a steady pulse with your hand before writing any code.

## Tempo and beats

A **beat** is one pulse. **Tempo** is the speed of that pulse, measured in **BPM**: beats per minute. At 60 BPM, a beat lasts one second; at 120 BPM, it lasts half a second. The rule is `seconds per beat = 60 / BPM`.

Raising BPM makes the same notes happen faster. Their pitches stay the same. A beat can contain a note, several shorter notes, or silence.

<figure class="rhythm-count">
  <figcaption>Count one bar at 120 BPM: it ends at 2 seconds.</figcaption>
  <div class="rhythm-count-beats" role="img" aria-label="Four beats at 120 BPM. Count 1 at zero seconds, 2 at half a second, 3 at one second, and 4 at one and a half seconds.">
    <span><strong>1</strong><small>0.0s</small></span>
    <span><strong>2</strong><small>0.5s</small></span>
    <span><strong>3</strong><small>1.0s</small></span>
    <span><strong>4</strong><small>1.5s</small></span>
  </div>
</figure>

## Bars, meter and the grid

A **bar** groups beats into a repeating count. The **meter** tells you how to group them: **4/4** means four quarter-note beats per bar, counted “1, 2, 3, 4.” **3/4** groups three: “1, 2, 3.” Here, the lower `4` names the quarter note as the beat unit.

tono defaults to 4/4 and **four grid steps per beat**, so a bar has **16 steps**. Rust's `note(start_step, length_steps, pitch)` uses that grid. Python's `Pattern.note(pitch, at=…, duration=…)` uses beats.

| Count aloud | 1 | 2 | 3 | 4 |
|---|---|---|---|---|
| Rust start step | 0 | 4 | 8 | 12 |
| Python `at` | 0 | 1 | 2 | 3 |

Code counts from zero. One beat is Rust length `4`, or Python duration `1`. Half a beat is length `2`, or duration `0.5`.

## A pattern, then an arrangement

A **sequence** is a list of timed notes. A **pattern** stores a reusable piece of that sequence, such as one bar. An **arrangement** places the pattern on an instrument track at particular bars. Defining a pattern alone does not play it.

[Install tono](/get-started/) first. Save Rust as `src/main.rs` and run `cargo run`, or save Python as `rhythm.py` and run `python rhythm.py`. This complete example starts with four piano notes, then writes five WAVs so you can hear one change at a time. It also saves the first pattern's compiled program for the looping example below.

::: code-group

```rust [Rust]
use tono_core::{prelude::*, song::Pattern};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pitches = ["C4", "E4", "G4", "C5"];
    // Name, BPM, beats per bar, number of repeats, note length in steps.
    let versions = [
        ("one-bar", 120.0, 4, 1, 4),
        ("four-bars", 120.0, 4, 4, 4),
        ("faster", 160.0, 4, 4, 4),
        ("short-notes", 120.0, 4, 4, 2),
        ("three-beats", 120.0, 3, 1, 4),
    ];
    for (name, bpm, beats_per_bar, repeats, note_steps) in versions {
        let mut song = Song::new(name, bpm);
        song.beats_per_bar = beats_per_bar;
        song.add_voice("piano", &GrandPiano::grand());
        let phrase = Pattern {
            name: "phrase".into(), bars: 1,
            notes: pitches[..beats_per_bar as usize].iter().enumerate()
                .map(|(beat, pitch)| note(beat as u32 * 4, note_steps, *pitch))
                .collect(),
        };
        song.add_pattern(phrase.name, phrase.bars, phrase.notes);
        song.arrange_repeat("piano", "phrase", 0, repeats);
        let program = song.compile(&CompileOptions {
            sample_rate: Some(48_000), ..Default::default()
        })?;
        if name == "one-bar" {
            std::fs::write("one-bar.program.json", program.to_json())?;
        }
        let (left, right) = program.render_stereo();
        let spec = hound::WavSpec {
            channels: 2, sample_rate: 48_000, bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = hound::WavWriter::create(format!("{name}.wav"), spec)?;
        for (&l, &r) in left.iter().zip(&right) {
            wav.write_sample((l.clamp(-1.0, 1.0) * 32767.0) as i16)?;
            wav.write_sample((r.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        }
        wav.finalize()?;
        println!("{name}.wav: {:.2}s", left.len() as f64 / 48_000.0);
    }
    Ok(())
}
```

```python [Python]
import wave
import tono

pitches = ["C4", "E4", "G4", "C5"]
# Name, BPM, beats per bar, number of repeats, note duration in beats.
versions = [
    ("one-bar", 120, 4, 1, 1),
    ("four-bars", 120, 4, 4, 1),
    ("faster", 160, 4, 4, 1),
    ("short-notes", 120, 4, 4, 0.5),
    ("three-beats", 120, 3, 1, 1),
]
for name, bpm, beats_per_bar, repeats, note_beats in versions:
    song = tono.Song(name, tempo=bpm)
    if beats_per_bar == 3:
        song.set_meter_map([(0, 3, 4)])  # 3/4, starting at bar zero.
    piano = song.track("piano", tono.instruments.piano())
    phrase = tono.Pattern(bars=1)
    for beat, pitch in enumerate(pitches[:beats_per_bar]):
        phrase.note(pitch, at=beat, duration=note_beats)
    song.arrange(piano, phrase, bars=range(repeats))
    program = song.compile(sample_rate=48_000)
    if name == "one-bar":
        program.save("one-bar.program.json")
    audio = program.render()
    pcm = (audio.clip(-1, 1) * 32767).astype("<i2")
    with wave.open(f"{name}.wav", "wb") as wav:
        wav.setnchannels(2)
        wav.setsampwidth(2)
        wav.setframerate(program.sample_rate)
        wav.writeframes(pcm.tobytes())
    print(f"{name}.wav: {len(audio) / program.sample_rate:.2f}s")
```

:::

## Hear what each change does

Open the WAVs in an audio player. All are 48 kHz stereo.

| File | What to listen for | File length |
|---|---|---|
| `one-bar.wav` | Four notes at 120 BPM | 4.00s |
| `four-bars.wav` | The same bar repeated four times | 10.00s |
| `faster.wav` | The same four-bar phrase at 160 BPM | 8.00s |
| `short-notes.wav` | Same starting times, shorter notes with gaps | 9.75s |
| `three-beats.wav` | Three notes counted “1, 2, 3” | 3.50s |

**Duration** is the complete note window. **Gate** describes how much of the space before the next note that window occupies: with notes starting one beat apart, duration `1` is a full gate; `0.5` is half. Shortening it creates gaps without moving the next note. tono's sequenced envelope includes its release inside that window; the [instrument lesson](/get-started/instruments) explains the shape.

The compiler adds **two seconds after the last note ends** for the render tail, including effect decay. Those seconds can be silent. A four-beat bar at 120 BPM takes two seconds, but `one-bar.wav` is four seconds long. In `short-notes.wav`, the final note ends a quarter-second earlier, so the file is shorter even though the bar grid is unchanged.

## Repeating a phrase versus looping playback

An arrangement with four repeats contains four copies and then ends. A playback **loop** returns to an earlier bar and keeps going. After running the first example, this loads its saved program, loops the first bar, and renders six seconds into memory without speakers:

::: code-group

```rust [Rust]
use std::sync::Arc;
use tono_core::{prelude::*, runtime::{At, Command, Performance}};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::from_json(&std::fs::read_to_string("one-bar.program.json")?)?;
    let mut player = Performance::new(Arc::new(program));
    player.schedule(Command::SetLoopBars(0, 1), At::Immediate)?;
    player.schedule(Command::Play, At::Immediate)?;
    let mut audio = vec![0.0; 48_000 * 6 * 2];
    let frames = player.fill(&mut audio);
    println!("Looped {frames} stereo frames");
    Ok(())
}
```

```python [Python]
import tono

program = tono.Program.load("one-bar.program.json")
with tono.Performance(program, headless=True) as player:
    player.set_loop_bars(0, 1)
    player.play()
    audio = player.fill(48_000 * 6)
    print(f"Looped {len(audio)} stereo frames")
```

:::

Both print `Looped 288000 stereo frames`. The loop starts at bar `0` and stops just before bar `1`: one two-second musical bar, without the exported file's extra tail. Call `fill` again to continue. See [live audio](/guides/live) for output and transport controls, or use a prepared loop from the [music library](/bgm).

[← Pitch](/get-started/pitch) · [Foundations overview](/get-started/basics) · [Next: instruments and envelopes →](/get-started/instruments)

<style scoped>
.rhythm-count { margin: 20px 0; }
.rhythm-count figcaption { margin-bottom: 8px; color: var(--vp-c-text-2); font-size: 13px; }
.rhythm-count-beats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); overflow: hidden; border: 1px solid var(--vp-c-divider); border-radius: 8px; }
.rhythm-count-beats > span { padding: 12px 8px; background: var(--vp-c-bg-soft); text-align: center; }
.rhythm-count-beats > span + span { border-left: 1px solid var(--vp-c-divider); }
.rhythm-count-beats strong { display: block; color: var(--vp-c-text-1); font-size: 20px; }
.rhythm-count-beats small { display: block; margin-top: 4px; color: var(--vp-c-text-2); font-size: 12px; }
</style>
