//! The purpose of a game object: whether a click on it does anything, and which
//! pointer is drawn over it.
//!
//! The door in the Deadmines, the ore vein on the wall beside it, the mailbox in
//! Sentinel Hill and a campfire nobody can touch are the same kind of entity to
//! this client: `ObjectType` is `GameObject`, with a display id, an open/shut
//! state, and a name that arrived by `CMSG_GAMEOBJECT_QUERY`. None of those
//! fields says which of them a player can act on.
//!
//! The **template** says it: one type byte and a union of twenty-four words,
//! both returned in `SMSG_GAMEOBJECT_QUERY_RESPONSE` and neither interpreted by
//! the packet parser. This module interprets them. It lives in `assets` because
//! every question it answers can be decided without a renderer, and
//! `vale objects` calls the same code the renderer calls.
//!
//! ## The pointer is keyed on the server's type enum
//!
//! `GAMEOBJECT_TYPE_*` is shared by client and server: the client learns a type
//! only because the server sent one, and the effect of `CMSG_GAMEOBJ_USE` is
//! decided in vmangos `GameObject::Use` by a switch over this enum. Keying the
//! pointer on the type therefore uses the table both ends agree on. This module
//! does not reproduce the 1.12.1 client's pointer rules exactly. Where the two
//! could differ it offers the click: a hand over something inert costs one
//! packet the server ignores, while a missing hand leaves a door the player
//! cannot open.
//!
//! ## The word that holds the lock depends on the type
//!
//! A door keeps its lock in `data[1]` and a chest keeps it in `data[0]`, because
//! a door's first word is `startOpen`. Reading `data[0]` for both would give
//! every door the lock id 0 or 1 ("no lock" or "lock 1"), and every Deadmines
//! door would draw as an ordinary openable one, with no error reported.
//! [`Kind::lock_word`] is the only place the offset is chosen, and a type with
//! no lock returns `None` rather than defaulting to zero.
//!
//! ## A chest is opened by a spell cast, not by the use packet
//!
//! The opcode names do not show this. `CMSG_GAMEOBJ_USE` reaches vmangos
//! `GameObject::Use`, whose complete `GAMEOBJECT_TYPE_CHEST` case is
//!
//! ```text
//! case GAMEOBJECT_TYPE_CHEST:                         // 3
//! {
//!     if (user->GetTypeId() != TYPEID_PLAYER) return;
//!     GetMap()->ScriptsStart(sGameObjectScripts, ...);
//!     TriggerLinkedGameObject(user);
//!     return;
//! }
//! ```
//!
//! It runs a script and a linked trap and sends no loot. The only code in
//! vmangos that sends a chest's loot is `Spell::EffectOpenLock`, which ends in
//! `SendLoot(guid, LOOT_SKINNING, LockType(m_spellInfo->EffectMiscValue[effIdx]))`.
//! A chest, an ore vein and a herb are therefore opened by casting a spell at
//! the object (`CMSG_CAST_SPELL` with `TARGET_FLAG_GAMEOBJECT`), and the lock
//! decides which spell. [`opener`] chooses the spell; [`Kind::opened_by_spell`]
//! lists the types that are opened this way.
//!
//! This was tested on a live server: `CMSG_GAMEOBJ_USE` sent at a herb node by a
//! character with 300 Herbalism produced no loot, no state change and no
//! refusal, which matches the case quoted above.
//!
//! An unlocked chest needs no special case. Every character is created knowing
//! `Opening` (3365, and the related spells 21651 and 22810), whose open-lock misc
//! value is `LOCKTYPE_OPEN`. The rule that finds Mining for a vein finds
//! `Opening` for a chest with no lock, so there is no fallback branch. The
//! source is the world database's `playercreateinfo_spell`, checked on the
//! running server.
//!
//! ## `GAMEOBJECT_TYPE_GENERIC`: signs that can be hovered but not used
//!
//! The world database has 1,869 objects of `GAMEOBJECT_TYPE_GENERIC`. 1,197 of
//! them are signposts, plaques and markers: hovering one shows its name, which
//! tells the player where they are. The Goldshire crossroads is six of them, one
//! game object per arm, each named for where the arm points, all on display
//! id 26.
//!
//! Two template fields set them apart:
//!
//! ```text
//! 5 GAMEOBJECT_TYPE_GENERIC   data[0] floatingTooltip   data[1] highlight
//!                             data[2] serverOnly        data[3] large
//! ```
//!
//! * `floatingTooltip` turns on the tooltip, and the tooltip follows the
//!   pointer instead of sitting in the screen corner where a unit's tooltip
//!   goes.
//! * vmangos `HandleGameObjectUseOpcode` refuses the type before any other
//!   check (`if (obj->GetGoType() == GAMEOBJECT_TYPE_GENERIC) return;`), so the
//!   server drops a use packet for one.
//!
//! "Can be clicked" and "can be hovered" are therefore separate questions:
//! [`Kind::usable`] answers the first and [`hover_of`] the second. When the two
//! were one test, the signs were filtered out of the pick as scenery, which 468
//! of the 1,869 generic objects are.
//!
//! ## Not decided here: whether this character may open this lock
//!
//! That depends on the character's skill, the keys in their bags and the quests
//! in their log, and the server checks all three; a refusal arrives as one of
//! the `ERR_USE_LOCKED_*` messages. The pointer shows what the object is; the
//! server decides whether the character may use it.

use crate::tables::lock::{action_applies, lock_type, KeyKind, Locks};

use super::cursor::Cursor;

