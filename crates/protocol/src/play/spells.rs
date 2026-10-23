//! The player's spellbook, action bar and cooldowns, and the server's answers
//! to a cast or a swing.
//!
//! [`crate::play::action`] covers what a unit is visibly doing: inbound
//! packets about any unit that drive an animation. This module covers the
//! rest: the spellbook the server sends at login, the action bar it stores
//! for the player, the cooldowns that gate the buttons, and the two packets
//! that answer a key press, `SMSG_CAST_RESULT` for a spell and the five
//! `SMSG_ATTACKSWING_*` refusals for a swing. It also holds the outbound
//! bodies, which are kept beside the subject they belong to.
//!
//! Three facts apply to the whole module.
//!
//! The failure code is an index, not a value. `SMSG_CAST_RESULT` carries one
//! byte, and it names a position in an enum the client and the server share.
//! [`CAST_FAILURE_KEYS`] is that enum in order. An off-by-one there does not
//! fail; it shows "Out of range" when the server said "Not enough mana". The
//! list has 146 entries for build 5875 and is copied from vmangos'
//! `SpellCastResult`. Every `#if` guard in that header is `> CLIENT_BUILD_1_x`
//! for an x below 1.12, so 1.12 includes every entry and declaration order is
//! the wire value. The 1.12.1 client also recognises 146 reasons, so two
//! independent sources agree on the length.
//!
//! The messages are the game's own. The keys above are `GlobalStrings.lua`
//! keys; `Interface\FrameXML\GlobalStrings.lua` is in the archives and has
//! 4,592 of them, so this client does not supply text such as "You are too
//! far away!". See `vale_assets::interface::strings`. A reason with no key
//! shows nothing, which is what the 1.12.1 client does for the three hidden
//! reasons.
//!
//! A cast's target block is a mask, not a guid. `SpellCastTargets::read`
//! starts with a `u16` and reads only what the mask names. This client sends
//! three shapes:
//!
//! * `TARGET_FLAG_SELF`, which is zero and carries nothing; the server fills
//!   in the target from the spell's implicit targeting.
//! * `TARGET_FLAG_UNIT` followed by a packed guid.
//! * `TARGET_FLAG_DEST_LOCATION` followed by three bare `f32`s, with no guid
//!   before them in this build.
//!
//! Sending the current selection with a self-cast causes "Invalid target".
//! Sending a plain guid where a packed one belongs makes the server read eight
//! bytes as one. Sending a unit where a destination belongs produces no error:
//! the server substitutes the caster's own position, so the spell lands at the
//! caster's feet and reports success.
//!
//! Source: vmangos `Handlers/SpellHandler.cpp` (`HandleCastSpellOpcode`),
//! `Spells/Spell.cpp` (`SpellCastTargets::read`, `Spell::SendCastResult`),
//! `Objects/Player.cpp` (`SendInitialSpells`, `SendSpellCooldown`),
//! `Chat/MasterPlayer.cpp` (`SendInitialActionButtons`).

use crate::bytes::{Reader, Writer};

// ---------------------------------------------------------------------------
// The spellbook
// ---------------------------------------------------------------------------

/// `SMSG_INITIAL_SPELLS`: every spell this character knows, sent once at login.
///
/// No other packet states the whole spellbook. This one arrives in the login
/// burst, and afterwards the server sends only changes (`SMSG_LEARNED_SPELL`,
/// `SMSG_REMOVED_SPELL`). A client that misses this packet has an empty action
/// bar for the whole session and reports no error.
///
/// ```text
/// u8  unknown (0)
/// u16 spellCount
/// (u16 spellId, u16 slot)  x spellCount      slot is "not slot id" — always 0
/// u16 cooldownCount
/// (u16 spellId, u16 itemId, u16 category, u32 spellMs, u32 categoryMs) x n
/// ```
///
/// The spell ids are `u16` here. Every other place this client reads a spell
/// id uses a `u32`. All 1.12 spell ids fit in 16 bits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spellbook {
    /// In the order the server listed them, which is the order the character
    /// learned them. The client's own spellbook sorts by skill line; this is the
    /// raw list.
    pub known: Vec<u32>,
    /// The spells still on cooldown at login. An empty list is the normal
    /// case for a character who has been logged out for a while.
    pub cooldowns: Vec<InitialCooldown>,
}

/// One entry of [`Spellbook::cooldowns`]: a spell that is not ready yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialCooldown {
    pub spell_id: u32,
    /// The item that started the cooldown, for an item-use cooldown, and 0 for a
    /// plain spell. A cooldown record is keyed on the pair: the same trinket used
    /// twice shares one record, and two different trinkets casting one spell do
    /// not.
    pub item_id: u32,
    /// `Spell.dbc`'s category: the shared bucket a whole family recovers on
    /// (every healthstone, every bandage).
    pub category: u32,
    /// Milliseconds left on the spell's own recovery.
    pub spell_ms: u32,
    /// Milliseconds left on the category's recovery. The top bit is a flag, not
    /// part of the duration: `SendInitialSpells` sets `0x80000000` on a permanent
    /// cooldown (and puts 1 ms in `spell_ms` beside it). Such a spell does not
    /// become ready again this session; it does not mean 24 days.
    pub category_ms: u32,
}

impl InitialCooldown {
    /// Whether the `0x80000000` marker described on `category_ms` is set.
    /// `category_duration_ms` is the category duration without it.
    pub fn is_permanent(&self) -> bool {
        self.category_ms & 0x8000_0000 != 0
    }

    pub fn category_duration_ms(&self) -> u32 {
        self.category_ms & 0x7FFF_FFFF
    }
}

/// A sanity bound on the two counted arrays. Both counts are `u16`s from the
/// wire, and a packet read at the wrong offset produces a very large one. The
/// parser refuses such a packet rather than allocating for an invalid count.
const MAX_SPELLS: u16 = 4096;

pub fn parse_initial_spells(body: &[u8]) -> Option<Spellbook> {
    let mut r = Reader::new(body);
    if !r.has(1 + 2) {
        return None;
    }
    let _unknown = r.u8();
    let count = r.u16();
    if count > MAX_SPELLS || !r.has(usize::from(count) * 4) {
        return None;
    }
    let mut known = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        known.push(u32::from(r.u16()));
        let _slot = r.u16();
    }

    // The cooldown block is the tail of the packet. A truncated block loses
    // only the cooldowns: the action bar needs the spellbook above it, and an
    // unknown cooldown shows as a ready button that the server refuses.
    let mut cooldowns = Vec::new();
    if r.has(2) {
        let count = r.u16();
        if count <= MAX_SPELLS {
            for _ in 0..count {
                if !r.has(2 + 2 + 2 + 4 + 4) {
                    break;
                }
                cooldowns.push(InitialCooldown {
                    spell_id: u32::from(r.u16()),
                    item_id: u32::from(r.u16()),
                    category: u32::from(r.u16()),
                    spell_ms: r.u32(),
                    category_ms: r.u32(),
                });
            }
        }
    }
    Some(Spellbook { known, cooldowns })
}

// ---------------------------------------------------------------------------
// The action bar
// ---------------------------------------------------------------------------

/// How many buttons the server stores. `MAX_ACTION_BUTTONS` in
/// `Objects/Player.h`. The packet is exactly this many `u32`s with no count in
/// front, so this number is the packet's framing.
pub const ACTION_BUTTONS: usize = 120;

/// One occupied slot of `SMSG_ACTION_BUTTONS`.
///
/// The word is packed: the low 24 bits are the action and the top byte is the
/// kind of action. An empty slot is a zero word and is not reported here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionButton {
    /// 0..[`ACTION_BUTTONS`], the position in the bar.
    pub slot: u8,
    /// A spell id, an item entry or a macro index, depending on [`Self::kind`].
    pub action: u32,
    /// One of [`action_kind`].
    pub kind: u8,
}

impl ActionButton {
    /// This button as one packed word, which is the only form the wire uses. The
    /// inverse of [`parse_action_buttons`], and what [`set_action_button_body`]
    /// sends.
    ///
    /// The kind is the top byte and the action the low 24 bits. Packing in one
    /// function means a caller cannot swap the two. The action is masked rather
    /// than asserted: `Player::IsActionButtonDataValid` refuses any value at or
    /// above `MAX_ACTION_BUTTON_ACTION_VALUE`, so a value that would overlap the
    /// kind byte is refused either way, and truncating it here keeps the function
    /// defined for every input.
    pub fn packed(action: u32, kind: u8) -> u32 {
        (action & 0x00FF_FFFF) | (u32::from(kind) << 24)
    }
}

/// `enum ActionButtonType` — the top byte of the packed word.
pub mod action_kind {
    pub const SPELL: u8 = 0x00;
    /// vmangos' header comments this value as "click?".
    pub const CLICK: u8 = 0x01;
    pub const MACRO: u8 = 0x40;
    pub const CLICK_MACRO: u8 = 0x41;
    pub const ITEM: u8 = 0x80;
}

/// `CMSG_SET_ACTION_BUTTON`: `u8 slot`, `u32 packed`. The packet is also sent
/// for a removal, with a zero word.
///
/// The action bar is client state, and this packet tells the server to store
/// it. The server sends no reply. `HandleSetActionButtonOpcode` reads exactly
/// these five bytes, treats a zero word as `removeActionButton(slot)`, and
/// otherwise validates the pair. A spell the character does not know, a
/// passive spell, or an item entry with no prototype is dropped without a
/// reply.
///
/// The slot is zero-based here, as it is in the packet the server sends at
/// login, and one-based everywhere in the interface. The caller converts. See
/// [`crate::play::spells::ACTION_BUTTONS`] for the bound, which `u8` does not
/// express exactly (120 fits, 250 does not).
///
/// The 1.12.1 client sends the same five bytes: the slot as a byte, then the
/// slot's packed word as a `u32`, the same word [`parse_action_buttons`]
/// reads. It then raises `ACTIONBAR_SLOT_CHANGED` with `slot + 1`.
pub fn set_action_button_body(slot: u8, packed: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(slot);
    w.u32(packed);
    w.buf
}

