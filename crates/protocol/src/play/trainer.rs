//! **The trainer** — what an NPC will teach you, what it costs, and buying one.
//!
//! ```text
//! CMSG_TRAINER_LIST       guid        right-click anything with the trainer flag
//!   -> SMSG_TRAINER_LIST              a type, one row per service, a greeting
//! CMSG_TRAINER_BUY_SPELL  guid, spell press Train
//!   -> SMSG_TRAINER_BUY_SUCCEEDED     …and the spell arrives as an ordinary
//!      / SMSG_TRAINER_BUY_FAILED      `SMSG_LEARNED_SPELL`
//! ```
//!
//! There is **no close opcode**, exactly as there is none for the gossip
//! window: `CloseTrainer()` zeroes the module's guid and raises
//! `TRAINER_CLOSED` locally, and the server learns when the next hello arrives.
//!
//! ## A row is 38 bytes and its fields do not read the way they are named
//!
//! `SendTrainerSpellHelper` writes eleven fields per service, and three of them
//! are worth stating outright because a plausible misreading of each renders as
//! a working-looking window:
//!
//! * **the spell id is the *teaching* spell**, not the spell taught. It is what
//!   `CMSG_TRAINER_BUY_SPELL` echoes back and what the client looks the row's
//!   name and rank up under: it takes `record[0]` straight into `Spell.dbc`
//!   and reads field 120 (`Name`) and field 129 (`Rank`). The client never resolves the trigger for either.
//! * **the required level is a single byte** sitting between two `u32` runs, so
//!   a parser that reads it as a word is three bytes out for the rest of the
//!   row and every service after it.
//! * **the two "profession" words are a cost, not a flag.** `GetTrainerServiceCost`
//!   returns all three of `cost`, `cpCost1`, `cpCost2` as numbers,
//!   and `ClassTrainer_SetSelection` compares the last two against
//!   `UnitCharacterPoints("player")` — so they are talent/profession points, and
//!   a nonzero second one is what puts the profession confirmation box up.
//!
//! ## The state byte is three values and the fourth is the client's
//!
//! vmangos' `TrainerSpellState` is `GREEN = 0`, `RED = 1`, `GRAY = 2`, and the
//! client turns each into one of the three words `GetTrainerServiceInfo` answers
//! with — `available` for 0, `used` for 2 and `unavailable` for anything
//! else. The fourth word, `header`, is not on
//! the wire at all: it is the client's own grouping row (`vale_assets::tables::trainer`).
//!
//! **Only a green service may be bought.** The client tests the state byte
//! against zero *before* building the packet and returns silently otherwise, so
//! a red row's Train button sends nothing. That is the client's own gate, not
//! the server's — vmangos refuses it too, with `TRAIN_FAIL_NOT_ENOUGH_SKILL`.

use crate::bytes::{Reader, Writer};

/// **What kind of trainer this is** — the word before the row list, and the one
/// thing that changes how the whole window is grouped.
///
/// `IsTradeskillTrainer()` is `type == 2` and `IsTalentTrainer()` is
/// `type == 1` *or* no trainer at all; the client has no
/// predicate for the other two, so 0 and 3 are named from vmangos' own
/// `npc_trainer` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrainerType {
    /// Spells, grouped under their `SkillLine` headers.
    #[default]
    Class,
    /// The one the client names: talents, with everything already known folded
    /// under a `KNOWN_TALENTS_HEADER` row.
    Talent,
    /// Professions and recipes, grouped as *step* against *learn* rather than
    /// by skill line.
    Tradeskill,
    /// A hunter's pet's abilities. Grouped like [`TrainerType::Class`], but off
    /// the *pet's* skill lines rather than the character's.
    Pet,
}

impl TrainerType {
    /// The wire's word. **An unknown value is [`TrainerType::Class`]**, which is
    /// the safe direction: a class trainer is the plain grouping, where reading
    /// an unknown as tradeskill would put every row under a Step/Learn header
    /// that means nothing.
    pub fn of(word: u32) -> TrainerType {
        match word {
            1 => TrainerType::Talent,
            2 => TrainerType::Tradeskill,
            3 => TrainerType::Pet,
            _ => TrainerType::Class,
        }
    }
}

/// What the character may do about one service — vmangos' `TrainerSpellState`,
/// and the three words the interface asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainerState {
    /// Green: learnable now.
    Available,
    /// Red: a level, a skill or a prerequisite is missing.
    Unavailable,
    /// Grey: already known.
    Used,
}

impl TrainerState {
    /// **Anything that is not 0 or 2 is red**, which is the client's own
    /// rule rather than a lenient reading here.
    pub fn of(byte: u8) -> TrainerState {
        match byte {
            0 => TrainerState::Available,
            2 => TrainerState::Used,
            _ => TrainerState::Unavailable,
        }
    }

