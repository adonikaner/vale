//! **The pet: its bar, its mood, its name, and the ten things you can tell it
//! to do.**
//!
//! The largest single gap in this client's protocol coverage was pets — twelve
//! unread opcodes, and with them two of the nine classes. What is here is the
//! half that crosses the wire; `crate::state::objects::PetState` is where it
//! lands, and the renderer's own half is `client/src/game/combat/pet.rs`.
//!
//! ## `SMSG_PET_SPELLS` is the whole panel in one packet
//!
//! ```text
//! u64  petGuid          raw, and zero means "the bar goes" — Player::RemovePetActionBar
//! i32  duration         a possess or charm's remaining time; 0 for an ordinary pet
//! u8   reactState       REACT_PASSIVE / DEFENSIVE / AGGRESSIVE
//! u8   commandState     COMMAND_STAY / FOLLOW / ATTACK / DISMISS
//! u8   0
//! u8   enabledFlags     0 enabled, 8 disabled
//! 10 x u32              the action bar, packed: action | (activeState << 24)
//! u8   spellCount
//! n  x u32              the spellbook, packed the same way
//! u16  cooldownCount
//! n  x (u32 spell, u16 category, u32 cooldownMs, u32 categoryCooldownMs)
//! ```
//!
//! **The three sends have the same shape and different middles.**
//! `PetSpellInitialize` writes the four state bytes above;
//! `PossessSpellInitialize` writes `int32 duration` and then a *whole* `uint32`
//! zero where those four bytes are, which is the same sixteen bytes and no state
//! at all; `CharmSpellInitialize` writes the state bytes and a spell list built
//! from `CharmSpellEntry` rather than from the pet's book. One reader covers all
//! three, which is what the client does.
//!
//! ## The packed word is the same one the action bar uses, with a different
//! ## high byte
//!
//! `MAKE_UNIT_ACTION_BUTTON(action, type)` is `action | (type << 24)`, and the
//! type is not a small enum: [`active_state`]'s five values are bit patterns
//! (`0x81` castable, `0xC1` castable and auto-casting, `0x01` passive, `0x07` a
//! command, `0x06` a reaction). A reader that treats the high byte as an index
//! gets a command slot where a spell belongs.
//!
//! Source: vmangos `Objects/Player.cpp` (`PetSpellInitialize`),
//! `Objects/Unit.cpp` (`WritePetSpellsCooldown`, `SendPetActionFeedback`),
//! `Server/Packets/Pet.cpp` and `Objects/UnitDefines.h`.

use crate::bytes::{Reader, Writer};

/// How many slots a pet's action bar has — `MAX_UNIT_ACTION_BAR_INDEX`.
///
/// Ten, and the middle five are the pet's *spells*
/// (`ACTION_BAR_INDEX_PET_SPELL_START`..`_END` is 3..7); the outer five are the
/// command and reaction buttons the server fills in for it.
pub const PET_BAR_SLOTS: usize = 10;

/// **What a bar slot's high byte means** — `ActiveStates` in
/// `Objects/UnitDefines.h`, and every value is a bit pattern rather than an
/// index.
pub mod active_state {
    /// A passive spell: on the bar, never pressed.
    pub const PASSIVE: u8 = 0x01;
    /// `ACT_REACTION` — one of the three react buttons.
    pub const REACTION: u8 = 0x06;
    /// `ACT_COMMAND` — one of the command buttons (attack, follow, stay).
    pub const COMMAND: u8 = 0x07;
    /// `ACT_DISABLED` — castable, auto-cast off.
    pub const DISABLED: u8 = 0x81;
    /// `ACT_ENABLED` — castable, auto-cast on.
    pub const ENABLED: u8 = 0xC1;

    /// **Is this slot a spell rather than a command?** — vmangos'
    /// `UnitActionBarEntry::IsActionBarForSpell`, which is the test that decides
    /// whether the slot's action is a spell id at all.
    pub fn is_spell(state: u8) -> bool {
        matches!(state, DISABLED | ENABLED | PASSIVE)
    }

    /// …and does it auto-cast? Only the one value says so.
    pub fn auto_casts(state: u8) -> bool {
        state == ENABLED
    }
}

/// `ReactStates` — how much the pet decides for itself.
pub mod react_state {
    pub const PASSIVE: u8 = 0;
    pub const DEFENSIVE: u8 = 1;
    pub const AGGRESSIVE: u8 = 2;
}

/// `CommandStates` — the last order it was given.
pub mod command_state {
    pub const STAY: u8 = 0;
    pub const FOLLOW: u8 = 1;
    pub const ATTACK: u8 = 2;
    pub const DISMISS: u8 = 3;
}

/// **The action ids the command and reaction slots carry**, which are the same
/// numbers as [`command_state`] and [`react_state`] — the slot's *type* is what
/// says which of the two tables to read it in.
///
/// This is worth stating because 0, 1 and 2 mean three different things
/// depending on that byte: with `ACT_COMMAND` they are stay, follow and attack;
/// with `ACT_REACTION` they are passive, defensive and aggressive.
pub const PET_ACTION_DISMISS: u32 = command_state::DISMISS as u32;

