//! Emitter tracks on an emitter's clock: how a particle emitter or a ribbon
//! reads its keyed properties.
//!
//! An emitter has no animation of its own. Its tracks are keyed on the
//! model's shared timeline, and [`ParticleClip`] is the window of that
//! timeline the emitter runs on: sequence 0's, for scenery. [`clip_time`]
//! turns an emitter's age into a time on that window, [`sample`] and [`gate`]
//! read a track there, and [`first`] reads the properties that are sampled
//! once. [`M2Particle::over_life`] is the colour, size and atlas cell of one
//! particle over its life.

use std::sync::Arc;

use super::{M2Particle, M2Track};

/// The emitter clock: sequence 0's window on the model's shared timeline,
/// and the global-sequence durations for the tracks that ignore it.
#[derive(Clone, Debug)]
pub struct ParticleClip {
    pub start: u32,
    pub end: u32,
    /// Wrap the clock onto the window, or hold its end. The 1.12.1 client
    /// loops a sequence when bit 0 of its flags is clear.
    pub loops: bool,
    pub global_sequences: Arc<[u32]>,
}

/// Seconds of emitter age onto the clip's millisecond clock. With no clip the
/// age in milliseconds is the clock.
pub fn clip_time(clip: &Option<ParticleClip>, age: f32) -> u32 {
    let ms = (age * 1000.0) as u32;
    match clip {
        Some(c) => {
            let span = c.end.saturating_sub(c.start).max(1);
            if c.loops {
                ms % span
            } else {
                ms.min(span)
            }
        }
        None => ms,
    }
}

/// A keyed emitter track at the clip clock, or its first key without one.
/// A missing track reads 0.
pub fn sample(track: &Option<M2Track>, clip: &Option<ParticleClip>, clip_ms: u32, now_ms: u32) -> f32 {
    match (track, clip) {
        (None, _) => 0.0,
        (Some(t), Some(c)) => {
            t.sample(c.start + clip_ms, c.start, c.end, &c.global_sequences, now_ms)[0]
        }
        (Some(t), None) => t.first(),
    }
}

/// The emission gate. No keys means always on (the 1.12.1 client's default);
/// with keys but no clip to run them on, on if any key is nonzero.
pub fn gate(track: &Option<M2Track>, clip: &Option<ParticleClip>, clip_ms: u32, now_ms: u32) -> bool {
    match (track, clip) {
        (None, _) => true,
        (Some(t), Some(c)) => {
            t.sample(c.start + clip_ms, c.start, c.end, &c.global_sequences, now_ms)[0] > 0.5
        }
        (Some(t), None) => t.values.iter().any(|&v| v > 0.5),
    }
}

/// The first key of an emitter property track, for the properties that are
/// sampled once, or `fallback` when the track is missing. See
/// [`M2Track::first`].
pub fn first(track: &Option<M2Track>, fallback: f32) -> f32 {
    track.as_ref().map(|t| t.first()).unwrap_or(fallback)
}

