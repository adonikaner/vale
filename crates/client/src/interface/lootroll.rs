//! **Who gets it, when there are four of you** — the client's half of
//! [`vale_protocol::play::lootroll`].
//!
//! ```text
//! SMSG_LOOT_START_ROLL   -> LootRolls::open   -> START_LOOT_ROLL(id, ms)
//!   Need / Greed / Pass  -> RollOnLoot(id, n)
//!     …on a bind-on-pickup item          -> CONFIRM_LOOT_ROLL(id, n)
//!     …otherwise, and after Confirm      -> CMSG_LOOT_ROLL(guid, slot, n)
//!                        -> CANCEL_LOOT_ROLL(id)   the frame goes
//! SMSG_LOOT_ROLL         -> a CHAT_MSG_LOOT line, and nothing else
//! SMSG_LOOT_ROLL_WON     -> …one more, and CANCEL_LOOT_ROLL(id)
//! SMSG_LOOT_ALL_PASSED   -> …and the row on the body becomes clickable
//! ```
//!
//! ## The `rollID` is this client's, and that is the whole of the design
//!
//! Nothing on the wire names a roll. Every packet says `(guid, item slot)` and
//! the interface says `rollID` — one number, passed to
//! `GetLootRollItemInfo`, `GetLootRollTimeLeft` and `RollOnLoot`. In the 1.12.1
//! client it is a plain counter over the pending list, bumped once per roll. So
//! [`LootRolls`] is that list, [`PendingRoll`]
//! is that record, and the crossing between the two namings happens **here and
//! nowhere else** — the same rule the loot window's two numberings follow.
//!
//! ## A fifth roll is deferred, not dropped, and not drawn
//!
//! `NUM_GROUP_LOOT_FRAMES` is 4 and the C side knows it: the client checks the
//! shown count before firing anything and, when it is full, marks the roll
//! deferred and **fires no event**. Whenever a frame frees up it walks the list
//! and promotes the first deferred roll it finds.
//!
//! That is reproduced exactly, including the part that looks like a bug: the
//! promotion re-sends the roll's **original** countdown rather than what is left
//! of it, so a deferred frame opens with a full bar over a clock that is already
//! half gone. `GroupLootFrame_OnUpdate` then clamps — `if ( (left < min) or
//! (left > max) ) then left = min` — which is the shipped Lua compensating for
//! it. Fixing it here would make the bar disagree with that clamp.
//!
//! ## A frame is held back until its item has a name, exactly as the window is
//!
//! `SMSG_LOOT_START_ROLL` carries an entry and **no display id**, so a roll on
//! an item this character has never met has nothing at all to draw: not a name,
//! not an icon, not a quality colour. `GroupLootFrame_OnShow` reads all five of
//! `GetLootRollItemInfo`'s answers in one line and runs **once**, when the frame
//! is shown — so a frame opened a round trip early is a blank frame for the
//! whole minute the roll lasts.
//!
//! The reference does not open it early. It hands the entry to the item cache
//! and only fires `START_LOOT_ROLL` either immediately, when the entry was
//! already cached, or from the cache's completion callback later. It is the
//! same shape [`super::loot::hold`] has and it is here for the same reason,
//! bounded by [`NAME_LIMIT`] so that a lost query cannot cost the player the
//! roll altogether.
//!
//! **A refusal counts as an answer**, on the loot window's own terms: vmangos
//! replies `entry | 0x80000000` for an item it will not describe, and a row the
//! server will never name is a row there is no point waiting for.
//!
//! ## Rolling on a bind-on-pickup item asks first
//!
//! The rule: a vote that is not Pass, on a roll whose item has
//! `Bonding == 1`, raises `CONFIRM_LOOT_ROLL(id, rollType)` and **sends
//! nothing**. `UIParent.lua` turns that into a `StaticPopup_Show`, whose accept
//! calls `ConfirmLootRoll(id, rollType)` — the same entry point with the flag
//! set, which skips the check. So the confirmation is the *client's* and the
//! server never hears about the first press.
//!
//! **The item template is what decides, and it may not have arrived.** A roll
//! opens on an item this character has never met about as often as not, and the
//! template is a round trip behind — the same cold-cache state the loot window
//! holds itself open for. Here the choice is the other way: an unknown template
//! is treated as **not** bind-on-pickup, so the press goes through. A press that
//! silently did nothing while a query was in flight is worse than a missing
//! confirmation, and the roll has a clock on it.
//!
//! ## The chat lines are the game's, and one of them is a CVar
//!
//! Every announcement is a `GlobalStrings.lua` key filled with the roller's
//! name, the item's link and — for two of the four — the number. The keys, the
//! branch that picks between them and the `showLootSpam` rule are all in
//! [`vale_protocol::play::lootroll`], where a test can run them with no
//! world. What is here is the *filling*: a name out of the object manager, a
//! link out of the item cache, and one `CHAT_MSG_LOOT` per line.
//!
//! **A line whose pieces have not arrived is not composed at all.** The name of
//! a group member across the zone and the template of an item nobody has seen
//! are both a query away, and a sentence reading `Player 4213 has selected Need
//! for: []` is worse than silence — the roll frame is on screen either way and
//! it has the picture.
//!
//! ## …and the row on the body is freed by nothing on the wire
//!
//! The one rule in this file with no packet behind it. See
//! [`vale_protocol::play::loot::Loot::unblock`], which explains it.