/// One slot of the bar, unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PetAction {
    /// A spell id, a [`command_state`] or a [`react_state`] — see [`Self::state`].
    pub action: u32,
    /// The high byte: one of [`active_state`]'s five.
    pub state: u8,
}

impl PetAction {
    /// The packed `uint32` the wire carries, which is also what
    /// `CMSG_PET_ACTION` sends back.
    pub fn packed(self) -> u32 {
        (self.action & 0x00FF_FFFF) | (u32::from(self.state) << 24)
    }

    pub fn unpack(packed: u32) -> PetAction {
        PetAction {
            action: packed & 0x00FF_FFFF,
            state: (packed >> 24) as u8,
        }
    }

    /// **Is there anything in this slot at all?** — which is the action alone,
    /// and deliberately not the state byte with it.
    ///
    /// `UnitActionBarEntry`'s constructor writes `ACT_DISABLED << 24` with a
    /// zero action, so the server's own empty slot is `0x81000000`. But the
    /// *client* empties a slot with [`pet_slot_emptied`], which keeps the
    /// slot's own state byte — so an emptied auto-casting spell is
    /// `0xC1000000`, and it comes back from the server that way too. A test
    /// that also required the state byte would read that as occupied and draw a
    /// button for a spell id of zero.
    pub fn is_empty(self) -> bool {
        self.action == 0
    }

    pub fn is_spell(self) -> bool {
        self.action != 0 && active_state::is_spell(self.state)
    }
}

/// One entry of the pet's own spellbook, which is the same packed word.
pub type PetSpell = PetAction;

/// One cooldown, as `WritePetSpellsCooldown` writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PetCooldown {
    pub spell_id: u32,
    pub category: u16,
    pub remaining_ms: u32,
    /// **The category's own cooldown, with `0x8000000` set for a permanent
    /// one** — vmangos ors the bit in rather than sending a duration, and a
    /// reader that treats it as milliseconds reports 37 hours.
    pub category_ms: u32,
}

/// The bit `WritePetSpellsCooldown` ors into `categoryCooldown` for a cooldown
/// that never runs out.
pub const COOLDOWN_PERMANENT: u32 = 0x0800_0000;

/// `SMSG_PET_SPELLS` — everything the pet panel is drawn from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PetSpells {
    /// **Zero means the bar goes.** `Player::RemovePetActionBar` sends this
    /// packet with nothing but eight zero bytes.
    pub pet: u64,
    /// A charm or possess's remaining milliseconds; 0 for an ordinary pet.
    pub duration_ms: i32,
    pub react: u8,
    pub command: u8,
    /// `0` enabled, `8` disabled — a pet that may not be commanded at all.
    pub flags: u8,
    pub bar: [PetAction; PET_BAR_SLOTS],
    pub spells: Vec<PetSpell>,
    pub cooldowns: Vec<PetCooldown>,
}

impl PetSpells {
    /// Whether this packet is the dismissal rather than a bar.
    pub fn is_dismissal(&self) -> bool {
        self.pet == 0
    }
}

/// **The whole panel**, or `None` for a body too short to carry the header.
///
/// A short *tail* is not refused: the bar, the spell list and the cooldowns each
/// stop where the body does. The three senders write different amounts after the
/// header and the dismissal writes nothing at all, so "the packet ended" is a
/// real shape rather than damage.
pub fn parse_pet_spells(body: &[u8]) -> Option<PetSpells> {
    let mut r = Reader::new(body);
    if !r.has(8) {
        return None;
    }
    let pet = r.u64();
    let mut spells = PetSpells {
        pet,
        ..PetSpells::default()
    };
    if pet == 0 || !r.has(8) {
        // `RemovePetActionBar` — eight bytes and no more.
        return Some(spells);
    }
    spells.duration_ms = r.u32() as i32;
    spells.react = r.u8();
    spells.command = r.u8();
    let _reserved = r.u8();
    spells.flags = r.u8();
    for slot in &mut spells.bar {
        if !r.has(4) {
            break;
        }
        *slot = PetAction::unpack(r.u32());
    }
    if !r.has(1) {
        return Some(spells);
    }
    let count = r.u8();
    for _ in 0..count {
        if !r.has(4) {
            break;
        }
        spells.spells.push(PetAction::unpack(r.u32()));
    }
    // **A `u16` count, not a `u8`** — the one width in this packet that differs
    // from the list above it.
    if !r.has(2) {
        return Some(spells);
    }
    let cooldowns = r.u16();
    for _ in 0..cooldowns {
        if !r.has(14) {
            break;
        }
        spells.cooldowns.push(PetCooldown {
            spell_id: r.u32(),
            category: r.u16(),
            remaining_ms: r.u32(),
            category_ms: r.u32(),
        });
    }
    Some(spells)
}

