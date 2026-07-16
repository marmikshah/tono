//! The output stage: loop-body extraction, loudness normalization + true-peak
//! limiting, and the write-time stereo treatments (Haas / Wide).

use super::Signal;
use crate::dsl::{Normalize, Stereo};
use crate::dsp::{
    db_to_lin, loudness_lufs, loudness_lufs_gated, peak_limit, true_peak, true_peak_oversampled,
};
use std::f32::consts::FRAC_PI_2;

/// Extract the loop region `[start_secs, end_secs)` and equal-power crossfade
/// its last `crossfade_secs` onto its head, returning a buffer that repeats
/// seamlessly. The output length is the region minus the crossfade.
///
/// Overlap-loop: with region `r` of length `L` and crossfade `x`, the body is
/// `r[0..L-x]` with its first `x` samples replaced by a sin/cos blend of the
/// head (`r[i]`, fading in) and the tail (`r[L-x+i]`, fading out). The wrap
/// `out[last] → out[0]` then lands on adjacent original samples, so there is no
/// discontinuity.
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
        let fade_in = (FRAC_PI_2 * t).sin();
        let fade_out = (FRAC_PI_2 * t).cos();
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

/// Opt-in output stage: gain-match to a LUFS target (if given), soft-limiting
/// peaks into the `ceiling_dbtp` true-peak ceiling. Unlike a whole-buffer
/// attenuation, the soft-knee limiter only compresses the peaks, so dense /
/// peaky material (a BGM mix, layered impacts) actually REACHES the loudness
/// target instead of being dragged back down. Two measure→gain→limit passes
/// converge within ~1 dB.
pub(super) fn normalize_output(samples: &mut [f32], nz: &Normalize) {
    let ceil = db_to_lin(nz.ceiling_dbtp);
    if let Some(target) = nz.target_lufs {
        for _ in 0..2 {
            let cur = loudness_lufs(samples);
            if cur <= -120.0 {
                break;
            }
            let g = db_to_lin(target - cur);
            for x in samples.iter_mut() {
                *x *= g;
            }
            soft_limit(samples, ceil);
        }
    }
    // Safety: catch inter-sample residue above the ceiling, then sample peak.
    true_peak_limit(samples, nz.ceiling_dbtp);
    peak_limit(&mut [samples]);
}

/// Engine ≥ 4 output stage over the whole program (1 = mono, 2 = stereo):
/// loudness is measured jointly with gated BS.1770 at the actual sample rate
/// and corrected with ONE shared gain (per-channel matching collapsed any
/// asymmetric mix toward center), and the ceiling is enforced against a real
/// oversampled true-peak estimate (the legacy linear estimate could never see
/// an inter-sample over, so the documented dBTP ceiling was not honored).
pub(super) fn normalize_output_v4(channels: &mut [&mut [f32]], nz: &Normalize, sr: u32) {
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
            let compressed = KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh();
            *x = v.signum() * compressed * ceil;
        }
    }
}

/// Scale so the estimated true peak sits at or below `ceiling_dbtp`. Pure
/// attenuation (never boosts), so it composes after loudness matching.
fn true_peak_limit(samples: &mut [f32], ceiling_dbtp: f32) {
    let ceil = db_to_lin(ceiling_dbtp);
    let tp = true_peak(samples);
    if tp > ceil && tp > 0.0 {
        let g = ceil / tp;
        for x in samples.iter_mut() {
            *x *= g;
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
