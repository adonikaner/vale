//! **The social panel's half of the session** — four packets in, six out, and
//! four events.
//!
//! The panel's *state* is not here: it lives on the Lua host, because
//! `FriendsFrameFriendButton_OnClick` re-reads the whole list inside the handler
//! that changed the selection. See [`crate::lua::panels::social`], which says so
//! at more length, and [`vale_protocol::play::social`], which owns the wire.
//!
//! What is here is the four things that need a Bevy system:
//!
//! ```text
//! supply_tables   ChrClasses/ChrRaces, AreaTable and GlobalStrings, per interface
//! apply_answers   the four SMSGs, folded into the board
//! learn_names     …and the names, which arrive by an entirely different road
//! announce        the four events, off the board's own three counters
//! send_verbs      the six CMSGs the panel owes
//! ```
//!
//! ## The names are the hard half and they are not in any of the four packets
//!
//! `SMSG_FRIEND_LIST` and `SMSG_IGNORE_LIST` are guids. The panel draws names.
//! So the list arriving is only the *first* of two things that have to happen,
//! and the second is a `CMSG_NAME_QUERY` round trip per guid — which is the
//! session thread's, through `ObjectManager::note_social_friends` and the query
//! pass. [`learn_names`] copies the answers across and bumps the board's
//! counter, so the row that drew as `UNKNOWN` redraws with a name a tick later
//! and nothing else has to know.
//!
//! That is why [`FriendListUpdate`] is raised off a counter rather than off the
//! packet: the list changes twice for one packet, and only the second change is
//! the one a player sees.
//!
//! ## `SMSG_FRIEND_STATUS` is both a change and a refusal
//!
//! One packet, twenty results, and they fall into three groups: the ones that
//! move a list, the ones that only say *no*, and the ones this client has no
//! name for at all. All three print a sentence — [`FriendsResult::message`]
//! names the `GlobalStrings.lua` key — and only the first group touches the
//! board. Getting that split wrong is a friends list that grows a phantom row
//! every time somebody types a name the server has never heard of.
//!
//! ## Where a who answer goes is decided *before* the search
//!
//! `SetWhoToUI(1)` is called by `ShowWhoPanel` and `SetWhoToUI(nil)` by the
//! parts of the interface that want the answer printed into the chat frame
//! instead. The flag is held on the board rather than queued with the search,
//! because the interface sets it on the frame the panel opens and the answer
//! arrives whole round trips later.

use bevy::prelude::*;

use super::events::{
    ChatMessageReceived, FriendListShow, FriendListUpdate, IgnoreListUpdate, WhoListUpdate,
};
use crate::world::session::Session;
use vale_protocol::play::social::FriendsResult;
use vale_protocol::play::spells::PlayerEvent;

/// What the session thread said about the two lists and the search, forwarded by
/// [`crate::world::incoming::drain_events`] on the same terms as every other
/// subject's.
#[derive(Message, Debug, Clone)]
pub enum SocialAnswer {
    /// `SMSG_FRIEND_LIST` — the whole list.
    Friends(Vec<vale_protocol::play::social::Friend>),
    /// `SMSG_IGNORE_LIST` — the whole list, as guids.
    Ignores(Vec<u64>),
    /// `SMSG_FRIEND_STATUS` — one answer about one player, which may be a
    /// refusal.
    Status(vale_protocol::play::social::FriendStatus),
    /// `SMSG_WHO` — the rows, and the online total behind them.
    Who(vale_protocol::play::social::WhoResults),
}

pub struct SocialPlugin;

impl Plugin for SocialPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SocialAnswer>().add_systems(
            Update,
            // **Ordered, and the order is a packet's own life.** The tables have
            // to be in place before an answer is folded in (a friend's class and
            // zone are resolved on read, but the who parse is not), the answers
            // before the names are copied onto them, and both before the
            // counters are read — otherwise the event that redraws the panel
            // fires a frame before the row it is about.
            (
                supply_tables,
                apply_answers,
                learn_names,
                announce,
                send_verbs,
            )
                .chain()
                .in_set(super::GameSet),
        );
    }
}

