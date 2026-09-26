//! **Every skill the character has**, out of `PLAYER_SKILL_INFO_1_1`.
//!
//! [`crate::play::stats`] reads *four* slots of this block by name — the two
//! weapon skills, the ranged one and defence — because that is what the
//! character sheet's damage lines need. This is the other reader of the same
//! 128 slots: the whole list, for the panel that draws all of it.
//!
//! Kept apart rather than folded in for the reason the two are already apart in
//! the interface: `PaperDollFrame` asks four questions with names, and
//! `SkillFrame` asks one question with no name at all. A `UnitStats` that
//! carried the list would make every stat read pay for it.
//!
//! ## The triplet, and the three widths inside it
//!
//! ```text
//! +0   id | step << 16
//! +1   value | max << 16
//! +2   temporary << 16 >> 16 | permanent << 16      (two signed shorts)
//! ```
//!
//! The client's own reads in `GetSkillLineInfo` agree: they address the same
//! three dwords as `[playerFields + slot*12 + 0x848/0x84c/0x84e/0x852]` — the conversion being
//! that the player descriptor block begins at `PLAYER_FIELD_DUEL_ARBITER`,
//! index 188, so `0x848/4 + 188` is 718 and that is `PLAYER_SKILL_INFO_1_1`.
//!
//! **The slots are not ordered and there are gaps.** `Player::SetSkill` writes
//! into the first empty one, so slot order is the order the character learned
//! things in and a zero id is a hole rather than the end — the client's own walk
//! runs all 128 and skips (a bound of 128 × 12 bytes). A reader that stops at the first zero loses everything after the
//! first skill the character ever unlearned.
//!
//! ## What the panel adds, and what it does not
//!
//! The **rank** the panel prints is `value + permanent bonus` and the **max** is
//! `max + permanent bonus`, both clamped at zero — see [`Skill::rank`]. The
//! *temporary* bonus is drawn separately, as the striped overfill on the bar.
//! Everything else the panel shows — the name, the heading, the order, whether a
//! line is listed at all — is `SkillLine.dbc` and `SkillRaceClassInfo.dbc`, and
//! lives in [`vale_assets::tables::skills`] where it can be checked with no
//! server.

use crate::bytes::Reader;
use crate::state::fields;
use crate::state::objects::Entity;

/// How many triplets the field holds — vmangos' `PLAYER_MAX_SKILLS`, and the
/// `0x600` bytes the client's own walk covers.
pub const SLOTS: usize = 128;

/// One occupied slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Skill {
    /// `SkillLine.dbc`'s id. Never zero in a [`Skills`]: a zero slot is a hole
    /// and is dropped on the way in.
    pub id: u16,
    /// The high half of the first dword. Unused by 1.12's panel and kept because
    /// the field has it — see the module note.
    pub step: u16,
    pub value: u16,
    pub max: u16,
    /// A buff's contribution, drawn as the striped overfill.
    pub temporary: i16,
    /// …and the permanent one, which is part of both numbers the panel prints.
    pub permanent: i16,
}

impl Skill {
    /// **What the panel calls `skillRank`** — the value plus the permanent
    /// bonus, and the bonus is only added when the value is above zero.
    ///
    /// That guard is not decoration: an unlearned line reads value 0, and
    /// without it a stale permanent bonus would draw a rank on a skill the
    /// character does not have.
    pub fn rank(&self) -> i32 {
        let value = i32::from(self.value);
        if value <= 0 {
            return value;
        }
        value + i32::from(self.permanent)
    }

    /// …and `skillMaxRank` — the same shape one word over.
    pub fn max_rank(&self) -> i32 {
        let max = i32::from(self.max);
        if max <= 0 {
            return max;
        }
        max + i32::from(self.permanent)
    }

    /// `skillModifier`, which the panel prints in green or red beside the rank.
    ///
    /// The **temporary** bonus alone: the permanent one is already inside
    /// [`Self::rank`], and counting it twice is what makes a buffed skill read
    /// `310 (+10)/300` when the character has 310 of 300.
    pub fn modifier(&self) -> i32 {
        i32::from(self.temporary)
    }
}

/// The character's whole skill block, holes removed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Skills(Vec<Skill>);

impl Skills {
    /// Read it off a unit, or `None` for anybody but the local player.
    ///
    /// `PRIVATE`, like the stat block and the exploration mask, so nobody else's
    /// ever arrives — see [`crate::play::stats::UnitStats::read`], which answers
    /// `None` for the same reason and would be the place to look if this ever
    /// started answering for a target.
    ///
    /// **`None` and "no skills" are different**, and both are possible: a
    /// character always has at least a language, so an empty list here means the
    /// update block has not landed yet rather than a character with nothing.
    pub fn read(entity: &Entity) -> Option<Skills> {
        if entity.object_type != Some(crate::state::update::ObjectType::Player) {
            return None;
        }
        Self::decode(|index| entity.field(index))
    }

