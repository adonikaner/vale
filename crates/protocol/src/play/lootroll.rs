//! **The roll that decides who gets it** — group loot's own five opcodes.
//!
//! ```text
//! SMSG_LOOT_START_ROLL   guid, slot, item, countdown   a frame appears
//! CMSG_LOOT_ROLL         guid, slot, vote              need, greed or pass
//!   -> SMSG_LOOT_ROLL    …one per vote, to everyone eligible
//!   -> SMSG_LOOT_ROLL_WON   winner, number             …and who took it
//!   -> SMSG_LOOT_ALL_PASSED                            …or that nobody did
//! ```
//!
//! The window on the body is [`super::loot`]; this is the machinery beside it.
//! They meet in exactly one place — a row whose [`super::loot::SlotType`] is
//! `RollOngoing` is drawn and refuses the click until the roll ends, and both
//! endings unblock it. See [`super::loot::Loot::unblock`].
//!
//! ## The roll has no id on the wire, and the client invents one
//!
//! Every packet in this family names the roll by **`(guid, item slot)`** and
//! nothing else. The interface names it by a single `rollID`:
//! `GroupLootFrame_OpenNewFrame(id, rollTime)`, `RollOnLoot(id, rollType)`,
//! `GetLootRollItemInfo(id)`. There is nothing on the wire to be that id.
//!
//! The client settles what it is. Its `SMSG_LOOT_START_ROLL` handler keeps a
//! pending-roll record per roll in a list, and the id it gives each one is **a
//! client-side counter, incremented per roll started, never reset within a
//! session.** So the id is local, it is dense, and it is meaningless to the
//! server: the roll a vote names is looked back up by that id and it is the
//! *guid and slot* that go out. See [`loot_roll_body`].
//!
//! **`SMSG_LOOT_ROLL_WON` and `SMSG_LOOT_ALL_PASSED` can name a roll that never
//! started here**, and the reference has a branch for it: both handlers look
//! the record up by `(guid, slot)` and, failing, build one with an id of
//! `0xffffffff` purely to compose the chat line from.
//! That is not a corner case — the server sends the result to everyone who was
//! *eligible*, and a member who joined the group after the roll opened is
//! eligible for nothing and was sent no start.
//!
//! ## A vote byte and a roll number are different numbers on the same field
//!
//! `SMSG_LOOT_ROLL` carries `(rollNumber, rollType)` and vmangos writes four
//! different pairs into them — `(0, 0)` for a Need *choice*, `(128, 2)` for a
//! Greed choice, `(128, 128)` for a pass, and `(1..100, 1|2)` for the actual
//! dice. So the byte that looks like a vote is only a vote for three of the
//! four, and the discriminator is **the number, not the type**: the client
//! tests the number as a *signed* byte, which is `number >= 128`.
//! See [`RollLine::of`], which is that branch table transcribed.
//!
//! ## …and one of those lines is a CVar
//!
//! `showLootSpam`, default `"1"`. Both announcement paths read the CVar's
//! value:
//!
//! * on: every vote and every die roll gets its own `CHAT_MSG_LOOT` line, and
//!   the win is `LOOT_ROLL_WON` / `LOOT_ROLL_YOU_WON` with no number in it.
//! * off: the per-vote lines are **not composed at all** (the client frees the
//!   record and returns), and the win carries the winning roll instead —
//!   `LOOT_ROLL_WON_NO_SPAM_NEED` and its three siblings.
//!
//! The polarity is the easy thing to get backwards, so it is stated twice: a
//! **non-zero** value is the chatty one.
//!
//! ## `SMSG_LOOT_ALL_PASSED` writes its two dead fields the other way round
//!
//! Every other packet here is `…entry, randomSuffix, randomPropId…`. That one is
//! `…entry, itemRandomPropId, randomSuffixId…` — vmangos'
//! `LootAllPassed::AppendBodyTo` says so, and it is not a typo on their side
//! either. Both are always zero in 1.12, so reading them transposed is invisible
//! until a server sends one; they are parsed in the server's own order anyway,
//! because a layout that matches the writer line for line is the only kind that
//! can be checked.

