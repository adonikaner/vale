//! **What a game object is *for***, which decides whether a click on it does
//! anything and which pointer goes over it.
//!
//! The door in the Deadmines, the ore vein on the wall beside it, the mailbox in
//! Sentinel Hill and the campfire nobody can touch are all the same kind of
//! thing to this client: an entity whose `ObjectType` is `GameObject`, with a
//! display id, an open/shut state, and a name that arrived by
//! `CMSG_GAMEOBJECT_QUERY`. Nothing in that says which of them is worth
//! pointing at.
//!
//! What says it is the **template**: one type byte and twenty-four words of
//! union, both of which come back in `SMSG_GAMEOBJECT_QUERY_RESPONSE` and
//! neither of which the packet interprets. This module is the interpretation,
//! and it lives in `assets` because every question it
//! answers is decidable with no renderer running, and `vale objects` calls
//! the same copy the renderer calls.
//!
//! ## The type table is the server's, and that is the right authority here
//!
//! `GAMEOBJECT_TYPE_*` is the shared vocabulary between the two halves: the
//! client learns a type only because the server sent one, and what a
//! `CMSG_GAMEOBJ_USE` *does* is decided by `GameObject::Use`'s switch over
//! exactly this enum. So keying the pointer on it is not a reconstruction of the
//! client's own chain — it is a reading of the same table both ends already
//! agree on. **The reference's own cursor chain was not matched**, and where
//! the two could differ this errs towards
//! *offering* the click: a hand over something inert costs a packet the server
//! ignores, where a missing hand is a door the player cannot open.
//!
//! ## Which word is the lock is per-type, and getting it wrong is silent
//!
//! A door keeps its lock in `data[1]` and a chest keeps it in `data[0]`, because
//! a door's first word is `startOpen`. A reader that took `data[0]` for both
//! would give every door in the game the lock id 0 or 1 — "no lock" and
//! "lock 1" — and every Deadmines door would draw as an ordinary openable one.
//! That is the failure this module is shaped to avoid: [`Kind::lock_word`] is
//! the single place the offset is chosen, and a type with no lock says so by
//! answering `None` rather than by defaulting to zero.
//!
//! ## A chest is not opened by the packet that opens a door
//!
//! This is the finding of the round and it is not guessable from the opcode
//! names. `CMSG_GAMEOBJ_USE` reaches `GameObject::Use`, whose whole
//! `GAMEOBJECT_TYPE_CHEST` arm is
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
//! — a script and a linked trap, and **no loot at all**. The only thing in
//! vmangos that opens a chest's loot is `Spell::EffectOpenLock`, which ends in
//! `SendLoot(guid, LOOT_SKINNING, LockType(m_spellInfo->EffectMiscValue[effIdx]))`.
//! So a chest, an ore vein and a herb are opened by **casting a spell at the
//! object** — `CMSG_CAST_SPELL` with `TARGET_FLAG_GAMEOBJECT` — and which spell
//! is decided by the lock. [`opener`] is that decision; [`Kind::opened_by_spell`]
//! is which types take that route.
//!
//! **Measured, not reasoned about.** Sending `CMSG_GAMEOBJ_USE` at a herb node
//! on a live server with 300 Herbalism produced nothing at all — no loot, no
//! state change, no refusal — which is exactly what the arm above does.
//!
//! **And an unlocked chest is not a special case.** Every character in the game
//! is created knowing `Opening` (3365, and its two siblings 21651 and 22810),
//! whose open-lock misc value is `LOCKTYPE_OPEN` — so the same rule that finds
//! Mining for a vein finds `Opening` for a chest with nothing on it, and there
//! is no fallback branch to get wrong. That is the world database's own
//! `playercreateinfo_spell`, checked on the running server rather than assumed.
//!
//! ## A street sign is a game object that can only be *looked* at
//!
//! `GAMEOBJECT_TYPE_GENERIC` is the type the world database has 1,869 of, and
//! it is not scenery: **1,197 of them are the signposts, the plaques and the
//! markers whose whole purpose is that hovering one tells you where you are.**
//! The Goldshire crossroads is six of them — one game object per arm, each
//! named for where the arm points, all on display id 26.
//!
//! Two things make them their own case, and both are in the template:
//!
//! ```text
//! 5 GAMEOBJECT_TYPE_GENERIC   data[0] floatingTooltip   data[1] highlight
//!                             data[2] serverOnly        data[3] large
//! ```
//!
//! * **`floatingTooltip` is the plate, and it *floats*** — it follows the
//!   pointer instead of sitting in the screen's corner where a unit's plate
//!   goes. That is the reported difference and it is the flag's own name.
//! * **and nothing can be *done* to one.** `HandleGameObjectUseOpcode` refuses
//!   the type outright — `if (obj->GetGoType() == GAMEOBJECT_TYPE_GENERIC)
//!   return;` — before any of its other checks, so a click is a packet the
//!   server drops on the floor.
//!
//! So **"can be clicked" and "can be hovered" are two questions**, and this
//! module answers them separately: [`Kind::usable`] is the first and
//! [`hover_of`] is the second. Conflating them is what left the signs inert —
//! they were filtered out of the pick as scenery, which is what 468 of the
//! 1,869 genuinely are.
//!
//! ## What is deliberately *not* here
//!
//! Whether **this** character may open **this** lock. That needs the skill the
//! character has, the key in their bag and the quest in their log, and the
//! server checks all three anyway — a refusal comes back as `ERR_USE_LOCKED_*`
//! off the message table. The pointer says what the thing is; the server says
//! whether you may.

