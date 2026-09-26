//! **What this character can do, and what the server says back when it tries.**
//!
//! [`crate::play::action`] is what a unit is visibly *doing* — packets about anybody,
//! inbound only, that drive an animation. This is the other half: the spellbook
//! the server hands us at login, the action bar it remembers for us, the
//! cooldowns that gate the buttons, and the two packets that answer a press —
//! `SMSG_CAST_RESULT` for a spell and the five `SMSG_ATTACKSWING_*` refusals for
//! a swing. Plus the outbound bodies, because a body lives beside the thing it
//! is about.
//!
//! Three things here are worth knowing before touching any of it.
//!
//! **The failure code is an index, not a value.** `SMSG_CAST_RESULT` carries one
//! byte, and what it names is a *position* in an enum the client and the server
//! agree on by construction — [`CAST_FAILURE_KEYS`] is that enum in order, and
//! an off-by-one there does not fail, it says "Out of range" when the server
//! said "Not enough mana". The list is 146 long for build 5875 and it is
//! transcribed from vmangos' `SpellCastResult`; every `#if` guard in that header
//! is `> CLIENT_BUILD_1_x` for an x below 1.12, so **1.12 gets all of them** and
//! declaration order is the wire value. Two independent readings agree on the
//! length: vmangos' enum, and the 146-entry name table the client itself
//! carries.
//!
//! **The message is the game's own.** The keys above are `GlobalStrings.lua`
//! keys — `Interface\FrameXML\GlobalStrings.lua` is in the archives, 4,592 of
//! them — so nothing in this client invents the words "You are too far away!".
//! See `vale_assets::interface::strings`. A key that is *absent* displays as nothing,
//! which is the client's own behaviour for the three hidden reasons.
//!
//! **A cast's target block is a mask, not a guid.** `SpellCastTargets::read`
//! begins with a `u16` and reads only what the mask claims; the three shapes
//! this client sends are `TARGET_FLAG_SELF` (which is **zero**, and carries
//! nothing at all — the server fills the target in from the spell's own implicit
//! targeting), `TARGET_FLAG_UNIT` followed by a *packed* guid, and
//! `TARGET_FLAG_DEST_LOCATION` followed by three bare `f32`s and **no guid in
//! front of them in this build**. Sending a selection with a self-cast is the
//! "Invalid target" bug; sending a plain guid where a packed one belongs is
//! eight bytes read as one; and sending a *unit* where a destination belongs is
//! the quiet one — the server substitutes the caster's own position rather than
//! refusing, so the spell lands underfoot and reports success.
//!
//! Source: vmangos `Handlers/SpellHandler.cpp` (`HandleCastSpellOpcode`),
//! `Spells/Spell.cpp` (`SpellCastTargets::read`, `Spell::SendCastResult`),
//! `Objects/Player.cpp` (`SendInitialSpells`, `SendSpellCooldown`),
//! `Chat/MasterPlayer.cpp` (`SendInitialActionButtons`).

use crate::bytes::{Reader, Writer};

// ---------------------------------------------------------------------------
// The spellbook
// ---------------------------------------------------------------------------

/// `SMSG_INITIAL_SPELLS`: every spell this character knows, said once at login.
///
/// **Nothing else ever states the spellbook.** It arrives in the login burst,
/// and after that the server only sends *differences* (`SMSG_LEARNED_SPELL`,
/// `SMSG_REMOVED_SPELL`), so a client that misses this packet has an empty
/// action bar for the whole session and no error anywhere.
///
/// ```text
/// u8  unknown (0)
/// u16 spellCount
/// (u16 spellId, u16 slot)  x spellCount      slot is "not slot id" — always 0
/// u16 cooldownCount
/// (u16 spellId, u16 itemId, u16 category, u32 spellMs, u32 categoryMs) x n
/// ```
///
/// **The spell ids are `u16` here**, alone among the places this client meets a
/// spell id — every other one is a `u32`. 1.12's spell ids fit, and the packet
/// predates them not fitting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spellbook {
    /// In the order the server listed them, which is the order the character
    /// learned them. The client's own spellbook sorts by skill line; this is the
    /// raw list.
    pub known: Vec<u32>,
    /// Whatever was still on cooldown at login — an empty list is the normal
    /// case for a character who has been logged out for a while.
    pub cooldowns: Vec<InitialCooldown>,
}

/// One entry of [`Spellbook::cooldowns`]: a spell that is not ready yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialCooldown {
    pub spell_id: u32,
    /// The item that started it, for an item-use cooldown, and 0 for a plain
    /// spell. A cooldown record is keyed on the *pair* — the same trinket used
    /// twice shares one record and two different trinkets casting one spell do
    /// not.
    pub item_id: u32,
    /// `Spell.dbc`'s category: the shared bucket a whole family recovers on
    /// (every healthstone, every bandage).
    pub category: u32,
    /// Milliseconds left on the spell's own recovery.
    pub spell_ms: u32,
    /// …and on the category's. **The top bit is a flag, not a duration**:
    /// `SendInitialSpells` sets `0x80000000` on a permanent cooldown (and puts
    /// 1 ms in `spell_ms` beside it), which is a spell that never comes back
    /// this session rather than one that comes back in 24 days.
    pub category_ms: u32,
}

impl InitialCooldown {
    /// The `0x80000000` marker above, and the category duration without it.
    pub fn is_permanent(&self) -> bool {
        self.category_ms & 0x8000_0000 != 0
    }

    pub fn category_duration_ms(&self) -> u32 {
        self.category_ms & 0x7FFF_FFFF
    }
}

/// A sanity bound on the two counted arrays. Both are `u16` off the wire and a
/// packet read at the wrong offset produces an enormous one; refusing is better
/// than allocating on a number that came from nowhere.
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

    // The cooldown block is the tail and a truncated one costs only the
    // cooldowns: the spellbook above it is what makes the bar work at all, and
    // an unknown cooldown shows as a ready button that the server refuses.
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

/// How many buttons the server remembers. `MAX_ACTION_BUTTONS` in
/// `Objects/Player.h`; the packet is exactly this many `u32`s with no count in
/// front of it, so the number *is* the framing.
pub const ACTION_BUTTONS: usize = 120;

/// One occupied slot of `SMSG_ACTION_BUTTONS`.
///
/// The word is packed: the low 24 bits are what the button *does* and the top
/// byte is what kind of thing that is. An empty slot is a zero word and is not
/// reported here at all.
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
    /// **This button as one word**, which is the only form the wire has —
    /// [`parse_action_buttons`]' inverse, and what
    /// [`set_action_button_body`] sends.
    ///
    /// The kind is the top byte and the action the low 24 bits, so a caller
    /// cannot pack the two the wrong way round in two places. The action is
    /// masked rather than asserted: `Player::IsActionButtonDataValid` refuses
    /// anything at or above `MAX_ACTION_BUTTON_ACTION_VALUE` outright, so a
    /// value that would collide with the kind byte is a refusal either way and
    /// truncating it here keeps the packing total.
    pub fn packed(action: u32, kind: u8) -> u32 {
        (action & 0x00FF_FFFF) | (u32::from(kind) << 24)
    }
}

/// `enum ActionButtonType` — the top byte of the packed word.
pub mod action_kind {
    pub const SPELL: u8 = 0x00;
    /// "click?", says the header beside it.
    pub const CLICK: u8 = 0x01;
    pub const MACRO: u8 = 0x40;
    pub const CLICK_MACRO: u8 = 0x41;
    pub const ITEM: u8 = 0x80;
}

/// `CMSG_SET_ACTION_BUTTON`: `u8 slot`, `u32 packed` — **and the packet is sent
/// for a removal too**, with a zero word.
///
/// The bar is the *client's* state and this is how the server is told to
/// remember it; nothing comes back. `HandleSetActionButtonOpcode` reads exactly
/// these five bytes, treats a zero word as `removeActionButton(slot)`, and
/// otherwise validates the pair — a spell the character does not know, a passive
/// one, or an item entry with no prototype is dropped in silence.
///
/// **The slot is zero-based here**, as it is in the packet the server sends
/// back at login, and one-based everywhere the interface touches it. The
/// crossing is the caller's; see [`crate::play::spells::ACTION_BUTTONS`] for the
/// bound, which `u8` does not quite express (120 fits, 250 does not).
///
/// The same as the client's own sender, which writes the slot as a byte and then `actionButtons[slot]` as a dword — the same array
/// [`parse_action_buttons`] fills — before firing `ACTIONBAR_SLOT_CHANGED` with
/// `slot + 1`.
pub fn set_action_button_body(slot: u8, packed: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(slot);
    w.u32(packed);
    w.buf
}

/// **Which bit of `PLAYER_FIELD_BYTES`' third byte is which extra bar**, and the
/// order is the interface's own argument order rather than the screen's.
///
/// `SetActionBarToggles(a, b, c, d, alwaysShow)` is called from exactly one place
/// in `Interface\FrameXML\` — `UIOptionsFrame_Save` — and the client packs its
/// first four arguments into bits 0..3 in order. `MultiActionBars.lua` then spends those four
/// numbers on the four frames, which is where the names come from.
///
/// **The fifth argument goes nowhere.** `ALWAYS_SHOW_MULTIBARS` — the "Always
/// Show ActionBars" checkbox — is passed to the C function and the C function
/// stops at four bits, so it is a saved *variable* rather than a saved field and
/// this client owes it nothing. Stated because a five-bit mask reads perfectly
/// plausibly and would set a bit the server hands straight back.
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
    /// **It is not shown on its own**: `MultiActionBar_Update` draws the left
    /// column only when bit 2 is set as well, which is the file's own
    /// `SHOW_MULTI_ACTIONBAR_3 and SHOW_MULTI_ACTIONBAR_4`. The bit still means
    /// what it says; the conjunction is the interface's.
    pub const LEFT: u8 = 0x08;
    /// The four of them, for a mask that cannot carry the fifth argument.
    pub const ALL: u8 = BOTTOM_LEFT | BOTTOM_RIGHT | RIGHT | LEFT;
}

