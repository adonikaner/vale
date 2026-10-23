//! Group loot rolls: the client side of [`vale_protocol::play::lootroll`].
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
//! ## The `rollID` is assigned by this client
//!
//! No packet names a roll. Every packet says `(guid, item slot)`, and the
//! interface uses `rollID`, one number passed to `GetLootRollItemInfo`,
//! `GetLootRollTimeLeft` and `RollOnLoot`. In the 1.12.1 client it is a
//! counter over the pending list, incremented once per roll. [`LootRolls`] is
//! that list and [`PendingRoll`] is one entry of it, and the conversion between
//! the two names happens here and nowhere else, as with the loot window's two
//! numberings.
//!
//! ## A fifth roll is deferred, not dropped and not drawn
//!
//! `NUM_GROUP_LOOT_FRAMES` is 4, and the 1.12.1 client enforces it: when four
//! frames are shown, a new roll is marked deferred and no event is raised.
//! Whenever a frame is freed, the first deferred roll in the list is promoted.
//!
//! This is reproduced exactly, including the part that looks like a bug: the
//! promotion sends the roll's original countdown rather than the time left,
//! so a deferred frame opens with a full bar over a clock that is already
//! partly run. `GroupLootFrame_OnUpdate` then clamps (`if ( (left < min) or
//! (left > max) ) then left = min`), which compensates for it in the shipped
//! Lua. Sending the remainder would make the bar disagree with that clamp.
//!
//! ## A frame waits until its item has a name, as the loot window does
//!
//! `SMSG_LOOT_START_ROLL` carries an entry and no display id, so a roll on an
//! item this character has not seen has nothing to draw: no name, no icon, no
//! quality colour. `GroupLootFrame_OnShow` reads all five of
//! `GetLootRollItemInfo`'s values in one line and runs once, when the frame is
//! shown, so a frame opened a round trip early stays blank for the whole minute
//! the roll lasts.
//!
//! The 1.12.1 client does not open it early. It raises `START_LOOT_ROLL`
//! immediately when the entry is already in its item cache, and otherwise when
//! the cache's query completes. [`super::loot::hold`] does the same for the
//! same reason; here the wait is bounded by [`NAME_LIMIT`] so that a lost query
//! cannot keep the roll from being offered.
//!
//! A refusal counts as an answer, as in the loot window: vmangos replies
//! `entry | 0x80000000` for an item it will not describe, and a row the server
//! will never name is not worth waiting for.
//!
//! ## Rolling on a bind-on-pickup item asks first
//!
//! A vote other than Pass, on a roll whose item has `Bonding == 1`, raises
//! `CONFIRM_LOOT_ROLL(id, rollType)` and sends nothing. `UIParent.lua` shows a
//! `StaticPopup_Show`, whose accept calls `ConfirmLootRoll(id, rollType)`: the
//! same entry point with the flag set, which skips the check. The
//! confirmation is local, and the server is not told about the first press.
//!
//! The item template decides, and it may not have arrived. A roll is often on
//! an item this character has not seen, whose template is a round trip away,
//! the same cold-cache case the loot window waits for. Here the choice is the
//! opposite: an unknown template is treated as not bind-on-pickup, so the
//! press is sent. A press that did nothing while a query was in flight is
//! worse than a missing confirmation, and the roll has a time limit.
//!
//! ## The chat lines, and the `showLootSpam` CVar
//!
//! Every announcement is a `GlobalStrings.lua` key filled with the roller's
//! name, the item's link and, for two of the four, the number. The keys, the
//! branch that chooses between them and the `showLootSpam` rule are in
//! [`vale_protocol::play::lootroll`], where a test can run them with no world.
//! This file fills them: a name from the object manager, a link from the item
//! cache, and one `CHAT_MSG_LOOT` per line.
//!
//! A line whose pieces have not arrived is not composed. The name of a group
//! member across the zone and the template of an item nobody has seen are
//! both a query away, and a line reading `Player 4213 has selected Need for:
//! []` is worse than no line; the roll frame is on screen either way and shows
//! the item.
//!
//! ## The row on the body is freed without a packet
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
/// [`crate::world::incoming::drain_events`], the same route
/// [`super::loot::LootAnswer`] takes, for the same reason: the session queue
/// has one reader.
#[derive(Message, Debug, Clone, Copy)]
pub(crate) enum RollAnswer {
    Started(RollStart),
    Cast(RollCast),
    Won(RollWon),
    AllPassed(RollAllPassed),
}

