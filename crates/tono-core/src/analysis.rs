//! Audio analysis: level/spectral stats, a spectrogram PNG, and a waveform PNG.
//!
//! This is what gives the agent "ears": after every render it reads these
//! numbers and views the images, then refines the graph.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dsp::{dbfs, loudness_lufs_gated, true_peak_oversampled};
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
    /// Spectral flatness, 0..1 — geometric ÷ arithmetic mean of the power
    /// spectrum. Near 0 = tonal/pitched (energy in a few partials); near 1 =
    /// noisy/hissy (energy spread evenly). Read it to tell a clean tone from a
    /// noise texture, or to catch a sound that turned buzzy.
    #[serde(default)]
    pub spectral_flatness: f32,
    /// Off-harmonic energy ratio, 0..1 — the share of spectral energy that does
    /// NOT sit on the harmonic grid of the detected fundamental. Low for a
    /// clean pitched tone; high for noise, deliberately inharmonic bodies
    /// (bells, metal), AND for aliasing/foldback — so it is the meter that
    /// shows an anti-aliasing fix working (e.g. engine-1 `drive` vs the raw
    /// curve). Interpret with the sound's intent: high on a "pure tone" means
    /// alias dirt; high on a bell is just the bell.
    #[serde(default)]
    pub inharmonicity: f32,
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
    /// Attack sharpness in dB/ms — how steeply the onset rises (the ~19 dB from
    /// 10% to 90% of peak ÷ the attack time). Big = a snappy click/impact;
    /// small = a slow swell/pad. The transient readout to act on directly.
    #[serde(default)]
    pub attack_slope_db_per_ms: f32,
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

/// Compute every numeric descriptor with **no filesystem access** — pure
/// compute. Pair it with [`spectrogram_png`] / [`waveform_png`] for the images;
/// writing them to disk is the shell's job. The `*_png_path` fields are left
/// empty for the caller to fill once it has written the files.
pub fn stats(samples: &[f32], sample_rate: u32) -> Analysis {
    let srf = sample_rate as f32;
    let duration_secs = samples.len() as f32 / srf;

    let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let rms = if samples.is_empty() {
        0.0
    } else {
        (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
    };

    let frames = stft(samples);
    let t = transients(samples, sample_rate);
    let peak_dbfs = dbfs(peak);
    let rms_dbfs = dbfs(rms);

    Analysis {
        duration_secs,
        peak_dbfs,
        rms_dbfs,
        spectral_centroid_hz: spectral_centroid(&frames, srf),
        spectral_flatness: spectral_flatness(&frames),
        inharmonicity: inharmonicity(&frames, srf),
        // Metering uses the honest kernels: a real oversampled true-peak
        // (linear interpolation could never read above the sample peak) and
        // gated, rate-correct BS.1770 loudness.
        true_peak_dbfs: dbfs(true_peak_oversampled(samples)),
        crest_factor_db: peak_dbfs - rms_dbfs,
        loudness_lufs: loudness_lufs_gated(&[samples], sample_rate),
        attack_time_ms: t.attack_ms,
        attack_slope_db_per_ms: t.attack_slope,
        decay_time_ms: t.decay_ms,
        onset_count: t.onsets,
        head_silence_ms: t.head_ms,
        tail_silence_ms: t.tail_ms,
        layers: Vec::new(), // filled by the caller for mixer documents
        spectrogram_png_path: String::new(),
        waveform_png_path: String::new(),
    }
}

/// [`stats`] over a stereo pair: the level metrics (peak, RMS, true peak,
/// LUFS) measure the actual two-channel program — BS.1770 channel-energy sum
/// for loudness, max across channels for peaks — while the spectral/transient
/// descriptors read the mid, which is what they describe best. Metering a
/// stereo export on its mono mid under-reads hard-panned peaks by up to 6 dB
/// and wide material by ~3 LU.
pub fn stats_stereo(left: &[f32], right: &[f32], sample_rate: u32) -> Analysis {
    let n = left.len().min(right.len());
    let (left, right) = (&left[..n], &right[..n]);
    let mid: Vec<f32> = (0..n).map(|i| 0.5 * (left[i] + right[i])).collect();
    let mut a = stats(&mid, sample_rate);
    let peak = left
        .iter()
        .chain(right.iter())
        .fold(0.0f32, |m, &x| m.max(x.abs()));
    let energy: f64 = left
        .iter()
        .chain(right.iter())
        .map(|x| *x as f64 * *x as f64)
        .sum();
    let rms = if n == 0 {
        0.0
    } else {
        ((energy / (2 * n) as f64) as f32).sqrt()
    };
    a.peak_dbfs = dbfs(peak);
    a.rms_dbfs = dbfs(rms);
    a.crest_factor_db = a.peak_dbfs - a.rms_dbfs;
    a.true_peak_dbfs = dbfs(true_peak_oversampled(left).max(true_peak_oversampled(right)));
    a.loudness_lufs = loudness_lufs_gated(&[left, right], sample_rate);
    a
}

/// Time-domain transient descriptors derived from a one-pole amplitude envelope.
struct Transients {
    attack_ms: f32,
    attack_slope: f32,
    decay_ms: f32,
    onsets: u32,
    head_ms: f32,
    tail_ms: f32,
}

/// dB rise from the 10%-of-peak onset to 90% of peak (20·log10(0.9/0.1)).
const ATTACK_RISE_DB: f32 = 19.085;

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
        attack_slope: 0.0,
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
    // Sharpness in dB/ms; floor the time at one sample so an instant onset
    // reports a large-but-finite slope instead of dividing by zero.
    let attack_slope = ATTACK_RISE_DB / attack_ms.max(ms(1));

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
        attack_slope,
        decay_ms,
        onsets,
        head_ms,
        tail_ms,
    }
}

