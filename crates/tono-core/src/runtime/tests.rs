use super::engine::balance;
use super::ring::SampleRing;
use super::*;
use crate::dsl::Node;
use crate::dsl::SoundDoc;
use crate::patch::Patch;

fn doc(duration: f32) -> SoundDoc {
    serde_json::from_str(&format!(
        r#"{{ "name": "t", "duration": {duration}, "root": {{ "type": "sine", "freq": 440 }} }}"#
    ))
    .unwrap()
}

fn pitch_patch() -> Patch {
    serde_json::from_str(
        r#"{ "doc": { "name": "t", "duration": 0.5, "root": { "type": "sine", "freq": 440 } },
             "params": [ { "name": "pitch", "paths": ["root.freq"], "min": 100, "max": 2000, "default": 440 } ] }"#,
    )
    .unwrap()
}

fn two_layer_doc() -> SoundDoc {
    serde_json::from_str(
        r#"{ "name": "m", "duration": 0.5, "root": { "type": "tracks", "tracks": [
               { "id": "bass", "node": { "type": "sine", "freq": 110 } },
               { "id": "arp",  "node": { "type": "sine", "freq": 880 } }
             ] } }"#,
    )
    .unwrap()
}

fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
}

#[test]
fn no_cap_spawns_unbounded() {
    // Default (unlimited) preserves the old behavior: every play() lives.
    let mut e = Engine::new(44_100);
    let p = e.load(&doc(1.0));
    for _ in 0..100 {
        e.play(p);
    }
    assert_eq!(e.active(), 100, "no cap → unbounded voices");
    assert_eq!(e.max_voices(), None);
}

#[test]
fn cap_bounds_the_sounding_voices() {
    let mut e = Engine::new(44_100);
    e.set_max_voices(4);
    let p = e.load(&doc(1.0));
    for _ in 0..12 {
        e.play(p); // equal priority — each steals the oldest sounding voice
    }
    let sounding = e.instances.iter().filter(|i| !i.stopping).count();
    assert!(
        sounding <= 4,
        "sounding voices exceeded the cap: {sounding}"
    );
    assert!(e.active() <= 8, "hard bound is 2×max: {}", e.active());
}

#[test]
fn higher_priority_steals_a_lower_one() {
    let mut e = Engine::new(44_100);
    e.set_max_voices(2);
    let p = e.load(&doc(1.0));
    let a = e.play_looping_prioritized(p, Priority::LOW);
    let b = e.play_looping_prioritized(p, Priority::LOW);
    let hi = e.play_prioritized(p, Priority::HIGH);
    assert!(e.is_active(hi), "the high-priority voice got in");
    // Exactly one low voice was declicked (stopping), not hard-cut.
    let stopping: Vec<u64> = e
        .instances
        .iter()
        .filter(|i| i.stopping)
        .map(|i| i.id)
        .collect();
    assert_eq!(stopping.len(), 1, "one low voice is fading out");
    assert!(
        stopping == vec![a.0] || stopping == vec![b.0],
        "the stolen voice is one of the two low loops"
    );
}

#[test]
fn outranked_voice_is_denied() {
    let mut e = Engine::new(44_100);
    e.set_max_voices(2);
    let p = e.load(&doc(1.0));
    e.play_looping_prioritized(p, Priority::HIGH);
    e.play_looping_prioritized(p, Priority::HIGH);
    let low = e.play_prioritized(p, Priority::LOW);
    assert!(!e.is_active(low), "a fully-outranked voice is denied");
    assert_eq!(
        e.instances.iter().filter(|i| !i.stopping).count(),
        2,
        "the high voices are untouched"
    );
}

#[test]
fn stealing_is_deterministic() {
    let run = || {
        let mut e = Engine::new(44_100);
        e.set_max_voices(3);
        let p = e.load(&doc(1.0));
        for i in 0..15 {
            let prio = Priority((i % 3) as u8 * 64);
            e.play_prioritized(p, prio);
        }
        let mut ids: Vec<u64> = e.instances.iter().map(|i| i.id).collect();
        ids.sort_unstable();
        ids
    };
    assert_eq!(run(), run(), "the same sequence yields the same survivors");
}