/// One roll the client is holding, whether or not a frame is showing it.
///
/// The fields of the 1.12.1 client's pending-roll record, minus those this
/// client does not use: the two random-property words are always zero in 1.12,
/// and the item link is composed where it is needed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingRoll {
    /// The interface's id for the roll, assigned by this client. See the
    /// module note.
    pub id: u32,
    /// The server's name for the roll, `(guid, item_slot)`, which a vote
    /// carries.
    pub guid: u64,
    pub item_slot: u32,
    pub entry: u32,
    /// The countdown the start packet gave, kept because the promotion of a
    /// deferred roll sends it rather than the remainder; see the module note.
    pub(crate) countdown_ms: u32,
    /// When the roll runs out, on [`Time`]'s clock. `GetLootRollTimeLeft` is
    /// this minus now, in milliseconds, and may be negative: the shipped
    /// `OnUpdate` clamps.
    pub(crate) deadline: f64,
    /// When the start arrived, on [`Time`]'s clock. [`NAME_LIMIT`] is measured
    /// from it.
    opened_at: f64,
    /// The item can be named, so a frame would draw something. False until the
    /// template arrives or the wait runs out; see the module note.
    named: bool,
    /// The roll has never had a frame, because all four were in use when it
    /// arrived. The promotion looks for this flag.
    ///
    /// Cleared on promotion and never set again, which is how it differs from
    /// `showing`: a roll whose frame closed because the player voted must not
    /// get a new frame when the next one is freed, and a single flag for both
    /// would give it one.
    deferred: bool,
    /// Whether a frame is showing the roll now, which decides whether hiding
    /// it raises `CANCEL_LOOT_ROLL`.
    showing: bool,
    /// The player has voted, so the frame is closed and the record waits only
    /// for the result. It prevents a second press from being sent.
    voted: bool,
}

/// Every roll this session is holding, in arrival order.
///
/// A `Vec` and not a map: it holds at most a handful of entries, every access
/// is either "find this id" or "the first deferred one", and at this size an
/// index would not be faster than a scan. The 1.12.1 client keeps a list
/// searched the same way.
#[derive(Resource, Default)]
pub struct LootRolls {
    rolls: Vec<PendingRoll>,
    /// The roll counter. Never reset within a session, so an id is
    /// unique for the life of the process and a stale `RollOnLoot(3)` from an
    /// addon cannot vote on somebody else's roll.
    next_id: u32,
    /// How many of [`Self::rolls`] have a frame, stored rather than counted so
    /// the cap check needs no scan.
    shown: usize,
}

impl LootRolls {
    /// One roll by the interface's id.
    pub fn get(&self, id: u32) -> Option<&PendingRoll> {
        self.rolls.iter().find(|roll| roll.id == id)
    }

    /// Whether any roll is held. Every system in this file checks this before
    /// doing any work, so a session with no group does nothing more.
    pub fn is_empty(&self) -> bool {
        self.rolls.is_empty()
    }

    /// How many rolls are held, frames and deferrals together.
    pub fn len(&self) -> usize {
        self.rolls.len()
    }

    /// `GetLootRollTimeLeft(id)`: milliseconds, possibly negative.
    ///
    /// The 1.12.1 client returns `deadline - now` with no clamp; the clamping
    /// is done by `GroupLootFrame_OnUpdate`, against the bar's min and max.
    /// `None` for an id nothing holds, for which the 1.12.1 client returns no
    /// values.
    pub(crate) fn time_left_ms(&self, id: u32, now: f64) -> Option<f64> {
        Some((self.get(id)?.deadline - now) * 1000.0)
    }

    /// Add a started roll to the list and return the id it was given.
    ///
    /// It does not open a frame. [`show_next`] checks whether a frame is free
    /// and the clock checks whether the item can be named, both from
    /// [`promote`], which also picks up a roll deferred behind four others. There
    /// is therefore one path to `START_LOOT_ROLL` rather than two.
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
    /// Returns the id to raise `START_LOOT_ROLL` for. One at a time rather than
    /// a loop, so the caller writes one message per promotion and the counting
    /// stays in the caller's loop.
    fn show_next(&mut self) -> Option<u32> {
        if self.shown >= MAX_ROLL_FRAMES {
            return None;
        }
        // Named as well as deferred. A frame given to a roll that cannot be
        // named yet draws an empty square with no text for the whole minute;
        // see the module note.
        let roll = self.rolls.iter_mut().find(|roll| roll.deferred && roll.named)?;
        roll.deferred = false;
        roll.showing = true;
        self.shown += 1;
        Some(roll.id)
    }