    /// **The byte back**, which is not decoration: the type filter's bit index
    /// *is* this number, so the ordering of the three
    /// is load-bearing wherever the mask is. See `vale_assets::tables::trainer`,
    /// which owns that mask — and which carries the three *words* the interface
    /// asks for, because those are the client's own strings rather
    /// than anything on the wire.
    pub fn byte(self) -> u8 {
        match self {
            TrainerState::Available => 0,
            TrainerState::Unavailable => 1,
            TrainerState::Used => 2,
        }
    }
}

/// One thing a trainer will teach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainerService {
    /// **The teaching spell**, which is both the wire's id and the row the name
    /// and rank are read from. See the module note.
    pub spell: u32,
    pub state: TrainerState,
    /// Copper, the reputation discount already applied by the server.
    pub cost: u32,
    /// Talent/profession points this costs — `GetTrainerServiceCost`'s second
    /// and third returns. The second being nonzero is what makes the Train
    /// button open the profession confirmation box.
    pub point_cost: (u32, u32),
    /// `GetTrainerServiceLevelReq`. A **byte** on the wire.
    pub req_level: u8,
    /// `GetTrainerServiceSkillReq` — a `SkillLine` id and a rank in it, both 0
    /// when there is no such requirement.
    pub req_skill: u32,
    pub req_skill_value: u32,
    /// Up to three prerequisite spells, zero-padded — the chain nodes
    /// `GetTrainerServiceNumAbilityReq` counts by testing each for `> 0`.
    pub req_spells: [u32; 3],
}

/// `SMSG_TRAINER_LIST`, whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrainerList {
    pub guid: u64,
    pub kind: TrainerType,
    pub services: Vec<TrainerService>,
    /// What `GetTrainerGreetingText()` answers — the words at the top of the
    /// window, written **after** the rows rather than before them.
    pub greeting: String,
}

/// Why a train did not happen — `SMSG_TRAINER_BUY_FAILED`'s **`u32`**, which is
/// vmangos' `TrainingFailureReason`. Wider than every other refusal code in the
/// game, which are bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainFailure {
    Unavailable,
    NotEnoughMoney,
    NotEnoughSkill,
}

impl TrainFailure {
    pub fn of(code: u32) -> Option<TrainFailure> {
        Some(match code {
            0 => TrainFailure::Unavailable,
            1 => TrainFailure::NotEnoughMoney,
            2 => TrainFailure::NotEnoughSkill,
            _ => return None,
        })
    }

    /// The `GlobalStrings.lua` key for what to show the player, or `None` for a
    /// reason the file has no sentence for.
    ///
    /// **A reading, stated as one.** The codes are vmangos' enum and the money
    /// one is unambiguous; the other two have no dedicated trainer line in
    /// 5875's strings — `ERR_TRAIN_FAIL` does not exist — so they say nothing,
    /// which is this client's rule for every refusal it cannot name.
    pub fn key(self) -> Option<&'static str> {
        match self {
            TrainFailure::NotEnoughMoney => Some("ERR_NOT_ENOUGH_MONEY"),
            _ => None,
        }
    }
}

/// How many bytes one service occupies — eleven fields, nine of them words.
/// Named because `SendTrainerList` sizes its own packet with it.
const SERVICE_BYTES: usize = 4 + 1 + 4 + 4 + 4 + 1 + 4 + 4 + 4 + 4 + 4;

// --- parsing ----------------------------------------------------------------

/// `SMSG_TRAINER_LIST`.
///
/// **The greeting is after the rows**, so a row-count that overruns the body
/// loses the words too — which is why the count is clamped rather than trusted:
/// a service list is at most a class's whole spellbook.
pub fn parse_trainer_list(body: &[u8]) -> Option<TrainerList> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 4) {
        return None;
    }
    let guid = r.u64();
    let kind = TrainerType::of(r.u32());
    let count = r.u32().min(512);
    let mut services = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if !r.has(SERVICE_BYTES) {
            break;
        }
        let spell = r.u32();
        let state = TrainerState::of(r.u8());
        let cost = r.u32();
        let point_cost = (r.u32(), r.u32());
        let req_level = r.u8();
        let req_skill = r.u32();
        let req_skill_value = r.u32();
        services.push(TrainerService {
            spell,
            state,
            cost,
            point_cost,
            req_level,
            req_skill,
            req_skill_value,
            req_spells: [r.u32(), r.u32(), r.u32()],
        });
    }
    Some(TrainerList {
        guid,
        kind,
        services,
        greeting: r.cstring(),
    })
}

/// `SMSG_TRAINER_BUY_SUCCEEDED` — the trainer and the **service** spell, which
/// is the id that was sent rather than the one that was learned.
pub fn parse_trainer_bought(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    Some((r.u64(), r.u32()))
}

/// `SMSG_TRAINER_BUY_FAILED` — trainer, service spell, reason.
pub fn parse_trainer_buy_failed(body: &[u8]) -> Option<(u64, u32, Option<TrainFailure>)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 4) {
        return None;
    }
    let guid = r.u64();
    let spell = r.u32();
    Some((guid, spell, TrainFailure::of(r.u32())))
}

// --- what we send -----------------------------------------------------------

