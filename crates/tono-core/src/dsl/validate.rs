//! Range and structure validation for a [`SoundDoc`] beyond what serde
//! enforces. Every message is human-readable so a caller can act on it. The
//! numbers live in [`bounds`](super::bounds); this file applies them.

use std::collections::{HashMap, HashSet};

use super::bounds::{
    CEILING_DBTP, EQ_GAIN_DB, FADER_GAIN, HAAS_MS, MAX_CONST_HZ, MAX_DELAY_SECS, MAX_DETUNE_CENTS,
    MAX_DURATION_SECS, MAX_FM_INDEX, MAX_FM_RATIO, MAX_MOD_HZ, MAX_MODAL_MODES, MAX_NODE_DEPTH,
    MAX_RAND_RATE, MAX_SF2_PRESET, MAX_TREMOLO_RATE_HZ, PAN, SAMPLE_RATE_HZ, SUPER_VOICES,
    TARGET_LUFS, tempo_map_problems,
};
use super::{
    Adsr, AutoLane, AutoTarget, Bus, ENGINE_VERSION, Modulator, Node, Playback, SCHEMA_VERSION,
    SeqWave, SoundDoc, Stereo, TempoPoint, Track, Value, note_to_hz,
};

impl Adsr {
    /// Range-check the envelope shape. `what` prefixes error messages
    /// (e.g. `"env"` ⇒ `"env.a must be >= 0"`).
    fn validate(&self, what: &str) -> Result<(), String> {
        for (n, v) in [("a", self.a), ("d", self.d), ("r", self.r)] {
            non_negative(&format!("{what}.{n}"), v)?;
        }
        in_unit(&format!("{what}.s"), self.s)?;
        in_unit(&format!("{what}.punch"), self.punch)
    }
}

/// Why a document failed validation. Wraps the human-readable reason — the
/// same message a caller can use to correct the document — behind a real
/// error type (`Display` + `std::error::Error`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidateError(String);

impl ValidateError {
    /// The human-readable reason.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValidateError {}

impl std::ops::Deref for ValidateError {
    type Target = str;
    /// Deref to the message so callers (and a decade of tests) can treat the
    /// error as the string it carries: `err.contains("freq")`.
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<ValidateError> for String {
    fn from(e: ValidateError) -> String {
        e.0
    }
}

impl SoundDoc {
    /// Validate ranges and structure beyond what serde already enforces.
    /// The error's message is human-readable and names the offending field.
    pub fn validate(&self) -> Result<(), ValidateError> {
        self.validate_inner().map_err(ValidateError)
    }

    fn validate_inner(&self) -> Result<(), String> {
        self.validate_header()?;
        match &self.root {
            Node::Tracks {
                tracks,
                master,
                buses,
            } => self.validate_mixer(tracks, master, buses),
            root => {
                if contains_tracks(root) {
                    return Err(
                        "tracks is the mixing console: it must be the document's root node".into(),
                    );
                }
                // A bare processor as the root has no input and renders
                // digital silence — say so instead of "succeeding".
                if root.is_processor() {
                    return Err(
                        "the root node must be a source (osc/noise/seq/mix/…), not a bare \
                         processor (filter/eq/dynamics/fx) — it would render silence"
                            .into(),
                    );
                }
                validate_node(root)
            }
        }
    }