/// `GAMEOBJECT_TYPE`, the values `GameObjectInfo::type` takes.
///
/// Only the types this client treats specially are named; every other value is
/// [`Kind::Other`] and draws nothing. The numbering is the server's enum, and
/// every value named here is the same from 1.12 to modern builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// 0 — a door. `data[0]` startOpen, `data[1]` lockId.
    Door,
    /// 1 — a lever, a valve, a button. Same first two words as a door.
    Button,
    /// 2 — a game object that hands out quests. `data[0]` lockId.
    QuestGiver,
    /// 3 — a chest, and also every ore vein and herb node; the lock tells them
    /// apart. `data[0]` lockId, `data[1]` lootId.
    Chest,
    /// 5 — signposts, plaques and markers. Never usable, often hoverable; see
    /// the module documentation and [`hover_of`]. `data[0]` floatingTooltip,
    /// `data[1]` highlight.
    ///
    /// This is the default because it is the one type that does nothing: a
    /// caller that has no template yet must not be treated as holding a door
    /// or a chest.
    #[default]
    Generic,
    /// 6 — a trap. Not clickable; named so that its absence from
    /// [`Kind::usable`] is visibly intended.
    Trap,
    /// 12 — an area-damage volume. Not clickable. Named for the same reason as
    /// [`Kind::Trap`]: it carries a lock, and a player does not use it.
    AreaDamage,
    /// 13 — a camera. Not clickable, for the same reason as [`Kind::Trap`].
    Camera,
    /// 7 — a chair. Clicking one sits the character in it.
    Chair,
    /// 9 — a sign, a plaque, a tombstone: `SMSG_GAMEOBJECT_PAGETEXT`.
    Text,
    /// 10 — the general-purpose interactive object: a brazier to light, a
    /// lever tied to a quest, a book on a stand. `data[0]` lockId.
    Goober,
    /// 19 — a mailbox.
    Mailbox,
    /// 20 — an auction house.
    AuctionHouse,
    /// 21 — a guard post.
    GuardPost,
    /// 22 — a portal or other object that casts a spell on you.
    SpellCaster,
    /// 23 — a meeting stone.
    MeetingStone,
    /// 24 — a battleground flag stand.
    FlagStand,
    /// 25 — a fishing school. `data[4]` lockId.
    FishingHole,
    /// 26 — a dropped battleground flag.
    FlagDrop,
    /// Every other type: invisible zone markers, spell focuses, scenery,
    /// transports.
    Other(u32),
}

impl Kind {
    /// The [`Kind`] for a template's type byte.
    pub fn of(object_type: u32) -> Kind {
        match object_type {
            0 => Kind::Door,
            1 => Kind::Button,
            2 => Kind::QuestGiver,
            3 => Kind::Chest,
            5 => Kind::Generic,
            6 => Kind::Trap,
            7 => Kind::Chair,
            9 => Kind::Text,
            10 => Kind::Goober,
            12 => Kind::AreaDamage,
            13 => Kind::Camera,
            19 => Kind::Mailbox,
            20 => Kind::AuctionHouse,
            21 => Kind::GuardPost,
            22 => Kind::SpellCaster,
            23 => Kind::MeetingStone,
            24 => Kind::FlagStand,
            25 => Kind::FishingHole,
            26 => Kind::FlagDrop,
            other => Kind::Other(other),
        }
    }

    /// The indices of the two template words that hold a page, as
    /// `(pageId, pageMaterial)`, or `None` for a type that cannot carry one.
    ///
    /// Two types can, and they keep the pair at different offsets, as with
    /// [`Self::lock_word`]. vmangos `GameObjectDefines.h` lists both unions:
    ///
    /// ```text
    /// 9  TEXT     data[0] pageID  data[1] language  data[2] pageMaterial  data[3] allowMounted
    /// 10 GOOBER   … data[7] pageId  data[8] language  data[9] pageMaterial …
    /// ```
    ///
    /// A `pageId` of zero is the usual case for a goober (a brazier, a lever)
    /// and means there is nothing to read. The caller tests for zero; this
    /// function does not.
    ///
    /// The server handles the two types differently. For a goober with a page,
    /// `GameObject::Use` sends `SMSG_GAMEOBJECT_PAGETEXT` carrying only the guid
    /// (`GameObject.cpp:1551`), and the client finds the page id here. A `TEXT`
    /// object has no case in `Use`, so the server sends nothing and the client
    /// opens the page from this template itself. Both paths end at the same
    /// query; see [`vale_protocol::play::pagetext`].
    pub fn page_words(self) -> Option<(usize, usize)> {
        match self {
            Kind::Text => Some((0, 2)),
            Kind::Goober => Some((7, 9)),
            _ => None,
        }
    }

    /// The index of the template word that holds this type's lock id, or
    /// `None` for a type with no lock.
    ///
    /// This is the only place the offset is chosen. It is a function rather
    /// than `data[0]` at the call site because a door and a chest keep the lock
    /// in different words; see the module documentation.
    pub fn lock_word(self) -> Option<usize> {
        Some(match self {
            // A door's and a button's first word is `startOpen`.
            Kind::Door | Kind::Button => 1,
            Kind::QuestGiver | Kind::Chest | Kind::Goober | Kind::FlagStand | Kind::FlagDrop => 0,
            // Traps, area-damage volumes and cameras rarely appear in the world
            // data. In the 1.12.1 client the first word of each is its lock.
            // None of the three is [`Self::usable`], so nothing in this client
            // asks; they are listed because `None` would be a wrong answer for
            // them, not a missing one.
            Kind::Trap | Kind::AreaDamage | Kind::Camera => 0,
            // `radius`, `lootId`, `minSuccessOpens`, `maxSuccessOpens`, then the
            // lock. This is the only type whose lock is not in the first two
            // words.
            Kind::FishingHole => 4,
            Kind::Generic
            | Kind::Chair
            | Kind::Text
            | Kind::Mailbox
            | Kind::AuctionHouse
            | Kind::GuardPost
            | Kind::SpellCaster
            | Kind::MeetingStone
            | Kind::Other(_) => return None,
        })
    }

