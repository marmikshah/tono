//! Browser rendering through the same deterministic DSP as native Tono.
//!
//! The raw WebAssembly ABI owns one request at a time. Call `tono_prepare(len)`,
//! copy UTF-8 SoundDoc JSON into the returned address in exported `memory`, and
//! call `tono_render()`. A return value of one exposes stereo f32 channel views
//! through `tono_left_ptr`, `tono_right_ptr`, `tono_frames`, and `tono_sample_rate`.
//! Zero exposes a UTF-8 message through `tono_error_ptr` and `tono_error_len`.
//!
//! Copy channel/error views before the next prepare, render, or reset. Re-read
//! `memory.buffer` after rendering: allocations can grow WebAssembly memory.
//! `tono_reset()` releases owned request/result buffers. A worker can serialize
//! requests and transfer copied channels to Web Audio without blocking the UI.

#![warn(missing_docs)]

use std::cell::RefCell;

use tono_core::dsl::{Node, SeqWave, SoundDoc, tempo_map_bpm_at, tempo_map_seconds_at};

/// Maximum UTF-8 request length, before allocating an input buffer.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
/// Maximum source document duration supported by the browser sound library.
pub const MAX_DURATION_SECONDS: f32 = 30.0;
/// Maximum sample rate supported by the browser sound library.
pub const MAX_SAMPLE_RATE: u32 = 48_000;
const MAX_GRAPH_NODES: usize = 128;
const MAX_GRAPH_DEPTH: usize = 32;
const MAX_FRAME_WORK: u64 = 32_000_000;
// Polyphonic scores perform many short note renders without retaining their
// scratch together. Keep full-window memory/graph bounds separate from that
// accumulated CPU work; the authored BGM catalog fits this explicit budget.
const MAX_NOTE_FRAME_WORK: u64 = 64_000_000;
const MAX_CONVOLUTION_FFT: u64 = 1 << 20;

/// Stereo floating-point audio ready for Web Audio.
#[derive(Debug)]
pub struct RenderedAudio {
    /// Sample rate of both channels.
    pub sample_rate: u32,
    /// Left channel, one f32 per frame.
    pub left: Vec<f32>,
    /// Right channel, one f32 per frame.
    pub right: Vec<f32>,
}

/// Parse, validate, bound resource use, and render SoundDoc JSON.
///
/// Browser limits supplement the core's validation: the library handles short
/// synthesized sounds, and cannot load a SoundFont from a native file path.
pub fn render_json(input: &[u8]) -> Result<RenderedAudio, String> {
    if input.is_empty() || input.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "document must contain 1–{MAX_INPUT_BYTES} UTF-8 bytes"
        ));
    }
    let doc: SoundDoc = serde_json::from_slice(input).map_err(|e| format!("document JSON: {e}"))?;
    doc.validate().map_err(|e| e.to_string())?;
    if doc.duration > MAX_DURATION_SECONDS {
        return Err(format!(
            "browser duration limit is {MAX_DURATION_SECONDS} seconds"
        ));
    }
    if doc.sample_rate > MAX_SAMPLE_RATE {
        return Err(format!("browser sample-rate limit is {MAX_SAMPLE_RATE} Hz"));
    }
    let frames = (doc.duration * doc.sample_rate as f32).ceil() as u64;
    let mut nodes = 0;
    let mut work = frames.saturating_mul(2); // final stereo output
    check_graph(&doc.root, 1, frames, doc.sample_rate, &mut nodes, &mut work)?;
    let (left, right) = tono_core::player::render_stereo(&doc);
    if left.len() != right.len() || left.iter().chain(&right).any(|sample| !sample.is_finite()) {
        return Err("renderer produced invalid audio".into());
    }
    Ok(RenderedAudio {
        sample_rate: doc.sample_rate,
        left,
        right,
    })
}

// The core's private estimator applies to compiled songs, counting final
// output memory. Browser requests also need limits on nested intermediates,
// unison voices, sequenced notes and FFT work before any DSP allocation.
fn check_graph(
    node: &Node,
    depth: usize,
    frames: u64,
    sample_rate: u32,
    nodes: &mut usize,
    work: &mut u64,
) -> Result<(), String> {
    check_graph_with_notes(node, depth, frames, sample_rate, nodes, work, &mut 0)
}

