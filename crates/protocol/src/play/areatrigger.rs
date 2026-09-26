//! **Instance portals, and everything else you enter by walking into it.**
//!
//! `CMSG_AREATRIGGER` is four bytes and it is the only thing this client has
//! ever had to *volunteer*. Every other packet here answers something — a
//! keypress, a click, a packet the server sent first — but nothing announces a
//! dungeon entrance: the swirl at the Deadmines is a sphere in a shipped table,
//! the server has no idea you are standing in it, and until the client says so
//! nothing at all happens. There is no missing reply to look for and no opcode
//! going unhandled, which is exactly why walking into a portal read as "the
//! portal is broken" rather than as a gap.
//!
//! It is not only instances. One id reaches
//! `WorldSession::HandleAreaTriggerOpcode` and the *server* decides what it
//! means: a teleport (121 rows in the reference database, of which the dungeon
//! entrances are one bucket), a tavern's rested state, a quest's "area
//! explored" objective, a battleground's flag capture, a scripted event. So the
//! client's half is the same four bytes for all of it, and the whole subject
//! divides cleanly: **which volumes exist and whether you are in one is
//! `vale_assets::tables::areatrigger`** — it is a shipped table and a geometry test,
//! decidable with no session and no window — and **when to ask and what to send
//! is here**.
//!
//! ## The clock and the latch
//!
//! Both are the client's, and both are described in the assets module. In
//! short: a **100 ms** tick, and an **occupancy latch** that
//! re-tests the row you last fired *before* scanning anything, so a trigger
//! sends once on entry and nothing more until you leave it. [`TriggerWatch`] is
//! that state machine and [`CHECK_INTERVAL`] is the period.
//!
//! The period matters rather than being a detail. The obvious implementation —
//! test every session tick — is [`crate::socket::session::TICK`], 25 ms, four times the
//! reference's rate; and the renderer's frame is faster still. That is not a
//! cosmetic difference: the server re-runs the containment test on receipt with
//! a **5-yard** tolerance (`IsPointInAreaTriggerZone(…, 5.0f)`) and drops the
//! packet if it fails, so the position the client had when it decided is the
//! thing being validated. Sending from the first frame of contact rather than
//! the first tick is safe here, but it is a difference from the reference that
//! is free to avoid.
//!
//! ## The refusal, which is the other half of "the portal did nothing"
//!
//! A trigger the server accepts and then declines to act on answers
//! `SMSG_TRANSFER_ABORTED` — one byte in 1.12, the reason and nothing else (no
//! map id; that is a later build's shape). Without reading it, an instance that
//! is full, or one you have zoned into too many times in an hour, is
//! **indistinguishable from the bug this module fixes**: you walk in, nothing
//! happens, and no packet is unhandled.
//!
//! Four of the five reasons are a `GlobalStrings.lua` key and the fifth is
//! deliberately silent — see [`TransferAbort`].

use crate::bytes::{Reader, Writer};
use std::time::Duration;

/// How often the check runs — the period at which the client re-arms its own
/// `"AreaTriggerCheck"` timer.
pub const CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// `CMSG_AREATRIGGER`: the id of the trigger just entered, and nothing else.
///
/// The client writes exactly this — the opcode word, then the first field of
/// the record it matched — and `HandleAreaTriggerOpcode` reads exactly one
/// `uint32`.
pub fn area_trigger_body(trigger_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(trigger_id);
    w.buf
}

/// The volumes, supplied by the caller.
///
/// A trait for the same reason [`crate::state::movement::Footing`] is one: the protocol
/// crate deliberately knows nothing about DBC files, and
/// `vale_assets::tables::areatrigger::AreaTriggers` implements both methods
/// directly.
///
/// **Two methods rather than one**, and the second is not a convenience. The
/// latch re-tests *one specific row* before any scan happens, and a caller given
/// only [`Self::containing`] would have to approximate that with "is the first
/// row here still the one I remember" — which answers wrongly the moment two
/// triggers overlap, and re-fires a portal you are standing in.
pub trait Triggers: Send {
    /// The first trigger on `map` whose volume contains `point`, in the table's
    /// own order. See the assets module for why order rather than distance.
    fn containing(&self, map: u32, point: [f32; 3]) -> Option<u32>;

    /// Whether that specific trigger still contains `point`.
    fn holds(&self, id: u32, map: u32, point: [f32; 3]) -> bool;
}

/// The caller's [`Triggers`], if it has any.
pub type TriggerTable = Box<dyn Triggers>;

