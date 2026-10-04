//! Instance portals, and every other volume entered by walking into it.
//!
//! `CMSG_AREATRIGGER` is four bytes, and it is the one packet this client sends
//! without anything prompting it: no keypress, click or earlier packet. Nothing
//! on the server announces a dungeon entrance. The portal at the Deadmines is a
//! sphere in a shipped table, the server does not know the character is
//! standing in it, and nothing happens until the client says so. A client that
//! never sends it has no unhandled opcode and no missing reply to show for it;
//! the portal simply does nothing.
//!
//! The same id serves every kind of trigger. `WorldSession::HandleAreaTriggerOpcode`
//! receives it and the server decides what it means: a teleport (121 rows in
//! the reference database, the dungeon entrances among them), a tavern's rest
//! state, a quest's "area explored" objective, a battleground's flag capture,
//! a scripted event. The subject divides in two. Which volumes exist and
//! whether the character is in one is `vale_assets::tables::areatrigger`, a
//! shipped table and a geometry test that need no session. When to ask, what
//! to send and what comes back is this module.
//!
//! ## The clock and the latch
//!
//! Both are the client's, and both are described in the assets module: a
//! 100 ms tick, and an occupancy latch that re-tests the last trigger sent
//! before testing any other, so a trigger is sent once on entry and not again
//! until the character leaves it. [`TriggerWatch`] is the latch and
//! [`CHECK_INTERVAL`] the period.
//!
//! The period is not a detail. Testing on every session tick
//! ([`crate::socket::session::TICK`], 25 ms) would test four times as often as
//! 1.12, and the renderer's frame is faster still. The server re-runs the
//! containment test on receipt with a 5-yard tolerance
//! (`IsPointInAreaTriggerZone(…, 5.0f)`) and drops the packet if it fails, so
//! the position the client held when it decided is what is validated.
//!
//! ## What comes back
//!
//! A trigger the server accepts and then declines to act on answers
//! `SMSG_TRANSFER_ABORTED`: one byte in 1.12, the reason and nothing else (no
//! map id; that is a later build's shape). An instance that is full, or one
//! entered too many times in an hour, otherwise looks the same as a portal
//! that was never reported. Four of the reasons are a `GlobalStrings.lua` key
//! and the others show nothing; see [`TransferAbort`].
//!
//! A teleport the character does not qualify for (too low a level, an unmet
//! condition) answers `SMSG_AREA_TRIGGER_MESSAGE` with the row's own text; see
//! [`parse_area_trigger_message`].

use crate::bytes::{Reader, Writer};
use std::time::Duration;

/// How often the check runs: the 1.12.1 client tests its triggers every 100 ms.
pub const CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// `CMSG_AREATRIGGER`: the id of the trigger just entered, and nothing else.
///
/// The body is the matched row's id, field 0 of `AreaTrigger.dbc`, and
/// `HandleAreaTriggerOpcode` reads exactly one `uint32`.
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
/// It has two methods because the latch re-tests one specific row before any
/// other is tested. A caller given only [`Self::containing`] would have to ask
/// whether the first row here is still the one it remembers, which answers
/// wrongly when two triggers overlap and sends a portal the character is
/// already standing in again.
pub trait Triggers: Send {
    /// The first trigger on `map` whose volume contains `point`, in the table's
    /// own order. See the assets module for why order rather than distance.
    fn containing(&self, map: u32, point: [f32; 3]) -> Option<u32>;

    /// Whether that specific trigger still contains `point`.
    fn holds(&self, id: u32, map: u32, point: [f32; 3]) -> bool;
}

/// The caller's [`Triggers`], if it has any.
pub type TriggerTable = Box<dyn Triggers>;

/// The send-once-on-entry rule: which trigger the character is standing in,
/// and therefore which one may not be sent again.
///
/// It holds an id rather than a position or a flag. Each tick first re-tests
/// that row. Remembering only that something was sent recently would refuse a
/// different trigger overlapping the first, and remembering a position would
/// send again as soon as the character moved a yard inside the same sphere.
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
    /// A map change clears the latch, as it does in the 1.12.1 client, which
    /// also takes the new map's rows and restarts the timer. An instance portal
    /// teleports the character, and the arrival point may sit inside the exit
    /// trigger. A latch carried across the change would hold the character out
    /// of a trigger never sent, and one keyed on the id alone would hold it out
    /// of the other map's row with the same id.
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

