//! **Pressing a button**: the action bar, the cooldowns that gate it, the cast
//! it sends, and the swing.
//!
//! The bar itself is the server's — `SMSG_ACTION_BUTTONS` says where the last
//! session left it — and everything about *using* one is the client's. Four rules
//! here are the game's own and each is load-bearing:
//!
//! * **Attack is not a cast.** Spell 6603 is the pseudo-spell every character
//!   carries in slot 1, and pressing it sends `CMSG_ATTACKSWING` at the
//!   selection. `CMSG_CAST_SPELL` with that id is refused by the server's own
//!   "which he shouldn't have" branch, so a client that treats the bar
//!   uniformly cannot melee at all.
//! * **A swing with no target acquires one.** The client picks the best
//!   candidate and swings at it, which is why pressing attack in a fight that
//!   started behind you works.
//! * **The target is resolved from the spell, not from the selection.** See
//!   [`vale_assets::tables::spellbook::resolve_aim`] — 14,002 of the game's 22,360
//!   spells commit with *no target at all*, and shipping the selection with one
//!   of those is how a self-buff comes back "Invalid target".
//! * **The global cooldown starts locally, at send.** Not on any packet: the
//!   client starts it the moment the cast goes out, and vmangos
//!   sends nothing for it. A client waiting for the server to say so has a bar that responds a round trip late and swings
//!   twice on a double press.
//!
//! ## Which clock a cooldown runs on
//!
//! Three of them, per spell, exactly as the client's own `SpellHistory` records
//! do — the spell's own recovery, its category's, and the global. The read takes
//! the longest of the three that apply, which is the mechanism that spreads one
//! cast's GCD across every other button. Who *starts* which is the part that is
//! easy to get wrong:
//!
//! ```text
//! global cooldown   locally, when the cast is sent
//! own recovery      when *our own* SMSG_SPELL_GO comes back
//! override/lockout  SMSG_SPELL_COOLDOWN — a counterspell, a GM reset
//! parked release    SMSG_COOLDOWN_EVENT — Stealth, Feign Death
//! a failed cast     clears the global cooldown only
//! ```
//!
//! The last line matters: a refused cast never reached its `SPELL_GO`, so there
//! is no recovery to clear — but the GCD was already started locally and would
//! otherwise lock the bar for a second and a half for nothing.
//!
//! ## …and the cast itself is the server's, which is the opposite rule
//!
//! The global cooldown is the one thing a press starts. **Everything else about
//! a cast waits for the server to say it happened**: the bar, the wind-up pose,
//! the art on the caster's hands, the missile, the sound. This client used to
//! draw all of it at the press and take it back off on a refusal, which is what
//! "the animation plays but the spell was not really cast" is — a spell that
//! never happened, drawn in full, twice a second while a key is held.
//!
//! 5875 does not do it, and the two sides agree about that. The client's press
//! is its local refusals, the pending record, the GCD (started from inside the
//! send) and the send; `SPELLCAST_START` is raised inside `SMSG_SPELL_START`'s
//! handler and in no other place. And vmangos labels its own `SendSpellStart()` `// will show cast bar` and its
//! `AddGCD` `// add gcd server side (client side is handled by client itself)`.
//! So the split is exactly: the cooldown is ours, the cast is theirs.
//!
//! What covers the round trip is [`Casting::pending`] — the 1.12 client's own
//! pending-cast record — which draws nothing and exists so that a repeat press is
//! dropped rather than sent again.
//!
//! ## The verb and the key that runs it are different things
//!
//! [`use_action`] is this module's `UseAction` — the C function
//! `ActionButton.lua` calls, taking a **one-based slot** and the game's own
//! `onSelf` flag. It does not know which key was pressed and must not: the key
//! is `Bindings.xml`'s business, it arrives here as a
//! [`Binding::ActionButton`][crate::input::bindings::Binding::ActionButton] message, and
//! `SELFACTIONBUTTON1` is the same verb with the flag set. See
//! [`crate::input::bindings`] for why that separation is the game's own rather than a
//! preference.

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

/// **How many slots this client holds — all 120 of them**, which is what
/// `SMSG_ACTION_BUTTONS` carries and what the interface asks about.
///
/// It was twelve for as long as twelve was all anything could reach: the game's
/// main bar is `1`..`=` and nothing else pressed a button. But the bar has
/// **pages** — six of them — and `ActionButton_GetPagedID` is
/// `id + (page - 1) * NUM_ACTIONBAR_BUTTONS`, so the moment the arrows either
/// side of it work (`--audit --clicks`, this round) the interface starts asking
/// about slot 13 through 72, and a twelve-slot vector answers "empty" to every
/// one of them. The server has always sent the whole 120.
///
/// **Which** twelve of them the interface is looking at is not this side's
/// question at all: `CURRENT_ACTIONBAR_PAGE` is a Lua global and
/// `GetBonusBarOffset()` is answered off [`ActionBar::bonus_bar`], and every
/// button adds the two to its own id before it asks anything here. So a slot
/// number arriving from the interface is already absolute.
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

    /// The auto-attack toggle rather than a cast — see the module comment.
    pub fn is_auto_attack(&self) -> bool {
        self.kind == action_kind::SPELL && self.action == SPELL_ATTACK
    }
}

/// The bar as this client holds it: 120 slots, resolved.
#[derive(Resource, Default)]
pub struct ActionBar {
    pub slots: Vec<Option<Slot>>,
    /// The `spellbook_version` these were built from, so the rebuild happens on
    /// a difference rather than every frame — resolving twelve spells means
    /// twelve string reads out of a 22,360-row DBC.
    built_from: Option<u32>,
    /// **Which parse of the DBCs the slots were resolved from** — see
    /// `GameAssets::tables_generation`, and [`rebuild_bar`], where the reason
    /// the server's version is not enough on its own is.
    built_with: u64,
    /// Every spell the character knows and could actually press — the
    /// press-able subset, which is not the spellbook's list; see
    /// [`super::spellbook`] for the one filter that differs.
    pub known: Vec<SpellInfo>,
    /// **`GetBonusBarOffset()`** — which of the four bonus bars the character's
    /// current form is showing, or 0 for the ordinary paged bar.
    ///
    /// Off `UNIT_FIELD_BYTES_1`'s form byte through
    /// `SpellShapeshiftForm.dbc`'s `bonusActionBar` column
    /// ([`vale_assets::tables::spellbook::ShapeshiftForms`]) — a client answer rather
    /// than a server one, because the packet carries the form and never the bar.
    /// See [`ActionbarBonusChanged`], which is what the interface redraws on.
    pub bonus_bar: u8,
    /// **`GetActionBarToggles()`** — which of the four extra bars this character
    /// has switched on, as a mask of
    /// [`vale_protocol::play::spells::multi_bar`] bits.
    ///
    /// The *only* piece of interface layout in 1.12 that the server keeps: it is
    /// `PLAYER_FIELD_BYTES` byte 2, written by `CMSG_SET_ACTIONBAR_TOGGLES` and
    /// handed straight back. See [`follow_bar_toggles`] for the copy and
    /// [`vale_protocol::state::objects::Entity::action_bar_toggles`] for the field.
    ///
    /// **No event goes with it**, unlike [`Self::bonus_bar`], and that is the
    /// interface's own arrangement rather than a gap: `UIParent.lua` reads this
    /// once, on `PLAYER_ENTERING_WORLD`, and every later change comes from the
    /// options panel — which writes its own `SHOW_MULTI_ACTIONBAR_*` globals and
    /// calls `MultiActionBar_Update()` itself, without ever asking again.
    pub toggles: u8,
}

/// **The ranged attack that is repeating itself**, or `None`.
///
/// Auto Shot and a wand's Shoot are not casts and they are not the melee
/// auto-attack either; they are the one press in the game whose *effect is a
/// loop the server runs*. `Spell::prepare` files an
/// `IsAutoRepeatRangedSpell()` in `CURRENT_AUTOREPEAT_SPELL` instead of casting
/// it, and `Unit::_UpdateAutoRepeatSpell` fires a triggered copy on the ranged
/// attack timer for as long as it stands — so **one `CMSG_CAST_SPELL` buys an
/// indefinite stream of `SMSG_SPELL_GO`s**, and until this existed the only
/// thing a player could do with the button was turn it on.
///
/// Held here rather than in `SessionStatus` because the *start* is entirely
/// this client's: the server acknowledges the cast like any other and says
/// nothing about the loop it began. What it does say is
/// [`vale_protocol::play::spells::PlayerEvent::AutoRepeatCancelled`], and every
/// way the loop can end — the press, the target dying, walking out of range, a
/// wand-user moving — comes through that one packet, which is why [`Self::stop`]
/// is reached from the drain rather than from the press.
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

    /// Note that it has started, and say so **once**.
    ///
    /// Idempotent on the spell for the reason every other edge in this module
    /// is: `START_AUTOREPEAT_SPELL` starts a flash clock in
    /// `ActionButton_OnEvent`, and re-arming it every press would leave the
    /// button lit or dark depending on how the two clocks happened to line up.
    pub(crate) fn begin(&mut self, spell_id: u32, events: &mut ActionEvents) {
        if self.spell == Some(spell_id) {
            return;
        }
        self.spell = Some(spell_id);
        events.autorepeat_start.write(StartAutorepeatSpell);
        // The checked border is a different reading from the flash —
        // `ActionButton_UpdateState` asks `IsCurrentAction or
        // IsAutoRepeatAction` — and nothing else re-runs it.
        events.state.write(ActionbarUpdateState);
    }

    /// …and that it has stopped, likewise once.
    pub(crate) fn stop(&mut self, events: &mut ActionEvents) {
        if self.spell.take().is_none() {
            return;
        }
        events.autorepeat_stop.write(StopAutorepeatSpell);
        events.state.write(ActionbarUpdateState);
    }
}

/// **The four resources a keypress writes**, bundled — and the bundle exists
/// for a hard reason rather than for tidiness.
///
/// A Bevy function system takes its parameters as one tuple and `SystemParam` is
/// implemented for tuples up to **sixteen**; [`run_bindings`] was at exactly
/// sixteen, so the next resource a press needed would not have been a
/// compile error about a limit — it is a "the method `chain` exists but its
/// trait bounds were not satisfied" on the *plugin*, one screen away from the
/// system that caused it. `LuaWorld` in `lua::api` carries the same note for the
/// same reason.
///
/// The membership is "what a press changes about *this character's* casting",
/// which is why the bar and the spellbook are not in it: those are rebuilt from
/// the server and read here.
#[derive(bevy::ecs::system::SystemParam)]
pub struct PressState<'w> {
    pub cooldowns: ResMut<'w, Cooldowns>,
    pub casting: ResMut<'w, Casting>,
    pub auto_repeat: ResMut<'w, AutoRepeat>,
    pub targeting: ResMut<'w, SpellTargeting>,
}

