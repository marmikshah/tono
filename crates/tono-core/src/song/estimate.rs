//! Resource estimates for a compiled Program (frames, events, peak voices,
//! memory) — bounded numbers the runtime preallocates from.

use super::compile::note_end;
use crate::dsl::{Node, SoundDoc};
use crate::program::ResourceEstimates;

/// The total frame count of a resolved document's render.
pub(crate) fn duration_frames(doc: &SoundDoc) -> u64 {
    (doc.duration * doc.sample_rate as f32).round().max(0.0) as u64
}

/// The (start, end) steps of every note of one compiled track — direct notes
/// plus placements, as rendered. `None` for a track that isn't seq-backed.
pub(crate) fn track_note_spans(doc: &SoundDoc, index: usize) -> Option<Vec<(u32, u32)>> {
    let Node::Tracks { tracks, .. } = &doc.root else {
        return None;
    };
    let node = &tracks.get(index)?.node;
    let seq = match node {
        Node::Seq { notes, .. } => {
            return Some(notes.iter().map(|n| (n.step, note_end(n))).collect());
        }
        Node::Chain { stages } => match stages.first() {
            Some(Node::Seq { notes, .. }) => notes,
            _ => return None,
        },
        _ => return None,
    };
    Some(seq.iter().map(|n| (n.step, note_end(n))).collect())
}

/// How many notes a compiled track plays (for [`TrackMeta`]).
pub(crate) fn track_note_count(doc: &SoundDoc, index: usize) -> u32 {
    track_note_spans(doc, index).map_or(0, |v| v.len() as u32)
}

/// The largest number of notes sounding at once within one track. Steps are
/// half-open intervals [start, end): at a shared position an ending note is
/// gone before the next starts (the sort applies −1 deltas before +1).
pub(crate) fn peak_overlap(mut spans: Vec<(u32, u32)>) -> u32 {
    let mut points: Vec<(u32, i64)> = Vec::with_capacity(spans.len() * 2);
    for (start, end) in spans.drain(..) {
        points.push((start, 1));
        points.push((end, -1));
    }
    points.sort();
    let mut current = 0i64;
    let mut peak = 0i64;
    for (_, delta) in points {
        current += delta;
        peak = peak.max(current);
    }
    peak.max(0) as u32
}

/// Bounded estimates of what the resolved document costs to render or run.
pub(crate) fn program_estimates(doc: &SoundDoc) -> ResourceEstimates {
    let mut events = 0u64;
    let mut peak_voices = 0u32;
    if let Node::Tracks { tracks, .. } = &doc.root {
        for i in 0..tracks.len() {
            if let Some(spans) = track_note_spans(doc, i) {
                events += spans.len() as u64;
                // Tracks all start at the song's head, so their per-track
                // peaks can coincide — summing them is the safe upper bound.
                peak_voices = peak_voices.saturating_add(peak_overlap(spans));
            }
        }
    }
    let frames = duration_frames(doc);
    ResourceEstimates {
        frames,
        events,
        peak_voices,
        memory_bytes: frames.saturating_mul(8),
    }
}

#[cfg(test)]
mod tests {
    use super::peak_overlap;

    #[test]
    fn peak_overlap_treats_ends_as_half_open() {
        assert_eq!(
            peak_overlap(vec![(0, 4), (4, 8)]),
            1,
            "back-to-back, never 2"
        );
        assert_eq!(peak_overlap(vec![(0, 5), (4, 8)]), 2, "one step of overlap");
        assert_eq!(peak_overlap(vec![]), 0);
    }
}