/// `SMSG_TRANSFER_ABORTED`: the server declined to move the character, and
/// why.
///
/// One byte in this build. The reasons are `Player.h`'s `TRANSFER_ABORT_*`.
/// The 1.12.1 client shows a `GlobalStrings.lua` line for reasons 1, 2, 3 and
/// 5, and nothing for 0, for 4 (`TRANSFER_ABORT_SILENTLY`) or for anything
/// above 5; in every case it then drops the pending transfer.
///
/// [`TransferAbort::from_code`] answers `None` for each silent reason. That
/// matches the client: `TRANSFER_ABORT_ERROR` is in `GlobalStrings.lua`
/// ("Transfer Aborted: Unknown Error") and the client never shows it.
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

    /// The `GlobalStrings.lua` key the 1.12.1 client shows for it. Looked up
    /// rather than composed, like every other message in this client.
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
/// is distinguishable from a body too short to read. The first is the server
/// working as designed and the second is a parse error, and reporting both as
/// `None` would hide the error.
pub fn parse_transfer_aborted(body: &[u8]) -> Option<u8> {
    let mut r = Reader::new(body);
    r.has(1).then(|| r.u8())
}

/// `SMSG_AREA_TRIGGER_MESSAGE`: the text a teleport sends instead of moving
/// the character, when the character is below its `required_level` or fails
/// its `required_condition`. vmangos sends the row's `message`, or the
/// server's own level line when the row has none
/// (`WorldSession::SendAreaTriggerMessage`).
///
/// The body is a `u32` length and then that many bytes of text, the last of
/// them a NUL. The text is taken up to its NUL, and a length past the end of
/// the body is cut at the end. `None` only for a body too short to hold the
/// length. The 1.12.1 client shows the text as a `UI_INFO_MESSAGE`, the yellow
/// line in `UIErrorsFrame`, and shows nothing for an empty one.
pub fn parse_area_trigger_message(body: &[u8]) -> Option<String> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let length = r.u32() as usize;
    let raw = r.bytes(length.min(r.remaining()));
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    Some(String::from_utf8_lossy(&raw[..end]).into_owned())
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
        // Still in it, having moved: nothing is sent. Without the latch a
        // seven-yard sphere would be sent ten times a second while crossed.
        assert_eq!(watch.poll(&table, 0, [1.0, 0.0, 0.0]), None);
        assert_eq!(watch.poll(&table, 0, [2.0, 0.0, 0.0]), None);
        assert_eq!(watch.occupied(), Some(78));
    }

    #[test]
    fn leaving_the_inner_trigger_reveals_the_one_it_was_inside() {
        let table = table();
        let mut watch = TriggerWatch::new();
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), Some(78));
        // Ten yards out is outside 78 and inside 79. The latch re-tests 78
        // itself rather than asking what is here; asking whether the first
        // trigger here is the remembered one would answer yes (79 is), and 79
        // would never be sent.
        assert_eq!(watch.poll(&table, 0, [10.0, 0.0, 0.0]), Some(79));
        assert_eq!(watch.poll(&table, 0, [11.0, 0.0, 0.0]), None);
        // Walking back into the inner one sends nothing, as in the 1.12.1
        // client: the latch is now 79, the tick first asks whether 79 still
        // holds the point, and at the origin it does, so 78 is never tested.
        // Overlapping triggers are therefore exclusive, latched to whichever
        // was entered first, and only leaving the outer one re-arms the inner.
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
        // The portal was sent and moved the character. Nothing on the new map
        // matches here…
        assert_eq!(watch.poll(&table, 1, [0.0, 0.0, 0.0]), None);
        assert_eq!(watch.occupied(), None);
        // …and coming back is an entry, not a continuation.
        assert_eq!(watch.poll(&table, 0, [0.0, 0.0, 0.0]), Some(78));
    }

    /// The body as vmangos wrote it for a teleport refused at level 50: the
    /// length counts the NUL.
    #[test]
    fn a_trigger_message_is_its_length_and_its_text() {
        let body = [
            0x0e, 0x00, 0x00, 0x00, b'T', b'e', b's', b't', b' ', b'T', b'e', b'l', b'e', b' ', b'e', b'v', b't', 0x00,
        ];
        assert_eq!(parse_area_trigger_message(&body).as_deref(), Some("Test Tele evt"));
        // A length past the body is cut at its end, and no NUL is needed.
        assert_eq!(parse_area_trigger_message(&[9, 0, 0, 0, b'h', b'i']).as_deref(), Some("hi"));
        assert_eq!(parse_area_trigger_message(&[1, 0, 0, 0, 0]).as_deref(), Some(""));
        assert_eq!(parse_area_trigger_message(&[1, 0]), None);
    }

    #[test]
    fn the_silent_abort_reasons_show_nothing() {
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
        // 4 is `TRANSFER_ABORT_SILENTLY`, which shows nothing, as do 0 and 6.
        assert_eq!(TransferAbort::from_code(4), None);
        assert_eq!(TransferAbort::from_code(0), None);
        assert_eq!(TransferAbort::from_code(6), None);
    }
}
