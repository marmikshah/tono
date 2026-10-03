//! Release-gate soak tests: the long forms of the
//! runtime stress paths. Both tests are `#[ignore]`d — they are run ON DEMAND
//! before a release, never in CI, so the regular Cargo test gate stays fast:
//!
//! ```sh
//! cargo test -p tono-core --test soak -- --include-ignored
//! ```
//!
//! The short always-on variants live with the unit tests
//! (`runtime::tests::spsc_threaded_pump_and_drain_is_byte_identical` and the
//! `runtime::performance::tests` suite); they are not duplicated here.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tono_core::dsl::{Adsr, SeqWave, SoundDoc};
use tono_core::program::Program;
use tono_core::runtime::{At, AudioSource, Command, Performance, spsc};
use tono_core::song::{CompileOptions, Section, Song, note};

const SR: u32 = 48_000;
/// The release gate: five minutes of audio rendered in debug in bounded
/// blocks (the ten-minute form overshot the ~60 s debug budget at ~115 s;
/// the bar-range loop makes the content repetitive either way, and the two
/// passes run on their own threads — the runtime path is fully exercised).
const SOAK_SECS: usize = 300;

fn amp() -> Adsr {
    Adsr {
        a: 0.005,
        d: 0.1,
        s: 0.8,
        r: 0.2,
        punch: 0.0,
    }
}

/// A multi-track song: three seq tracks over four bars with two named
/// sections — big enough to exercise the streaming song source, the loop
/// wrap, and section seeks; small enough that the schedule-time and
/// loop-wrap re-renders stay cheap.
fn soak_program() -> Arc<Program> {
    let mut song = Song::new("soak", 120.0);
    song.add_track("bass", SeqWave::Bass, amp());
    song.add_track("keys", SeqWave::Epiano, amp());
    song.add_track("lead", SeqWave::Square, amp());
    song.add_pattern(
        "riff",
        1,
        vec![note(0, 4, "C2"), note(8, 4, "G2"), note(12, 2, "A#2")],
    );
    song.add_pattern(
        "stab",
        1,
        vec![note(0, 2, "C4"), note(6, 2, "D#4"), note(10, 2, "G4")],
    );
    song.add_pattern(
        "line",
        1,
        vec![
            note(0, 4, "C5"),
            note(4, 4, "D5"),
            note(8, 4, "D#5"),
            note(12, 4, "G5"),
        ],
    );
    song.arrange_repeat("bass", "riff", 0, 4);
    song.arrange_repeat("keys", "stab", 0, 4);
    song.arrange_repeat("lead", "line", 0, 4);
    song.sections.push(Section {
        name: "a".into(),
        bar: 0,
        bars: 2,
    });
    song.sections.push(Section {
        name: "b".into(),
        bar: 2,
        bars: 2,
    });
    let program = song
        .compile(&CompileOptions {
            sample_rate: Some(SR),
            ..CompileOptions::default()
        })
        .expect("compiles");
    assert!(program.is_streamable(), "the soak song streams natively");
    Arc::new(program)
}

