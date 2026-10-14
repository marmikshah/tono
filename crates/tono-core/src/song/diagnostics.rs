//! One-pass validation for song compilation: the tempo/meter maps, pickup,
//! grid placement, and sections/markers checks (T1003–T1006). Every problem
//! is collected, never first-error — see [`crate::diag`].

use super::{MeterPoint, Song};
use crate::diag::{CompileError, Diagnostic};
use crate::dsl::bounds::{MAX_METER_DENOMINATOR, MAX_METER_POINTS, MapProblem, tempo_map_problems};
use crate::units::Beat;

/// Every problem with a meter map: bounded length, first point at bar 0,
/// numerators ≥ 1, power-of-two denominators up to 64, strictly ascending
/// bars. An empty map (the song's default meter throughout) has none.
pub fn meter_map_problems(map: &[MeterPoint]) -> Vec<MapProblem> {
    let mut out = Vec::new();
    let mut push = |path: String, message: String, remediation: &'static str| {
        out.push(MapProblem {
            path,
            message,
            remediation,
        })
    };
    if map.len() > MAX_METER_POINTS {
        push(
            "meter_map".into(),
            format!(
                "meter_map is capped at {MAX_METER_POINTS} points, got {}",
                map.len()
            ),
            "consolidate repeated time-signature changes",
        );
    }
    if let Some(first) = map.first()
        && first.bar != 0
    {
        push(
            "meter_map[0].bar".into(),
            "meter_map's first point must be at bar 0".into(),
            "add the opening time signature at bar 0",
        );
    }
    for (i, p) in map.iter().enumerate() {
        if p.numerator < 1 {
            push(
                format!("meter_map[{i}].numerator"),
                "time-signature numerator must be ≥ 1".into(),
                "use a numerator like 3 (3/4) or 6 (6/8)",
            );
        }
        if !p.denominator.is_power_of_two() || p.denominator > MAX_METER_DENOMINATOR {
            push(
                format!("meter_map[{i}].denominator"),
                format!(
                    "time-signature denominator must be a power of two ≤ {MAX_METER_DENOMINATOR}, got {}",
                    p.denominator
                ),
                "use 2, 4, 8, 16, 32, or 64",
            );
        }
        if i > 0 && p.bar <= map[i - 1].bar {
            push(
                format!("meter_map[{i}].bar"),
                "meter_map must be strictly ascending by bar".into(),
                "sort the changes and merge any duplicates",
            );
        }
    }
    out
}

impl Song {
    /// Validate the tempo/meter maps, pickup, grid placement, and
    /// sections/markers — one pass, every problem collected (T1003–T1006).
    pub(crate) fn validate_maps(&self, diags: &mut CompileError) {
        for p in tempo_map_problems(&self.tempo_map) {
            diags.push(
                Diagnostic::error("T1003", p.path, p.message).with_remediation(p.remediation),
            );
        }
        for p in meter_map_problems(&self.meter_map) {
            diags.push(
                Diagnostic::error("T1004", p.path, p.message).with_remediation(p.remediation),
            );
        }
        if let Some(pickup) = self.pickup
            && pickup < Beat::zero()
        {
            diags.push(
                Diagnostic::error("T1004", "pickup", "the pickup bar can't be negative")
                    .with_remediation("use zero (no pickup) or a positive beat length"),
            );
        }
        // Grid placement (T1005): every placement must land on a step.
        if !self.plain_meter() {
            for (i, pl) in self.arrangement.iter().enumerate() {
                if self.beat_to_step(self.beat_at_bar(pl.bar)).is_err() {
                    diags.push(
                        Diagnostic::error("T1005", format!("arrangement[{i}].bar"), format!("bar {} lands between grid steps", pl.bar))
                            .with_remediation("raise steps_per_beat, or move the placement/meter change onto the grid"),
                    );
                }
            }
        }
        // Sections and markers (T1006).
        for (i, s) in self.sections.iter().enumerate() {
            if s.name.is_empty() {
                diags.push(
                    Diagnostic::error(
                        "T1006",
                        format!("sections[{i}].name"),
                        "a section needs a name",
                    )
                    .with_remediation("name it (e.g. \"verse\", \"chorus\")"),
                );
            }
            if s.bars < 1 {
                diags.push(
                    Diagnostic::error(
                        "T1006",
                        format!("sections[{i}].bars"),
                        "a section must be at least one bar",
                    )
                    .with_remediation("set bars ≥ 1"),
                );
            }
        }
        for (i, m) in self.markers.iter().enumerate() {
            if m.name.is_empty() {
                diags.push(
                    Diagnostic::error(
                        "T1006",
                        format!("markers[{i}].name"),
                        "a marker needs a name",
                    )
                    .with_remediation("name it (e.g. \"drop\", \"cue\")"),
                );
            }
        }
    }
}
