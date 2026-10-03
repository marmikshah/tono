//! Original, asset-free background music: twelve themes in three arrangements.
//!
//! Each theme has a composed melody, four-bar chord progression, bass and
//! rhythm. [`score`] returns its editable [`Song`]; [`generate`] compiles a
//! warmed-up, seamless [`SoundDoc`] with an exact four-bar loop period.
//! No recordings or soundfonts are required.
//!
//! ```
//! use tono_core::bgm::{BgmSpec, BgmVariant, generate, score};
//! let spec = BgmSpec::new("lantern-trail", BgmVariant::Calm, 42);
//! let song = score(&spec)?;
//! let loop_doc = generate(&spec)?;
//! assert_eq!(song.bpm, 88.0);
//! # Ok::<(), anyhow::Error>(())
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    catalog::{Bass, Drums, ElectricPiano, Flute, GrandPiano, Guitar, Mallets, Voice, VoiceParams},
    dsl::{Adsr, Node, Playback, SeqNote, SeqWave, SoundDoc, Value},
    dsp::Rng,
    song::{Song, note_vel},
};

/// Revision of the compositions, arrangements, and loop construction.
pub const BGM_VERSION: u32 = 1;
const BGM_ENGINE: u32 = 5;
const BGM_SCHEMA: u32 = 2;
const LOOP_BARS: u32 = 4;
const STEPS_PER_BAR: u32 = 16;

/// An arrangement changes instrumentation and rhythmic density while keeping
/// the theme's melody, harmony, key, tempo, and four-bar period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BgmVariant {
    /// The authored reference arrangement.
    Original,
    /// Softer lead, sustained harmony, half-time bass, and light percussion.
    Calm,
    /// Brighter lead, moving bass, fast arpeggio, and a fuller beat.
    Drive,
}

/// The three available musical arrangements.
pub const VARIANTS: [BgmVariant; 3] = [BgmVariant::Original, BgmVariant::Calm, BgmVariant::Drive];

impl BgmVariant {
    /// Stable identifier used in filenames and serialized requests.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Calm => "calm",
            Self::Drive => "drive",
        }
    }

    /// Display name of the arrangement.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Calm => "Calm",
            Self::Drive => "Drive",
        }
    }
}

/// A reproducible request for one composition and arrangement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BgmSpec {
    /// Required composition revision.
    pub bgm_version: u32,
    /// Stable theme identifier from [`COMPOSITIONS`].
    pub composition: String,
    /// Instrumentation and rhythmic density.
    pub variant: BgmVariant,
    /// Seed for small performance-velocity differences and synthesis noise.
    pub seed: u64,
    /// Output sample rate, 8000..=192000 Hz; browser defaults use 48000 Hz.
    pub sample_rate: u32,
}

impl BgmSpec {
    /// Choose a composition and arrangement at 48 kHz.
    pub fn new(composition: impl Into<String>, variant: BgmVariant, seed: u64) -> Self {
        Self {
            bgm_version: BGM_VERSION,
            composition: composition.into(),
            variant,
            seed,
            sample_rate: 48_000,
        }
    }

    /// Reject unknown compositions, unsupported versions, and invalid rates.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.bgm_version == BGM_VERSION,
            "unsupported BGM version {}; this generator supports {}",
            self.bgm_version,
            BGM_VERSION
        );
        anyhow::ensure!(
            composition(&self.composition).is_some(),
            "unknown BGM composition '{}'",
            self.composition
        );
        anyhow::ensure!(
            (8_000..=192_000).contains(&self.sample_rate),
            "sample_rate must be in [8000, 192000] Hz"
        );
        Ok(())
    }
}

/// Discovery metadata for a composed four-bar musical theme.
#[derive(Debug, Clone, Serialize)]
pub struct Composition {
    /// Stable theme identifier.
    pub id: &'static str,
    /// Display title.
    pub title: &'static str,
    /// Category slug for filtering.
    pub category: &'static str,
    /// Mood and intended use.
    pub description: &'static str,
    /// Search terms for mood, instruments, and context.
    pub tags: &'static [&'static str],
    /// Tempo in beats per minute.
    pub bpm: f32,
    /// Musical key and mode.
    pub key: &'static str,
    /// Number of 4/4 bars in the rendered loop.
    pub bars: u32,
    #[serde(skip)]
    tonic: u8,
    #[serde(skip)]
    scale: &'static [i8; 7],
    #[serde(skip)]
    progression: [i8; 4],
    #[serde(skip)]
    melody: &'static [(u32, u32, i8)],
    #[serde(skip)]
    lead: Instrument,
    #[serde(skip)]
    harmony: Instrument,
    #[serde(skip)]
    syncopated: bool,
}

