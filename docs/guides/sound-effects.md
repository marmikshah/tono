# Design sound effects

[Install tono](/get-started/) first. Save each Rust example as `src/main.rs` and run `cargo run`, or save the Python code as `sfx.py` and run `python sfx.py`.

## Make a laser zap

A square wave falls from 880 to 180 Hz while an envelope fades it out. This writes `laser.wav` and its editable `laser.json` recipe: 0.25 seconds, mono, 48 kHz.

::: code-group

```rust [Rust]
use std::error::Error;
use tono_core::{dsl::SoundDoc, render::render};

fn main() -> Result<(), Box<dyn Error>> {
    let recipe = serde_json::json!({
        "name": "laser", "duration": 0.25, "sample_rate": 48000,
        "version": 2, "engine": 5, "seed": 42,
        "root": { "type": "mul", "inputs": [
            { "type": "square", "duty": 0.25,
              "freq": { "slide": {
                  "from": 880, "to": 180, "secs": 0.18, "curve": "exp"
              } } },
            { "type": "env", "a": 0.001, "d": 0.18, "s": 0.0, "r": 0.04 }
        ] }
    });
    let doc: SoundDoc = serde_json::from_value(recipe)?;
    doc.validate()?;
    std::fs::write("laser.json", serde_json::to_string_pretty(&doc)?)?;

    let spec = hound::WavSpec {
        channels: 1, sample_rate: doc.sample_rate, bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create("laser.wav", spec)?;
    for sample in render(&doc) {
        wav.write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    wav.finalize()?;
    Ok(())
}
```

```python [Python]
import json
from pathlib import Path
import wave

import numpy as np
import tono

recipe = {
    "name": "laser", "duration": 0.25, "sample_rate": 48000,
    "version": 2, "engine": 5, "seed": 42,
    "root": {"type": "mul", "inputs": [
        {"type": "square", "duty": 0.25,
         "freq": {"slide": {
             "from": 880, "to": 180, "secs": 0.18, "curve": "exp",
         }}},
        {"type": "env", "a": 0.001, "d": 0.18, "s": 0.0, "r": 0.04},
    ]},
}
Path("laser.json").write_text(json.dumps(recipe, indent=2))
audio = tono.render(json.dumps(recipe))
pcm = (np.clip(audio, -1.0, 1.0) * 32767.0).astype("<i2")
with wave.open("laser.wav", "wb") as wav:
    wav.setnchannels(1)
    wav.setsampwidth(2)
    wav.setframerate(recipe["sample_rate"])
    wav.writeframes(pcm.tobytes())
```

:::

Open `laser.wav` in an audio player. `mul` applies the envelope to the oscillator; frequencies are in Hz and envelope times are in seconds.

## Lower the pitch of a saved recipe

Run this from the directory containing `laser.json`. It halves both ends of the sweep, saves `low-laser.json`, and renders the result into memory.

::: code-group

```rust [Rust]
use std::error::Error;
use tono_core::{dsl::SoundDoc, render::render};

fn main() -> Result<(), Box<dyn Error>> {
    let mut recipe: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("laser.json")?)?;
    recipe["name"] = serde_json::json!("low-laser");
    let slide = &mut recipe["root"]["inputs"][0]["freq"]["slide"];
    slide["from"] = serde_json::json!(440);
    slide["to"] = serde_json::json!(90);
    let doc: SoundDoc = serde_json::from_value(recipe)?;
    doc.validate()?;
    std::fs::write("low-laser.json", serde_json::to_string_pretty(&doc)?)?;
    let audio = render(&doc);
    println!("Rendered {} mono samples", audio.len());
    Ok(())
}
```

```python [Python]
import json
from pathlib import Path

import tono

recipe = json.loads(Path("laser.json").read_text())
recipe["name"] = "low-laser"
slide = recipe["root"]["inputs"][0]["freq"]["slide"]
slide.update({"from": 440, "to": 90})
edited = json.dumps(recipe, indent=2)
audio = tono.render(edited)
Path("low-laser.json").write_text(edited)
print(f"Rendered {audio.size} mono samples")
```

:::

Both versions contain 12,000 samples. To save the edited audio, reuse the WAV writer above with `low-laser.wav` as the filename.

## Change the character

Edit the first recipe and run it again:

| Change | Result |
|---|---|
| Swap `from` and `to` | Rising jump sound |
| Use `"type": "sine"` instead of `"square"`, and remove `duty` | Softer tone |
| Raise envelope `a` to `0.02` | Gentler attack |
| Raise `d` to `0.4` and document `duration` to `0.5` | Longer tail |

For ready-made variations, see [generating game SFX](/guides/generation). For layered sounds, filters, noise, and modulation, use the [SoundDoc node reference](/reference/sounddoc). The [sound library](/sounds) includes editable recipes to start from.
