//! The loot window: what is on a body while the window is open.
//!
//! The client side of [`vale_protocol::play::loot`]: one window at a time,
//! kept open for as long as the server says it is open, and the three events
//! `LootFrame.lua` handles.
//!
//! ```text
//! right-click a corpse  -> LiveSession::loot(guid)
//!   SMSG_LOOT_RESPONSE  -> LootWindow::open   -> LOOT_OPENED
//!   click a row         -> LiveSession::loot_item(server index)
//!   SMSG_LOOT_REMOVED   -> LootWindow::remove -> LOOT_SLOT_CLEARED(row)
//!   close it            -> LiveSession::loot_release(guid)
//!   SMSG_LOOT_RELEASE_RESPONSE            -> LOOT_CLOSED
//! ```
//!
//! ## Three rules, each with a visible fault when broken
//!
//! `LOOT_SLOT_CLEARED` carries the row, not the server's index.
//! `LootFrame_OnEvent` computes `local slot = arg1 - ((this.page - 1) *
//! numLootToShow)` and hides `LootButton<slot>`; given the server's sparse
//! index it hides the wrong button, or none. The conversion happens once, in
//! [`vale_protocol::play::loot::Loot::remove`].
//!
//! The window closes when the server says so. vmangos'
//! `HandleLootReleaseOpcode` discards the guid the client sends, so the only
//! statement that the body is closed is `SMSG_LOOT_RELEASE_RESPONSE`. Clearing
//! the window when the release is sent would leave the server holding a body
//! this client believes it released, and the next right-click on it would do
//! nothing.
//!
//! A refusal opens no window and shows a message. `SMSG_LOOT_RESPONSE` is the
//! same opcode either way; a refusal reaches [`crate::interface::messages`] as
//! the error line its `ERR_LOOT_*` key names, which the 1.12.1 client shows in
//! the red text at the top of the screen.
//!
//! ## `LOOT_OPENED` waits until the rows have names
//!
//! `LootFrame` is the one panel in this directory with no refresh event.
//! `MerchantFrame` reads its rows again on `MERCHANT_UPDATE`, `QuestFrame` on
//! `QUEST_ITEM_UPDATE`, `ContainerFrame` on `BAG_UPDATE`; each of those event
//! arms calls the panel's `*_Update`. `LootFrame_OnEvent`'s `LOOT_OPENED` arm
//! is `ShowUIPanel(this)` and nothing else, and `UIParent.lua`'s `ShowUIPanel`
//! returns immediately for a frame that is already visible. `LootFrame_Update`
//! is called from three places in the shipped file: `LootFrame_OnShow` and the
//! two page buttons.
//!
//! Raising `LOOT_OPENED` again at an open loot window therefore does nothing,
//! and a row whose template arrived just after the window opened stays blank
//! for as long as the body is open. An item the character had not seen before
//! drew its icon (`SMSG_LOOT_RESPONSE` carries a display id) with no name.
//!
//! Hiding and showing the frame again does not work either:
//! `LootFrame_OnHide` calls `CloseLoot()`, which releases the body.
//!
//! This is inferred, not measured: with no refresh path in the Lua, and with
//! the 1.12.1 client never drawing blank loot rows, that client must have the
//! names before it raises the event. Its item cache's completion callback is
//! where it would get them (see [`crate::world::templates`]). This module
//! reproduces the behaviour, a window that opens with names, and not a
//! verified route to it.
//!
//! [`hold`] implements it: the answer is stored at once, and the event waits
//! until every row is named or refused, bounded by [`HOLD_LIMIT`] so that a
//! lost query cannot keep the window from opening. On a local server the wait
//! is one tick and one round trip, about 30 ms.
//!
//! ## The last row taken closes the window
//!
//! Measured against the running server: emptying a one-row corpse with
//! `vale live` (`loot 1`, `take 1`) returns `SMSG_LOOT_REMOVED` and then
//! nothing over 21 seconds of listening. vmangos keeps the body open for as
//! long as the client does.
//!
//! The shipped interface does not close it either: `LootFrame_OnEvent`'s
//! `LOOT_SLOT_CLEARED` arm hides the button and then pages down only if every
//! button is hidden and there is a next page. With no next page it returns,
//! and the window stays open and blank until the player presses the X.
//!
//! The close is therefore this client's own, inferred from those two
//! measurements. [`announce`] sends [`TakeLoot::Close`] when
//! [`vale_protocol::play::loot::Loot::is_empty`] becomes true on a removal,
//! and the window closes as every other close does, on
//! `SMSG_LOOT_RELEASE_RESPONSE`.
//!
//! The check runs on a removal and never on the response. A body can arrive
//! empty, and the game has a sound for that (`LootFrame_OnShow` plays
//! `LOOTWINDOWOPENEMPTY` when `GetNumLootItems()` is zero), so a window that
//! closed itself on opening would differ from the 1.12.1 client.
//!
//! ## The money is two packets
//!
//! `SMSG_LOOT_CLEAR_MONEY` empties the coin row and `SMSG_LOOT_MONEY_NOTIFY`
//! states the player's share, which in a group is a fraction. They are handled
//! separately for the reason the protocol module gives: the notify goes to
//! everyone in range and the clear only to those with the window open, so
//! treating one as the other leaves a coin row nobody can click.

