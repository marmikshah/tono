//! Real-time allocation gates: the audio-callback
//! paths must not touch the heap once their scratch is grown.
//!
//! A counting [`GlobalAlloc`] wraps the system allocator for THIS test crate
//! only (a `#[global_allocator]` is one-per-crate, which is why this gets its
//! own integration test file). Each test warms up with counting off, then
//! enables the counter across the measured `fill` calls and demands zero.
//!
//! The documented boundary (runtime/mod.rs, `SCRATCH_FRAMES`): the runtime's
//! scratch is pre-sized to 8192 frames at construction; a block LARGER than
//! that grows the scratch exactly once, on the first such call. So the first
//! oversized `fill` may allocate (that growth is asserted once, explicitly),
//! and every call after it — at any block size up to the largest seen — must
//! be allocation-free.
//!
//! Counting is thread-local so allocations from another test or the test
//! harness cannot contaminate the measured audio path.
//!
//! Not covered (documented control-side operations, off the audio path):
//! scheduling commands, `Performance::stinger`/`swap_to` (they render at
//! schedule time), `Pump::pump` (the control thread's own buffer), and a
//! stream seek / loop wrap (the `SongSource::Stream` rebuild is an O(duration)
//! re-probe — a control-side operation).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use tono_core::dsl::{Adsr, SeqWave, SoundDoc};
use tono_core::runtime::{
    At, AudioSource, Command, Engine, Performance, SCRATCH_FRAMES, StreamSource,
};
use tono_core::song::{CompileOptions, Song, note};