impl M2Particle {
    /// The over-life ramp: two linear segments split at `mid_point`, each with
    /// its own atlas-cell range, as the 1.12.1 client evaluates it. Returns the
    /// colour, the size and the atlas cell at `u` of a particle's life.
    pub fn over_life(&self, u: f32) -> ([f32; 4], f32, u16) {
        let u = u.clamp(0.0, 1.0);
        let mid = self.mid_point.clamp(1e-3, 1.0);
        let (a, b, t, cells) = if u <= mid {
            (0, 1, u / mid, self.cells[0])
        } else {
            (1, 2, (u - mid) / (1.0 - mid).max(1e-3), self.cells[1])
        };
        let mut color = [0.0f32; 4];
        for (c, out) in color.iter_mut().enumerate() {
            let (ca, cb) = (f32::from(self.colors[a][c]), f32::from(self.colors[b][c]));
            *out = (ca + (cb - ca) * t) / 255.0;
        }
        let size = self.scales[a] + (self.scales[b] - self.scales[a]) * t;
        // The cell walks begin..=end across its segment:
        // `floor(begin + (end - begin + 1) · t)`, clamped onto the range.
        //
        // The walk can run in either direction. A segment may be authored
        // `[15, 0]`: the second half of `Spells\FarSight_Impact_Base.m2`'s
        // emitter 1 is a flipbook played backwards over the sprite's decay. A walk
        // that assumed `begin <= end` clamped onto an empty range and panicked on
        // the first frame Eagle Eye's focus drew (`min > max. min = 15, max = 0`).
        // The step is signed and the clamp is onto the sorted pair.
        let (begin, end) = (i32::from(cells[0]), i32::from(cells[1]));
        let span = ((end - begin).abs() + 1) as f32;
        let step = (span * t).floor() as i32;
        let idx = if end >= begin { begin + step } else { begin - step };
        let cell = idx.clamp(begin.min(end), begin.max(end)) as u16;
        (color, size, cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(start: u32, end: u32, loops: bool) -> Option<ParticleClip> {
        Some(ParticleClip {
            start,
            end,
            loops,
            global_sequences: Arc::from(&[][..]),
        })
    }

    fn plain_def() -> M2Particle {
        M2Particle {
            blend: 3,
            emitter_type: 1,
            rows: 1,
            columns: 1,
            mid_point: 0.5,
            colors: [[255, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 0]],
            scales: [1.0, 2.0, 4.0],
            cells: [[0, 1], [2, 3]],
            ..Default::default()
        }
    }

    /// A one-shot clip clamps rather than wraps, so a slash finishes: a trail
    /// whose height returns to zero at 267 ms must stay at zero, not restart
    /// every 1.1 seconds.
    #[test]
    fn a_one_shot_clip_clamps_and_a_looping_one_wraps() {
        assert_eq!(clip_time(&clip(3300, 4433, false), 2.0), 1133);
        assert_eq!(clip_time(&clip(3300, 4433, false), 0.5), 500);
        assert_eq!(clip_time(&clip(0, 1000, true), 2.5), 500);
    }

    /// The over-life ramp is two segments split at the midpoint, and each
    /// half has its own atlas-cell range. A single 0..1 lerp (what the
    /// reference viewer did) gets both wrong.
    #[test]
    fn the_over_life_ramp_breaks_at_the_midpoint() {
        let def = plain_def();
        // At birth: the first key exactly.
        let (color, size, cell) = def.over_life(0.0);
        assert_eq!(color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(size, 1.0);
        assert_eq!(cell, 0);
        // At the midpoint: the middle key, and the first segment's last cell.
        let (color, size, cell) = def.over_life(0.5);
        assert_eq!(color, [0.0, 1.0, 0.0, 128.0 / 255.0]);
        assert_eq!(size, 2.0);
        assert_eq!(cell, 1);
        // Past it: the second segment interpolates toward the death key over
        // its own cells.
        let (color, size, cell) = def.over_life(0.75);
        assert_eq!(size, 3.0);
        assert!(color[2] > 0.4 && color[3] < 0.3);
        assert_eq!(cell, 3);
        // At death: clamped, not wrapped.
        let (_, size, cell) = def.over_life(1.0);
        assert_eq!(size, 4.0);
        assert_eq!(cell, 3);
    }

    /// A cell range may run backwards, and the walk follows it rather than
    /// panicking on an empty clamp. `Spells\FarSight_Impact_Base.m2`'s emitter
    /// 1 is `[[0, 15], [15, 0]]`: a flipbook forward over the first half and
    /// back over the second. Before the fix, the first frame Eagle Eye's focus
    /// drew panicked with `min > max. min = 15, max = 0` in the merge pass.
    #[test]
    fn a_reversed_cell_range_walks_backwards() {
        let mut def = plain_def();
        def.cells = [[0, 15], [15, 0]];
        let (_, _, at_birth) = def.over_life(0.0);
        let (_, _, at_mid) = def.over_life(0.5);
        let (_, _, past_mid) = def.over_life(0.75);
        let (_, _, at_death) = def.over_life(1.0);
        assert_eq!(at_birth, 0);
        assert_eq!(at_mid, 15, "the first half ends at its last cell");
        assert!(past_mid < 15 && past_mid > 0, "…and the second walks down: {past_mid}");
        assert_eq!(at_death, 0, "clamped onto the sorted pair, not wrapped");
    }

    /// No keys is always on; keys with no clip are on when any key is.
    #[test]
    fn a_track_with_no_keys_is_open_and_one_with_no_clip_reads_its_keys() {
        assert!(gate(&None, &None, 0, 0));
        let off = Some(M2Track {
            values: vec![0.0, 0.0],
            times: vec![0, 100],
            dim: 1,
            global_sequence: -1,
            ..Default::default()
        });
        assert!(!gate(&off, &None, 0, 0));
        assert_eq!(sample(&None, &None, 0, 0), 0.0);
        assert_eq!(first(&None, 2.5), 2.5);
    }
}
