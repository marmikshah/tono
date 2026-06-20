//! End-to-end checks of the MCP tool surface: author → edit → undo → export →
//! bank → save/replay. These run the real DSP and real files, not mocks.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use sonarium::review::Archetype;
use sonarium::server::{
    AddToBankReq, AuthorReq, CreateBankReq, EditReq, ExportBankReq, ExportReq, IdReq, MakeLoopReq,
    ReplaySessionReq, ReviewReq, SaveSessionReq, ScaffoldReq, SetParamReq, Sonarium, VariantsReq,
    rehydrate,
};
use sonarium::session::Store;

fn fresh(tag: &str) -> (Sonarium, PathBuf) {
    let dir = std::env::temp_dir()
        .join("sonarium_pipeline_test")
        .join(format!("{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(Store::new(dir.clone()).unwrap());
    (Sonarium::new(store), dir)
}

fn author_req(json: &str) -> Parameters<AuthorReq> {
    Parameters(serde_json::from_str(&format!(r#"{{ "graph": {json} }}"#)).unwrap())
}

const LASER: &str = r#"{
  "name": "laser_zap", "duration": 0.22, "root": {
    "type": "mix", "inputs": [
      { "type": "mul", "inputs": [
        { "type": "square", "duty": 0.25,
          "freq": { "slide": { "from": 880, "to": 180, "secs": 0.18, "curve": "exp" } } },
        { "type": "env", "a": 0.0, "d": 0.18, "s": 0.0, "r": 0.02, "punch": 0.3 } ] },
      { "type": "mul", "inputs": [
        { "type": "noise" },
        { "type": "env", "a": 0.0, "d": 0.04, "s": 0.0, "r": 0.0 } ] } ] } }"#;

#[tokio::test]
async fn scaffold_layered_sfx_builds_four_editable_layers() {
    let (srv, _dir) = fresh("scaffold");
    let res = srv
        .scaffold_layered_sfx(Parameters(ScaffoldReq {
            base_freq: Some(200.0),
            seed: Some(7),
            name: Some("impact_skeleton".into()),
        }))
        .await
        .unwrap();
    assert_eq!(result_id(&res), "impact_skeleton");

    let structured = res.structured_content.as_ref().unwrap();
    // Four named, band-disciplined layers in role order — a usable balance.
    let layers = structured["analysis"]["layers"].as_array().unwrap();
    let ids: Vec<&str> = layers.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["sub", "body", "top", "transient"]);
    assert!(structured["analysis"]["peak_dbfs"].as_f64().unwrap() <= 0.0);

    // The whole structure rides back in the result, exposed for editing, and
    // is stamped current (schema v2 per-layer streams + engine 1).
    let g = &structured["graph"];
    assert_eq!(g["version"], 2);
    assert_eq!(g["engine"], 1);
    assert_eq!(g["root"]["tracks"].as_array().unwrap().len(), 4);

    // base_freq out of range is rejected, not silently clamped.
    assert!(
        srv.scaffold_layered_sfx(Parameters(ScaffoldReq {
            base_freq: Some(0.0),
            seed: None,
            name: None,
        }))
        .await
        .is_err()
    );
}

#[tokio::test]
async fn review_sound_grades_through_the_server() {
    let (srv, _dir) = fresh("review");
    srv.scaffold_layered_sfx(Parameters(ScaffoldReq {
        base_freq: Some(200.0),
        seed: Some(1),
        name: Some("imp".into()),
    }))
    .await
    .unwrap();
    let rev = srv
        .review_sound(Parameters(ReviewReq {
            id: "imp".into(),
            archetype: Some(Archetype::Impact),
        }))
        .await
        .unwrap()
        .0;
    // The universal checks always run; the archetype adds its own.
    assert!(rev.findings.iter().any(|f| f.criterion == "peak"));
    assert!(rev.findings.iter().any(|f| f.criterion == "crest"));
    assert_eq!(
        rev.pass + rev.warn + rev.fail,
        rev.findings.len() as u32,
        "tally must match the findings"
    );
    // A non-pass finding always carries a concrete fix.
    for f in &rev.findings {
        assert_eq!(f.fix.is_empty(), format!("{:?}", f.status) == "Pass");
    }
    // A non-existent id is an error, not a panic.
    assert!(
        srv.review_sound(Parameters(ReviewReq {
            id: "nope".into(),
            archetype: None,
        }))
        .await
        .is_err()
    );
}

/// Extract the structured `id` from an authoring tool result.
fn result_id(res: &rmcp::model::CallToolResult) -> String {
    res.structured_content.as_ref().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Fetch a sound's current graph as JSON (via the public tool surface).
async fn graph_json(srv: &Sonarium, id: &str) -> serde_json::Value {
    let resp = srv
        .get_sound(Parameters(IdReq { id: id.into() }))
        .await
        .unwrap();
    serde_json::to_value(&resp.0.graph).unwrap()
}

#[tokio::test]
async fn author_renders_artifacts_and_analysis() {
    let (srv, dir) = fresh("author");
    let res = srv.author_sound(author_req(LASER)).await.unwrap();
    assert_eq!(result_id(&res), "laser_zap"); // slug id from the name

    // WAV + graph JSON + both feedback images land in the working dir.
    for f in [
        "laser_zap.wav",
        "laser_zap.json",
        "laser_zap.png",
        "laser_zap_wave.png",
    ] {
        assert!(dir.join(f).exists(), "missing {f}");
    }
    let a = &res.structured_content.as_ref().unwrap()["analysis"];
    assert!(a["peak_dbfs"].as_f64().unwrap() <= 0.0);
    assert!(a["spectral_centroid_hz"].as_f64().unwrap() > 0.0);
    // Summary text + two inline images.
    assert_eq!(res.content.len(), 3);
}

#[tokio::test]
async fn author_stamps_the_current_schema_version() {
    let (srv, _dir) = fresh("version_stamp");
    let res = srv.author_sound(author_req(LASER)).await.unwrap();
    let id = result_id(&res);
    let g = graph_json(&srv, &id).await;
    // LASER omits `version`; authoring resolves it to the current schema so
    // the stored doc (and the journaled step) pin their render semantics.
    assert_eq!(
        g["version"].as_u64().unwrap() as u32,
        sonarium::dsl::SCHEMA_VERSION
    );
}

#[tokio::test]
async fn edit_undo_redo_cycle_round_trips() {
    let (srv, _dir) = fresh("editing");
    srv.author_sound(author_req(LASER)).await.unwrap();

    // Surgical edit: retune the square's slide start.
    srv.set_param(Parameters(
        serde_json::from_str::<SetParamReq>(
            r#"{ "id": "laser_zap", "path": "root.inputs[0].inputs[0].freq.slide.from",
                 "value": 440 }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    let g = graph_json(&srv, "laser_zap").await;
    assert_eq!(
        g["root"]["inputs"][0]["inputs"][0]["freq"]["slide"]["from"],
        440.0
    );

    // Batch edit, then walk back and forward through history.
    srv.edit_sound(Parameters(
        serde_json::from_str::<EditReq>(
            r#"{ "id": "laser_zap", "ops": [
                 { "op": "remove", "path": "root.inputs[1]" } ] }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    let id = Parameters(IdReq {
        id: "laser_zap".into(),
    });
    srv.undo_sound(Parameters(IdReq {
        id: "laser_zap".into(),
    }))
    .await
    .unwrap();
    let h = srv.history(id).await.unwrap();
    assert_eq!((h.0.undo_depth, h.0.redo_depth), (1, 1));
    srv.redo_sound(Parameters(IdReq {
        id: "laser_zap".into(),
    }))
    .await
    .unwrap();
    let v = graph_json(&srv, "laser_zap").await;
    assert_eq!(v["root"]["inputs"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn export_writes_all_three_formats() {
    let (srv, dir) = fresh("export");
    srv.author_sound(author_req(LASER)).await.unwrap();
    for format in ["wav", "flac", "ogg"] {
        let req: ExportReq = serde_json::from_str(&format!(
            r#"{{ "id": "laser_zap", "format": "{format}",
                 "dest": "{}/out.{format}" }}"#,
            dir.display()
        ))
        .unwrap();
        let res = srv.export(Parameters(req)).await.unwrap();
        let written = PathBuf::from(&res.0.path);
        assert!(written.exists() && std::fs::metadata(&written).unwrap().len() > 0);
    }
}

#[tokio::test]
async fn bank_export_writes_manifest_and_engine_files() {
    let (srv, dir) = fresh("bank");
    srv.author_sound(author_req(LASER)).await.unwrap();
    srv.create_bank(Parameters(CreateBankReq { name: "SFX".into() }))
        .await
        .unwrap();
    srv.add_to_bank(Parameters(
        serde_json::from_str::<AddToBankReq>(
            r#"{ "bank_id": "sfx", "sound_id": "laser_zap", "category": "weapon" }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();

    let dest = dir.join("pack");
    let res = srv
        .export_bank(Parameters(
            serde_json::from_str::<ExportBankReq>(&format!(
                r#"{{ "bank_id": "sfx", "dest": "{}", "by_category": true, "engine": "godot" }}"#,
                dest.display()
            ))
            .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(res.0.count, 1);
    assert!(dest.join("sounds.json").exists());
    assert!(dest.join("weapon/laser_zap.wav").exists());
    assert_eq!(res.0.engine_files, vec!["weapon/laser_zap.wav.import"]);
    assert_eq!(res.0.entries[0].file, "weapon/laser_zap.wav");
}

#[tokio::test]
async fn make_loop_reports_seam_and_marks_wav() {
    let (srv, dir) = fresh("loop");
    srv.author_sound(author_req(
        r#"{ "name": "bed", "duration": 1.0, "root": { "type": "chain", "stages": [
            { "type": "noise", "color": "pink" },
            { "type": "lowpass", "cutoff": 700 } ] } }"#,
    ))
    .await
    .unwrap();
    let res = srv
        .make_loop(Parameters(
            serde_json::from_str::<MakeLoopReq>(r#"{ "id": "bed", "crossfade_secs": 0.25 }"#)
                .unwrap(),
        ))
        .await
        .unwrap();
    // Last content item is the loop report.
    let report = res.content.last().unwrap().as_text().unwrap();
    assert!(report.text.contains("seam discontinuity"));
    // The WAV now carries a smpl chunk.
    let bytes = std::fs::read(dir.join("bed.wav")).unwrap();
    assert!(bytes.windows(4).any(|w| w == b"smpl"));
}

#[tokio::test]
async fn variants_are_distinct_and_level_matched() {
    let (srv, _dir) = fresh("variants");
    srv.author_sound(author_req(LASER)).await.unwrap();
    let res = srv
        .generate_variants(Parameters(
            serde_json::from_str::<VariantsReq>(
                r#"{ "id": "laser_zap", "count": 3, "seed": 9, "target_lufs": -16 }"#,
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(res.0.count, 3);
    let ids: Vec<&str> = res.0.variants.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids.len(), 3);
    for v in &res.0.variants {
        assert!(
            (v.loudness_lufs - (-16.0)).abs() < 2.0,
            "level-matched, got {}",
            v.loudness_lufs
        );
    }
}

#[tokio::test]
async fn restart_rehydrates_the_library() {
    let (srv, dir) = fresh("rehydrate");
    srv.author_sound(author_req(LASER)).await.unwrap();
    drop(srv);

    // A brand-new store over the same dir: empty until rehydrated.
    let store = Arc::new(Store::new(dir).unwrap());
    assert!(store.get("laser_zap").is_none());
    let restored = rehydrate(&store);
    assert_eq!(restored, 1);
    assert!(store.get("laser_zap").is_some());
}

#[tokio::test]
async fn shipped_example_session_replays_clean() {
    let (srv, dir) = fresh("example");
    let example =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples/laser-session.json");
    let res = srv
        .replay_session(Parameters(ReplaySessionReq {
            path: example.to_string_lossy().into_owned(),
        }))
        .await
        .unwrap();
    assert_eq!(res.0.applied, 6);
    // The session builds a 4-sound library and a 2-member bank.
    for f in ["laser_zap.wav", "laser_zap_mut.wav", "laser_zap_mut_3.wav"] {
        assert!(dir.join(f).exists(), "missing {f}");
    }
    let banks = srv.list_banks().await;
    assert_eq!(banks.0.banks[0].id, "blaster_pack");
    assert_eq!(banks.0.banks[0].members.len(), 2);
}

#[tokio::test]
async fn river_flows_showcase_session_replays() {
    // A real piece of music — Yiruma's "River Flows in You", complete, 800
    // notes converted from MIDI (tempo map + sustain pedal intact) onto the
    // piano instrument — replays from its session file alone.
    let (srv, dir) = fresh("river");
    let example =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples/river-flows-in-you.json");
    let res = srv
        .replay_session(Parameters(ReplaySessionReq {
            path: example.to_string_lossy().into_owned(),
        }))
        .await
        .unwrap();
    assert_eq!(res.0.applied, 1);
    let g = graph_json(&srv, "river_flows_in_you").await;
    let seq = &g["root"]["stages"][0];
    assert_eq!(seq["wave"], "piano");
    assert_eq!(seq["notes"].as_array().unwrap().len(), 800);
    assert!(g["duration"].as_f64().unwrap() > 160.0); // the whole piece
    assert!(dir.join("river_flows_in_you.wav").exists());
}

#[tokio::test]
async fn band_demo_session_replays_with_four_instruments() {
    // The instrument set playing together: kit + bass + epiano + strings
    // through a compressor and reverb — a band from one author_sound call.
    let (srv, dir) = fresh("band");
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples/band-demo.json");
    srv.replay_session(Parameters(ReplaySessionReq {
        path: example.to_string_lossy().into_owned(),
    }))
    .await
    .unwrap();
    let g = graph_json(&srv, "band_demo").await;
    // The band sits on the mixing console: four panned tracks + a master bus.
    let tracks = g["root"]["tracks"].as_array().unwrap();
    let waves: Vec<&str> = tracks
        .iter()
        .map(|t| t["node"]["wave"].as_str().unwrap())
        .collect();
    assert_eq!(waves, vec!["kit", "bass", "epiano", "strings"]);
    assert!(g["root"]["master"].as_array().unwrap().len() == 2);
    // A mixer document writes a true stereo file.
    let reader = hound::WavReader::open(dir.join("band_demo.wav")).unwrap();
    assert_eq!(reader.spec().channels, 2);
}

#[tokio::test]
async fn bgm_showcases_replay_as_seamless_mixer_loops() {
    // The three game-BGM showcases: each is a tracks-root mixer document
    // rendered loop-ready (the WAV carries a smpl chunk engines read).
    for (recipe, id) in [
        ("evening-glade.json", "evening_glade"),
        ("iron-gauntlet.json", "iron_gauntlet"),
        ("sunny-steps.json", "sunny_steps"),
    ] {
        let (srv, dir) = fresh(&format!("bgm_{id}"));
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("docs/examples")
            .join(recipe);
        srv.replay_session(Parameters(ReplaySessionReq {
            path: example.to_string_lossy().into_owned(),
        }))
        .await
        .unwrap();
        let g = graph_json(&srv, id).await;
        assert_eq!(g["root"]["type"], "tracks", "{id} mixes on the console");
        assert_eq!(g["playback"]["mode"], "loop", "{id} ships as a loop");
        let bytes = std::fs::read(dir.join(format!("{id}.wav"))).unwrap();
        assert!(
            bytes.windows(4).any(|w| w == b"smpl"),
            "{id} WAV carries the loop chunk"
        );
    }
    // The boss track's bass riff ducks under its own kick.
    let (srv, _dir) = fresh("bgm_duck_check");
    let example =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples/iron-gauntlet.json");
    srv.replay_session(Parameters(ReplaySessionReq {
        path: example.to_string_lossy().into_owned(),
    }))
    .await
    .unwrap();
    let g = graph_json(&srv, "iron_gauntlet").await;
    let bass = &g["root"]["tracks"][1]["node"];
    assert_eq!(
        bass["stages"].as_array().unwrap().last().unwrap()["type"],
        "duck"
    );
}

#[tokio::test]
async fn every_example_recipe_replays() {
    // The whole docs/examples library is executable documentation: every
    // recipe must replay clean into a fresh session. (Individual showcases
    // also have focused assertions in their own tests above.)
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let (srv, _d) = fresh(&format!("sweep_{stem}"));
        let res = srv
            .replay_session(Parameters(ReplaySessionReq {
                path: path.to_string_lossy().into_owned(),
            }))
            .await
            .unwrap_or_else(|e| panic!("{stem} failed to replay: {e}"));
        assert!(res.0.applied >= 1, "{stem} applied no steps");
        count += 1;
    }
    assert!(
        count >= 9,
        "expected the full recipe library, found {count}"
    );
}

#[tokio::test]
async fn layered_authoring_flow_round_trips_and_replays() {
    use sonarium::server::{AddLayerReq, LayerOpsReq, SetLayerReq};

    let (a, dir_a) = fresh("layers_a");
    a.author_sound(author_req(LASER)).await.unwrap();

    // First add_layer wraps the plain root as layer "laser_zap" and stacks
    // a sub layer next to it.
    let res = a
        .add_layer(Parameters(
            serde_json::from_str::<AddLayerReq>(
                r#"{ "id": "laser_zap", "layer": "sub",
                     "node": { "type": "mul", "inputs": [
                        { "type": "sine", "freq": 80 },
                        { "type": "env", "d": 0.15 } ] },
                     "gain": 0.6 }"#,
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    let texts: Vec<String> = res
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("wrapped as layer 'laser_zap'")),
        "the wrap must be announced: {texts:?}"
    );

    // Duplicate layers are rejected with the listing.
    let err = a
        .add_layer(Parameters(
            serde_json::from_str::<AddLayerReq>(
                r#"{ "id": "laser_zap", "layer": "sub", "node": { "type": "noise" } }"#,
            )
            .unwrap(),
        ))
        .await
        .unwrap_err();
    assert!(
        err.contains("already exists") && err.contains("laser_zap, sub"),
        "{err}"
    );

    // The describe map speaks layers now.
    let desc = a
        .describe_sound(Parameters(IdReq {
            id: "laser_zap".into(),
        }))
        .await
        .unwrap();
    let ids: Vec<&str> = desc.0.layers.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, vec!["laser_zap", "sub"]);

    // Layer-relative set_param; absolute paths with `layer` are rejected.
    a.set_param(Parameters(
        serde_json::from_str::<SetParamReq>(
            r#"{ "id": "laser_zap", "layer": "sub", "path": "inputs[0].freq", "value": 60 }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    let err = a
        .set_param(Parameters(
            serde_json::from_str::<SetParamReq>(
                r#"{ "id": "laser_zap", "layer": "sub",
                     "path": "root.tracks[1].node.inputs[0].freq", "value": 60 }"#,
            )
            .unwrap(),
        ))
        .await
        .unwrap_err();
    assert!(err.contains("drop the 'root.tracks[..]' prefix"), "{err}");
    let err = a
        .set_param(Parameters(
            serde_json::from_str::<SetParamReq>(
                r#"{ "id": "laser_zap", "layer": "sub", "path": "gain", "value": 0.5 }"#,
            )
            .unwrap(),
        ))
        .await
        .unwrap_err();
    assert!(err.contains("set_layer"), "{err}");

    // Mixer moves + structural ops.
    a.set_layer(Parameters(
        serde_json::from_str::<SetLayerReq>(
            r#"{ "id": "laser_zap", "layer": "sub", "gain": 0.4, "at": 0.02 }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    a.layer_ops(Parameters(
        serde_json::from_str::<LayerOpsReq>(
            r#"{ "id": "laser_zap", "op": "duplicate", "layer": "sub", "new_id": "sub_b" }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    a.layer_ops(Parameters(
        serde_json::from_str::<LayerOpsReq>(
            r#"{ "id": "laser_zap", "op": "remove", "layer": "sub_b" }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();

    // A failing mixer move leaves history and redo exactly as they were —
    // no spurious no-op revision, no destroyed redo stack.
    let before = a
        .history(Parameters(IdReq {
            id: "laser_zap".into(),
        }))
        .await
        .unwrap();
    let err = a
        .set_layer(Parameters(
            serde_json::from_str::<SetLayerReq>(
                r#"{ "id": "laser_zap", "layer": "sub", "gain": 5.0 }"#,
            )
            .unwrap(),
        ))
        .await
        .unwrap_err();
    assert!(err.contains("gain must be in [0, 2]"), "{err}");
    let after = a
        .history(Parameters(IdReq {
            id: "laser_zap".into(),
        }))
        .await
        .unwrap();
    assert_eq!(
        (before.0.undo_depth, before.0.redo_depth),
        (after.0.undo_depth, after.0.redo_depth),
        "failed edits must not touch history"
    );

    // The whole layered flow replays byte-identically in a fresh dir.
    let saved = a
        .save_session(Parameters(SaveSessionReq { dest: None }))
        .await
        .unwrap();
    let (b, dir_b) = fresh("layers_b");
    b.replay_session(Parameters(ReplaySessionReq { path: saved.0.path }))
        .await
        .unwrap();
    let wav_a = std::fs::read(dir_a.join("laser_zap.wav")).unwrap();
    let wav_b = std::fs::read(dir_b.join("laser_zap.wav")).unwrap();
    assert_eq!(wav_a, wav_b, "layered session must replay byte-identically");
}

#[tokio::test]
async fn replayed_session_reproduces_audio_byte_for_byte() {
    // Session A: author, surgically edit, mutate (explicit seed), bank it.
    let (a, dir_a) = fresh("replay_a");
    a.author_sound(author_req(LASER)).await.unwrap();
    a.set_param(Parameters(
        serde_json::from_str::<SetParamReq>(
            r#"{ "id": "laser_zap", "path": "root.inputs[0].inputs[0].duty", "value": 0.5 }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    a.mutate_sound(Parameters(
        serde_json::from_str(r#"{ "id": "laser_zap", "amount": 0.2, "seed": 7 }"#).unwrap(),
    ))
    .await
    .unwrap();
    a.create_bank(Parameters(CreateBankReq { name: "SFX".into() }))
        .await
        .unwrap();
    a.add_to_bank(Parameters(
        serde_json::from_str::<AddToBankReq>(
            r#"{ "bank_id": "sfx", "sound_id": "laser_zap_mut" }"#,
        )
        .unwrap(),
    ))
    .await
    .unwrap();
    let saved = a
        .save_session(Parameters(SaveSessionReq { dest: None }))
        .await
        .unwrap();
    assert_eq!(saved.0.steps, 5);

    // Session B: a fresh working directory, replay the saved file.
    let (b, dir_b) = fresh("replay_b");
    let res = b
        .replay_session(Parameters(ReplaySessionReq { path: saved.0.path }))
        .await
        .unwrap();
    assert_eq!(res.0.applied, 5);

    // Same sounds, same bank, byte-identical audio.
    for f in ["laser_zap.wav", "laser_zap_mut.wav"] {
        let wav_a = std::fs::read(dir_a.join(f)).unwrap();
        let wav_b = std::fs::read(dir_b.join(f)).unwrap();
        assert_eq!(wav_a, wav_b, "{f} must replay byte-identically");
    }
    assert!(dir_b.join("bank_sfx.json").exists());

    // The replayed session's journal mirrors the applied steps exactly (no
    // re-journaling doubling) — saving it again yields the same 5 steps.
    let resaved = b
        .save_session(Parameters(SaveSessionReq { dest: None }))
        .await
        .unwrap();
    assert_eq!(resaved.0.steps, 5);
}

#[tokio::test]
async fn replay_refuses_a_non_fresh_session() {
    // Build a session and save it.
    let (a, _dir_a) = fresh("guard_src");
    a.author_sound(author_req(LASER)).await.unwrap();
    let saved = a
        .save_session(Parameters(SaveSessionReq { dest: None }))
        .await
        .unwrap();

    // A session that already has content must refuse the replay outright —
    // ids derive from names, so replaying over existing sounds would silently
    // edit the wrong targets.
    let (b, _dir_b) = fresh("guard_dst");
    b.author_sound(author_req(LASER)).await.unwrap();
    let err = b
        .replay_session(Parameters(ReplaySessionReq {
            path: saved.0.path.clone(),
        }))
        .await
        .err()
        .expect("replay into a non-fresh session must be refused");
    assert!(err.contains("fresh session"), "{err}");

    // Replaying a session's own live journal is refused for the same reason.
    let err = a
        .replay_session(Parameters(ReplaySessionReq { path: saved.0.path }))
        .await
        .err()
        .expect("self-replay must be refused");
    assert!(err.contains("fresh session"), "{err}");
}