/// Which bit of `PLAYER_FIELD_BYTES`' third byte is which extra action bar.
/// The order is the interface's argument order, not the screen order.
///
/// `SetActionBarToggles(a, b, c, d, alwaysShow)` is called from one place in
/// `Interface\FrameXML\`, `UIOptionsFrame_Save`. The client packs the first
/// four arguments into bits 0..3 in order. `MultiActionBars.lua` assigns those
/// four values to the four frames, which is where the names come from.
///
/// The fifth argument is not sent. `ALWAYS_SHOW_MULTIBARS`, the "Always Show
/// ActionBars" checkbox, is passed to `SetActionBarToggles`, but the client
/// packs only four bits. It is a saved variable rather than a saved field, and
/// this client does not send it. A five-bit mask would look correct and would
/// set a bit the server returns unchanged.
pub mod multi_bar {
    /// `SHOW_MULTI_ACTIONBAR_1` — `MultiBarBottomLeft`, slots 61..72
    /// (`BOTTOMLEFT_ACTIONBAR_PAGE` is 6).
    pub const BOTTOM_LEFT: u8 = 0x01;
    /// `SHOW_MULTI_ACTIONBAR_2` — `MultiBarBottomRight`, slots 49..60.
    pub const BOTTOM_RIGHT: u8 = 0x02;
    /// `SHOW_MULTI_ACTIONBAR_3` — `MultiBarRight`, slots 25..36.
    pub const RIGHT: u8 = 0x04;
    /// `SHOW_MULTI_ACTIONBAR_4` — `MultiBarLeft`, slots 37..48.
    ///
    /// It is not shown alone: `MultiActionBar_Update` draws the left column only
    /// when bit 2 is also set, which that file writes as
    /// `SHOW_MULTI_ACTIONBAR_3 and SHOW_MULTI_ACTIONBAR_4`. The bit keeps its own
    /// meaning; the condition is applied by the interface.
    pub const LEFT: u8 = 0x08;
    /// All four bits. The mask cannot carry the fifth argument.
    pub const ALL: u8 = BOTTOM_LEFT | BOTTOM_RIGHT | RIGHT | LEFT;
}

/// `CMSG_SET_ACTIONBAR_TOGGLES`: one byte, which is the whole packet.
///
/// The mask of [`multi_bar`] bits. The server stores it in
/// `PLAYER_FIELD_BYTES` byte 2 and returns it in the next values block, so the
/// four bars survive a logout. They are the only piece of interface layout in
/// 1.12 that does. The server sends no reply:
/// `HandleSetActionBarTogglesOpcode` calls `SetByteValue` and returns.
///
/// No local state is written either, which is the opposite of
/// [`set_action_button_body`]'s rule. The 1.12.1 client does the same: it
/// sends the four Lua arguments as the mask and does not change its own copy
/// of the field. The interface has already moved the frames itself
/// (`MultiActionBar_Update` runs from the checkbox's `OnClick`, not from the
/// save), so the field is read again only at the next
/// `PLAYER_ENTERING_WORLD`.
///
/// The high four bits are masked off. See [`multi_bar`] for the fifth
/// argument, which looks as if it belongs in them and does not.
pub fn set_actionbar_toggles_body(mask: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(mask & multi_bar::ALL);
    w.buf
}

/// Parse `SMSG_ACTION_BUTTONS`, keeping only the occupied slots.
///
/// A short body is read as far as it goes rather than refused: the bar is
/// display state, and a partial bar is better than none.
pub fn parse_action_buttons(body: &[u8]) -> Vec<ActionButton> {
    let mut r = Reader::new(body);
    let mut buttons = Vec::new();
    for slot in 0..ACTION_BUTTONS {
        if !r.has(4) {
            break;
        }
        let packed = r.u32();
        if packed == 0 {
            continue;
        }
        buttons.push(ActionButton {
            slot: slot as u8,
            action: packed & 0x00FF_FFFF,
            kind: ((packed & 0xFF00_0000) >> 24) as u8,
        });
    }
    buttons
}

/// The pseudo-spell every character has in slot 1: Attack.
///
/// It is not a cast. Pressing it sends `CMSG_ATTACKSWING` at the current
/// selection and toggles melee. `CMSG_CAST_SPELL` with this id would be refused
/// by `HandleCastSpellOpcode`'s "which he shouldn't have" branch. 6603 is the
/// only spell in the game with `SPELL_EFFECT_ATTACK`, and the 1.12.1 client
/// identifies the Attack button by that effect.
pub const SPELL_ATTACK: u32 = 6603;

// ---------------------------------------------------------------------------
// Cooldowns
// ---------------------------------------------------------------------------

/// `SMSG_SPELL_COOLDOWN`: the server's override path for cooldowns.
///
/// It is not the ordinary path. A plain cast's own recovery is computed by the
/// client from `Spell.dbc` and started when its `SMSG_SPELL_GO` comes back;
/// vmangos sends no packet for it. This packet arrives for a school lockout (a
/// counterspell), a pet's spell list, or a GM's reset, and it may name several
/// spells.
///
/// ```text
/// u64 guid                 whose cooldowns these are
/// (u32 spellId, u32 ms) x n    to the end of the body
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellCooldowns {
    pub guid: u64,
    pub entries: Vec<(u32, u32)>,
}

pub fn parse_spell_cooldowns(body: &[u8]) -> Option<SpellCooldowns> {
    let mut r = Reader::new(body);
    if !r.has(8) {
        return None;
    }
    let guid = r.u64();
    let mut entries = Vec::new();
    while r.has(8) {
        let spell_id = r.u32();
        entries.push((spell_id, r.u32()));
    }
    Some(SpellCooldowns { guid, entries })
}

/// `SMSG_COOLDOWN_EVENT`: start a cooldown that was held.
///
/// The second half of `SPELL_ATTR_COOLDOWN_ON_EVENT`. Stealth and Feign Death
/// start their recovery when they end, not when they are cast, so the client
/// inserts the record on hold and this packet starts it. The body is
/// `u32 spellId`, then a plain `u64` guid. The guid is second here and first
/// in `SMSG_SPELL_COOLDOWN`; reading it in the wrong order produces a cooldown
/// on spell 0.
pub fn parse_cooldown_event(body: &[u8]) -> Option<(u32, u64)> {
    let mut r = Reader::new(body);
    if !r.has(4 + 8) {
        return None;
    }
    let spell_id = r.u32();
    Some((spell_id, r.u64()))
}

/// `SMSG_CLEAR_COOLDOWN`: this cooldown has ended, however long it had left.
///
/// Without this packet a school lockout displays wrongly.
/// `Player::LockOutSpells` sends `SMSG_SPELL_COOLDOWN` naming every spell of
/// the school with the full duration; `Player::RemoveSpellLockout` sends one
/// of these per spell when the lockout is lifted early. A client that ignores
/// them runs every cooldown sweep to the full stated length of the lockout,
/// which is right only when the lockout runs its course. This was reported as
/// the cooldown displaying inconsistently.
///
/// It is not sent only for interrupts. vmangos sends it from four places
/// (`Player.cpp:22354`, `:22365`, `:22381`, `:22469`): any cooldown removal,
/// the warlock Ritual of Doom correction, and the no-cooldown cheat.
///
/// ## How the field order was determined, and why a wrong order is safe
///
/// `u32 spellId` then `u64 guid`, the same shape as [`parse_cooldown_event`]:
/// the same server, the same subject, and the fields in the order
/// `SendClearCooldown(spellId, target)` assigns them. The serialiser itself is
/// not confirmed: `WorldPackets::Spell::ClearCooldown` is a packet class that
/// could not be fetched, and the layout has not been confirmed against the
/// 1.12.1 client.
///
/// Two checks make a wrong reading safe. The body must be exactly twelve
/// bytes; a `u64`-first layout would also be twelve, so this check alone does
/// not decide the order. The caller also compares the guid against the
/// player's own, which the server always sends. With the fields reversed, the
/// guid would hold a spell id in its low half and would not match, so a wrong
/// reading refuses every packet rather than clearing a random spell. The
/// refusals appear in the session's warning channel.
pub fn parse_clear_cooldown(body: &[u8]) -> Option<(u32, u64)> {
    if body.len() != 4 + 8 {
        return None;
    }
    let mut r = Reader::new(body);
    let spell_id = r.u32();
    Some((spell_id, r.u64()))
}

/// `SMSG_ITEM_COOLDOWN`: an item's guid and one of its spells, with no
/// duration.
///
/// ```text
/// u64 item, u32 spell
/// ```
///
/// vmangos sends it only from `Player::ApplyEquipCooldown`, once per on-use
/// spell of an item just equipped, unless the item carries
/// `ITEM_FLAG_NO_EQUIP_COOLDOWN`. It is the thirty-second wait before a trinket
/// can be used after it is put on. The 1.12.1 client starts a cooldown of
/// [`EQUIP_COOLDOWN_MS`] on that item's spell, with no category cooldown, and
/// raises `ACTIONBAR_UPDATE_COOLDOWN`, `SPELL_UPDATE_COOLDOWN` and
/// `BAG_UPDATE_COOLDOWN`.
pub fn parse_item_cooldown(body: &[u8]) -> Option<(u64, u32)> {
    if body.len() < 12 {
        return None;
    }
    let mut r = Reader::new(body);
    Some((r.u64(), r.u32()))
}

/// The cooldown an `SMSG_ITEM_COOLDOWN` starts: thirty seconds, the same value
/// vmangos gives the spell on its side (`AddCooldown(..., 30 * IN_MILLISECONDS)`
/// in `Player::ApplyEquipCooldown`).
pub const EQUIP_COOLDOWN_MS: u32 = 30_000;

// ---------------------------------------------------------------------------
// The server's answers to a cast or a swing, and the player event queue
// ---------------------------------------------------------------------------

/// `SMSG_CAST_RESULT`: the answer to the player's own `CMSG_CAST_SPELL`.
///
/// Sent on success as well as on failure, so the failure is an `Option` and
/// the packet is not. `status` is 0 when the cast was accepted and 2 when it
/// was not, and the reason byte follows only the 2. A success carries nothing
/// the client needs, since the cast itself arrives as
/// `SMSG_SPELL_START`/`SMSG_SPELL_GO` like any other unit's. Because a success
/// is not a failure, the cast bar keeps running when one arrives.
///
/// ```text
/// u32 spellId
/// u8  status          0 accepted, 2 failed
/// u8  reason          failures only — an index into CAST_FAILURE_KEYS
/// u32 u32 u32         the three EQUIPPED_ITEM_CLASS reasons only — see below
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CastResult {
    pub spell_id: u32,
    /// `None` when the cast was accepted.
    pub failure: Option<u8>,
    /// The equipment the refusal is about, for the three reasons that carry it.
    ///
    /// See [`EquipRequirement`]: without it the message is shown with its `%s`
    /// still in it, which is what warriors reported.
    pub requirement: Option<EquipRequirement>,
}

