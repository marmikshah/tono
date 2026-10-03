//! A curated, asset-free sound library: 64 recipes and six useful voicings.
//!
//! Each entry generates an editable, engine-pinned [`SoundDoc`]. Its named
//! layers are ordinary graph nodes, ready for the renderer or a future visual
//! editor. No recordings, soundfonts, network requests, or audio files are
//! required. Save the returned document to preserve an exact sound.
//!
//! ```
//! use tono_core::library::{LibrarySpec, LibraryVariant, generate};
//! let spec = LibrarySpec::new("metal-clang", LibraryVariant::Soft, 42);
//! let doc = generate(&spec)?;
//! assert!(tono_core::render::render(&doc).iter().any(|x| x.abs() > 0.01));
//! # Ok::<(), anyhow::Error>(())
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dsl::{
    Adsr, Curve, Mode, Modulator, Node, NoiseColor, Playback, SoundDoc, Track, Value,
};
use crate::dsp::Rng;

/// Revision of the curated recipes and their voicings.
pub const LIBRARY_VERSION: u32 = 1;
const LIBRARY_ENGINE: u32 = 5;
const LIBRARY_SCHEMA: u32 = 2;

/// A useful, named starting point for the editable controls in [`LibrarySpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LibraryVariant {
    /// The reference sound.
    Classic,
    /// Rounder tone and a gentler attack.
    Soft,
    /// More upper harmonics and a sharper, shorter attack.
    Bright,
    /// A fifth lower, with a darker and slightly longer body.
    Low,
    /// A fifth higher, with a shorter, lighter body.
    High,
    /// An extended decay or slower movement.
    Long,
}

/// All six voicings, in listening order.
pub const VARIANTS: [LibraryVariant; 6] = [
    LibraryVariant::Classic,
    LibraryVariant::Soft,
    LibraryVariant::Bright,
    LibraryVariant::Low,
    LibraryVariant::High,
    LibraryVariant::Long,
];

impl LibraryVariant {
    /// Stable identifier used in URLs, source filenames, and JSON.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Soft => "soft",
            Self::Bright => "bright",
            Self::Low => "low",
            Self::High => "high",
            Self::Long => "long",
        }
    }

    /// Human-readable voicing name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Soft => "Soft",
            Self::Bright => "Bright",
            Self::Low => "Low",
            Self::High => "High",
            Self::Long => "Long",
        }
    }
}

/// A saved request for one library sound. The controls are explicit so callers
/// can adjust them after choosing a voicing without depending on hidden UI state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibrarySpec {
    /// Required authored recipe revision.
    pub library_version: u32,
    /// Stable recipe identifier; discover these in [`RECIPES`].
    pub recipe: String,
    /// Starting voicing; controls below contain its actual rendered settings.
    pub variant: LibraryVariant,
    /// Seed for coherent pitch/timing changes and synthesis noise.
    pub seed: u64,
    /// Output sample rate, 8000..=192000 Hz.
    pub sample_rate: u32,
    /// Tone brightness / contact hardness, 0..=1.
    pub brightness: f32,
    /// Attack speed and transient emphasis, 0..=1.
    pub punch: f32,
    /// Multiply the nominal duration by 0.25..=3.
    pub duration_scale: f32,
    /// Transpose the sound by -24..=24 semitones.
    pub pitch_semitones: f32,
    /// Seeded parameter variation, 0..=1; zero keeps the authored settings fixed.
    pub variation: f32,
}

impl LibrarySpec {
    /// Create a 48 kHz request using the chosen voicing and restrained variation.
    /// Unknown recipe names are rejected by [`generate`].
    pub fn new(recipe: impl Into<String>, variant: LibraryVariant, seed: u64) -> Self {
        let (brightness, punch, duration_scale, pitch_semitones) = match variant {
            LibraryVariant::Classic => (0.5, 0.5, 1.0, 0.0),
            LibraryVariant::Soft => (0.15, 0.2, 1.05, -2.0),
            LibraryVariant::Bright => (0.85, 0.85, 0.9, 2.0),
            LibraryVariant::Low => (0.35, 0.6, 1.1, -7.0),
            LibraryVariant::High => (0.7, 0.5, 0.85, 7.0),
            LibraryVariant::Long => (0.45, 0.35, 1.65, 0.0),
        };
        Self {
            library_version: LIBRARY_VERSION,
            recipe: recipe.into(),
            variant,
            seed,
            sample_rate: 48_000,
            brightness,
            punch,
            duration_scale,
            pitch_semitones,
            variation: 0.08,
        }
    }

    /// Reject unsupported revisions, unknown recipes, and unbounded controls
    /// before building or rendering a graph.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.library_version == LIBRARY_VERSION,
            "unsupported library version {}; this generator supports {}",
            self.library_version,
            LIBRARY_VERSION
        );
        anyhow::ensure!(
            recipe(&self.recipe).is_some(),
            "unknown library recipe '{}'",
            self.recipe
        );
        anyhow::ensure!(
            (8_000..=192_000).contains(&self.sample_rate),
            "sample_rate must be in [8000, 192000] Hz"
        );
        for (name, value, min, max) in [
            ("brightness", self.brightness, 0.0, 1.0),
            ("punch", self.punch, 0.0, 1.0),
            ("duration_scale", self.duration_scale, 0.25, 3.0),
            ("pitch_semitones", self.pitch_semitones, -24.0, 24.0),
            ("variation", self.variation, 0.0, 1.0),
        ] {
            anyhow::ensure!(
                value.is_finite() && (min..=max).contains(&value),
                "{name} must be finite and in [{min}, {max}]"
            );
        }
        Ok(())
    }
}

