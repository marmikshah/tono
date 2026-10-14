//! `tono.Song` and `tono.Track`: tracks of catalog voices, arranged
//! patterns, buses, automation and maps, compiled to a `Program`. Rust owns
//! the semantics, so an equivalent song compiles to the same Program hash
//! from either language (`crates/tono-core/tests/equivalence.rs` pins it).

use std::collections::HashMap;
use std::sync::Arc;

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};

use tono_core::dsl::bounds::{FADER_GAIN, tempo_map_problems};
use tono_core::dsl::{AutoCurve, AutoTarget, Bus, Node, Send, TempoPoint};
use tono_core::song::{
    CompileOptions, CompileTarget, Marker, MeterPoint, Section, Song as CoreSong, SongLane,
    SongPoint, SongTrack, meter_map_problems,
};
use tono_core::units::Beat;

use crate::convert::{compile_error, py_beat, py_to_json};
use crate::instruments::Voice;
use crate::pattern::Pattern;
use crate::program::Program;

/// Build one bus effect node from a `(type, params)` pair, e.g.
/// `("reverb", {"room": 0.5, "mix": 0.3})`. Unknown or non-processor types
/// are ValueErrors naming the accepted processor types; a known type with bad
/// or unknown params is a ValueError naming the node and its fields.
fn build_effect(kind: &str, params: &Bound<'_, PyAny>) -> PyResult<Node> {
    if !Node::PROCESSOR_KINDS.contains(&kind) {
        return Err(PyValueError::new_err(format!(
            "unknown effect type {kind:?} — bus effects must be processor nodes, one of: {}",
            Node::PROCESSOR_KINDS.join(", ")
        )));
    }
    let dict = params.cast::<PyDict>().map_err(|_| {
        PyTypeError::new_err(format!(
            "effect {kind:?} params must be a dict, e.g. ({kind:?}, {{...}})"
        ))
    })?;
    let mut value = serde_json::Map::new();
    value.insert("type".into(), kind.into());
    for (k, v) in dict.iter() {
        value.insert(k.extract::<String>()?, py_to_json(&v)?);
    }
    let node: Node = serde_json::from_value(serde_json::Value::Object(value))
        .map_err(|e| PyValueError::new_err(format!("effect {kind:?}: {e}")))?;
    // serde fills missing fields with defaults but would also silently drop an
    // unknown param — catch that by round-tripping: every given key must
    // survive the trip through the typed node.
    let round_trip = serde_json::to_value(&node).expect("a node serializes");
    let fields = round_trip
        .as_object()
        .expect("a node serializes to an object");
    let unknown: Vec<String> = dict
        .iter()
        .filter_map(|(k, _)| k.extract::<String>().ok())
        .filter(|k| !fields.contains_key(k))
        .collect();
    if !unknown.is_empty() {
        let accepted: Vec<&str> = fields.keys().map(String::as_str).collect();
        return Err(PyValueError::new_err(format!(
            "effect {kind:?}: unknown param(s) {} — accepted: {}",
            unknown.join(", "),
            accepted.join(", ")
        )));
    }
    Ok(node)
}

/// Resolve a `Track` handle or a track name string to the name (shared by
/// `arrange` and `automate`).
fn track_name_of(track: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(cell) = track.cast::<Track>() {
        Ok(cell.borrow().name.clone())
    } else if let Ok(name) = track.extract::<String>() {
        Ok(name)
    } else {
        Err(PyValueError::new_err(
            "track must be a Track or a track name string",
        ))
    }
}

/// A handle on one of a song's tracks — returned by `Song.track`. Arranging
/// onto it arranges onto the final (slugified, deduplicated) track name, which
/// is also the rendered layer id. The routing methods (`route`, `send`, …)
/// mutate the parent song, which the handle keeps a reference to (one-way —
/// the song never stores handles, so there is no reference cycle).
#[pyclass(module = "tono")]
pub(crate) struct Track {
    name: String,
    song: Py<Song>,
}

#[pymethods]
impl Track {
    /// The track's final name.
    #[getter]
    fn name(&self) -> String {
        self.name.clone()
    }

    /// Route the track's main output to mix bus `bus` (added with
    /// `Song.add_bus`). An unknown bus name is a ValueError listing the
    /// song's buses.
    fn route(&self, py: Python<'_>, bus: &str) -> PyResult<()> {
        let mut song = self.song.borrow_mut(py);
        song.require_bus(bus)?;
        song.track_mut(&self.name).bus = Some(bus.to_string());
        Ok(())
    }

