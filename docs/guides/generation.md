# Generate game sound effects

Create a seeded batch from a built-in starter. [Install tono](/get-started/) first; the Python batch example also needs the optional CLI from that page. Save Rust code as `src/main.rs` and run `cargo run`, or save Python code as `generate.py` and run `python generate.py`.

## Generate four footsteps

This writes four WAVs and editable recipes to `candidates/`, using seeds 7–10. Use a new output directory for each batch. Python calls the CLI because the starter generator is currently a Rust API.

::: code-group

```rust [Rust]
use std::error::Error;
use tono_core::generate::{SfxSpec, SfxTemplate, generate_sfx};

fn main() -> Result<(), Box<dyn Error>> {
    std::fs::create_dir("candidates")?;
    for index in 0..4 {
        let mut spec = SfxSpec::new(SfxTemplate::Footstep, 7 + index);
        spec.brightness = 0.3;
        spec.punch = 0.7;
        spec.variation = 0.2;
        let doc = generate_sfx(&spec)?;
        let path = format!("candidates/footstep_v{index}");
        std::fs::write(format!("{path}.json"), serde_json::to_string_pretty(&doc)?)?;

        let audio = tono_core::render::render(&doc);
        let wav_spec = hound::WavSpec {
            channels: 2, sample_rate: doc.sample_rate, bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = hound::WavWriter::create(format!("{path}.wav"), wav_spec)?;
        for sample in audio {
            let pcm = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            wav.write_sample(pcm)?;
            wav.write_sample(pcm)?;
        }
        wav.finalize()?;
    }
    Ok(())
}
```

```python [Python]
import subprocess

subprocess.run([
    "tono", "generate", "footstep",
    "--seed", "7", "--count", "4",
    "--brightness", "0.3", "--punch", "0.7", "--variation", "0.2",
    "--sample-rate", "48000", "--format", "wav", "-o", "candidates",
], check=True)
```

:::

Open `candidates/footstep_v0.wav` through `footstep_v3.wav` to compare them. These are 48 kHz stereo files with identical left and right channels. The CLI also writes an `index.html` audition page, waveform images, analysis, and a manifest.

## Render a candidate into memory

Use the saved recipe in your game or audio pipeline. This renders the first candidate into mono floating-point samples without accessing a speaker.

::: code-group

```rust [Rust]
use std::error::Error;
use tono_core::{dsl::SoundDoc, render::render};

fn main() -> Result<(), Box<dyn Error>> {
    let json = std::fs::read_to_string("candidates/footstep_v0.json")?;
    let doc: SoundDoc = serde_json::from_str(&json)?;
    doc.validate()?;
    let audio: Vec<f32> = render(&doc);
    println!("{} samples at {} Hz", audio.len(), doc.sample_rate);
    Ok(())
}
```

```python [Python]
import json
from pathlib import Path

import tono

recipe = Path("candidates/footstep_v0.json").read_text()
audio = tono.render(recipe)  # Mono float32 NumPy array.
print(f"{audio.size} samples at {json.loads(recipe)['sample_rate']} Hz")
```

:::

Save the JSON alongside the audio: it contains the complete graph, seed, and engine revision needed to reproduce that sound.

## Pick a starter and tune it

In Rust, change `SfxTemplate::Footstep`; in Python, change `"footstep"` in the command.

| Rust | CLI / Python | Sound |
|---|---|---|
| `Coin` | `coin` | Rising pickup chime |
| `Jump` | `jump` | Upward arcade sweep |
| `Laser` | `laser` | Descending zap |
| `Explosion` | `explosion` | Noise blast and low body |
| `Impact` | `impact` | Resonant strike |
| `Footstep` | `footstep` | Contact thud |
| `UiConfirm` | `ui-confirm` | Ascending confirmation |
| `UiCancel` | `ui-cancel` | Descending cancellation |

`brightness`, `punch`, and `variation` accept values from 0 to 1. Increase brightness for a sharper tone, punch for a stronger attack, or variation for more pitch and timing differences. The same spec and seed reproduce the same recipe within its supported revision.

For a larger collection, browse the [sound library](/sounds) or [background music](/bgm). To build a graph yourself, follow [sound effects](/guides/sound-effects). See the [CLI reference](/reference/cli) for batch options and output formats.