/// The cast this client believes is in progress, for the cast bar.
///
/// **The server's state, not a prediction — and that is this round's
/// correction.** It used to begin at the send, on the argument that waiting a
/// round trip would leave the bar starting late. The reference does wait:
/// `SPELLCAST_START` is raised in exactly one place in the 1.12 client and
/// that place is `SMSG_SPELL_START`'s own handler, while vmangos labels the packet `// will show cast bar`. So a
/// cast this client shows is a cast the server took, and a refused press now
/// shows nothing at all where it used to show a wind-up and then take it back.
///
/// [`Self::pending`] is the state that covers the gap, and it is the
/// reference's too.
#[derive(Resource, Default)]
pub struct Casting {
    pub spell_id: u32,
    pub name: String,
    pub started: Option<Instant>,
    pub duration: Duration,
    /// **What has been asked for and not yet answered** — the spell id sent to
    /// the server, held until it says yes (`SMSG_SPELL_START`), no
    /// (`SMSG_CAST_RESULT` with a failure) or nothing more (a next-swing
    /// ability's own release).
    ///
    /// The 1.12 client keeps the same record: the press writes the spell and its
    /// targets into it and the failure handler clears it. Two things here read it, and neither is a cast bar — it
    /// draws nothing:
    ///
    /// * the **repeat press**, which is dropped rather than sent again. The
    ///   reference does send, and the server answers every one of them with
    ///   `SPELL_FAILED_SPELL_IN_PROGRESS`; dropping it locally is the same
    ///   outcome with one packet instead of ten, and it is the same judgement
    ///   already made one screen down for a cast that *is* running.
    /// * the **spell targeting cursor**, so that a second press while the first
    ///   is in flight does not put the cursor back up.
    ///
    /// **An ask is always answered**, which is what makes it safe to gate on:
    /// `Spell::SendCastResult` writes a status byte on both paths and always
    /// sends, so every `CMSG_CAST_SPELL` that reaches `Spell::prepare` comes
    /// back as an acceptance or a refusal. The one exception is
    /// `HandleCastSpellOpcode`'s "which he shouldn't have" branch — a spell the
    /// character does not know, or a passive — which returns with no reply at
    /// all; a passive is refused here before the send and nothing else in this
    /// client casts a spell that is not in the character's own book, so that
    /// branch is unreachable rather than merely unlikely. Leaving the world
    /// resets the whole resource either way (see `forget`).
    ///
    /// **That last paragraph was wrong, and the character it wedged is the
    /// proof.** `HandleCastSpellOpcode` returns silently on
    /// `!HasActiveSpell(spellId)` — *has it as an active spell*, not *knows it* —
    /// and an action button holding a **superseded rank** is exactly that: the
    /// server sends `SMSG_SUPERCEDED_SPELL` when a higher rank is learned so the
    /// client can swap it in the bar and the book, this client does not read that
    /// opcode at all, and the stale id stays in `character_action` for ever.
    /// Measured on the reporter's own warrior: button 73 holds Heroic Strike
    /// 11566 and `character_spell` has only 11567; button 75 holds Rend 11572
    /// against 11573.
    ///
    /// So the ask is **not** always answered, and one unanswerable press used to
    /// take the character out for the rest of the session: every later press of
    /// any other spell met the gate below and printed "another action is in
    /// progress". [`Self::asked_at`] is the deadline that keeps a dropped reply
    /// to the one press it belongs to.
    pub pending: Option<u32>,
    /// When [`Self::pending`] was armed, so an ask nobody answers expires.
    ///
    /// The same judgement `session::logout` and the character delete already
    /// make: **where the server has a path that sends nothing at all, a deadline
    /// is a refusal rather than a dead socket.** It is deliberately not a
    /// message — the reference shows nothing here either, and the press really
    /// did do nothing — so what it restores is only the ability to press
    /// something else.
    pub asked_at: Option<Instant>,
    /// **The spell being channelled, if one is** — a different state from
    /// [`Self::started`] and deliberately not the same field.
    ///
    /// A channel has no wind-up: it is an instant on the wire, so `started` is
    /// `None` for one and the *whole* of it happens after `SMSG_SPELL_GO`. The
    /// two must be told apart because the server tells them apart —
    /// `Spell::prepare`'s "another action is in progress" refusal skips
    /// channels (`IsNonMeleeSpellCasted(false, true, true)`, whose second
    /// argument is `skipChanneled`), so a cast pressed during a channel is
    /// *accepted* and simply replaces it, where one pressed during a wind-up is
    /// not. See [`in_progress`].
    pub channelling: Option<u32>,
    /// **The next-swing ability waiting on the weapon** — Heroic Strike,
    /// Raptor Strike, Cleave — and a third state again rather than a variation
    /// on the two above.
    ///
    /// It draws **no bar at all** and must not: nothing is winding up, and
    /// `in_progress` deliberately does not consult it, because the server does
    /// not either — `Spell::prepare` refuses on `CURRENT_GENERIC_SPELL` and a
    /// melee spell is not one, so a Fireball pressed with Heroic Strike queued
    /// is accepted by the server and must be accepted here.
    ///
    /// What it is for is **`IsCurrentAction`**, which is the reference's own
    /// use of the same slot: it compares the button's spell against the
    /// client's `CURRENT_MELEE_SPELL` before it looks at
    /// anything else, which is what lights Heroic Strike's border the moment it
    /// is pressed and leaves it lit until the swing lands. Without it the whole
    /// family was pressable and drew no state at all.
    ///
    /// **Cleared by the server and never by a clock here**: the release
    /// (`SMSG_SPELL_GO` naming us), a refusal, or an interrupt. See
    /// [`drain_events`], and note that a queue nothing discharges *stays*
    /// queued — a player standing out of reach keeps the ability armed, which
    /// is the reference's behaviour and not a leak.
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

    /// **Put a cast on the bar, on the server's word.**
    ///
    /// The duration is `SMSG_SPELL_START`'s own `m_timer` rather than
    /// `Spell.dbc`'s base cast time, which is the second thing waiting for the
    /// packet buys: the server has already folded in haste, talents and any
    /// modifier this client does not model, so the bar is the length the cast
    /// really is instead of the length the file says.
    ///
    /// `name` is the caller's, because the id has to be resolved through the
    /// catalog and this module is not the one that holds it.
    pub(crate) fn begin(&mut self, spell_id: u32, name: String, cast_time_ms: u32) {
        self.spell_id = spell_id;
        self.name = name;
        self.duration = Duration::from_millis(u64::from(cast_time_ms));
        self.started = (cast_time_ms > 0).then(Instant::now);
        // The cast the server took ends whatever was being channelled — its own
        // behaviour, since it accepts the cast and drops the channel.
        self.channelling = None;
        // …and it is no longer waiting for an answer, whatever it was — nor for
        // the deadline that would have let go of one. See [`Self::asked_at`].
        self.pending = None;
        self.asked_at = None;
    }

    /// **Push the cast back**, and say whether there was a cast to push.
    ///
    /// `SMSG_SPELL_DELAYED` is a difference the server has already applied to
    /// its own `m_timer`, so the bar simply gets longer: `started` stays where
    /// it is and the finish line moves, which is exactly what
    /// `CastingBarFrame_OnEvent`'s own arm does with `arg1` (it slides *both*
    /// ends, which comes to the same thing since it re-states the range).
    ///
    /// **A channel is refused here.** vmangos pushes one back through
    /// `DelayedChannel`, which *shortens* `m_timer` and announces itself as
    /// `MSG_CHANNEL_UPDATE`; a `SPELLCAST_DELAYED` raised over a channelling bar
    /// would move the wrong end of it. And a cast with no wind-up has no bar at
    /// all — an instant's `SMSG_SPELL_DELAYED` cannot happen, but the answer to
    /// one is "nothing to lengthen" rather than a bar conjured out of it.
    pub(crate) fn delay(&mut self, delay_ms: u32) -> bool {
        if self.started.is_none() || self.channelling.is_some() {
            return false;
        }
        self.duration += Duration::from_millis(u64::from(delay_ms));
        true
    }

    /// End whatever was on the bar, and say whether there was anything on it.
    ///
    /// **The answer is what decides whether `SPELLCAST_STOP` is raised**, and it
    /// is the symmetric half of [`send_cast`]'s rule that an *instant* raises no
    /// `SPELLCAST_START`: a stop for a bar that never appeared is at best a
    /// no-op and at worst arrives while a channel bar is up, where
    /// `CastingBarFrame_OnEvent` greens it out and fades it — which is a
    /// channel bar that vanishes a frame after it appears.
    pub(crate) fn end(&mut self) -> bool {
        self.channelling = None;
        self.started.take().is_some()
    }
}

/// **Take the queued swing back off**, if it is the spell this packet is about.
///
/// One door for the three packets that can empty the queue, and it raises the
/// bar's own event exactly once: `ActionButton_UpdateState` is the only thing
/// that re-reads `IsCurrentAction`, and nothing else in the directory would run
/// it — the border would stay lit until the next unrelated slot update.
pub(crate) fn disarm_next_swing(casting: &mut Casting, spell_id: u32, events: &mut ActionEvents) {
    if casting.next_swing != Some(spell_id) {
        return;
    }
    casting.next_swing = None;
    events.state.write(ActionbarUpdateState);
}

/// **Take the volley back off when the server refuses the press that armed
/// it** — the one end of an auto-repeat that no packet announces.
///
/// [`AutoRepeat::stop`] is otherwise reached only from `SMSG_CANCEL_AUTO_REPEAT`,
/// and that packet cannot arrive for this case. `SpellCaster::InterruptSpell`
/// sends it from inside a guard on `m_currentSpells[CURRENT_AUTOREPEAT_SPELL]`
/// being set, and a press the server refuses never installs one: `Spell::prepare`
/// runs `CheckCast`, finds the refusal is not one of the two
/// `IsAcceptableAutorepeatError` lets through (`SPELL_CAST_OK` and
/// `SPELL_FAILED_MOVING`, the second so a hunter may arm the loop on the run),
/// sends `SMSG_CAST_FAILED` and calls `finish(false)`.
///
/// So the refusal packet is the only thing that will ever be said about it, and
/// a client that does not read it here keeps the checked border lit for the
/// session. Pressing the button again makes it worse rather than better: a
/// client that believes the loop is running sends
/// `CMSG_CANCEL_AUTO_REPEAT_SPELL`, which the server answers by interrupting a
/// spell it does not have, and says nothing about that either.
///
/// **Gated on the id**, unlike [`Casting::pending`] one line up in the same
/// arm: a refusal for some *other* spell says nothing about a volley that is
/// genuinely running, and the ranged loop survives casting through it — that is
/// what `SetCurrentCastedSpell`'s `Category == 351` test is for.
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

/// **A cast waiting to be pointed at something** — 1.12's targeting cursor.
///
/// The third outcome of the aiming rule, and the one this client did not have:
/// press a heal with nothing selected and the reference client does not refuse,
/// it hands you the cursor and waits for a click. `resolve_aim`'s own doc has
/// the branch and it turns on one bit — a spell that
/// wants a hostile unit gets a message, and everything else gets asked.
///
/// **The state is a spell id and nothing else.** Which unit satisfies it is
/// re-asked at the click, through the same [`resolve_aim`] the press went
/// through, because the world moves between the two: the candidate you pointed
/// at may have died, changed faction or walked out of range while the cursor
/// was up, and a cached answer would cast at it anyway.
#[derive(Resource, Default)]
pub struct SpellTargeting {
    /// The spell awaiting a target, if any.
    spell: Option<u32>,
    /// **Whether it is waiting for a *place* rather than for a unit** — the same
    /// cursor's other job, and the whole of Blizzard, Flamestrike and Rain of
    /// Fire. See [`vale_assets::tables::spellbook::CastAim::WantsGround`].
    ///
    /// A field beside the id rather than three resources, because everything
    /// else about the mode is identical: one spell waiting, one cursor, one
    /// click to end it, one Escape to stand it down. What differs is only what
    /// the click resolves to.
    asking: Asking,
    /// Where the pointer is on the floor this frame, in **WoW axes** — `None`
    /// when it is on the sky, or off the loaded world. Written by
    /// [`crate::interface::target::spell_ground_under_pointer`] and only while
    /// [`Self::wants_ground`] is true, because the ray it costs is a walk down
    /// the terrain.
    pub over_ground: Option<[f32; 3]>,
    /// Whether whatever the pointer is over right now would satisfy it — which
    /// is the whole of `Cast.blp` against `UnableCast.blp`, and the only
    /// feedback the player gets before committing.
    pub over_valid: bool,
    /// **True for the frame a click was taken by this mode.** A click that casts
    /// must not also retarget, and the two decisions are made by two systems:
    /// `pick_spell_target` runs first and sets this, `target::select_on_click`
    /// reads it and stands down. A flag rather than an ordering alone, because
    /// ordering says which runs first and not which one *acted*.
    pub took_click: bool,
}

impl SpellTargeting {
    /// `SpellIsTargeting()` — is the cursor up?
    pub fn is_targeting(&self) -> bool {
        self.spell.is_some()
    }

    /// The spell the cursor is holding, if it is up.
    pub fn spell(&self) -> Option<u32> {
        self.spell
    }

    /// Is the cursor waiting for a **patch of floor** rather than for a unit?
    pub fn wants_ground(&self) -> bool {
        self.spell.is_some() && self.asking == Asking::Ground
    }

    /// …or for an **item** — the enchanting formulas, the poisons and the
    /// sharpening stones, which 1.12 aims by clicking a bag square.
    ///
    /// The interface has no verb of its own for it: `ContainerFrame.lua:596` is
    /// a plain `UseContainerItem(bag, slot)` and the C side decides whether
    /// that is a use or an answer. So the reader is
    /// [`crate::interface::items`]' own use path, which is where the
    /// clicked slot has already been resolved to an item.
    pub fn wants_item(&self) -> bool {
        self.spell.is_some() && self.asking == Asking::Item
    }

    /// `SpellStopTargeting()` — put the cursor away with nothing cast.
    ///
    /// Escape, a right click, a click on empty ground, and the interface's own
    /// verb all end here. It is deliberately not an error: the player changed
    /// their mind, which is not a refused cast and raises no `SPELLCAST_FAILED`.
    pub fn stop(&mut self) {
        self.spell = None;
        self.asking = Asking::Unit;
        self.over_ground = None;
        self.over_valid = false;
    }

    /// Put the cursor up. `pub(super)` rather than private for the same reason
    /// [`Self::stop`] is public: the *other* half of this mode lives in
    /// [`super::target`], which ends it on a click and has to be able to start
    /// one to check that it declines while it is up.
    pub(crate) fn begin(&mut self, spell_id: u32, asking: Asking) {
        self.spell = Some(spell_id);
        self.asking = asking;
        self.over_ground = None;
        self.over_valid = false;
    }
}

/// **What the waiting cursor is waiting for.**
///
/// Three modes and one cursor: the click that ends it is the same gesture and
/// only its answer differs — a unit, three floats, or an item's guid. See
/// [`SpellTargeting`], whose whole state is this plus the spell id.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Asking {
    #[default]
    Unit,
    Ground,
    /// `TARGET_FLAG_ITEM` — answered by a bag or paperdoll click rather than by
    /// a click in the world, which is why nothing in
    /// [`super::target`] ends this one.
    Item,
}