/// `CMSG_SET_ACTIONBAR_TOGGLES`: **one byte, and that is the whole packet**.
///
/// The mask of [`multi_bar`] bits, which the server stores in
/// `PLAYER_FIELD_BYTES` byte 2 and hands back in the next values block — so the
/// four bars survive a logout, and are the one piece of interface layout in 1.12
/// that does. Nothing is answered: `HandleSetActionBarTogglesOpcode` is a
/// `SetByteValue` and a return.
///
/// **Nothing local is written here either**, which is the opposite of
/// [`set_action_button_body`]'s rule and is the client's own: it packs the
/// four Lua arguments, sends, and does not touch the player object. The
/// interface has already moved the frames itself (`MultiActionBar_Update` runs
/// from the checkbox's `OnClick`, not from the save), so the field is only ever
/// read again at the *next* `PLAYER_ENTERING_WORLD`.
///
/// The high four bits are masked off rather than trusted — see [`multi_bar`] for
/// the fifth argument that looks like it belongs in them and does not.
pub fn set_actionbar_toggles_body(mask: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(mask & multi_bar::ALL);
    w.buf
}

/// Parse `SMSG_ACTION_BUTTONS`, keeping only the occupied slots.
///
/// A short body is read as far as it goes rather than refused: the bar is
/// cosmetic state and half of one is strictly better than none.
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

/// The pseudo-spell every character carries in slot 1: **Attack**.
///
/// It is not a cast. Pressing it sends `CMSG_ATTACKSWING` at the current
/// selection and toggles melee; `CMSG_CAST_SPELL` with this id would be refused
/// by `HandleCastSpellOpcode`'s "which he shouldn't have" branch. 6603 is the
/// only spell in the game carrying `SPELL_EFFECT_ATTACK`, which is how the real
/// client recognises it.
pub const SPELL_ATTACK: u32 = 6603;

// ---------------------------------------------------------------------------
// Cooldowns
// ---------------------------------------------------------------------------

/// `SMSG_SPELL_COOLDOWN`: the server's **override** path for cooldowns.
///
/// Not the ordinary one. A plain cast's own recovery is computed by the client
/// from `Spell.dbc` and started when its `SMSG_SPELL_GO` comes back — vmangos
/// sends no packet for it at all. This is what arrives for a school lockout (a
/// counterspell), a pet's list, or a GM's reset, and it may name several spells.
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

/// `SMSG_COOLDOWN_EVENT`: **start the cooldown that was parked**.
///
/// The other half of `SPELL_ATTR_COOLDOWN_ON_EVENT` — Stealth and Feign Death
/// take their recovery when they *break*, not when they are cast, so the client
/// inserts the record on hold and this releases it. `u32 spellId`, then a plain
/// `u64` guid, in that order (the guid is second here and first in
/// `SMSG_SPELL_COOLDOWN`, which is the sort of asymmetry that reads as a
/// cooldown on spell 0).
pub fn parse_cooldown_event(body: &[u8]) -> Option<(u32, u64)> {
    let mut r = Reader::new(body);
    if !r.has(4 + 8) {
        return None;
    }
    let spell_id = r.u32();
    Some((spell_id, r.u64()))
}

/// `SMSG_CLEAR_COOLDOWN`: **this cooldown is over now**, whatever it had left.
///
/// The half of the cooldown conversation this client had never read, and the
/// one that makes a school lockout look wrong. `Player::LockOutSpells` sends
/// `SMSG_SPELL_COOLDOWN` naming every spell of the school and its full
/// duration; `Player::RemoveSpellLockout` sends **one of these per spell** when
/// the lockout is lifted early. Ignore them and every swirl runs to the full
/// length the lockout was *stated* at, which is right about as often as it is
/// wrong — the "displays inconsistently" report.
///
/// It is not only interrupts. vmangos sends it from four places
/// (`Player.cpp:22354`, `:22365`, `:22381`, `:22469`) — any cooldown removal,
/// the warlock Ritual of Doom fix-up, and the no-cooldown cheat.
///
/// ## The layout is a reading, and it is one that cannot lie quietly
///
/// `u32 spellId` then `u64 guid`, which is [`parse_cooldown_event`]'s shape —
/// the same server, the same subject, the fields in the order
/// `SendClearCooldown(spellId, target)` assigns them. What is *not* measured is
/// the serialiser itself: `WorldPackets::Spell::ClearCooldown` is a packet class
/// this session could not fetch, and the client's `SMSG_CLEAR_COOLDOWN` handler
/// was not checked.
///
/// Two things make the reading safe rather than merely likely. **The body must
/// be exactly twelve bytes** — a `u64`-first layout would still be twelve, so
/// that is not the check — and **the caller compares the guid against the
/// player's own**, which the server always sends. Reversed, the "guid" would be
/// a spell id in its low half and would not match, so a wrong reading refuses
/// every packet instead of clearing a random spell. That is the direction to be
/// wrong in, and it is visible: the refusals land in the session's warning
/// channel.
pub fn parse_clear_cooldown(body: &[u8]) -> Option<(u32, u64)> {
    if body.len() != 4 + 8 {
        return None;
    }
    let mut r = Reader::new(body);
    let spell_id = r.u32();
    Some((spell_id, r.u64()))
}

// ---------------------------------------------------------------------------
// What the server says about a press
// ---------------------------------------------------------------------------

/// `SMSG_CAST_RESULT`: the answer to our own `CMSG_CAST_SPELL`.
///
/// **Sent on success as well as on failure**, which is why the failure is an
/// `Option` rather than the packet being one: `status` is 0 for "the cast was
/// accepted" and 2 for "it was not", and the reason byte only follows the 2.
/// A success carries nothing the client needs — the cast itself arrives as
/// `SMSG_SPELL_START`/`SMSG_SPELL_GO` like anybody else's — but its *absence*
/// is what tells a cast bar to keep running.
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
    /// **What the refusal is about**, for the three reasons that say so.
    ///
    /// See [`EquipRequirement`]: without it the message is drawn with its own
    /// `%s` still in it, which is what the warrior report was.
    pub requirement: Option<EquipRequirement>,
}

/// The gear a refused cast wanted, as `Spell::SendCastResult` states it.
///
/// **This is the argument to a `%s`, and it is the whole reason the tail is
/// read at all.** `GlobalStrings.lua` spells the three equipped-item refusals
/// as `"Must have a %s equipped"` and its two per-hand variants, so a client
/// that stops at the reason byte draws the placeholder verbatim — which is
/// exactly what a warrior pressing Rend without a weapon used to see.
///
/// The values are the *spell's* own `Spell.dbc` columns rather than anything
/// about the character, which is why one number can name the requirement: the
/// server is quoting the row it just checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipRequirement {
    /// `EquippedItemClass` — an `ItemClass.dbc` id ("Weapon", "Armor").
    pub class: u32,
    /// `EquippedItemSubClassMask` — a **mask** of `ItemSubClass.dbc` subclasses,
    /// so a spell that takes any one-handed weapon sets several bits at once.
    pub subclass_mask: u32,
    /// `EquippedItemInventoryTypeMask`, unread here: naming a slot is what the
    /// *reason byte* already does (main hand, off hand, either).
    pub inventory_type_mask: u32,
}

/// `status = 2`, the only failing value `SendCastResult` writes.
const CAST_STATUS_FAILED: u8 = 2;

/// The three reasons whose tail is the equipment trio above.
///
/// vmangos writes it under `SPELL_FAILED_EQUIPPED_ITEM_CLASS` and its two
/// per-hand variants and under nothing else — the other three cases in that
/// `switch` (`NOT_READY`, `REQUIRES_SPELL_FOCUS`, `REQUIRES_AREA`) each write a
/// single `u32` this client has no table to name yet, so they are deliberately
/// left unread rather than parsed into a number nobody can turn into words.
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

/// `SMSG_SPELL_FAILED_OTHER`: a cast in progress was **interrupted**.
///
/// `u64` caster guid — plain, not packed — then `u32 spellId`. vmangos never
/// sends `SMSG_SPELL_FAILURE` at all; `Spell::SendInterrupted` sends this, and
/// it is what stops a wind-up animation that would otherwise be held for the
/// whole of a cast bar that has already been cancelled.
pub fn parse_spell_failed_other(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    Some((guid, r.u32()))
}

/// `SMSG_LEARNED_SPELL`: one more spell — **`{u16 spellId, int16 actionBarSlot}`**,
/// not one `u32`.
///
/// **The spellbook is stated once and amended afterwards.** A trainer visit, a
/// level-up and a quest reward all arrive this way, so a client that reads only
/// `SMSG_INITIAL_SPELLS` has a spellbook that is correct at login and quietly
/// stale for the rest of the session.
///
/// **This was read as a single `u32` for a long time and never misbehaved**,
/// which is the interesting half. `LearnedSpell::AppendBodyTo` writes the id as
/// a `uint16` and then a second `int16` its own comment calls "not used";
/// vmangos leaves that field at its `= 0` initialiser and never assigns it, so
/// the four bytes little-endian *are* the id and the wrong read produced the
/// right answer on every packet this project has ever seen. It is corrected
/// because the next server to put anything in that field would turn every
/// learned spell into an id above 65,536 — a failure that would present as "the
/// trainer did nothing" with no parse error anywhere.
///
/// The trailing field is deliberately not returned: nothing in the 1.12 client
/// reads it either.
pub fn parse_learned_spell(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(2).then(|| u32::from(r.u16()))
}

/// `SMSG_REMOVED_SPELL`: one fewer, and it is **two bytes** where the packet
/// above it is four. `RemovedSpell::AppendBodyTo` writes `uint16(spellId)` and
/// stops.
pub fn parse_removed_spell(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(2).then(|| u32::from(r.u16()))
}

/// `SMSG_SUPERCEDED_SPELL`: **a higher rank replaced a lower one**, `{u16 old,
/// u16 new}`.
///
/// This is the packet that keeps an action bar from rotting, and it had never
/// been read. The server's own comment beside the send is "new spell replace old
/// in action bars and spell book" — the swap is the *client's* to perform, and a
/// client that ignores this leaves the superseded id sitting in `character_action`
/// for ever, because nothing ever restates the bar (see [`parse_action_buttons`]).
///
/// **What that costs is not cosmetic.** `HandleCastSpellOpcode` refuses a spell
/// the character does not have *active* — which a superseded rank is — and
/// returns with **no reply at all**, so pressing the stale button did nothing and
/// wedged the pending record until [`super::super::socket::handler`]'s deadline
/// let it go. Measured against the running server on this project's own warrior
/// (`characters.character_action` against `characters.character_spell`, guid
/// 378): button 73 held Heroic Strike 11566 with only 11567 active, button 75
/// Rend 11572 against 11573.
///
/// Both ids are truncated to 16 bits by the server
/// (`SupercededSpell::AppendBodyTo`), which is the same narrowing
/// `SMSG_INITIAL_SPELLS` applies to every id it lists, so nothing is lost: no
/// 1.12 spell id reaches 65,536.
pub fn parse_superceded_spell(body: &[u8]) -> Option<(u32, u32)> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let old = u32::from(r.u16());
    Some((old, u32::from(r.u16())))
}