    /// Whether this type is opened by casting a spell at it rather than by
    /// `CMSG_GAMEOBJ_USE`.
    ///
    /// True for the two types that exist to hold loot; the module documentation
    /// quotes the server case that sends none on the use path. Every other type,
    /// including doors, levers, chairs and mailboxes, is sent the use packet. The
    /// server checks a door's lock on that path (`GameObject::PlayerCanUse`
    /// walks its `LOCK_KEY_ITEM` slots), so a locked door is not cast at.
    pub fn opened_by_spell(self) -> bool {
        matches!(self, Kind::Chest | Kind::FishingHole)
    }

    /// Whether a click on this type is sent to the server.
    ///
    /// `false` means the pointer stays an arrow and a click does nothing. That
    /// is the answer for most game objects in the world: the 1,869 invisible
    /// zone markers, the 2,332 spell focuses, the 2,297 scenery pieces and every
    /// transport are not used by a player.
    ///
    /// Traps and every `Other` are intentionally excluded; the module
    /// documentation explains which way this module errs when unsure.
    pub fn usable(self) -> bool {
        matches!(
            self,
            Kind::Door
                | Kind::Button
                | Kind::QuestGiver
                | Kind::Chest
                | Kind::Chair
                | Kind::Text
                | Kind::Goober
                | Kind::Mailbox
                | Kind::AuctionHouse
                | Kind::GuardPost
                | Kind::SpellCaster
                | Kind::MeetingStone
                | Kind::FlagStand
                | Kind::FishingHole
                | Kind::FlagDrop
        )
    }
}

/// `GAMEOBJECT_FLAGS`: the bits of the game object's update field that decide
/// whether a click on it does anything.
///
/// The template describes what an object is; this field describes its current
/// state, and the server changes it during a session. A door that is
/// mid-swing, a chest another player is looting and an event object whose
/// event is not running all have ordinary templates and a bit set here.
///
/// The bits are named constants because two of the three tested bits are read
/// together and the third is read against a different field; see
/// [`interactable`].
pub mod go_flags {
    /// The object is animating. `GameObjectDefines.h`: "disables interaction
    /// while animated".
    pub const IN_USE: u32 = 0x0000_0001;
    /// The tooltip shows "Locked". The server's comment on the bit says so, and
    /// the 1.12.1 client shows the line in the object's tooltip.
    pub const LOCKED: u32 = 0x0000_0002;
    /// The server decides per character whether this object may be used; the
    /// answer is [`go_dyn_flags::ACTIVATE`] in the private part of the update
    /// block.
    pub const INTERACT_COND: u32 = 0x0000_0004;
    /// No character may use the object.
    pub const NO_INTERACT: u32 = 0x0000_0010;
}

/// `GAMEOBJECT_DYN_FLAGS`: the server's per-player answer to
/// [`go_flags::INTERACT_COND`].
pub mod go_dyn_flags {
    /// This character may use the object. Tested only when `INTERACT_COND` is
    /// set.
    pub const ACTIVATE: u32 = 0x0000_0001;
}

/// The `GAMEOBJECT_STATE` value for a destroyed object.
///
/// It has its own constant because it is the one state for which a click is
/// refused rather than handled differently: the client shows
/// `ERR_USE_DESTROYED`.
pub const STATE_DESTROYED: u8 = 2;

/// Whether a click on this object is sent to the server now: [`Kind::usable`]
/// plus the three tests the 1.12.1 client applies to the object's update-field
/// flags.
///
/// ```text
/// IN_USE | NO_INTERACT set                     -> no
/// INTERACT_COND set and ACTIVATE clear         -> no
/// otherwise                                    -> the type decides
/// ```
///
/// The type alone does not decide usability. A chest from another player's
/// kill, a door that is already swinging and an event object between events
/// are `Kind::Chest` and `Kind::Door` with ordinary templates; only the flags
/// separate them from objects the player may use. Deciding by type alone drew
/// the interact hand over all of them and sent packets the server drops.
///
/// The 1.12.1 client also tests the creator unit's reaction to the player.
/// That test is left out: a world game object has no creator, so it decides
/// nothing for objects a player meets outside a battleground, and applying it
/// would require resolving a guid, which this crate cannot do.
pub fn interactable(kind: Kind, flags: u32, dyn_flags: u32) -> bool {
    if !kind.usable() {
        return false;
    }
    if flags & (go_flags::IN_USE | go_flags::NO_INTERACT) != 0 {
        return false;
    }
    if flags & go_flags::INTERACT_COND != 0 && dyn_flags & go_dyn_flags::ACTIVATE == 0 {
        return false;
    }
    true
}

/// The requirement the tooltip shows for opening an object, or `None` when it
/// shows none.
///
/// This is `Lock.dbc` column 0, shown only when that column's `Action` applies
/// to the object's current state. Only column 0 is used; see
/// [`Locks::first_slot`] for the column rule and [`action_applies`] for the
/// state test.
///
/// `None` is by far the most common answer. Three cases produce it: a type with
/// no lock word, a lock id of 0, and a lock whose only key is in a column other
/// than 0, which the 1.12.1 client does not show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// `LOCK_KEY_SKILL`: a `LockType.dbc` row and the rank wanted, which is 0
    /// for most of them.
    Skill { lock_type: u32, rank: u32 },
    /// `LOCK_KEY_ITEM`: an item entry — a key, a quest token.
    Item(u32),
}