fn check_graph_with_notes(
    node: &Node,
    depth: usize,
    frames: u64,
    sample_rate: u32,
    nodes: &mut usize,
    work: &mut u64,
    note_work: &mut u64,
) -> Result<(), String> {
    *nodes += 1;
    if depth > MAX_GRAPH_DEPTH || *nodes > MAX_GRAPH_NODES {
        return Err(format!(
            "browser graph limit is {MAX_GRAPH_NODES} nodes and {MAX_GRAPH_DEPTH} levels"
        ));
    }
    let weight = match node {
        Node::Super { voices, .. } => u64::from(*voices),
        Node::Modal { modes, .. } => modes.len().max(1) as u64,
        Node::Seq {
            wave: SeqWave::Sampler,
            ..
        } => {
            return Err("SoundFont sampler requires a native file; use a synthesized instrument in the browser".into());
        }
        Node::Seq { .. } => {
            // Sequencers allocate one output window, but synthesize only each
            // note's gate. Counting every event as a whole song overestimates
            // ordinary scores by orders of magnitude. The note estimate also
            // includes full scratch for notes partly clipped by the window.
            *note_work = note_work.saturating_add(sequence_work(node, frames, sample_rate));
            if *note_work > MAX_NOTE_FRAME_WORK {
                return Err(format!(
                    "browser note render-work limit exceeded ({note_work} > {MAX_NOTE_FRAME_WORK}); shorten held notes or reduce overlapping voices"
                ));
            }
            1
        }
        Node::Granular {
            density, grain_ms, ..
        } => ((*density * *grain_ms / 1000.0).ceil() as u64).max(1),
        Node::Convolve {
            decay,
            size,
            predelay,
            mix,
            ..
        } if *mix > 0.0 => {
            let ir_seconds = if *size > 0.0 { *size } else { *decay };
            let ir_frames = ((ir_seconds + *predelay) * sample_rate as f32).ceil() as u64;
            let fft = frames
                .saturating_add(ir_frames)
                .saturating_sub(1)
                .next_power_of_two();
            if fft > MAX_CONVOLUTION_FFT {
                return Err("browser convolution workspace limit exceeded".into());
            }
            // Three complex FFT buffers dominate the convolution workspace.
            *work = work.saturating_add(fft.saturating_mul(12));
            1
        }
        _ => 1,
    };
    *work = work.saturating_add(frames.saturating_mul(weight));
    if *work > MAX_FRAME_WORK {
        return Err(format!(
            "browser render-work limit exceeded ({work} > {MAX_FRAME_WORK}); shorten the sound or simplify its graph"
        ));
    }
    for child in node.children() {
        check_graph_with_notes(
            child,
            depth + 1,
            frames,
            sample_rate,
            nodes,
            work,
            note_work,
        )?;
    }
    Ok(())
}

