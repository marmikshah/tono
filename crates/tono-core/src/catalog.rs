//! catalog — a library of ready-to-play instruments.
//!
//! Each constructor returns an [`Instrument`]: a synthesized voice with a tuned
//! envelope, mixer defaults, and voice parameters — no soundfonts, no files,
//! generated entirely from the graph and byte-identical every render. Hand one
//! to [`Song::add`](crate::song::Song::add) and write its notes on the shared
//! beat timeline.
//!
//! ```
//! use tono_core::catalog::{GrandPiano, Bass, Drums, Guitar};
//! use tono_core::song::Song;
//!
//! let doc = Song::new("demo", 120.0)
//!     .add(GrandPiano::grand(), |t| {
//!         t.at(0.0).note("C4", 1.0).at(1.0).note("E4", 1.0)
//!             .at(2.0).chord(&["C4", "E4", "G4"], 2.0);
//!     })
//!     .add(Bass::finger(), |t| {
//!         t.at(0.0).note("C2", 2.0).at(2.0).note("G1", 2.0);
//!     })
//!     .add(Drums::acoustic(), |t| {
//!         t.at(0.0).kick().at(1.0).snare().at(0.5).hat().at(1.5).hat();
//!     })
//!     .to_doc()
//!     .unwrap(); // a normal, deterministic SoundDoc
//! # assert!(matches!(doc.root, tono_core::dsl::Node::Tracks { .. }));
//! ```
//!
//! Variants (`GrandPiano::bright()`, `Guitar::steel()`, `Bass::sub()`, …) are
//! just alternate constructors. Tweak any instrument further with the builder
//! methods: `.gain(..)`, `.pan(..)`, `.named(..)`.

use serde::{Deserialize, Serialize};

use crate::dsl::{Adsr, KitStyle, SeqWave};

/// Extra per-voice synthesis parameters an instrument sets on its `seq`. Only
/// the fields the chosen `wave` reads matter — the rest are ignored (e.g.
/// `pluck_decay` only affects `Guitar`, `fm_*` only the FM voices). `None`
/// leaves the engine default in place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VoiceParams {
    /// Square/pulse duty, 0..1 (the `Square` voice).
    pub duty: Option<f32>,
    /// FM modulator/carrier frequency ratio (the FM voices).
    pub fm_ratio: Option<f32>,
    /// FM modulation index at the strike — brightness (the FM voices).
    pub fm_index: Option<f32>,
    /// FM strike decay in seconds — how fast the brightness fades (the FM voices).
    pub fm_strike: Option<f32>,
    /// Karplus-Strong feedback decay, 0.8..1 — string ring time (`Guitar`).
    pub pluck_decay: Option<f32>,
    /// Guitar body-resonance depth, 0..1 (`Guitar`).
    pub pluck_body: Option<f32>,
    /// Guitar pick-attack level, 0..1 (`Guitar`).
    pub pluck_pick: Option<f32>,
    /// Guitar string brightness/damping, −1..1 (`Guitar`).
    pub pluck_tone: Option<f32>,
    /// Piano hammer hardness — spectral brightness (the `Piano` voice, engine ≥ 3).
    pub piano_hammer: Option<f32>,
    /// Piano hammer strike position — the spectral comb notch (`Piano`).
    pub piano_strike: Option<f32>,
    /// Piano string-stiffness scale — inharmonic partial stretch (`Piano`).
    pub piano_inharm: Option<f32>,
    /// Piano unison detune width — the two-string beating (`Piano`).
    pub piano_detune: Option<f32>,
    /// Piano ring-time scale (`Piano`).
    pub piano_decay: Option<f32>,
    /// Drum-kit voicing (the `Kit` voice); `None` = the classic kit.
    pub kit: Option<KitStyle>,
    /// Bass filter floor / sweep / body / sub / drive knobs (the `Bass` voice).
    pub bass_cutoff: Option<f32>,
    pub bass_env: Option<f32>,
    pub bass_env_vel: Option<f32>,
    pub bass_decay: Option<f32>,
    pub bass_click: Option<f32>,
    pub bass_body: Option<f32>,
    pub bass_sub: Option<f32>,
    pub bass_sub_ratio: Option<f32>,
    pub bass_drive: Option<f32>,
    pub bass_body_decay: Option<f32>,
}

