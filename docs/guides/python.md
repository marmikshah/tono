# Arrays and saved files

[Install tono](/get-started/) and run the [quickstart](/get-started/quickstart) to create `blip.json`. For each task, replace Rust's `src/main.rs` and run `cargo run`, or save the Python code as `example.py` and run `python example.py`. Run from the directory containing `blip.json`.

## Load a recipe into samples

Render the JSON, inspect its peak, and check that rendering it again gives identical samples. Python returns a mono `float32` NumPy array; Rust returns `Vec<f32>`.

::: code-group

```rust [Rust]
use std::error::Error;
use tono_core::{dsl::SoundDoc, render::render};

fn main() -> Result<(), Box<dyn Error>> {
    let json = std::fs::read_to_string("blip.json")?;
    let doc: SoundDoc = serde_json::from_str(&json)?;
    doc.validate()?;
    let audio = render(&doc);
    let peak = audio.iter().fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    println!("{} mono samples, peak={peak:.3}", audio.len());
    assert_eq!(audio, render(&doc));
    Ok(())
}
```

```python [Python]
from pathlib import Path

import numpy as np
import tono

recipe = Path("blip.json").read_text()
audio = tono.render(recipe)
print(f"{audio.size} mono samples, peak={np.abs(audio).max():.3f}")
assert audio.dtype == np.float32
assert np.array_equal(audio, tono.render(recipe))
```

:::

The quickstart recipe produces 12,000 samples. For WAV export, reuse the [quickstart writer](/get-started/quickstart).

## Save and reload a compiled song

This creates `riff.program.json`, reloads it with integrity checking, and renders the saved song.

::: code-group

```rust [Rust]
use std::error::Error;
use tono_core::{catalog::Bass, prelude::*, program::Program};

fn main() -> Result<(), Box<dyn Error>> {
    let mut song = Song::new("riff", 120.0).with_seed(7);
    song.add_voice("bass", &Bass::finger());
    song.add_pattern("notes", 1, vec![note(0, 2, "C2"), note(8, 2, "G2")]);
    song.arrange_repeat("bass", "notes", 0, 1);
    let program = song.compile(&CompileOptions {
        sample_rate: Some(48000), ..CompileOptions::default()
    }).expect("valid score");
    std::fs::write("riff.program.json", program.to_json())?;

    let loaded = Program::from_json(&std::fs::read_to_string("riff.program.json")?)?;
    assert_eq!(loaded.hash, program.hash);
    let (left, right) = loaded.render_stereo();
    println!("{} stereo frames", left.len());
    assert_eq!(left.len(), right.len());
    Ok(())
}
```

```python [Python]
import tono

song = tono.Song("riff", tempo=120, seed=7)
bass = song.track("bass", tono.instruments.bass("finger"))
notes = tono.Pattern(bars=1)
notes.note("C2", at=0, duration=0.5)
notes.note("G2", at=2, duration=0.5)
song.arrange(bass, notes, bars=[0])
program = song.compile(sample_rate=48000)
program.save("riff.program.json")

loaded = tono.Program.load("riff.program.json")
assert loaded.hash == program.hash
audio = loaded.render()
print(f"{audio.shape[0]} stereo frames")
assert audio.shape[1] == 2  # columns are left, right
```

:::

For more tracks and stereo WAV export, see [songs](/guides/songs). For scheduled playback without an audio device, see [live audio](/guides/live).
