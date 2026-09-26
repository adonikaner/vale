//! **What it takes to open something** — `Lock.dbc` and `LockType.dbc`.
//!
//! A door, a chest, an ore vein and a strongbox are the *same* game-object type
//! as far as the server's template is concerned; what tells them apart is the
//! lock the template names, and that is the only place the difference is
//! written down. A vein is a chest whose lock says "Mining"; a herb is one whose
//! lock says "Herbalism"; the footlocker at the bottom of a mine is one whose
//! lock says "Pick Lock"; and the ordinary chest is one with no lock at all.
//!
//! So this table is what puts a pick on the pointer over an ore vein, and it is
//! the only input that can. See [`crate::look::object`], which is its one
//! consumer.
//!
//! ## The two tables
//!
//! ```text
//! Lock.dbc       209 rows x 33 fields   [0] id
//!                                       [1.. 8] type[8]     1 item, 2 skill
//!                                       [9..16] index[8]    -> LockType.dbc, or an item entry
//!                                      [17..24] skill[8]    the rank wanted, 0 for any
//!                                      [25..32] action[8]
//! LockType.dbc    19 rows x 29 fields   [0] id  [1] name
//! ```
//!
//! **Eight parallel arrays of one thing, not eight records** — the row is a
//! *list* of up to eight ways in, and a reader that took field 1 alone would
//! answer "Pick Lock" for the treasure chests whose first way in is a skill and
//! whose second is a key.
//!
//! The layout is vmangos' `LockEntry` and it is **measured rather than
//! assumed**: the four locks the ore and herb nodes in the running world
//! database actually name — 29 for Silverleaf and Peacebloom, 38 for a Copper
//! Vein, 39 for Tin, 41 for Iron — come out of these columns as
//! `(skill, Herbalism, 0)`, `(skill, Mining, 0)`, `(skill, Mining, 65)` and
//! `(skill, Mining, 125)`. Those are the four right answers, and the last two
//! are the ranks a miner reads off their own skill window.
//!
//! ## `LockType` ids are not skill line ids
//!
//! `LockType.dbc` row 2 is "Herbalism" and row 3 is "Mining"; the *skill lines*
//! of those names are 182 and 186. Nothing here crosses the two, and the reason
//! to say so is that both are small integers in adjacent columns of the same
//! subject — exactly the shape this project keeps paying for.
//!
//! The nineteen rows are 1 Pick Lock, 2 Herbalism, 3 Mining, 4 Disarm Trap,
//! 5 Open, 6 Treasure, 7 Calcified Elven Gems, 8 Close, and eleven more that no
//! 1.12 game object names.

use super::dbc::Dbc;
use std::collections::HashMap;

/// The `LockType.dbc` ids this client has a rule about, by name.
///
/// Named rather than matched as literals: 2 and 3 are one apart, both are "a gathering profession",
/// and swapping them puts a mining pick over a peacebloom.
pub mod lock_type {
    /// A rogue's `Pick Lock`, and a skeleton key.
    pub const PICK_LOCK: u32 = 1;
    pub const HERBALISM: u32 = 2;
    pub const MINING: u32 = 3;
    pub const DISARM_TRAP: u32 = 4;
    /// The `Opening` spell every quest goober and most levers are opened with —
    /// requires nothing, which is why so many locks carry a skill entry with a
    /// rank of zero.
    pub const OPEN: u32 = 5;
    pub const TREASURE: u32 = 6;
    pub const CALCIFIED_ELVEN_GEMS: u32 = 7;
    pub const CLOSE: u32 = 8;
}

/// `LOCK_KEY_TYPE`: what one of a lock's eight slots is keyed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// The slot names an **item entry** — a key, a skeleton key, a quest item.
    Item(u32),
    /// The slot names a **`LockType.dbc` row** and a rank of that skill.
    Skill { lock_type: u32, rank: u32 },
}

/// One way into a locked thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub kind: KeyKind,
    /// **Which of the row's eight columns this came out of**, kept because the
    /// reference's tooltip reads column **0 and no other** — see
    /// [`Locks::first_slot`], where the measurement is.
    pub slot: u8,
    /// `Action[slot]` — **what this way in does to the thing**, which decides
    /// whether it applies at all right now. See [`action_applies`].
    pub action: u32,
}

/// `Lock.dbc` and `LockType.dbc`, joined.
#[derive(Debug, Clone, Default)]
pub struct Locks {
    /// Lock id -> its ways in, in the row's own column order. **The order is
    /// kept**, because the first slot is the one a tooltip names.
    locks: HashMap<u32, Vec<Key>>,
    /// `LockType.dbc` id -> its name, for the "Requires Mining" line.
    names: HashMap<u32, String>,
}

