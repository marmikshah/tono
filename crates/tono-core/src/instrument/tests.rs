use super::*;
use crate::dsl::Adsr;
use crate::patch::Patch;

fn saw_patch() -> Patch {
    // A sustaining subtractive voice with a `pitch` param on the oscillator.
    serde_json::from_str(
        r#"{ "doc": { "name":"lead", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":220 },
                { "type":"lowpass", "cutoff":1800, "q":0.8 } ] } },
             "params": [ { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
    )
    .unwrap()
}

fn peak(b: &[f32]) -> f32 {
    b.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
}

fn bits(b: &[f32]) -> Vec<u32> {
    b.iter().map(|x| x.to_bits()).collect()
}

#[test]
fn note_maths() {
    assert!((Note::A4.freq() - 440.0).abs() < 1e-3);
    assert!((Note::C4.freq() - 261.6256).abs() < 1e-2);
    assert_eq!(Note::parse("A4"), Some(Note::A4));
    assert_eq!(Note::parse("midi:60"), Some(Note::C4));
    assert_eq!(Note::C4.transpose(12), Note(72));
}

#[test]
fn plays_polyphonic_pitched_notes() {
    let mut inst = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    // A three-note chord.
    inst.note_on(Note::C4, 0.9);
    inst.note_on(Note(64), 0.8); // E4
    inst.note_on(Note(67), 0.7); // G4
    assert_eq!(inst.active_voices(), 3);

    let mut out = vec![0.0f32; 512 * 2];
    assert_eq!(inst.fill(&mut out), 512);
    assert!(peak(&out) > 0.0, "chord makes sound");
    // Mono duplicated to stereo.
    assert!((0..512).all(|f| out[f * 2] == out[f * 2 + 1]));
}

#[test]
fn note_off_releases_then_culls() {
    let amp = Adsr {
        a: 0.001,
        d: 0.001,
        s: 0.8,
        r: 0.01,
        punch: 0.0,
    };
    let design = InstrumentDesign::new(saw_patch()).with_amp(amp);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    inst.note_on(Note::A4, 1.0);
    assert_eq!(inst.active_voices(), 1);
    // Let attack/decay settle, then release.
    let mut out = vec![0.0f32; 256 * 2];
    inst.fill(&mut out);
    inst.note_off(Note::A4);
    // Serve well past the 10 ms release (480 frames) so it culls.
    let mut tail = vec![0.0f32; 2048 * 2];
    inst.fill(&mut tail);
    assert_eq!(inst.active_voices(), 0, "released voice is culled");
}

#[test]
fn transpose_makes_any_sound_playable() {
    // A bare saw with no pitch param — playable via transposition.
    let patch: Patch = serde_json::from_str(
        r#"{ "doc": { "name":"buzz", "duration":1.0, "engine":2, "root": { "type":"sawtooth", "freq":220 } } }"#,
    )
    .unwrap();
    let design = InstrumentDesign::new(patch); // no "pitch" param ⇒ Transpose
    assert!(matches!(design.pitch, PitchMap::Transpose { .. }));
    let mut inst = Instrument::new(design, 48_000).unwrap();
    inst.note_on(Note::C4, 1.0);
    inst.note_on(Note(72), 1.0); // an octave up
    let mut out = vec![0.0f32; 256 * 2];
    inst.fill(&mut out);
    assert!(peak(&out) > 0.0);
}

#[test]
fn pitch_bend_repitches_live() {
    // Bending A4 up an octave is a pure repitch, so it is bit-for-bit A5
    // struck plain (same oscillator phase increment, same baked filter).
    let mut a = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    a.note_on(Note::A4, 1.0);
    a.set_bend(12.0);
    let mut b = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    b.note_on(Note(81), 1.0); // A5
    let (mut oa, mut ob) = (vec![0.0f32; 2048], vec![0.0f32; 2048]);
    a.fill(&mut oa);
    b.fill(&mut ob);
    assert_eq!(bits(&oa), bits(&ob), "A4 + octave bend == A5");
}