/// [`Requirement`] for one object.
///
/// `state` is `GAMEOBJECT_STATE` and `locked` is [`go_flags::LOCKED`]. Both come
/// from update fields rather than the template, because the action test is
/// about the object's current state. A strongbox that has been picked stops
/// showing "Requires Pick Lock" as soon as the flag clears, because the action
/// column no longer applies.
pub fn requirement_of(locks: &Locks, lock_id: u32, state: u8, locked: bool) -> Option<Requirement> {
    let key = locks.first_slot(lock_id)?;
    if !action_applies(key.action, state, locked) {
        return None;
    }
    Some(match key.kind {
        KeyKind::Skill { lock_type, rank } => Requirement::Skill { lock_type, rank },
        KeyKind::Item(entry) => Requirement::Item(entry),
    })
}

/// The five colours the 1.12.1 client uses for a lock requirement, as linear
/// `[r, g, b]` in 0..1.
///
/// The client grades `(need, have)` into one of five colours; each constant
/// gives its ARGB value. This is the gathering-difficulty ramp, the same one a
/// mining node's name is drawn in, so the colour of "Requires Mining" shows the
/// character's skill against the node's requirement.
pub mod difficulty {
    /// `0xff808080`, `have >= need + 100`.
    pub const TRIVIAL: [f32; 3] = [0.5, 0.5, 0.5];
    /// `0xff40c040`, `have >= need + 50`. This is not the interface's
    /// `GREEN_FONT_COLOR`, which is `0/1/0`.
    pub const EASY: [f32; 3] = [0.25, 0.75, 0.25];
    /// `0xffffff00`, `have >= need + 25`.
    pub const MEDIUM: [f32; 3] = [1.0, 1.0, 0.0];
    /// `0xffff8040`, `have >= need`.
    pub const HARD: [f32; 3] = [1.0, 0.5, 0.25];
    /// `0xffff2020`, `have < need`; the same red as `RED_FONT_COLOR_CODE`.
    pub const IMPOSSIBLE: [f32; 3] = [1.0, 32.0 / 255.0, 32.0 / 255.0];
}

/// The [`difficulty`] colour for `have` against `need`, testing the bands from
/// `TRIVIAL` down to `IMPOSSIBLE` as the 1.12.1 client does.
pub fn difficulty_colour(have: u32, need: u32) -> [f32; 3] {
    if have >= need.saturating_add(100) {
        difficulty::TRIVIAL
    } else if have >= need.saturating_add(50) {
        difficulty::EASY
    } else if have >= need.saturating_add(25) {
        difficulty::MEDIUM
    } else if have >= need {
        difficulty::HARD
    } else {
        difficulty::IMPOSSIBLE
    }
}

/// The best way this character has to open one lock, and whether it is enough.
///
/// The result of the lock check. It decides both the tooltip line's colour and
/// whether the click is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockWay {
    /// The open-lock spell the character knows for one of the lock's skill
    /// slots, or `None` when the way in was a key.
    pub spell: Option<u32>,
    /// The key's item entry the character is carrying, or `None` when the way
    /// in was a spell.
    pub item: Option<u32>,
    /// Their value in the skill line that spell belongs to, and the rank the
    /// slot wants. Both zero for an item way in.
    pub have: u32,
    pub need: u32,
    /// Whether this way in succeeds: `have >= need`, or the item is held.
    /// `false` is, for example, a vein a miner is thirty points short of; the
    /// tooltip draws that requirement in red.
    pub enough: bool,
}

/// [`LockWay`] for one lock, checked slot by slot.
///
/// `None` means the lock has no applicable way in: an unlocked object (id 0), a
/// lock id missing from the table, or a lock whose every slot is ruled out by
/// [`action_applies`] for the object's current state. `None` is not a refusal:
/// `Opening` still applies to an unlocked chest through the ordinary path (see
/// [`opener`]).
///
/// The two closures supply the character data that no file holds. `by_skill`
/// returns, for a `LockType.dbc` row, the spell this character knows that opens
/// it and the character's value in that spell's skill line; `by_item` returns
/// whether the character carries an item entry. `level` is `GAMEOBJECT_LEVEL`;
/// a slot whose rank is 0 requires five times that level instead. vmangos
/// writes that field only for transports, so in practice it is zero and any
/// character meets a rank-0 slot.
///
/// The first slot the character satisfies is returned. If none succeeds, the
/// first slot that matched but fell short is returned, so the tooltip can show
/// "Requires Mining" in red instead of showing nothing.
pub fn can_open(
    locks: &Locks,
    lock_id: u32,
    state: u8,
    locked: bool,
    level: u32,
    by_skill: impl Fn(u32) -> Option<(u32, u32)>,
    by_item: impl Fn(u32) -> bool,
) -> Option<LockWay> {
    let mut best: Option<LockWay> = None;
    for key in locks.keys(lock_id) {
        if !action_applies(key.action, state, locked) {
            continue;
        }
        match key.kind {
            KeyKind::Skill { lock_type, rank } => {
                let Some((spell, have)) = by_skill(lock_type) else {
                    continue;
                };
                let need = if rank == 0 { level.saturating_mul(5) } else { rank };
                let way = LockWay {
                    spell: Some(spell),
                    item: None,
                    have,
                    need,
                    enough: have >= need,
                };
                if way.enough {
                    return Some(way);
                }
                best.get_or_insert(way);
            }
            KeyKind::Item(entry) => {
                if by_item(entry) {
                    return Some(LockWay {
                        spell: None,
                        item: Some(entry),
                        have: 0,
                        need: 0,
                        enough: true,
                    });
                }
            }
        }
    }
    best
}

/// What hovering a game object shows, read from the template. This is separate
/// from whether the object can be clicked.
///
/// See the module documentation. Both flags are template fields of
/// `GAMEOBJECT_TYPE_GENERIC` only; for every other type they follow from the
/// type: an object that can be used is highlighted, and its tooltip is placed
/// where a unit's goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hover {
    /// The tooltip follows the pointer instead of sitting in the corner of the
    /// screen. From `floatingTooltip`; this is the visible difference between a
    /// street sign and a chest.
    pub floating: bool,
    /// The model lights up under the pointer.
    pub highlight: bool,
}