/// How many slots a `Lock` row carries.
const SLOTS: usize = 8;
/// `LOCK_KEY_ITEM`.
const KEY_ITEM: u32 = 1;
/// `LOCK_KEY_SKILL`.
const KEY_SKILL: u32 = 2;

/// **Does one of a lock's ways in apply to a thing in this state?** —
/// the client's eleven branches, transcribed.
///
/// `Lock.dbc`'s fourth run of eight is an `Action`, and 1.12 uses three values
/// of it. The reference tests the action against the game object's
/// `GAMEOBJECT_STATE` and its `GO_FLAG_LOCKED` bit before it will draw a
/// requirement for that slot or count it as a way in:
///
/// ```text
/// 0  open       state READY   and not locked
/// 1  unlock     state READY   and locked
/// 2  close      state ACTIVE
/// 3              state READY
/// 4              state ACTIVE_ALTERNATIVE
/// …              anything but ACTIVE_ALTERNATIVE
/// ```
///
/// The data says the same thing from the other side: lock 2 is a strongbox
/// whose `Pick Lock` slot is action 1 and whose `Open` and `Treasure` slots are
/// action 0, so while the box is locked you pick it and once it is not you open
/// it — and lock 77 is `Quick Open` (0) beside `Quick Close` (2), which is a
/// door you can do one of two things to depending on which way it is standing.
///
/// A lock **on an item** has no state and no flag of its own; see
/// [`crate::look::object`], which is the only caller.
pub fn action_applies(action: u32, state: u8, locked: bool) -> bool {
    /// `GO_STATE_ACTIVE` — open, used, not reset.
    const ACTIVE: u8 = 0;
    /// `GO_STATE_READY` — shut, reset.
    const READY: u8 = 1;
    /// `GO_STATE_ACTIVE_ALTERNATIVE` — the second used state.
    const ALTERNATIVE: u8 = 2;
    match action {
        0 => state == READY && !locked,
        1 => state == READY && locked,
        2 => state == ACTIVE,
        3 => state == READY,
        4 => state == ALTERNATIVE,
        _ => state != ALTERNATIVE,
    }
}

impl Locks {
    /// Parse both, tolerating either being absent.
    ///
    /// **A missing table is an empty answer rather than a failure**, and the
    /// degradation is stated: no `Lock.dbc` is an ore vein that draws the plain
    /// interact hand and a tooltip with no "Requires Mining" line. It is still
    /// clickable and the server still refuses it if the skill is short, so
    /// nothing becomes unplayable — which is the direction every other optional
    /// table in [`super::dbc::DisplayTables`] errs in too.
    pub fn parse(lock_dbc: &[u8], lock_type_dbc: &[u8]) -> Locks {
        let mut locks = HashMap::new();
        if let Ok(table) = Dbc::parse(lock_dbc) {
            for record in 0..table.record_count {
                let Some(id) = table.u32_at(record, 0) else {
                    continue;
                };
                let mut keys = Vec::new();
                for slot in 0..SLOTS {
                    let kind = table.u32_at(record, 1 + slot).unwrap_or(0);
                    let index = table.u32_at(record, 1 + SLOTS + slot).unwrap_or(0);
                    let rank = table.u32_at(record, 1 + SLOTS * 2 + slot).unwrap_or(0);
                    // A slot with no type is an unused column, and every row has
                    // several: 209 rows of eight slots hold well under 209 keys
                    // between them.
                    let action = table.u32_at(record, 1 + SLOTS * 3 + slot).unwrap_or(0);
                    let kind = match kind {
                        KEY_ITEM => KeyKind::Item(index),
                        KEY_SKILL => KeyKind::Skill {
                            lock_type: index,
                            rank,
                        },
                        _ => continue,
                    };
                    keys.push(Key {
                        kind,
                        slot: slot as u8,
                        action,
                    });
                }
                if !keys.is_empty() {
                    locks.insert(id, keys);
                }
            }
        }
        let mut names = HashMap::new();
        if let Ok(table) = Dbc::parse(lock_type_dbc) {
            for record in 0..table.record_count {
                let (Some(id), Some(name)) = (table.u32_at(record, 0), table.string_at(record, 1))
                else {
                    continue;
                };
                if !name.is_empty() {
                    names.insert(id, name);
                }
            }
        }
        Locks { locks, names }
    }

    /// The ways into one lock; nothing for a lock id of zero, and nothing for
    /// one the table does not carry.
    pub fn keys(&self, lock_id: u32) -> &[Key] {
        self.locks.get(&lock_id).map_or(&[][..], Vec::as_slice)
    }