use bevy::prelude::*;

use vale_protocol::play::lootroll::{
    RollAllPassed, RollCast, RollLine, RollStart, RollVote, RollWon, MAX_ROLL_FRAMES,
    SHOW_LOOT_SPAM,
};

use super::events::{CancelLootRoll, ConfirmLootRoll, StartLootRoll};
use crate::world::session::Session;

/// One of the four packets, handed on from
/// [`super::action::drain_events`] — the same door
/// [`super::loot::LootAnswer`] comes through, and for the same reason: the
/// session queue has exactly one reader.
#[derive(Message, Debug, Clone, Copy)]
pub enum RollAnswer {
    Started(RollStart),
    Cast(RollCast),
    Won(RollWon),
    AllPassed(RollAllPassed),
}

/// **One roll the client is holding**, whether or not a frame is showing it.
///
/// The reference's pending-roll record, minus the fields this client
/// has no use for: the two random-property words are always zero in 1.12 and
/// the item link is composed where it is needed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingRoll {
    /// **The interface's name for it**, and this client's own invention. See
    /// the module note.
    pub id: u32,
    /// …and the wire's, which is what a vote actually carries.
    pub guid: u64,
    pub item_slot: u32,
    pub entry: u32,
    /// What the start said, kept because the promotion of a deferred roll
    /// re-sends it rather than the remainder — see the module note.
    pub countdown_ms: u32,
    /// When it runs out, on [`Time`]'s own clock. `GetLootRollTimeLeft` is this
    /// minus now, in milliseconds, and it is allowed to go negative: the shipped
    /// `OnUpdate` clamps.
    pub deadline: f64,
    /// When the start arrived, on [`Time`]'s own clock — what [`NAME_LIMIT`] is
    /// measured from.
    opened_at: f64,
    /// **Its item can be named**, so a frame would draw something. False until
    /// the template lands or the wait runs out — see the module note.
    named: bool,
    /// **It has never had a frame**, because all four were busy when it
    /// arrived — what the promotion walks the list looking for.
    ///
    /// **Cleared on promotion and never set again**, which is the difference
    /// between this and "is a frame showing it": a roll whose frame came down
    /// because the player voted must not be handed a new one when the next
    /// frame frees up, and one flag doing both jobs would do exactly that.
    deferred: bool,
    /// …and whether a frame is showing it *now*, which is what decides whether
    /// hiding it owes the interface a `CANCEL_LOOT_ROLL`.
    showing: bool,
    /// **We have voted**, so the frame is down and the record is only waiting
    /// for the result to name it. It is what stops a second press going out.
    voted: bool,
}

/// **Every roll this session is holding**, in arrival order.
///
/// A `Vec` and not a map: it is at most a handful of entries, every access is
/// either "find this id" or "the first deferred one", and both are a scan an
/// index would not beat at this size. The reference uses a linked list walked
/// the same way.
#[derive(Resource, Default)]
pub struct LootRolls {
    rolls: Vec<PendingRoll>,
    /// The roll counter. Never reset within a session, so an id is
    /// unique for the life of the process and a stale `RollOnLoot(3)` from an
    /// addon cannot vote on somebody else's roll.
    next_id: u32,
    /// How many of [`Self::rolls`] have a frame, kept rather than counted so
    /// the cap check is not a scan.
    shown: usize,
}

impl LootRolls {
    /// One roll by the interface's id.
    pub fn get(&self, id: u32) -> Option<&PendingRoll> {
        self.rolls.iter().find(|roll| roll.id == id)
    }

    /// **Whether anything is going on at all**, which is what every system in
    /// this file asks before doing any work. A session with no group never gets
    /// past this line.
    pub fn is_empty(&self) -> bool {
        self.rolls.is_empty()
    }

    /// How many rolls are held, frames and deferrals together.
    pub fn len(&self) -> usize {
        self.rolls.len()
    }