/// The equipment a refused cast required, as `Spell::SendCastResult` sends it.
///
/// This is the argument to a `%s`, and the reason the tail is read.
/// `GlobalStrings.lua` writes the three equipped-item refusals as
/// `"Must have a %s equipped"` and two per-hand variants, so a client that
/// stops at the reason byte shows the placeholder verbatim. A warrior pressing
/// Rend without a weapon used to see that.
///
/// The values are the spell's own `Spell.dbc` columns, not facts about the
/// character, which is why one value can name the requirement: the server
/// copies the row it just checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipRequirement {
    /// `EquippedItemClass` — an `ItemClass.dbc` id ("Weapon", "Armor").
    pub class: u32,
    /// `EquippedItemSubClassMask` — a mask of `ItemSubClass.dbc` subclasses, so a
    /// spell that accepts any one-handed weapon sets several bits at once.
    pub subclass_mask: u32,
    /// `EquippedItemInventoryTypeMask`, not read here: the reason byte already
    /// names the slot (main hand, off hand, either).
    pub inventory_type_mask: u32,
}

/// `status = 2`, the only failing value `SendCastResult` writes.
const CAST_STATUS_FAILED: u8 = 2;

/// The three reasons whose tail is the equipment trio above.
///
/// vmangos writes the trio under `SPELL_FAILED_EQUIPPED_ITEM_CLASS` and its
/// two per-hand variants and under nothing else. The other three cases in that
/// `switch` (`NOT_READY`, `REQUIRES_SPELL_FOCUS`, `REQUIRES_AREA`) each write a
/// single `u32` that this client has no table to name yet, so they are left
/// unread rather than parsed into a number that cannot be shown as text.
const EQUIPPED_ITEM_CLASS: u8 = 0x19;
const EQUIPPED_ITEM_CLASS_MAINHAND: u8 = 0x1a;
const EQUIPPED_ITEM_CLASS_OFFHAND: u8 = 0x1b;

pub fn parse_cast_result(body: &[u8]) -> Option<CastResult> {
    let mut r = Reader::new(body);
    if !r.has(4 + 1) {
        return None;
    }
    let spell_id = r.u32();
    let status = r.u8();
    let failure = if status == CAST_STATUS_FAILED && r.has(1) {
        Some(r.u8())
    } else {
        None
    };
    // The tail is read only for the reasons that carry one, and only when the
    // whole of it is there: a short body is a packet this client has
    // misunderstood, and half an argument would name the wrong item class.
    let requirement = match failure {
        Some(EQUIPPED_ITEM_CLASS | EQUIPPED_ITEM_CLASS_MAINHAND | EQUIPPED_ITEM_CLASS_OFFHAND)
            if r.has(4 * 3) =>
        {
            Some(EquipRequirement {
                class: r.u32(),
                subclass_mask: r.u32(),
                inventory_type_mask: r.u32(),
            })
        }
        _ => None,
    };
    Some(CastResult { spell_id, failure, requirement })
}

/// `SMSG_SPELL_FAILED_OTHER`: a cast in progress was interrupted.
///
/// `u64` caster guid (plain, not packed), then `u32 spellId`. vmangos never
/// sends `SMSG_SPELL_FAILURE`; `Spell::SendInterrupted` sends this packet. It
/// stops a cast animation that would otherwise be held for the whole length of
/// a cast bar that has already been cancelled.
pub fn parse_spell_failed_other(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    Some((guid, r.u32()))
}

/// `SMSG_LEARNED_SPELL`: one new spell, as `{u16 spellId, int16
/// actionBarSlot}`, not one `u32`.
///
/// The spellbook is stated once and changed afterwards. A trainer visit, a
/// level-up and a quest reward all arrive this way, so a client that reads
/// only `SMSG_INITIAL_SPELLS` has a spellbook that is correct at login and
/// out of date for the rest of the session.
///
/// This packet was read as a single `u32` for a long time with no visible
/// error. `LearnedSpell::AppendBodyTo` writes the id as a `uint16` and then an
/// `int16` that its own comment calls "not used". vmangos leaves that field at
/// its `= 0` initialiser and never assigns it, so the four bytes read
/// little-endian equal the id, and the wrong read gave the right answer on
/// every packet this project has received. It is corrected because a server
/// that put any value in that field would turn every learned spell into an id
/// above 65,536. That failure would appear as "the trainer did nothing" with
/// no parse error.
///
/// The trailing field is not returned, because the 1.12.1 client does not use
/// it either.
pub fn parse_learned_spell(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(2).then(|| u32::from(r.u16()))
}

/// `SMSG_REMOVED_SPELL`: one spell removed. The body is two bytes, where the
/// packet above is four. `RemovedSpell::AppendBodyTo` writes `uint16(spellId)`
/// and nothing else.
pub fn parse_removed_spell(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(2).then(|| u32::from(r.u16()))
}

/// `SMSG_SUPERCEDED_SPELL`: a higher rank replaced a lower one, as `{u16 old,
/// u16 new}`.
///
/// This packet keeps action bar buttons pointing at known spells. The server's
/// comment beside the send is "new spell replace old in action bars and spell
/// book": the client performs the replacement. A client that ignores the packet leaves the superseded id in
/// `character_action` permanently, because no packet restates the bar (see
/// [`parse_action_buttons`]).
///
/// The effect is not only visual. `HandleCastSpellOpcode` refuses a spell the
/// character does not have active, which a superseded rank is not, and returns
/// with no reply. Pressing the old button did nothing, and the pending record
/// stayed set until [`super::super::socket::handler`]'s deadline released it.
/// Measured against the running server on this project's warrior
/// (`characters.character_action` against `characters.character_spell`, guid
/// 378): button 73 held Heroic Strike 11566 with only 11567 active, and button
/// 75 held Rend 11572 with 11573 active.
///
/// The server truncates both ids to 16 bits
/// (`SupercededSpell::AppendBodyTo`), the same narrowing
/// `SMSG_INITIAL_SPELLS` applies to every id it lists. Nothing is lost: no
/// 1.12 spell id reaches 65,536.
pub fn parse_superceded_spell(body: &[u8]) -> Option<(u32, u32)> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let old = u32::from(r.u16());
    Some((old, u32::from(r.u16())))
}

/// Why a `CMSG_ATTACKSWING` was refused.
///
/// The melee counterpart of a cast result, sent as five opcodes rather than
/// one field. None of them has a body; the opcode is the message, so this
/// enum is the whole packet.
///
/// The strings are the game's own (`GlobalStrings.lua`). Two of them appear
/// often: swinging at a mob from outside melee range gives `BADATTACKPOS`,
/// and swinging while facing away gives `BADATTACKFACING`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackRefusal {
    NotInRange,
    BadFacing,
    NotStanding,
    DeadTarget,
    CantAttack,
}

impl AttackRefusal {
    /// Which of the five, or `None` for any other opcode.
    pub fn of(opcode: crate::opcodes::Opcode) -> Option<AttackRefusal> {
        use crate::opcodes::Opcode as O;
        Some(match opcode {
            O::SMSG_ATTACKSWING_NOTINRANGE => AttackRefusal::NotInRange,
            O::SMSG_ATTACKSWING_BADFACING => AttackRefusal::BadFacing,
            O::SMSG_ATTACKSWING_NOTSTANDING => AttackRefusal::NotStanding,
            O::SMSG_ATTACKSWING_DEADTARGET => AttackRefusal::DeadTarget,
            O::SMSG_ATTACKSWING_CANT_ATTACK => AttackRefusal::CantAttack,
            _ => return None,
        })
    }

    /// The `GlobalStrings.lua` key for the message shown to the player.
    ///
    /// `ERR_BADATTACKPOS` is "You are too far away!" and `ERR_BADATTACKFACING`
    /// is "You are facing the wrong way!". Both are the game's own strings, not
    /// text this client supplies.
    pub fn key(self) -> &'static str {
        match self {
            AttackRefusal::NotInRange => "ERR_BADATTACKPOS",
            AttackRefusal::BadFacing => "ERR_BADATTACKFACING",
            AttackRefusal::NotStanding => "ERR_ATTACK_STUNNED",
            AttackRefusal::DeadTarget => "ERR_ATTACK_DEAD",
            AttackRefusal::CantAttack => "ERR_INVALID_ATTACK_TARGET",
        }
    }
}

/// `SMSG_ATTACKSTART`: a unit has started auto-attacking another.
///
/// Two plain guids, attacker then victim (`Unit::SendMeleeAttackStart`
/// writes `GetObjectGuid()` twice with no packing). Sent to every client in
/// range, so this is how the client learns that a distant creature has
/// started a fight. For the local player it is the server's confirmation
/// that the requested swing started.
pub fn parse_attack_start(body: &[u8]) -> Option<(u64, u64)> {
    let mut r = Reader::new(body);
    if !r.has(16) {
        return None;
    }
    let attacker = r.u64();
    Some((attacker, r.u64()))
}

/// `MSG_CHANNEL_START`: a channelled spell has started, and how long it lasts.
///
/// `{u32 spellId, u32 durationMs}`, from `Spell::SendChannelStart` in vmangos,
/// sent with `SendDirectMessage` and therefore only to the caster. There is no
/// broadcast form: another player's channel is shown by
/// `UNIT_CHANNEL_SPELL` in their update block, which this client does not
/// read.
pub fn parse_channel_start(body: &[u8]) -> Option<(u32, u32)> {
    let mut r = Reader::new(body);
    if !r.has(8) {
        return None;
    }
    let spell = r.u32();
    Some((spell, r.u32()))
}

/// `SMSG_SPELL_DELAYED`: the cast in progress was pushed back by damage.
///
/// `{u64 caster, u32 delayMs}`. `Spell::Delayed` writes an `ObjectGuid`
/// plain (there is no `GetPackGUID()` on this one) and then the delay. It is
/// sent with `SendDirectMessage` and only when the caster is a player, so it
/// is always about the player. The guid is read and checked anyway, as in
/// [`parse_spell_cooldowns`], because that guarantee comes from the current
/// server rather than from the packet format.
///
/// The delay is a difference, not a new length. vmangos adds it to `m_timer`
/// and clamps at the spell's cast time, so a cast can be pushed back several
/// times and each packet gives only the amount of that pushback.
pub fn parse_spell_delayed(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(12) {
        return None;
    }
    let guid = r.u64();
    Some((guid, r.u32()))
}

/// `MSG_CHANNEL_UPDATE`: the time left on the channel.
///
/// `{u32 remainingMs}`. Zero means the channel has ended; an interrupted or
/// cancelled channel sends this packet with zero rather than a packet of its
/// own.
pub fn parse_channel_update(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

/// `SMSG_ATTACKSTOP`: a unit has stopped auto-attacking.
///
/// This packet has packed guids, where `SMSG_ATTACKSTART` has plain ones. The
/// two functions are adjacent in `Unit.cpp` and use different encodings. The
/// victim may be a packed zero (`SendAttackStop(nullptr)` writes an empty
/// `PackedGuid`), which means "stop attacking, no particular target": the
/// answer to a swing at a friendly or dead unit.
pub fn parse_attack_stop(body: &[u8]) -> Option<(u64, u64)> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let attacker = r.packed_guid();
    let victim = r.packed_guid();
    Some((attacker, victim))
}