/// `SMSG_PET_MODE` — the same four state bytes on their own, for when only the
/// mood changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PetMode {
    pub pet: u64,
    pub react: u8,
    pub command: u8,
    pub flags: u8,
}

pub fn parse_pet_mode(body: &[u8]) -> Option<PetMode> {
    let mut r = Reader::new(body);
    if !r.has(12) {
        return None;
    }
    let pet = r.u64();
    let react = r.u8();
    let command = r.u8();
    let _reserved = r.u8();
    let flags = r.u8();
    Some(PetMode {
        pet,
        react,
        command,
        flags,
    })
}

/// `SMSG_PET_NAME_QUERY_RESPONSE` — **the one place a pet's name comes from.**
///
/// A pet is a creature and a creature's name comes from
/// `SMSG_CREATURE_QUERY_RESPONSE`, which for a pet answers the *species*
/// ("Wolf"). What the player called it is here, keyed by
/// `UNIT_FIELD_PETNUMBER` rather than by guid — which is what makes the name
/// survive being dismissed and called back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetName {
    pub pet_number: u32,
    pub name: String,
    /// `UNIT_FIELD_PET_NAME_TIMESTAMP`, which is what the *client* compares
    /// against to decide whether to ask again.
    pub timestamp: u32,
}

pub fn parse_pet_name(body: &[u8]) -> Option<PetName> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let pet_number = r.u32();
    let name = r.cstring();
    if !r.has(4) {
        return None;
    }
    Some(PetName {
        pet_number,
        name,
        timestamp: r.u32(),
    })
}

/// `SMSG_PET_ACTION_FEEDBACK` — **one byte, and the whole packet is which
/// sentence to say.**
///
/// The same shape as the five attack-swing refusals: an opcode with no body
/// whose content is a `GlobalStrings.lua` key.
pub mod feedback {
    /// `FEEDBACK_PET_DEAD` — nothing to attack, said as "Your pet is dead."
    pub const PET_DEAD: u8 = 1;
    /// `FEEDBACK_NOTHING_TO_ATT`.
    pub const NOTHING_TO_ATTACK: u8 = 2;
    /// `FEEDBACK_CANT_ATT_TARGET`.
    pub const CANT_ATTACK_TARGET: u8 = 3;
    /// `FEEDBACK_NO_PATH_TO`.
    pub const NO_PATH_TO: u8 = 4;

    /// **What each of the four says** — the client's whole handler is: read
    /// the byte, and pick one of four error rows by it:
    ///
    /// ```text
    ///   1 -> row 336   ERR_PET_SPELL_DEAD        "Your pet is dead."
    ///   2 -> row 160   ERR_NO_ATTACK_TARGET
    ///   3 -> row 161   ERR_INVALID_ATTACK_TARGET
    ///   4 -> row 337   ERR_PET_SPELL_NOPATH
    ///   anything else  nothing is said
    /// ```
    ///
    /// **Two of the four are not pet keys at all** — 160 and 161 are the same
    /// rows a swing at nothing uses — which is the sort of thing only the
    /// client says: a reader inventing four `ERR_PET_*` keys from the enum's
    /// names would have shipped two that do not exist.
    ///
    /// And `ERR_PET_SPELL_NOPATH` is one of the keys the shipped
    /// `GlobalStrings.lua` does **not** carry, so the fourth says nothing on
    /// screen. That is the reference's own behaviour rather than a gap here.
    pub fn key(message: u8) -> Option<&'static str> {
        Some(match message {
            PET_DEAD => "ERR_PET_SPELL_DEAD",
            NOTHING_TO_ATTACK => "ERR_NO_ATTACK_TARGET",
            CANT_ATTACK_TARGET => "ERR_INVALID_ATTACK_TARGET",
            NO_PATH_TO => "ERR_PET_SPELL_NOPATH",
            _ => return None,
        })
    }
}

/// **Why a tame failed** — the client indexes `reason - 1` into a twelve-way
/// table whose default is `PETTAME_UNKNOWNERROR`.
///
/// The sentence is then substituted into `ERR_TAME_FAILED`, which is the whole
/// of `"%s."` — so the key below is the message and the row it lands in only
/// adds the full stop.
pub mod tame_failure {
    /// The eleven reasons, in the order the client's table gives them.
    pub const KEYS: [&str; 11] = [
        "PETTAME_INVALIDCREATURE",
        "PETTAME_TOOMANY",
        "PETTAME_CREATUREALREADYOWNED",
        "PETTAME_NOTTAMEABLE",
        "PETTAME_ANOTHERSUMMONACTIVE",
        "PETTAME_UNITSCANTTAME",
        "PETTAME_NOPETAVAILABLE",
        "PETTAME_INTERNALERROR",
        "PETTAME_TOOHIGHLEVEL",
        "PETTAME_DEAD",
        "PETTAME_NOTDEAD",
    ];

    /// **…and the default, which is a real answer rather than a fallback.**
    /// Any reason outside 1..11 — including
    /// zero — says "Unknown taming error" rather than nothing.
    pub const UNKNOWN: &str = "PETTAME_UNKNOWNERROR";

