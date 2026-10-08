//! Action bar input: the action bar, the cooldowns that gate it, the cast a
//! button sends, and the melee swing.
//!
//! The server owns the bar's contents: `SMSG_ACTION_BUTTONS` restores the
//! layout the last session left. Using a button is the client's job. Four rules
//! of the 1.12.1 client apply here, and the bar does not work without each of
//! them:
//!
//! * Attack is not a cast. Spell 6603 is the pseudo-spell every character
//!   carries in slot 1, and pressing it sends `CMSG_ATTACKSWING` at the
//!   selection. The server refuses `CMSG_CAST_SPELL` with that id in its
//!   "which he shouldn't have" branch, so a client that sends every button as
//!   a cast cannot melee.
//! * A swing with no target acquires one. The client picks the best candidate
//!   and swings at it, so pressing attack in a fight that started behind the
//!   character works.
//! * The target comes from the spell, not from the selection. See
//!   [`vale_assets::tables::spellbook::resolve_aim`]: 14,002 of the game's
//!   22,360 spells are cast with no target, and sending the selection with one
//!   of those makes a self-buff fail with "Invalid target".
//! * The global cooldown starts locally, when the cast is sent. No packet
//!   starts it: the client starts it as the cast goes out, and vmangos sends
//!   nothing for it. A client that waited for the server would update the bar
//!   a round trip late and send two casts on a double press.
//!
//! ## Which clock a cooldown runs on
//!
//! Each spell has three timers: its own recovery, its category's recovery, and
//! the global cooldown. A read takes the longest of the three that apply, which
//! is how one cast's GCD reaches every other button. Each timer is started by a
//! different event:
//!
//! ```text
//! global cooldown   locally, when the cast is sent
//! own recovery      when this character's SMSG_SPELL_GO comes back
//! override/lockout  SMSG_SPELL_COOLDOWN — a counterspell, a GM reset
//! parked release    SMSG_COOLDOWN_EVENT — Stealth, Feign Death
//! a failed cast     clears the global cooldown only
//! ```
//!
//! A refused cast never reached its `SPELL_GO`, so it has no recovery to clear.
//! Its GCD was already started locally, and without the clear it would lock the
//! bar for 1.5 seconds.
//!
//! ## The server owns the cast itself
//!
//! The global cooldown is the only thing a press starts. Everything else about
//! a cast waits for the server to confirm it: the cast bar, the wind-up pose,
//! the spell art on the caster's hands, the missile, the sound. This client
//! used to draw all of it at the press and remove it on a refusal, so a refused
//! spell ("the animation plays but the spell was not really cast") played in
//! full, twice a second while a key was held.
//!
//! The 1.12.1 client (build 5875) and vmangos agree on this split. The client's
//! press does four things: its local refusals, the pending record, the GCD, and
//! the send. It raises `SPELLCAST_START` only on receiving `SMSG_SPELL_START`.
//! vmangos comments its `SendSpellStart()` with `// will show cast bar` and its
//! `AddGCD` with `// add gcd server side (client side is handled by client itself)`.
//! The client owns the cooldown and the server owns the cast.
//!
//! [`Casting::pending`], the equivalent of the 1.12.1 client's pending-cast
//! record, covers the round trip. It draws nothing; it exists so that a
//! repeated press is dropped instead of sent again.
//!
//! ## The verb and the key that runs it are separate
//!
//! [`use_action`] is this module's `UseAction`, the C function
//! `ActionButton.lua` calls with a one-based slot and the game's `onSelf` flag.
//! It does not know which key was pressed. The key belongs to `Bindings.xml`;
//! it arrives here as a
//! [`Binding::ActionButton`][crate::input::bindings::Binding::ActionButton] message, and
//! `SELFACTIONBUTTON1` is the same verb with the flag set. See
//! [`crate::input::bindings`] for why the game itself keeps the two separate.

use super::api;
use crate::input::bindings::{Binding, BindingPressed, BindingSet};
use super::events::{
    ActionbarBonusChanged, ActionbarPageChanged, ActionbarSlotChanged, ActionbarUpdateCooldown,
    ActionbarUpdateState, SpellUpdateCooldown, SpellcastChannelStart,
    SpellcastChannelStop, SpellcastChannelUpdate, SpellcastDelayed, SpellcastFailed,
    SpellcastInterrupted,
    SpellcastStart, SpellcastStop, StartAutorepeatSpell, StopAutorepeatSpell, ALL_SLOTS,
};
use super::messages::UiErrors;
use super::target::Selection;
use crate::assets::GameAssets;
use crate::world::entities::{Sheath, SheathRequest};
use crate::world::session::{ActiveSession, LocalPlayer, Session, WorldEntity};
use vale_assets::tables::faction::Reaction;
use vale_assets::tables::spellbook::{resolve_aim, CastAim, Candidate, SpellInfo};
use vale_protocol::play::spells::{action_kind, CastTarget, SPELL_ATTACK};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::time::{Duration, Instant};

/// The number of action slots this client holds: 120, which is what
/// `SMSG_ACTION_BUTTONS` carries and what the interface asks about.
///
/// The bar has six pages, and `ActionButton_GetPagedID` computes
/// `id + (page - 1) * NUM_ACTIONBAR_BUTTONS`. Once the page arrows on either
/// side of the main bar work (`--audit --clicks`), the interface asks about
/// slots 13 to 72, and a twelve-slot vector answers "empty" for all of them.
/// The value was 12 while the main bar's keys `1`..`=` were the only way to
/// press a button. The server has always sent all 120.
///
/// The interface chooses which twelve slots it shows, not this module:
/// `CURRENT_ACTIONBAR_PAGE` is a Lua global, `GetBonusBarOffset()` is answered
/// from [`ActionBar::bonus_bar`], and every button adds both to its own id
/// before it asks anything here. A slot number from the interface is therefore
/// already absolute.
pub const BAR_SLOTS: usize = 120;

/// One slot of the bar, resolved against the archives.
#[derive(Clone, Debug)]
pub struct Slot {
    /// The spell id or item entry the server remembers here.
    pub action: u32,
    pub kind: u8,
    /// What the archives say about it, for a spell. `None` for an item (whose
    /// name is a server answer) or a spell `Spell.dbc` does not carry.
    pub spell: Option<SpellInfo>,
}

impl Slot {
    pub fn label(&self) -> String {
        match &self.spell {
            Some(info) => info.label(),
            None if self.kind == action_kind::ITEM => format!("item {}", self.action),
            None => format!("spell {}", self.action),
        }
    }

    /// Whether this slot is the auto-attack toggle rather than a cast. See the
    /// module comment.
    pub fn is_auto_attack(&self) -> bool {
        self.kind == action_kind::SPELL && self.action == SPELL_ATTACK
    }
}

/// The bar as this client holds it: 120 slots, resolved.
#[derive(Resource, Default)]
pub struct ActionBar {
    pub slots: Vec<Option<Slot>>,
    /// The `spellbook_version` these were built from, so the rebuild happens on
    /// a difference rather than every frame. Resolving twelve spells means
    /// twelve string reads from a 22,360-row DBC.
    built_from: Option<u32>,
    /// Which parse of the DBCs the slots were resolved from. See
    /// `GameAssets::tables_generation`, and [`rebuild_bar`] for why the
    /// server's version alone is not enough.
    built_with: u64,
    /// Every spell the character knows and can press. This list differs from
    /// the spellbook's list by one filter; see [`super::spellbook`].
    pub known: Vec<SpellInfo>,
    /// `GetBonusBarOffset()`: which of the four bonus bars the character's
    /// current form shows, or 0 for the ordinary paged bar.
    ///
    /// Read from `UNIT_FIELD_BYTES_1`'s form byte through
    /// `SpellShapeshiftForm.dbc`'s `bonusActionBar` column
    /// ([`vale_assets::tables::spellbook::ShapeshiftForms`]). The client
    /// computes this value itself, because the packet carries the form and
    /// never the bar. See [`ActionbarBonusChanged`], the event the interface
    /// redraws on.
    pub bonus_bar: u8,
    /// `GetActionBarToggles()`: which of the four extra bars this character
    /// has switched on, as a mask of
    /// [`vale_protocol::play::spells::multi_bar`] bits.
    ///
    /// This is the only piece of interface layout in 1.12 that the server
    /// keeps: `PLAYER_FIELD_BYTES` byte 2, written by
    /// `CMSG_SET_ACTIONBAR_TOGGLES` and sent back unchanged. See
    /// [`follow_bar_toggles`] for the copy and
    /// [`vale_protocol::state::objects::Entity::action_bar_toggles`] for the field.
    ///
    /// No event goes with it, unlike [`Self::bonus_bar`], and the interface
    /// does not need one: `UIParent.lua` reads the value once, on
    /// `PLAYER_ENTERING_WORLD`. Every later change comes from the options
    /// panel, which writes its own `SHOW_MULTI_ACTIONBAR_*` globals and calls
    /// `MultiActionBar_Update()` itself without reading the value again.
    pub toggles: u8,
}

/// The ranged attack that is repeating, or `None`.
///
/// Auto Shot and a wand's Shoot are neither ordinary casts nor the melee
/// auto-attack. Pressing one starts a loop that the server runs.
/// `Spell::prepare` files an `IsAutoRepeatRangedSpell()` in
/// `CURRENT_AUTOREPEAT_SPELL` instead of casting it, and
/// `Unit::_UpdateAutoRepeatSpell` fires a triggered copy on the ranged attack
/// timer for as long as it stays there. One `CMSG_CAST_SPELL` therefore
/// produces an indefinite stream of `SMSG_SPELL_GO`s. Before this resource
/// existed, the button could only turn the loop on.
///
/// It lives here rather than in `SessionStatus` because the client records the
/// start itself: the server acknowledges the cast like any other and says
/// nothing about the loop. The end does come from the server, as
/// [`vale_protocol::play::spells::PlayerEvent::AutoRepeatCancelled`]. Every way
/// the loop can end (the press, the target dying, walking out of range, a wand
/// user moving) produces that one packet, so [`Self::stop`] is called from the
/// event drain rather than from the press.
#[derive(Resource, Default)]
pub struct AutoRepeat {
    /// Which spell, for `IsAutoRepeatAction`.
    pub spell: Option<u32>,
}

impl AutoRepeat {
    /// Whether `spell_id` is the one running — what `IsAutoRepeatAction` asks
    /// of each button in turn.
    pub fn is(&self, spell_id: u32) -> bool {
        self.spell == Some(spell_id)
    }

    /// Record that the loop has started, and announce it once.
    ///
    /// Idempotent on the spell, like every other edge in this module:
    /// `START_AUTOREPEAT_SPELL` starts a flash clock in `ActionButton_OnEvent`,
    /// and restarting it on every press would leave the button lit or dark
    /// depending on how the two clocks line up.
    pub(crate) fn begin(&mut self, spell_id: u32, events: &mut ActionEvents) {
        if self.spell == Some(spell_id) {
            return;
        }
        self.spell = Some(spell_id);
        events.autorepeat_start.write(StartAutorepeatSpell);
        // The checked border is a separate reading from the flash:
        // `ActionButton_UpdateState` asks `IsCurrentAction or
        // IsAutoRepeatAction`, and nothing else re-runs it.
        events.state.write(ActionbarUpdateState);
    }

    /// Record that the loop has stopped, and announce it once.
    pub(crate) fn stop(&mut self, events: &mut ActionEvents) {
        if self.spell.take().is_none() {
            return;
        }
        events.autorepeat_stop.write(StopAutorepeatSpell);
        events.state.write(ActionbarUpdateState);
    }
}

/// The four resources a keypress writes, bundled into one system parameter.
///
/// A Bevy function system takes its parameters as one tuple, and `SystemParam`
/// is implemented for tuples of up to sixteen elements. [`run_bindings`] had
/// exactly sixteen. A seventeenth parameter does not produce an error about
/// the limit; it produces "the method `chain` exists but its trait bounds were
/// not satisfied" on the plugin, away from the system that caused it.
/// `LuaWorld` in `lua::api` is bundled for the same reason.
///
/// The members are what a press changes about this character's casting. The
/// bar and the spellbook are not members: they are rebuilt from the server and
/// only read here.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PressState<'w> {
    pub cooldowns: ResMut<'w, Cooldowns>,
    pub casting: ResMut<'w, Casting>,
    pub auto_repeat: ResMut<'w, AutoRepeat>,
    pub targeting: ResMut<'w, SpellTargeting>,
}

/// The cast this client believes is in progress, for the cast bar.
///
/// This is the server's state, not a prediction. The cast used to begin at the
/// send, so that the bar would not start a round trip late. The 1.12.1 client
/// waits: it raises `SPELLCAST_START` only on receiving `SMSG_SPELL_START`, and
/// vmangos comments that packet `// will show cast bar`. A cast this client
/// shows is therefore one the server accepted, and a refused press shows
/// nothing, where it used to show a wind-up and then remove it.
///
/// [`Self::pending`] covers the time between the press and the answer. The
/// 1.12.1 client keeps the same state.
#[derive(Resource, Default)]
pub struct Casting {
    pub spell_id: u32,
    pub name: String,
    pub started: Option<Instant>,
    pub duration: Duration,
    /// The spell id sent to the server and not yet answered. It is held until
    /// the server accepts (`SMSG_SPELL_START`), refuses (`SMSG_CAST_RESULT`
    /// with a failure), or a next-swing ability is released.
    ///
    /// The 1.12.1 client keeps the same record: the press fills it with the
    /// spell and its targets, and a cast failure clears it. Two things here read
    /// it, and neither draws anything:
    ///
    /// * A repeated press, which is dropped instead of sent again. The 1.12.1
    ///   client does send it, and the server answers each repeat with
    ///   `SPELL_FAILED_SPELL_IN_PROGRESS`. Dropping it locally has the same
    ///   result with one packet instead of ten, and matches the rule applied in
    ///   [`cast_known_spell`] to a cast that is already running.
    /// * The spell targeting cursor, so that a second press while the first is
    ///   in flight does not put the cursor back up.
    ///
    /// The gate relies on every ask being answered. `Spell::SendCastResult`
    /// writes a status byte on both paths and always sends, so every
    /// `CMSG_CAST_SPELL` that reaches `Spell::prepare` gets an acceptance or a
    /// refusal. The exception is `HandleCastSpellOpcode`'s "which he shouldn't
    /// have" branch, which returns without a reply. A passive is refused here
    /// before the send, so it never reaches that branch. Leaving the world
    /// resets the whole resource either way (see `forget`).
    ///
    /// A spell from the character's own book can still reach that branch.
    /// `HandleCastSpellOpcode` returns without a reply on
    /// `!HasActiveSpell(spellId)`, which tests whether the spell is active, not
    /// whether it is known, and an action button holding a superseded rank
    /// fails that test. The server sends `SMSG_SUPERCEDED_SPELL` when a higher
    /// rank is learned so the client can replace the id in the bar and the
    /// book. [`crate::world::incoming`] reads that opcode, and
    /// [`super::supersede`] repairs ranks that went stale in
    /// `character_action` before it was read. Measured on the reporting
    /// player's warrior: button 73 held Heroic Strike 11566 while
    /// `character_spell` had only 11567, and button 75 held Rend 11572 against
    /// 11573.
    ///
    /// Without a deadline, one unanswered press blocked the character for the
    /// rest of the session: every later press of any spell met the gate in
    /// [`cast_known_spell`] and printed "another action is in progress".
    /// [`Self::asked_at`] is the deadline that limits a dropped reply to the
    /// press it belongs to.
    pub pending: Option<u32>,
    /// When [`Self::pending`] was set, so that an unanswered ask expires.
    ///
    /// `session::logout` and the character delete apply the same rule: where
    /// the server has a path that sends nothing, a deadline counts as a refusal
    /// rather than as a dead socket. The expiry shows no message, because the
    /// 1.12.1 client shows nothing here either and the press did nothing. It
    /// only lets the player press something else.
    pub asked_at: Option<Instant>,
    /// The spell being channelled, if any. This is a separate state from
    /// [`Self::started`] and is kept in a separate field.
    ///
    /// A channel has no wind-up: on the wire it is an instant, so `started` is
    /// `None` and the whole channel happens after `SMSG_SPELL_GO`. The two must
    /// be distinguished because the server distinguishes them.
    /// `Spell::prepare`'s "another action is in progress" refusal skips
    /// channels (`IsNonMeleeSpellCasted(false, true, true)`, whose second
    /// argument is `skipChanneled`), so a cast pressed during a channel is
    /// accepted and replaces it, while one pressed during a wind-up is refused.
    /// See [`in_progress`].
    pub channelling: Option<u32>,
    /// The next-swing ability waiting on the weapon (Heroic Strike, Raptor
    /// Strike, Cleave). This is a third state, separate from the two above.
    ///
    /// It draws no bar, because nothing is winding up. `in_progress` does not
    /// consult it, because the server does not either: `Spell::prepare`
    /// refuses on `CURRENT_GENERIC_SPELL`, and a melee spell is not one, so a
    /// Fireball pressed with Heroic Strike queued is accepted by the server and
    /// must be accepted here.
    ///
    /// It exists for `IsCurrentAction`. The 1.12.1 client answers
    /// `IsCurrentAction` true for the button holding the queued next-swing
    /// spell, which lights Heroic Strike's border from the press until the
    /// swing lands. Without this field the whole family of abilities could be
    /// pressed but showed no state.
    ///
    /// Only the server clears it, never a timer here: the release
    /// (`SMSG_SPELL_GO` naming this character), a refusal, or an interrupt. See
    /// [`drain_events`]. A queued ability that nothing releases stays queued: a
    /// player standing out of reach keeps it armed, as in the 1.12.1 client.
    /// This is not a leak.
    pub next_swing: Option<u32>,
}

