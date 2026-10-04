---
guideLanguage: false
---

# Sound basics

Tono turns instructions into sound. Choose notes, decide when they play, and give them an instrument. Start by listening to this phrase, then follow the Rust and Python lessons below.

## Hear what changes

<TonoLearnSound />

Try one change at a time: raise **Pitch** to hear the whole phrase higher, increase **Tempo** to hear it faster, then switch **Instrument** to change its character. Shorter **Note length** ends each note earlier, leaving a rest before the next note. Note starts stay on the same beats.

## The building blocks

| Concept | What you create in tono | What it changes |
|---|---|---|
| **Pitch** | A note such as `C4` in a song, or `440` Hz on a raw oscillator | How high or low a tone sounds |
| **Tempo** | A song’s BPM, such as `120` | How quickly beats pass; 120 BPM means two beats per second |
| **Instrument** | A piano, bass, drum kit, or synthesized voice | The character of the sound, called its timbre |
| **Note** | A pitch, start time, and duration | One event: what plays and when |
| **Pattern / sequence** | A group of timed notes | A reusable melody, rhythm, or chord progression |
| **Track** | An instrument and the patterns assigned to it | One part of a song, with its own mix settings |
| **Arrangement** | Patterns placed at particular bars | When each part enters, repeats, or stops |
| **Envelope / effects** | Volume shape, filtering, delay, and other processing | How a note starts, fades, and sounds |

A **beat** is a regular pulse. A **bar** groups beats; the examples use four beats per bar, called 4/4 time. At 120 BPM, one beat lasts 0.5 seconds and one four-beat bar lasts 2 seconds.

## How a song becomes audio

```text
Timed notes → Pattern
Instrument → Track
Pattern placements on tracks → Song

Song → compile → Program → render → Audio samples → WAV
```

Create a `Song` with a tempo, give a track an instrument, make a `Pattern` of notes, and arrange that pattern on the track. Compiling checks the score and produces a **Program**, a reusable bundle containing the resolved sound graph. Rendering produces numbers representing the audio waveform.

You can also create a sound effect directly as a **SoundDoc**: a JSON graph of sound sources and processing nodes. A `seq` node is the lower-level sequencer used for timed notes; the Song and Pattern APIs handle this graph for you. A one-shot laser or impact can use oscillators, noise, and envelopes without needing a song.

## Learn in order

1. [Install tono](/get-started/) and [render your first sound](/get-started/quickstart).
2. [Pitch and notes](/get-started/pitch): name a note, move it up an octave, and create a phrase.
3. [Tempo and patterns](/get-started/rhythm): place notes on beats, create rests, and repeat a pattern.
4. [Instruments and tone](/get-started/instruments): play the same phrase with different voices and shape its sound.

Then use the task guides to [compose songs](/guides/songs), [design effects](/guides/sound-effects), or [run scheduled audio](/guides/live).

## Samples, seconds, and sound files

An audio **sample** is one amplitude value. A **frame** contains one sample per channel. **Sample rate** tells you how many frames are produced per second. At 48 kHz, a one-second mono render has 48,000 samples. Stereo has a left and right sample for every frame. Sample rate controls how densely audio is sampled; tempo controls musical timing.

For a mono SoundDoc, Rust’s renderer returns `Vec<f32>` and Python’s returns a NumPy `float32` array. For a song, Rust’s `render_stereo()` returns separate left and right vectors; Python’s `Program.render()` returns an array shaped `(frames, 2)`. A WAV stores those samples with metadata such as sample rate and channel count. Rendering creates audio; opening a WAV in a player or starting a playback runtime lets you hear it.

Keep the editable Song or SoundDoc recipe with the WAV so you can change it and render again. A saved Program can be reloaded for playback or rendering without compiling the score again.