/// Something the server said about **what this character just tried to do**.
///
/// A queue of these is drained by whoever is showing the interface, on exactly
/// the terms `ObjectManager::take_chat` is: each of them is an *event* that has
/// to be acted on once, rather than state that can be read again. A cast that
/// failed, a swing that was refused, a cooldown that started — none of them
/// leaves any other trace, which is the same argument the swing counters make
/// from the other side.
/// **Not `Copy`, and not `Eq`.** Every variant was a handful of numbers until
/// `SMSG_RESURRECT_REQUEST` arrived carrying the caster's *name* and
/// `MSG_CORPSE_QUERY` a place with floats in it. Both are real parts of their
/// packets — the name is what the game's own `RESURRECT_REQUEST` popup says, and
/// nothing else on the wire carries it for a creature caster — so the derive is
/// what gives, rather than the shape of the event.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// `CMSG_CAST_SPELL` was accepted. The cast itself arrives as
    /// `SMSG_SPELL_START`/`SMSG_SPELL_GO` like anybody else's.
    CastAccepted { spell_id: u32 },
    /// …or refused, with an index into [`CAST_FAILURE_KEYS`].
    ///
    /// `requirement` is the argument to the message's own `%s` for the three
    /// equipped-item reasons and `None` for every other — see
    /// [`EquipRequirement`].
    CastFailed {
        spell_id: u32,
        reason: u8,
        requirement: Option<EquipRequirement>,
    },
    /// A cast in progress was interrupted — ours if the guid is ours.
    CastInterrupted { guid: u64, spell_id: u32 },
    /// **`SMSG_SPELL_START` naming *us* as the caster: the server has taken the
    /// cast, and this is what puts the bar up.**
    ///
    /// vmangos says so in its own comment — `// will show cast bar` above the
    /// `SendSpellStart()` in `Spell::prepare` — and the 1.12 client agrees from
    /// the other side: `SPELLCAST_START` (event 337) is raised in exactly one
    /// place, and that place is the body of this packet's handler. Nothing on
    /// the press path raises it.
    ///
    /// **Sent for an instant too**, with `cast_time_ms` of zero: `prepare` sends
    /// it for every non-triggered spell whatever its cast time, and the
    /// `SMSG_SPELL_GO` follows immediately. So this is the packet that starts a
    /// cast's *animation* as well, for us exactly as for anybody else.
    CastStarted { spell_id: u32, cast_time_ms: u32 },
    /// **`SMSG_SPELL_GO` naming *us* as the caster: a spell of ours went off.**
    ///
    /// Not the same statement as [`Self::CastAccepted`], which is
    /// `SMSG_CAST_RESULT` and arrives when the server *takes* the request. For
    /// nearly every spell the two are a wind-up apart and nothing needs the
    /// second; for the one family whose whole point is that the two are far
    /// apart — a **next-swing** ability, which is accepted at the press and
    /// released when the weapon lands, possibly seconds later — this is the only
    /// thing on the wire that says the queue emptied.
    ///
    /// The world already reads the same packet for everybody's animations
    /// (`ObjectManager::apply_cast`); this is the caster-filtered edge on the
    /// queue the interface reads, which is a different reader with a different
    /// need — the same split [`Self::ChannelStart`] documents.
    CastReleased { spell_id: u32 },
    /// **`SMSG_SPELL_DELAYED`: our cast was knocked back, by this many
    /// milliseconds.**
    ///
    /// The one packet that says a cast is going to take longer than it said it
    /// would, and the interface has its own name for it —
    /// `CastingBarFrame_OnLoad` registers `SPELLCAST_DELAYED` and its arm slides
    /// both ends of the bar by `arg1 / 1000`. Without it a pushed-back cast runs
    /// its bar out and sits at full while the server is still casting, which is
    /// the "stuck casting" half of the report.
    ///
    /// Caster-only on the wire, so there is no guid here: what the handler does
    /// with the one in the body is check it, and what reaches the interface is
    /// only ever about us. See [`parse_spell_delayed`].
    CastDelayed { delay_ms: u32 },
    /// **`MSG_CHANNEL_START`: a channel of ours has begun, and for how long.**
    ///
    /// Beside the two above rather than folded into `CastAccepted`, because a
    /// channel is the one cast whose *length* arrives in its own packet after
    /// the fact: the wire says `SMSG_SPELL_GO` immediately and nothing about a
    /// bar, so the interface has nothing to show until this lands. The world's
    /// copy of it drives the held pose
    /// ([`crate::state::objects::ObjectManager::apply_channel_start`]); this is the
    /// same news on the queue the interface reads, which is a different reader
    /// with a different need.
    ChannelStart { spell_id: u32, duration_ms: u32 },
    /// `MSG_CHANNEL_UPDATE`: how much of it is left. **Zero is the end**, which
    /// is what an interrupted or completed channel sends.
    ChannelUpdate { remaining_ms: u32 },
    /// `CMSG_ATTACKSWING` was refused; the swing did not start.
    AttackRefused(AttackRefusal),
    /// **`SMSG_CANCEL_AUTO_REPEAT`: the ranged loop has stopped.**
    ///
    /// An empty body — the opcode is the message — and it is the *only* thing
    /// the server ever says about an auto-repeat. There is no "it started"
    /// packet, because starting one is an ordinary `CMSG_CAST_SPELL` this
    /// client sent itself; every other end is here.
    ///
    /// And it has more senders than the player's own cancel:
    /// `SpellCaster::InterruptSpell` routes *every* stop through
    /// `Player::SendAutoRepeatCancel` — the target dying, walking out of range,
    /// a `CheckCast` that stops passing, and (for a wand, category 351) simply
    /// moving. So a client that only cleared its own state on its own press
    /// would keep flashing the button through a fight that had already ended.
    ///
    /// On the queue rather than in [`crate::socket::session::SessionStatus`] because it
    /// is an edge: what it changes is a state this client set locally, and two
    /// arrivals mean two stops.
    AutoRepeatCancelled,
    /// **A sound the server decided to play**, and the only thing on the wire
    /// that says so — `SMSG_PLAY_SOUND`, `SMSG_PLAY_MUSIC` and
    /// `SMSG_PLAY_OBJECT_SOUND`. See [`crate::play::sound`].
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`] because
    /// there is no state to hold: two arrivals are two noises, and a status
    /// field would collapse them into one. The same argument
    /// [`Self::AutoRepeatCancelled`] makes.
    ///
    /// **Not on an entity, unlike the two spell visuals**, even though one of
    /// the three names an object. A sound is played *at this client* — the
    /// server chose the audience by who it sent the packet to — where a visual
    /// is a thing that becomes true about a unit and stays true for as long as
    /// its models are up. The object's guid is a *position*, and it is carried
    /// as one.
    PlaySound(crate::play::sound::Cue),
    /// **An item arrived in a bag** — `SMSG_ITEM_PUSH_RESULT`, and the only
    /// packet that says so whatever brought it. See [`crate::play::items::ItemPush`].
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`] for the
    /// reason every event here is: the inventory *state* is already update
    /// fields and already arrives, and what this adds is the edge — that a
    /// stack growing by three was a loot rather than a purchase.
    ///
    /// **Carries the guid rather than being filtered to us**, because it is
    /// broadcast to the whole group for a loot and the reader has to know whose
    /// bag it went into.
    ItemReceived(crate::play::items::ItemPush),
    /// **A bar the server counts down for us has started or been restated** —
    /// the breath meter and its two siblings. See [`crate::play::timers`].
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`] for the
    /// same reason the cancel above is: what the interface does with it is
    /// `MirrorTimer_Show`, which *claims a frame*, and two arrivals are two
    /// statements about the bar even when every field matches. Reading it as
    /// state would also lose the one thing the packet is for — the server
    /// resends a start to say "paused", because its own pause event is
    /// unusable.
    MirrorTimerStarted(crate::play::timers::MirrorTimerStart),
    /// …and the same bar hidden. `SMSG_STOP_MIRROR_TIMER`, one `u32`.
    MirrorTimerStopped { timer: crate::play::timers::MirrorTimer },
    /// …and frozen where it stands. **The shipped handler for this cannot
    /// work** — see [`crate::play::timers`] — and it is read anyway, because what is
    /// faithful is the packet.
    MirrorTimerPaused {
        timer: crate::play::timers::MirrorTimer,
        paused: bool,
    },
    /// The server started or refreshed a cooldown: milliseconds from now.
    /// **Not the ordinary path** — see [`parse_spell_cooldowns`].
    CooldownStarted { spell_id: u32, ms: u32 },
    /// A parked cooldown was released (`SPELL_ATTR_COOLDOWN_ON_EVENT`).
    CooldownReleased { spell_id: u32 },
    /// `SMSG_CLEAR_COOLDOWN` — **this one is over**, whatever it had left. See
    /// [`parse_clear_cooldown`], and note that this is a *removal* where
    /// [`Self::CooldownReleased`] is a start: the two names are one letter
    /// apart and mean opposite things.
    CooldownCleared { spell_id: u32 },
    /// **The pet refused an order** — `SMSG_PET_ACTION_FEEDBACK`, one byte,
    /// and the whole packet is which sentence to say. See
    /// [`crate::play::pet::feedback`].
    ///
    /// On this queue rather than in the session status for the reason every
    /// edge here is: two refusals of the same order are two messages, and a
    /// field holding the last one would say it once.
    PetFeedback(u8),
    /// …and refused a *cast*, which is the pet's own `SMSG_CAST_RESULT`:
    /// `reason` indexes the same 146-entry table.
    PetCastFailed { spell_id: u32, reason: u8 },
    /// `SMSG_PET_TAME_FAILURE` — a `PetTameFailureReason`.
    PetTameFailure(u8),
    /// `SMSG_PET_BROKEN` — an empty body. "Your pet has run away", which is a
    /// hunter pet whose loyalty ran out.
    PetBroken,
    /// `SMSG_PET_NAME_INVALID` — also an empty body, and also one sentence.
    PetNameInvalid,
    /// `SMSG_PET_UNLEARN_CONFIRM` — the guid, and what resetting it costs.
    PetUnlearnConfirm { pet: u64, cost: u32 },
    /// `SMSG_PET_ACTION_SOUND` — the pet said something. `talk` is a
    /// [`crate::play::pet::pet_talk`] selector, not a sound entry.
    PetTalk { pet: u64, talk: u32 },
    /// `SMSG_PET_DISMISS_SOUND` — a model-data id and a place; the pet itself
    /// is already gone. See [`crate::play::pet::PetDismissSound`].
    PetDismissSound(crate::play::pet::PetDismissSound),
    /// The spellbook changed after login.
    SpellLearned(u32),
    SpellRemoved(u32),
    /// **A rank was replaced by a higher one** — `SMSG_SUPERCEDED_SPELL`, and
    /// the one spellbook delta that is also a *bar* delta.
    ///
    /// `slots` is which action-bar slots held the old id and now hold the new
    /// one, already swapped in [`crate::state::objects::ObjectManager`]'s own copy.
    /// It is carried rather than re-derived because the bar is allowed to hold
    /// the same spell twice and may already have held `new` elsewhere, so "which
    /// slots have `new` in them now" is not the same question as "which slots
    /// did this packet change".
    ///
    /// The reader owes the server a `CMSG_SET_ACTION_BUTTON` per slot: the bar
    /// is client state the server only stores, so a swap that is not sent back
    /// is one that comes undone at the next login. See
    /// [`set_action_button_body`].
    SpellSuperceded { old: u32, new: u32, slots: Vec<u8> },
    /// **We gained a level** — `SMSG_LEVELUP_INFO`, and it is the only thing
    /// that says so. `UNIT_FIELD_LEVEL` moving is not the same statement: it
    /// moves at every login and for every creature that streams into view.
    LevelUp(LevelUp),
    /// **The server declined to move us** — `SMSG_TRANSFER_ABORTED`, and it is
    /// the only thing that ever says an instance portal did nothing on purpose.
    ///
    /// Carries the raw byte rather than a decoded reason, because three of the
    /// codes are deliberately silent and only the reader knows whether it wants
    /// to say so; [`crate::play::areatrigger::TransferAbort::from_code`] is the
    /// decode, and `None` from it is "show nothing", which is the client's own
    /// behaviour rather than an unhandled case.
    ///
    /// On this queue for the same reason the logout states below are: it is an
    /// edge — two refusals of two portals are two statements, and every field
    /// of them matches.
    TransferAborted { reason: u8 },
    /// **The server is about to move us to another map** —
    /// `SMSG_TRANSFER_PENDING`, whose whole purpose is to raise the loading
    /// screen a moment before the old world is torn down.
    ///
    /// The twin of [`Self::TransferAborted`] on the way *in*, and an edge for
    /// the same reason: two teleports to the same map are two statements, and
    /// a client that noticed the change by watching `map_id` instead would
    /// notice it only after `SMSG_NEW_WORLD` — which is the far side of the
    /// gap the screen exists to cover. See
    /// [`crate::state::movement::parse_transfer_pending`].
    TransferPending { map_id: u32 },
    /// **What the server said about leaving** — see [`crate::play::logout`], which
    /// owns the four packets and the reason the client does not hold the clock.
    ///
    /// On this queue rather than in [`crate::socket::session::SessionStatus`] because
    /// each of the four is an *edge*: `PLAYER_CAMPING` is raised once when the
    /// request is taken and `LOGOUT_CANCEL` once when it is given back, and a
    /// state a reader polls cannot tell one arrival from two.
    Logout(crate::play::logout::Logout),
    /// **How long before the body may be taken back** —
    /// `SMSG_CORPSE_RECLAIM_DELAY`, in milliseconds, said once at the moment of
    /// release and never restated. See [`crate::play::death`].
    CorpseReclaimDelay { ms: u32 },
    /// Where the body is, or `None` for "there isn't one" — the reply to our own
    /// `MSG_CORPSE_QUERY`. An answer to a question, so it belongs on the queue
    /// rather than in the status: a second query with the same answer is still a
    /// second answer.
    CorpseLocated(Option<crate::play::death::CorpseLocation>),
    /// Somebody has offered to resurrect us — `SMSG_RESURRECT_REQUEST`.
    ResurrectOffered(crate::play::death::ResurrectOffer),
    /// …and the spirit healer's version of the same offer, which costs 25%
    /// durability and brings sickness: `SMSG_SPIRIT_HEALER_CONFIRM`, whose
    /// whole body is the healer's guid.
    SpiritHealerOffered { healer: u64 },
    /// **An item verb was refused** — `SMSG_INVENTORY_CHANGE_FAILURE`, the
    /// answer every right-click, equip and swap shares. See
    /// [`crate::play::items::InventoryFailure`], which carries the reason and the one
    /// number any of them takes.
    ///
    /// On this queue rather than in the world, because it is an *edge* about an
    /// action the player just took and there is no state it changes: the item
    /// stayed exactly where it was, which is the point.
    InventoryFailed(crate::play::items::InventoryFailure),
    /// **A loot window opened** — `SMSG_LOOT_RESPONSE`, which is also how the
    /// server *refuses* to open one. See [`crate::play::loot`], where the byte that
    /// tells the two apart is.
    ///
    /// On this queue rather than in the world for the reason the whole family
    /// is an edge: the interface's answer is `ShowUIPanel(LootFrame)`, and a
    /// second arrival about the same corpse is a second window rather than a
    /// value that happens to match.
    LootOpened(crate::play::loot::Loot),
    /// One row is gone — `SMSG_LOOT_REMOVED`, by the **server's** index. It
    /// arrives for a row somebody else took as well as for one we took.
    LootRemoved { index: u8 },
    /// The coins are gone from the window — `SMSG_LOOT_CLEAR_MONEY`, no body.
    /// **Not the same statement as [`Self::LootMoneyGained`]**, and conflating
    /// them leaves a coin row on every other group member's screen.
    LootMoneyCleared,
    /// …and *your share*, in copper — `SMSG_LOOT_MONEY_NOTIFY`.
    LootMoneyGained { copper: u32 },
    /// **The server agrees the window is shut** — `SMSG_LOOT_RELEASE_RESPONSE`.
    /// The client's own `CMSG_LOOT_RELEASE` is not what closes it; see
    /// [`crate::play::loot`].
    LootClosed { guid: u64 },
    /// **A group roll has opened on one row** — `SMSG_LOOT_START_ROLL`, and the
    /// four below are the rest of that family. See [`crate::play::lootroll`],
    /// where the whole subject is, including why the id the interface uses is
    /// not on the wire at all.
    ///
    /// On this queue rather than in the world for the same reason the loot
    /// window is: a roll is an edge with a clock on it, and a second start
    /// naming the same `(guid, slot)` is a second roll on a respawned body
    /// rather than a value that happens to match.
    LootRollStarted(crate::play::lootroll::RollStart),
    /// `SMSG_LOOT_ROLL` — somebody chose, or somebody's dice landed. Sent to
    /// everyone eligible, so it arrives for our own vote too.
    LootRollCast(crate::play::lootroll::RollCast),
    /// `SMSG_LOOT_ROLL_WON` — and the item is already in the winner's bags.
    LootRollWon(crate::play::lootroll::RollWon),
    /// `SMSG_LOOT_ALL_PASSED` — **the one ending that leaves the item on the
    /// body**, and the only statement that its row is clickable again.
    LootRollAllPassed(crate::play::lootroll::RollAllPassed),

    // --- quests ---
    //
    // See [`crate::play::quest`], which owns the wire. Every one of these is an edge
    // about a conversation the player is having: a second arrival with
    // identical fields is a second page of dialogue, not a value that happens
    // to match, so none of them can be read as state.

    /// **What is over a giver's head** — `SMSG_QUESTGIVER_STATUS`, one guid and
    /// one of eight answers.
    ///
    /// The one member of this family that *is* also state — the mark stays over
    /// the head until it changes — but it arrives as an edge and the world is
    /// where it is kept; see [`crate::state::objects::ObjectManager::quest_status`].
    QuestStatus {
        guid: u64,
        status: crate::play::quest::DialogStatus,
    },
    /// `SMSG_QUESTGIVER_QUEST_LIST` — the greeting, and what is on offer.
    ///
    /// **The four page variants are boxed and the rest are not.** A
    /// `QuestTemplate` is 384 bytes of strings and arrays where an
    /// `AttackRefused` is one; unboxed, every value on this queue — and there is
    /// one per packet the session reads — would be as wide as the widest member,
    /// which is also the one that arrives least often.
    QuestGreeting(Box<crate::play::quest::QuestGreeting>),
    /// `SMSG_QUESTGIVER_QUEST_DETAILS` — the page before you accept.
    QuestDetails(Box<crate::play::quest::QuestDetails>),
    /// `SMSG_QUESTGIVER_OFFER_REWARD` — …and the one when it is done.
    QuestReward(Box<crate::play::quest::QuestReward>),
    /// `SMSG_QUESTGIVER_REQUEST_ITEMS` — …or the one when it is not. **Not sent
    /// for every unfinished quest**; see [`crate::play::quest::QuestProgress`].
    QuestProgress(Box<crate::play::quest::QuestProgress>),
    /// `SMSG_QUESTGIVER_QUEST_COMPLETE` — what handing it in paid.
    QuestComplete(crate::play::quest::QuestComplete),
    /// `SMSG_QUEST_QUERY_RESPONSE` — what a quest *is*. Cached per id, exactly
    /// as an item template is.
    QuestTemplate(Box<crate::play::quest::QuestTemplate>),
    /// `SMSG_QUESTUPDATE_ADD_KILL` — one more of something.
    QuestKill(crate::play::quest::QuestKill),
    /// **`SMSG_QUESTUPDATE_ADD_ITEM` — an item objective moved**, as
    /// `(entry, added)` plus the bag count at the moment it arrived. See
    /// [`crate::play::quest::parse_quest_item`]: the second word is an
    /// increment and the count on screen is the client's own arithmetic over
    /// its bags.
    ///
    /// **`have` is carried rather than looked up by the reader**, because it is
    /// only correct on the session thread — see
    /// [`crate::socket::handler::player::quest_item`], which is where the
    /// argument is.
    QuestItem { entry: u32, added: u32, have: u32 },
    /// `SMSG_QUESTUPDATE_COMPLETE` — every objective is done. **The log's own
    /// state bit says so too**; this is the edge that plays the sound and
    /// writes the chat line.
    QuestObjectivesDone { quest_id: u32 },
    /// `SMSG_QUESTUPDATE_FAILED` and its timer twin.
    QuestFailed { quest_id: u32, timed_out: bool },
    /// `SMSG_QUESTGIVER_QUEST_INVALID` — **a reason, not a quest id**, and the
    /// difference matters: `SendCanTakeQuestResponse` writes the refusal code
    /// where every other packet in this family writes an id.
    QuestRefused { reason: u32 },

    // --- talking to an NPC ---
    //
    // See [`crate::play::gossip`]. The same edge argument as the quest family's: a
    // second identical menu is a second page of conversation.

    /// `SMSG_GOSSIP_MESSAGE` — the menu, whose *text* is a round trip behind.
    GossipShow(Box<crate::play::gossip::GossipMenu>),
    /// `SMSG_GAMEOBJECT_PAGETEXT` — **this thing has pages**, and the packet
    /// says nothing else: the page id is in the template the client already
    /// holds. See [`crate::play::pagetext`].
    GameObjectPageText { guid: u64 },
    /// `SMSG_PAGE_TEXT_QUERY_RESPONSE` — one page, and the id of the next.
    /// The server answers the whole chain in a burst, so these arrive several
    /// at a time and each names its successor.
    PageText(crate::play::pagetext::Page),
    /// `SMSG_GOSSIP_POI` — **a place named on the world map**, and the client
    /// keeps exactly one. See [`crate::play::gossip::parse_gossip_poi`].
    GossipPoi {
        flags: u32,
        position: (f32, f32),
        icon: u32,
        data: u32,
        name: String,
    },
    /// `SMSG_GOSSIP_COMPLETE` — the server closed the window. No body.
    GossipClosed,
    /// `SMSG_NPC_TEXT_UPDATE` — the words a menu's text id stood for.
    NpcText { text_id: u32, text: String },

    /// **`SMSG_BINDER_CONFIRM` — an innkeeper is asking to be made home.**
    ///
    /// The guid is the innkeeper's and it has to be carried through: the answer
    /// is `CMSG_BINDER_ACTIVATE` naming that same guid, which
    /// `HandleBinderActivateOpcode` looks up with `GetNPCIfCanInteractWith(…,
    /// UNIT_NPC_FLAG_INNKEEPER)` and drops without a word if it does not match
    /// an innkeeper in range.
    ///
    /// **Nothing is bound when this arrives.** vmangos closes the gossip window
    /// *before* sending it, so a client that drops this gets the whole visible
    /// effect of the click — the window shuts — and no bind at all. See
    /// [`crate::play::bindpoint`].
    BinderConfirm { guid: u64 },
    /// `SMSG_PLAYERBOUND` — it went through, and this is the area it was set
    /// to. `SMSG_BINDPOINTUPDATE` carries the same area id and the position
    /// beside it; this is the one that says *who* bound us.
    PlayerBound { guid: u64, area_id: u32 },
    /// `SMSG_DUEL_REQUESTED` — a duel was asked for, by us or at us. Which of
    /// the two is the initiator guid against our own; see [`crate::play::duel`].
    DuelRequested { arbiter: u64, initiator: u64 },
    /// `SMSG_DUEL_COUNTDOWN` — it was accepted, and starts in this many
    /// milliseconds.
    DuelCountdown { ms: u32 },
    /// `SMSG_DUEL_OUTOFBOUNDS` / `SMSG_DUEL_INBOUNDS` — left the flag's area,
    /// and came back. `true` is out.
    DuelBounds { out: bool },
    /// `SMSG_DUEL_COMPLETE` — over. `started` is false for a duel declined or
    /// abandoned before the countdown ended.
    DuelComplete { started: bool },
    /// `SMSG_DUEL_WINNER` — who won, broadcast to everyone nearby.
    DuelWinner(crate::play::duel::DuelWinner),
    /// `SMSG_SUMMON_REQUEST` — somebody wants to bring us to them. See
    /// [`crate::play::summon`].
    SummonRequest(crate::play::summon::SummonRequest),
    /// `SMSG_PLAYED_TIME` — `/played` answered, in seconds.
    PlayedTime { total: u32, level: u32 },
    /// `SMSG_FISH_NOT_HOOKED` / `SMSG_FISH_ESCAPED` — the bobber was clicked
    /// with nothing on it, or too late. `true` is escaped. Both are bodiless and
    /// each is one line of the message table.
    Fish { escaped: bool },
    /// `SMSG_LIST_INVENTORY` — the vendor window.
    VendorShow(Box<crate::play::gossip::VendorList>),
    /// `SMSG_BUY_ITEM` — a purchase went through; the slot's stock moved.
    VendorSold {
        guid: u64,
        slot: u32,
        left: Option<u32>,
    },
    /// `SMSG_BUY_FAILED` — …or did not, and why.
    BuyFailed {
        entry: u32,
        reason: Option<crate::play::gossip::BuyFailure>,
    },
    /// `SMSG_SELL_ITEM` — a sale refused. **The success has no packet**: money
    /// and the item's removal arrive as ordinary update fields.
    SellFailed {
        item: u64,
        reason: Option<crate::play::gossip::SellFailure>,
    },
    /// `SMSG_TRAINER_LIST` — what an NPC will teach. See [`crate::play::trainer`].
    TrainerShow(Box<crate::play::trainer::TrainerList>),
    /// `SMSG_TRAINER_BUY_SUCCEEDED` — a service was learned. **The spell itself
    /// arrives separately**, as an ordinary `SMSG_LEARNED_SPELL`, so this only
    /// says which row to re-colour.
    TrainerBought { spell: u32 },
    /// `SMSG_TRAINER_BUY_FAILED` — …or was not, and why.
    TrainerBuyFailed {
        spell: u32,
        reason: Option<crate::play::trainer::TrainFailure>,
    },

    // --- the stable ---
    //
    // See [`crate::play::stable`]. Two packets, and the second is the answer to
    // all four of the verbs.

    /// `MSG_LIST_STABLED_PETS` — the whole stable window. Boxed for the reason
    /// [`Self::TrainerShow`] is: three named pets is the largest variant in
    /// this enum by some way, and every other one pays for it.
    StableList(Box<crate::play::stable::StableList>),
    /// `SMSG_STABLE_RESULT` — one byte, kept raw beside its reading so an
    /// unrecognised code is data rather than a dropped packet.
    StableResult {
        byte: u8,
        result: Option<crate::play::stable::StableResult>,
    },

    // --- the bank ---
    //
    // See [`crate::play::bank`]. A guid and a refusal; the contents are
    // fields.

    /// `SMSG_SHOW_BANK` — the window opens, at this banker.
    BankShow(u64),
    /// `SMSG_BUY_BANK_SLOT_RESULT` — a slot purchase refused. Kept raw beside
    /// its reading, like the stable's byte.
    BankSlotResult {
        code: u32,
        result: Option<crate::play::bank::BankSlotResult>,
    },

    // --- the party ---
    //
    // See [`crate::play::group`]. Five of the six are edges rather than state: an
    // invitation is a popup, a decline is a chat line, a refusal is a message.
    // Only [`Self::GroupList`] is the roster, and it arrives *whole* every time
    // anything about the group moves.

    /// `SMSG_GROUP_INVITE` — somebody is asking us to join theirs. The body is
    /// their name and nothing else, and it is what the popup says.
    GroupInvite { name: String },
    /// `SMSG_GROUP_DECLINE` — somebody we invited said no. Only the inviter is
    /// told.
    GroupDecline { name: String },
    /// `SMSG_GROUP_LIST` — **the whole roster**, everybody but us. Boxed for the
    /// reason `TrainerShow` is: a party of four with names is the largest
    /// variant here by an order of magnitude.
    GroupList(Box<crate::play::group::GroupList>),
    /// `SMSG_GROUP_DESTROYED` — the party is over. **No body, and it is not the
    /// same packet as an empty roster**: the server never sends one of those.
    GroupDestroyed,
    /// `SMSG_GROUP_SET_LEADER` — who leads now, **by name**, where the request
    /// that caused it was by guid.
    GroupNewLeader { name: String },
    /// `SMSG_PARTY_COMMAND_RESULT` — what came of an invite or a leave, as an
    /// index into `GlobalStrings.lua` and the name it applies to.
    PartyResult(crate::play::group::PartyCommandResult),
    /// `SMSG_PARTY_MEMBER_STATS` / `_FULL` — a member's health, mana, level and
    /// zone, for the ones too far away to be in the object manager at all.
    PartyMemberStats(crate::play::group::PartyMemberStats),
    /// `MSG_RAID_READY_CHECK` — **one opcode carrying two different pieces of
    /// news**, told apart by whether it has a body. See
    /// [`crate::play::group::ReadyCheck`].
    RaidReadyCheck(crate::play::group::ReadyCheck),

    // --- reputation ---
    //
    // See [`crate::play::reputation`]. All four are statements about the 64
    // reputation-list slots and none of them is drawable on its own: the
    // standings are deltas from a `Faction.dbc` base, and which rows exist at
    // all is a rule the client keeps to itself.

    /// `SMSG_INITIALIZE_FACTIONS` — **the whole standing table**, 64 slots,
    /// replacing whatever was held. Boxed for the same reason `GroupList` is:
    /// 64 pairs is 512 bytes and every other variant here is a handful.
    FactionsInitialized(Box<crate::play::reputation::FactionStates>),
    /// `SMSG_SET_FACTION_STANDING` — one or more slots' deltas moved. A list
    /// rather than a pair, because reputation spills onto parent factions and
    /// arrives as one packet naming both.
    FactionStandings(Vec<(u32, i32)>),
    /// `SMSG_SET_FACTION_VISIBLE` — a faction met for the first time, which is
    /// a *new row* on the panel rather than a change to one.
    FactionVisible { reputation_list_id: u32 },
    /// `SMSG_SET_FACTION_ATWAR` — the server's own statement about the box, and
    /// the whole flag byte rather than a boolean.
    FactionAtWar { reputation_list_id: u32, flags: u8 },
    /// `SMSG_SET_FORCED_REACTIONS` — the **whole** forced-reaction map, as
    /// `(factionId, rank)`, replacing whatever was held. See
    /// [`crate::play::reputation::parse_forced_reactions`], which says why a
    /// merge would be wrong.
    ForcedReactions(Vec<(u32, u32)>),

    /// **`SMSG_EXPLORATION_EXPERIENCE` — a new place, and what it paid.**
    ///
    /// `{u32 areaId, u32 xp}`, and it is the *only* announcement a discovery
    /// makes: `PLAYER_EXPLORED_ZONES` moving is a `PRIVATE` field with no event
    /// of its own, so a client that does not read this has a world map that
    /// fills in silently and a "Discovered:" line that never appears.
    ///
    /// **Sent even when the experience is zero** — vmangos says so in its own
    /// comment a line above the call (`Player.cpp`, "Exploration packet should
    /// be sent even if no XP is gained"), which is what makes it a reliable edge
    /// rather than a level-dependent one.
    Discovered { area: u32, experience: u32 },

    // --- the flight master ---
    //
    // See [`crate::play::taxi`]. Three edges and no state: the map arrives whole and
    // is replaced whole, and the flight it buys is an `SMSG_MONSTER_MOVE` that
    // reaches the mover through an entirely different arm.
    /// `SMSG_SHOWTAXINODES` — the flight map: a master, the node under it, and
    /// the 256-bit mask of everywhere this character has been.
    TaxiShow(crate::play::taxi::TaxiMenu),
    /// `SMSG_TAXINODE_STATUS` — whether this master's own node is known. Also
    /// the second half of a discovery, in which case the byte is already 1.
    TaxiNodeStatus { guid: u64, known: bool },
    /// **`SMSG_NEW_TAXI_PATH` — a flight point discovered, and it has no body.**
    /// The whole packet is the fact that it arrived; what it is *for* is the
    /// `TaxiNodeDiscovered` sound and, in the reference, a refresh of whatever
    /// map is open.
    NewTaxiPath,
    /// `SMSG_ACTIVATETAXIREPLY` — yes, or one of twelve reasons why not.
    TaxiReply(crate::play::taxi::TaxiReply),

    // --- who you know ---
    //
    // See [`crate::play::social`]. The two lists are guids with no names in
    // them, so none of the three is drawable until `CMSG_NAME_QUERY` has
    // answered — which is why they are events carrying the wire's own contents
    // rather than anything resolved.
    /// `SMSG_FRIEND_LIST` — **the whole friends list**, replacing whatever was
    /// held. It arrives once, at login: every later change is a
    /// [`Self::FriendStatus`] and nothing re-sends this.
    FriendList(Vec<crate::play::social::Friend>),
    /// `SMSG_IGNORE_LIST` — the same statement about the ignore list, and the
    /// same once-only arrival.
    IgnoreList(Vec<u64>),
    /// `SMSG_FRIEND_STATUS` — one answer about one player: an add, a removal, a
    /// refusal, or a friend logging in or out.
    FriendStatus(crate::play::social::FriendStatus),
    /// `SMSG_WHO` — the search's answer, and the online total behind it.
    WhoResults(crate::play::social::WhoResults),

    // --- what is in the box on the corner ---
    //
    // See [`crate::play::mail`]. Like the two social lists, none of this is
    // drawable on arrival: a letter's sender is a guid `CMSG_NAME_QUERY` has
    // to answer for, and its words are a second round trip of their own.
    /// `SMSG_MAIL_LIST_RESULT` — **the whole inbox**, replacing whatever was
    /// held. Boxed for the reason [`Self::TrainerShow`] is: it is by some way
    /// the widest variant in this enum and it arrives once a minute at most.
    MailList(Box<Vec<crate::play::mail::MailHeader>>),

    // --- the trade window ---
    //
    // See [`crate::play::trade`]. Two packets: the state machine, and the
    // offer — either side's, said by the server.

    /// `SMSG_TRADE_STATUS` — a request, an open, an accept, a refusal or a
    /// close; see [`crate::play::trade::TradeStatus`].
    TradeStatus(crate::play::trade::TradeStatusPacket),
    /// `SMSG_TRADE_STATUS_EXTENDED` — one side's seven slots and money,
    /// whole. Boxed for the reason [`Self::MailList`] is.
    TradeOffer(Box<crate::play::trade::TradeOffer>),
    /// `SMSG_SEND_MAIL_RESULT` — the one answer all seven verbs share.
    MailResult(crate::play::mail::MailResponse),
    /// `SMSG_RECEIVED_MAIL` — something has arrived. The packet's own arrival
    /// is the whole of its content.
    MailReceived,
    /// `MSG_QUERY_NEXT_MAIL_TIME` — seconds until the next letter, where **0
    /// means one is already waiting**; see
    /// [`crate::play::mail::parse_next_mail_time`].
    MailNextTime(f32),
    /// `SMSG_ITEM_TEXT_QUERY_RESPONSE` — the words of one letter, by the text
    /// id that was asked for.
    ItemText { id: u32, text: String },
    /// `SMSG_CHANNEL_NOTIFY` — see [`crate::play::channels`].
    ChannelNotify(Box<crate::play::channels::ChannelNotify>),
    /// `SMSG_CHANNEL_LIST` — who is on a channel.
    ChannelList(Box<crate::play::channels::ChannelList>),
    /// `SMSG_TEXT_EMOTE` — somebody's `/dance`; see [`crate::play::emotetext`].
    TextEmote(Box<crate::play::emotetext::TextEmote>),

    // ---- what the character may hold, and what a talent does to a spell ----
    /// `SMSG_SET_PROFICIENCY` — one item class's whole mask; see
    /// [`crate::play::skills::Proficiency`]. Replaces that class, never merges.
    Proficiency(crate::play::skills::Proficiency),
    /// `SMSG_SET_FLAT_SPELL_MODIFIER` / `_PCT_` — one bit of one operation's
    /// running total; see [`SpellModifier`], whose note is that these are
    /// totals and not deltas.
    SpellModifier(SpellModifier),
}

