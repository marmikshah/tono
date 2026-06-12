//! Audio analysis: level/spectral stats, a spectrogram PNG, and a waveform PNG.
//!
//! This is what gives the agent "ears": after every render it reads these
//! numbers and views the images, then refines the graph.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::dsp::dbfs;
use rustfft::{FftPlanner, num_complex::Complex};

/// Numeric + visual summary of a rendered sound.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Analysis {
    /// Length of the sound in seconds.
    pub duration_secs: f32,
    /// Peak level in dBFS (0 = full scale).
    pub peak_dbfs: f32,
    /// RMS (average) level in dBFS.
    pub rms_dbfs: f32,
    /// Spectral centroid in Hz — the "center of mass" of the spectrum, a proxy
    /// for perceived brightness.
    pub spectral_centroid_hz: f32,
    /// Inter-sample (true) peak in dBFS, estimated by 4× oversampling. Values
    /// above 0 mean the signal will clip on conversion / playback.
    pub true_peak_dbfs: f32,
    /// Crest factor (peak − RMS) in dB — a punchiness / transient measure. Big
    /// for percussive hits, small for sustained / compressed material.
    pub crest_factor_db: f32,
    /// Approximate integrated loudness in LUFS (K-weighted, ungated). Useful for
    /// matching perceived level across a sound set; not a certified meter.
    pub loudness_lufs: f32,
    /// Attack time in ms: from the onset (envelope > 10% of peak) to first
    /// reaching 90% of peak. Small ⇒ a sharp/punchy transient.
    pub attack_time_ms: f32,
    /// Decay/tail length in ms: from the peak to the last point the envelope is
    /// above 10% of peak. Big ⇒ a long ringing tail.
    pub decay_time_ms: f32,
    /// Number of distinct attacks detected (hysteresis on the envelope). >1 ⇒ an
    /// arpeggio / multi-hit / accidental double-trigger.
    pub onset_count: u32,
    /// Leading near-silence in ms before the sound starts.
    pub head_silence_ms: f32,
    /// Trailing near-silence in ms after the sound ends (trim candidate).
    pub tail_silence_ms: f32,
    /// Per-layer contribution stats for mixer documents (post-fader,
    /// pre-master — a master compressor/reverb reshapes the bus after these
    /// are measured). Empty for plain documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<crate::render::LayerStats>,
    /// Filesystem path to the rendered spectrogram PNG.
    pub spectrogram_png_path: String,
    /// Filesystem path to the rendered waveform / amplitude-envelope PNG (so the
    /// agent can SEE the time-domain shape: attack, decay, double-triggers).
    pub waveform_png_path: String,
}

const FFT_SIZE: usize = 1024;
const HOP: usize = 256;

