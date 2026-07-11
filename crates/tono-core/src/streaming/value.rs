//! Per-sample [`Value`] evaluation — the closed-form modulators plus Rand's
//! stateful walk, byte-identical to the offline `eval_value`.

use crate::dsl::{Curve, Modulator, Shape, Value, note_to_hz};
use crate::dsp::{Rng, adsr_env};
use crate::render::{osc, rand_seed};

/// A per-sample evaluator for a dsl [`Value`], byte-identical to `eval_value` at
/// the given absolute sample index. Const/Note are constant; the modulators match
/// the offline formulas. `Rand` is stateful (carries its self-seeded walk) so it
/// must be stepped once per sample in order.
pub(super) enum Val {
    Const(f32),
    Slide {
        from: f32,
        to: f32,
        secs: f32,
        curve: Curve,
        srf: f32,
    },
    Lfo {
        shape: Shape,
        rate: f32,
        depth: f32,
        center: f32,
        srf: f32,
    },
    Arp {
        steps: Vec<f32>,
        rate: f32,
        srf: f32,
    },
    EnvMod {
        a: f32,
        d: f32,
        s: f32,
        r: f32,
        punch: f32,
        from: f32,
        to: f32,
        srf: f32,
        rel_start: f32,
    },
    Rand {
        rng: Rng,
        prev: f32,
        next: f32,
        phase: f32,
        inc: f32,
        from: f32,
        to: f32,
    },
}

impl Val {
    pub(super) fn build(v: &Value, sr: u32, n: usize) -> Self {
        let srf = sr as f32;
        match v {
            Value::Const(c) => Val::Const(*c),
            Value::Note(name) => Val::Const(note_to_hz(name).unwrap_or(440.0)),
            Value::Modulated(m) => match m {
                Modulator::Slide {
                    from,
                    to,
                    secs,
                    curve,
                } => Val::Slide {
                    from: *from,
                    to: *to,
                    // Floor `secs` so an unvalidated `secs == 0` can't make
                    // `t/secs` a NaN that then poisons the whole stream —
                    // the same guard the offline renderer applies.
                    secs: secs.max(1e-6),
                    curve: *curve,
                    srf,
                },
                Modulator::Lfo {
                    shape,
                    rate,
                    depth,
                    center,
                } => Val::Lfo {
                    shape: *shape,
                    rate: *rate,
                    depth: *depth,
                    center: *center,
                    srf,
                },
                // Empty steps would divide by zero in eval; an unvalidated doc
                // must not panic (the offline path yields 0.0 the same way).
                Modulator::Arp { steps, .. } if steps.is_empty() => Val::Const(0.0),
                Modulator::Arp { steps, rate } => Val::Arp {
                    steps: steps.clone(),
                    rate: *rate,
                    srf,
                },
                Modulator::EnvMod { adsr, from, to } => Val::EnvMod {
                    a: adsr.a,
                    d: adsr.d,
                    s: adsr.s,
                    r: adsr.r,
                    punch: adsr.punch,
                    from: *from,
                    to: *to,
                    srf,
                    rel_start: (n as f32 / srf - adsr.r).max(0.0),
                },
                Modulator::Rand {
                    from,
                    to,
                    rate,
                    seed,
                } => {
                    let mut rng = Rng::new(rand_seed(*seed, *from, *to, *rate));
                    let inc = rate.max(1e-4) / srf;
                    let prev = rng.range(*from, *to);
                    let next = rng.range(*from, *to);
                    Val::Rand {
                        rng,
                        prev,
                        next,
                        phase: 0.0,
                        inc,
                        from: *from,
                        to: *to,
                    }
                }
            },
        }
    }

    pub(super) fn eval(&mut self, t: usize) -> f32 {
        match self {
            Val::Const(c) => *c,
            Val::Slide {
                from,
                to,
                secs,
                curve,
                srf,
            } => {
                let tt = t as f32 / *srf;
                let p = (tt / *secs).clamp(0.0, 1.0);
                match curve {
                    Curve::Lin => *from + (*to - *from) * p,
                    Curve::Exp if *from > 0.0 && *to > 0.0 => *from * (*to / *from).powf(p),
                    Curve::Exp => {
                        let e = p * p;
                        *from + (*to - *from) * e
                    }
                }
            }
            Val::Lfo {
                shape,
                rate,
                depth,
                center,
                srf,
            } => {
                let phase = (t as f32 / *srf * *rate).fract();
                *center + *depth * osc(*shape, phase)
            }
            Val::Arp { steps, rate, srf } => {
                let tt = t as f32 / *srf;
                let idx = (tt * *rate) as usize % steps.len();
                steps[idx]
            }
            Val::EnvMod {
                a,
                d,
                s,
                r,
                punch,
                from,
                to,
                srf,
                rel_start,
            } => {
                let v = adsr_env(t as f32 / *srf, *a, *d, *s, *r, *punch, *rel_start);
                *from + (*to - *from) * v
            }
            Val::Rand {
                rng,
                prev,
                next,
                phase,
                inc,
                from,
                to,
            } => {
                let sm = *phase * *phase * (3.0 - 2.0 * *phase);
                let out = *prev + (*next - *prev) * sm;
                *phase += *inc;
                while *phase >= 1.0 {
                    *phase -= 1.0;
                    *prev = *next;
                    *next = rng.range(*from, *to);
                }
                out
            }
        }
    }
}