/// Why a `CMSG_ATTACKSWING` was refused.
///
/// **The melee counterpart of a cast result, and it is five opcodes rather than
/// one field.** There is no body to read on any of them — the opcode *is* the
/// message — so this is the whole of the packet.
///
/// The strings are the game's own (`GlobalStrings.lua`), and the two that
/// matter are the two the player sees constantly: walk up to a mob and swing
/// and you get `BADATTACKPOS` until you are inside melee range, and swing with
/// your back turned and you get `BADATTACKFACING`.
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

    /// The `GlobalStrings.lua` key for what to show the player.
    ///
    /// `ERR_BADATTACKPOS` is "You are too far away!" and `ERR_BADATTACKFACING`
    /// is "You are facing the wrong way!" — the two lines every player of this
    /// game has read a thousand times, and neither of them is a string this
    /// client made up.
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

/// `SMSG_ATTACKSTART`: a unit has begun auto-attacking another.
///
/// Two **plain** guids, attacker then victim (`Unit::SendMeleeAttackStart`
/// streams `GetObjectGuid()` twice with no packing). Broadcast to everyone in
/// sight, so this is how the client knows a creature two hundred yards of
/// packets away has engaged — and, for the local player, it is the server's
/// confirmation that the swing we asked for took.
pub fn parse_attack_start(body: &[u8]) -> Option<(u64, u64)> {
    let mut r = Reader::new(body);
    if !r.has(16) {
        return None;
    }
    let attacker = r.u64();
    Some((attacker, r.u64()))
}