impl Casting {
    /// How far through, 0..1, or `None` when nothing is being cast.
    pub fn progress(&self) -> Option<f32> {
        let started = self.started?;
        if self.duration.is_zero() {
            return None;
        }
        let elapsed = started.elapsed().as_secs_f32() / self.duration.as_secs_f32();
        (elapsed <= 1.0).then_some(elapsed)
    }

    /// Put a cast on the bar, on the server's word.
    ///
    /// The duration is `SMSG_SPELL_START`'s `m_timer`, not `Spell.dbc`'s base
    /// cast time. The server has already applied haste, talents and any
    /// modifier this client does not model, so the bar has the cast's real
    /// length. This is a second benefit of waiting for the packet.
    ///
    /// The caller supplies `name`, because the id is resolved through the
    /// catalog, which this module does not hold.
    pub(crate) fn begin(&mut self, spell_id: u32, name: String, cast_time_ms: u32) {
        self.spell_id = spell_id;
        self.name = name;
        self.duration = Duration::from_millis(u64::from(cast_time_ms));
        self.started = (cast_time_ms > 0).then(Instant::now);
        // The accepted cast ends any channel, as on the server, which accepts
        // the cast and drops the channel.
        self.channelling = None;
        // The cast is no longer waiting for an answer, so the pending id and its
        // deadline are cleared. See [`Self::asked_at`].
        self.pending = None;
        self.asked_at = None;
    }

    /// Lengthen the cast, and return whether there was a cast to lengthen.
    ///
    /// `SMSG_SPELL_DELAYED` carries a difference the server has already applied
    /// to its own `m_timer`, so the bar gets longer: `started` stays and the end
    /// moves. `CastingBarFrame_OnEvent` does the same with `arg1` (it moves both
    /// ends, which has the same effect because it restates the range).
    ///
    /// A channel is refused. vmangos delays a channel through `DelayedChannel`,
    /// which shortens `m_timer` and is announced as `MSG_CHANNEL_UPDATE`; a
    /// `SPELLCAST_DELAYED` raised over a channel bar would move the wrong end of
    /// it. A cast with no wind-up has no bar. An instant cannot receive
    /// `SMSG_SPELL_DELAYED`, but if one did, there is nothing to lengthen and no
    /// bar is created.
    pub(crate) fn delay(&mut self, delay_ms: u32) -> bool {
        if self.started.is_none() || self.channelling.is_some() {
            return false;
        }
        self.duration += Duration::from_millis(u64::from(delay_ms));
        true
    }

    /// End whatever was on the bar, and return whether anything was on it.
    ///
    /// The return value decides whether `SPELLCAST_STOP` is raised. It mirrors
    /// [`send_cast`]'s rule that an instant raises no `SPELLCAST_START`. A stop
    /// for a bar that never appeared is at best a no-op. At worst it arrives
    /// while a channel bar is up, and `CastingBarFrame_OnEvent` turns that bar
    /// green and fades it, so the channel bar vanishes a frame after it
    /// appears.
    pub(crate) fn end(&mut self) -> bool {
        self.channelling = None;
        self.started.take().is_some()
    }
}

/// Clear the queued next-swing ability, if it is the spell this packet names.
///
/// The three packets that can empty the queue all call this, and it raises the
/// bar's event once. `ActionButton_UpdateState` is the only function that
/// re-reads `IsCurrentAction`, and nothing else in FrameXML would run it, so
/// without the event the border would stay lit until the next unrelated slot
/// update.
pub(crate) fn disarm_next_swing(casting: &mut Casting, spell_id: u32, events: &mut ActionEvents) {
    if casting.next_swing != Some(spell_id) {
        return;
    }
    casting.next_swing = None;
    events.state.write(ActionbarUpdateState);
}

/// Clear the auto-repeat when the server refuses the press that started it.
/// No packet announces this end of the loop.
///
/// [`AutoRepeat::stop`] is otherwise called only on `SMSG_CANCEL_AUTO_REPEAT`,
/// which cannot arrive in this case. `SpellCaster::InterruptSpell` sends it
/// only when `m_currentSpells[CURRENT_AUTOREPEAT_SPELL]` is set, and a refused
/// press never sets it: `Spell::prepare` runs `CheckCast`, finds that the
/// result is not one of the two `IsAcceptableAutorepeatError` accepts
/// (`SPELL_CAST_OK` and `SPELL_FAILED_MOVING`, the second so that a hunter can
/// start the loop while running), sends `SMSG_CAST_FAILED` and calls
/// `finish(false)`.
///
/// The refusal packet is therefore the only notice of this end. A client that
/// ignores it keeps the checked border lit for the session. Pressing the
/// button again does not help: a client that believes the loop is running
/// sends `CMSG_CANCEL_AUTO_REPEAT_SPELL`, the server interrupts a spell it does
/// not have, and it sends nothing about that either.
///
/// This checks the spell id, unlike the [`Casting::pending`] clear next to it
/// in the same arm. A refusal for a different spell says nothing about a loop
/// that is running, and the ranged loop survives other casts; that is what
/// `SetCurrentCastedSpell`'s `Category == 351` test is for.
pub(crate) fn disarm_auto_repeat(
    auto_repeat: &mut AutoRepeat,
    spell_id: u32,
    events: &mut ActionEvents,
) {
    if !auto_repeat.is(spell_id) {
        return;
    }
    auto_repeat.stop(events);
}

/// A cast waiting for the player to choose its target: 1.12's targeting
/// cursor.
///
/// This is the third outcome of the aiming rule. Pressing a heal with nothing
/// selected does not fail in the 1.12.1 client; the client shows the targeting
/// cursor and waits for a click. `resolve_aim`'s doc describes the decision,
/// which depends on one bit: a spell that requires a hostile unit gets an
/// error message, and every other spell asks.
///
/// The state is only a spell id. The unit that satisfies it is resolved again
/// at the click, through the same [`resolve_aim`] the press used, because the
/// world changes in between: the unit under the pointer may have died, changed
/// faction or moved out of range while the cursor was up, and a cached answer
/// would cast at it anyway.
#[derive(Resource, Default)]
pub struct SpellTargeting {
    /// The spell awaiting a target, if any.
    spell: Option<u32>,
    /// What the cursor is waiting for: a unit, a location or an item. Blizzard,
    /// Flamestrike and Rain of Fire use the location mode. See
    /// [`vale_assets::tables::spellbook::CastAim::WantsGround`].
    ///
    /// It is a field beside the id rather than three resources, because the
    /// modes are otherwise identical: one spell waiting, one cursor, one click
    /// to end it, one Escape to cancel it. Only what the click resolves to
    /// differs.
    asking: Asking,
    /// Where the pointer meets the floor this frame, in WoW axes. `None` when
    /// the pointer is on the sky or off the loaded world. Written by
    /// [`crate::interface::target::spell_ground_under_pointer`], only while
    /// [`Self::wants_ground`] is true, because the ray walks the terrain.
    pub over_ground: Option<[f32; 3]>,
    /// Whether the unit under the pointer would satisfy the spell. It selects
    /// `Cast.blp` or `UnableCast.blp`, the only feedback the player gets before
    /// clicking.
    pub over_valid: bool,
    /// True for the frame in which this mode consumed a click. A click that
    /// casts must not also change the target, and two systems make those
    /// decisions: `pick_spell_target` runs first and sets this flag, and
    /// `target::select_on_click` reads it and does nothing. System order alone
    /// is not enough, because it says which system runs first, not which one
    /// acted.
    pub took_click: bool,
}

impl SpellTargeting {
    /// `SpellIsTargeting()`: whether the cursor is up.
    pub fn is_targeting(&self) -> bool {
        self.spell.is_some()
    }

    /// The spell the cursor is holding, if it is up.
    pub fn spell(&self) -> Option<u32> {
        self.spell
    }

    /// Whether the cursor is waiting for a location rather than a unit.
    pub fn wants_ground(&self) -> bool {
        self.spell.is_some() && self.asking == Asking::Ground
    }

    /// Whether the cursor is waiting for an item: the enchanting formulas, the
    /// poisons and the sharpening stones, which 1.12 aims by clicking a bag
    /// slot.
    ///
    /// The interface has no separate function for this: `ContainerFrame.lua:596`
    /// calls `UseContainerItem(bag, slot)`, and the C side decides whether that
    /// is a use or a target choice. The reader is therefore the use path in
    /// [`crate::interface::items`], where the clicked slot has already been
    /// resolved to an item.
    pub fn wants_item(&self) -> bool {
        self.spell.is_some() && self.asking == Asking::Item
    }

    /// `SpellStopTargeting()`: put the cursor away without casting.
    ///
    /// Escape, a right click, a click on empty ground and the interface's own
    /// function all end here. It is not an error: the player cancelled, which
    /// is not a refused cast and raises no `SPELLCAST_FAILED`.
    pub fn stop(&mut self) {
        self.spell = None;
        self.asking = Asking::Unit;
        self.over_ground = None;
        self.over_valid = false;
    }

    /// Put the cursor up. `pub(crate)` rather than private for the same reason
    /// [`Self::stop`] is public: the other half of this mode is in
    /// [`super::target`], which ends it on a click and must be able to start
    /// one to test that it declines while the cursor is up.
    pub(crate) fn begin(&mut self, spell_id: u32, asking: Asking) {
        self.spell = Some(spell_id);
        self.asking = asking;
        self.over_ground = None;
        self.over_valid = false;
    }
}

/// What the targeting cursor is waiting for.
///
/// There are three modes and one cursor. The click that ends each mode is the
/// same gesture; only the answer differs: a unit, three floats, or an item's
/// guid. See [`SpellTargeting`], whose state is this value plus the spell id.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Asking {
    #[default]
    Unit,
    Ground,
    /// `TARGET_FLAG_ITEM`. Answered by a bag or paperdoll click rather than a
    /// click in the world, so nothing in [`super::target`] ends this mode.
    Item,
}

/// What the player pointed the waiting cast at: a unit, an item, a trade slot
/// or a location.
///
/// One enum rather than separate parameters through [`cast_known_spell`],
/// because each variant is the answer to the cursor's question, whichever kind
/// of question it was. `None` at that call site means that nobody has been
/// asked yet, which decides between putting the cursor up and refusing.
#[derive(Debug, Clone, Copy)]
enum Pointed {
    Unit(Entity),
    /// An item's guid, from a bag or paperdoll click. See [`SpellItemPicked`],
    /// the message that carries it.
    Item(u64),
    /// A slot in the trade window, identified by slot number rather than by
    /// guid. See [`vale_protocol::play::spells::CastTarget::TradeSlot`].
    TradeSlot(u8),
    /// In WoW axes, the frame `CastTarget::Dest` is sent in. The conversion
    /// happens once, where the ray is walked, rather than next to the socket.
    Ground([f32; 3]),
}

/// An item was clicked for the waiting cast: the other end of
/// [`SpellTargeting::wants_item`].
///
/// Written by the use path in [`crate::interface::items`] rather than by the
/// world pick, because 1.12's interface has no separate function for this: the
/// bag slot's `OnClick` calls `UseContainerItem(bag, slot)`, and the C side
/// decides whether that is a use or a target choice. That path has already
/// resolved the item by the time it has a slot, so the guid is available.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellItemPicked {
    /// The item object's guid, which is what `TARGET_FLAG_ITEM` carries.
    pub guid: u64,
}

/// A unit was picked for the waiting cast: the click, sent as a message.
///
/// Picking and casting are separate systems because they need different parts
/// of the world. Finding what is under the pointer needs the hover state and
/// the mouse; casting needs the bar, the cooldowns, the session and the error
/// frame, which already make up the largest parameter list in this module. A
/// message between them keeps [`run_bindings`] the only entry point for a
/// cast, the property described in [`cast_known_spell`]'s doc.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellTargetPicked {
    /// The unit clicked, as an entity: the same handle [`Selection`] holds, so
    /// the cast path resolves it the same way it resolves a selection.
    pub unit: Entity,
}

/// The same click, landed on the floor, for a cast placed at a location.
///
/// A separate message rather than a variant of [`SpellTargetPicked`], for the
/// same reason that keeps [`run_bindings`] the only entry point for a cast: one
/// system writes both, but they answer different questions, and the world pick
/// that produces a unit cannot produce a point (its ray tests pick boxes, not
/// the ground).
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellGroundPicked {
    /// The point on the floor, in WoW axes and yards, which is the frame the
    /// packet uses. See [`Pointed::Ground`].
    pub at: [f32; 3],
}

/// Whether a cast is in progress that a new press must not disturb.
///
/// Only a wind-up counts. This is a named function rather than
/// `casting.started.is_some()` at the two call sites because the field does not
/// show the reason: it mirrors `Spell::prepare`'s gate,
/// `IsNonMeleeSpellCasted(withDelayed = false, skipChanneled = true,
/// skipAutorepeat = true)`, whose second and third arguments exclude a channel
/// and an auto-shot.
fn in_progress(casting: &Casting) -> bool {
    casting.started.is_some()
}

/// How long an unanswered `CMSG_CAST_SPELL` holds the button.
///
/// Every reply this waits for takes one round trip, because vmangos answers
/// inside the same `Spell::prepare` that reads the request. Three seconds is
/// two orders of magnitude longer than that; the deadline only matters for the
/// paths that send no answer. See [`Casting::asked_at`] and
/// [`expire_the_ask`], the only reader.
const ASK_DEADLINE: Duration = Duration::from_secs(3);

/// Clear an ask the server never answered.
///
/// Two of `HandleCastSpellOpcode`'s branches return without a reply: an
/// unknown spell id, and a spell the character does not have active, which
/// includes a superseded rank left on an action button. One press could
/// therefore set the pending record permanently, and it then blocked every
/// later press: the "another action is in progress, stuck on the character"
/// report. See [`Casting::pending`] for the character it was measured on.
///
/// [`crate::world::incoming`] reads `SMSG_SUPERCEDED_SPELL` and
/// [`super::supersede`] repairs ranks already stale, so the bar should not
/// hold an old rank. This function is the fallback for any reply that is
/// still dropped: without it, one unanswered packet disables casting for the
/// rest of the session.
fn expire_the_ask(mut casting: ResMut<Casting>) {
    let Some(since) = casting.asked_at else { return };
    if since.elapsed() < ASK_DEADLINE {
        return;
    }
    casting.asked_at = None;
    if let Some(spell_id) = casting.pending.take() {
        warn!("no answer to the cast of spell {spell_id} — letting the button go");
    }
}

/// One spell's three timers, the same three the 1.12.1 client keeps per spell.
#[derive(Clone, Copy, Debug)]
struct Record {
    category: u32,
    gcd_category: u32,
    /// The spell's own recovery, and its category's.
    recovery: Option<(Instant, Duration)>,
    category_recovery: Option<(Instant, Duration)>,
    /// The global cooldown this cast started.
    gcd: Option<(Instant, Duration)>,
    /// Parked until `SMSG_COOLDOWN_EVENT` — the durations are known and the
    /// clocks have not started.
    parked: Option<Duration>,
}