/// [`Hover`] for one template.
///
/// `data` is the raw 24-word union from `SMSG_GAMEOBJECT_QUERY_RESPONSE`. Words
/// missing from a short slice read as zero, which gives the answer for an
/// object whose template has not arrived: it is not picked.
///
/// A generic object with `highlight` set and `floatingTooltip` clear gets no
/// hover here, which deviates from the template. 189 rows have that shape.
/// Honouring the flag would give a model that glows under the pointer, shows
/// nothing and does nothing, and that wins the depth test over a unit standing
/// behind it, which is the fault the sign filter was added to prevent. Not
/// picking such an object loses a glow; picking it hides the unit.
pub fn hover_of(kind: Kind, data: &[u32]) -> Hover {
    let word = |n: usize| data.get(n).copied().unwrap_or(0) != 0;
    match kind {
        Kind::Generic => {
            let floating = word(0);
            Hover {
                floating,
                // Highlight only with the tooltip; see this function's doc.
                highlight: floating && word(1),
            }
        }
        // A usable object is highlighted, and its tooltip is anchored like a
        // unit's, where `GameTooltip_SetDefaultAnchor` puts it.
        kind if kind.usable() => Hover {
            floating: false,
            highlight: true,
        },
        _ => Hover::default(),
    }
}

/// Whether the pointer ray tests this game object at all.
///
/// True when the object is usable or its hover shows a floating tooltip.
/// Everything else (the spell focuses that are anvils and forges, transports,
/// traps, the 468 inert generic objects) is excluded, because a game object
/// that wins the depth test clears the hover of any unit behind it.
pub fn worth_pointing_at(kind: Kind, data: &[u32]) -> bool {
    kind.usable() || hover_of(kind, data).floating
}

/// The pointer drawn over one game object, or `None` for the arrow.
///
/// The lock is tested before the type. An ore vein and a strongbox are both
/// `Chest`, and only the vein's lock names Mining; testing the type first would
/// draw a hand over every vein.
///
/// `lock_id` is the template word chosen by [`Kind::lock_word`]. It is zero for
/// a type with no lock and for an unlocked object, and [`Locks::skill_lock`]
/// returns `None` for zero.
pub fn over_object(kind: Kind, locks: &Locks, lock_id: u32) -> Option<Cursor> {
    if !kind.usable() {
        return None;
    }
    if let Some((lock_type, _)) = locks.skill_lock(lock_id) {
        match lock_type {
            lock_type::HERBALISM => return Some(Cursor::GatherHerbs),
            lock_type::MINING => return Some(Cursor::Mine),
            lock_type::PICK_LOCK => return Some(Cursor::PickLock),
            // `Open`, `Treasure`, `Disarm Trap`, `Close` and the eleven unused
            // rows fall through to the type below. No pointer image means
            // "opened by the Opening spell", and every quest goober uses such
            // a lock.
            _ => {}
        }
    }
    Some(match kind {
        // A sign (`Generic`) never reaches this match: `usable()` is false for
        // it and the function has already returned. The 1.12.1 client shows
        // the ordinary arrow over a signpost, and a hand would advertise a
        // click that does nothing.
        Kind::Mailbox => Cursor::Mail,
        // A quest goober does not get the speech bubble: in 1.12 the bubble is
        // for units, and the 1.12.1 client shows the plain hand over the
        // ball-and-chain clicked to start a quest. The bubble is used only for
        // the quest-giver type.
        Kind::QuestGiver => Cursor::Speak,
        Kind::AuctionHouse => Cursor::Buy,
        // Every other usable type gets the hand, the 1.12.1 client's default
        // pointer over a game object.
        _ => Cursor::Interact,
    })
}

/// The spell this character would cast at this lock, or `None` when the
/// character has no way to open it.
///
/// The rule is the one the server checks when the cast arrives: vmangos
/// `Spell::CanOpenLock` walks the lock's eight slots and, for each
/// `LOCK_KEY_SKILL`, compares `m_spellInfo->EffectMiscValue[effIdx]` with the
/// slot's `LockType`. The spell to send is one the character knows whose
/// open-lock misc value is a `LockType` this lock names.
///
/// A lock id of zero is treated as `LOCKTYPE_OPEN`. `CanOpenLock` returns
/// `SPELL_CAST_OK` for lock 0 whatever the spell, and every character is
/// created knowing `Opening`, so treating an unlocked chest as if its lock
/// named `Open` gives the right spell without a special case.
///
/// The rank is not chosen and does not matter; see
/// [`crate::tables::spellbook::Spells::open_lock_spells`].
///
/// The character's skill value is not checked here. A miner thirty points
/// short of a vein still sends the cast, and the server replies with
/// `ERR_USE_LOCKED_WITH_SPELL_S`. Checking locally would need
/// `PLAYER_SKILL_INFO_1_1`, and a refusal made by this client has no message
/// text, so the click would do nothing and show nothing.
pub fn opener(
    kind: Kind,
    locks: &Locks,
    lock_id: u32,
    spells: &crate::tables::spellbook::Spells,
    knows: impl Fn(u32) -> bool,
) -> Option<u32> {
    if !kind.opened_by_spell() {
        return None;
    }
    // Every `LockType` this lock names, plus `Open` for a lock that names none.
    let mut wanted: Vec<u32> = locks
        .keys(lock_id)
        .iter()
        .filter_map(|key| match key.kind {
            crate::tables::lock::KeyKind::Skill { lock_type, .. } => Some(lock_type),
            crate::tables::lock::KeyKind::Item(_) => None,
        })
        .collect();
    // `Open` is always appended last: it is the answer for a lock that names
    // no skill, and the fallback for a lock whose skills no spell opens.
    //
    // The fallback applies to four lock types: `Treasure (DND)`,
    // `Calcified Elven Gems (DND)`, `Gahz'ridian (DND)` and `Fishing` have no
    // open-lock spell anywhere in `Spell.dbc` (`vale objects` counts them).
    // The server refuses `Opening` cast at one of those with
    // `Spell::CanOpenLock`'s `ERR_USE_LOCKED_WITH_SPELL_S`, which the player
    // can read; returning `None` here would make the click do nothing and
    // show nothing. This module errs the same way everywhere.
    wanted.push(lock_type::OPEN);
    wanted
        .into_iter()
        .flat_map(|lock_type| spells.open_lock_spells(lock_type).iter().copied())
        .find(|spell| knows(*spell))
}