use crate::bytes::{Reader, Writer};

/// **How long a roll lasts**, out of vmangos' `LOOT_ROLL_TIMEOUT`
/// (`Group.cpp:67`): one minute, in milliseconds.
///
/// Never used to decide anything — the countdown is on the wire and the client
/// takes what it is given. It is here as the number a test can be written
/// against and as the answer to "how stale can a pending roll get".
pub const ROLL_TIMEOUT_MS: u32 = 60_000;

/// **How many roll frames the game has**: `NUM_GROUP_LOOT_FRAMES = 4`, in
/// `LootFrame.lua`'s own second line.
///
/// The C side knows it too and enforces it: it compares the shown count
/// against 4 and, when it is full, marks the record deferred and **fires no
/// event at all**. It walks the list afterwards and promotes the deferred ones
/// as frames free up. So a fifth simultaneous roll is not dropped and not
/// drawn — it waits its turn, and it waits on the *original* countdown, since
/// the promotion re-sends the stored countdown rather than the remainder.
pub const MAX_ROLL_FRAMES: usize = 4;

/// What the player pressed, out of vmangos' `RollVote`.
///
/// The three the client may send; `ROLL_NOT_EMITED_YET = 3` and
/// `ROLL_NOT_VALID = 4` exist on the server and `MAX_ROLL_FROM_CLIENT = 3` is
/// what `HandleLootRoll` refuses above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollVote {
    Pass,
    Need,
    Greed,
}

impl RollVote {
    /// The interface's own argument: `RollOnLoot(rollID, 0|1|2)`, wired into
    /// `GroupLootFrameTemplate`'s three buttons in `LootFrame.xml`.
    ///
    /// `None` for anything else, which is what an addon passing 7 gets — the
    /// server would refuse it (`>= MAX_ROLL_FROM_CLIENT`) and a packet nobody
    /// answers is worse than a press that did nothing.
    pub fn of(byte: u8) -> Option<RollVote> {
        Some(match byte {
            0 => RollVote::Pass,
            1 => RollVote::Need,
            2 => RollVote::Greed,
            _ => return None,
        })
    }

    pub fn byte(self) -> u8 {
        match self {
            RollVote::Pass => 0,
            RollVote::Need => 1,
            RollVote::Greed => 2,
        }
    }
}

/// `SMSG_LOOT_START_ROLL` — a frame appears, and a clock starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RollStart {
    /// The **body**, not the roll: this and [`Self::item_slot`] are together the
    /// only name the roll has on the wire.
    pub guid: u64,
    /// The server's index into its own loot table — the same number
    /// `CMSG_AUTOSTORE_LOOT_ITEM` and `SMSG_LOOT_REMOVED` speak, and **not** a
    /// row on any screen. See [`super::loot`].
    pub item_slot: u32,
    pub entry: u32,
    /// Always zero in 1.12 — vmangos writes a literal 0. Kept so the parse
    /// matches the writer line for line.
    pub random_suffix: u32,
    pub random_property: u32,
    /// Milliseconds. [`ROLL_TIMEOUT_MS`] in practice, and taken from the wire
    /// rather than assumed.
    pub countdown_ms: u32,
}

/// `SMSG_LOOT_ROLL` — somebody voted, or somebody's dice landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RollCast {
    pub guid: u64,
    pub item_slot: u32,
    /// Who — **not** who is winning. The winner is [`RollWon::winner`].
    pub roller: u64,
    pub entry: u32,
    pub random_suffix: u32,
    pub random_property: u32,
    /// 1..100 for a real roll, and **128 for "this is a choice rather than a
    /// roll"** — see [`RollLine::of`].
    pub number: u8,
    /// 0 = need chosen, 1 = need roll, 2 = greed (either), 128 = passed. Not a
    /// [`RollVote`]: the two numberings only agree on 2.
    pub kind: u8,
}