use crate::tables::lock::{action_applies, lock_type, KeyKind, Locks};

use super::cursor::Cursor;

/// `GAMEOBJECT_TYPE`, the values `GameObjectInfo::type` takes.
///
/// Only the ones this client has an opinion about are named; everything else is
/// [`Kind::Other`] and draws nothing. The numbering is the server's own enum and
/// is unchanged from 1.12 through to modern builds for every row here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// 0 — a door. `data[0]` startOpen, `data[1]` lockId.
    Door,
    /// 1 — a lever, a valve, a button. Same first two words as a door.
    Button,
    /// 2 — a game object that hands out quests. `data[0]` lockId.
    QuestGiver,
    /// 3 — a chest, **and every ore vein and herb in the game**: the difference
    /// is the lock. `data[0]` lockId, `data[1]` lootId.
    Chest,
    /// 5 — the signposts, the plaques and the markers. **Never usable** and
    /// often hoverable; see the module note and [`hover_of`]. `data[0]`
    /// floatingTooltip, `data[1]` highlight.
    ///
    /// **The default**, because it is the one type that does nothing at all: a
    /// caller with no answer yet must not be holding a door or a chest.
    #[default]
    Generic,
    /// 6 — a trap. Not clickable; named so that its absence from
    /// [`Kind::usable`] is on purpose rather than an oversight.
    Trap,
    /// 12 — an area-damage volume. Not clickable, and named for the same
    /// reason [`Kind::Trap`] is: it carries a lock and it is not a thing a
    /// player touches.
    AreaDamage,
    /// 13 — a camera. Same.
    Camera,
    /// 7 — a chair. Clicking one sits the character in it.
    Chair,
    /// 9 — a sign, a plaque, a tombstone: `SMSG_GAMEOBJECT_PAGETEXT`.
    Text,
    /// 10 — the catch-all "do something" object: a brazier to light, a lever
    /// with a quest behind it, a book on a stand. `data[0]` lockId.
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
    /// Everything else: the invisible zone markers, the spell focuses, the
    /// scenery, the transports.
    Other(u32),
}

impl Kind {
    /// The type byte, read.
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