/// The waveform / amplitude-envelope image (min/max per column on a dark field),
/// encoded to bytes by [`waveform_png`].
fn waveform_image(samples: &[f32]) -> image::RgbImage {
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
    img
}

/// PNG-encode an RGB image into a byte buffer (no filesystem) — the form the
/// WASM playground and the MCP server hand to clients without a disk round-trip.
fn png_bytes(img: &image::RgbImage) -> anyhow::Result<Vec<u8>> {
    use image::{ImageEncoder, codecs::png::PngEncoder};
    let mut buf = Vec::new();
    PngEncoder::new(&mut buf).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

/// Render a sound's waveform image straight to PNG bytes.
pub fn waveform_png(samples: &[f32]) -> anyhow::Result<Vec<u8>> {
    png_bytes(&waveform_image(samples))
}

/// Render a sound's log-frequency spectrogram straight to PNG bytes.
pub fn spectrogram_png(samples: &[f32]) -> anyhow::Result<Vec<u8>> {
    png_bytes(&spectrogram_image(&stft(samples)))
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

/// Frame-averaged power per bin (skipping DC), shared by the spectral
/// descriptors. Length `FFT_SIZE/2`, index 0 (DC) left at 0.
fn power_spectrum(frames: &[Vec<f32>]) -> Vec<f64> {
    let bins = FFT_SIZE / 2;
    let mut power = vec![0.0f64; bins];
    for frame in frames {
        for (k, &m) in frame.iter().enumerate().take(bins).skip(1) {
            power[k] += (m as f64) * (m as f64);
        }
    }
    power
}

/// Spectral flatness in [0,1]: geometric ÷ arithmetic mean of the (non-DC)
/// power spectrum. ~0 for a tone (energy in a few bins), ~1 for flat noise.
fn spectral_flatness(frames: &[Vec<f32>]) -> f32 {
    if frames.is_empty() {
        return 0.0;
    }
    let power = power_spectrum(frames);
    let used = &power[1..];
    let total: f64 = used.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let n = used.len() as f64;
    let log_sum: f64 = used.iter().map(|&p| p.max(1e-20).ln()).sum();
    let geo = (log_sum / n).exp();
    let arith = total / n;
    (geo / arith).clamp(0.0, 1.0) as f32
}

/// Off-harmonic energy ratio in [0,1]: `1 − (energy near harmonics of the
/// detected fundamental ÷ total energy)`. The fundamental is the strongest
/// spectral peak above ~40 Hz in the frame-averaged spectrum; each harmonic
/// contributes a leakage-aware tolerance band, counted once. High for noise,
/// inharmonic bodies, and aliasing/foldback alike.
fn inharmonicity(frames: &[Vec<f32>], sample_rate: f32) -> f32 {
    if frames.is_empty() {
        return 0.0;
    }
    let bins = FFT_SIZE / 2;
    let bin_hz = sample_rate / FFT_SIZE as f32;
    let power = power_spectrum(frames);
    let total: f64 = power.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    // Fundamental = strongest bin above ~40 Hz.
    let min_bin = ((40.0 / bin_hz).ceil() as usize).max(1);
    let (mut peak_bin, mut peak_pow) = (min_bin, 0.0f64);
    for (k, &p) in power.iter().enumerate().take(bins).skip(min_bin) {
        if p > peak_pow {
            peak_pow = p;
            peak_bin = k;
        }
    }
    let f0 = peak_bin as f32 * bin_hz;
    if f0 <= 0.0 {
        return 0.0;
    }
    // Sum energy within a tolerance band of each harmonic k·f0, each bin once.
    let nyq = sample_rate * 0.5;
    let mut counted = vec![false; bins];
    let mut harm = 0.0f64;
    let mut k = 1;
    loop {
        let fh = f0 * k as f32;
        if fh >= nyq {
            break;
        }
        let tol = (0.03 * fh).max(1.5 * bin_hz); // leakage-aware
        let lo = (((fh - tol) / bin_hz).floor().max(0.0)) as usize;
        let hi = (((fh + tol) / bin_hz).ceil() as usize).min(bins - 1);
        for (b, c) in counted.iter_mut().enumerate().take(hi + 1).skip(lo) {
            if !*c {
                *c = true;
                harm += power[b];
            }
        }
        k += 1;
    }
    (1.0 - (harm / total).min(1.0)).clamp(0.0, 1.0) as f32
}

/// The log-frequency spectrogram image (time on X, LOG frequency on Y, low
/// frequencies at the bottom), encoded to bytes by [`spectrogram_png`]. A log
/// axis spreads the bass/low-mids — where pitched bodies, basslines and modal
/// partials live — instead of crushing them into the bottom strip a linear axis
/// gives.
fn spectrogram_image(frames: &[Vec<f32>]) -> image::RgbImage {
    use image::{ImageBuffer, Rgb, RgbImage};

    let bins = FFT_SIZE / 2;
    let target_w = 800u32;
    let target_h = 256u32;

    if frames.is_empty() {
        return ImageBuffer::from_pixel(target_w, target_h, Rgb(magma(0.0)));
    }

    // Normalize on a dB scale across the whole image for good contrast.
    let max_mag = frames
        .iter()
        .flat_map(|f| f.iter())
        .fold(1e-9f32, |m, &x| m.max(x));

    let num_frames = frames.len();
    // Log-frequency Y: each output row maps to a bin between bin 1 and the
    // top bin on a log scale (low frequencies at the bottom). `ln_range` is
    // the log span; `frac` runs 0 (bottom) → 1 (top).
    let bin_lo = 1.0f32;
    let bin_hi = (bins - 1) as f32;
    let ln_range = (bin_hi / bin_lo).ln();

    let mut img: RgbImage = ImageBuffer::new(target_w, target_h);
    for xo in 0..target_w {
        let fi = (xo as usize * num_frames / target_w as usize).min(num_frames - 1);
        let frame = &frames[fi];
        for yo in 0..target_h {
            let frac = (target_h - 1 - yo) as f32 / (target_h - 1) as f32;
            let bin_f = bin_lo * (frac * ln_range).exp();
            let k = (bin_f.round() as usize).min(bins - 1);
            let mag = frame.get(k).copied().unwrap_or(0.0);
            // dB relative to the loudest bin, mapped into [0, 1] over a 70 dB range.
            let db = 20.0 * (mag / max_mag).max(1e-9).log10();
            let t = ((db + 70.0) / 70.0).clamp(0.0, 1.0);
            img.put_pixel(xo, yo, Rgb(magma(t)));
        }
    }
    img
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
        let a = stats(&samples, sr);

        assert!((a.duration_secs - 0.1).abs() < 0.01);
        assert!((a.peak_dbfs + 6.02).abs() < 0.2); // 0.5 amplitude ≈ −6 dBFS
        assert!((a.rms_dbfs + 9.03).abs() < 0.3); // sine RMS = peak − 3.01 dB
        assert!((a.spectral_centroid_hz - 440.0).abs() < 60.0);
        assert!(a.true_peak_dbfs >= a.peak_dbfs - 0.01);
        // Both feedback images encode to non-empty PNG bytes (writing them to
        // disk is the shell's job, not the pure core's).
        assert!(spectrogram_png(&samples).unwrap().len() > 8);
        assert!(waveform_png(&samples).unwrap().len() > 8);
    }

    #[test]
    fn stereo_stats_measure_the_pair_not_the_mid() {
        // A hard-panned signal: the 0.5·(L+R) mid halves it, so metering the
        // mid under-reads the shipped audio by 6 dB.
        let sr = 44_100u32;
        let left: Vec<f32> = (0..sr as usize / 2)
            .map(|i| 0.8 * (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let right = vec![0.0f32; left.len()];
        let s = stats_stereo(&left, &right, sr);
        let mid: Vec<f32> = left.iter().map(|x| 0.5 * x).collect();
        let m = stats(&mid, sr);
        assert!(
            (s.peak_dbfs - (m.peak_dbfs + 6.02)).abs() < 0.1,
            "pair peak {} must read ~6 dB above the mid's {}",
            s.peak_dbfs,
            m.peak_dbfs
        );
        assert!(s.loudness_lufs > m.loudness_lufs + 2.0);
    }

    #[test]
    fn silence_hits_the_lufs_sentinel() {
        assert_eq!(loudness_lufs_gated(&[&[]], 44_100), -120.0);
        let quiet = loudness_lufs_gated(&[&vec![0.0f32; 4410]], 44_100);
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
        let a = stats(&samples, sr as u32);
        assert_eq!(a.onset_count, 2);
    }

    fn tone(sr: u32, hz: f32) -> Vec<f32> {
        (0..sr)
            .map(|i| (std::f32::consts::TAU * hz * i as f32 / sr as f32).sin() * 0.5)
            .collect()
    }

    fn noise(sr: u32, seed: u64) -> Vec<f32> {
        let mut rng = crate::dsp::Rng::new(seed);
        (0..sr).map(|_| rng.bi() * 0.5).collect()
    }

    #[test]
    fn spectral_flatness_separates_tone_from_noise() {
        let sr = 44_100u32;
        let ft = spectral_flatness(&stft(&tone(sr, 440.0)));
        let fnz = spectral_flatness(&stft(&noise(sr, 1)));
        assert!(
            ft < 0.05,
            "a pure tone should be tonal (flatness ≈ 0), got {ft}"
        );
        assert!(fnz > 0.3, "white noise should be flat, got {fnz}");
    }

    #[test]
    fn inharmonicity_low_for_tone_high_for_noise() {
        let sr = 44_100u32;
        let it = inharmonicity(&stft(&tone(sr, 440.0)), sr as f32);
        let inz = inharmonicity(&stft(&noise(sr, 2)), sr as f32);
        assert!(it < 0.25, "a tone's energy sits on its harmonics, got {it}");
        assert!(inz > 0.5, "noise has little harmonic energy, got {inz}");
    }

    #[test]
    fn attack_slope_is_steeper_for_a_click_than_a_swell() {
        let sr = 44_100u32;
        // Instant onset, then a decay.
        let click: Vec<f32> = (0..sr / 2)
            .map(|i| (1.0 - i as f32 / (sr as f32 * 0.5)).max(0.0))
            .collect();
        // A 200 ms linear swell up to level.
        let swell: Vec<f32> = (0..sr / 2)
            .map(|i| (i as f32 / (sr as f32 * 0.2)).min(1.0) * 0.8)
            .collect();
        let click_slope = transients(&click, sr).attack_slope;
        let swell_slope = transients(&swell, sr).attack_slope;
        assert!(
            click_slope > swell_slope * 5.0,
            "click {click_slope} should be far steeper than swell {swell_slope}"
        );
    }
}
