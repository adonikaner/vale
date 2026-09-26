//! **What a piece of scenery is doing** — which of its clips this placement is
//! playing now, and what it picks when that one ends.
//!
//! Most of the world does not move: a tree is a tree. But 17 of a Darkshire
//! tile's 128 doodad models carry a skeleton and 38 of the 300 in Stormwind's
//! `MODD` sets do — bellows, gryphon roosts, a windmill, the Great Forge's
//! wheels and pistons, and the torches whose flame is a bone. `vale models
//! <map> <x> <y>` and `vale wmos …` report both counts.
//!
//! ## Scenery plays `Stand`, and the variety is in the variations
//!
//! A creature has a gait, a swing, a flinch and a death. Scenery has one
//! animation id — `AnimationData` **0**, `Stand` — and every one of these models
//! names it. What it does *not* have is one clip: a model carries several rows
//! of that id and the client picks between them each time the last one ends.
//!
//! **`GRYPHONROOST01.m2` is the worked example, and it is measured rather than
//! assumed.** `vale anim` on it:
//!
//! ```text
//! [0] Stand   667..11333  p19663   z 0..2.88   the bird sitting
//! [1] Stand 13333..18000  p3276    z 0..2.87
//! [2] Stand 18667..25333  p3276    z 0..4.44   wings up
//! [3] Stand 26667..30333  p3276    z 0..4.53
//! [4] Stand 31667..32667  p3276    z 0..2.87
//! ```
//!
//! `19663 + 4 x 3276 = 32767 = 0x7FFF` — **the weights sum to exactly the full
//! range**, which is what confirms `probability` sits at `+0x14` of the 68-byte
//! record and means what it appears to. So a roost sits still three fifths of
//! the time and takes one of four flourishes for the rest.
//!
//! Picking uniformly instead plays a flourish four times in five. That was the
//! first version of this module and it reads as a bird with a twitch.
//!
//! ## Two placements of one model must not agree
//!
//! Ten roosts in the Great Forge playing one clip from one frame is the thing
//! the reference most obviously does not do. Each placement rolls its own
//! sequence and starts part-way into it, from a seed stable for the life of the
//! world — a doodad streams in and out as the player walks past, and a fresh
//! draw per spawn would make one visibly restart.
//!
//! **The seed cannot be the `MDDF`/`MODD` id alone, and that was the bug.** A
//! `MODD` spawn carries **its building's** id rather than one of its own — which
//! is why `SolidId::spawn` exists — so every gryphon roost in Ironforge hashed
//! to one number and they moved in unison. [`seed`] mixes in the placement's own
//! position, which is the one thing genuinely per spawn.
//!
//! ## …and a global sequence is not phased
//!
//! One runs on the world's own clock by definition — `M2Skeleton::pose` takes it
//! as `now_ms`, separately from the clip's elapsed time — and a torch flame is
//! one. Offsetting it would invent a per-instance clock the file does not have.

use crate::world::m2::M2Skeleton;

/// `AnimationData.dbc` id 0 — `Stand`. The only animation scenery plays.
pub const STAND: u16 = 0;

/// One clip of a piece of scenery, chosen and running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clip {
    /// Index into `M2Skeleton::sequences` — a *row*, not an animation id, since
    /// every row here shares id 0.
    pub sequence: usize,
    /// How long it runs, in milliseconds. Never zero: a row with no length is
    /// not a clip and [`pick`] does not return one.
    pub duration_ms: u32,
}

/// **Does this model move at all?**
///
/// False for the great majority of the world, for three different reasons none
/// of which is a failure: no skeleton, no sequences, or no `Stand`. The caller
/// draws the bind pose, which is what this client did before any of this.
pub fn animates(skeleton: &M2Skeleton) -> bool {
    skeleton.sequences.iter().any(|s| s.id == STAND && s.end > s.start)
}

/// **Which clip to play**, weighted by each row's own `probability`.
///
/// `roll` is any word; the caller advances it per pick with [`next_roll`], so a
/// placement's successive choices differ and two placements diverge from the
/// first. See the module note for the weights and why uniform is wrong.
///
/// Rows of zero length are skipped: a few models declare a sequence they have no
/// keys for, and one would end on the frame it started — a placement re-rolling
/// sixty times a second.
///
/// **All-zero weights fall back to uniform.** Nothing shipped does, but a zero
/// total would divide by zero, and an evenly-mixed model beats a still one.
pub fn pick(skeleton: &M2Skeleton, roll: u32) -> Option<Clip> {
    let rows: Vec<(usize, u32, u32)> = skeleton
        .sequences
        .iter()
        .enumerate()
        .filter(|(_, s)| s.id == STAND && s.end > s.start)
        .map(|(i, s)| (i, s.end - s.start, u32::from(s.probability)))
        .collect();
    let &(first, first_len, _) = rows.first()?;
    let total: u32 = rows.iter().map(|(_, _, p)| *p).sum();
    if total == 0 {
        let (sequence, duration_ms, _) = rows[(roll as usize) % rows.len()];
        return Some(Clip { sequence, duration_ms });
    }
    let mut want = roll % total;
    for (sequence, duration_ms, probability) in rows {
        if want < probability {
            return Some(Clip { sequence, duration_ms });
        }
        want -= probability;
    }
    // Unreachable while `total` is the sum; answered rather than panicked.
    Some(Clip { sequence: first, duration_ms: first_len })
}

