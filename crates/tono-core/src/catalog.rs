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

use crate::dsl::{Adsr, SeqWave};

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
}

/// A short constructor: an instrument with unity gain, centered, no voice params.
fn voice(name: &str, wave: SeqWave, env: Adsr) -> Instrument {
    Instrument {
        name: name.to_string(),
        wave,
        env,
        gain: 1.0,
        pan: 0.0,
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

/// The acoustic **grand piano** — two detuned FM strings per note with a hammer
/// thump, velocity brightness, and a natural pitch-dependent decay (bass rings
/// for seconds, treble dies fast). The flagship voice; play it like a piano and
/// let `len` (note length) hold the key.
pub struct GrandPiano;

impl GrandPiano {
    /// The concert grand — balanced, pedal-length release.
    pub fn grand() -> Instrument {
        voice("grand piano", SeqWave::Piano, piano_env(0.35))
    }

    /// Brighter and more forward — a touch of punch, tighter release. Cuts
    /// through a busy mix.
    pub fn bright() -> Instrument {
        Instrument {
            gain: 1.05,
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

    /// Soft and rounded — quieter, long pedal release for ballads and pads.
    pub fn mellow() -> Instrument {
        voice("mellow piano", SeqWave::Piano, piano_env(0.6)).gain(0.9)
    }

    /// Tight and dry — short release, no pedal. Upright / honky-tonk feel.
    pub fn upright() -> Instrument {
        voice("upright piano", SeqWave::Piano, piano_env(0.12))
    }
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

    /// Picked — more attack punch and a tighter release.
    pub fn pick() -> Instrument {
        voice(
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

    /// A pure **sub** bass — a deep sine, felt more than heard. Keep it mono and
    /// low.
    pub fn sub() -> Instrument {
        voice(
            "sub bass",
            SeqWave::Sine,
            Adsr {
                a: 0.005,
                d: 0.0,
                s: 1.0,
                r: 0.1,
                punch: 0.0,
            },
        )
    }

    /// A raw sawtooth synth bass — bright and buzzy for electronic tracks.
    pub fn synth() -> Instrument {
        voice(
            "synth bass",
            SeqWave::Sawtooth,
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

/// The **guitar** — a Karplus-Strong plucked string: a noise burst rings
/// through a tuned feedback loop. `pluck_decay` sets how long it rings, which is
/// what separates the variants (nylon is warm and short, steel bright, electric
/// sustains).
pub struct Guitar;

impl Guitar {
    /// Nylon-string — warm and short, a soft fingerpicked classical tone.
    pub fn nylon() -> Instrument {
        Instrument {
            voice: VoiceParams {
                pluck_decay: Some(0.90),
                ..VoiceParams::default()
            },
            ..voice("nylon guitar", SeqWave::Pluck, pluck_env(0.25))
        }
    }

    /// Steel-string acoustic — brighter and longer-ringing.
    pub fn steel() -> Instrument {
        Instrument {
            voice: VoiceParams {
                pluck_decay: Some(0.965),
                ..VoiceParams::default()
            },
            ..voice("steel guitar", SeqWave::Pluck, pluck_env(0.35))
        }
    }

    /// Electric — long sustain, the string rings on.
    pub fn electric() -> Instrument {
        Instrument {
            voice: VoiceParams {
                pluck_decay: Some(0.99),
                ..VoiceParams::default()
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
    /// An acoustic kit on the GM map.
    pub fn acoustic() -> Instrument {
        voice(
            "drums",
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
            GrandPiano::upright(),
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
        ];
        for i in &all {
            assert!(!i.name.is_empty());
            assert!(i.gain > 0.0);
        }
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
