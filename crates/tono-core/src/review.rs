//! Deterministic sound review: grade a rendered sound against archetype
//! targets plus a universal ship checklist, emitting actionable findings.
//!
//! This is the "Review" half of a review → polish → review loop. It is a pure
//! function of the [`Analysis`] (and, for loops, the seam) — so a given sound
//! always reviews the same way: the critique is reproducible, not vibes. The
//! targets encode the `sound-designer` methodology (archetype table + ship
//! checklist); each finding names the measured value, the target, a PASS/WARN/
//! FAIL verdict, and the concrete fix to try next.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::Analysis;
use crate::dsl::{Playback, SoundDoc};

/// The SFX/music archetype a sound is judged against. Omit it to run only the
/// universal checks (clipping, silence, loop seam, onset count).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Archetype {
    /// Laser / zap: short, bright, falling, very punchy.
    Laser,
    /// Coin / pickup: two bright blips, moderate punch.
    Coin,
    /// Jump: short rising sweep, fast gate.
    Jump,
    /// Explosion / impact: low-centred body with a ring tail.
    Impact,
    /// UI click / confirm: tiny, bright, instant.
    Ui,
    /// Ambience / bed: sustained, dark, low crest, looping.
    Ambience,
    /// BGM / band: a mixed musical loop.
    Bgm,
}

/// A single criterion's verdict. Ordered so the worst across all findings is
/// the overall grade (`Fail` > `Warn` > `Pass`).
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Meets the target.
    Pass,
    /// Borderline — worth a look, not a blocker.
    Warn,
    /// Out of spec — fix before shipping.
    Fail,
}

/// One graded criterion.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Finding {
    /// What was checked (e.g. `"peak"`, `"attack"`, `"loop seam"`).
    pub criterion: String,
    /// PASS / WARN / FAIL.
    pub status: Status,
    /// The measured value, formatted with units.
    pub value: String,
    /// The target this was judged against.
    pub target: String,
    /// The concrete next edit to try (empty when passing).
    pub fix: String,
}

/// The full review of one sound.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Review {
    /// The archetype judged against (if any).
    pub archetype: Option<Archetype>,
    /// Overall grade — the worst finding's status.
    pub grade: Status,
    /// How many findings landed at each status.
    pub pass: u32,
    pub warn: u32,
    pub fail: u32,
    /// Every graded criterion, worst first.
    pub findings: Vec<Finding>,
    /// One-line human summary.
    pub summary: String,
}

/// Inclusive numeric range a criterion should fall within.
struct Range {
    lo: f32,
    hi: f32,
}

/// Per-archetype targets. `None` fields are not checked for that archetype.
struct Targets {
    duration_s: Option<Range>,
    attack_max_ms: Option<f32>,
    centroid_hz: Option<Range>,
    crest_db: Option<Range>,
    /// True for one-shots (expect a single onset); false for loops/music.
    one_shot: bool,
}

impl Archetype {
    fn targets(self) -> Targets {
        let r = |lo, hi| Some(Range { lo, hi });
        match self {
            Archetype::Laser => Targets {
                duration_s: r(0.1, 0.4),
                attack_max_ms: Some(5.0),
                centroid_hz: r(2000.0, 8000.0),
                crest_db: r(12.0, 99.0),
                one_shot: true,
            },
            Archetype::Coin => Targets {
                duration_s: r(0.3, 0.9),
                attack_max_ms: Some(10.0),
                centroid_hz: r(1500.0, 4500.0),
                crest_db: r(8.0, 14.0),
                one_shot: true,
            },
            Archetype::Jump => Targets {
                duration_s: r(0.2, 0.4),
                attack_max_ms: Some(10.0),
                centroid_hz: None, // rising — direction, not a fixed band
                crest_db: r(6.0, 14.0),
                one_shot: true,
            },
            Archetype::Impact => Targets {
                duration_s: r(0.4, 2.0),
                attack_max_ms: Some(10.0),
                centroid_hz: r(150.0, 1000.0),
                crest_db: r(10.0, 16.0),
                one_shot: true,
            },
            Archetype::Ui => Targets {
                duration_s: r(0.05, 0.25),
                attack_max_ms: Some(5.0),
                centroid_hz: r(2000.0, 8000.0),
                crest_db: r(12.0, 99.0),
                one_shot: true,
            },
            Archetype::Ambience => Targets {
                duration_s: None,
                attack_max_ms: None,
                centroid_hz: r(0.0, 1500.0),
                crest_db: r(0.0, 8.0),
                one_shot: false,
            },
            Archetype::Bgm => Targets {
                duration_s: None,
                attack_max_ms: None,
                centroid_hz: None,
                crest_db: r(6.0, 14.0),
                one_shot: false,
            },
        }
    }
}

fn finding(criterion: &str, status: Status, value: String, target: &str, fix: &str) -> Finding {
    Finding {
        criterion: criterion.into(),
        status,
        value,
        target: target.into(),
        fix: if status == Status::Pass {
            String::new()
        } else {
            fix.into()
        },
    }
}

