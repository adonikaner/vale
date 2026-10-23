//! The session side of the social panel: four packets in, six out, and four
//! events.
//!
//! The panel's state is not here. It lives on the Lua host, because
//! `FriendsFrameFriendButton_OnClick` re-reads the whole list inside the
//! handler that changed the selection; see [`crate::lua::panels::social`]. The
//! packets are parsed and built in [`vale_protocol::play::social`].
//!
//! This file holds the parts that need a Bevy system:
//!
//! ```text
//! supply_tables   ChrClasses/ChrRaces, AreaTable and GlobalStrings, per interface
//! apply_answers   the four SMSGs, folded into the board
//! learn_names     the names, which arrive by a different route
//! announce        the four events, off the board's own three counters
//! send_verbs      the six CMSGs the panel owes
//! ```
//!
//! ## The names are not in any of the four packets
//!
//! `SMSG_FRIEND_LIST` and `SMSG_IGNORE_LIST` carry guids, and the panel draws
//! names. Each guid therefore needs a `CMSG_NAME_QUERY` round trip, which the
//! session thread's query pass sends after [`announce`] hands it the lists
//! (`LiveSession::note_social_friends` and `note_social_ignores`).
//! [`learn_names`] copies the answers onto the board and increments its
//! counter, so a row drawn as `UNKNOWN` is redrawn with the name a frame later.
//!
//! For this reason [`FriendListUpdate`] is raised from a counter and not from
//! the packet: one packet changes the list twice, and the player sees the
//! second change.
//!
//! ## `SMSG_FRIEND_STATUS` is both a change and a refusal
//!
//! The packet has twenty results in three groups: those that change a list,
//! those that refuse, and those this client has no message for. All three
//! print a line ([`FriendsResult::message`] names the `GlobalStrings.lua` key),
//! and only the first group changes the board. A refusal that changed the
//! board would add a row to the friends list every time a name the server does
//! not know was typed.
//!
//! ## Where a who answer goes is set before the search
//!
//! `ShowWhoPanel` calls `SetWhoToUI(1)`, and the parts of the interface that
//! print the answer into the chat frame call `SetWhoToUI(nil)`. The flag is
//! held on the board rather than queued with the search, because the
//! interface sets it on the frame the panel opens and the answer arrives one
//! or more round trips later.

use bevy::prelude::*;

use super::events::{
    ChatMessageReceived, FriendListShow, FriendListUpdate, IgnoreListUpdate, WhoListUpdate,
};
use crate::world::session::Session;
use vale_protocol::play::social::FriendsResult;
use vale_protocol::state::events::PlayerEvent;

/// What the session thread said about the two lists and the search, forwarded by
/// [`crate::world::incoming::drain_events`] on the same terms as every other
/// subject's.
#[derive(Message, Debug, Clone)]
pub(crate) enum SocialAnswer {
    /// `SMSG_FRIEND_LIST`: the whole list.
    Friends(Vec<vale_protocol::play::social::Friend>),
    /// `SMSG_IGNORE_LIST`: the whole list, as guids.
    Ignores(Vec<u64>),
    /// `SMSG_FRIEND_STATUS`: one answer about one player, which may be a
    /// refusal.
    Status(vale_protocol::play::social::FriendStatus),
    /// `SMSG_WHO`: the rows, and the total number online.
    Who(vale_protocol::play::social::WhoResults),
}

pub(crate) struct SocialPlugin;

impl Plugin for SocialPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SocialAnswer>().add_systems(
            Update,
            // Chained in the order a packet is handled. The tables are needed
            // before an answer is folded in (a friend's class and zone are
            // resolved on read, but the who parse is not), the answers before
            // the names are copied onto them, and both before the counters are
            // read. In any other order the event that redraws the panel fires
            // a frame before the row it is about.
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

/// Give the board its three tables, once per interface.
///
/// Per interface rather than once at startup, for the reason
/// [`crate::glue::charcreate`] gives: the host, board included, is replaced
/// whenever the directory changes, so a character who logs out and back in
/// would otherwise see a friends list with no class or zone names. All three
/// banks are cached, so a second supply is three pointer copies.
fn supply_tables(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
) {
    let Some(host) = host else { return };
    if crate::lua::panels::social::held(&host).borrow().strings.is_some() {
        return;
    }
    let social = crate::lua::panels::social::held(&host);
    let mut board = social.borrow_mut();
    board.strings = Some(assets.strings());
    // `None` rather than an error for either table: without them a friend's
    // class and zone draw as `UNKNOWN`, which is what the 1.12.1 client shows
    // when its lookup misses. See `crate::lua::panels::social`.
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
        // Drained anyway, so a list that arrives at login before the interface
        // is up is not replayed on the frame the host appears.
        answers.clear();
        return;
    };
    let social = crate::lua::panels::social::held(&host);
    let mut board = social.borrow_mut();
    for answer in answers.read() {
        match answer {
            SocialAnswer::Friends(list) => {
                board.set_friends(list);
                // `FRIENDLIST_SHOW` as well as the update, because this packet
                // is also the answer to `ShowFriends()` (`/friends` with no
                // name), and `FriendsFrame_OnEvent`'s first arm selects the tab.
                // With only the update, `/friends` shows nothing.
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
                // The rows go on the board whichever way the flag points, so a
                // `/who` printed into the chat frame is still readable by the
                // panel when it is next opened. The 1.12.1 client does the same:
                // `SetWhoToUI` decides where the announcement goes, not where
                // the answer is kept.
                board.set_who(results.rows.clone(), results.online);
            }
        }
    }
}

