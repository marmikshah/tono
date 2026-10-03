//! Structured diagnostics for song compilation.
//!
//! Validation collects ALL problems in one pass instead of stopping at the
//! first error. Each [`Diagnostic`] carries a stable machine `code`, a
//! [`Severity`], the object `path` it blames, a human-readable `message`, and
//! optional `remediation` text a caller can act on. Streaming blockers are
//! warnings for offline targets and errors for runtime targets.
//!
//! Codes are stable — tools pattern-match on them, so an existing code never
//! changes meaning. The bands are:
//!
//! - `T1xxx` — song composition/compile errors (unknown references, …)
//! - `T2xxx` — document validation
//! - `T3xxx` — program artifact/load
//!
//! The compiler (a later slice) returns `Err(`[`CompileError`]`)` exactly when
//! [`CompileError::has_errors`] is true; a collection of warnings alone never
//! fails a compile.

use serde::Serialize;

use crate::song::SongError;

/// How bad a [`Diagnostic`] is. Serializes lowercase (`"error"`, `"warning"`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The compile fails.
    Error,
    /// Reported, but the compile still succeeds (e.g. a streaming blocker).
    Warning,
}

impl std::fmt::Display for Severity {
    /// The same lowercase word serde uses.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => f.write_str("error"),
            Severity::Warning => f.write_str("warning"),
        }
    }
}

/// One structured problem: stable code, severity, the blamed object path, a
/// message, and optional remediation text. `code` is `&'static str` because
/// codes are a fixed vocabulary compiled into the binary, not user data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Stable machine code (`T1xxx`/`T2xxx`/`T3xxx` — see the module docs).
    pub code: &'static str,
    /// Error or warning.
    pub severity: Severity,
    /// The object path the diagnostic blames (e.g. `arrangement`).
    pub path: String,
    /// What is wrong, in words a caller can act on.
    pub message: String,
    /// How to fix it. Omitted from JSON when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl Diagnostic {
    /// An error-severity diagnostic.
    pub fn error(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            path: path.into(),
            message: message.into(),
            remediation: None,
        }
    }

    /// A warning-severity diagnostic (e.g. a streaming blocker — reported,
    /// never fatal).
    pub fn warning(
        code: &'static str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Diagnostic {
            code,
            severity: Severity::Warning,
            path: path.into(),
            message: message.into(),
            remediation: None,
        }
    }

    /// Attach remediation text (builder style).
    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }
}

/// The diagnostics a compile produced — the error type of `Song::compile`.
/// May be empty or hold only warnings; whether the compile FAILED is decided
/// solely by [`has_errors`](Self::has_errors), never by emptiness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompileError(pub Vec<Diagnostic>);

impl CompileError {
    /// A single diagnostic.
    pub fn one(d: Diagnostic) -> Self {
        CompileError(vec![d])
    }

    /// Append a diagnostic (validation collects everything in one pass).
    pub fn push(&mut self, d: Diagnostic) {
        self.0.push(d);
    }

    /// No diagnostics at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The error-severity diagnostics only.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| d.severity == Severity::Error)
    }

    /// Whether any diagnostic is an error — the one source of truth for
    /// "did the compile fail". Warnings alone never fail a compile.
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.severity == Severity::Error)
    }
}

