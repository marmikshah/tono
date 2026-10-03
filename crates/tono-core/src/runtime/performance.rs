//! performance — a running Program: sample-accurate transport, a bounded
//! scheduled-command queue, stingers, click-free program swaps, metrics, and
//! deterministic command capture/replay (ADR 0005).
//!
//! A [`Performance`] is the runtime half of `Song::compile` → `Program`: the
//! host schedules commands at frames, beats, bars, markers, or sections, and
//! the render executes them at exact frames in submission order — no Python,
//! game loop, or OS timer ever needs to wake on a musical boundary. The
//! callback path performs no allocation beyond first-use growth of the
//! pre-sized scratch (see [`SCRATCH_FRAMES`]): a stinger is rendered at
//! schedule time, so firing one mid-callback only mixes a pre-rendered
//! buffer — no render, no allocation on the render path.
//!
//! This API is **stable** — frozen at 1.10.0-rc.1 (docs/api-tiers.md).

use std::collections::VecDeque;
use std::sync::Arc;

use super::engine::Ramp;
use super::{AudioSource, SCRATCH_FRAMES, StreamSource, Transport, TransportState, Tween};
use crate::dsl::SoundDoc;
use crate::program::Program;

/// The command queue's capacity. A full queue rejects the new command (the
/// caller decides what to drop) and counts it — the defined exhaustion
/// behavior (ADR 0005).
pub const COMMAND_QUEUE_CAP: usize = 4096;

/// The program-swap crossfade length in frames (~21 ms at 48 kHz): long
/// enough to be click-free, short enough to read as a cut at a bar line.
const SWAP_FADE_FRAMES: usize = 1024;

/// The master-gain ramp length in frames: gain rides are click-free without
/// waiting a beat.
const GAIN_RAMP_FRAMES: usize = 256;

/// Where a command lands on the timeline, resolved to an exact frame at
/// schedule time (the transport lives on the control side, so musical
/// resolution never touches the render path).
#[derive(Debug, Clone, PartialEq)]
pub enum At {
    /// The next frame.
    Immediate,
    /// An absolute frame.
    Frame(u64),
    /// An absolute beat (through the tempo map).
    Beat(f64),
    /// An absolute bar (through the meter map and pickup).
    Bar(u32),
    /// The next whole beat after the current position.
    NextBeat,
    /// The next bar line after the current position.
    NextBar,
    /// A named marker's position.
    Marker(String),
    /// A named section's first bar.
    Section(String),
}

/// One schedulable runtime command.
#[derive(Debug, Clone)]
pub enum Command {
    /// Start or resume the transport.
    Play,
    /// Hold the transport.
    Pause,
    /// Stop and rewind.
    Stop,
    /// Seek to a beat (the song source follows, deterministically).
    SeekBeat(f64),
    /// Seek to a bar.
    SeekBar(u32),
    /// Seek to a named section's first bar.
    SeekSection(String),
    /// Loop a bar range.
    SetLoopBars(u32, u32),
    /// Clear the loop.
    ClearLoop,
    /// Master gain (ramped, click-free).
    SetGain(f32),
    /// Swap to a different program (crossfaded; the new program starts from
    /// its frame 0 with its own transport).
    Swap(Arc<Program>),
    /// Fire a one-shot over the song (already rendered — the render happened
    /// at schedule time, never on the render path).
    Stinger {
        /// The pre-rendered interleaved stereo samples. Owning the buffer in
        /// the command makes captures self-contained and releases it when no
        /// queued, captured, or active stinger refers to it.
        samples: Arc<[f32]>,
        /// The stinger's gain.
        gain: f32,
    },
}

/// A command pinned to an exact frame with its submission order — the unit
/// of deterministic replay.
#[derive(Debug, Clone)]
pub struct TimestampedCommand {
    /// The frame the command executes at.
    pub at_frame: u64,
    /// Submission order — the deterministic tie-break for identical frames.
    pub seq: u64,
    /// The command.
    pub command: Command,
}

/// Why a schedule call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerformanceError {
    /// The queue is full; the command was NOT accepted (and counted).
    QueueFull,
    /// The `At` named a marker or section the program doesn't have.
    UnknownPosition(String),
    /// The swap target failed to load (hash or version — the last valid
    /// program keeps running).
    BadProgram(String),
}

impl std::fmt::Display for PerformanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PerformanceError::QueueFull => {
                f.write_str("the command queue is full — the command was rejected and counted")
            }
            PerformanceError::UnknownPosition(name) => {
                write!(f, "no marker or section named '{name}'")
            }
            PerformanceError::BadProgram(why) => write!(f, "program rejected: {why}"),
        }
    }
}

impl std::error::Error for PerformanceError {}

/// A point-in-time health snapshot — read off the audio path; all counters
/// advance on the render side without formatting or allocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerformanceMetrics {
    /// Frames rendered since start.
    pub frames_rendered: u64,
    /// Commands executed at their exact frames.
    pub commands_executed: u64,
    /// Commands rejected by a full queue.
    pub commands_dropped: u64,
    /// Deepest the queue has been.
    pub queue_depth_max: usize,
    /// Program swaps performed.
    pub swaps: u64,
    /// Stingers fired.
    pub stingers_fired: u64,
}

/// The song's playback source: native streaming when the program is
/// streamable (byte-identical to the bounce), the pre-rendered bounce
/// otherwise. Seeks are deterministic: the stream rebuilds and fast-forwards
/// (O(distance)); the buffer just moves its cursor.
enum SongSource {
    Stream {
        doc: Box<SoundDoc>,
        source: Box<StreamSource>,
        inter: Vec<f32>,
    },
    Buffer {
        left: Vec<f32>,
        right: Vec<f32>,
        pos: usize,
    },
}