/// Analyze samples: compute stats, write a spectrogram PNG to `png_path`, and a
/// waveform PNG next to it at `<stem>_wave.png` (both paths are returned in the
/// [`Analysis`]).
pub fn analyze(samples: &[f32], sample_rate: u32, png_path: &Path) -> anyhow::Result<Analysis> {
    let srf = sample_rate as f32;
    let duration_secs = samples.len() as f32 / srf;

    let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let rms = if samples.is_empty() {
        0.0
    } else {
        (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
    };

    let frames = stft(samples);
    let centroid = spectral_centroid(&frames, srf);
    write_spectrogram(&frames, png_path)?;

    // Time-domain feedback: a waveform image + transient descriptors.
    let wave_path = waveform_path(png_path);
    write_waveform(samples, &wave_path)?;
    let t = transients(samples, sample_rate);

    let peak_dbfs = dbfs(peak);
    let rms_dbfs = dbfs(rms);

    Ok(Analysis {
        duration_secs,
        peak_dbfs,
        rms_dbfs,
        spectral_centroid_hz: centroid,
        true_peak_dbfs: dbfs(true_peak(samples)),
        crest_factor_db: peak_dbfs - rms_dbfs,
        loudness_lufs: loudness_lufs(samples),
        attack_time_ms: t.attack_ms,
        decay_time_ms: t.decay_ms,
        onset_count: t.onsets,
        head_silence_ms: t.head_ms,
        tail_silence_ms: t.tail_ms,
        layers: Vec::new(), // filled by the caller for mixer documents
        spectrogram_png_path: png_path.to_string_lossy().into_owned(),
        waveform_png_path: wave_path.to_string_lossy().into_owned(),
    })
}

/// Sibling path for the waveform PNG: `<stem>_wave.png` next to the spectrogram.
fn waveform_path(png_path: &Path) -> std::path::PathBuf {
    let stem = png_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wave");
    png_path.with_file_name(format!("{stem}_wave.png"))
}

/// Time-domain transient descriptors derived from a one-pole amplitude envelope.
struct Transients {
    attack_ms: f32,
    decay_ms: f32,
    onsets: u32,
    head_ms: f32,
    tail_ms: f32,
}

/// A one-pole amplitude-envelope follower: instant attack, ~3 ms release.
fn envelope(samples: &[f32], sr: u32) -> Vec<f32> {
    let rel = (-1.0 / (0.003 * sr as f32)).exp();
    let mut e = 0.0f32;
    samples
        .iter()
        .map(|x| {
            let r = x.abs();
            e = if r > e { r } else { r + rel * (e - r) };
            e
        })
        .collect()
}

fn transients(samples: &[f32], sr: u32) -> Transients {
    let zero = Transients {
        attack_ms: 0.0,
        decay_ms: 0.0,
        onsets: 0,
        head_ms: 0.0,
        tail_ms: 0.0,
    };
    if samples.len() < 2 {
        return zero;
    }
    let env = envelope(samples, sr);
    let peak = env.iter().fold(0.0f32, |m, &x| m.max(x));
    if peak < 1e-6 {
        return zero;
    }
    let srf = sr as f32;
    let ms = |samps: usize| samps as f32 / srf * 1000.0;
    let peak_idx = env
        .iter()
        .enumerate()
        .fold(
            (0usize, 0.0f32),
            |(bi, bv), (i, &v)| {
                if v > bv { (i, v) } else { (bi, bv) }
            },
        )
        .0;

    let onset = env.iter().position(|&v| v > 0.1 * peak).unwrap_or(0);
    let reach90 = env[onset..]
        .iter()
        .position(|&v| v >= 0.9 * peak)
        .map(|i| onset + i)
        .unwrap_or(peak_idx);
    let attack_ms = ms(reach90.saturating_sub(onset));

    let last_above = env
        .iter()
        .rposition(|&v| v > 0.1 * peak)
        .unwrap_or(peak_idx);
    let decay_ms = ms(last_above.saturating_sub(peak_idx));

    let sil = 0.02 * peak;
    let head = env.iter().position(|&v| v > sil).unwrap_or(0);
    let tail_last = env.iter().rposition(|&v| v > sil).unwrap_or(env.len() - 1);
    let head_ms = ms(head);
    let tail_ms = ms(env.len().saturating_sub(1).saturating_sub(tail_last));

    // Onset count with hysteresis (rising through 30% peak, re-arm below 15%).
    let (hi, lo) = (0.3 * peak, 0.15 * peak);
    let mut onsets = 0u32;
    let mut armed = true;
    for &v in &env {
        if armed && v > hi {
            onsets += 1;
            armed = false;
        } else if !armed && v < lo {
            armed = true;
        }
    }

    Transients {
        attack_ms,
        decay_ms,
        onsets,
        head_ms,
        tail_ms,
    }
}

/// Render a waveform / amplitude image (time on X, amplitude on Y, centered) so
/// the agent can read attack, decay, and double-triggers the spectrogram hides.
fn write_waveform(samples: &[f32], path: &Path) -> anyhow::Result<()> {
    use image::{ImageBuffer, Rgb, RgbImage};
    let w: u32 = 640;
    let h: u32 = 160;
    let mut img: RgbImage = ImageBuffer::from_pixel(w, h, Rgb([12, 12, 18]));
    let mid = (h / 2) as i32;
    let amp = (mid - 2) as f32;
    let n = samples.len().max(1);
    for x in 0..w {
        let start = (x as usize * n / w as usize).min(n - 1);
        let end = (((x + 1) as usize * n / w as usize).max(start + 1)).min(n);
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for &s in &samples[start..end] {
            lo = lo.min(s);
            hi = hi.max(s);
        }
        let y_hi = (mid as f32 - hi * amp) as i32;
        let y_lo = (mid as f32 - lo * amp) as i32;
        for y in y_hi.min(y_lo)..=y_hi.max(y_lo) {
            if (0..h as i32).contains(&y) {
                img.put_pixel(x, y as u32, Rgb([80, 220, 160]));
            }
        }
    }
    // Faint center line.
    for x in 0..w {
        img.put_pixel(x, mid as u32, Rgb([44, 44, 54]));
    }
    img.save(path)?;
    Ok(())
}

/// Estimate the inter-sample (true) peak by 4× linear-interpolation oversampling.
/// Returns linear amplitude (use [`crate::dsp::dbfs`] for dBTP). Exposed so the
/// renderer's output stage can limit to a true-peak ceiling.
pub fn true_peak(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    }
    let mut peak = 0.0f32;
    for w in samples.windows(2) {
        for k in 0..4 {
            let t = k as f32 / 4.0;
            let v = (w[0] * (1.0 - t) + w[1] * t).abs();
            if v > peak {
                peak = v;
            }
        }
    }
    peak
}

