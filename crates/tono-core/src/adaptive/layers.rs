//! Vertical layers: intensity-driven stems, one-shot stingers, and the
//! sidechain duck.

use super::{
    AdaptiveMusic, DUCK_ATTACK_SECS, Layer, LoopBuffer, SoundDoc, Stinger, doc_at,
    render_stereo_pair,
};
use crate::runtime::AudioSource;

impl AdaptiveMusic {
    /// Add a stem that fades to full volume once `intensity >= fade_in_at`
    /// (`0.0` = always on). It plays silently underneath until then.
    /// Returns the layer's index (for [`layer_gain`](Self::layer_gain)).
    pub fn add_layer(
        &mut self,
        source: impl AudioSource + Send + 'static,
        fade_in_at: f32,
    ) -> usize {
        // Clamp like set_intensity does — intensity can never exceed 1, so an
        // unclamped threshold above it would be a layer that never plays.
        let fade_in_at = fade_in_at.clamp(0.0, 1.0);
        let on = self.intensity >= fade_in_at;
        self.layers.push(Layer {
            source: Box::new(source),
            fade_in_at,
            gain: if on { 1.0 } else { 0.0 },
            target: if on { 1.0 } else { 0.0 },
        });
        self.layers.len() - 1
    }

    /// Add a looping stem rendered from `doc` **at the engine's sample rate**
    /// — the doc-taking convenience over [`add_layer`](Self::add_layer).
    pub fn add_layer_doc(&mut self, doc: &SoundDoc, fade_in_at: f32) -> usize {
        self.add_layer(LoopBuffer::from_doc_at(doc, self.sample_rate), fade_in_at)
    }

    /// Add a **phase-locked stem set**: every stem is rendered and forced onto
    /// one shared loop length, so their intensity cross-fades stay sample-aligned
    /// and never drift phase — the guarantee layered adaptive music needs.
    ///
    /// The shared length is `duration_beats` at the current tempo (set
    /// [`set_tempo`](Self::set_tempo) first); with no tempo it falls back to the
    /// first stem's natural rendered length. Each `(doc, fade_in_at)` becomes a
    /// layer, exactly as [`add_layer`](Self::add_layer). Returns the shared grid
    /// length in frames — schedule a beat clock against it.
    ///
    /// Because every stem is trimmed/padded to the same frame count, the set is
    /// **provably sample-identical in length** (author each `doc` to
    /// `duration_beats` so the fit is a trim, not a silent pad).
    pub fn add_stem_set(&mut self, stems: &[(&SoundDoc, f32)], duration_beats: f64) -> usize {
        let tempo_grid = self
            .frames_per_beat()
            .map(|fpb| (duration_beats * fpb).round() as usize)
            .filter(|f| *f > 0);
        let Some(((first_doc, first_fade), rest)) = stems.split_first() else {
            return tempo_grid.unwrap_or(0);
        };
        if let Some(grid) = tempo_grid {
            for (doc, fade_in_at) in stems {
                self.add_layer(
                    LoopBuffer::from_doc_len_at(doc, grid, self.sample_rate),
                    *fade_in_at,
                );
            }
            return grid;
        }
        // No tempo: the first stem's natural length is the grid — render it
        // once and reuse the buffers rather than rendering again for length.
        let (left, right) = render_stereo_pair(&doc_at(first_doc, self.sample_rate));
        let grid = left.len();
        self.add_layer(LoopBuffer::from_stereo(left, right), *first_fade);
        for (doc, fade_in_at) in rest {
            self.add_layer(
                LoopBuffer::from_doc_len_at(doc, grid, self.sample_rate),
                *fade_in_at,
            );
        }
        grid
    }

    /// Set the intensity, 0..1 — stems cross-fade toward their new levels.
    pub fn set_intensity(&mut self, x: f32) {
        self.intensity = x.clamp(0.0, 1.0);
        for l in &mut self.layers {
            l.target = if self.intensity >= l.fade_in_at {
                1.0
            } else {
                0.0
            };
        }
    }

    /// Fire a one-shot stinger over the bed (rendered now, mixed until it ends).
    pub fn stinger(&mut self, doc: &SoundDoc) {
        let (left, right) = render_stereo_pair(&doc_at(doc, self.sample_rate));
        self.stinger_stereo(left, right);
    }

    /// Fire a stinger from pre-rendered stereo — render off the audio thread and
    /// hand the buffers in, so a real-time caller never renders under a lock.
    pub fn stinger_stereo(&mut self, left: Vec<f32>, right: Vec<f32>) {
        self.stingers.push(Stinger::new(left, right));
    }

    /// The current intensity.
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// A layer's current (cross-faded) gain — for a mixer readout.
    pub fn layer_gain(&self, index: usize) -> Option<f32> {
        self.layers.get(index).map(|l| l.gain)
    }

    /// Number of stingers still ringing.
    pub fn active_stingers(&self) -> usize {
        self.stingers.len()
    }

    /// Duck the whole bed now — drop its gain to `1.0 - depth` and recover to
    /// unity over `release`. A fast sidechain for stingers or SFX, independent of
    /// the (slower) intensity cross-fade. `depth` clamps to `0..=1`; a stinger
    /// duck is typically shallow and short (e.g. `0.4`, ~180 ms).
    pub fn duck(&mut self, depth: f32, release: std::time::Duration) {
        // Attack toward the floor rather than stepping to it — an instantaneous
        // gain jump is an audible click. `fill` ramps `duck_gain` down over a few
        // ms, then releases back to unity.
        self.duck_target = (1.0 - depth).clamp(0.0, 1.0);
        let secs = release.as_secs_f32().max(1.0 / self.sample_rate as f32);
        self.duck_coeff = 1.0 - (-1.0 / (secs * self.sample_rate as f32)).exp();
        let attack = DUCK_ATTACK_SECS * self.sample_rate as f32;
        self.duck_attack = 1.0 - (-1.0 / attack).exp();
    }
}
