//! Technical quality and reproducibility checks for game SFX starters.

use tono_core::{
    dsl::SoundDoc,
    generate::{SFX_TEMPLATE_VERSION, SfxSpec, SfxTemplate, generate_sfx},
    render,
};

#[test]
fn all_templates_render_cleanly_at_control_and_sample_rate_extremes() {
    for template in SfxTemplate::ALL {
        for seed in [0, 1, 42, u64::MAX] {
            for sample_rate in [8_000, 48_000, 192_000] {
                for (brightness, punch, variation) in
                    [(0.0, 0.0, 0.0), (0.5, 0.5, 0.15), (1.0, 1.0, 1.0)]
                {
                    let mut spec = SfxSpec::new(template, seed);
                    spec.sample_rate = sample_rate;
                    spec.brightness = brightness;
                    spec.punch = punch;
                    spec.variation = variation;
                    let doc = generate_sfx(&spec).unwrap();
                    doc.validate().unwrap();
                    assert_eq!(doc.engine, Some(5), "version-1 recipe engine pin");
                    assert_eq!(doc.version, Some(2), "version-1 recipe schema pin");
                    let audio = render::render(&doc);
                    let peak = audio.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
                    let rms = (audio.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
                        / audio.len() as f64)
                        .sqrt();
                    assert!(audio.iter().all(|x| x.is_finite()), "{spec:?}");
                    assert!(peak <= 0.95, "headroom: {spec:?}; peak={peak}");
                    assert!(rms > 0.005, "audibility: {spec:?}; rms={rms}");
                    assert_eq!(audio[0], 0.0, "quiet attack: {spec:?}");
                    assert!(
                        audio[audio.len() - sample_rate as usize / 1000..]
                            .iter()
                            .all(|x| x.abs() < 1e-5),
                        "quiet endpoint: {spec:?}"
                    );
                    assert!(doc.duration < 1.1, "short one-shot: {spec:?}");
                }
            }
        }
    }
}

#[test]
fn template_version_one_preserves_saved_graphs() {
    // Changing a recipe requires a new template revision. These canonical
    // document hashes pin its parameters, seed interpretation, and versions.
    let actual = SfxTemplate::ALL.map(|template| {
        let doc = generate_sfx(&SfxSpec::new(template, 42)).unwrap();
        tono_core::program::content_hash(&doc)
    });
    assert_eq!(
        actual,
        [
            17_243_757_088_351_923_463,
            14_705_021_557_523_368_555,
            9_407_506_146_578_316_463,
            6_874_477_661_133_897_558,
            17_038_792_009_147_450_867,
            3_506_272_610_478_067_323,
            6_284_363_925_577_883_429,
            12_084_382_493_479_151_040,
        ]
    );
}

#[test]
fn saved_specs_and_documents_reproduce_the_same_audio() {
    for template in SfxTemplate::ALL {
        let spec = SfxSpec::new(template, 42);
        let saved: SfxSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(spec, saved);
        let doc = generate_sfx(&spec).unwrap();
        let replay = generate_sfx(&saved).unwrap();
        assert_eq!(
            serde_json::to_value(&doc).unwrap(),
            serde_json::to_value(&replay).unwrap()
        );
        let saved_doc: SoundDoc =
            serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(render::render(&doc), render::render(&saved_doc));
        let varied = generate_sfx(&SfxSpec::new(template, 43)).unwrap();
        assert_ne!(render::render(&doc), render::render(&varied), "{template}");
        assert_eq!(doc.name, varied.name);
        assert!((varied.duration / doc.duration - 1.0).abs() < 0.07);
    }
}

#[test]
fn zero_variation_keeps_recipe_parameters_fixed() {
    for template in SfxTemplate::ALL {
        let mut first = SfxSpec::new(template, 0);
        first.variation = 0.0;
        let mut second = first.clone();
        second.seed = 123;
        let a = generate_sfx(&first).unwrap();
        let b = generate_sfx(&second).unwrap();
        assert_eq!(a.duration, b.duration);
        assert_eq!(
            serde_json::to_value(a.root).unwrap(),
            serde_json::to_value(b.root).unwrap()
        );
    }
}

#[test]
fn invalid_requests_are_rejected_before_generation() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        for field in ["brightness", "punch", "variation"] {
            let mut spec = SfxSpec::new(SfxTemplate::Coin, 0);
            match field {
                "brightness" => spec.brightness = value,
                "punch" => spec.punch = value,
                _ => spec.variation = value,
            }
            assert!(generate_sfx(&spec).unwrap_err().to_string().contains(field));
        }
    }
    for sample_rate in [0, 7_999, 192_001, u32::MAX] {
        let mut spec = SfxSpec::new(SfxTemplate::Coin, 0);
        spec.sample_rate = sample_rate;
        assert!(
            generate_sfx(&spec)
                .unwrap_err()
                .to_string()
                .contains("sample_rate")
        );
    }
    let mut spec = SfxSpec::new(SfxTemplate::Coin, 0);
    spec.template_version = SFX_TEMPLATE_VERSION + 1;
    assert!(
        generate_sfx(&spec)
            .unwrap_err()
            .to_string()
            .contains("version")
    );
}