#[derive(Debug, Clone, Copy)]
enum Instrument {
    Piano,
    Keys,
    Mallet,
    Pluck,
    Flute,
    Bell,
    Pulse,
    Fm,
}
use Instrument::{Bell, Flute as FluteVoice, Fm, Keys, Mallet, Piano, Pluck, Pulse};
const MAJOR: [i8; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i8; 7] = [0, 2, 3, 5, 7, 8, 10];
const DORIAN: [i8; 7] = [0, 2, 3, 5, 7, 9, 10];

macro_rules! theme {
    ($id:literal, $title:literal, $category:literal, $description:literal, [$($tag:literal),+], $bpm:literal, $key:literal, $tonic:literal, $scale:ident, $chords:expr, $melody:expr, $lead:expr, $harmony:expr, $syncopated:literal) => {
        Composition { id: $id, title: $title, category: $category, description: $description, tags: &[$($tag),+], bpm: $bpm, key: $key, bars: LOOP_BARS, tonic: $tonic, scale: &$scale, progression: $chords, melody: $melody, lead: $lead, harmony: $harmony, syncopated: $syncopated }
    };
}

/// Twelve original compositions, each available in three arrangements.
/// Melodies are authored scale-degree phrases rather than randomized note runs.
pub static COMPOSITIONS: &[Composition] = &[
    theme!(
        "pocket-horizon",
        "Pocket Horizon",
        "menu",
        "A welcoming mallet melody over warm keys for a bright title screen.",
        ["menu", "welcoming", "mallets"],
        120.0,
        "C major",
        60,
        MAJOR,
        [0, 5, 3, 4],
        &[
            (0, 3, 4),
            (4, 3, 2),
            (8, 5, 0),
            (14, 2, 1),
            (16, 5, 5),
            (22, 2, 7),
            (26, 3, 6),
            (30, 2, 4),
            (32, 3, 3),
            (36, 3, 5),
            (40, 5, 7),
            (46, 2, 5),
            (48, 5, 4),
            (54, 2, 1),
            (58, 5, 0)
        ],
        Mallet,
        Keys,
        false
    ),
    theme!(
        "puzzle-circuit",
        "Puzzle Circuit",
        "puzzle",
        "A precise glassy motif and nimble pulse for solving and building.",
        ["puzzle", "glassy", "focused"],
        108.0,
        "D major",
        62,
        MAJOR,
        [0, 3, 5, 4],
        &[
            (0, 2, 0),
            (3, 2, 2),
            (6, 3, 4),
            (12, 3, 2),
            (16, 2, 3),
            (19, 2, 5),
            (22, 3, 7),
            (28, 3, 5),
            (32, 2, 5),
            (35, 2, 7),
            (38, 3, 9),
            (44, 3, 7),
            (48, 3, 4),
            (54, 3, 1),
            (60, 3, 0)
        ],
        Fm,
        Keys,
        true
    ),
    theme!(
        "sunlit-garden",
        "Sunlit Garden",
        "cozy",
        "A relaxed plucked tune for tending a garden or a peaceful home.",
        ["cozy", "garden", "guitar"],
        96.0,
        "G major",
        67,
        MAJOR,
        [0, 4, 5, 3],
        &[
            (0, 5, 0),
            (6, 2, 2),
            (10, 5, 4),
            (16, 5, 4),
            (22, 2, 6),
            (26, 5, 8),
            (32, 5, 5),
            (38, 2, 7),
            (42, 5, 6),
            (48, 5, 5),
            (54, 2, 3),
            (58, 5, 0)
        ],
        Pluck,
        Piano,
        false
    ),
    theme!(
        "lantern-trail",
        "Lantern Trail",
        "exploration",
        "A gentle modal flute line for wandering forests and quiet paths.",
        ["exploration", "forest", "flute"],
        88.0,
        "D dorian",
        62,
        DORIAN,
        [0, 3, 6, 0],
        &[
            (0, 6, 0),
            (8, 3, 2),
            (12, 3, 4),
            (16, 6, 5),
            (24, 3, 3),
            (28, 3, 2),
            (32, 6, 6),
            (40, 3, 8),
            (44, 3, 7),
            (48, 7, 4),
            (56, 3, 2),
            (60, 3, 0)
        ],
        FluteVoice,
        Mallet,
        false
    ),
    theme!(
        "starfall-hollow",
        "Starfall Hollow",
        "fantasy",
        "A bell-led minor theme for magic, ruins, and a small enchanted world.",
        ["fantasy", "magic", "bells"],
        84.0,
        "A minor",
        57,
        MINOR,
        [0, 5, 2, 6],
        &[
            (0, 5, 0),
            (6, 2, 4),
            (10, 5, 7),
            (16, 5, 8),
            (22, 2, 7),
            (26, 5, 5),
            (32, 5, 4),
            (38, 2, 2),
            (42, 5, 7),
            (48, 5, 6),
            (54, 2, 4),
            (58, 5, 0)
        ],
        Bell,
        Keys,
        false
    ),
    theme!(
        "neon-sprint",
        "Neon Sprint",
        "action",
        "A clipped pulse hook and forward rhythm for fast arcade action.",
        ["action", "arcade", "pulse"],
        144.0,
        "E minor",
        64,
        MINOR,
        [0, 5, 2, 6],
        &[
            (0, 2, 0),
            (3, 2, 0),
            (6, 2, 4),
            (10, 2, 2),
            (14, 2, 4),
            (16, 2, 5),
            (19, 2, 5),
            (22, 2, 7),
            (26, 2, 6),
            (30, 2, 5),
            (32, 2, 2),
            (35, 2, 4),
            (38, 2, 7),
            (42, 2, 4),
            (46, 2, 2),
            (48, 2, 6),
            (51, 2, 4),
            (54, 2, 2),
            (58, 2, 1),
            (62, 1, 0)
        ],
        Pulse,
        Fm,
        true
    ),
    theme!(
        "orbit-relay",
        "Orbit Relay",
        "sci-fi",
        "A crystalline signal motif over a steady electronic orbit.",
        ["sci-fi", "space", "electronic"],
        128.0,
        "D minor",
        62,
        MINOR,
        [0, 6, 5, 4],
        &[
            (0, 3, 0),
            (4, 3, 7),
            (10, 3, 4),
            (14, 2, 2),
            (16, 3, 6),
            (20, 3, 8),
            (26, 3, 7),
            (30, 2, 4),
            (32, 3, 5),
            (36, 3, 7),
            (42, 3, 9),
            (46, 2, 7),
            (48, 3, 4),
            (52, 3, 6),
            (58, 3, 1),
            (62, 1, 0)
        ],
        Fm,
        Keys,
        true
    ),
    theme!(
        "hidden-passage",
        "Hidden Passage",
        "suspense",
        "A sparse low motif and dark harmony for searching an uncertain place.",
        ["suspense", "mystery", "dark"],
        76.0,
        "C minor",
        60,
        MINOR,
        [0, 1, 0, 6],
        &[
            (0, 6, 0),
            (10, 5, 1),
            (16, 6, 2),
            (26, 5, 1),
            (32, 6, 0),
            (42, 5, 4),
            (48, 6, 6),
            (58, 5, 0)
        ],
        Keys,
        Fm,
        false
    ),
    theme!(
        "cloudline",
        "Cloudline",
        "chill",
        "A mellow electric-piano phrase for downtime and unhurried play.",
        ["chill", "mellow", "electric piano"],
        92.0,
        "F major",
        65,
        MAJOR,
        [0, 5, 1, 4],
        &[
            (0, 5, 2),
            (7, 2, 4),
            (10, 5, 7),
            (16, 5, 9),
            (23, 2, 7),
            (26, 5, 5),
            (32, 5, 6),
            (39, 2, 4),
            (42, 5, 1),
            (48, 5, 4),
            (55, 2, 2),
            (58, 5, 0)
        ],
        Keys,
        Keys,
        true
    ),
    theme!(
        "market-morning",
        "Market Morning",
        "cozy",
        "A playful piano tune and walking bass for shops and friendly towns.",
        ["cozy", "town", "piano"],
        112.0,
        "C major",
        60,
        MAJOR,
        [0, 3, 4, 0],
        &[
            (0, 3, 0),
            (4, 3, 2),
            (8, 3, 4),
            (12, 3, 2),
            (16, 3, 3),
            (20, 3, 5),
            (24, 3, 7),
            (28, 3, 5),
            (32, 3, 4),
            (36, 3, 6),
            (40, 3, 8),
            (44, 3, 6),
            (48, 3, 7),
            (52, 3, 4),
            (56, 3, 2),
            (60, 3, 0)
        ],
        Piano,
        Pluck,
        false
    ),
    theme!(
        "clockwork-quest",
        "Clockwork Quest",
        "adventure",
        "A nimble wooden motif with a determined beat for light adventure.",
        ["adventure", "clockwork", "mallets"],
        124.0,
        "B minor",
        59,
        MINOR,
        [0, 3, 5, 4],
        &[
            (0, 2, 0),
            (3, 2, 2),
            (6, 4, 4),
            (12, 3, 2),
            (16, 2, 3),
            (19, 2, 5),
            (22, 4, 7),
            (28, 3, 5),
            (32, 2, 5),
            (35, 2, 7),
            (38, 4, 9),
            (44, 3, 7),
            (48, 3, 6),
            (54, 3, 4),
            (60, 3, 0)
        ],
        Mallet,
        Fm,
        true
    ),
    theme!(
        "midnight-save",
        "Midnight Save",
        "menu",
        "A quiet minor electric-piano refrain for pause screens and late sessions.",
        ["menu", "night", "reflective"],
        104.0,
        "E minor",
        64,
        MINOR,
        [0, 5, 3, 4],
        &[
            (0, 6, 0),
            (8, 3, 2),
            (12, 3, 4),
            (16, 6, 5),
            (24, 3, 7),
            (28, 3, 5),
            (32, 6, 3),
            (40, 3, 5),
            (44, 3, 7),
            (48, 6, 6),
            (56, 3, 4),
            (60, 3, 0)
        ],
        Keys,
        Piano,
        false
    ),
];