/// **Hand the board its three tables**, once per interface.
///
/// Per interface rather than once at startup for [`crate::glue::charcreate`]'s reason:
/// the host — board included — is replaced whenever the directory swaps, so a
/// character who logs out and back in would otherwise find a friends list with
/// no class or zone names in it. All three banks are cached, so the second
/// supply is three pointer copies.
fn supply_tables(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
) {
    let Some(host) = host else { return };
    if host.social().borrow().strings.is_some() {
        return;
    }
    let mut board = host.social().borrow_mut();
    board.strings = Some(assets.strings());
    // **`None` rather than an error** for either table: without them a friend's
    // class and zone draw as `UNKNOWN`, which is the reference's own answer when
    // its own lookup misses. See `crate::lua::panels::social`.
    if let Ok(tables) = assets.display_tables() {
        board.areas = tables.areas().cloned().map(std::sync::Arc::new);
    }
    board.classes = Some(assets.char_create());
}

/// Fold the server's four answers into the board.
fn apply_answers(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut answers: MessageReader<SocialAnswer>,
    mut show: MessageWriter<FriendListShow>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    let Some(host) = host else {
        // **Drained anyway**, so a login that arrives before the interface is up
        // does not replay the whole list on the frame the host appears.
        answers.clear();
        return;
    };
    let mut board = host.social().borrow_mut();
    for answer in answers.read() {
        match answer {
            SocialAnswer::Friends(list) => {
                board.set_friends(list);
                // **`FRIENDLIST_SHOW` as well as the update**, because this
                // packet is also the answer to `ShowFriends()` — `/friends` with
                // no name — and `FriendsFrame_OnEvent`'s first arm is what
                // selects the tab. A client that raises only the update leaves
                // `/friends` doing nothing visible.
                show.write(FriendListShow);
            }
            SocialAnswer::Ignores(list) => board.set_ignores(list),
            SocialAnswer::Status(status) => {
                if let Some(line) = sentence(&board, status) {
                    super::chat::system_note(&mut chat, line);
                }
                apply_status(&mut board, status);
            }
            SocialAnswer::Who(results) => {
                // **The rows go on the board whichever way the flag points**, so
                // that a `/who` printed into the chat frame is still readable by
                // the panel when it is next opened — which is what the reference
                // does: `SetWhoToUI` decides where the *announcement* goes, not
                // where the answer is kept.
                board.set_who(results.rows.clone(), results.online);
            }
        }
    }
}

/// **Patch the board for one `SMSG_FRIEND_STATUS`** — the three groups in the
/// module note.
fn apply_status(
    board: &mut crate::lua::panels::social::Social,
    status: &vale_protocol::play::social::FriendStatus,
) {
    match status.result {
        // An online add or a friend logging in carries the whole row — level,
        // zone and class — so it replaces rather than merely marking presence.
        FriendsResult::AddedOnline | FriendsResult::Online => match status.friend {
            Some(friend) => board.note_friend(friend),
            None => board.add_friend_guid(status.guid),
        },
        // An offline add carries nothing but the guid, and neither does a friend
        // going offline — which still has to clear the row's own online half, or
        // the panel goes on listing the zone they were last seen in.
        FriendsResult::AddedOffline => board.add_friend_guid(status.guid),
        FriendsResult::Offline => {
            board.note_friend(vale_protocol::play::social::Friend {
                guid: status.guid,
                ..Default::default()
            });
        }
        FriendsResult::Removed => board.remove_friend_guid(status.guid),
        FriendsResult::IgnoreAdded => board.add_ignore_guid(status.guid),
        FriendsResult::IgnoreRemoved => board.remove_ignore_guid(status.guid),
        // Every remaining result is a refusal — "already your friend", "player
        // not found", "your list is full" — and touches neither list. The
        // sentence has already been said.
        _ => {}
    }
}

