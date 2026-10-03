//! The output stage: loop-body extraction, loudness normalization + true-peak
//! limiting, and the write-time stereo treatments (Haas / Wide).

use super::Signal;
use crate::dsl::{Normalize, Stereo};
use crate::dsp::{db_to_lin, loudness_lufs_gated, peak_limit, true_peak_oversampled};
use std::f32::consts::FRAC_PI_2;

/// Extract `[start_secs, end_secs)` and blend its tail onto its head with an
/// equal-power crossfade. Returns the region length minus the crossfade;
/// a degenerate region returns the input unchanged.
pub fn make_loop_buffer(
    samples: &[f32],
    sr: u32,
    start_secs: f32,
    end_secs: Option<f32>,
    crossfade_secs: f32,
) -> Signal {
    let len = samples.len();
    let s = ((start_secs * sr as f32) as usize).min(len);
    let e = end_secs
        .map(|x| (x * sr as f32) as usize)
        .unwrap_or(len)
        .min(len);
    if e <= s {
        return samples.to_vec();
    }
    let region = &samples[s..e];
    let l = region.len();
    let x = ((crossfade_secs * sr as f32) as usize).min(l / 2);
    if x == 0 {
        return region.to_vec();
    }
    let out_len = l - x;
    let mut out = region[..out_len].to_vec();
    for (i, o) in out.iter_mut().take(x).enumerate() {
        let t = (i as f32 + 0.5) / x as f32;
        let fade_in = crate::dsp::sin(FRAC_PI_2 * t);
        let fade_out = crate::dsp::cos(FRAC_PI_2 * t);
        *o = region[i] * fade_in + region[out_len + i] * fade_out;
    }
    out
}

/// The loop-seam discontinuity in dB: the sample jump from the last sample back
/// to the first (lower ⇒ a cleaner seamless loop).
pub fn loop_seam_db(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return -120.0;
    }
    let jump = (samples[0] - samples[samples.len() - 1]).abs();
    20.0 * jump.max(1e-9).log10()
}

/// Normalize all channels with one shared gated-loudness gain, then limit
/// against the oversampled true peak while preserving stereo balance.
pub(super) fn normalize_output(channels: &mut [&mut [f32]], nz: &Normalize, sr: u32) {
    let ceil = db_to_lin(nz.ceiling_dbtp);
    if let Some(target) = nz.target_lufs {
        for _ in 0..2 {
            let cur = {
                let views: Vec<&[f32]> = channels.iter().map(|c| &**c).collect();
                loudness_lufs_gated(&views, sr)
            };
            if cur <= -120.0 {
                break;
            }
            let g = db_to_lin(target - cur);
            for c in channels.iter_mut() {
                for x in c.iter_mut() {
                    *x *= g;
                }
                soft_limit(c, ceil);
            }
        }
    }
    // Shared true-peak gain, then the joint sample-peak safety net.
    let tp = channels
        .iter()
        .map(|c| true_peak_oversampled(c))
        .fold(0.0f32, f32::max);
    if tp > ceil && tp > 0.0 {
        let g = ceil / tp;
        for c in channels.iter_mut() {
            for x in c.iter_mut() {
                *x *= g;
            }
        }
    }
    peak_limit(channels);
}

/// Soft-knee peak limiter: transparent below `0.7 × ceil`, smoothly (tanh)
/// compressed above, never exceeding `ceil`. C1-continuous at the knee.
fn soft_limit(samples: &mut [f32], ceil: f32) {
    const KNEE: f32 = 0.7;
    // A degenerate ceiling must not turn the mix into inf/NaN.
    let ceil = ceil.max(1e-9);
    for x in samples.iter_mut() {
        let v = *x / ceil;
        let a = v.abs();
        if a > KNEE {
            let compressed = KNEE + (1.0 - KNEE) * crate::dsp::tanh((a - KNEE) / (1.0 - KNEE));
            *x = v.signum() * compressed * ceil;
        }
    }
}

/// Turn a finished mono render into a stereo (left, right) pair per the doc's
/// [`Stereo`] mode. Mono is the identity; Haas / Wide add width. The pair is
/// jointly peak-limited so widening never clips.
pub fn stereoize(mono: &[f32], stereo: Stereo, sr: u32) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = match stereo {
        Stereo::Mono => (mono.to_vec(), mono.to_vec()),
        Stereo::Haas { ms, pan } => {
            let d = ((ms / 1000.0) * sr as f32) as usize;
            let delayed: Vec<f32> = (0..mono.len())
                .map(|i| if i >= d { mono[i - d] } else { 0.0 })
                .collect();
            // pan >= 0 → right leads (left is the delayed/trailing side).
            if pan >= 0.0 {
                (delayed, mono.to_vec())
            } else {
                (mono.to_vec(), delayed)
            }
        }
        Stereo::Wide { amount } => {
            let dec = allpass_decorrelate(mono, sr);
            let a = amount.clamp(0.0, 1.0);
            let mut l = Vec::with_capacity(mono.len());
            let mut r = Vec::with_capacity(mono.len());
            for i in 0..mono.len() {
                let mid = mono[i];
                let side = a * (mono[i] - dec[i]) * 0.5;
                l.push(mid + side);
                r.push(mid - side);
            }
            (l, r)
        }
    };
    peak_limit(&mut [&mut l, &mut r]);
    (l, r)
}

/// Decorrelate a mono signal with a short Schroeder all-pass chain (for the
/// `Wide` pseudo-stereo mode).
fn allpass_decorrelate(input: &[f32], sr: u32) -> Vec<f32> {
    let scale = sr as f32 / 44_100.0;
    let mut sig = input.to_vec();
    for &tune in &[225usize, 556, 441] {
        let len = ((tune as f32 * scale) as usize).max(1);
        let mut buf = vec![0.0f32; len];
        let mut idx = 0usize;
        let g = 0.7;
        for s in sig.iter_mut() {
            let buffered = buf[idx];
            let y = -*s * g + buffered;
            buf[idx] = *s + buffered * g;
            idx = (idx + 1) % len;
            *s = y;
        }
    }
    sig
}