/// `MSG_CHANNEL_START`: a channelled spell has begun, and how long it runs.
///
/// `{u32 spellId, u32 durationMs}` — `Spell::SendChannelStart` in vmangos, sent
/// with `SendDirectMessage` and therefore **only to the caster**. There is no
/// broadcast form: what another player's channel looks like is
/// `UNIT_CHANNEL_SPELL` in their update block, which this client does not read.
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
/// `{u64 caster, u32 delayMs}` — `Spell::Delayed` streams an `ObjectGuid`
/// **plain** (there is no `GetPackGUID()` on this one) and then the delay. Sent
/// with `SendDirectMessage` and only when the caster is a player, so it is ours
/// by construction; the guid is read and checked anyway, exactly as
/// [`parse_spell_cooldowns`]' is, because "ours by construction" is a property
/// of today's server rather than of the packet.
///
/// **The delay is a difference, not a new length.** vmangos adds it to `m_timer`
/// and clamps at the spell's own cast time, so a cast can be pushed back several
/// times and each packet says only how much this one moved it.
pub fn parse_spell_delayed(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(12) {
        return None;
    }
    let guid = r.u64();
    Some((guid, r.u32()))
}

/// `MSG_CHANNEL_UPDATE`: how much of it is left.
///
/// `{u32 remainingMs}`, and **zero means it is over** — an interrupted or
/// cancelled channel sends one of these rather than anything of its own.
pub fn parse_channel_update(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

/// `SMSG_ATTACKSTOP`: …and has stopped.
///
/// **Packed guids here, where `SMSG_ATTACKSTART` has plain ones.** Both are one
/// function apart in `Unit.cpp` and they disagree, which is exactly the sort of
/// thing that is cheap to check and expensive to assume. The victim may be a
/// packed *zero* (`SendAttackStop(nullptr)` writes an empty `PackedGuid`), which
/// is the server saying "stop attacking, no particular target" — the answer to
/// a swing at something friendly or already dead.
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
/// The full table is sixteen bits wide and most of it names things this client
/// cannot yet aim at — an item, a gameobject, a corpse, a string. The three here
/// are the three this client can send.
pub mod target_flag {
    /// **Zero, and it means "no target block at all".** `SpellCastTargets::read`
    /// tests the whole mask for equality with this before it reads anything, so
    /// a self-cast is two bytes on the wire and the server fills the rest in
    /// from the spell's own implicit targeting.
    pub const SELF: u16 = 0x0000;
    /// A packed unit guid follows.
    pub const UNIT: u16 = 0x0002;
    /// **Three plain `f32`s follow and nothing else** — no guid, in any build.
    /// This is the placed half of a cast: Blizzard, Flamestrike, Rain of Fire.
    /// `SpellCastTargets::read` validates them with `IsValidMapCoord` and throws
    /// the whole packet out if they fail, so a `NaN` here is a disconnection
    /// rather than a refusal.
    pub const DEST_LOCATION: u16 = 0x0040;
    /// **A packed *game object* guid follows** — a chest, an ore vein, a herb.
    ///
    /// `0x0800`, from vmangos' `SpellCastTargetFlags`, where it is
    /// `TARGET_FLAG_GAMEOBJECT` and sits between `TARGET_FLAG_UNIT_DEAD`
    /// (`0x400`) and `TARGET_FLAG_TRADE_ITEM` (`0x1000`). Worth naming its
    /// neighbours: the block is a bitmask of sixteen flags of which this client
    /// sends four, and the two either side of this one are a *corpse* and an
    /// *item* — both of which also carry a guid, so a wrong flag here is a
    /// well-formed packet the server reads as being about something else.
    ///
    /// It exists because **a chest is not opened by `CMSG_GAMEOBJ_USE`**: see
    /// `vale_assets::look::object`, where the server arm that does nothing
    /// is quoted. Gathering a herb is a spell cast at the plant.
    pub const GAMEOBJECT: u16 = 0x0800;
    /// **A packed *item* guid follows** — the weapon an imbue goes on.
    ///
    /// `0x0010`, from vmangos' `SpellCastTargetFlags`, read as a packed guid in
    /// `SpellCastTargets::read` like every other guid in the block. It is the
    /// bit `Spell::ValidateExplicitTargetMask` insists on for any spell whose
    /// `Spell.dbc` `Targets` column carries it, which is why Rockbiter Weapon
    /// sent as anything else is refused — and, before this existed, why it was
    /// refused *locally* as "Invalid target".
    ///
    /// The item is picked by the client rather than pointed at: see
    /// `vale_assets::tables::spellbook::CastAim::Item`.
    pub const ITEM: u16 = 0x0010;
    /// **A square in the trade window follows**, and what follows is a *slot
    /// number* packed like a guid rather than a guid.
    ///
    /// `0x1000`, the neighbour above [`GAMEOBJECT`]. See
    /// [`super::CastTarget::TradeSlot`], where the server's own reading of it is
    /// quoted: only `TRADE_SLOT_NONTRADED` is accepted, and while the trade is
    /// still open the cast is *stored* rather than performed.
    pub const TRADE_ITEM: u16 = 0x1000;
}

/// What a cast is aimed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CastTarget {
    /// The spell says who it hits; send no target. Ice Armor, Battle Shout.
    SelfImplicit,
    /// This unit, whoever it is — including ourselves.
    Unit(u64),
    /// **A place, in the server's own axes and yards** — `x, y, z`, which for a
    /// ground-targeted spell is the point the player clicked on the floor. See
    /// [`target_flag::DEST_LOCATION`].
    ///
    /// The client is the only thing that can produce it: nothing in the protocol
    /// asks where the pointer is, and vmangos' `Spell::SetTargetMap` falls back
    /// to *the caster's own position* when the flag is absent — so a placed
    /// spell sent without one lands underfoot rather than being refused.
    Dest([f32; 3]),
    /// **A square in the trade window**, and it is the one target that is not an
    /// object at all: the "guid" written is the **trade slot number**.
    ///
    /// `Spell::CheckCast`'s last clause reads it as
    /// `TradeSlots(m_targets.getItemTargetGuid().GetRawValue())` and refuses
    /// anything but `TRADE_SLOT_NONTRADED` with `SPELL_FAILED_ITEM_NOT_READY`;
    /// with the trade still open it then **stores** the spell against the trade
    /// and answers `SPELL_FAILED_DONT_REPORT`, which is why an enchant aimed at
    /// the "will not be traded" square produces no visible answer and lands only
    /// when both sides accept. The echo comes back as
    /// `SMSG_TRADE_STATUS_EXTENDED`'s `spell` field, which
    /// [`crate::play::trade::TradeOffer::spell`] already carries.
    TradeSlot(u8),
    /// **A game object** — the ore vein, the herb, the chest a lock is on.
    ///
    /// The only target kind this client sends that is not a unit or a place, and
    /// the only one that is ever *implied* rather than chosen: the player clicks
    /// the plant, and the spell is picked from the lock rather than from a bar.
    /// See `vale_assets::look::object::opener`.
    Object(u64),
    /// **An item in the bags or on the body** — the weapon a Rockbiter goes on,
    /// the blade a sharpening stone is dragged down.
    ///
    /// Chosen by the client rather than by the player for every spell that
    /// carries `SPELL_ATTR_HELD_ITEM_ONLY`, which vmangos comments *"Client
    /// automatically selects item from mainhand slot as a cast target"*. See
    /// `vale_assets::tables::spellbook::CastAim::Item`, which is where that
    /// choice is made.
    Item(u64),
}

