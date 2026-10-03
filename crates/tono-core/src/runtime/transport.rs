//! A sample-accurate clock for a compiled Program: position in frames with
//! exact conversions to beats and bars
//! through the program's tempo and meter maps. The transport owns no audio;
//! it answers "where am I" and "what frame is that", deterministically, so
//! scheduling never needs Python, a game loop, or an OS timer to wake on a
//! musical boundary.

use crate::dsl::TempoPoint;
use crate::units::{Beat, MeterPoint};

/// Transport playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    /// At frame 0 (or the loop start), not advancing.
    Stopped,
    /// Advancing with the render.
    Playing,
    /// Holding position, not advancing.
    Paused,
}

/// What one [`Transport::advance`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Advance {
    /// The playhead wrapped at the loop end this call.
    pub wrapped: bool,
    /// The playhead reached the end with no loop (the transport is now
    /// Stopped at the last frame).
    pub finished: bool,
}

/// A sample-accurate musical clock. All arithmetic before the single
/// frame-boundary crossings is the exact rational/f64 segment walk shared
/// with the compiler, so the transport and the offline render
/// can never disagree about where a beat lands.
#[derive(Debug, Clone)]
pub struct Transport {
    sample_rate: u32,
    bpm: f32,
    beats_per_bar: u32,
    tempo_map: Vec<TempoPoint>,
    meter_map: Vec<MeterPoint>,
    pickup: Option<Beat>,
    length_frames: u64,
    state: TransportState,
    position: u64,
    loop_range: Option<(u64, u64)>,
}

impl Transport {
    /// A transport for a compiled program, stopped at frame 0.
    pub fn for_program(program: &crate::program::Program) -> Self {
        let meta = &program.meta;
        Transport {
            sample_rate: program.doc.sample_rate,
            bpm: meta.tempo_bpm,
            beats_per_bar: meta.beats_per_bar,
            tempo_map: meta.tempo_map.clone(),
            meter_map: meta.meter_map.clone(),
            pickup: meta.pickup,
            length_frames: program.estimates.frames,
            state: TransportState::Stopped,
            position: 0,
            loop_range: None,
        }
    }

    /// The clock's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The current state.
    pub fn state(&self) -> TransportState {
        self.state
    }

    /// Whether the transport is advancing.
    pub fn is_playing(&self) -> bool {
        self.state == TransportState::Playing
    }

    /// Start or resume advancing.
    pub fn play(&mut self) {
        self.state = TransportState::Playing;
    }

    /// Hold position.
    pub fn pause(&mut self) {
        self.state = TransportState::Paused;
    }

    /// Stop and rewind to frame 0.
    pub fn stop(&mut self) {
        self.state = TransportState::Stopped;
        self.position = 0;
    }

    /// Seconds elapsed at `beat` (constant tempo, or the map's segment walk).
    fn seconds_at_beat(&self, beat: f64) -> f64 {
        if self.tempo_map.is_empty() {
            beat * 60.0 / self.bpm.max(1.0) as f64
        } else {
            crate::dsl::tempo_map_seconds_at(&self.tempo_map, beat)
        }
    }

    /// Beats elapsed at `seconds` (the inverse walk).
    fn beats_at_seconds(&self, seconds: f64) -> f64 {
        if self.tempo_map.is_empty() {
            seconds * self.bpm.max(1.0) as f64 / 60.0
        } else {
            crate::dsl::tempo_map_beat_at_seconds(&self.tempo_map, seconds)
        }
    }

    /// The frame a beat lands on (rounds halves away from zero).
    pub fn frame_at_beat(&self, beat: f64) -> u64 {
        (self.seconds_at_beat(beat) * self.sample_rate as f64)
            .round()
            .max(0.0) as u64
    }

    /// The beat at a frame (the inverse conversion).
    pub fn beat_at_frame(&self, frame: u64) -> f64 {
        self.beats_at_seconds(frame as f64 / self.sample_rate as f64)
    }

    /// The frame a bar starts on (through the meter map and pickup).
    pub fn frame_at_bar(&self, bar: u32) -> u64 {
        let beat = crate::units::beat_at_bar(&self.meter_map, self.beats_per_bar, self.pickup, bar);
        self.frame_at_beat(beat.to_f64())
    }

    /// The position in frames.
    pub fn position_frames(&self) -> u64 {
        self.position
    }

    /// The position in beats.
    pub fn position_beats(&self) -> f64 {
        self.beat_at_frame(self.position)
    }

    /// The position in bars (bar index plus intra-bar fraction).
    pub fn position_bars(&self) -> f64 {
        let beat = Beat::new((self.position_beats() * 1e9).round() as i64, 1_000_000_000);
        let bars =
            crate::units::bar_count_at_beat(&self.meter_map, self.beats_per_bar, self.pickup, beat);
        // bar_count gives bars elapsed with ceil semantics; the current bar
        // index is one less when the position sits inside it.
        let idx = bars.saturating_sub(1);
        let start =
            crate::units::beat_at_bar(&self.meter_map, self.beats_per_bar, self.pickup, idx);
        let len = crate::units::bar_len(&self.meter_map, self.beats_per_bar, self.pickup, idx);
        let within = if len > Beat::zero() {
            (beat.to_f64() - start.to_f64()) / len.to_f64()
        } else {
            0.0
        };
        idx as f64 + within.clamp(0.0, 1.0)
    }

    /// The program length in frames.
    pub fn length_frames(&self) -> u64 {
        self.length_frames
    }

    /// Seek to a frame (clamped to the program length).
    pub fn seek_frame(&mut self, frame: u64) {
        self.position = frame.min(self.length_frames);
    }

    /// Seek to a beat position.
    pub fn seek_beat(&mut self, beat: f64) {
        self.seek_frame(self.frame_at_beat(beat));
    }