/// `SMSG_LOOT_ROLL_WON` — and the item is already in their bags.
///
/// The server stores it before it sends this (`Group::CountTheRoll` calls
/// `StoreNewItem` and then `NotifyItemRemoved`), so the winner needs no click
/// and everyone with the window open gets an `SMSG_LOOT_REMOVED` for the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RollWon {
    pub guid: u64,
    pub item_slot: u32,
    pub entry: u32,
    pub random_suffix: u32,
    pub random_property: u32,
    pub winner: u64,
    pub number: u8,
    pub kind: u8,
}

/// `SMSG_LOOT_ALL_PASSED` — nobody wanted it, and the row is clickable again.
///
/// **The only ending that leaves the item on the body.** vmangos clears
/// `is_blocked` and sends nothing else; the reference does the local half
/// itself, writing `LOOT_SLOT_TYPE_ALLOW_LOOT` straight into the open window's
/// slot record. No packet says so — see [`super::loot::Loot::unblock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RollAllPassed {
    pub guid: u64,
    pub item_slot: u32,
    pub entry: u32,
    /// **Read before the suffix**, which is the other way round from every
    /// other packet in this family. See the module note.
    pub random_property: u32,
    pub random_suffix: u32,
}

/// **Which `GlobalStrings.lua` line a vote is announced as** — the client's own
/// branch table, in its own order.
///
/// Every arm is a key rather than a sentence, on the terms
/// [`super::loot::LootError::key`] states: the wording is the shipped file's,
/// and a key it does not carry displays as nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollLine {
    /// `%s has selected Need for: …`
    ChoseNeed,
    /// `%s has selected Greed for: …`
    ChoseGreed,
    /// `%s passed on: …`
    Passed,
    /// `Need Roll - %d for …`, and its greed twin. **Carries the number.**
    Rolled,
}

impl RollLine {
    /// The branch, out of `(number, kind)`.
    ///
    /// **The number is what decides, and it is tested as a signed byte.**
    /// The client tests its sign, so "a choice rather than a roll" is
    /// `number >= 128` — which is exactly the 128 vmangos writes for a pass and
    /// for a greed choice. Reading `kind` first instead puts a Greed *choice*
    /// (128, 2) and a greed *roll* (57, 2) on the same line, which is a chat
    /// window announcing that somebody rolled 128.
    pub fn of(number: u8, kind: u8) -> RollLine {
        let chose = number >= 128;
        match (chose, kind) {
            (true, 2) => RollLine::ChoseGreed,
            (true, _) => RollLine::Passed,
            (false, 0) => RollLine::ChoseNeed,
            (false, _) => RollLine::Rolled,
        }
    }

    /// Whether this line carries the roll number — the one shape difference
    /// between the four, and what decides how many arguments the sentence takes.
    pub fn has_number(self) -> bool {
        matches!(self, RollLine::Rolled)
    }

    /// **Whether the sentence names the roller after the link rather than
    /// before it**, which is true for the dice and for nothing else.
    ///
    /// `LOOT_ROLL_ROLLED_NEED` is `"Need Roll - %d for …|h%s by %s"` where every
    /// other key in the family opens with the name. It is not a numbered key
    /// either, so nothing in the text says so — the order is only visible in
    /// the order the client passes the arguments, which puts the roller last.
    pub fn names_the_roller_last(self) -> bool {
        matches!(self, RollLine::Rolled)
    }