fn sequence_work(node: &Node, frames: u64, sample_rate: u32) -> u64 {
    let Node::Seq {
        bpm,
        steps_per_beat,
        tempo_map,
        humanize,
        swing,
        wave,
        notes,
        ..
    } = node
    else {
        return 0;
    };
    let mut work = 0u64;
    for note in notes {
        // Constant-tempo math mirrors the renderer's f32 step duration. Under
        // a tempo map, both hosts use the core's shared f64 timing functions.
        // Humanize can place a note up to 12% of a step early. Count that
        // earliest position (plus two frames of rounding margin) so any
        // stochastic displacement stays within the resource bound.
        let (earliest, len) = if tempo_map.is_empty() {
            let step = sample_rate as f32 * 60.0 / *bpm / (*steps_per_beat).max(1) as f32;
            let delay = if note.step % 2 == 1 {
                *swing * 0.5 * step
            } else {
                0.0
            };
            let earliest =
                (note.step as f32 * step + delay - *humanize * 0.12 * step - 2.0).max(0.0) as u64;
            let len = ((note.len as f32 * step).min(frames as f32) as u64).max(1);
            (earliest, len)
        } else {
            let beat = note.step as f64 / *steps_per_beat as f64;
            let step = sample_rate as f64 * 60.0
                / tempo_map_bpm_at(tempo_map, beat)
                / *steps_per_beat as f64;
            let delay = if note.step % 2 == 1 {
                *swing as f64 * 0.5 * step
            } else {
                0.0
            };
            let earliest = (tempo_map_seconds_at(tempo_map, beat) * sample_rate as f64 + delay
                - *humanize as f64 * 0.12 * step
                - 2.0)
                .max(0.0) as u64;
            let end_beat =
                note.step.saturating_add(note.len.max(1)) as f64 / *steps_per_beat as f64;
            let end = (tempo_map_seconds_at(tempo_map, end_beat) * sample_rate as f64)
                .round()
                .max(0.0) as u64;
            (earliest, end.saturating_sub(earliest).max(1).min(frames))
        };
        // Even discarded notes require timing/tempo-map lookup. Piano builds
        // up to 18 double-string partials; pluck initializes up to sr/20 delay
        // samples, independently of a note's audible length.
        work = work.saturating_add(32 + tempo_map.len() as u64 * 4);
        if earliest >= frames {
            continue;
        }
        let audible = len.min(frames - earliest);
        let (voice_weight, setup) = match wave {
            SeqWave::Piano => (8, 18 * 32),
            SeqWave::Pluck => (4, sample_rate as u64 / 20 + 96),
            SeqWave::Bell | SeqWave::Epiano | SeqWave::Organ | SeqWave::Bass => (6, 128),
            _ => (4, 32),
        };
        // ADSR, pitch, and duty buffers each cover len even when the audible
        // part is shorter. Voice synthesis and summing cover audible frames.
        // Natural voice decay/release occurs inside the encoded note length;
        // no voice tail is implicitly rendered beyond that length by Seq.
        work = work
            .saturating_add(len.saturating_mul(3))
            .saturating_add(audible.saturating_mul(voice_weight))
            .saturating_add(setup);
    }
    work
}

#[derive(Default)]
struct Session {
    input: Vec<u8>,
    output: Option<RenderedAudio>,
    error: String,
}

impl Session {
    fn prepare(&mut self, bytes: usize) -> usize {
        *self = Self::default();
        if bytes == 0 || bytes > MAX_INPUT_BYTES {
            self.error = format!("document must contain 1–{MAX_INPUT_BYTES} UTF-8 bytes");
            return 0;
        }
        self.input = vec![0; bytes];
        self.input.as_mut_ptr() as usize
    }

    fn render(&mut self) -> u32 {
        self.output = None;
        self.error.clear();
        match render_json(&self.input) {
            Ok(audio) => {
                self.output = Some(audio);
                1
            }
            Err(error) => {
                self.error = error;
                0
            }
        }
    }
}

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session::default());
}

/// Raw WebAssembly ABI revision (independent of the synthesis engine revision).
#[unsafe(no_mangle)]
pub extern "C" fn tono_abi_version() -> u32 {
    1
}

/// DSP engine revision used to validate documents and invalidate host caches.
#[unsafe(no_mangle)]
pub extern "C" fn tono_engine_version() -> u32 {
    tono_core::dsl::ENGINE_VERSION
}

/// Reset the session and allocate initialized UTF-8 input bytes; zero on error.
#[unsafe(no_mangle)]
pub extern "C" fn tono_prepare(bytes: usize) -> usize {
    SESSION.with(|session| session.borrow_mut().prepare(bytes))
}

/// Render the prepared request; one on success, zero with an error message.
#[unsafe(no_mangle)]
pub extern "C" fn tono_render() -> u32 {
    SESSION.with(|session| session.borrow_mut().render())
}

/// Successful result length in frames; zero without a successful result.
#[unsafe(no_mangle)]
pub extern "C" fn tono_frames() -> usize {
    SESSION.with(|s| s.borrow().output.as_ref().map_or(0, |o| o.left.len()))
}

/// Successful result sample rate; zero without a successful result.
#[unsafe(no_mangle)]
pub extern "C" fn tono_sample_rate() -> u32 {
    SESSION.with(|s| s.borrow().output.as_ref().map_or(0, |o| o.sample_rate))
}