    /// `GetLootRollTimeLeft(id)` — milliseconds, and **allowed to be negative**.
    ///
    /// The client answers `deadline - now` with no clamp at all; the clamping is
    /// `GroupLootFrame_OnUpdate`'s, against the bar's own min and max. `None`
    /// for an id nothing holds, which is what the reference answers by
    /// returning no values.
    pub fn time_left_ms(&self, id: u32, now: f64) -> Option<f64> {
        Some((self.get(id)?.deadline - now) * 1000.0)
    }

    /// Take a start into the list, and answer the id it was given.
    ///
    /// **It does not open a frame.** Whether one is free is [`show_next`]'s
    /// question and whether the item can be named yet is the clock's, and both
    /// are asked in [`promote`] — which is also where a roll deferred behind
    /// four others is picked up, so there is one path to `START_LOOT_ROLL`
    /// rather than two.
    fn open(&mut self, start: &RollStart, now: f64) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.rolls.push(PendingRoll {
            id,
            guid: start.guid,
            item_slot: start.item_slot,
            entry: start.entry,
            countdown_ms: start.countdown_ms,
            deadline: now + f64::from(start.countdown_ms) / 1000.0,
            opened_at: now,
            named: false,
            deferred: true,
            showing: false,
            voted: false,
        });
        id
    }

    /// Give the next deferred roll a frame, if there is one and a frame is free.
    ///
    /// Answers the id to raise `START_LOOT_ROLL` for. One at a time rather than
    /// a loop, so that the caller writes one message per promotion and the
    /// arithmetic stays in the caller's own drain.
    fn show_next(&mut self) -> Option<u32> {
        if self.shown >= MAX_ROLL_FRAMES {
            return None;
        }
        // **Named as well as deferred.** A frame given to a roll nothing can
        // name yet draws an empty square with no text under it for the whole
        // minute — see the module note.
        let roll = self.rolls.iter_mut().find(|roll| roll.deferred && roll.named)?;
        roll.deferred = false;
        roll.showing = true;
        self.shown += 1;
        Some(roll.id)
    }

    /// **The frame for this roll comes down**, whether because we voted or
    /// because the roll ended.
    ///
    /// Answers whether a `CANCEL_LOOT_ROLL` is owed: a roll that never had a
    /// frame has nothing to hide, and raising the event for it would tell four
    /// other `GroupLootFrame`s nothing while costing a Lua call each.
    fn hide(&mut self, id: u32) -> bool {
        let Some(roll) = self.rolls.iter_mut().find(|roll| roll.id == id) else {
            return false;
        };
        if !roll.showing {
            return false;
        }
        roll.showing = false;
        // **Not `deferred = true`.** A frame that has been used is spent; the
        // reference clears its byte on promotion and never sets it again.
        self.shown = self.shown.saturating_sub(1);
        true
    }

    /// The roll a result names, by the **wire's** name for it.
    fn find(&self, guid: u64, item_slot: u32) -> Option<u32> {
        self.rolls
            .iter()
            .find(|roll| roll.guid == guid && roll.item_slot == item_slot)
            .map(|roll| roll.id)
    }

    /// Forget a roll entirely — its result has arrived.
    fn close(&mut self, id: u32) {
        self.rolls.retain(|roll| roll.id != id);
    }

    /// **This one can be drawn now** — its template landed, or the wait for it
    /// ran out. See the module note.
    fn name_landed(&mut self, id: u32) {
        if let Some(roll) = self.rolls.iter_mut().find(|roll| roll.id == id) {
            roll.named = true;
        }
    }

    /// Every roll still waiting for a name, as `(id, entry, opened_at)`.
    ///
    /// Answers an empty vector once everything is named, which is what keeps
    /// the clock from taking the world lock on every frame of a roll's minute.
    fn unnamed(&self) -> Vec<(u32, u32, f64)> {
        self.rolls
            .iter()
            .filter(|roll| !roll.named)
            .map(|roll| (roll.id, roll.entry, roll.opened_at))
            .collect()
    }

    /// Mark a vote sent. The frame is already down; this is what stops a second.
    fn voted(&mut self, id: u32) {
        if let Some(roll) = self.rolls.iter_mut().find(|roll| roll.id == id) {
            roll.voted = true;
        }
    }
}

/// **How long a roll may outlive its own clock before this client forgets it.**
///
/// A departure from the reference, stated as one. `Group::Update` counts a
/// timed-out roll for you and the result always comes back, so the client never
/// needed to expire a record and does not: a `PendingLootRoll` lives until a
/// win or an all-passed names it.
///
/// What it does not survive is the server never answering — a creature unloaded
/// under the roll, a group disbanded, a reconnect. The reference's answer to
/// that is a `GroupLootFrame` sitting on the screen for the rest of the session
/// with a bar pinned at zero, which is a worse bug than the one this risks.
/// Generous on purpose: a roll cannot be lost to a late packet inside a quarter
/// of a minute.
const EXPIRY_GRACE: f64 = 15.0;