use bevy::prelude::*;

use vale_protocol::play::loot::Loot;

use super::events::{LootClosed, LootOpened, LootSlotCleared};
use crate::world::session::Session;
use crate::world::templates::TemplatesArrived;

/// One of the five packets, handed on from
/// [`crate::world::incoming::drain_events`].
///
/// A message rather than a second drain of the session queue, as
/// [`super::timers::MirrorTimerAnswer`] is: the queue has one reader, and a
/// second would take part of its events.
#[derive(Message, Debug, Clone)]
pub enum LootAnswer {
    /// `SMSG_LOOT_RESPONSE`: the window, or a refusal sent with the same
    /// opcode.
    Opened(Loot),
    /// `SMSG_LOOT_REMOVED`, by the server's index.
    Removed(u8),
    /// `SMSG_LOOT_CLEAR_MONEY`.
    MoneyCleared,
    /// `SMSG_LOOT_MONEY_NOTIFY`: the player's share, in copper.
    MoneyGained(u32),
    /// `SMSG_LOOT_RELEASE_RESPONSE`: the body is closed.
    Closed(u64),
}

/// The open loot window, or nothing.
///
/// One at a time, as on the server: `Player::GetLootGuid` is a single guid,
/// and opening a second body releases the first.
#[derive(Resource, Default)]
pub struct LootWindow {
    open: Option<Loot>,
    /// The time the answer arrived, while `LOOT_OPENED` has not been raised
    /// yet. See [`hold`] and the module note: the event draws the rows, and it
    /// is raised only once they can be named.
    holding: Option<f32>,
}

impl LootWindow {
    /// What is on the body, or `None` while no window is up.
    pub fn get(&self) -> Option<&Loot> {
        self.open.as_ref()
    }

    /// Whether the window has an answer it has not drawn yet; see [`hold`].
    pub fn is_holding(&self) -> bool {
        self.holding.is_some()
    }

    /// Whose body it is, which a release names.
    pub fn guid(&self) -> Option<u64> {
        Some(self.open.as_ref()?.guid)
    }

    /// `GetNumLootItems()`: rows, coins included. 0 while nothing is open,
    /// which `LootFrame_OnShow` reads as an empty body.
    pub fn rows(&self) -> usize {
        self.open.as_ref().map_or(0, Loot::rows)
    }

