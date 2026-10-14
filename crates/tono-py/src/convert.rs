//! Conversions shared by the typed API: the exception types, beat parsing,
//! Python-to-JSON for effect params, and diagnostics as dicts.

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use tono_core::diag::{CompileError as CoreCompileError, Diagnostic};
use tono_core::dsl::SeqWave;
use tono_core::units::Beat;

pyo3::create_exception!(
    tono,
    TonoError,
    pyo3::exceptions::PyException,
    "The base error for every typed-API failure (program load, …)."
);
pyo3::create_exception!(
    tono,
    CompileError,
    TonoError,
    "A song failed to compile. `.diagnostics` is a list of dicts — \
     {code, severity, path, message, remediation?} — one per problem found."
);

/// The document string of a seq wave (`SeqWave` serializes lowercase).
pub(crate) fn wave_str(wave: SeqWave) -> String {
    serde_json::to_value(wave)
        .expect("a SeqWave serializes")
        .as_str()
        .expect("a SeqWave serializes to a string")
        .to_owned()
}

/// Diagnostics as Python dicts: {code, severity, path, message, remediation?}.
pub(crate) fn diagnostics_list<'py>(
    py: Python<'py>,
    diagnostics: &[Diagnostic],
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    for d in diagnostics {
        let dict = PyDict::new(py);
        dict.set_item("code", d.code)?;
        dict.set_item("severity", d.severity.to_string())?;
        dict.set_item("path", &d.path)?;
        dict.set_item("message", &d.message)?;
        if let Some(remediation) = &d.remediation {
            dict.set_item("remediation", remediation)?;
        }
        list.append(dict)?;
    }
    Ok(list)
}

/// Map a core compile failure to the Python `CompileError`, with the
/// structured diagnostics attached as `.diagnostics`.
pub(crate) fn compile_error(py: Python<'_>, err: CoreCompileError) -> PyErr {
    let pyerr = CompileError::new_err(err.to_string());
    match diagnostics_list(py, &err.0) {
        Ok(list) => {
            // Attaching to a fresh exception instance cannot realistically
            // fail; worst case the exception carries only its message.
            let _ = pyerr.value(py).setattr("diagnostics", list);
        }
        Err(e) => return e,
    }
    pyerr
}

/// A `(num, den)` pair to a normalized beat, validating the denominator.
fn beat_from_parts(num: i64, den: i64) -> PyResult<Beat> {
    if den <= 0 || den > i64::from(u32::MAX) {
        return Err(PyValueError::new_err(format!(
            "beat denominator must be in 1..={}, got {den}",
            u32::MAX
        )));
    }
    Ok(Beat::new(num, den as u32))
}

/// A float to a beat by its EXACT binary value (mantissa × 2^exponent): 0.5
/// is exactly 1/2, and float 0.1 is 0.1's binary expansion — NOT 1/10 (pass
/// `fractions.Fraction` for exact decimals). Values whose exact form doesn't
/// fit the beat rational (i64/u32) are rejected, pointing at `Fraction`.
fn beat_from_f64(x: f64) -> PyResult<Beat> {
    if !x.is_finite() {
        return Err(PyValueError::new_err(format!(
            "a beat must be finite, got {x}"
        )));
    }
    if x == 0.0 {
        return Ok(Beat::zero());
    }
    // IEEE-754 double: value = ±mantissa × 2^e2.
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (mantissa, e2) = if raw_exp == 0 {
        (frac, -1074i32) // subnormal: frac × 2^-1074
    } else {
        (frac | (1u64 << 52), raw_exp - 1075) // normal: (2^52 | frac) × 2^(exp-1075)
    };
    let mut m = mantissa as i64;
    if bits >> 63 == 1 {
        m = -m;
    }
    // Strip trailing zeros so the denominator stays as small as possible.
    let tz = m.trailing_zeros();
    m >>= tz;
    let e2 = e2 + tz as i32;
    let inexact = |what: &str| {
        PyValueError::new_err(format!(
            "float {x} {what} — pass fractions.Fraction for exact decimals \
             (e.g. Fraction(1, 10) for 1/10 of a beat)"
        ))
    };
    if e2 >= 0 {
        if e2 > 62 {
            return Err(inexact("is too large to represent exactly as a beat"));
        }
        let num = i64::try_from((m as i128) << (e2 as u32))
            .map_err(|_| inexact("is too large to represent exactly as a beat"))?;
        Ok(Beat::from_int(num))
    } else {
        let shift = (-e2) as u32;
        if shift >= 32 {
            return Err(inexact("has no exact beat representation"));
        }
        Ok(Beat::new(m, 1u32 << shift))
    }
}

/// Convert a Python beat position to an exact `units::Beat` — the one
/// conversion every beat-taking method (`set_tempo_map`, `set_pickup`,
/// `add_marker`) shares. Accepts an int (whole beats), a float (its EXACT
/// binary value — see [`beat_from_f64`]), a `fractions.Fraction` (exact
/// decimals), or a `(num, den)` int tuple. Anything else is a TypeError
/// naming the accepted forms.
pub(crate) fn py_beat(obj: &Bound<'_, PyAny>) -> PyResult<Beat> {
    if let Ok(whole) = obj.extract::<i64>() {
        return Ok(Beat::from_int(whole));
    }
    // fractions.Fraction (duck-typed: integer numerator/denominator) — before
    // the float arm, since a Fraction's __float__ would round it to a double.
    if let (Ok(num), Ok(den)) = (
        obj.getattr("numerator").and_then(|n| n.extract::<i64>()),
        obj.getattr("denominator").and_then(|d| d.extract::<i64>()),
    ) {
        return beat_from_parts(num, den);
    }
    if let Ok((num, den)) = obj.extract::<(i64, i64)>() {
        return beat_from_parts(num, den);
    }
    if let Ok(x) = obj.extract::<f64>() {
        return beat_from_f64(x);
    }
    let type_name = obj
        .get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "?".into());
    Err(PyTypeError::new_err(format!(
        "a beat must be an int (whole beats), a float, a fractions.Fraction, or a \
         (num, den) int tuple — got {type_name:?}"
    )))
}

/// A Python value to JSON for effect params: None/bool/int/float/str, and
/// dicts/lists recursively (a modulator param like `{"type": "lfo", ...}` is
/// just a nested dict).
pub(crate) fn py_to_json(obj: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    if obj.is_none() {
        return Ok(serde_json::Value::Null);
    }
    if let Ok(b) = obj.extract::<bool>() {
        return Ok(b.into());
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(i.into());
    }
    if let Ok(f) = obj.extract::<f64>() {
        return serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                PyValueError::new_err(format!("non-finite number {f} in effect params"))
            });
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(s.into());
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            map.insert(k.extract::<String>()?, py_to_json(&v)?);
        }
        return Ok(serde_json::Value::Object(map));
    }
    if let Ok(items) = obj.extract::<Vec<Bound<'_, PyAny>>>() {
        return Ok(serde_json::Value::Array(
            items.iter().map(py_to_json).collect::<PyResult<Vec<_>>>()?,
        ));
    }
    let type_name = obj
        .get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "?".into());
    Err(PyTypeError::new_err(format!(
        "unsupported effect param value of type {type_name:?} — use None, bool, int, float, \
         str, list, or dict"
    )))
}

/// Register the exception types on the extension module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("TonoError", m.py().get_type::<TonoError>())?;
    m.add("CompileError", m.py().get_type::<CompileError>())?;
    Ok(())
}
