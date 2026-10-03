//! Authored game-audio starters with bounded, deterministic variation.
//!
//! Generate an ordinary, engine-pinned [`SoundDoc`], then use the existing
//! renderers, editor, or runtime. Generation performs no I/O and needs no
//! external assets.
//!
//! ```
//! use tono_core::generate::{SfxSpec, SfxTemplate, generate_sfx};
//!
//! let mut spec = SfxSpec::new(SfxTemplate::Laser, 42);
//! spec.brightness = 0.7;
//! let doc = generate_sfx(&spec)?;
//! let samples = tono_core::render::render(&doc);
//! assert!(!samples.is_empty());
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::{fmt, str::FromStr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dsl::{Adsr, Curve, Mode, Modulator, Node, NoiseColor, SoundDoc, Value};
use crate::dsp::Rng;

/// Revision of the authored recipes. Save the generated document for replay
/// across future recipe revisions; unsupported specs are rejected explicitly.
pub const SFX_TEMPLATE_VERSION: u32 = 1;

// The recipe revision includes these pins. A future engine/schema default
// must not silently change the document produced by a saved version-1 spec.
const TEMPLATE_ENGINE: u32 = 5;
const TEMPLATE_SCHEMA: u32 = 2;

/// The eight built-in game sound starters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SfxTemplate {
    /// A bright, rising pickup chime.
    Coin,
    /// An upward arcade pitch sweep.
    Jump,
    /// A descending sci-fi zap.
    Laser,
    /// A noisy blast with a low-frequency body.
    Explosion,
    /// A short resonant strike.
    Impact,
    /// A muted contact thud and surface noise.
    Footstep,
    /// A soft ascending two-tone confirmation.
    UiConfirm,
    /// A soft descending two-tone cancellation.
    UiCancel,
}

impl SfxTemplate {
    /// All starters, in discovery order.
    pub const ALL: [Self; 8] = [
        Self::Coin,
        Self::Jump,
        Self::Laser,
        Self::Explosion,
        Self::Impact,
        Self::Footstep,
        Self::UiConfirm,
        Self::UiCancel,
    ];

    /// Stable identifier used in serialized specs and the CLI.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Coin => "coin",
            Self::Jump => "jump",
            Self::Laser => "laser",
            Self::Explosion => "explosion",
            Self::Impact => "impact",
            Self::Footstep => "footstep",
            Self::UiConfirm => "ui-confirm",
            Self::UiCancel => "ui-cancel",
        }
    }

    /// Short description for template discovery.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Coin => "bright rising pickup chime",
            Self::Jump => "upward arcade sweep",
            Self::Laser => "descending sci-fi zap",
            Self::Explosion => "noise blast with a low body",
            Self::Impact => "short resonant strike",
            Self::Footstep => "muted thud and surface noise",
            Self::UiConfirm => "soft ascending confirmation",
            Self::UiCancel => "soft descending cancellation",
        }
    }

    /// Nominal length in seconds, before seeded timing variation.
    pub const fn duration_secs(self) -> f32 {
        match self {
            Self::Coin => 0.32,
            Self::Jump => 0.26,
            Self::Laser => 0.24,
            Self::Explosion => 0.85,
            Self::Impact => 0.30,
            Self::Footstep => 0.16,
            Self::UiConfirm => 0.20,
            Self::UiCancel => 0.22,
        }
    }
}

impl fmt::Display for SfxTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for SfxTemplate {
    type Err = anyhow::Error;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|template| template.id() == id)
            .ok_or_else(|| {
                anyhow::anyhow!("unknown SFX template '{id}'; list them with tono templates")
            })
    }
}

/// A serializable generation request. Controls are finite values in `0..=1`.
/// The seed selects coherent pitch/timing variation and synthesis noise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SfxSpec {
    /// Recipe revision required to regenerate this request.
    pub template_version: u32,
    /// Sound identity to preserve across candidates.
    pub template: SfxTemplate,
    /// Deterministic generation and synthesis seed.
    pub seed: u64,
    /// Output sample rate, `8000..=192000` Hz.
    pub sample_rate: u32,
    /// Tone height, filter openness, and strike hardness.
    pub brightness: f32,
    /// Attack speed and transient emphasis.
    pub punch: f32,
    /// Amount of seeded pitch/timing variation; zero keeps recipe parameters
    /// fixed, though noise-based sounds still use the synthesis seed.
    pub variation: f32,
}

