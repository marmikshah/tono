//! Emit native sample bits for the browser ABI's cross-platform regression test.

use serde_json::{Value, json};

fn main() -> Result<(), String> {
    let mut cases: Vec<Value> = [
        include_str!("../../tono-core/tests/fixtures/blip.json"),
        include_str!("../../tono-core/tests/fixtures/hat.json"),
    ]
    .into_iter()
    .map(|text| serde_json::from_str(text).unwrap())
    .collect();
    for doc in &mut cases {
        doc["duration"] = json!(0.06);
    }
    let noise = json!({"name":"noise", "duration":0.06, "seed":98765, "root":{"type":"noise", "color":"pink"}});
    cases.push(noise.clone());
    let mut wide_seed = noise.clone();
    wide_seed["name"] = json!("u64-seed");
    wide_seed["seed"] = json!(u64::MAX);
    cases.push(wide_seed);
    let mut wide = noise.clone();
    wide["name"] = json!("wide");
    wide["stereo"] = json!({"mode":"wide", "amount":0.7});
    cases.push(wide);
    let mut looped = noise.clone();
    looped["name"] = json!("loop");
    looped["playback"] = json!({"mode":"loop", "crossfade_secs":0.01});
    cases.push(looped);
    let mut normalized = noise;
    normalized["name"] = json!("normalized");
    normalized["normalize"] = json!({"target_lufs":-16, "ceiling_dbtp":-1});
    cases.push(normalized);
    cases.push(
        json!({"name":"tracks", "duration":0.06, "root":{"type":"tracks", "tracks":[
            {"id":"left", "pan":-0.75, "node":{"type":"sine", "freq":220}},
            {"id":"right", "pan":0.75, "node":{"type":"sine", "freq":330}}
        ]}}),
    );
    cases.push(
        json!({"name":"sequence", "duration":0.08, "root":{"type":"seq", "bpm":120,
        "wave":"piano", "env":{"a":0.002, "d":0.01, "s":0.3, "r":0.02},
        "notes":[{"step":0, "len":1, "pitch":"C4"}, {"step":0, "len":1, "pitch":"G4"}]}}),
    );
    cases.push(
        json!({"name":"convolve", "duration":0.06, "seed":12, "root":{"type":"chain", "stages":[
            {"type":"noise"}, {"type":"convolve", "decay":0.03, "mix":0.5}
        ]}}),
    );
    cases.push(json!({"name":"granular", "duration":0.06, "seed":12, "root":{"type":"chain", "stages":[
        {"type":"sawtooth", "freq":220}, {"type":"granular", "grain_ms":10, "density":120, "pitch":1.1, "mix":0.5}
    ]}}));
    let references: Vec<_> = cases
        .into_iter()
        .map(|document| {
            let audio = tono_web::render_json(&serde_json::to_vec(&document).unwrap())?;
            Ok(json!({
            "document": document,
            "source": serde_json::to_string(&document).unwrap(),
                "sampleRate": audio.sample_rate,
                "leftBits": audio.left.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
                "rightBits": audio.right.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
            }))
        })
        .collect::<Result<_, String>>()?;
    println!(
        "{}",
        serde_json::to_string(&references).map_err(|e| e.to_string())?
    );
    Ok(())
}