impl SongSource {
    /// Build the source for a program: native streaming when possible, the
    /// pre-rendered bounce otherwise.
    fn build(program: &Program) -> SongSource {
        if let Some(source) = StreamSource::from_doc(&program.doc) {
            SongSource::Stream {
                doc: Box::new(program.doc.clone()),
                source: Box::new(source),
                inter: vec![0.0; SCRATCH_FRAMES * 2],
            }
        } else {
            let (left, right) = program.render_stereo();
            SongSource::Buffer {
                left,
                right,
                pos: 0,
            }
        }
    }

    fn fill(&mut self, left: &mut [f32], right: &mut [f32]) {
        match self {
            SongSource::Stream { source, inter, .. } => {
                let n = left.len();
                if inter.len() < n * 2 {
                    inter.resize(n * 2, 0.0);
                }
                source.fill(&mut inter[..n * 2]);
                for i in 0..n {
                    left[i] = inter[i * 2];
                    right[i] = inter[i * 2 + 1];
                }
            }
            SongSource::Buffer {
                left: l,
                right: r,
                pos,
            } => {
                let n = left.len();
                let avail = l.len().saturating_sub(*pos);
                let take = avail.min(n);
                left[..take].copy_from_slice(&l[*pos..*pos + take]);
                right[..take].copy_from_slice(&r[*pos..*pos + take]);
                for i in take..n {
                    left[i] = 0.0;
                    right[i] = 0.0;
                }
                *pos += take;
            }
        }
    }

    fn seek(&mut self, frame: usize) {
        match self {
            SongSource::Stream { doc, source, .. } => {
                **source = StreamSource::from_doc(doc).expect("was streamable");
                let mut remaining = frame;
                let mut block = [0.0f32; 2048];
                while remaining > 0 {
                    let take = remaining.min(1024);
                    source.fill(&mut block[..take * 2]);
                    remaining -= take;
                }
            }
            SongSource::Buffer { pos, .. } => *pos = frame,
        }
    }
}

/// A sounding stinger: a buffer pre-rendered at schedule time with its play
/// head and declick gain ramp. Mixed into the output without touching the
/// allocator (capacity is reserved when the stinger is scheduled).
struct ActiveStinger {
    /// The interleaved stereo buffer, rendered at the program's sample rate.
    buf: Arc<[f32]>,
    /// Play head in samples (frames × 2).
    pos: usize,
    /// The declick ramp: 1.0 → the scheduled gain over 2 ms (the same shape
    /// the engine applies to a fresh one-shot).
    gain: Ramp,
}

/// A running program — see the module docs.
pub struct Performance {
    program: Arc<Program>,
    song: SongSource,
    transport: Transport,
    queue: VecDeque<TimestampedCommand>,
    /// Program sources pre-built at schedule time, keyed by the command's
    /// seq — the build (a full probe render or bounce) never happens on the
    /// audio path (same discipline as stinger pre-rendering).
    swap_sources: std::collections::HashMap<u64, SongSource>,
    clock: u64,
    seq: u64,
    master_gain: f32,
    gain_ramp: Option<(f32, f32, usize)>, // (from, to, frames left)
    fade: Option<(SongSource, usize)>,    // (outgoing source, frames left)
    metrics: PerformanceMetrics,
    capture: Option<Vec<TimestampedCommand>>,
    sample_rate: u32,
    /// Stingers sounding now (pre-reserved at schedule time).
    stingers: Vec<ActiveStinger>,
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    scratch_fl: Vec<f32>,
    scratch_fr: Vec<f32>,
    scratch_e: Vec<f32>,
}

impl Performance {
    /// Load a compiled program, stopped at frame 0. The engine's sample rate
    /// is the program's.
    pub fn new(program: Arc<Program>) -> Self {
        let sr = program.doc.sample_rate;
        let transport = Transport::for_program(&program);
        let song = SongSource::build(&program);
        Performance {
            program,
            song,
            transport,
            queue: VecDeque::with_capacity(COMMAND_QUEUE_CAP.min(64)),
            swap_sources: std::collections::HashMap::new(),
            clock: 0,
            seq: 0,
            master_gain: 1.0,
            gain_ramp: None,
            fade: None,
            metrics: PerformanceMetrics::default(),
            capture: None,
            sample_rate: sr,
            stingers: Vec::new(),
            scratch_l: vec![0.0; SCRATCH_FRAMES],
            scratch_r: vec![0.0; SCRATCH_FRAMES],
            scratch_fl: vec![0.0; SCRATCH_FRAMES],
            scratch_fr: vec![0.0; SCRATCH_FRAMES],
            scratch_e: vec![0.0; SCRATCH_FRAMES * 2],
        }
    }

    /// The running program.
    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    /// The transport (position, loops — seek helpers are also commands).
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// The current master gain.
    pub fn master_gain(&self) -> f32 {
        self.master_gain
    }

    /// A health snapshot (queue depth is sampled at call time).
    pub fn metrics(&self) -> PerformanceMetrics {
        self.metrics.clone()
    }

    /// The current queue depth.
    pub fn queue_depth(&self) -> usize {
        self.queue.len()
    }

    /// The command clock (frames rendered).
    pub fn clock(&self) -> u64 {
        self.clock
    }