/// **The client's own send-once-on-entry rule**: which trigger the character is
/// currently standing in, and therefore which one may not fire again.
///
/// Deliberately holds an id rather than a position or a bool. The client holds
/// the matched record and the tick's first act is to re-test *that row*; a
/// client that instead remembered "I have sent something recently" would refuse
/// a second, genuinely different trigger overlapping the first, and one that
/// remembered a position would re-fire the moment it moved a yard inside the
/// same sphere.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TriggerWatch {
    /// The row last reported, held until the character leaves it.
    occupied: Option<u32>,
    /// The map the latch is about. A far teleport clears it — see [`Self::poll`].
    map: Option<u32>,
}

impl TriggerWatch {
    pub fn new() -> TriggerWatch {
        TriggerWatch::default()
    }

    /// The trigger the character is being held out of, if any.
    pub fn occupied(&self) -> Option<u32> {
        self.occupied
    }

    /// Ask once. Returns the trigger to report, or `None` — which covers all
    /// three of "nothing here", "still inside the one already reported", and
    /// "the caller has no table".
    ///
    /// **The map change clears the latch** (the client rebinds the range and
    /// restarts the timer). That is load-bearing rather than tidy: an
    /// instance portal *teleports you*, and the far side of the Deadmines
    /// portal is a spawn point which may itself sit inside the exit trigger. A
    /// latch carried across the map change would hold you out of a trigger you
    /// have never fired; one keyed on the id alone would hold you out of the
    /// *other* map's row that happens to share it.
    pub fn poll(&mut self, triggers: &dyn Triggers, map: u32, point: [f32; 3]) -> Option<u32> {
        if self.map != Some(map) {
            self.map = Some(map);
            self.occupied = None;
        }
        if let Some(id) = self.occupied {
            if triggers.holds(id, map, point) {
                return None;
            }
            self.occupied = None;
        }
        let entered = triggers.containing(map, point)?;
        self.occupied = Some(entered);
        Some(entered)
    }
}

/// **`SMSG_TRANSFER_ABORTED`: the server declined to move you, and why.**
///
/// One byte in this build. The reasons are `Player.h`'s `TRANSFER_ABORT_*` and
/// the client's own handler reads the byte, subtracts one, refuses anything
/// above four and picks one of five entries. Four of the five entries load a
/// `GlobalStrings.lua` key; the **fourth entry is the out-of-range case
/// itself**, which is what makes `TRANSFER_ABORT_SILENTLY` silent — the
/// client's behaviour rather than an inference from vmangos' comment. Every
/// path, message or not, then logs "World transfer aborted..." and takes the
/// pending transfer down.
///
/// So reason 0, reason 4 and anything past 5 all show nothing, and
/// [`TransferAbort::from_code`] answers `None` for each of them. That is the
/// client's behaviour and not a gap: `TRANSFER_ABORT_ERROR` is in
/// `GlobalStrings.lua` ("Transfer Aborted: Unknown Error") and the shipped
/// client never reaches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferAbort {
    /// 1 — the instance is full.
    MaxPlayers,
    /// 2 — the instance was not found.
    NotFound,
    /// 3 — you have entered too many instances recently. The one a player
    /// actually meets: vmangos counts five an hour by default.
    TooManyInstances,
    /// 5 — an encounter is in progress.
    ZoneInCombat,
}

impl TransferAbort {
    /// The reason byte, or `None` for the silent ones — see the type note.
    pub fn from_code(code: u8) -> Option<TransferAbort> {
        match code {
            1 => Some(TransferAbort::MaxPlayers),
            2 => Some(TransferAbort::NotFound),
            3 => Some(TransferAbort::TooManyInstances),
            5 => Some(TransferAbort::ZoneInCombat),
            _ => None,
        }
    }

    /// The `GlobalStrings.lua` key the reference shows for it. Looked up rather
    /// than composed, like every other message in this client.
    pub fn message_key(self) -> &'static str {
        match self {
            TransferAbort::MaxPlayers => "TRANSFER_ABORT_MAX_PLAYERS",
            TransferAbort::NotFound => "TRANSFER_ABORT_NOT_FOUND",
            TransferAbort::TooManyInstances => "TRANSFER_ABORT_TOO_MANY_INSTANCES",
            TransferAbort::ZoneInCombat => "TRANSFER_ABORT_ZONE_IN_COMBAT",
        }
    }
}