/// The chat line one result prints, in the game's own words.
///
/// `None` for a result [`FriendsResult::message`] has no key for, which is the
/// mute family and anything a later build added: the reference says nothing for
/// those either.
fn sentence(
    board: &crate::lua::panels::social::Social,
    status: &vale_protocol::play::social::FriendStatus,
) -> Option<String> {
    let key = status.result.message()?;
    let text = board.strings.as_ref()?.get(key)?.to_string();
    // **The name is whatever this client already knows**, which for a refusal
    // about somebody who does not exist is nothing at all — so the substitution
    // is skipped and the bare sentence is said. That is better than printing the
    // format string: `ERR_FRIEND_NOT_FOUND` has no `%s` in it anyway, and the
    // keys that do are the ones about a player already on a list.
    let slots = FriendsResult::name_slots(key);
    if slots == 0 {
        return Some(text);
    }
    let name = board.name_of(status.guid)?;
    let mut out = text;
    for _ in 0..slots {
        out = out.replacen("%s", &name, 1);
    }
    Some(out)
}

/// **Copy in whatever `CMSG_NAME_QUERY` has answered** — see the module note.
///
/// **Locks the world only while something is missing**, which after the first
/// second of a session is never: the guard is a scan of two short lists against
/// a mutex the session thread writes forty times a second.
fn learn_names(host: Option<NonSendMut<crate::lua::host::LuaHost>>, session: Res<Session>) {
    let Some(host) = host else { return };
    let mut board = host.social().borrow_mut();
    if !board.wants_names() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(world) = active.live.world().lock() else {
        return;
    };
    board.learn_names(|guid| world.players.get(&guid).map(|info| info.name.clone()));
}

/// **Tell the session thread which guids the two lists hold**, so the queries go
/// out — and raise the three panel events off the board's own counters.
///
/// One system rather than two because both halves read the same three counters:
/// the guid list only has to be re-stated when a list has moved, which is
/// exactly when an event is raised.
fn announce(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    mut last: Local<Option<(u32, u32, u32)>>,
    mut friends: MessageWriter<FriendListUpdate>,
    mut ignores: MessageWriter<IgnoreListUpdate>,
    mut who: MessageWriter<WhoListUpdate>,
) {
    let Some(host) = host else {
        // A host swap resets the board, so the remembered versions have to go
        // with it — otherwise the first change after a relog matches by accident
        // and the panel never hears its first event.
        *last = None;
        return;
    };
    let board = host.social().borrow();
    let now = (
        board.friends_version,
        board.ignores_version,
        board.who_version,
    );
    let before = std::mem::replace(&mut *last, Some(now));
    let Some(before) = before else { return };
    if before == now {
        return;
    }
    if before.0 != now.0 {
        friends.write(FriendListUpdate);
    }
    if before.1 != now.1 {
        ignores.write(IgnoreListUpdate);
    }
    if before.2 != now.2 {
        who.write(WhoListUpdate);
    }
    // **The guid lists, restated whenever either moved.** `note_social_*`
    // replaces rather than merges, so somebody removed stops being asked about —
    // and the hint it raises is only set for a guid the name cache has never
    // answered, so a list that has not changed costs a lock and two walks.
    if before.0 == now.0 && before.1 == now.1 {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(mut world) = active.live.world().lock() else {
        return;
    };
    if before.0 != now.0 {
        world.note_social_friends(board.friend_guids());
    }
    if before.1 != now.1 {
        world.note_social_ignores(board.ignore_guids());
    }
}

/// Send the six the panel owes.
fn send_verbs(host: Option<NonSendMut<crate::lua::host::LuaHost>>, session: Res<Session>) {
    let Some(mut host) = host else { return };
    let verbs = host.take_social_verbs();
    if verbs.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for verb in verbs {
        active.live.social(verb);
    }
}

/// The arm [`crate::world::incoming`] calls, kept here so the four packet kinds
/// and the four board writes are one file apart rather than two.
pub fn answer_of(event: &PlayerEvent) -> Option<SocialAnswer> {
    Some(match event {
        PlayerEvent::FriendList(list) => SocialAnswer::Friends(list.clone()),
        PlayerEvent::IgnoreList(list) => SocialAnswer::Ignores(list.clone()),
        PlayerEvent::FriendStatus(status) => SocialAnswer::Status(status.clone()),
        PlayerEvent::WhoResults(results) => SocialAnswer::Who(results.clone()),
        _ => return None,
    })
}