#[test]
fn one_patch_spawns_many_independent_instances() {
    let mut e = Engine::new(44_100);
    let p = e.load(&doc(1.0));
    let _a = e.play(p);
    let _b = e.play(p);
    let _c = e.play(p);
    assert_eq!(
        e.active(),
        3,
        "resource → instance: many instances of one patch"
    );

    let mut out = vec![0.0f32; 512 * 2];
    assert_eq!(e.fill(&mut out), 512);
    assert!(peak(&out) > 0.0, "the mix should produce audio");
}

#[test]
fn gain_tween_ramps_to_silence() {
    let mut e = Engine::new(1000);
    let p = e.load(&doc(1.0));
    let h = e.play(p);
    e.set_gain(h, 0.0, Tween::frames(100));
    let mut out = vec![0.0f32; 50 * 2];
    e.fill(&mut out);
    let mut rest = vec![0.0f32; 100 * 2];
    e.fill(&mut rest);
    assert!(
        peak(&rest[80 * 2..]) < 1e-3,
        "gain reached 0 after the tween"
    );
}

#[test]
fn stop_declicks_and_culls_the_instance() {
    let mut e = Engine::new(44_100);
    let p = e.load(&doc(5.0));
    let h = e.play_looping(p);
    assert!(e.is_active(h));
    e.stop(h, Tween::ms(10.0, 44_100));
    let mut out = vec![0.0f32; 1024 * 2];
    e.fill(&mut out);
    e.fill(&mut out);
    assert!(!e.is_active(h), "stopped instance is culled once silent");
}

#[test]
fn one_shot_culls_itself_at_end() {
    let mut e = Engine::new(1000);
    let p = e.load(&doc(0.1));
    e.play(p);
    assert_eq!(e.active(), 1);
    let mut out = vec![0.0f32; 256 * 2];
    e.fill(&mut out);
    assert_eq!(e.active(), 0, "a finished one-shot removes itself");
}

#[test]
fn param_resolves_and_set_param_keeps_it_playing() {
    let mut e = Engine::new(44_100);
    let p = e.load_patch(&pitch_patch());
    let pitch = e.param(p, "pitch").expect("pitch param");
    assert!(e.param(p, "nope").is_none());
    let h = e.play_looping(p);

    let mut out = vec![0.0f32; 256 * 2];
    e.fill(&mut out);
    e.set_param(h, pitch, 880.0, Tween::ms(5.0, 44_100));
    // Crossfade in progress: still exactly one live instance, still audible.
    assert_eq!(e.active(), 1);
    let mut out2 = vec![0.0f32; 1024 * 2];
    e.fill(&mut out2);
    assert!(peak(&out2) > 0.0, "still playing at the new pitch");
}

#[test]
fn layer_resolves_and_gain_change_is_click_free() {
    let mut e = Engine::new(44_100);
    let p = e.load(&two_layer_doc());
    let arp = e.layer(p, "arp").expect("arp layer");
    assert!(e.layer(p, "missing").is_none());
    let h = e.play_looping(p);
    let mut out = vec![0.0f32; 256 * 2];
    e.fill(&mut out);
    e.set_layer_gain(h, arp, 0.0, Tween::ms(20.0, 44_100));
    e.fill(&mut out);
    assert!(e.is_active(h), "layer move does not drop the instance");
}