impl SfxSpec {
    /// Start at 48 kHz, medium brightness/punch, and restrained variation.
    pub fn new(template: SfxTemplate, seed: u64) -> Self {
        Self {
            template_version: SFX_TEMPLATE_VERSION,
            template,
            seed,
            sample_rate: 48_000,
            brightness: 0.5,
            punch: 0.5,
            variation: 0.15,
        }
    }

    /// Reject unsupported revisions, sample rates, and out-of-range controls
    /// before building a graph or allocating audio buffers.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.template_version == SFX_TEMPLATE_VERSION,
            "unsupported SFX template version {}; this generator supports {}",
            self.template_version,
            SFX_TEMPLATE_VERSION
        );
        anyhow::ensure!(
            (8_000..=192_000).contains(&self.sample_rate),
            "sample_rate must be in [8000, 192000] Hz"
        );
        for (name, value) in [
            ("brightness", self.brightness),
            ("punch", self.punch),
            ("variation", self.variation),
        ] {
            anyhow::ensure!(
                value.is_finite() && (0.0..=1.0).contains(&value),
                "{name} must be finite and in [0, 1]"
            );
        }
        Ok(())
    }
}

/// Generate a validated, mono, one-shot document from an authored recipe.
/// Pitch and timing change coherently within a sound; the recipe's topology
/// and interval relationships remain fixed. Save the returned document to
/// retain its exact graph and engine revision independently of the generator.
pub fn generate_sfx(spec: &SfxSpec) -> anyhow::Result<SoundDoc> {
    spec.validate()?;
    let mut rng = Rng::new(spec.seed ^ 0xD1B5_4A32_D192_ED03);
    let pitch = crate::dsp::powf(2.0, rng.bi() * spec.variation * 220.0 / 1200.0)
        * (0.8 + 0.4 * spec.brightness);
    let duration = spec.template.duration_secs() * (1.0 + rng.bi() * spec.variation * 0.2);
    // Keep all oscillator/filter values below Nyquist even at 8 kHz.
    let hz = |value: f32| (value * pitch).min(spec.sample_rate as f32 * 0.42);
    let bright = spec.brightness;
    let punch = spec.punch;
    let source = match spec.template {
        SfxTemplate::Coin => Node::Fm {
            freq: arp(&[hz(1046.5), hz(1569.8)], duration),
            ratio: 2.0,
            index: Value::Const(0.15 + 0.4 * bright),
        },
        SfxTemplate::Jump => Node::Chain {
            stages: vec![
                Node::Square {
                    freq: slide(hz(180.0), hz(740.0), duration * 0.75),
                    duty: Value::Const(0.35),
                },
                lowpass(Value::Const(hz(1600.0 + 1600.0 * bright))),
            ],
        },
        SfxTemplate::Laser => mix(&[
            (
                0.8,
                Node::Sawtooth {
                    freq: slide(hz(1500.0), hz(100.0), duration * 0.85),
                },
            ),
            (0.2, noise(NoiseColor::White, hz(1800.0 + 1200.0 * bright))),
        ]),
        SfxTemplate::Explosion => mix(&[
            (
                0.75,
                Node::Chain {
                    stages: vec![
                        Node::Noise {
                            color: NoiseColor::Pink,
                        },
                        lowpass(slide(
                            hz(1800.0 + 1400.0 * bright),
                            hz(160.0),
                            duration * 0.8,
                        )),
                        Node::Highpass {
                            cutoff: Value::Const(35.0),
                            q: 0.707,
                        },
                    ],
                },
            ),
            (
                0.25,
                Node::Sine {
                    freq: slide(hz(100.0), hz(38.0), duration * 0.6),
                },
            ),
        ]),
        SfxTemplate::Impact => {
            let hardness = 0.2 + 0.75 * bright;
            // Match the fundamental to the exciter's contact time. A high
            // fundamental under a soft, long strike cancels its own ringing.
            let contact_secs = 0.008 * (1.0 - hardness) + 0.0003 * hardness;
            let fundamental = 0.45 / contact_secs;
            Node::Chain {
                stages: vec![
                    Node::Impact {
                        hardness,
                        velocity: 0.85,
                    },
                    Node::Modal {
                        modes: [1.0, 2.37, 4.13]
                            .into_iter()
                            .enumerate()
                            .map(|(i, ratio)| Mode {
                                freq: hz(fundamental * ratio),
                                decay: duration * (0.65 - i as f32 * 0.12),
                                gain: 1.0 / (i as f32 + 1.0),
                            })
                            .collect(),
                        mix: 0.75 - 0.25 * punch,
                    },
                ],
            }
        }
        SfxTemplate::Footstep => mix(&[
            (0.65, noise(NoiseColor::Pink, hz(450.0 + 1700.0 * bright))),
            (
                0.35,
                Node::Sine {
                    freq: slide(hz(125.0), hz(65.0), duration * 0.5),
                },
            ),
        ]),
        SfxTemplate::UiConfirm => mix(&[
            (
                0.85,
                Node::Sine {
                    freq: arp(&[hz(659.3), hz(988.9)], duration),
                },
            ),
            (
                0.15,
                Node::Sine {
                    freq: arp(&[hz(329.65), hz(494.45)], duration),
                },
            ),
        ]),
        SfxTemplate::UiCancel => gain(
            Node::Chain {
                stages: vec![
                    Node::Triangle {
                        freq: arp(&[hz(587.3), hz(440.0)], duration),
                    },
                    // Remove the triangle integrator's startup DC before
                    // shaping the short UI cue.
                    Node::Highpass {
                        cutoff: Value::Const(35.0),
                        q: 0.707,
                    },
                ],
            },
            0.8,
        ),
    };
    let attack = 0.001 + (1.0 - punch) * 0.005;
    // Finish before the buffer endpoint, leaving a short silent margin. The
    // shared outer envelope also catches resonator and filter tails.
    let root = gain(
        Node::Mul {
            inputs: vec![
                source,
                Node::Env {
                    adsr: Adsr {
                        a: attack,
                        d: duration * 0.9 - attack,
                        s: 0.0,
                        r: duration * 0.08,
                        punch: 0.2 + 0.15 * punch,
                    },
                },
            ],
        },
        0.56,
    );
    let mut doc = SoundDoc::new(spec.template.id(), root);
    doc.engine = TEMPLATE_ENGINE;
    doc.version = TEMPLATE_SCHEMA;
    doc.duration = duration;
    doc.sample_rate = spec.sample_rate;
    doc.seed = spec.seed;
    doc.validate()?;
    Ok(doc)
}

fn slide(from: f32, to: f32, secs: f32) -> Value {
    Value::Modulated(Modulator::Slide {
        from,
        to,
        secs,
        curve: Curve::Exp,
    })
}

fn arp(steps: &[f32], duration: f32) -> Value {
    Value::Modulated(Modulator::Arp {
        steps: steps.to_vec(),
        rate: 2.0 / duration,
    })
}

fn lowpass(cutoff: Value) -> Node {
    Node::Lowpass { cutoff, q: 0.707 }
}

fn noise(color: NoiseColor, cutoff: f32) -> Node {
    Node::Chain {
        stages: vec![Node::Noise { color }, lowpass(Value::Const(cutoff))],
    }
}

fn gain(source: Node, amount: f32) -> Node {
    Node::Chain {
        stages: vec![
            source,
            Node::Gain {
                amount: Value::Const(amount),
            },
        ],
    }
}

fn mix(inputs: &[(f32, Node)]) -> Node {
    Node::Mix {
        inputs: inputs
            .iter()
            .map(|(amount, node)| gain(node.clone(), *amount))
            .collect(),
    }
}