/// Read-only left f32 channel address; zero without a successful result.
#[unsafe(no_mangle)]
pub extern "C" fn tono_left_ptr() -> usize {
    SESSION.with(|s| {
        s.borrow()
            .output
            .as_ref()
            .map_or(0, |o| o.left.as_ptr() as usize)
    })
}

/// Read-only right f32 channel address; zero without a successful result.
#[unsafe(no_mangle)]
pub extern "C" fn tono_right_ptr() -> usize {
    SESSION.with(|s| {
        s.borrow()
            .output
            .as_ref()
            .map_or(0, |o| o.right.as_ptr() as usize)
    })
}

/// Read-only UTF-8 error address; zero without an error.
#[unsafe(no_mangle)]
pub extern "C" fn tono_error_ptr() -> usize {
    SESSION.with(|s| {
        let session = s.borrow();
        if session.error.is_empty() {
            0
        } else {
            session.error.as_ptr() as usize
        }
    })
}

/// Error-message length in UTF-8 bytes.
#[unsafe(no_mangle)]
pub extern "C" fn tono_error_len() -> usize {
    SESSION.with(|s| s.borrow().error.len())
}

/// Release all owned request/result buffers and clear errors.
#[unsafe(no_mangle)]
pub extern "C" fn tono_reset() {
    SESSION.with(|session| *session.borrow_mut() = Session::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn basic() -> serde_json::Value {
        json!({"name":"browser-test", "duration":0.03, "root":{"type":"noise"}, "seed":42})
    }

    #[test]
    fn native_equivalence_covers_stereo_tracks_and_loops() {
        let mut cases = vec![basic()];
        let mut wide = basic();
        wide["stereo"] = json!({"mode":"wide", "amount":0.7});
        cases.push(wide);
        let mut looped = basic();
        looped["playback"] = json!({"mode":"loop", "crossfade_secs":0.005});
        cases.push(looped);
        cases.push(
            json!({"name":"tracks", "duration":0.03, "root":{"type":"tracks", "tracks":[
                {"id":"left", "pan":-0.75, "node":{"type":"sine", "freq":220}},
                {"id":"right", "pan":0.75, "node":{"type":"sine", "freq":330}}
            ]}}),
        );
        for case in cases {
            let input = serde_json::to_vec(&case).unwrap();
            let audio = render_json(&input).unwrap();
            let doc: SoundDoc = serde_json::from_slice(&input).unwrap();
            let native = tono_core::player::render_stereo(&doc);
            assert_eq!(audio.left, native.0);
            assert_eq!(audio.right, native.1);
            assert_eq!(audio.sample_rate, doc.sample_rate);
        }
    }

    #[test]
    fn invalid_and_oversized_requests_fail_before_rendering() {
        assert!(
            render_json(b"not json")
                .unwrap_err()
                .contains("document JSON")
        );
        let mut doc = basic();
        doc["engine"] = json!(0);
        assert!(
            render_json(&serde_json::to_vec(&doc).unwrap())
                .unwrap_err()
                .contains("engine")
        );
        doc = basic();
        doc["duration"] = json!(31);
        assert!(
            render_json(&serde_json::to_vec(&doc).unwrap())
                .unwrap_err()
                .contains("duration limit")
        );
        doc = basic();
        doc["sample_rate"] = json!(96_000);
        assert!(
            render_json(&serde_json::to_vec(&doc).unwrap())
                .unwrap_err()
                .contains("sample-rate limit")
        );
        assert!(render_json(&vec![b' '; MAX_INPUT_BYTES + 1]).is_err());
    }

    #[test]
    fn unsupported_sampler_and_expensive_graphs_fail_explicitly() {
        let sampler = json!({"name":"sampler", "root":{"type":"seq", "bpm":120,
            "wave":"sampler", "sf2":"piano.sf2", "env":{"a":0, "d":0, "s":1, "r":0},
            "notes":[{"step":0, "len":1, "pitch":"C4"}]}});
        assert!(
            render_json(&serde_json::to_vec(&sampler).unwrap())
                .unwrap_err()
                .contains("native file")
        );
        let expensive = json!({"name":"expensive", "duration":30, "root":{"type":"mix", "inputs":
            (0..30).map(|_| json!({"type":"sine", "freq":440})).collect::<Vec<_>>()}});
        assert!(
            render_json(&serde_json::to_vec(&expensive).unwrap())
                .unwrap_err()
                .contains("render-work")
        );
        let wide = json!({"name":"wide", "root":{"type":"mix", "inputs":
            (0..MAX_GRAPH_NODES).map(|_| json!({"type":"sine", "freq":440})).collect::<Vec<_>>()}});
        assert!(
            render_json(&serde_json::to_vec(&wide).unwrap())
                .unwrap_err()
                .contains("graph limit")
        );
        let mut nested = json!({"type":"sine", "freq":440});
        for _ in 0..MAX_GRAPH_DEPTH {
            nested = json!({"type":"mul", "inputs":[nested]});
        }
        let deep = json!({"name":"deep", "root":nested});
        assert!(
            render_json(&serde_json::to_vec(&deep).unwrap())
                .unwrap_err()
                .contains("graph limit")
        );
        let convolution = json!({"name":"large-fft", "duration":1,
        "root":{"type":"chain", "stages":[
            {"type":"noise"}, {"type":"convolve", "decay":30, "predelay":30, "mix":1}
        ]}});
        assert!(
            render_json(&serde_json::to_vec(&convolution).unwrap())
                .unwrap_err()
                .contains("convolution workspace")
        );
    }

    #[test]
    fn session_clears_stale_output_and_recovers_after_error() {
        let mut session = Session::default();
        let input = serde_json::to_vec(&basic()).unwrap();
        assert_ne!(session.prepare(input.len()), 0);
        session.input.copy_from_slice(&input);
        assert_eq!(session.render(), 1);
        assert!(session.output.is_some());
        assert_eq!(session.prepare(MAX_INPUT_BYTES + 1), 0);
        assert!(session.output.is_none());
        assert!(!session.error.is_empty());
        session.prepare(4);
        session.input.copy_from_slice(b"null");
        assert_eq!(session.render(), 0);
        assert!(session.output.is_none());
        session.prepare(input.len());
        session.input.copy_from_slice(&input);
        assert_eq!(session.render(), 1);
        assert!(session.error.is_empty());
    }

    #[test]
    fn abi_accessors_expose_only_the_current_result() {
        let input = serde_json::to_vec(&basic()).unwrap();
        assert_eq!(tono_abi_version(), 1);
        assert_eq!(tono_engine_version(), tono_core::dsl::ENGINE_VERSION);
        assert_ne!(tono_prepare(input.len()), 0);
        SESSION.with(|session| session.borrow_mut().input.copy_from_slice(&input));
        assert_eq!(tono_render(), 1);
        assert_eq!(tono_sample_rate(), 44_100);
        assert_eq!(tono_frames(), 1323);
        assert_ne!(tono_left_ptr(), 0);
        assert_ne!(tono_right_ptr(), 0);
        assert_eq!(tono_error_len(), 0);
        assert_eq!(tono_prepare(MAX_INPUT_BYTES + 1), 0);
        assert_eq!(tono_frames(), 0);
        assert_eq!(tono_sample_rate(), 0);
        assert_eq!(tono_left_ptr(), 0);
        assert!(tono_error_len() > 0);
        assert_ne!(tono_error_ptr(), 0);
        tono_reset();
        assert_eq!(tono_error_len(), 0);
        assert_eq!(tono_error_ptr(), 0);
    }

    fn sequence(wave: &str, notes: serde_json::Value) -> Node {
        serde_json::from_value(json!({"type":"seq", "bpm":120, "steps_per_beat":4,
            "wave":wave, "env":{"a":0.002, "d":0.05, "s":0.5, "r":0.02}, "notes":notes}))
        .unwrap()
    }

    #[test]
    fn musical_work_counts_note_gates_and_accepts_a_layered_loop() {
        let notes = json!(
            (0..64)
                .map(|index| json!({"step":index * 2, "len":2, "pitch":"C4"}))
                .collect::<Vec<_>>()
        );
        let mut doc = SoundDoc::new(
            "score",
            Node::Mix {
                inputs: vec![
                    sequence("piano", notes.clone()),
                    sequence("bass", notes.clone()),
                    sequence("fm", notes.clone()),
                    sequence("kit", notes),
                ],
            },
        );
        doc.duration = 16.0;
        doc.sample_rate = 48_000;
        doc.playback = tono_core::dsl::Playback::Loop {
            start_secs: 8.0,
            end_secs: Some(16.0),
            crossfade_secs: 0.02,
        };
        doc.validate().unwrap();
        let frames = (doc.duration * doc.sample_rate as f32).ceil() as u64;
        let mut nodes = 0;
        let mut work = frames * 2;
        check_graph(&doc.root, 1, frames, doc.sample_rate, &mut nodes, &mut work).unwrap();
        assert!(work < MAX_FRAME_WORK);
        // Old estimate charged all 256 events as the entire 16-second score.
        assert!(frames * 256 * 8 > MAX_FRAME_WORK);
    }

    #[test]
    fn held_notes_and_clipped_note_scratch_remain_bounded() {
        let held = sequence(
            "piano",
            json!(
                (0..32)
                    .map(|_| json!({"step":0, "len":u32::MAX, "pitch":"C2"}))
                    .collect::<Vec<_>>()
            ),
        );
        let mut work = 48_000 * 30 * 2;
        assert!(
            check_graph(&held, 1, 48_000 * 30, 48_000, &mut 0, &mut work)
                .unwrap_err()
                .contains("render-work")
        );
        let clipped = sequence("sine", json!([{"step":7, "len":16, "pitch":"C4"}]));
        // The note starts at .875 s, yet its pitch/duty/ADSR buffers each
        // cover the capped full 1 s gate before only .125 s is mixed.
        assert!(sequence_work(&clipped, 8_000, 8_000) >= 3 * 8_000);
        let outside = sequence("sine", json!([{"step":100, "len":16, "pitch":"C4"}]));
        assert!(sequence_work(&outside, 8_000, 8_000) < 100);
    }

    #[test]
    fn pluck_setup_and_tempo_changes_are_counted_before_render() {
        let mut pluck = sequence("pluck", json!([{"step":0, "len":1, "pitch":"E0"}]));
        if let Node::Seq { steps_per_beat, .. } = &mut pluck {
            *steps_per_beat = 48_000;
        }
        assert!(sequence_work(&pluck, 48_000, 48_000) >= 48_000 / 20);
        let mut mapped = sequence("sine", json!([{"step":0, "len":8, "pitch":"C4"}]));
        if let Node::Seq { tempo_map, .. } = &mut mapped {
            *tempo_map = serde_json::from_value(json!([
                {"at":{"num":0,"den":1}, "bpm":120},
                {"at":{"num":1,"den":1}, "bpm":60}
            ]))
            .unwrap();
        }
        // First beat takes .5 s; second takes 1 s. Shared tempo-map timing
        // counts the 1.5-second note rather than assuming fixed 120 BPM.
        assert!(sequence_work(&mapped, 16_000, 8_000) >= 12_000 * 7);
    }

    #[test]
    fn every_authored_bgm_fits_the_browser_resource_limits() {
        let mut failures = Vec::new();
        let mut max_graph_work = 0;
        let mut max_note_work = 0;
        for composition in tono_core::bgm::COMPOSITIONS {
            for variant in tono_core::bgm::VARIANTS {
                let spec = tono_core::bgm::BgmSpec::new(composition.id, variant, 42);
                let doc = tono_core::bgm::generate(&spec).unwrap();
                assert!(doc.duration <= MAX_DURATION_SECONDS);
                assert!(doc.sample_rate <= MAX_SAMPLE_RATE);
                assert!(serde_json::to_vec(&doc).unwrap().len() <= MAX_INPUT_BYTES);
                let frames = (doc.duration * doc.sample_rate as f32).ceil() as u64;
                let mut work = frames * 2;
                let mut note_work = 0;
                if let Err(error) = check_graph_with_notes(
                    &doc.root,
                    1,
                    frames,
                    doc.sample_rate,
                    &mut 0,
                    &mut work,
                    &mut note_work,
                ) {
                    failures.push(format!("{} / {}: {error}", composition.id, variant.id()));
                }
                max_graph_work = max_graph_work.max(work);
                max_note_work = max_note_work.max(note_work);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        eprintln!("BGM max graph work: {max_graph_work}; max note work: {max_note_work}");
    }
}