    /// The key, given whether the roller is the local player and which of the
    /// two dice this is.
    ///
    /// **`Rolled` has no `_SELF` form.** `GlobalStrings.lua` carries
    /// `LOOT_ROLL_ROLLED_NEED_SELF` and `LOOT_ROLL_ROLLED_GREED_SELF`, and
    /// the client references neither: it picks between the two
    /// third-person keys on `kind == 2` alone, whoever rolled. The keys are in
    /// the file and the client does not use them, which is a different thing
    /// from a key that is missing — so this reproduces the client rather than
    /// the file.
    pub fn key(self, is_self: bool, kind: u8) -> &'static str {
        match (self, is_self) {
            (RollLine::ChoseNeed, true) => "LOOT_ROLL_NEED_SELF",
            (RollLine::ChoseNeed, false) => "LOOT_ROLL_NEED",
            (RollLine::ChoseGreed, true) => "LOOT_ROLL_GREED_SELF",
            (RollLine::ChoseGreed, false) => "LOOT_ROLL_GREED",
            (RollLine::Passed, true) => "LOOT_ROLL_PASSED_SELF",
            (RollLine::Passed, false) => "LOOT_ROLL_PASSED",
            (RollLine::Rolled, _) if kind == 2 => "LOOT_ROLL_ROLLED_GREED",
            (RollLine::Rolled, _) => "LOOT_ROLL_ROLLED_NEED",
        }
    }
}

/// **Which key announces the win**, given who won and whether the chatty CVar
/// is on — the client runs the same four-way branch in both win paths.
///
/// `spam` is `showLootSpam`, and **non-zero is the chatty one**: with it on the
/// line is plain, because every vote already had a line of its own; with it off
/// the number is folded into the win, since it is the only line there will be.
pub fn won_key(is_self: bool, spam: bool, kind: u8) -> &'static str {
    match (is_self, spam) {
        (true, true) => "LOOT_ROLL_YOU_WON",
        (false, true) => "LOOT_ROLL_WON",
        (true, false) if kind == 1 => "LOOT_ROLL_YOU_WON_NO_SPAM_NEED",
        (true, false) => "LOOT_ROLL_YOU_WON_NO_SPAM_GREED",
        (false, false) if kind == 1 => "LOOT_ROLL_WON_NO_SPAM_NEED",
        (false, false) => "LOOT_ROLL_WON_NO_SPAM_GREED",
    }
}

/// `Everyone passed on: …` — the one line in this family with no branch at all.
pub const ALL_PASSED_KEY: &str = "LOOT_ROLL_ALL_PASSED";

/// The name of the CVar behind [`won_key`]'s `spam` argument. See the module
/// note, and `assets::interface::cvars`, which already carries it with its
/// default of `"1"`.
pub const SHOW_LOOT_SPAM: &str = "showLootSpam";

/// `SMSG_LOOT_START_ROLL`.
pub fn parse_loot_start_roll(body: &[u8]) -> Option<RollStart> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 * 5) {
        return None;
    }
    Some(RollStart {
        guid: r.u64(),
        item_slot: r.u32(),
        entry: r.u32(),
        random_suffix: r.u32(),
        random_property: r.u32(),
        countdown_ms: r.u32(),
    })
}

/// `SMSG_LOOT_ROLL`.
pub fn parse_loot_roll(body: &[u8]) -> Option<RollCast> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 8 + 4 * 3 + 2) {
        return None;
    }
    Some(RollCast {
        guid: r.u64(),
        item_slot: r.u32(),
        roller: r.u64(),
        entry: r.u32(),
        random_suffix: r.u32(),
        random_property: r.u32(),
        number: r.u8(),
        kind: r.u8(),
    })
}

/// `SMSG_LOOT_ROLL_WON`.
pub fn parse_loot_roll_won(body: &[u8]) -> Option<RollWon> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 * 4 + 8 + 2) {
        return None;
    }
    Some(RollWon {
        guid: r.u64(),
        item_slot: r.u32(),
        entry: r.u32(),
        random_suffix: r.u32(),
        random_property: r.u32(),
        winner: r.u64(),
        number: r.u8(),
        kind: r.u8(),
    })
}

/// `SMSG_LOOT_ALL_PASSED` — **and its last two fields are transposed** relative
/// to the rest of the family. See the module note.
pub fn parse_loot_all_passed(body: &[u8]) -> Option<RollAllPassed> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 * 4) {
        return None;
    }
    Some(RollAllPassed {
        guid: r.u64(),
        item_slot: r.u32(),
        entry: r.u32(),
        random_property: r.u32(),
        random_suffix: r.u32(),
    })
}