    /// The same, from bare `(index, value)` pairs — for tests on the far side of
    /// the crate boundary, on [`crate::play::stats::UnitStats::from_fields`]'
    /// own terms: same decode, so a test written against this cannot pass while
    /// the client's read fails.
    pub fn from_fields(fields: &[(u16, u32)]) -> Option<Skills> {
        Self::decode(|index| {
            fields
                .iter()
                .find(|(at, _)| *at == index)
                .map(|(_, value)| *value)
        })
    }

    fn decode(field: impl Fn(u16) -> Option<u32>) -> Option<Skills> {
        let mut out = Vec::new();
        let mut any = false;
        for slot in 0..SLOTS as u16 {
            let base = fields::player::SKILL_INFO_1_1 + slot * 3;
            let Some(first) = field(base) else {
                continue;
            };
            any = true;
            let id = first as u16;
            if id == 0 {
                continue;
            }
            let value = field(base + 1).unwrap_or(0);
            let bonus = field(base + 2).unwrap_or(0);
            out.push(Skill {
                id,
                step: (first >> 16) as u16,
                value: value as u16,
                max: (value >> 16) as u16,
                temporary: bonus as u16 as i16,
                permanent: (bonus >> 16) as u16 as i16,
            });
        }
        any.then_some(Skills(out))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Skill> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// One line's slot, or `None` for a line the character does not have.
    pub fn get(&self, id: u16) -> Option<&Skill> {
        self.0.iter().find(|skill| skill.id == id)
    }
}


/// **What the character is allowed to hold** — `SMSG_SET_PROFICIENCY`, one
/// packet per item class, and the whole of it is `{u8 item_class, u32
/// subclass_mask}`.
///
/// The server sends one on login for each class the character has any
/// proficiency in and another every time a skill is learned or unlearned, and
/// each is the **complete** statement for that class rather than a delta — so a
/// reader stores it by class and replaces.
///
/// Two classes ever arrive: `2` weapons and `4` armour. The mask's bits are the
/// item's `subclass` — sword 0, axe 1, bow 2, and so on for weapons; cloth 1,
/// leather 2, mail 3, plate 4 and shield 6 for armour — and the client's answer
/// to "may I equip this" is one `&`.
///
/// **This is the only channel the answer arrives on.** Nothing in the archives
/// says which class may hold which weapon: `Item.dbc` is not shipped, and the
/// skill lines that grant a proficiency are the server's own
/// `playercreateinfo_spell` and trainer tables. A client with no reader for this
/// packet has to draw every item as usable, which is what this one did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proficiency {
    /// `ITEM_CLASS_WEAPON` (2) or `ITEM_CLASS_ARMOR` (4).
    pub item_class: u8,
    /// A bit per subclass of that class.
    pub subclass_mask: u32,
}

/// `ITEM_CLASS_WEAPON`, the first of the two classes this packet is ever sent
/// for.
pub const ITEM_CLASS_WEAPON: u8 = 2;
/// `ITEM_CLASS_ARMOR`, the second.
pub const ITEM_CLASS_ARMOR: u8 = 4;

pub fn parse_proficiency(body: &[u8]) -> Option<Proficiency> {
    let mut r = Reader::new(body);
    if !r.has(1 + 4) {
        return None;
    }
    Some(Proficiency { item_class: r.u8(), subclass_mask: r.u32() })
}

/// **Every class's mask, as the session has been told them.**
///
/// A list rather than a map because there are two of them and the reference
/// walks its own array linearly. An absent class is "the server has said
/// nothing", which [`Proficiencies::allows`] answers `true` for — see its note,
/// which is the one decision in this file that can be wrong in a way nobody
/// notices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proficiencies {
    classes: Vec<Proficiency>,
}

impl Proficiencies {
    /// Take one packet, replacing whatever that class said before.
    pub fn set(&mut self, said: Proficiency) {
        match self.classes.iter_mut().find(|held| held.item_class == said.item_class) {
            Some(held) => *held = said,
            None => self.classes.push(said),
        }
    }

    /// The mask for one class, or `None` if the server has not spoken about it.
    pub fn mask(&self, item_class: u8) -> Option<u32> {
        self.classes.iter().find(|held| held.item_class == item_class).map(|held| held.subclass_mask)
    }