/// A ready-to-play instrument: a synth voice plus a tuned envelope, mixer
/// defaults, and [`VoiceParams`]. Build one from the catalog constructors
/// ([`GrandPiano`], [`Bass`], [`Drums`], …) and add it to a
/// [`Song`](crate::song::Song).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instrument {
    /// Display name — becomes the song track / rendered layer id.
    pub name: String,
    /// The synthesized voice.
    pub wave: SeqWave,
    /// The per-note amplitude envelope.
    pub env: Adsr,
    /// Channel fader, 0..2 (1 = unity).
    pub gain: f32,
    /// Stereo position, −1 (hard left) .. 1 (hard right).
    pub pan: f32,
    /// Reverb send, 0..1 — wraps the track in a reverb (0 = dry, default).
    pub reverb: f32,
    /// Per-track swing override (0..1); `None` uses the song's swing.
    pub swing: Option<f32>,
    /// Per-track humanize override (0..1); `None` uses the song's humanize.
    pub humanize: Option<f32>,
    /// Voice-specific synthesis parameters.
    pub voice: VoiceParams,
}

impl Instrument {
    /// Rename the instrument (the track / layer id). Handy when a song has two
    /// of the same instrument.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the channel fader, 0..2 (1 = unity).
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Set the stereo position, −1 (hard left) .. 1 (hard right).
    pub fn pan(mut self, pan: f32) -> Self {
        self.pan = pan;
        self
    }

    /// Set the reverb send, 0..1 (0 = dry). Wraps this track in a reverb.
    pub fn reverb(mut self, amount: f32) -> Self {
        self.reverb = amount;
        self
    }

    /// Override the song's swing for this track, 0..1.
    pub fn swing(mut self, swing: f32) -> Self {
        self.swing = Some(swing);
        self
    }

    /// Override the song's humanize for this track, 0..1.
    pub fn humanize(mut self, humanize: f32) -> Self {
        self.humanize = Some(humanize);
        self
    }
}

/// A short constructor: an instrument with unity gain, centered, no voice params.
fn voice(name: &str, wave: SeqWave, env: Adsr) -> Instrument {
    Instrument {
        name: name.to_string(),
        wave,
        env,
        gain: 1.0,
        pan: 0.0,
        reverb: 0.0,
        swing: None,
        humanize: None,
        voice: VoiceParams::default(),
    }
}

/// The piano envelope hint from the `Piano` voice: instant attack, full sustain
/// (the model shapes the natural decay), a short release = the damper falling.
fn piano_env(release: f32) -> Adsr {
    Adsr {
        a: 0.002,
        d: 0.0,
        s: 1.0,
        r: release,
        punch: 0.0,
    }
}

/// The acoustic **grand piano** — an inharmonic additive model (engine 3):
/// stretched partials, each with its own decay (the bright attack mellowing to a
/// warm sustain), a hammer-strike spectrum opened by velocity, over a detuned
/// unison pair. Bass rings for seconds, treble dies fast. The flagship voice;
/// play it like a piano and let `len` (note length) hold the key.
pub struct GrandPiano;

impl GrandPiano {
    /// The concert grand — the reference voice, all tone knobs at default.
    pub fn grand() -> Instrument {
        voice("grand piano", SeqWave::Piano, piano_env(0.35))
    }

    /// Hard-voiced and forward — a harder hammer and a higher strike lift the
    /// upper partials; cuts through a busy mix.
    pub fn bright() -> Instrument {
        Instrument {
            gain: 1.05,
            voice: piano_tone(&[
                (Tone::Hammer, 1.6),
                (Tone::Strike, 0.11),
                (Tone::Decay, 1.05),
            ]),
            ..voice(
                "bright piano",
                SeqWave::Piano,
                Adsr {
                    punch: 0.15,
                    r: 0.22,
                    ..piano_env(0.22)
                },
            )
        }
    }

