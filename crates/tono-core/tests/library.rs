//! Quality gates for the asset-free curated sound library.

use std::collections::HashSet;

use tono_core::{
    dsl::{Node, Playback, SoundDoc},
    library::{LIBRARY_VERSION, LibrarySpec, LibraryVariant, RECIPES, VARIANTS, generate, recipe},
    render,
};

#[test]
fn discovery_has_distinct_recipes_and_six_meaningful_voicings() {
    assert_eq!(RECIPES.len(), 64);
    assert_eq!(VARIANTS.len(), 6);
    let mut ids = HashSet::new();
    let mut categories = HashSet::new();
    let mut canonical_graphs = HashSet::new();
    for entry in RECIPES {
        assert!(ids.insert(entry.id), "duplicate id: {}", entry.id);
        assert!(!entry.title.is_empty() && !entry.description.is_empty());
        assert!(!entry.tags.is_empty());
        assert!(std::ptr::eq(entry, recipe(entry.id).unwrap()));
        categories.insert(entry.category);
        let mut variants = HashSet::new();
        for variant in VARIANTS {
            let doc = generate(&LibrarySpec::new(entry.id, variant, 42)).unwrap();
            let identity = serde_json::to_string(&(doc.duration, &doc.root)).unwrap();
            assert!(
                variants.insert(identity),
                "duplicate voicing: {} {variant:?}",
                entry.id
            );
            if variant == LibraryVariant::Classic {
                assert!(
                    canonical_graphs
                        .insert(serde_json::to_string(&(doc.duration, doc.root)).unwrap()),
                    "duplicate recipe: {}",
                    entry.id
                );
            }
        }
    }
    assert_eq!(categories.len(), 9);
    assert_eq!(RECIPES.iter().filter(|entry| entry.looping).count(), 8);
    assert!(recipe("missing").is_none());
}

#[test]
fn every_voicing_renders_audibly_at_low_and_standard_sample_rates() {
    for entry in RECIPES {
        for variant in VARIANTS {
            for sample_rate in [8_000, 48_000] {
                let mut spec = LibrarySpec::new(entry.id, variant, 42);
                spec.sample_rate = sample_rate;
                let doc = generate(&spec).unwrap();
                assert_eq!(doc.engine, 5);
                assert_eq!(doc.version, 2);
                let audio = render::render(&doc);
                let peak = audio.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
                let rms = (audio.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
                    / audio.len() as f64)
                    .sqrt();
                assert!(audio.iter().all(|x| x.is_finite()), "{spec:?}");
                assert!(peak <= 0.99, "peak={peak}: {spec:?}");
                assert!(rms > 0.002, "rms={rms}: {spec:?}");
                assert!(doc.duration <= 6.5, "{spec:?}");
                if entry.looping {
                    assert!(matches!(doc.playback, Playback::Loop { .. }));
                    assert!(audio.len() as f32 / (sample_rate as f32) < doc.duration * 0.8);
                } else {
                    assert_eq!(audio[0], 0.0, "quiet onset: {spec:?}");
                    let tail = &audio[audio.len() - sample_rate as usize / 1000..];
                    assert!(
                        tail.iter().all(|x| x.abs() < 1e-5),
                        "quiet endpoint: {spec:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn saved_specs_and_sources_reproduce_exactly_with_editable_layers() {
    for entry in RECIPES {
        let mut spec = LibrarySpec::new(entry.id, LibraryVariant::Classic, u64::MAX);
        spec.sample_rate = 8_000;
        let saved: LibrarySpec =
            serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(spec, saved);
        let doc = generate(&spec).unwrap();
        let replay = generate(&saved).unwrap();
        let json = serde_json::to_string(&doc).unwrap();
        assert_eq!(json, serde_json::to_string(&replay).unwrap());
        let saved_doc: SoundDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(render::render(&doc), render::render(&saved_doc));
        let Node::Tracks { tracks, .. } = &doc.root else {
            panic!("missing editable layers: {}", entry.id)
        };
        let mut layer_ids = HashSet::new();
        for track in tracks {
            assert!(
                layer_ids.insert(track.id.as_deref().unwrap()),
                "duplicate layer: {}",
                entry.id
            );
        }
        spec.variation = 0.0;
        let a = generate(&spec).unwrap();
        spec.seed = 0;
        let b = generate(&spec).unwrap();
        assert_eq!(a.duration, b.duration);
        assert_eq!(
            serde_json::to_string(&a.root).unwrap(),
            serde_json::to_string(&b.root).unwrap()
        );
    }
}

#[test]
fn extreme_controls_and_seeds_make_bounded_valid_graphs() {
    for entry in RECIPES {
        for (seed, sample_rate, brightness, punch, duration_scale, pitch_semitones) in [
            (0, 8_000, 0.0, 0.0, 0.25, -24.0),
            (u64::MAX, 192_000, 1.0, 1.0, 3.0, 24.0),
        ] {
            let mut spec = LibrarySpec::new(entry.id, LibraryVariant::Classic, seed);
            spec.sample_rate = sample_rate;
            spec.brightness = brightness;
            spec.punch = punch;
            spec.duration_scale = duration_scale;
            spec.pitch_semitones = pitch_semitones;
            spec.variation = 1.0;
            generate(&spec).unwrap().validate().unwrap();
        }
    }
}

#[test]
fn invalid_library_requests_are_rejected_before_rendering() {
    let base = LibrarySpec::new("ui-click", LibraryVariant::Classic, 42);
    for field in [
        "brightness",
        "punch",
        "duration_scale",
        "pitch_semitones",
        "variation",
    ] {
        for value in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            -f32::MAX,
        ] {
            let mut spec = base.clone();
            match field {
                "brightness" => spec.brightness = value,
                "punch" => spec.punch = value,
                "duration_scale" => spec.duration_scale = value,
                "pitch_semitones" => spec.pitch_semitones = value,
                _ => spec.variation = value,
            }
            assert!(generate(&spec).unwrap_err().to_string().contains(field));
        }
    }
    for sample_rate in [0, 7_999, 192_001, u32::MAX] {
        let mut spec = base.clone();
        spec.sample_rate = sample_rate;
        assert!(
            generate(&spec)
                .unwrap_err()
                .to_string()
                .contains("sample_rate")
        );
    }
    let mut spec = base.clone();
    spec.library_version = LIBRARY_VERSION + 1;
    assert!(generate(&spec).unwrap_err().to_string().contains("version"));
    spec = base;
    spec.recipe = "missing".into();
    assert!(generate(&spec).unwrap_err().to_string().contains("recipe"));
}

#[test]
fn library_revision_one_preserves_canonical_graphs() {
    let hash = RECIPES
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, entry| {
            let doc = generate(&LibrarySpec::new(entry.id, LibraryVariant::Classic, 42)).unwrap();
            (hash ^ tono_core::program::content_hash(&doc)).wrapping_mul(0x100_0000_01b3)
        });
    assert_eq!(
        hash, 4_675_144_601_048_724_766,
        "recipe edits require a new library revision"
    );
}