    /// Seek to a bar (through the meter map and pickup).
    pub fn seek_bar(&mut self, bar: u32) {
        self.seek_frame(self.frame_at_bar(bar));
    }

    /// Loop a frame range [start, end). At `end` the playhead wraps to
    /// `start`; seeking outside the range is allowed (the loop engages when
    /// the playhead reaches `end`). Invalid ranges (empty, past the end) are
    /// rejected with `false` and leave the old range.
    pub fn set_loop_frames(&mut self, start: u64, end: u64) -> bool {
        if start >= end || end > self.length_frames {
            return false;
        }
        self.loop_range = Some((start, end));
        true
    }

    /// Loop a bar range (through the meter map), or report an invalid range.
    pub fn set_loop_bars(&mut self, start_bar: u32, end_bar: u32) -> bool {
        let (start, end) = (self.frame_at_bar(start_bar), self.frame_at_bar(end_bar));
        self.set_loop_frames(start, end)
    }

    /// Clear the loop range.
    pub fn clear_loop(&mut self) {
        self.loop_range = None;
    }

    /// The loop range in frames, if set.
    pub fn loop_range(&self) -> Option<(u64, u64)> {
        self.loop_range
    }

    /// Advance the playhead by `frames` (no-op unless Playing). At the loop
    /// end the playhead wraps; at the program end with no loop it stops.
    pub fn advance(&mut self, frames: u64) -> Advance {
        if self.state != TransportState::Playing {
            return Advance {
                wrapped: false,
                finished: false,
            };
        }
        let mut pos = self.position.saturating_add(frames);
        let mut wrapped = false;
        if let Some((start, end)) = self.loop_range
            && pos >= end
        {
            let span = end - start;
            pos = start + (pos - start) % span;
            wrapped = true;
        }
        if !wrapped && pos >= self.length_frames {
            self.position = self.length_frames;
            self.state = TransportState::Stopped;
            return Advance {
                wrapped: false,
                finished: true,
            };
        }
        self.position = pos;
        Advance {
            wrapped,
            finished: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped_transport() -> Transport {
        Transport {
            sample_rate: 48_000,
            bpm: 120.0,
            beats_per_bar: 4,
            tempo_map: vec![
                TempoPoint {
                    at: Beat::zero(),
                    bpm: 120.0,
                },
                TempoPoint {
                    at: Beat::from_int(4),
                    bpm: 240.0,
                },
            ],
            meter_map: vec![],
            pickup: None,
            length_frames: 192_000,
            state: TransportState::Stopped,
            position: 0,
            loop_range: None,
        }
    }

    #[test]
    fn frames_and_beats_cross_exactly() {
        let t = mapped_transport();
        assert_eq!(t.frame_at_beat(4.0), 96_000, "4 beats at 120 BPM = 2 s");
        assert_eq!(t.frame_at_beat(8.0), 144_000, "2 s at 120 + 1 s at 240");
        assert_eq!(t.beat_at_frame(96_000), 4.0);
        assert_eq!(t.beat_at_frame(144_000), 8.0);
        // The constant-tempo transport.
        let mut plain = mapped_transport();
        plain.tempo_map.clear();
        assert_eq!(plain.frame_at_beat(8.0), 192_000);
        assert_eq!(plain.beat_at_frame(192_000), 8.0);
    }

    #[test]
    fn bars_convert_through_meter_and_pickup() {
        let mut t = mapped_transport();
        t.tempo_map.clear();
        t.meter_map = vec![MeterPoint {
            bar: 0,
            numerator: 6,
            denominator: 8,
        }];
        assert_eq!(t.frame_at_bar(1), 72_000, "bar 1 of 6/8 = 3 beats = 1.5 s");
        t.pickup = Some(Beat::from_int(1));
        assert_eq!(t.frame_at_bar(1), 24_000, "bar 1 after a one-beat pickup");
        // Position in bars with a fraction: 3 beats is 2/3 through bar 1
        // (bar 0 = the 1-beat pickup, bar 1 = 3 beats).
        t.seek_frame(72_000);
        let bars = t.position_bars();
        assert!((bars - 1.6666666666666667).abs() < 1e-9, "{bars}");
    }

    #[test]
    fn play_pause_stop_seek() {
        let mut t = mapped_transport();
        assert_eq!(t.state(), TransportState::Stopped);
        assert!(!t.advance(100).finished);
        assert_eq!(t.position_frames(), 0, "stopped transport doesn't advance");
        t.play();
        let a = t.advance(48_000);
        assert!(!a.finished && !a.wrapped);
        assert_eq!(t.position_frames(), 48_000);
        t.pause();
        assert!(!t.advance(48_000).finished);
        assert_eq!(t.position_frames(), 48_000, "paused holds");
        t.seek_bar(1);
        assert_eq!(t.position_frames(), 96_000);
        t.stop();
        assert_eq!(t.position_frames(), 0);
        assert_eq!(t.state(), TransportState::Stopped);
    }

    #[test]
    fn loop_wraps_and_end_finishes() {
        let mut t = mapped_transport();
        assert!(t.set_loop_bars(1, 2));
        assert!(!t.set_loop_frames(96_000, 48_000), "empty range rejected");
        assert!(!t.set_loop_frames(0, 999_999), "past the end rejected");
        t.play();
        let a = t.advance(200_000);
        assert!(a.wrapped);
        assert_eq!(t.position_frames(), 96_000 + 8_000, "wrapped into the loop");
        // No loop: stops at the program end.
        t.clear_loop();
        t.seek_frame(190_000);
        let a = t.advance(48_000);
        assert!(a.finished);
        assert_eq!(t.position_frames(), 192_000);
        assert_eq!(t.state(), TransportState::Stopped);
    }
}