    /// Soft-voiced ballad grand — a softer hammer rounds off the top over a long
    /// pedal decay. Warm, but still a full grand.
    pub fn mellow() -> Instrument {
        Instrument {
            gain: 0.9,
            voice: piano_tone(&[
                (Tone::Hammer, 0.65),
                (Tone::Strike, 0.14),
                (Tone::Decay, 1.1),
            ]),
            ..voice("mellow piano", SeqWave::Piano, piano_env(0.6))
        }
    }

    /// Intimate felt piano — a blanket over the strings: a very soft hammer
    /// strips the highs to a muffled core, the hammer thump sitting proud.
    pub fn felt() -> Instrument {
        Instrument {
            gain: 0.85,
            voice: piano_tone(&[
                (Tone::Hammer, 0.35),
                (Tone::Strike, 0.16),
                (Tone::Decay, 0.8),
            ]),
            ..voice("felt piano", SeqWave::Piano, piano_env(0.5))
        }
    }

    /// Boxy parlour upright — short, stiff strings stretch the partials sharp
    /// (a metallic jangle); slightly hard, dry, no pedal.
    pub fn upright() -> Instrument {
        Instrument {
            voice: piano_tone(&[
                (Tone::Hammer, 1.15),
                (Tone::Strike, 0.115),
                (Tone::Inharm, 1.6),
                (Tone::Detune, 1.3),
                (Tone::Decay, 0.7),
            ]),
            ..voice("upright piano", SeqWave::Piano, piano_env(0.12))
        }
    }

    /// Tack/bar piano — a wide, deliberately out-of-tune unison warble over
    /// short inharmonic strings and a bright tinny attack. Plinky, fast-decaying.
    pub fn honky_tonk() -> Instrument {
        Instrument {
            voice: piano_tone(&[
                (Tone::Hammer, 1.5),
                (Tone::Strike, 0.11),
                (Tone::Inharm, 1.7),
                (Tone::Detune, 12.0),
                (Tone::Decay, 0.65),
            ]),
            ..voice("honky-tonk piano", SeqWave::Piano, piano_env(0.12))
        }
    }
}

/// Which piano tone knob a [`piano_tone`] entry sets.
enum Tone {
    Hammer,
    Strike,
    Inharm,
    Detune,
    Decay,
}

/// Build [`VoiceParams`] for a piano variant from a list of (knob, value) pairs;
/// unlisted knobs stay `None` (the concert-grand default).
fn piano_tone(knobs: &[(Tone, f32)]) -> VoiceParams {
    let mut v = VoiceParams::default();
    for (knob, value) in knobs {
        match knob {
            Tone::Hammer => v.piano_hammer = Some(*value),
            Tone::Strike => v.piano_strike = Some(*value),
            Tone::Inharm => v.piano_inharm = Some(*value),
            Tone::Detune => v.piano_detune = Some(*value),
            Tone::Decay => v.piano_decay = Some(*value),
        }
    }
    v
}

/// The **electric piano** — a Rhodes-style tine voice: soft FM body plus a
/// bright tine that pings on the attack and fades. Velocity opens the tine.
pub struct ElectricPiano;

impl ElectricPiano {
    /// Classic Rhodes — warm body, bell-like tine.
    pub fn rhodes() -> Instrument {
        voice(
            "electric piano",
            SeqWave::Epiano,
            Adsr {
                a: 0.002,
                d: 0.0,
                s: 0.7,
                r: 0.3,
                punch: 0.0,
            },
        )
    }

    /// Barkier, shorter — a Wurlitzer-ish reed with quicker decay.
    pub fn wurli() -> Instrument {
        voice(
            "wurli",
            SeqWave::Epiano,
            Adsr {
                a: 0.002,
                d: 0.0,
                s: 0.55,
                r: 0.18,
                punch: 0.1,
            },
        )
    }

    /// A glassy DX-style FM electric piano — the two-operator FM voice tuned to
    /// a 1:1 ratio with a short bright strike.
    pub fn dx() -> Instrument {
        Instrument {
            voice: VoiceParams {
                fm_ratio: Some(1.0),
                fm_index: Some(3.5),
                fm_strike: Some(0.4),
                ..VoiceParams::default()
            },
            ..voice(
                "dx piano",
                SeqWave::Fm,
                Adsr {
                    a: 0.002,
                    d: 0.0,
                    s: 0.6,
                    r: 0.3,
                    punch: 0.0,
                },
            )
        }
    }
}

