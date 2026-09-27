//! **What is on the body, while the window is open.**
//!
//! The client's half of [`vale_protocol::play::loot`]: one window at a time, held
//! for as long as the server says it is open, and the three events
//! `LootFrame.lua` is written against.
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
//! ## Three things about it are not obvious and each is a visible fault
//!
//! **`LOOT_SLOT_CLEARED` carries the *row*, not the server's index.**
//! `LootFrame_OnEvent` does `local slot = arg1 - ((this.page - 1) *
//! numLootToShow)` and hides `LootButton<slot>`; hand it the server's sparse
//! index and it hides the wrong button, or none. The crossing happens once, in
//! [`vale_protocol::play::loot::Loot::remove`].
//!
//! **The window closes on the server's word.** `HandleLootReleaseOpcode`
//! discards the guid the client sends, so the only statement that the body is
//! shut is `SMSG_LOOT_RELEASE_RESPONSE` coming back. Clearing on the send leaves
//! the server holding a body this client thinks it has let go of, and the next
//! right-click on it does nothing at all.
//!
//! **A refusal opens no window and says so.** `SMSG_LOOT_RESPONSE` is the same
//! opcode either way; a refusal reaches [`crate::interface::messages`] as the error
//! line its own `ERR_LOOT_*` key names, which is what the reference puts in the
//! red text at the top of the screen.
//!
//! ## `LOOT_OPENED` is held until the rows have names, and it has to be
//!
//! **`LootFrame` is the one panel in the whole directory with no refresh event.**
//! `MerchantFrame` re-reads its rows on `MERCHANT_UPDATE`, `QuestFrame` on
//! `QUEST_ITEM_UPDATE`, `ContainerFrame` on `BAG_UPDATE` — each of those is an
//! arm that calls the panel's own `*_Update` outright. `LootFrame_OnEvent`'s
//! `LOOT_OPENED` arm is `ShowUIPanel(this)` and nothing else, and
//! `UIParent.lua`'s `ShowUIPanel` **returns immediately for a frame that is
//! already visible**. `LootFrame_Update` is reachable from exactly three places in
//! the shipped file: `LootFrame_OnShow` and the two page buttons.
//!
//! So re-raising `LOOT_OPENED` at an open loot window does *nothing*, and a row
//! whose template landed a moment after the window opened stays blank for as
//! long as the body is held. That was the bug: an item this character had never
//! met drew its icon (`SMSG_LOOT_RESPONSE` carries a display id) with no name
//! beside it, for ever.
//!
//! Hiding and re-showing is not the fix either — `LootFrame_OnHide` calls
//! `CloseLoot()`, so it would release the body.
//!
//! **The reading, and it is a reading rather than a measurement**: those two
//! facts together — no refresh path in the Lua, and a reference client that does
//! not draw blank loot rows — mean the reference must already have the names by
//! the time it signals the event. The item cache's completion callback is
//! the machinery it would do that with (see
//! [`crate::world::templates`]). What is reproduced here is the
//! *behaviour* — the window opens named — and not a verified route to it.
//!
//! [`hold`] is that: the answer is folded in at once, and the event waits for
//! every row to be named or refused, bounded by [`HOLD_LIMIT`] so that a lost
//! query cannot cost the player the window altogether. On a local server it is
//! one tick and one round trip, about 30 ms.
//!
//! ## The last row taken closes the window, and nobody else was going to
//!
//! **Measured against the running server**: empty a corpse with `vale live`
//! — `loot 1`, `take 1` on a one-row body — and back comes
//! `SMSG_LOOT_REMOVED` and then, over 21 seconds of listening, nothing at all.
//! vmangos holds the body open for as long as the client holds it.
//!
//! The shipped interface does not close it either: `LootFrame_OnEvent`'s
//! `LOOT_SLOT_CLEARED` arm hides the button and then pages down *only if* every
//! button is hidden **and there is a next page**. With no next page it returns,
//! and the window sits there blank until the player presses the X.
//!
//! So the close is the client's own — a reading of those two measurements.
//! [`announce`] sends [`TakeLoot::Close`] the moment
//! [`vale_protocol::play::loot::Loot::is_empty`] turns true on a removal, and the
//! window then shuts the way every other close does: on
//! `SMSG_LOOT_RELEASE_RESPONSE`.
//!
//! **On a removal and never on the response.** A body can arrive with nothing
//! on it and the game has a sound for that — `LootFrame_OnShow` plays
//! `LOOTWINDOWOPENEMPTY` when `GetNumLootItems()` is zero — so a window that
//! closed itself on sight would be a departure rather than a convenience.
//!
//! ## …and the money is two packets that are easy to fold into one
//!
//! `SMSG_LOOT_CLEAR_MONEY` empties the coin row and `SMSG_LOOT_MONEY_NOTIFY`
//! says what *your share* was — which in a group is a fraction. They are
//! answered separately here for the reason the protocol module gives: the notify
//! goes to everyone in range and the clear only to the people with the window
//! open, so treating one as the other leaves a coin row nobody can click.

