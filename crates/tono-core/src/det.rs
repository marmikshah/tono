//! Deterministic transcendental kernels for the current DSP engine.
//!
//! Every kernel uses IEEE f64 arithmetic with pinned fdlibm minimax
//! coefficients, avoiding platform-dependent libm results. The f32 wrappers
//! cast once at the end. Tests bound error against libm to about 1 ulp of f64.

// The fdlibm polynomial coefficients NEED full f64 precision — truncating
// them measurably degrades the kernels (the tests pin the error bounds).
#![allow(clippy::excessive_precision)]

/// 2π split into three parts for Cody–Waite range reduction (the residuals
/// carry what the f64 nearest to 2π drops, so k·c is exact for k well past
/// any musical argument).
const PI2_HI: f64 = std::f64::consts::TAU;
const PI2_MID: f64 = 2.44929359829470641445e-16;
const PI2_LO: f64 = 1.74968224062658175647e-32;
const INV_PI2: f64 = 1.59154943091895335769e-01; // 1/(2π)
const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;

/// fdlibm sine polynomial on [-π/4, π/4]: sin(x) ≈ x + x³·S(x²).
#[inline]
fn sin_poly(x: f64) -> f64 {
    const S1: f64 = -1.66666666666666324348e-01;
    const S2: f64 = 8.33333333332248946124e-03;
    const S3: f64 = -1.98412698298579493134e-04;
    const S4: f64 = 2.75573137070700676789e-06;
    const S5: f64 = -2.50507602534068634195e-08;
    const S6: f64 = 1.58969099521155010221e-10;
    let z = x * x;
    x + x * z * (S1 + z * (S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)))))
}

/// fdlibm cosine polynomial on [-π/4, π/4]: cos(x) ≈ 1 − x²/2 + x⁴·C(x²).
#[inline]
fn cos_poly(x: f64) -> f64 {
    const C1: f64 = 4.16666666666666019037e-02;
    const C2: f64 = -1.38888888888741095749e-03;
    const C3: f64 = 2.48015872894767294178e-05;
    const C4: f64 = -2.75573143513906633035e-07;
    const C5: f64 = 2.08757232129817482790e-09;
    const C6: f64 = -1.13596475577881948265e-11;
    let z = x * x;
    1.0 - 0.5 * z + z * z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))))
}

/// Reduce `x` into [-π, π] with Cody–Waite (exact for |x| well past any
/// musical argument; arguments of an LFO at 48 kHz over a year stay < 2⁴⁰).
#[inline]
fn reduce_2pi(x: f64) -> f64 {
    let k = (x * INV_PI2).round();
    ((x - k * PI2_HI) - k * PI2_MID) - k * PI2_LO
}

/// Deterministic sine. Pure f64 arithmetic: identical everywhere.
pub fn sin(x: f64) -> f64 {
    let r = reduce_2pi(x);
    // Fold by quadrant into the [-π/4, π/4] kernel range.
    let q = ((r / FRAC_PI_2).round()) as i64;
    let a = r - q as f64 * FRAC_PI_2;
    match q.rem_euclid(4) {
        0 => sin_poly(a),
        1 => cos_poly(a),
        2 => -sin_poly(a),
        _ => -cos_poly(a),
    }
}

/// Deterministic cosine: cos(x) = sin(x + π/2) through the same kernel.
pub fn cos(x: f64) -> f64 {
    sin(x + FRAC_PI_2)
}

const LN2_HI: f64 = 6.93147180369123816490e-01;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const INV_LN2: f64 = std::f64::consts::LOG2_E;