/// The **organ** — a tonewheel drawbar voice that sustains at full level while
/// the key is held; `len` does the phrasing.
pub struct Organ;

impl Organ {
    /// Full drawbars, gentle key-click percussion.
    pub fn tonewheel() -> Instrument {
        voice(
            "organ",
            SeqWave::Organ,
            Adsr {
                a: 0.005,
                d: 0.0,
                s: 1.0,
                r: 0.08,
                punch: 0.0,
            },
        )
    }

    /// Punchier attack for rock stabs.
    pub fn rock() -> Instrument {
        voice(
            "rock organ",
            SeqWave::Organ,
            Adsr {
                a: 0.002,
                d: 0.0,
                s: 1.0,
                r: 0.05,
                punch: 0.2,
            },
        )
        .gain(1.05)
    }
}

/// The **string ensemble** — three detuned band-limited saws per note with a
/// slow bow swell. Notes bloom ~150 ms after the attack, so write them slightly
/// early.
pub struct Strings;

impl Strings {
    /// A balanced ensemble swell — pads, sustained chords.
    pub fn ensemble() -> Instrument {
        voice(
            "strings",
            SeqWave::Strings,
            Adsr {
                a: 0.15,
                d: 0.0,
                s: 1.0,
                r: 0.4,
                punch: 0.0,
            },
        )
    }

    /// Warmer and slower — a longer swell and tail for cinematic beds.
    pub fn warm() -> Instrument {
        voice(
            "warm strings",
            SeqWave::Strings,
            Adsr {
                a: 0.3,
                d: 0.0,
                s: 1.0,
                r: 0.7,
                punch: 0.0,
            },
        )
        .gain(0.95)
    }
}

/// The **bass** — the low end. Variants trade the voice under the hood.
pub struct Bass;

impl Bass {
    /// Fingered electric bass — a filtered saw whose cutoff snaps with velocity
    /// over a solid sine sub. Punchy, dark, sits under the mix.
    pub fn finger() -> Instrument {
        voice(
            "bass",
            SeqWave::Bass,
            Adsr {
                a: 0.005,
                d: 0.1,
                s: 0.9,
                r: 0.12,
                punch: 0.0,
            },
        )
    }

    /// Picked — a bright plectrum tick, a fast-closing filter, a touch of grit.
    pub fn pick() -> Instrument {
        Instrument {
            voice: bass_tone(
                300.0, 800.0, 1200.0, 0.08, 2500.0, 0.75, 0.40, 1.0, 0.05, 1.6,
            ),
            ..voice(
                "pick bass",
                SeqWave::Bass,
                Adsr {
                    a: 0.002,
                    d: 0.08,
                    s: 0.85,
                    r: 0.08,
                    punch: 0.25,
                },
            )
        }
    }

    /// A pure **sub** bass — a deep sine with a whisper of body so it still
    /// translates on small speakers; long hold, felt more than heard.
    pub fn sub() -> Instrument {
        Instrument {
            voice: bass_tone(100.0, 150.0, 200.0, 0.25, 0.0, 0.22, 0.95, 1.0, 0.0, 4.0),
            ..voice(
                "sub bass",
                SeqWave::Bass,
                Adsr {
                    a: 0.005,
                    d: 0.0,
                    s: 1.0,
                    r: 0.1,
                    punch: 0.0,
                },
            )
        }
    }

    /// A bright, wide, tanh-driven synth bass over a fat octave-down sub — the
    /// modern electronic/EDM voice.
    pub fn synth() -> Instrument {
        Instrument {
            voice: bass_tone(600.0, 1500.0, 800.0, 0.25, 0.0, 0.85, 0.35, 0.5, 0.35, 6.0),
            ..voice(
                "synth bass",
                SeqWave::Bass,
                Adsr {
                    a: 0.003,
                    d: 0.06,
                    s: 0.8,
                    r: 0.08,
                    punch: 0.1,
                },
            )
        }
    }
}