// ---------------------------------------------------------------------------
// Outbound
// ---------------------------------------------------------------------------

/// `SpellCastTargets`' mask bits, as far as this client sends them.
///
/// The full table is sixteen bits wide. The constants here are the flags this
/// client sends.
pub mod target_flag {
    /// Zero, meaning there is no target block. `SpellCastTargets::read` compares
    /// the whole mask with this value before reading anything, so a self-cast is
    /// two bytes on the wire and the server fills in the target from the spell's
    /// implicit targeting.
    pub const SELF: u16 = 0x0000;
    /// A packed unit guid follows.
    pub const UNIT: u16 = 0x0002;
    /// Three plain `f32`s follow and nothing else; no guid in any build. This is
    /// the ground-targeted form of a cast: Blizzard, Flamestrike, Rain of Fire.
    /// `SpellCastTargets::read` validates them with `IsValidMapCoord` and rejects
    /// the whole packet if they fail, so a `NaN` here causes a disconnection
    /// rather than a refusal.
    pub const DEST_LOCATION: u16 = 0x0040;
    /// A packed game object guid follows: a chest, an ore vein, a herb.
    ///
    /// `0x0800`, from vmangos' `SpellCastTargetFlags`, where it is
    /// `TARGET_FLAG_GAMEOBJECT`, between `TARGET_FLAG_UNIT_DEAD` (`0x400`) and
    /// `TARGET_FLAG_TRADE_ITEM` (`0x1000`). The neighbours matter: the block is a
    /// bitmask of sixteen flags of which this client sends four, and the two on
    /// either side of this one are a corpse and an item. Both also carry a guid,
    /// so a wrong flag here produces a well-formed packet that the server reads
    /// as being about a different object.
    ///
    /// It exists because a chest is not opened by `CMSG_GAMEOBJ_USE`: see
    /// `vale_assets::look::object`, which quotes the server code that does
    /// nothing for it. Gathering a herb is a spell cast at the plant.
    pub const GAMEOBJECT: u16 = 0x0800;
    /// A packed item guid follows: the weapon an imbue is applied to.
    ///
    /// `0x0010`, from vmangos' `SpellCastTargetFlags`, read as a packed guid in
    /// `SpellCastTargets::read` like every other guid in the block.
    /// `Spell::ValidateExplicitTargetMask` requires this bit for any spell whose
    /// `Spell.dbc` `Targets` column has it. That is why Rockbiter Weapon sent with
    /// any other target is refused, and why, before this flag existed here, it was
    /// refused locally as "Invalid target".
    ///
    /// The client selects the item; the player does not point at it. See
    /// `vale_assets::tables::spellbook::CastAim::Item`.
    pub const ITEM: u16 = 0x0010;
    /// A trade window slot follows, as a slot number packed like a guid rather
    /// than a guid.
    ///
    /// `0x1000`, the neighbour above [`GAMEOBJECT`]. See
    /// [`super::CastTarget::TradeSlot`], which quotes how the server reads it:
    /// only `TRADE_SLOT_NONTRADED` is accepted, and while the trade is still open
    /// the cast is stored rather than performed.
    pub const TRADE_ITEM: u16 = 0x1000;
}

/// What a cast is aimed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CastTarget {
    /// The spell's implicit targeting decides who it hits, and no target is
    /// sent. Examples: Ice Armor, Battle Shout.
    SelfImplicit,
    /// This unit, which may be the player.
    Unit(u64),
    /// A position in the server's axes and yards, `x, y, z`. For a
    /// ground-targeted spell it is the point the player clicked on the ground.
    /// See [`target_flag::DEST_LOCATION`].
    ///
    /// Only the client can produce it: no packet asks where the pointer is, and
    /// vmangos' `Spell::SetTargetMap` falls back to the caster's own position when
    /// the flag is absent. A ground-targeted spell sent without it lands at the
    /// caster's feet rather than being refused.
    Dest([f32; 3]),
    /// A trade window slot. It is the one target that is not an object: the
    /// "guid" written is the trade slot number.
    ///
    /// The last clause of `Spell::CheckCast` reads it as
    /// `TradeSlots(m_targets.getItemTargetGuid().GetRawValue())` and refuses
    /// anything but `TRADE_SLOT_NONTRADED` with `SPELL_FAILED_ITEM_NOT_READY`.
    /// While the trade is open it then stores the spell against the trade and
    /// answers `SPELL_FAILED_DONT_REPORT`. That is why an enchant aimed at the
    /// "will not be traded" slot produces no visible answer and is applied only
    /// when both sides accept. The server reports it back in
    /// `SMSG_TRADE_STATUS_EXTENDED`'s `spell` field, which
    /// [`crate::play::trade::TradeOffer::spell`] already carries.
    TradeSlot(u8),
    /// A game object: an ore vein, a herb, a locked chest.
    ///
    /// The only target kind this client sends that is not a unit or a position,
    /// and the only one chosen by the client rather than the player: the player
    /// clicks the plant, and the spell is taken from the lock rather than from a
    /// button. See `vale_assets::look::object::opener`.
    Object(u64),
    /// An item in the bags or equipped: the weapon Rockbiter is applied to, the
    /// blade a sharpening stone is used on.
    ///
    /// The client chooses it, not the player, for every spell with
    /// `SPELL_ATTR_HELD_ITEM_ONLY`; vmangos' comment on that flag is "Client
    /// automatically selects item from mainhand slot as a cast target". See
    /// `vale_assets::tables::spellbook::CastAim::Item`, which makes that choice.
    Item(u64),
}

/// `CMSG_CAST_SPELL`: `u32 spellId`, then the target block.
///
/// See [`CastTarget`] and the module comment: sending the current selection
/// with a spell that takes no target makes the server refuse a buff with
/// "Invalid target".
pub fn cast_spell_body(spell_id: u32, target: CastTarget) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(spell_id);
    write_cast_target(&mut w, target);
    w.buf
}

/// The target block itself, which `CMSG_USE_ITEM` also carries.
///
/// One writer for both, because both opcodes are read through the same
/// `SpellCastTargets::read` on the server, and a block that differed between
/// them would produce a packet the server rejects with no message.
pub(crate) fn write_cast_target(w: &mut Writer, target: CastTarget) {
    match target {
        CastTarget::SelfImplicit => {
            w.u16(target_flag::SELF);
        }
        CastTarget::Unit(guid) => {
            w.u16(target_flag::UNIT);
            w.packed_guid(guid);
        }
        CastTarget::Dest([x, y, z]) => {
            w.u16(target_flag::DEST_LOCATION);
            w.f32(x).f32(y).f32(z);
        }
        // Packed like a unit guid and not plain — `SpellCastTargets::read`
        // reads every guid in the block through the same `readPackGUID`.
        CastTarget::Object(guid) => {
            w.u16(target_flag::GAMEOBJECT);
            w.packed_guid(guid);
        }
        // Packed too — `SpellCastTargets::read` reads `TARGET_FLAG_ITEM`
        // through the same `readPackGUID` as the unit and the game object.
        CastTarget::Item(guid) => {
            w.u16(target_flag::ITEM);
            w.packed_guid(guid);
        }
        // The trade slot number is also read through the same `readPackGUID`,
        // although it is a number rather than a guid. See
        // [`CastTarget::TradeSlot`].
        CastTarget::TradeSlot(slot) => {
            w.u16(target_flag::TRADE_ITEM);
            w.packed_guid(u64::from(slot));
        }
    }
}

/// `CMSG_CANCEL_CAST`: `u32 spellId` and nothing else.
pub fn cancel_cast_body(spell_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(spell_id);
    w.buf
}

/// `CMSG_CANCEL_AURA`: remove a buff the player has, `u32 spellId`.
///
/// The same one-field shape as the cast cancel, for a different request. This
/// is `BuffButton_OnClick`'s right-click, and it is the only action the
/// interface can take on an aura.
///
/// A spell id, not a slot, although every other aura packet in 1.12 is keyed
/// by slot: `WorldSession::HandleCancelAuraOpcode` reads one `uint32` and
/// looks the spell up. The server refuses a negative aura or one with
/// `SPELL_ATTR_NO_AURA_CANCEL`, and sends nothing when it does, so the
/// client-side `AFLAG_CANCELABLE` check is what stops a right-click on a
/// debuff from sending a packet that has no effect.
pub fn cancel_aura_body(spell_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(spell_id);
    w.buf
}

/// `SMSG_UPDATE_AURA_DURATION`: `u8 slot`, `u32 remaining ms`.
///
/// Five bytes, and the only statement in the 1.12 protocol of how long a buff
/// has left. It carries no spell id; the slot is the only key.
/// `SpellAuraHolder::UpdateAuraDuration` sends it only when the aura's target
/// is a player, and not at all for a permanent aura. So:
///
/// * a buff on another unit has no timer, which is why the game's own target
///   and party frames draw none;
/// * an aura for which this packet never arrives lasts until cancelled, which
///   is `GetPlayerBuff`'s second return value.
pub fn parse_aura_duration(body: &[u8]) -> Option<(u8, u32)> {
    let mut r = Reader::new(body);
    if !r.has(1 + 4) {
        return None;
    }
    let slot = r.u8();
    Some((slot, r.u32()))
}

/// `SMSG_LEVELUP_INFO`: the level reached, and what it gave.
///
/// ```text
/// u32 level
/// u32 healthGained
/// u32 powerGained[5]     mana, rage, focus, energy, happiness
/// u32 statGained[5]      strength, agility, stamina, intellect, spirit
/// ```
///
/// `Player::GiveLevel` writes exactly that: the mana and four literal zeroes,
/// then the five stat deltas against `GetCreateStat`. Forty-eight bytes.
///
/// Every number after the first is a delta. All of them are read, not only
/// the level, because `ChatFrame_OnEvent`'s `PLAYER_LEVEL_UP` branch takes
/// nine arguments and compares `arg3` through `arg9` with `> 0` before
/// formatting each. When this client raised the event with only the level,
/// the handler failed on its second line; `--audit --events` reported the
/// failure.
///
/// The packet is also the only statement that a level-up happened, as
/// opposed to the level being different: `UNIT_FIELD_LEVEL` changes when a
/// target is selected, when a creature comes into view, and at every login.
pub fn parse_levelup(body: &[u8]) -> Option<LevelUp> {
    let mut r = Reader::new(body);
    if !r.has(4 * 12) {
        return None;
    }
    let level = r.u32();
    let health = r.u32();
    let mana = r.u32();
    // Rage, focus, energy and happiness — written as literal zeroes and read
    // past rather than kept, because no level-up in 1.12 grants any of them and
    // the interface has no argument for one.
    for _ in 0..4 {
        r.u32();
    }
    let mut stats = [0u32; 5];
    for stat in &mut stats {
        *stat = r.u32();
    }
    Some(LevelUp { level, health, mana, stats })
}