    /// Route the track's main output back to the master bus (the default).
    fn route_master(&self, py: Python<'_>) {
        self.song.borrow_mut(py).track_mut(&self.name).bus = None;
    }

    /// Add a post-fader send from this track to `bus` at `amount` (0..1).
    /// A track sends to a given bus only once — a duplicate target is a
    /// ValueError (remove it with `clear_sends` first).
    #[pyo3(signature = (bus, amount=0.5))]
    fn send(&self, py: Python<'_>, bus: &str, amount: f32) -> PyResult<()> {
        let mut song = self.song.borrow_mut(py);
        song.require_bus(bus)?;
        if !(amount.is_finite() && (0.0..=1.0).contains(&amount)) {
            return Err(PyValueError::new_err(format!(
                "send amount must be in [0, 1], got {amount}"
            )));
        }
        let track = song.track_mut(&self.name);
        if track.sends.iter().any(|s| s.bus == bus) {
            return Err(PyValueError::new_err(format!(
                "track '{}' already sends to bus '{bus}' — clear it with clear_sends, or \
                 adjust the existing send",
                self.name
            )));
        }
        track.sends.push(Send {
            bus: bus.to_string(),
            amount,
        });
        Ok(())
    }

    /// Remove this track's sends: `clear_sends()` removes them all,
    /// `clear_sends("verb")` only the send to that bus.
    #[pyo3(signature = (bus=None))]
    fn clear_sends(&self, py: Python<'_>, bus: Option<&str>) {
        let mut song = self.song.borrow_mut(py);
        let track = song.track_mut(&self.name);
        match bus {
            None => track.sends.clear(),
            Some(b) => track.sends.retain(|s| s.bus != b),
        }
    }

    fn __repr__(&self) -> String {
        format!("Track('{}')", self.name)
    }
}

/// A full song: tracks (catalog voices), patterns (phrases), and an
/// arrangement. `compile` validates and lowers it to an immutable `Program`.
#[pyclass(module = "tono")]
pub(crate) struct Song {
    inner: CoreSong,
    /// Registered patterns, keyed by the Python `id()` of the `Pattern`
    /// object: the first `arrange` of a pattern registers it as `pattern_{n}`
    /// and keeps it alive (so its id can't be recycled) for later placements.
    patterns: HashMap<usize, (Py<Pattern>, String)>,
}

impl Song {
    /// The song track named `name`, mutably — a `Track` handle's name comes
    /// from `Song.track`, so a missing name is a bug, not user error.
    fn track_mut(&mut self, name: &str) -> &mut SongTrack {
        self.inner
            .tracks
            .iter_mut()
            .find(|t| t.name == name)
            .expect("a Track handle outlives its song track")
    }

    /// ValueError unless `bus` names one of the song's buses.
    fn require_bus(&self, bus: &str) -> PyResult<()> {
        if self.inner.buses.iter().any(|b| b.id == bus) {
            return Ok(());
        }
        let known: Vec<&str> = self.inner.buses.iter().map(|b| b.id.as_str()).collect();
        if known.is_empty() {
            Err(PyValueError::new_err(format!(
                "unknown bus {bus:?} — the song has no buses yet (add one with add_bus)"
            )))
        } else {
            Err(PyValueError::new_err(format!(
                "unknown bus {bus:?} — the song's buses are: {}",
                known.join(", ")
            )))
        }
    }
}

#[pymethods]
impl Song {
    /// A song named `name` at `tempo` BPM. `seed` pins the deterministic RNG
    /// stream everything stochastic draws from (None = the document default,
    /// 0): same song + same seed ⇒ same program hash.
    #[new]
    #[pyo3(signature = (name, tempo=120.0, seed=None))]
    fn new(name: &str, tempo: f32, seed: Option<u64>) -> Self {
        let mut inner = CoreSong::new(name, tempo);
        if let Some(seed) = seed {
            inner = inner.with_seed(seed);
        }
        Song {
            inner,
            patterns: HashMap::new(),
        }
    }

    /// The song's name.
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// The tempo in beats per minute.
    #[getter]
    fn tempo(&self) -> f32 {
        self.inner.bpm
    }