#[test]
fn centered_bend_is_a_no_op() {
    let mut bent = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    bent.note_on(Note::C4, 0.8);
    bent.set_bend(0.0); // dead center
    let mut plain = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    plain.note_on(Note::C4, 0.8);
    let (mut ob, mut op) = (vec![0.0f32; 1024], vec![0.0f32; 1024]);
    bent.fill(&mut ob);
    plain.fill(&mut op);
    assert_eq!(bits(&ob), bits(&op), "a centered wheel changes nothing");
}

#[test]
fn brightness_sweeps_the_voice_filter() {
    let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
    let mut bright = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    bright.note_on(Note::C4, 0.9);
    let mut a = vec![0.0f32; 1024 * 2];
    bright.fill(&mut a);

    // A voice struck after the knob is turned down is darker.
    let mut dark = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    dark.set_brightness(0.15);
    dark.note_on(Note::C4, 0.9);
    let mut b = vec![0.0f32; 1024 * 2];
    dark.fill(&mut b);
    assert!(rms(&b) < rms(&a), "lower brightness darkens a new note");

    // Turning the knob down on the already-sounding bright voice darkens it too.
    bright.set_brightness(0.15);
    let mut c = vec![0.0f32; 1024 * 2];
    bright.fill(&mut c);
    assert!(
        rms(&c) < rms(&a),
        "live brightness sweep darkens a held note"
    );
}

#[test]
fn vibrato_moves_the_sound_wobble_still_sounds() {
    // Vibrato bends the pitch over the block, so it diverges from a dry note.
    let mut dry = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    let mut vib = Instrument::new(
        InstrumentDesign::new(saw_patch()).with_vibrato(6.0, 50.0),
        48_000,
    )
    .unwrap();
    dry.note_on(Note::A4, 1.0);
    vib.note_on(Note::A4, 1.0);
    let (mut a, mut b) = (vec![0.0f32; 4096 * 2], vec![0.0f32; 4096 * 2]);
    dry.fill(&mut a);
    vib.fill(&mut b);
    assert!(bits(&a) != bits(&b), "vibrato changes the sound");
    assert!(peak(&b) > 0.0);

    // Filter wobble is live on a filtered voice — it makes sound.
    let mut wob = Instrument::new(
        InstrumentDesign::new(saw_patch()).with_wobble(4.0, 1.5),
        48_000,
    )
    .unwrap();
    wob.note_on(Note::C4, 0.9);
    let mut w = vec![0.0f32; 4096 * 2];
    wob.fill(&mut w);
    assert!(peak(&w) > 0.0, "wobble instrument sounds");
}

#[test]
fn voice_stealing_caps_polyphony() {
    let design = InstrumentDesign::new(saw_patch()).with_max_voices(4);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    for n in 60..70 {
        inst.note_on(Note(n), 0.8);
    }
    // Stolen voices ramp out over ~5 ms instead of hard-cutting; once the
    // declick window has rendered, the pool is back at the cap.
    let mut out = vec![0.0f32; 1024 * 2];
    inst.fill(&mut out);
    assert_eq!(
        inst.active_voices(),
        4,
        "capped at max_voices once steals declick"
    );
}

#[test]
fn voice_stealing_ramps_instead_of_cutting() {
    // One sustained voice at full level, pool of one: the next note must
    // fade the victim out, not step it to silence mid-sample.
    let amp = Adsr {
        a: 0.001,
        d: 0.0,
        s: 1.0,
        r: 0.3,
        punch: 0.0,
    };
    let design = InstrumentDesign::new(saw_patch())
        .with_amp(amp)
        .with_max_voices(1);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    inst.note_on(Note::A4, 1.0);
    let mut warm = vec![0.0f32; 512 * 2];
    inst.fill(&mut warm); // the voice reaches full sustain
    inst.note_on(Note::C4, 1.0); // pool full: steals the A4
    let mut fade = vec![0.0f32; 512 * 2];
    inst.fill(&mut fade);
    let mut max_jump = 0.0f32;
    let mut prev = warm[warm.len() - 2];
    for f in 0..512 {
        max_jump = max_jump.max((fade[f * 2] - prev).abs());
        prev = fade[f * 2];
    }
    // A saw at full level steps by up to ~2.0 when hard-cut at the wrap;
    // the 5 ms ramp keeps adjacent samples close.
    assert!(max_jump < 0.5, "steal clicked: max sample jump {max_jump}");
}