/// **What the player pointed the waiting cast at** — a unit or a place.
///
/// One enum rather than two parameters through [`cast_known_spell`], because
/// they are the same event seen twice: the cursor asked a question and this is
/// the answer, whichever kind of question it was. `None` at that call site still
/// means "nobody has been asked yet", which is the distinction that decides
/// between putting the cursor up and refusing.
#[derive(Debug, Clone, Copy)]
enum Pointed {
    Unit(Entity),
    /// **An item's own guid**, from a bag or paperdoll click — see
    /// [`SpellItemPicked`], which is the message that carries it.
    Item(u64),
    /// **A square in the trade window**, which carries a slot number rather
    /// than a guid — see
    /// [`vale_protocol::play::spells::CastTarget::TradeSlot`].
    TradeSlot(u8),
    /// In **WoW axes**, which is the frame `CastTarget::Dest` goes out in — so
    /// the conversion happens once, where the ray is walked, rather than beside
    /// the socket.
    Ground([f32; 3]),
}

/// **A unit was picked for the waiting cast** — the click, as a message.
///
/// The pick and the cast are two systems because they need two different halves
/// of the world: deciding *what is under the pointer* wants the hover and the
/// mouse, and casting at it wants the bar, the cooldowns, the session and the
/// error frame — which is already the largest parameter list in this module.
/// One message between them keeps [`run_bindings`] the single door onto a cast,
/// which is the property [`cast_known_spell`]'s own doc is about.
/// **An item was clicked for the waiting cast** — the other end of
/// [`SpellTargeting::wants_item`].
///
/// Written by [`crate::interface::items`]' use path rather than by the
/// world pick, because 1.12's interface has no verb for this: the bag square's
/// `OnClick` is a plain `UseContainerItem(bag, slot)` and the C side decides
/// whether that is a use or an answer. By the time that path has a slot it has
/// already resolved the item, so the guid is free.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellItemPicked {
    /// The item object's own guid, which is what `TARGET_FLAG_ITEM` carries.
    pub guid: u64,
}

#[derive(Message, Debug, Clone, Copy)]
pub struct SpellTargetPicked {
    /// The unit clicked, as an entity — the same handle [`Selection`] holds, so
    /// the cast path resolves it exactly as it resolves a selection.
    pub unit: Entity,
}

/// **…and the same click, landed on the floor** — where a placed cast goes.
///
/// Its own message rather than a variant of the one above for the reason that
/// keeps [`run_bindings`] the single door onto a cast: the two are written by
/// the same system but they are answers to two different questions, and the
/// world pick that produces a unit cannot produce a point (the ray it runs is
/// against pick boxes, not against the ground).
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellGroundPicked {
    /// The point on the floor, in **WoW axes and yards** — already the frame the
    /// wire wants. See [`Pointed::Ground`].
    pub at: [f32; 3],
}

/// **Is a cast in progress that a fresh press must not disturb?**
///
/// A wind-up, and only a wind-up. Named rather than written as
/// `casting.started.is_some()` at the two call sites because the *reason* is not
/// obvious from the field: it is `Spell::prepare`'s own gate,
/// `IsNonMeleeSpellCasted(withDelayed = false, skipChanneled = true,
/// skipAutorepeat = true)`, whose second and third arguments are why a channel
/// and an auto-shot do not count.
fn in_progress(casting: &Casting) -> bool {
    casting.started.is_some()
}

/// How long an unanswered `CMSG_CAST_SPELL` holds the button.
///
/// Every reply this can be waiting for is one packet's round trip — vmangos
/// answers inside the same `Spell::prepare` that reads the request — so this is
/// two orders of magnitude longer than the wait it is bounding, and it exists
/// only for the paths that answer *nothing*. See [`Casting::asked_at`], and
/// [`expire_the_ask`], which is the one reader.
const ASK_DEADLINE: Duration = Duration::from_secs(3);

/// **Let go of an ask the server never answered.**
///
/// Two of `HandleCastSpellOpcode`'s branches return with no reply at all — an
/// unknown spell id, and a spell the character does not have *active*, which a
/// superseded rank left on an action button is — so the pending record could be
/// armed for ever by one press. It gated every later press, which is the
/// "another action is in progress, stuck on the character" report; see
/// [`Casting::pending`] for the character it was measured on.
///
/// **The right fix is one directory over and is not this**: reading
/// `SMSG_SUPERCEDED_SPELL` so the bar never holds a dead rank in the first
/// place. This is the backstop for it and for every other silent drop, and it is
/// worth having on its own terms — a client that can be taken out for a session
/// by one unanswered packet is a client with no floor under it.
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

/// One spell's three timers, as the client's own `SpellHistory` node holds them.
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
    /// whichever timer is the reason.
    ///
    /// **Resolved against every record, not just this spell's**, which is the
    /// whole mechanism of a global cooldown: one cast's GCD is a record on *that*
    /// spell, and it gates this one because both name the same
    /// `startRecoveryCategory`. The longest remaining wins.
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

    /// Is this spell ready to press?
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

    /// The global cooldown, started locally at send — see the module comment.
    fn start_gcd(&mut self, spell: &SpellInfo) {
        if spell.gcd_ms == 0 {
            return;
        }
        let gcd = Some((Instant::now(), Duration::from_millis(u64::from(spell.gcd_ms))));
        self.entry(spell).gcd = gcd;
    }

    /// The spell's own recovery, when our own `SMSG_SPELL_GO` returns.
    ///
    /// Parked instead of started for the handful of spells whose cooldown begins
    /// when the effect breaks (`SPELL_ATTR_COOLDOWN_ON_EVENT`).
    ///
    /// `pub(crate)` because the packet that anchors it is read one
    /// directory up — see `world::incoming`'s `CastReleased` arm, which is the
    /// only caller and which took it over from a system in this file.
    /// `mods` is the character's talents, which shorten a good many of these —
    /// see [`crate::world::spellmods`]. It is applied to the spell's own
    /// recovery and not to the category's: `SMSG_SET_*_SPELL_MODIFIER`'s
    /// `COOLDOWN` operation is about the row and the shared category is a
    /// property of the group.
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

    /// The server's own statement, which overrides whatever was computed.
    ///
    /// **The categories come with it when they can**, and that is not a
    /// nicety. A record is created by whichever of the two paths reaches the
    /// spell first, and [`Self::entry`] is an `or_insert` — so a record this
    /// function made with `category: 0` was *permanently* categoryless, and
    /// from then on that spell's own category cooldown gated nothing else in
    /// its category. A school lockout names every spell of the school at once,
    /// so one interrupt was enough to flatten the categories of a whole
    /// school's worth of records for the rest of the session — which is the
    /// other half of the "school cooldowns display inconsistently" report, and
    /// the half that outlives the lockout.
    ///
    /// `None` for a spell the catalogue does not carry, which keeps the
    /// timer and loses only the grouping — the same degradation as before,
    /// now the exception rather than the rule.
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
        // **Filled in even on a record that already existed**, because the one
        // that was already there may be the categoryless one this function used
        // to make. Self-healing rather than only correct going forward.
        if let Some(spell) = spell {
            record.category = spell.category;
            record.gcd_category = spell.gcd_category;
        }
        record.recovery = timer;
        record.parked = None;
    }

    /// **`SMSG_CLEAR_COOLDOWN` — this spell's timers are over now.**
    ///
    /// All four of them, and the parked one with them: the server is saying the
    /// spell is ready, and leaving a category or GCD timer behind on the record
    /// would keep the swirl turning on every *other* spell that shares it. The
    /// record itself is kept rather than removed so the categories it carries
    /// survive for the next cast — see [`Self::set`], where losing them is the
    /// bug this is careful not to reintroduce.
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

/// Everything this module announces, in one param.
///
/// Bundled rather than listed per system because a verb writes three or four of
/// them and the signatures were the larger half of each function. Two systems
/// holding this are sequenced by Bevy (a writer is a `ResMut` underneath), which
/// is fine — they are chained anyway.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ActionEvents<'w> {
    pub slot_changed: MessageWriter<'w, ActionbarSlotChanged>,
    pub page: MessageWriter<'w, ActionbarPageChanged>,
    pub bonus: MessageWriter<'w, ActionbarBonusChanged>,
    pub cooldown: MessageWriter<'w, ActionbarUpdateCooldown>,
    /// **The spellbook's half of the same fact** — see [`Self::cooldown_moved`],
    /// which is the only thing that writes either.
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

/// **The three NPC windows' answers, in one param.**
impl ActionEvents<'_> {
    /// **A timer moved, and 1.12 says so under two names.**
    ///
    /// `ACTIONBAR_UPDATE_COOLDOWN` is what an action button listens for and
    /// `SPELL_UPDATE_COOLDOWN` is what a *spell* button listens for:
    /// `SpellButton_OnLoad` registers it (`SpellBookFrame.lua:209`) and
    /// `SpellButton_OnEvent` answers it with `SpellButton_UpdateButton()`, which
    /// is the only thing in the file that re-reads a cooldown. So a client that
    /// raised only the first has a spellbook whose swirls never move — which is
    /// exactly what it had, with the book coming right the moment it was closed
    /// and reopened or its page turned, because those go through
    /// `SpellBookFrame_Update` and rebuild every button unconditionally.
    ///
    /// A method rather than two lines at each of the six sites, because the
    /// failure mode is a *missing* line at one of them and nothing reports it —
    /// `SPELL_UPDATE_COOLDOWN` has been in [`super::events::FIRED`] for as long
    /// as it has existed, so `--audit --events` fires it and every count says
    /// the name is answered.
    pub(crate) fn cooldown_moved(&mut self) {
        self.cooldown.write(ActionbarUpdateCooldown);
        self.spell_cooldown.write(SpellUpdateCooldown);
    }

    /// …and the same thing from outside this module. See
    /// [`begin_item_cast`], which is the one caller.
    pub(super) fn timer_moved(&mut self) {
        self.cooldown_moved();
    }
}

/// **A right-click that casts a spell starts the global cooldown**, exactly as
/// pressing the spell would.
///
/// The item verbs in [`super::items`] send `CMSG_USE_ITEM` and change nothing
/// else in the session, so a potion's cooldown swirl did not start until the
/// server's own `SMSG_SPELL_COOLDOWN` came back a round trip later. That is not
/// a fault in the item path: it is the one local thing [`send_cast`] does that a
/// second way of starting a cast has to do too, which is why it is one function
/// here rather than a copy there.
///
/// **The bar is no longer part of it.** It used to be — this started the cast
/// bar and raised `SPELLCAST_START` for a bandage — and that half has moved to
/// where every other cast's now is, [`PlayerEvent::CastStarted`]. The server
/// sends `SMSG_SPELL_START` for an item's spell like any other non-triggered
/// cast, so the bar arrives on its own and, unlike this, it arrives only if the
/// server took the item.
pub(crate) fn begin_item_cast(
    info: &SpellInfo,
    cooldowns: &mut Cooldowns,
    events: &mut ActionEvents,
) {
    cooldowns.start_gcd(info);
    events.timer_moved();
}

/// This module's chain, so that what feeds it can order itself against the whole
/// of it rather than against whichever system it happens to read from.
///
/// [`super::spellbook`] is the one thing that does today: the book has to be
/// current before a `CastSpell` row is resolved against it.
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
                    // and the slot news it is followed by arrive in that order —
                    // `BonusActionBar_OnEvent` shows the frame and the buttons
                    // under it then read their own slots.
                    follow_bonus_bar,
                    // …and the *other* thing the character's own record says
                    // about which bars are on screen, which needs no ordering
                    // against anything: nothing reads it until the interface
                    // asks, and the interface asks once. See
                    // [`follow_bar_toggles`].
                    follow_bar_toggles,
                    // …and the third thing about the character that decides
                    // what a button looks like — see [`follow_usability`],
                    // which is about the *fade* where the two above are about
                    // which bar is on screen.
                    follow_usability,
                    // …and the fourth, which is the server's own statement
                    // rather than anything about the character — see
                    // [`follow_attack_state`], and note that every *press*-side
                    // write of the same event is a verb further down this
                    // chain.
                    follow_attack_state,
                    // …and the fifth, which is neither the character nor the
                    // server but *this client's own* half-finished press — see
                    // [`follow_cast_state`].
                    follow_cast_state,
                    rebuild_bar,
                    // The events before the input, so a cooldown the server just
                    // stated is in place before this frame's press is judged.
                    crate::world::incoming::drain_events,
                    // …and after them, because every one of the four replies
                    // that clears an ask arrives through the drain: expiring
                    // first would race a reply that is already in the queue.
                    expire_the_ask,
                    // **`cancel_cast` is gone from this list**, and that is the
                    // point rather than a tidy-up: it was a system reading
                    // `just_pressed(Escape)`, and Escape is `TOGGLEGAMEMENU` in
                    // the game's own defaults — so the key did two things and
                    // could not be rebound away from either. What cancels a
                    // cast now is `SpellStopCasting()`, an arm of
                    // [`run_bindings`] below.
                    run_bindings,
                )
                    .chain()
                    // **After the whole of the targeting chain**, because a cast
                    // binds against the selection and a swing goes at it: judged
                    // first, a press would use the previous frame's target. See
                    // `target::TargetSet`.
                    .after(super::target::TargetSet)
                    // …and after the key table has turned this frame's keys into
                    // binding names, since `run_bindings` reads those as messages
                    // and a message written later in the frame is read next frame.
                    .after(BindingSet)
                    .in_set(ActionSet)
                    .in_set(super::GameSet),
            )
            .add_systems(Update, forget.in_set(super::GameSet));
    }
}