/// `CMSG_CAST_SPELL`: `u32 spellId`, then the target block.
///
/// See [`CastTarget`] and the module comment: shipping the current selection
/// with a spell that wants no target is how a buff becomes "Invalid target".
pub fn cast_spell_body(spell_id: u32, target: CastTarget) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(spell_id);
    write_cast_target(&mut w, target);
    w.buf
}

/// The target block itself, which `CMSG_USE_ITEM` carries too.
///
/// One writer rather than two, because the two opcodes read through the *same*
/// `SpellCastTargets::read` on the server and a block that disagreed between
/// them would be a packet the server throws out with no message at all.
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
        // …and so is the trade square's slot number, which is read through the
        // same `readPackGUID` and is a *number* rather than a guid. See
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

/// `CMSG_CANCEL_AURA`: **drop a buff we are carrying**, `u32 spellId`.
///
/// The same one-field shape as the cast cancel and a different question — this
/// is `BuffButton_OnClick`'s right-click, and it is the only thing the interface
/// can *do* to an aura.
///
/// **A spell id, not a slot**, which is worth stating because every other aura
/// packet in 1.12 is slot-keyed: `WorldSession::HandleCancelAuraOpcode` reads
/// one `uint32` and looks the spell up. The server refuses on its own terms —
/// a negative aura, or one carrying `SPELL_ATTR_NO_AURA_CANCEL` — and says
/// nothing when it does, so the client's own `AFLAG_CANCELABLE` check is what
/// keeps a right-click on a debuff from being a packet that vanishes.
pub fn cancel_aura_body(spell_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(spell_id);
    w.buf
}