    /// The track names, in declaration order.
    #[getter]
    fn track_names(&self) -> Vec<String> {
        self.inner.tracks.iter().map(|t| t.name.clone()).collect()
    }

    /// Add a track playing `voice` under `name` (slugified and deduplicated —
    /// the final name keeps layer ids stable across faces) and return its
    /// `Track` handle. Notes come from the patterns arranged onto it.
    fn track(slf: &Bound<'_, Song>, name: &str, voice: PyRef<'_, Voice>) -> Track {
        let final_name = {
            let mut song = slf.borrow_mut();
            song.inner.add_voice(name, &voice.inner);
            song.inner
                .tracks
                .last()
                .expect("add_voice just pushed a track")
                .name
                .clone()
        };
        Track {
            name: final_name,
            song: slf.clone().unbind(),
        }
    }

    /// Arrange `pattern` onto `track` (a `Track` handle or a track name).
    /// `bars` is an int (place at that one bar) or a range/list of ints
    /// (place at each). The first arrange of a pattern registers it under
    /// `pattern_{n}` (1-based, in registration order). With a meter map or
    /// pickup set, a bar that lands between grid steps is a ValueError here
    /// (rather than a compile error later).
    #[pyo3(signature = (track, pattern, bars=None))]
    fn arrange(
        &mut self,
        track: &Bound<'_, PyAny>,
        pattern: &Bound<'_, Pattern>,
        bars: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let track_name = track_name_of(track)?;
        let bars: Vec<u32> = match bars {
            None => vec![0],
            Some(obj) => {
                if let Ok(bar) = obj.extract::<u32>() {
                    vec![bar]
                } else if let Ok(list) = obj.extract::<Vec<u32>>() {
                    list
                } else {
                    return Err(PyValueError::new_err(
                        "bars must be an int or a range/list of ints",
                    ));
                }
            }
        };

        // With a meter map or pickup, bars move through the exact beat walk —
        // check every placement lands on the grid BEFORE mutating anything
        // (the compiler's T1005 catches maps set after the arrange).
        if !(self.inner.meter_map.is_empty() && self.inner.pickup.is_none()) {
            let spb = i128::from(self.inner.steps_per_beat.max(1));
            for &bar in &bars {
                let beat = self.inner.beat_at_bar(bar);
                if (beat.num as i128 * spb) % i128::from(beat.den) != 0 {
                    return Err(PyValueError::new_err(format!(
                        "bar {bar} lands at beat {beat}, between grid steps ({} steps per \
                         beat) — raise steps_per_beat, or move the placement/meter change \
                         onto the grid",
                        self.inner.steps_per_beat.max(1)
                    )));
                }
            }
        }

        let key = pattern.as_ptr() as usize;
        let registered = match self.patterns.get(&key) {
            Some((_, name)) => name.clone(),
            None => {
                // 1-based count of registered patterns, skipping names a
                // loaded (from_json) song already carries.
                let mut n = self.patterns.len() + 1;
                let mut name = format!("pattern_{n}");
                while self.inner.patterns.iter().any(|p| p.name == name) {
                    n += 1;
                    name = format!("pattern_{n}");
                }
                let (bars_len, notes) = {
                    let p = pattern.borrow();
                    (p.inner.bars, p.inner.notes.clone())
                };
                self.inner.add_pattern(&name, bars_len, notes);
                self.patterns
                    .insert(key, (pattern.clone().unbind(), name.clone()));
                name
            }
        };
        for bar in bars {
            self.inner.arrange(&track_name, &registered, bar);
        }
        Ok(())
    }

