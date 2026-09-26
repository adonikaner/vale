//! **What is on a corpse, and taking it** — the loot window's whole wire.
//!
//! ```text
//! CMSG_LOOT              guid          right-click a body
//!   -> SMSG_LOOT_RESPONSE              gold, and one row per item
//! CMSG_AUTOSTORE_LOOT_ITEM  slot       click a row
//!   -> SMSG_LOOT_REMOVED   slot        …and it is gone from everyone's window
//! CMSG_LOOT_MONEY                      click the coins
//!   -> SMSG_LOOT_CLEAR_MONEY           …and they are gone
//!   -> SMSG_LOOT_MONEY_NOTIFY  copper  …and this is your share
//! CMSG_LOOT_RELEASE      guid          close it
//!   -> SMSG_LOOT_RELEASE_RESPONSE      …and the server agrees it is closed
//! ```
//!
//! Seven opcodes, four of them one field long. What makes the family worth its
//! own module is not the parsing but three rules that are invisible in it, each
//! of which reads as a working loot window that quietly loses items.
//!
//! ## A refusal is the *same opcode* as an answer, told apart by one byte
//!
//! `Player::SendLootError` writes `guid`, `uint8(0)`, `uint8(error)` — ten bytes
//! of `SMSG_LOOT_RESPONSE`, where a real answer is `guid`, `uint8(lootType)`,
//! `uint32(gold)`, `uint8(count)`, and the rows. There is no separate opcode.
//!
//! **The discriminator is that no real loot type is 0.** `LootType` runs
//! `LOOT_CORPSE = 1` .. `LOOT_DISENCHANTING = 4` and `LOOT_SKINNING = 6`, and
//! the three above 20 are rewritten to one of those before the packet is built
//! (`LOOT_FISHINGHOLE` and `LOOT_FISHING_FAIL` both become `LOOT_FISHING`,
//! `LOOT_INSIGNIA` becomes `LOOT_CORPSE`) — so the byte on the wire is always
//! 1..6 for an answer and always 0 for a refusal. Reading a refusal as an answer
//! gives an empty window over a corpse you were not allowed to touch, with the
//! error's own code parsed as a gold amount.
//!
//! ## The slot on the wire is not the row on the screen
//!
//! `LootView` writes `uint8(i)` — the item's index in the server's *whole* loot
//! table — and then **skips** every row this viewer may not see: another
//! player's round-robin item, a blocked roll, a quest item for somebody else. So
//! the indices arrive sparse and out of nobody's control: a corpse can answer
//! rows 0, 3 and 7. `CMSG_AUTOSTORE_LOOT_ITEM` and `SMSG_LOOT_REMOVED` both
//! speak that same server index.
//!
//! The interface counts from 1 and expects them dense — `for index = 1,
//! LOOTFRAME_NUMBUTTONS` against `GetNumLootItems()`. **The two numberings are
//! crossed in exactly one place**, [`Loot::at`], for the same reason the bags'
//! two are: crossing them twice is how a client ends up looting the wrong row.
//!
//! ## …and the numbering is fixed for the window's whole life
//!
//! The server never re-packs its indices — a take sets `is_looted` and
//! `NotifyItemRemoved` echoes the same index — and the reference interface
//! numbers its buttons **once**, at `LOOT_OPENED`: its `LOOT_SLOT_CLEARED` arm
//! only hides `LootButton<slot>`, it never re-runs `LootFrame_Update`. So a
//! store that *erases* a taken row renumbers everything under buttons that
//! still hold their opening slots: the last button's click resolves to nothing
//! (a corpse whose second item cannot be looted, which is the reported bug) and
//! a middle button's click loots the row *below* the one it shows. A taken row
//! is therefore **marked, never removed** — see [`LootItem::taken`] — and the
//! coin row keeps row 1 after the coins are gone for the same reason.
//!
//! ## Money is two packets and neither of them is the amount taken
//!
//! `SMSG_LOOT_CLEAR_MONEY` has no body and means *the coins are gone from this
//! window*; `SMSG_LOOT_MONEY_NOTIFY` carries a `u32` and means *this is your
//! share*, which in a group is a fraction of what was there. They are separate
//! because the second is sent to everyone in range and the first only to the
//! people with the window open. Treating the notify as the clear leaves a coin
//! row on the screen for every other member of the group.
//!
//! ## …and the release is the server's to confirm
//!
//! `HandleLootReleaseOpcode` **reads the guid off the wire and throws it away**,
//! using its own stored one instead. So the guid this client sends is a
//! formality; what closes the window is `SMSG_LOOT_RELEASE_RESPONSE` coming
//! back. Closing locally on the send is what produces a window that will not
//! reopen, because the server still thinks you have it.

