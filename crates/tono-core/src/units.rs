//! Typed units and exact musical time for the composition/compile layer
//!
//! The composition model speaks in exact [`Beat`] rationals; audio speaks in
//! integer [`Frames`]. The two meet exactly once, at the scheduling boundary,
//! through [`beat_to_frames`] — every placement lands on the same frame on
//! every platform because the rounding rule is specified, not emergent. The
//! [`SampleRate`] and [`Tempo`] make the conversion units explicit.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A count of audio frames (one sample per channel) — the engine's unit of
/// position and length on the audio timeline.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Frames(pub u64);

/// A sample rate in Hz (frames per second), e.g. 44100 or 48000.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SampleRate(pub u32);

/// A tempo in beats per minute. Below 1 BPM the conversion to frames floors
/// the tempo at 1 — the same clamp the song compiler applies
/// (`Song::to_doc`), so a degenerate tempo can't produce absurd frame counts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Tempo(pub f32);

impl From<u64> for Frames {
    fn from(n: u64) -> Self {
        Frames(n)
    }
}

impl std::ops::Add for Frames {
    type Output = Frames;
    fn add(self, rhs: Frames) -> Frames {
        Frames(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Frames {
    type Output = Frames;
    fn sub(self, rhs: Frames) -> Frames {
        Frames(self.0 - rhs.0)
    }
}

/// An exact musical position or duration as a rational number of beats:
/// `num / den`, always normalized (denominator positive, gcd-reduced, zero
/// canonicalized to `0/1`). Tuplets and repeated transforms (stretch, rotate,
/// concatenate) stay exact — no floating-point drift ever accumulates before
/// the frame boundary.
///
/// `Beat::new(2, 4)` IS `Beat::new(1, 2)`. A zero `den` is floored to 1 (the
/// same degenerate-value clamp the song grid applies), so deserialization can
/// never produce a division by zero. Serde is the flat struct `{"num":..,"den":..}`;
/// deserializing normalizes through [`Beat::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
pub struct Beat {
    /// The numerator (beats × `den`), carries the sign.
    pub num: i64,
    /// The denominator, always > 0 after normalization.
    pub den: u32,
}

impl Beat {
    /// A normalized beat: gcd-reduced, `den > 0`, zero as `0/1`. A `den` of 0
    /// is floored to 1.
    pub const fn new(num: i64, den: u32) -> Beat {
        let den = if den == 0 { 1 } else { den };
        if num == 0 {
            return Beat { num: 0, den: 1 };
        }
        // Euclid on |num| and den; the gcd fits i64 because it divides den
        // (< 2^32), so `num / g` can never overflow — not even i64::MIN.
        let mut a = num.unsigned_abs();
        let mut b = den as u64;
        while b != 0 {
            let t = a % b;
            a = b;
            b = t;
        }
        let g = a;
        Beat {
            num: num / g as i64,
            den: (den as u64 / g) as u32,
        }
    }

    /// Zero beats — the origin of the musical timeline.
    pub const fn zero() -> Beat {
        Beat { num: 0, den: 1 }
    }

    /// A whole number of beats (`n/1`).
    pub const fn from_int(n: i64) -> Beat {
        Beat { num: n, den: 1 }
    }

    /// Exact sum, erroring when the (unreduced) result doesn't fit a `Beat`.
    pub fn checked_add(self, other: Beat) -> Result<Beat, BeatError> {
        let num = self.num as i128 * other.den as i128 + other.num as i128 * self.den as i128;
        let den = self.den as i128 * other.den as i128;
        checked(num, den)
    }

    /// Exact difference, erroring when the (unreduced) result doesn't fit a
    /// `Beat`.
    pub fn checked_sub(self, other: Beat) -> Result<Beat, BeatError> {
        let num = self.num as i128 * other.den as i128 - other.num as i128 * self.den as i128;
        let den = self.den as i128 * other.den as i128;
        checked(num, den)
    }

    /// Exact scaling by the rational factor `num / den`, erroring when the
    /// (unreduced) result doesn't fit a `Beat`.
    pub fn mul_rational(self, num: i64, den: u32) -> Result<Beat, BeatError> {
        checked(
            self.num as i128 * num as i128,
            self.den as i128 * den as i128,
        )
    }

    /// Exact scaling by a whole number (e.g. a 4-beat phrase repeated 3 times
    /// is `phrase.scale(3)`).
    pub fn scale(self, factor: i64) -> Result<Beat, BeatError> {
        self.mul_rational(factor, 1)
    }

    /// The floating-point value, for the single crossing to frames at the
    /// scheduling boundary (and nowhere else — composition math stays exact).
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// Normalize an exact i128 intermediate back into a `Beat`, or report overflow.
fn checked(num: i128, den: i128) -> Result<Beat, BeatError> {
    let num = i64::try_from(num).map_err(|_| BeatError::Overflow)?;
    let den = u32::try_from(den).map_err(|_| BeatError::Overflow)?;
    Ok(Beat::new(num, den))
}

impl From<(i64, u32)> for Beat {
    fn from((num, den): (i64, u32)) -> Beat {
        Beat::new(num, den)
    }
}

impl Ord for Beat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Cross-multiply in i128: num/den vs other.num/other.den. Both
        // denominators are positive, so the products compare the same way;
        // i128 keeps even i64::MAX × u32::MAX from overflowing.
        (self.num as i128 * other.den as i128).cmp(&(other.num as i128 * self.den as i128))
    }
}

impl PartialOrd for Beat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Beat {
    /// `3/2`, or the bare integer when the denominator is 1 (`4`, not `4/1`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Deserializing routes through [`Beat::new`] so the normalization invariant
/// holds no matter where the value came from.
impl<'de> Deserialize<'de> for Beat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            num: i64,
            den: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Beat::new(raw.num, raw.den))
    }
}