    /// Free a row a group roll was holding. This is the one rule in the loot
    /// packets with no packet of its own behind it.
    ///
    /// Applied only if the window is the body the roll named: a result can
    /// arrive about a corpse this client never opened, or one it opened,
    /// released and replaced. See [`vale_protocol::play::loot::Loot::unblock`],
    /// where the rule is documented, and [`super::lootroll`], the only caller.
    pub(super) fn unblock_row(&mut self, guid: u64, index: u8) {
        let Some(loot) = self.open.as_mut().filter(|loot| loot.guid == guid) else {
            return;
        };
        loot.unblock(index);
    }

    /// Whether the window is a fishing catch: `IsFishingLoot()`.
    pub fn is_fishing(&self) -> bool {
        self.open
            .as_ref()
            .and_then(|loot| loot.kind)
            .is_some_and(|kind| kind.is_fishing())
    }
}

pub struct LootPlugin;

impl Plugin for LootPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LootAnswer>()
            .add_message::<TakeLoot>()
            .init_resource::<LootWindow>()
            .add_message::<LootNamesLanded>()
            .add_message::<TemplatesArrived>()
            .add_systems(
                Update,
                (announce, hold, presses, take)
                    .chain()
                    .in_set(super::GameSet),
            )
            .add_systems(
                Update,
                templates_landed
                    .after(crate::world::incoming::drain_queues)
                    .in_set(super::GameSet),
            );
    }
}

/// Write [`LootNamesLanded`] when templates arrive while a window is open.
///
/// The loot window has no refresh event: 5875 has no `LOOT_SLOT_CHANGED`, and
/// `LootFrame_OnEvent`'s `LOOT_OPENED` arm is `ShowUIPanel(this)`, which
/// `UIParent.lua` returns from at once for a frame that is already visible.
/// The window is held shut until its rows can be named ([`hold`]); this
/// message covers a hold that timed out.
fn templates_landed(
    mut arrived: MessageReader<TemplatesArrived>,
    window: Res<LootWindow>,
    mut landed: MessageWriter<LootNamesLanded>,
) {
    if arrived.read().count() > 0 && window.get().is_some() {
        landed.write(LootNamesLanded);
    }
}

/// A row the player pressed, or the coins.
///
/// A message rather than a direct send from `lua::loot`, as every other
/// interface write in this client is: a registered Lua closure has no
/// `&mut World` and cannot reach the socket. See [`crate::lua::api::verbs`].
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeLoot {
    /// A screen row, one-based, as the interface counts. Converted to the
    /// server's index here and nowhere else.
    Row(usize),
    /// `CloseLoot()`: a release, which the server confirms.
    Close,
}