    /// Resolve an [`At`] to an absolute frame, now.
    fn resolve(&self, at: &At) -> Result<u64, PerformanceError> {
        Ok(match at {
            At::Immediate => self.clock,
            At::Frame(f) => *f,
            At::Beat(b) => self.deadline_for_transport_frame(self.transport.frame_at_beat(*b)),
            At::Bar(b) => self.deadline_for_transport_frame(self.transport.frame_at_bar(*b)),
            At::NextBeat => {
                let pos = self.transport.position_beats();
                self.deadline_for_transport_frame(self.transport.frame_at_beat(pos.floor() + 1.0))
            }
            At::NextBar => self.deadline_for_transport_frame(
                self.transport
                    .frame_at_bar(self.transport.position_bars().floor() as u32 + 1),
            ),
            At::Marker(name) => {
                let marker = self
                    .program
                    .meta
                    .markers
                    .iter()
                    .find(|m| &m.name == name)
                    .ok_or_else(|| PerformanceError::UnknownPosition(name.clone()))?;
                self.deadline_for_transport_frame(self.transport.frame_at_beat(marker.at.to_f64()))
            }
            At::Section(name) => {
                let section = self
                    .program
                    .meta
                    .sections
                    .iter()
                    .find(|s| &s.name == name)
                    .ok_or_else(|| PerformanceError::UnknownPosition(name.clone()))?;
                self.deadline_for_transport_frame(self.transport.frame_at_bar(section.bar))
            }
        })
    }

    /// Map a song-timeline frame to the render clock used by the queue. Seeks,
    /// pauses, loops, and swaps let those clocks diverge, so musical positions
    /// are distances from the current playhead rather than raw song frames.
    fn deadline_for_transport_frame(&self, target: u64) -> u64 {
        let position = self.transport.position_frames();
        let distance = if target >= position {
            target - position
        } else if let Some((start, end)) = self.transport.loop_range()
            && (start..end).contains(&target)
            && position < end
        {
            (end - position).saturating_add(target - start)
        } else {
            0
        };
        self.clock.saturating_add(distance)
    }

    fn require_queue_room(&mut self) -> Result<(), PerformanceError> {
        if self.queue.len() < COMMAND_QUEUE_CAP {
            return Ok(());
        }
        self.metrics.commands_dropped += 1;
        Err(PerformanceError::QueueFull)
    }

    fn enqueue(&mut self, command: Command, frame: u64) -> u64 {
        debug_assert!(self.queue.len() < COMMAND_QUEUE_CAP);
        self.seq += 1;
        let stamped = TimestampedCommand {
            at_frame: frame,
            seq: self.seq,
            command,
        };
        // Swap sources build HERE, at schedule time (a full probe render or
        // bounce — O(duration)), never inside `fill` (see execute).
        if let Command::Swap(program) = &stamped.command {
            self.swap_sources
                .insert(stamped.seq, SongSource::build(program));
        }
        let pos = self
            .queue
            .iter()
            .position(|c| (c.at_frame, c.seq) > (stamped.at_frame, stamped.seq))
            .unwrap_or(self.queue.len());
        if let Some(capture) = &mut self.capture {
            capture.push(stamped.clone());
        }
        self.queue.insert(pos, stamped);
        self.metrics.queue_depth_max = self.metrics.queue_depth_max.max(self.queue.len());
        self.seq
    }

    /// Schedule a command at `at`. Musical positions resolve to exact frames
    /// now; identical frames execute in submission order. A full queue
    /// rejects the command (counted in the metrics).
    pub fn schedule(&mut self, command: Command, at: At) -> Result<u64, PerformanceError> {
        let frame = self.resolve(&at)?;
        self.require_queue_room()?;
        Ok(self.enqueue(command, frame))
    }

    /// Schedule a stinger: render the doc NOW — the fire inside [`fill`](AudioSource::fill)
    /// then only mixes a pre-rendered buffer, so the render path never
    /// renders or allocates at fire time — and fire it at `at` with `gain`.
    pub fn stinger(&mut self, doc: &SoundDoc, gain: f32, at: At) -> Result<u64, PerformanceError> {
        let frame = self.resolve(&at)?;
        self.require_queue_room()?;
        // Stingers render at the program's rate: the runtime's one internal
        // rate (resampling to the device belongs at the adapter).
        let mut doc = doc.clone();
        doc.sample_rate = self.sample_rate;
        let (left, right) = crate::player::render_stereo(&doc);
        let mut buf = Vec::with_capacity(left.len() * 2);
        for i in 0..left.len() {
            buf.push(left[i]);
            buf.push(right[i]);
        }
        let samples: Arc<[f32]> = buf.into();
        // Pre-reserve voice room for every queued stinger (including this
        // one): the fire then never grows the Vec on the render path.
        let pending = self
            .queue
            .iter()
            .filter(|c| matches!(c.command, Command::Stinger { .. }))
            .count()
            + 1;
        self.stingers.reserve(pending);
        Ok(self.enqueue(Command::Stinger { samples, gain }, frame))
    }

    /// Schedule a program swap (crossfaded at `at`). The target must load
    /// clean — a rejected target changes nothing (the last valid program
    /// keeps running).
    pub fn swap_to(&mut self, program: Arc<Program>, at: At) -> Result<u64, PerformanceError> {
        program
            .verify()
            .map_err(|e| PerformanceError::BadProgram(e.to_string()))?;
        if program.doc.sample_rate != self.sample_rate {
            return Err(PerformanceError::BadProgram(format!(
                "sample rate mismatch — performance is {} Hz, program is {} Hz",
                self.sample_rate, program.doc.sample_rate
            )));
        }
        self.schedule(Command::Swap(program), at)
    }