    /// **Which two words of the template's union hold a page**, as
    /// `(pageId, pageMaterial)`, or `None` for a type that can carry none.
    ///
    /// Two types can, and they keep the pair at different offsets — the same
    /// shape, and the same hazard, as [`Self::lock_word`]. `GameObjectDefines.h`
    /// spells both unions out:
    ///
    /// ```text
    /// 9  TEXT     data[0] pageID  data[1] language  data[2] pageMaterial  data[3] allowMounted
    /// 10 GOOBER   … data[7] pageId  data[8] language  data[9] pageMaterial …
    /// ```
    ///
    /// A `pageId` of zero is the ordinary case for a goober — a brazier, a
    /// lever — and means there is nothing to read, which is the caller's test
    /// rather than this function's.
    ///
    /// **The two are reached differently and that is the server's doing.** A
    /// goober with a page is announced: `GameObject::Use` sends
    /// `SMSG_GAMEOBJECT_PAGETEXT` with the guid and nothing else
    /// (`GameObject.cpp:1551`), leaving the client to find the page id here. A
    /// `TEXT` object has no `Use` case at all, so nothing is sent and the client
    /// opens it off this template by itself. Both end at the same query; see
    /// [`vale_protocol::play::pagetext`].
    pub fn page_words(self) -> Option<(usize, usize)> {
        match self {
            Kind::Text => Some((0, 2)),
            Kind::Goober => Some((7, 9)),
            _ => None,
        }
    }