    /// The key for a reason byte. Never `None`: the reference always says
    /// something.
    pub fn key(reason: u8) -> &'static str {
        reason
            .checked_sub(1)
            .and_then(|i| KEYS.get(usize::from(i)))
            .copied()
            .unwrap_or(UNKNOWN)
    }
}

pub fn parse_pet_feedback(body: &[u8]) -> Option<u8> {
    let mut r = Reader::new(body);
    r.has(1).then(|| r.u8())
}

/// `SMSG_PET_CAST_FAILED` — the pet's own half of `SMSG_CAST_RESULT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PetCastFailed {
    pub spell_id: u32,
    /// `SPELL_RESULT_STATUS_FAIL`, which is the only value vmangos sends.
    pub status: u8,
    /// A `SpellCastResult`, indexing the same 146-entry table
    /// `SMSG_CAST_RESULT` does.
    pub reason: u8,
}

pub fn parse_pet_cast_failed(body: &[u8]) -> Option<PetCastFailed> {
    let mut r = Reader::new(body);
    if !r.has(6) {
        return None;
    }
    Some(PetCastFailed {
        spell_id: r.u32(),
        status: r.u8(),
        reason: r.u8(),
    })
}

/// `SMSG_PET_TAME_FAILURE` — one byte, a `PetTameFailureReason`.
pub fn parse_pet_tame_failure(body: &[u8]) -> Option<u8> {
    let mut r = Reader::new(body);
    r.has(1).then(|| r.u8())
}

/// **What `SMSG_PET_ACTION_SOUND`'s value means** — vmangos' `PetTalk`, and
/// the two columns the client routes them to.
///
/// The client's handler maps the wire value through the pet's talk channel
/// (wire value + 1 into a table of talk states): 0 plays the
/// `CreatureSoundData` **pet-order** column (`+0x70`, `A_IMP_ORDER`) and 1 the
/// **pet-attack** column (`+0x6c`, `A_IMP_KILL`). Any other value says
/// nothing — states 0 and 4 of that table are the aggro and death barks,
/// reached by the client's own callers rather than by this packet.
///
/// vmangos sends 0 from `HandlePetActionHelper` when a special spell is
/// ordered and 1 from `Unit::Attack`; the four warlock demons are the only
/// rows in 1.12 that state either column.
pub mod pet_talk {
    /// `PET_TALK_SPECIAL_SPELL` — plays the pet-order column.
    pub const SPECIAL_SPELL: u32 = 0;
    /// `PET_TALK_ATTACK` — plays the pet-attack column.
    pub const ATTACK: u32 = 1;
}

/// `SMSG_PET_ACTION_SOUND` — the pet said something. The value is a
/// [`pet_talk`] selector rather than a `SoundEntries.dbc` row.
pub fn parse_pet_action_sound(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(12) {
        return None;
    }
    Some((r.u64(), r.u32()))
}

/// `SMSG_PET_DISMISS_SOUND` — **a model-data id and a place, not a guid**: the
/// pet is already gone by the time this arrives, so the packet carries what a
/// despawned unit no longer can. The client walks
/// `CreatureModelData`'s sound column into `CreatureSoundData` and plays the
/// pet-dismiss column at the position, one yard up. vmangos never sends it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PetDismissSound {
    /// A `CreatureModelData.dbc` row id.
    pub model_id: u32,
    /// Where the pet stood, in the wire's own (x, y, z).
    pub position: [f32; 3],
}

pub fn parse_pet_dismiss_sound(body: &[u8]) -> Option<PetDismissSound> {
    let mut r = Reader::new(body);
    if !r.has(16) {
        return None;
    }
    Some(PetDismissSound {
        model_id: r.u32(),
        position: [r.f32(), r.f32(), r.f32()],
    })
}

/// `SMSG_PET_UNLEARN_CONFIRM` — the guid, and what resetting it costs in copper.
pub fn parse_pet_unlearn_confirm(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(12) {
        return None;
    }
    Some((r.u64(), r.u32()))
}

// --- what this client sends ------------------------------------------------

/// `CMSG_PET_ACTION` — **press a slot.**
///
/// `data` is the slot's own packed word (see [`PetAction::packed`]), which is
/// why the bar is kept as it arrived rather than as three parallel lists: the
/// press sends the number back unchanged.
pub fn pet_action_body(pet: u64, data: u32, target: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(pet).u32(data).u64(target);
    w.buf
}

/// `CMSG_PET_NAME_QUERY` — **the number first and the guid second**, which is
/// the reverse of every other query in this protocol and is what a reader of
/// the wrong order loses the name to.
pub fn pet_name_query_body(pet_number: u32, pet: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(pet_number).u64(pet);
    w.buf
}