    /// A quantized section transition: seek to the section's first bar at
    /// the next bar line (or immediately with `At::Immediate`). The latest
    /// transition wins: scheduling one while another is pending drops the
    /// older pending seek (defined interruption behavior). An unknown
    /// section is rejected now — never a silent no-op later.
    pub fn transition_to_section(&mut self, name: &str, at: At) -> Result<u64, PerformanceError> {
        if !self.program.meta.sections.iter().any(|s| s.name == name) {
            return Err(PerformanceError::UnknownPosition(name.to_string()));
        }
        let frame = self.resolve(&at)?;
        // Drop pending section seeks; executed ones are history.
        let replaced: Vec<u64> = self
            .queue
            .iter()
            .filter(|c| matches!(c.command, Command::SeekSection(_)))
            .map(|c| c.seq)
            .collect();
        self.queue
            .retain(|c| !matches!(c.command, Command::SeekSection(_)));
        if let Some(capture) = &mut self.capture {
            capture.retain(|c| !replaced.contains(&c.seq));
        }
        self.require_queue_room()?;
        Ok(self.enqueue(Command::SeekSection(name.to_string()), frame))
    }

    /// Start recording scheduled commands for deterministic replay.
    pub fn start_capture(&mut self) {
        self.capture = Some(Vec::new());
    }

    /// Stop recording and take the captured commands.
    pub fn stop_capture(&mut self) -> Vec<TimestampedCommand> {
        self.capture.take().unwrap_or_default()
    }

    /// Replay captured commands at their recorded frames, in order.
    pub fn replay(&mut self, commands: &[TimestampedCommand]) {
        // Same pre-reservation as `stinger()`: replayed stingers must not
        // grow the voice Vec at fire time either (control side here).
        let stingers = commands
            .iter()
            .filter(|c| matches!(c.command, Command::Stinger { .. }))
            .count();
        if stingers > 0 {
            self.stingers.reserve(stingers);
        }
        for c in commands {
            // Bypass capture (a replay isn't a new session) and the queue cap
            // is respected: a captured queue always fits again. Swap sources
            // build here too — replay IS schedule time.
            let frame = c.at_frame;
            if self.queue.len() >= COMMAND_QUEUE_CAP {
                self.metrics.commands_dropped += 1;
                continue;
            }
            self.seq += 1;
            if let Command::Swap(program) = &c.command {
                self.swap_sources
                    .insert(self.seq, SongSource::build(program));
            }
            self.queue.push_back(TimestampedCommand {
                at_frame: frame,
                seq: self.seq,
                command: c.command.clone(),
            });
        }
        self.queue
            .make_contiguous()
            .sort_by_key(|c| (c.at_frame, c.seq));
    }

    /// A state snapshot: transport position/state, master gain, loop range.
    /// Applying it returns the performance to this exact control state.
    pub fn snapshot(&self) -> PerformanceSnapshot {
        PerformanceSnapshot {
            position: self.transport.position_frames(),
            state: self.transport.state(),
            master_gain: self.master_gain,
            loop_range: self.transport.loop_range(),
        }
    }

    /// Restore a snapshot (deterministic: the song source re-seeks).
    pub fn apply_snapshot(&mut self, snapshot: &PerformanceSnapshot) {
        self.master_gain = snapshot.master_gain;
        if let Some((start, end)) = snapshot.loop_range {
            self.transport.set_loop_frames(start, end);
        } else {
            self.transport.clear_loop();
        }
        match snapshot.state {
            TransportState::Stopped => self.transport.stop(),
            TransportState::Playing => self.transport.play(),
            TransportState::Paused => self.transport.pause(),
        }
        self.transport.seek_frame(snapshot.position);
        self.song.seek(self.transport.position_frames() as usize);
    }

    /// Execute one command (at its exact frame).
    fn execute(&mut self, stamped: TimestampedCommand) {
        self.metrics.commands_executed += 1;
        match stamped.command {
            Command::Play => self.transport.play(),
            Command::Pause => self.transport.pause(),
            Command::Stop => {
                self.transport.stop();
                self.song.seek(0);
            }
            Command::SeekBeat(beat) => {
                let frame = self.transport.frame_at_beat(beat);
                self.transport.seek_frame(frame);
                self.song.seek(frame as usize);
            }
            Command::SeekBar(bar) => {
                let frame = self.transport.frame_at_bar(bar);
                self.transport.seek_frame(frame);
                self.song.seek(frame as usize);
            }
            Command::SeekSection(name) => {
                if let Some(section) = self.program.meta.sections.iter().find(|s| s.name == name) {
                    let frame = self.transport.frame_at_bar(section.bar);
                    self.transport.seek_frame(frame);
                    self.song.seek(frame as usize);
                }
            }
            Command::SetLoopBars(start, end) => {
                self.transport.set_loop_bars(start, end);
            }
            Command::ClearLoop => self.transport.clear_loop(),
            Command::SetGain(gain) => {
                let from = self.current_gain();
                self.master_gain = gain.clamp(0.0, 2.0);
                self.gain_ramp = Some((from, self.master_gain, GAIN_RAMP_FRAMES));
            }
            Command::Swap(program) => {
                // The source was pre-built at schedule time (off the audio
                // path); a foreign id (a hand-built command that never went
                // through schedule) is inert, like a foreign stinger.
                if let Some(new_source) = self.swap_sources.remove(&stamped.seq) {
                    let outgoing = std::mem::replace(&mut self.song, new_source);
                    self.fade = Some((outgoing, SWAP_FADE_FRAMES));
                    self.transport = Transport::for_program(&program);
                    self.transport.play();
                    self.program = program;
                    self.metrics.swaps += 1;
                }
            }
            Command::Stinger { samples, gain } => {
                let mut ramp = Ramp::new(1.0);
                ramp.set(gain.max(0.0), Tween::ms(2.0, self.sample_rate));
                self.stingers.push(ActiveStinger {
                    buf: samples,
                    pos: 0,
                    gain: ramp,
                });
                self.metrics.stingers_fired += 1;
            }
        }
    }

