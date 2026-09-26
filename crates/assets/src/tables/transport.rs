//! **What a moving platform does with no packet to tell it** —
//! `TransportAnimation.dbc`.
//!
//! An elevator is the one thing in the world that moves and is never *told* to.
//! A creature gets `SMSG_MONSTER_MOVE`, a player gets `MSG_MOVE_*`, a spell gets
//! its own packet; the Thunder Bluff lift, the Undercity elevator and the
//! Deeprun Tram get **nothing at all**, because server and client derive their
//! position from the same shipped table and the same clock. Read no table and
//! the platform stands at its spawn point for ever, which is the report this
//! module exists for: *"you fall straight through one that is not at the bottom
//! of its shaft."*
//!
//! ```text
//! TransportAnimation.dbc   630 records x 7 fields   [0] id
//!                                                   [1] gameobject ENTRY
//!                                                   [2] time, ms into the cycle
//!                                                   [3..5] x, y, z offset
//!                                                   [6] AnimationData sequence
//! ```
//!
//! 33 entries carry rows. Four a 1.12 player actually meets:
//!
//! ```text
//! 4170   Mesa Elevator    13 nodes   61.2 y down, 5 s dwell each end, 30 s round
//! 20649  Undervator        6 nodes   55.5 y down and 41.0 y up, 16.7 s round
//! 80023  Vator                       …Gnomeregan
//! 176080 Subway           53 nodes   1,000+ y along x — the Deeprun Tram
//! ```
//!
//! **The offsets are in the object's own frame and mostly one axis.** Every
//! elevator in the game moves in `z` alone — `x` and `y` read `±0.000` for all
//! of 4170, 4171, 11898, 20649 and their siblings — and the tram is the
//! population that moves in `x`. That bounds how much the *caller's* frame
//! conversion can go wrong on the reported subject: a lift's offset is
//! unaffected by any rotation about the vertical.
//!
//! **Ironforge's two lifts are not in this table at all** (entries 32056 and
//! 32057 in the world database, no rows here), so nothing here moves them. That
//! is a finished answer rather than a gap: the file does not describe them.
//!
//! ## Where in the cycle — and this is the half that is not guessable
//!
//! The phase is **not** the client's own clock modulo the cycle, which is what
//! this repo's own bug list assumed for four rounds. It is the
//! `UPDATEFLAG_TRANSPORT` word: `Object::BuildMovementUpdate` appends
//! `uint32 GetPathProgress()` to the movement block of any game object carrying
//! that flag, and `ElevatorTransport::Update` holds `m_pathProgress` at
//! `GetTimeSinceCreation() % TotalTime`. So the server states where in the cycle
//! the platform is at the moment it becomes visible to you, and the client
//! carries that forward on its own clock — which is how two clients that
//! started hours apart agree.
//!
//! **The reference does exactly that.** It stores `pathProgress - now` on the
//! game object, guarded on `updateFlags & 2`, and the per-frame interpolation
//! opens with
//!
//! ```text
//! phase = (stored progress - creation + now) % last node's TimeSeg
//! ```
//!
//! so the divisor is
//! `node[count-1].TimeSeg` — **the cycle length is the last node's own time**,
//! not a column of the file. That is [`Transports::total_time`].
//!
//! Below it, the bracketing pair's positions are the two ends of a
//! linear leg, and the previous node's sequence is compared against a
//! field the object was initialised to **208** with — the sequence id, this
//! module's [`Node::sequence`].
//!
//! ## What this module is not
//!
//! It is the rule and nothing else: an entry and a phase in, an offset out.
//! Turning that into a world position wants the object's stationary position and
//! its rotation, which are the client's — see
//! `crates/client/src/world/entities/transport.rs`, the only consumer, and
//! `vale objects elevator`, which is the check.

use super::dbc::Dbc;
use std::collections::HashMap;