impl std::fmt::Display for CompileError {
    /// One line per diagnostic: `code severity path: message (remediation)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{} {} {}: {}", d.code, d.severity, d.path, d.message)?;
            if let Some(r) = &d.remediation {
                write!(f, " ({r})")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

/// Song-compile errors become structured diagnostics in the `T1xxx` band.
/// Codes are pinned (tests assert the exact strings): `T1000` empty song,
/// `T1001` unknown track, `T1002` unknown pattern, `T1099` the catch-all for
/// a document that failed to build or validate.
impl From<&SongError> for Diagnostic {
    fn from(e: &SongError) -> Self {
        let (code, path, remediation) = match e {
            SongError::Empty => (
                "T1000",
                "tracks",
                "add at least one track (Song::add_track or Song::add)",
            ),
            SongError::UnknownTrack(_) => (
                "T1001",
                "arrangement",
                "add a track with this name or fix the placement's track field",
            ),
            SongError::UnknownPattern(_) => (
                "T1002",
                "arrangement",
                "add a pattern with this name or fix the placement's pattern field",
            ),
            SongError::Compile(_) => (
                "T1099",
                "doc",
                "fix the underlying document error and recompile",
            ),
        };
        // The message keeps SongError's existing wording verbatim — one
        // wording to learn, now with a code and a path attached.
        Diagnostic::error(code, path, e.to_string()).with_remediation(remediation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_one_line_per_diagnostic() {
        let d = Diagnostic::error(
            "T1001",
            "arrangement",
            "arrangement references unknown track 'nope'",
        )
        .with_remediation("add a track with this name or fix the placement's track field");
        assert_eq!(
            CompileError::one(d).to_string(),
            "T1001 error arrangement: arrangement references unknown track 'nope' \
             (add a track with this name or fix the placement's track field)"
        );
        // Without remediation there is no trailing parenthetical.
        assert_eq!(
            CompileError::one(Diagnostic::error("T1000", "tracks", "song has no tracks"))
                .to_string(),
            "T1000 error tracks: song has no tracks"
        );
        // Several diagnostics, one per line.
        let mut e = CompileError::one(Diagnostic::error("T1000", "tracks", "song has no tracks"));
        e.push(Diagnostic::warning(
            "T1500",
            "master",
            "reverb blocks streaming",
        ));
        assert_eq!(
            e.to_string(),
            "T1000 error tracks: song has no tracks\nT1500 warning master: reverb blocks streaming"
        );
    }

    #[test]
    fn severity_filters_and_decides_failure() {
        let mut e = CompileError::default();
        assert!(e.is_empty());
        assert!(!e.has_errors());
        e.push(Diagnostic::warning(
            "T1500",
            "master",
            "reverb blocks streaming",
        ));
        assert!(!e.has_errors(), "warnings alone never fail a compile");
        assert_eq!(e.errors().count(), 0);
        e.push(Diagnostic::error("T1000", "tracks", "song has no tracks"));
        assert!(e.has_errors());
        assert_eq!(e.errors().count(), 1);
        assert_eq!(e.errors().next().unwrap().code, "T1000");
    }

    #[test]
    fn song_error_codes_are_stable() {
        let empty = Diagnostic::from(&SongError::Empty);
        assert_eq!(empty.code, "T1000");
        assert_eq!(empty.path, "tracks");
        assert_eq!(empty.message, "song has no tracks");
        assert!(empty.remediation.is_some());

        let track = Diagnostic::from(&SongError::UnknownTrack("nope".into()));
        assert_eq!(track.code, "T1001");
        assert_eq!(track.path, "arrangement");
        assert!(
            track.message.contains("nope"),
            "the track name is in the message: {}",
            track.message
        );
        assert_eq!(
            track.remediation.as_deref(),
            Some("add a track with this name or fix the placement's track field")
        );

        let pattern = Diagnostic::from(&SongError::UnknownPattern("ghost".into()));
        assert_eq!(pattern.code, "T1002");
        assert_eq!(pattern.path, "arrangement");
        assert!(pattern.message.contains("ghost"));

        let compile = Diagnostic::from(&SongError::Compile("bad doc".into()));
        assert_eq!(compile.code, "T1099");
        assert_eq!(compile.message, "bad doc");

        // Every converted diagnostic is an error carrying remediation.
        for d in [empty, track, pattern, compile] {
            assert_eq!(d.severity, Severity::Error);
            assert!(d.remediation.is_some());
        }
    }

    #[test]
    fn serde_skips_absent_remediation_and_lowercases_severity() {
        let bare = serde_json::to_value(Diagnostic::error("T1000", "tracks", "song has no tracks"))
            .unwrap();
        assert_eq!(
            bare,
            serde_json::json!({
                "code": "T1000",
                "severity": "error",
                "path": "tracks",
                "message": "song has no tracks",
            }),
            "no remediation key at all when None"
        );
        let full = serde_json::to_value(
            Diagnostic::warning("T1500", "master", "reverb blocks streaming")
                .with_remediation("render offline instead"),
        )
        .unwrap();
        assert_eq!(full["severity"], "warning");
        assert_eq!(full["remediation"], "render offline instead");
    }
}
