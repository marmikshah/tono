//! Small shared DSP core: the deterministic PRNG, dB conversions, and the
//! output peak limit. One copy of each — these protect the project's
//! determinism contract (same graph + seed ⇒ identical bytes), so they must
//! never fork per module.

/// The SplitMix64 golden-gamma increment — the seed-spacing constant every
/// deterministic stream derivation shares.
pub(crate) const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// FNV-1a offset basis and prime — the hashing primitives behind stable
/// layer/node stream keys.
pub(crate) const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
pub(crate) const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

/// The SplitMix64 finalizer — THE bit mixer behind every seed derivation
/// (`Rng`, `node_seed`, the per-track streams). One copy: a re-typed variant
/// that drifted by one constant would silently fork the determinism contract.
pub(crate) fn splitmix_mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic PRNG (SplitMix64). Seeded from a sound's seed so the same
/// graph + seed renders identical audio every time. `Clone` lets a stereo
/// master bus run the same processor on both channels with identical draws.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    /// A stream seeded at `seed` — the same seed replays the same draws.
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// The next 64 raw bits of the stream.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        splitmix_mix(self.0)
    }

    /// Uniform in [0, 1) with 24-bit resolution (a full f32 mantissa).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [-1, 1) — white noise samples.
    pub fn bi(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// FNV-1a over a layer id: the stable per-layer RNG stream key for schema-v2
/// mixer documents. Lives here (not in the renderer) because `validate` also
/// uses it to reject the rare id pair whose hashes collide — a collision would
/// silently give two layers identical noise.
pub fn layer_stream_key(id: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Descend a structural node-path key by one child index (FNV-1a step). The path
/// makes each RNG leaf's identity a deterministic function of its POSITION in the
/// graph (not of evaluation order), so under `engine >= 2` a per-sample streaming
/// render draws the same randomness as the offline whole-buffer render.
pub(crate) fn node_path(parent: u64, child: usize) -> u64 {
    (parent ^ (child as u64).wrapping_add(1)).wrapping_mul(FNV_PRIME)
}

/// Finalize a structural path (seeded from `doc.seed` at the root) into a per-node
/// RNG seed (SplitMix64 mix).
pub(crate) fn node_seed(path: u64) -> u64 {
    splitmix_mix(path.wrapping_add(GOLDEN_GAMMA))
}

/// The ADSR envelope value at time `t` seconds — the ONE copy of the envelope
/// math shared by the offline `adsr` buffer, the streaming per-sample
/// evaluator, and `EnvMod`. `rel_start` anchors the release (offline renders
/// anchor it to the end of the buffer).
pub(crate) fn adsr_env(t: f32, a: f32, d: f32, s: f32, r: f32, punch: f32, rel_start: f32) -> f32 {
    let mut v = if t < a {
        if a > 0.0 { t / a } else { 1.0 }
    } else if t < a + d {
        let p = if d > 0.0 { (t - a) / d } else { 1.0 };
        1.0 - (1.0 - s) * p
    } else if t < rel_start {
        s
    } else if r > 0.0 {
        let p = ((t - rel_start) / r).clamp(0.0, 1.0);
        s * (1.0 - p)
    } else {
        0.0
    };
    let punch_win = a + d;
    if punch > 0.0 && punch_win > 0.0 && t < punch_win {
        v *= 1.0 + punch * (1.0 - t / punch_win);
    }
    v
}

/// Classic Freeverb tunings (samples at the 44.1 kHz reference) and damping —
/// shared verbatim by the offline and streaming reverbs so they cannot drift.
pub(crate) const FREEVERB_COMB_TUNINGS: [usize; 6] = [1116, 1188, 1277, 1356, 1422, 1491];
pub(crate) const FREEVERB_ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];
pub(crate) const FREEVERB_DAMP: f32 = 0.2;

/// Modulated-delay effect tunings shared by both renderers.
pub(crate) const CHORUS_BASE_SECS: f32 = 0.015;
pub(crate) const CHORUS_SWING_SECS: f32 = 0.010;
pub(crate) const FLANGER_BASE_SECS: f32 = 0.0025;
pub(crate) const FLANGER_SWING_SECS: f32 = 0.002;

/// Linear amplitude → dBFS (floored at −180 dB so silence stays finite).
pub fn dbfs(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// dB → linear gain.
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Output sample-peak ceiling (≈ −0.1 dBFS).
pub const CEIL: f32 = 0.989;

/// Attenuate (never boost) all channels by one shared gain so the joint peak
/// never exceeds [`CEIL`]. Leaving quiet sounds quiet keeps the analyzer's
/// level readings meaningful; the shared gain keeps stereo images intact.
pub fn peak_limit(channels: &mut [&mut [f32]]) {
    // Sanitize first: an unvalidated document can put NaN/inf into the graph,
    // and the f32::max fold discards NaN — the buffer would measure peak 0,
    // skip limiting, and NaN the encoded file. Scrub non-finite to silence.
    let mut peak = 0.0f32;
    for c in channels.iter_mut() {
        for x in c.iter_mut() {
            if !x.is_finite() {
                *x = 0.0;
            }
            peak = peak.max(x.abs());
        }
    }
    if peak > CEIL {
        let g = CEIL / peak;
        for c in channels.iter_mut() {
            for x in c.iter_mut() {
                *x *= g;
            }
        }
    }
}

/// LEGACY (engine ≤ 3) inter-sample peak estimate by 4× *linear* interpolation.
/// Linear interpolation is bounded by the adjacent samples, so this can never
/// exceed the sample peak — it under-reads true peaks by up to ~3 dB. Kept
/// bit-exact because the engine ≤ 3 normalize output stage limited against it;
/// everything else should use [`true_peak_oversampled`].
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

// The three intermediate phases of a 4× polyphase windowed-sinc interpolator
// (12 taps each, Hann window, per-phase unity DC gain). Precomputed constants
// so the estimate is bit-identical across platforms (no libm at runtime).
const TP_PHASES: [[f32; 12]; 3] = [
    [
        -0.001_630_944,
        0.010_354_987,
        -0.030_093_31,
        0.069_125_3,
        -0.161_381_1,
        0.896_035_25,
        0.288_544_92,
        -0.103_407_112,
        0.046_242_866,
        -0.018_517_122,
        0.004_893_635,
        -0.000_167_362,
    ],
    [
        -0.000_985_122,
        0.010_349_617,
        -0.033_673_15,
        0.080_066_491,
        -0.180_965_97,
        0.625_208_14,
        0.625_208_14,
        -0.180_965_97,
        0.080_066_491,
        -0.033_673_15,
        0.010_349_617,
        -0.000_985_122,
    ],
    [
        -0.000_167_362,
        0.004_893_635,
        -0.018_517_122,
        0.046_242_866,
        -0.103_407_112,
        0.288_544_92,
        0.896_035_25,
        -0.161_381_1,
        0.069_125_3,
        -0.030_093_31,
        0.010_354_987,
        -0.001_630_944,
    ],
];

/// Estimate the inter-sample (true) peak by 4× polyphase windowed-sinc
/// oversampling (BS.1770-style). Returns linear amplitude (use [`dbfs`] for
/// dBTP). Unlike linear interpolation, this genuinely reconstructs peaks that
/// land between samples — a sine sampled at its zero crossings still reads ~1.
pub fn true_peak_oversampled(samples: &[f32]) -> f32 {
    let mut peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let at = |i: isize| -> f32 {
        if i < 0 || i as usize >= samples.len() {
            0.0
        } else {
            samples[i as usize]
        }
    };
    for i in 0..samples.len() as isize {
        for phase in &TP_PHASES {
            let mut v = 0.0f32;
            for (t, c) in phase.iter().enumerate() {
                v += c * at(i + t as isize - 5);
            }
            peak = peak.max(v.abs());
        }
    }
    peak
}

/// LEGACY (engine ≤ 3) K-weighted integrated loudness: ungated, mono, and
/// pinned to the standard 48 kHz coefficient table at every sample rate. Kept
/// bit-exact because the engine ≤ 3 normalize output stage gain-matched
/// against it; everything else should use [`loudness_lufs_gated`]. Returns
/// −120 for silence.
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

/// The BS.1770 K-weighting biquad coefficients for `sr`, derived from the
/// analog prototype (a +4 dB spherical-head high shelf at ~1.68 kHz and the
/// RLB rumble high-pass at ~38 Hz) via the bilinear transform. At 48 kHz this
/// reproduces the standard's coefficient table.
/// Returns `(shelf_b, shelf_a, highpass_b, highpass_a)`.
fn k_weighting_coeffs(sr: u32) -> ([f32; 3], [f32; 2], [f32; 3], [f32; 2]) {
    let fs = sr as f64;
    let (f0, gain_db, q) = (
        1_681.974_450_955_533,
        3.999_843_853_973_347,
        0.707_175_236_955_419_6,
    );
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let d = 1.0 + k / q + k * k;
    let shelf_b = [
        ((vh + vb * k / q + k * k) / d) as f32,
        ((2.0 * (k * k - vh)) / d) as f32,
        ((vh - vb * k / q + k * k) / d) as f32,
    ];
    let shelf_a = [
        ((2.0 * (k * k - 1.0)) / d) as f32,
        ((1.0 - k / q + k * k) / d) as f32,
    ];
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let d = 1.0 + k / q + k * k;
    let hp_b = [1.0, -2.0, 1.0];
    let hp_a = [
        ((2.0 * (k * k - 1.0)) / d) as f32,
        ((1.0 - k / q + k * k) / d) as f32,
    ];
    (shelf_b, shelf_a, hp_b, hp_a)
}

/// K-weight one channel at its actual sample rate.
fn k_weight(samples: &[f32], sr: u32) -> Vec<f32> {
    let (sb, sa, hb, ha) = k_weighting_coeffs(sr);
    biquad_df1(&biquad_df1(samples, sb, sa), hb, ha)
}

/// ITU-R BS.1770-4 gated integrated loudness over one or more channels (pass
/// `[mono]` or `[left, right]`): K-weighting at the actual sample rate,
/// 400 ms blocks at 75% overlap, the −70 LUFS absolute gate, then the −10 LU
/// relative gate; channel energies sum per the spec. Accumulates in f64, so
/// long renders don't stall an f32 accumulator. Returns −120 for silence.
pub fn loudness_lufs_gated(channels: &[&[f32]], sr: u32) -> f32 {
    // Gate over the shortest channel so mismatched lengths can't panic the
    // block slicing (in-repo callers pass equal lengths; the fn is pub).
    let n = channels.iter().map(|c| c.len()).min().unwrap_or(0);
    if n == 0 {
        return -120.0;
    }
    let weighted: Vec<Vec<f32>> = channels.iter().map(|c| k_weight(c, sr)).collect();
    let sum_ms = |range: std::ops::Range<usize>| -> f64 {
        weighted
            .iter()
            .map(|w| {
                w[range.clone()]
                    .iter()
                    .map(|x| *x as f64 * *x as f64)
                    .sum::<f64>()
                    / range.len() as f64
            })
            .sum()
    };
    let lufs = |ms: f64| -0.691 + 10.0 * ms.max(1e-12).log10();
    let block = (sr as usize * 2) / 5; // 400 ms
    if n < block || block == 0 {
        // Too short to gate: integrate over the whole signal.
        return lufs(sum_ms(0..n)) as f32;
    }
    let hop = (block / 4).max(1); // 75% overlap
    let blocks: Vec<f64> = (0..=(n - block))
        .step_by(hop)
        .map(|s| sum_ms(s..s + block))
        .collect();
    // Absolute gate at −70 LUFS.
    let above: Vec<f64> = blocks.into_iter().filter(|&ms| lufs(ms) > -70.0).collect();
    if above.is_empty() {
        return -120.0;
    }
    // Relative gate 10 LU below the mean of the absolute-gated blocks.
    let rel = lufs(above.iter().sum::<f64>() / above.len() as f64) - 10.0;
    let gated: Vec<f64> = above.into_iter().filter(|&ms| lufs(ms) > rel).collect();
    if gated.is_empty() {
        return -120.0;
    }
    lufs(gated.iter().sum::<f64>() / gated.len() as f64) as f32
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

/// MIDI note number (fractional) for a frequency in Hz — A4 = 440 = 69.
/// The inverse of the wire encoding: seq pitches travel as Hz, and the drum
/// kit / exporters recover the MIDI number from the onset frequency.
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// −ln(1000): decay-rate constant so an exponential ring reaches −60 dB
/// (×0.001) after its nominal decay time. Shared by the modal resonators
/// (offline + streaming twins) and the pluck body bank.
pub(crate) const NEG_LN_1000: f32 = -6.907_755;

/// The LTI coefficients `(a1, a2, b0)` of one modal resonator (a two-pole
/// damped sine): the pole radius places the ring at exactly −60 dB after
/// `decay` seconds, and `b0` normalises the impulse-response peak to `gain`,
/// so a mode's loudness is its gain regardless of ring time. One definition
/// for the offline and streaming modal banks.
pub(crate) fn modal_coeffs(freq: f32, decay: f32, gain: f32, sr: u32) -> (f32, f32, f32) {
    let srf = sr as f32;
    let nyq = srf * 0.5;
    let f0 = freq.clamp(1.0, (nyq - 1.0).max(1.0));
    let decay = decay.max(1e-3);
    let w0 = std::f32::consts::TAU * f0 / srf;
    let (sin0, cos0) = (w0.sin(), w0.cos());
    // r so the ring reaches −60 dB (×0.001) after `decay` seconds.
    let r = (NEG_LN_1000 / (decay * srf)).exp();
    (2.0 * r * cos0, -r * r, gain * sin0)
}

/// The delay-line allocation for a `delay` node, in samples: validate() caps
/// `secs` at 30 s, and the clamp guards unvalidated docs identically on the
/// offline and streaming paths.
pub(crate) fn delay_line_len(secs: f32, sr: u32) -> usize {
    ((secs.min(30.0) * sr as f32) as usize).max(1)
}

/// The quantization levels of a bitcrush: `1 << bits`, with the shift clamped
/// so an unvalidated `bits >= 32` can't overflow on either path.
pub(crate) fn bitcrush_levels(bits: u8) -> f32 {
    (1u32 << (bits as u32).min(31)) as f32
}

/// The Freeverb delay-line lengths in samples at `sr` — `(comb, allpass)`,
/// with `spread` extra samples per line for stereo tail decorrelation (0 for
/// mono). One layout for the offline and streaming reverb builds.
pub(crate) fn freeverb_lengths(sr: u32, spread: usize) -> (Vec<usize>, Vec<usize>) {
    let scale = sr as f32 / 44_100.0;
    let combs = FREEVERB_COMB_TUNINGS
        .iter()
        .map(|t| (((t + spread) as f32 * scale) as usize).max(1))
        .collect();
    let allpasses = FREEVERB_ALLPASS_TUNINGS
        .iter()
        .map(|t| (((t + spread) as f32 * scale) as usize).max(1))
        .collect();
    (combs, allpasses)
}

/// The Freeverb room-size → feedback-coefficient mapping (shared by the
/// offline and streaming reverb builds).
pub(crate) fn freeverb_feedback(room: f32) -> f32 {
    0.7 + 0.28 * room.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_reference_vectors() {
        // Standard SplitMix64 test vectors for seed 0 — pins byte-exactness of
        // every noise render across releases.
        let mut rng = Rng::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn unit_and_bi_stay_in_range() {
        let mut rng = Rng::new(42);
        for _ in 0..1000 {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
        }
        let mut rng = Rng::new(42);
        for _ in 0..1000 {
            let b = rng.bi();
            assert!((-1.0..1.0).contains(&b));
        }
    }

    #[test]
    fn db_conversions_roundtrip() {
        assert_eq!(dbfs(1.0), 0.0);
        assert!((dbfs(0.5) + 6.0206).abs() < 0.001);
        assert!((db_to_lin(-6.0206) - 0.5).abs() < 0.001);
        assert_eq!(dbfs(0.0), -180.0); // silence floor
    }

    #[test]
    fn k_weighting_at_48k_matches_the_standard_table() {
        let (sb, sa, hb, ha) = k_weighting_coeffs(48_000);
        let expect = |got: f32, want: f32| {
            assert!((got - want).abs() < 1e-4, "got {got}, want {want}");
        };
        expect(sb[0], 1.535_124_9);
        expect(sb[1], -2.691_696_2);
        expect(sb[2], 1.198_392_8);
        expect(sa[0], -1.690_659_3);
        expect(sa[1], 0.732_480_8);
        assert_eq!(hb, [1.0, -2.0, 1.0]);
        expect(ha[0], -1.990_047_5);
        expect(ha[1], 0.990_072_3);
    }

    #[test]
    fn oversampled_true_peak_sees_between_the_samples() {
        // A sine at fs/4 with phase π/4 samples at ±0.7071 while its real
        // peak is 1.0 — the classic inter-sample-over test. The legacy linear
        // estimate is mathematically bounded by the sample peak.
        let x: Vec<f32> = (0..1024)
            .map(|i| (std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4).sin())
            .collect();
        let sample_peak = x.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!((sample_peak - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        assert!(true_peak(&x) <= sample_peak + 1e-6, "legacy: bounded");
        let tp = true_peak_oversampled(&x);
        assert!(
            (0.98..=1.05).contains(&tp),
            "oversampled estimate {tp} should recover the hidden 1.0 peak"
        );
    }

    #[test]
    fn gated_loudness_ignores_silence_and_sums_channels() {
        let sr = 48_000u32;
        let tau = std::f32::consts::TAU;
        let tone: Vec<f32> = (0..4 * sr as usize)
            .map(|i| 0.25 * (tau * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let solo = loudness_lufs_gated(&[&tone], sr);
        // Pad with 8 s of silence: gating must hold the reading near-steady
        // (only the tone/silence boundary blocks dilute it slightly) while an
        // ungated integration drops by ~5 dB.
        let mut padded = tone.clone();
        padded.extend(std::iter::repeat_n(0.0f32, 8 * sr as usize));
        let gated = loudness_lufs_gated(&[&padded], sr);
        assert!(
            (gated - solo).abs() < 0.35,
            "gated {gated} vs solo {solo}: silence must not drag the reading"
        );
        assert!(
            loudness_lufs(&padded) < solo - 4.0,
            "the ungated legacy reading is dragged down by the padding"
        );
        // Stereo: the same program in both channels reads +3 LU (energy sum).
        let stereo = loudness_lufs_gated(&[&tone, &tone], sr);
        assert!(
            (stereo - solo - 3.01).abs() < 0.1,
            "stereo {stereo} vs mono {solo}: channels sum per BS.1770"
        );
        assert_eq!(loudness_lufs_gated(&[], sr), -120.0);
    }

    #[test]
    fn peak_limit_attenuates_jointly_never_boosts() {
        let mut l = vec![2.0f32, 0.5];
        let mut r = vec![1.0f32, 0.25];
        peak_limit(&mut [&mut l, &mut r]);
        assert!((l[0] - CEIL).abs() < 1e-6); // loudest sample hits the ceiling
        assert!((r[0] - CEIL / 2.0).abs() < 1e-6); // image preserved
        let mut quiet = vec![0.1f32, -0.2];
        peak_limit(&mut [&mut quiet]);
        assert_eq!(quiet, vec![0.1, -0.2]); // untouched
    }
}