/// Discovery metadata for an authored sound. All entries are original
/// procedural recipes licensed with the crate; every voicing is editable.
#[derive(Debug, Clone, Serialize)]
pub struct Recipe {
    /// Stable recipe slug.
    pub id: &'static str,
    /// Name displayed in a library browser.
    pub title: &'static str,
    /// Stable category slug for filtering.
    pub category: &'static str,
    /// The intended sound and its useful context.
    pub description: &'static str,
    /// Search terms describing the timbre and use.
    pub tags: &'static [&'static str],
    /// Nominal raw graph duration before voicing / seeded variation.
    pub duration: f32,
    /// Whether the generated document renders a seamless loop body.
    pub looping: bool,
    #[serde(skip)]
    shape: RecipeShape,
}

#[derive(Debug, Clone, Copy)]
enum Tone {
    Sine,
    Pulse,
    Saw,
    Fm(f32, f32),
}

#[derive(Debug, Clone, Copy)]
enum RecipeShape {
    Phrase {
        freq: f32,
        notes: &'static [f32],
        tone: Tone,
        chord: bool,
    },
    Sweep {
        from: f32,
        to: f32,
        tone: Tone,
        noise: f32,
        attack: f32,
    },
    Hit {
        freq: f32,
        ratios: &'static [f32],
        decay: f32,
        hardness: f32,
        noise: f32,
    },
    Shot {
        color: NoiseColor,
        cutoff: f32,
        body: f32,
        end: f32,
        noise: f32,
        attack: f32,
        pulses: u32,
    },
    Texture {
        color: NoiseColor,
        cutoff: f32,
        density: f32,
        grain: f32,
        hum: f32,
        rate: f32,
    },
}

use RecipeShape::{Hit, Phrase, Shot, Sweep, Texture};
use Tone::{Fm, Pulse, Saw, Sine};

macro_rules! entry {
    ($id:literal, $title:literal, $category:literal, $description:literal, [$($tag:literal),+], $duration:literal, $looping:literal, $shape:expr) => {
        Recipe { id: $id, title: $title, category: $category, description: $description, tags: &[$($tag),+], duration: $duration, looping: $looping, shape: $shape }
    };
}