/// Look up a composed theme by its stable identifier.
pub fn composition(id: &str) -> Option<&'static Composition> {
    COMPOSITIONS.iter().find(|theme| theme.id == id)
}

/// Author one editable four-bar score. Notes have musical names; tracks have
/// stable ids and explicit catalog voicings. Seeded velocity differences are
/// small and preserve the composed pitches, timing, and harmonic progression.
pub fn score(spec: &BgmSpec) -> anyhow::Result<Song> {
    spec.validate()?;
    let theme = composition(&spec.composition).expect("validated theme");
    let mut rng = Rng::new(spec.seed ^ 0xC6BC_2796_92B5_C323);
    let calm = spec.variant == BgmVariant::Calm;
    let drive = spec.variant == BgmVariant::Drive;
    let mut song =
        Song::new(format!("{}-{}", theme.id, spec.variant.id()), theme.bpm).with_seed(spec.seed);
    song.engine = BGM_ENGINE;
    song.version = BGM_SCHEMA;
    let lead = instrument(theme.lead, spec.variant)
        .gain(if calm { 0.27 } else { 0.34 })
        .pan(-0.08);
    let harmony = instrument(
        theme.harmony,
        if calm {
            BgmVariant::Calm
        } else {
            BgmVariant::Original
        },
    )
    .gain(if calm { 0.15 } else { 0.13 })
    .pan(0.14);
    let bass =
        if calm { Bass::sub() } else { Bass::finger() }.gain(if drive { 0.31 } else { 0.25 });
    let pulse = instrument(if theme.syncopated { Fm } else { Mallet }, spec.variant)
        .gain(if calm {
            0.075
        } else if drive {
            0.15
        } else {
            0.10
        })
        .pan(0.35);
    let drums = Drums::electronic()
        .gain(if calm {
            0.12
        } else if drive {
            0.30
        } else {
            0.22
        })
        .pan(-0.12);
    for (name, voice) in [
        ("melody", lead),
        ("harmony", harmony),
        ("bass", bass),
        ("arpeggio", pulse),
        ("drums", drums),
    ] {
        song.add_voice(name, &voice);
    }
    let mut melody = Vec::new();
    for &(step, len, degree) in theme.melody {
        melody.push(musical_note(
            theme,
            step,
            len,
            degree,
            0,
            velocity(
                &mut rng,
                if calm {
                    0.62
                } else if drive {
                    0.90
                } else {
                    0.80
                },
            ),
        ));
    }
    song.add_pattern("melody", LOOP_BARS, melody);
    let mut chords = Vec::new();
    let mut bassline = Vec::new();
    let mut arpeggio = Vec::new();
    let mut beat = Vec::new();
    for (bar, &root) in theme.progression.iter().enumerate() {
        let offset = bar as u32 * STEPS_PER_BAR;
        let chord_starts: &[u32] = if calm {
            &[0]
        } else if drive {
            &[0, 6, 10]
        } else {
            &[0, 8]
        };
        let chord_length = if calm {
            15
        } else if drive {
            4
        } else {
            7
        };
        for &start in chord_starts {
            for degree in [root, root + 2, root + 4] {
                chords.push(musical_note(
                    theme,
                    offset + start,
                    chord_length,
                    degree,
                    -12,
                    velocity(&mut rng, if calm { 0.65 } else { 0.68 }),
                ));
            }
        }
        let bass_starts: &[u32] = if calm {
            &[0, 8]
        } else if drive {
            &[0, 3, 6, 8, 10, 12, 14]
        } else if theme.syncopated {
            &[0, 6, 8, 14]
        } else {
            &[0, 4, 8, 12]
        };
        for (index, &start) in bass_starts.iter().enumerate() {
            let degree = if index % 4 == 3 {
                root + 4
            } else if index % 4 == 2 {
                root + 7
            } else {
                root
            };
            bassline.push(musical_note(
                theme,
                offset + start,
                if calm {
                    7
                } else if drive {
                    2
                } else {
                    3
                },
                degree,
                -24,
                velocity(&mut rng, 0.84),
            ));
        }
        let arp_starts: &[u32] = if calm {
            &[2, 10]
        } else if drive {
            &[0, 2, 4, 6, 8, 10, 12, 14]
        } else {
            &[2, 6, 10, 14]
        };
        for (index, &start) in arp_starts.iter().enumerate() {
            let degree = root + [0, 2, 4, 7][index % 4];
            arpeggio.push(musical_note(
                theme,
                offset + start,
                if calm { 3 } else { 1 },
                degree,
                0,
                velocity(&mut rng, 0.60),
            ));
        }
        if calm {
            beat.push(note_vel(offset, 2, "midi:36", 0.38));
            beat.push(note_vel(offset + 8, 1, "midi:42", 0.25));
        } else {
            let kicks: &[u32] = if drive {
                &[0, 6, 8, 10]
            } else if theme.syncopated {
                &[0, 6, 8]
            } else {
                &[0, 8]
            };
            for &step in kicks {
                beat.push(note_vel(
                    offset + step,
                    2,
                    "midi:36",
                    velocity(&mut rng, 0.82),
                ));
            }
            for step in [4, 12] {
                beat.push(note_vel(
                    offset + step,
                    1,
                    "midi:38",
                    velocity(&mut rng, if drive { 0.73 } else { 0.55 }),
                ));
            }
            let hat_starts: &[u32] = if drive {
                &[0, 2, 4, 6, 8, 10, 12, 14]
            } else {
                &[2, 6, 10, 14]
            };
            for &step in hat_starts {
                beat.push(note_vel(
                    offset + step,
                    1,
                    "midi:42",
                    velocity(&mut rng, if drive { 0.36 } else { 0.27 }),
                ));
            }
        }
    }
    for (name, notes) in [
        ("harmony", chords),
        ("bass", bassline),
        ("arpeggio", arpeggio),
        ("drums", beat),
    ] {
        song.add_pattern(name, LOOP_BARS, notes);
    }
    for name in ["melody", "harmony", "bass", "arpeggio", "drums"] {
        song.arrange(name, name, 0);
    }
    song.master = vec![Node::Gain {
        amount: Value::Const(0.78),
    }];
    Ok(song)
}