/// The reason byte out of `SMSG_TRANSFER_ABORTED`.
///
/// Returns the raw code rather than a [`TransferAbort`], so that a silent reason
/// is distinguishable from a body too short to read — the first is the server
/// working as designed and the second is a parse bug, and folding them together
/// is how a length error hides for a year.
pub fn parse_transfer_aborted(body: &[u8]) -> Option<u8> {
    let mut r = Reader::new(body);
    r.has(1).then(|| r.u8())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of concentric spheres, so the latch's two questions can differ.
    struct Fake {
        /// (id, radius) on map 0, in table order.
        rows: Vec<(u32, f32)>,
    }

    impl Fake {
        fn contains(&self, id: u32, map: u32, point: [f32; 3]) -> bool {
            if map != 0 {
                return false;
            }
            let d = point[0] * point[0] + point[1] * point[1] + point[2] * point[2];
            self.rows
                .iter()
                .find(|(row, _)| *row == id)
                .is_some_and(|(_, r)| d <= r * r)
        }
    }

    impl Triggers for Fake {
        fn containing(&self, map: u32, point: [f32; 3]) -> Option<u32> {
            self.rows
                .iter()
                .map(|(id, _)| *id)
                .find(|id| self.contains(*id, map, point))
        }

        fn holds(&self, id: u32, map: u32, point: [f32; 3]) -> bool {
            self.contains(id, map, point)
        }
    }

    fn table() -> Fake {
        Fake {
            rows: vec![(78, 7.0), (79, 30.0)],
        }
    }

    #[test]
    fn the_body_is_the_id_and_nothing_else() {
        assert_eq!(area_trigger_body(78), vec![78, 0, 0, 0]);
    }

    #[test]
    fn a_trigger_fires_once_on_entry_and_not_again_while_it_is_stood_in() {
        let table = table();
        let mut watch = TriggerWatch::new();
        // Outside everything.
        assert_eq!(watch.poll(&table, 0, [100.0, 0.0, 0.0]), None);
        // Walk in: fires.
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), Some(78));
        // Still in it, having moved: silent. Ten ticks a second for the couple
        // of seconds it takes to cross a seven-yard sphere is what this stops.
        assert_eq!(watch.poll(&table, 0, [1.0, 0.0, 0.0]), None);
        assert_eq!(watch.poll(&table, 0, [2.0, 0.0, 0.0]), None);
        assert_eq!(watch.occupied(), Some(78));
    }

    #[test]
    fn leaving_the_inner_trigger_reveals_the_one_it_was_inside() {
        let table = table();
        let mut watch = TriggerWatch::new();
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), Some(78));
        // Ten yards out is outside 78 and inside 79. The latch re-tests **78**
        // specifically rather than asking what is here — a client that asked
        // "am I still in the first trigger at this point" would be told yes
        // (79 is), and 79 would never fire.
        assert_eq!(watch.poll(&table, 0, [10.0, 0.0, 0.0]), Some(79));
        assert_eq!(watch.poll(&table, 0, [11.0, 0.0, 0.0]), None);
        // **And walking back into the inner one fires nothing**, which is the
        // client's rule taken to its conclusion rather than an oversight here:
        // the latch is now 79, the first thing the tick asks is whether 79
        // still holds the point, and at the origin it does — so the scan that
        // would have found 78 never runs. Overlapping triggers are therefore
        // mutually exclusive, latched to whichever was entered first, and only
        // leaving the outer one re-arms the inner.
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), None);
        assert_eq!(watch.occupied(), Some(79));
        // Out of both, then back in: an entry again.
        assert_eq!(watch.poll(&table, 0, [100.0, 0.0, 0.0]), None);
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), Some(78));
    }

    #[test]
    fn a_map_change_clears_the_latch() {
        let table = table();
        let mut watch = TriggerWatch::new();
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), Some(78));
        // The portal fired and moved us. Nothing on the new map matches here…
        assert_eq!(watch.poll(&table, 1, [0.0, 0.0, 0.0]), None);
        assert_eq!(watch.occupied(), None);
        // …and coming back is an entry, not a continuation.
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), Some(78));
    }

    #[test]
    fn the_silent_abort_reasons_are_the_clients_own() {
        assert_eq!(parse_transfer_aborted(&[3]), Some(3));
        assert_eq!(parse_transfer_aborted(&[]), None);
        assert_eq!(
            TransferAbort::from_code(3).map(TransferAbort::message_key),
            Some("TRANSFER_ABORT_TOO_MANY_INSTANCES")
        );
        assert_eq!(
            TransferAbort::from_code(1).map(TransferAbort::message_key),
            Some("TRANSFER_ABORT_MAX_PLAYERS")
        );
        assert_eq!(
            TransferAbort::from_code(5).map(TransferAbort::message_key),
            Some("TRANSFER_ABORT_ZONE_IN_COMBAT")
        );
        // 4 is `TRANSFER_ABORT_SILENTLY` and its jump-table entry is the
        // out-of-range label; 0 and 6 never reach the table at all.
        assert_eq!(TransferAbort::from_code(4), None);
        assert_eq!(TransferAbort::from_code(0), None);
        assert_eq!(TransferAbort::from_code(6), None);
    }
}
