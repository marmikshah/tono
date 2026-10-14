//! A frame-stamped action queue shared by the runtime's clocks: the commands
//! of a [`Performance`](super::Performance) and the quantized changes of an
//! [`AdaptiveMusic`](crate::adaptive::AdaptiveMusic). Entries fire in
//! `(frame, submission)` order, so actions on the same frame keep the order
//! they were scheduled in, and a `fill` can split its block at the next due
//! frame to land every action exactly.

use std::collections::VecDeque;

/// An action pinned to a frame with its submission number.
pub(crate) struct Stamped<A> {
    /// The frame the action fires at.
    pub(crate) at: u64,
    /// Submission order — the deterministic tie-break for identical frames.
    pub(crate) seq: u64,
    pub(crate) action: A,
}

/// The queue, kept sorted by `(at, seq)`.
pub(crate) struct Timeline<A> {
    items: VecDeque<Stamped<A>>,
    next_seq: u64,
}

impl<A> Timeline<A> {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Timeline {
            items: VecDeque::with_capacity(cap),
            next_seq: 0,
        }
    }

    /// Queue `action` for frame `at`; returns its submission number.
    pub(crate) fn push(&mut self, at: u64, action: A) -> u64 {
        self.next_seq += 1;
        let seq = self.next_seq;
        let pos = self
            .items
            .iter()
            .position(|e| (e.at, e.seq) > (at, seq))
            .unwrap_or(self.items.len());
        self.items.insert(pos, Stamped { at, seq, action });
        seq
    }

    /// The frame of the earliest queued action.
    pub(crate) fn next_at(&self) -> Option<u64> {
        self.items.front().map(|e| e.at)
    }

    /// Take the earliest action if it is due at `now`.
    pub(crate) fn pop_due(&mut self, now: u64) -> Option<Stamped<A>> {
        if self.items.front()?.at <= now {
            self.items.pop_front()
        } else {
            None
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Stamped<A>> {
        self.items.iter()
    }

    /// The latest-firing queued action.
    #[cfg(test)]
    pub(crate) fn last(&self) -> Option<&Stamped<A>> {
        self.items.back()
    }

    /// Drop every entry `keep` rejects (the order of the rest is unchanged).
    pub(crate) fn retain(&mut self, keep: impl FnMut(&Stamped<A>) -> bool) {
        self.items.retain(keep);
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }
}