/// Why exact beat arithmetic failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatError {
    /// The exact result doesn't fit in a `Beat` (i64 numerator, u32
    /// denominator) — a pathological value, not a musical one.
    Overflow,
}

impl std::fmt::Display for BeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BeatError::Overflow => f.write_str("beat arithmetic overflow"),
        }
    }
}

impl std::error::Error for BeatError {}

/// Convert an exact beat position to an audio frame — the ONE place musical
/// time crosses to audio time. All composition math before this
/// boundary stays rational, so the conversion never compounds rounding error.
///
/// The rule is specified exactly: `seconds = beats × 60 / bpm` in `f64`, then
/// `frames = round(seconds × rate)`, where `f64::round` rounds halves AWAY
/// FROM ZERO (a `.5` frame rounds up for positive beats). Every placement
/// therefore lands on the same frame on every platform and in every process.
///
/// Degenerate inputs clamp rather than blow up: a tempo below 1 BPM is
/// floored at 1 (the same clamp `Song::to_doc` applies), and a negative beat
/// — a position before the song's start — clamps to frame 0, since `Frames`
/// can't be negative.
pub fn beat_to_frames(beat: Beat, tempo: Tempo, rate: SampleRate) -> Frames {
    let bpm = (tempo.0 as f64).max(1.0);
    let seconds = beat.to_f64() * 60.0 / bpm;
    let frames = seconds * rate.0 as f64;
    Frames(frames.round().max(0.0) as u64)
}

/// A time-signature change at a bar (0-based), for the meter map: from `bar`
/// until the next change, a bar is `numerator`/`denominator` long. In the
/// song's beat grid a bar is `numerator × (4 / denominator)` quarter-note
/// beats (6/8 = 3, 3/4 = 3, 4/4 = 4). Denominators must be powers of two.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MeterPoint {
    /// The bar the change takes effect at (0-based). The first point must be
    /// bar 0 when a meter map is present.
    pub bar: u32,
    /// Beats per bar (the time-signature numerator).
    pub numerator: u32,
    /// The note value one beat is written in (4 = quarter, 8 = eighth).
    pub denominator: u32,
}

/// The meter in effect at `bar` under a meter map (the last point at or
/// before it; `default_numerator`/4 when the map is empty or starts later).
pub fn meter_at(map: &[MeterPoint], default_numerator: u32, bar: u32) -> MeterPoint {
    let fallback = MeterPoint {
        bar: 0,
        numerator: default_numerator.max(1),
        denominator: 4,
    };
    map.iter()
        .rev()
        .find(|p| p.bar <= bar)
        .copied()
        .unwrap_or(fallback)
}