/// **Drop the character's bar, cast and cooldowns when they leave the world.**
///
/// All three are that character's and none of them is corrected by simply
/// logging in as someone else: `rebuild_bar` re-resolves the slots only when the
/// spellbook version *differs*, and a cooldown is an `Instant` against a clock
/// that keeps running — so a mage's twelve buttons were still on screen behind
/// the character-select list, and a spell that was on a two-minute cooldown when
/// its owner logged out was still greyed out on the next character to hold that
/// slot. See [`super::events::PlayerLeavingWorld`].
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
    // …and the loop, whose `SMSG_CANCEL_AUTO_REPEAT` is not coming: the socket
    // it would have arrived on is the one that just closed. A hunter who logs
    // out shooting would otherwise log back in with the button flashing.
    *auto_repeat = AutoRepeat::default();
    // …and the cursor with them, or the next character logs in holding a spell
    // the previous one pressed.
    targeting.stop();
}

/// **Which twelve of the 120 slots the bar is showing**, off the character's
/// form.
///
/// The whole of `GetBonusBarOffset()`, and the reason it is a system rather than
/// a read at the call site is `UPDATE_BONUS_ACTIONBAR`: the interface does not
/// poll it. `BonusActionBarFrame` is **hidden at load** — its `OnLoad` asks once,
/// before there is a character to ask about, and after that it moves only on the
/// event — and `ActionButtonUp` routes the press through whichever of the two
/// frames is shown. So a client that never raises this has a bonus bar that
/// cannot appear and twelve buttons reading page one, which for a warrior or a
/// druid is empty.
///
/// Written on a change only, including the change from "no character" back to
/// zero: `forget` resets the bar and the next login must re-raise it.
fn follow_bonus_bar(
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut bar: ResMut<ActionBar>,
    mut events: ActionEvents,
    // The form as of the last look. Gating on it rather than on the *offset*
    // keeps the table lookup — a mutex and an `Arc` clone — off every frame,
    // and a form changes a few times a fight at most.
    mut seen: Local<Option<u8>>,
    // **And re-stated when the interface arrives.** This client loads FrameXML
    // a second *after* login, so the first announcement of everything is made
    // to an empty room — see `lua::host::load_bindings`, which raises
    // `PLAYER_ENTERING_WORLD` when the load finishes for exactly this reason.
    // Without re-announcing here, `BonusActionBarFrame` stays where its own
    // `OnLoad` left it: shown, but at the un-slid `y = 0` under the main bar.
    mut entering: MessageReader<super::events::PlayerEnteringWorld>,
) {
    let entered = entering.read().count() > 0;
    // **`None` while there is no character**, so logging back in as the same
    // class re-raises the event. `forget` clears the bar and a plain `u8` here
    // would compare equal to the form the last session ended in, leaving the
    // bonus bar hidden for the whole of the new one.
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
    // **And nothing is rebuilt with it.** A form changes which twelve of the
    // 120 the interface *reads*, not what is in any of them, and the reading is
    // `ActionButton_GetPagedID`'s — Lua's, not this side's. Forcing a rebuild
    // here used to be right while the empty page was filled from the spellbook
    // and is now 120 DBC lookups per stance dance for nothing.
    bar.bonus_bar = offset;
    events.bonus.write(ActionbarBonusChanged);
    // …and the twelve buttons themselves, which read their own slot rather than
    // being told it. `BonusActionBar_OnEvent` shows the frame; nothing in it
    // refills the buttons.
    events.slot_changed.write(ActionbarSlotChanged(ALL_SLOTS));
}

/// **Which of the four extra bars are on**, off the character's own record.
///
/// A copy and nothing else — one byte out of `PLAYER_FIELD_BYTES` into
/// [`ActionBar::toggles`], where `GetActionBarToggles()` can answer it. Every
/// interesting decision about the four bars is in the game's own files:
/// `UIParent.lua` asks this once per `PLAYER_ENTERING_WORLD`,
/// `MultiActionBar_Update` spends the four answers on four frames, and
/// `ActionButton_GetPagedID` works out that a `MultiBarBottomLeft` button is
/// slot `id + 60` from the *name of its parent*.
///
/// So this client owes the four bars exactly two things — this byte and the
/// packet that changes it ([`Binding::SetActionBarToggles`]) — and no layout at
/// all.
///
/// **Unconditional rather than change-gated**, unlike [`follow_bonus_bar`]:
/// there is no event to raise and therefore nothing to suppress, and a `u8`
/// copied from a snapshot every frame costs less than the `Local` that would
/// decide not to. `ActionBar::default()` is the answer with no character, which
/// is four bars off — the same picture a fresh account gets.
fn follow_bar_toggles(
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut bar: ResMut<ActionBar>,
) {
    let toggles = player.single().map_or(0, |me| me.action_bar_toggles);
    if bar.toggles != toggles {
        bar.toggles = toggles;
    }
}

/// **Tell the bar when the *server* changed the attack state**, which nothing
/// did.
///
/// `ActionButton_UpdateState` is what draws the checked border, it reads
/// `IsCurrentAction or IsAutoRepeatAction`, and `ACTIONBAR_UPDATE_STATE` is the
/// only event that re-runs it. Every write of that event in this file was
/// **press-driven** — the eight of them are all inside a verb — so the border
/// only ever redrew when the player clicked the button, and it then showed the
/// state as of that instant.
///
/// That is one bug wearing three faces, and all three were reported together:
///
/// ```text
/// right-click a mob to attack   SMSG_ATTACKSTART lands, nothing redraws
///                               -> the indicator does not come on
/// click the Attack button       the press raises the event, so the border
///                               finally draws the *old* truth and lights up —
///                               while the click itself toggles the attack off,
///                               because attack_target correctly saw it was on
/// the target dies or clears     SMSG_ATTACKSTOP lands, nothing redraws
///                               -> the indicator stays lit
/// ```
///
/// The toggle in [`attack_target`] was right the whole time; only the drawing
/// was wrong.
///
/// **`live.attacking()` and not `WorldEntity::engaged()`**, which is the same
/// fact through a narrower door: `engaged` is `attacking == target_guid()`, so
/// swinging at something that is no longer selected reads as false there. This
/// is the source `IsCurrentAction` itself answers from, so the button and the
/// event cannot hold two opinions.
///
/// The **auto-repeat** half needs nothing here: its every end already goes
/// through `AutoRepeat::stop`, which raises the state event beside
/// `STOP_AUTOREPEAT_SPELL` — see [`AutoRepeat`], where the reason the server's
/// `SMSG_CANCEL_AUTO_REPEAT` is the only statement on the wire is written down.
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
    // **Swinging at a *different* unit is not leaving combat**, so the pair is
    // raised on the presence changing rather than on the value: a target swap
    // mid-fight keeps the flash going, which is what the reference does.
    match (was.is_some(), now.is_some()) {
        (false, true) => {
            entered.write(super::events::PlayerEnterCombat);
        }
        (true, false) => {
            left.write(super::events::PlayerLeaveCombat);
        }
        _ => {}
    }
    // …and the border, on every change including the swap: `IsCurrentAction`
    // is about the *slot*, and a client that swings at somebody else has the
    // same answer — but it costs one message and removes a case to reason
    // about.
    state.write(ActionbarUpdateState);
    *was = now;
}

/// **…and when the press that is still in the air changes** — the cast in
/// flight, and the cursor waiting to be pointed.
///
/// [`crate::interface::api::is_current_action`] answers off both, and a border that
/// is only *raised* by the press that started it never goes out: a cast ends at
/// `SMSG_SPELL_GO`, at an interruption or at a refusal, and a cursor ends at a
/// click, an Escape or a spell that could not be aimed — six endings, none of
/// which is the button being pressed again. Watching the pair is one system
/// against six, and it cannot be forgotten by the seventh.
///
/// The same shape as [`follow_attack_state`] above and for the same reason: a
/// `Local` holding what was last reported, and one message on a difference.
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
    // **Not on the first frame**, which is what the `Option` around the pair
    // buys: a session opens with both empty, and raising the event for that
    // would re-run every visible button's state on the frame the bar is built.
    if was.is_some() {
        state.write(ActionbarUpdateState);
    }
    *was = Some(now);
}

/// **Tell the bar when the *conditions* moved**, which is a different question
/// from when the player's power moved.
///
/// `ACTIONBAR_UPDATE_USABLE` is the one event that re-runs
/// `ActionButton_UpdateUsable`, and `super::vitals` raises it
/// on the player's own power — which was the whole of what decided a fade until
/// `SpellInfo::castable_now` arrived. Now three more things decide it and none
/// of them is power:
///
/// * **the aura state** — a Seal landing is what makes Judgement pressable, and
///   a block is what makes Revenge pressable;
/// * **combo points** — every finisher on a rogue's bar;
/// * **the form** — a warrior changing stance re-gates half their abilities.
///
/// Without this the button stays as it was until something *else* raised the
/// event, which in combat is the next power tick: the fade would be right most
/// of the time and late the rest, which reads as "sometimes it works". That is
/// the same shape as the report this round came from, so it is worth a system
/// of six lines rather than a note.
///
/// **A `Local` rather than change detection**, because the fields live on a
/// `WorldEntity` the poll rewrites wholesale every tick — `Changed` there is
/// every tick and says nothing.
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
    // **Not on the first sight of it.** The first value is recorded rather than
    // raised on, so logging in does not cost a pass over every registered
    // button before the interface is up.
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
    // **And which parse of the DBCs the slots were resolved from.** The bar's
    // names, icons and cast times come out of `Spell.dbc`, so a bar built
    // before the tables were forgotten is stale in a way no packet states —
    // see `GameAssets::tables_generation`. Without it an edited spell kept its
    // old picture on the bar until the slot itself changed, which is what the
    // server's version tracks.
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
        // A passive is in the spellbook and cannot be cast — the server refuses
        // one outright — and 4,929 of the game's spells are passive.
        //
        // …and a `DO_NOT_DISPLAY` spell is in neither: the client's *castable*
        // list makes the same sign-bit test on the record's `Attributes` byte
        // that the spellbook add does. Without
        // it the character's GM and world-buff spells are in this list, under
        // `Interface\Icons\Temp`.
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

    // **The fallback fill is gone, and this note is what stood here.** For six
    // rounds the empty slots of the visible page were filled from the
    // character's spellbook, because until a spell could be *dragged* onto the
    // bar a fresh character's empty bar was a client that could do nothing at
    // all and could not be tested against the server. That deviation was
    // written down as one to delete rather than port, and this is the round
    // that deletes it: [`super::cursor`] carries a spell or an item onto a
    // button and `CMSG_SET_ACTION_BUTTON` remembers it.
    //
    // What the bar shows now is `SMSG_ACTION_BUTTONS` and nothing else — the
    // server's own `playercreateinfo_action` for a new character, and whatever
    // the last session left for an old one — which is the real client exactly.

    // One message for the whole bar rather than twelve — `ActionButton.lua`'s own
    // `arg1 == 0` convention, see `events::ActionbarSlotChanged`.
    events.slot_changed.write(ActionbarSlotChanged(ALL_SLOTS));
}


// **Where the cast bar used to end.** `finish_casts` was a system polling the
// player entity's release counter and then reading `last_spell` to find out
// which spell had landed — and it lost that read to any release arriving in the
// same poll after ours, which wedged the character for the rest of the session.
// It is `crate::world::incoming`'s `PlayerEvent::CastReleased` arm now, which is
// per packet and carries its own id; the arm says what it cost.