#[test]
fn note_names_round_trip_and_convert() {
    assert_eq!(Note::A4.to_string(), "A4");
    assert_eq!(Note::C4.to_string(), "C4");
    assert_eq!("F#3".parse::<Note>().unwrap().to_string(), "F#3");
    assert_eq!(Note::from(60u8), Note::C4);
    assert!("nonsense".parse::<Note>().is_err());
}

#[test]
fn transpose_scales_all_pitched_nodes() {
    // Regression: modal mode freqs and the ring-mod carrier must transpose too.
    let mut doc: SoundDoc = serde_json::from_str(
        r#"{ "name":"b", "duration":0.1, "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":100 },
            { "type":"ringmod", "freq":200 },
            { "type":"modal", "modes":[{ "freq":300, "decay":0.3, "gain":1.0 }], "mix":0.5 } ] } }"#,
    )
    .unwrap();
    transpose(&mut doc.root, 2.0);
    let v = serde_json::to_value(&doc).unwrap();
    let stages = &v["root"]["stages"];
    assert_eq!(stages[0]["freq"], 200.0);
    assert_eq!(stages[1]["freq"], 400.0);
    assert_eq!(stages[2]["modes"][0]["freq"], 600.0);
}

#[test]
fn handles_and_note_off_count() {
    let mut inst = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    let h = inst.note_on(Note::C4, 0.9);
    assert!(inst.is_active(h));
    assert_eq!(inst.voice_note(h), Some(Note::C4));
    assert_eq!(inst.note_off(Note::C4), 1);
    assert_eq!(inst.note_off(Note::C4), 0, "already releasing");
    assert_eq!(inst.note_off(Note(80)), 0, "no such note");
}

#[test]
fn set_param_validates_and_note_on_never_panics() {
    let mut inst = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    assert!(!inst.set_param("nope", 1.0), "unknown param rejected");
    assert!(inst.set_param("pitch", 300.0), "valid value accepted");
    // note_on must never panic on a control event.
    inst.note_on(Note::A4, 1.0);
    assert_eq!(inst.active_voices(), 1);
}

#[test]
fn percussive_voice_culls_without_note_off() {
    // sustain = 0 ⇒ a one-shot fired via note_on only must not leak voices.
    let amp = Adsr {
        a: 0.001,
        d: 0.02,
        s: 0.0,
        r: 0.05,
        punch: 0.0,
    };
    let design = InstrumentDesign::new(saw_patch()).with_amp(amp);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    inst.note_on(Note::C4, 1.0);
    let mut out = vec![0.0f32; 2048 * 2];
    inst.fill(&mut out); // past the ~20 ms decay to silence
    assert_eq!(
        inst.active_voices(),
        0,
        "percussive one-shot reclaims its voice"
    );
}