#[test]
fn hard_pan_silences_the_opposite_channel() {
    let (l, r) = balance(1.0); // +1 = hard right
    assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
    let (l, r) = balance(-1.0); // -1 = hard left
    assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
    let (l, r) = balance(0.0);
    assert!(
        (l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6,
        "unity at centre"
    );
}

#[test]
fn ring_pushes_pops_and_wraps() {
    let r = SampleRing::new(4); // 4 usable slots
    assert!(r.pop().is_none());
    for i in 0..4 {
        assert!(r.push(i as f32));
    }
    assert!(!r.push(9.0), "full");
    assert_eq!(r.pop(), Some(0.0));
    assert!(r.push(9.0), "space freed after a pop");
    let got: Vec<f32> = std::iter::from_fn(|| r.pop()).collect();
    assert_eq!(got, vec![1.0, 2.0, 3.0, 9.0]);
}

#[test]
fn split_pumps_audio_across_the_seam() {
    let mut e = Engine::new(44_100);
    let p = e.load(&doc(1.0));
    let (mut ctl, mut rend) = e.split(1024);
    ctl.play_looping(p); // Deref → Engine::play_looping
    assert!(ctl.pump(512) > 0, "controller produced frames");
    let mut out = vec![0.0f32; 512 * 2];
    assert_eq!(rend.fill(&mut out), 512);
    assert!(peak(&out) > 0.0, "renderer drained real audio");
}

#[test]
fn spsc_pumps_a_mixer_across_the_seam() {
    // The generalized seam drives a whole Mixer, not just an Engine — the
    // shape the Python owned-stream binding pumps.
    let mut engine = Engine::new(44_100);
    let p = engine.load(&doc(1.0));
    engine.play_looping(p);
    let mut mixer = Mixer::new();
    mixer.add(engine);
    let (mut ctl, mut rend) = spsc(mixer, 1024);
    assert!(ctl.pump(512) > 0, "pump produced frames");
    assert_eq!(ctl.source_count(), 1, "deref reaches the Mixer");
    let mut out = vec![0.0f32; 512 * 2];
    assert_eq!(rend.fill(&mut out), 512);
    assert!(peak(&out) > 0.0, "renderer drained real audio");
}

#[test]
fn pump_never_drops_rendered_frames() {
    // The split path must deliver the same bytes as an unsplit engine:
    // pumping more than the ring can take must not advance play heads
    // past what was actually delivered.
    let mk = || {
        let mut e = Engine::new(44_100);
        let p = e.load(&doc(1.0));
        e.play_looping(p);
        e
    };
    let mut reference = mk();
    let mut expected = vec![0.0f32; 192 * 2];
    reference.fill(&mut expected);

    let (mut ctl, mut rend) = mk().split(64);
    let mut got = Vec::new();
    let mut out = vec![0.0f32; 64 * 2];
    while got.len() < expected.len() {
        ctl.pump(200); // over-ask: the ring only holds 64 frames
        rend.fill(&mut out);
        got.extend_from_slice(&out);
    }
    assert_eq!(
        &got[..expected.len()],
        &expected[..],
        "over-pumping dropped rendered frames"
    );
}

#[test]
fn renderer_drains_whole_frames_only() {
    // A partial frame in the ring must not shift channel alignment.
    let ring = SampleRing::new(8);
    for s in [1.0f32, 2.0, 3.0] {
        ring.push(s); // one and a half frames
    }
    let mut rend = Renderer {
        ring: std::sync::Arc::new(ring),
    };
    let mut out = vec![9.0f32; 4];
    rend.fill(&mut out);
    assert_eq!(out, vec![1.0, 2.0, 0.0, 0.0], "half frame must stay queued");
    rend.ring.push(4.0);
    let mut out = vec![9.0f32; 2];
    rend.fill(&mut out);
    assert_eq!(
        out,
        vec![3.0, 4.0],
        "queued half frame pairs with the next sample"
    );
}

#[test]
fn renderer_underrun_writes_silence() {
    let e = Engine::new(44_100);
    let (_ctl, mut rend) = e.split(256); // nothing pumped
    let mut out = vec![1.0f32; 128 * 2];
    rend.fill(&mut out);
    assert!(peak(&out) < 1e-9, "underrun is clean silence, not garbage");
}

#[test]
fn control_and_audio_sides_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Controller>();
    assert_send::<Renderer>();
}