    /// **Which word of the template's union holds this type's lock id**, or
    /// `None` for a type that has no lock at all.
    ///
    /// See the module note: this is the single place the offset is chosen, and
    /// the door-against-chest difference is why it exists as a function rather
    /// than as `data[0]` at the call site.
    pub fn lock_word(self) -> Option<usize> {
        Some(match self {
            // A door's and a button's first word is `startOpen`.
            Kind::Door | Kind::Button => 1,
            Kind::QuestGiver | Kind::Chest | Kind::Goober | Kind::FlagStand | Kind::FlagDrop => 0,
            // …and the three the world's own data almost never shows, taken from
            // the client's table rather than inferred: a trap's first word is
            // its lock, and so are an area-damage volume's and a camera's.
            // None of the three is [`Self::usable`], so nothing in this client
            // asks — they are here because the table answered and a `None`
            // would have been a wrong answer rather than a missing one.
            Kind::Trap | Kind::AreaDamage | Kind::Camera => 0,
            // `radius`, `lootId`, `minSuccessOpens`, `maxSuccessOpens`, then the
            // lock — the one type whose lock is not in the first two words.
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

    /// **Is this opened by casting at it rather than by `CMSG_GAMEOBJ_USE`?**
    ///
    /// The two types whose whole purpose is loot — see the module note, where
    /// the server arm that does nothing is quoted. Everything else, doors and
    /// levers and chairs and mailboxes included, goes out as the use packet;
    /// a **door**'s lock is checked by the server on that path
    /// (`GameObject::PlayerCanUse` walks its `LOCK_KEY_ITEM` slots), so a
    /// locked door is not a reason to cast at one.
    pub fn opened_by_spell(self) -> bool {
        matches!(self, Kind::Chest | Kind::FishingHole)
    }

    /// **Is a click on this worth sending?**
    ///
    /// `false` is the pointer staying an arrow and the button doing nothing —
    /// which is what the great majority of game objects in the world deserve:
    /// the 1,869 invisible zone markers, the 2,332 spell focuses, the 2,297
    /// scenery pieces and every transport are all things a player walks past.
    ///
    /// A trap is deliberately absent, and so is every `Other`: see the module
    /// note on which direction the error goes.
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

/// `GAMEOBJECT_FLAGS` — the bits of the game object's own field that decide
/// whether a click on it does anything.
///
/// The template says what a thing *is*; this word says what it is doing right
/// now, and the server changes it during a session. A door that is mid-swing,
/// a chest somebody else is standing over and an event object that is not
/// running yet are all perfectly ordinary templates with a bit set here.
///
/// Named rather than written as literals because two of the three are read
/// together and the third is read against a *different* word — see
/// [`interactable`].
pub mod go_flags {
    /// The thing is animating. `GameObjectDefines.h`: "disables interaction
    /// while animated".
    pub const IN_USE: u32 = 0x0000_0001;
    /// **"Locked" goes on the plate.** The server's own comment on the bit says
    /// exactly that, and the reference's tooltip is where it happens.
    pub const LOCKED: u32 = 0x0000_0002;
    /// **Ask before letting this character touch it** — the answer is
    /// [`go_dyn_flags::ACTIVATE`] in the private half of the update block.
    pub const INTERACT_COND: u32 = 0x0000_0004;
    /// Nobody may touch it at all.
    pub const NO_INTERACT: u32 = 0x0000_0010;
}

/// `GAMEOBJECT_DYN_FLAGS` — the server's per-player answer to
/// [`go_flags::INTERACT_COND`].
pub mod go_dyn_flags {
    /// **This character may act on it.** Tested only when `INTERACT_COND` is
    /// set.
    pub const ACTIVATE: u32 = 0x0000_0001;
}

/// `GAMEOBJECT_STATE`'s second used value — a destroyed thing.
///
/// Its own constant because it is the one state that makes a click *refuse*
/// rather than merely do something different: the client answers
/// `ERR_USE_DESTROYED` for it.
pub const STATE_DESTROYED: u8 = 2;

/// **Is a click on this worth sending right now?** — [`Kind::usable`] plus the
/// three tests the reference makes on the object's own live flags.
///
/// ```text
/// IN_USE | NO_INTERACT set                     -> no
/// INTERACT_COND set and ACTIVATE clear         -> no
/// otherwise                                    -> the type decides
/// ```
///
/// **The type alone is not enough and that is the whole reason this exists.**
/// A chest that is somebody else's kill, a door that is already swinging and
/// every event object standing inert between events are all `Kind::Chest` and
/// `Kind::Door` with perfectly ordinary templates; what separates them from the
/// ones a player may touch is this word and nothing else. Judging usability off
/// the type put a hand on the pointer over all of them and sent a packet the
/// server drops.
///
/// **What is deliberately not here** is the reference's fourth test, the
/// creator unit's reaction to the player: a world game object has
/// no creator, so it decides nothing for anything a player meets outside a
/// battleground, and reading it would mean resolving a guid this crate has no
/// access to.
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

/// **What the plate says it takes to open this**, or `None` for nothing at all.
///
/// This is `Lock.dbc` column **0**, gated by that column's `Action` against the
/// thing's current state — and it is narrow on purpose. See
/// [`Locks::first_slot`], where the reference's own two instructions are
/// quoted, and [`action_applies`], where the gate is.
///
/// `None` is the overwhelmingly common answer, and three different things
/// produce it: a type with no lock word at all, a lock id of 0, and — the case
/// this was written for — a lock whose only key is in a column the reference
/// never looks at.
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
/// `state` is `GAMEOBJECT_STATE` and `locked` is [`go_flags::LOCKED`]; both are
/// live fields rather than template words, because the gate is about what the
/// thing is doing now. A strongbox that has already been picked stops saying
/// "Requires Pick Lock" the moment the flag clears, which is the action column
/// doing its job.
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

/// **The five colours the reference grades a requirement with**, as linear
/// `[r, g, b]` in 0..1.
///
/// The client grades `(need, have)` into one of five colours, each quoted
/// beside it as ARGB. It is the gathering-difficulty ramp — the same one a
/// mining node's name is drawn in — and it is what makes "Requires Mining" a
/// statement about *you* rather than about the vein.
pub mod difficulty {
    /// `0xff808080`, `have >= need + 100`.
    pub const TRIVIAL: [f32; 3] = [0.5, 0.5, 0.5];
    /// `0xff40c040`, `have >= need + 50`. **Not** the interface's
    /// `GREEN_FONT_COLOR`, which is `0/1/0`.
    pub const EASY: [f32; 3] = [0.25, 0.75, 0.25];
    /// `0xffffff00`, `have >= need + 25`.
    pub const MEDIUM: [f32; 3] = [1.0, 1.0, 0.0];
    /// `0xffff8040`, `have >= need`.
    pub const HARD: [f32; 3] = [1.0, 0.5, 0.25];
    /// `0xffff2020`, `have < need` — the same red `RED_FONT_COLOR_CODE` is.
    pub const IMPOSSIBLE: [f32; 3] = [1.0, 32.0 / 255.0, 32.0 / 255.0];
}

/// [`difficulty`], chosen — in the client's own order.
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

/// **The best way this character has into one lock**, and whether it is enough.
///
/// The answer to the client's lock check, which is the one function both
/// halves of this subject hang off: the plate's colour and whether the click
/// goes at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockWay {
    /// The open-lock spell the character knows for one of the lock's skill
    /// slots, or `None` when the way in was a key.
    pub spell: Option<u32>,
    /// …or the item entry they are carrying.
    pub item: Option<u32>,
    /// Their value in the skill line that spell belongs to, and the rank the
    /// slot wants. Both zero for an item way in.
    pub have: u32,
    pub need: u32,
    /// **Is this way in actually good enough?** `have >= need`, or an item
    /// held. `false` is a vein a miner is thirty points short of, and it is
    /// what the plate draws in red.
    pub enough: bool,
}

/// [`LockWay`] for one lock — the reference's `CanOpen`, slot by slot.
///
/// `None` means the lock names **no applicable way in at all**: an unlocked
/// thing (id 0), a lock the table does not carry, or one whose every slot is
/// ruled out by [`action_applies`] for the state this thing is in. That is the
/// reference's `false` return and it is *not* a refusal — `Opening` still
/// applies to an unlocked chest through the ordinary path (see [`opener`]).
///
/// The two closures are the half that is in no file. `by_skill` answers, for a
/// `LockType.dbc` row, the spell this character knows that opens it and their
/// value in that spell's skill line; `by_item` answers whether they are
/// carrying an entry. `level` is `GAMEOBJECT_LEVEL`, which is the rank a slot
/// wanting **0** falls back to five times over — vmangos writes
/// that field for transports and for nothing else, so in practice it is zero
/// and a rank-0 slot is met by anybody.
///
/// **The first satisfiable slot wins, and a slot that matches but falls short
/// is still remembered** — that is the reference's own shape, where the out
/// params are written on every matching slot and the walk stops only on a
/// success. It is what lets the plate say "Requires Mining" in red rather than
/// saying nothing.
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

/// **What hovering one is worth**, out of the template — which is a different
/// question from whether it can be clicked.
///
/// See the module note. Both flags are `GAMEOBJECT_TYPE_GENERIC`'s own; every
/// other type answers them from what it *is*, because a thing you can open is a
/// thing worth lighting up and its plate goes where a unit's goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hover {
    /// **The plate follows the pointer** instead of sitting in the corner of
    /// the screen. `floatingTooltip`, and it is the whole of what makes a
    /// street sign feel different from a chest.
    pub floating: bool,
    /// The model lights up under the pointer.
    pub highlight: bool,
}

/// [`Hover`] for one template.
///
/// `data` is the raw 24-word union out of `SMSG_GAMEOBJECT_QUERY_RESPONSE`; a
/// short slice reads as zeroes, which is the "not resolved yet" answer and is
/// the right one — a game object with no template is not worth pointing at.
///
/// **A generic with `highlight` and no `floatingTooltip` reads as nothing here,
/// and that is a deviation.** 189 rows are shaped that way, and taking them at
/// face value would mean a model that glows under the pointer, says nothing and
/// does nothing — and, worse, that wins the depth test over a *unit* standing
/// behind it, which is the exact fault the sign filter was added for. Erring
/// towards not-picked costs a glow; erring the other way costs the mob.
pub fn hover_of(kind: Kind, data: &[u32]) -> Hover {
    let word = |n: usize| data.get(n).copied().unwrap_or(0) != 0;
    match kind {
        Kind::Generic => {
            let floating = word(0);
            Hover {
                floating,
                // Only alongside the plate — see this function's own note.
                highlight: floating && word(1),
            }
        }
        // Everything a click does something to is worth lighting, and its plate
        // is a unit's: anchored where `GameTooltip_SetDefaultAnchor` puts one.
        kind if kind.usable() => Hover {
            floating: false,
            highlight: true,
        },
        _ => Hover::default(),
    }
}

/// **Is this worth putting in the pick at all?**
///
/// The union of the two questions, and the one thing that decides whether the
/// ray even considers a game object: something a click acts on, or something a
/// hover says. Everything else — the spell focuses that are the anvils and
/// forges, the transports, the traps, the 468 inert generics — stays out, and
/// staying out matters because a game object that wins the depth test **clears
/// the unit hover behind it**.
pub fn worth_pointing_at(kind: Kind, data: &[u32]) -> bool {
    kind.usable() || hover_of(kind, data).floating
}

/// **What the pointer shows over one game object**, or `None` for the arrow.
///
/// The lock is asked *first* and the type second, which is the order the whole
/// feature is about: an ore vein and a strongbox are both `Chest`, and the only
/// thing that separates them is that one's lock says Mining. A type-first rule
/// would draw a hand over every vein in the game.
///
/// `lock_id` is [`Kind::lock_word`]'s word out of the template — zero for a type
/// with no lock and for an unlocked one, which [`Locks::skill_lock`] answers
/// `None` to.
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
            // rows fall through to the type below: "the Opening spell will do
            // it" is not a thing a pointer has a picture for, and every quest
            // goober in the game is keyed on exactly that.
            _ => {}
        }
    }
    Some(match kind {
        // **A sign never reaches here** — `usable()` is false for `Generic` and
        // this function returns early on that. Named so the absence is on
        // purpose: the reference draws the ordinary arrow over a signpost, and
        // a hand over something no click can act on would be a pointer that
        // lies.
        Kind::Mailbox => Cursor::Mail,
        // A quest goober is *not* a speech bubble: 1.12's bubble is a unit's,
        // and the reference draws the plain hand over the ball-and-chain you
        // click to start a quest. The bubble is kept for the one type whose
        // whole purpose is a conversation.
        Kind::QuestGiver => Cursor::Speak,
        Kind::AuctionHouse => Cursor::Buy,
        // Everything else that can be clicked is the hand, which is the
        // reference's own default over a game object.
        _ => Cursor::Interact,
    })
}