    /// Set the tempo map: `[(beat, bpm), ...]` — tempo changes at exact beat
    /// positions (each beat accepts int / float / Fraction / (num, den); a
    /// float is its EXACT binary value, so 0.1 isn't 1/10 — use Fraction for
    /// exact decimals). Validated eagerly against the compiler's rules
    /// (ValueError naming the problem): the first point must sit at beat 0,
    /// beats strictly ascend, tempos are positive and finite, bounded length.
    /// `set_tempo_map([])` clears the map (constant tempo).
    fn set_tempo_map(&mut self, points: Vec<(Bound<'_, PyAny>, f32)>) -> PyResult<()> {
        let mut map = Vec::with_capacity(points.len());
        for (beat, bpm) in &points {
            map.push(TempoPoint {
                at: py_beat(beat)?,
                bpm: *bpm,
            });
        }
        if let Some(p) = tempo_map_problems(&map).into_iter().next() {
            return Err(PyValueError::new_err(format!(
                "{}: {} ({})",
                p.path, p.message, p.remediation
            )));
        }
        self.inner.tempo_map = map;
        Ok(())
    }

    /// Set the meter map: `[(bar, numerator, denominator), ...]` — the time
    /// signature from each bar on (6/8 is `(bar, 6, 8)`). Validated eagerly
    /// against the compiler's rules (ValueError naming the problem): the
    /// first point must be bar 0, bars strictly ascend, numerator ≥ 1,
    /// denominator a power of two ≤ 64, bounded length.
    /// `set_meter_map([])` clears the map (the song's default 4/4).
    fn set_meter_map(&mut self, points: Vec<(u32, u32, u32)>) -> PyResult<()> {
        let map: Vec<MeterPoint> = points
            .iter()
            .map(|&(bar, numerator, denominator)| MeterPoint {
                bar,
                numerator,
                denominator,
            })
            .collect();
        if let Some(p) = meter_map_problems(&map).into_iter().next() {
            return Err(PyValueError::new_err(format!(
                "{}: {} ({})",
                p.path, p.message, p.remediation
            )));
        }
        self.inner.meter_map = map;
        Ok(())
    }

    /// Set the pickup (anacrusis): bar 0's length in beats when it isn't a
    /// full bar (same beat forms as the tempo map — e.g.
    /// `set_pickup(Fraction(1, 2))` for an eighth-note pickup in 4/4).
    /// Negative is a ValueError.
    fn set_pickup(&mut self, beat: &Bound<'_, PyAny>) -> PyResult<()> {
        let beat = py_beat(beat)?;
        if beat < Beat::zero() {
            return Err(PyValueError::new_err("the pickup bar can't be negative"));
        }
        self.inner.pickup = Some(beat);
        Ok(())
    }

    /// Clear the pickup — bar 0 is full length again.
    fn clear_pickup(&mut self) {
        self.inner.pickup = None;
    }

    /// Add a named section (`name` starting at `bar`, `bars` long) — musical
    /// metadata compiled into the Program for the runtime's quantized
    /// transitions. Empty names and zero-length sections are ValueErrors.
    fn add_section(&mut self, name: &str, bar: u32, bars: u32) -> PyResult<()> {
        if name.is_empty() {
            return Err(PyValueError::new_err(
                "a section needs a name (e.g. \"verse\", \"chorus\")",
            ));
        }
        if bars < 1 {
            return Err(PyValueError::new_err("a section must be at least one bar"));
        }
        self.inner.sections.push(Section {
            name: name.to_string(),
            bar,
            bars,
        });
        Ok(())
    }

    /// Add a named marker at an exact beat (same beat forms as the tempo
    /// map) — metadata compiled into the Program. Empty names are ValueErrors.
    fn add_marker(&mut self, name: &str, beat: &Bound<'_, PyAny>) -> PyResult<()> {
        if name.is_empty() {
            return Err(PyValueError::new_err(
                "a marker needs a name (e.g. \"drop\", \"cue\")",
            ));
        }
        self.inner.markers.push(Marker {
            name: name.to_string(),
            at: py_beat(beat)?,
        });
        Ok(())
    }

    /// Add a mix bus: a named submix tracks route to (`Track.route`) and feed
    /// (`Track.send`), with an insert chain of `effects` — a list of
    /// `(type, params)` tuples like `("reverb", {"room": 0.5, "mix": 0.3})`.
    /// The id is a short slug (a-z, 0-9, _), unique and never `"master"`;
    /// `gain` is the return fader (0..2). Unknown/non-processor effect types
    /// and unknown params are ValueErrors naming the accepted forms.
    #[pyo3(signature = (id, gain=1.0, effects=None))]
    fn add_bus(
        &mut self,
        id: &str,
        gain: f32,
        effects: Option<Vec<(String, Bound<'_, PyAny>)>>,
    ) -> PyResult<()> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(PyValueError::new_err(format!(
                "bus ids are short slugs (a-z, 0-9, _), got '{id}'"
            )));
        }
        if id == "master" {
            return Err(PyValueError::new_err(
                "'master' is reserved for the master chain; pick another bus id",
            ));
        }
        if self.inner.buses.iter().any(|b| b.id == id) {
            return Err(PyValueError::new_err(format!(
                "duplicate bus id '{id}' — ids must be unique"
            )));
        }
        if self.inner.tracks.iter().any(|t| t.name == id) {
            return Err(PyValueError::new_err(format!(
                "bus id '{id}' is also a track name — a track and its bus must be named apart"
            )));
        }
        if !(gain.is_finite() && FADER_GAIN.contains(&gain)) {
            return Err(PyValueError::new_err(format!(
                "bus '{id}': gain must be in [0, 2], got {gain}"
            )));
        }
        let mut chain = Vec::new();
        for (kind, params) in effects.unwrap_or_default() {
            chain.push(build_effect(&kind, &params)?);
        }
        self.inner.buses.push(Bus {
            id: id.to_string(),
            gain,
            effects: chain,
        });
        Ok(())
    }