/// The length of bar `index` in quarter-note beats: `pickup` for bar 0 when
/// set, otherwise the meter in effect (`numerator × 4/denominator`).
pub fn bar_len(
    map: &[MeterPoint],
    default_numerator: u32,
    pickup: Option<Beat>,
    index: u32,
) -> Beat {
    if index == 0
        && let Some(p) = pickup
    {
        return p;
    }
    let meter = meter_at(map, default_numerator, index);
    Beat::new(meter.numerator as i64 * 4, meter.denominator)
}

/// The exact beat `bar` starts at — the pickup plus the meter walk,
/// segment-wise (the map is short, so a pathological bar costs O(map), not
/// O(bar)). Saturates rather than wraps at absurd bars.
pub fn beat_at_bar(
    map: &[MeterPoint],
    default_numerator: u32,
    pickup: Option<Beat>,
    bar: u32,
) -> Beat {
    let mut beats = Beat::zero();
    if bar == 0 {
        return beats;
    }
    beats = beats
        .checked_add(bar_len(map, default_numerator, pickup, 0))
        .unwrap_or(Beat::new(i64::MAX, 1));
    let mut i = 1u32;
    while i < bar {
        let seg_end = map
            .iter()
            .map(|p| p.bar)
            .filter(|b| *b > i)
            .min()
            .unwrap_or(u32::MAX)
            .min(bar);
        let span = bar_len(map, default_numerator, pickup, i)
            .scale(i64::from(seg_end - i))
            .unwrap_or(Beat::new(i64::MAX, 1));
        beats = beats.checked_add(span).unwrap_or(Beat::new(i64::MAX, 1));
        i = seg_end;
    }
    beats
}