/// **Which spell this character would cast at this lock**, or `None` for a lock
/// they have no way into.
///
/// The rule is one join and it is the same one the *server* checks on the way
/// back: `Spell::CanOpenLock` walks the lock's eight slots and, for each
/// `LOCK_KEY_SKILL`, compares `m_spellInfo->EffectMiscValue[effIdx]` against
/// the slot's `LockType`. So the spell to send is one the character knows whose
/// open-lock misc value is a `LockType` this lock names.
///
/// **A lock id of zero is `LOCKTYPE_OPEN`, not "no answer".** `CanOpenLock`
/// returns `SPELL_CAST_OK` outright for lock 0 whatever the spell is, and every
/// character is created knowing `Opening` — so treating an unlocked chest as if
/// its lock said `Open` gives the right spell through the ordinary path instead
/// of a special case.
///
/// **The rank is not chosen and does not matter** — see
/// [`crate::tables::spellbook::Spells::open_lock_spells`].
///
/// **The character's own skill is deliberately not checked here.** A vein a
/// miner is thirty points short of still gets the cast, and the server answers
/// with the game's own `ERR_USE_LOCKED_WITH_SPELL_S`. Judging it locally would
/// need `PLAYER_SKILL_INFO_1_1` and would err in the one direction that cannot
/// be recovered from: a refusal this client invented has no sentence attached
/// to it, so the click would do nothing and say nothing.
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
    // **`Open` last, always** — as the answer for a lock that names no skill,
    // and as the fallback for one whose skills nothing opens.
    //
    // That second case is real and it is four lock types: `Treasure (DND)`,
    // `Calcified Elven Gems (DND)`, `Gahz'ridian (DND)` and `Fishing` have no
    // open-lock spell in the whole of `Spell.dbc` (see `vale objects`, which
    // counts them). Sending `Opening` at one of those is refused by the server
    // with `Spell::CanOpenLock`'s own `ERR_USE_LOCKED_WITH_SPELL_S` — which is a
    // sentence the player can read, where returning `None` here would make the
    // click do nothing and say nothing. Same direction as every other choice in
    // this module.
    wanted.push(lock_type::OPEN);
    wanted
        .into_iter()
        .flat_map(|lock_type| spells.open_lock_spells(lock_type).iter().copied())
        .find(|spell| knows(*spell))
}

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

    /// **The whole point of the module in one assertion**: an ore vein and a
    /// strongbox are the same type and must not draw the same pointer.
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

    /// **A door's lock is `data[1]` and a chest's is `data[0]`**, which is the
    /// silent-failure this module's shape exists to stop. The template here is
    /// the Deadmines' Factory Door as the world database actually has it:
    /// `startOpen` 0, `lockId` 85.
    #[test]
    fn a_doors_lock_is_the_second_word_and_a_chests_is_the_first() {
        let door = [0u32, 85, 0, 0];
        let chest = [29u32, 1414, 0, 1];
        assert_eq!(Kind::Door.lock_word(), Some(1));
        assert_eq!(door[Kind::Door.lock_word().unwrap()], 85);
        assert_eq!(Kind::Chest.lock_word(), Some(0));
        assert_eq!(chest[Kind::Chest.lock_word().unwrap()], 29);
        // …and a type with no lock says so rather than answering zero, so a
        // caller cannot accidentally look up lock 0 and get a plausible "no".
        assert_eq!(Kind::Mailbox.lock_word(), None);
        assert_eq!(Kind::Other(5).lock_word(), None);
    }

    /// **A street sign is hovered and never clicked**, which is the pair of
    /// answers this module had to grow a second question for.
    ///
    /// The template is the Goldshire crossroads' own, as the world database has
    /// it: type 5, `floatingTooltip` 1, `highlight` 1 — one game object per arm
    /// of the signpost, each named for where the arm points.
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
        // …and the 468 inert ones stay out of the pick entirely, which is what
        // stops them clearing the unit hover behind them.
        let scenery = [0u32, 0, 0, 0];
        assert!(!worth_pointing_at(Kind::Generic, &scenery));
        assert_eq!(hover_of(Kind::Generic, &scenery), Hover::default());
        // …and so does the highlight-without-a-plate shape — see `hover_of`,
        // where the deviation is stated.
        let glow_only = [0u32, 1, 0, 0];
        assert!(!worth_pointing_at(Kind::Generic, &glow_only));
        assert!(!hover_of(Kind::Generic, &glow_only).highlight);
    }

    /// **Everything a click acts on lights up and anchors like a unit**, which
    /// is the other half of [`hover_of`] and needs no template at all.
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

    /// The scenery draws nothing, which is most of the game objects in the
    /// world — see [`Kind::usable`].
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

    /// **A chest is a cast and a door is a use**, which is the routing the whole
    /// round turned on — and the *lock* decides which spell, not the type.
    #[test]
    fn a_chest_takes_a_spell_and_a_door_takes_the_use_packet() {
        assert!(Kind::Chest.opened_by_spell());
        assert!(Kind::FishingHole.opened_by_spell());
        for used in [Kind::Door, Kind::Button, Kind::Goober, Kind::Mailbox, Kind::Chair] {
            assert!(!used.opened_by_spell(), "{used:?}");
        }
    }

    /// **Which spell**, out of the lock and the character's own book.
    ///
    /// The last two assertions are the pair that matters: an unlocked chest gets
    /// `Opening` through the ordinary path rather than through a special case,
    /// and a lock the character has no skill for still gets *something* sent, so
    /// the server can refuse it in words.
    #[test]
    fn the_lock_and_the_book_between_them_choose_the_spell() {
        use crate::tables::spellbook::Spells;

        // A three-row `Spell.dbc`: Herb Gathering, Mining and Opening, each an
        // `SPELL_EFFECT_OPEN_LOCK_ITEM` (33) with its own misc value — which is
        // exactly what the shipped file has for 2366, 2575 and 3365.
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

    /// **The three live flags, each on its own**, which is what separates a
    /// chest this character may open from one that is somebody else's.
    ///
    /// Every one of these is a `Kind::Chest` with an ordinary template, so a
    /// judgement made off the type alone put the interact hand over all four.
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
        // `LOCKED` is emphatically **not** one of them: a locked door is still
        // clicked, and the server answers in words.
        assert!(interactable(Kind::Door, go_flags::LOCKED, 0));
        // …and the type still decides first, whatever the flags say.
        assert!(!interactable(Kind::Generic, 0, go_dyn_flags::ACTIVATE));
    }

    /// **The Food Crate**, which is the report this round is about: a lock
    /// whose only key is in column 1 must produce no requirement line.
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
        // …and the same key in column 0 does produce one.
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
        // …and the action gate still applies to it: action 0 wants a thing that
        // is shut and not locked, so a locked one says nothing.
        assert_eq!(requirement_of(&built, 43, 1, true), None);
    }

    /// **The ramp, band by band**, because the whole of what the line says
    /// about *you* is its colour.
    #[test]
    fn the_requirement_is_coloured_by_the_rank_against_your_own() {
        assert_eq!(difficulty_colour(225, 125), difficulty::TRIVIAL);
        assert_eq!(difficulty_colour(175, 125), difficulty::EASY);
        assert_eq!(difficulty_colour(150, 125), difficulty::MEDIUM);
        assert_eq!(difficulty_colour(125, 125), difficulty::HARD);
        assert_eq!(difficulty_colour(124, 125), difficulty::IMPOSSIBLE);
        // **A rank of zero is `HARD`, not `TRIVIAL`**, which reads backwards
        // and is what the arithmetic says: the first band wants `have >= need
        // + 100`, so a lock wanting nothing from a character with no skill in
        // that line at all comes out orange. It is the ordinary colour of
        // "Requires Open" in the reference, and a ramp written to feel right
        // instead would have got it wrong.
        assert_eq!(difficulty_colour(0, 0), difficulty::HARD);
    }

    /// **The way in, and whether it is good enough** — the two answers that
    /// between them decide the plate's colour and whether the click goes.
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
        // A miner thirty points short: the way in is found, and it is not
        // enough — which is what draws the line in red instead of hiding it.
        let short = can_open(&built, 41, 1, false, 0, |_| Some((2575, 95)), |_| false)
            .expect("the slot matched");
        assert_eq!(short.spell, Some(2575));
        assert_eq!((short.have, short.need), (95, 125));
        assert!(!short.enough);
        // …and one who is not.
        let ample = can_open(&built, 41, 1, false, 0, |_| Some((2575, 150)), |_| false)
            .expect("the slot matched");
        assert!(ample.enough);
        // A character with no mining spell at all finds nothing.
        assert_eq!(can_open(&built, 41, 1, false, 0, |_| None, |_| false), None);
        // …and the action gate still rules the slot out for a locked thing.
        assert_eq!(can_open(&built, 41, 1, true, 0, |_| Some((2575, 150)), |_| false), None);

        // A key in the bags is flatly enough, with nothing to grade.
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

    /// **A rank of zero falls back to the object's level, five times over** —
    /// and it is nearly always zero because vmangos writes
    /// `GAMEOBJECT_LEVEL` for transports alone.
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

    /// The four types with a pointer of their own, so a later edit to the
    /// fall-through cannot quietly take one back to the hand.
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