/// Fold what the server said into the window, and tell the interface.
fn announce(
    mut answers: MessageReader<LootAnswer>,
    mut window: ResMut<LootWindow>,
    session: Res<Session>,
    time: Res<Time>,
    mut say: super::messages::Announce,
    mut cleared: MessageWriter<LootSlotCleared>,
    mut closed: MessageWriter<LootClosed>,
    // The release for an emptied body. Written here and read by [`take`] in
    // the same frame (the four systems are chained), so the close is sent from
    // the one system that writes to the socket.
    mut release: MessageWriter<TakeLoot>,
) {
    for answer in answers.read() {
        match answer {
            LootAnswer::Opened(loot) => match loot.error {
                // A refusal uses the same opcode. The 1.12.1 client shows its
                // message in the error text and opens no window.
                //
                // Three of the thirteen keys are not in `GlobalStrings.lua`, so
                // three refusals show nothing. The 1.12.1 client behaves the
                // same, and `UiErrors::key` follows that rule.
                Some(error) => say.key(error.key()),
                None => {
                    // The names arrive one round trip later. Every entry is
                    // queued for a template query, as the vendor list does;
                    // `LiveSession::want_items` takes the world lock once and
                    // `want_item` skips an entry the session already knows. The
                    // icon needs no query: the row carries its display id.
                    if let Some(active) = session.active.as_ref() {
                        active.live.want_items(loot.items.iter().map(|item| item.entry));
                    }
                    window.open = Some(loot.clone());
                    // `LOOT_OPENED` is not raised yet. The event is the only
                    // thing that draws a row, so it waits until the rows can be
                    // named; see [`hold`] and the module note.
                    window.holding = Some(time.elapsed_secs());
                }
            },
            LootAnswer::Removed(index) => {
                // The row, not the index; see the module note. A removal for a
                // row this client never held is normal: a group member took an
                // item this player could not see.
                if let Some(row) = window.open.as_mut().and_then(|loot| loot.remove(*index)) {
                    cleared.write(LootSlotCleared { row });
                }
                close_if_empty(&window, &mut release);
            }
            LootAnswer::MoneyCleared => {
                if let Some(loot) = window.open.as_mut() {
                    // Row 1 is cleared and nothing is renumbered: `coin_row`
                    // records that the row existed, so the items below keep the
                    // numbers their buttons were given at `LOOT_OPENED`. See the
                    // stable-numbering rule in [`vale_protocol::play::loot`]'s
                    // module note.
                    loot.gold = 0;
                    cleared.write(LootSlotCleared { row: 1 });
                }
                // The coins are a row too, so a body whose last row was its
                // money closes by the same rule; see the module note.
                close_if_empty(&window, &mut release);
            }
            // Not a window event. In the 1.12.1 client the share is a chat line
            // (`LOOT_MONEY_SPLIT`), which is left to the chat path rather than
            // duplicated here. This arm exists so the packet is read rather
            // than counted as unhandled.
            LootAnswer::MoneyGained(_) => {}
            LootAnswer::Closed(guid) => {
                // Only if it names the open body. A release response for another
                // guid is a stale answer about a window that has already been
                // replaced, and acting on it would close the open one.
                if window.guid() == Some(*guid) {
                    window.open = None;
                    // A body released before its names arrived must not open;
                    // otherwise the hold would outlive the loot.
                    window.holding = None;
                    closed.write(LootClosed);
                }
            }
        }
    }
}

/// Request a release of the body if nothing is left on it.
///
/// A function called from [`announce`] rather than a system, because only the
/// two packets that can empty a window need the check; a system would run it
/// every frame. The rule is [`Loot::is_empty`], in the protocol crate, where a
/// test can run it with no window.
fn close_if_empty(window: &LootWindow, release: &mut MessageWriter<TakeLoot>) {
    if window.get().is_some_and(Loot::is_empty) {
        release.write(TakeLoot::Close);
    }
}

/// How long, in seconds, the window may wait for names before opening.
///
/// An upper bound, not the expected wait: against a local server the wait is
/// one session tick and one round trip, about 30 ms, and a lost query is sent
/// again on [`vale_protocol`]'s two-second interval. The bound covers an
/// answer that never arrives: a window with a blank row is better than a
/// corpse whose window never opens.
const HOLD_LIMIT: f32 = 3.0;

/// An item template arrived while a loot window was open.
///
/// Written by [`templates_landed`] on a [`TemplatesArrived`], and read in
/// [`crate::lua`] for the case [`hold`] cannot cover: a template that arrives after [`HOLD_LIMIT`] has let the window open with a
/// blank row. `LootFrame` has no event for that, so the reader calls the
/// frame's `LootFrame_Update` directly.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LootNamesLanded;

/// Raise `LOOT_OPENED` once every row can be named, or once [`HOLD_LIMIT`]
/// has passed.
///
/// Implements the module note's section on `LOOT_OPENED`. An entry is
/// settled when the session can name it or the server has refused to, and the
/// event is raised when every row is settled or [`HOLD_LIMIT`] has passed. The
/// coin row is settled at once: it is composed from `GlobalStrings` and needs
/// no packet.
fn hold(
    mut window: ResMut<LootWindow>,
    session: Res<Session>,
    time: Res<Time>,
    mut opened: MessageWriter<LootOpened>,
) {
    let Some(since) = window.holding else { return };
    let expired = time.elapsed_secs() - since >= HOLD_LIMIT;
    if !expired && !settled(&window, &session) {
        return;
    }
    window.holding = None;
    opened.write(LootOpened);
}