use bevy::prelude::*;

use vale_protocol::play::loot::Loot;

use super::events::{LootClosed, LootOpened, LootSlotCleared};
use crate::world::session::Session;

/// One of the five packets, handed on from [`super::action::drain_events`].
///
/// A message rather than a second drain of the session queue, on the same terms
/// [`super::timers::MirrorTimerAnswer`] is: the queue has exactly one reader and
/// a second would steal half of it.
#[derive(Message, Debug, Clone)]
pub enum LootAnswer {
    /// `SMSG_LOOT_RESPONSE` — the window, or the refusal wearing its opcode.
    Opened(Loot),
    /// `SMSG_LOOT_REMOVED`, by the **server's** index.
    Removed(u8),
    /// `SMSG_LOOT_CLEAR_MONEY`.
    MoneyCleared,
    /// `SMSG_LOOT_MONEY_NOTIFY` — your share, in copper.
    MoneyGained(u32),
    /// `SMSG_LOOT_RELEASE_RESPONSE` — the body is shut.
    Closed(u64),
}

/// **The open loot window, or nothing.**
///
/// One at a time, which is the server's own model: `Player::GetLootGuid` is a
/// single guid and opening a second body releases the first.
#[derive(Resource, Default)]
pub struct LootWindow {
    open: Option<Loot>,
    /// **The answer is in and `LOOT_OPENED` has not gone out yet**, as the
    /// moment it landed. See [`hold`] and the module note: the event is what
    /// draws the rows, and it may only be raised once they can be named.
    holding: Option<f32>,
}

impl LootWindow {
    /// What is on the body, or `None` while no window is up.
    pub fn get(&self) -> Option<&Loot> {
        self.open.as_ref()
    }

    /// Whether the window has an answer it has not been allowed to draw yet —
    /// see [`hold`].
    pub fn is_holding(&self) -> bool {
        self.holding.is_some()
    }

    /// Whose body it is — what a release has to name.
    pub fn guid(&self) -> Option<u64> {
        Some(self.open.as_ref()?.guid)
    }

    /// `GetNumLootItems()` — rows, coins included. **0 while nothing is open**,
    /// which is what `LootFrame_OnShow` reads to decide it is an empty body.
    pub fn rows(&self) -> usize {
        self.open.as_ref().map_or(0, Loot::rows)
    }

    /// **Free a row a group roll was holding** — the roll's statement about this
    /// window, and the one rule in the loot family with no packet behind it.
    ///
    /// Only if it is the body the roll named: a result can arrive about a corpse
    /// this client never opened, and about one it opened, released and replaced.
    /// See [`vale_protocol::play::loot::Loot::unblock`], where the rule and
    /// its address are, and [`super::lootroll`], which is the only caller.
    pub(super) fn unblock_row(&mut self, guid: u64, index: u8) {
        let Some(loot) = self.open.as_mut().filter(|loot| loot.guid == guid) else {
            return;
        };
        loot.unblock(index);
    }