/// One keyframe: where the platform is, this many milliseconds into the cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    /// Milliseconds from the start of the cycle. The rows are held **sorted** by
    /// it because the file is not: `AddPathNodeToTransport` keys a `std::map` by
    /// exactly this, and the reference indexes an array in the same order.
    pub time_ms: u32,
    /// The offset from the object's **stationary** position, in the object's own
    /// frame and in the file's axes.
    pub offset: [f32; 3],
    /// The `AnimationData` sequence the platform plays over this leg — 162 and
    /// 164 for the elevators, which is the pair that reads as *moving* against
    /// *stopped*.
    ///
    /// **Read and carried, played by nothing.** The reference switches on it;
    /// this client draws every elevator model in its first frame,
    /// and the one the report names has no skeleton at all — `vale anim`
    /// answers "nothing to animate" for `THUNDERBLUFFELEVATOR\ELEVATORCAR.m2`.
    /// Kept because it costs a word, and because whoever wants the door
    /// animation will want it here rather than in a second pass over the file.
    pub sequence: u32,
}

/// Every animated transport in the game, by game-object **entry**.
#[derive(Debug, Clone, Default)]
pub struct Transports {
    by_entry: HashMap<u32, Vec<Node>>,
}

impl Transports {
    /// Parse `TransportAnimation.dbc`.
    ///
    /// An absent or damaged file is an **empty table**, which is exactly the
    /// behaviour this client had before it read one: every platform stands at
    /// its spawn point. A stated degradation rather than an error, on the same
    /// terms as every other optional table in [`super::dbc`].
    pub fn parse(raw: &[u8]) -> Transports {
        let Ok(dbc) = Dbc::parse(raw) else {
            return Transports::default();
        };
        let mut by_entry: HashMap<u32, Vec<Node>> = HashMap::new();
        for record in 0..dbc.record_count {
            let (Some(entry), Some(time_ms)) = (dbc.u32_at(record, 1), dbc.u32_at(record, 2))
            else {
                continue;
            };
            // A row naming no transport is dropped rather than filed under 0:
            // the lookup is by entry, and entry 0 is no game object.
            if entry == 0 {
                continue;
            }
            by_entry.entry(entry).or_default().push(Node {
                time_ms,
                offset: [
                    dbc.f32_at(record, 3).unwrap_or(0.0),
                    dbc.f32_at(record, 4).unwrap_or(0.0),
                    dbc.f32_at(record, 5).unwrap_or(0.0),
                ],
                sequence: dbc.u32_at(record, 6).unwrap_or(0),
            });
        }
        for nodes in by_entry.values_mut() {
            nodes.sort_by_key(|node| node.time_ms);
            // Two rows at the same millisecond are a zero-length leg. The
            // shipped file has none; the guard costs one `dedup` and what it
            // avoids is a division by zero in [`Self::offset_at`].
            nodes.dedup_by_key(|node| node.time_ms);
        }
        // **One node is not an animation**, and neither is none: the cycle
        // length is the last node's time, so a single row would give a platform
        // that is permanently at phase 0 with a divisor of its own time. Both
        // read here as "this entry does not animate", which is the same answer
        // the 1,600 game objects with no row at all get.
        by_entry.retain(|_, nodes| nodes.len() >= 2);
        Transports { by_entry }
    }

    /// How many entries carry an animation — **33** in 1.12.
    pub fn len(&self) -> usize {
        self.by_entry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_entry.is_empty()
    }

    /// Every entry that animates, ascending. For `vale objects`.
    pub fn entries(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.by_entry.keys().copied().collect();
        out.sort_unstable();
        out
    }

    /// This entry's keyframes in time order, or `None` for one that does not
    /// animate — which is nearly every game object, and both Ironforge lifts.
    pub fn nodes(&self, entry: u32) -> Option<&[Node]> {
        self.by_entry.get(&entry).map(Vec::as_slice)
    }