    /// Close the frame for this roll, because the player voted or the roll
    /// ended.
    ///
    /// Returns whether `CANCEL_LOOT_ROLL` must be raised: a roll that never had
    /// a frame has nothing to hide, and raising the event for it would cost a
    /// Lua call in each of the four `GroupLootFrame`s for nothing.
    fn hide(&mut self, id: u32) -> bool {
        let Some(roll) = self.rolls.iter_mut().find(|roll| roll.id == id) else {
            return false;
        };
        if !roll.showing {
            return false;
        }
        roll.showing = false;
        // Not `deferred = true`. A roll whose frame has been used does not get
        // another; the 1.12.1 client clears its deferred flag on promotion and
        // never sets it again.
        self.shown = self.shown.saturating_sub(1);
        true
    }

    /// The roll a result names, by the server's name for it.
    fn find(&self, guid: u64, item_slot: u32) -> Option<u32> {
        self.rolls
            .iter()
            .find(|roll| roll.guid == guid && roll.item_slot == item_slot)
            .map(|roll| roll.id)
    }

    /// Forget a roll whose result has arrived.
    fn close(&mut self, id: u32) {
        self.rolls.retain(|roll| roll.id != id);
    }

    /// Mark a roll as drawable: its template arrived, or the wait for it ran
    /// out. See the module note.
    fn name_landed(&mut self, id: u32) {
        if let Some(roll) = self.rolls.iter_mut().find(|roll| roll.id == id) {
            roll.named = true;
        }
    }

    /// Every roll still waiting for a name, as `(id, entry, opened_at)`.
    ///
    /// Returns an empty vector once everything is named, so the clock does not
    /// take the world lock on every frame of a roll's minute.
    fn unnamed(&self) -> Vec<(u32, u32, f64)> {
        self.rolls
            .iter()
            .filter(|roll| !roll.named)
            .map(|roll| (roll.id, roll.entry, roll.opened_at))
            .collect()
    }

    /// Mark a vote as sent. The frame is already closed; this prevents a
    /// second vote.
    fn voted(&mut self, id: u32) {
        if let Some(roll) = self.rolls.iter_mut().find(|roll| roll.id == id) {
            roll.voted = true;
        }
    }
}

/// How long, past its countdown, a roll is kept before this client forgets
/// it.
///
/// This differs from the 1.12.1 client. vmangos' `Group::Update` resolves a
/// timed-out roll itself and the result always arrives, so the 1.12.1
/// client never expires a record: a pending roll lives until a win or an
/// all-passed names it.
///
/// That fails when the server never answers: a creature unloaded during the
/// roll, a group disbanded, a reconnect. The 1.12.1 client then leaves a
/// `GroupLootFrame` on screen for the rest of the session with its bar at
/// zero, which is worse than the risk this takes. The value is large so that
/// a packet arriving up to a quarter of a minute late still finds its roll.
const EXPIRY_GRACE: f64 = 15.0;

pub(crate) struct LootRollPlugin;