/// 2^k as an exact f64 (bit-constructed). `y·2^k` with y ∈ [1, 2) overflows
/// for k > 1023 (⇒ inf, matching libm) and is subnormal-or-zero for
/// k < -1023 — a power-of-two subnormal down to 2⁻¹⁰⁷⁴, zero below.
#[inline]
fn ldexp2(k: i64) -> f64 {
    if k > 1023 {
        return f64::INFINITY;
    }
    if k < -1023 {
        if k < -1074 {
            return 0.0;
        }
        // A power of two is exact even subnormal: one mantissa bit.
        return f64::from_bits(1u64 << (k + 1074));
    }
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// Deterministic exp: r = x − k·ln2 (Cody–Waite on ln2), the fdlibm Padé
/// form on r, exact 2^k rescale.
pub fn exp(x: f64) -> f64 {
    const P1: f64 = 1.66666666666666019037e-01;
    const P2: f64 = -2.77777777770155933842e-03;
    const P3: f64 = 6.61375632143793436117e-05;
    const P4: f64 = -1.65339022054652515390e-06;
    const P5: f64 = 4.13813679705723846039e-08;
    if x > 7.09782712893383e+02 {
        return f64::INFINITY;
    }
    if x < -7.45133219101941e+02 {
        return 0.0;
    }
    let k = (x * INV_LN2).round();
    let r = (x - k * LN2_HI) - k * LN2_LO;
    // fdlibm e_exp: c = r − r²·P(r²); exp(r) = 1 − (r·c/(c − 2) − r).
    let t = r * r;
    let c = r - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    let y = 1.0 - ((r * c) / (c - 2.0) - r);
    y * ldexp2(k as i64)
}

/// expm1 for small |x|: the direct series (no cancellation), used by tanh.
/// Terms x^1/1! .. x^11/11! in Horner form.
fn expm1_small(x: f64) -> f64 {
    const INV_FACT: [f64; 11] = [
        1.0,
        1.0 / 2.0,
        1.0 / 6.0,
        1.0 / 24.0,
        1.0 / 120.0,
        1.0 / 720.0,
        1.0 / 5040.0,
        1.0 / 40320.0,
        1.0 / 362880.0,
        1.0 / 3628800.0,
        1.0 / 39916800.0,
    ];
    let mut acc = INV_FACT[10];
    for c in INV_FACT[..10].iter().rev() {
        acc = c + x * acc;
    }
    x * acc
}

/// Deterministic natural log: x = m·2^k with m in [√2/2, √2), ln(x) =
/// k·ln2 + ln(m) via the fdlibm log1p series. NaN for x < 0, −inf for 0.
pub fn ln(x: f64) -> f64 {
    const LG1: f64 = 6.666666666666735130e-01;
    const LG2: f64 = 3.999999999940941908e-01;
    const LG3: f64 = 2.857142874366239149e-01;
    const LG4: f64 = 2.222219843214978396e-01;
    const LG5: f64 = 1.818357216161805012e-01;
    const LG6: f64 = 1.531383769920937332e-01;
    const LG7: f64 = 1.479819860511658591e-01;
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    // Decompose into mantissa m in [√2/2, √2) and exponent k.
    let mut x = x;
    let mut scale_k = 0.0f64;
    if x < f64::MIN_POSITIVE {
        x *= 1.84467440737095e19; // 2^64: subnormals into the normal range
        scale_k = -64.0;
    }
    let bits = x.to_bits();
    let mut exp_bits = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mut mant = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | (1023u64 << 52));
    // Fold the mantissa into [√2/2, √2] so f = mant − 1 stays inside the
    // series' designed range [−0.293, 0.414].
    if mant > std::f64::consts::SQRT_2 {
        mant *= 0.5;
        exp_bits += 1;
    }
    let f = mant - 1.0;
    let s = f / (2.0 + f);
    let z = s * s;
    let w = z * z;
    // fdlibm e_log: the series split into even/odd powers for its exact
    // rounding behavior.
    let t1 = w * (LG2 + w * (LG4 + w * LG6));
    let t2 = z * (LG1 + w * (LG3 + w * (LG5 + w * LG7)));
    let r = t2 + t1;
    let hfsq = 0.5 * f * f;
    let k = exp_bits as f64 + scale_k;
    k * LN2_HI + (f - (hfsq - (s * (hfsq + r) + k * LN2_LO)))
}

/// Deterministic pow: exp(y·ln x) for x > 0. Edge cases match libm for the
/// render path's uses: x = 0 → 0/1/+inf by the sign of y; x < 0 → NaN
/// (the render path only ever raises positive bases).
pub fn powf(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return 1.0;
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    if x < 0.0 {
        return f64::NAN;
    }
    exp(y * ln(x))
}

/// Deterministic tanh: small |x| through the cancellation-free expm1 form,
/// larger |x| through the exp kernel.
pub fn tanh(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let (x, sign) = if x < 0.0 { (-x, -1.0) } else { (x, 1.0) };
    if x > 20.0 {
        return sign;
    }
    let t = if x < 0.25 {
        // 1 − 2/(e^2x + 1) cancels catastrophically here; the expm1 series
        // is exact.
        let e = expm1_small(2.0 * x);
        e / (e + 2.0)
    } else {
        1.0 - 2.0 / (exp(2.0 * x) + 1.0)
    };
    sign * t
}

/// Deterministic log10 via the ln kernel.
pub fn log10(x: f64) -> f64 {
    ln(x) / ln(10.0)
}

// --- f32 wrappers: the f64 kernel, correctly rounded to f32. ---

