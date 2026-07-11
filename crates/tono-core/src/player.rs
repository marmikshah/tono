//! Real-time audition: a [`Player`] that serves a document's audio in blocks to
//! an audio callback (a native `cpal` stream, or a browser `AudioWorklet`).
//!
//! This is the host-agnostic seam between the deterministic engine and live
//! playback. The **invariant** that makes manual sound-design safe: the audio a
//! `Player` serves is byte-identical to an offline bounce of the same document
//! (see `player_output_matches_offline_bounce`). So you can audition live, tweak,
//! and still ship the exact thing you heard.
//!
//! The `Player` is buffer-backed — [`Player::set_doc`] re-renders the document
//! (instant for short SFX) and [`Player::fill`] serves it with a play cursor
//! and looping. Its per-sample counterpart is [`crate::streaming`]: the
//! stateful block renderer for the causal subset of the graph; the runtime
//! picks between them and both stay byte-identical to the offline bounce.

use crate::dsl::SoundDoc;
use crate::render;

/// Render a document to a finished stereo pair (mixer bus, or a stereoized mono
/// graph) — the exact audio an offline bounce would write.
pub fn render_stereo(doc: &SoundDoc) -> (Vec<f32>, Vec<f32>) {
    let product = render::render_product(doc);
    match product.stereo {
        Some((l, r)) => (l, r),
        None => render::stereoize(&product.mono, doc.stereo, doc.sample_rate),
    }
}

/// A real-time audition player. Owns a document and serves its rendered audio in
/// interleaved-stereo blocks to an audio callback.
pub struct Player {
    doc: SoundDoc,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Current play head, in frames.
    pos: usize,
    /// Loop back to the start at the end instead of stopping.
    pub looping: bool,
    /// Whether the play head advances on `fill`.
    pub playing: bool,
}

impl Player {
    /// Build a player for `doc`, rendering it immediately. Not yet playing.
    pub fn new(doc: SoundDoc) -> Self {
        let (left, right) = render_stereo(&doc);
        Self {
            doc,
            left,
            right,
            pos: 0,
            looping: true,
            playing: false,
        }
    }

    /// Replace the document and re-render (the cursor is clamped into range, so
    /// a live edit keeps playing from roughly where it was). Byte-identical to a
    /// fresh bounce of the new document.
    pub fn set_doc(&mut self, doc: SoundDoc) {
        let (left, right) = render_stereo(&doc);
        self.pos = self.pos.min(left.len());
        self.doc = doc;
        self.left = left;
        self.right = right;
    }

    /// Length of the rendered audio in frames.
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    /// Current play-head position, in frames.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Move the play head to `frame` (clamped into range). Lets a re-rendered
    /// player resume where the outgoing one left off, for a click-free swap.
    pub fn seek(&mut self, frame: usize) {
        self.pos = frame.min(self.left.len());
    }

    /// Start playing from the current position.
    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Stop and rewind to the start.
    pub fn stop(&mut self) {
        self.playing = false;
        self.pos = 0;
    }

    /// Fill an interleaved-stereo output block (`[L, R, L, R, …]`) and advance
    /// the play head. Frames past the end are written as silence; with
    /// `looping`, the head wraps to the start. When not `playing`, writes
    /// silence without advancing. Returns the number of frames written from the
    /// rendered audio (non-silent).
    pub fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        if !self.playing {
            out.fill(0.0);
            return 0;
        }
        let n = self.left.len();
        let mut served = 0;
        for f in 0..frames {
            if self.pos >= n {
                if self.looping && n > 0 {
                    self.pos = 0;
                } else {
                    out[f * 2] = 0.0;
                    out[f * 2 + 1] = 0.0;
                    // The guard above proves looping-with-content is impossible
                    // here: the sound is over.
                    self.playing = false;
                    continue;
                }
            }
            out[f * 2] = self.left[self.pos];
            out[f * 2 + 1] = self.right[self.pos];
            self.pos += 1;
            served += 1;
        }
        served
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::SoundDoc;

    fn coin() -> SoundDoc {
        serde_json::from_str(
            r#"{ "name":"coin", "duration":0.18, "root": { "type":"mul", "inputs": [
                { "type":"square", "duty":0.5, "freq": { "arp": { "steps":[988,1319], "rate":14 } } },
                { "type":"env", "a":0.0, "d":0.16, "s":0.0, "r":0.0, "punch":0.2 } ] } }"#,
        )
        .expect("valid doc")
    }

    /// The determinism invariant: audio served by the Player, block by block, is
    /// byte-identical to an offline bounce of the same document.
    #[test]
    fn player_output_matches_offline_bounce() {
        let doc = coin();
        let (exp_l, exp_r) = render_stereo(&doc);

        let mut p = Player::new(doc);
        p.looping = false;
        p.play();

        // Serve through odd-sized blocks so block boundaries don't align to the
        // signal — the cursor must still reproduce every sample exactly.
        let mut got_l = Vec::new();
        let mut got_r = Vec::new();
        let mut block = vec![0.0f32; 257 * 2];
        while p.playing && got_l.len() < exp_l.len() {
            let served = p.fill(&mut block);
            for f in 0..served {
                got_l.push(block[f * 2]);
                got_r.push(block[f * 2 + 1]);
            }
        }

        assert_eq!(got_l.len(), exp_l.len(), "served frame count");
        assert_eq!(
            got_l.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            exp_l.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            "left channel byte-identical to offline bounce"
        );
        assert_eq!(
            got_r.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            exp_r.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            "right channel byte-identical to offline bounce"
        );
    }

    #[test]
    fn paused_player_writes_silence_without_advancing() {
        let mut p = Player::new(coin());
        let mut block = vec![1.0f32; 64];
        let served = p.fill(&mut block); // not playing
        assert_eq!(served, 0);
        assert!(block.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn looping_wraps_at_the_end() {
        let mut p = Player::new(coin());
        p.play();
        let n = p.frames();
        let mut block = vec![0.0f32; (n + 50) * 2];
        let served = p.fill(&mut block);
        // Every frame is served because looping wraps past the end.
        assert_eq!(served, n + 50);
    }
}