// ---------------------------------------------------------------------------
// The counting allocator.
// ---------------------------------------------------------------------------

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.get() {
            COUNT.set(COUNT.get() + 1);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Vec growth goes through realloc — count it too.
        if ENABLED.get() {
            COUNT.set(COUNT.get() + 1);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn start_counting() {
    COUNT.set(0);
    ENABLED.set(true);
}

fn stop_counting() -> usize {
    ENABLED.set(false);
    COUNT.get()
}

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn amp() -> Adsr {
    Adsr {
        a: 0.005,
        d: 0.1,
        s: 0.8,
        r: 0.2,
        punch: 0.0,
    }
}

/// A small streamable multi-track program at 48 kHz (a plain compiled song
/// streams natively, so the Performance takes the `SongSource::Stream` path).
fn program() -> Arc<tono_core::program::Program> {
    let mut song = Song::new("rt-alloc", 120.0);
    song.add_track("bass", SeqWave::Bass, amp());
    song.add_track("keys", SeqWave::Epiano, amp());
    song.add_pattern("riff", 1, vec![note(0, 4, "C2"), note(8, 4, "G2")]);
    song.add_pattern("stab", 1, vec![note(0, 2, "C4"), note(6, 2, "D#4")]);
    song.arrange_repeat("bass", "riff", 0, 4);
    song.arrange_repeat("keys", "stab", 0, 4);
    Arc::new(
        song.compile(&CompileOptions {
            sample_rate: Some(48_000),
            ..CompileOptions::default()
        })
        .expect("compiles"),
    )
}

fn blip() -> SoundDoc {
    serde_json::from_str(
        r#"{ "name": "blip", "duration": 0.2, "root": { "type": "mul", "inputs": [
            { "type": "sawtooth", "freq": 880 },
            { "type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.01 } ] } }"#,
    )
    .unwrap()
}

/// The block sizes under test: odd, typical, the scratch boundary itself,
/// and one LARGER than `SCRATCH_FRAMES`.
const SIZES: [usize; 5] = [333, 512, 1024, SCRATCH_FRAMES, SCRATCH_FRAMES + 4096];

// ---------------------------------------------------------------------------
// The gates.
// ---------------------------------------------------------------------------

#[test]
fn performance_fill_is_allocation_free_after_warmup() {
    let mut p = Performance::new(program());
    // Control side: scheduling may allocate (queue growth) — never measured.
    p.schedule(Command::Play, At::Immediate).unwrap();
    // A gain ride lands inside the measured region: executing a scheduled
    // command mid-block is part of the allocation-free promise.
    p.schedule(Command::SetGain(0.7), At::Frame(6 * 1024))
        .unwrap();

    // Warm up every block size with counting OFF. The oversized block grows
    // the scratch once here (performance.rs render_slice's first-use growth).
    let mut block = vec![0.0f32; SIZES[SIZES.len() - 1] * 2];
    for size in SIZES {
        p.fill(&mut block[..size * 2]);
    }

    start_counting();
    for _ in 0..3 {
        for size in SIZES {
            p.fill(&mut block[..size * 2]);
        }
    }
    let allocs = stop_counting();
    assert_eq!(allocs, 0, "Performance::fill allocated on the audio path");
}

#[test]
fn performance_first_oversized_fill_grows_scratch_once() {
    let mut p = Performance::new(program());
    p.schedule(Command::Play, At::Immediate).unwrap();
    // Blocks at or under SCRATCH_FRAMES never allocate: the scratch was
    // pre-sized at construction.
    let mut block = vec![0.0f32; (SCRATCH_FRAMES + 4096) * 2];
    p.fill(&mut block[..512 * 2]);
    start_counting();
    p.fill(&mut block[..SCRATCH_FRAMES * 2]);
    assert_eq!(stop_counting(), 0, "scratch-boundary block must not grow");

    // The first block LARGER than SCRATCH_FRAMES grows the scratch exactly
    // once — the documented boundary (this call MAY allocate).
    start_counting();
    p.fill(&mut block[..(SCRATCH_FRAMES + 4096) * 2]);
    let growth = stop_counting();
    assert!(growth > 0, "the first oversized fill grows the scratch");
    start_counting();
    p.fill(&mut block[..(SCRATCH_FRAMES + 4096) * 2]);
    p.fill(&mut block[..(SCRATCH_FRAMES + 4096) * 2]);
    assert_eq!(
        stop_counting(),
        0,
        "the oversized scratch is grown once, then reused"
    );
}

#[test]
fn performance_fill_firing_a_stinger_is_allocation_free() {
    let mut p = Performance::new(program());
    p.schedule(Command::Play, At::Immediate).unwrap();
    // Schedule-time work (the stinger's render + voice-capacity reserve)
    // happens HERE, off the audio path — before counting starts.
    p.stinger(&blip(), 0.8, At::Frame(8 * 1024)).unwrap();

    let mut block = vec![0.0f32; 1024 * 2];
    for _ in 0..4 {
        p.fill(&mut block); // frames 0..4096, counted off
    }
    start_counting();
    for _ in 0..12 {
        p.fill(&mut block); // the stinger fires at frame 8192, mid-block
    }
    let allocs = stop_counting();
    assert_eq!(p.metrics().stingers_fired, 1, "the stinger really fired");
    assert_eq!(
        allocs, 0,
        "firing a stinger inside Performance::fill must not render or allocate \
         (the render happened at schedule time)"
    );
}

#[test]
fn stream_source_fill_is_allocation_free_after_warmup() {
    let program = program();
    let mut src = StreamSource::from_doc(&program.doc).expect("the program streams");
    let mut block = vec![0.0f32; SIZES[SIZES.len() - 1] * 2];
    for size in SIZES {
        src.fill(&mut block[..size * 2]); // warm-up: oversized grows scratch once
    }
    start_counting();
    for _ in 0..3 {
        for size in SIZES {
            src.fill(&mut block[..size * 2]);
        }
    }
    assert_eq!(
        stop_counting(),
        0,
        "StreamSource::fill allocated on the audio path"
    );
}

#[test]
fn renderer_fill_is_allocation_free() {
    let mut engine = Engine::new(48_000);
    let patch = engine.load(&blip());
    engine.play_looping(patch);
    let (mut ctl, mut rend) = engine.split(4096);
    ctl.pump(1024); // control side: the pump buffer grows here, never measured
    let mut block = vec![0.0f32; 1024 * 2];
    rend.fill(&mut block[..512 * 2]); // warm-up (the Renderer owns no scratch)
    start_counting();
    for size in [128usize, 333, 512, 1024] {
        ctl.pump(size); // keep the ring fed (control thread's job)
        rend.fill(&mut block[..size * 2]);
    }
    assert_eq!(
        stop_counting(),
        0,
        "Renderer::fill allocated on the audio path"
    );
}

/// A small second program for swap tests (a plain lead, also streamable).
fn other_program() -> Arc<tono_core::program::Program> {
    let mut song = Song::new("rt-alloc-other", 100.0);
    song.add_track("lead", SeqWave::Square, amp());
    song.tracks[0].notes.push(note(0, 16, "A4"));
    Arc::new(
        song.compile(&CompileOptions {
            sample_rate: Some(48_000),
            ..CompileOptions::default()
        })
        .expect("compiles"),
    )
}

#[test]
fn performance_fill_across_a_swap_is_allocation_free() {
    let mut p = Performance::new(program());
    p.schedule(Command::Play, At::Immediate).unwrap();
    // The new program's source builds HERE, at schedule time (a full probe
    // render) — off the audio path, before counting starts.
    p.swap_to(other_program(), At::Frame(8 * 1024)).unwrap();

    let mut block = vec![0.0f32; 1024 * 2];
    for _ in 0..4 {
        p.fill(&mut block); // frames 0..4096, counted off
    }
    start_counting();
    for _ in 0..12 {
        p.fill(&mut block); // the swap executes at frame 8192, mid-block
    }
    let allocs = stop_counting();
    assert_eq!(p.metrics().swaps, 1, "the swap really happened");
    assert_eq!(
        allocs, 0,
        "executing a swap inside Performance::fill must not render or allocate \
         (the source built at schedule time)"
    );
}