/// What one level-up gave. See [`parse_levelup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelUp {
    pub level: u32,
    /// Hit points gained.
    pub health: u32,
    /// Mana gained, which is zero for a class that does not use mana: the server
    /// writes the mana delta only when `GetPowerType() == POWER_MANA`. The zero
    /// is required: it makes the same FrameXML handler print "You have gained 42
    /// hit points." for a warrior and "…and 30 mana." for a mage.
    pub mana: u32,
    /// Strength, agility, stamina, intellect, spirit — in that order, which is
    /// `SPELL_STAT0_NAME`..`SPELL_STAT4_NAME`'s.
    pub stats: [u32; 5],
}

// ---------------------------------------------------------------------------
// The failure table
// ---------------------------------------------------------------------------

/// Wire reason -> the `GlobalStrings.lua` key that names it.
///
/// Position is the meaning of this table. See the module comment: the byte on
/// the wire is an index into `SpellCastResult`'s declaration order, and for
/// build 5875 that order is exactly this list. Do not sort it, insert into it,
/// or rename an entry; each name is a lookup key in a file shipped inside the
/// MPQs.
///
/// Three of the reasons name a key `GlobalStrings.lua` does not contain
/// (`AUTOTRACK_INTERRUPTED`, `HUNGER_SATIATED`, `THIRST_SATIATED`, and the
/// happiness case of `NO_POWER`). This is not an error in the copy: the
/// client shows nothing for them, and a missing key is how it does so.
pub const CAST_FAILURE_KEYS: [&str; 146] = [
    "SPELL_FAILED_AFFECTING_COMBAT",             // 0x00 You are in combat
    "SPELL_FAILED_ALREADY_AT_FULL_HEALTH",       // 0x01 You are already at full Health.
    "SPELL_FAILED_ALREADY_AT_FULL_POWER",        // 0x02 You are already at full %s.
    "SPELL_FAILED_ALREADY_BEING_TAMED",          // 0x03 That creature is already being tamed
    "SPELL_FAILED_ALREADY_HAVE_CHARM",           // 0x04 You already control a charmed creature
    "SPELL_FAILED_ALREADY_HAVE_SUMMON",          // 0x05 You already control a summoned creature
    "SPELL_FAILED_ALREADY_OPEN",                 // 0x06 Already open
    "SPELL_FAILED_AURA_BOUNCED",                 // 0x07 A more powerful spell is already active
    "SPELL_FAILED_AUTOTRACK_INTERRUPTED",        // 0x08 hidden/unused
    "SPELL_FAILED_BAD_IMPLICIT_TARGETS",         // 0x09 You have no target.
    "SPELL_FAILED_BAD_TARGETS",                  // 0x0a Invalid target
    "SPELL_FAILED_CANT_BE_CHARMED",              // 0x0b Target can't be charmed
    "SPELL_FAILED_CANT_BE_DISENCHANTED",         // 0x0c Item cannot be disenchanted
    "SPELL_FAILED_CANT_BE_PROSPECTED",           // 0x0d There are no gems in this
    "SPELL_FAILED_CANT_CAST_ON_TAPPED",          // 0x0e Target is tapped
    "SPELL_FAILED_CANT_DUEL_WHILE_INVISIBLE",    // 0x0f
    "SPELL_FAILED_CANT_DUEL_WHILE_STEALTHED",    // 0x10
    "SPELL_FAILED_CANT_STEALTH",                 // 0x11 You are too close to enemies
    "SPELL_FAILED_CASTER_AURASTATE",             // 0x12 You can't do that yet
    "SPELL_FAILED_CASTER_DEAD",                  // 0x13 You are dead
    "SPELL_FAILED_CHARMED",                      // 0x14 Can't do that while charmed
    "SPELL_FAILED_CHEST_IN_USE",                 // 0x15 That is already being used
    "SPELL_FAILED_CONFUSED",                     // 0x16 Can't do that while confused
    "SPELL_FAILED_DONT_REPORT",                  // 0x17 never displayed — see `is_silent`
    "SPELL_FAILED_EQUIPPED_ITEM",                // 0x18 Must have the proper item equipped
    "SPELL_FAILED_EQUIPPED_ITEM_CLASS",          // 0x19 Must have a %s equipped
    "SPELL_FAILED_EQUIPPED_ITEM_CLASS_MAINHAND", // 0x1a
    "SPELL_FAILED_EQUIPPED_ITEM_CLASS_OFFHAND",  // 0x1b
    "SPELL_FAILED_ERROR",                        // 0x1c Internal error
    "SPELL_FAILED_FIZZLE",                       // 0x1d Fizzled
    "SPELL_FAILED_FLEEING",                      // 0x1e Can't do that while fleeing
    "SPELL_FAILED_FOOD_LOWLEVEL",                // 0x1f
    "SPELL_FAILED_HIGHLEVEL",                    // 0x20 Target is too high level
    "SPELL_FAILED_HUNGER_SATIATED",              // 0x21 hidden/unused
    "SPELL_FAILED_IMMUNE",                       // 0x22 Immune
    "SPELL_FAILED_INTERRUPTED",                  // 0x23 Interrupted
    "SPELL_FAILED_INTERRUPTED_COMBAT",           // 0x24 Interrupted
    "SPELL_FAILED_ITEM_ALREADY_ENCHANTED",       // 0x25
    "SPELL_FAILED_ITEM_GONE",                    // 0x26 Item is gone
    "SPELL_FAILED_ITEM_NOT_FOUND",               // 0x27
    "SPELL_FAILED_ITEM_NOT_READY",               // 0x28 Item is not ready yet.
    "SPELL_FAILED_LEVEL_REQUIREMENT",            // 0x29 You are not high enough level
    "SPELL_FAILED_LINE_OF_SIGHT",                // 0x2a Target not in line of sight
    "SPELL_FAILED_LOWLEVEL",                     // 0x2b Target is too low level
    "SPELL_FAILED_LOW_CASTLEVEL",                // 0x2c Skill not high enough
    "SPELL_FAILED_MAINHAND_EMPTY",               // 0x2d Your weapon hand is empty
    "SPELL_FAILED_MOVING",                       // 0x2e Can't do that while moving
    "SPELL_FAILED_NEED_AMMO",                    // 0x2f
    "SPELL_FAILED_NEED_AMMO_POUCH",              // 0x30 Requires: %s
    "SPELL_FAILED_NEED_EXOTIC_AMMO",             // 0x31 Requires exotic ammo: %s
    "SPELL_FAILED_NOPATH",                       // 0x32 No path available
    "SPELL_FAILED_NOT_BEHIND",                   // 0x33 You must be behind your target
    "SPELL_FAILED_NOT_FISHABLE",                 // 0x34
    "SPELL_FAILED_NOT_HERE",                     // 0x35 You can't use that here
    "SPELL_FAILED_NOT_INFRONT",                  // 0x36 You must be in front of your target
    "SPELL_FAILED_NOT_IN_CONTROL",               // 0x37
    "SPELL_FAILED_NOT_KNOWN",                    // 0x38 Spell not learned
    "SPELL_FAILED_NOT_MOUNTED",                  // 0x39 You are mounted
    "SPELL_FAILED_NOT_ON_TAXI",                  // 0x3a You are in flight
    "SPELL_FAILED_NOT_ON_TRANSPORT",             // 0x3b You are on a transport
    "SPELL_FAILED_NOT_READY",                    // 0x3c Not yet recovered
    "SPELL_FAILED_NOT_SHAPESHIFT",               // 0x3d You are in shapeshift form
    "SPELL_FAILED_NOT_STANDING",                 // 0x3e You must be standing to do that
    "SPELL_FAILED_NOT_TRADEABLE",                // 0x3f
    "SPELL_FAILED_NOT_TRADING",                  // 0x40
    "SPELL_FAILED_NOT_UNSHEATHED",               // 0x41 You have to be unsheathed to do that!
    "SPELL_FAILED_NOT_WHILE_GHOST",              // 0x42 Can't cast as ghost
    "SPELL_FAILED_NO_AMMO",                      // 0x43 Out of ammo
    "SPELL_FAILED_NO_CHARGES_REMAIN",            // 0x44 No charges remain
    "SPELL_FAILED_NO_CHAMPION",                  // 0x45
    "SPELL_FAILED_NO_COMBO_POINTS",              // 0x46 That ability requires combo points
    "SPELL_FAILED_NO_DUELING",                   // 0x47 Dueling isn't allowed here
    "SPELL_FAILED_NO_ENDURANCE",                 // 0x48 Not enough endurance
    "SPELL_FAILED_NO_FISH",                      // 0x49 There aren't any fish here
    "SPELL_FAILED_NO_ITEMS_WHILE_SHAPESHIFTED",  // 0x4a
    "SPELL_FAILED_NO_MOUNTS_ALLOWED",            // 0x4b You can't mount here
    "SPELL_FAILED_NO_PET",                       // 0x4c You do not have a pet
    "SPELL_FAILED_NO_POWER",                     // 0x4d Not enough mana / rage / energy
    "SPELL_FAILED_NOTHING_TO_DISPEL",            // 0x4e Nothing to dispel
    "SPELL_FAILED_NOTHING_TO_STEAL",             // 0x4f Nothing to steal
    "SPELL_FAILED_ONLY_ABOVEWATER",              // 0x50 Cannot use while swimming
    "SPELL_FAILED_ONLY_DAYTIME",                 // 0x51 Can only use during the day
    "SPELL_FAILED_ONLY_INDOORS",                 // 0x52 Can only use indoors
    "SPELL_FAILED_ONLY_MOUNTED",                 // 0x53 Can only use while mounted
    "SPELL_FAILED_ONLY_NIGHTTIME",               // 0x54 Can only use during the night
    "SPELL_FAILED_ONLY_OUTDOORS",                // 0x55 Can only use outside
    "SPELL_FAILED_ONLY_SHAPESHIFT",              // 0x56 Must be in %s
    "SPELL_FAILED_ONLY_STEALTHED",               // 0x57 You must be in stealth mode
    "SPELL_FAILED_ONLY_UNDERWATER",              // 0x58 Can only use while swimming
    "SPELL_FAILED_OUT_OF_RANGE",                 // 0x59 Out of range.
    "SPELL_FAILED_PACIFIED",                     // 0x5a Can't use that ability while pacified
    "SPELL_FAILED_POSSESSED",                    // 0x5b You are possessed
    "SPELL_FAILED_REAGENTS",                     // 0x5c client-side message
    "SPELL_FAILED_REQUIRES_AREA",                // 0x5d You need to be in %s
    "SPELL_FAILED_REQUIRES_SPELL_FOCUS",         // 0x5e Requires %s
    "SPELL_FAILED_ROOTED",                       // 0x5f You are unable to move
    "SPELL_FAILED_SILENCED",                     // 0x60 Can't do that while silenced
    "SPELL_FAILED_SPELL_IN_PROGRESS",            // 0x61 Another action is in progress
    "SPELL_FAILED_SPELL_LEARNED",                // 0x62 You have already learned the spell
    "SPELL_FAILED_SPELL_UNAVAILABLE",            // 0x63 The spell is not available to you
    "SPELL_FAILED_STUNNED",                      // 0x64 Can't do that while stunned
    "SPELL_FAILED_TARGETS_DEAD",                 // 0x65 Your target is dead
    "SPELL_FAILED_TARGET_AFFECTING_COMBAT",      // 0x66 Target is in combat
    "SPELL_FAILED_TARGET_AURASTATE",             // 0x67 You can't do that yet
    "SPELL_FAILED_TARGET_DUELING",               // 0x68 Target is currently dueling
    "SPELL_FAILED_TARGET_ENEMY",                 // 0x69 Target is hostile
    "SPELL_FAILED_TARGET_ENRAGED",               // 0x6a
    "SPELL_FAILED_TARGET_FRIENDLY",              // 0x6b Target is friendly
    "SPELL_FAILED_TARGET_IN_COMBAT",             // 0x6c The target can't be in combat
    "SPELL_FAILED_TARGET_IS_PLAYER",             // 0x6d Can't target players
    "SPELL_FAILED_TARGET_NOT_DEAD",              // 0x6e Target is alive
    "SPELL_FAILED_TARGET_NOT_IN_PARTY",          // 0x6f Target is not in your party
    "SPELL_FAILED_TARGET_NOT_LOOTED",            // 0x70 Creature must be looted first
    "SPELL_FAILED_TARGET_NOT_PLAYER",            // 0x71 Target is not a player
    "SPELL_FAILED_TARGET_NO_POCKETS",            // 0x72 No pockets to pick
    "SPELL_FAILED_TARGET_NO_WEAPONS",            // 0x73 Target has no weapons equipped
    "SPELL_FAILED_TARGET_UNSKINNABLE",           // 0x74 Creature is not skinnable
    "SPELL_FAILED_THIRST_SATIATED",              // 0x75 hidden/unused
    "SPELL_FAILED_TOO_CLOSE",                    // 0x76 Target too close
    "SPELL_FAILED_TOO_MANY_OF_ITEM",             // 0x77
    "SPELL_FAILED_TOTEMS",                       // 0x78 client-side message
    "SPELL_FAILED_TRAINING_POINTS",              // 0x79 Not enough training points
    "SPELL_FAILED_TRY_AGAIN",                    // 0x7a Failed attempt
    "SPELL_FAILED_UNIT_NOT_BEHIND",              // 0x7b Target needs to be behind you
    "SPELL_FAILED_UNIT_NOT_INFRONT",             // 0x7c Target needs to be in front of you
    "SPELL_FAILED_WRONG_PET_FOOD",               // 0x7d Your pet doesn't like that food
    "SPELL_FAILED_NOT_WHILE_FATIGUED",           // 0x7e Can't cast while fatigued
    "SPELL_FAILED_TARGET_NOT_IN_INSTANCE",       // 0x7f Target must be in this instance
    "SPELL_FAILED_NOT_WHILE_TRADING",            // 0x80 Can't cast while trading
    "SPELL_FAILED_TARGET_NOT_IN_RAID",           // 0x81
    "SPELL_FAILED_DISENCHANT_WHILE_LOOTING",     // 0x82
    "SPELL_FAILED_PROSPECT_WHILE_LOOTING",       // 0x83
    "SPELL_FAILED_PROSPECT_NEED_MORE",           // 0x84 client-side message
    "SPELL_FAILED_TARGET_FREEFORALL",            // 0x85
    "SPELL_FAILED_NO_EDIBLE_CORPSES",            // 0x86 There are no nearby corpses to eat
    "SPELL_FAILED_ONLY_BATTLEGROUNDS",           // 0x87 Can only use in battlegrounds
    "SPELL_FAILED_TARGET_NOT_GHOST",             // 0x88 Target is not a ghost
    "SPELL_FAILED_TOO_MANY_SKILLS",              // 0x89 Your pet can't learn any more skills
    "SPELL_FAILED_TRANSFORM_UNUSABLE",           // 0x8a You can't use the new item
    "SPELL_FAILED_WRONG_WEATHER",                // 0x8b The weather isn't right for that
    "SPELL_FAILED_DAMAGE_IMMUNE",                // 0x8c
    "SPELL_FAILED_PREVENTED_BY_MECHANIC",        // 0x8d Can't do that while %s
    "SPELL_FAILED_PLAY_TIME",                    // 0x8e Maximum play time exceeded
    "SPELL_FAILED_REPUTATION",                   // 0x8f Your reputation isn't high enough
    "SPELL_FAILED_MIN_SKILL",                    // 0x90 Your skill is not high enough.
    "SPELL_FAILED_UNKNOWN",                      // 0x91 Unknown reason
];