#[test]
fn master_reverb_tail_outlives_the_voice() {
    let reverb: Node =
        serde_json::from_str(r#"{ "type":"reverb", "room":0.8, "mix":0.6 }"#).unwrap();
    let design = InstrumentDesign::new(saw_patch()).with_master(vec![reverb]);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    inst.note_on(Note::A4, 1.0);
    let mut out = vec![0.0f32; 256 * 2];
    inst.fill(&mut out);
    inst.all_notes_off();
    for _ in 0..40 {
        inst.fill(&mut out); // let the ~120 ms release finish and cull the voice
    }
    assert_eq!(inst.active_voices(), 0);
    // The one shared master reverb still rings after the voice is gone.
    let mut tail = vec![0.0f32; 256 * 2];
    inst.fill(&mut tail);
    assert!(
        peak(&tail) > 0.0,
        "shared reverb tail continues past the note"
    );
}

#[test]
fn sustain_pedal_defers_release() {
    let amp = Adsr {
        a: 0.001,
        d: 0.001,
        s: 0.8,
        r: 0.01,
        punch: 0.0,
    };
    let design = InstrumentDesign::new(saw_patch()).with_amp(amp);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    inst.set_sustain(true);
    inst.note_on(Note::A4, 1.0);
    let mut out = vec![0.0f32; 256 * 2];
    inst.fill(&mut out);
    inst.note_off(Note::A4); // deferred by the pedal
    for _ in 0..40 {
        inst.fill(&mut out);
    }
    assert_eq!(inst.active_voices(), 1, "held by the sustain pedal");
    inst.set_sustain(false); // pedal up → release
    for _ in 0..40 {
        inst.fill(&mut out);
    }
    assert_eq!(inst.active_voices(), 0, "released on pedal-up");
}

#[test]
fn unison_spreads_detuned_copies_across_stereo() {
    let design = InstrumentDesign::new(saw_patch()).with_unison(4, 22.0, 1.0);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    inst.note_on(Note::C4, 0.9);
    let mut out = vec![0.0f32; 2048 * 2];
    inst.fill(&mut out);
    assert!(peak(&out) > 0.0, "unison makes sound");
    // Detuned copies panned L/R decorrelate the channels.
    let differs = (0..2048).any(|f| out[f * 2] != out[f * 2 + 1]);
    assert!(differs, "unison + width produces a stereo image");
}

#[test]
fn no_unison_stays_centered_mono() {
    // The default (one copy) must remain identical L/R — no silent regression
    // from the stereo bus.
    let mut inst = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
    inst.note_on(Note::C4, 0.9);
    let mut out = vec![0.0f32; 512 * 2];
    inst.fill(&mut out);
    assert!((0..512).all(|f| out[f * 2] == out[f * 2 + 1]), "centered");
}

#[test]
fn mono_mode_reuses_one_voice_with_last_note_priority() {
    let design = InstrumentDesign::new(saw_patch()).with_mode(PlayMode::Mono { legato: true });
    let mut inst = Instrument::new(design, 48_000).unwrap();
    let h = inst.note_on(Note::C4, 0.9);
    inst.note_on(Note(64), 0.9); // E4 — retunes the one voice
    assert_eq!(inst.active_voices(), 1, "mono holds a single voice");
    assert_eq!(
        inst.voice_note(h),
        Some(Note(64)),
        "voice follows the new note"
    );
    assert_eq!(inst.note_off(Note(64)), 1);
    assert_eq!(
        inst.voice_note(h),
        Some(Note::C4),
        "last-note priority: falls back to still-held C4"
    );
    assert_eq!(inst.active_voices(), 1);
    assert_eq!(inst.note_off(Note::C4), 1);
    for _ in 0..4 {
        inst.fill(&mut vec![0.0f32; 4096 * 2]); // past the release
    }
    assert_eq!(inst.active_voices(), 0, "released once nothing is held");
}

#[test]
fn mono_glide_eases_between_notes() {
    let design = InstrumentDesign::new(saw_patch())
        .with_mode(PlayMode::Mono { legato: true })
        .with_glide(0.1);
    let mut inst = Instrument::new(design, 48_000).unwrap();
    let h = inst.note_on(Note::C4, 0.9); // built at C4, scale 1.0
    assert_eq!(inst.voice_pitch_scale(h), Some(1.0));
    inst.note_on(Note(72), 0.9); // C5, an octave up ⇒ target scale 2.0
    let mut blk = vec![0.0f32; 64 * 2];
    inst.fill(&mut blk);
    let p = inst.voice_pitch_scale(h).unwrap();
    assert!(p > 1.0 && p < 1.5, "eases up rather than jumping: {p}");
    for _ in 0..6 {
        inst.fill(&mut vec![0.0f32; 4096 * 2]);
    }
    assert!(
        inst.voice_pitch_scale(h).unwrap() > 1.9,
        "arrives near the octave"
    );
}

#[test]
fn design_round_trips_through_serde() {
    let design = InstrumentDesign::new(saw_patch());
    let json = serde_json::to_string(&design).unwrap();
    let recalled: InstrumentDesign = serde_json::from_str(&json).unwrap();
    assert!(
        Instrument::new(recalled, 48_000).is_ok(),
        "preset recall works"
    );
}