/// `CMSG_PET_SET_ACTION` — drag a spell onto a slot.
///
/// vmangos decides how many entries it has **from the packet's own length**:
/// twenty-four bytes is a swap of two slots and sixteen is one. So a single move
/// must be sent as exactly one entry, and a swap as exactly two.
///
/// **The displaced slot comes first**, which is not a convention but the check's
/// requirement — see [`place_on_pet_bar`].
pub fn pet_set_action_body(pet: u64, moves: &[(u32, u32)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(pet);
    for (position, data) in moves.iter().take(2) {
        w.u32(*position).u32(*data);
    }
    w.buf
}

/// **What dropping a word on a pet-bar slot does** — the whole of the client's
/// one function that writes the pet bar and builds the packet from what it
/// wrote.
///
/// It is a *rule* rather than a send because the packet's contents are decided
/// by the state of the bar: the same gesture is one entry or two depending on
/// whether anything had to be pushed out of the way, and the server's own check
/// (`HandlePetSetAction`) will silently drop a packet whose two entries do not
/// describe a consistent swap.
///
/// ## The four things it does, in the client's own order
///
/// 1. **A word dropped on the slot it already occupies is refused** — nothing
///    is sent and nothing moves, which is the gesture that cancels a drag.
/// 2. **A passive spell is refused** (`Attributes & 0x40`). Passed
///    in as `is_passive` because `Spell.dbc` is not this crate's to read.
/// 3. **The same action already on the bar elsewhere is displaced**:
///    the bar is scanned for a slot holding the same action —
///    compared as `word & 0x3fffffff`, so the two auto-cast bits are ignored and
///    a spell does not fail to match itself because its dot is on — and that
///    slot takes the destination's old occupant. This is what makes a drag
///    *within* the bar a swap rather than a duplication.
/// 4. **…and failing that, a command or reaction being sat on is pushed to the
///    first empty spell slot**. The three command and three
///    reaction buttons cannot be removed, only moved, so a drop on one that has
///    nowhere to go is refused outright rather than destroying it.
///
/// The entry for the displaced slot is written **first** and the destination
/// second, which is exactly what vmangos'
/// `move_command` check reads: `actions[0].data` must equal what the server
/// still holds at `actions[1].position`, and the other way round.
///
/// Returns `None` when the drop is refused, and otherwise the entries to send —
/// which are also, applied in order, the bar's new state. `hand_back` is
/// the client function's own return value: the destination's old
/// occupant goes back onto the cursor only when nothing was displaced, because
/// a displacement has already found it a home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetBarWrite {
    /// `(position, data)` in the wire's order — one entry, or two with the
    /// displaced slot first.
    pub moves: Vec<(u32, u32)>,
    /// Whether the destination's old occupant is handed back to the cursor.
    pub hand_back: bool,
}

/// **The empty word a removal writes** — `packed & 0xffff0000`, which keeps
/// the slot's own state byte rather than writing a fixed one.
///
/// So emptying an auto-casting spell writes `0xC1000000` and emptying a passive
/// writes `0x01000000`. A reader that sent one constant for all three would be
/// telling the server the wrong `ActiveStates` for the emptied slot.
pub fn pet_slot_emptied(packed: u32) -> u32 {
    packed & 0xFFFF_0000
}

pub fn place_on_pet_bar(
    bar: &mut [PetAction; PET_BAR_SLOTS],
    position: usize,
    word: u32,
    is_passive: impl Fn(u32) -> bool,
) -> Option<PetBarWrite> {
    if position >= PET_BAR_SLOTS {
        return None;
    }
    // 1. The same word back where it already is.
    if bar[position].packed() == word {
        return None;
    }
    let action = PetAction::unpack(word);
    let is_spell_word = active_state::is_spell(action.state);
    // 2. A passive spell may not be placed.
    if is_spell_word && action.action != 0 && is_passive(action.action & 0xFFFF) {
        return None;
    }

    // 3. The duplicate elsewhere on the bar, ignoring the two auto-cast bits.
    // Skipped for an *empty* spell word, which is what a removal writes and
    // would otherwise match every empty slot.
    let is_removal = is_spell_word && action.action == 0;
    let mut displaced = None;
    if !is_removal {
        const IGNORE_AUTOCAST: u32 = 0x3FFF_FFFF;
        displaced = (0..PET_BAR_SLOTS).find(|&slot| {
            slot != position && bar[slot].packed() & IGNORE_AUTOCAST == word & IGNORE_AUTOCAST
        });
    }
    // 4. …or a command or reaction that has to be got out of the way.
    if displaced.is_none() {
        let sat_on = bar[position].state;
        if matches!(sat_on, active_state::COMMAND | active_state::REACTION) {
            displaced = (0..PET_BAR_SLOTS).find(|&slot| {
                let here = bar[slot];
                !matches!(here.state, active_state::COMMAND | active_state::REACTION)
                    && here.action & 0xFFFF == 0
            });
            // Nowhere to put it: the whole drop is refused rather than
            // destroying a button that cannot be destroyed.
            displaced?;
        }
    }

    let mut moves = Vec::new();
    if let Some(slot) = displaced {
        bar[slot] = bar[position];
        moves.push((slot as u32, bar[slot].packed()));
    }
    bar[position] = action;
    moves.push((position as u32, word));
    Some(PetBarWrite {
        moves,
        hand_back: displaced.is_none(),
    })
}