    /// Whether the window is a fishing catch — `IsFishingLoot()`.
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
            .add_systems(
                Update,
                (announce, hold, presses, take)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// **A row the player pressed**, or the coins.
///
/// A message rather than a direct send from `lua::loot`, for the reason every
/// other interface write in this client is one: a registered Lua closure has no
/// `&mut World` and cannot reach the socket. See [`crate::lua::api::verbs`].
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeLoot {
    /// A **screen row**, one-based, exactly as the interface counts. Crossed
    /// into the server's index here and nowhere else.
    Row(usize),
    /// `CloseLoot()` — and it is a *release*, which the server confirms.
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
    // **The release the emptied body earns.** Written here and read by [`take`]
    // in the same frame — the four systems are `.chain()`ed — so the close goes
    // through the one place that talks to the socket rather than reaching for
    // it from a second.
    mut release: MessageWriter<TakeLoot>,
) {
    for answer in answers.read() {
        match answer {
            LootAnswer::Opened(loot) => match loot.error {
                // **A refusal is the same opcode**, and what the reference does
                // with it is put the sentence in the error text. No window.
                //
                // Three of the thirteen keys are not in `GlobalStrings.lua`, so
                // three refusals say nothing at all — which is the client's own
                // behaviour and the rule `UiErrors::key` is written around.
                Some(error) => say.key(error.key()),
                None => {
                    // **The names are a round trip behind** — queue every entry
                    // the item cache cannot name yet, exactly as the vendor
                    // list does. The icon needs no trip at all: the row carries
                    // its own display id.
                    if let Some(active) = session.active.as_ref() {
                        let world = active.live.world();
                        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
                        for item in &loot.items {
                            if !world.items.contains_key(&item.entry) {
                                world.want_item(item.entry);
                            }
                        }
                    }
                    window.open = Some(loot.clone());
                    // **Not `LOOT_OPENED` yet.** The event is the only thing
                    // that ever draws a row, so it waits until the rows can be
                    // named — see [`hold`] and the module note.
                    window.holding = Some(time.elapsed_secs());
                }
            },
            LootAnswer::Removed(index) => {
                // **The row, not the index** — see the module note. A removal
                // for a row this viewer never held is an ordinary arrival: a
                // group member took something we could not see.
                if let Some(row) = window.open.as_mut().and_then(|loot| loot.remove(*index)) {
                    cleared.write(LootSlotCleared { row });
                }
                close_if_empty(&window, &mut release);
            }
            LootAnswer::MoneyCleared => {
                if let Some(loot) = window.open.as_mut() {
                    // Row 1 goes dark and **nothing renumbers**: `coin_row`
                    // remembers the row existed, so the items below keep the
                    // numbers their buttons were given at `LOOT_OPENED` — the
                    // stable-numbering rule in [`vale_protocol::play::loot`]'s
                    // module note.
                    loot.gold = 0;
                    cleared.write(LootSlotCleared { row: 1 });
                }
                // **The coins are a row too**, so a body whose last thing was
                // its money closes on the same rule — see the module note.
                close_if_empty(&window, &mut release);
            }
            // **Not a window event at all.** The share is a chat line
            // (`LOOT_MONEY_SPLIT`) in the reference and is deliberately left to
            // the chat path rather than duplicated here; what this arm exists
            // for is that the packet is read rather than counted as unhandled.
            LootAnswer::MoneyGained(_) => {}
            LootAnswer::Closed(guid) => {
                // **Only if it is the body we think we have.** A release
                // response for another guid is a stale answer to a window that
                // has already been replaced, and acting on it would shut the one
                // that is up.
                if window.guid() == Some(*guid) {
                    window.open = None;
                    // A body released before its names arrived is a window that
                    // must never open — otherwise the hold outlives the loot.
                    window.holding = None;
                    closed.write(LootClosed);
                }
            }
        }
    }
}

/// Ask for the body to be let go if there is nothing left on it.
///
/// A free function beside [`announce`] rather than a system of its own, because
/// the question is only worth asking on the two packets that can empty a window
/// — a system would ask it every frame of every session to answer "no". The
/// rule itself is [`Loot::is_empty`], in the protocol crate where a test can run
/// it with no window.
fn close_if_empty(window: &LootWindow, release: &mut MessageWriter<TakeLoot>) {
    if window.get().is_some_and(Loot::is_empty) {
        release.write(TakeLoot::Close);
    }
}

/// **How long the window may be held back waiting for names.**
///
/// A last resort rather than a budget: against a server on this machine the
/// wait is one session tick and one round trip, about 30 ms, and the query is
/// re-asked on [`vale_protocol`]'s own two-second beat if the first one is
/// lost. What this bounds is the case where the answer never comes at all —
/// a window that opens with a blank row is bad, and a corpse that never opens a
/// window is worse.
const HOLD_LIMIT: f32 = 3.0;

/// **An item template landed while a loot window was up.**
///
/// Written by [`crate::world::templates`], which is the one drain of
/// the arrival queue, and read in [`crate::lua`] — where the interface is — for
/// the case [`hold`] cannot cover: a template that arrives after
/// [`HOLD_LIMIT`] has already let the window open blank. `LootFrame` has no
/// event for that, so the refresh is the frame's own `LootFrame_Update`, called
/// where it stands.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LootNamesLanded;

/// **Let the window open once its rows can be named**, or once waiting has
/// stopped being worth it.
///
/// The whole of the module note's third section, and it is four lines of rule:
/// an entry is *settled* when the world can name it or the server has refused
/// to, and the event goes out when every row is settled or [`HOLD_LIMIT`] has
/// passed. The coin row settles immediately — it is composed out of
/// `GlobalStrings` and needs no packet.
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
/// **A refusal counts.** vmangos answers an entry it will not describe with
/// `entry | 0x80000000`, which lands in `unknown_items` — and a row the server
/// will never name is a row there is no point waiting for.
fn settled(window: &LootWindow, session: &Session) -> bool {
    let Some(loot) = window.get() else {
        return true;
    };
    let Some(active) = session.active.as_ref() else {
        // No session is no answers coming; do not hold the window hostage to it.
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
/// non-send parameter pins its system to the main thread — and because the
/// press has two sources: the loot window's own buttons and an addon calling
/// `LootSlot(2)`. Both arrive on the host's queue and neither is special.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<TakeLoot>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_loot_presses() {
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
                // **The coins and an item are different packets**, and the row
                // is what decides which — see [`Loot::is_money`], which is the
                // interface's own arithmetic.
                if loot.is_money(*row) {
                    active.live.loot_money();
                } else if let Some(item) = loot.at(*row) {
                    // A row the server said is not clickable is not sent. The
                    // reference draws it and refuses the click; sending it
                    // anyway is a packet the server answers with an error.
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
            // `announce` stamps the hold with the clock and `hold` reads it back
            // — see [`hold`]. There is no session in these tests, so every row
            // settles at once and the event goes out on the same frame the
            // answer did.
            .init_resource::<Time>()
            .init_resource::<crate::interface::messages::UiStrings>()
            // The release an emptied body earns is written by `announce` and
            // read by `take`, which no test here runs — reading the queue is
            // what the assertions do instead.
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

    /// The whole loop: a window opens, a row is taken, and the interface is told
    /// the **row** rather than the server's index — and the count does not
    /// move, because the buttons were numbered once at `LOOT_OPENED`.
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
        // …and the untaken row still answers under its opening number, which is
        // what the second click sends: the reported "cannot loot the second
        // item" was this store renumbering under the buttons.
        assert_eq!(window.get().and_then(|l| l.at(1)).map(|i| i.index), Some(0));
        assert_eq!(window.get().and_then(|l| l.at(2)), None);
    }

    /// **A refusal opens no window.** It is the same opcode as an answer, and
    /// treating it as one gives an empty loot frame over a corpse the player was
    /// not allowed to touch.
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

    /// **The window shuts on the server's word and only for the right body.** A
    /// stale release response naming a corpse that has already been replaced
    /// must not close the one that is up.
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

    /// **The event that draws the rows waits for the rows to have names.**
    ///
    /// With no session there is nothing to wait on, so this pins the other half
    /// of [`hold`]: the hold is released and `LOOT_OPENED` goes out exactly
    /// once, on the frame the answer landed. What it guards against is the
    /// window opening *twice* — `LootFrame_OnShow` plays a sound.
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

    /// **A body released while the window was still held shut never opens.**
    ///
    /// Otherwise the hold outlives the loot: the release lands, `open` is
    /// cleared, and a stale `holding` raises `LOOT_OPENED` at an empty window.
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

    /// **The last row taken closes the window**, because nothing else will.
    ///
    /// Measured against the running server first: an emptied body produces
    /// `SMSG_LOOT_REMOVED` and then nothing, for as long as anyone listens. See
    /// the module note and [`Loot::is_empty`].
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
        // **And the window is still open until the server says otherwise** —
        // the release is a request, and `SMSG_LOOT_RELEASE_RESPONSE` is what
        // shuts it. See the module note's third paragraph.
        assert!(app.world().resource::<LootWindow>().get().is_some());
    }

    /// **The coins hold the window open on their own**, and close it on their
    /// own when they are the last thing left.
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

    /// **A body that opens with nothing on it is not closed on sight.**
    ///
    /// `LootFrame_OnShow` plays `LOOTWINDOWOPENEMPTY` when `GetNumLootItems()`
    /// is zero, so the game has a sound for exactly this and closing it would
    /// be a departure. The check is on a removal and never on the response.
    #[test]
    fn an_empty_body_still_opens_its_window() {
        let mut app = app();
        app.world_mut().write_message(LootAnswer::Opened(window(0, Vec::new())));
        app.update();
        assert!(asked(&mut app).is_empty());
        assert!(app.world().resource::<LootWindow>().get().is_some());
        assert_eq!(opened(&mut app), 1, "and it is drawn, empty");
    }

    /// Taking the coins clears row 1 and leaves the items **where they are** —
    /// the coin row keeps its place dark rather than vanishing, so the item
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
        // …and the item is still row 2, which is the number its button holds.
        assert_eq!(window.get().and_then(|l| l.at(2)).map(|i| i.entry), Some(2589));
    }
}