impl Plugin for LootRollPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RollAnswer>()
            .add_message::<RollPress>()
            .init_resource::<LootRolls>()
            .add_systems(
                Update,
                // The clock runs last, so a start whose template is already
                // cached opens its frame on the frame it arrived.
                (announce, presses, vote, clock)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// A button press on a `GroupLootFrame`, or an addon calling `RollOnLoot`.
///
/// A message rather than a direct send, as every other interface write in this
/// client is; see [`crate::lua::api::verbs`].
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RollPress {
    pub id: u32,
    pub(crate) vote: RollVote,
    /// The bind-on-pickup confirmation has already been shown. False for
    /// `RollOnLoot`, true for `ConfirmLootRoll`; see the module note.
    pub(crate) confirmed: bool,
}

/// Apply the server's packets to the list, raise the interface events, and
/// print the chat lines.
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
    // The `showLootSpam` setting, read once per drain rather than per packet:
    // it is a string compare, and a roll among five people is five packets.
    let spam = cvars.flag(SHOW_LOOT_SPAM);
    for answer in answers.read() {
        match answer {
            RollAnswer::Started(start) => {
                // The name and the icon are a round trip away for an item this
                // character has not seen. The template is queued now
                // (`LiveSession::want_item`, which skips a known entry) so the
                // frame's `OnShow` has them. The loot window does the same for
                // its rows.
                want(&session, start.entry);
                // No frame yet. The clock opens one once a frame is free and the
                // item can be named; see the module note.
                rolls.open(start, now);
            }
            // Neither a window event nor a frame event: another player's vote
            // is a chat line and nothing else. The voter's frame closed on
            // their own client.
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
                    // The number only when the quiet setting includes it in this
                    // line; with `showLootSpam` on, it already had its own line.
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
                // The one line in the family with no branch and no roller:
                // `LOOT_ROLL_ALL_PASSED` names only the item.
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

/// Give every free frame to a roll waiting for one.
///
/// Called from the three places a frame can be freed: a result arriving, the
/// player voting, and a roll expiring. With nothing waiting, it costs one
/// `shown >= MAX_ROLL_FRAMES` check and no allocation.
fn promote(rolls: &mut LootRolls, started: &mut MessageWriter<StartLootRoll>) {
    while let Some(id) = rolls.show_next() {
        let Some(roll) = rolls.get(id) else { break };
        // The original countdown, not the remainder. The module note gives the
        // shipped `OnUpdate` clamp that compensates for it.
        let countdown_ms = roll.countdown_ms;
        started.write(StartLootRoll { id, countdown_ms });
    }
}

/// End a roll: close its frame, free its row on the body, and forget it.
///
/// A function rather than a system, as [`super::loot::close_if_empty`] is: only
/// the two packets that end a roll need it, and both do exactly this.
fn finish(
    rolls: &mut LootRolls,
    window: &mut super::loot::LootWindow,
    cancelled: &mut MessageWriter<CancelLootRoll>,
    guid: u64,
    item_slot: u32,
) {
    // The row is freed whether or not this client knew about the roll. A
    // member who joined the group after the roll started was sent no start
    // packet and still has the body open with a blocked row.
    //
    // The slot is a `u32` here and a `u8` in the window, and the two are the
    // same number: vmangos' `LootView` writes `uint8(i)`, and the roll packets
    // widen it. A slot that does not fit in a `u8` comes from a server this
    // client cannot follow, so nothing is freed rather than truncating to a row
    // that would be unblocked by mistake.
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
/// A separate system rather than a `NonSendMut` on [`vote`], for the reason
/// [`super::loot::presses`] gives: a non-send parameter pins its system to the
/// main thread.
fn presses(host: Option<NonSendMut<crate::lua::host::LuaHost>>, mut out: MessageWriter<RollPress>) {
    let Some(host) = host else { return };
    for press in crate::lua::panels::lootroll::take(&host) {
        out.write(press);
    }
}

/// Send what the player pressed, or ask for confirmation first.
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
        // A roll already voted on sends nothing. The 1.12.1 client also ignores
        // a press once it has voted, which prevents a scripted double press or
        // an addon racing the button from sending twice.
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
        // The frame closes on the press, not on the answer: the 1.12.1 client
        // sets its voted flag and raises `CANCEL_LOOT_ROLL` before anything
        // comes back. The record stays, because the result still has to find
        // it.
        if rolls.hide(press.id) {
            cancelled.write(CancelLootRoll { id: press.id });
        }
        // The freed frame goes to the next waiting roll on the clock's next
        // run, so there is one path to `START_LOOT_ROLL`, not three.
    }
}

/// The roll list's clock: give a named roll a frame, and forget one the server
/// never finished.
///
/// The one system here that runs on an ordinary frame. For a session not in a
/// group it does only the `is_empty` check on its first line.
fn clock(
    mut rolls: ResMut<LootRolls>,
    session: Res<Session>,
    time: Res<Time>,
    mut cancelled: MessageWriter<CancelLootRoll>,
    mut started: MessageWriter<StartLootRoll>,
) {
    // The whole cost of this file for a session that is not in a group.
    if rolls.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();

    // Forget rolls the server never finished; see [`EXPIRY_GRACE`].
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

    // Give a frame to every roll that can now be named. The world lock is
    // taken only while a roll is still waiting for a template, which is one
    // round trip out of a roll's minute.
    let waiting = rolls.unnamed();
    if !waiting.is_empty() {
        for id in settled(&session, &waiting, now) {
            rolls.name_landed(id);
        }
    }
    promote(&mut rolls, &mut started);
}

/// How long, in seconds, a roll may wait for its item to have a name.
///
/// The same bound, for the same reason, as [`super::loot::HOLD_LIMIT`]: against
/// a local server the wait is one session tick and one round trip, and the
/// bound covers an answer that never arrives. A frame that opens blank is
/// better than a roll the player is never offered, and the roll's countdown
/// runs either way.
const NAME_LIMIT: f64 = 3.0;

/// Which of the waiting rolls can be drawn now.
///
/// A refusal counts, as in [`super::loot::settled`]: vmangos answers
/// `entry | 0x80000000` for an item it will not describe, which is stored in
/// `unknown_items`, and a roll the server will never name is not worth waiting
/// for.
fn settled(session: &Session, waiting: &[(u32, u32, f64)], now: f64) -> Vec<u32> {
    let named: Option<Vec<u32>> = session.active.as_ref().map(|active| {
        let world = active.live.read();
        waiting
            .iter()
            .filter(|(_, entry, _)| {
                world.items.contains_key(entry) || world.unknown_items.contains(entry)
            })
            .map(|(id, _, _)| *id)
            .collect()
    });
    // With no session no answers will come, so nothing waits. Every unit test
    // here runs in this state.
    let named = named.unwrap_or_default();
    waiting
        .iter()
        .filter(|(id, _, opened_at)| {
            named.contains(id) || session.active.is_none() || now - opened_at >= NAME_LIMIT
        })
        .map(|(id, _, _)| *id)
        .collect()
}

/// Queue an item template query. `LiveSession::want_item` takes the world lock
/// and skips an entry the session can already name.
fn want(session: &Session, entry: u32) {
    if let Some(active) = session.active.as_ref() {
        active.live.want_item(entry);
    }
}

/// Whether rolling on this item needs a confirmation: `Bonding == 1`. An item
/// whose template has not arrived is treated as not; see the module note.
fn binds_on_pickup(session: &Session, entry: u32) -> bool {
    let Some(active) = session.active.as_ref() else {
        return false;
    };
    let world = active.live.read();
    world.items.get(&entry).is_some_and(|item| item.bonding == 1)
}

/// One `CHAT_MSG_LOOT` line, said by nobody.
///
/// The author is empty because these lines are the client's output rather
/// than a player's speech. A system line has the same shape, which
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

/// A roll line about a player, or `None` while one of its pieces has not
/// arrived.
///
/// The arguments are in the 1.12.1 client's order. Every branch ends with the
/// same seven (see [`link_pieces`]) and they differ only in what comes before
/// them:
///
/// ```text
/// "You passed on: …"                  <seven>
/// "%s passed on: …"            name    <seven>
/// "You won: … (Need - %1$d)"   number  <seven>
/// "%s won: … (Need - %2$d)"    name  number  <seven>
/// "Need Roll - %d for … by %s" number  <seven>  name
/// ```
///
/// The last of those is why the name does not always come first.
/// `LOOT_ROLL_ROLLED_NEED` is the one key in the family that puts the roller at
/// the end of the line, and filling it in the other order writes the name
/// where the roll number goes. See [`RollLine::names_the_roller_last`].
fn compose(
    session: &Session,
    strings: &super::messages::UiStrings,
    entry: u32,
    who: u64,
    number: Option<u8>,
    // Whether the key names the roller after the link rather than before it:
    // `RollLine::names_the_roller_last`, true only for the roll result.
    trailing: bool,
    key: impl FnOnce(bool) -> &'static str,
) -> Option<String> {
    let active = session.active.as_ref()?;
    let world = active.live.read();
    let is_self = world.player_guid == Some(who);
    let name = world.players.get(&who)?.name.clone();
    let item = world.items.get(&entry)?;
    let pieces = link_pieces(item.entry, item.quality, &item.name);
    let key = key(is_self);
    drop(world);

    let number = number.map(|number| number.to_string());
    let last = trailing.then_some(name.as_str());
    let mut arguments: Vec<&str> = Vec::with_capacity(10);
    // The self forms take no name ("You passed on: …" against "%s passed on:
    // …"), and the trailing form takes the name after the link.
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

/// A roll line about an item only: `LOOT_ROLL_ALL_PASSED`.
fn item_line(
    session: &Session,
    strings: &super::messages::UiStrings,
    entry: u32,
    key: &str,
) -> Option<String> {
    let active = session.active.as_ref()?;
    let world = active.live.read();
    let item = world.items.get(&entry)?;
    let pieces = link_pieces(item.entry, item.quality, &item.name);
    drop(world);
    let arguments: Vec<&str> = pieces.iter().map(String::as_str).collect();
    fill(strings, key, &arguments)
}

/// The `GlobalStrings.lua` sentence, filled.
///
/// `None` for a key the file does not contain, by the rule
/// [`crate::interface::messages`] states for every other line this client
/// shows: a key that resolves to nothing shows nothing, rather than the chat
/// window printing `LOOT_ROLL_PASSED`.
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

/// The item hyperlink split into its pieces, because the shipped keys are
/// written against the pieces rather than the whole link.
///
/// `LOOT_ROLL_PASSED` is `"%s passed on: %s|Hitem:%d:%d:%d:%d|h[%s]|h%s"`:
/// seven slots after the name, of which only the first looks like the link.
/// Passing [`crate::lua::panels::container::item_link`]'s whole string to the
/// second slot fills one and leaves six directives printed in the chat window.
///
/// The seven, in the 1.12.1 client's argument order: the quality colour, the
/// entry, the enchant, the random property, the random suffix, the item's
/// name, and the terminator. The three zeros come from the server: vmangos
/// writes zeros for the two random words in 1.12, and nothing enchants a
/// corpse drop, so this is the link `item_link` builds, written out.
///
/// The terminator is dropped with the colour, as the 1.12.1 client does:
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

    /// Open a roll and mark it drawable at once, the state every test below
    /// uses. The wait for a name has its own test.
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

    /// Four frames, and the fifth roll waits: the cap the 1.12.1 client
    /// enforces, and the promotion when a frame is freed.
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
        // One frame closes and the deferred roll takes it, not the roll that
        // just left it. This is why `deferred` and `showing` are two flags: a
        // roll whose frame closed does not get another, and a single flag for
        // both would reopen the frame that just closed.
        assert!(rolls.hide(0));
        assert_eq!(rolls.show_next(), Some(4));
        assert_eq!(rolls.show_next(), None, "and roll 0 is not offered again");
    }

    /// A roll has two names, converted here. A result carries the server's
    /// name; a button carries the interface's.
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

    /// Hiding twice raises one cancel. This keeps a vote and the result that
    /// follows it from raising the event twice for the same frame, and keeps
    /// the shown count from drifting under the cap.
    #[test]
    fn only_the_first_hide_owes_the_interface_anything() {
        let mut rolls = LootRolls::default();
        opened(&mut rolls, 7, 0, 2589, 0.0);
        rolls.show_next();
        assert!(rolls.hide(0));
        assert!(!rolls.hide(0), "twice is not two frames");
        assert!(!rolls.hide(99), "and neither is an id nothing holds");
        // The freed frame is free, which a drifted count would hide: four more
        // must all open.
        for slot in 1..=MAX_ROLL_FRAMES as u32 {
            opened(&mut rolls, 7, slot, 2589, 0.0);
            assert!(rolls.show_next().is_some(), "{slot}");
        }
    }

    /// `GetLootRollTimeLeft` is in milliseconds and may be negative: the clamp
    /// is in the shipped `OnUpdate`, not in the API function.
    #[test]
    fn the_clock_is_milliseconds_and_the_client_does_not_clamp_it() {
        let mut rolls = LootRolls::default();
        opened(&mut rolls, 7, 0, 2589, 100.0);
        assert_eq!(rolls.time_left_ms(0, 100.0), Some(60_000.0));
        assert_eq!(rolls.time_left_ms(0, 130.0), Some(30_000.0));
        assert_eq!(rolls.time_left_ms(0, 200.0), Some(-40_000.0));
        assert_eq!(rolls.time_left_ms(9, 100.0), None);
    }

    /// A voted roll keeps its record, because the result still has to find it,
    /// but it never takes a frame again, which the promotion would otherwise
    /// give it as soon as one was freed.
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

    /// A roll with no name yet gets no frame: `GroupLootFrame_OnShow` runs once
    /// and there is no refresh event, so a frame opened a round trip early
    /// stays blank for the roll's whole minute.
    #[test]
    fn a_roll_whose_item_has_no_name_is_not_drawn_yet() {
        let mut rolls = LootRolls::default();
        let id = rolls.open(&start(7, 0, 2589), 0.0);
        assert_eq!(rolls.show_next(), None, "nothing to draw on it");
        assert_eq!(rolls.len(), 1, "held, not dropped");
        // The unnamed roll does not hold the free frame: the named roll behind
        // it takes it.
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