/// Grade a sound. `seam_db` is the loop-seam discontinuity in dB, supplied by
/// the caller when the document loops (and `None` otherwise).
pub fn review(
    doc: &SoundDoc,
    a: &Analysis,
    archetype: Option<Archetype>,
    seam_db: Option<f32>,
) -> Review {
    let mut f = Vec::new();
    let is_loop = matches!(doc.playback, Playback::Loop { .. });

    // ---- Universal ship checklist (every sound) ----

    // Sample peak — the renderer's limiter already caps it at ≈ −0.1 dBFS, so
    // hot is fine and true clipping is impossible here; the only fault is being
    // buried. (Inter-sample clipping is the `true peak` check below.)
    if a.peak_dbfs < -12.0 {
        f.push(finding(
            "peak",
            Status::Warn,
            format!("{:.1} dBFS", a.peak_dbfs),
            "−12..−0.1 dBFS",
            "raise gain or add a normalize target_lufs",
        ));
    } else {
        f.push(finding(
            "peak",
            Status::Pass,
            format!("{:.1} dBFS", a.peak_dbfs),
            "−12..−0.1 dBFS",
            "",
        ));
    }

    // Inter-sample (true) peak: only an actual over-0 dBTP reading clips on
    // conversion/playback — a hot-but-under sound is fine for game SFX.
    if a.true_peak_dbfs > 1.0 {
        f.push(finding(
            "true peak",
            Status::Fail,
            format!("{:.1} dBTP", a.true_peak_dbfs),
            "≤ 0 dBTP",
            "add normalize { ceiling_dbtp: -1 }",
        ));
    } else if a.true_peak_dbfs > 0.0 {
        f.push(finding(
            "true peak",
            Status::Warn,
            format!("{:.1} dBTP", a.true_peak_dbfs),
            "≤ 0 dBTP",
            "add normalize { ceiling_dbtp: -1 } for streaming safety",
        ));
    }

    // Leading dead air.
    f.push(silence_check(
        "head silence",
        a.head_silence_ms,
        10.0,
        "trim with env.a, the first note's step, or the layer's at",
    ));
    // Trailing dead air (ships as file size + latency). A trim hint, not a
    // blocker — a resonant tail decaying toward silence is legitimate, so this
    // never grades worse than WARN.
    f.push(silence_check(
        "tail silence",
        a.tail_silence_ms,
        100.0,
        "shorten the document duration (or keep it for a deliberate ring-out)",
    ));

    // Loop seam (loops only).
    if is_loop {
        let seam = seam_db.unwrap_or(0.0);
        let status = if seam <= -40.0 {
            Status::Pass
        } else if seam <= -25.0 {
            Status::Warn
        } else {
            Status::Fail
        };
        f.push(finding(
            "loop seam",
            status,
            format!("{seam:.1} dB"),
            "< −40 dB",
            "raise playback.loop.crossfade_secs, or match start/end levels",
        ));
    }

    // ---- Archetype-specific ----
    if let Some(arch) = archetype {
        let t = arch.targets();

        // One-shots expect exactly one onset; loops/music do not.
        if t.one_shot {
            let status = if a.onset_count == 1 {
                Status::Pass
            } else if a.onset_count == 0 {
                Status::Fail
            } else {
                Status::Warn
            };
            f.push(finding(
                "onset count",
                status,
                format!("{}", a.onset_count),
                "1 (single hit)",
                "overlapping notes / a double-trigger / chorus re-attack — separate or shorten",
            ));
        }

        if let Some(r) = t.duration_s {
            f.push(range_check(
                "duration",
                a.duration_secs,
                &r,
                "s",
                2,
                "trim the duration / envelope to the archetype length",
            ));
        }
        if let Some(max) = t.attack_max_ms {
            let status = if a.attack_time_ms <= max {
                Status::Pass
            } else if a.attack_time_ms <= max * 3.0 {
                Status::Warn
            } else {
                Status::Fail
            };
            f.push(finding(
                "attack",
                status,
                format!("{:.0} ms", a.attack_time_ms),
                &format!("< {max:.0} ms"),
                "set env.a: 0 and add punch; check head silence isn't eating the onset",
            ));
        }
        if let Some(r) = t.centroid_hz {
            f.push(range_check(
                "centroid",
                a.spectral_centroid_hz,
                &r,
                "Hz",
                0,
                "too dark: raise a filter cutoff / brighter wave; too harsh: lowpass 6–9 kHz",
            ));
        }
        if let Some(r) = t.crest_db {
            f.push(range_check(
                "crest",
                a.crest_factor_db,
                &r,
                "dB",
                0,
                "low: add punch, shorten attack; high: a compressor after the transient",
            ));
        }
    }

    // ---- Tally ----
    f.sort_by_key(|x| std::cmp::Reverse(x.status)); // worst first
    let (mut pass, mut warn, mut fail) = (0u32, 0u32, 0u32);
    for x in &f {
        match x.status {
            Status::Pass => pass += 1,
            Status::Warn => warn += 1,
            Status::Fail => fail += 1,
        }
    }
    let grade = if fail > 0 {
        Status::Fail
    } else if warn > 0 {
        Status::Warn
    } else {
        Status::Pass
    };
    let arch_label = archetype
        .map(|a| format!("{a:?}").to_lowercase())
        .unwrap_or_else(|| "generic".into());
    let summary = format!(
        "{} [{arch_label}]: {grade:?} — {pass} pass, {warn} warn, {fail} fail",
        doc.name
    )
    .to_uppercase();

    Review {
        archetype,
        grade,
        pass,
        warn,
        fail,
        findings: f,
        summary,
    }
}