/// Build [`VoiceParams`] for a bass variant (all ten `bass_*` knobs).
#[allow(clippy::too_many_arguments)]
fn bass_tone(
    cutoff: f32,
    env: f32,
    env_vel: f32,
    decay: f32,
    click: f32,
    body: f32,
    sub: f32,
    sub_ratio: f32,
    drive: f32,
    body_decay: f32,
) -> VoiceParams {
    VoiceParams {
        bass_cutoff: Some(cutoff),
        bass_env: Some(env),
        bass_env_vel: Some(env_vel),
        bass_decay: Some(decay),
        bass_click: Some(click),
        bass_body: Some(body),
        bass_sub: Some(sub),
        bass_sub_ratio: Some(sub_ratio),
        bass_drive: Some(drive),
        bass_body_decay: Some(body_decay),
        ..VoiceParams::default()
    }
}

/// The **guitar** — a Karplus-Strong plucked string: a noise burst rings
/// through a tuned feedback loop. `pluck_decay` sets how long it rings, which is
/// what separates the variants (nylon is warm and short, steel bright, electric
/// sustains).
pub struct Guitar;

impl Guitar {
    /// Nylon-string — warm and short: a dark loop, a big woody body, barely any
    /// pick attack. A soft fingerpicked classical tone.
    pub fn nylon() -> Instrument {
        Instrument {
            voice: VoiceParams {
                pluck_decay: Some(0.90),
                pluck_tone: Some(-0.35),
                pluck_body: Some(0.55),
                pluck_pick: Some(0.05),
                ..VoiceParams::default()
            },
            ..voice("nylon guitar", SeqWave::Pluck, pluck_env(0.25))
        }
    }

    /// Steel-string acoustic — a sizzly bright loop, a present body, a clear pick
    /// attack. Longer-ringing.
    pub fn steel() -> Instrument {
        Instrument {
            voice: VoiceParams {
                pluck_decay: Some(0.965),
                pluck_tone: Some(0.30),
                pluck_body: Some(0.45),
                pluck_pick: Some(0.30),
                ..VoiceParams::default()
            },
            ..voice("steel guitar", SeqWave::Pluck, pluck_env(0.35))
        }
    }

    /// Electric — the brightest loop, no acoustic body, a pick click, long clean
    /// sustain. The solid-body pickup tone.
    pub fn electric() -> Instrument {
        Instrument {
            voice: VoiceParams {
                pluck_decay: Some(0.99),
                pluck_tone: Some(0.45),
                pluck_pick: Some(0.25),
                ..VoiceParams::default() // pluck_body None ⇒ 0.0 = solid body
            },
            ..voice("electric guitar", SeqWave::Pluck, pluck_env(0.5))
        }
    }
}

/// The pluck envelope: near-instant attack, full sustain (the string decays on
/// its own via feedback), a short release when the note ends.
fn pluck_env(release: f32) -> Adsr {
    Adsr {
        a: 0.001,
        d: 0.0,
        s: 1.0,
        r: release,
        punch: 0.0,
    }
}

/// The **drum kit** — the General MIDI drum map. Hit drums by name from the
/// track writer (`.kick()`, `.snare()`, `.hat()`, …) or by GM note.
pub struct Drums;

impl Drums {
    /// A deeper, more realistic acoustic kit — the go-to.
    pub fn acoustic() -> Instrument {
        drum_kit("drums", KitStyle::Acoustic)
    }

    /// The original synthesized GM kit (byte-frozen).
    pub fn classic() -> Instrument {
        Instrument {
            voice: VoiceParams::default(), // no kit key ⇒ classic, unchanged
            ..drum_kit("classic drums", KitStyle::Classic)
        }
    }

    /// Clean synthesized electronic drums — tight, punchy, crisp.
    pub fn electronic() -> Instrument {
        drum_kit("electronic drums", KitStyle::Electronic)
    }