    /// The document-level fields: the format pins, length, rate, and the
    /// output stage (stereo treatment, normalization, loop playback).
    fn validate_header(&self) -> Result<(), String> {
        let v = self.version;
        if v != SCHEMA_VERSION {
            return Err(format!(
                "unsupported document version {v}; expected {SCHEMA_VERSION}"
            ));
        }
        let e = self.engine;
        if e != ENGINE_VERSION {
            return Err(format!("unsupported engine {e}; expected {ENGINE_VERSION}"));
        }
        if !(self.duration > 0.0 && self.duration <= MAX_DURATION_SECS) {
            return Err(format!(
                "duration must be in (0, {MAX_DURATION_SECS}] seconds, got {}",
                self.duration
            ));
        }
        if !SAMPLE_RATE_HZ.contains(&self.sample_rate) {
            return Err(format!(
                "sample_rate must be in [{}, {}] Hz, got {}",
                SAMPLE_RATE_HZ.start(),
                SAMPLE_RATE_HZ.end(),
                self.sample_rate
            ));
        }
        match self.stereo {
            Stereo::Mono => {}
            Stereo::Haas { ms, pan } => {
                if !HAAS_MS.contains(&ms) {
                    return Err(format!("stereo.haas.ms must be in [0.5, 40], got {ms}"));
                }
                if !PAN.contains(&pan) {
                    return Err(format!("stereo.haas.pan must be in [-1, 1], got {pan}"));
                }
            }
            Stereo::Wide { amount } => in_unit("stereo.wide.amount", amount)?,
        }
        if let Some(nz) = &self.normalize {
            if let Some(t) = nz.target_lufs
                && !TARGET_LUFS.contains(&t)
            {
                return Err(format!(
                    "normalize.target_lufs must be in [-60, 0] LUFS, got {t}"
                ));
            }
            if !CEILING_DBTP.contains(&nz.ceiling_dbtp) {
                return Err(format!(
                    "normalize.ceiling_dbtp must be in [-12, 0] dBTP, got {}",
                    nz.ceiling_dbtp
                ));
            }
        }
        if let Playback::Loop {
            start_secs,
            end_secs,
            crossfade_secs,
        } = self.playback
        {
            if start_secs < 0.0 || start_secs >= self.duration {
                return Err(format!(
                    "playback.loop.start_secs must be in [0, duration), got {start_secs}"
                ));
            }
            if let Some(end) = end_secs {
                if end <= start_secs {
                    return Err(format!(
                        "playback.loop.end_secs ({end}) must be > start_secs ({start_secs})"
                    ));
                }
                if end > self.duration {
                    return Err(format!(
                        "playback.loop.end_secs ({end}) must be <= duration ({})",
                        self.duration
                    ));
                }
            }
            non_negative("playback.loop.crossfade_secs", crossfade_secs)?;
        }
        Ok(())
    }