    /// **May the character hold this item?**
    ///
    /// **A class the server has said nothing about is permitted**, and that is
    /// deliberate rather than lenient: proficiency is only ever sent for weapons
    /// and armour, so every other class — a potion, a bag, a reagent, a quest
    /// item — has no packet and must not be drawn red. Refusing what has not
    /// been mentioned would grey out a character's whole bag on the frame before
    /// the two packets land.
    pub fn allows(&self, item_class: u8, item_subclass: u8) -> bool {
        match self.mask(item_class) {
            Some(mask) => item_subclass < 32 && mask & (1 << item_subclass) != 0,
            None => true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(slots: &[(u16, u32, u32, u32)]) -> Skills {
        let mut fields = Vec::new();
        for (slot, first, value, bonus) in slots {
            let base = fields::player::SKILL_INFO_1_1 + slot * 3;
            fields.push((base, *first));
            fields.push((base + 1, *value));
            fields.push((base + 2, *bonus));
        }
        Skills::from_fields(&fields).expect("a block")
    }

    /// The three widths, and the one that is two *signed* shorts.
    #[test]
    fn a_slot_is_three_dwords_and_the_last_is_signed() {
        // Defense (95), 300 of 300, temporary +4, permanent -1.
        let skills = block(&[(
            0,
            95,
            300 | (300 << 16),
            (4u32 & 0xffff) | ((-1i32 as u32) << 16),
        )]);
        let skill = skills.get(95).expect("defense");
        assert_eq!((skill.value, skill.max), (300, 300));
        assert_eq!((skill.temporary, skill.permanent), (4, -1));
        assert_eq!(skill.rank(), 299, "value plus the permanent bonus");
        assert_eq!(skill.max_rank(), 299, "…and the max moves with it");
        assert_eq!(skill.modifier(), 4, "the temporary one alone");
    }

    /// **A zero id is a hole, not the end**, which is what a character who has
    /// unlearned a profession has in the middle of the block.
    #[test]
    fn a_gap_does_not_end_the_walk() {
        let skills = block(&[
            (0, 98, 300 | (300 << 16), 0),
            (1, 0, 0, 0),
            (2, 164, 150 | (300 << 16), 0),
        ]);
        assert_eq!(skills.len(), 2, "the hole is dropped, the tail is kept");
        assert!(skills.get(164).is_some(), "Blacksmithing survived the gap");
    }

    /// An unlearned line reads zero and must not pick up a stale bonus — the
    /// client's own guard before it adds one.
    #[test]
    fn a_zero_value_keeps_its_zero() {
        let skills = block(&[(0, 186, 0, 25 << 16)]);
        let skill = skills.get(186).expect("the line");
        assert_eq!(skill.rank(), 0);
        assert_eq!(skill.max_rank(), 0);
    }

    /// A block nobody wrote is `None`, which is not the same as a character
    /// with no skills.
    #[test]
    fn an_empty_block_answers_none() {
        assert!(Skills::from_fields(&[]).is_none());
    }
    /// **One class's whole mask, and a re-send replaces it** — see
    /// [`Proficiencies`], whose note is that this packet is never a delta.
    #[test]
    fn a_proficiency_packet_replaces_its_class() {
        let body = |class: u8, mask: u32| {
            let mut out = vec![class];
            out.extend_from_slice(&mask.to_le_bytes());
            out
        };
        let weapons = parse_proficiency(&body(ITEM_CLASS_WEAPON, 0b0101)).expect("weapons");
        assert_eq!(weapons.item_class, ITEM_CLASS_WEAPON);
        assert_eq!(weapons.subclass_mask, 0b0101);
        let mut held = Proficiencies::default();
        held.set(weapons);
        held.set(parse_proficiency(&body(ITEM_CLASS_ARMOR, 0b0010)).expect("armour"));
        // A sword (0) and an axe (2) are in; a bow (1) is not.
        assert!(held.allows(ITEM_CLASS_WEAPON, 0));
        assert!(!held.allows(ITEM_CLASS_WEAPON, 1));
        assert!(held.allows(ITEM_CLASS_WEAPON, 2));
        // …and the two classes do not read each other's mask.
        assert!(held.allows(ITEM_CLASS_ARMOR, 1));
        assert!(!held.allows(ITEM_CLASS_ARMOR, 0));
        // A second packet for a class is that class's new whole answer.
        held.set(parse_proficiency(&body(ITEM_CLASS_WEAPON, 0b0010)).expect("weapons again"));
        assert!(!held.allows(ITEM_CLASS_WEAPON, 0), "the old mask is gone, not merged");
        assert!(held.allows(ITEM_CLASS_WEAPON, 1));
    }

    /// **A class the server has said nothing about is permitted**, which is the
    /// one decision here that can be wrong in a way nobody notices — see
    /// [`Proficiencies::allows`].
    #[test]
    fn an_unmentioned_class_is_allowed() {
        let mut held = Proficiencies::default();
        assert!(held.is_empty());
        // A potion (class 0), a bag (1), a reagent (9) — no packet is ever sent
        // for any of them.
        assert!(held.allows(0, 0));
        assert!(held.allows(9, 3));
        held.set(Proficiency { item_class: ITEM_CLASS_WEAPON, subclass_mask: 0 });
        // …and a class that *was* mentioned with an empty mask allows nothing.
        assert!(!held.allows(ITEM_CLASS_WEAPON, 0));
        assert!(held.allows(ITEM_CLASS_ARMOR, 0), "still unmentioned");
        // A subclass past the mask's width is out rather than wrapping.
        assert!(!held.allows(ITEM_CLASS_WEAPON, 32));
    }

    /// A body shorter than five bytes is not a proficiency.
    #[test]
    fn a_short_proficiency_body_is_none() {
        assert!(parse_proficiency(&[]).is_none());
        assert!(parse_proficiency(&[2, 0, 0, 0]).is_none());
        assert!(parse_proficiency(&[2, 0, 0, 0, 0]).is_some());
    }

}