    /// Automate a track's `target` (`"gain"` or `"pan"`) with beat-addressed
    /// breakpoints `[(beat, value), ...]` (floats; compiled to seconds through
    /// the tempo map). `curve` is `"linear"` (default), `"step"`, or `"exp"`.
    /// REPLACES any existing lane for that target on the track; the other
    /// target's lane is kept. An unknown track is a ValueError naming the
    /// song's tracks.
    #[pyo3(signature = (track, target, points, curve="linear"))]
    fn automate(
        &mut self,
        track: &Bound<'_, PyAny>,
        target: &str,
        points: Vec<(f32, f32)>,
        curve: &str,
    ) -> PyResult<()> {
        let name = track_name_of(track)?;
        let target = match target {
            "gain" => AutoTarget::Gain,
            "pan" => AutoTarget::Pan,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown automation target {other:?} — expected 'gain' or 'pan'"
                )));
            }
        };
        let curve = match curve {
            "linear" => AutoCurve::Linear,
            "step" => AutoCurve::Step,
            "exp" => AutoCurve::Exp,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown automation curve {other:?} — expected one of: linear, step, exp"
                )));
            }
        };
        if !self.inner.tracks.iter().any(|t| t.name == name) {
            let known: Vec<&str> = self.inner.tracks.iter().map(|t| t.name.as_str()).collect();
            return Err(PyValueError::new_err(format!(
                "unknown track {name:?} — the song's tracks are: {}",
                known.join(", ")
            )));
        }
        let track = self.track_mut(&name);
        track.automation.retain(|lane| lane.target != target);
        track.automation.push(SongLane {
            target,
            curve,
            points: points
                .into_iter()
                .map(|(at, v)| SongPoint { at, v })
                .collect(),
        });
        Ok(())
    }

    /// Compile the song to an immutable `Program` — validation collects every
    /// problem in one pass and failures raise `tono.CompileError` carrying the
    /// structured `.diagnostics`. `target` is `"offline"` (where streaming
    /// blockers are warnings) or `"runtime"` (where they are errors).
    #[pyo3(signature = (sample_rate=None, target="offline"))]
    fn compile(&self, py: Python<'_>, sample_rate: Option<u32>, target: &str) -> PyResult<Program> {
        let target = match target {
            "offline" => CompileTarget::Offline,
            "runtime" => CompileTarget::Runtime,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown compile target {other:?} — expected 'offline' or 'runtime'"
                )));
            }
        };
        let opts = CompileOptions {
            sample_rate,
            target,
        };
        match py.detach(|| self.inner.compile(&opts)) {
            Ok(program) => Ok(Program {
                inner: Arc::new(program),
            }),
            Err(err) => Err(compile_error(py, err)),
        }
    }

    /// Serialize the song (its saveable project form).
    fn to_json(&self) -> String {
        serde_json::to_string(&self.inner).expect("a song serializes")
    }

    /// Load a song saved with `to_json`.
    #[classmethod]
    fn from_json(_cls: &Bound<'_, PyType>, json: &str) -> PyResult<Self> {
        let inner: CoreSong =
            serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Song {
            inner,
            patterns: HashMap::new(),
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "Song('{}', tempo={:?}, tracks={})",
            self.inner.name,
            self.inner.bpm,
            self.inner.tracks.len()
        )
    }
}

/// Register `Song` and `Track`.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Track>()?;
    m.add_class::<Song>()
}