/// Compile a warmed-up musical loop. A complete cycle precedes the audible
/// region; a short guard repeats the next downbeat for the crossfade. Including
/// that guard in the selected region preserves the exact four-bar frame count
/// when the renderer subtracts its crossfade length.
pub fn generate(spec: &BgmSpec) -> anyhow::Result<SoundDoc> {
    let mut song = score(spec)?;
    let theme = composition(&spec.composition).expect("validated theme");
    let names: Vec<_> = song.tracks.iter().map(|track| track.name.clone()).collect();
    for name in &names {
        song.arrange(name, name, LOOP_BARS);
        // Only the first bar of this extra placement is in the render window.
        song.arrange(name, name, LOOP_BARS * 2);
    }
    let mut doc = song.to_doc()?;
    let period = (theme.bars as f64 * 4.0 * 60.0 / theme.bpm as f64 * spec.sample_rate as f64)
        .round() as usize;
    let crossfade = (0.035 * spec.sample_rate as f64).round() as usize;
    // Match the seq's f32 step scheduling at the repeated downbeat. Rounding
    // one cycle twice can put the guard one sample into a fresh kick attack.
    // Anchoring both ends here preserves the exact period and a quiet join.
    let step_frames = spec.sample_rate as f32 * 60.0 / theme.bpm / song.steps_per_beat as f32;
    let boundary = (LOOP_BARS * 2 * STEPS_PER_BAR) as f32 * step_frames;
    let boundary = boundary as usize;
    let start = boundary - period;
    let end = boundary + crossfade;
    doc.sample_rate = spec.sample_rate;
    doc.seed = spec.seed;
    doc.duration = frame_seconds(end + 2, spec.sample_rate);
    doc.playback = Playback::Loop {
        start_secs: frame_seconds(start, spec.sample_rate),
        end_secs: Some(frame_seconds(end, spec.sample_rate)),
        crossfade_secs: frame_seconds(crossfade, spec.sample_rate),
    };
    doc.validate()?;
    Ok(doc)
}