/// What the player is waiting on, per spell.
#[derive(Resource, Default)]
pub struct Cooldowns(HashMap<u32, Record>);

impl Cooldowns {
    /// How long until `spell` can be cast, in seconds, and the full length of
    /// the timer that blocks it.
    ///
    /// The read checks every record, not just this spell's, because that is
    /// how a global cooldown works: one cast's GCD is stored on that spell's
    /// record, and it blocks this spell because both have the same
    /// `startRecoveryCategory`. The longest remaining timer wins.
    pub fn remaining(&self, spell: &SpellInfo) -> Option<(f32, f32)> {
        let now = Instant::now();
        let left = |timer: Option<(Instant, Duration)>| {
            timer.and_then(|(start, duration)| {
                let ends = start + duration;
                (ends > now).then(|| {
                    (
                        (ends - now).as_secs_f32(),
                        duration.as_secs_f32().max(f32::EPSILON),
                    )
                })
            })
        };
        let mut longest: Option<(f32, f32)> = None;
        for (id, record) in &self.0 {
            let mine = *id == spell.id;
            let shared_category = spell.category != 0 && record.category == spell.category;
            let shared_gcd = spell.gcd_category != 0 && record.gcd_category == spell.gcd_category;
            let candidates = [
                (mine, left(record.recovery)),
                (mine || shared_category, left(record.category_recovery)),
                (mine || shared_gcd, left(record.gcd)),
            ];
            for (applies, timer) in candidates {
                if !applies {
                    continue;
                }
                if let Some(timer) = timer {
                    if longest.is_none_or(|(most, _)| timer.0 > most) {
                        longest = Some(timer);
                    }
                }
            }
        }
        longest
    }

    /// Whether this spell is ready to press.
    pub fn ready(&self, spell: &SpellInfo) -> bool {
        self.remaining(spell).is_none()
    }

    fn entry(&mut self, spell: &SpellInfo) -> &mut Record {
        self.0.entry(spell.id).or_insert(Record {
            category: spell.category,
            gcd_category: spell.gcd_category,
            recovery: None,
            category_recovery: None,
            gcd: None,
            parked: None,
        })
    }

    /// Start the global cooldown, locally, when the cast is sent. See the
    /// module comment.
    fn start_gcd(&mut self, spell: &SpellInfo) {
        if spell.gcd_ms == 0 {
            return;
        }
        let gcd = Some((Instant::now(), Duration::from_millis(u64::from(spell.gcd_ms))));
        self.entry(spell).gcd = gcd;
    }

    /// Start the spell's own recovery, when this character's `SMSG_SPELL_GO`
    /// arrives.
    ///
    /// For the few spells whose cooldown begins when the effect ends
    /// (`SPELL_ATTR_COOLDOWN_ON_EVENT`), the duration is parked instead of
    /// started.
    ///
    /// `pub(crate)` because the packet that triggers it is handled in another
    /// module: `world::incoming`'s `CastReleased` arm is the only caller, and
    /// it replaced a system in this file.
    ///
    /// `mods` holds the character's talents, many of which shorten these
    /// timers; see [`crate::world::spellmods`]. They apply to the spell's own
    /// recovery and not to the category's, because
    /// `SMSG_SET_*_SPELL_MODIFIER`'s `COOLDOWN` operation applies to the spell
    /// row, and the shared category belongs to the group.
    pub(crate) fn start_recovery(
        &mut self,
        spell: &SpellInfo,
        mods: &crate::world::spellmods::SpellMods,
    ) {
        let now = Instant::now();
        let own = Duration::from_millis(u64::from(mods.apply_u32(
            vale_protocol::play::spells::spell_mod_op::COOLDOWN,
            spell.spell_family_flags,
            spell.recovery_ms,
        )));
        let shared = Duration::from_millis(u64::from(spell.category_recovery_ms));
        let parked = spell.cooldown_on_event();
        let record = self.entry(spell);
        if parked {
            record.parked = Some(own);
            return;
        }
        if !own.is_zero() {
            record.recovery = Some((now, own));
        }
        if !shared.is_zero() {
            record.category_recovery = Some((now, shared));
        }
    }

    /// A refused cast: clear the global cooldown it started and nothing else.
    pub(crate) fn clear_gcd(&mut self, spell_id: u32) {
        if let Some(record) = self.0.get_mut(&spell_id) {
            record.gcd = None;
        }
    }

    /// Do what [`Self::set`] does, unless the spell's own cooldown already has
    /// at least `ms` left. For a wait that must not shorten a longer one, which
    /// is what an item's equip cooldown is beside the cooldown of its last use.
    pub(crate) fn set_at_least(&mut self, spell_id: u32, ms: u32, spell: Option<&SpellInfo>) {
        let wanted = Duration::from_millis(u64::from(ms));
        let longer = self.0.get(&spell_id).and_then(|record| record.recovery).is_some_and(
            |(start, length)| length.saturating_sub(start.elapsed()) >= wanted,
        );
        if !longer {
            self.set(spell_id, ms, spell);
        }
    }

    /// Apply the server's statement of a cooldown, which overrides any
    /// computed value.
    ///
    /// The categories are filled in when they are known. A record is created
    /// by whichever of the two paths reaches the spell first, and
    /// [`Self::entry`] uses `or_insert`, so a record this function created with
    /// `category: 0` used to stay without a category permanently. That spell's
    /// category cooldown then blocked nothing else in its category. A school
    /// lockout names every spell of the school at once, so one interrupt
    /// removed the categories from a whole school's records for the rest of the
    /// session. That was the second cause of the "school cooldowns display
    /// inconsistently" report, and the one that lasted beyond the lockout.
    ///
    /// `spell` is `None` for a spell the catalogue does not carry. The timer is
    /// still kept and only the grouping is lost.
    pub(crate) fn set(&mut self, spell_id: u32, ms: u32, spell: Option<&SpellInfo>) {
        let timer = Some((Instant::now(), Duration::from_millis(u64::from(ms))));
        let record = self.0.entry(spell_id).or_insert(Record {
            category: 0,
            gcd_category: 0,
            recovery: None,
            category_recovery: None,
            gcd: None,
            parked: None,
        });
        // Filled in on an existing record too, because that record may be one
        // this function used to create without categories. This repairs old
        // records as well as setting new ones correctly.
        if let Some(spell) = spell {
            record.category = spell.category;
            record.gcd_category = spell.gcd_category;
        }
        record.recovery = timer;
        record.parked = None;
    }

    /// `SMSG_CLEAR_COOLDOWN`: this spell's timers have ended.
    ///
    /// All four are cleared, including the parked one. The server says the
    /// spell is ready, and a category or GCD timer left on the record would
    /// keep the cooldown animation running on every other spell that shares
    /// it. The record is kept rather than removed so its categories survive for
    /// the next cast; see [`Self::set`], where losing them caused a bug.
    pub(crate) fn clear(&mut self, spell_id: u32) {
        if let Some(record) = self.0.get_mut(&spell_id) {
            record.recovery = None;
            record.category_recovery = None;
            record.gcd = None;
            record.parked = None;
        }
    }

    /// Release a parked cooldown (`SMSG_COOLDOWN_EVENT`).
    pub(crate) fn release(&mut self, spell_id: u32) {
        if let Some(record) = self.0.get_mut(&spell_id) {
            if let Some(duration) = record.parked.take() {
                record.recovery = Some((Instant::now(), duration));
            }
        }
    }
}

/// Every event this module raises, as one system parameter.
///
/// Bundled rather than listed per system because a verb writes three or four
/// of them, and the parameters made up most of each function signature. Bevy
/// runs two systems that hold this one after the other (a writer is a `ResMut`
/// underneath); they are chained anyway.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ActionEvents<'w> {
    pub slot_changed: MessageWriter<'w, ActionbarSlotChanged>,
    pub page: MessageWriter<'w, ActionbarPageChanged>,
    pub bonus: MessageWriter<'w, ActionbarBonusChanged>,
    pub cooldown: MessageWriter<'w, ActionbarUpdateCooldown>,
    /// The spellbook's event for the same change. See [`Self::cooldown_moved`],
    /// the only writer of either.
    pub spell_cooldown: MessageWriter<'w, SpellUpdateCooldown>,
    pub state: MessageWriter<'w, ActionbarUpdateState>,
    pub autorepeat_start: MessageWriter<'w, StartAutorepeatSpell>,
    pub autorepeat_stop: MessageWriter<'w, StopAutorepeatSpell>,
    pub cast_start: MessageWriter<'w, SpellcastStart>,
    pub cast_stop: MessageWriter<'w, SpellcastStop>,
    pub cast_failed: MessageWriter<'w, SpellcastFailed>,
    pub cast_interrupted: MessageWriter<'w, SpellcastInterrupted>,
    pub cast_delayed: MessageWriter<'w, SpellcastDelayed>,
    pub channel_start: MessageWriter<'w, SpellcastChannelStart>,
    pub channel_update: MessageWriter<'w, SpellcastChannelUpdate>,
    pub channel_stop: MessageWriter<'w, SpellcastChannelStop>,
}

/// Events that are always raised together.
impl ActionEvents<'_> {
    /// A timer changed. 1.12 announces this under two event names.
    ///
    /// Action buttons listen for `ACTIONBAR_UPDATE_COOLDOWN`, and spell buttons
    /// listen for `SPELL_UPDATE_COOLDOWN`: `SpellButton_OnLoad` registers it
    /// (`SpellBookFrame.lua:209`) and `SpellButton_OnEvent` answers it with
    /// `SpellButton_UpdateButton()`, the only function in that file that
    /// re-reads a cooldown. When this client raised only the first event, the
    /// spellbook's cooldown animations never moved. The book showed the right
    /// state after it was closed and reopened or its page turned, because those
    /// call `SpellBookFrame_Update`, which rebuilds every button.
    ///
    /// This is one method rather than two lines at each of the six call sites,
    /// because a missing line at one site produces no error.
    /// `SPELL_UPDATE_COOLDOWN` has always been listed in
    /// [`super::events::FIRED`], so `--audit --events` fires it and every count
    /// reports the name as handled.
    pub(crate) fn cooldown_moved(&mut self) {
        self.cooldown.write(ActionbarUpdateCooldown);
        self.spell_cooldown.write(SpellUpdateCooldown);
    }

    /// The same call, for callers outside this module. See
    /// [`begin_item_cast`], the only caller.
    pub(super) fn timer_moved(&mut self) {
        self.cooldown_moved();
    }
}

/// Start the global cooldown for an item use that casts a spell, as pressing
/// the spell would.
///
/// The item functions in [`super::items`] send `CMSG_USE_ITEM` and change
/// nothing else in the session, so a potion's cooldown animation did not start
/// until the server's `SMSG_SPELL_COOLDOWN` arrived a round trip later. The
/// global cooldown is the one local step of [`send_cast`] that any other way of
/// starting a cast must also take, so it is one function here rather than a
/// copy in the item code.
///
/// This no longer starts the cast bar. It used to start the bar and raise
/// `SPELLCAST_START` for a bandage; that now happens where it does for every
/// other cast, in [`PlayerEvent::CastStarted`]. The server sends
/// `SMSG_SPELL_START` for an item's spell like any other non-triggered cast,
/// so the bar appears on its own, and only if the server accepted the item.
pub(crate) fn begin_item_cast(
    info: &SpellInfo,
    cooldowns: &mut Cooldowns,
    events: &mut ActionEvents,
) {
    cooldowns.start_gcd(info);
    events.timer_moved();
}

/// This module's system chain, so that systems feeding it can be ordered
/// against the whole chain rather than against whichever system they read from.
///
/// [`super::spellbook`] is the current user: the book must be up to date before
/// a `CastSpell` row is resolved against it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActionSet;

pub struct ActionPlugin;

impl Plugin for ActionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionBar>()
            .init_resource::<Cooldowns>()
            .init_resource::<Casting>()
            .init_resource::<AutoRepeat>()
            .init_resource::<SpellTargeting>()
            .add_message::<SpellTargetPicked>()
            .add_message::<SpellItemPicked>()
            .add_message::<SpellGroundPicked>()
            .add_systems(
                Update,
                (
                    // The form before the bar, so that `UPDATE_BONUS_ACTIONBAR`
                    // arrives before the slot updates that follow it:
                    // `BonusActionBar_OnEvent` shows the frame, and the buttons in
                    // it then read their own slots.
                    follow_bonus_bar,
                    // Copies which extra bars are on screen from the character's
                    // record. It needs no ordering: nothing reads it until the
                    // interface asks, and the interface asks once. See
                    // [`follow_bar_toggles`].
                    follow_bar_toggles,
                    // Tracks the character state that decides whether a button
                    // is faded. See [`follow_usability`]; the two systems above
                    // decide which bar is on screen.
                    follow_usability,
                    // Tracks the server's attack state. See
                    // [`follow_attack_state`]; every press-side write of the same
                    // event happens in a verb later in this chain.
                    follow_attack_state,
                    // Tracks this client's own unfinished press. See
                    // [`follow_cast_state`].
                    follow_cast_state,
                    rebuild_bar,
                    // Events before input, so that a cooldown the server just
                    // stated is in place before this frame's press is checked.
                    crate::world::incoming::drain_events,
                    // After the drain, because all four replies that clear an
                    // ask arrive through it: expiring first could expire an ask
                    // whose reply is already queued.
                    expire_the_ask,
                    // `cancel_cast` was removed from this list. It was a system
                    // that read `just_pressed(Escape)`, but Escape is bound to
                    // `TOGGLEGAMEMENU` in the game's defaults, so the key did two
                    // things and could not be rebound away from either. Casts
                    // are now cancelled by `SpellStopCasting()`, an arm of
                    // [`run_bindings`] below.
                    run_bindings,
                )
                    .chain()
                    // After the whole targeting chain, because a cast binds
                    // against the selection and a swing goes at it. Run before
                    // it, a press would use the previous frame's target. See
                    // `target::TargetSet`.
                    .after(super::target::TargetSet)
                    // After the key table has turned this frame's keys into
                    // binding names: `run_bindings` reads those as messages, and a
                    // message written later in the frame is read the next frame.
                    .after(BindingSet)
                    .in_set(ActionSet)
                    .in_set(super::GameSet),
            )
            .add_systems(Update, forget.in_set(super::GameSet));
    }
}

/// Reset the character's bar, cast and cooldowns when the character leaves the
/// world.
///
/// All three belong to that character, and logging in as another character
/// does not correct them: `rebuild_bar` re-resolves the slots only when the
/// spellbook version differs, and a cooldown is an `Instant` on a clock that
/// keeps running. Without this, a mage's twelve buttons stayed on screen
/// behind the character-select list, and a spell on a two-minute cooldown at
/// logout was still greyed out for the next character holding that slot. See
/// [`super::events::PlayerLeavingWorld`].
fn forget(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut bar: ResMut<ActionBar>,
    mut cooldowns: ResMut<Cooldowns>,
    mut casting: ResMut<Casting>,
    mut auto_repeat: ResMut<AutoRepeat>,
    mut targeting: ResMut<SpellTargeting>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    *bar = ActionBar::default();
    *cooldowns = Cooldowns::default();
    *casting = Casting::default();
    // Reset the auto-repeat loop too: its `SMSG_CANCEL_AUTO_REPEAT` will not
    // arrive, because the socket it would come on has closed. Otherwise a
    // hunter who logs out while shooting logs back in with the button flashing.
    *auto_repeat = AutoRepeat::default();
    // Clear the cursor too, or the next character logs in holding a spell the
    // previous one pressed.
    targeting.stop();
}