    /// **The first `LockType` a lock is keyed on**, which is the one a pointer
    /// and a tooltip are about.
    ///
    /// A lock with an item key first and a skill key second answers with the
    /// skill, deliberately: the pointer's job is to say *what to do here*, and
    /// "mine it" is actionable where "carry the right key" is not something a
    /// cursor can show.
    pub fn skill_lock(&self, lock_id: u32) -> Option<(u32, u32)> {
        self.keys(lock_id).iter().find_map(|key| match key.kind {
            KeyKind::Skill { lock_type, rank } => Some((lock_type, rank)),
            KeyKind::Item(_) => None,
        })
    }

    /// **The way in that column 0 holds**, or `None` — which is the ordinary
    /// case, because 1.12's `Lock.dbc` leaves column 0 empty on a great many
    /// rows and puts the real key in column 1, 2 or 3.
    ///
    /// It has a name of its own because the reference's game-object tooltip
    /// reads exactly this and nothing else: `Type[0]` at `[lock+4]` and
    /// `Index[0]` at `[lock+0x24]`, with no loop over the eight anywhere. So a
    /// Food Crate on lock 43 — whose only key, `Open Kneeling`, is in column 1
    /// — draws **no requirement line at all** in the reference, and a reader
    /// that searched the eight columns for one wrote "Requires Open Kneeling"
    /// under the name. That is the bug this exists to stop.
    ///
    /// [`Self::skill_lock`] is the *other* question — what a pointer should be
    /// a picture of — and it deliberately does search all eight.
    pub fn first_slot(&self, lock_id: u32) -> Option<Key> {
        self.keys(lock_id).iter().copied().find(|key| key.slot == 0)
    }

    /// …and the first **item** it will take, for the "Requires <key>" line on a
    /// door that wants one.
    pub fn item_lock(&self, lock_id: u32) -> Option<u32> {
        self.keys(lock_id).iter().find_map(|key| match key.kind {
            KeyKind::Item(entry) => Some(entry),
            KeyKind::Skill { .. } => None,
        })
    }

    /// What `LockType.dbc` calls one of its rows — "Mining", "Herbalism",
    /// "Pick Lock" — for the tooltip's `LOCKED_WITH_SPELL` line.
    pub fn lock_type_name(&self, lock_type: u32) -> Option<&str> {
        self.names.get(&lock_type).map(String::as_str)
    }

    /// How many rows each table gave up, for `vale objects` to report.
    pub fn counts(&self) -> (usize, usize) {
        (self.locks.len(), self.names.len())
    }