    /// Roland TR-808 style — a long booming sub kick and ringy cowbell.
    pub fn tr808() -> Instrument {
        drum_kit("808 drums", KitStyle::Eight08)
    }
}

/// A drum kit on the GM map with the given [`KitStyle`].
fn drum_kit(name: &str, style: KitStyle) -> Instrument {
    Instrument {
        voice: VoiceParams {
            kit: Some(style),
            ..VoiceParams::default()
        },
        ..voice(
            name,
            SeqWave::Kit,
            Adsr {
                a: 0.001,
                d: 0.0,
                s: 1.0,
                r: 0.05,
                punch: 0.0,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_constructor_builds_a_distinct_named_instrument() {
        let all = [
            GrandPiano::grand(),
            GrandPiano::bright(),
            GrandPiano::mellow(),
            GrandPiano::felt(),
            GrandPiano::upright(),
            GrandPiano::honky_tonk(),
            ElectricPiano::rhodes(),
            ElectricPiano::wurli(),
            ElectricPiano::dx(),
            Organ::tonewheel(),
            Organ::rock(),
            Strings::ensemble(),
            Strings::warm(),
            Bass::finger(),
            Bass::pick(),
            Bass::sub(),
            Bass::synth(),
            Guitar::nylon(),
            Guitar::steel(),
            Guitar::electric(),
            Drums::acoustic(),
            Drums::classic(),
            Drums::electronic(),
            Drums::tr808(),
        ];
        for i in &all {
            assert!(!i.name.is_empty());
            assert!(i.gain > 0.0);
        }
    }

    #[test]
    fn grand_is_the_default_voice_and_variants_set_tone_knobs() {
        // The flagship grand leaves every knob at default (byte-identical to the
        // bare engine-3 piano); the variants each set a distinct spectrum.
        assert_eq!(GrandPiano::grand().voice, VoiceParams::default());
        assert_eq!(GrandPiano::bright().voice.piano_hammer, Some(1.6));
        assert!(
            GrandPiano::felt().voice.piano_hammer.unwrap() < 0.5,
            "felt is soft"
        );
        assert_eq!(GrandPiano::honky_tonk().voice.piano_detune, Some(12.0));
        assert_eq!(GrandPiano::upright().voice.piano_inharm, Some(1.6));
    }

    #[test]
    fn bass_variants_set_tone_and_finger_is_the_default_voice() {
        assert_eq!(Bass::finger().voice, VoiceParams::default());
        assert_eq!(Bass::pick().voice.bass_click, Some(2500.0)); // plectrum tick
        assert_eq!(Bass::synth().voice.bass_sub_ratio, Some(0.5)); // octave-down sub
        assert_eq!(Bass::sub().wave, SeqWave::Bass); // sub now uses the Bass voice
    }

    #[test]
    fn guitar_variants_set_body_and_tone() {
        assert_eq!(Guitar::nylon().voice.pluck_body, Some(0.55));
        assert!(
            Guitar::nylon().voice.pluck_tone.unwrap() < 0.0,
            "nylon is dark"
        );
        assert!(
            Guitar::steel().voice.pluck_tone.unwrap() > 0.0,
            "steel is bright"
        );
        assert_eq!(
            Guitar::electric().voice.pluck_body,
            None,
            "electric = solid body"
        );
    }

    #[test]
    fn guitar_variants_differ_in_ring_time() {
        let n = Guitar::nylon().voice.pluck_decay.unwrap();
        let s = Guitar::steel().voice.pluck_decay.unwrap();
        let e = Guitar::electric().voice.pluck_decay.unwrap();
        assert!(n < s && s < e, "nylon rings shortest, electric longest");
    }

    #[test]
    fn builder_methods_override_defaults() {
        let i = Bass::finger().named("low end").gain(0.8).pan(-0.3);
        assert_eq!(i.name, "low end");
        assert_eq!(i.gain, 0.8);
        assert_eq!(i.pan, -0.3);
    }

    #[test]
    fn round_trips_through_serde() {
        let i = Guitar::steel();
        let json = serde_json::to_string(&i).unwrap();
        let back: Instrument = serde_json::from_str(&json).unwrap();
        assert_eq!(i, back);
    }
}