/// Which twelve of the 120 slots the bar shows, based on the character's form.
///
/// This implements `GetBonusBarOffset()`. It is a system rather than a read at
/// the call site because of `UPDATE_BONUS_ACTIONBAR`: the interface does not
/// poll the offset. `BonusActionBarFrame` is hidden at load; its `OnLoad` asks
/// once, before a character exists, and after that it changes only on the
/// event. `ActionButtonUp` sends the press through whichever of the two frames
/// is shown. A client that never raises the event has a bonus bar that cannot
/// appear and twelve buttons reading page one, which is empty for a warrior or
/// a druid.
///
/// The event is raised on a change only, including the change from no
/// character back to zero: `forget` resets the bar, and the next login must
/// raise it again.
fn follow_bonus_bar(
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut bar: ResMut<ActionBar>,
    mut events: ActionEvents,
    // The form at the last check. Comparing the form rather than the offset
    // keeps the table lookup (a mutex and an `Arc` clone) out of most frames; a
    // form changes a few times per fight at most.
    mut seen: Local<Option<u8>>,
    // Re-announced when the interface loads. This client loads FrameXML a
    // second after login, so the first announcement of everything reaches no
    // listeners. `lua::host::load_bindings` raises `PLAYER_ENTERING_WORLD` when
    // the load finishes for this reason. Without re-announcing here,
    // `BonusActionBarFrame` stays as its `OnLoad` left it: shown, but at the
    // unslid `y = 0` under the main bar.
    mut entering: MessageReader<super::events::PlayerEnteringWorld>,
) {
    let entered = entering.read().count() > 0;
    // `None` while there is no character, so that logging back in as the same
    // class raises the event again. `forget` clears the bar, and a plain `u8`
    // here would compare equal to the form the last session ended in, leaving
    // the bonus bar hidden for the whole new session.
    let Ok(me) = player.single() else {
        *seen = None;
        return;
    };
    let form = me.shapeshift_form;
    if *seen == Some(form) && !entered {
        return;
    }
    *seen = Some(form);
    let offset = assets
        .display_tables()
        .ok()
        .map_or(0, |tables| tables.shapeshift().bonus_bar(form));
    if bar.bonus_bar == offset && !entered {
        return;
    }
    // Nothing is rebuilt. A form changes which twelve of the 120 slots the
    // interface reads, not their contents, and `ActionButton_GetPagedID` in Lua
    // does that reading. A forced rebuild here was needed while the empty page
    // was filled from the spellbook; now it would cost 120 DBC lookups per
    // stance change for nothing.
    bar.bonus_bar = offset;
    events.bonus.write(ActionbarBonusChanged);
    // Also update the twelve buttons, which read their own slot rather than
    // being told it. `BonusActionBar_OnEvent` shows the frame but does not
    // refresh the buttons.
    events.slot_changed.write(ActionbarSlotChanged(ALL_SLOTS));
}

/// Which of the four extra bars are on, copied from the character's record.
///
/// This only copies one byte from `PLAYER_FIELD_BYTES` into
/// [`ActionBar::toggles`], where `GetActionBarToggles()` can answer it. Every
/// other decision about the four bars is in the game's own files:
/// `UIParent.lua` asks once per `PLAYER_ENTERING_WORLD`,
/// `MultiActionBar_Update` applies the four answers to four frames, and
/// `ActionButton_GetPagedID` derives that a `MultiBarBottomLeft` button is slot
/// `id + 60` from the name of its parent.
///
/// This client therefore provides exactly two things for the four bars: this
/// byte and the packet that changes it ([`Binding::SetActionBarToggles`]). It
/// provides no layout.
///
/// Unconditional rather than gated on a change, unlike [`follow_bonus_bar`]:
/// there is no event to raise and so nothing to suppress, and copying a `u8`
/// from a snapshot every frame costs less than the `Local` that would skip it.
/// `ActionBar::default()` gives the value with no character: all four bars
/// off, the same as a new account.
fn follow_bar_toggles(
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut bar: ResMut<ActionBar>,
) {
    let toggles = player.single().map_or(0, |me| me.action_bar_toggles);
    if bar.toggles != toggles {
        bar.toggles = toggles;
    }
}

/// Tell the bar when the server changes the attack state.
///
/// `ActionButton_UpdateState` draws the checked border. It reads
/// `IsCurrentAction or IsAutoRepeatAction`, and `ACTIONBAR_UPDATE_STATE` is the
/// only event that re-runs it. All eight writes of that event in this file used
/// to be inside a verb, so the border redrew only when the player clicked the
/// button, and then showed the state at that moment.
///
/// That one bug produced three symptoms, reported together:
///
/// ```text
/// right-click a mob to attack   SMSG_ATTACKSTART lands, nothing redraws
///                               -> the indicator does not come on
/// click the Attack button       the press raises the event, so the border
///                               draws the old state and lights up,
///                               while the click itself toggles the attack off,
///                               because attack_target correctly saw it was on
/// the target dies or clears     SMSG_ATTACKSTOP lands, nothing redraws
///                               -> the indicator stays lit
/// ```
///
/// The toggle in [`attack_target`] was correct; only the drawing was wrong.
///
/// This reads `live.attacking()`, not `WorldEntity::engaged()`. `engaged` is
/// `attacking == target_guid()`, so swinging at a unit that is no longer
/// selected reads as false there. `IsCurrentAction` answers from
/// `live.attacking()`, so the button and the event cannot disagree.
///
/// The auto-repeat side needs nothing here: every end of the loop already goes
/// through `AutoRepeat::stop`, which raises the state event together with
/// `STOP_AUTOREPEAT_SPELL`. See [`AutoRepeat`] for why the server's
/// `SMSG_CANCEL_AUTO_REPEAT` is the only notice on the wire.
fn follow_attack_state(
    session: Res<Session>,
    mut was: Local<Option<u64>>,
    mut state: MessageWriter<ActionbarUpdateState>,
    mut entered: MessageWriter<super::events::PlayerEnterCombat>,
    mut left: MessageWriter<super::events::PlayerLeaveCombat>,
) {
    let now = session
        .active
        .as_ref()
        .and_then(|active| active.live.attacking());
    if *was == now {
        return;
    }
    // Swinging at a different unit is not leaving combat, so the pair of events
    // is raised when an attack target appears or disappears, not when it
    // changes. A target swap mid-fight keeps the flash going, as in the 1.12.1
    // client.
    match (was.is_some(), now.is_some()) {
        (false, true) => {
            entered.write(super::events::PlayerEnterCombat);
        }
        (true, false) => {
            left.write(super::events::PlayerLeaveCombat);
        }
        _ => {}
    }
    // The border is redrawn on every change, including a swap.
    // `IsCurrentAction` is about the slot and gives the same answer after a
    // swap, but the extra message is cheap and removes a case to reason about.
    state.write(ActionbarUpdateState);
    *was = now;
}

/// Tell the bar when the unfinished press changes: the cast in flight, and the
/// cursor waiting for a target.
///
/// [`crate::interface::api::is_current_action`] answers from both. A border
/// raised only by the press that started it never goes out: a cast ends at
/// `SMSG_SPELL_GO`, an interruption or a refusal, and a cursor ends at a click,
/// an Escape or a spell that could not be aimed. That is six endings, and none
/// of them is the button being pressed again. One system watching the pair
/// replaces six writes, and a seventh ending added later is covered without a
/// change here.
///
/// Same structure as [`follow_attack_state`] and for the same reason: a
/// `Local` holding the last reported value, and one message on a difference.
fn follow_cast_state(
    casting: Res<Casting>,
    targeting: Res<SpellTargeting>,
    mut was: Local<Option<(Option<u32>, Option<u32>)>>,
    mut state: MessageWriter<ActionbarUpdateState>,
) {
    let now = (
        casting.started.map(|_| casting.spell_id),
        targeting.spell(),
    );
    if *was == Some(now) {
        return;
    }
    // Not on the first frame; the `Option` around the pair provides that. A
    // session opens with both empty, and raising the event then would re-run
    // every visible button's state on the frame the bar is built.
    if was.is_some() {
        state.write(ActionbarUpdateState);
    }
    *was = Some(now);
}

/// Tell the bar when the conditions for using a spell change, which is a
/// separate question from when the player's power changes.
///
/// `ACTIONBAR_UPDATE_USABLE` is the only event that re-runs
/// `ActionButton_UpdateUsable`, and `super::vitals` raises it when the
/// player's power changes. Power was the only input to the fade until
/// `SpellInfo::castable_now` was added. Three other inputs now affect it:
///
/// * the aura state: a Seal landing makes Judgement usable, and a block makes
///   Revenge usable;
/// * combo points: every finisher on a rogue's bar;
/// * the form: a warrior changing stance changes which half of their abilities
///   are usable.
///
/// Without this system a button keeps its old state until something else
/// raises the event, which in combat is the next power tick. The fade would be
/// correct most of the time and late the rest, which players report as
/// "sometimes it works".
///
/// A `Local` rather than change detection, because these fields live on a
/// `WorldEntity` that the poll rewrites every tick: `Changed` fires every tick
/// and carries no information.
fn follow_usability(
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut was: Local<Option<(u32, u8, u8)>>,
    mut usable: MessageWriter<super::events::ActionbarUpdateUsable>,
) {
    let now = player
        .single()
        .map(|me| (me.aura_state, me.combo_points, me.shapeshift_form))
        .ok();
    if *was == now {
        return;
    }
    // Not on the first value. It is recorded without raising the event, so
    // logging in does not run every registered button before the interface is
    // up.
    if was.is_some() && now.is_some() {
        usable.write(super::events::ActionbarUpdateUsable);
    }
    *was = now;
}

/// Resolve the server's bar against `Spell.dbc`, when either has changed.
fn rebuild_bar(
    session: Res<Session>,
    assets: Res<GameAssets>,
    mut bar: ResMut<ActionBar>,
    mut events: ActionEvents,
) {
    let Some(active) = session.active.as_ref() else {
        if bar.built_from.is_some() {
            *bar = ActionBar::default();
            events.slot_changed.write(ActionbarSlotChanged(ALL_SLOTS));
        }
        return;
    };
    // The cheap accessor, not `status()`: this runs every frame and the full
    // status clones a `Vec` of warnings and a map of unhandled opcodes.
    let version = active.live.spellbook_version();
    // Also check which parse of the DBCs the slots were resolved from. The
    // bar's names, icons and cast times come from `Spell.dbc`, so a bar built
    // before the tables were dropped and re-read is stale, and no packet
    // reports it; see `GameAssets::tables_generation`. Without this check an
    // edited spell kept its old icon on the bar until the slot itself changed,
    // which is what the server's version tracks.
    let parse = assets.tables_generation();
    if bar.built_from == Some(version) && bar.built_with == parse {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let Some(catalog) = tables.spellbook() else {
        return;
    };
    let (buttons, known) = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        (world.action_buttons.clone(), world.spellbook.known.clone())
    };

    bar.built_from = Some(version);
    bar.built_with = parse;
    bar.known = known
        .iter()
        .filter_map(|id| catalog.info(*id))
        // A passive is in the spellbook but cannot be cast; the server refuses
        // it. 4,929 of the game's spells are passive.
        //
        // A `DO_NOT_DISPLAY` spell is in neither list: the 1.12.1 client leaves
        // it out of its castable list, as it does out of the spellbook. Without
        // this filter the character's GM and world-buff spells appear in this
        // list with the icon `Interface\Icons\Temp`.
        .filter(|info| !info.is_passive() && !info.hidden())
        .collect();
    bar.known.sort_by(|a, b| a.name.cmp(&b.name));

    bar.slots = (0..BAR_SLOTS)
        .map(|slot| {
            buttons
                .iter()
                .find(|button| usize::from(button.slot) == slot)
                .map(|button| Slot {
                    action: button.action,
                    kind: button.kind,
                    spell: (button.kind == action_kind::SPELL)
                        .then(|| catalog.info(button.action))
                        .flatten(),
                })
        })
        .collect();

    // The bar shows `SMSG_ACTION_BUTTONS` and nothing else: the server's
    // `playercreateinfo_action` for a new character, and whatever the last
    // session left for an existing one. This matches the 1.12.1 client. Empty
    // slots stay empty; [`super::cursor`] carries a spell or an item onto a
    // button and `CMSG_SET_ACTION_BUTTON` stores it.

    // One message for the whole bar rather than twelve, using `ActionButton.lua`'s
    // `arg1 == 0` convention; see `events::ActionbarSlotChanged`.
    events.slot_changed.write(ActionbarSlotChanged(ALL_SLOTS));
}


// The cast bar ends in `crate::world::incoming`'s `PlayerEvent::CastReleased`
// arm, which runs per packet and carries its own spell id. It used to end
// here, in `finish_casts`, a system that polled the player entity's release
// counter and then read `last_spell` to find which spell had landed. A second
// release in the same poll replaced that value, and the character could not
// cast for the rest of the session. That arm documents the failure.

/// Cancel the wind-up, the pending ask or the channel: the C side of
/// `SpellStopCasting()`.
///
/// `Bindings.xml` binds Escape only to `TOGGLEGAMEMENU`, and that binding's
/// body calls `SpellStopCasting()`. Cancelling a cast and clearing a target are
/// both done by the client. The cast is cancelled before the target is
/// cleared, because a wind-up is the more urgent of the two, and
/// `target::select_on_click` checks `Casting::started` for the same reason
/// rather than relying on system order.
///
/// This is a function rather than a system: [`run_bindings`] calls it from an
/// arm, like every other write. It replaces a system that read
/// `just_pressed(Escape)` directly; because the game already binds that key,
/// the key did two things.
fn stop_casting(
    active: &crate::world::session::ActiveSession,
    casting: &mut Casting,
    events: &mut ActionEvents,
) {
    // A channel can be cancelled too, and it ends through its own event. See
    // `drain_events`' interrupt arm for why the ordinary stop would not show.
    let channelled = casting.channelling.is_some();
    // A cast that has been sent and not yet answered can also be cancelled.
    // Because the bar waits for `SMSG_SPELL_START`, there is a round trip in
    // which Escape would otherwise do nothing: the packet would arrive, the bar
    // would go up, and the keypress meant to stop it would already be spent.
    // The cancel follows the cast on the same socket, so the server receives
    // them in the order they were pressed.
    let pending = casting.pending.take();
    if !channelled && casting.started.is_none() && pending.is_none() {
        return;
    }
    active.live.cancel_cast(pending.unwrap_or(casting.spell_id));
    // The events follow the bar, not the keypress. `end` returns whether there
    // was a wind-up; a channel has none and stops through its own event either
    // way; a cancelled ask has neither, so it raises nothing. A stop over a bar
    // that never appeared would be coloured and faded by
    // `CastingBarFrame_OnEvent`.
    let had_bar = casting.end();
    if channelled {
        events.channel_stop.write(SpellcastChannelStop);
    } else if had_bar {
        events.cast_stop.write(SpellcastStop);
    }
}

/// What the player asked for this frame, from every source it can come from.
///
/// One parameter rather than several, for the same reason [`ActionEvents`] is
/// one: [`run_bindings`] is at Bevy's limit of sixteen, and these readers are
/// read next to each other at its start. The bindings and the spell cursor's
/// answers belong together, since each is the player pointing at something,
/// and they are cleared together when there is no character to act.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Asked<'w, 's> {
    pressed: MessageReader<'w, 's, BindingPressed>,
    /// The second source of a cast: a click rather than a key. See
    /// [`SpellTargetPicked`]. Read here rather than handled where it is written,
    /// so that every cast in the client still goes through one function.
    picked: MessageReader<'w, 's, SpellTargetPicked>,
    /// The third source: a bag slot. See [`SpellItemPicked`].
    item_picked: MessageReader<'w, 's, SpellItemPicked>,
    /// The fourth source: the trade window's enchant slot. See
    /// [`crate::interface::trade::TradeSlotPicked`]. The same gesture as the
    /// bag slot, aimed at a slot number rather than an item.
    trade_picked: MessageReader<'w, 's, crate::interface::trade::TradeSlotPicked>,
    /// The cursor put up by an item rather than by a press, such as a
    /// sharpening stone right-clicked in a bag. See
    /// [`crate::interface::items::BeginItemTargeting`], which is written where
    /// the stone's bag position is recorded.
    begin_item: MessageReader<'w, 's, crate::interface::items::BeginItemTargeting>,
    /// The click in the world with a location as the answer. See
    /// [`SpellGroundPicked`].
    placed: MessageReader<'w, 's, SpellGroundPicked>,
}

impl Asked<'_, '_> {
    /// Discard this frame's input, so that a binding pressed on the login
    /// screen does not fire on the first frame in the world.
    fn clear(&mut self) {
        self.pressed.clear();
        self.picked.clear();
        self.placed.clear();
    }
}