/// Silence: PASS under `warn_at`, else WARN. Never FAIL — trailing/leading
/// silence is a trim hint (file size + latency), not a quality blocker.
fn silence_check(name: &str, value: f32, warn_at: f32, fix: &str) -> Finding {
    let status = if value < warn_at {
        Status::Pass
    } else {
        Status::Warn
    };
    finding(
        name,
        status,
        format!("{value:.0} ms"),
        &format!("< {warn_at:.0} ms"),
        fix,
    )
}

/// Inside `[lo, hi]` is PASS; within 25% outside is WARN; further is FAIL.
/// `prec` is the display precision (seconds need decimals, Hz/dB don't).
fn range_check(name: &str, value: f32, r: &Range, unit: &str, prec: usize, fix: &str) -> Finding {
    let margin = (r.hi - r.lo).abs() * 0.25 + 1e-3;
    let status = if value >= r.lo && value <= r.hi {
        Status::Pass
    } else if value >= r.lo - margin && value <= r.hi + margin {
        Status::Warn
    } else {
        Status::Fail
    };
    finding(
        name,
        status,
        format!("{value:.prec$} {unit}"),
        &format!("{:.prec$}–{:.prec$} {unit}", r.lo, r.hi),
        fix,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{loop_seam_db, render};

    fn doc(json: &str) -> SoundDoc {
        serde_json::from_str(json).expect("deserialize")
    }

    fn analyze_doc(d: &SoundDoc) -> Analysis {
        crate::analysis::stats(&render(d), d.sample_rate)
    }

    #[test]
    fn clean_laser_passes_its_archetype() {
        let d = doc(
            r#"{ "name": "laser", "duration": 0.22, "root": { "type": "mul", "inputs": [
                { "type": "square", "duty": 0.25,
                  "freq": { "slide": { "from": 1800, "to": 300, "secs": 0.18, "curve": "exp" } } },
                { "type": "env", "a": 0.0, "d": 0.18, "s": 0.0, "r": 0.02, "punch": 0.4 } ] } }"#,
        );
        let a = analyze_doc(&d);
        let r = review(&d, &a, Some(Archetype::Laser), None);
        assert_ne!(
            r.grade,
            Status::Fail,
            "clean laser should not fail: {}",
            r.summary
        );
        // Every finding names a fix unless it passed.
        for fnd in &r.findings {
            assert_eq!(fnd.fix.is_empty(), fnd.status == Status::Pass);
        }
    }

    #[test]
    fn ambience_crest_too_high_is_flagged() {
        // A percussive blip judged as an ambience bed: crest far above 8 dB.
        let d = doc(
            r#"{ "name": "blip", "duration": 0.3, "root": { "type": "mul", "inputs": [
                { "type": "sine", "freq": 660 },
                { "type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.02, "punch": 0.6 } ] } }"#,
        );
        let a = analyze_doc(&d);
        let r = review(&d, &a, Some(Archetype::Ambience), None);
        let crest = r.findings.iter().find(|x| x.criterion == "crest").unwrap();
        assert_ne!(
            crest.status,
            Status::Pass,
            "a punchy blip is not a calm bed"
        );
        assert!(!crest.fix.is_empty());
    }

    #[test]
    fn loop_seam_is_graded_for_loops() {
        let d = doc(r#"{ "name": "bed", "duration": 1.0,
                 "playback": { "mode": "loop", "crossfade_secs": 0.3 },
                 "root": { "type": "noise", "color": "pink" } }"#);
        let mono = render(&d);
        let seam = loop_seam_db(&mono);
        let a = analyze_doc(&d);
        let r = review(&d, &a, Some(Archetype::Ambience), Some(seam));
        assert!(
            r.findings.iter().any(|x| x.criterion == "loop seam"),
            "a loop must be graded on its seam"
        );
    }

    #[test]
    fn generic_review_runs_only_universal_checks() {
        let d = doc(r#"{ "name": "x", "duration": 0.3, "root": { "type": "sine", "freq": 440 } }"#);
        let a = analyze_doc(&d);
        let r = review(&d, &a, None, None);
        assert!(r.archetype.is_none());
        // No archetype ⇒ no attack/centroid/crest/onset criteria.
        assert!(r.findings.iter().all(|x| x.criterion != "centroid"));
        assert!(r.findings.iter().any(|x| x.criterion == "peak"));
    }
}