    /// The gain currently applied (mid-ramp aware).
    fn current_gain(&self) -> f32 {
        match &self.gain_ramp {
            Some((from, to, left)) => {
                from + (to - from) * (1.0 - *left as f32 / GAIN_RAMP_FRAMES as f32)
            }
            None => self.master_gain,
        }
    }

    /// Render one slice of song + stingers + fade into the interleaved
    /// output, advancing the transport. `out` is stereo-interleaved. All
    /// buffers are the pre-sized scratch fields — no allocation here.
    fn render_slice(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        if self.scratch_l.len() < frames {
            self.scratch_l.resize(frames, 0.0);
            self.scratch_r.resize(frames, 0.0);
            self.scratch_fl.resize(frames, 0.0);
            self.scratch_fr.resize(frames, 0.0);
        }
        if self.scratch_e.len() < out.len() {
            self.scratch_e.resize(out.len(), 0.0);
        }
        if self.transport.is_playing() {
            // Loop-aware: a slice spanning the loop end is rendered in
            // chunks, so the post-wrap part of the slice comes from the
            // loop start — not from past the loop end.
            let mut done = 0usize;
            while done < frames {
                let chunk = match self.transport.loop_range() {
                    Some((_, end)) => {
                        let pos = self.transport.position_frames();
                        (end.saturating_sub(pos) as usize).min(frames - done)
                    }
                    None => frames - done,
                };
                if chunk == 0 {
                    // At the loop end: wrap before rendering further.
                    let advance = self.transport.advance(0);
                    if advance.wrapped
                        && let Some((start, _)) = self.transport.loop_range()
                    {
                        self.song.seek(start as usize);
                    }
                    continue;
                }
                self.song.fill(
                    &mut self.scratch_l[done..done + chunk],
                    &mut self.scratch_r[done..done + chunk],
                );
                let advance = self.transport.advance(chunk as u64);
                done += chunk;
                if advance.wrapped
                    && let Some((start, _)) = self.transport.loop_range()
                {
                    self.song.seek(start as usize);
                }
                if advance.finished {
                    // Stopped at the program end: silence the rest.
                    self.scratch_l[done..frames].fill(0.0);
                    self.scratch_r[done..frames].fill(0.0);
                    break;
                }
            }
        } else {
            self.scratch_l[..frames].fill(0.0);
            self.scratch_r[..frames].fill(0.0);
        }
        // The outgoing program's tail during a swap crossfade (rendered here;
        // its weight advances per frame in the mix loop below).
        let fading = self.fade.is_some();
        if fading {
            let (outgoing, _) = self.fade.as_mut().expect("checked");
            outgoing.fill(
                &mut self.scratch_fl[..frames],
                &mut self.scratch_fr[..frames],
            );
        }
        // Stingers: pre-rendered buffers mixed straight in — no render, no
        // allocation on this path (the render happened at schedule time).
        {
            let out_frames = frames;
            let stereo = &mut self.scratch_e[..out.len()];
            stereo.fill(0.0);
            for s in &mut self.stingers {
                let take = ((s.buf.len() - s.pos) / 2).min(out_frames);
                for f in 0..take {
                    let g = s.gain.tick();
                    stereo[f * 2] += s.buf[s.pos + f * 2] * g;
                    stereo[f * 2 + 1] += s.buf[s.pos + f * 2 + 1] * g;
                }
                s.pos += take * 2;
            }
            self.stingers.retain(|s| s.pos < s.buf.len());
        }
        // The gain ramp and the swap crossfade advance per FRAME, so their
        // trajectories are identical under any block size — a slice's length
        // must never shape the ramp (the transport and command frames are
        // already blocking-invariant).
        let mut fade_left = self.fade.as_ref().map(|(_, left)| *left);
        for i in 0..frames {
            let g = if let Some((from, to, left)) = self.gain_ramp {
                let u = (GAIN_RAMP_FRAMES - left) as f32 / GAIN_RAMP_FRAMES as f32;
                self.gain_ramp = (left > 1).then_some((from, to, left - 1));
                from + (to - from) * u
            } else {
                self.master_gain
            };
            let mut left = self.scratch_l[i] * g;
            let mut right = self.scratch_r[i] * g;
            if let Some(fl) = fade_left.filter(|fl| *fl > 0) {
                let fade_u = fl as f32 / SWAP_FADE_FRAMES as f32;
                fade_left = Some(fl - 1);
                left += self.scratch_fl[i] * fade_u;
                right += self.scratch_fr[i] * fade_u;
            }
            out[i * 2] = left + self.scratch_e[i * 2];
            out[i * 2 + 1] = right + self.scratch_e[i * 2 + 1];
        }
        // Commit the crossfade progress: done when its frames ran out.
        if let Some((_, left)) = &mut self.fade {
            match fade_left {
                Some(fl) if fl > 0 => *left = fl,
                _ => self.fade = None,
            }
        }
        self.metrics.frames_rendered += frames as u64;
    }
}