/// `CMSG_TRAINER_BUY_SPELL` — the trainer and the **service** spell id, exactly
/// as the list gave it (`record[0]`, untouched).
///
/// `CMSG_TRAINER_LIST` is a bare guid and goes out through
/// [`crate::play::gossip::guid_body`], which every hello in this family shares.
pub fn buy_spell_body(guid: u64, spell: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.u32(spell);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(w: &mut Writer, spell: u32, state: u8, cost: u32, level: u8) {
        w.u32(spell);
        w.u8(state);
        w.u32(cost);
        w.u32(0);
        w.u32(0);
        w.u8(level);
        w.u32(6); // reqSkill — Frost
        w.u32(1); // reqSkillValue
        w.u32(0);
        w.u32(0);
        w.u32(0);
    }

    fn list(rows: &[(u32, u8, u32, u8)]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(0x77);
        w.u32(0);
        w.u32(rows.len() as u32);
        for (spell, state, cost, level) in rows {
            row(&mut w, *spell, *state, *cost, *level);
        }
        w.bytes(b"Greetings, $N.\0");
        w.buf
    }

    /// The list, whole — and **the greeting only survives if every row is the
    /// right width**, which is what makes it the check that the byte-wide level
    /// field was read as a byte.
    #[test]
    fn a_trainer_list_carries_a_type_its_rows_and_a_greeting() {
        let parsed =
            parse_trainer_list(&list(&[(145, 0, 1000, 8), (3140, 1, 20000, 22)])).expect("a list");
        assert_eq!(parsed.guid, 0x77);
        assert_eq!(parsed.kind, TrainerType::Class);
        assert_eq!(parsed.services.len(), 2);
        assert_eq!(parsed.services[0].spell, 145);
        assert_eq!(parsed.services[0].state, TrainerState::Available);
        assert_eq!(parsed.services[0].cost, 1000);
        assert_eq!(parsed.services[0].req_level, 8);
        assert_eq!(parsed.services[0].req_skill, 6);
        assert_eq!(parsed.services[1].state, TrainerState::Unavailable);
        assert_eq!(parsed.services[1].req_level, 22);
        assert_eq!(
            parsed.greeting, "Greetings, $N.",
            "the words are after the rows, so this is the row-width check"
        );
    }

    /// The three states, **anything else is red** — the client's own rule
    /// rather than a lenient fallback — and the byte round-trips, which is what
    /// the filter mask is indexed by.
    #[test]
    fn the_state_byte_is_three_values_and_the_rest_are_red() {
        for byte in [0u8, 1, 2] {
            assert_eq!(TrainerState::of(byte).byte(), byte);
        }
        assert_eq!(TrainerState::of(0), TrainerState::Available);
        assert_eq!(TrainerState::of(2), TrainerState::Used);
        assert_eq!(TrainerState::of(10), TrainerState::Unavailable);
        assert_eq!(TrainerState::of(255), TrainerState::Unavailable);
    }

    /// The four trainer types, and an unknown one degrading to the plain
    /// grouping rather than to Step/Learn headers that mean nothing.
    #[test]
    fn the_trainer_type_names_four_and_degrades_to_class() {
        assert_eq!(TrainerType::of(0), TrainerType::Class);
        assert_eq!(TrainerType::of(1), TrainerType::Talent);
        assert_eq!(TrainerType::of(2), TrainerType::Tradeskill);
        assert_eq!(TrainerType::of(3), TrainerType::Pet);
        assert_eq!(TrainerType::of(99), TrainerType::Class);
    }

    /// A body whose second row never arrived keeps the first rather than
    /// discarding the window — the same tolerance every parser here has, and
    /// the greeting is simply absent rather than read out of the wreckage.
    #[test]
    fn a_truncated_row_stops_cleanly() {
        let whole = list(&[(145, 0, 1000, 8), (3140, 0, 20000, 22)]);
        let parsed = parse_trainer_list(&whole[..8 + 4 + 4 + SERVICE_BYTES]).expect("a list");
        assert_eq!(parsed.services.len(), 1);
        assert_eq!(parsed.services[0].spell, 145);
        assert!(parsed.greeting.is_empty());
    }

    /// The two answers and the one body.
    #[test]
    fn the_verb_and_its_answers_round_trip() {
        let mut w = Writer::new();
        w.u64(0x77);
        w.u32(145);
        assert_eq!(parse_trainer_bought(&w.buf), Some((0x77, 145)));
        let mut w = Writer::new();
        w.u64(0x77);
        w.u32(145);
        w.u32(1);
        assert_eq!(
            parse_trainer_buy_failed(&w.buf),
            Some((0x77, 145, Some(TrainFailure::NotEnoughMoney)))
        );
        assert_eq!(TrainFailure::of(3), None, "the numbering stops at two");
        let body = buy_spell_body(9, 145);
        assert_eq!(&body[0..8], &9u64.to_le_bytes());
        assert_eq!(&body[8..12], &145u32.to_le_bytes());
    }
}