/// The 64 authored sounds, grouped by category. Six voicings per entry produce
/// 384 reusable sounds before any seed or control variation.
pub static RECIPES: &[Recipe] = &[
    entry!(
        "ui-click",
        "Quiet click",
        "ui",
        "A compact tactile tap for buttons and selection.",
        ["button", "tap", "short"],
        0.065,
        false,
        Shot {
            color: NoiseColor::White,
            cutoff: 1900.0,
            body: 740.0,
            end: 320.0,
            noise: 0.35,
            attack: 0.02,
            pulses: 1
        }
    ),
    entry!(
        "ui-hover",
        "Hover tick",
        "ui",
        "A soft single tone for focus changes and navigation.",
        ["hover", "focus", "minimal"],
        0.095,
        false,
        Phrase {
            freq: 1174.7,
            notes: &[1.0],
            tone: Sine,
            chord: false
        }
    ),
    entry!(
        "ui-toggle-on",
        "Toggle on",
        "ui",
        "Two clean notes stepping upward for enabled state.",
        ["switch", "enable", "ascending"],
        0.19,
        false,
        Phrase {
            freq: 523.25,
            notes: &[1.0, 1.5],
            tone: Sine,
            chord: false
        }
    ),
    entry!(
        "ui-toggle-off",
        "Toggle off",
        "ui",
        "Two low notes falling gently for disabled state.",
        ["switch", "disable", "descending"],
        0.21,
        false,
        Phrase {
            freq: 523.25,
            notes: &[1.0, 0.75],
            tone: Fm(1.0, 0.3),
            chord: false
        }
    ),
    entry!(
        "ui-notification",
        "Glass notification",
        "ui",
        "A rounded bell pair for a new message or reminder.",
        ["message", "bell", "alert"],
        0.65,
        false,
        Phrase {
            freq: 783.99,
            notes: &[1.0, 1.25],
            tone: Fm(2.75, 0.8),
            chord: false
        }
    ),
    entry!(
        "ui-success",
        "Task complete",
        "ui",
        "A warm major triad resolving a successful action.",
        ["success", "complete", "positive"],
        0.48,
        false,
        Phrase {
            freq: 523.25,
            notes: &[1.0, 1.25, 1.5],
            tone: Fm(1.0, 0.45),
            chord: false
        }
    ),
    entry!(
        "ui-warning",
        "Gentle warning",
        "ui",
        "A repeated amber tone for attention without a harsh alarm.",
        ["warning", "attention", "repeat"],
        0.44,
        false,
        Phrase {
            freq: 698.46,
            notes: &[1.0, 1.0],
            tone: Pulse,
            chord: false
        }
    ),
    entry!(
        "ui-error",
        "Action blocked",
        "ui",
        "A muted descending pair for an unavailable action.",
        ["error", "blocked", "negative"],
        0.31,
        false,
        Phrase {
            freq: 392.0,
            notes: &[1.0, 0.84],
            tone: Fm(1.5, 0.65),
            chord: false
        }
    ),
    entry!(
        "coin-pickup",
        "Coin pickup",
        "arcade",
        "A sparkling two-note reward with a quick high finish.",
        ["coin", "pickup", "retro"],
        0.24,
        false,
        Phrase {
            freq: 987.77,
            notes: &[1.0, 2.0],
            tone: Pulse,
            chord: false
        }
    ),
    entry!(
        "double-jump",
        "Double jump",
        "arcade",
        "A springy pulse sweep for a second jump or bounce.",
        ["jump", "bounce", "platformer"],
        0.30,
        false,
        Sweep {
            from: 220.0,
            to: 1080.0,
            tone: Pulse,
            noise: 0.0,
            attack: 0.02
        }
    ),
    entry!(
        "power-up",
        "Power up",
        "arcade",
        "Four rising notes announce an upgrade or new ability.",
        ["upgrade", "reward", "ascending"],
        0.57,
        false,
        Phrase {
            freq: 329.63,
            notes: &[1.0, 1.25, 1.5, 2.0],
            tone: Pulse,
            chord: false
        }
    ),
    entry!(
        "level-up",
        "Level up",
        "arcade",
        "A bright six-note fanfare for a level or achievement.",
        ["level", "achievement", "fanfare"],
        0.90,
        false,
        Phrase {
            freq: 523.25,
            notes: &[1.0, 1.25, 1.5, 2.0, 1.5, 2.0],
            tone: Fm(2.0, 0.55),
            chord: false
        }
    ),
    entry!(
        "extra-life",
        "Extra life",
        "arcade",
        "A playful octave answer for an exceptional reward.",
        ["life", "bonus", "reward"],
        0.68,
        false,
        Phrase {
            freq: 659.25,
            notes: &[1.0, 1.5, 1.25, 2.0],
            tone: Pulse,
            chord: false
        }
    ),
    entry!(
        "game-over",
        "Game over",
        "arcade",
        "A low descending minor phrase for the end of a run.",
        ["defeat", "end", "minor"],
        0.94,
        false,
        Phrase {
            freq: 392.0,
            notes: &[1.0, 0.84, 0.75, 0.5],
            tone: Fm(1.0, 0.7),
            chord: false
        }
    ),
    entry!(
        "checkpoint",
        "Checkpoint",
        "arcade",
        "A clear open fifth that marks saved progress.",
        ["save", "progress", "positive"],
        0.51,
        false,
        Phrase {
            freq: 587.33,
            notes: &[1.0, 1.5, 2.0],
            tone: Sine,
            chord: false
        }
    ),
    entry!(
        "menu-start",
        "Start game",
        "arcade",
        "A compact rising saw phrase to launch a new session.",
        ["start", "menu", "launch"],
        0.42,
        false,
        Phrase {
            freq: 261.63,
            notes: &[1.0, 1.5, 2.0],
            tone: Saw,
            chord: false
        }
    ),
    entry!(
        "laser-light",
        "Light laser",
        "sci-fi",
        "A sharp downward zap for small energy weapons.",
        ["laser", "zap", "weapon"],
        0.19,
        false,
        Sweep {
            from: 1900.0,
            to: 160.0,
            tone: Saw,
            noise: 0.08,
            attack: 0.01
        }
    ),
    entry!(
        "laser-heavy",
        "Heavy laser",
        "sci-fi",
        "A low, gritty energy sweep with a longer body.",
        ["laser", "heavy", "weapon"],
        0.43,
        false,
        Sweep {
            from: 680.0,
            to: 48.0,
            tone: Fm(1.5, 1.7),
            noise: 0.22,
            attack: 0.015
        }
    ),
    entry!(
        "plasma-shot",
        "Plasma shot",
        "sci-fi",
        "A hollow FM burst with a noisy plasma edge.",
        ["plasma", "energy", "burst"],
        0.31,
        false,
        Sweep {
            from: 410.0,
            to: 85.0,
            tone: Fm(3.5, 2.2),
            noise: 0.27,
            attack: 0.015
        }
    ),
    entry!(
        "shield-hit",
        "Shield hit",
        "sci-fi",
        "A bright metallic resonance for a force-field collision.",
        ["shield", "hit", "metallic"],
        0.52,
        false,
        Hit {
            freq: 380.0,
            ratios: &[1.0, 1.43, 2.89, 4.12],
            decay: 0.75,
            hardness: 0.88,
            noise: 0.1
        }
    ),
    entry!(
        "teleport-in",
        "Teleport in",
        "sci-fi",
        "A swelling harmonic rise for a portal arrival.",
        ["teleport", "portal", "rise"],
        0.78,
        false,
        Sweep {
            from: 130.0,
            to: 940.0,
            tone: Fm(2.0, 1.1),
            noise: 0.2,
            attack: 0.28
        }
    ),
    entry!(
        "teleport-out",
        "Teleport out",
        "sci-fi",
        "A high shimmer sinking into a low departure tone.",
        ["teleport", "portal", "fall"],
        0.66,
        false,
        Sweep {
            from: 1100.0,
            to: 100.0,
            tone: Fm(2.7, 0.85),
            noise: 0.14,
            attack: 0.1
        }
    ),
    entry!(
        "scanner-pulse",
        "Scanner pulse",
        "sci-fi",
        "Three precise pings for a scan or targeting lock.",
        ["scanner", "radar", "ping"],
        0.59,
        false,
        Phrase {
            freq: 880.0,
            notes: &[1.0, 1.0, 1.5],
            tone: Sine,
            chord: false
        }
    ),
    entry!(
        "energy-charge",
        "Energy charge",
        "sci-fi",
        "An accelerating electronic rise before a charged action.",
        ["charge", "power", "riser"],
        1.15,
        false,
        Sweep {
            from: 65.0,
            to: 740.0,
            tone: Pulse,
            noise: 0.18,
            attack: 0.42
        }
    ),
    entry!(
        "air-whoosh",
        "Air whoosh",
        "motion",
        "A broad airy sweep for transitions and passing objects.",
        ["whoosh", "transition", "air"],
        0.48,
        false,
        Shot {
            color: NoiseColor::Pink,
            cutoff: 4300.0,
            body: 0.0,
            end: 0.0,
            noise: 1.0,
            attack: 0.27,
            pulses: 1
        }
    ),
    entry!(
        "cloth-swish",
        "Cloth swish",
        "motion",
        "A soft textured swish for clothing and quick movement.",
        ["cloth", "swish", "movement"],
        0.28,
        false,
        Shot {
            color: NoiseColor::Brown,
            cutoff: 2700.0,
            body: 0.0,
            end: 0.0,
            noise: 1.0,
            attack: 0.13,
            pulses: 2
        }
    ),
    entry!(
        "sword-swing",
        "Sword swing",
        "motion",
        "A fast bright air cut followed by a faint metallic tone.",
        ["sword", "swing", "blade"],
        0.24,
        false,
        Sweep {
            from: 840.0,
            to: 230.0,
            tone: Fm(3.1, 0.6),
            noise: 0.8,
            attack: 0.17
        }
    ),
    entry!(
        "fast-pass",
        "Fast pass",
        "motion",
        "A compact falling hiss for fast objects crossing the scene.",
        ["pass", "speed", "whoosh"],
        0.33,
        false,
        Sweep {
            from: 650.0,
            to: 180.0,
            tone: Sine,
            noise: 0.87,
            attack: 0.32
        }
    ),
    entry!(
        "rocket-launch",
        "Rocket launch",
        "motion",
        "A rising low engine body under a noisy launch blast.",
        ["rocket", "launch", "engine"],
        1.20,
        false,
        Sweep {
            from: 46.0,
            to: 185.0,
            tone: Saw,
            noise: 0.68,
            attack: 0.21
        }
    ),
    entry!(
        "servo-rise",
        "Servo rise",
        "motion",
        "A small motor climbing in pitch for robotic movement.",
        ["servo", "robot", "motor"],
        0.46,
        false,
        Sweep {
            from: 150.0,
            to: 520.0,
            tone: Fm(4.0, 0.9),
            noise: 0.12,
            attack: 0.08
        }
    ),
    entry!(
        "wood-knock",
        "Wood knock",
        "impact",
        "A hollow, short wooden contact for props and footsteps.",
        ["wood", "knock", "foley"],
        0.22,
        false,
        Hit {
            freq: 180.0,
            ratios: &[1.0, 2.31, 3.97],
            decay: 0.31,
            hardness: 0.48,
            noise: 0.06
        }
    ),
    entry!(
        "metal-clang",
        "Metal clang",
        "impact",
        "An inharmonic metal strike with a clear ringing tail.",
        ["metal", "clang", "ring"],
        1.05,
        false,
        Hit {
            freq: 290.0,
            ratios: &[1.0, 1.47, 2.09, 3.31, 4.92],
            decay: 0.91,
            hardness: 0.88,
            noise: 0.09
        }
    ),
    entry!(
        "glass-tap",
        "Glass tap",
        "impact",
        "A delicate high glass contact for fragile objects.",
        ["glass", "tap", "delicate"],
        0.62,
        false,
        Hit {
            freq: 970.0,
            ratios: &[1.0, 2.32, 3.94],
            decay: 0.77,
            hardness: 0.95,
            noise: 0.0
        }
    ),
    entry!(
        "ceramic-hit",
        "Ceramic hit",
        "impact",
        "A rounded porcelain clink with a muted body.",
        ["ceramic", "clink", "porcelain"],
        0.39,
        false,
        Hit {
            freq: 520.0,
            ratios: &[1.0, 1.92, 3.15],
            decay: 0.56,
            hardness: 0.7,
            noise: 0.04
        }
    ),
    entry!(
        "stone-hit",
        "Stone hit",
        "impact",
        "A dry irregular stone impact with a gritty transient.",
        ["stone", "rock", "dry"],
        0.28,
        false,
        Hit {
            freq: 115.0,
            ratios: &[1.0, 1.71, 2.87, 4.01],
            decay: 0.28,
            hardness: 0.65,
            noise: 0.24
        }
    ),
    entry!(
        "rubber-bounce",
        "Rubber bounce",
        "impact",
        "A soft low pitch drop for a ball or elastic object.",
        ["rubber", "ball", "bounce"],
        0.25,
        false,
        Sweep {
            from: 230.0,
            to: 68.0,
            tone: Sine,
            noise: 0.08,
            attack: 0.02
        }
    ),
    entry!(
        "heavy-thud",
        "Heavy thud",
        "impact",
        "A deep contact body with a brief dusty top.",
        ["thud", "heavy", "low"],
        0.46,
        false,
        Shot {
            color: NoiseColor::Brown,
            cutoff: 920.0,
            body: 94.0,
            end: 36.0,
            noise: 0.42,
            attack: 0.015,
            pulses: 1
        }
    ),
    entry!(
        "small-explosion",
        "Small explosion",
        "impact",
        "A compact blast of dark noise and dropping sub bass.",
        ["explosion", "blast", "debris"],
        0.82,
        false,
        Shot {
            color: NoiseColor::Pink,
            cutoff: 3200.0,
            body: 125.0,
            end: 32.0,
            noise: 0.77,
            attack: 0.02,
            pulses: 1
        }
    ),
    entry!(
        "key-tap",
        "Key tap",
        "mechanical",
        "A crisp compact key strike for keyboard and keypad actions.",
        ["key", "keyboard", "tap"],
        0.075,
        false,
        Hit {
            freq: 890.0,
            ratios: &[1.0, 1.87, 3.2],
            decay: 0.21,
            hardness: 0.82,
            noise: 0.2
        }
    ),
    entry!(
        "switch-click",
        "Switch click",
        "mechanical",
        "Two dry contacts for a physical toggle switch.",
        ["switch", "click", "contact"],
        0.14,
        false,
        Shot {
            color: NoiseColor::White,
            cutoff: 2800.0,
            body: 460.0,
            end: 230.0,
            noise: 0.52,
            attack: 0.01,
            pulses: 2
        }
    ),
    entry!(
        "latch-close",
        "Latch close",
        "mechanical",
        "A low latch knock followed by a small metallic contact.",
        ["latch", "door", "close"],
        0.23,
        false,
        Hit {
            freq: 220.0,
            ratios: &[1.0, 2.17, 3.52],
            decay: 0.32,
            hardness: 0.72,
            noise: 0.32
        }
    ),
    entry!(
        "camera-shutter",
        "Camera shutter",
        "mechanical",
        "A quick two-stage shutter snap for captures and photos.",
        ["camera", "shutter", "capture"],
        0.19,
        false,
        Shot {
            color: NoiseColor::White,
            cutoff: 4700.0,
            body: 190.0,
            end: 100.0,
            noise: 0.78,
            attack: 0.01,
            pulses: 2
        }
    ),
    entry!(
        "ratchet-turn",
        "Ratchet turn",
        "mechanical",
        "Six short contacts mimic a gear or ratcheting turn.",
        ["ratchet", "gear", "tool"],
        0.52,
        false,
        Shot {
            color: NoiseColor::Pink,
            cutoff: 2200.0,
            body: 720.0,
            end: 430.0,
            noise: 0.56,
            attack: 0.01,
            pulses: 6
        }
    ),
    entry!(
        "electrical-spark",
        "Electrical spark",
        "mechanical",
        "An irregular bright crackle for a short electrical discharge.",
        ["spark", "electric", "crackle"],
        0.33,
        false,
        Texture {
            color: NoiseColor::White,
            cutoff: 6200.0,
            density: 95.0,
            grain: 0.0015,
            hum: 120.0,
            rate: 17.0
        }
    ),
    entry!(
        "soft-wind",
        "Soft wind",
        "ambience",
        "A slowly breathing dark air bed for outdoor scenes.",
        ["wind", "air", "loop"],
        3.2,
        true,
        Texture {
            color: NoiseColor::Pink,
            cutoff: 1500.0,
            density: 0.0,
            grain: 0.01,
            hum: 0.0,
            rate: 0.32
        }
    ),
    entry!(
        "rainfall",
        "Rainfall",
        "ambience",
        "A steady high rain wash with scattered close drops.",
        ["rain", "water", "loop"],
        3.4,
        true,
        Texture {
            color: NoiseColor::White,
            cutoff: 6100.0,
            density: 165.0,
            grain: 0.004,
            hum: 0.0,
            rate: 0.7
        }
    ),
    entry!(
        "fire-crackle",
        "Fire crackle",
        "ambience",
        "A low warm bed and sparse pops for a small fire.",
        ["fire", "crackle", "loop"],
        3.1,
        true,
        Texture {
            color: NoiseColor::Brown,
            cutoff: 2400.0,
            density: 24.0,
            grain: 0.012,
            hum: 0.0,
            rate: 0.58
        }
    ),
    entry!(
        "ocean-surf",
        "Ocean surf",
        "ambience",
        "A broad noise swell evoking a wave washing onto shore.",
        ["ocean", "surf", "water"],
        3.8,
        true,
        Texture {
            color: NoiseColor::Pink,
            cutoff: 3300.0,
            density: 0.0,
            grain: 0.01,
            hum: 0.0,
            rate: 0.22
        }
    ),
    entry!(
        "cave-drone",
        "Cave drone",
        "ambience",
        "A low resonant drone under dark air for cavern scenes.",
        ["cave", "drone", "dark"],
        3.6,
        true,
        Texture {
            color: NoiseColor::Brown,
            cutoff: 540.0,
            density: 1.5,
            grain: 0.08,
            hum: 55.0,
            rate: 0.19
        }
    ),
    entry!(
        "spaceship-hum",
        "Spaceship hum",
        "ambience",
        "A steady mechanical hum for a cabin or engine room.",
        ["spaceship", "engine", "hum"],
        3.0,
        true,
        Texture {
            color: NoiseColor::Pink,
            cutoff: 850.0,
            density: 0.0,
            grain: 0.01,
            hum: 82.41,
            rate: 1.8
        }
    ),
    entry!(
        "night-insects",
        "Night insects",
        "ambience",
        "A stylized high chirring texture for a nocturnal bed.",
        ["night", "insects", "chirp"],
        3.3,
        true,
        Texture {
            color: NoiseColor::White,
            cutoff: 3900.0,
            density: 58.0,
            grain: 0.008,
            hum: 1800.0,
            rate: 9.0
        }
    ),
    entry!(
        "underwater-rumble",
        "Underwater rumble",
        "ambience",
        "A dark low rumble for submerged or pressurized scenes.",
        ["underwater", "rumble", "low"],
        3.5,
        true,
        Texture {
            color: NoiseColor::Brown,
            cutoff: 330.0,
            density: 8.0,
            grain: 0.035,
            hum: 38.0,
            rate: 0.4
        }
    ),
    entry!(
        "kick-tight",
        "Tight kick",
        "percussion",
        "A short pitch-dropping kick with a compact click.",
        ["kick", "drum", "tight"],
        0.26,
        false,
        Shot {
            color: NoiseColor::White,
            cutoff: 2300.0,
            body: 150.0,
            end: 49.0,
            noise: 0.13,
            attack: 0.01,
            pulses: 1
        }
    ),
    entry!(
        "kick-sub",
        "Sub kick",
        "percussion",
        "A longer low electronic kick for bass-heavy rhythms.",
        ["kick", "sub", "electronic"],
        0.55,
        false,
        Shot {
            color: NoiseColor::Pink,
            cutoff: 820.0,
            body: 86.0,
            end: 32.0,
            noise: 0.045,
            attack: 0.008,
            pulses: 1
        }
    ),
    entry!(
        "snare-dry",
        "Dry snare",
        "percussion",
        "A papery noise snap with a short low drum body.",
        ["snare", "drum", "dry"],
        0.25,
        false,
        Shot {
            color: NoiseColor::White,
            cutoff: 6100.0,
            body: 210.0,
            end: 140.0,
            noise: 0.76,
            attack: 0.008,
            pulses: 1
        }
    ),
    entry!(
        "clap-short",
        "Short clap",
        "percussion",
        "Three closely spaced noise contacts finish in a dry clap.",
        ["clap", "hand", "rhythm"],
        0.23,
        false,
        Shot {
            color: NoiseColor::White,
            cutoff: 3500.0,
            body: 0.0,
            end: 0.0,
            noise: 1.0,
            attack: 0.01,
            pulses: 3
        }
    ),
    entry!(
        "hat-closed",
        "Closed hat",
        "percussion",
        "A thin high noise tick for precise rhythmic subdivisions.",
        ["hat", "closed", "short"],
        0.095,
        false,
        Shot {
            color: NoiseColor::White,
            cutoff: 7200.0,
            body: 0.0,
            end: 0.0,
            noise: 1.0,
            attack: 0.01,
            pulses: 1
        }
    ),
    entry!(
        "hat-open",
        "Open hat",
        "percussion",
        "A bright longer cymbal wash with a metallic edge.",
        ["hat", "open", "cymbal"],
        0.51,
        false,
        Hit {
            freq: 1800.0,
            ratios: &[1.0, 1.36, 1.91, 2.71],
            decay: 0.78,
            hardness: 0.94,
            noise: 0.82
        }
    ),
    entry!(
        "shaker",
        "Shaker",
        "percussion",
        "Two softer noise contacts for a small hand shaker.",
        ["shaker", "hand", "rhythm"],
        0.17,
        false,
        Shot {
            color: NoiseColor::Pink,
            cutoff: 5400.0,
            body: 0.0,
            end: 0.0,
            noise: 1.0,
            attack: 0.08,
            pulses: 2
        }
    ),
    entry!(
        "rim-click",
        "Rim click",
        "percussion",
        "A compact hollow rim strike for sparse drum patterns.",
        ["rim", "click", "drum"],
        0.13,
        false,
        Hit {
            freq: 690.0,
            ratios: &[1.0, 1.78, 2.64],
            decay: 0.23,
            hardness: 0.81,
            noise: 0.11
        }
    ),
    entry!(
        "bell-soft",
        "Soft bell",
        "tonal",
        "A single rounded bell for cues and simple musical accents.",
        ["bell", "soft", "musical"],
        1.14,
        false,
        Phrase {
            freq: 659.25,
            notes: &[1.0],
            tone: Fm(3.5, 0.95),
            chord: false
        }
    ),
    entry!(
        "glass-chord",
        "Glass chord",
        "tonal",
        "A simultaneous glassy major chord for reveals and titles.",
        ["chord", "glass", "reveal"],
        1.23,
        false,
        Phrase {
            freq: 392.0,
            notes: &[1.0, 1.25, 1.5, 2.0],
            tone: Fm(2.75, 0.65),
            chord: true
        }
    ),
    entry!(
        "warm-stinger",
        "Warm stinger",
        "tonal",
        "A soft low major chord for a calm transition or scene end.",
        ["stinger", "warm", "transition"],
        1.05,
        false,
        Phrase {
            freq: 220.0,
            notes: &[1.0, 1.25, 1.5],
            tone: Fm(1.0, 0.55),
            chord: true
        }
    ),
    entry!(
        "mystery-chime",
        "Mystery chime",
        "tonal",
        "A sparse minor bell pattern for discoveries and puzzles.",
        ["mystery", "puzzle", "minor"],
        1.27,
        false,
        Phrase {
            freq: 440.0,
            notes: &[1.0, 1.2, 1.5, 2.0],
            tone: Fm(2.41, 0.85),
            chord: false
        }
    ),
];