/// Approximate ITU-R BS.1770 K-weighted integrated loudness (ungated). The
/// K-weighting biquads use the standard 48 kHz coefficients, so this is an
/// approximation at other sample rates — fine for relative level matching.
/// Exposed so the renderer's output stage can gain-match to a LUFS target.
/// Returns −120 for silence.
pub fn loudness_lufs(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    // Stage 1: high-shelf. Stage 2: high-pass.
    let shelf = biquad_df1(
        samples,
        [1.535_124_9, -2.691_696_2, 1.198_392_8],
        [-1.690_659_3, 0.732_480_8],
    );
    let weighted = biquad_df1(&shelf, [1.0, -2.0, 1.0], [-1.990_047_5, 0.990_072_3]);
    let ms = weighted.iter().map(|x| x * x).sum::<f32>() / weighted.len() as f32;
    -0.691 + 10.0 * ms.max(1e-12).log10()
}

/// Direct-Form I biquad over a buffer. `b` = feed-forward, `a` = the two
/// feedback coefficients (a0 assumed 1).
fn biquad_df1(input: &[f32], b: [f32; 3], a: [f32; 2]) -> Vec<f32> {
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0, 0.0, 0.0);
    input
        .iter()
        .map(|&x0| {
            let y0 = b[0] * x0 + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
            x2 = x1;
            x1 = x0;
            y2 = y1;
            y1 = y0;
            y0
        })
        .collect()
}

/// Short-time Fourier transform → a Vec of magnitude frames, each of length
/// `FFT_SIZE / 2` (positive frequencies only).
fn stft(samples: &[f32]) -> Vec<Vec<f32>> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            // Hann window.
            let x = std::f32::consts::PI * i as f32 / (FFT_SIZE as f32 - 1.0);
            x.sin().powi(2)
        })
        .collect();

    let n = samples.len();
    let num_frames = if n <= FFT_SIZE {
        1
    } else {
        1 + (n - FFT_SIZE) / HOP
    };

    let bins = FFT_SIZE / 2;
    let mut frames = Vec::with_capacity(num_frames);
    let mut buf = vec![Complex { re: 0.0, im: 0.0 }; FFT_SIZE];
    for f in 0..num_frames {
        let start = f * HOP;
        for i in 0..FFT_SIZE {
            let s = samples.get(start + i).copied().unwrap_or(0.0);
            buf[i] = Complex {
                re: s * window[i],
                im: 0.0,
            };
        }
        fft.process(&mut buf);
        let mags: Vec<f32> = buf[..bins].iter().map(|c| c.norm()).collect();
        frames.push(mags);
    }
    frames
}

/// Spectral centroid (Hz) averaged over all STFT frames.
fn spectral_centroid(frames: &[Vec<f32>], sample_rate: f32) -> f32 {
    let bins = FFT_SIZE / 2;
    let bin_hz = sample_rate / FFT_SIZE as f32;
    let mut weighted = 0.0f64;
    let mut total = 0.0f64;
    for frame in frames {
        for (k, &mag) in frame.iter().enumerate().take(bins) {
            let f = k as f32 * bin_hz;
            weighted += (f * mag) as f64;
            total += mag as f64;
        }
    }
    if total > 0.0 {
        (weighted / total) as f32
    } else {
        0.0
    }
}