    /// One lock, without the file — for the rule tests in
    /// [`crate::look::object`], which are about the *decision* rather than
    /// about the parse and would otherwise each have to synthesise a DBC.
    #[cfg(test)]
    pub(crate) fn with_lock(mut self, lock_id: u32, keys: Vec<Key>) -> Locks {
        self.locks.insert(lock_id, keys);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same, with the fourth run of eight filled in.
    fn lock_dbc_with_actions(rows: &[(u32, [(u32, u32, u32); SLOTS], [u32; SLOTS])]) -> Vec<u8> {
        let fields = 1 + SLOTS * 4;
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for (id, slots, actions) in rows {
            out.extend_from_slice(&id.to_le_bytes());
            for (kind, _, _) in slots {
                out.extend_from_slice(&kind.to_le_bytes());
            }
            for (_, index, _) in slots {
                out.extend_from_slice(&index.to_le_bytes());
            }
            for (_, _, rank) in slots {
                out.extend_from_slice(&rank.to_le_bytes());
            }
            for action in actions {
                out.extend_from_slice(&action.to_le_bytes());
            }
        }
        out.push(0);
        out
    }

    /// `Lock.dbc`'s shape, laid out the way the file lays it out: one id, then
    /// four runs of eight.
    fn lock_dbc(rows: &[(u32, [(u32, u32, u32); SLOTS])]) -> Vec<u8> {
        let fields = 1 + SLOTS * 4;
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for (id, slots) in rows {
            out.extend_from_slice(&id.to_le_bytes());
            for (kind, _, _) in slots {
                out.extend_from_slice(&kind.to_le_bytes());
            }
            for (_, index, _) in slots {
                out.extend_from_slice(&index.to_le_bytes());
            }
            for (_, _, rank) in slots {
                out.extend_from_slice(&rank.to_le_bytes());
            }
            for _ in slots {
                out.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        out.push(0);
        out
    }

    /// **The eight slots are columns and reading them as records is the bug
    /// this pins.** A lock keyed first on an item and second on Mining must
    /// answer Mining to [`Locks::skill_lock`]; a reader that took field 1 and
    /// its two neighbours would answer with the item's entry read as a lock
    /// type, which is a number in the right range and a plausible wrong picture.
    #[test]
    fn a_rows_eight_slots_are_columns_rather_than_records() {
        let raw = lock_dbc(&[(
            41,
            [
                (KEY_ITEM, 1234, 0),
                (KEY_SKILL, lock_type::MINING, 125),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
                (0, 0, 0),
            ],
        )]);
        let locks = Locks::parse(&raw, &[]);
        assert_eq!(
            locks.keys(41).len(),
            2,
            "both filled slots, and none of the six empty ones"
        );
        assert_eq!(locks.item_lock(41), Some(1234));
        assert_eq!(
            locks.skill_lock(41),
            Some((lock_type::MINING, 125)),
            "the skill is found past the item, which is what a pointer is about"
        );
        assert!(locks.keys(0).is_empty(), "lock id 0 is 'not locked'");
        assert!(
            locks.keys(999).is_empty(),
            "and an id the file does not carry is the same"
        );
    }

    /// **Column 0 is empty on a great many rows and that is what the tooltip
    /// reads**, so the two questions must not answer the same thing.
    ///
    /// The row is `Lock.dbc` 43 as the shipped file has it — the one a Food
    /// Crate carries: nothing in column 0, `Open Kneeling` in column 1. The
    /// reference draws no requirement line for it at all, and the report this
    /// pins was a plate reading "Food Crate / Requires Open Kneeling".
    #[test]
    fn column_zero_is_a_different_question_from_the_first_key() {
        let mut slots = [(0u32, 0u32, 0u32); SLOTS];
        slots[1] = (KEY_SKILL, 13, 0);
        let locks = Locks::parse(&lock_dbc(&[(43, slots)]), &[]);
        assert_eq!(
            locks.skill_lock(43),
            Some((13, 0)),
            "the pointer searches all eight, and finds it"
        );
        assert_eq!(
            locks.first_slot(43),
            None,
            "…and the plate reads column 0, which is empty — so it says nothing"
        );
        // …and a row that *does* fill column 0 answers both.
        let mut slots = [(0u32, 0u32, 0u32); SLOTS];
        slots[0] = (KEY_SKILL, lock_type::MINING, 125);
        let locks = Locks::parse(&lock_dbc(&[(41, slots)]), &[]);
        assert_eq!(
            locks.first_slot(41).map(|key| key.kind),
            Some(KeyKind::Skill { lock_type: lock_type::MINING, rank: 125 })
        );
    }

    /// **The action column decides whether a way in applies at all**, and the
    /// two shapes the shipped file uses are a strongbox and a door.
    #[test]
    fn a_ways_action_is_checked_against_the_state_it_is_in() {
        const ACTIVE: u8 = 0;
        const READY: u8 = 1;
        // Lock 2, the beginner's strongbox: pick it while it is locked, open it
        // once it is not.
        assert!(action_applies(1, READY, true), "pick a locked, shut box");
        assert!(!action_applies(1, READY, false), "…and not one already picked");
        assert!(action_applies(0, READY, false), "open an unlocked, shut box");
        assert!(!action_applies(0, READY, true), "…and not a locked one");
        // Lock 77, a door: open it when it is shut, close it when it is open.
        assert!(action_applies(0, READY, false));
        assert!(!action_applies(0, ACTIVE, false));
        assert!(action_applies(2, ACTIVE, false));
        assert!(!action_applies(2, READY, false));
    }

    /// **The action column is read, and it is the fourth run of eight.** A
    /// reader off by one run would take the *rank* for an action, which for an
    /// ore vein is 125 — an action value that falls into the catch-all arm and
    /// silently applies to everything.
    #[test]
    fn the_action_is_the_fourth_run_and_not_the_third() {
        let mut slots = [(0u32, 0u32, 0u32); SLOTS];
        slots[0] = (KEY_SKILL, lock_type::PICK_LOCK, 25);
        let locks = Locks::parse(&lock_dbc_with_actions(&[(2, slots, [1, 0, 0, 0, 0, 0, 0, 0])]), &[]);
        let key = locks.first_slot(2).expect("column 0 is filled");
        assert_eq!(key.action, 1, "Unlock, out of the run after the ranks");
        assert_eq!(key.slot, 0);
        assert_eq!(key.kind, KeyKind::Skill { lock_type: lock_type::PICK_LOCK, rank: 25 });
    }

    /// A missing file is an empty table and not a panic — see [`Locks::parse`].
    #[test]
    fn an_absent_table_answers_nothing_rather_than_failing() {
        let locks = Locks::parse(&[], &[]);
        assert_eq!(locks.counts(), (0, 0));
        assert_eq!(locks.skill_lock(41), None);
        assert_eq!(locks.lock_type_name(lock_type::MINING), None);
    }
}