/// **A seed stable for the life of the world and different per placement.**
///
/// `unique_id` is the `MDDF`/`MODD` id and is not enough alone: a `MODD` spawn
/// carries its building's id, so all of a building's roosts share it. The
/// position is what distinguishes them, quantised to a tenth of a yard so a
/// float that round-trips differently does not reseed the placement.
pub fn seed(unique_id: u32, position: [f32; 3]) -> u32 {
    let mut h = u64::from(unique_id);
    for axis in position {
        let q = (axis * 10.0) as i64 as u64;
        h = mix64(h ^ q);
    }
    (mix64(h) & 0xFFFF_FFFF) as u32
}

/// Advance a roll — the same mixer, so a placement's successive picks are as
/// unrelated as two placements' first ones.
pub fn next_roll(roll: u32) -> u32 {
    (mix64(u64::from(roll)) & 0xFFFF_FFFF) as u32
}

/// splitmix64's finaliser.
///
/// The requirement is that **adjacent inputs land far apart**, which a plain
/// modulus does not give: `MDDF` ids are handed out in placement order, so ten
/// torches along a wall have ten consecutive ids and any weak hash puts them in
/// lockstep — the failure this file exists to avoid. The same lesson
/// `render::lamps`' flicker learned, and the same fix.
fn mix64(seed: u64) -> u64 {
    let mut h = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::m2::M2Sequence;

    fn row(start: u32, end: u32, probability: u16) -> M2Sequence {
        M2Sequence {
            id: STAND,
            variation: 0,
            start,
            end,
            move_speed: 0.0,
            flags: 0,
            probability,
            bounds: [[0.0; 3]; 2],
            radius: 0.0,
        }
    }

    /// The gryphon roost's own five rows, as `vale anim` reads them.
    fn roost() -> M2Skeleton {
        M2Skeleton::new(
            Vec::new(),
            vec![
                row(667, 11333, 19663),
                row(13333, 18000, 3276),
                row(18667, 25333, 3276),
                row(26667, 30333, 3276),
                row(31667, 32667, 3276),
            ],
            Vec::new(),
        )
    }

    /// **Scenery with no `Stand` does not move**, and a row with no length is
    /// not a clip: it would end on the frame it started and re-roll for ever.
    #[test]
    fn only_a_stand_row_of_real_length_counts_as_moving() {
        assert!(animates(&roost()));
        assert!(!animates(&M2Skeleton::new(Vec::new(), Vec::new(), Vec::new())));
        let empty = M2Skeleton::new(Vec::new(), vec![row(500, 500, 0x7fff)], Vec::new());
        assert!(!animates(&empty));
        assert_eq!(pick(&empty, 7), None);
    }

    /// **The roost sits still most of the time**, which is the difference
    /// between the reference's courtyard and a chorus line.
    ///
    /// Its weights are `19663 + 4 x 3276 = 32767`, so the bird sitting should
    /// come up about three fifths of the time and each flourish about a tenth.
    /// Uniform gives the idle a fifth — a flourish four times in five.
    #[test]
    fn the_idle_variation_wins_three_fifths_of_the_time() {
        let s = roost();
        let mut counts = [0usize; 5];
        let mut roll = seed(1, [0.0; 3]);
        for _ in 0..20_000 {
            counts[pick(&s, roll).expect("a clip").sequence] += 1;
            roll = next_roll(roll);
        }
        let idle = counts[0] as f64 / 20_000.0;
        assert!(
            (0.55..0.65).contains(&idle),
            "the idle came up {idle:.3}, the weights say 0.600: {counts:?}"
        );
        for flourish in &counts[1..] {
            let share = *flourish as f64 / 20_000.0;
            assert!(
                (0.07..0.13).contains(&share),
                "a flourish came up {share:.3}, the weights say 0.100: {counts:?}"
            );
        }
    }

    /// **Spawns of one building diverge**, which is the report: ten roosts in
    /// the Great Forge moving as one. The id alone cannot do it — every `MODD`
    /// spawn of a building carries the building's — so the position is in the
    /// seed.
    #[test]
    fn spawns_of_one_building_diverge_by_position() {
        let seeds: Vec<u32> = (0..10)
            .map(|n| seed(4242, [n as f32 * 3.5, 12.0, -7.25]))
            .collect();
        let distinct: std::collections::BTreeSet<u32> = seeds.iter().copied().collect();
        assert_eq!(distinct.len(), 10, "spawns shared a seed: {seeds:?}");

        let s = roost();
        let opened: std::collections::BTreeSet<usize> =
            seeds.iter().map(|r| pick(&s, *r).expect("a clip").sequence).collect();
        assert!(opened.len() >= 2, "ten spawns all opened on {opened:?}");

        // Stable, which stops a placement restarting each time it streams back.
        assert_eq!(seed(4242, [3.5, 12.0, -7.25]), seed(4242, [3.5, 12.0, -7.25]));
    }

    /// A one-row model always plays that row — a torch loops its flame — and
    /// still reports a real duration to run it on.
    #[test]
    fn a_one_row_model_keeps_playing_its_one_clip() {
        let s = M2Skeleton::new(Vec::new(), vec![row(0, 3333, 0x7fff)], Vec::new());
        for roll in [0u32, 1, 99, u32::MAX] {
            let clip = pick(&s, roll).expect("a clip");
            assert_eq!(clip.sequence, 0);
            assert_eq!(clip.duration_ms, 3333);
        }
    }
}