use crate::bytes::{Reader, Writer};

/// Why the loot window is open, out of vmangos' `LootType`.
///
/// The three above 20 never reach the wire — see the module note — so this is
/// the whole of what a client can be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootType {
    Corpse,
    Pickpocketing,
    Fishing,
    Disenchanting,
    /// **Never sent as itself**: `SendLoot` rewrites skinning to
    /// `LOOT_PICKPOCKETING` because the 1.12 client has no case for 6. Carried
    /// so that a server which does send it is read rather than refused.
    Skinning,
}

impl LootType {
    /// The wire's byte, or `None` for **0 — which is a refusal rather than a
    /// type** (see the module note) — and for anything else.
    pub fn of(byte: u8) -> Option<LootType> {
        Some(match byte {
            1 => LootType::Corpse,
            2 => LootType::Pickpocketing,
            3 => LootType::Fishing,
            4 => LootType::Disenchanting,
            6 => LootType::Skinning,
            _ => return None,
        })
    }

    /// **Whether this window is a fishing catch**, which is the one thing the
    /// shipped interface asks: `LootFrame_OnShow` swaps the window's portrait
    /// for `FishingLoot-Icon` and plays a different sound.
    pub fn is_fishing(self) -> bool {
        matches!(self, LootType::Fishing)
    }
}

/// Why the server would not open a loot window.
///
/// vmangos' `LootError`, whose numbering is sparse — 1, 2, 3 and 7 are not
/// used — and whose sentences are the client's own. The strings are named
/// rather than reproduced: see [`LootError::key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootError {
    DidntKill,
    TooFar,
    BadFacing,
    Locked,
    NotStanding,
    Stunned,
    PlayerNotFound,
    PlayTimeExceeded,
    MasterInvFull,
    MasterUniqueItem,
    MasterOther,
    AlreadyPickpocketed,
    NotWhileShapeshifted,
}

impl LootError {
    pub fn of(byte: u8) -> Option<LootError> {
        Some(match byte {
            0 => LootError::DidntKill,
            4 => LootError::TooFar,
            5 => LootError::BadFacing,
            6 => LootError::Locked,
            8 => LootError::NotStanding,
            9 => LootError::Stunned,
            10 => LootError::PlayerNotFound,
            11 => LootError::PlayTimeExceeded,
            12 => LootError::MasterInvFull,
            13 => LootError::MasterUniqueItem,
            14 => LootError::MasterOther,
            15 => LootError::AlreadyPickpocketed,
            16 => LootError::NotWhileShapeshifted,
            _ => return None,
        })
    }