/// Run the bindings that fired this frame.
///
/// This is the entire input path for an action, and it names no key. The key
/// table turned a key press into a [`Binding`], and this function turns a
/// `Binding` into a verb: the same two steps `Bindings.xml` and the C API
/// take.
#[allow(clippy::too_many_arguments)]
fn run_bindings(
    mut asked: Asked,
    session: Res<Session>,
    assets: Res<GameAssets>,
    bar: Res<ActionBar>,
    book: Res<super::spellbook::Spellbook>,
    // Read, never written. It answers the one question `UseAction`'s
    // `checkCursor` asks, which lets this module handle its half of a verb
    // whose other half is in [`super::cursor`].
    cursor: Res<super::cursor::Cursor>,
    selection: Res<Selection>,
    // The caster's `Transform` as well as its record, because the range check
    // below measures drawn positions rather than the last snapshot. The local
    // player is dead-reckoned ahead of the session thread, and the difference
    // while running is a fifth of a yard per frame.
    player: Query<(Entity, &WorldEntity, &Transform, &Sheath), With<LocalPlayer>>,
    units: Query<(&WorldEntity, &Transform)>,
    // Resolves every unit token. Party frame clicks arrive as
    // `SpellTargetUnit("party2")`, and this is the only function in this
    // client that knows what that token names. See the arm below.
    tokens: crate::interface::api::Units,
    mut press: PressState,
    mut errors: UiErrors,
    mut events: ActionEvents,
    mut sheathing: MessageWriter<SheathRequest>,
    // Where an item slot's use is sent. See [`use_action`]; the line that
    // writes it explains why `interface::items` performs the use and this
    // module does not.
    mut items: MessageWriter<super::items::UseCarriedItem>,
    // The settings, the time of day and the sky, bundled. See
    // [`crate::interface::api::Surroundings`]: this system was at Bevy's
    // sixteen-parameter limit, and the settings were already a parameter. The
    // settings are read for one flag ([`self_cast`] below); the other two are
    // inputs to the cast rule and reach it through `caster_conditions`.
    around: crate::interface::api::Surroundings,
) {
    let Some(active) = session.active.as_ref() else {
        // Drain anyway: a binding pressed on the login screen must not fire on
        // the first frame in the world.
        asked.clear();
        return;
    };
    // Taken from a parameter this system already holds, rather than from two
    // more `Res` parameters. See [`crate::interface::api::Units::friendship`].
    let friendship = tokens.friendship();
    // The other inputs a press is checked against, from the same bundle as the
    // settings. See [`crate::interface::api::CastWorld`].
    let world = around.cast_world();
    let Ok((entity, me, here, sheath)) = player.single() else {
        asked.clear();
        return;
    };
    // `autoSelfCast` is the persistent form of the `SELFACTIONBUTTON` flag. The
    // binding sets it for one press; the CVar sets it for every press. The
    // 1.12.1 client treats the two identically, so they are or-ed together here
    // rather than passed through [`cast_known_spell`] as two bools: either one
    // being true is the condition.
    //
    // Read every frame rather than cached, because any script can write a CVar
    // between two presses, and the read is a hash lookup.
    let self_cast = around.cvars().flag(vale_assets::tables::spellbook::AUTO_SELF_CAST);
    // An item's targeting request is handled first. It is read before the
    // presses so that a cursor put up by an item is in place before anything
    // else this frame checks it, and drained either way so that it cannot fire
    // late.
    if let Some(begun) = asked.begin_item.read().last() {
        press.targeting.begin(begun.0, Asking::Item);
    }
    // The units, locations, items and trade slots the spell cursor was pointed
    // at, from each source: the world pick (`target::select_on_click`, as a
    // message), a bag or trade click (as a message), and a unit frame
    // (`SpellTargetUnit`, as a binding, collected in the loop below). They are
    // gathered first and cast last, so that a key pressed in the same frame,
    // which is a new intention, is handled before the answer to a question the
    // client asked.
    let mut pointed_at: Vec<Pointed> = asked
        .picked
        .read()
        .map(|p| Pointed::Unit(p.unit))
        .chain(asked.placed.read().map(|p| Pointed::Ground(p.at)))
        .chain(asked.item_picked.read().map(|p| Pointed::Item(p.guid)))
        .chain(asked.trade_picked.read().map(|p| Pointed::TradeSlot(p.trade_slot)))
        .collect();
    for BindingPressed(binding) in asked.pressed.read() {
        match binding {
            Binding::ActionButton(slot) => use_action(
                *slot, self_cast, active, &assets, &bar, &selection, &units, me, friendship, world, here, entity,
                &mut press.cooldowns, &mut press.casting, &mut press.auto_repeat, &mut press.targeting,
                        &mut errors, &mut events,
                &mut sheathing, &mut items,
            ),
            Binding::SelfActionButton(slot) => use_action(
                *slot, true, active, &assets, &bar, &selection, &units, me, friendship, world, here, entity,
                &mut press.cooldowns, &mut press.casting, &mut press.auto_repeat, &mut press.targeting,
                        &mut errors, &mut events,
                &mut sheathing, &mut items,
            ),
            // The click's half of `UseAction(slot, 1)`: the use, which applies
            // only when nothing is being carried. The other case belongs to
            // [`super::cursor`], which drops the carried item into the slot. The
            // two conditions are exclusive, so neither module needs to know the
            // other ran. See [`Binding::UseOrPlaceAction`], which lists the four
            // cases the 1.12.1 client distinguishes.
            Binding::UseOrPlaceAction(slot) => {
                if cursor.held.is_none() {
                    use_action(
                        *slot, self_cast, active, &assets, &bar, &selection, &units, me, friendship, world, here, entity,
                        &mut press.cooldowns, &mut press.casting, &mut press.auto_repeat, &mut press.targeting,
                        &mut errors, &mut events,
                        &mut sheathing, &mut items,
                    );
                }
            }
            Binding::AttackTarget => {
                attack_target(active, &selection, &units, me, entity, &mut sheathing);
                events.state.write(ActionbarUpdateState);
            }
            // A row of the book, not a spell id; see
            // [`Binding::CastSpellbookRow`]. A row the book does not have is
            // dropped without a message: the panel computed the index, so an
            // out-of-range row is an error in this client's arithmetic and not
            // something to report to the player.
            Binding::CastSpellbookRow(row) => {
                if let Some(info) = book.book.spell(usize::from(*row)) {
                    cast_known_spell(
                        info, self_cast, None, active, &assets, &selection, &units, me, friendship, world, here, entity,
                        &mut press.cooldowns, &mut press.casting, &mut press.auto_repeat, &mut press.targeting,
                        &mut errors, &mut events,
                        &mut sheathing,
                    );
                }
            }
            // A recipe, which is a spell id rather than a row; the panel resolved
            // it against the list it drew. The cast uses the ordinary path: a
            // potion targets the caster through the aiming rule, and an enchant
            // asks for an item the same way a formula cast from a bag does. The
            // repeat count is not handled here; the repeat counter in
            // [`super::tradeskill`] reads the same message.
            Binding::CastRecipe { spell, .. } => {
                let tables = assets.display_tables().ok();
                if let Some(info) = tables
                    .as_deref()
                    .and_then(|t| t.spellbook())
                    .and_then(|catalog| catalog.info(*spell))
                {
                    cast_known_spell(
                        &info, false, None, active, &assets, &selection, &units, me, friendship, world, here, entity,
                        &mut press.cooldowns, &mut press.casting, &mut press.auto_repeat, &mut press.targeting,
                        &mut errors, &mut events,
                        &mut sheathing,
                    );
                }
            }
            // The spell cursor, clicked through a unit frame rather than in the
            // world. `TargetFrame_OnClick` casts at `"target"` and a party frame
            // at its own unit; a click in the world arrives as
            // [`SpellTargetPicked`] above, and both end in the same place.
            //
            // Any token is resolved through one function; see
            // [`crate::interface::api::Units::resolve`].
            //
            // This arm used to resolve only `"player"` and `"target"`, the two
            // tokens available without the `Units` parameter; together they
            // cover every call site in FrameXML that a client with no party can
            // reach. That caused "positive click spells do not work on party
            // member frames": `PartyMemberFrame_OnClick` is
            // `if SpellIsTargeting() then SpellTargetUnit("party"..id)`, and a
            // `party1` that resolved to nothing discarded the click. The cursor
            // stayed up, no packet was sent, and no error was shown. The same
            // two lines are in `UnitFrame.lua`, `PartyMemberPetFrame_OnClick`
            // and `TargetFrame_OnClick`, so every unit frame in the game except
            // two was affected.
            //
            // `Units::resolve` already knows that `party3` is a guid in
            // [`crate::interface::party::Party`] and that `pet` resolves to the
            // charmed unit before the summoned pet. A partial copy of it here is
            // what caused the gap.
            Binding::SpellTargetUnit(token) => {
                if let Some(unit) = tokens.resolve(*token) {
                    pointed_at.push(Pointed::Unit(unit));
                }
            }
            Binding::SpellStopTargeting => press.targeting.stop(),
            // `SpellStopCasting()`, called from Escape's binding. It replaces
            // the former `just_pressed(Escape)` system. The interface decides
            // whether a cast is running (see `ActionAnswers::spell_is_casting`),
            // so by the time this arrives that question is answered; only the
            // cancel remains.
            Binding::SpellStopCasting => {
                stop_casting(active, &mut press.casting, &mut events);
            }
            Binding::ToggleSheath => toggle_sheath(entity, sheath, me, &mut sheathing),
            // Nothing to store: the page is the interface's
            // `CURRENT_ACTIONBAR_PAGE`, and every button adds it to its own id
            // before asking this client anything, so the twelve slot numbers
            // that arrive here are already absolute. The C side only raises the
            // event; see [`Binding::ChangeActionBarPage`].
            Binding::ChangeActionBarPage => {
                events.page.write(ActionbarPageChanged);
            }
            // The extra bars work the opposite way. The page belongs to the
            // interface and this client only announces it; the four extra bars
            // belong to the server, so the C side sends a packet and raises no
            // event. The options panel has already shown and hidden its frames
            // by the time this arrives. See [`Binding::SetActionBarToggles`] and
            // [`ActionBar::toggles`], where the answer comes back.
            Binding::SetActionBarToggles(mask) => {
                active.live.set_actionbar_toggles(*mask);
            }
            // Targeting verbs are handled in `target.rs` (including
            // `Binding::TargetToken`, the party frame's click), and the buff
            // cancel in `auras.rs`; each reads the same messages. Only the module
            // that holds the state a verb needs can resolve it; see
            // [`Binding::CancelPlayerBuff`], whose handle indexes a list this
            // module does not have.
            Binding::TargetNearestEnemy
            | Binding::TargetPreviousEnemy
            | Binding::TargetNearestFriend
            | Binding::TargetPreviousFriend
            | Binding::TargetLastEnemy
            | Binding::AssistUnit(_)
            | Binding::TargetSelf
            | Binding::TargetToken(_)
            | Binding::CancelPlayerBuff(_)
            // The four logout bindings belong to `interface::logout`, and so does
            // the instance reset, for the reason given in its arm there.
            | Binding::ResetInstances
            | Binding::Logout
            | Binding::Quit
            | Binding::CancelLogout
            | Binding::ForceQuit
            // `ReloadUI` belongs to `lua::host`, which rebuilds the interpreter
            // without touching the world.
            | Binding::ReloadUI
            // The five death bindings belong to `interface::death`.
            | Binding::RepopMe
            | Binding::RetrieveCorpse
            | Binding::AcceptResurrect
            | Binding::DeclineResurrect
            | Binding::AcceptXPLoss
            // The innkeeper's confirmation belongs to `interface::binder`,
            // because it needs the guid of the innkeeper who asked, and only that
            // module holds it. The pet trainer's Accept belongs to
            // `interface::untrainer` for the same reason.
            | Binding::ConfirmBinder
            | Binding::ConfirmPetUnlearn
            // The summon's Accept and `/played` belong to `interface::summon` and
            // `interface::played`.
            | Binding::ConfirmSummon
            | Binding::RequestTimePlayed
            // `/roll`, the raid target icons, quest sharing and the tutorial
            // tips belong to `interface::randomroll`, `interface::raidtarget`,
            // `interface::questshare` and `interface::tutorial`.
            | Binding::RandomRoll { .. }
            | Binding::SetRaidTarget { .. }
            | Binding::QuestLogPushQuest
            | Binding::ConfirmAcceptQuest
            | Binding::FlagTutorial(_)
            | Binding::ClearTutorials
            | Binding::ResetTutorials
            // The two options checkboxes belong to `interface::uioptions`.
            | Binding::ShowHelm(_)
            | Binding::ShowCloak(_)
            // The two item right-click bindings belong to `interface::items`: the
            // item's prototype decides whether the click uses or equips it, and
            // only that module holds the prototype.
            | Binding::UseContainerItem { .. }
            | Binding::UseInventoryItem(_)
            // The six item-carrying bindings belong to `interface::cursor` for the
            // same reason: a left click's meaning depends on whether the pointer
            // already holds something, and only that module knows.
            | Binding::PickupContainerItem { .. }
            | Binding::PickupInventoryItem(_)
            | Binding::SplitContainerItem { .. }
            | Binding::PutItemInContainer(_)
            | Binding::AutoEquipCursorItem
            | Binding::DeleteCursorItem
            // The four bar-filling bindings belong to the same module for the
            // same reason: `PickupAction` picks up or places depending on what is
            // already carried, and only the cursor knows. This module owns what a
            // slot does, not what is in it.
            | Binding::PickupSpellbookRow(_)
            | Binding::PickupAction(_)
            | Binding::PlaceAction(_)
            | Binding::ClearCursor
            // `ClearTarget` belongs to `combat::target`: this module owns what a
            // slot does, not who is selected.
            //
            // The character controls belong to `input::controls`: a movement key
            // is a held state that the mover reads every tick, and this module
            // handles edges. `Jump` is an edge but is not handled here either,
            // for the same reason `UseContainerItem` is not: the module that owns
            // the socket sends the packet.
            | Binding::ClearTarget
            | Binding::Control(_, _)
            | Binding::Jump
            | Binding::SitOrStand
            | Binding::ToggleAutoRun
            | Binding::ToggleRun
            | Binding::FollowUnit(_)
            // `CameraZoom` belongs to `world::camera`, and the window binding to
            // the crate root.
            | Binding::CameraZoom(_)
            // The pet bar is a different bar with a different packet; see
            // [`super::pet`], which drains the same queue.
            | Binding::CastPetAction(_)
            | Binding::TogglePetAutocast(_)
            // Dragging within the pet bar is dispatched by `combat::cursor` and
            // handled by `combat::pet`.
            | Binding::PickupPetAction(_)
            // The stance bar is a third bar; see [`super::shapeshift`], which
            // drains the same queue.
            | Binding::CastShapeshiftForm(_)
            | Binding::PetAttack
            | Binding::PetStopAttack
            | Binding::PetAbandon
            // The pointer query belongs to `combat::cursor`; see
            // [`super::cursor::Cursor::asked`].
            | Binding::AskCursor(_)
            | Binding::Screenshot => {}
        }
    }

    // The spell cursor's answers are handled last. The spell is looked up again
    // here rather than carried on the pick, because a press in the loop above
    // may have replaced or cancelled the waiting spell, and a stale id would
    // cast a spell the player has already moved on from.
    for pointed in pointed_at.drain(..) {
        let Some(info) = press
            .targeting
            .spell()
            .and_then(|id| assets.display_tables().ok()?.spellbook()?.info(id))
        else {
            press.targeting.stop();
            continue;
        };
        cast_known_spell(
            &info, self_cast, Some(pointed), active, &assets, &selection, &units, me, friendship, world, here, entity,
            &mut press.cooldowns, &mut press.casting, &mut press.auto_repeat, &mut press.targeting,
            &mut errors, &mut events, &mut sheathing,
        );
    }
}

/// `ToggleSheath()`: draw the equipped weapons, or put them away.
///
/// The equipment decides whether melee or ranged is drawn: a hunter with a bow
/// and no melee weapon draws the bow. Stowed is always stowed, so the cycle has
/// two states, and the choice is made only when drawing.
///
/// This sends a request rather than writing the state; see [`SheathRequest`],
/// and `world::entities::sheath` for why there is exactly one executor. This is
/// also the one path in the client that should play a draw or stow animation
/// and does not. That deviation is documented in `entities::sheath`, because
/// it belongs to the executor.
fn toggle_sheath(
    entity: Entity,
    sheath: &Sheath,
    me: &WorldEntity,
    sheathing: &mut MessageWriter<SheathRequest>,
) {
    use vale_assets::look::sheath as policy;
    let state = if sheath.state() == policy::UNARMED {
        // Melee first, as the 1.12.1 client prefers. Ranged is drawn only when
        // the character has nothing else to draw.
        if me.weapons[0].is_empty() && me.weapons[1].is_empty() && !me.weapons[2].is_empty() {
            policy::RANGED
        } else {
            policy::MELEE
        }
    } else {
        policy::UNARMED
    };
    sheathing.write(SheathRequest { entity, state });
}