// --- what we send -----------------------------------------------------------

/// `CMSG_LOOT_ROLL` — the guid and slot the roll is *really* named by, and the
/// vote.
///
/// The interface's `rollID` never leaves the client: it looks the pending
/// record up by it and writes the record's guid and slot out.
pub fn loot_roll_body(guid: u64, item_slot: u32, vote: RollVote) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.u32(item_slot);
    w.u8(vote.byte());
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(countdown: u32) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(0x1234_5678_9abc_def0);
        w.u32(3);
        w.u32(2589);
        w.u32(0);
        w.u32(0);
        w.u32(countdown);
        w.buf
    }

    /// The start, field for field against `LootStartRoll::AppendBodyTo`.
    #[test]
    fn a_start_carries_the_body_the_slot_the_item_and_the_clock() {
        let roll = parse_loot_start_roll(&start(60_000)).expect("a start");
        assert_eq!(roll.guid, 0x1234_5678_9abc_def0);
        assert_eq!(roll.item_slot, 3, "the server's index, not a screen row");
        assert_eq!(roll.entry, 2589);
        assert_eq!(roll.countdown_ms, ROLL_TIMEOUT_MS);
        // A short body is no roll rather than a roll with a zero clock — a
        // frame that opened with a dead timer would never close itself.
        let mut short = start(60_000);
        short.truncate(short.len() - 1);
        assert_eq!(parse_loot_start_roll(&short), None);
    }

    /// **The four `(number, kind)` pairs vmangos writes**, and the branch that
    /// tells a choice from a die. Getting this wrong prints "somebody rolled
    /// 128" for every pass in the game.
    #[test]
    fn the_number_and_not_the_type_says_whether_anybody_rolled() {
        // `CountRollVote`: pass, need, greed — the three choices.
        assert_eq!(RollLine::of(128, 128), RollLine::Passed);
        assert_eq!(RollLine::of(0, 0), RollLine::ChoseNeed);
        assert_eq!(RollLine::of(128, 2), RollLine::ChoseGreed);
        // …and `CountTheRoll`: the dice, `urand(1, 100)` with the vote as type.
        assert_eq!(RollLine::of(1, 1), RollLine::Rolled);
        assert_eq!(RollLine::of(100, 1), RollLine::Rolled);
        assert_eq!(RollLine::of(57, 2), RollLine::Rolled);
        // 127 is the last number that is a roll and 128 the first that is not.
        assert_eq!(RollLine::of(127, 2), RollLine::Rolled);
        assert_eq!(RollLine::of(128, 1), RollLine::Passed);
        // …and only the dice carry a number into their sentence, and only the
        // dice name the roller at the end of it.
        assert!(RollLine::of(57, 2).has_number());
        assert!(!RollLine::of(128, 2).has_number());
        assert!(RollLine::of(57, 2).names_the_roller_last());
        assert!(!RollLine::of(0, 0).names_the_roller_last());
        assert!(!RollLine::of(128, 128).names_the_roller_last());
    }

    /// Every line names a `GlobalStrings.lua` key, and the two families differ
    /// exactly where the client says they do: three have a `_SELF` form and the
    /// dice do not.
    #[test]
    fn every_line_names_a_global_string_and_only_three_have_a_self_form() {
        for (line, kind) in [
            (RollLine::ChoseNeed, 0u8),
            (RollLine::ChoseGreed, 2),
            (RollLine::Passed, 128),
        ] {
            assert!(line.key(true, kind).ends_with("_SELF"), "{line:?}");
            assert!(!line.key(false, kind).ends_with("_SELF"), "{line:?}");
        }
        assert_eq!(
            RollLine::Rolled.key(true, 1),
            RollLine::Rolled.key(false, 1),
            "the client references neither ROLLED _SELF key"
        );
        assert_eq!(RollLine::Rolled.key(true, 2), "LOOT_ROLL_ROLLED_GREED");
        for key in [
            RollLine::ChoseNeed.key(true, 0),
            RollLine::Passed.key(false, 128),
            RollLine::Rolled.key(false, 1),
            won_key(true, true, 1),
            won_key(false, false, 2),
            ALL_PASSED_KEY,
        ] {
            assert!(key.starts_with("LOOT_ROLL_"), "{key}");
        }
    }

    /// **`showLootSpam`'s polarity**, which is the easy thing to invert: a
    /// non-zero value is the chatty one, and the *quiet* setting is the one that
    /// folds the number into the win.
    #[test]
    fn the_quiet_setting_is_the_one_that_carries_the_number() {
        assert_eq!(won_key(true, true, 1), "LOOT_ROLL_YOU_WON");
        assert_eq!(won_key(false, true, 2), "LOOT_ROLL_WON");
        assert!(won_key(true, false, 1).contains("NO_SPAM_NEED"));
        assert!(won_key(true, false, 2).contains("NO_SPAM_GREED"));
        assert!(won_key(false, false, 1).contains("NO_SPAM_NEED"));
        assert!(won_key(false, false, 2).contains("NO_SPAM_GREED"));
    }

    /// The two long answers, and **the one whose tail is transposed**.
    #[test]
    fn the_win_and_the_all_passed_are_read_in_their_own_writers_order() {
        let mut w = Writer::new();
        w.u64(7);
        w.u32(3);
        w.u32(2589);
        w.u32(11); // randomSuffix
        w.u32(22); // itemRandomPropId
        w.u64(99);
        w.u8(84);
        w.u8(1);
        let won = parse_loot_roll_won(&w.buf).expect("a win");
        assert_eq!(won.winner, 99);
        assert_eq!(won.number, 84);
        assert_eq!(won.kind, 1);
        assert_eq!(won.random_suffix, 11);
        assert_eq!(won.random_property, 22);

        // …and `LootAllPassed::AppendBodyTo` writes the property first.
        let mut w = Writer::new();
        w.u64(7);
        w.u32(3);
        w.u32(2589);
        w.u32(22); // itemRandomPropId
        w.u32(11); // randomSuffixId
        let passed = parse_loot_all_passed(&w.buf).expect("all passed");
        assert_eq!(passed.random_property, 22);
        assert_eq!(passed.random_suffix, 11);
    }

    /// The vote, and the body it becomes. **The `rollID` is not in it** — the
    /// wire names a roll by the body and the slot.
    #[test]
    fn a_vote_goes_out_as_the_body_and_the_slot() {
        assert_eq!(RollVote::of(0), Some(RollVote::Pass));
        assert_eq!(RollVote::of(1), Some(RollVote::Need));
        assert_eq!(RollVote::of(2), Some(RollVote::Greed));
        // `MAX_ROLL_FROM_CLIENT` is 3, and the server drops anything above it.
        assert_eq!(RollVote::of(3), None);
        assert_eq!(RollVote::of(255), None);

        let body = loot_roll_body(0x1234_5678_9abc_def0, 3, RollVote::Greed);
        assert_eq!(body.len(), 13);
        assert_eq!(&body[..8], &0x1234_5678_9abc_def0u64.to_le_bytes());
        assert_eq!(&body[8..12], &3u32.to_le_bytes());
        assert_eq!(body[12], 2);
    }

    /// A truncated body is no roll at all rather than a roll with zeroes in it.
    /// These packets decide who gets an item; half of one is not better than
    /// none, which is the opposite of the rule the loot *window* follows.
    #[test]
    fn a_short_body_is_refused_rather_than_padded() {
        assert_eq!(parse_loot_roll(&[0u8; 33]), None);
        assert_eq!(parse_loot_roll_won(&[0u8; 33]), None);
        assert_eq!(parse_loot_all_passed(&[0u8; 23]), None);
        assert!(parse_loot_roll(&[0u8; 34]).is_some());
        assert!(parse_loot_roll_won(&[0u8; 34]).is_some());
        assert!(parse_loot_all_passed(&[0u8; 24]).is_some());
    }
}