pub struct LootRollPlugin;

impl Plugin for LootRollPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RollAnswer>()
            .add_message::<RollPress>()
            .init_resource::<LootRolls>()
            .add_systems(
                Update,
                // **The clock last**, so a start that arrives with its template
                // already cached opens its frame in the same frame it landed.
                (announce, presses, vote, clock)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// **A button on a `GroupLootFrame`**, or an addon calling `RollOnLoot`.
///
/// A message rather than a direct send, on the terms every other interface write
/// in this client follows — see [`crate::lua::api::verbs`].
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollPress {
    pub id: u32,
    pub vote: RollVote,
    /// **The bind-on-pickup question has already been asked.** False for
    /// `RollOnLoot`, true for `ConfirmLootRoll` — see the module note.
    pub confirmed: bool,
}

/// Fold what the server said into the list, tell the interface, and say it in
/// the chat.
fn announce(
    mut answers: MessageReader<RollAnswer>,
    mut rolls: ResMut<LootRolls>,
    mut window: ResMut<super::loot::LootWindow>,
    session: Res<Session>,
    time: Res<Time>,
    cvars: Res<crate::settings::cvars::CVars>,
    strings: Res<super::messages::UiStrings>,
    mut cancelled: MessageWriter<CancelLootRoll>,
    mut said: MessageWriter<super::events::ChatMessageReceived>,
) {
    if answers.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();
    // **The chatty half of `showLootSpam`**, read once per drain rather than per
    // packet: it is a string compare and a roll of five people is five packets.
    let spam = cvars.flag(SHOW_LOOT_SPAM);
    for answer in answers.read() {
        match answer {
            RollAnswer::Started(start) => {
                // The name and the picture are both a round trip away for an
                // item this character has never met; ask now so the frame's own
                // `OnShow` has them. The window does the same for its rows.
                want(&session, start.entry);
                // **No frame yet.** The clock opens it, once there is one free
                // and the item can be named — see the module note.
                rolls.open(start, now);
            }
            // **Not a window event and not a frame event** — a vote is a chat
            // line and nothing else. The frame belonging to the *voter* came
            // down on their own client.
            RollAnswer::Cast(cast) => {
                if !spam {
                    continue;
                }
                let line = RollLine::of(cast.number, cast.kind);
                if let Some(text) = compose(
                    &session,
                    &strings,
                    cast.entry,
                    cast.roller,
                    line.has_number().then_some(cast.number),
                    line.names_the_roller_last(),
                    |is_self| line.key(is_self, cast.kind),
                ) {
                    say(&mut said, text);
                }
            }
            RollAnswer::Won(won) => {
                if let Some(text) = compose(
                    &session,
                    &strings,
                    won.entry,
                    won.winner,
                    // **The number only when the quiet setting folds it in** —
                    // with `showLootSpam` on it already had a line of its own.
                    (!spam).then_some(won.number),
                    false,
                    |is_self| vale_protocol::play::lootroll::won_key(is_self, spam, won.kind),
                ) {
                    say(&mut said, text);
                }
                finish(
                    &mut rolls,
                    &mut window,
                    &mut cancelled,
                    won.guid,
                    won.item_slot,
                );
            }
            RollAnswer::AllPassed(passed) => {
                // The one line in the family with no branch, and no roller
                // either — `LOOT_ROLL_ALL_PASSED` names only the item.
                if let Some(text) = item_line(
                    &session,
                    &strings,
                    passed.entry,
                    vale_protocol::play::lootroll::ALL_PASSED_KEY,
                ) {
                    say(&mut said, text);
                }
                finish(
                    &mut rolls,
                    &mut window,
                    &mut cancelled,
                    passed.guid,
                    passed.item_slot,
                );
            }
        }
    }
}

/// **Give every free frame to a roll that is waiting for one.**
///
/// Called from all three places a frame can free up: a result arriving, the
/// player voting, and a roll outliving its clock. A frame freed with nobody
/// waiting is one `shown >= MAX_ROLL_FRAMES` check and no allocation.
fn promote(rolls: &mut LootRolls, started: &mut MessageWriter<StartLootRoll>) {
    while let Some(id) = rolls.show_next() {
        let Some(roll) = rolls.get(id) else { break };
        // **The original countdown, not the remainder** — see the module note,
        // where the shipped `OnUpdate` clamp that compensates for it is.
        let countdown_ms = roll.countdown_ms;
        started.write(StartLootRoll { id, countdown_ms });
    }
}

/// **A roll is over**: take its frame down, free its row on the body, forget it.
///
/// A free function rather than a system, on [`super::loot::close_if_empty`]'s
/// own terms: the question is only worth asking on the two packets that end a
/// roll, and both endings do exactly this.
fn finish(
    rolls: &mut LootRolls,
    window: &mut super::loot::LootWindow,
    cancelled: &mut MessageWriter<CancelLootRoll>,
    guid: u64,
    item_slot: u32,
) {
    // **The row is freed whether or not this client knew about the roll.** A
    // member who joined the group after it opened was sent no start and still
    // has the body open with a blocked row on it.
    //
    // The slot is a `u32` here and a `u8` in the window, and the two really are
    // the same number: `LootView` writes `uint8(i)` and the roll family widened
    // it for nothing. A slot that does not fit is a server this client cannot
    // follow, so it frees nothing rather than truncating into a row it would
    // then unblock by accident.
    if let Ok(index) = u8::try_from(item_slot) {
        window.unblock_row(guid, index);
    }
    let Some(id) = rolls.find(guid, item_slot) else {
        return;
    };
    if rolls.hide(id) {
        cancelled.write(CancelLootRoll { id });
    }
    rolls.close(id);
}

/// Drain what the interface pressed.
///
/// Its own system rather than a `NonSendMut` on [`vote`], for the reason
/// [`super::loot::presses`] gives: a non-send parameter pins its system to the
/// main thread.
fn presses(host: Option<NonSendMut<crate::lua::host::LuaHost>>, mut out: MessageWriter<RollPress>) {
    let Some(mut host) = host else { return };
    for press in host.take_roll_presses() {
        out.write(press);
    }
}

/// Send what the player pressed — **or ask them first**.
fn vote(
    mut presses: MessageReader<RollPress>,
    mut rolls: ResMut<LootRolls>,
    session: Res<Session>,
    mut confirm: MessageWriter<ConfirmLootRoll>,
    mut cancelled: MessageWriter<CancelLootRoll>,
) {
    if presses.is_empty() {
        return;
    }
    for press in presses.read() {
        let Some(roll) = rolls.get(press.id) else {
            continue;
        };
        // **Already voted is silence, not a second packet.** The client returns
        // outright on its voted flag, which is what stops a scripted double
        // press and an addon racing the button.
        if roll.voted {
            continue;
        }
        let (guid, item_slot, entry) = (roll.guid, roll.item_slot, roll.entry);
        if !press.confirmed && press.vote != RollVote::Pass && binds_on_pickup(&session, entry) {
            confirm.write(ConfirmLootRoll {
                id: press.id,
                vote: press.vote,
            });
            continue;
        }
        if let Some(active) = session.active.as_ref() {
            active.live.loot_roll(guid, item_slot, press.vote);
        }
        rolls.voted(press.id);
        // **The frame goes on the press, not on the answer** — the client sets
        // the flag and fires `CANCEL_LOOT_ROLL` before anything comes back.
        // The record stays: the result still has to find it.
        if rolls.hide(press.id) {
            cancelled.write(CancelLootRoll { id: press.id });
        }
        // …and the frame that just freed up goes to whoever was waiting, on the
        // next tick of the clock. One path to `START_LOOT_ROLL`, not three.
    }
}

/// **The roll list's own clock**: let a named roll have a frame, and forget one
/// the server never finished.
///
/// The one system here that does anything on an ordinary frame, and the whole of
/// what it does for a session that is not in a group is the `is_empty` check
/// on its first line.
fn clock(
    mut rolls: ResMut<LootRolls>,
    session: Res<Session>,
    time: Res<Time>,
    mut cancelled: MessageWriter<CancelLootRoll>,
    mut started: MessageWriter<StartLootRoll>,
) {
    // The whole cost of this file for a session that never groups.
    if rolls.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();

    // **Forget what the server never finished** — see [`EXPIRY_GRACE`].
    let dead: Vec<u32> = rolls
        .rolls
        .iter()
        .filter(|roll| now > roll.deadline + EXPIRY_GRACE)
        .map(|roll| roll.id)
        .collect();
    for id in dead {
        if rolls.hide(id) {
            cancelled.write(CancelLootRoll { id });
        }
        rolls.close(id);
    }

    // **…and let anything that can now be named have a frame.** The world lock
    // is taken only while something is still waiting for a template, which is
    // one round trip out of a roll's minute.
    let waiting = rolls.unnamed();
    if !waiting.is_empty() {
        for id in settled(&session, &waiting, now) {
            rolls.name_landed(id);
        }
    }
    promote(&mut rolls, &mut started);
}

/// **How long a roll may be held back waiting for its item to have a name.**
///
/// The same bound and the same argument as [`super::loot::HOLD_LIMIT`]: against
/// a server on this machine the wait is one session tick and one round trip,
/// and what this covers is the answer that never comes. A frame that opens
/// blank is bad; a roll the player is never offered at all is worse, and this
/// one has a clock running against it either way.
const NAME_LIMIT: f64 = 3.0;

/// Which of the waiting rolls can be drawn now.
///
/// **A refusal counts**, on [`super::loot::settled`]'s own terms: vmangos
/// answers `entry | 0x80000000` for an item it will not describe, which lands
/// in `unknown_items`, and a roll the server will never name is one there is no
/// point waiting for.
fn settled(session: &Session, waiting: &[(u32, u32, f64)], now: f64) -> Vec<u32> {
    let named: Option<Vec<u32>> = session.active.as_ref().map(|active| {
        let world = active.live.world();
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        waiting
            .iter()
            .filter(|(_, entry, _)| {
                world.items.contains_key(entry) || world.unknown_items.contains(entry)
            })
            .map(|(id, _, _)| *id)
            .collect()
    });
    // **No session is no answers coming**, so nothing is held hostage to it —
    // which is also the state every unit test here runs in.
    let named = named.unwrap_or_default();
    waiting
        .iter()
        .filter(|(id, _, opened_at)| {
            named.contains(id) || session.active.is_none() || now - opened_at >= NAME_LIMIT
        })
        .map(|(id, _, _)| *id)
        .collect()
}

/// Queue an item template if the session cannot name it yet.
fn want(session: &Session, entry: u32) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let world = active.live.world();
    let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
    if !world.items.contains_key(&entry) {
        world.want_item(entry);
    }
}