    /// **What it says, as a `GlobalStrings.lua` key** — never a sentence
    /// composed here, on the same terms every other refusal in this crate
    /// follows. A key the file does not carry displays as nothing, which is the
    /// client's own behaviour.
    ///
    /// **Ten of the thirteen are in the shipped file and three are not**:
    /// `ERR_LOOT_PLAY_TIME_EXCEEDED`, `ERR_LOOT_ALREADY_PICKPOCKETED` and
    /// `ERR_LOOT_NOTWHILESHAPESHIFTED`. That is checked rather than assumed —
    /// `grep` over the 4,592 keys — and it is the same shape as the mirror
    /// timers' missing `FEIGNDEATH_LABEL`: the client says nothing, so this one
    /// says nothing. The keys are still named, because a name that resolves to
    /// nothing is a different thing from a code with no name at all, and the
    /// first is fixable by a locale the second is not.
    pub fn key(self) -> &'static str {
        match self {
            LootError::DidntKill => "ERR_LOOT_DIDNT_KILL",
            LootError::TooFar => "ERR_LOOT_TOO_FAR",
            LootError::BadFacing => "ERR_LOOT_BAD_FACING",
            LootError::Locked => "ERR_LOOT_LOCKED",
            LootError::NotStanding => "ERR_LOOT_NOTSTANDING",
            LootError::Stunned => "ERR_LOOT_STUNNED",
            LootError::PlayerNotFound => "ERR_LOOT_PLAYER_NOT_FOUND",
            LootError::PlayTimeExceeded => "ERR_LOOT_PLAY_TIME_EXCEEDED",
            LootError::MasterInvFull => "ERR_LOOT_MASTER_INV_FULL",
            LootError::MasterUniqueItem => "ERR_LOOT_MASTER_UNIQUE_ITEM",
            LootError::MasterOther => "ERR_LOOT_MASTER_OTHER",
            LootError::AlreadyPickpocketed => "ERR_LOOT_ALREADY_PICKPOCKETED",
            LootError::NotWhileShapeshifted => "ERR_LOOT_NOTWHILESHAPESHIFTED",
        }
    }
}

/// What the *client* may do with a row, out of `LootSlotType`.
///
/// Only the first matters for a solo player and the rest are the group's, but
/// the byte is on every row and the interface's `LootSlotIsItem` is written
/// against it — a row that is `RollOngoing` or `Locked` is drawn and refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotType {
    /// The ordinary one: click it and it is yours.
    AllowLoot,
    /// A group roll is running. Drawn, not clickable.
    RollOngoing,
    /// The master looter's to hand out.
    Master,
    /// Shown in red — a requirement is not met.
    Locked,
    /// Solo looting: no binding confirmation.
    Owner,
}

impl SlotType {
    pub fn of(byte: u8) -> SlotType {
        match byte {
            1 => SlotType::RollOngoing,
            2 => SlotType::Master,
            3 => SlotType::Locked,
            4 => SlotType::Owner,
            // **Unknown reads as lootable**, deliberately: the alternative is a
            // row nobody can click on a server that adds a sixth type, where
            // this is at worst a click the server refuses.
            _ => SlotType::AllowLoot,
        }
    }

    /// Whether a click on this row would be sent at all.
    pub fn is_clickable(self) -> bool {
        matches!(self, SlotType::AllowLoot | SlotType::Owner)
    }
}

/// One row of a loot window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LootItem {
    /// **The server's index into its own loot table**, which is what
    /// `CMSG_AUTOSTORE_LOOT_ITEM` takes and what `SMSG_LOOT_REMOVED` names. Not
    /// the row on the screen — see the module note.
    pub index: u8,
    pub entry: u32,
    pub count: u32,
    /// `ItemPrototype::DisplayInfoID`, which is the *only* thing about the item
    /// that arrives in this packet: the name, the quality and the icon all need
    /// `CMSG_ITEM_QUERY_SINGLE`, exactly as a bag slot's do.
    pub display_id: u32,
    /// **Always zero on the wire in 1.12.** `operator<<(ByteBuffer&, LootItem
    /// const&)` writes a literal `uint32(0)` here where later builds put the
    /// random suffix. Kept as a field rather than skipped so that the layout in
    /// the parser matches the layout in the server's own writer, line for line.
    pub random_suffix: u32,
    pub random_property: u32,
    pub slot_type: SlotType,
    /// **Taken, but still holding its row.** The window's numbering is fixed
    /// for its whole life — see the module note — so a looted row is marked
    /// rather than removed, and [`Loot::at`] answers `None` for it exactly as
    /// the reference's `GetLootSlotInfo` answers a hidden button.
    pub taken: bool,
}