impl AudioSource for Performance {
    /// Render interleaved stereo, executing due commands at their exact
    /// frames (submission order on ties) — the block is split at command
    /// boundaries so a command never lands early or late.
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        let mut done = 0usize;
        while done < frames {
            let due = self
                .queue
                .front()
                .filter(|c| c.at_frame <= self.clock + (frames - done) as u64)
                .map(|c| c.at_frame);
            match due {
                Some(at) => {
                    let at = (at.saturating_sub(self.clock) as usize).min(frames - done);
                    if at > 0 {
                        self.render_slice(&mut out[done * 2..(done + at) * 2]);
                        self.clock += at as u64;
                        done += at;
                    }
                    let stamped = self.queue.pop_front().expect("front was due");
                    self.execute(stamped);
                }
                None => {
                    self.render_slice(&mut out[done * 2..]);
                    self.clock += (frames - done) as u64;
                    done = frames;
                }
            }
        }
        frames
    }
}

/// A point-in-time control snapshot (see [`Performance::snapshot`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceSnapshot {
    /// Transport position in frames.
    pub position: u64,
    /// Transport state.
    pub state: TransportState,
    /// Master gain.
    pub master_gain: f32,
    /// Loop range in frames.
    pub loop_range: Option<(u64, u64)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{Adsr, SeqWave};
    use crate::song::{CompileOptions, Song, note};

    fn amp() -> Adsr {
        Adsr {
            a: 0.005,
            d: 0.1,
            s: 0.8,
            r: 0.2,
            punch: 0.0,
        }
    }

    fn demo_program() -> Arc<Program> {
        let mut song = Song::new("perf-demo", 120.0);
        song.add_track("bass", SeqWave::Bass, amp());
        song.add_track("keys", SeqWave::Epiano, amp());
        song.add_pattern("riff", 1, vec![note(0, 4, "C2"), note(8, 4, "G2")]);
        song.add_pattern("stab", 1, vec![note(0, 2, "C4"), note(6, 2, "D#4")]);
        song.arrange_repeat("bass", "riff", 0, 4);
        song.arrange_repeat("keys", "stab", 0, 4);
        song.sections.push(crate::song::Section {
            name: "second".into(),
            bar: 2,
            bars: 2,
        });
        Arc::new(song.compile(&CompileOptions::default()).unwrap())
    }

    fn other_program() -> Arc<Program> {
        let mut song = Song::new("perf-other", 100.0);
        song.add_track("lead", SeqWave::Square, amp());
        song.tracks[0].notes.push(note(0, 16, "A4"));
        Arc::new(song.compile(&CompileOptions::default()).unwrap())
    }

    fn other_program_at(sample_rate: u32) -> Arc<Program> {
        let mut song = Song::new("perf-other-rate", 100.0);
        song.add_track("lead", SeqWave::Square, amp());
        song.tracks[0].notes.push(note(0, 16, "A4"));
        Arc::new(
            song.compile(&CompileOptions {
                sample_rate: Some(sample_rate),
                ..CompileOptions::default()
            })
            .unwrap(),
        )
    }

    fn stinger_doc() -> SoundDoc {
        serde_json::from_str(
            r#"{ "name": "blip", "duration": 0.2, "root": { "type": "mul", "inputs": [
                { "type": "sawtooth", "freq": 880 },
                { "type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.01 } ] } }"#,
        )
        .unwrap()
    }

    fn bounce_interleaved(program: &Program) -> Vec<f32> {
        let (l, r) = program.render_stereo();
        let mut out = Vec::with_capacity(l.len() * 2);
        for i in 0..l.len() {
            out.push(l[i]);
            out.push(r[i]);
        }
        out
    }

    fn fill_all(p: &mut Performance, frames: usize, block: usize) -> Vec<f32> {
        let mut got = Vec::with_capacity(frames * 2);
        while got.len() < frames * 2 {
            let take = block.min(frames - got.len() / 2);
            let mut buf = vec![0.0f32; take * 2];
            p.fill(&mut buf);
            got.extend_from_slice(&buf);
        }
        got
    }

    fn bits(s: &[f32]) -> Vec<u32> {
        s.iter().map(|x| x.to_bits()).collect()
    }

    #[test]
    fn plays_byte_identical_to_the_bounce() {
        let program = demo_program();
        let expected = bounce_interleaved(&program);
        let mut p = Performance::new(program.clone());
        assert!(program.is_streamable(), "the demo streams natively");
        p.schedule(Command::Play, At::Immediate).unwrap();
        for block in [1usize, 7, 64, 333, 4096] {
            let mut p = Performance::new(program.clone());
            p.schedule(Command::Play, At::Immediate).unwrap();
            let got = fill_all(&mut p, expected.len() / 2, block);
            assert_eq!(bits(&got), bits(&expected), "block size {block} diverged");
        }
    }

    #[test]
    fn scheduled_gain_lands_on_the_exact_frame() {
        let program = demo_program();
        let expected = bounce_interleaved(&program);
        let at = 10_000usize;
        let mut p = Performance::new(program);
        p.schedule(Command::Play, At::Immediate).unwrap();
        p.schedule(Command::SetGain(0.0), At::Frame(at as u64))
            .unwrap();
        let got = fill_all(&mut p, expected.len() / 2, 512);
        // Before the command: byte-identical to the bounce.
        assert_eq!(bits(&got[..at * 2]), bits(&expected[..at * 2]));
        // Well after it (past the ramp and its block-boundary wind-down):
        // silence.
        let tail_rms: f32 = {
            let start = (at + 1_024) * 2;
            let sum: f32 = got[start..].iter().map(|x| x * x).sum();
            (sum / (got.len() - start) as f32).sqrt()
        };
        assert_eq!(tail_rms, 0.0, "gain 0 after the ramp");
        assert_eq!(p.metrics().commands_executed, 2);
    }

    #[test]
    fn seek_and_loop_reproduce_the_bounce_region() {
        let program = demo_program();
        let expected = bounce_interleaved(&program);
        // Seek to bar 2 (beat 8, 4 s at 120 BPM — wait: 8 beats × 0.5 s = 4 s;
        // the demo is longer than that? length is 4 bars + tail).
        let bar2 = {
            let t = Transport::for_program(&program);
            t.frame_at_bar(2) as usize
        };
        let mut p = Performance::new(program.clone());
        p.schedule(Command::SeekBar(2), At::Immediate).unwrap();
        p.schedule(Command::Play, At::Immediate).unwrap();
        let got = fill_all(&mut p, 8_000, 512);
        assert_eq!(bits(&got), bits(&expected[bar2 * 2..(bar2 + 8_000) * 2]));
        // Loop bars 1..2: the wrapped second pass repeats the region.
        let mut p = Performance::new(program);
        p.schedule(Command::SetLoopBars(1, 2), At::Immediate)
            .unwrap();
        p.schedule(Command::SeekBar(1), At::Immediate).unwrap();
        p.schedule(Command::Play, At::Immediate).unwrap();
        let bar1 = {
            let t = Transport::for_program(p.program());
            t.frame_at_bar(1) as usize
        };
        let span = bar2 - bar1;
        let got = fill_all(&mut p, span + 2_000, 256);
        assert_eq!(
            bits(&got[..2_000 * 2]),
            bits(&expected[bar1 * 2..(bar1 + 2_000) * 2]),
            "first pass"
        );
        assert_eq!(
            bits(&got[span * 2..(span + 2_000) * 2]),
            bits(&expected[bar1 * 2..(bar1 + 2_000) * 2]),
            "the wrapped pass repeats the loop region exactly"
        );
    }

    #[test]
    fn section_transition_lands_on_the_section_bar() {
        let program = demo_program();
        let expected = bounce_interleaved(&program);
        let bar2 = {
            let t = Transport::for_program(&program);
            t.frame_at_bar(2) as usize
        };
        let mut p = Performance::new(program);
        p.schedule(Command::Play, At::Immediate).unwrap();
        p.transition_to_section("second", At::Frame(5_000)).unwrap();
        let got = fill_all(&mut p, 12_000, 512);
        assert_eq!(
            bits(&got[5_000 * 2..7_000 * 2]),
            bits(&expected[bar2 * 2..(bar2 + 2_000) * 2]),
            "post-transition audio is the section's first bar"
        );
        // An unknown section is a structured error, not a silence.
        let mut p = Performance::new(demo_program());
        assert!(p.transition_to_section("nope", At::Immediate).is_err());
    }

    #[test]
    fn stinger_fires_on_the_exact_beat() {
        let program = demo_program();
        let stinger = stinger_doc();
        // Beat 4 at 120 BPM = 2 s = frame 88 200 at 44 100.
        let at = {
            let t = Transport::for_program(&program);
            t.frame_at_beat(4.0) as usize
        };
        let mut p = Performance::new(program.clone());
        p.schedule(Command::Play, At::Immediate).unwrap();
        p.stinger(&stinger, 1.0, At::Beat(4.0)).unwrap();
        let got = fill_all(&mut p, at + 4_000, 512);
        // The stinger's first sample lands exactly on the beat (the blip's
        // env has a 0 attack, so the onset is immediate and large).
        let before = &got[(at - 8) * 2..at * 2];
        let after = &got[at * 2..(at + 64) * 2];
        let bmax = before.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let amax = after.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(amax > bmax, "the stinger onset lands on the beat");
        // The onset lands within the beat's first frames (the blip's env has
        // a 0 attack, so sample 0 is 0 and the saw is audible immediately).
        let mix_window = &got[at * 2..(at + 64) * 2];
        let bounce = bounce_interleaved(&program);
        let diverges = mix_window
            .iter()
            .zip(&bounce[at * 2..(at + 64) * 2])
            .any(|(a, b)| (a - b).abs() > 1e-6);
        assert!(diverges, "the mix at the beat carries the stinger");
        assert_eq!(p.metrics().stingers_fired, 1);
    }

    #[test]
    fn swap_crossfades_deterministically() {
        let run = || {
            let mut p = Performance::new(demo_program());
            p.schedule(Command::Play, At::Immediate).unwrap();
            p.swap_to(other_program(), At::Frame(20_000)).unwrap();
            fill_all(&mut p, 60_000, 512)
        };
        let a = run();
        let b = run();
        assert_eq!(bits(&a), bits(&b), "the swap is deterministic");
        // Click-free: no outlier discontinuity at the swap point (the
        // crossfade blends the two programs).
        let window = &a[19_000 * 2..21_500 * 2];
        let max_step = window
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_step < 1.5, "no click at the swap ({max_step})");
        assert_eq!(Performance::new(demo_program()).metrics().swaps, 0);
    }

    #[test]
    fn capture_and_replay_reproduces_the_take() {
        let program = demo_program();
        let scripted = |p: &mut Performance| {
            p.schedule(Command::Play, At::Immediate).unwrap();
            p.schedule(Command::SetGain(0.5), At::Frame(8_000)).unwrap();
            p.schedule(Command::SeekBar(1), At::Frame(12_000)).unwrap();
            p.schedule(Command::SetGain(1.0), At::Frame(16_000))
                .unwrap();
        };
        let mut p = Performance::new(program.clone());
        p.start_capture();
        scripted(&mut p);
        let captured = p.stop_capture();
        assert_eq!(captured.len(), 4);
        let take_a = fill_all(&mut p, 40_000, 512);

        let mut q = Performance::new(program);
        q.replay(&captured);
        let take_b = fill_all(&mut q, 40_000, 512);
        assert_eq!(bits(&take_a), bits(&take_b), "replay reproduces the take");
    }

    #[test]
    fn captured_stinger_replays_on_a_fresh_performance() {
        let program = demo_program();
        let mut p = Performance::new(program.clone());
        p.start_capture();
        p.schedule(Command::Play, At::Immediate).unwrap();
        p.stinger(&stinger_doc(), 0.75, At::Frame(1_000)).unwrap();
        let captured = p.stop_capture();
        let take_a = fill_all(&mut p, 12_000, 333);

        let mut q = Performance::new(program);
        q.replay(&captured);
        let take_b = fill_all(&mut q, 12_000, 512);
        assert_eq!(bits(&take_a), bits(&take_b));
        assert_eq!(q.metrics().stingers_fired, 1);
    }

    #[test]
    fn musical_deadlines_are_relative_to_the_current_transport() {
        let mut p = Performance::new(demo_program());
        p.schedule(Command::Play, At::Immediate).unwrap();
        fill_all(&mut p, 1_000, 512);
        p.schedule(Command::Pause, At::Immediate).unwrap();
        fill_all(&mut p, 5_000, 512);
        assert_eq!(p.clock(), 6_000);
        assert_eq!(p.transport().position_frames(), 1_000);

        let next_beat = p.transport().frame_at_beat(1.0);
        p.schedule(Command::SetGain(0.5), At::NextBeat).unwrap();
        let queued = p.queue.back().unwrap();
        assert_eq!(queued.at_frame, 6_000 + next_beat - 1_000);
    }

    #[test]
    fn swap_rejects_a_different_sample_rate_in_core() {
        let mut p = Performance::new(demo_program());
        let err = p
            .swap_to(other_program_at(48_000), At::Immediate)
            .unwrap_err();
        assert!(matches!(err, PerformanceError::BadProgram(_)));
        assert_eq!(p.program().doc.sample_rate, 44_100);
    }

    #[test]
    fn a_full_queue_rejects_and_counts() {
        let mut p = Performance::new(demo_program());
        for i in 0..COMMAND_QUEUE_CAP {
            p.schedule(Command::SetGain(0.5), At::Frame(1_000_000 + i as u64))
                .unwrap();
        }
        let err = p.schedule(Command::Play, At::Immediate).unwrap_err();
        assert_eq!(err, PerformanceError::QueueFull);
        assert_eq!(p.metrics().commands_dropped, 1);
        assert_eq!(p.queue_depth(), COMMAND_QUEUE_CAP);
    }

    #[test]
    fn snapshot_restores_the_control_state() {
        let mut p = Performance::new(demo_program());
        p.schedule(Command::Play, At::Immediate).unwrap();
        fill_all(&mut p, 10_000, 512);
        let snap = p.snapshot();
        let take_a = fill_all(&mut p, 4_000, 512);
        p.apply_snapshot(&snap);
        let take_b = fill_all(&mut p, 4_000, 512);
        assert_eq!(
            bits(&take_a),
            bits(&take_b),
            "the snapshot replays the position"
        );

        let mut stopped = Performance::new(demo_program());
        stopped.transport.seek_frame(12_345);
        let snap = stopped.snapshot();
        stopped.apply_snapshot(&snap);
        assert_eq!(stopped.transport.state(), TransportState::Stopped);
        assert_eq!(stopped.transport.position_frames(), 12_345);
    }

    #[test]
    fn gain_ride_and_swap_fade_are_block_size_invariant() {
        // The ramp and crossfade advance per frame, so any blocking yields
        // the same bytes (the threaded soak hammers this across threads).
        let run_gain = |block: usize| {
            let mut p = Performance::new(demo_program());
            p.schedule(Command::Play, At::Immediate).unwrap();
            p.schedule(Command::SetGain(0.5), At::Frame(1_000)).unwrap();
            p.schedule(Command::SetGain(1.0), At::Frame(1_500)).unwrap();
            fill_all(&mut p, 8_000, block)
        };
        for block in [1usize, 7, 333, 512, 4096] {
            assert_eq!(
                bits(&run_gain(block)),
                bits(&run_gain(512)),
                "gain ride diverged at block size {block}"
            );
        }
        let run_swap = |block: usize| {
            let mut p = Performance::new(demo_program());
            p.schedule(Command::Play, At::Immediate).unwrap();
            p.swap_to(other_program(), At::Frame(1_000)).unwrap();
            fill_all(&mut p, 8_000, block)
        };
        for block in [1usize, 7, 333, 512, 4096] {
            assert_eq!(
                bits(&run_swap(block)),
                bits(&run_swap(512)),
                "swap crossfade diverged at block size {block}"
            );
        }
    }
}