    /// **How long one round trip takes** — the *last node's own time*, which is
    /// the reference's divisor and not a column of the file.
    pub fn total_time(&self, entry: u32) -> Option<u32> {
        self.nodes(entry)
            .and_then(|nodes| nodes.last())
            .map(|node| node.time_ms)
    }

    /// **Where the platform is**, `phase_ms` into its cycle.
    ///
    /// The wrap is taken here rather than by the caller, so a caller may hand
    /// over the raw `path_progress + elapsed` sum and cannot get it wrong.
    /// `None` for an entry that does not animate.
    ///
    /// Linear between the two bracketing nodes, which is what both ends do:
    /// `ElevatorTransport::Update` computes a per-axis velocity over the leg,
    /// and the client divides the same two `TimeSeg`s. **A platform
    /// waiting at a floor is two nodes with equal offsets**, so a dwell falls
    /// out of the same arithmetic and is not a special case — the Mesa
    /// Elevator's five seconds at the top are the gap between its first two
    /// rows.
    pub fn offset_at(&self, entry: u32, phase_ms: u64) -> Option<[f32; 3]> {
        let nodes = self.nodes(entry)?;
        let total = u64::from(nodes.last()?.time_ms);
        if total == 0 {
            return Some(nodes[0].offset);
        }
        let phase = u32::try_from(phase_ms % total).unwrap_or(0);
        let (prev, next) = self.bracket(nodes, phase);
        let span = next.time_ms.saturating_sub(prev.time_ms);
        if span == 0 {
            return Some(prev.offset);
        }
        let along = (phase - prev.time_ms) as f32 / span as f32;
        let lerp = |a: f32, b: f32| a + (b - a) * along;
        Some([
            lerp(prev.offset[0], next.offset[0]),
            lerp(prev.offset[1], next.offset[1]),
            lerp(prev.offset[2], next.offset[2]),
        ])
    }

    /// The sequence the platform is playing at `phase_ms` — the **previous**
    /// node's, which is what the client reads. Carried on the same terms as
    /// [`Node::sequence`]: nothing in this client plays it yet.
    pub fn sequence_at(&self, entry: u32, phase_ms: u64) -> Option<u32> {
        let nodes = self.nodes(entry)?;
        let total = u64::from(nodes.last()?.time_ms);
        if total == 0 {
            return Some(nodes[0].sequence);
        }
        let phase = u32::try_from(phase_ms % total).unwrap_or(0);
        Some(self.bracket(nodes, phase).0.sequence)
    }