/// `UseAction(slot, _, onSelf)`: the game's verb, with a one-based slot.
///
/// `on_self` is the game's `SELFACTIONBUTTON` flag or its persistent form, the
/// `autoSelfCast` CVar. [`run_bindings`] ors the two together because the
/// 1.12.1 client treats them identically. It is passed to [`resolve_aim`], so
/// holding Alt (or enabling the option) and pressing a heal with an enemy
/// selected heals the caster instead of failing.
#[allow(clippy::too_many_arguments)]
fn use_action(
    slot: u8,
    on_self: bool,
    active: &ActiveSession,
    assets: &GameAssets,
    bar: &ActionBar,
    selection: &Selection,
    units: &Query<(&WorldEntity, &Transform)>,
    me: &WorldEntity,
    // The group roster and the character's reputation, which are half of the
    // friend-or-foe decision and which the aiming rule needs; see
    // [`crate::interface::api::Friendship`]. Without them a spell aimed at a
    // free-for-all player of the same faction treats that player as a friend,
    // and the client refuses a cast the server would have allowed.
    friendship: crate::interface::api::Friendship<'_>,
    // The time of day and the sky, which are inputs to the caster-state
    // checks; see [`crate::interface::api::CastWorld`].
    world: crate::interface::api::CastWorld<'_>,
    here: &Transform,
    entity: Entity,
    cooldowns: &mut Cooldowns,
    casting: &mut Casting,
    auto_repeat: &mut AutoRepeat,
    targeting: &mut SpellTargeting,
    errors: &mut UiErrors,
    events: &mut ActionEvents,
    sheathing: &mut MessageWriter<SheathRequest>,
    items: &mut MessageWriter<super::items::UseCarriedItem>,
) {
    let Some(action) = api::action(bar, slot) else {
        return;
    };

    // The auto-attack toggle is detected by the slot's kind as well as the id:
    // an item whose entry happens to be 6603 is not the Attack button.
    // [`cast_known_spell`] makes the same decision by spell id alone; a slot
    // with no `SpellInfo` never reaches it.
    if action.is_auto_attack() {
        attack_target(active, selection, units, me, entity, sheathing);
        events.state.write(ActionbarUpdateState);
        return;
    }
    // An item is used, not cast. This module only passes on the item entry,
    // because `SMSG_ACTION_BUTTONS` carries nothing else. Finding the item is
    // the inventory's job, and using it is the right-click path; see
    // [`super::items::UseCarriedItem`]. That is why this is one line rather
    // than a second copy of the equip-or-use decision.
    //
    // Nothing happens when the character is not carrying the item, as in the
    // 1.12.1 client: `UseAction` finds nothing and returns, and the button has
    // no icon in the first place.
    if action.kind == action_kind::ITEM {
        items.write(super::items::UseCarriedItem(action.action));
        return;
    }
    let Some(info) = action.spell.as_ref() else {
        // A macro, or a spell `Spell.dbc` does not carry. Neither is
        // implemented, and neither should be sent without the checks.
        errors.key("SPELL_FAILED_SPELL_UNAVAILABLE");
        return;
    };
    cast_known_spell(
        info, on_self, None, active, assets, selection, units, me, friendship, world, here, entity,
        cooldowns, casting, auto_repeat, targeting, errors, events, sheathing,
    );
}

/// Cast a spell this character knows, from whichever path reached it.
///
/// Split out of [`use_action`] when the spellbook panel was added, because
/// `CastSpell(id, bookType)` leads to the same three steps: the cooldown
/// refusal, the aiming rule and the send. The only difference between a bar
/// press and a spellbook click is how the [`SpellInfo`] was found. Two copies
/// of the aiming call would be two places where a self-buff could start
/// sending the selection, which is the failure [`resolve_aim`] exists to
/// prevent and which looks like a server bug.
///
/// ## Two presses the server answers with no reply
///
/// `SpellButton_OnClick` calls `CastSpell(id, bookType)` for every row pressed.
/// It has no passive check and no attack check, so both belong here, on the C
/// side, as in the 1.12.1 client. A mistake here produces no refusal the
/// player can read, because `WorldSession::HandleCastSpellOpcode` drops the
/// packet without replying:
///
/// ```cpp
/// if (!_player->HasActiveSpell(spellId) || spellInfo->IsPassiveSpell())
/// {
///     sLog.Out(... "casts spell %u which he shouldn't have" ...);
///     recvPacket.rpos(recvPacket.wpos());
///     return;                       // no SMSG_CAST_RESULT, of either kind
/// }
/// ```
///
/// While this client drew the cast at the press (see [`send_cast`]), the only
/// visible result of clicking a passive was a cast animation followed by
/// nothing, with no error line and nothing in any log. 42 of this test
/// character's 137 spells are passive.
///
/// Attack has the same result for a different reason. Spell 6603 is not
/// passive and is in the book, so it passes the check above and the server
/// tries to cast it. The 1.12.1 client never sends it, because pressing Attack
/// is a swing (`CMSG_ATTACKSWING`). [`use_action`] has always handled this for
/// the bar. The spellbook's General page carries the same pseudo-spell and had
/// no such check, so clicking it there wound up, released and did nothing.
///
/// ## The click from the targeting cursor
///
/// `pointed` is what the player pointed at with the targeting cursor (a unit,
/// an item, a trade slot or a location), and it replaces the selection for
/// this press. It is `Some` only when returning from [`SpellTargeting`], and it
/// changes two things: the candidate offered to the aiming rule, and the
/// meaning of an answer that cannot be bound. The cursor has already been
/// shown once, so a second `WantsTarget` means the player pointed at something
/// the spell cannot target, and the result is the client's "Invalid target"
/// rather than another request.
#[allow(clippy::too_many_arguments)]
fn cast_known_spell(
    info: &SpellInfo,
    on_self: bool,
    pointed: Option<Pointed>,
    active: &ActiveSession,
    assets: &GameAssets,
    selection: &Selection,
    units: &Query<(&WorldEntity, &Transform)>,
    me: &WorldEntity,
    // The same pair [`use_action`] carries, for the same reason.
    friendship: crate::interface::api::Friendship<'_>,
    // The time of day and the sky, which are inputs to the caster-state
    // checks; see [`crate::interface::api::CastWorld`].
    world: crate::interface::api::CastWorld<'_>,
    here: &Transform,
    entity: Entity,
    cooldowns: &mut Cooldowns,
    casting: &mut Casting,
    auto_repeat: &mut AutoRepeat,
    targeting: &mut SpellTargeting,
    errors: &mut UiErrors,
    events: &mut ActionEvents,
    sheathing: &mut MessageWriter<SheathRequest>,
) {
    // Pressing an auto-repeat while it is running stops it; that is all the
    // second press does. This check comes before every check below (cooldown,
    // aiming rule, range) because none of them applies to turning something
    // off, and a hunter whose target has moved away would otherwise get "out of
    // range" instead of stopping.
    //
    // Nothing is cleared here. The server answers
    // `CMSG_CANCEL_AUTO_REPEAT_SPELL` with `SMSG_CANCEL_AUTO_REPEAT`, and
    // [`AutoRepeat::stop`] runs on that, so the press and the four ends the
    // client cannot predict share one code path. See [`AutoRepeat`].
    if info.is_auto_repeat_ranged() && auto_repeat.is(info.id) {
        active.live.cancel_auto_repeat();
        return;
    }
    match press_kind(info) {
        // Attack is a swing wherever it is pressed from; see the doc comment
        // above.
        PressKind::Swing => {
            attack_target(active, selection, units, me, entity, sheathing);
            events.state.write(ActionbarUpdateState);
            return;
        }
        // Refused without a message, as in the 1.12.1 client: it draws a
        // passive's button greyed, and pressing it produces no message, sound or
        // animation. `GlobalStrings.lua` has no key for "that spell is passive",
        // and the server would not send one either; it drops the packet.
        PressKind::Refused => return,
        PressKind::Cast => {}
    }

    // A press while a cast is already running. `Spell::prepare` starts with
    // `IsNonMeleeSpellCasted(false, true, true)` and answers
    // `SPELL_FAILED_SPELL_IN_PROGRESS`, so sending the packet achieves nothing.
    //
    // Repeating the spell that is already being cast is dropped without a
    // message. This is a design choice, not observed behaviour: the server
    // would answer with an error, but pressing the key again is the player
    // repeating the same request, not asking for a second action, and the
    // 1.12.1 client does not show an error line per keypress. A different spell
    // gets the server's message, because that action really is refused and the
    // player needs the reason.
    //
    // "Already running" includes "already sent and unanswered", which stops
    // repeated sends. Because a cast is not drawn until `SMSG_SPELL_START`
    // arrives, there is a window between sending and the answer in which
    // `in_progress` is false, and every repeated press would send another
    // `CMSG_CAST_SPELL`. The 1.12.1 client does send those and lets the server
    // refuse each one; dropping the repeat here gives the player the same
    // result with one packet instead of ten.
    if casting.pending == Some(info.id) {
        return;
    }
    if in_progress(casting) || casting.pending.is_some() {
        if casting.spell_id != info.id {
            errors.key("SPELL_FAILED_SPELL_IN_PROGRESS");
        }
        return;
    }

    // A press while the button is still recovering is refused locally, with the
    // client's message. Otherwise the server would send
    // `SPELL_FAILED_NOT_READY` a round trip later.
    if !cooldowns.ready(info) {
        let key = vale_assets::tables::spellbook::failure_override("SPELL_FAILED_NOT_READY", 0)
            .unwrap_or("SPELL_FAILED_NOT_READY");
        errors.key(key);
        return;
    }

    let tables = assets.display_tables().ok();
    let reaction = |unit: &WorldEntity| {
        tables
            .as_deref()
            .map_or(Reaction::Neutral, |t| friendship.reaction(t, me, unit))
    };
    let candidate = |unit: &WorldEntity| Candidate {
        guid: unit.guid,
        is_self: unit.guid == me.guid,
        reaction: reaction(unit),
        unit_flags: unit.unit_flags,
        dead: unit.dead,
    };
    // The candidate offered to the aiming rule: the unit the player pointed at
    // if there is one, otherwise the selection. A click with the targeting
    // cursor up does not change the selection (it does not in the 1.12.1 client
    // either), so this replaces the target for this press only and writes
    // nothing.
    let selected = match pointed {
        Some(Pointed::Unit(unit)) => Some(unit),
        // A location, an item or a trade slot is not a candidate for the unit
        // binder, and must not hide the selection either: the ground and item
        // branches below run before the selection is tried, so this offers only
        // what the aiming rule would have seen on the original press.
        Some(Pointed::Ground(_)) | Some(Pointed::Item(_)) | Some(Pointed::TradeSlot(_)) | None => {
            selection.entity
        }
    }
    .and_then(|entity| units.get(entity).ok())
    .map(|(unit, _)| candidate(unit));
    let myself = Some(Candidate {
        guid: me.guid,
        is_self: true,
        reaction: Reaction::Friendly,
        unit_flags: 0,
        dead: me.dead,
    });

    // `on_self` is the CVar or the binding; see [`run_bindings`]. With neither,
    // a friendly cast with an enemy selected is refused rather than redirected
    // to the caster, which is 1.12's default (`autoSelfCast` registers as
    // `"0"`).
    //
    // The client chooses the main-hand weapon itself rather than asking. See
    // `CastAim::Item`: a spell with `SPELL_ATTR_HELD_ITEM_ONLY` targets the
    // weapon and fails with "Your weapon hand is empty" when there is none.
    let aim = resolve_aim(info, selected, myself, on_self, me.main_hand_item);
    let target = match aim {
        CastAim::SelfImplicit => CastTarget::SelfImplicit,
        CastAim::Unit(guid) => CastTarget::Unit(guid),
        // Nothing bound, so the client asks instead of failing. This handles a
        // heal pressed with nothing selected: the 1.12.1 client shows the
        // targeting cursor and waits for a click, and this client printing "No
        // target" instead was the reported bug. See [`SpellTargeting`]. Once the
        // player has already been asked, the cursor is put away and the answer
        // becomes a refusal, as `aim_at` describes.
        CastAim::WantsTarget if pointed.is_none() => {
            targeting.begin(info.id, Asking::Unit);
            return;
        }
        CastAim::WantsTarget => {
            targeting.stop();
            errors.key(vale_assets::tables::spellbook::INVALID_TARGET);
            events.cast_failed.write(SpellcastFailed);
            return;
        }
        // The same question about a location. The answer is three floats and no
        // guid; see [`CastTarget::Dest`]. A unit is not a valid answer for a
        // placed spell: `SpellTargetUnit` through a unit frame arrives here as
        // `Pointed::Unit`, and the correct response is to keep asking rather
        // than send a cast the server would place at the caster's feet.
        CastAim::WantsGround => match pointed {
            Some(Pointed::Ground(at)) => CastTarget::Dest(at),
            _ => {
                targeting.begin(info.id, Asking::Ground);
                return;
            }
        },
        // `TARGET_FLAG_ITEM` with the weapon's guid, and no cursor. The 1.12.1
        // client chooses the main hand itself for every spell with the held-item
        // bit: Rockbiter, Windfury, the poisons and the sharpening stones.
        // Before this branch existed, all of them failed with "Invalid target",
        // because the target word fell past `UNIT_FAMILY` into the local
        // refusal.
        CastAim::Item(guid) => CastTarget::Item(guid),
        // The remaining case: an enchanting formula, a poison or a sharpening
        // stone, which 1.12 aims by clicking a bag slot (or the trade window's
        // enchant slot). The answer comes back through [`SpellItemPicked`]. Same
        // structure as the unit cursor: ask once, and refuse the second time,
        // because that means the player pointed at something the spell cannot
        // target.
        CastAim::WantsItem => match pointed {
            Some(Pointed::Item(guid)) => CastTarget::Item(guid),
            Some(Pointed::TradeSlot(slot)) => CastTarget::TradeSlot(slot),
            None => {
                targeting.begin(info.id, Asking::Item);
                return;
            }
            _ => {
                targeting.stop();
                errors.key(vale_assets::tables::spellbook::INVALID_TARGET);
                events.cast_failed.write(SpellcastFailed);
                return;
            }
        },
        // Refused before it reaches the socket, with the client's message.
        CastAim::Refused(key) => {
            targeting.stop();
            errors.key(key);
            events.cast_failed.write(SpellcastFailed);
            return;
        }
    };
    // The cursor's spell is used up: the press that reached this point is
    // either the answer it was waiting for or a new press that replaces it.
    targeting.stop();

    // The conditions the aiming rule does not cover: whether the caster is
    // alive, can pay the cost, and is within range of the unit. See
    // [`vale_assets::tables::spellbook::check_cast`], which holds the rule; this
    // code only measures. The distance is measured surface to surface, because
    // the server's range check uses that measurement.
    let distance = match target {
        CastTarget::Unit(guid) if guid != me.guid => reach_between(units, me, here, guid),
        // A placed cast measures from the caster's centre to the point. vmangos'
        // `CheckRange` ends in a plain `IsWithinDist3d` against the destination
        // with no reach subtracted at either end. This is a different
        // measurement from the unit branch above, not a special case of it.
        CastTarget::Dest(at) => {
            Some(here.translation.distance(crate::render::axes::to_bevy(at)))
        }
        _ => None,
    };
    // Whether the bound unit is alive, the one condition the aiming rule cannot
    // answer: `unsatisfied` tests relations, and a corpse is still hostile. Read
    // from the same two candidates the binder was given, so the state checked
    // here is the state it bound. A guid that matches neither (nothing can
    // currently produce one) checks nothing, which favours sending.
    let target_dead = match target {
        CastTarget::Unit(guid) if guid == me.guid => Some(me.dead),
        CastTarget::Unit(guid) => selected.filter(|who| who.guid == guid).map(|who| who.dead),
        _ => None,
    };
    // The caster checks are one function for both paths; see
    // [`super::api::caster_conditions`], where the item path and this one were
    // made consistent.
    let conditions = super::api::caster_conditions(me, info, distance, target_dead, world);
    if let Some(key) = vale_assets::tables::spellbook::check_cast(info, &conditions) {
        let key = vale_assets::tables::spellbook::failure_override(key, info.power_type).unwrap_or(key);
        errors.key(key);
        events.cast_failed.write(SpellcastFailed);
        return;
    }

    // A next-swing spell is queued, not cast: the packet is sent and no
    // animation plays, because the swing that releases it has not happened yet.
    // See [`LiveSession::cast_on_next_swing`].
    if info.on_next_swing() {
        // Pressing the armed ability again is dropped, for the same reason as
        // the `in_progress` check above: the player is repeating the request,
        // not asking for a second action. This state also depends on it:
        // vmangos' `SetCurrentCastedSpell` interrupts the queued spell before it
        // accepts the new one, so a second press sends
        // `SPELL_FAILED_INTERRUPTED` for the same spell id, and the clear would
        // turn off the border of an ability that is still armed.
        if casting.next_swing == Some(info.id) {
            return;
        }
        active.live.cast(info.id, target);
        // Armed at the press. This state belongs to the client, unlike the cast
        // bar, which waits for `SMSG_SPELL_START`. The 1.12.1 client marks the
        // ability as queued when it sends the packet, and no packet reports it
        // afterwards: the server's next message about it is the `SMSG_SPELL_GO`
        // when the weapon lands, which clears it.
        casting.next_swing = Some(info.id);
        cooldowns.start_gcd(info);
        events.cooldown_moved();
        events.state.write(ActionbarUpdateState);
        return;
    }

    // A ranged ability uses the ranged slot, so the bow is drawn with it. This
    // is the same explicit request [`attack_target`] makes for melee, for the
    // same reason. The per-animation reconcile would draw it eventually from the
    // shot's `AnimationData` flags, but only after a shot had been drawn, and it
    // never draws a ranged weapon, because `vale_assets::look::sheath::reconcile`
    // has no case that returns `RANGED`. Its ranged exemption applies to a state
    // only a request can set, and before this nothing made that request, so the
    // ranged half of the policy was unreachable.
    if info.uses_ranged_slot() {
        sheathing.write(SheathRequest {
            entity,
            state: vale_assets::look::sheath::RANGED,
        });
    }

    // An auto-repeat's press fires nothing, like every other press.
    // `Spell::update`'s `PREPARING` arm does not
    // `cast()` an auto-repeat when its timer runs out, so the first arrow leaves
    // on the ranged attack timer, which this client does not own and cannot
    // predict, and every later arrow is a new triggered cast. The only local
    // state is that the loop is running, which no packet reports.
    if info.is_auto_repeat_ranged() {
        active.live.cast(info.id, target);
        auto_repeat.begin(info.id, events);
        return;
    }
    send_cast(active, info, target, cooldowns, casting, events);
}