/// Bars elapsed at `beat` under the meter map (binary search over the
/// monotonic beat walk; saturates at absurd inputs).
pub fn bar_count_at_beat(
    map: &[MeterPoint],
    default_numerator: u32,
    pickup: Option<Beat>,
    beat: Beat,
) -> u32 {
    if beat <= Beat::zero() {
        return 0;
    }
    let mut lo = 0u32;
    let mut hi = 1u32;
    while beat_at_bar(map, default_numerator, pickup, hi) < beat {
        hi = hi.saturating_mul(2);
        if hi == u32::MAX {
            return hi;
        }
    }
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if beat_at_bar(map, default_numerator, pickup, mid) < beat {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_normalizes_by_gcd() {
        assert_eq!(Beat::new(2, 4), Beat::new(1, 2));
        assert_eq!(Beat::new(-3, 6), Beat { num: -1, den: 2 });
        assert_eq!(Beat::new(7, 1), Beat { num: 7, den: 1 });
        // Zero is canonical and a zero denominator can't divide by zero.
        assert_eq!(Beat::new(0, 7), Beat::zero());
        assert_eq!(Beat::new(5, 0), Beat::from_int(5));
    }

    #[test]
    fn orders_across_denominators() {
        assert!(Beat::new(1, 3) < Beat::new(1, 2));
        assert!(Beat::new(2, 3) > Beat::new(1, 2));
        assert_eq!(
            Beat::new(2, 4).cmp(&Beat::new(1, 2)),
            std::cmp::Ordering::Equal
        );
        // Sorting mixes denominators correctly.
        let mut v = vec![Beat::new(3, 4), Beat::new(1, 3), Beat::new(1, 2)];
        v.sort();
        assert_eq!(v, vec![Beat::new(1, 3), Beat::new(1, 2), Beat::new(3, 4)]);
    }

    #[test]
    fn comparison_never_overflows() {
        // Naive i64 cross-multiplication would overflow here; i128 doesn't.
        let a = Beat::new(i64::MAX, u32::MAX);
        let b = Beat::new(i64::MAX, u32::MAX - 1);
        assert!(a < b, "same numerator, smaller denominator is larger");
        assert!(Beat::new(i64::MAX, 1) > Beat::new(1, u32::MAX));
        assert!(Beat::new(i64::MIN, 1) < Beat::new(-1, u32::MAX));
    }

    #[test]
    fn add_sub_stay_exact() {
        assert_eq!(
            Beat::new(1, 3).checked_add(Beat::new(1, 6)).unwrap(),
            Beat::new(1, 2)
        );
        assert_eq!(
            Beat::new(1, 2).checked_sub(Beat::new(1, 3)).unwrap(),
            Beat::new(1, 6)
        );
        assert_eq!(
            Beat::zero().checked_sub(Beat::new(1, 4)).unwrap(),
            Beat::new(-1, 4)
        );
    }

    #[test]
    fn add_reports_overflow() {
        assert_eq!(
            Beat::new(i64::MAX, 1).checked_add(Beat::from_int(1)),
            Err(BeatError::Overflow)
        );
        assert_eq!(
            Beat::new(1, u32::MAX).checked_add(Beat::new(1, u32::MAX - 1)),
            Err(BeatError::Overflow),
            "the unreduced denominator u32::MAX * (u32::MAX - 1) doesn't fit"
        );
    }

    #[test]
    fn scales_rationally() {
        assert_eq!(Beat::new(2, 3).mul_rational(3, 4).unwrap(), Beat::new(1, 2));
        assert_eq!(Beat::new(1, 2).scale(3).unwrap(), Beat::new(3, 2));
        assert_eq!(Beat::new(1, 3).mul_rational(0, 1).unwrap(), Beat::zero());
    }

    #[test]
    fn triplet_math_is_exact_at_the_frame_boundary() {
        // 1/3 beat at 120 BPM = 1/6 s; at 48 kHz that is EXACTLY 8000 frames.
        assert_eq!(
            beat_to_frames(Beat::new(1, 3), Tempo(120.0), SampleRate(48_000)),
            Frames(8000)
        );
        // A whole beat at 120 BPM / 48 kHz = 0.5 s = 24000 frames.
        assert_eq!(
            beat_to_frames(Beat::from_int(1), Tempo(120.0), SampleRate(48_000)),
            Frames(24_000)
        );
    }

    #[test]
    fn rounds_half_away_from_zero() {
        // 1 beat at 40 BPM = 1.5 s; at 3 Hz that lands exactly on 4.5 frames,
        // and the specified rule rounds UP (banker's rounding would give 4).
        assert_eq!(
            beat_to_frames(Beat::from_int(1), Tempo(40.0), SampleRate(3)),
            Frames(5)
        );
    }

    #[test]
    fn clamps_degenerate_inputs() {
        // Tempo below 1 BPM floors at 1: 1 beat = 60 s at 48 kHz.
        assert_eq!(
            beat_to_frames(Beat::from_int(1), Tempo(0.5), SampleRate(48_000)),
            Frames(2_880_000)
        );
        // A position before the song's start has no frame: clamps to 0
        // (even though -4.5 would round to -5 away from zero).
        assert_eq!(
            beat_to_frames(Beat::new(-1, 2), Tempo(120.0), SampleRate(48_000)),
            Frames(0)
        );
    }

    #[test]
    fn integer_units_do_the_obvious_arithmetic() {
        assert_eq!(Frames(10) + Frames(5), Frames(15));
        assert_eq!(Frames(10) - Frames(5), Frames(5));
        assert_eq!(Frames::from(3u64), Frames(3));
        assert!(SampleRate(96_000) > SampleRate(44_100));
    }

    #[test]
    fn beat_displays_compactly() {
        assert_eq!(Beat::new(3, 2).to_string(), "3/2");
        assert_eq!(Beat::from_int(4).to_string(), "4");
        assert_eq!(Beat::new(-1, 2).to_string(), "-1/2");
    }

    #[test]
    fn beat_serde_is_a_flat_normalized_struct() {
        assert_eq!(
            serde_json::to_string(&Beat::new(1, 2)).unwrap(),
            r#"{"num":1,"den":2}"#
        );
        // Deserializing normalizes: 2/4 comes back as 1/2.
        let b: Beat = serde_json::from_str(r#"{"num":2,"den":4}"#).unwrap();
        assert_eq!(b, Beat::new(1, 2));
    }

    #[test]
    fn units_serialize_as_the_bare_inner_value() {
        assert_eq!(serde_json::to_string(&Frames(8)).unwrap(), "8");
        assert_eq!(serde_json::to_string(&Tempo(127.5)).unwrap(), "127.5");
        let f: Frames = serde_json::from_str("8").unwrap();
        assert_eq!(f, Frames(8));
    }
}