/// `SMSG_UPDATE_AURA_DURATION`: `u8 slot`, `u32 remaining ms`.
///
/// **Five bytes, and the only statement the 1.12 protocol makes about how long
/// a buff has left.** It carries no spell id — the slot is the whole of the
/// join — and `SpellAuraHolder::UpdateAuraDuration` sends it only when the
/// aura's *target* is a player, and returns early for a permanent one. So:
///
/// * a buff on somebody else has no timer and never will, which is why the
///   game's own target and party frames draw none;
/// * an aura this never arrives for is **until cancelled**, which is exactly
///   `GetPlayerBuff`'s second return.
pub fn parse_aura_duration(body: &[u8]) -> Option<(u8, u32)> {
    let mut r = Reader::new(body);
    if !r.has(1 + 4) {
        return None;
    }
    let slot = r.u8();
    Some((slot, r.u32()))
}

/// `SMSG_LEVELUP_INFO`: the level reached, and what it bought.
///
/// ```text
/// u32 level
/// u32 healthGained
/// u32 powerGained[5]     mana, rage, focus, energy, happiness
/// u32 statGained[5]      strength, agility, stamina, intellect, spirit
/// ```
///
/// `Player::GiveLevel` writes exactly that — the mana and four literal zeroes,
/// then the five stat deltas against `GetCreateStat`. Forty-eight bytes.
///
/// **Every number after the first is a delta**, and all of them are read here
/// rather than just the level, because `ChatFrame_OnEvent`'s `PLAYER_LEVEL_UP`
/// branch is nine arguments long and compares `arg3` through `arg9` with `> 0`
/// before formatting each — so a client that raised the event with only the
/// level took the handler down on its second line. (It did, for one round;
/// `--audit --events` is what said so.)
///
/// The packet is also the only thing that says a level-up **happened** as
/// opposed to that the level is now different: `UNIT_FIELD_LEVEL` moves when a
/// target is selected, when a creature streams in, and at every login.
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

/// What one level bought — see [`parse_levelup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelUp {
    pub level: u32,
    /// Hit points gained.
    pub health: u32,
    /// …and mana, which is **zero for a class that does not run on it** — the
    /// server writes the mana delta only when `GetPowerType() == POWER_MANA`.
    /// That zero is load-bearing rather than incidental: it is what makes the
    /// chat line read "You have gained 42 hit points." for a warrior and "…and
    /// 30 mana." for a mage, off the same handler.
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
/// **Position is the whole meaning of this table.** See the module comment: the
/// byte on the wire is an index into `SpellCastResult`'s declaration order, and
/// for build 5875 that order is exactly this list. Do not sort it, do not
/// insert into it, and do not "tidy" a name — the name is a lookup key in a file
/// shipped inside the MPQs.
///
/// Three of the reasons name a key `GlobalStrings.lua` does not contain
/// (`AUTOTRACK_INTERRUPTED`, `HUNGER_SATIATED`, `THIRST_SATIATED`, and the
/// happiness case of `NO_POWER`). That is not a gap in the transcription: the
/// client displays nothing for them, and a missing key is how it does it.
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

/// **`SPELL_FAILED_OUT_OF_RANGE`** — the one reason worth *measuring* rather
/// than only saying, because it is the server stating a distance this client
/// can state too. Named rather than written as `0x59` at the call site, which
/// is this repo's standing rule for anything on the wire; the table above is
/// the authority for the number.
///
/// Read by `crate::game::combat::desync` in the renderer.
pub const SPELL_FAILED_OUT_OF_RANGE: u8 = 0x59;

/// `SPELL_FAILED_DONT_REPORT`, which the client never displays. The server
/// substitutes it for the real reason on a passive spell, and it also comes back
/// for a good deal of internal machinery.
pub const DONT_REPORT: u8 = 0x17;

/// The `GlobalStrings.lua` key for a wire reason, or `None` for a code outside
/// the table or one the client deliberately swallows.
pub fn cast_failure_key(reason: u8) -> Option<&'static str> {
    if reason == DONT_REPORT {
        return None;
    }
    CAST_FAILURE_KEYS.get(usize::from(reason)).copied()
}


/// **A talent's arithmetic, as the server states it** —
/// `SMSG_SET_FLAT_SPELL_MODIFIER` and `SMSG_SET_PCT_SPELL_MODIFIER`, three
/// bytes and a signed dword each.
///
/// One packet per **bit**, not per talent: `Player::SendSpellMod` loops over the
/// 64 bits of the modifier's `SpellFamilyFlags` mask and sends the running total
/// for each bit that is in it. So a talent affecting six spells is six packets,
/// each carrying the sum of every modifier this character has for that bit and
/// that operation — which is why a reader **replaces** rather than accumulates.
/// Getting that backwards doubles a talent every time it is re-sent, and it is
/// re-sent on every login and every talent change.
///
/// The two opcodes differ only in what the number means: flat is added, percent
/// is a percentage added to 100. Both are signed, and a negative percent is how
/// every cast-time and cost reduction in the game is written.
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

