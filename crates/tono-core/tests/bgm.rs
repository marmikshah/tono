//! Composed music quality, score recall, and musical loop timing.

use std::collections::HashSet;

use tono_core::{
    bgm::{BGM_VERSION, BgmSpec, BgmVariant, COMPOSITIONS, VARIANTS, composition, generate, score},
    dsl::{Node, Playback, SoundDoc},
    render,
    song::Song,
};

#[test]
fn twelve_compositions_have_real_scores_and_distinct_arrangements() {
    assert_eq!(COMPOSITIONS.len(), 12);
    assert_eq!(VARIANTS.len(), 3);
    let mut ids = HashSet::new();
    let mut melodic_phrases = HashSet::new();
    for theme in COMPOSITIONS {
        assert!(ids.insert(theme.id));
        assert!(std::ptr::eq(theme, composition(theme.id).unwrap()));
        assert!(!theme.title.is_empty() && !theme.description.is_empty() && !theme.tags.is_empty());
        assert_eq!(theme.bars, 4);
        let mut arrangements = HashSet::new();
        for variant in VARIANTS {
            let song = score(&BgmSpec::new(theme.id, variant, 42)).unwrap();
            assert_eq!(song.bpm, theme.bpm);
            assert_eq!(song.length_bars(), 4);
            assert_eq!(song.tracks.len(), 5);
            assert_eq!(song.patterns.len(), 5);
            let melody = song
                .patterns
                .iter()
                .find(|pattern| pattern.name == "melody")
                .unwrap();
            assert!(melody.notes.len() >= 8, "authored phrase: {}", theme.id);
            let harmony = song
                .patterns
                .iter()
                .find(|pattern| pattern.name == "harmony")
                .unwrap();
            assert!(harmony.notes.len() >= 12, "four chords: {}", theme.id);
            assert!(arrangements.insert(serde_json::to_string(&song).unwrap()));
            if variant == BgmVariant::Original {
                let pitches = melody
                    .notes
                    .iter()
                    .map(|note| serde_json::to_string(&note.pitch).unwrap())
                    .collect::<Vec<_>>();
                assert!(
                    melodic_phrases.insert(pitches),
                    "distinct melody: {}",
                    theme.id
                );
            }
        }
    }
}

#[test]
fn every_arrangement_renders_cleanly_with_an_exact_four_bar_period() {
    for theme in COMPOSITIONS {
        for variant in VARIANTS {
            let mut spec = BgmSpec::new(theme.id, variant, 42);
            spec.sample_rate = 8_000;
            let doc = generate(&spec).unwrap();
            let rendered = render::render_product(&doc);
            let expected = (theme.bars as f64 * 4.0 * 60.0 / theme.bpm as f64
                * spec.sample_rate as f64)
                .round() as usize;
            assert_eq!(rendered.mono.len(), expected, "musical period: {spec:?}");
            assert!(doc.duration < 30.0, "browser render bound: {spec:?}");
            let peak = rendered.mono.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
            let rms = (rendered
                .mono
                .iter()
                .map(|x| (*x as f64).powi(2))
                .sum::<f64>()
                / expected as f64)
                .sqrt();
            assert!(rendered.mono.iter().all(|x| x.is_finite()), "{spec:?}");
            assert!(peak <= 0.95, "headroom: {spec:?}; {peak}");
            assert!(rms > 0.005, "audibility: {spec:?}; {rms}");
            let (left, right) = rendered.stereo.unwrap();
            assert_eq!(left.len(), expected);
            assert_eq!(right.len(), expected);
            for channel in [&left, &right] {
                assert!(
                    render::loop_seam_db(channel) < -40.0,
                    "quiet seam: {spec:?}; {} dB",
                    render::loop_seam_db(channel)
                );
            }
        }
    }
}