#[test]
fn stream_source_streams_a_streamable_doc() {
    let d: SoundDoc = serde_json::from_str(
        r#"{ "name":"s", "duration":0.1, "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":220 },
            { "type":"lowpass", "cutoff":900, "q":0.7 } ] } }"#,
    )
    .unwrap();
    let mut src = StreamSource::from_doc(&d).expect("streamable");
    let mut out = vec![0.0f32; 256 * 2];
    assert_eq!(src.fill(&mut out), 256);
    assert!(peak(&out) > 0.0, "streams real audio");
    // Mono duplicated to stereo: channels are identical.
    assert!((0..256).all(|f| out[f * 2] == out[f * 2 + 1]));
}

#[test]
fn stream_source_matches_the_bounce_including_its_peak_limit() {
    // A full-scale sine peaks above the 0.989 ceiling, so the offline
    // bounce attenuates it. The stream must carry the identical gain or
    // it plays louder than the bounce and can clip the DAC.
    let d: SoundDoc = serde_json::from_str(
        r#"{ "name":"loud", "duration":0.1, "root": { "type":"sine", "freq":220 } }"#,
    )
    .unwrap();
    let bounce = crate::render::render(&d);
    let mut src = StreamSource::from_doc(&d).expect("streamable");
    let mut out = vec![0.0f32; bounce.len() * 2];
    src.fill(&mut out);
    for (i, b) in bounce.iter().enumerate() {
        assert_eq!(
            out[i * 2].to_bits(),
            b.to_bits(),
            "stream diverges from the bounce at sample {i}"
        );
    }
}

#[test]
fn stream_source_rejects_non_streamable() {
    let d: SoundDoc = serde_json::from_str(
        r#"{ "name":"n", "duration":0.05, "root": { "type":"noise", "color":"white" } }"#,
    )
    .unwrap();
    assert!(StreamSource::from_doc(&d).is_none());
}

#[test]
fn mixer_sums_and_reaches_in_by_type() {
    let mut e = Engine::new(44_100);
    let p = e.load(&doc(1.0));
    e.play_looping(p);
    let mut mixer = Mixer::new();
    let id = mixer.add(e);
    assert_eq!(mixer.source_count(), 1);
    // Reach back into the owned Engine and spawn another instance.
    mixer.get_mut::<Engine>(id).unwrap().play_looping(p);
    assert_eq!(mixer.get_mut::<Engine>(id).unwrap().active(), 2);
    mixer.set_gain(id, 0.5);
    let mut out = vec![0.0f32; 256 * 2];
    assert_eq!(mixer.fill(&mut out), 256);
    assert!(peak(&out) > 0.0, "mixer sums its sources");
    mixer.remove(id);
    assert_eq!(mixer.source_count(), 0);
}

/// A fixed stereo source: every frame is `(l, r)`, forever. Deterministic.
struct Const {
    l: f32,
    r: f32,
}
impl AudioSource for Const {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        for frame in out.chunks_mut(2) {
            frame[0] = self.l;
            frame[1] = self.r;
        }
        out.len() / 2
    }
}

/// Sounds one full block, then silence — to test that a reverb tail outlives it.
struct Burst {
    fired: bool,
}
impl AudioSource for Burst {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let v = if self.fired { 0.0 } else { 1.0 };
        out.fill(v);
        self.fired = true;
        out.len() / 2
    }
}

#[test]
fn no_bus_mix_is_the_plain_additive_sum() {
    // With no buses/effects, the routing mixer must be byte-identical to a
    // bare additive sum (back-compat for existing callers like tono-py).
    let mut mixer = Mixer::new();
    mixer.add(Const { l: 0.3, r: -0.2 });
    let b = mixer.add(Const { l: 0.1, r: 0.4 });
    mixer.set_gain(b, 0.5);
    let mut out = vec![0.0f32; 64 * 2];
    mixer.fill(&mut out);
    for frame in out.chunks(2) {
        assert_eq!(frame[0], 0.3 + 0.1 * 0.5);
        assert_eq!(frame[1], -0.2 + 0.4 * 0.5);
    }
}

