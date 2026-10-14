//! `tono.Voice` and the `tono.instruments` submodule: one function per
//! catalog family, each taking a variant slug.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use tono_core::catalog::{self, Voice as CoreVoice};

use crate::convert::wave_str;

/// A catalog instrument voice — what a `Song` track plays. Construct one with
/// the `tono.instruments` functions; tune it with the builder methods.
#[pyclass(module = "tono")]
pub(crate) struct Voice {
    pub(crate) inner: CoreVoice,
}

#[pymethods]
impl Voice {
    /// The voice's display name (read-only).
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// The synthesis wave the voice plays, as its document string (`"piano"`,
    /// `"bass"`, `"kit"`, …) (read-only).
    #[getter]
    fn wave(&self) -> String {
        wave_str(self.inner.wave)
    }

    /// Set the channel fader (0..2, 1 = unity); returns the voice for chaining.
    fn gain<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.gain = x;
        slf
    }

    /// Set the stereo position (−1 hard left .. 1 hard right); returns the
    /// voice for chaining.
    fn pan<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.pan = x;
        slf
    }

    /// Set the reverb send (0..1, 0 = dry); returns the voice for chaining.
    fn reverb<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.reverb = x;
        slf
    }

    /// Override the song's swing for this track (0..1); returns the voice for
    /// chaining.
    fn swing<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.swing = Some(x);
        slf
    }

    /// Override the song's humanize for this track (0..1); returns the voice
    /// for chaining.
    fn humanize<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.humanize = Some(x);
        slf
    }

    fn __repr__(&self) -> String {
        format!("Voice('{}')", self.inner.name)
    }
}

/// Reject an unknown instrument variant, naming the valid ones.
fn unknown_variant(family: &str, variant: &str, valid: &[&str]) -> PyErr {
    PyValueError::new_err(format!(
        "unknown {family} variant {variant:?} — expected one of: {}",
        valid.join(", ")
    ))
}

/// One `tono.instruments` family function: variant slug → catalog constructor
/// (the same slugs the `tono catalog` CLI lists). `$default` is a `tt` rather
/// than a `literal` so PyO3 sees the bare string and renders it in the
/// function's `__text_signature__` (a `literal` fragment arrives wrapped in an
/// invisible group, which PyO3 prints as `...`).
macro_rules! instrument_fn {
    ($name:ident, $doc:literal, $family:literal, $default:tt, $($slug:literal => $ctor:path),+ $(,)?) => {
        #[doc = $doc]
        #[pyfunction]
        #[pyo3(signature = (variant = $default))]
        fn $name(variant: &str) -> PyResult<Voice> {
            let inner = match variant {
                $($slug => $ctor(),)+
                other => return Err(unknown_variant($family, other, &[$($slug),+])),
            };
            Ok(Voice { inner })
        }
    };
}

instrument_fn!(
    piano,
    "A piano voice — variants: grand (default), bright, mellow, felt, upright, honky-tonk.",
    "piano",
    "grand",
    "grand" => catalog::GrandPiano::grand,
    "bright" => catalog::GrandPiano::bright,
    "mellow" => catalog::GrandPiano::mellow,
    "felt" => catalog::GrandPiano::felt,
    "upright" => catalog::GrandPiano::upright,
    "honky-tonk" => catalog::GrandPiano::honky_tonk,
);
instrument_fn!(
    electric_piano,
    "An electric piano voice — variants: rhodes (default), wurli, dx.",
    "electric_piano",
    "rhodes",
    "rhodes" => catalog::ElectricPiano::rhodes,
    "wurli" => catalog::ElectricPiano::wurli,
    "dx" => catalog::ElectricPiano::dx,
);
instrument_fn!(
    organ,
    "An organ voice — variants: tonewheel (default), rock.",
    "organ",
    "tonewheel",
    "tonewheel" => catalog::Organ::tonewheel,
    "rock" => catalog::Organ::rock,
);
instrument_fn!(
    strings,
    "A string ensemble voice — variants: warm (default), ensemble.",
    "strings",
    "warm",
    "warm" => catalog::Strings::warm,
    "ensemble" => catalog::Strings::ensemble,
);
instrument_fn!(
    bass,
    "A bass voice — variants: finger (default), pick, sub, synth.",
    "bass",
    "finger",
    "finger" => catalog::Bass::finger,
    "pick" => catalog::Bass::pick,
    "sub" => catalog::Bass::sub,
    "synth" => catalog::Bass::synth,
);
instrument_fn!(
    guitar,
    "A guitar voice — variants: nylon (default), steel, electric.",
    "guitar",
    "nylon",
    "nylon" => catalog::Guitar::nylon,
    "steel" => catalog::Guitar::steel,
    "electric" => catalog::Guitar::electric,
);
instrument_fn!(
    drums,
    "A General MIDI drum kit — variants: acoustic (default), classic, electronic, tr808.",
    "drums",
    "acoustic",
    "acoustic" => catalog::Drums::acoustic,
    "classic" => catalog::Drums::classic,
    "electronic" => catalog::Drums::electronic,
    "tr808" => catalog::Drums::tr808,
);
instrument_fn!(
    brass,
    "A brass voice — variants: section (default), stab.",
    "brass",
    "section",
    "section" => catalog::Brass::section,
    "stab" => catalog::Brass::stab,
);
instrument_fn!(
    flute,
    "A flute voice — variants: concert (default).",
    "flute",
    "concert",
    "concert" => catalog::Flute::concert,
);
instrument_fn!(
    mallets,
    "A mallets voice — variants: marimba (default), vibraphone, glockenspiel.",
    "mallets",
    "marimba",
    "marimba" => catalog::Mallets::marimba,
    "vibraphone" => catalog::Mallets::vibraphone,
    "glockenspiel" => catalog::Mallets::glockenspiel,
);
instrument_fn!(
    bells,
    "A bells voice — variants: tubular (default).",
    "bells",
    "tubular",
    "tubular" => catalog::Bells::tubular,
);

/// Register `Voice` and the `instruments` submodule.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Voice>()?;
    let instruments = PyModule::new(m.py(), "instruments")?;
    instruments.add_function(wrap_pyfunction!(piano, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(electric_piano, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(organ, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(strings, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(bass, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(guitar, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(drums, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(brass, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(flute, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(mallets, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(bells, &instruments)?)?;
    m.add_submodule(&instruments)?;
    Ok(())
}