/// Render STFT magnitude frames to a PNG heatmap (time on X, frequency on Y,
/// low frequencies at the bottom).
fn write_spectrogram(frames: &[Vec<f32>], path: &Path) -> anyhow::Result<()> {
    use image::{ImageBuffer, Rgb, RgbImage, imageops};

    let bins = FFT_SIZE / 2;
    let w = frames.len().max(1) as u32;
    let h = bins as u32;

    // Normalize on a dB scale across the whole image for good contrast.
    let max_mag = frames
        .iter()
        .flat_map(|f| f.iter())
        .fold(1e-9f32, |m, &x| m.max(x));

    let mut img: RgbImage = ImageBuffer::new(w, h);
    for (x, frame) in frames.iter().enumerate() {
        for k in 0..bins {
            let mag = frame.get(k).copied().unwrap_or(0.0);
            // dB relative to the loudest bin, mapped into [0, 1] over a 70 dB range.
            let db = 20.0 * (mag / max_mag).max(1e-9).log10();
            let t = ((db + 70.0) / 70.0).clamp(0.0, 1.0);
            // Low frequencies at the bottom of the image.
            let y = (bins - 1 - k) as u32;
            img.put_pixel(x as u32, y, Rgb(magma(t)));
        }
    }

    // Scale to a readable fixed size (nearest-neighbor keeps it crisp & cheap).
    let target_w = w.clamp(1, 800).max(256);
    let target_h = 256u32;
    let scaled = imageops::resize(&img, target_w, target_h, imageops::FilterType::Nearest);
    scaled.save(path)?;
    Ok(())
}

/// Magma-ish colormap: maps t in [0,1] to an RGB triple (dark → purple →
/// orange → pale yellow).
fn magma(t: f32) -> [u8; 3] {
    const STOPS: [[f32; 3]; 5] = [
        [0.0, 0.0, 0.05],
        [0.30, 0.07, 0.40],
        [0.70, 0.18, 0.40],
        [0.98, 0.55, 0.25],
        [0.99, 0.96, 0.78],
    ];
    let t = t.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32;
    let i = t.floor() as usize;
    let frac = t - i as f32;
    let a = STOPS[i];
    let b = STOPS[(i + 1).min(STOPS.len() - 1)];
    [
        ((a[0] + (b[0] - a[0]) * frac) * 255.0) as u8,
        ((a[1] + (b[1] - a[1]) * frac) * 255.0) as u8,
        ((a[2] + (b[2] - a[2]) * frac) * 255.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_sine_reports_sane_levels_and_images() {
        let sr = 44_100u32;
        let samples: Vec<f32> = (0..sr / 10)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin() * 0.5)
            .collect();
        let dir = std::env::temp_dir().join("sonarium_analysis_test");
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("sine.png");
        let a = analyze(&samples, sr, &png).unwrap();

        assert!((a.duration_secs - 0.1).abs() < 0.01);
        assert!((a.peak_dbfs + 6.02).abs() < 0.2); // 0.5 amplitude ≈ −6 dBFS
        assert!((a.rms_dbfs + 9.03).abs() < 0.3); // sine RMS = peak − 3.01 dB
        assert!((a.spectral_centroid_hz - 440.0).abs() < 60.0);
        assert!(a.true_peak_dbfs >= a.peak_dbfs - 0.01);
        // Both feedback images land on disk, waveform at the `_wave` sibling.
        assert!(std::path::Path::new(&a.spectrogram_png_path).exists());
        assert!(a.waveform_png_path.ends_with("sine_wave.png"));
        assert!(std::path::Path::new(&a.waveform_png_path).exists());
    }

    #[test]
    fn silence_hits_the_lufs_sentinel() {
        assert_eq!(loudness_lufs(&[]), -120.0);
        let quiet = loudness_lufs(&vec![0.0f32; 4410]);
        assert!(quiet <= -120.0);
    }

    #[test]
    fn onsets_count_distinct_hits() {
        let sr = 44_100usize;
        let mut samples = vec![0.0f32; sr / 2];
        // Two short bursts separated by silence.
        for burst in [0usize, sr / 4] {
            for sample in samples.iter_mut().skip(burst).take(sr / 50) {
                *sample = 0.8;
            }
        }
        let dir = std::env::temp_dir().join("sonarium_analysis_test");
        std::fs::create_dir_all(&dir).unwrap();
        let a = analyze(&samples, sr as u32, &dir.join("bursts.png")).unwrap();
        assert_eq!(a.onset_count, 2);
    }
}