/// **Escape**, which is deliberately not a binding.
///
/// `Bindings.xml` declares no Escape binding but `TOGGLEGAMEMENU`; cancelling a
/// cast and clearing a target are both the client's own. It cancels the cast
/// *before* clearing the target — a wind-up is the more urgent of the two — and
/// `target::select_on_click` checks `Casting::started` for the same reason rather
/// than an ordering between them.
/// **Cancel the wind-up, the ask or the channel** — `SpellStopCasting()`'s own
/// half, and the whole of what Escape used to be a raw-key system for here.
///
/// A free function rather than a system now: the *key* is `TOGGLEGAMEMENU`'s
/// business and its body is what calls the verb, so this runs from
/// [`run_bindings`]' arm like every other write. What was lost with the system
/// is nothing — `just_pressed(Escape)` was this client's own reading of a key
/// the game already declares a binding for, and having both meant the key did
/// two things.
fn stop_casting(
    active: &crate::world::session::ActiveSession,
    casting: &mut Casting,
    events: &mut ActionEvents,
) {
    // A channel is cancellable too, and it ends through its own event — see
    // `drain_events`' interrupt arm for why the ordinary stop would not show.
    let channelled = casting.channelling.is_some();
    // **A cast that has been asked for and not yet answered is cancellable
    // too**, and it has to be: since the bar waits for `SMSG_SPELL_START`,
    // there is now a round trip in which Escape used to work and would
    // otherwise do nothing at all — the packet would arrive, the bar would go
    // up, and the keypress that meant to stop it is already spent. The cancel
    // is ordered behind the cast on the same socket, so the server sees them in
    // the order they were pressed.
    let pending = casting.pending.take();
    if !channelled && casting.started.is_none() && pending.is_none() {
        return;
    }
    active.live.cancel_cast(pending.unwrap_or(casting.spell_id));
    // **The events follow the bar and not the keypress.** `end` answers whether
    // there was a wind-up; a channel has none and stops through its own event
    // either way; and a cancelled *ask* has neither, so it raises nothing — a
    // stop over a bar that never appeared is what `CastingBarFrame_OnEvent`
    // would colour and fade.
    let had_bar = casting.end();
    if channelled {
        events.channel_stop.write(SpellcastChannelStop);
    } else if had_bar {
        events.cast_stop.write(SpellcastStop);
    }
}

/// **What the player asked for this frame**, from the two doors it can arrive
/// through.
///
/// One parameter rather than two, for the same reason [`ActionEvents`] is one:
/// [`run_bindings`] is at Bevy's sixteen and the two are read in adjacent lines
/// at the top of it. The verbs and the spell cursor's answer are genuinely one
/// subject — both are "the player pointed at something" — and both are cleared
/// together when there is nobody to act.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Asked<'w, 's> {
    pressed: MessageReader<'w, 's, BindingPressed>,
    /// **The second door onto a cast, and it is a click rather than a key** —
    /// see [`SpellTargetPicked`]. Read here rather than acted on where it is
    /// written, so that every cast in the client still goes through one place.
    picked: MessageReader<'w, 's, SpellTargetPicked>,
    /// …and the third door, which is a bag square — see [`SpellItemPicked`].
    item_picked: MessageReader<'w, 's, SpellItemPicked>,
    /// …and the fourth, which is the **trade window's enchant square** — see
    /// [`crate::interface::trade::TradeSlotPicked`]. The same gesture as
    /// the bag square, aimed at a slot number rather than at an item.
    trade_picked: MessageReader<'w, 's, crate::interface::trade::TradeSlotPicked>,
    /// **…and the cursor put up by an *item*** rather than by a press — a
    /// sharpening stone right-clicked in the bag. See
    /// [`crate::interface::items::BeginItemTargeting`], which is written
    /// where the stone's own place is remembered.
    begin_item: MessageReader<'w, 's, crate::interface::items::BeginItemTargeting>,
    /// …and the same door with a place behind it — see [`SpellGroundPicked`].
    placed: MessageReader<'w, 's, SpellGroundPicked>,
}

impl Asked<'_, '_> {
    /// Throw the frame away — a binding pressed on the login screen must not
    /// fire on the first frame in the world.
    fn clear(&mut self) {
        self.pressed.clear();
        self.picked.clear();
        self.placed.clear();
    }
}

/// Run whatever bindings fired this frame.
///
/// **This is the whole of the input path for an action**, and it names no key:
/// the key table turned a press into a [`Binding`] and this turns a `Binding`
/// into a verb, which is the same two steps `Bindings.xml` and the C API make.
#[allow(clippy::too_many_arguments)]
fn run_bindings(
    mut asked: Asked,
    session: Res<Session>,
    assets: Res<GameAssets>,
    bar: Res<ActionBar>,
    book: Res<super::spellbook::Spellbook>,
    // **Read, never written** — the one question `UseAction`'s `checkCursor`
    // asks, and the reason this module can answer half of a verb whose other
    // half is [`super::cursor`]'s.
    cursor: Res<super::cursor::Cursor>,
    selection: Res<Selection>,
    // **The caster's own `Transform` as well as its record**, because the range
    // check below measures the drawn positions rather than the last snapshot —
    // the local player is dead-reckoned ahead of the session thread and the
    // difference at a run is a fifth of a yard a frame.
    player: Query<(Entity, &WorldEntity, &Transform, &Sheath), With<LocalPlayer>>,
    units: Query<(&WorldEntity, &Transform)>,
    // **Every unit token, resolved the one way** — the party frames' clicks
    // arrive as `SpellTargetUnit("party2")` and there is exactly one function
    // in this client that knows what that names. See the arm below.
    tokens: crate::interface::api::Units,
    mut press: PressState,
    mut errors: UiErrors,
    mut events: ActionEvents,
    mut sheathing: MessageWriter<SheathRequest>,
    // **An item slot's own outlet** — see [`use_action`], where the one line
    // that writes it says why the doing is `interface::items`' rather than this
    // module's.
    mut items: MessageWriter<super::items::UseCarriedItem>,
    // **The settings, the hour and the sky overhead**, bundled — see
    // [`crate::interface::api::Surroundings`], whose own note is that this system was
    // at Bevy's sixteen-parameter ceiling and the settings were already here.
    // The settings are read for exactly one flag ([`self_cast`] below); the
    // other two are the cast rule's, and reach it through `caster_conditions`.
    around: crate::interface::api::Surroundings,
) {
    let Some(active) = session.active.as_ref() else {
        // Drain anyway: a binding pressed on the login screen must not fire on
        // the first frame in the world.
        asked.clear();
        return;
    };
    // **Off the param this system already holds**, rather than two more `Res`
    // beside it — see [`crate::interface::api::Units::friendship`].
    let friendship = tokens.friendship();
    // …and the other half of what a press is judged against, off the same
    // bundle the settings come from — see [`crate::interface::api::CastWorld`].
    let world = around.cast_world();
    let Ok((entity, me, here, sheath)) = player.single() else {
        asked.clear();
        return;
    };
    // **Who the spell cursor was pointed at**, from either of its two doors: the
    // world pick (`target::select_on_click`, as a message) and a unit frame
    // (`SpellTargetUnit`, as a binding, collected in the loop below). Gathered
    // first and cast last, so that a key pressed in the same frame — a fresh
    // intention — is judged before the answer to a question the client asked.
    // **`autoSelfCast`, which is the standing form of the `SELFACTIONBUTTON`
    // flag.** The binding turns it on for one press; the CVar turns it on for
    // every press, and it is the *same input* — the client has a single branch
    // for both and there is no second one. So it is or-ed in
    // here rather than plumbed through [`cast_known_spell`] as a second bool:
    // one of them being true is the whole condition.
    //
    // Read per frame rather than cached because a CVar can be written by any
    // script between two presses, and the read is a hash lookup.
    let self_cast = around.cvars().flag(vale_assets::tables::spellbook::AUTO_SELF_CAST);
    // **A stone asked its question first.** Read before the presses so a cursor
    // put up by an item is standing by the time anything else in this frame
    // looks at it, and drained either way so it cannot fire late.
    if let Some(begun) = asked.begin_item.read().last() {
        press.targeting.begin(begun.0, Asking::Item);
    }
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
            // **The click's own half of `UseAction(slot, 1)`** — the *use*, and
            // only when nothing is being carried. The other branch belongs to
            // [`super::cursor`], which drops what is held into the slot; the two
            // conditions are exclusive, so neither module has to know the other
            // ran. See [`Binding::UseOrPlaceAction`], where the client's own
            // four-way test is.
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
            // **A row of the book, not a spell id** — see
            // [`Binding::CastSpellbookRow`]. A row the book does not have is
            // dropped silently rather than reported: the panel is what composed
            // the index, so an out-of-range one is this client's arithmetic
            // being wrong and not something to tell the player about.
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
            // **A recipe, which is a spell id rather than a row** — the panel
            // resolved it against the list it drew. The cast is the ordinary
            // pipeline: a potion self-targets through the aiming rule and an
            // enchant asks for an item exactly as a bag-cast formula does. The
            // count is not this arm's: the repeat counter reads the same
            // message in [`super::tradeskill`].
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
            // **The spell cursor, clicked through a unit frame rather than
            // through the world.** `TargetFrame_OnClick` casts at `"target"` and
            // a party frame at its own unit; the world's own click arrives as
            // [`SpellTargetPicked`] above and both end in the same place.
            //
            // Only the two tokens this client can resolve without the `Units`
            // parameter this system does not hold — which is `"player"` and
            // `"target"`, and between them they are every call site in the
            // directory that a client with no party can reach.
            // **Any token, through the one resolver** — see
            // [`crate::interface::api::Units::resolve`].
            //
            // This answered for `player` and `target` and nothing else, which
            // is the whole of "positive click spells do not work on party
            // member frames": `PartyMemberFrame_OnClick` is
            // `if SpellIsTargeting() then SpellTargetUnit("party"..id)`, and a
            // `party1` that resolved to nothing dropped the click on the floor
            // — the cursor stayed up, no packet went out, and no error was
            // said. The same two lines are in `UnitFrame.lua`,
            // `PartyMemberPetFrame_OnClick` and `TargetFrame_OnClick`, so the
            // gap was every unit frame in the game except two.
            //
            // `Units::resolve` is the function that already knows `party3` is a
            // guid in [`crate::interface::party::Party`] and that `pet`
            // means charm-before-summon; reimplementing a subset here is what
            // produced the subset.
            Binding::SpellTargetUnit(token) => {
                if let Some(unit) = tokens.resolve(*token) {
                    pointed_at.push(Pointed::Unit(unit));
                }
            }
            Binding::SpellStopTargeting => press.targeting.stop(),
            // **`SpellStopCasting()`** — Escape's own, and the *whole* of what
            // this used to be a `just_pressed(Escape)` system for. The reading
            // of "is there a cast" is made in the interface (see
            // `ActionAnswers::spell_is_casting`), so by the time this arrives
            // the answer has already been given; what is left is the doing.
            Binding::SpellStopCasting => {
                stop_casting(active, &mut press.casting, &mut events);
            }
            Binding::ToggleSheath => toggle_sheath(entity, sheath, me, &mut sheathing),
            // **Nothing to store**: the page lives in the interface's own
            // `CURRENT_ACTIONBAR_PAGE` and every button adds it to its own id
            // before it asks this client anything, so the twelve slot numbers
            // that arrive here are already absolute. What the C side owes is
            // the news — see [`Binding::ChangeActionBarPage`].
            Binding::ChangeActionBarPage => {
                events.page.write(ActionbarPageChanged);
            }
            // **…and the other half of the same subject, which is the exact
            // opposite shape.** The page is the interface's and this client only
            // announces it; the four extra bars are the *server's*, so what the
            // C side owes here is a packet and no news at all — the options
            // panel has already shown and hidden its own frames by the time this
            // arrives. See [`Binding::SetActionBarToggles`] and
            // [`ActionBar::toggles`], which is where the answer comes back.
            Binding::SetActionBarToggles(mask) => {
                active.live.set_actionbar_toggles(*mask);
            }
            // Targeting verbs are `target.rs`'s — including
            // `Binding::TargetToken`, the party frame's click — and the buff
            // cancel is
            // `auras.rs`'s — each reads the same messages. Only the module that
            // holds the state a verb needs can resolve it; see
            // [`Binding::CancelPlayerBuff`], whose handle indexes a list this
            // one does not have.
            Binding::TargetNearestEnemy
            | Binding::TargetPreviousEnemy
            | Binding::TargetSelf
            | Binding::TargetToken(_)
            | Binding::CancelPlayerBuff(_)
            // …and the four about leaving, which are `interface::logout`'s — plus
            // the reset, which shares that file for the reason its own arm
            // there gives.
            | Binding::ResetInstances
            | Binding::Logout
            | Binding::Quit
            | Binding::CancelLogout
            | Binding::ForceQuit
            // …and the one that leaves nothing at all: `lua::host`'s, which
            // rebuilds the interpreter under the world rather than touching it.
            | Binding::ReloadUI
            // …and the five about dying, which are `interface::death`'s.
            | Binding::RepopMe
            | Binding::RetrieveCorpse
            | Binding::AcceptResurrect
            | Binding::DeclineResurrect
            | Binding::AcceptXPLoss
            // …and the innkeeper's, which is `interface::binder`'s: it needs
            // the guid that asked, and only that module is holding one. The pet
            // trainer's Accept is `interface::untrainer`'s for the same reason.
            | Binding::ConfirmBinder
            | Binding::ConfirmPetUnlearn
            // …and the summon's Accept and `/played`, which are
            // `interface::summon`'s and `interface::played`'s.
            | Binding::ConfirmSummon
            | Binding::RequestTimePlayed
            // …and the two about right-clicking an item, which are
            // `interface::items`' — the item's own prototype is what decides whether
            // the click uses it or wears it, and only that module holds one.
            | Binding::UseContainerItem { .. }
            | Binding::UseInventoryItem(_)
            // …and the six about *carrying* one, which are `interface::cursor`'s for
            // the same reason: what a left click means depends on whether the
            // pointer is already holding something, and only that module knows.
            | Binding::PickupContainerItem { .. }
            | Binding::PickupInventoryItem(_)
            | Binding::SplitContainerItem { .. }
            | Binding::PutItemInContainer(_)
            | Binding::AutoEquipCursorItem
            | Binding::DeleteCursorItem
            // …and the four about *filling* the bar, which are the same
            // module's for the same reason: `PickupAction` is a pick-up or a
            // place depending on what is already carried, and only the cursor
            // knows. This module owns what a slot *does*; it does not own what
            // is in one.
            | Binding::PickupSpellbookRow(_)
            | Binding::PickupAction(_)
            | Binding::PlaceAction(_)
            | Binding::ClearCursor
            // …and the character's own controls, which are
            // `input::controls`': what a movement key means is a *held
            // state* the mover reads every tick, and this module is about
            // edges. `Jump` is an edge and still not this module's, for the
            // same reason `UseContainerItem` is not: the thing that owns the
            // socket owns the packet.
            // …and the selection's own, which is `combat::target`'s: this
            // module owns what a slot does, not who is selected.
            | Binding::ClearTarget
            | Binding::Control(_, _)
            | Binding::Jump
            | Binding::SitOrStand
            | Binding::ToggleAutoRun
            | Binding::ToggleRun
            | Binding::FollowUnit(_)
            // …and the one about the camera, which is `world::camera`'s, and
            // the one about the window, which is the crate root's.
            | Binding::CameraZoom(_)
            // …and the *pet's* bar, which is a different bar with a different
            // packet — see [`super::pet`], which drains the same queue.
            | Binding::CastPetAction(_)
            | Binding::TogglePetAutocast(_)
            // …and the drag within it, which is `combat::cursor`'s dispatch and
            // `combat::pet`'s work.
            | Binding::PickupPetAction(_)
            // …and the *stance* bar, which is a third bar again — see
            // [`super::shapeshift`], which drains the same queue.
            | Binding::CastShapeshiftForm(_)
            | Binding::PetAttack
            | Binding::PetStopAttack
            | Binding::PetAbandon
            // …and the pointer's own, which is `combat::cursor`'s — see
            // [`super::cursor::Cursor::asked`].
            | Binding::AskCursor(_)
            | Binding::Screenshot => {}
        }
    }

    // **…and the spell cursor's answer, last.** The spell is re-looked-up here
    // rather than carried on the pick, because a press in the loop above may
    // have replaced or cancelled what was waiting — and a stale id would cast a
    // spell the player has already moved on from.
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