// make_loop_buffer truncates f32 seconds × rate. Pick a representable value
// that reconstructs the exact requested integer frame rather than losing one
// to floating-point rounding at the loop boundaries.
fn frame_seconds(frame: usize, sample_rate: u32) -> f32 {
    let mut seconds = frame as f32 / sample_rate as f32;
    while ((seconds * sample_rate as f32) as usize) < frame {
        seconds = f32::from_bits(seconds.to_bits() + 1);
    }
    while ((seconds * sample_rate as f32) as usize) > frame {
        seconds = f32::from_bits(seconds.to_bits() - 1);
    }
    seconds
}

fn velocity(rng: &mut Rng, center: f32) -> f32 {
    center + rng.bi() * 0.035
}

fn musical_note(
    theme: &Composition,
    step: u32,
    len: u32,
    degree: i8,
    octave: i8,
    gain: f32,
) -> SeqNote {
    let midi = theme.tonic as i16
        + octave as i16
        + theme.scale[degree.rem_euclid(7) as usize] as i16
        + 12 * degree.div_euclid(7) as i16;
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let pitch = format!(
        "{}{}",
        names[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    );
    note_vel(step, len, &pitch, gain)
}

fn instrument(kind: Instrument, variant: BgmVariant) -> Voice {
    let calm = variant == BgmVariant::Calm;
    let drive = variant == BgmVariant::Drive;
    let env = Adsr {
        a: if calm { 0.012 } else { 0.003 },
        d: 0.13,
        s: if calm { 0.55 } else { 0.72 },
        r: 0.05,
        punch: 0.0,
    };
    let mut voice = match kind {
        Piano => {
            if calm {
                GrandPiano::felt()
            } else {
                GrandPiano::mellow()
            }
        }
        Keys => ElectricPiano::rhodes(),
        Mallet => Mallets::marimba(),
        Pluck => Guitar::nylon(),
        FluteVoice => Flute::concert(),
        Bell => {
            let mut voice = ElectricPiano::dx();
            voice.voice.fm_ratio = Some(3.5);
            voice.voice.fm_index = Some(if calm { 0.7 } else { 1.6 });
            voice.voice.fm_strike = Some(0.4);
            voice
        }
        Pulse | Fm => Voice {
            name: "synth".into(),
            wave: if matches!(kind, Pulse) && !calm {
                SeqWave::Square
            } else {
                SeqWave::Fm
            },
            env,
            gain: 1.0,
            pan: 0.0,
            reverb: 0.0,
            swing: None,
            humanize: None,
            voice: VoiceParams {
                fm_ratio: Some(if matches!(kind, Pulse) { 1.0 } else { 2.0 }),
                fm_index: Some(if calm {
                    0.45
                } else if drive {
                    2.3
                } else {
                    1.0
                }),
                fm_strike: Some(0.22),
                duty: Some(0.37),
                ..VoiceParams::default()
            },
        },
    };
    // Every note closes within its musical gate; this also gives the loop a
    // quiet, clean join at its next downbeat without a tempo-changing fade.
    voice.env.r = voice.env.r.min(0.12);
    if drive {
        voice.env.a = voice.env.a.min(0.003);
    }
    voice
}