    /// The mixer: its layers, their sidechain wiring, the master chain, the
    /// buses and the routing between them.
    fn validate_mixer(
        &self,
        tracks: &[Track],
        master: &[Node],
        buses: &[Bus],
    ) -> Result<(), String> {
        if tracks.is_empty() {
            return Err("tracks must be non-empty".into());
        }
        if !matches!(self.stereo, Stereo::Mono) {
            return Err(
                "a tracks document builds its stereo image from per-layer pan — remove the \
                 doc-level stereo treatment (set stereo mode 'mono') and pan the layers \
                 instead"
                    .into(),
            );
        }
        let mut ids = LayerIds::default();
        for (i, t) in tracks.iter().enumerate() {
            validate_layer(i, t, self.duration, &mut ids)?;
        }
        validate_sidechains(tracks)?;
        for (i, m) in master.iter().enumerate() {
            if !m.is_processor() {
                return Err(format!(
                    "master[{i}] must be a processor (filter/eq/dynamics/fx)"
                ));
            }
            validate_node(m)?;
        }
        validate_buses(buses, &ids.ids)?;
        validate_routing(tracks, buses)
    }
}

/// The layer ids seen so far and the RNG stream each hashes to.
#[derive(Default)]
struct LayerIds {
    ids: HashSet<String>,
    streams: HashMap<u64, String>,
}

/// How a layer is named in its error messages: by id when it has one — that
/// is the address the caller used — else by position.
fn layer_name(i: usize, t: &Track) -> String {
    match &t.id {
        Some(id) => format!("layer '{id}'"),
        None => format!("tracks[{i}]"),
    }
}

/// One layer: its id, fader, pan, placement, automation and graph.
fn validate_layer(i: usize, t: &Track, duration: f32, ids: &mut LayerIds) -> Result<(), String> {
    let who = layer_name(i, t);
    if let Some(id) = &t.id {
        validate_layer_id(&who, id, ids)?;
    }
    if !PAN.contains(&t.pan) {
        return Err(format!("{who}: pan must be in [-1, 1], got {}", t.pan));
    }
    if !FADER_GAIN.contains(&t.gain) {
        return Err(format!("{who}: gain must be in [0, 2], got {}", t.gain));
    }
    if !(0.0..duration).contains(&t.at) {
        return Err(format!(
            "{who}: at must be in [0, duration {duration}), got {} — the layer would be \
             entirely outside the render window",
            t.at
        ));
    }
    validate_automation(&who, &t.automation)?;
    if contains_tracks(&t.node) {
        return Err("tracks cannot nest inside a track".into());
    }
    validate_node(&t.node)
}

/// A layer id is a unique slug that is not the master's name and whose RNG
/// stream collides with no other layer's (two layers would otherwise share
/// one noise stream; `u64::MAX` is the master bus's stream).
fn validate_layer_id(who: &str, id: &str, ids: &mut LayerIds) -> Result<(), String> {
    if !is_slug(id) {
        return Err(format!(
            "{who}: layer ids are short slugs (a-z, 0-9, _), got '{id}'"
        ));
    }
    if id == "master" {
        return Err("'master' is reserved for the master chain; pick another layer id".into());
    }
    if !ids.ids.insert(id.to_string()) {
        return Err(format!("duplicate layer id '{id}' — ids must be unique"));
    }
    let key = crate::dsp::layer_stream_key(id);
    if key == u64::MAX {
        return Err(format!(
            "{who}: this id collides with the master bus's RNG stream — rename the layer"
        ));
    }
    if let Some(other) = ids.streams.insert(key, id.to_string()) {
        return Err(format!(
            "layer ids '{other}' and '{id}' hash to the same RNG stream — rename one of them"
        ));
    }
    Ok(())
}

/// At most one lane per target (the renderer applies the first, so a second
/// would be silently dead), with finite non-negative times and values in the
/// target's range.
fn validate_automation(who: &str, automation: &[AutoLane]) -> Result<(), String> {
    let mut seen: Vec<AutoTarget> = Vec::new();
    for lane in automation {
        let (lname, range) = match lane.target {
            AutoTarget::Gain => ("gain", FADER_GAIN),
            AutoTarget::Pan => ("pan", PAN),
        };
        if seen.contains(&lane.target) {
            return Err(format!(
                "{who}: duplicate automation lane for '{lname}' — only the first applies, so \
                 this one would be dead"
            ));
        }
        seen.push(lane.target);
        for (pi, p) in lane.points.iter().enumerate() {
            if !p.t.is_finite() || p.t < 0.0 {
                return Err(format!(
                    "{who}: automation[{lname}].points[{pi}].t must be >= 0 seconds, got {}",
                    p.t
                ));
            }
            if !range.contains(&p.v) {
                return Err(format!(
                    "{who}: automation[{lname}].points[{pi}].v must be in [{}, {}], got {}",
                    range.start(),
                    range.end(),
                    p.v
                ));
            }
        }
    }
    Ok(())
}

/// Sidechain wiring is a cross-track check, so it runs once every id is
/// known (a source may be declared after its follower).
fn validate_sidechains(tracks: &[Track]) -> Result<(), String> {
    for (i, t) in tracks.iter().enumerate() {
        let Some(sc) = &t.sidechain else { continue };
        let who = layer_name(i, t);
        if !(0.0..=1.0).contains(&sc.amount) {
            return Err(format!(
                "{who}: sidechain.amount must be in [0, 1], got {}",
                sc.amount
            ));
        }
        if !sc.attack.is_finite() || sc.attack < 0.0 {
            return Err(format!(
                "{who}: sidechain.attack must be >= 0 seconds, got {}",
                sc.attack
            ));
        }
        if !sc.release.is_finite() || sc.release < 0.0 {
            return Err(format!(
                "{who}: sidechain.release must be >= 0 seconds, got {}",
                sc.release
            ));
        }
        if t.id.as_deref() == Some(sc.source.as_str()) {
            return Err(format!(
                "{who}: sidechain source '{}' is the track itself — a track cannot duck to \
                 its own signal",
                sc.source
            ));
        }
        let Some(source) = tracks
            .iter()
            .find(|s| s.id.as_deref() == Some(sc.source.as_str()))
        else {
            return Err(format!(
                "{who}: sidechain source '{}' is not a layer id in this document — point it \
                 at the track whose signal should drive the duck",
                sc.source
            ));
        };
        if source.sidechain.is_some() {
            return Err(format!(
                "{who}: sidechain source '{}' is itself a follower — follower-of-follower \
                 chains are not supported; duck directly to the source's source",
                sc.source
            ));
        }
    }
    Ok(())
}

/// Bus ids are unique slugs that can't collide with a layer id or the
/// master; effects are processors like the master chain.
fn validate_buses(buses: &[Bus], layer_ids: &HashSet<String>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for b in buses {
        if !is_slug(&b.id) {
            return Err(format!(
                "bus ids are short slugs (a-z, 0-9, _), got '{}'",
                b.id
            ));
        }
        if b.id == "master" {
            return Err("'master' is reserved for the master chain; pick another bus id".into());
        }
        if !seen.insert(b.id.clone()) {
            return Err(format!("duplicate bus id '{}' — ids must be unique", b.id));
        }
        if layer_ids.contains(&b.id) {
            return Err(format!(
                "bus id '{}' is also a layer id — a track and its bus must be named apart",
                b.id
            ));
        }
        if !FADER_GAIN.contains(&b.gain) {
            return Err(format!(
                "bus '{}': gain must be in [0, 2], got {}",
                b.id, b.gain
            ));
        }
        for (i, fx) in b.effects.iter().enumerate() {
            if !fx.is_processor() {
                return Err(format!(
                    "bus '{}' effects[{i}] must be a processor (filter/eq/dynamics/fx)",
                    b.id
                ));
            }
            if contains_tracks(fx) {
                return Err(format!("bus '{}' effects cannot nest tracks", b.id));
            }
            validate_node(fx)?;
        }
    }
    Ok(())
}

/// Every routing reference names a real bus, and a track sends to a bus at
/// most once.
fn validate_routing(tracks: &[Track], buses: &[Bus]) -> Result<(), String> {
    let is_bus = |id: &str| buses.iter().any(|b| b.id == id);
    for (i, t) in tracks.iter().enumerate() {
        let who = layer_name(i, t);
        if let Some(bus) = &t.bus
            && !is_bus(bus)
        {
            return Err(format!(
                "{who}: routed to bus '{bus}', which is not a bus in this document"
            ));
        }
        let mut seen: Vec<&str> = Vec::new();
        for s in &t.sends {
            if !(0.0..=1.0).contains(&s.amount) {
                return Err(format!(
                    "{who}: send to '{}' must have amount in [0, 1], got {}",
                    s.bus, s.amount
                ));
            }
            if !is_bus(&s.bus) {
                return Err(format!(
                    "{who}: sends to bus '{}', which is not a bus in this document",
                    s.bus
                ));
            }
            if seen.contains(&s.bus.as_str()) {
                return Err(format!(
                    "{who}: duplicate send to bus '{}' — raise one send's amount instead",
                    s.bus
                ));
            }
            seen.push(&s.bus);
        }
    }
    Ok(())
}

/// A layer or bus id: non-empty, lowercase ASCII letters, digits and `_`.
fn is_slug(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// True if a `tracks` node appears anywhere in this subtree. Iterative (an
/// explicit stack) so a pathologically deep programmatic document can't
/// overflow the call stack before the depth cap in `validate_node_at` bites.
fn contains_tracks(node: &Node) -> bool {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if matches!(n, Node::Tracks { .. }) {
            return true;
        }
        stack.extend(n.children());
    }
    false
}

/// A finite value (rejects NaN/±inf — they would render NaN audio).
fn finite(name: &str, v: f32) -> Result<(), String> {
    if !v.is_finite() {
        return Err(format!("{name} must be a finite number, got {v}"));
    }
    Ok(())
}

/// A finite, strictly positive value.
fn positive(name: &str, v: f32) -> Result<(), String> {
    finite(name, v)?;
    if v <= 0.0 {
        return Err(format!("{name} must be > 0, got {v}"));
    }
    Ok(())
}

/// A finite, non-negative value.
fn non_negative(name: &str, v: f32) -> Result<(), String> {
    finite(name, v)?;
    if v < 0.0 {
        return Err(format!("{name} must be >= 0, got {v}"));
    }
    Ok(())
}

fn in_unit(name: &str, v: f32) -> Result<(), String> {
    if !(0.0..=1.0).contains(&v) {
        return Err(format!("{name} must be in [0, 1], got {v}"));
    }
    Ok(())
}

impl crate::dsl::FmKnobs {
    fn validate(&self) -> Result<(), String> {
        positive("seq.fm_ratio", self.fm_ratio)?;
        if !(0.0..=MAX_FM_INDEX).contains(&self.fm_index) {
            return Err(format!(
                "seq.fm_index must be in [0, {MAX_FM_INDEX}], got {}",
                self.fm_index
            ));
        }
        positive("seq.fm_strike", self.fm_strike)
    }
}

impl crate::dsl::PluckKnobs {
    fn validate(&self) -> Result<(), String> {
        if !(0.8..1.0).contains(&self.pluck_decay) {
            return Err(format!(
                "seq.pluck_decay must be in [0.8, 1), got {}",
                self.pluck_decay
            ));
        }
        in_unit("seq.pluck_body", self.pluck_body)?;
        in_unit("seq.pluck_pick", self.pluck_pick)?;
        if !(-1.0..=1.0).contains(&self.pluck_tone) {
            return Err(format!(
                "seq.pluck_tone must be in [-1, 1], got {}",
                self.pluck_tone
            ));
        }
        Ok(())
    }
}

impl crate::dsl::PianoKnobs {
    fn validate(&self) -> Result<(), String> {
        positive("seq.piano_hammer", self.piano_hammer)?;
        positive("seq.piano_strike", self.piano_strike)?;
        positive("seq.piano_inharm", self.piano_inharm)?;
        non_negative("seq.piano_detune", self.piano_detune)?;
        positive("seq.piano_decay", self.piano_decay)
    }
}

impl crate::dsl::BassKnobs {
    fn validate(&self) -> Result<(), String> {
        positive("seq.bass_cutoff", self.bass_cutoff)?;
        non_negative("seq.bass_env", self.bass_env)?;
        non_negative("seq.bass_env_vel", self.bass_env_vel)?;
        positive("seq.bass_decay", self.bass_decay)?;
        non_negative("seq.bass_click", self.bass_click)?;
        non_negative("seq.bass_body", self.bass_body)?;
        non_negative("seq.bass_sub", self.bass_sub)?;
        positive("seq.bass_sub_ratio", self.bass_sub_ratio)?;
        in_unit("seq.bass_drive", self.bass_drive)?;
        positive("seq.bass_body_decay", self.bass_body_decay)
    }
}

impl crate::dsl::Sf2Knobs {
    /// Only meaningful when the seq's wave is `sampler` — the caller gates it.
    fn validate(&self) -> Result<(), String> {
        if self.sf2.is_empty() {
            return Err(
                "seq.sf2 must point at a SoundFont (.sf2) file when wave is 'sampler'".into(),
            );
        }
        if self.sf2_preset > MAX_SF2_PRESET {
            return Err(format!(
                "seq.sf2_preset must be in [0, {MAX_SF2_PRESET}], got {}",
                self.sf2_preset
            ));
        }
        Ok(())
    }
}

fn validate_value(v: &Value, what: &str) -> Result<(), String> {
    match v {
        Value::Const(c) => finite(what, *c),
        Value::Note(s) => note_to_hz(s).map(|_| ()).ok_or_else(|| {
            format!("{what}: '{s}' is not a valid note (e.g. \"A4\", \"C#3\", \"midi:69\")")
        }),
        Value::Modulated(m) => match m {
            Modulator::Slide { from, to, secs, .. } => {
                finite(&format!("{what}: slide.from"), *from)?;
                finite(&format!("{what}: slide.to"), *to)?;
                positive(&format!("{what}: slide.secs"), *secs)
            }
            Modulator::Lfo {
                rate,
                depth,
                center,
                ..
            } => {
                positive(&format!("{what}: lfo.rate"), *rate)?;
                finite(&format!("{what}: lfo.depth"), *depth)?;
                finite(&format!("{what}: lfo.center"), *center)
            }
            Modulator::Arp { steps, rate } => {
                if steps.is_empty() {
                    return Err(format!("{what}: arp.steps must be non-empty"));
                }
                for (i, s) in steps.iter().enumerate() {
                    finite(&format!("{what}: arp.steps[{i}]"), *s)?;
                }
                positive(&format!("{what}: arp.rate"), *rate)
            }
            Modulator::EnvMod { adsr, from, to } => {
                finite(&format!("{what}: env.from"), *from)?;
                finite(&format!("{what}: env.to"), *to)?;
                adsr.validate(&format!("{what}: env"))?;
                // The same flatten footgun as Node::Env: the ADSR fields are
                // inlined on the modulator, so an "adsr" object is silently
                // dropped and the parameter would pin at `from` forever.
                if adsr.a == 0.0 && adsr.d == 0.0 && adsr.s == 0.0 && adsr.r == 0.0 {
                    return Err(format!(
                        "{what}: env is constant — a/d/s/r are all 0. The envelope fields \
                         are inlined on the modulator (e.g. {{\"env\":{{\"a\":0.01,\"d\":0.1,\
                         \"s\":0.7,\"r\":0.2,\"from\":..,\"to\":..}}}}); don't nest them \
                         under \"adsr\""
                    ));
                }
                Ok(())
            }
            Modulator::Rand { from, to, rate, .. } => {
                finite(&format!("{what}: rand.from"), *from)?;
                finite(&format!("{what}: rand.to"), *to)?;
                positive(&format!("{what}: rand.rate"), *rate)?;
                if *rate > MAX_RAND_RATE {
                    return Err(format!(
                        "{what}: rand.rate must be in (0, {MAX_RAND_RATE}], got {rate}"
                    ));
                }
                Ok(())
            }
        },
    }
}

/// Validate a `Value` that names a frequency: a constant must be finite and
/// strictly positive (a modulated form is clamped per-sample at render time).
fn validate_freq_value(v: &Value, what: &str) -> Result<(), String> {
    match v {
        Value::Const(c) => {
            positive(what, *c)?;
            if *c > MAX_CONST_HZ {
                return Err(format!("{what} must be <= {MAX_CONST_HZ} Hz, got {c}"));
            }
        }
        // note_to_hz already bounds a resolved note to MAX_CONST_HZ.
        Value::Note(_) => {}
        Value::Modulated(m) => validate_freq_mod(m, what)?,
    }
    validate_value(v, what)
}

/// Bound a modulated frequency's endpoints far below the f32-overflow regime,
/// so products like fm.freq × fm.ratio can never turn oscillator phases NaN.
/// (center/depth are bounded independently, so an LFO's peak can reach 2× —
/// loose, but still orders of magnitude from either danger zone.)
fn validate_freq_mod(m: &Modulator, what: &str) -> Result<(), String> {
    let check = |name: &str, x: f32| {
        if x.abs() > MAX_MOD_HZ {
            Err(format!(
                "{what}: {name} must be within ±{MAX_MOD_HZ} Hz, got {x}"
            ))
        } else {
            Ok(())
        }
    };
    match m {
        Modulator::Slide { from, to, .. } => {
            check("slide.from", *from)?;
            check("slide.to", *to)
        }
        Modulator::Lfo { depth, center, .. } => {
            check("lfo.center", *center)?;
            check("lfo.depth", *depth)
        }
        Modulator::Arp { steps, .. } => {
            for (i, s) in steps.iter().enumerate() {
                check(&format!("arp.steps[{i}]"), *s)?;
            }
            Ok(())
        }
        Modulator::EnvMod { from, to, .. } | Modulator::Rand { from, to, .. } => {
            check("from", *from)?;
            check("to", *to)
        }
    }
}

fn validate_gain_db(name: &str, v: f32) -> Result<(), String> {
    if !(-EQ_GAIN_DB..=EQ_GAIN_DB).contains(&v) {
        return Err(format!(
            "{name} must be in [-{EQ_GAIN_DB}, {EQ_GAIN_DB}] dB, got {v}"
        ));
    }
    Ok(())
}

/// A seq tempo map follows the shared rule ([`tempo_map_problems`]); the
/// sampler plays through one shared synthesizer whose note offs are
/// scheduled in absolute frames, so it keeps its constant-tempo path — a map
/// there is an authoring error.
fn validate_tempo_map(map: &[TempoPoint], wave: SeqWave) -> Result<(), String> {
    if map.is_empty() {
        return Ok(());
    }
    if wave == SeqWave::Sampler {
        return Err("seq.tempo_map is not supported on the sampler wave".into());
    }
    match tempo_map_problems(map).into_iter().next() {
        Some(p) => Err(format!("seq.{}: {}", p.path, p.message)),
        None => Ok(()),
    }
}

/// Validate a `Value` whose constant form must lie in [0, 1] (modulated forms
/// are clamped at render time).
fn validate_unit_value(v: &Value, what: &str) -> Result<(), String> {
    if let Value::Const(c) = v {
        in_unit(what, *c)?;
    }
    validate_value(v, what)
}

fn validate_node(node: &Node) -> Result<(), String> {
    validate_node_at(node, 0)
}

fn validate_node_at(node: &Node, depth: usize) -> Result<(), String> {
    if depth > MAX_NODE_DEPTH {
        return Err(format!(
            "the graph nests deeper than {MAX_NODE_DEPTH} levels — flatten it (e.g. into tracks)"
        ));
    }
    match node {
        Node::Square { freq, duty } => {
            validate_freq_value(freq, "square.freq")?;
            validate_unit_value(duty, "square.duty")
        }
        Node::Triangle { freq } => validate_freq_value(freq, "triangle.freq"),
        Node::Sawtooth { freq } => validate_freq_value(freq, "sawtooth.freq"),
        Node::Sine { freq } => validate_freq_value(freq, "sine.freq"),
        Node::Noise { .. } => Ok(()),
        Node::Impact { hardness, velocity } => {
            in_unit("impact.hardness", *hardness)?;
            in_unit("impact.velocity", *velocity)
        }
        Node::Dust { density, decay } => {
            positive("dust.density", *density)?;
            non_negative("dust.decay", *decay)
        }
        Node::Fm { freq, ratio, index } => {
            validate_freq_value(freq, "fm.freq")?;
            positive("fm.ratio", *ratio)?;
            if *ratio > MAX_FM_RATIO {
                return Err(format!("fm.ratio must be <= {MAX_FM_RATIO}, got {ratio}"));
            }
            validate_value(index, "fm.index")
        }
        // Bound exhaustively (no `..`): the compiler then forces a validation
        // decision for every knob this variant grows.
        Node::Seq {
            bpm,
            tempo_map,
            steps_per_beat,
            wave,
            duty,
            fm,
            pluck,
            piano,
            kit: _,
            bass,
            sf2,
            swing,
            humanize,
            env,
            notes,
        } => {
            positive("seq.bpm", *bpm)?;
            if *steps_per_beat < 1 {
                return Err("seq.steps_per_beat must be >= 1".into());
            }
            validate_tempo_map(tempo_map, *wave)?;
            if notes.is_empty() {
                return Err("seq.notes must be non-empty".into());
            }
            validate_unit_value(duty, "seq.duty")?;
            fm.validate()?;
            pluck.validate()?;
            piano.validate()?;
            bass.validate()?;
            in_unit("seq.swing", *swing)?;
            in_unit("seq.humanize", *humanize)?;
            if *wave == SeqWave::Sampler {
                sf2.validate()?;
            }
            env.validate("seq.env")?;
            for note in notes {
                if note.len < 1 {
                    return Err("seq note.len must be >= 1".into());
                }
                in_unit("seq note.gain", note.gain)?;
                validate_freq_value(&note.pitch, "seq note.pitch")?;
            }
            Ok(())
        }
        Node::Env { adsr } => {
            adsr.validate("env")?;
            // An all-zero envelope is always silent — never intended. It's also
            // the tell-tale of the flatten footgun: the env's a/d/s/r are inlined
            // (`{"type":"env","a":..,"d":..}`), so wrapping them in an `"adsr"`
            // object silently drops them all to 0. Reject it with that hint.
            if adsr.a == 0.0 && adsr.d == 0.0 && adsr.s == 0.0 && adsr.r == 0.0 {
                return Err("env is silent — a/d/s/r are all 0. The envelope fields \
                    are inlined on the node (e.g. {\"type\":\"env\",\"a\":0.01,\"d\":0.1,\
                    \"s\":0.7,\"r\":0.2}); don't nest them under \"adsr\""
                    .into());
            }
            Ok(())
        }
        // Nested mixers are rejected earlier; this guards direct calls.
        Node::Tracks { .. } => Err("tracks must be the document's root node".into()),
        Node::Mix { inputs } | Node::Mul { inputs } => {
            if inputs.is_empty() {
                return Err("mix/mul requires at least one input".into());
            }
            inputs
                .iter()
                .try_for_each(|n| validate_node_at(n, depth + 1))
        }
        Node::Chain { stages } => {
            if stages.is_empty() {
                return Err("chain requires at least one stage".into());
            }
            // A leading processor has no input and renders digital silence —
            // the worst outcome for a sound authored blind. Sources first.
            if stages[0].is_processor() {
                return Err(
                    "chain's first stage must be a source (osc/noise/seq/mix/…), not a \
                     processor (filter/eq/dynamics/fx) — it would render silence"
                        .into(),
                );
            }
            stages
                .iter()
                .try_for_each(|n| validate_node_at(n, depth + 1))
        }
        Node::Lowpass { cutoff, q }
        | Node::Highpass { cutoff, q }
        | Node::Bandpass { cutoff, q }
        | Node::Notch { cutoff, q } => {
            validate_freq_value(cutoff, "filter.cutoff")?;
            positive("filter.q", *q)
        }
        Node::Peak { cutoff, q, gain_db } => {
            validate_freq_value(cutoff, "peak.cutoff")?;
            positive("peak.q", *q)?;
            validate_gain_db("peak.gain_db", *gain_db)
        }
        Node::Lowshelf { cutoff, gain_db } | Node::Highshelf { cutoff, gain_db } => {
            validate_freq_value(cutoff, "shelf.cutoff")?;
            validate_gain_db("shelf.gain_db", *gain_db)
        }
        Node::Super {
            freq,
            voices,
            detune_cents,
            ..
        } => {
            validate_freq_value(freq, "super.freq")?;
            if !SUPER_VOICES.contains(voices) {
                return Err(format!(
                    "super.voices must be in [{}, {}], got {voices}",
                    SUPER_VOICES.start(),
                    SUPER_VOICES.end()
                ));
            }
            if !(0.0..=MAX_DETUNE_CENTS).contains(detune_cents) {
                return Err(format!(
                    "super.detune_cents must be in [0, {MAX_DETUNE_CENTS}], got {detune_cents}"
                ));
            }
            Ok(())
        }
        Node::Gain { amount } => validate_value(amount, "gain.amount"),
        Node::Delay { secs, feedback } => {
            positive("delay.secs", *secs)?;
            if *secs > MAX_DELAY_SECS {
                return Err(format!(
                    "delay.secs must be in (0, {MAX_DELAY_SECS}] seconds, got {secs}"
                ));
            }
            in_unit("delay.feedback", *feedback)
        }
        Node::Reverb { room, mix } => {
            in_unit("reverb.room", *room)?;
            in_unit("reverb.mix", *mix)
        }
        Node::Modal { modes, mix } => {
            if modes.is_empty() {
                return Err("modal.modes must be non-empty".into());
            }
            if modes.len() > MAX_MODAL_MODES {
                return Err(format!(
                    "modal.modes must have at most {MAX_MODAL_MODES} modes, got {}",
                    modes.len()
                ));
            }
            for (i, m) in modes.iter().enumerate() {
                positive(&format!("modal.modes[{i}].freq"), m.freq)?;
                positive(&format!("modal.modes[{i}].decay"), m.decay)?;
                in_unit(&format!("modal.modes[{i}].gain"), m.gain)?;
            }
            in_unit("modal.mix", *mix)
        }
        Node::Drive { amount, .. } => validate_value(amount, "drive.amount"),
        Node::Tremolo { rate, depth } => {
            finite("tremolo.rate", *rate)?;
            if !(0.0..=MAX_TREMOLO_RATE_HZ).contains(rate) {
                return Err(format!(
                    "tremolo.rate must be in [0, {MAX_TREMOLO_RATE_HZ}] Hz, got {rate}"
                ));
            }
            in_unit("tremolo.depth", *depth)
        }
        Node::Chorus { rate, depth, mix } => {
            positive("chorus.rate", *rate)?;
            in_unit("chorus.depth", *depth)?;
            in_unit("chorus.mix", *mix)
        }
        Node::Compress {
            threshold,
            ratio,
            attack,
            release,
            makeup,
        } => {
            finite("compress.threshold", *threshold)?;
            // JSON 1e308 deserializes to f32 inf — finite first, then the bound.
            finite("compress.ratio", *ratio)?;
            if *ratio < 1.0 {
                return Err(format!("compress.ratio must be >= 1, got {ratio}"));
            }
            non_negative("compress.attack", *attack)?;
            non_negative("compress.release", *release)?;
            finite("compress.makeup", *makeup)
        }
    }
}