/// The distance between two units minus both their combat reach: vmangos'
/// `GetCombatDistance`, the distance `Spell::CheckRange` uses.
///
/// `None` when either unit has no `Transform` this frame, which means it has
/// not been placed yet. A missing measurement must mean "do not refuse", not
/// zero or infinity.
fn reach_between(
    units: &Query<(&WorldEntity, &Transform)>,
    me: &WorldEntity,
    here: &Transform,
    target: u64,
) -> Option<f32> {
    let (other, there) = units.iter().find(|(unit, _)| unit.guid == target)?;
    let centres = here.translation.distance(there.translation);
    Some((centres - me.combat_reach - other.combat_reach).max(0.0))
}

/// What pressing a known spell does, before any cooldown, aiming rule or
/// socket is involved.
///
/// Three outcomes rather than two, because the two that are not casts are
/// unrelated: one is a different verb, and the other does nothing. Named and
/// tested rather than written as two `if`s in [`cast_known_spell`], because
/// these two cases were the first two bug reports in this area, and because the
/// server gives no reply for either, so nothing downstream would show which
/// branch was taken.
#[derive(Debug, PartialEq, Eq)]
enum PressKind {
    /// The auto-attack pseudo-spell: `CMSG_ATTACKSWING`, not `CMSG_CAST_SPELL`.
    Swing,
    /// A passive. The server drops the packet without replying
    /// (`HandleCastSpellOpcode`'s `IsPassiveSpell()` branch), so a client that
    /// sends one shows a cast that never completes.
    Refused,
    Cast,
}

fn press_kind(info: &SpellInfo) -> PressKind {
    if info.id == SPELL_ATTACK {
        PressKind::Swing
    } else if info.is_passive() {
        PressKind::Refused
    } else {
        PressKind::Cast
    }
}

/// Send the cast request, start the global cooldown, and draw nothing.
///
/// These are the three parts of a press in build 5875. The 1.12.1 client runs its local refusals, records the
/// spell and its targets as pending, starts the global cooldown as the cast is
/// sent, and sends the packet. It starts no bar, plays no wind-up and changes
/// no counter: it raises `SPELLCAST_START` only on receiving
/// `SMSG_SPELL_START`.
///
/// The global cooldown belongs to the client, and both sides confirm it. The
/// 1.12.1 client takes its length from `Spell.dbc`'s `StartRecoveryTime`,
/// applies `SPELLMOD_GLOBAL_COOLDOWN` (21) and raises `SPELL_UPDATE_COOLDOWN`;
/// vmangos' `Spell::prepare` comments
/// `// add gcd server side (client side is handled by client itself)`. The
/// cooldown animation therefore starts at the press while everything else
/// waits, as in the 1.12.1 client.
fn send_cast(
    active: &ActiveSession,
    info: &SpellInfo,
    target: CastTarget,
    cooldowns: &mut Cooldowns,
    casting: &mut Casting,
    events: &mut ActionEvents,
) {
    active.live.cast(info.id, target);
    casting.pending = Some(info.id);
    casting.asked_at = Some(Instant::now());
    cooldowns.start_gcd(info);
    events.cooldown_moved();
}

