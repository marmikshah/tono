//! Small shared DSP core: the deterministic PRNG, dB conversions, and the
//! output peak limit. One copy of each — these protect the project's
//! determinism contract (same graph + seed ⇒ identical bytes), so they must
//! never fork per module.

/// Deterministic PRNG (SplitMix64). Seeded from a sound's seed so the same
/// graph + seed renders identical audio every time. `Clone` lets a stereo
/// master bus run the same processor on both channels with identical draws.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
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
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

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
    let peak = channels
        .iter()
        .flat_map(|c| c.iter())
        .fold(0.0f32, |m, &x| m.max(x.abs()));
    if peak > CEIL {
        let g = CEIL / peak;
        for c in channels.iter_mut() {
            for x in c.iter_mut() {
                *x *= g;
            }
        }
    }
}

/// Estimate the inter-sample (true) peak by 4× linear-interpolation oversampling.
/// Returns linear amplitude (use [`dbfs`] for dBTP). Exposed so the renderer's
/// output stage can limit to a true-peak ceiling.
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