/// `ToggleSheath()` — draw what is equipped, or put it away.
///
/// **Which of melee and ranged it draws is the wardrobe's answer, not a
/// toggle's**: a hunter with a bow and no melee weapon draws the bow. Stowed is
/// always stowed, so the cycle is two-state and the *choice* only happens on the
/// way out.
///
/// A request rather than a write — see [`SheathRequest`], and
/// `world::entities::sheath` for why there is exactly one executor. This is also
/// the one path in the client that is *supposed* to play a draw/stow animation
/// and does not; that deviation is stated in `entities::sheath`'s own comment
/// rather than here, because it belongs to the executor.
fn toggle_sheath(
    entity: Entity,
    sheath: &Sheath,
    me: &WorldEntity,
    sheathing: &mut MessageWriter<SheathRequest>,
) {
    use vale_assets::look::sheath as policy;
    let state = if sheath.state() == policy::UNARMED {
        // Melee first: the client's own preference, and the only case where the
        // ranged slot wins is a character with nothing else to draw.
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

/// `UseAction(slot, _, onSelf)` — the game's own verb, one-based.
///
/// `on_self` is the game's `SELFACTIONBUTTON` flag **or its standing form, the
/// `autoSelfCast` CVar** — see [`run_bindings`], which or-s the two together
/// because `BindTarget`'s arm has one branch for both. It is passed straight to
/// [`resolve_aim`], so holding Alt (or ticking the box) and pressing a heal with
/// an enemy selected heals you, rather than being refused.
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
    // **The roster and the character's own reputation**, which are half of
    // friend-or-foe and which the aiming rule cannot reach without — see
    // [`crate::interface::api::Friendship`]. Without them a spell aimed at a
    // free-for-all player of your own faction binds them as a friend, which
    // is the client refusing a cast the server would have allowed.
    friendship: crate::interface::api::Friendship<'_>,
    // …and the hour and the sky, which are the caster-state chain's own
    // inputs — see [`crate::interface::api::CastWorld`].
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

    // **The auto-attack toggle, by the *kind* of slot rather than by the spell**
    // — an item whose entry happens to be 6603 is not the Attack button. The
    // same decision by spell id alone is in [`cast_known_spell`], which is where
    // a slot holding no `SpellInfo` cannot reach.
    if action.is_auto_attack() {
        attack_target(active, selection, units, me, entity, sheathing);
        events.state.write(ActionbarUpdateState);
        return;
    }
    // **An item is a use, not a cast** — and the whole of what this side owes it
    // is the entry, because `SMSG_ACTION_BUTTONS` carries nothing else. Where
    // that entry *is* is the inventory's question and the doing is the
    // right-click's own body; see [`super::items::UseCarriedItem`], which is why
    // this is one line rather than a second copy of the equip-or-use split.
    //
    // Silent when the character is not carrying one, which is the reference's
    // behaviour: `UseAction` finds nothing and returns, and the button is drawn
    // without an icon in the first place.
    if action.kind == action_kind::ITEM {
        items.write(super::items::UseCarriedItem(action.action));
        return;
    }
    let Some(info) = action.spell.as_ref() else {
        // A macro, or a spell `Spell.dbc` does not carry. Neither is implemented
        // and neither should be sent blind.
        errors.key("SPELL_FAILED_SPELL_UNAVAILABLE");
        return;
    };
    cast_known_spell(
        info, on_self, None, active, assets, selection, units, me, friendship, world, here, entity,
        cooldowns, casting, auto_repeat, targeting, errors, events, sheathing,
    );
}

/// **Cast a spell this character knows**, from wherever it was reached.
///
/// Split out of [`use_action`] when the spellbook panel arrived, because
/// `CastSpell(id, bookType)` is a second door onto exactly this: the cooldown
/// refusal, the aiming rule and the send are the same three steps, and the only
/// thing that differs between a bar press and a spellbook click is how the
/// [`SpellInfo`] was found. Two copies of the aiming call is two places for a
/// self-buff to start shipping the selection — which is the failure
/// [`resolve_aim`] exists to prevent and the one that reads as a server bug.
///
/// ## The two the server answers with *silence*
///
/// `SpellButton_OnClick` calls `CastSpell(id, bookType)` for **every** row it is
/// pressed on — it has no passive test and no attack test of its own, so both
/// belong here, on the C side, exactly as the real client has them. Getting that
/// wrong is not a refusal a player can read, because
/// `WorldSession::HandleCastSpellOpcode` **drops the packet without replying**:
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
/// So with this client's prediction on top (the cast is drawn at the press, see
/// [`send_cast`]) the whole visible outcome of clicking a passive was **a cast
/// animation followed by nothing at all**, with no error line and nothing in any
/// log. 42 of this test character's 137 spells are passive.
///
/// **And Attack is the same shape for a different reason.** Spell 6603 is not
/// passive and *is* in the book, so it passes the branch above and the server
/// tries to cast it — the client never sends it, because pressing Attack is a
/// swing (`CMSG_ATTACKSWING`). [`use_action`] has always known that about the
/// *bar*; the spellbook's General page carries the same pseudo-spell and had no
/// such test, so clicking it there wound up, released and did nothing.
///
/// ## …and the third door, which is a click
///
/// `pointed` is what the player **pointed at** with the targeting cursor — a
/// unit, or a patch of floor — and it stands in for the selection for this one
/// press. It is `Some` only on the way back from [`SpellTargeting`], and it
/// changes exactly two things: the candidate the aiming rule is offered, and
/// what an unbindable answer means — the cursor has already been up once, so a
/// second `WantsTarget` is the player pointing at something the spell cannot
/// have and gets the client's own "Invalid target" rather than another round of
/// asking.
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
    // …and the same pair [`use_action`] carries, for the same reason.
    friendship: crate::interface::api::Friendship<'_>,
    // …and the hour and the sky, which are the caster-state chain's own
    // inputs — see [`crate::interface::api::CastWorld`].
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
    // **An auto-repeat pressed while it is already running is a *stop*, and
    // that is the whole of what the button does the second time.** It goes
    // before every check below — the cooldown, the aiming rule, the range —
    // because none of them is a question about turning something off, and a
    // hunter whose target has walked away would otherwise be told "out of
    // range" instead of being allowed to stop shooting.
    //
    // **Nothing is cleared here**: `CMSG_CANCEL_AUTO_REPEAT_SPELL` is answered
    // with `SMSG_CANCEL_AUTO_REPEAT`, and [`AutoRepeat::stop`] runs off *that*
    // so the press and the four ends the client could not have predicted are
    // one code path. See [`AutoRepeat`].
    if info.is_auto_repeat_ranged() && auto_repeat.is(info.id) {
        active.live.cancel_auto_repeat();
        return;
    }
    match press_kind(info) {
        // **Attack is a swing wherever it is pressed from** — see the doc
        // comment above.
        PressKind::Swing => {
            attack_target(active, selection, units, me, entity, sheathing);
            events.state.write(ActionbarUpdateState);
            return;
        }
        // **Refused silently**, which is the reference's own behaviour rather
        // than a shortcut: the real client draws a passive's button greyed and
        // pressing it produces no message, no sound and no animation. There is
        // no `GlobalStrings.lua` key for "that spell is passive" to print, and
        // the server would not have sent one either — it drops the packet.
        PressKind::Refused => return,
        PressKind::Cast => {}
    }

    // **A press on top of a cast already running.** `Spell::prepare` opens with
    // `IsNonMeleeSpellCasted(false, true, true)` and answers
    // `SPELL_FAILED_SPELL_IN_PROGRESS`, so the packet buys nothing.
    //
    // **Repeating the spell that is already going out is dropped without a
    // message**, and that half is a judgement rather than a reading: the server
    // would answer with one, but "press the key again" is a player saying the
    // same thing twice rather than asking for a second action, and a line of
    // error text per keypress is not what the reference shows. A *different*
    // spell gets the server's own words, because that is a genuinely refused
    // action and the player is owed the reason.
    //
    // **And "already running" now includes "already asked for"**, which is the
    // half that closes the spam. Since a cast is no longer drawn until
    // `SMSG_SPELL_START` arrives, the window between the packet leaving and the
    // answer landing is one in which `in_progress` is false and every repeat
    // press would put another `CMSG_CAST_SPELL` on the wire. The reference does
    // send those and lets the server refuse each one; dropping the repeat here
    // is the same outcome for the player, with one packet instead of ten.
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
    // client's own message — which the server would otherwise send back as
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
    // **What the aiming rule is offered**: the unit the player pointed at if
    // there was one, and the selection otherwise. A click with the targeting
    // cursor up does *not* change the selection (the reference does not either),
    // so this is a substitution for the press rather than a write.
    let selected = match pointed {
        Some(Pointed::Unit(unit)) => Some(unit),
        // A *place* is not a candidate for the unit binder, and it must not
        // suppress the selection either: the ground branch below runs before the
        // selection is ever tried, so what this offers is only what the aiming
        // rule would have seen on the original press.
        // An item is not a candidate for the unit binder either, for the same
        // reason a place is not: the item branch runs before the selection is
        // ever tried, so what this offers is only what the aiming rule would
        // have seen on the original press.
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

    // **`on_self` is the CVar or the binding** — see [`run_bindings`]. With
    // neither, a friendly cast with an enemy selected is refused rather than
    // redirected onto the caster, which is 1.12's own default (`autoSelfCast`
    // registers as `"0"`).
    // **The main hand, which the client picks rather than asking for.** See
    // `CastAim::Item`: a spell carrying `SPELL_ATTR_HELD_ITEM_ONLY` binds the
    // weapon itself and says "Your weapon hand is empty" when there is none.
    let aim = resolve_aim(info, selected, myself, on_self, me.main_hand_item);
    let target = match aim {
        CastAim::SelfImplicit => CastTarget::SelfImplicit,
        CastAim::Unit(guid) => CastTarget::Unit(guid),
        // **Nothing bound, and the client asks rather than complains.** This is
        // the whole of "a heal pressed with nothing selected": the reference
        // puts up the targeting cursor and waits for a click, and printing "No
        // target" instead was the report. See [`SpellTargeting`] — the cursor
        // is put away and the answer becomes a refusal once the player has
        // already been asked, which is what `aim_at` says.
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
        // **…and the same question about a place.** The answer is three floats
        // and no guid — see [`CastTarget::Dest`], and note that a *unit* pointed
        // at a placed spell is not an answer at all: `SpellTargetUnit` through a
        // unit frame reaches here with `Pointed::Unit`, and the honest response
        // is to keep asking rather than to send a cast the server would put at
        // the caster's feet.
        CastAim::WantsGround => match pointed {
            Some(Pointed::Ground(at)) => CastTarget::Dest(at),
            _ => {
                targeting.begin(info.id, Asking::Ground);
                return;
            }
        },
        // **`TARGET_FLAG_ITEM` and the weapon's own guid**, with no cursor: the
        // reference picks the main hand itself for every spell carrying the
        // held-item bit, which is Rockbiter, Windfury, the poisons and the
        // sharpening stones. It was this branch's absence that made all of
        // them "Invalid target" — the word fell past `UNIT_FAMILY` into the
        // local refusal.
        CastAim::Item(guid) => CastTarget::Item(guid),
        // …and the residue: an enchanting formula, which 1.12 aims by clicking
        // a bag slot. There is no item cursor here yet, so it takes the unit
        // cursor's second-press behaviour — asked once, refused after — rather
        // than pretending to send something.
        // …and the residue, which is an enchanting formula, a poison or a
        // sharpening stone: 1.12 aims those by clicking a bag square, and the
        // answer comes back through [`SpellItemPicked`]. Same three-way shape
        // as the unit cursor — ask once, and refuse the second time, which is
        // the player pointing at something the spell cannot have.
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
        // Refused before it reaches the socket, with the client's own message.
        CastAim::Refused(key) => {
            targeting.stop();
            errors.key(key);
            events.cast_failed.write(SpellcastFailed);
            return;
        }
    };
    // Whatever the cursor was holding is spent: the press that got here is the
    // one it was waiting for, or a fresh press that outranks it.
    targeting.stop();

    // **…and the conditions the aiming rule does not cover**: am I alive, can I
    // pay for it, is that unit within reach. See
    // [`vale_assets::tables::spellbook::check_cast`], which is the rule; what is here
    // is only the measuring, and the one measurement worth reading twice is the
    // distance — **surface to surface**, because that is what the server's own
    // range check is written against.
    let distance = match target {
        CastTarget::Unit(guid) if guid != me.guid => reach_between(units, me, here, guid),
        // **A placed cast measures to the point, centre to point** — vmangos'
        // `CheckRange` ends in a plain `IsWithinDist3d` against the destination
        // with no reach subtracted at either end, which is a different
        // measurement from the unit branch above and not a special case of it.
        CastTarget::Dest(at) => {
            Some(here.translation.distance(crate::render::axes::to_bevy(at)))
        }
        _ => None,
    };
    // **…and whether whoever it bound is alive**, which is the one condition the
    // aiming rule cannot answer: `unsatisfied` tests relations, and a corpse is
    // still hostile. Read off the same two candidates the binder was handed, so
    // the state judged here is the state it bound. A guid that is neither — which
    // nothing can currently produce — asks nothing, which errs towards sending.
    let target_dead = match target {
        CastTarget::Unit(guid) if guid == me.guid => Some(me.dead),
        CastTarget::Unit(guid) => selected.filter(|who| who.guid == guid).map(|who| who.dead),
        _ => None,
    };
    // **The caster's half is one function for both doors** — see
    // [`super::api::caster_conditions`], which is where the item door
    // and this one were made to agree.
    let conditions = super::api::caster_conditions(me, info, distance, target_dead, world);
    if let Some(key) = vale_assets::tables::spellbook::check_cast(info, &conditions) {
        let key = vale_assets::tables::spellbook::failure_override(key, info.power_type).unwrap_or(key);
        errors.key(key);
        events.cast_failed.write(SpellcastFailed);
        return;
    }

    // **A next-swing spell is queued, not cast** — the packet goes and the
    // animation does not, because the swing that discharges it has not happened
    // yet. See [`LiveSession::cast_on_next_swing`].
    if info.on_next_swing() {
        // **Pressing the armed ability again is dropped**, which is the same
        // judgement the `in_progress` block above makes one screen up and the
        // same reason: the player is saying the same thing twice rather than
        // asking for a second action. It is also the one ordering this state
        // cannot survive without — vmangos' `SetCurrentCastedSpell` interrupts
        // the queued spell before it accepts the new one, so a second press
        // sends `SPELL_FAILED_INTERRUPTED` *for the same spell id* and the
        // clear below would put the border out on an ability that is still
        // armed.
        if casting.next_swing == Some(info.id) {
            return;
        }
        active.live.cast(info.id, target);
        // **Armed at the press, and this one really is the client's** — unlike
        // the cast bar, which now waits for `SMSG_SPELL_START`. The client sets
        // `CURRENT_MELEE_SPELL` inside the send, just before the packet goes,
        // and there is no packet that would say so afterwards: the
        // server's next word about it is the `SMSG_SPELL_GO` when the weapon
        // lands, which is what empties it.
        casting.next_swing = Some(info.id);
        cooldowns.start_gcd(info);
        events.cooldown_moved();
        events.state.write(ActionbarUpdateState);
        return;
    }

    // **A ranged ability comes out of the ranged slot, so the bow comes out
    // with it.** The same explicit request the melee attack makes in
    // [`attack_target`] and for the same reason: the per-animation reconcile
    // would get there eventually off the shot's own `AnimationData` flags, but
    // only *after* a shot has been drawn — and it never draws a ranged weapon
    // at all, because `vale_assets::look::sheath::reconcile` has no branch that
    // answers `RANGED`. Its ranged exemption is written against a state only a
    // request can put the unit in, which until now nothing did: this client had
    // the whole ranged half of that policy and no way to reach it.
    if info.uses_ranged_slot() {
        sheathing.write(SheathRequest {
            entity,
            state: vale_assets::look::sheath::RANGED,
        });
    }

    // **An auto-repeat's press fires nothing**, which is now what every press
    // does and used to be this one's exception. `Spell::update`'s `PREPARING`
    // arm refuses to `cast()` an auto-repeat when its timer runs out, so the
    // first arrow leaves on the *ranged attack timer* — a clock this client does
    // not own and cannot guess — and every one after it is a fresh triggered
    // cast. What is local is only that the loop is *running*, which nothing on
    // the wire ever says.
    if info.is_auto_repeat_ranged() {
        active.live.cast(info.id, target);
        auto_repeat.begin(info.id, events);
        return;
    }
    send_cast(active, info, target, cooldowns, casting, events);
}

/// **How far apart two units are, less both their bulk** — vmangos'
/// `GetCombatDistance`, which is the distance `Spell::CheckRange` is written
/// against.
///
/// `None` when either unit has no `Transform` this frame, which is a unit that
/// has not been placed yet: a missing measurement must read as "do not refuse"
/// rather than as zero or infinity.
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

/// **What pressing a spell this character knows actually does**, before a
/// cooldown, an aiming rule or a socket is involved.
///
/// Three answers rather than two, because the two that are not a cast are
/// nothing alike: one is a different *verb* and one is nothing at all. Named
/// and tested rather than written as two `if`s inside [`cast_known_spell`],
/// because it is the whole of what this round's first two reports were — and
/// because both failures are silent at the server, so nothing downstream would
/// ever have said which branch was taken.
#[derive(Debug, PartialEq, Eq)]
enum PressKind {
    /// The auto-attack pseudo-spell: `CMSG_ATTACKSWING`, not `CMSG_CAST_SPELL`.
    Swing,
    /// A passive. The server drops the packet without replying
    /// (`HandleCastSpellOpcode`'s `IsPassiveSpell()` branch), so a client that
    /// sends one draws a cast with nothing at the end of it.
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

/// **Ask for the cast, start the global cooldown, and draw nothing.**
///
/// The three halves of a press in 5875, and the third is the one this client
/// used to get wrong. The press path runs its local refusals, writes the spell
/// and its targets into a pending record, starts the global cooldown from
/// inside the send itself, and puts the packet on the wire. It starts no bar,
/// plays no wind-up and moves no counter: `SPELLCAST_START` is raised in one
/// place in the whole client and that place is `SMSG_SPELL_START`'s handler.
///
/// **The global cooldown really is the client's**, which is the half that stays
/// local — and both sides say so. The client's starter reads `Spell.dbc`'s own
/// `StartRecoveryTime`, applies `SPELLMOD_GLOBAL_COOLDOWN` (21) and raises
/// `SPELL_UPDATE_COOLDOWN`; vmangos' `Spell::prepare` writes
/// `// add gcd server side (client side is handled by client itself)`. So the
/// swirl starts under the finger while everything else waits, which is the
/// reference's feel exactly.
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

/// `AttackTarget()` — swing at the selection, or acquire something to swing at.
///
/// **A swing with no target picks one.** That is the client's own behaviour
/// and without it the attack key does nothing at all in exactly the situation a player presses it hardest — a fight
/// that started behind them.
///
/// **And starting an attack draws the weapon.** That is the client's, not the
/// server's — vmangos' `HandleAttackSwingOpcode` never touches the sheath state
/// — and it is why this client punched everything in the game while wearing a
/// sword. The reconcile in `world::entities::sheath` would get there too, off
/// the swing's own `AnimationData` flags, but only on the first swing the server
/// reports: the explicit draw is what makes the weapon appear on the *press*,
/// with no round trip.
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
        // Already swinging at this unit: the press is a toggle off, which is
        // what the real client's Attack button does. **The weapon stays out** —
        // there is no stow branch in the client's policy at all, and a fighter
        // who breaks off mid-fight standing there with empty hands would be the
        // wrong half of this fix.
        if active.live.attacking() == Some(guid) {
            active.live.attack(None);
        } else {
            swing_at(guid);
        }
        return;
    }
    // Nothing selected: whoever is attacking us, which is the case this exists
    // for. The full nearest-enemy acquire belongs with Tab's scan and is not
    // duplicated here.
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

    /// **A cooldown moving says so under both of the game's names.**
    ///
    /// The bug this pins had every check reporting success:
    /// `SPELL_UPDATE_COOLDOWN` is in [`super::super::events::FIRED`], so
    /// `vale framexml` counted it answered and `--audit --events` fired it at
    /// the twelve spell buttons and watched their handlers run. Nothing in the
    /// *game* ever wrote it, so the swirls in the spellbook only moved when the
    /// panel was rebuilt for some other reason.
    ///
    /// Written through the one method both go out of, so the assertion is that
    /// the pair cannot come apart rather than that six particular call sites
    /// each have two lines.
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

    /// **A refused press puts the volley back out**, and a refusal about
    /// anything else leaves it alone.
    ///
    /// This is the "Auto Shot lights up when nothing is being attacked" report.
    /// The press arms the loop locally because nothing on the wire ever says it
    /// started; the server then refuses the cast and — crucially — sends **no**
    /// `SMSG_CANCEL_AUTO_REPEAT`, because `SpellCaster::InterruptSpell` only
    /// sends one for a spell it actually installed and `Spell::prepare` never
    /// installed this one. So `SMSG_CAST_FAILED` is the only statement that will
    /// ever be made about it.
    ///
    /// The second half of the assertion is the half that could have been got
    /// wrong by clearing unconditionally: a running volley survives casting
    /// other spells through it (that is what `SetCurrentCastedSpell`'s
    /// `Category == 351` test is for), so a Frostbolt that runs out of mana
    /// must not stop the arrows.
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

    /// **The volley's two edges are announced once each**, and a repeat of
    /// either says nothing.
    ///
    /// Both matter and for different reasons. `START_AUTOREPEAT_SPELL` arms a
    /// *flash clock* in `ActionButton_OnEvent`, so re-arming it on a press that
    /// changed nothing leaves the button lit or dark depending on how the two
    /// clocks line up; and `STOP_AUTOREPEAT_SPELL` is answered by
    /// `ActionButton_StopFlash()`, which for a bar with no auto-repeat on it at
    /// all would stop the *melee* attack's flash — a different animation with a
    /// different owner.
    ///
    /// The stop arriving from the wire rather than from the press is the shape
    /// worth pinning: `SMSG_CANCEL_AUTO_REPEAT` is the only statement either
    /// end of the loop makes, and the target dying is not something the client
    /// could have predicted.
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
        // …and the checked border is re-read on each *real* edge and no others:
        // `ActionButton_UpdateState` asks `IsCurrentAction or IsAutoRepeatAction`
        // and nothing else re-runs it.
        assert_eq!(
            world
                .resource::<bevy::ecs::message::Messages<ActionbarUpdateState>>()
                .len(),
            2
        );
    }

    /// **`IsAutoRepeatAction` is asked of the slot's spell, not of its id.**
    ///
    /// An item slot whose *entry* happens to equal the running spell's id is
    /// not the auto-repeat button, and the whole point of asking per button is
    /// that twelve of them answer for themselves — the two events carry
    /// nothing.
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

    /// **One cast's global cooldown gates every other button**, which is the
    /// whole reason a cooldown read resolves against every record rather than
    /// this spell's own.
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

        // A spell off the global cooldown is unaffected — the shape that lets an
        // instant ability be used inside one.
        let mut off_gcd = spell(1766, 0, 0);
        off_gcd.gcd_category = 0;
        assert!(cooldowns.ready(&off_gcd));
    }

    /// **A refused cast clears the global cooldown and nothing else.** It never
    /// reached its `SMSG_SPELL_GO`, so there is no recovery to clear — and
    /// leaving the GCD standing locks the whole bar for a second and a half on a
    /// cast that never happened.
    #[test]
    fn a_refused_cast_gives_the_bar_back() {
        let mut cooldowns = Cooldowns::default();
        let fireball = spell(133, 1500, 0);
        cooldowns.start_gcd(&fireball);
        cooldowns.clear_gcd(fireball.id);
        assert!(cooldowns.ready(&fireball));
    }

    /// **An item's own cooldown is its spell's, and the server writes it.**
    ///
    /// `SMSG_SPELL_COOLDOWN` names the item's `ON_USE` spell and nothing about
    /// the item, so the record the bag frame's swirl reads is the one
    /// [`Cooldowns::set`] already files — which is why `GetContainerItemCooldown`
    /// needed a read and not a second timer.
    #[test]
    fn an_items_cooldown_is_recorded_under_its_own_spell() {
        let mut cooldowns = Cooldowns::default();
        // 2024 is Healing Potion's spell; the item is a Superior Healing Potion.
        let potion = spell(2024, 0, 0);
        cooldowns.set(potion.id, 60_000, Some(&potion));
        let (remaining, duration) = cooldowns.remaining(&potion).expect("on cooldown");
        assert!(remaining > 59.0, "{remaining}");
        assert!((duration - 60.0).abs() < 0.01, "{duration}");
        // …and it is *that spell's* record, not a global one: a different
        // item's spell is unaffected.
        assert!(cooldowns.ready(&spell(439, 0, 0)));
    }

    /// **A right-click that casts starts the global cooldown at the press, and
    /// the bar only when the server says so** — see [`begin_item_cast`].
    #[test]
    fn using_an_item_starts_the_global_cooldown_and_waits_for_the_bar() {
        let mut cooldowns = Cooldowns::default();
        let mut casting = Casting::default();
        // First Aid's own: an eight-second cast with a global cooldown.
        let mut bandage = spell(746, 1500, 0);
        bandage.cast_time_ms = 8000;
        // The message writers are not needed for the two pieces of state, which
        // is the half a test can see with no app: the bar's own record and the
        // cooldown.
        cooldowns.start_gcd(&bandage);
        assert!(!cooldowns.ready(&bandage), "the global cooldown runs at the press");
        assert!(casting.started.is_none(), "and nothing is on the bar yet");

        // …and the bar arrives with `SMSG_SPELL_START`, at the length the
        // *server* states rather than the file's.
        casting.begin(746, "Linen Bandage".into(), 7000);
        assert_eq!(casting.spell_id, 746);
        assert!(casting.started.is_some(), "an eight-second cast has a bar");
        assert!(casting.progress().is_some_and(|p| p < 0.01));
        assert_eq!(casting.duration.as_millis(), 7000);

        // …and an *instant* — a potion — puts no bar up at all, even though the
        // packet arrives for one.
        casting.begin(2024, "Healing Potion".into(), 0);
        assert!(casting.started.is_none(), "an instant has no wind-up");
    }

    /// The spell's own recovery is a different timer from the GCD and outlives
    /// it: a 30-second cooldown is still running when the bar is free again.
    #[test]
    fn a_spells_own_recovery_outlives_the_global_cooldown() {
        let mut cooldowns = Cooldowns::default();
        let fire_blast = spell(2136, 1500, 8000);
        cooldowns.start_recovery(&fire_blast, &crate::world::spellmods::SpellMods::default());
        let (remaining, duration) = cooldowns.remaining(&fire_blast).expect("recovering");
        assert!(remaining > 7.0, "{remaining}");
        assert!((duration - 8.0).abs() < 0.01, "{duration}");
        // …and it does not gate a different spell, only its own button.
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

    /// **Attack is spell 6603 and it is not a cast.** Sending it through
    /// `CMSG_CAST_SPELL` is refused by the server's own "which he shouldn't
    /// have" branch, so the distinction has to be made before the send.
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
        // An *item* whose entry happens to equal 6603 is not the attack toggle.
        let item = Slot {
            action: SPELL_ATTACK,
            kind: action_kind::ITEM,
            spell: None,
        };
        assert!(!item.is_auto_attack());
    }

    /// **The two presses that are not a cast**, and the reason both had to be
    /// decided here rather than left to the server.
    ///
    /// `SpellButton_OnClick` calls `CastSpell(id, bookType)` on **every** row it
    /// is pressed on — it has no passive test of its own and no attack test —
    /// so the spellbook's General page hands this client spell 6603 and its 42
    /// passives (on the test warrior; 4,929 in the game) along with everything
    /// castable. `HandleCastSpellOpcode` then *drops the packet without
    /// replying*, so with this client's cast drawn at the press the whole
    /// visible outcome was a wind-up, a release and nothing at all.
    #[test]
    fn attack_is_a_swing_and_a_passive_is_refused_before_the_socket() {
        let mut attack = spell(SPELL_ATTACK, 0, 0);
        // Attack is **not** passive and **is** in the book, so nothing else here
        // would have caught it — the id is the whole of the rule.
        assert!(!attack.is_passive());
        assert_eq!(press_kind(&attack), PressKind::Swing);
        // …and it stays a swing whatever else the row says.
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

    /// **Only a bar that was running raises a stop**, which is the symmetric
    /// half of `send_cast`'s rule that an instant raises no start.
    ///
    /// `CastingBarFrame_OnEvent`'s stop branch acts on whatever is *shown*, so
    /// a stop for a cast that never had a bar colours and fades whatever else
    /// is up there — which for a channel is the bar that has just appeared.
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

    /// **A press asks and the server answers; nothing is on the bar in
    /// between.**
    ///
    /// The state that replaced the local prediction, and the two things it has
    /// to get right. The ask holds no bar — `progress` is `None` and
    /// `in_progress` is false, because there is no wind-up, only a packet in
    /// flight — and it is cleared by whichever answer arrives first. If it were
    /// not cleared the button would be wedged for the rest of the session, and
    /// if it held a bar a refused cast would show one, which is the whole bug.
    #[test]
    fn an_unanswered_ask_holds_no_bar_and_is_cleared_by_the_answer() {
        let mut casting = Casting { pending: Some(133), ..Default::default() };
        assert_eq!(casting.progress(), None, "an ask is not a cast");
        assert!(!in_progress(&casting), "and it is not a wind-up either");

        // The server took it: the bar goes up and the ask is spent.
        casting.begin(133, "Fireball".into(), 2500);
        assert_eq!(casting.pending, None);
        assert!(in_progress(&casting));

        // …and an instant's answer is the same call with no bar in it.
        let mut casting = Casting { pending: Some(1449), ..Default::default() };
        casting.begin(1449, "Arcane Explosion".into(), 0);
        assert_eq!(casting.pending, None);
        assert!(!in_progress(&casting), "an instant leaves nothing running");
    }

    /// **A channel is not a wind-up**, and the difference is what decides
    /// whether a fresh press is refused.
    ///
    /// `Spell::prepare`'s "another action is in progress" gate is
    /// `IsNonMeleeSpellCasted(false, true, true)` — `skipChanneled` is `true`,
    /// so a cast pressed during a channel is accepted by the server and simply
    /// replaces it, where one pressed during a wind-up is refused.
    #[test]
    fn a_channel_does_not_block_a_press_and_a_wind_up_does() {
        let mut casting = Casting::default();
        assert!(!in_progress(&casting));

        // A channel: no `started`, so nothing is blocked…
        casting.channelling = Some(12051);
        casting.spell_id = 12051;
        assert!(!in_progress(&casting));
        assert_eq!(casting.progress(), None, "a channel has no wind-up to show");

        // …and pressing something takes the channel down, exactly as the
        // server's own acceptance of the cast does.
        let mut fireball = spell(133, 1500, 0);
        fireball.cast_time_ms = 1500;
        casting.begin(133, "Fireball".into(), 1500);
        assert_eq!(casting.channelling, None);
        assert!(in_progress(&casting), "a wind-up blocks");
    }

    /// **A pushback lengthens the bar and never starts one.**
    ///
    /// `SMSG_SPELL_DELAYED` is the only packet in the protocol that restates a
    /// cast's length after it has begun — the server has already added the time
    /// to its own `m_timer` — so a client that does not read it runs the bar
    /// out and sits at full while the server is still casting. That is the
    /// visible half of the "stuck casting a spell that was interrupted" report.
    ///
    /// The two refusals are the ones that would move the wrong bar: a channel
    /// is pushed back through `MSG_CHANNEL_UPDATE` instead (vmangos'
    /// `DelayedChannel` *shortens* its timer), and nothing that has no wind-up
    /// has a bar to lengthen.
    #[test]
    fn a_pushback_lengthens_the_bar_it_finds_and_conjures_none() {
        let mut casting = Casting::default();
        assert!(!casting.delay(500), "there was no cast");

        casting.begin(133, "Fireball".into(), 3500);
        assert_eq!(casting.duration, Duration::from_millis(3500));

        // Two blows land while it runs: each says how much *it* added.
        assert!(casting.delay(500));
        assert!(casting.delay(1000));
        assert_eq!(
            casting.duration,
            Duration::from_millis(5000),
            "a pushback is a difference, not a new length",
        );

        // An instant has no bar, and a channel's is slid by its own event.
        let mut instant = Casting::default();
        instant.begin(1449, "Arcane Explosion".into(), 0);
        assert!(!instant.delay(500));
        let mut channel = Casting::default();
        channel.channelling = Some(12051);
        channel.started = Some(Instant::now());
        assert!(!channel.delay(500));
    }

    /// **The spell cursor is spent by whatever ends it, and never twice.**
    ///
    /// The state machine behind the report: press a heal with nothing suitable
    /// selected and the client asks instead of complaining, and the question is
    /// answered by exactly one of four things — a click on a unit, a click on
    /// empty ground, Escape, or a fresh press. `cast_known_spell` calls `stop`
    /// on every path out of the aiming rule for that reason: a cursor that
    /// survived its own answer would cast the *previous* spell at the next
    /// thing clicked.
    #[test]
    fn the_spell_cursor_is_put_away_by_whatever_answers_it() {
        let mut targeting = SpellTargeting::default();
        assert!(!targeting.is_targeting());
        assert_eq!(targeting.spell(), None);

        targeting.begin(2050, Asking::Unit); // Lesser Heal
        assert!(targeting.is_targeting());
        assert_eq!(targeting.spell(), Some(2050));
        assert!(!targeting.wants_ground());

        // `over_valid` is the cursor's own red-or-green and is re-decided every
        // frame — a `stop` must not leave it standing, or the next question
        // opens green over whatever the pointer happens to be on.
        targeting.over_valid = true;
        targeting.stop();
        assert!(!targeting.is_targeting());
        assert!(!targeting.over_valid);
        assert_eq!(targeting.spell(), None);

        // A second question replaces the first outright: two spells cannot be
        // waiting at once, because there is one cursor.
        targeting.begin(2050, Asking::Unit);
        targeting.begin(1459, Asking::Unit);
        assert_eq!(targeting.spell(), Some(1459));
    }

    /// **…and the placed half of the same cursor carries its own two fields
    /// out.** A ground question that ended must leave neither the mode nor the
    /// point behind it: the mode would make the next unit cast wait for a patch
    /// of floor, and a stale point would place the next Blizzard where the last
    /// one went.
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

        // …and a unit question after a placed one is a unit question: the mode
        // is the *new* press's, not whatever was standing.
        targeting.begin(10, Asking::Ground);
        targeting.over_ground = Some([1.0, 2.0, 3.0]);
        targeting.begin(2050, Asking::Unit);
        assert!(!targeting.wants_ground());
        assert_eq!(targeting.over_ground, None);
    }

    /// **The third mode is exclusive with the other two**, which is the whole
    /// of what [`Asking`] replaced two bools to guarantee: a cursor waiting for
    /// an item must not read as waiting for a patch of floor, or
    /// `spell_ground_under_pointer` walks the terrain every frame for an answer
    /// nobody wants, and the click that ends it lands in the wrong branch.
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

        // …and the mode is the new press's, whichever way round they come.
        targeting.begin(2828, Asking::Item);
        targeting.begin(10, Asking::Ground);
        assert!(!targeting.wants_item());
        assert!(targeting.wants_ground());
        targeting.begin(2828, Asking::Item);
        assert!(!targeting.wants_ground());
    }

    /// **A next-swing spell is a queued swing, not a cast** — the whole of what
    /// decides whether the art is drawn at the press. Heroic Strike is the
    /// measured case: `Attributes 0x00050014`, which is
    /// `ON_NEXT_SWING_NO_DAMAGE` and *not* `ON_NEXT_SWING`.
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