/// A whole loot window as the server described it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Loot {
    /// Whose body or chest this is. Needed to release it, and the one thing
    /// that says the window is still about the same corpse.
    pub guid: u64,
    /// `None` for a **refusal**, which is the same opcode — see [`LootError`]
    /// and the module note.
    pub kind: Option<LootType>,
    pub error: Option<LootError>,
    /// Copper on the body. **0 means no coin row at all**, which is how the
    /// interface tells one from an empty one — and it goes to 0 when the coins
    /// are taken, while [`Self::coin_row`] remembers the row existed.
    pub gold: u32,
    /// **Whether the window opened with a coin row** — fixed for its life, even
    /// after the coins are taken, so the item rows never renumber under the
    /// interface's buttons. See the module note.
    pub coin_row: bool,
    pub items: Vec<LootItem>,
}

impl Loot {
    /// **Row `n` on the screen, one-based, as the interface counts.**
    ///
    /// The one place the server's sparse indices and the interface's dense
    /// one-based rows are crossed. Row 1 is the coins when the window opened
    /// with any — which is `LootFrame.lua`'s own arithmetic: it asks
    /// `LootSlotIsCoin(slot)` of every slot it draws, and only slot 1 can ever
    /// answer yes. A **taken** row answers `None` while everything below it
    /// keeps its number — the stable-numbering rule in the module note.
    pub fn at(&self, row: usize) -> Option<&LootItem> {
        let row = row.checked_sub(1)?;
        let row = match self.coin_row {
            true => row.checked_sub(1)?,
            false => row,
        };
        let item = self.items.get(row)?;
        (!item.taken).then_some(item)
    }

    /// How many rows the window has, coins included. **Fixed for the window's
    /// life** — a taken row still counts, exactly as the reference's
    /// `numLootItems` is read once at `LOOT_OPENED` and never again.
    pub fn rows(&self) -> usize {
        self.items.len() + usize::from(self.coin_row)
    }

    /// **Nothing is left on the body** — the whole of the auto-close rule.
    ///
    /// ## The server does not close an emptied window, and that was measured
    ///
    /// Against the running vmangos, `vale live`: kill a Juvenile Snow
    /// Leopard, `loot 1` (one row, `Discolored Fang`), `take 1`. Back comes
    /// `SMSG_LOOT_REMOVED index 0` and then, over the next 21 seconds of
    /// listening on the loot queue, **nothing at all** — no
    /// `SMSG_LOOT_RELEASE_RESPONSE`, no second `SMSG_LOOT_RESPONSE`. The body
    /// stays open on the server for as long as the client holds it.
    ///
    /// The shipped interface does not close it either. `LootFrame_OnEvent`'s
    /// `LOOT_SLOT_CLEARED` arm hides the button, then pages down if every
    /// button is hidden *and there is a next page*; with no next page it falls
    /// off the end and returns. So an emptied window in this client stayed up,
    /// blank, until the player pressed the X.
    ///
    /// **The close is therefore the client's own**, which is a reading of those
    /// two measurements. What is reproduced is the behaviour: the last row
    /// going sends `CMSG_LOOT_RELEASE`, and the window shuts on the answer
    /// like every other close. See
    /// `crate::socket::session::LiveSession::loot_release` and
    /// `vale-client`'s `game::npc::loot`.
    ///
    /// **A row that is present and not taken keeps the window open**, whether
    /// or not *this* viewer may click it: a group member's roll is still a row
    /// on the screen, and a window that vanished from under one would be worse
    /// than one that waits.
    pub fn is_empty(&self) -> bool {
        self.gold == 0 && self.items.iter().all(|item| item.taken)
    }

    /// Whether row `n` is the coins — and still has any to take.
    pub fn is_money(&self, row: usize) -> bool {
        self.coin_row && self.gold > 0 && row == 1
    }