fn blip() -> SoundDoc {
    serde_json::from_str(
        r#"{ "name": "blip", "duration": 0.2, "root": { "type": "mul", "inputs": [
            { "type": "sawtooth", "freq": 880 },
            { "type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.01 } ] } }"#,
    )
    .unwrap()
}

/// The soak script: play with the whole song looping, a stinger every 4
/// bars, a gain ride every 8, and a section seek (inside the loop) every 32.
/// Everything is scheduled up front at absolute clock frames. Returns
/// `(commands, stingers)` scheduled, for the metrics assertions.
fn script(p: &mut Performance, blip: &SoundDoc, bars: u32, bar_frames: u64) -> (u64, u64) {
    let mut commands = 0;
    let mut stingers = 0;
    p.schedule(Command::Play, At::Immediate).unwrap();
    commands += 1;
    p.schedule(Command::SetLoopBars(0, 4), At::Immediate)
        .unwrap();
    commands += 1;
    for bar in (4..bars).step_by(4) {
        p.stinger(blip, 0.7, At::Frame(bar as u64 * bar_frames))
            .unwrap();
        commands += 1;
        stingers += 1;
    }
    for bar in (8..bars).step_by(8) {
        let gain = if (bar / 8) % 2 == 0 { 0.6 } else { 1.0 };
        p.schedule(Command::SetGain(gain), At::Frame(bar as u64 * bar_frames))
            .unwrap();
        commands += 1;
    }
    for bar in (32..bars).step_by(32) {
        let section = if (bar / 32) % 2 == 1 { "b" } else { "a" };
        p.schedule(
            Command::SeekSection(section.into()),
            At::Frame(bar as u64 * bar_frames),
        )
        .unwrap();
        commands += 1;
    }
    (commands, stingers)
}

/// The first sample index where two streams differ by bits, if any.
fn divergence(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
}

/// Render `total_frames` through `p` in bounded blocks of cycling sizes,
/// asserting finiteness and advancing metrics; `sink` receives each finished
/// block and its absolute frame offset.
fn drive(p: &mut Performance, total_frames: usize, mut sink: impl FnMut(&[f32], usize)) {
    const BLOCKS: [usize; 4] = [1024, 2048, 480, 1536];
    let mut block = vec![0.0f32; 2048 * 2];
    let mut done = 0usize;
    let mut i = 0usize;
    let mut last_rendered = 0u64;
    while done < total_frames {
        let frames = BLOCKS[i % BLOCKS.len()].min(total_frames - done);
        p.fill(&mut block[..frames * 2]);
        sink(&block[..frames * 2], done);
        // The counters keep advancing (sampled — metrics() is read off the
        // audio path, not per block).
        if i.is_multiple_of(64) {
            let m = p.metrics();
            assert!(
                m.frames_rendered > last_rendered,
                "frames_rendered must keep advancing (block {i})"
            );
            last_rendered = m.frames_rendered;
        }
        done += frames;
        i += 1;
    }
}

#[test]
#[ignore = "release gate: 5 min of audio (debug-bounded); run on demand via --include-ignored"]
fn performance_long_session_no_engine_underruns() {
    let program = soak_program();
    let total_frames = SOAK_SECS * SR as usize;
    // 4/4 at 120 BPM: one bar is 2 s.
    let bars = (SOAK_SECS / 2) as u32;
    let bar_frames = 2 * SR as u64;

    // Pass A renders the reference take on its own thread, streaming blocks
    // through a bounded channel; pass B renders the same script here and
    // compares bit-for-bit as the blocks arrive — byte-exact without
    // materializing two 115 MB takes, and the two renders overlap so the
    // debug run stays well under ~60 s.
    let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, Vec<f32>)>(64);
    let program_a = program.clone();
    let a = std::thread::spawn(move || {
        let mut p = Performance::new(program_a);
        let (commands, stingers) = script(&mut p, &blip(), bars, bar_frames);
        drive(&mut p, total_frames, |block, at| {
            // The finiteness check folds into the pass that renders.
            assert!(
                block.iter().all(|x| x.is_finite()),
                "non-finite sample at frame {at}"
            );
            tx.send((at, block.to_vec())).expect("pass B is listening");
        });
        (p.metrics(), commands, stingers)
    });

    let mut b = Performance::new(program);
    let (commands, stingers) = script(&mut b, &blip(), bars, bar_frames);
    drive(&mut b, total_frames, |block, at| {
        assert!(
            block.iter().all(|x| x.is_finite()),
            "non-finite sample at frame {at}"
        );
        let (exp_at, expected) = rx.recv().expect("pass A is streaming");
        assert_eq!(exp_at, at, "the passes render in lockstep");
        if let Some(d) = divergence(block, &expected) {
            panic!("run B diverged from run A at frame {}", at + d / 2);
        }
    });

    for (m, pass) in [
        (a.join().expect("pass A panicked").0, "A"),
        (b.metrics(), "B"),
    ] {
        assert_eq!(
            m.frames_rendered, total_frames as u64,
            "pass {pass}: every frame rendered"
        );
        assert_eq!(
            m.commands_executed, commands,
            "pass {pass}: every scheduled command ran"
        );
        assert_eq!(m.commands_dropped, 0, "pass {pass}: nothing rejected");
        assert_eq!(m.stingers_fired, stingers, "pass {pass}");
        assert_eq!(m.swaps, 0, "pass {pass}");
        assert_eq!(
            m.queue_depth_max, commands as usize,
            "pass {pass}: all commands were queued up front"
        );
    }
}

#[test]
#[ignore = "timing-sensitive (real-thread scheduling); run on demand via --include-ignored"]
fn threaded_pump_drain_long_soak() {
    let program = soak_program();
    let total = 6 * SR as usize; // a few seconds of audio
    let script = |p: &mut Performance| {
        p.schedule(Command::Play, At::Immediate).unwrap();
        p.schedule(Command::SetGain(0.8), At::Frame(3 * SR as u64))
            .unwrap();
    };

    // The single-threaded reference: the same script, fixed 512-frame blocks.
    let mut reference = Performance::new(program.clone());
    script(&mut reference);
    let mut expected = Vec::with_capacity(total * 2);
    let mut block = vec![0.0f32; 512 * 2];
    while expected.len() < total * 2 {
        let frames = (total - expected.len() / 2).min(512);
        reference.fill(&mut block[..frames * 2]);
        expected.extend_from_slice(&block[..frames * 2]);
    }

    // The threaded take: a producer pumping a Performance in varied (odd)
    // block sizes while the consumer drains in different varied sizes. The
    // producer publishes its pumped total; the consumer only drains what has
    // been pumped (ring occupancy == pumped − drained exactly: pump never
    // renders past ring space, the consumer never underruns), so no silence
    // can be injected and no frame can be dropped.
    let (mut ctl, mut rend) = spsc(Performance::new(program), 4096);
    script(&mut ctl); // DerefMut → Performance
    let pumped = Arc::new(AtomicUsize::new(0));
    let pumped_producer = pumped.clone();
    let producer = std::thread::spawn(move || {
        const SIZES: [usize; 4] = [333, 512, 127, 1000];
        let mut done = 0usize;
        let mut i = 0usize;
        while done < total {
            let n = ctl.pump(SIZES[i % SIZES.len()]);
            if n == 0 {
                std::thread::yield_now(); // ring full: consumer drains next
                continue;
            }
            done += n;
            pumped_producer.store(done, Ordering::Release);
            i += 1;
        }
    });
    const SIZES: [usize; 4] = [192, 481, 64, 1024];
    let mut got = Vec::with_capacity(total * 2);
    let mut i = 0usize;
    while got.len() < total * 2 {
        let want = (total * 2 - got.len()).min(SIZES[i % SIZES.len()] * 2);
        let available = pumped.load(Ordering::Acquire) * 2 - got.len();
        if available >= want {
            let mut block = vec![0.0f32; want];
            rend.fill(&mut block);
            got.extend_from_slice(&block);
            i += 1;
        } else {
            std::thread::yield_now();
        }
    }
    producer.join().unwrap();
    assert_eq!(got.len(), expected.len(), "no dropped frames");
    if let Some(d) = divergence(&got, &expected) {
        panic!("threaded pump/drain diverged from the reference at sample {d}");
    }
}