/// Update the board for one `SMSG_FRIEND_STATUS`, by the three groups in the
/// module note.
fn apply_status(
    board: &mut crate::lua::panels::social::Social,
    status: &vale_protocol::play::social::FriendStatus,
) {
    match status.result {
        // An online add or a friend logging in carries the whole row (level,
        // zone and class), so it replaces the row rather than only marking it
        // online.
        FriendsResult::AddedOnline | FriendsResult::Online => match status.friend {
            Some(friend) => board.note_friend(friend),
            None => board.add_friend_guid(status.guid),
        },
        // An offline add carries only the guid, and so does a friend going
        // offline. The second still clears the row's online fields; otherwise
        // the panel keeps listing the zone the friend was last seen in.
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
        // Every remaining result is a refusal ("already your friend", "player
        // not found", "your list is full") and changes neither list. Its line
        // has already been printed.
        _ => {}
    }
}

/// The chat line one result prints, in the game's own words.
///
/// `None` for a result [`FriendsResult::message`] has no key for: the mute
/// results and anything a later build added. The 1.12.1 client prints nothing
/// for those either.
fn sentence(
    board: &crate::lua::panels::social::Social,
    status: &vale_protocol::play::social::FriendStatus,
) -> Option<String> {
    let key = status.result.message()?;
    let text = board.strings.as_ref()?.get(key)?.to_string();
    // The name is whatever this client already knows. For a refusal about a
    // player who does not exist that is nothing, and the function returns
    // `None` rather than printing the format string. `ERR_FRIEND_NOT_FOUND`
    // has no `%s`, and the keys that do are about a player already on a list.
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

/// Copy in the names `CMSG_NAME_QUERY` has answered; see the module note.
///
/// Takes the world lock only while a name is missing, which after the first
/// second of a session is never. The check is a scan of two short lists, in
/// place of a mutex the session thread writes forty times a second.
fn learn_names(host: Option<NonSendMut<crate::lua::host::LuaHost>>, session: Res<Session>) {
    let Some(host) = host else { return };
    let social = crate::lua::panels::social::held(&host);
    let mut board = social.borrow_mut();
    if !board.wants_names() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let world = active.live.read();
    board.learn_names(|guid| world.players.get(&guid).map(|info| info.name.clone()));
}

/// Tell the session thread which guids the two lists hold, so their name
/// queries are sent, and raise the three panel events from the board's
/// counters.
///
/// One system rather than two because both parts read the same three counters:
/// the guid lists need restating only when a list has changed, which is when
/// an event is raised.
fn announce(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    mut last: Local<Option<(u32, u32, u32)>>,
    mut friends: MessageWriter<FriendListUpdate>,
    mut ignores: MessageWriter<IgnoreListUpdate>,
    mut who: MessageWriter<WhoListUpdate>,
) {
    let Some(host) = host else {
        // Replacing the host resets the board, so the remembered versions are
        // cleared too. Otherwise the first change after a relog could match a
        // remembered version and the panel would not receive its first event.
        *last = None;
        return;
    };
    let social = crate::lua::panels::social::held(&host);
    let board = social.borrow();
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
    // The guid lists are restated whenever either changed.
    // `LiveSession::note_social_friends` and `note_social_ignores` take the
    // world lock and replace the list rather than merge it, so a removed player
    // is no longer queried. The query hint is set only for a guid the name
    // cache has never answered, so restating an unchanged list costs a lock and
    // two walks.
    if before.0 == now.0 && before.1 == now.1 {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    if before.0 != now.0 {
        active.live.note_social_friends(board.friend_guids());
    }
    if before.1 != now.1 {
        active.live.note_social_ignores(board.ignore_guids());
    }
}

/// Send the six the panel owes.
fn send_verbs(host: Option<NonSendMut<crate::lua::host::LuaHost>>, session: Res<Session>) {
    let Some(host) = host else { return };
    let verbs = crate::lua::panels::social::take(&host);
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

/// The arm [`crate::world::incoming`] calls, kept in this file so the four
/// packet types and the four board writes are in the same file.
pub(crate) fn answer_of(event: &PlayerEvent) -> Option<SocialAnswer> {
    Some(match event {
        PlayerEvent::FriendList(list) => SocialAnswer::Friends(list.clone()),
        PlayerEvent::IgnoreList(list) => SocialAnswer::Ignores(list.clone()),
        PlayerEvent::FriendStatus(status) => SocialAnswer::Status(status.clone()),
        PlayerEvent::WhoResults(results) => SocialAnswer::Who(results.clone()),
        _ => return None,
    })
}