    /// **The group roll on this row is over — it may be clicked again.**
    ///
    /// A row that opened as [`SlotType::RollOngoing`] is drawn and refuses the
    /// click, and **nothing on the wire ever takes that back.** vmangos clears
    /// its own `is_blocked` on both endings and sends no packet at all about it;
    /// the reference does the local half itself: it checks the open window's
    /// guid against the one the roll named and then writes
    /// `LOOT_SLOT_TYPE_ALLOW_LOOT` — a literal 0 — into the slot's record.
    ///
    /// So this is the whole of "the item is not lootable after everybody
    /// passed", and it is called from **both** endings, not only the pass:
    /// `SMSG_LOOT_ROLL_WON`'s handler does the same.
    /// The win is usually invisible anyway — the server has already stored the
    /// item and an `SMSG_LOOT_REMOVED` marks the row taken — but a winner whose
    /// bags are full is left holding a row that would otherwise stay dead.
    ///
    /// `false` for a slot this window does not hold, which is the ordinary case
    /// for a roll that ended on a body nobody here has open.
    pub fn unblock(&mut self, index: u8) -> bool {
        let Some(item) = self.items.iter_mut().find(|item| item.index == index) else {
            return false;
        };
        // **Only the blocked state is lifted.** A `Master` or a `Locked` row is
        // refused for a reason the roll has nothing to do with, and the
        // reference writes its 0 over whatever was there only because a rolled
        // row can be nothing else.
        if item.slot_type != SlotType::RollOngoing {
            return false;
        }
        item.slot_type = SlotType::AllowLoot;
        true
    }

    /// Mark a row taken — what `SMSG_LOOT_REMOVED` does, by the **server's**
    /// index.
    ///
    /// Answers the screen row it holds (and keeps), so the interface can be
    /// told which button to hide. `None` for an index this window does not
    /// hold — an ordinary arrival rather than an error: a group member looting
    /// a row this viewer could not see is removed for them and never existed
    /// here — and `None` the second time, which is the same statement repeated.
    pub fn remove(&mut self, index: u8) -> Option<usize> {
        let at = self
            .items
            .iter()
            .position(|item| item.index == index && !item.taken)?;
        self.items[at].taken = true;
        Some(at + 1 + usize::from(self.coin_row))
    }
}

/// `SMSG_LOOT_RESPONSE` — the window, or the refusal wearing the same opcode.
pub fn parse_loot_response(body: &[u8]) -> Option<Loot> {
    let mut r = Reader::new(body);
    if !r.has(8 + 1) {
        return None;
    }
    let guid = r.u64();
    let kind = r.u8();
    // **Zero is a refusal**, and the byte after it is the reason. See the module
    // note: there is no other opcode for this.
    if kind == 0 {
        return Some(Loot {
            guid,
            kind: None,
            error: r.has(1).then(|| LootError::of(r.u8())).flatten(),
            ..Loot::default()
        });
    }
    if !r.has(4 + 1) {
        return None;
    }
    let gold = r.u32();
    let count = r.u8();
    let mut items = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        // 1 + 4*5 + 1. A truncated tail stops the read rather than failing it:
        // the rows already parsed are real and the window is better short than
        // absent.
        if !r.has(1 + 20 + 1) {
            break;
        }
        items.push(LootItem {
            index: r.u8(),
            entry: r.u32(),
            count: r.u32(),
            display_id: r.u32(),
            random_suffix: r.u32(),
            random_property: r.u32(),
            slot_type: SlotType::of(r.u8()),
            taken: false,
        });
    }
    Some(Loot {
        guid,
        kind: LootType::of(kind),
        error: None,
        gold,
        coin_row: gold > 0,
        items,
    })
}

/// `SMSG_LOOT_RELEASE_RESPONSE` — the guid, and a byte that is always 1.
///
/// **This is what closes the window**, not the send that asked for it: the
/// server reads the guid off `CMSG_LOOT_RELEASE` and ignores it in favour of its
/// own record, so until this arrives the server still thinks the body is open.
pub fn parse_loot_release_response(body: &[u8]) -> Option<u64> {
    let mut r = Reader::new(body);
    r.has(8).then(|| r.u64())
}