#[test]
fn bus_insert_scales_only_its_bus() {
    // A gain insert on one bus halves it; a source on master is untouched.
    let mut mixer = Mixer::new_at(44_100);
    mixer.add(Const { l: 0.4, r: 0.4 }); // master, dry
    let music = mixer.bus("music");
    mixer.add_to(music, Const { l: 0.4, r: 0.4 });
    mixer.set_bus_effects(music, vec![gain_node(0.5)]).unwrap();
    let mut out = vec![0.0f32; 32 * 2];
    mixer.fill(&mut out);
    // master 0.4 (dry) + music 0.4 * 0.5 (halved) = 0.6
    for frame in out.chunks(2) {
        assert!((frame[0] - 0.6).abs() < 1e-6, "got {}", frame[0]);
    }
}

#[test]
fn reverb_send_tail_outlives_the_source() {
    let mut mixer = Mixer::new_at(44_100);
    let sfx = mixer.bus("sfx");
    mixer.add_to(sfx, Burst { fired: false });
    let rev = mixer.fx_bus("rev", vec![reverb_node()]).unwrap();
    mixer.set_send(sfx, rev, 0.9);
    // First block: the burst sounds.
    let mut out = vec![0.0f32; 128 * 2];
    mixer.fill(&mut out);
    assert!(peak(&out) > 0.0);
    // Later blocks: the source is silent, but the reverb tail keeps ringing.
    let mut tail = 0.0f32;
    for _ in 0..8 {
        mixer.fill(&mut out);
        tail = tail.max(peak(&out));
    }
    assert!(
        tail > 0.0,
        "reverb send should ring after the source goes silent"
    );
}

#[test]
fn master_fader_scales_the_whole_mix() {
    // set_bus_gain(MASTER, ..) must actually attenuate the output.
    let mut mixer = Mixer::new_at(44_100);
    mixer.add(Const { l: 0.4, r: 0.4 });
    mixer.set_bus_gain(BusId::MASTER, 0.5);
    let mut out = vec![0.0f32; 32 * 2];
    mixer.fill(&mut out);
    for frame in out.chunks(2) {
        assert!(
            (frame[0] - 0.2).abs() < 1e-6,
            "master fader must scale output, got {}",
            frame[0]
        );
    }
}

#[test]
fn source_routed_onto_an_fx_bus_still_sounds() {
    // add_to(fx_bus, ..) used to silently drop the source; it must be mixed
    // through the bus's inserts and returned to master.
    let mut mixer = Mixer::new_at(44_100);
    let rev = mixer.fx_bus("rev", vec![gain_node(0.5)]).unwrap();
    mixer.add_to(rev, Const { l: 0.8, r: 0.8 });
    let mut out = vec![0.0f32; 32 * 2];
    mixer.fill(&mut out);
    assert!(
        peak(&out) > 0.3,
        "a source on an fx bus must be audible through its inserts, got {}",
        peak(&out)
    );
}

#[test]
fn bus_routing_is_deterministic() {
    let build = || {
        let mut mixer = Mixer::new_at(44_100);
        let music = mixer.bus("music");
        mixer.add_to(music, Const { l: 0.2, r: 0.1 });
        mixer.set_bus_effects(music, vec![gain_node(0.7)]).unwrap();
        let rev = mixer.fx_bus("rev", vec![reverb_node()]).unwrap();
        mixer.set_send(music, rev, 0.5);
        mixer.master_effects(vec![gain_node(0.9)]).unwrap();
        mixer
    };
    let render = |mut mixer: Mixer| {
        let mut acc = Vec::new();
        let mut out = vec![0.0f32; 96 * 2];
        for _ in 0..6 {
            mixer.fill(&mut out);
            acc.extend_from_slice(&out);
        }
        acc
    };
    assert_eq!(
        render(build()),
        render(build()),
        "bus routing must be byte-identical"
    );
}

fn gain_node(amount: f32) -> Node {
    serde_json::from_str(&format!(r#"{{ "type": "gain", "amount": {amount} }}"#)).unwrap()
}

fn reverb_node() -> Node {
    serde_json::from_str(r#"{ "type": "reverb", "room": 0.6, "mix": 0.5 }"#).unwrap()
}