/// `SPELL_FAILED_OUT_OF_RANGE`: the one reason that is measured as well as
/// shown, because the server is stating a distance this client can also
/// compute. Named rather than written as `0x59` at the call site, which is
/// this repository's rule for any wire value; the table above is the source
/// of the number.
///
/// Read by `crate::game::combat::desync` in the renderer.
pub const SPELL_FAILED_OUT_OF_RANGE: u8 = 0x59;

/// `SPELL_FAILED_DONT_REPORT`, which the client never displays. The server
/// substitutes it for the real reason on a passive spell, and also sends it
/// for several internal cases.
pub const DONT_REPORT: u8 = 0x17;

/// The `GlobalStrings.lua` key for a wire reason, or `None` for a code outside
/// the table or one the client does not display.
pub fn cast_failure_key(reason: u8) -> Option<&'static str> {
    if reason == DONT_REPORT {
        return None;
    }
    CAST_FAILURE_KEYS.get(usize::from(reason)).copied()
}


/// A talent's modifier as the server states it:
/// `SMSG_SET_FLAT_SPELL_MODIFIER` and `SMSG_SET_PCT_SPELL_MODIFIER`, three
/// bytes and a signed dword each.
///
/// One packet per bit, not per talent: `Player::SendSpellMod` loops over the
/// 64 bits of the modifier's `SpellFamilyFlags` mask and sends the running
/// total for each bit that is set. A talent affecting six spells sends six
/// packets, each carrying the sum of every modifier this character has for
/// that bit and that operation. A reader therefore replaces rather than
/// accumulates. Accumulating doubles a talent every time it is re-sent, and it
/// is re-sent at every login and every talent change.
///
/// The two opcodes differ only in what the number means: flat is added, and
/// percent is a percentage added to 100. Both are signed, and every cast-time
/// and cost reduction in the game is a negative percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellModifier {
    /// Which bit of a spell's `SpellFamilyFlags` this is about, `0..63`.
    pub effect_bit: u8,
    /// Which of the modifier operations — see [`spell_mod_op`].
    pub op: u8,
    /// The running total for this bit and this operation.
    pub value: i32,
    /// `true` for `SMSG_SET_PCT_SPELL_MODIFIER`.
    pub percent: bool,
}

/// The `SpellModOp` values this client reads, by their vmangos names.
///
/// The enum has 32 values and the server sends all of them. These five change
/// a number the client itself shows or predicts. The rest are the server's
/// own calculations (damage, threat, crit chance, proc chance), and this
/// client has no value of its own to apply them to.
pub mod spell_mod_op {
    /// How long the aura lasts, which is the `$d` a description prints.
    pub const DURATION: u8 = 1;
    /// How far the spell reaches.
    pub const RANGE: u8 = 5;
    /// The cast time.
    pub const CASTING_TIME: u8 = 10;
    /// The recovery, which is what the button's sweep is drawn from.
    pub const COOLDOWN: u8 = 11;
    /// The power cost of the cast.
    pub const COST: u8 = 14;
}