/// `SMSG_LOOT_REMOVED` — one row is gone, by the **server's** index.
pub fn parse_loot_removed(body: &[u8]) -> Option<u8> {
    let mut r = Reader::new(body);
    r.has(1).then(|| r.u8())
}

/// `SMSG_LOOT_MONEY_NOTIFY` — *your share*, in copper, which in a group is not
/// what was on the body.
pub fn parse_loot_money_notify(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

// --- what we send -----------------------------------------------------------

/// `CMSG_LOOT` — open the window on this guid.
pub fn loot_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_LOOT_RELEASE` — close it.
///
/// The guid is written because the packet has the field; the server discards it
/// (`recv_data.read_skip<uint64>()`) and releases whatever it last recorded.
pub fn loot_release_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_AUTOSTORE_LOOT_ITEM` — take row `index`, by the **server's** index.
pub fn autostore_loot_item_body(index: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(index);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(kind: u8, gold: u32, rows: &[(u8, u32, u32, u8)]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(0x1234_5678_9abc_def0);
        w.u8(kind);
        w.u32(gold);
        w.u8(rows.len() as u8);
        for (index, entry, count, slot) in rows {
            w.u8(*index);
            w.u32(*entry);
            w.u32(*count);
            w.u32(7);
            w.u32(0);
            w.u32(0);
            w.u8(*slot);
        }
        w.buf
    }

    /// The ordinary window: gold, and the rows the server let this viewer see.
    #[test]
    fn a_loot_response_carries_the_gold_and_one_row_per_item() {
        let loot = parse_loot_response(&response(1, 137, &[(0, 2589, 3, 0), (3, 858, 1, 0)]))
            .expect("a window");
        assert_eq!(loot.kind, Some(LootType::Corpse));
        assert_eq!(loot.error, None);
        assert_eq!(loot.gold, 137);
        assert_eq!(loot.items.len(), 2);
        assert_eq!(loot.items[0].entry, 2589);
        assert_eq!(loot.items[0].count, 3);
        assert_eq!(loot.items[1].index, 3, "the server's index, not the row");
    }

    /// **A refusal is the same opcode**, and reading it as an answer gives an
    /// empty window with the error code parsed as an amount of gold. The whole
    /// discriminator is that no real loot type is 0.
    #[test]
    fn a_refusal_wears_the_same_opcode_and_is_told_apart_by_a_zero_type() {
        let mut w = Writer::new();
        w.u64(9);
        w.u8(0);
        w.u8(4);
        let loot = parse_loot_response(&w.buf).expect("a refusal");
        assert_eq!(loot.kind, None);
        assert_eq!(loot.error, Some(LootError::TooFar));
        assert_eq!(loot.gold, 0, "a refusal has no gold to misread");
        assert!(loot.items.is_empty());
        // …and every type the wire can carry is a window rather than a refusal.
        for kind in [1u8, 2, 3, 4, 6] {
            let parsed = parse_loot_response(&response(kind, 0, &[])).expect("a window");
            assert!(parsed.kind.is_some(), "type {kind} read as a refusal");
            assert_eq!(parsed.error, None);
        }
    }

    /// **The two numberings cross in one place.** The interface counts rows from
    /// 1 with the coins first; the server counts its own table from 0 and skips
    /// what this viewer may not see.
    #[test]
    fn the_screen_row_and_the_servers_index_are_different_numbers() {
        let loot = parse_loot_response(&response(1, 50, &[(0, 2589, 1, 0), (3, 858, 1, 0)]))
            .expect("a window");
        assert_eq!(loot.rows(), 3, "two items and a coin row");
        assert!(loot.is_money(1));
        assert_eq!(loot.at(1), None, "row 1 is the coins, not an item");
        assert_eq!(loot.at(2).map(|i| i.index), Some(0));
        assert_eq!(loot.at(3).map(|i| i.index), Some(3));
        assert_eq!(loot.at(4), None);

        // …and with no coins the rows shift down by one.
        let dry = parse_loot_response(&response(1, 0, &[(0, 2589, 1, 0), (3, 858, 1, 0)]))
            .expect("a window");
        assert_eq!(dry.rows(), 2);
        assert!(!dry.is_money(1));
        assert_eq!(dry.at(1).map(|i| i.index), Some(0));
        assert_eq!(dry.at(2).map(|i| i.index), Some(3));
    }

    /// A removal names the *server's* index and answers the screen row it
    /// holds — **and keeps**: the numbering is fixed for the window's life, so
    /// the rows below a taken one do not move. The old model erased the row,
    /// which renumbered every button under the interface's feet: the last
    /// button's click resolved to nothing and a middle button's looted the row
    /// below the one it showed. An index this window never held is an ordinary
    /// arrival — a group member took a row this viewer could not see.
    #[test]
    fn a_removal_marks_the_row_and_the_others_keep_their_numbers() {
        let mut loot = parse_loot_response(&response(1, 50, &[(0, 2589, 1, 0), (3, 858, 1, 0)]))
            .expect("a window");
        assert_eq!(loot.remove(0), Some(2));
        // The taken row is gone from the screen and nothing renumbers:
        assert_eq!(loot.at(2), None, "a taken row answers nothing");
        assert_eq!(
            loot.at(3).map(|i| i.index),
            Some(3),
            "the row below a taken one keeps its number"
        );
        assert_eq!(loot.rows(), 3, "the count is fixed for the window's life");
        assert_eq!(loot.remove(0), None, "twice is not an error");
        assert_eq!(loot.remove(9), None, "and neither is somebody else's row");
        assert_eq!(loot.remove(3), Some(3));
        assert_eq!(loot.at(3), None);
    }

    /// **Taking the coins does not renumber the items either** — the second
    /// trigger of the same bug: `gold = 0` used to flip the `+1` offset inside
    /// `at`, shifting every item row the moment the coins were taken.
    #[test]
    fn clearing_the_coins_leaves_every_item_row_where_it_was() {
        let mut loot = parse_loot_response(&response(1, 50, &[(0, 2589, 1, 0), (3, 858, 1, 0)]))
            .expect("a window");
        assert!(loot.is_money(1));
        loot.gold = 0;
        assert!(!loot.is_money(1), "an emptied coin row is not clickable");
        assert_eq!(loot.at(1), None, "…and draws nothing");
        assert_eq!(loot.at(2).map(|i| i.index), Some(0));
        assert_eq!(loot.at(3).map(|i| i.index), Some(3));
        assert_eq!(loot.rows(), 3);
    }

    /// **A row held by a group roll is drawn and refused, and the roll's end is
    /// the only thing that frees it** — a rule with no packet behind it. See
    /// [`Loot::unblock`], and `crate::play::lootroll`.
    #[test]
    fn a_rolled_row_is_unclickable_until_the_roll_ends() {
        let mut loot =
            parse_loot_response(&response(1, 0, &[(0, 2589, 1, 1), (3, 858, 1, 0), (5, 774, 1, 3)]))
                .expect("a window");
        assert!(!loot.at(1).expect("row 1").slot_type.is_clickable());
        assert!(loot.unblock(0));
        assert!(loot.at(1).expect("row 1").slot_type.is_clickable());
        // Twice is not an error and says nothing happened, which is what a
        // `SMSG_LOOT_ROLL_WON` after a `SMSG_LOOT_ALL_PASSED` would be.
        assert!(!loot.unblock(0));
        // …and a slot this window does not hold is the ordinary case: the roll
        // ended on a body nobody here has open.
        assert!(!loot.unblock(9));
        // **A row refused for another reason keeps its refusal.** Only the
        // blocked state is lifted — index 5 is `Locked`, which is a requirement
        // the roll has nothing to say about.
        assert!(!loot.unblock(5));
        assert!(!loot.at(3).expect("row 3").slot_type.is_clickable());
    }

    /// A row type this build has no name for is **lootable** rather than
    /// unclickable: a click the server refuses beats a row nobody can press.
    #[test]
    fn an_unknown_slot_type_is_lootable() {
        assert_eq!(SlotType::of(0), SlotType::AllowLoot);
        assert_eq!(SlotType::of(99), SlotType::AllowLoot);
        assert!(SlotType::of(4).is_clickable());
        assert!(!SlotType::of(1).is_clickable());
        assert!(!SlotType::of(3).is_clickable());
    }

    /// A truncated tail keeps the rows that did parse. These are packets off a
    /// twenty-year-old protocol and a short window beats none.
    #[test]
    fn a_truncated_row_stops_the_read_rather_than_losing_the_window() {
        let mut body = response(1, 10, &[(0, 2589, 1, 0), (1, 858, 1, 0)]);
        body.truncate(body.len() - 5);
        let loot = parse_loot_response(&body).expect("a window");
        assert_eq!(loot.gold, 10);
        assert_eq!(loot.items.len(), 1);
    }

    /// The three short ones, and the two bodies this client writes.
    #[test]
    fn the_short_packets_and_the_bodies_round_trip() {
        assert_eq!(parse_loot_removed(&[7]), Some(7));
        assert_eq!(parse_loot_removed(&[]), None);
        assert_eq!(parse_loot_money_notify(&137u32.to_le_bytes()), Some(137));
        assert_eq!(parse_loot_money_notify(&[0, 0]), None);
        let mut w = Writer::new();
        w.u64(42);
        w.u8(1);
        assert_eq!(parse_loot_release_response(&w.buf), Some(42));

        assert_eq!(loot_body(42), 42u64.to_le_bytes());
        assert_eq!(loot_release_body(42), 42u64.to_le_bytes());
        assert_eq!(autostore_loot_item_body(3), vec![3]);
    }

    /// Every error the server can send has a key, and it is the game's own name
    /// rather than a sentence composed here.
    #[test]
    fn every_error_names_a_global_string() {
        for byte in 0..=16u8 {
            let Some(error) = LootError::of(byte) else {
                continue;
            };
            assert!(error.key().starts_with("ERR_LOOT_"), "{byte} -> {error:?}");
        }
        // The gaps are gaps and not silently mapped to something.
        for byte in [1u8, 2, 3, 7, 17, 200] {
            assert_eq!(LootError::of(byte), None, "{byte} is not an error code");
        }
    }

    /// **The whole of the auto-close rule**, and the two ways it must not fire.
    ///
    /// Measured against the running server first: an emptied body produces
    /// `SMSG_LOOT_REMOVED` and nothing else, ever — see [`Loot::is_empty`],
    /// which carries the run. So the close is the client's, and this is the
    /// predicate behind it.
    #[test]
    fn a_body_is_empty_only_when_every_row_and_the_coins_are_gone() {
        let mut loot = parse_loot_response(&response(1, 137, &[(0, 2589, 3, 0), (3, 858, 1, 0)]))
            .expect("a window");
        assert!(!loot.is_empty(), "two rows and coins");

        loot.remove(0);
        assert!(!loot.is_empty(), "one row still on the body");
        loot.remove(3);
        // **The coins still hold it open.** They are a row on the screen and
        // they are cleared by their own packet — a window that shut on the last
        // *item* would take the money with it.
        assert!(!loot.is_empty(), "the coin row is still there");
        loot.gold = 0;
        assert!(loot.is_empty());
    }

    /// **An empty answer is not a closed window**, which is why the client
    /// checks this on a removal and never on the response.
    ///
    /// `LootFrame_OnShow` plays `LOOTWINDOWOPENEMPTY` when `GetNumLootItems()`
    /// is zero — the game has a *sound* for opening a body with nothing on it,
    /// so closing one on sight would be a departure rather than a convenience.
    #[test]
    fn a_body_that_opens_with_nothing_on_it_is_already_empty() {
        let loot = parse_loot_response(&response(1, 0, &[])).expect("a window");
        assert_eq!(loot.rows(), 0);
        assert!(loot.is_empty());
    }

}
