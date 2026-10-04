# Your first sound

[Install tono](/get-started/) first. Save the Rust example as `src/main.rs` in `tono-demo`, or the Python example as `first_sound.py` in your activated environment.

This makes a short blip, then changes its frequency from 880 to 220 Hz for a lower version. Each version saves a WAV and an editable JSON recipe. No audio device is needed.

::: code-group

```rust [Rust]
use std::error::Error;
use tono_core::{dsl::SoundDoc, render::render};

fn main() -> Result<(), Box<dyn Error>> {
    let mut recipe = serde_json::json!({
        "name": "blip", "duration": 0.25, "sample_rate": 48000,
        "engine": 5, "seed": 7,
        "root": { "type": "mul", "inputs": [
            { "type": "sine", "freq": 880 },
            { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 }
        ] }
    });

    for (frequency, filename) in [(880, "blip.wav"), (220, "low-blip.wav")] {
        recipe["root"]["inputs"][0]["freq"] = serde_json::json!(frequency);
        let doc: SoundDoc = serde_json::from_value(recipe.clone())?;
        doc.validate()?;
        let audio = render(&doc);
        let spec = hound::WavSpec {
            channels: 1, sample_rate: doc.sample_rate, bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = hound::WavWriter::create(filename, spec)?;
        for sample in audio {
            wav.write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        }
        wav.finalize()?;
        std::fs::write(filename.replace(".wav", ".json"), serde_json::to_string_pretty(&doc)?)?;
    }
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
    "name": "blip", "duration": 0.25, "sample_rate": 48000,
    "engine": 5, "seed": 7,
    "root": {"type": "mul", "inputs": [
        {"type": "sine", "freq": 880},
        {"type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05},
    ]},
}

for frequency, filename in [(880, "blip.wav"), (220, "low-blip.wav")]:
    recipe["root"]["inputs"][0]["freq"] = frequency
    audio = tono.render(json.dumps(recipe))
    pcm = (np.clip(audio, -1.0, 1.0) * 32767.0).astype("<i2")
    with wave.open(filename, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(recipe["sample_rate"])
        wav.writeframes(pcm.tobytes())
    Path(filename).with_suffix(".json").write_text(json.dumps(recipe, indent=2))
```

:::

Run it:

::: code-group

```sh [Rust]
cargo run
```

```sh [Python]
python first_sound.py
```

:::

Open `blip.wav` and `low-blip.wav` in an audio player: both are 0.25-second, 48 kHz mono WAVs; the second is two octaves lower. The matching `.json` files can be edited or rendered by the CLI.

Next, learn [how sound works](/get-started/basics), then explore [pitch and notes](/get-started/pitch). For practical tasks, see [sound effects](/guides/sound-effects), [songs](/guides/songs), or [loading recipes and NumPy audio](/guides/python). The [SoundDoc reference](/reference/sounddoc) lists the available nodes and fields.
