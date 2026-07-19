//! Horizontal sections: swapping one looping bed for another on a beat/bar
//! boundary, with click-free cross-fades (including mid-fade reversals and
//! deferred third-section switches).

use super::{Action, AdaptiveMusic, LoopBuffer, Scheduled, SectionFade, SoundDoc};
use crate::runtime::AudioSource;

impl AdaptiveMusic {
    // ---- Horizontal sections ----

    /// Add a horizontal section (a looping bed). The first section added starts
    /// playing immediately; switch between them with [`transition_to`](Self::transition_to).
    pub fn add_section(&mut self, name: impl Into<String>, doc: &SoundDoc) -> usize {
        self.add_section_buffer(name, LoopBuffer::from_doc_at(doc, self.sample_rate))
    }

    /// Add a section from a pre-rendered [`LoopBuffer`] — render off the audio
    /// thread and hand the buffer in, so a real-time caller never renders under a
    /// lock (mirrors [`add_layer`](Self::add_layer)).
    pub fn add_section_buffer(&mut self, name: impl Into<String>, buffer: LoopBuffer) -> usize {
        let index = self.sections.len();
        self.sections.push(super::Section {
            name: name.into(),
            buffer,
        });
        if self.current_section.is_none() {
            self.current_section = Some(index);
        }
        index
    }

    /// Cross-fade to another section on a beat/bar boundary — horizontal
    /// re-sequencing (e.g. swap "explore" for "battle" on the next bar). The target
    /// enters from its downbeat. A no-op for an unknown or already-current section.
    pub fn transition_to(&mut self, section: usize, q: super::Quantize) {
        if section >= self.sections.len() {
            return;
        }
        // A new transition supersedes any still-pending one (no stacking of
        // duplicate/contradictory transitions). Requesting the current section
        // just cancels a pending transition.
        self.pending
            .retain(|s| !matches!(s.action, Action::Transition { .. }));
        // While a cross-fade A→B is in flight the effective current section is
        // B: requesting B again is a no-op, and requesting A cancels the fade
        // back — the fade runs in reverse from its current gain, so the
        // reversal is click-free.
        if let Some(f) = &self.section_fade {
            if f.to == section {
                return;
            }
            if self.current_section == Some(section) {
                // Reverse from the gain the next sample would have used —
                // sample-continuous by construction.
                self.section_fade = Some(SectionFade {
                    to: f.to,
                    from_gain: f.from_gain + f.frames_done as f32 * f.step,
                    step: -f.step,
                    frames_done: 0,
                });
                return;
            }
            // A third section: leave the in-flight fade alone — the onward
            // transition queues behind it (see begin_transition), because a
            // hard cut would drop the fade target's partial contribution
            // mid-ramp.
        }
        if self.current_section == Some(section) {
            return;
        }
        self.apply_or_schedule(q, Action::Transition { to: section });
    }

    /// Start the section cross-fade now: rewind the target to its head so it enters
    /// on its downbeat, and ramp it in over the declick window.
    pub(super) fn begin_transition(&mut self, to: usize) {
        // Already there (a duplicate/queued transition): do nothing. Starting a
        // fade to the current section would fill the same buffer twice per block
        // and advance its play head twice — an audible speed-up.
        if self.current_section == Some(to) {
            return;
        }
        if self.current_section.is_none() {
            self.current_section = Some(to);
            return;
        }
        if let Some(f) = &self.section_fade {
            // Already heading there (a duplicated schedule fired): no-op.
            if f.to == to {
                return;
            }
            // A mid-flight fade to ANOTHER section completes first: cutting
            // over now would drop the fade target's partial contribution
            // mid-ramp — the hard-cut click class the reversal avoids. Queue
            // the onward transition for the frame the fade settles (at the
            // target on a forward fade, back at the source on a reversal).
            let g_now = f.from_gain + f.frames_done as f32 * f.step;
            let wait = if f.step > 0.0 {
                (1.0 - g_now) / f.step
            } else {
                g_now / -f.step
            };
            let fire_at = self.position + (wait.ceil().max(0.0) as u64);
            self.pending.push(Scheduled {
                fire_at,
                action: Action::Transition { to },
            });
            return;
        }
        self.sections[to].buffer.reset();
        self.section_fade = Some(SectionFade {
            to,
            from_gain: 0.0,
            step: self.section_step,
            frames_done: 0,
        });
    }

    /// The section currently sounding, if any.
    pub fn current_section(&self) -> Option<usize> {
        self.current_section
    }

    /// Look up a section index by name.
    pub fn section_named(&self, name: &str) -> Option<usize> {
        self.sections.iter().position(|s| s.name == name)
    }
}