/// `AttackTarget()`: swing at the selection, or find a unit to swing at.
///
/// A swing with no target picks one. The 1.12.1 client does this, and without
/// it the attack key does nothing in the situation where a player most needs
/// it: a fight that started behind them.
///
/// Starting an attack also draws the weapon. The client does this, not the
/// server: vmangos' `HandleAttackSwingOpcode` never changes the sheath state.
/// Without it this client fought unarmed while wearing a sword. The reconcile
/// in `world::entities::sheath` would also draw it, from the swing's
/// `AnimationData` flags, but only on the first swing the server reports; the
/// explicit draw shows the weapon at the press, with no round trip.
fn attack_target(
    active: &ActiveSession,
    selection: &Selection,
    units: &Query<(&WorldEntity, &Transform)>,
    me: &WorldEntity,
    entity: Entity,
    sheathing: &mut MessageWriter<SheathRequest>,
) {
    let mut swing_at = |guid: u64| {
        active.live.attack(Some(guid));
        sheathing.write(SheathRequest {
            entity,
            state: vale_assets::look::sheath::MELEE,
        });
    };
    if let Some(guid) = selection.guid {
        // Already swinging at this unit: the press turns the attack off, as the
        // 1.12.1 client's Attack button does. The weapon stays drawn, because
        // stopping an attack does not stow it; a fighter who stopped attacking
        // mid-fight would otherwise stand with empty hands.
        if active.live.attacking() == Some(guid) {
            active.live.attack(None);
        } else {
            swing_at(guid);
        }
        return;
    }
    // Nothing selected: swing at whatever is attacking this character, which
    // is the case this exists for. The full nearest-enemy search belongs to
    // Tab's scan and is not duplicated here.
    if let Some((attacker, _)) = units
        .iter()
        .find(|(unit, _)| !unit.is_self && unit.target == Some(me.guid))
    {
        swing_at(attacker.guid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cooldown change is announced under both of the game's event names.
    ///
    /// The bug this test covers passed every other check:
    /// `SPELL_UPDATE_COOLDOWN` is in [`super::super::events::FIRED`], so
    /// `vale framexml` counted it as handled, and `--audit --events` fired it at
    /// the twelve spell buttons and saw their handlers run. Nothing in the game
    /// wrote it, so the spellbook's cooldown animations moved only when the
    /// panel was rebuilt for another reason.
    ///
    /// The test calls the one method both events go through, so it asserts
    /// that the pair cannot be separated, not that six call sites each have two
    /// lines.
    #[test]
    fn a_cooldown_moving_raises_both_of_the_names_it_has() {
        fn moved(mut events: ActionEvents) {
            events.cooldown_moved();
        }

        let mut app = App::new();
        crate::interface::events::register(&mut app);
        app.add_systems(Update, moved);
        app.update();

        let world = app.world();
        assert_eq!(
            world
                .resource::<bevy::ecs::message::Messages<ActionbarUpdateCooldown>>()
                .len(),
            1,
            "the action bar was not told"
        );
        assert_eq!(
            world
                .resource::<bevy::ecs::message::Messages<SpellUpdateCooldown>>()
                .len(),
            1,
            "the spellbook was not told"
        );
    }

    /// A refused press turns the auto-repeat off, and a refusal for another
    /// spell leaves it running.
    ///
    /// This covers the "Auto Shot lights up when nothing is being attacked"
    /// report. The press starts the loop locally because no packet says it
    /// started. The server then refuses the cast and sends no
    /// `SMSG_CANCEL_AUTO_REPEAT`, because `SpellCaster::InterruptSpell` sends
    /// one only for a spell it installed, and `Spell::prepare` never installed
    /// this one. `SMSG_CAST_FAILED` is the only notice about it.
    ///
    /// The second assertion catches an unconditional clear: a running loop
    /// survives other casts (that is what `SetCurrentCastedSpell`'s
    /// `Category == 351` test is for), so a Frostbolt that fails for lack of
    /// mana must not stop the arrows.
    #[test]
    fn a_refused_press_puts_the_volley_back_out() {
        #[derive(Resource, Default)]
        struct Script(Vec<u32>);

        fn run(mut auto: ResMut<AutoRepeat>, mut events: ActionEvents, mut script: ResMut<Script>) {
            let steps: Vec<u32> = script.0.drain(..).collect();
            // Arm Auto Shot (75), then refuse Frostbolt (116), then refuse
            // Auto Shot itself.
            auto.begin(steps[0], &mut events);
            disarm_auto_repeat(&mut auto, steps[1], &mut events);
            assert!(auto.is(steps[0]), "a refusal about another spell says nothing about the volley");
            disarm_auto_repeat(&mut auto, steps[2], &mut events);
        }

        let mut app = App::new();
        crate::interface::events::register(&mut app);
        app.init_resource::<AutoRepeat>()
            .insert_resource(Script(vec![75, 116, 75]))
            .add_systems(Update, run);
        app.update();

        let world = app.world();
        assert!(
            world.resource::<AutoRepeat>().spell.is_none(),
            "the refusal for the volley's own spell ends it"
        );
        assert_eq!(
            world
                .resource::<bevy::ecs::message::Messages<StopAutorepeatSpell>>()
                .len(),
            1,
            "`ActionButton_StopFlash()` runs once, and only for the real end"
        );
    }

    /// Each of the loop's two edges is announced once, and a repeat of either
    /// announces nothing.
    ///
    /// Both matter, for different reasons. `START_AUTOREPEAT_SPELL` starts a
    /// flash clock in `ActionButton_OnEvent`, so restarting it on a press that
    /// changed nothing leaves the button lit or dark depending on how the two
    /// clocks line up. `STOP_AUTOREPEAT_SPELL` is answered by
    /// `ActionButton_StopFlash()`, which on a bar with no auto-repeat would stop
    /// the melee attack's flash, a different animation with a different owner.
    ///
    /// The test has the stop arrive from the server rather than from the
    /// press: `SMSG_CANCEL_AUTO_REPEAT` is the only notice of either end of the
    /// loop, and the client cannot predict the target dying.
    #[test]
    fn a_volley_announces_each_of_its_two_ends_once() {
        #[derive(Resource, Default)]
        struct Script(Vec<u32>);

        fn run(mut auto: ResMut<AutoRepeat>, mut events: ActionEvents, mut script: ResMut<Script>) {
            for step in script.0.drain(..) {
                match step {
                    0 => auto.stop(&mut events),
                    id => auto.begin(id, &mut events),
                }
            }
        }

        let mut app = App::new();
        crate::interface::events::register(&mut app);
        app.init_resource::<AutoRepeat>()
            // Press Auto Shot, press it again (which in the client sends the
            // cancel and changes nothing here), then the server's cancel, then
            // a second cancel arriving after the loop is already down.
            .insert_resource(Script(vec![75, 75, 0, 0]))
            .add_systems(Update, run);
        app.update();

        let world = app.world();
        let starts = world
            .resource::<bevy::ecs::message::Messages<StartAutorepeatSpell>>()
            .len();
        let stops = world
            .resource::<bevy::ecs::message::Messages<StopAutorepeatSpell>>()
            .len();
        assert_eq!((starts, stops), (1, 1));
        assert!(world.resource::<AutoRepeat>().spell.is_none());
        // The checked border is re-read on each real edge and on no other:
        // `ActionButton_UpdateState` asks `IsCurrentAction or IsAutoRepeatAction`,
        // and nothing else re-runs it.
        assert_eq!(
            world
                .resource::<bevy::ecs::message::Messages<ActionbarUpdateState>>()
                .len(),
            2
        );
    }

    /// `IsAutoRepeatAction` checks the slot's kind as well as its id.
    ///
    /// An item slot whose entry equals the running spell's id is not the
    /// auto-repeat button. Each of the twelve buttons answers for itself,
    /// because the two events carry no slot.
    #[test]
    fn only_the_slot_holding_the_repeating_spell_answers() {
        let mut bar = ActionBar { slots: vec![None; BAR_SLOTS], ..ActionBar::default() };
        bar.slots[0] = Some(Slot { action: 75, kind: action_kind::SPELL, spell: None });
        bar.slots[1] = Some(Slot { action: 75, kind: action_kind::ITEM, spell: None });
        bar.slots[2] = Some(Slot { action: 133, kind: action_kind::SPELL, spell: None });

        // Slots are one-based on the way in, as the interface passes them.
        assert!(api::is_auto_repeat_action(&bar, Some(75), 1));
        assert!(!api::is_auto_repeat_action(&bar, Some(75), 2), "an item is not a spell");
        assert!(!api::is_auto_repeat_action(&bar, Some(75), 3));
        assert!(!api::is_auto_repeat_action(&bar, None, 1), "nothing is repeating");
    }

    fn spell(id: u32, gcd_ms: u32, recovery_ms: u32) -> SpellInfo {
        SpellInfo {
            id,
            name: format!("spell {id}"),
            range_yards: 30.0,
            recovery_ms,
            gcd_category: 133,
            gcd_ms,
            implicit_target_a: 6,
            ..SpellInfo::default()
        }
    }

    /// One cast's global cooldown blocks every other button. This is why a
    /// cooldown read checks every record rather than only this spell's.
    #[test]
    fn a_global_cooldown_spreads_across_the_bar() {
        let mut cooldowns = Cooldowns::default();
        let fireball = spell(133, 1500, 0);
        let frostbolt = spell(116, 1500, 0);
        assert!(cooldowns.ready(&frostbolt));

        cooldowns.start_gcd(&fireball);
        assert!(!cooldowns.ready(&fireball));
        assert!(
            !cooldowns.ready(&frostbolt),
            "a spell sharing the GCD category is not ready either"
        );

        // A spell off the global cooldown is unaffected, which lets an instant
        // ability be used during one.
        let mut off_gcd = spell(1766, 0, 0);
        off_gcd.gcd_category = 0;
        assert!(cooldowns.ready(&off_gcd));
    }

    /// A refused cast clears the global cooldown and nothing else. It never
    /// reached its `SMSG_SPELL_GO`, so it has no recovery to clear, and leaving
    /// the GCD in place would lock the whole bar for 1.5 seconds after a cast
    /// that never happened.
    #[test]
    fn a_refused_cast_gives_the_bar_back() {
        let mut cooldowns = Cooldowns::default();
        let fireball = spell(133, 1500, 0);
        cooldowns.start_gcd(&fireball);
        cooldowns.clear_gcd(fireball.id);
        assert!(cooldowns.ready(&fireball));
    }

    /// An item's cooldown is recorded under its spell, and the server writes
    /// it.
    ///
    /// `SMSG_SPELL_COOLDOWN` names the item's `ON_USE` spell and not the item,
    /// so the bag frame's cooldown animation reads the record
    /// [`Cooldowns::set`] already stores. That is why
    /// `GetContainerItemCooldown` needed only a read and not a second timer.
    #[test]
    fn an_items_cooldown_is_recorded_under_its_own_spell() {
        let mut cooldowns = Cooldowns::default();
        // 2024 is Healing Potion's spell; the item is a Superior Healing Potion.
        let potion = spell(2024, 0, 0);
        cooldowns.set(potion.id, 60_000, Some(&potion));
        let (remaining, duration) = cooldowns.remaining(&potion).expect("on cooldown");
        assert!(remaining > 59.0, "{remaining}");
        assert!((duration - 60.0).abs() < 0.01, "{duration}");
        // It is that spell's record, not a global one: a different item's
        // spell is unaffected.
        assert!(cooldowns.ready(&spell(439, 0, 0)));
    }

    /// An item use that casts starts the global cooldown at the press, and the
    /// bar only when the server confirms the cast. See [`begin_item_cast`].
    #[test]
    fn using_an_item_starts_the_global_cooldown_and_waits_for_the_bar() {
        let mut cooldowns = Cooldowns::default();
        let mut casting = Casting::default();
        // A First Aid spell: an eight-second cast with a global cooldown.
        let mut bandage = spell(746, 1500, 0);
        bandage.cast_time_ms = 8000;
        // The message writers are not needed to check the two pieces of state a
        // test can see without an app: the bar's record and the cooldown.
        cooldowns.start_gcd(&bandage);
        assert!(!cooldowns.ready(&bandage), "the global cooldown runs at the press");
        assert!(casting.started.is_none(), "and nothing is on the bar yet");

        // The bar appears with `SMSG_SPELL_START`, at the length the server
        // states rather than the file's.
        casting.begin(746, "Linen Bandage".into(), 7000);
        assert_eq!(casting.spell_id, 746);
        assert!(casting.started.is_some(), "an eight-second cast has a bar");
        assert!(casting.progress().is_some_and(|p| p < 0.01));
        assert_eq!(casting.duration.as_millis(), 7000);

        // An instant, such as a potion, puts up no bar, even though the packet
        // arrives for it.
        casting.begin(2024, "Healing Potion".into(), 0);
        assert!(casting.started.is_none(), "an instant has no wind-up");
    }

    /// The spell's own recovery is a different timer from the GCD and outlives
    /// it: Fire Blast's 8-second cooldown is still running when the bar is free
    /// again.
    #[test]
    fn a_spells_own_recovery_outlives_the_global_cooldown() {
        let mut cooldowns = Cooldowns::default();
        let fire_blast = spell(2136, 1500, 8000);
        cooldowns.start_recovery(&fire_blast, &crate::world::spellmods::SpellMods::default());
        let (remaining, duration) = cooldowns.remaining(&fire_blast).expect("recovering");
        assert!(remaining > 7.0, "{remaining}");
        assert!((duration - 8.0).abs() < 0.01, "{duration}");
        // It does not block a different spell, only its own button.
        assert!(cooldowns.ready(&spell(133, 1500, 0)));
    }

    /// A parked cooldown does not run until the server says so
    /// (`SPELL_ATTR_COOLDOWN_ON_EVENT` — Stealth, Feign Death).
    #[test]
    fn a_parked_cooldown_starts_on_the_servers_event() {
        let mut cooldowns = Cooldowns::default();
        let mut stealth = spell(1784, 1000, 10_000);
        stealth.attributes = vale_assets::tables::spellbook::spell_attributes::COOLDOWN_ON_EVENT;
        cooldowns.start_recovery(&stealth, &crate::world::spellmods::SpellMods::default());
        assert!(
            cooldowns.ready(&stealth),
            "the recovery is known and its clock has not started"
        );
        cooldowns.release(stealth.id);
        assert!(!cooldowns.ready(&stealth));
    }

    /// The server's own statement overrides whatever was computed — a school
    /// lockout after a counterspell, a GM reset.
    #[test]
    fn the_servers_cooldown_wins() {
        let mut cooldowns = Cooldowns::default();
        let fireball = spell(133, 1500, 0);
        assert!(cooldowns.ready(&fireball));
        cooldowns.set(fireball.id, 5000, Some(&fireball));
        let (remaining, _) = cooldowns.remaining(&fireball).expect("locked out");
        assert!(remaining > 4.0, "{remaining}");
    }

    /// Attack is spell 6603, and it is not a cast. The server refuses it
    /// through `CMSG_CAST_SPELL` in its "which he shouldn't have" branch, so the
    /// distinction must be made before sending.
    #[test]
    fn the_attack_pseudo_spell_is_recognised() {
        let attack = Slot {
            action: SPELL_ATTACK,
            kind: action_kind::SPELL,
            spell: None,
        };
        assert!(attack.is_auto_attack());
        let fireball = Slot {
            action: 133,
            kind: action_kind::SPELL,
            spell: None,
        };
        assert!(!fireball.is_auto_attack());
        // An item whose entry happens to equal 6603 is not the attack toggle.
        let item = Slot {
            action: SPELL_ATTACK,
            kind: action_kind::ITEM,
            spell: None,
        };
        assert!(!item.is_auto_attack());
    }

    /// The two presses that are not casts, and why both are decided here
    /// rather than by the server.
    ///
    /// `SpellButton_OnClick` calls `CastSpell(id, bookType)` on every row
    /// pressed; it has no passive check and no attack check. The spellbook's
    /// General page therefore passes this client spell 6603 and its 42 passives
    /// (on the test warrior; 4,929 in the game) along with every castable
    /// spell. `HandleCastSpellOpcode` drops the packet without replying, so
    /// while this client drew the cast at the press, the only visible result
    /// was a wind-up, a release and nothing else.
    #[test]
    fn attack_is_a_swing_and_a_passive_is_refused_before_the_socket() {
        let mut attack = spell(SPELL_ATTACK, 0, 0);
        // Attack is not passive and is in the book, so nothing else here would
        // catch it; the id is the entire rule.
        assert!(!attack.is_passive());
        assert_eq!(press_kind(&attack), PressKind::Swing);
        // It stays a swing whatever else the row says.
        attack.cast_time_ms = 1500;
        assert_eq!(press_kind(&attack), PressKind::Swing);

        let mut block = spell(107, 0, 0);
        block.attributes = vale_assets::tables::spellbook::spell_attributes::PASSIVE;
        assert_eq!(press_kind(&block), PressKind::Refused);

        assert_eq!(press_kind(&spell(133, 1500, 0)), PressKind::Cast);
    }

    /// An instant spell has no cast bar; a 1.5-second one does, and it ends.
    #[test]
    fn the_cast_bar_runs_only_for_a_spell_with_a_cast_time() {
        let mut casting = Casting::default();
        casting.begin(133, "Fireball".into(), 0);
        assert_eq!(casting.progress(), None, "an instant has no bar");

        let mut fireball = spell(133, 1500, 0);
        fireball.cast_time_ms = 1500;
        casting.begin(133, "Fireball".into(), 1500);
        let progress = casting.progress().expect("a bar");
        assert!(progress < 0.1, "just started: {progress}");
        casting.end();
        assert_eq!(casting.progress(), None);
    }

    /// Only a bar that was running raises a stop. This mirrors `send_cast`'s
    /// rule that an instant raises no start.
    ///
    /// `CastingBarFrame_OnEvent`'s stop branch acts on whatever bar is shown,
    /// so a stop for a cast that never had a bar colours and fades whatever
    /// else is shown, which for a channel is the bar that has just appeared.
    #[test]
    fn a_stop_is_raised_only_for_a_bar_that_was_running() {
        let mut casting = Casting::default();
        // An instant: begun, ended, and there was nothing to stop.
        casting.begin(133, "Fireball".into(), 0);
        assert!(!casting.end());

        let mut fireball = spell(133, 1500, 0);
        fireball.cast_time_ms = 1500;
        casting.begin(133, "Fireball".into(), 1500);
        assert!(casting.end(), "a wind-up ends with news");
        assert!(!casting.end(), "and does not end twice");
    }

    /// A press sends a request and the server answers; nothing is on the bar
    /// in between.
    ///
    /// This state replaced the local prediction and must get two things right.
    /// The ask holds no bar (`progress` is `None` and `in_progress` is false,
    /// because there is no wind-up, only a packet in flight), and whichever
    /// answer arrives first clears it. If it were not cleared, the button would
    /// be blocked for the rest of the session; if it held a bar, a refused cast
    /// would show one, which was the original bug.
    #[test]
    fn an_unanswered_ask_holds_no_bar_and_is_cleared_by_the_answer() {
        let mut casting = Casting { pending: Some(133), ..Default::default() };
        assert_eq!(casting.progress(), None, "an ask is not a cast");
        assert!(!in_progress(&casting), "and it is not a wind-up either");

        // The server took it: the bar goes up and the ask is spent.
        casting.begin(133, "Fireball".into(), 2500);
        assert_eq!(casting.pending, None);
        assert!(in_progress(&casting));

        // An instant's answer is the same call with no bar.
        let mut casting = Casting { pending: Some(1449), ..Default::default() };
        casting.begin(1449, "Arcane Explosion".into(), 0);
        assert_eq!(casting.pending, None);
        assert!(!in_progress(&casting), "an instant leaves nothing running");
    }

    /// A channel is not a wind-up, and the difference decides whether a new
    /// press is refused.
    ///
    /// `Spell::prepare`'s "another action is in progress" gate is
    /// `IsNonMeleeSpellCasted(false, true, true)`. `skipChanneled` is `true`,
    /// so a cast pressed during a channel is accepted by the server and
    /// replaces it, while one pressed during a wind-up is refused.
    #[test]
    fn a_channel_does_not_block_a_press_and_a_wind_up_does() {
        let mut casting = Casting::default();
        assert!(!in_progress(&casting));

        // A channel: no `started`, so nothing is blocked.
        casting.channelling = Some(12051);
        casting.spell_id = 12051;
        assert!(!in_progress(&casting));
        assert_eq!(casting.progress(), None, "a channel has no wind-up to show");

        // Pressing something ends the channel, as the server's acceptance of
        // the cast does.
        let mut fireball = spell(133, 1500, 0);
        fireball.cast_time_ms = 1500;
        casting.begin(133, "Fireball".into(), 1500);
        assert_eq!(casting.channelling, None);
        assert!(in_progress(&casting), "a wind-up blocks");
    }

    /// A pushback lengthens the bar and never starts one.
    ///
    /// `SMSG_SPELL_DELAYED` is the only packet in the protocol that restates a
    /// cast's length after it has begun; the server has already added the time
    /// to its own `m_timer`. A client that ignores it runs the bar out and
    /// leaves it full while the server is still casting, which is the visible
    /// part of the "stuck casting a spell that was interrupted" report.
    ///
    /// The two refusals prevent moving the wrong bar: a channel is delayed
    /// through `MSG_CHANNEL_UPDATE` instead (vmangos' `DelayedChannel` shortens
    /// its timer), and a cast with no wind-up has no bar to lengthen.
    #[test]
    fn a_pushback_lengthens_the_bar_it_finds_and_conjures_none() {
        let mut casting = Casting::default();
        assert!(!casting.delay(500), "there was no cast");

        casting.begin(133, "Fireball".into(), 3500);
        assert_eq!(casting.duration, Duration::from_millis(3500));

        // Two hits land while it runs; each packet states how much that hit
        // added.
        assert!(casting.delay(500));
        assert!(casting.delay(1000));
        assert_eq!(
            casting.duration,
            Duration::from_millis(5000),
            "a pushback is a difference, not a new length",
        );

        // An instant has no bar, and a channel's bar is moved by its own event.
        let mut instant = Casting::default();
        instant.begin(1449, "Arcane Explosion".into(), 0);
        assert!(!instant.delay(500));
        let mut channel = Casting::default();
        channel.channelling = Some(12051);
        channel.started = Some(Instant::now());
        assert!(!channel.delay(500));
    }

    /// The spell cursor is cleared by whatever ends it, and only once.
    ///
    /// This is the state machine behind the report: press a heal with nothing
    /// suitable selected and the client asks for a target instead of failing.
    /// One of four things answers: a click on a unit, a click on empty ground,
    /// Escape, or a new press. `cast_known_spell` calls `stop` on every path out
    /// of the aiming rule for that reason: a cursor that survived its own
    /// answer would cast the previous spell at the next thing clicked.
    #[test]
    fn the_spell_cursor_is_put_away_by_whatever_answers_it() {
        let mut targeting = SpellTargeting::default();
        assert!(!targeting.is_targeting());
        assert_eq!(targeting.spell(), None);

        targeting.begin(2050, Asking::Unit); // Lesser Heal
        assert!(targeting.is_targeting());
        assert_eq!(targeting.spell(), Some(2050));
        assert!(!targeting.wants_ground());

        // `over_valid` is the cursor's red-or-green state and is recomputed
        // every frame. `stop` must clear it, or the next question starts green
        // over whatever the pointer is on.
        targeting.over_valid = true;
        targeting.stop();
        assert!(!targeting.is_targeting());
        assert!(!targeting.over_valid);
        assert_eq!(targeting.spell(), None);

        // A second question replaces the first: two spells cannot wait at once,
        // because there is one cursor.
        targeting.begin(2050, Asking::Unit);
        targeting.begin(1459, Asking::Unit);
        assert_eq!(targeting.spell(), Some(1459));
    }

    /// The ground mode of the same cursor clears its own two fields. A ground
    /// question that ended must leave neither the mode nor the point behind:
    /// the mode would make the next unit cast wait for a location, and a stale
    /// point would place the next Blizzard where the last one went.
    #[test]
    fn a_placed_question_takes_its_mode_and_its_point_away_with_it() {
        let mut targeting = SpellTargeting::default();
        targeting.begin(10, Asking::Ground); // Blizzard
        assert!(targeting.is_targeting());
        assert!(targeting.wants_ground());

        targeting.over_ground = Some([-9449.0, -12.0, 56.0]);
        targeting.over_valid = true;
        targeting.stop();
        assert!(!targeting.wants_ground());
        assert_eq!(targeting.over_ground, None);
        assert!(!targeting.over_valid);

        // A unit question after a ground question is a unit question: the mode
        // comes from the new press, not from the previous state.
        targeting.begin(10, Asking::Ground);
        targeting.over_ground = Some([1.0, 2.0, 3.0]);
        targeting.begin(2050, Asking::Unit);
        assert!(!targeting.wants_ground());
        assert_eq!(targeting.over_ground, None);
    }

    /// The item mode excludes the other two, which is what [`Asking`]
    /// guarantees in place of the two bools it replaced. A cursor waiting for
    /// an item must not read as waiting for a location, or
    /// `spell_ground_under_pointer` walks the terrain every frame for an answer
    /// nobody uses, and the click that ends it goes to the wrong branch.
    #[test]
    fn an_item_question_is_not_a_ground_question() {
        let mut targeting = SpellTargeting::default();
        targeting.begin(2828, Asking::Item); // Sharpen Blade
        assert!(targeting.is_targeting());
        assert!(targeting.wants_item());
        assert!(!targeting.wants_ground());

        targeting.stop();
        assert!(!targeting.wants_item());
        assert!(!targeting.is_targeting());

        // The mode comes from the new press, in either order.
        targeting.begin(2828, Asking::Item);
        targeting.begin(10, Asking::Ground);
        assert!(!targeting.wants_item());
        assert!(targeting.wants_ground());
        targeting.begin(2828, Asking::Item);
        assert!(!targeting.wants_ground());
    }

    /// A next-swing spell is a queued swing, not a cast, which decides whether
    /// the spell art is drawn at the press. Heroic Strike is the measured case:
    /// `Attributes 0x00050014`, which is `ON_NEXT_SWING_NO_DAMAGE` and not
    /// `ON_NEXT_SWING`.
    #[test]
    fn heroic_strike_is_a_queued_swing_rather_than_a_cast() {
        use vale_assets::tables::spellbook::spell_attributes;
        let mut heroic_strike = spell(78, 0, 0);
        heroic_strike.attributes = 0x0005_0014;
        assert!(heroic_strike.on_next_swing());
        assert_eq!(
            press_kind(&heroic_strike),
            PressKind::Cast,
            "it is still a cast to send — only the prediction differs"
        );

        // The other bit of the pair, and an ordinary instant with neither.
        let mut cleave = spell(845, 0, 0);
        cleave.attributes = spell_attributes::ON_NEXT_SWING;
        assert!(cleave.on_next_swing());
        assert!(!spell(133, 1500, 0).on_next_swing());
    }
}
