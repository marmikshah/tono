//! End-to-end discovery, batch export, preservation, and stereo regression.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use tono_core::{
    dsl::SoundDoc,
    generate::{SfxSpec, SfxTemplate, generate_sfx},
    render,
};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tono-generation-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tono"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn discovery_lists_all_generatable_starters() {
    let scratch = Scratch::new();
    let output = scratch.run(&["templates"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 8);
    for template in SfxTemplate::ALL {
        assert!(text.lines().any(|line| line.starts_with(template.id())));
    }
}

#[test]
fn default_batch_exports_replayable_documents_audio_and_manifest() {
    let scratch = Scratch::new();
    let output = scratch.run(&["generate", "coin", "--seed", "42"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dir = scratch.0.join("target/generated/coin-42");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["manifest_version"], 1);
    let preview = fs::read_to_string(dir.join(manifest["preview"].as_str().unwrap())).unwrap();
    assert_eq!(preview.matches("<audio ").count(), 4);
    let candidates = manifest["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 4);
    for (index, candidate) in candidates.iter().enumerate() {
        let spec: SfxSpec = serde_json::from_value(candidate["spec"].clone()).unwrap();
        assert_eq!(spec.seed, 42 + index as u64);
        let document = dir.join(candidate["document"].as_str().unwrap());
        assert!(preview.contains(candidate["document"].as_str().unwrap()));
        assert!(preview.contains(candidate["audio"].as_str().unwrap()));
        let doc: SoundDoc = serde_json::from_slice(&fs::read(document).unwrap()).unwrap();
        let replay = generate_sfx(&spec).unwrap();
        assert_eq!(
            serde_json::to_value(&doc).unwrap(),
            serde_json::to_value(&replay).unwrap()
        );
        assert_eq!(
            candidate["document_hash"].as_u64().unwrap(),
            tono_core::program::content_hash(&doc)
        );
        let mut wav =
            hound::WavReader::open(dir.join(candidate["audio"].as_str().unwrap())).unwrap();
        assert_eq!(wav.spec().channels, 2);
        assert_eq!(wav.spec().sample_rate, 48_000);
        let actual = wav.samples::<i16>().map(Result::unwrap).collect::<Vec<_>>();
        let expected = render::render(&replay)
            .iter()
            .flat_map(|sample| [pcm(*sample); 2])
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        for field in ["spectrogram", "waveform", "stats"] {
            assert!(dir.join(candidate[field].as_str().unwrap()).is_file());
        }
    }
    let before = fs::read(dir.join("coin_v0.wav")).unwrap();
    let retry = scratch.run(&["generate", "coin", "--seed", "42"]);
    assert!(!retry.status.success());
    assert!(String::from_utf8_lossy(&retry.stderr).contains("not empty"));
    assert_eq!(before, fs::read(dir.join("coin_v0.wav")).unwrap());
}

#[test]
fn invalid_requests_leave_no_output_directory() {
    let scratch = Scratch::new();
    for args in [
        vec!["generate", "unknown", "-o", "out"],
        vec!["generate", "coin", "--brightness", "NaN", "-o", "out"],
        vec!["generate", "coin", "--punch", "-0.1", "-o", "out"],
        vec!["generate", "coin", "--variation", "1.1", "-o", "out"],
        vec!["generate", "coin", "--sample-rate", "7999", "-o", "out"],
        vec!["generate", "coin", "--count", "33", "-o", "out"],
        vec!["generate", "coin", "--count", "0", "-o", "out"],
        vec!["generate", "coin", "--format", "mp3", "-o", "out"],
    ] {
        let output = scratch.run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(!scratch.0.join("out").exists(), "{args:?}");
    }
}

#[test]
fn compressed_exports_record_their_format_and_wrap_seeds() {
    let scratch = Scratch::new();
    for (format, header) in [("flac", b"fLaC"), ("ogg", b"OggS")] {
        let output = scratch.run(&[
            "generate",
            "ui-confirm",
            "--seed",
            "18446744073709551615",
            "--count",
            "2",
            "--sample-rate",
            "8000",
            "--format",
            format,
            "-o",
            format,
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let dir = scratch.0.join(format);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        for (index, seed) in [u64::MAX, 0].into_iter().enumerate() {
            let candidate = &manifest["candidates"][index];
            assert_eq!(candidate["spec"]["seed"].as_u64(), Some(seed));
            let filename = candidate["audio"].as_str().unwrap();
            assert!(filename.ends_with(format));
            let bytes = fs::read(dir.join(filename)).unwrap();
            assert_eq!(&bytes[..4], header);
        }
    }
}

#[test]
fn a_plain_document_exports_its_stereo_treatment() {
    let scratch = Scratch::new();
    let mut doc = generate_sfx(&SfxSpec::new(SfxTemplate::Coin, 42)).unwrap();
    doc.name = "stereo".into();
    for stereo in [
        tono_core::dsl::Stereo::Haas { ms: 12.0, pan: 0.5 },
        tono_core::dsl::Stereo::Wide { amount: 0.7 },
    ] {
        doc.stereo = stereo;
        fs::write(
            scratch.0.join("stereo.json"),
            serde_json::to_string(&doc).unwrap(),
        )
        .unwrap();
        let output = scratch.run(&["render", "stereo.json", "-o", "rendered"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (left, right) = render::stereoize(&render::render(&doc), doc.stereo, doc.sample_rate);
        let mut wav = hound::WavReader::open(scratch.0.join("rendered/stereo.wav")).unwrap();
        let actual = wav.samples::<i16>().map(Result::unwrap).collect::<Vec<_>>();
        let expected = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [pcm(*l), pcm(*r)])
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert!(actual.chunks_exact(2).any(|frame| frame[0] != frame[1]));
    }
}

fn pcm(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}