/// Can every item row be named yet?
///
/// A refusal counts as settled. vmangos answers an entry it will not describe
/// with `entry | 0x80000000`, which is stored in `unknown_items`, and a row the
/// server will never name is not worth waiting for.
fn settled(window: &LootWindow, session: &Session) -> bool {
    let Some(loot) = window.get() else {
        return true;
    };
    let Some(active) = session.active.as_ref() else {
        // With no session no answers will come, so the window is not held.
        return true;
    };
    let world = active.live.world();
    let world = world.lock().unwrap_or_else(|e| e.into_inner());
    loot.items
        .iter()
        .all(|item| world.items.contains_key(&item.entry) || world.unknown_items.contains(&item.entry))
}

/// Drain what the interface pressed into the message [`take`] reads.
///
/// A separate system from [`take`] rather than a `NonSendMut` on it, because a
/// non-send parameter pins its system to the main thread, and because a press
/// has two sources: the loot window's buttons and an addon calling
/// `LootSlot(2)`. Both arrive on the host's queue and are handled the same way.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<TakeLoot>,
) {
    let Some(host) = host else { return };
    for press in crate::lua::panels::loot::take(&host) {
        out.write(press);
    }
}

/// Send what the player pressed.
fn take(mut presses: MessageReader<TakeLoot>, session: Res<Session>, window: Res<LootWindow>) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for press in presses.read() {
        match press {
            TakeLoot::Row(row) => {
                let Some(loot) = window.get() else { continue };
                // The coins and an item are taken with different packets, and
                // the row decides which. [`Loot::is_money`] uses the
                // interface's own arithmetic.
                if loot.is_money(*row) {
                    active.live.loot_money();
                } else if let Some(item) = loot.at(*row) {
                    // A row the server marked not clickable is not sent. The
                    // 1.12.1 client draws it and refuses the click; the server
                    // answers such a packet with an error.
                    if item.slot_type.is_clickable() {
                        active.live.loot_item(item.index);
                    }
                }
            }
            TakeLoot::Close => {
                if let Some(guid) = window.guid() {
                    active.live.loot_release(guid);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::play::loot::{LootError, LootItem, LootType, SlotType};

    fn item(index: u8, entry: u32) -> LootItem {
        LootItem {
            index,
            entry,
            count: 1,
            display_id: 0,
            random_suffix: 0,
            random_property: 0,
            slot_type: SlotType::AllowLoot,
            taken: false,
        }
    }

    fn window(gold: u32, items: Vec<LootItem>) -> Loot {
        Loot {
            guid: 7,
            kind: Some(LootType::Corpse),
            error: None,
            gold,
            coin_row: gold > 0,
            items,
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<LootAnswer>()
            .add_message::<LootOpened>()
            .add_message::<LootSlotCleared>()
            .add_message::<LootClosed>()
            .init_resource::<LootWindow>()
            .init_resource::<Session>()
            // `announce` stamps the hold with the clock and `hold` reads it; see
            // [`hold`]. There is no session in these tests, so every row is
            // settled at once and the event is raised on the frame the answer
            // arrives.
            .init_resource::<Time>()
            .init_resource::<crate::interface::messages::UiStrings>()
            // The release for an emptied body is written by `announce` and read
            // by `take`, which no test here runs; the assertions read the queue
            // instead.
            .add_message::<TakeLoot>()
            .add_systems(Update, (announce, hold).chain());
        crate::interface::messages::Announce::register(&mut app);
        app
    }

    /// What [`announce`] asked the socket for this frame.
    fn asked(app: &mut App) -> Vec<TakeLoot> {
        let messages = app.world().resource::<Messages<TakeLoot>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).copied().collect()
    }

    fn opened(app: &mut App) -> usize {
        let messages = app.world().resource::<Messages<LootOpened>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).count()
    }

    fn cleared_rows(app: &mut App) -> Vec<usize> {
        let messages = app.world().resource::<Messages<LootSlotCleared>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|m| m.row).collect()
    }

    /// A window opens, a row is taken, and the interface is given the row
    /// rather than the server's index. The count does not change, because the
    /// buttons were numbered once at `LOOT_OPENED`.
    #[test]
    fn a_removal_reaches_the_interface_as_the_row_it_holds() {
        let mut app = app();
        app.world_mut()
            .write_message(LootAnswer::Opened(window(0, vec![item(0, 2589), item(5, 858)])));
        app.update();
        assert_eq!(app.world().resource::<LootWindow>().rows(), 2);

        app.world_mut().write_message(LootAnswer::Removed(5));
        app.update();
        assert_eq!(cleared_rows(&mut app), vec![2], "row 2, not index 5");
        let window = app.world().resource::<LootWindow>();
        assert_eq!(window.rows(), 2, "the numbering is fixed for the window's life");
        // The untaken row still answers under its opening number, which the
        // second click sends. The reported "cannot loot the second item" was
        // this store renumbering the rows under the buttons.
        assert_eq!(window.get().and_then(|l| l.at(1)).map(|i| i.index), Some(0));
        assert_eq!(window.get().and_then(|l| l.at(2)), None);
    }

    /// A refusal opens no window. It uses the same opcode as an answer, and
    /// treating it as one would show an empty loot frame for a corpse the
    /// player may not loot.
    #[test]
    fn a_refusal_opens_nothing() {
        let mut app = app();
        app.world_mut().write_message(LootAnswer::Opened(Loot {
            guid: 7,
            kind: None,
            error: Some(LootError::TooFar),
            gold: 0,
            coin_row: false,
            items: Vec::new(),
        }));
        app.update();
        assert!(app.world().resource::<LootWindow>().get().is_none());
        assert_eq!(app.world().resource::<LootWindow>().rows(), 0);
    }

    /// The window closes when the server says so, and only for the open body.
    /// A stale release response naming a corpse that has already been replaced
    /// must not close the open one.
    #[test]
    fn only_a_release_for_this_body_closes_the_window() {
        let mut app = app();
        app.world_mut()
            .write_message(LootAnswer::Opened(window(0, vec![item(0, 2589)])));
        app.update();
        app.world_mut().write_message(LootAnswer::Closed(999));
        app.update();
        assert!(
            app.world().resource::<LootWindow>().get().is_some(),
            "somebody else's release closed this window"
        );
        app.world_mut().write_message(LootAnswer::Closed(7));
        app.update();
        assert!(app.world().resource::<LootWindow>().get().is_none());
    }

    /// The event that draws the rows waits for the rows to have names.
    ///
    /// With no session there is nothing to wait for, so this tests the other
    /// branch of [`hold`]: the hold is released and `LOOT_OPENED` is raised
    /// exactly once, on the frame the answer arrived. It guards against the
    /// window opening twice, which would play `LootFrame_OnShow`'s sound twice.
    #[test]
    fn the_window_opens_once_and_only_when_its_rows_can_be_named() {
        let mut app = app();
        app.world_mut()
            .write_message(LootAnswer::Opened(window(0, vec![item(0, 2589)])));
        app.update();
        assert_eq!(opened(&mut app), 1);
        assert!(!app.world().resource::<LootWindow>().is_holding());
        app.update();
        assert_eq!(opened(&mut app), 1, "the hold is not re-released every frame");
    }

    /// A body released while the window is still held never opens.
    ///
    /// Otherwise the hold would outlive the loot: the release arrives, `open`
    /// is cleared, and a stale `holding` raises `LOOT_OPENED` for an empty
    /// window.
    #[test]
    fn a_release_during_the_hold_cancels_it() {
        let mut app = app();
        // Hold it by hand, which is what a session with an unnamed row does.
        app.world_mut()
            .write_message(LootAnswer::Opened(window(0, vec![item(0, 2589)])));
        app.update();
        app.world_mut().resource_mut::<LootWindow>().holding = Some(0.0);
        app.world_mut().write_message(LootAnswer::Closed(7));
        app.update();
        let window = app.world().resource::<LootWindow>();
        assert!(window.get().is_none());
        assert!(!window.is_holding(), "the hold went with the body");
    }

    /// The last row taken closes the window; neither the server nor the
    /// interface does.
    ///
    /// Measured against the running server: an emptied body produces
    /// `SMSG_LOOT_REMOVED` and then nothing. See the module note and
    /// [`Loot::is_empty`].
    #[test]
    fn emptying_the_body_asks_for_it_to_be_released() {
        let mut app = app();
        app.world_mut()
            .write_message(LootAnswer::Opened(window(0, vec![item(0, 2589), item(5, 858)])));
        app.update();
        assert!(asked(&mut app).is_empty(), "a full body is not released");

        app.world_mut().write_message(LootAnswer::Removed(0));
        app.update();
        assert!(asked(&mut app).is_empty(), "one row is still on it");

        app.world_mut().write_message(LootAnswer::Removed(5));
        app.update();
        assert_eq!(asked(&mut app), vec![TakeLoot::Close]);
        // The window stays open until the server closes it: the release is a
        // request, and `SMSG_LOOT_RELEASE_RESPONSE` closes the window. See the
        // module note.
        assert!(app.world().resource::<LootWindow>().get().is_some());
    }

    /// The coins alone keep the window open, and taking them closes it when
    /// they are the last row.
    #[test]
    fn the_coin_row_is_a_row_for_the_purpose_of_closing() {
        let mut app = app();
        app.world_mut()
            .write_message(LootAnswer::Opened(window(137, vec![item(0, 2589)])));
        app.update();

        app.world_mut().write_message(LootAnswer::Removed(0));
        app.update();
        assert!(asked(&mut app).is_empty(), "the money is still on the body");

        app.world_mut().write_message(LootAnswer::MoneyCleared);
        app.update();
        assert_eq!(asked(&mut app), vec![TakeLoot::Close]);
    }

    /// A body that opens empty is not closed on opening.
    ///
    /// `LootFrame_OnShow` plays `LOOTWINDOWOPENEMPTY` when `GetNumLootItems()`
    /// is zero, so the game has a sound for this case, and closing the window
    /// would differ from the 1.12.1 client. The check runs on a removal and
    /// never on the response.
    #[test]
    fn an_empty_body_still_opens_its_window() {
        let mut app = app();
        app.world_mut().write_message(LootAnswer::Opened(window(0, Vec::new())));
        app.update();
        assert!(asked(&mut app).is_empty());
        assert!(app.world().resource::<LootWindow>().get().is_some());
        assert_eq!(opened(&mut app), 1, "and it is drawn, empty");
    }

    /// Taking the coins clears row 1 and leaves the items where they are: the
    /// coin row stays as an empty row rather than being removed, so the item
    /// buttons keep the numbers they were given at `LOOT_OPENED`.
    #[test]
    fn clearing_the_money_clears_row_one_and_renumbers_nothing() {
        let mut app = app();
        app.world_mut()
            .write_message(LootAnswer::Opened(window(137, vec![item(0, 2589)])));
        app.update();
        assert_eq!(app.world().resource::<LootWindow>().rows(), 2);
        app.world_mut().write_message(LootAnswer::MoneyCleared);
        app.update();
        assert_eq!(cleared_rows(&mut app), vec![1]);
        let window = app.world().resource::<LootWindow>();
        assert_eq!(window.rows(), 2, "the count is fixed for the window's life");
        assert!(window.get().is_some_and(|l| !l.is_money(1)), "row 1 is spent");
        assert_eq!(window.get().and_then(|l| l.at(1)), None);
        // The item is still row 2, the number its button holds.
        assert_eq!(window.get().and_then(|l| l.at(2)).map(|i| i.entry), Some(2589));
    }
}