/// **Does rolling on this need a confirmation?** `Bonding == 1`, and an item
/// nobody can name yet is treated as no — see the module note.
fn binds_on_pickup(session: &Session, entry: u32) -> bool {
    let Some(active) = session.active.as_ref() else {
        return false;
    };
    let world = active.live.world();
    let world = world.lock().unwrap_or_else(|e| e.into_inner());
    world.items.get(&entry).is_some_and(|item| item.bonding == 1)
}

/// One `CHAT_MSG_LOOT` line, said by nobody.
///
/// The author is empty because these are the client's own output rather than
/// anybody's speech — the same shape a system line has, which is what
/// `ChatFrame_OnEvent` expects for the whole `CHAT_MSG_LOOT` group.
fn say(
    said: &mut MessageWriter<super::events::ChatMessageReceived>,
    text: String,
) {
    said.write(super::events::ChatMessageReceived {
        event: "CHAT_MSG_LOOT",
        text,
        author: String::new(),
        flag: "",
        channel: String::new(),
        ..Default::default()
    });
}

/// **A roll line about a person**, or `None` while a piece of it is in flight.
///
/// The order is the client's own argument order. Every one of the branches
/// ends in the same seven — see [`link_pieces`] — and they differ only in
/// what goes in front:
///
/// ```text
/// "You passed on: …"                  <seven>
/// "%s passed on: …"            name    <seven>
/// "You won: … (Need - %1$d)"   number  <seven>
/// "%s won: … (Need - %2$d)"    name  number  <seven>
/// "Need Roll - %d for … by %s" number  <seven>  name
/// ```
///
/// **The last of those is why the name is not simply first.**
/// `LOOT_ROLL_ROLLED_NEED` is the one key in the family that puts the roller at
/// the end of the sentence, and filling it in the other order writes the name
/// where the roll number goes. See [`RollLine::names_the_roller_last`].
fn compose(
    session: &Session,
    strings: &super::messages::UiStrings,
    entry: u32,
    who: u64,
    number: Option<u8>,
    // Whether the key names the roller *after* the link rather than before it
    // — `RollLine::names_the_roller_last`, and true for the dice alone.
    trailing: bool,
    key: impl FnOnce(bool) -> &'static str,
) -> Option<String> {
    let active = session.active.as_ref()?;
    let world = active.live.world();
    let world = world.lock().unwrap_or_else(|e| e.into_inner());
    let is_self = world.player_guid == Some(who);
    let name = world.players.get(&who)?.name.clone();
    let item = world.items.get(&entry)?;
    let pieces = link_pieces(item.entry, item.quality, &item.name);
    let key = key(is_self);
    drop(world);

    let number = number.map(|number| number.to_string());
    let last = trailing.then_some(name.as_str());
    let mut arguments: Vec<&str> = Vec::with_capacity(10);
    // **The self forms name nobody** — "You passed on: …" against "%s passed
    // on: …" — and the trailing form names them after the link instead.
    if !is_self && !trailing {
        arguments.push(&name);
    }
    if let Some(number) = number.as_deref() {
        arguments.push(number);
    }
    arguments.extend(pieces.iter().map(String::as_str));
    arguments.extend(last);
    fill(strings, key, &arguments)
}