#[test]
fn loop_regions_preserve_the_grid_at_low_standard_and_high_rates() {
    for theme in COMPOSITIONS {
        for sample_rate in [8_000, 44_100, 48_000, 192_000] {
            let mut spec = BgmSpec::new(theme.id, BgmVariant::Original, u64::MAX);
            spec.sample_rate = sample_rate;
            let doc = generate(&spec).unwrap();
            doc.validate().unwrap();
            assert_eq!(doc.version, 2);
            assert_eq!(doc.engine, 5);
            let Playback::Loop {
                start_secs,
                end_secs: Some(end_secs),
                crossfade_secs,
            } = doc.playback
            else {
                panic!("loop playback")
            };
            let start = (start_secs * sample_rate as f32) as usize;
            let end = (end_secs * sample_rate as f32) as usize;
            let crossfade = (crossfade_secs * sample_rate as f32) as usize;
            let period = (theme.bars as f64 * 4.0 * 60.0 / theme.bpm as f64 * sample_rate as f64)
                .round() as usize;
            assert!(start.abs_diff(period) <= 2, "one warm-up cycle: {spec:?}");
            assert_eq!(
                end - start - crossfade,
                period,
                "crossfade preserves tempo: {spec:?}"
            );
            assert!(doc.duration < 30.0);
            let Node::Tracks { tracks, .. } = doc.root else {
                panic!("named tracks")
            };
            assert_eq!(tracks.len(), 5);
            let ids: HashSet<_> = tracks
                .iter()
                .map(|track| track.id.as_deref().unwrap())
                .collect();
            assert_eq!(ids.len(), tracks.len());
        }
    }
}

#[test]
fn saved_scores_specs_and_documents_replay_exactly() {
    for theme in COMPOSITIONS {
        let mut spec = BgmSpec::new(theme.id, BgmVariant::Calm, 7);
        spec.sample_rate = 8_000;
        let saved_spec: BgmSpec =
            serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(spec, saved_spec);
        let doc = generate(&spec).unwrap();
        let regenerated = generate(&saved_spec).unwrap();
        assert_eq!(
            serde_json::to_string(&doc).unwrap(),
            serde_json::to_string(&regenerated).unwrap()
        );
        let original_score = score(&spec).unwrap();
        let saved_score: Song =
            serde_json::from_str(&serde_json::to_string(&original_score).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&original_score.to_doc().unwrap()).unwrap(),
            serde_json::to_string(&saved_score.to_doc().unwrap()).unwrap()
        );
        let saved_doc: SoundDoc =
            serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(render::render(&doc), render::render(&saved_doc));
    }
}

#[test]
fn invalid_music_requests_fail_before_synthesis() {
    let mut spec = BgmSpec::new("pocket-horizon", BgmVariant::Original, 42);
    for sample_rate in [0, 7_999, 192_001, u32::MAX] {
        spec.sample_rate = sample_rate;
        assert!(
            generate(&spec)
                .unwrap_err()
                .to_string()
                .contains("sample_rate")
        );
    }
    spec.sample_rate = 48_000;
    spec.bgm_version = BGM_VERSION + 1;
    assert!(generate(&spec).unwrap_err().to_string().contains("version"));
    spec.bgm_version = BGM_VERSION;
    spec.composition = "missing".into();
    assert!(
        generate(&spec)
            .unwrap_err()
            .to_string()
            .contains("composition")
    );
}

#[test]
fn composition_revision_one_pins_the_canonical_arrangements() {
    let hash = COMPOSITIONS
        .iter()
        .flat_map(|theme| VARIANTS.map(|variant| (theme, variant)))
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, (theme, variant)| {
            let doc = generate(&BgmSpec::new(theme.id, variant, 42)).unwrap();
            (hash ^ tono_core::program::content_hash(&doc)).wrapping_mul(0x100_0000_01b3)
        });
    assert_eq!(
        hash, 15_918_766_156_745_047_698,
        "composition edits require a new BGM revision"
    );
}