/// `SMSG_SET_FLAT_SPELL_MODIFIER` / `_PCT_`: `{u8 effect_bit, u8 op, i32 value}`.
pub fn parse_spell_modifier(body: &[u8], percent: bool) -> Option<SpellModifier> {
    let mut r = Reader::new(body);
    if !r.has(1 + 1 + 4) {
        return None;
    }
    let effect_bit = r.u8();
    let op = r.u8();
    let value = r.u32() as i32;
    // A mask has 64 bits. A bit index outside that range is refused, because
    // storing it under a wrapped index would modify the wrong spells.
    if effect_bit >= 64 {
        return None;
    }
    Some(SpellModifier { effect_bit, op, value, percent })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    /// `SMSG_UPDATE_AURA_DURATION` is five bytes, and the first is a byte.
    /// Reading the slot as a `u32` puts every timer on slot 0 and reads the
    /// duration three bytes short: a plausible number under the wrong icon.
    #[test]
    fn an_aura_duration_is_a_slot_byte_and_a_millisecond_word() {
        let mut w = Writer::new();
        w.u8(4);
        w.u32(30_000);
        assert_eq!(parse_aura_duration(&w.buf), Some((4, 30_000)));
        // A truncated body is refused rather than read as slot 4 with no time,
        // which would show a permanent buff as expiring this instant.
        assert_eq!(parse_aura_duration(&[4, 0, 0]), None);
    }

    /// `SMSG_LEVELUP_INFO` is twelve words and the interface reads nine of them.
    /// The four zero powers between the mana and the stats must be skipped: a
    /// reader that took the stats directly after the mana would report every
    /// level-up as granting nothing, and the `arg5 > 0` checks would hide the
    /// error.
    #[test]
    fn a_level_up_carries_its_health_its_mana_and_five_stats() {
        let mut w = Writer::new();
        w.u32(12).u32(42).u32(30);
        for _ in 0..4 {
            w.u32(0); // rage, focus, energy, happiness — always zero in 1.12
        }
        for stat in [1u32, 1, 2, 1, 1] {
            w.u32(stat);
        }
        let gained = parse_levelup(&w.buf).expect("a level-up");
        assert_eq!(gained.level, 12);
        assert_eq!(gained.health, 42);
        assert_eq!(gained.mana, 30);
        assert_eq!(gained.stats, [1, 1, 2, 1, 1]);
        // Short of the full twelve words is refused: the stats would otherwise
        // read as zeroes, which is indistinguishable from a level that granted
        // none.
        assert_eq!(parse_levelup(&w.buf[..40]), None);
    }

    /// The buff cancel is keyed by spell id, where every other aura packet in 1.12
    /// is keyed by slot. See [`cancel_aura_body`].
    #[test]
    fn the_buff_cancel_names_a_spell_and_not_a_slot() {
        assert_eq!(cancel_aura_body(168), vec![168, 0, 0, 0]);
    }

    /// The spellbook's ids are `u16`, and the slot beside each one is not a
    /// slot. Reading the pair as one `u32` gives a spellbook that looks
    /// plausible and is wrong: every id doubled and every other one dropped.
    #[test]
    fn the_spellbook_is_pairs_of_u16() {
        let mut w = Writer::new();
        w.u8(0);
        w.u16(3);
        for id in [133u16, 168, 6603] {
            w.u16(id).u16(0);
        }
        w.u16(0); // no cooldowns

        let book = parse_initial_spells(&w.buf).expect("a spellbook");
        assert_eq!(book.known, vec![133, 168, 6603]);
        assert!(book.cooldowns.is_empty());
    }

    /// The three spellbook change packets use different widths and lengths:
    /// superseded is two `u16`s, learned is a `u16` and a trailing field, and
    /// removed is a single `u16`.
    #[test]
    fn the_three_spellbook_deltas_are_three_different_lengths() {
        let mut w = Writer::new();
        w.u16(11566).u16(11567);
        assert_eq!(parse_superceded_spell(&w.buf), Some((11566, 11567)));

        // Heroic Strike Rank 7 -> Rank 8, the pair measured on this project's
        // warrior. `vale spellbook 11566` names the ranks: 7 and 8, not 8 and 9.
        // The rank number is the label, not the spell id.
        // A body one byte short is refused rather than misread.
        assert_eq!(parse_superceded_spell(&w.buf[..3]), None);
        assert_eq!(parse_superceded_spell(&[]), None);

        assert_eq!(parse_removed_spell(&[0x2e, 0x2d]), Some(11566));
    }

    /// `SMSG_LEARNED_SPELL` is a `u16` and a second field, not a `u32`.
    ///
    /// It was read as a `u32` here for a long time and gave the right answer
    /// every time, because vmangos never assigns the trailing `actionBarSlot`
    /// and a zero high half makes the two readings equal. This test puts a value
    /// in that field: the `u32` reading would give 0x0005_2D2E (339,758), which is
    /// no spell.
    #[test]
    fn a_learned_spell_is_the_low_half_and_the_slot_beside_it_is_not_part_of_it() {
        let mut w = Writer::new();
        w.u16(11567).u16(5);
        assert_eq!(parse_learned_spell(&w.buf), Some(11567));

        // The ordinary packet, where that field is zero, reads the same either way,
        // which is why the wrong width produced no visible error.
        let mut plain = Writer::new();
        plain.u16(11567).u16(0);
        assert_eq!(parse_learned_spell(&plain.buf), Some(11567));

        assert_eq!(parse_learned_spell(&[0x2f]), None);
    }

    /// The cooldown block is behind the spell list, so a miscount above it
    /// reads durations out of spell ids. A body that stops before it loses
    /// only the cooldowns.
    #[test]
    fn the_cooldowns_are_read_across_the_spell_list() {
        let mut w = Writer::new();
        w.u8(0);
        w.u16(1);
        w.u16(1856).u16(0);
        w.u16(1);
        w.u16(1856).u16(0).u16(133).u32(12_000).u32(0);

        let book = parse_initial_spells(&w.buf).expect("a spellbook");
        assert_eq!(book.known, vec![1856]);
        assert_eq!(
            book.cooldowns,
            vec![InitialCooldown {
                spell_id: 1856,
                item_id: 0,
                category: 133,
                spell_ms: 12_000,
                category_ms: 0,
            }]
        );
        assert!(!book.cooldowns[0].is_permanent());

        // Truncated after the spell list: the book survives, the tail does not.
        let short = &w.buf[..w.buf.len() - 6];
        let book = parse_initial_spells(short).expect("a spellbook");
        assert_eq!(book.known, vec![1856]);
        assert!(book.cooldowns.is_empty());
    }

    /// The permanent marker is a flag in the top bit of the category duration.
    /// Read as a duration it is 24 days, which looks the same as "ready" to code
    /// that checks only whether it has elapsed.
    #[test]
    fn a_permanent_cooldown_is_a_flag_and_not_a_duration() {
        let cd = InitialCooldown {
            spell_id: 1,
            item_id: 0,
            category: 0,
            spell_ms: 1,
            category_ms: 0x8000_0000,
        };
        assert!(cd.is_permanent());
        assert_eq!(cd.category_duration_ms(), 0);
    }

    /// The action bar is 120 bare words with no count in front, and the top
    /// byte of each is its kind. An empty slot is a zero word and is skipped.
    #[test]
    fn an_action_button_is_a_packed_word() {
        let mut w = Writer::new();
        w.u32(SPELL_ATTACK); // slot 0: Attack, kind SPELL (0)
        w.u32(0); // slot 1: empty
        w.u32(133 | (u32::from(action_kind::SPELL) << 24)); // slot 2: Fireball
        w.u32(6948 | (u32::from(action_kind::ITEM) << 24)); // slot 3: a hearthstone
        let buttons = parse_action_buttons(&w.buf);
        assert_eq!(
            buttons,
            vec![
                ActionButton { slot: 0, action: SPELL_ATTACK, kind: action_kind::SPELL },
                ActionButton { slot: 2, action: 133, kind: action_kind::SPELL },
                ActionButton { slot: 3, action: 6948, kind: action_kind::ITEM },
            ]
        );
    }

    /// The same word, written back. The kind is the top byte in both directions.
    /// A second packing function could get that wrong without any visible error,
    /// since `HandleSetActionButtonOpcode` drops an unknown type without a reply.
    #[test]
    fn a_slot_is_written_back_in_the_shape_it_arrived() {
        // Round-trip: pack what the parser produced and it is the same word.
        let mut w = Writer::new();
        w.u32(ActionButton::packed(6948, action_kind::ITEM));
        let back = parse_action_buttons(&w.buf);
        assert_eq!(
            back,
            vec![ActionButton { slot: 0, action: 6948, kind: action_kind::ITEM }]
        );
        // The five bytes on the wire: slot, then the word little-endian.
        assert_eq!(
            set_action_button_body(2, ActionButton::packed(133, action_kind::SPELL)),
            vec![2, 0x85, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            set_action_button_body(0, ActionButton::packed(6948, action_kind::ITEM)),
            vec![0, 0x24, 0x1b, 0x00, 0x80]
        );
        // A removal is the same packet with a zero word, not the absence of a
        // packet; this is the server's `if (!packetData)` branch.
        assert_eq!(set_action_button_body(11, 0), vec![11, 0, 0, 0, 0]);
    }

    /// The four extra bars are four bits of one byte, in the interface's argument
    /// order.
    ///
    /// Tested as values rather than as an enum, because the whole packet is this
    /// byte and a transposition would not be visible: swapping bars 1 and 2 puts
    /// the same two rows of buttons on screen in each other's places, with no
    /// error, and the change shows only after a relog.
    #[test]
    fn the_four_extra_bars_are_the_low_four_bits() {
        assert_eq!(multi_bar::BOTTOM_LEFT, 1);
        assert_eq!(multi_bar::BOTTOM_RIGHT, 2);
        assert_eq!(multi_bar::RIGHT, 4);
        assert_eq!(multi_bar::LEFT, 8);
        assert_eq!(multi_bar::ALL, 0x0F);
        // One byte, and nothing else — no slot, no count, no padding.
        assert_eq!(set_actionbar_toggles_body(multi_bar::ALL), vec![0x0F]);
        assert_eq!(set_actionbar_toggles_body(0), vec![0]);
        assert_eq!(
            set_actionbar_toggles_body(multi_bar::BOTTOM_LEFT | multi_bar::RIGHT),
            vec![0x05]
        );
    }

    /// A fifth bit cannot be sent. `SetActionBarToggles` takes five Lua arguments
    /// and the client packs four, so "Always Show ActionBars" is a saved variable
    /// rather than a saved field. A mask that carried it would set a bit in
    /// `PLAYER_FIELD_BYTES` that the server returns unchanged and nothing reads.
    #[test]
    fn the_always_show_flag_is_not_a_fifth_bit() {
        assert_eq!(set_actionbar_toggles_body(0xFF), vec![0x0F]);
        assert_eq!(set_actionbar_toggles_body(0x10), vec![0]);
    }

    /// A cast result is also sent on success. Treating every arrival as a failure
    /// would cancel every cast bar as soon as it appears.
    #[test]
    fn a_cast_result_says_accepted_as_well_as_refused() {
        let mut ok = Writer::new();
        ok.u32(133).u8(0);
        assert_eq!(
            parse_cast_result(&ok.buf),
            Some(CastResult { spell_id: 133, failure: None, requirement: None })
        );

        let mut failed = Writer::new();
        failed.u32(133).u8(2).u8(0x4d); // SPELL_FAILED_NO_POWER
        let result = parse_cast_result(&failed.buf).expect("a result");
        assert_eq!(result.failure, Some(0x4d));
        assert_eq!(cast_failure_key(0x4d), Some("SPELL_FAILED_NO_POWER"));
        assert_eq!(result.requirement, None, "this reason carries no tail");
    }

    /// The three equipped-item refusals carry the argument to their `%s`, and
    /// this is what warriors reported: shown without it,
    /// `SPELL_FAILED_EQUIPPED_ITEM_CLASS_MAINHAND` reads "Must have a %s
    /// equipped in the main hand" on screen.
    #[test]
    fn the_equipped_item_refusals_carry_the_gear_they_wanted() {
        // Rend's shape: class 2 (Weapon), a mask of melee subclasses.
        let mut w = Writer::new();
        w.u32(772).u8(2).u8(0x1a).u32(2).u32(0b0101_0000_0101).u32(0);
        let result = parse_cast_result(&w.buf).expect("a result");
        assert_eq!(result.failure, Some(0x1a));
        assert_eq!(
            result.requirement,
            Some(EquipRequirement {
                class: 2,
                subclass_mask: 0b0101_0000_0101,
                inventory_type_mask: 0,
            })
        );
        assert_eq!(
            cast_failure_key(0x1a),
            Some("SPELL_FAILED_EQUIPPED_ITEM_CLASS_MAINHAND")
        );

        // A truncated tail is not read as a partial argument. A body that ends after
        // the reason byte must give `None` rather than reading past the end; the
        // requirement is three `u32`s or nothing.
        let mut short = Writer::new();
        short.u32(772).u8(2).u8(0x1a).u32(2);
        let result = parse_cast_result(&short.buf).expect("a result");
        assert_eq!(result.failure, Some(0x1a), "the reason still reads");
        assert_eq!(result.requirement, None);

        // A reason that is not one of the three never reads a tail, even when bytes
        // follow it.
        let mut other = Writer::new();
        other.u32(772).u8(2).u8(0x12).u32(2).u32(4).u32(0);
        assert_eq!(parse_cast_result(&other.buf).expect("a result").requirement, None);
    }

    /// The table is an index, so its length is the check: 146 for build 5875,
    /// the count in vmangos' enum and the number of reasons the 1.12.1 client
    /// recognises.
    #[test]
    fn the_failure_table_is_the_1_12_enum_in_order() {
        assert_eq!(CAST_FAILURE_KEYS.len(), 146);
        assert_eq!(cast_failure_key(0x00), Some("SPELL_FAILED_AFFECTING_COMBAT"));
        assert_eq!(cast_failure_key(0x59), Some("SPELL_FAILED_OUT_OF_RANGE"));
        // The named constant is the same code. It is named so that a call site
        // cannot test the wrong byte; a wrong byte would measure nothing and report
        // no error.
        assert_eq!(
            cast_failure_key(SPELL_FAILED_OUT_OF_RANGE),
            Some("SPELL_FAILED_OUT_OF_RANGE")
        );
        assert_eq!(cast_failure_key(0x91), Some("SPELL_FAILED_UNKNOWN"));
        // A code past the end of the table gives `None` rather than a panic or
        // a wrong message.
        assert_eq!(cast_failure_key(0x92), None);
        assert_eq!(cast_failure_key(0xFF), None);
        // The reason the client never displays.
        assert_eq!(cast_failure_key(DONT_REPORT), None);
    }

    /// A self-cast carries no guid. The mask is zero and the body is six bytes.
    /// Sending the current selection instead makes Ice Armor fail with "Invalid
    /// target".
    #[test]
    fn a_self_cast_sends_no_target_and_a_unit_cast_sends_a_packed_one() {
        assert_eq!(
            cast_spell_body(1459, CastTarget::SelfImplicit),
            vec![0xB3, 0x05, 0x00, 0x00, 0x00, 0x00]
        );

        let body = cast_spell_body(133, CastTarget::Unit(0xF130_0000_0001_2345));
        let mut expected = Writer::new();
        expected.u32(133).u16(target_flag::UNIT).packed_guid(0xF130_0000_0001_2345);
        assert_eq!(body, expected.buf);
        // The guid is in packed form: a plain guid would be eight bytes with no mask
        // in front.
        assert_eq!(body.len(), 4 + 2 + 1 + 5);
    }

    /// A ground-targeted cast is three bare floats and no guid: the shape
    /// `SpellCastTargets::read` reads for `TARGET_FLAG_DEST_LOCATION`, in that
    /// order and in the server's axes.
    ///
    /// The length is the important assertion: 3.x puts a packed guid in front of
    /// the coordinates and 1.12 does not, so a body four bytes longer than this
    /// is read past its end by the server and dropped.
    #[test]
    fn a_placed_cast_sends_three_floats_and_no_guid() {
        // Blizzard, somewhere in Elwynn.
        let body = cast_spell_body(10, CastTarget::Dest([-9449.5, -12.0, 56.25]));
        let mut expected = Writer::new();
        expected
            .u32(10)
            .u16(target_flag::DEST_LOCATION)
            .f32(-9449.5)
            .f32(-12.0)
            .f32(56.25);
        assert_eq!(body, expected.buf);
        assert_eq!(body.len(), 4 + 2 + 12);

        // The item use packet writes the same block, because both opcodes are read by
        // the same `SpellCastTargets::read` on the server.
        let used = crate::play::items::use_item_body(255, 23, 0, CastTarget::Dest([1.0, 2.0, 3.0]));
        assert_eq!(&used[3..], &cast_spell_body(0, CastTarget::Dest([1.0, 2.0, 3.0]))[4..]);
    }

    /// `SMSG_ATTACKSTART` streams plain guids and `SMSG_ATTACKSTOP` packed ones,
    /// one function apart in `Unit.cpp`.
    #[test]
    fn the_attack_pair_disagree_about_packing() {
        let mut start = Writer::new();
        start.u64(7).u64(0xF130_0000_0001_2345);
        assert_eq!(parse_attack_start(&start.buf), Some((7, 0xF130_0000_0001_2345)));

        let mut stop = Writer::new();
        stop.packed_guid(7).packed_guid(0xF130_0000_0001_2345).u32(0);
        assert_eq!(parse_attack_stop(&stop.buf), Some((7, 0xF130_0000_0001_2345)));

        // `SendAttackStop(nullptr)` writes an empty packed guid for the victim:
        // "stop attacking, nobody in particular".
        let mut cleared = Writer::new();
        cleared.packed_guid(7).packed_guid(0).u32(0);
        assert_eq!(parse_attack_stop(&cleared.buf), Some((7, 0)));
    }

    /// The guid is second in `SMSG_COOLDOWN_EVENT` and first in
    /// `SMSG_SPELL_COOLDOWN`.
    #[test]
    fn the_two_cooldown_packets_put_the_guid_at_opposite_ends() {
        let mut event = Writer::new();
        event.u32(1784).u64(9);
        assert_eq!(parse_cooldown_event(&event.buf), Some((1784, 9)));

        let mut list = Writer::new();
        list.u64(9).u32(133).u32(1500).u32(168).u32(0);
        assert_eq!(
            parse_spell_cooldowns(&list.buf),
            Some(SpellCooldowns {
                guid: 9,
                entries: vec![(133, 1500), (168, 0)],
            })
        );
    }

    /// Each of the five refusals is an opcode with an empty body, and each names
    /// a string the game itself ships.
    #[test]
    fn every_swing_refusal_maps_to_one_of_the_games_own_strings() {
        use crate::opcodes::Opcode;
        assert_eq!(
            AttackRefusal::of(Opcode::SMSG_ATTACKSWING_NOTINRANGE),
            Some(AttackRefusal::NotInRange)
        );
        assert_eq!(AttackRefusal::NotInRange.key(), "ERR_BADATTACKPOS");
        assert_eq!(AttackRefusal::BadFacing.key(), "ERR_BADATTACKFACING");
        assert_eq!(AttackRefusal::of(Opcode::SMSG_ATTACKSTOP), None);
    }
    /// A moved action bar slot sends the word that comes back at the next login.
    ///
    /// The round trip that drag-and-drop depends on, as far as it can be checked
    /// without a server: the word `set_action_button_body` sends is the word
    /// `parse_action_buttons` reads, and vmangos' `HandleSetActionButtonOpcode`
    /// splits it with the same two macros: `ACTION_BUTTON_ACTION` is the low 24
    /// bits and `ACTION_BUTTON_TYPE` the top byte.
    ///
    /// A zero word is a removal on both sides:
    /// `if (!packet.packetData) removeActionButton(button)`. This client sends a
    /// zero word for a slot emptied by a pick-up.
    #[test]
    fn a_moved_slot_round_trips_through_the_packed_word() {
        // A spell, an item and a macro — the three kinds a slot can hold.
        for (action, kind) in [
            (133u32, action_kind::SPELL),
            (13446, action_kind::ITEM),
            (7, action_kind::MACRO),
        ] {
            let packed = ActionButton::packed(action, kind);
            let body = set_action_button_body(11, packed);
            assert_eq!(body.len(), 5, "u8 slot, u32 word");
            assert_eq!(body[0], 11, "zero-based on the wire");
            assert_eq!(u32::from_le_bytes([body[1], body[2], body[3], body[4]]), packed);

            // The server's own split, which is what it stores and sends back in
            // `SMSG_ACTION_BUTTONS`.
            assert_eq!(packed & 0x00FF_FFFF, action, "ACTION_BUTTON_ACTION");
            assert_eq!((packed >> 24) as u8, kind, "ACTION_BUTTON_TYPE");
        }

        // An emptied slot is a zero word, which the server reads as a removal
        // rather than as "spell 0".
        assert_eq!(set_action_button_body(11, 0)[1..], [0, 0, 0, 0]);
    }

    /// Three bytes and a signed dword. The two opcodes differ only in what the
    /// number means; see [`SpellModifier`].
    #[test]
    fn a_spell_modifier_is_a_bit_an_operation_and_a_signed_total() {
        let body = |bit: u8, op: u8, value: i32| {
            let mut out = vec![bit, op];
            out.extend_from_slice(&value.to_le_bytes());
            out
        };
        let flat = parse_spell_modifier(&body(3, spell_mod_op::CASTING_TIME, -500), false)
            .expect("flat");
        assert_eq!(flat.effect_bit, 3);
        assert_eq!(flat.op, spell_mod_op::CASTING_TIME);
        assert_eq!(flat.value, -500, "a reduction is a negative, not a magnitude");
        assert!(!flat.percent);
        let pct =
            parse_spell_modifier(&body(3, spell_mod_op::COST, -20), true).expect("percent");
        assert!(pct.percent);
        assert_eq!(pct.value, -20);
        // A bit outside the mask's 64 is refused, because filing it under a wrapped
        // index would modify the wrong spells.
        assert!(parse_spell_modifier(&body(64, 0, 1), false).is_none());
        assert!(parse_spell_modifier(&body(63, 0, 1), false).is_some());
        // A short body is also refused.
        assert!(parse_spell_modifier(&[0, 0, 0, 0, 0], false).is_none());
    }

    /// The trade slot's "guid" is a slot number. See [`CastTarget::TradeSlot`],
    /// which quotes how the server reads it.
    #[test]
    fn a_trade_slot_target_writes_the_slot_where_a_guid_would_go() {
        let body = cast_spell_body(7418, CastTarget::TradeSlot(6));
        // spell id, then the flag, then the packed "guid".
        assert_eq!(&body[..4], &7418u32.to_le_bytes());
        assert_eq!(&body[4..6], &target_flag::TRADE_ITEM.to_le_bytes());
        // A packed guid of 6 is one mask byte and one value byte.
        assert_eq!(&body[6..], &[0x01, 0x06]);
    }

}