/// **The `SpellModOp` values this client reads**, by their vmangos names.
///
/// The enum runs to 32 and the server sends every one of them; these five are
/// the ones that change a number the client itself prints or predicts. The rest
/// are the server's arithmetic — damage, threat, crit chance, proc chance — and
/// nothing here would be able to check a value it had modified.
pub mod spell_mod_op {
    /// How long the aura lasts.
    pub const DURATION: u8 = 1;
    /// The `$d` and the cast bar's own length.
    pub const RANGE: u8 = 5;
    /// The wind-up.
    pub const CASTING_TIME: u8 = 10;
    /// The recovery, which is what the button's sweep is drawn from.
    pub const COOLDOWN: u8 = 11;
    /// The power the cast is paid for with.
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
    // A bit outside the 64 a mask has is a packet this client cannot file, and
    // filing it under a wrapped index would modify the wrong spells.
    if effect_bit >= 64 {
        return None;
    }
    Some(SpellModifier { effect_bit, op, value, percent })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    /// **`SMSG_UPDATE_AURA_DURATION` is five bytes, and the first is a byte.**
    /// Reading the slot as a `u32` puts every timer on slot 0 and reads the
    /// duration three bytes short — a plausible number under the wrong icon.
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

    /// **`SMSG_LEVELUP_INFO` is twelve words and the interface reads nine of
    /// them.** The four zero powers between the mana and the stats are the trap:
    /// a reader that took the stats from directly after the mana would report
    /// every level-up as granting nothing at all, which is exactly the shape
    /// `arg5 > 0` hides.
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

    /// The buff cancel is a **spell id**, where every other aura packet in 1.12
    /// is keyed by slot — see [`cancel_aura_body`].
    #[test]
    fn the_buff_cancel_names_a_spell_and_not_a_slot() {
        assert_eq!(cancel_aura_body(168), vec![168, 0, 0, 0]);
    }

    /// The spellbook's ids are `u16`, and the slot beside each one is not a
    /// slot. Reading the pair as one `u32` gives a spellbook of plausible
    /// nonsense — every id doubled and every other one dropped.
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

    /// **The three spellbook deltas disagree about width**, and each one is a
    /// different length: superseded is two `u16`s, learned is a `u16` and a
    /// trailing field, removed is a bare `u16`.
    #[test]
    fn the_three_spellbook_deltas_are_three_different_lengths() {
        let mut w = Writer::new();
        w.u16(11566).u16(11567);
        assert_eq!(parse_superceded_spell(&w.buf), Some((11566, 11567)));

        // Heroic Strike Rank 7 -> Rank 8, the pair measured on this project's
        // own warrior (`vale spellbook 11566` names the ranks — not 8 and
        // 9, which is the label rather than the id).
        // A body a byte short is refused rather than read crooked.
        assert_eq!(parse_superceded_spell(&w.buf[..3]), None);
        assert_eq!(parse_superceded_spell(&[]), None);

        assert_eq!(parse_removed_spell(&[0x2e, 0x2d]), Some(11566));
    }

    /// **`SMSG_LEARNED_SPELL` is a `u16` and a second field, not a `u32`.**
    ///
    /// It was read as a `u32` here for a long time and gave the right answer
    /// every time, because vmangos never assigns the trailing `actionBarSlot`
    /// and a zero high half makes the two readings agree. This pins the
    /// difference by putting something in that field: the `u32` reading would
    /// answer 0x0005_2D2E — 339,758 — and resolve to no spell at all.
    #[test]
    fn a_learned_spell_is_the_low_half_and_the_slot_beside_it_is_not_part_of_it() {
        let mut w = Writer::new();
        w.u16(11567).u16(5);
        assert_eq!(parse_learned_spell(&w.buf), Some(11567));

        // …and the ordinary packet, where that field is zero, reads the same —
        // which is why the wrong width was invisible.
        let mut plain = Writer::new();
        plain.u16(11567).u16(0);
        assert_eq!(parse_learned_spell(&plain.buf), Some(11567));

        assert_eq!(parse_learned_spell(&[0x2f]), None);
    }

    /// The cooldown block is behind the spell list, so a miscount above it
    /// reads durations out of spell ids. And a body that stops before it costs
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

    /// The permanent marker is a *flag* in the top bit of the category
    /// duration. Read as a duration it is 24 days, which is indistinguishable
    /// from "ready" to anything that only looks at whether it has elapsed.
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

    /// …and the same word, written back. **The kind is the top byte in both
    /// directions**, which is the one thing a second packing could get wrong —
    /// and would get wrong invisibly, since `HandleSetActionButtonOpcode` drops
    /// an unknown type without answering.
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
        // **A removal is the same packet with a zero word**, not the absence of
        // one — the server's own `if (!packetData)` branch.
        assert_eq!(set_action_button_body(11, 0), vec![11, 0, 0, 0, 0]);
    }

    /// **The four extra bars are four bits of one byte, and the order is the
    /// interface's argument order.**
    ///
    /// Pinned as *values* rather than as an enum, because the whole packet is
    /// this byte and a transposition would be invisible: switching bars 1 and 2
    /// puts the same two rows of buttons on the screen in the wrong two places,
    /// with no error anywhere and no way to notice until a relog moved them.
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

    /// **A fifth bit cannot get onto the wire**, which is the one mistake this
    /// packet invites: `SetActionBarToggles` takes *five* Lua arguments and the
    /// client packs four, so "Always Show
    /// ActionBars" is a saved variable rather than a saved field. A mask that
    /// carried it would set a bit in `PLAYER_FIELD_BYTES` that the server hands
    /// straight back and nothing ever reads.
    #[test]
    fn the_always_show_flag_is_not_a_fifth_bit() {
        assert_eq!(set_actionbar_toggles_body(0xFF), vec![0x0F]);
        assert_eq!(set_actionbar_toggles_body(0x10), vec![0]);
    }

    /// **A cast result is sent on success too.** Treating the packet's arrival
    /// as a failure cancels every cast bar the moment it starts.
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

    /// **The three equipped-item refusals carry the argument to their own
    /// `%s`**, and reading it is the whole of the warrior report: drawn without
    /// it, `SPELL_FAILED_EQUIPPED_ITEM_CLASS_MAINHAND` reads "Must have a %s
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

        // **A truncated tail is not half an argument.** A body that stops after
        // the reason byte must answer `None` rather than naming whatever the
        // reader ran off into — the requirement is three `u32`s or it is
        // nothing.
        let mut short = Writer::new();
        short.u32(772).u8(2).u8(0x1a).u32(2);
        let result = parse_cast_result(&short.buf).expect("a result");
        assert_eq!(result.failure, Some(0x1a), "the reason still reads");
        assert_eq!(result.requirement, None);

        // …and a reason that is *not* one of the three never reads a tail, even
        // when bytes happen to follow it.
        let mut other = Writer::new();
        other.u32(772).u8(2).u8(0x12).u32(2).u32(4).u32(0);
        assert_eq!(parse_cast_result(&other.buf).expect("a result").requirement, None);
    }

    /// The table is an index and its length is the check. 146 for build 5875,
    /// agreed on by vmangos' enum and by the client's own name table.
    #[test]
    fn the_failure_table_is_the_1_12_enum_in_order() {
        assert_eq!(CAST_FAILURE_KEYS.len(), 146);
        assert_eq!(cast_failure_key(0x00), Some("SPELL_FAILED_AFFECTING_COMBAT"));
        assert_eq!(cast_failure_key(0x59), Some("SPELL_FAILED_OUT_OF_RANGE"));
        // …and the named constant is the same code, which is the whole reason
        // it is named: a call site testing the wrong byte would measure nothing
        // and report nothing, silently.
        assert_eq!(
            cast_failure_key(SPELL_FAILED_OUT_OF_RANGE),
            Some("SPELL_FAILED_OUT_OF_RANGE")
        );
        assert_eq!(cast_failure_key(0x91), Some("SPELL_FAILED_UNKNOWN"));
        // Off the end is silence rather than a panic or a wrong message.
        assert_eq!(cast_failure_key(0x92), None);
        assert_eq!(cast_failure_key(0xFF), None);
        // …and the one the client is documented never to display.
        assert_eq!(cast_failure_key(DONT_REPORT), None);
    }

    /// **A self-cast carries no guid at all.** The mask is zero and the body is
    /// six bytes; shipping the current selection instead is what turns Ice
    /// Armor into "Invalid target".
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
        // And the guid really is the packed form: a plain one would be eight
        // bytes with no mask in front.
        assert_eq!(body.len(), 4 + 2 + 1 + 5);
    }

    /// **A placed cast is three bare floats and no guid** — the shape
    /// `SpellCastTargets::read` reads for `TARGET_FLAG_DEST_LOCATION`, in that
    /// order and in the server's own axes.
    ///
    /// The length is the assertion that matters: 3.x puts a packed guid in front
    /// of the coordinates and 1.12 does not, so a body four bytes longer than
    /// this is a packet the server reads off the end of and drops.
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

        // …and the item verb writes the *same* block, because both opcodes end
        // in the same `SpellCastTargets::read` on the server.
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
    /// **What a moved bar slot sends is what comes back at the next login.**
    ///
    /// The round trip a drag-and-drop depends on, as far as it can be checked
    /// without a server: the word `set_action_button_body` sends is the word
    /// `parse_action_buttons` reads, and vmangos'
    /// `HandleSetActionButtonOpcode` splits it with the same two macros —
    /// `ACTION_BUTTON_ACTION` is the low 24 bits and `ACTION_BUTTON_TYPE` the
    /// top byte.
    ///
    /// **A zero word is a removal on both sides**, which is the half that is
    /// easy to leave out: `if (!packet.packetData) removeActionButton(button)`,
    /// and this client sends exactly that for a slot a pick-up emptied.
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

            // …and the server's own split, which is what it stores and hands
            // back in `SMSG_ACTION_BUTTONS`.
            assert_eq!(packed & 0x00FF_FFFF, action, "ACTION_BUTTON_ACTION");
            assert_eq!((packed >> 24) as u8, kind, "ACTION_BUTTON_TYPE");
        }

        // An emptied slot is a zero word, which the server reads as a removal
        // rather than as "spell 0".
        assert_eq!(set_action_button_body(11, 0)[1..], [0, 0, 0, 0]);
    }

    /// **Three bytes and a signed dword**, and the two opcodes differ only in
    /// what the number means — see [`SpellModifier`].
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
        // **A bit outside the mask's 64 is refused**, because filing it under a
        // wrapped index would modify the wrong spells.
        assert!(parse_spell_modifier(&body(64, 0, 1), false).is_none());
        assert!(parse_spell_modifier(&body(63, 0, 1), false).is_some());
        // …and a short body is not one.
        assert!(parse_spell_modifier(&[0, 0, 0, 0, 0], false).is_none());
    }

    /// **The trade square's "guid" is a slot number** — see
    /// [`CastTarget::TradeSlot`], whose note carries the server's own reading.
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