/// `CMSG_PET_SPELL_AUTOCAST` — the little dot on a pet spell.
pub fn pet_spell_autocast_body(pet: u64, spell_id: u32, on: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(pet).u32(spell_id).u8(u8::from(on));
    w.buf
}

/// `CMSG_PET_STOP_ATTACK`, `CMSG_PET_ABANDON`, `CMSG_PET_UNLEARN` — a guid and
/// nothing else, three opcodes sharing one body.
pub fn pet_guid_body(pet: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(pet);
    w.buf
}

/// `CMSG_PET_RENAME`.
pub fn pet_rename_body(pet: u64, name: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(pet).cstring(name);
    w.buf
}

/// `CMSG_PET_CANCEL_AURA` — cancel one of the pet's own buffs.
pub fn pet_cancel_aura_body(pet: u64, spell_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(pet).u32(spell_id);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The header is sixteen bytes and the three senders disagree about what
    /// four of them mean**, which is why the reader takes them as four bytes
    /// rather than as a struct: `PossessSpellInitialize` writes a whole `uint32`
    /// zero where `PetSpellInitialize` writes react, command, a reserved byte
    /// and the enabled flags.
    #[test]
    fn a_pet_bar_is_read_slot_by_slot_with_its_high_byte_intact() {
        let mut w = Writer::new();
        w.u64(0xF140_0000_0000_0001)
            .u32(0)
            .u8(react_state::DEFENSIVE)
            .u8(command_state::FOLLOW)
            .u8(0)
            .u8(0);
        // Slot 0 is a command, slot 3 an auto-casting spell, slot 4 a passive.
        let slots: [(u32, u8); PET_BAR_SLOTS] = [
            (command_state::ATTACK as u32, active_state::COMMAND),
            (command_state::FOLLOW as u32, active_state::COMMAND),
            (command_state::STAY as u32, active_state::COMMAND),
            (2649, active_state::ENABLED),
            (17253, active_state::PASSIVE),
            (0, active_state::DISABLED),
            (0, active_state::DISABLED),
            (0, active_state::DISABLED),
            (react_state::DEFENSIVE as u32, active_state::REACTION),
            (react_state::PASSIVE as u32, active_state::REACTION),
        ];
        for (action, state) in slots {
            w.u32(PetAction { action, state }.packed());
        }
        w.u8(2).u32(PetAction { action: 2649, state: active_state::ENABLED }.packed());
        w.u32(PetAction { action: 17253, state: active_state::PASSIVE }.packed());
        w.u16(1).u32(2649).u16(0).u32(4500).u32(0);

        let bar = parse_pet_spells(&w.buf).expect("the packet reads");
        assert_eq!(bar.pet, 0xF140_0000_0000_0001);
        assert_eq!(bar.react, react_state::DEFENSIVE);
        assert_eq!(bar.command, command_state::FOLLOW);
        assert_eq!(bar.bar[0].state, active_state::COMMAND);
        assert_eq!(bar.bar[0].action, u32::from(command_state::ATTACK));
        assert!(!bar.bar[0].is_spell(), "a command slot is not a spell");
        assert!(bar.bar[3].is_spell());
        assert!(active_state::auto_casts(bar.bar[3].state));
        assert!(bar.bar[4].is_spell(), "a passive is still a spell slot");
        assert!(!active_state::auto_casts(bar.bar[4].state));
        assert!(bar.bar[5].is_empty());
        assert_eq!(bar.spells.len(), 2);
        assert_eq!(
            bar.cooldowns,
            vec![PetCooldown {
                spell_id: 2649,
                category: 0,
                remaining_ms: 4500,
                category_ms: 0
            }]
        );
    }

    /// **Eight zero bytes is the pet going away**, and it is a real packet
    /// rather than a truncated one — `Player::RemovePetActionBar`.
    #[test]
    fn a_bare_guid_is_the_bar_being_taken_down() {
        let mut w = Writer::new();
        w.u64(0);
        let bar = parse_pet_spells(&w.buf).expect("the packet reads");
        assert!(bar.is_dismissal());
        assert!(bar.spells.is_empty());
    }

    /// **The pet's name is keyed by its number, not its guid**, and the query
    /// puts the number first — the reverse of every other query here.
    #[test]
    fn a_pet_name_answers_by_number_and_is_asked_for_in_that_order() {
        let mut w = Writer::new();
        w.u32(77).cstring("Growlfang").u32(1_234_567);
        let name = parse_pet_name(&w.buf).expect("the packet reads");
        assert_eq!(name.pet_number, 77);
        assert_eq!(name.name, "Growlfang");
        assert_eq!(name.timestamp, 1_234_567);

        let body = pet_name_query_body(77, 0xF140_0000_0000_0001);
        let mut r = Reader::new(&body);
        assert_eq!(r.u32(), 77, "the number leads");
        assert_eq!(r.u64(), 0xF140_0000_0000_0001);
    }

    /// **The four feedback keys are not four `ERR_PET_*` names**, which is the
    /// whole reason they were taken from the client rather than composed from
    /// the server's enum: two of them are the ordinary attack-refusal rows.
    #[test]
    fn the_feedback_keys_are_the_clients_and_two_are_not_pet_keys() {
        assert_eq!(feedback::key(feedback::PET_DEAD), Some("ERR_PET_SPELL_DEAD"));
        assert_eq!(
            feedback::key(feedback::NOTHING_TO_ATTACK),
            Some("ERR_NO_ATTACK_TARGET")
        );
        assert_eq!(
            feedback::key(feedback::CANT_ATTACK_TARGET),
            Some("ERR_INVALID_ATTACK_TARGET")
        );
        assert_eq!(feedback::key(feedback::NO_PATH_TO), Some("ERR_PET_SPELL_NOPATH"));
        // …and anything else says nothing at all.
        assert_eq!(feedback::key(0), None);
        assert_eq!(feedback::key(5), None);
    }

    /// **A tame failure always says something** — the client's out-of-range
    /// case is a key rather than a return.
    #[test]
    fn every_tame_failure_reason_has_a_sentence() {
        assert_eq!(tame_failure::key(1), "PETTAME_INVALIDCREATURE");
        assert_eq!(tame_failure::key(11), "PETTAME_NOTDEAD");
        assert_eq!(tame_failure::key(0), tame_failure::UNKNOWN);
        assert_eq!(tame_failure::key(12), tame_failure::UNKNOWN);
    }

    /// A bar with the three commands at 0..2, two spells at 3 and 4, and the
    /// rest empty — the shape every pet in the game arrives with.
    fn a_bar() -> [PetAction; PET_BAR_SLOTS] {
        let mut bar = [PetAction {
            action: 0,
            state: active_state::DISABLED,
        }; PET_BAR_SLOTS];
        for (slot, command) in [command_state::ATTACK, command_state::FOLLOW, command_state::STAY]
            .into_iter()
            .enumerate()
        {
            bar[slot] = PetAction {
                action: u32::from(command),
                state: active_state::COMMAND,
            };
        }
        bar[3] = PetAction {
            action: 2649,
            state: active_state::ENABLED,
        };
        bar[4] = PetAction {
            action: 17253,
            state: active_state::DISABLED,
        };
        bar
    }

    /// **Picking a spell up empties its slot first**, which is what
    /// `PickupPetAction` does before anything is dropped anywhere. Every drop
    /// test below starts from that state, because starting from the un-emptied
    /// one measures a gesture the client cannot make.
    fn picked_up(bar: &mut [PetAction; PET_BAR_SLOTS], slot: usize) -> u32 {
        let carried = bar[slot].packed();
        bar[slot] = PetAction::unpack(pet_slot_emptied(carried));
        carried
    }

    /// **A spell dropped on an occupied slot is one entry, and the old occupant
    /// comes back on the cursor** — the client hands the old occupant back
    /// only when nothing was displaced. The swap is completed by the player's next
    /// drop rather than by a two-entry packet.
    #[test]
    fn a_spell_dropped_on_another_hands_the_old_one_back() {
        let mut bar = a_bar();
        let carried = picked_up(&mut bar, 3);
        let write = place_on_pet_bar(&mut bar, 4, carried, |_| false).expect("the drop lands");
        assert_eq!(write.moves, vec![(4, carried)]);
        assert!(
            write.hand_back,
            "nothing was displaced, so slot 4's spell goes onto the cursor"
        );
        assert_eq!(bar[4].action, 2649);
    }

    /// …and on an empty slot, likewise one entry.
    #[test]
    fn a_drop_on_an_empty_slot_is_a_single_entry() {
        let mut bar = a_bar();
        let carried = picked_up(&mut bar, 3);
        let write = place_on_pet_bar(&mut bar, 7, carried, |_| false).expect("the drop lands");
        assert_eq!(write.moves, vec![(7, carried)]);
        assert!(write.hand_back);
    }

    /// **Moving a command button is the duplicate scan doing its job**, and it
    /// is the only gesture that produces a two-entry packet.
    ///
    /// A command is picked up *without* its slot being emptied — the pick-up
    /// sends nothing for anything but a spell — so when it is
    /// dropped elsewhere the scan finds it still sitting in its old slot and
    /// displaces the destination's occupant into it. That is the swap, and the
    /// pair is exactly what `HandlePetSetAction`'s `move_command` check reads.
    #[test]
    fn moving_a_command_displaces_through_the_slot_it_came_from() {
        let mut bar = a_bar();
        // No `picked_up`: a command's pick-up leaves the bar alone.
        let attack = bar[0].packed();
        let spell = bar[3].packed();
        let write = place_on_pet_bar(&mut bar, 3, attack, |_| false).expect("the drop lands");
        assert_eq!(
            write.moves,
            vec![(0, spell), (3, attack)],
            "the displaced slot first, then the destination"
        );
        assert!(!write.hand_back, "the old occupant found a home");
        assert_eq!(bar[0].action, 2649, "…and the bar agrees with the packet");
        assert_eq!(bar[3].state, active_state::COMMAND);
    }

    /// **A command sat on by something with nowhere to go refuses the drop**
    /// rather than destroying a button the server will not let go of.
    #[test]
    fn a_command_with_nowhere_to_go_refuses_the_drop() {
        let mut full = a_bar();
        for slot in 3..PET_BAR_SLOTS {
            full[slot] = PetAction {
                action: 1000 + slot as u32,
                state: active_state::DISABLED,
            };
        }
        // A word that is on no slot, dropped on the attack command.
        let carried = PetAction {
            action: 5555,
            state: active_state::DISABLED,
        }
        .packed();
        assert_eq!(place_on_pet_bar(&mut full, 0, carried, |_| false), None);
    }

    /// **The same word back where it came from is the cancel**, and **a passive
    /// may not be placed at all** — `Attributes & 0x40`, which is the one input
    /// this rule cannot read for itself.
    #[test]
    fn a_no_op_drop_and_a_passive_are_both_refused() {
        let mut bar = a_bar();
        let here = bar[3].packed();
        assert_eq!(place_on_pet_bar(&mut bar, 3, here, |_| false), None);

        let carried = PetAction {
            action: 17253,
            state: active_state::DISABLED,
        }
        .packed();
        assert_eq!(
            place_on_pet_bar(&mut bar, 7, carried, |id| id == 17253),
            None,
            "a passive is refused wherever it is dropped"
        );
    }

    /// **Emptying a slot keeps its own state byte** — `packed & 0xffff0000`,
    /// not a fixed `ACT_DISABLED`, so the server is told the right
    /// `ActiveStates` for the slot that emptied.
    #[test]
    fn a_removal_keeps_the_slots_state_and_matches_no_other_slot() {
        let auto_casting = PetAction {
            action: 2649,
            state: active_state::ENABLED,
        }
        .packed();
        assert_eq!(pet_slot_emptied(auto_casting), 0xC100_0000);
        assert_eq!(
            pet_slot_emptied(PetAction { action: 17253, state: active_state::PASSIVE }.packed()),
            0x0100_0000
        );

        // …and writing it is one entry, not a swap with the first empty slot it
        // happens to look like — the duplicate scan is skipped for it.
        let mut bar = a_bar();
        let write =
            place_on_pet_bar(&mut bar, 3, pet_slot_emptied(auto_casting), |_| false).expect("lands");
        assert_eq!(write.moves, vec![(3, 0xC100_0000)]);
        assert!(
            bar[3].is_empty(),
            "an emptied auto-casting slot still keeps its 0xC1 state byte"
        );
    }

    /// **The duplicate scan ignores the two auto-cast bits**, or a spell whose
    /// dot is on would fail to recognise itself and be duplicated onto the bar.
    #[test]
    fn the_duplicate_scan_ignores_the_autocast_bits() {
        let mut bar = a_bar();
        // The same spell as slot 3, which is auto-casting, carried with its
        // auto-cast bit *off*.
        let carried = PetAction {
            action: 2649,
            state: active_state::DISABLED,
        }
        .packed();
        let write = place_on_pet_bar(&mut bar, 7, carried, |_| false).expect("the drop lands");
        assert_eq!(
            write.moves.len(),
            2,
            "slot 3 holds the same spell and is displaced rather than left as a copy"
        );
        assert_eq!(write.moves[0].0, 3);
    }

    /// **The dismiss sound names a model and a place, not a pet** — the unit is
    /// gone by the time it arrives, which is the whole reason for its shape.
    #[test]
    fn the_dismiss_sound_carries_a_model_id_and_a_position() {
        let mut w = Writer::new();
        w.u32(2281).f32(-8_900.5).f32(-120.25).f32(81.5);
        let sound = parse_pet_dismiss_sound(&w.buf).expect("the packet reads");
        assert_eq!(sound.model_id, 2281);
        assert_eq!(sound.position, [-8_900.5, -120.25, 81.5]);
        // …and a body one float short is damage, not a shorter form.
        assert_eq!(parse_pet_dismiss_sound(&w.buf[..12]), None);
    }

    /// **A press sends the slot's own packed word back unchanged**, which is why
    /// the bar is kept packed rather than as three lists.
    #[test]
    fn a_press_returns_the_packed_word_it_was_given() {
        let slot = PetAction {
            action: 2649,
            state: active_state::ENABLED,
        };
        let body = pet_action_body(0xAA, slot.packed(), 0xBB);
        let mut r = Reader::new(&body);
        assert_eq!(r.u64(), 0xAA);
        assert_eq!(PetAction::unpack(r.u32()), slot);
        assert_eq!(r.u64(), 0xBB);
    }
}