/// …and one about an item only — `LOOT_ROLL_ALL_PASSED`.
fn item_line(
    session: &Session,
    strings: &super::messages::UiStrings,
    entry: u32,
    key: &str,
) -> Option<String> {
    let active = session.active.as_ref()?;
    let world = active.live.world();
    let world = world.lock().unwrap_or_else(|e| e.into_inner());
    let item = world.items.get(&entry)?;
    let pieces = link_pieces(item.entry, item.quality, &item.name);
    drop(world);
    let arguments: Vec<&str> = pieces.iter().map(String::as_str).collect();
    fill(strings, key, &arguments)
}

/// The `GlobalStrings.lua` sentence, filled.
///
/// **`None` for a key the file does not carry**, on the terms
/// [`crate::interface::messages`] states for every other line this client shows: a
/// key that resolves to nothing shows nothing, and the alternative here would
/// be a chat window printing `LOOT_ROLL_PASSED`.
fn fill(
    strings: &super::messages::UiStrings,
    key: &str,
    arguments: &[&str],
) -> Option<String> {
    let text = strings.get()?.get(key)?;
    Some(vale_assets::interface::strings::substitute_positional(
        text, arguments,
    ))
}

/// **The item hyperlink taken apart**, because the shipped keys are written
/// against its pieces rather than against the link.
///
/// `LOOT_ROLL_PASSED` is `"%s passed on: %s|Hitem:%d:%d:%d:%d|h[%s]|h%s"` —
/// seven slots after the name, and only the first of them is anything a caller
/// would think of as "the link". Handing
/// [`crate::lua::panels::container::item_link`]'s whole string to the second
/// slot fills one and leaves six directives standing in the chat window.
///
/// The seven, in the order the reference pushes them: the quality colour, the
/// entry, the enchant, the random property, the random suffix, the item's name,
/// and the terminator. **The three zeros are the wire's own**: vmangos writes
/// literal zeros for the two random words in 1.12 and nothing enchants a corpse
/// drop, so this is the same link `item_link` builds, spelled out.
///
/// The terminator is dropped with the colour, which is the client's own rule:
/// `"|r"` normally, the empty string when the prefix is empty.
fn link_pieces(entry: u32, quality: u32, name: &str) -> [String; 7] {
    let colour = crate::lua::panels::container::quality_colour(quality);
    let close = if colour.is_empty() { "" } else { "|r" };
    [
        colour.to_string(),
        entry.to_string(),
        "0".to_string(),
        "0".to_string(),
        "0".to_string(),
        name.to_string(),
        close.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open a roll and let it be drawn at once, which is the state every test
    /// below is about. The wait for a name has its own test.
    fn opened(rolls: &mut LootRolls, guid: u64, slot: u32, entry: u32, now: f64) -> u32 {
        let id = rolls.open(&start(guid, slot, entry), now);
        rolls.name_landed(id);
        id
    }

    fn start(guid: u64, slot: u32, entry: u32) -> RollStart {
        RollStart {
            guid,
            item_slot: slot,
            entry,
            random_suffix: 0,
            random_property: 0,
            countdown_ms: 60_000,
        }
    }

    /// **Four frames, and the fifth roll waits its turn** — the cap the C side
    /// enforces, and the promotion when a frame frees up.
    #[test]
    fn a_fifth_roll_is_held_back_until_a_frame_frees_up() {
        let mut rolls = LootRolls::default();
        for slot in 0..MAX_ROLL_FRAMES as u32 {
            opened(&mut rolls, 7, slot, 2589, 0.0);
            assert_eq!(
                rolls.show_next(),
                Some(slot),
                "roll {slot} should have opened a frame"
            );
        }
        // The fifth is held: nothing to raise `START_LOOT_ROLL` for.
        opened(&mut rolls, 7, 4, 858, 0.0);
        assert_eq!(rolls.len(), 5, "held, not dropped");
        assert_eq!(rolls.show_next(), None, "and not promoted while full");

        rolls.show_next();
        // One frame comes down and the **deferred** roll takes it — not the
        // one that just vacated it. That is the whole reason `deferred` and
        // `showing` are two flags: a roll whose frame closed is spent, and one
        // flag doing both jobs would re-open the frame that just shut.
        assert!(rolls.hide(0));
        assert_eq!(rolls.show_next(), Some(4));
        assert_eq!(rolls.show_next(), None, "and roll 0 is not offered again");
    }

    /// A roll is named two ways and they are crossed here. The wire's name is
    /// what a result carries; the interface's is what a button carries.
    #[test]
    fn a_result_finds_the_roll_by_the_wires_name_for_it() {
        let mut rolls = LootRolls::default();
        opened(&mut rolls, 7, 3, 2589, 0.0);
        opened(&mut rolls, 9, 3, 858, 0.0);
        assert_eq!(rolls.find(7, 3), Some(0));
        assert_eq!(rolls.find(9, 3), Some(1), "the same slot on another body");
        assert_eq!(rolls.find(7, 4), None);
        rolls.close(0);
        assert_eq!(rolls.find(7, 3), None);
        assert_eq!(rolls.len(), 1);
    }

    /// **Hiding twice owes one cancel**, which is what stops a vote and the
    /// result that follows it raising the event for the same frame twice — and
    /// what keeps the shown count from drifting under the cap.
    #[test]
    fn only_the_first_hide_owes_the_interface_anything() {
        let mut rolls = LootRolls::default();
        opened(&mut rolls, 7, 0, 2589, 0.0);
        rolls.show_next();
        assert!(rolls.hide(0));
        assert!(!rolls.hide(0), "twice is not two frames");
        assert!(!rolls.hide(99), "and neither is an id nothing holds");
        // …and the freed frame is genuinely free, which a drifted count would
        // hide: four more must all open.
        for slot in 1..=MAX_ROLL_FRAMES as u32 {
            opened(&mut rolls, 7, slot, 2589, 0.0);
            assert!(rolls.show_next().is_some(), "{slot}");
        }
    }

    /// `GetLootRollTimeLeft` is milliseconds and **is allowed to go negative** —
    /// the clamp is the shipped `OnUpdate`'s, not the C function's.
    #[test]
    fn the_clock_is_milliseconds_and_the_client_does_not_clamp_it() {
        let mut rolls = LootRolls::default();
        opened(&mut rolls, 7, 0, 2589, 100.0);
        assert_eq!(rolls.time_left_ms(0, 100.0), Some(60_000.0));
        assert_eq!(rolls.time_left_ms(0, 130.0), Some(30_000.0));
        assert_eq!(rolls.time_left_ms(0, 200.0), Some(-40_000.0));
        assert_eq!(rolls.time_left_ms(9, 100.0), None);
    }

    /// A voted roll keeps its record — the result still has to find it — but it
    /// never takes a frame again, which is what the promotion walk would
    /// otherwise do to it the moment one freed up.
    #[test]
    fn a_voted_roll_is_never_given_a_frame_back() {
        let mut rolls = LootRolls::default();
        opened(&mut rolls, 7, 0, 2589, 0.0);
        rolls.show_next();
        rolls.voted(0);
        rolls.hide(0);
        assert_eq!(rolls.show_next(), None);
        assert_eq!(rolls.len(), 1, "the record outlives its frame");
        assert!(rolls.get(0).expect("still held").voted);
    }

    /// **A roll with no name yet gets no frame**, which is the whole of the hold
    /// rule: `GroupLootFrame_OnShow` runs once and there is no refresh event, so
    /// a frame opened a round trip early stays blank for the roll's whole
    /// minute.
    #[test]
    fn a_roll_whose_item_has_no_name_is_not_drawn_yet() {
        let mut rolls = LootRolls::default();
        let id = rolls.open(&start(7, 0, 2589), 0.0);
        assert_eq!(rolls.show_next(), None, "nothing to draw on it");
        assert_eq!(rolls.len(), 1, "held, not dropped");
        // …and the frame it was waiting for is not spent on the roll behind it.
        let second = rolls.open(&start(7, 1, 858), 0.0);
        rolls.name_landed(second);
        assert_eq!(rolls.show_next(), Some(second), "the named one goes first");
        rolls.name_landed(id);
        assert_eq!(rolls.show_next(), Some(id));
        assert_eq!(rolls.show_next(), None);
    }

    /// The ids are unique for the life of the process, so a stale press cannot
    /// vote on somebody else's roll.
    #[test]
    fn an_id_is_never_reused_within_a_session() {
        let mut rolls = LootRolls::default();
        opened(&mut rolls, 7, 0, 2589, 0.0);
        rolls.close(0);
        assert_eq!(opened(&mut rolls, 7, 0, 2589, 0.0), 1);
        assert_eq!(rolls.get(0), None, "the old id resolves to nothing");
    }
}
