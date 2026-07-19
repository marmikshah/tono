//! `tono play` — audition a document through the speakers (feature `play`).
//!
//! Slim, self-contained playback (the CLI is the published face, so it can't
//! depend on the unpublished `tono-play` crate): a buffered, non-looping
//! Player on a cpal stream, playing exactly the offline bounce.

use std::time::Duration;

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tono_core::dsl::SoundDoc;
use tono_core::runtime::write_interleaved;

/// Play `doc` through the default output device until it ends (or `secs`,
/// whichever comes first) — blocking.
pub fn play_doc(doc: &SoundDoc, secs: f32) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default output device"))?;
    let config = device.default_output_config()?;
    let channels = (config.channels() as usize).max(1);
    if config.sample_format() != cpal::SampleFormat::F32 {
        anyhow::bail!(
            "unsupported output sample format {:?} (need f32)",
            config.sample_format()
        );
    }
    let sample_rate = config.sample_rate().0;

    // The audition is the bounce, byte-identical: render at the device's rate.
    let mut d = doc.clone();
    d.sample_rate = sample_rate;
    let mut player = tono_core::player::Player::new(d);
    let total_frames = player.frames();
    player.looping = false;
    player.play();

    let mut scratch: Vec<f32> = Vec::new();
    let mut played = 0usize;
    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            // Never unwind into cpal's C frame (UB) — contain and go silent.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let frames = data.len() / channels;
                if scratch.len() < frames * 2 {
                    scratch.resize(frames * 2, 0.0);
                }
                let st = &mut scratch[..frames * 2];
                played += player.fill(st);
                write_interleaved(data, channels, st);
            }));
            if result.is_err() {
                data.fill(0.0);
            }
        },
        move |e| eprintln!("tono play: stream error: {e}"),
        None,
    )?;
    stream.play()?;

    // Return when the document has played out (or the time budget expires),
    // so `tono play` ends instead of idling on silence.
    let budget = Duration::from_secs_f32(secs.max(0.0));
    let start = std::time::Instant::now();
    while played < total_frames && start.elapsed() < budget {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    Ok(())
}