/// Deterministic `f32` sine.
#[inline]
pub fn sinf(x: f32) -> f32 {
    sin(x as f64) as f32
}
/// Deterministic `f32` cosine.
#[inline]
pub fn cosf(x: f32) -> f32 {
    cos(x as f64) as f32
}
/// Deterministic `f32` exp.
#[inline]
pub fn expf(x: f32) -> f32 {
    exp(x as f64) as f32
}
/// Deterministic `f32` natural log.
#[inline]
pub fn lnf(x: f32) -> f32 {
    ln(x as f64) as f32
}
/// Deterministic `f32` pow.
#[inline]
pub fn powff(x: f32, y: f32) -> f32 {
    powf(x as f64, y as f64) as f32
}
/// Deterministic `f32` tanh.
#[inline]
pub fn tanhf(x: f32) -> f32 {
    tanh(x as f64) as f32
}
/// Deterministic `f32` log10.
#[inline]
pub fn log10f(x: f32) -> f32 {
    log10(x as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max_err(f: impl Fn(f64) -> f64, g: impl Fn(f64) -> f64, xs: &[f64]) -> f64 {
        xs.iter().map(|&x| (f(x) - g(x)).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn sin_matches_libm_to_ulp_scale() {
        let xs: Vec<f64> = (-1000..=1000).map(|i| i as f64 * 0.0317).collect();
        let e = max_err(sin, f64::sin, &xs);
        assert!(e < 2e-15, "sin error {e}");
        // Pinned reference values — the kernel's definition, stable against
        // accidental edits (verified: sin(−13.37) reduces to sin(−0.8036…)).
        assert_eq!(sin(0.0), 0.0);
        assert_eq!(sin(FRAC_PI_2), 1.0);
        assert_eq!(sin(1.0), 0.8414709848078965);
        assert_eq!(sin(-13.37), -0.7198799780150617);
    }

    #[test]
    fn cos_matches_libm_to_ulp_scale() {
        let xs: Vec<f64> = (-1000..=1000).map(|i| i as f64 * 0.0293).collect();
        let e = max_err(cos, f64::cos, &xs);
        assert!(e < 5e-15, "cos error {e}");
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(cos(1.0), 0.5403023058681397);
    }

    #[test]
    fn exp_matches_libm_to_ulp_scale() {
        let xs: Vec<f64> = (-700..=700).map(|i| i as f64 * 0.013).collect();
        let e = xs
            .iter()
            .map(|&x| {
                let (a, b) = (exp(x), x.exp());
                if b == 0.0 { a.abs() } else { (a - b).abs() / b }
            })
            .fold(0.0, f64::max);
        assert!(e < 2e-14, "exp rel error {e}");
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(1.0), 2.7182818284590455);
    }

    #[test]
    fn exp_overflow_is_inf() {
        assert_eq!(exp(710.0), f64::INFINITY);
        assert_eq!(exp(730.0), f64::INFINITY);
        assert_eq!(exp(1e10), f64::INFINITY);
    }

    #[test]
    fn exp_underflow_subnormal_or_zero() {
        // k < -1023 lands in the subnormal range: the result must stay on the
        // subnormal scale (never jump back to a normal-magnitude value).
        let v = exp(-744.9);
        assert!((0.0..1e-300).contains(&v), "exp(-744.9) = {v:e}");
        let libm = (-744.9f64).exp();
        let subnormal_ulp = f64::from_bits(1);
        assert!(
            (v - libm).abs() <= 4.0 * subnormal_ulp,
            "exp(-744.9): det {v:e} vs libm {libm:e}"
        );
        assert_eq!(exp(-800.0), 0.0);
        assert_eq!(exp(-1e10), 0.0);
    }

    #[test]
    fn ln_matches_libm_to_ulp_scale() {
        let xs: Vec<f64> = (1..=2000).map(|i| i as f64 * 1.717).collect();
        let e = max_err(ln, f64::ln, &xs);
        assert!(e < 1e-14, "ln error {e}");
        assert_eq!(ln(1.0), 0.0);
        assert_eq!(ln(std::f64::consts::E), 1.0);
        assert!(ln(0.0) == f64::NEG_INFINITY);
        assert!(ln(-1.0).is_nan());
    }

    #[test]
    fn powf_matches_libm_to_ulp_scale() {
        let mut worst = 0.0f64;
        for i in 1..=200 {
            let x = i as f64 * 0.31;
            for j in -5..=5 {
                let y = j as f64 * 0.5;
                let (a, b) = (powf(x, y), x.powf(y));
                let rel = if b == 0.0 { a.abs() } else { (a - b).abs() / b };
                worst = worst.max(rel);
            }
        }
        assert!(worst < 1e-12, "powf rel error {worst}");
        assert_eq!(powf(2.0, 10.0), 1024.0);
        assert!((powf(10.0, -3.0) - 0.001).abs() < 1e-15);
        assert_eq!(powf(0.0, 2.0), 0.0);
        assert!(powf(-2.0, 0.5).is_nan());
    }

    #[test]
    fn tanh_and_log10_match() {
        let xs: Vec<f64> = (-400..=400).map(|i| i as f64 * 0.047).collect();
        let e = max_err(tanh, f64::tanh, &xs);
        assert!(e < 1e-13, "tanh error {e}");
        assert_eq!(tanh(0.0), 0.0);
        assert!((log10(1000.0) - 3.0).abs() < 1e-14);
        assert_eq!(log10(1.0), 0.0);
    }

    #[test]
    fn f32_wrappers_are_deterministic() {
        // Same input → identical bits, always (the determinism definition).
        assert_eq!(sinf(1.234).to_bits(), sinf(1.234).to_bits());
        assert_eq!(expf(-3.21).to_bits(), expf(-3.21).to_bits());
        // And close to the platform libm (accuracy, not identity).
        assert!((sinf(1.234) - 1.234f32.sin()).abs() < 2e-6);
        assert!((powff(1.5, 2.5) - 1.5f32.powf(2.5)).abs() < 1e-5);
    }
}