/// The `AnimationData.dbc` id a game object plays for
/// `SMSG_GAMEOBJECT_CUSTOM_ANIM` with `anim` 0..3: `Custom0`..`Custom3`, ids
/// 153..156. The fishing bobber (`World\Goober\G_FishingBobber.m2`) carries
/// `Custom0` and plays it when a fish bites. `None` for an `anim` of 4 or more,
/// which the 1.12.1 client ignores.
pub fn custom_anim(anim: u8) -> Option<u16> {
    (anim < 4).then(|| CUSTOM0_ANIM + u16::from(anim))
}

/// `Custom0` in `AnimationData.dbc`; see [`custom_anim`].
pub const CUSTOM0_ANIM: u16 = 153;

/// `Despawn` in `AnimationData.dbc`: what `SMSG_GAMEOBJECT_DESPAWN_ANIM` plays.
pub const DESPAWN_ANIM: u16 = 157;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::lock::{Key, KeyKind};

    /// A `Locks` built by hand, so the rule can be tested without the file.
    fn locks(entries: &[(u32, KeyKind)]) -> Locks {
        let mut built = Locks::default();
        for (id, kind) in entries {
            built = built.with_lock(*id, vec![Key { kind: *kind, slot: 0, action: 0 }]);
        }
        built
    }

    /// An ore vein and a strongbox have the same type and must draw different
    /// pointers; the lock separates them.
    #[test]
    fn the_lock_separates_a_vein_from_a_chest() {
        let table = locks(&[
            (
                38,
                KeyKind::Skill {
                    lock_type: lock_type::MINING,
                    rank: 0,
                },
            ),
            (
                29,
                KeyKind::Skill {
                    lock_type: lock_type::HERBALISM,
                    rank: 0,
                },
            ),
            (
                2,
                KeyKind::Skill {
                    lock_type: lock_type::PICK_LOCK,
                    rank: 25,
                },
            ),
        ]);
        assert_eq!(over_object(Kind::Chest, &table, 38), Some(Cursor::Mine));
        assert_eq!(
            over_object(Kind::Chest, &table, 29),
            Some(Cursor::GatherHerbs)
        );
        assert_eq!(over_object(Kind::Chest, &table, 2), Some(Cursor::PickLock));
        assert_eq!(
            over_object(Kind::Chest, &table, 0),
            Some(Cursor::Interact),
            "an unlocked chest is the plain hand"
        );
    }

    /// A door's lock is `data[1]` and a chest's is `data[0]`; reading the wrong
    /// word fails without an error. The door template is the Deadmines'
    /// Factory Door as the world database has it: `startOpen` 0, `lockId` 85.
    #[test]
    fn a_doors_lock_is_the_second_word_and_a_chests_is_the_first() {
        let door = [0u32, 85, 0, 0];
        let chest = [29u32, 1414, 0, 1];
        assert_eq!(Kind::Door.lock_word(), Some(1));
        assert_eq!(door[Kind::Door.lock_word().unwrap()], 85);
        assert_eq!(Kind::Chest.lock_word(), Some(0));
        assert_eq!(chest[Kind::Chest.lock_word().unwrap()], 29);
        // A type with no lock returns `None` rather than zero, so a caller
        // cannot look up lock 0 by mistake and get a plausible "no lock".
        assert_eq!(Kind::Mailbox.lock_word(), None);
        assert_eq!(Kind::Other(5).lock_word(), None);
    }

    /// A street sign is hovered but never clicked, which is why usability and
    /// hover are separate questions.
    ///
    /// The template is the Goldshire crossroads sign as the world database has
    /// it: type 5, `floatingTooltip` 1, `highlight` 1. There is one game object
    /// per arm of the signpost, each named for where the arm points.
    #[test]
    fn a_sign_is_worth_pointing_at_and_cannot_be_used() {
        let sign = [1u32, 1, 0, 0];
        assert!(!Kind::Generic.usable(), "the server refuses the type outright");
        assert!(worth_pointing_at(Kind::Generic, &sign), "…and it is still hovered");
        assert_eq!(
            hover_of(Kind::Generic, &sign),
            Hover { floating: true, highlight: true },
            "its plate follows the pointer, and it lights up"
        );
        // The 468 inert generic objects are excluded from the pick, so they
        // cannot clear the hover of a unit behind them.
        let scenery = [0u32, 0, 0, 0];
        assert!(!worth_pointing_at(Kind::Generic, &scenery));
        assert_eq!(hover_of(Kind::Generic, &scenery), Hover::default());
        // A template with `highlight` set and no tooltip is excluded too; see
        // `hover_of`, which documents the deviation.
        let glow_only = [0u32, 1, 0, 0];
        assert!(!worth_pointing_at(Kind::Generic, &glow_only));
        assert!(!hover_of(Kind::Generic, &glow_only).highlight);
    }

    /// A usable object is highlighted and its tooltip is anchored like a
    /// unit's. This is the non-generic branch of [`hover_of`] and needs no
    /// template.
    #[test]
    fn a_usable_object_lights_up_and_its_plate_does_not_float() {
        for kind in [Kind::Door, Kind::Chest, Kind::Mailbox, Kind::Chair] {
            let hover = hover_of(kind, &[]);
            assert!(hover.highlight, "{kind:?} lights up");
            assert!(!hover.floating, "{kind:?}'s plate goes where a unit's goes");
            assert!(worth_pointing_at(kind, &[]), "{kind:?} is in the pick");
        }
        // A trap and an anvil are neither, with or without a template.
        for kind in [Kind::Trap, Kind::Other(8), Kind::Other(11)] {
            assert_eq!(hover_of(kind, &[1, 1, 1, 1]), Hover::default(), "{kind:?}");
            assert!(!worth_pointing_at(kind, &[1, 1, 1, 1]), "{kind:?}");
        }
    }

    /// Scenery, which is most of the game objects in the world, draws no
    /// pointer; see [`Kind::usable`].
    #[test]
    fn the_scenery_is_not_clickable() {
        let table = Locks::default();
        for inert in [Kind::Generic, Kind::Other(8), Kind::Other(11), Kind::Trap] {
            assert_eq!(over_object(inert, &table, 0), None, "{inert:?}");
            assert!(!inert.usable(), "{inert:?}");
        }
        assert!(Kind::Door.usable());
        assert!(Kind::Chest.usable());
    }

    /// A chest is opened by a spell cast and a door by the use packet. The
    /// lock, not the type, decides which spell.
    #[test]
    fn a_chest_takes_a_spell_and_a_door_takes_the_use_packet() {
        assert!(Kind::Chest.opened_by_spell());
        assert!(Kind::FishingHole.opened_by_spell());
        for used in [Kind::Door, Kind::Button, Kind::Goober, Kind::Mailbox, Kind::Chair] {
            assert!(!used.opened_by_spell(), "{used:?}");
        }
    }

    /// The spell is chosen from the lock and the spells the character knows.
    ///
    /// The `Opening` assertions cover two cases: an unlocked chest gets
    /// `Opening` through the ordinary path rather than a special case, and a
    /// lock the character has no skill for still sends a cast, so the server
    /// replies with a refusal message.
    #[test]
    fn the_lock_and_the_book_between_them_choose_the_spell() {
        use crate::tables::spellbook::Spells;

        // A three-row `Spell.dbc`: Herb Gathering, Mining and Opening, each an
        // `SPELL_EFFECT_OPEN_LOCK_ITEM` (33) with its own misc value, as the
        // shipped file has for 2366, 2575 and 3365.
        let spells = Spells::parse(
            &spell_dbc(&[
                (2366, 33, lock_type::HERBALISM),
                (2575, 33, lock_type::MINING),
                (3365, 33, lock_type::OPEN),
            ]),
            &[], &[], &[], &[], &[], &[],
        )
        .expect("the stub table parses");

        let table = locks(&[
            (29, KeyKind::Skill { lock_type: lock_type::HERBALISM, rank: 0 }),
            (38, KeyKind::Skill { lock_type: lock_type::MINING, rank: 0 }),
            (99, KeyKind::Skill { lock_type: lock_type::TREASURE, rank: 0 }),
        ]);
        let herbalist = |spell: u32| spell == 2366 || spell == 3365;

        assert_eq!(opener(Kind::Chest, &table, 29, &spells, herbalist), Some(2366));
        assert_eq!(
            opener(Kind::Chest, &table, 38, &spells, herbalist),
            Some(3365),
            "a miner's vein to a herbalist falls through to Opening, which the \
             server refuses in words"
        );
        assert_eq!(
            opener(Kind::Chest, &table, 0, &spells, herbalist),
            Some(3365),
            "an unlocked chest is Opening, through the ordinary path"
        );
        assert_eq!(
            opener(Kind::Chest, &table, 99, &spells, herbalist),
            Some(3365),
            "…and so is a lock type nothing in the file opens"
        );
        assert_eq!(
            opener(Kind::Door, &table, 29, &spells, herbalist),
            None,
            "a door is never cast at, whatever its lock says"
        );
        assert_eq!(
            opener(Kind::Chest, &table, 29, &spells, |_| false),
            None,
            "a character who knows nothing sends nothing"
        );
    }

    /// A `Spell.dbc` with just the columns [`opener`] reads.
    fn spell_dbc(rows: &[(u32, u32, u32)]) -> Vec<u8> {
        use crate::tables::spellbook::spell_fields;
        let fields = 173;
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for (id, effect, misc) in rows {
            let mut record = vec![0u32; fields];
            record[0] = *id;
            record[spell_fields::EFFECT] = *effect;
            record[spell_fields::EFFECT_MISC_VALUE] = *misc;
            for word in record {
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
        out.push(0);
        out
    }

    /// Each of the three tested update-field flags on its own. They separate a
    /// chest this character may open from one that belongs to someone else.
    ///
    /// Every case is a `Kind::Chest` with an ordinary template, so a decision
    /// made from the type alone drew the interact hand over all four.
    #[test]
    fn the_objects_own_flags_decide_whether_a_click_is_worth_sending() {
        assert!(interactable(Kind::Chest, 0, 0), "the plain case");
        assert!(
            !interactable(Kind::Chest, go_flags::IN_USE, 0),
            "mid-animation: the server drops the packet"
        );
        assert!(
            !interactable(Kind::Chest, go_flags::NO_INTERACT, 0),
            "switched off outright"
        );
        assert!(
            !interactable(Kind::Chest, go_flags::INTERACT_COND, 0),
            "conditional, and the server has not said yes"
        );
        assert!(
            interactable(Kind::Chest, go_flags::INTERACT_COND, go_dyn_flags::ACTIVATE),
            "…and it has"
        );
        // `LOCKED` is not one of the tested flags: a locked door is still
        // clicked, and the server replies with a message.
        assert!(interactable(Kind::Door, go_flags::LOCKED, 0));
        // The type is tested first, whatever the flags say.
        assert!(!interactable(Kind::Generic, 0, go_dyn_flags::ACTIVATE));
    }

    /// The Food Crate case: a lock whose only key is in column 1 produces no
    /// requirement line.
    #[test]
    fn a_key_outside_column_zero_is_not_a_requirement() {
        let mut built = Locks::default();
        built = built.with_lock(
            43,
            vec![Key {
                kind: KeyKind::Skill { lock_type: 13, rank: 0 },
                slot: 1,
                action: 0,
            }],
        );
        assert_eq!(
            requirement_of(&built, 43, 1, false),
            None,
            "the reference reads column 0 and the crate has none, so it says nothing"
        );
        // The same key in column 0 does produce one.
        let mut built = Locks::default();
        built = built.with_lock(
            43,
            vec![Key {
                kind: KeyKind::Skill { lock_type: 13, rank: 0 },
                slot: 0,
                action: 0,
            }],
        );
        assert_eq!(
            requirement_of(&built, 43, 1, false),
            Some(Requirement::Skill { lock_type: 13, rank: 0 })
        );
        // The action test still applies: action 0 requires an object that is
        // shut and not locked, so a locked one shows nothing.
        assert_eq!(requirement_of(&built, 43, 1, true), None);
    }

    /// Each band of the difficulty ramp. The colour is how the requirement line
    /// shows the character's skill against the lock's rank.
    #[test]
    fn the_requirement_is_coloured_by_the_rank_against_your_own() {
        assert_eq!(difficulty_colour(225, 125), difficulty::TRIVIAL);
        assert_eq!(difficulty_colour(175, 125), difficulty::EASY);
        assert_eq!(difficulty_colour(150, 125), difficulty::MEDIUM);
        assert_eq!(difficulty_colour(125, 125), difficulty::HARD);
        assert_eq!(difficulty_colour(124, 125), difficulty::IMPOSSIBLE);
        // A rank of zero with no skill is `HARD`, not `TRIVIAL`. The first band
        // requires `have >= need + 100`, so a lock that requires nothing, held
        // against a character with no skill in that line, comes out orange.
        // That is the usual colour of "Requires Open" in the 1.12.1 client.
        assert_eq!(difficulty_colour(0, 0), difficulty::HARD);
    }

    /// The way in and whether it is enough. Together they decide the tooltip
    /// line's colour and whether the click is sent.
    #[test]
    fn the_best_way_in_is_found_and_graded() {
        let mut built = Locks::default();
        built = built.with_lock(
            41,
            vec![Key {
                kind: KeyKind::Skill { lock_type: lock_type::MINING, rank: 125 },
                slot: 0,
                action: 0,
            }],
        );
        // A miner thirty points short: the way in is found and is not enough,
        // so the line is drawn in red instead of being hidden.
        let short = can_open(&built, 41, 1, false, 0, |_| Some((2575, 95)), |_| false)
            .expect("the slot matched");
        assert_eq!(short.spell, Some(2575));
        assert_eq!((short.have, short.need), (95, 125));
        assert!(!short.enough);
        // A miner with enough skill.
        let ample = can_open(&built, 41, 1, false, 0, |_| Some((2575, 150)), |_| false)
            .expect("the slot matched");
        assert!(ample.enough);
        // A character with no mining spell at all finds nothing.
        assert_eq!(can_open(&built, 41, 1, false, 0, |_| None, |_| false), None);
        // The action test rules the slot out for a locked object.
        assert_eq!(can_open(&built, 41, 1, true, 0, |_| Some((2575, 150)), |_| false), None);

        // A key in the bags is always enough and has no grade.
        let mut keyed = Locks::default();
        keyed = keyed.with_lock(
            36,
            vec![Key { kind: KeyKind::Item(3467), slot: 0, action: 0 }],
        );
        let held = can_open(&keyed, 36, 1, false, 0, |_| None, |entry| entry == 3467)
            .expect("the key is in the bags");
        assert_eq!((held.item, held.spell, held.enough), (Some(3467), None, true));
        assert_eq!(can_open(&keyed, 36, 1, false, 0, |_| None, |_| false), None);
    }

    /// A slot with rank 0 requires five times the object's level. The level is
    /// nearly always zero because vmangos writes `GAMEOBJECT_LEVEL` only for
    /// transports.
    #[test]
    fn a_slot_wanting_no_rank_falls_back_to_five_times_the_level() {
        let mut built = Locks::default();
        built = built.with_lock(
            29,
            vec![Key {
                kind: KeyKind::Skill { lock_type: lock_type::HERBALISM, rank: 0 },
                slot: 0,
                action: 0,
            }],
        );
        let way = can_open(&built, 29, 1, false, 0, |_| Some((2366, 1)), |_| false).unwrap();
        assert_eq!(way.need, 0, "no level, so anybody may");
        assert!(way.enough);
        let way = can_open(&built, 29, 1, false, 12, |_| Some((2366, 1)), |_| false).unwrap();
        assert_eq!(way.need, 60, "…and a level of 12 wants 60");
        assert!(!way.enough);
    }

    /// The four types with their own pointer, so that a change to the default
    /// branch cannot silently turn one of them back into the hand.
    #[test]
    fn the_named_types_keep_their_own_pointers() {
        let table = Locks::default();
        assert_eq!(over_object(Kind::Mailbox, &table, 0), Some(Cursor::Mail));
        assert_eq!(over_object(Kind::QuestGiver, &table, 0), Some(Cursor::Speak));
        assert_eq!(
            over_object(Kind::AuctionHouse, &table, 0),
            Some(Cursor::Buy)
        );
        assert_eq!(over_object(Kind::Door, &table, 0), Some(Cursor::Interact));
    }
}