    /// The last node at or before `phase` and the first after it.
    ///
    /// `phase` is already inside the cycle, and the cycle's length *is* the last
    /// node's time, so `phase < nodes.last().time_ms` always holds and the
    /// search always finds a `next`. The fallback exists so a caller that
    /// somehow passes a phase past the end gets the final leg rather than a
    /// panic — `nodes` is never shorter than two, which [`Self::parse`]
    /// guarantees.
    fn bracket(&self, nodes: &[Node], phase: u32) -> (Node, Node) {
        match nodes.iter().position(|node| node.time_ms > phase) {
            Some(0) => (nodes[0], nodes[0]),
            Some(next) => (nodes[next - 1], nodes[next]),
            None => (nodes[nodes.len() - 1], nodes[nodes.len() - 1]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `TransportAnimation.dbc` in memory, so the rules can be checked
    /// with no archive.
    fn dbc(rows: &[(u32, u32, [f32; 3], u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&7u32.to_le_bytes());
        out.extend_from_slice(&28u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for (i, (entry, time, offset, sequence)) in rows.iter().enumerate() {
            out.extend_from_slice(&(i as u32).to_le_bytes());
            out.extend_from_slice(&entry.to_le_bytes());
            out.extend_from_slice(&time.to_le_bytes());
            for axis in offset {
                out.extend_from_slice(&axis.to_le_bytes());
            }
            out.extend_from_slice(&sequence.to_le_bytes());
        }
        out.push(0);
        out
    }

    /// A lift: down over ten seconds, five at the bottom, back up, five at the
    /// top — written **out of order**, which is how the shipped file is.
    fn lift() -> Transports {
        Transports::parse(&dbc(&[
            (7, 20_000, [0.0, 0.0, -60.0], 162),
            (7, 0, [0.0, 0.0, 0.0], 164),
            (7, 30_000, [0.0, 0.0, 0.0], 164),
            (7, 10_000, [0.0, 0.0, -60.0], 162),
        ]))
    }

    /// **The cycle is the last node's own time, and the rows are sorted on the
    /// way in.**
    ///
    /// Both halves are load-bearing and neither is visible from the file: a
    /// reader that trusted the file's order would bracket a phase against
    /// whichever rows happened to be adjacent, and one that looked for a
    /// duration column would not find one.
    #[test]
    fn the_cycle_is_the_last_nodes_own_time() {
        let lift = lift();
        assert_eq!(lift.total_time(7), Some(30_000));
        assert_eq!(lift.len(), 1);
        assert_eq!(lift.entries(), vec![7]);
        let times: Vec<u32> = lift.nodes(7).unwrap().iter().map(|n| n.time_ms).collect();
        assert_eq!(times, vec![0, 10_000, 20_000, 30_000]);
    }

    /// The four positions a rider cares about: both ends, both middles — and
    /// **the dwell, which is two nodes with the same offset and no special
    /// case**.
    #[test]
    fn a_lift_is_at_both_ends_and_halfway_between() {
        let lift = lift();
        let z = |phase| lift.offset_at(7, phase).expect("entry 7 animates")[2];
        assert_eq!(z(0), 0.0, "at the top");
        assert_eq!(z(5_000), -30.0, "halfway down");
        assert_eq!(z(10_000), -60.0, "at the bottom");
        assert_eq!(z(15_000), -60.0, "…and still there five seconds later");
        assert_eq!(z(25_000), -30.0, "halfway back up");
        // The wrap is taken here rather than by the caller, so a raw
        // `progress + elapsed` sum is a legal argument.
        assert_eq!(z(30_000), 0.0, "a whole cycle later is the top again");
        assert_eq!(z(35_000), -30.0);
        // …and a sum well past `u32::MAX`, which a session that has been up
        // for a while produces: `path_progress + elapsed` is milliseconds and
        // is never taken modulo anything by the caller.
        assert_eq!(z(30_000 * 200_000 + 5_000), -30.0, "and no overflow");
    }

    /// The sequence is the leg's, not the next node's — the client reads the
    /// *previous* one.
    #[test]
    fn the_sequence_is_the_leg_being_travelled() {
        let lift = lift();
        assert_eq!(lift.sequence_at(7, 0), Some(164));
        assert_eq!(lift.sequence_at(7, 5_000), Some(164), "the descent's own");
        assert_eq!(lift.sequence_at(7, 12_000), Some(162), "the wait at the bottom");
    }

    /// **An entry with no animation answers `None` rather than zero**, which is
    /// what leaves a door, a chest and both Ironforge lifts exactly where the
    /// server put them.
    #[test]
    fn an_entry_with_no_rows_does_not_animate() {
        let lift = lift();
        assert_eq!(lift.offset_at(999, 0), None);
        assert_eq!(lift.total_time(999), None);
        assert_eq!(lift.sequence_at(999, 0), None);
    }

    /// A single row is not an animation, and neither is a missing file: both
    /// leave the platform where it was spawned, which is this client's
    /// behaviour before the table existed.
    #[test]
    fn one_node_and_no_file_both_read_as_not_animating() {
        let one = Transports::parse(&dbc(&[(7, 0, [0.0, 0.0, 0.0], 0)]));
        assert!(one.is_empty());
        assert_eq!(one.offset_at(7, 0), None);
        assert!(Transports::parse(&[]).is_empty());
        assert!(Transports::parse(b"not a dbc at all").is_empty());
    }
}
