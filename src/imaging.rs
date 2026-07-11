//! Shell-side analysis I/O: write the feedback images to disk.
//!
//! `tono-core` stays pure compute — it produces the numbers ([`stats`]) and the
//! PNG bytes ([`spectrogram_png`] / [`waveform_png`]) but never touches the
//! filesystem. This shell helper is where those bytes become files.

use std::path::{Path, PathBuf};

use tono_core::analysis::{self, Analysis};

/// Sibling path for the waveform PNG: `<stem>_wave.png` next to the spectrogram.
fn waveform_path(png_path: &Path) -> PathBuf {
    let stem = png_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wave");
    png_path.with_file_name(format!("{stem}_wave.png"))
}

/// Compute the stats and write the spectrogram + waveform PNGs to disk, returning
/// the [`Analysis`] with both paths filled. The waveform lands at the `<stem>_wave`
/// sibling of `png_path`. Pass the stereo pair when the render has one, so the
/// level metrics measure the audio that actually ships (the images always read
/// the mono mid).
pub fn analyze_to_disk(
    mono: &[f32],
    stereo: Option<(&[f32], &[f32])>,
    sample_rate: u32,
    png_path: &Path,
) -> anyhow::Result<Analysis> {
    // One STFT feeds both the spectrogram image and the numeric stats — for a
    // minutes-long render it is the most expensive analysis step.
    let frames = analysis::spectral_frames(mono);
    std::fs::write(png_path, analysis::spectrogram_png_with(&frames)?)?;
    let wave_path = waveform_path(png_path);
    std::fs::write(&wave_path, analysis::waveform_png(mono)?)?;

    let mut a = match stereo {
        Some((l, r)) => analysis::stats_stereo_with(l, r, sample_rate, &frames),
        None => analysis::stats_with(mono, sample_rate, &frames),
    };
    a.spectrogram_png_path = png_path.to_string_lossy().into_owned();
    a.waveform_png_path = wave_path.to_string_lossy().into_owned();
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_both_images_and_fills_paths() {
        let sr = 44_100u32;
        let samples: Vec<f32> = (0..sr / 10)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin() * 0.5)
            .collect();
        let dir = std::env::temp_dir().join("tono_imaging_test");
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("sine.png");

        let a = analyze_to_disk(&samples, None, sr, &png).unwrap();

        assert!(Path::new(&a.spectrogram_png_path).exists());
        assert!(a.waveform_png_path.ends_with("sine_wave.png"));
        assert!(Path::new(&a.waveform_png_path).exists());
        assert!((a.duration_secs - 0.1).abs() < 0.01);
    }
}