/// Find one recipe by its stable identifier.
pub fn recipe(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|recipe| recipe.id == id)
}

/// Generate a validated, editable document with named synthesis layers. The
/// recipe topology and intervals stay fixed while controls alter its character.
/// Loops render through the engine's existing equal-power loop crossfade.
pub fn generate(spec: &LibrarySpec) -> anyhow::Result<SoundDoc> {
    spec.validate()?;
    let recipe = recipe(&spec.recipe).expect("validated recipe");
    let mut rng = Rng::new(spec.seed ^ 0xA24B_AED4_963E_E407);
    let pitch = crate::dsp::powf(
        2.0,
        (spec.pitch_semitones + rng.bi() * spec.variation * 1.5) / 12.0,
    );
    let duration = recipe.duration * spec.duration_scale * (1.0 + rng.bi() * spec.variation * 0.08);
    let hz = |freq: f32| (freq * pitch).clamp(12.0, spec.sample_rate as f32 * 0.38);
    let bright = 0.55 + spec.brightness * 0.9;
    let attack = 0.0006 + (1.0 - spec.punch) * 0.004;
    let mut layers = Vec::new();
    match recipe.shape {
        Phrase {
            freq,
            notes,
            tone,
            chord,
        } => {
            let spacing = if chord {
                0.0
            } else {
                duration * 0.61 / notes.len() as f32
            };
            for (index, ratio) in notes.iter().enumerate() {
                let at = index as f32 * spacing;
                let length = if chord {
                    duration * 0.94
                } else {
                    (duration - at) * 0.88
                };
                let level = if chord {
                    0.82 / notes.len() as f32
                } else {
                    0.65 / (1.0 + notes.len() as f32 * 0.14)
                };
                layers.push(layer(
                    &format!("note_{}", index + 1),
                    tone_node(
                        tone,
                        Value::Const(hz(freq * ratio)),
                        bright,
                        spec.sample_rate,
                    ),
                    level,
                    at,
                    length,
                    attack,
                    spec.punch,
                ));
            }
        }
        Sweep {
            from,
            to,
            tone,
            noise,
            attack: swell,
        } => {
            layers.push(layer(
                "body",
                tone_node(
                    tone,
                    slide(hz(from), hz(to), duration * 0.82),
                    bright,
                    spec.sample_rate,
                ),
                0.85 * (1.0 - noise * 0.65),
                0.0,
                duration * 0.95,
                attack + duration * swell,
                spec.punch,
            ));
            if noise > 0.0 {
                let air = filtered_noise(NoiseColor::Pink, hz(3200.0 * bright), 80.0);
                layers.push(layer(
                    "air",
                    air,
                    noise * 0.65,
                    0.0,
                    duration * 0.93,
                    attack + duration * swell * 0.7,
                    spec.punch * 0.3,
                ));
            }
        }
        Hit {
            freq,
            ratios,
            decay,
            hardness,
            noise,
        } => {
            let hardness = (hardness + (spec.brightness - 0.5) * 0.38).clamp(0.12, 0.99);
            let modes = ratios
                .iter()
                .enumerate()
                .map(|(index, ratio)| Mode {
                    freq: hz(freq * ratio),
                    decay: duration * decay / (1.0 + index as f32 * 0.21),
                    gain: 1.0 / (1.0 + index as f32 * 0.8),
                })
                .collect();
            let resonator = chain(
                Node::Impact {
                    hardness,
                    velocity: 0.95,
                },
                vec![Node::Modal { modes, mix: 0.87 }],
            );
            layers.push(layer(
                "resonance",
                resonator,
                1.1,
                0.0,
                duration * 0.96,
                attack * 0.35,
                spec.punch * 0.2,
            ));
            layers.push(layer(
                "contact",
                Node::Sine {
                    freq: slide(hz(freq), hz(freq * 0.87), duration * 0.15),
                },
                0.18,
                0.0,
                duration * 0.32,
                attack,
                spec.punch * 0.35,
            ));
            if noise > 0.0 {
                layers.push(layer(
                    "surface",
                    filtered_noise(NoiseColor::White, hz(4300.0 * bright), 130.0),
                    noise * 0.65,
                    0.0,
                    duration * 0.63,
                    attack,
                    spec.punch,
                ));
            }
        }
        Shot {
            color,
            cutoff,
            body,
            end,
            noise,
            attack: swell,
            pulses,
        } => {
            let spacing = duration
                * if pulses == 3 {
                    0.1
                } else {
                    0.57 / pulses as f32
                };
            for index in 0..pulses {
                let at = index as f32 * spacing;
                let length = if pulses == 3 && index < 2 {
                    duration * 0.16
                } else {
                    (duration - at) * if pulses > 3 { 0.20 } else { 0.91 }
                };
                let highpass = if recipe.category == "percussion" && body == 0.0 {
                    hz(1900.0)
                } else {
                    45.0
                };
                layers.push(layer(
                    &format!("contact_{}", index + 1),
                    filtered_noise(color, hz(cutoff * bright), highpass),
                    noise * if pulses > 1 { 0.67 } else { 0.82 },
                    at,
                    length,
                    attack + length * swell,
                    spec.punch,
                ));
            }
            if body > 0.0 {
                layers.push(layer(
                    "body",
                    Node::Sine {
                        freq: slide(hz(body), hz(end), duration * 0.38),
                    },
                    (1.0 - noise * 0.75) * 0.85,
                    0.0,
                    duration * 0.93,
                    attack,
                    spec.punch,
                ));
            }
        }
        Texture {
            color,
            cutoff,
            density,
            grain,
            hum,
            rate,
        } => {
            let bed = chain(
                filtered_noise(color, hz(cutoff * bright), 30.0),
                vec![Node::Tremolo {
                    rate: (rate / spec.duration_scale).min(40.0),
                    depth: if density > 0.0 { 0.35 } else { 0.7 },
                }],
            );
            let bed_gain = if recipe.id == "night-insects" || recipe.id == "electrical-spark" {
                0.13
            } else {
                0.58
            };
            layers.push(texture_layer(
                "bed",
                bed,
                bed_gain,
                recipe.looping,
                duration,
                attack,
            ));
            if density > 0.0 {
                let grains = chain(
                    Node::Dust {
                        density: density * (0.7 + spec.punch * 0.6),
                        decay: grain * spec.duration_scale,
                    },
                    vec![
                        Node::Highpass {
                            cutoff: Value::Const(hz(if hum > 900.0 { 2200.0 } else { 180.0 })),
                            q: 0.707,
                        },
                        Node::Lowpass {
                            cutoff: Value::Const(hz(cutoff * bright)),
                            q: 0.707,
                        },
                    ],
                );
                layers.push(texture_layer(
                    "particles",
                    grains,
                    0.42,
                    recipe.looping,
                    duration,
                    attack,
                ));
            }
            if hum > 0.0 {
                let motor = chain(
                    Node::Fm {
                        freq: Value::Const(hz(hum)),
                        ratio: 2.0,
                        index: Value::Const(if hum > 900.0 {
                            0.12
                        } else {
                            0.2 + spec.brightness * 0.35
                        }),
                    },
                    vec![Node::Tremolo {
                        rate,
                        depth: if hum > 900.0 { 0.94 } else { 0.22 },
                    }],
                );
                layers.push(texture_layer(
                    "tone",
                    motor,
                    if hum > 900.0 { 0.31 } else { 0.26 },
                    recipe.looping,
                    duration,
                    attack,
                ));
            }
        }
    }
    let root = Node::Tracks {
        tracks: layers,
        master: vec![Node::Gain {
            amount: Value::Const(0.64),
        }],
        buses: Vec::new(),
    };
    let mut doc = SoundDoc::new(format!("{}-{}", recipe.id, spec.variant.id()), root);
    doc.engine = LIBRARY_ENGINE;
    doc.version = LIBRARY_SCHEMA;
    doc.duration = duration;
    doc.sample_rate = spec.sample_rate;
    doc.seed = spec.seed;
    if recipe.looping {
        doc.playback = Playback::Loop {
            start_secs: duration * 0.12,
            end_secs: None,
            crossfade_secs: duration * 0.13,
        };
    }
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

fn tone_node(tone: Tone, frequency: Value, brightness: f32, sample_rate: u32) -> Node {
    match tone {
        Sine => Node::Sine { freq: frequency },
        Pulse => chain(
            Node::Square {
                freq: frequency,
                duty: Value::Const(0.37),
            },
            vec![Node::Lowpass {
                cutoff: Value::Const((3900.0 * brightness).min(sample_rate as f32 * 0.4)),
                q: 0.707,
            }],
        ),
        Saw => chain(
            Node::Sawtooth { freq: frequency },
            vec![Node::Lowpass {
                cutoff: Value::Const((3400.0 * brightness).min(sample_rate as f32 * 0.4)),
                q: 0.707,
            }],
        ),
        Fm(ratio, index) => Node::Fm {
            freq: frequency,
            ratio,
            index: Value::Const(index * brightness),
        },
    }
}

fn chain(source: Node, processors: Vec<Node>) -> Node {
    let mut stages = Vec::with_capacity(processors.len() + 1);
    stages.push(source);
    stages.extend(processors);
    Node::Chain { stages }
}

fn filtered_noise(color: NoiseColor, cutoff: f32, highpass: f32) -> Node {
    chain(
        Node::Noise { color },
        vec![
            Node::Highpass {
                cutoff: Value::Const(highpass.min(cutoff * 0.8)),
                q: 0.707,
            },
            Node::Lowpass {
                cutoff: Value::Const(cutoff),
                q: 0.707,
            },
        ],
    )
}

fn layer(
    id: &str,
    source: Node,
    gain: f32,
    at: f32,
    duration: f32,
    attack: f32,
    punch: f32,
) -> Track {
    let attack = attack.min(duration * 0.6);
    let envelope = Node::Env {
        adsr: Adsr {
            a: attack,
            d: (duration - attack).max(0.001),
            s: 0.0,
            r: 0.0,
            punch: punch * 0.2,
        },
    };
    track(
        id,
        Node::Mul {
            inputs: vec![source, envelope],
        },
        gain,
        at,
    )
}

fn texture_layer(
    id: &str,
    source: Node,
    gain: f32,
    looping: bool,
    duration: f32,
    attack: f32,
) -> Track {
    if looping {
        track(id, source, gain, 0.0)
    } else {
        layer(id, source, gain, 0.0, duration * 0.94, attack, 0.3)
    }
}

fn track(id: &str, node: Node, gain: f32, at: f32) -> Track {
    Track {
        id: Some(id.into()),
        node,
        pan: 0.0,
        gain,
        at,
        mute: false,
        automation: Vec::new(),
        sidechain: None,
        bus: None,
        sends: Vec::new(),
    }
}
