//! The guild's half of the session: seven packets in, nineteen requests out,
//! and four events.
//!
//! The guild's state is not here. It is the board on the Lua host, because
//! the guild tab reads back what it wrote inside one handler; see
//! [`crate::lua::panels::guild`]. [`vale_protocol::play::guild`] has the wire
//! formats. This file is the systems between the two:
//!
//! ```text
//! supply_tables     ChrClasses, AreaTable and GlobalStrings, per interface
//! track_membership  the character's own guild id and rank, from its fields
//! apply_answers     the seven packets: the board, the chat lines, the events
//! announce          GUILD_ROSTER_UPDATE and PLAYER_GUILD_UPDATE, from the
//!                   board's two counters
//! send_verbs        the requests the interface queued, and the guild queries
//!                   `GetGuildInfo` asked for
//! ```
//!
//! ## Membership is a field, not a packet
//!
//! Joining, leaving and being promoted change `PLAYER_GUILDID` and
//! `PLAYER_GUILDRANK` on the character's own object. [`track_membership`]
//! reads the two fields four times a second and writes them to the board,
//! which raises `PLAYER_GUILD_UPDATE` when either moved.
//!
//! ## An event states a change and carries no row
//!
//! `SMSG_GUILD_EVENT` names a member who joined, left, changed rank or logged
//! in. The roster held is then out of date, and the only way to correct it is
//! to ask for it again. `FriendsFrame_OnEvent` does that when
//! `GUILD_ROSTER_UPDATE` carries a first argument and the guild tab is
//! showing, so an event that changes the roster raises
//! `GUILD_ROSTER_UPDATE` with 1, and an arriving roster raises it with no
//! argument.
//!
//! ## Which sentence each event prints
//!
//! The keys are matched to the events by name: each of vmangos'
//! `GuildEvents` values has one `ERR_GUILD_*` key in `GlobalStrings.lua` whose
//! placeholders take the strings the server sends, and the two log-in events
//! reuse the friends list's keys. The message of the day prints nothing
//! here: `ChatFrame_OnEvent` prints it on `GUILD_MOTD`.

use bevy::prelude::*;

use super::events::{
    ChatMessageReceived, GuildInviteRequest, GuildMotd, GuildRosterUpdate, PlayerGuildUpdate,
};
use crate::world::session::Session;
use vale_protocol::play::guild::{CommandResult, EventKind, GuildEvent, GuildInfo};
use vale_protocol::play::spells::PlayerEvent;
use vale_protocol::socket::session::GuildVerb;

/// What the session thread said about the guild, forwarded by
/// [`crate::world::incoming::drain_events`].
#[derive(Message, Debug, Clone)]
pub enum GuildAnswer {
    Query(vale_protocol::play::guild::GuildQuery),
    Roster(vale_protocol::play::guild::Roster),
    Event(GuildEvent),
    Result(CommandResult),
    Invite(vale_protocol::play::guild::Invite),
    /// The name of the player who declined the character's invitation.
    Decline(String),
    Info(GuildInfo),
}

pub struct GuildPlugin;

impl Plugin for GuildPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<GuildAnswer>()
            .init_resource::<OwnName>()
            .add_systems(
                Update,
                // In packet order: the tables before an answer is turned into
                // words, the membership and the answers before the counters
                // are read, and the requests last, so a query for a guild
                // first seen this frame goes out this frame.
                (
                    supply_tables,
                    track_membership,
                    apply_answers,
                    announce,
                    send_verbs,
                )
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// The character's own name, for the one sentence that differs when the
/// member removed is the character. `None` until the name is known.
#[derive(Resource, Default)]
struct OwnName(Option<String>);

/// Give the board its three tables, once per interface. The host and its
/// board are replaced when the interface directory changes, so this runs
/// again after a relog. All three are cached by [`crate::assets::GameAssets`].
fn supply_tables(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
) {
    let Some(host) = host else { return };
    if host.guild().borrow().strings.is_some() {
        return;
    }
    let mut board = host.guild().borrow_mut();
    board.strings = Some(assets.strings());
    // Without either table a member's class and zone draw as `UNKNOWN`.
    if let Ok(tables) = assets.display_tables() {
        board.areas = tables.areas().cloned().map(std::sync::Arc::new);
    }
    board.classes = Some(assets.char_create());
}

/// How often the character's guild fields are read, in seconds. The read
/// takes the world lock, which the session thread also takes, and a change of
/// guild or rank is rare.
const MEMBERSHIP_INTERVAL: f32 = 0.25;

/// Copy the character's guild id and rank from its update fields to the
/// board.
fn track_membership(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    time: Res<Time>,
    mut own: ResMut<OwnName>,
    mut wait: Local<f32>,
) {
    let Some(host) = host else { return };
    *wait -= time.delta_secs();
    if *wait > 0.0 {
        return;
    }
    *wait = MEMBERSHIP_INTERVAL;
    use vale_protocol::state::fields::player;
    // No session is no guild: the board of a character who logged out must
    // not keep the last character's membership.
    let membership = session.active.as_ref().and_then(|active| {
        let world = active.live.world().lock().ok()?;
        let me = world.player()?;
        if own.0.is_none() {
            own.0 = world.players.get(&me.guid).map(|info| info.name.clone());
        }
        Some((
            me.field(player::GUILDID).unwrap_or(0),
            me.field(player::GUILDRANK).unwrap_or(0),
        ))
    });
    if membership.is_none() {
        own.0 = None;
    }
    let (guild, rank) = membership.unwrap_or((0, 0));
    host.guild().borrow_mut().set_membership(guild, rank);
}

/// Fold the server's seven answers into the board, the chat frame and the
/// events.
fn apply_answers(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    own: Res<OwnName>,
    mut answers: MessageReader<GuildAnswer>,
    mut chat: MessageWriter<ChatMessageReceived>,
    mut roster: MessageWriter<GuildRosterUpdate>,
    mut motd: MessageWriter<GuildMotd>,
    mut invite: MessageWriter<GuildInviteRequest>,
) {
    let Some(host) = host else {
        // Drained, so answers that arrive before the interface is up are not
        // replayed on the frame the host appears.
        answers.clear();
        return;
    };
    let mut board = host.guild().borrow_mut();
    let string = |board: &crate::lua::panels::guild::Guild, key: &str| -> Option<String> {
        board.strings.as_ref()?.get(key).map(str::to_string)
    };
    for answer in answers.read() {
        match answer {
            GuildAnswer::Query(query) => board.note_query(query.clone()),
            // The roster counter raises `GUILD_ROSTER_UPDATE`; see `announce`.
            GuildAnswer::Roster(list) => board.set_roster(list.clone()),
            GuildAnswer::Event(event) => {
                if event.kind == EventKind::Motd {
                    let text = event.params.first().cloned().unwrap_or_default();
                    board.set_motd(text.clone());
                    motd.write(GuildMotd(text));
                    continue;
                }
                if let Some(line) = event_key(event, own.0.as_deref())
                    .and_then(|key| string(&board, key))
                    .map(|text| fill(&text, &event_words(event)))
                {
                    super::chat::system_note(&mut chat, line);
                }
                if event.kind.changes_roster() {
                    roster.write(GuildRosterUpdate { refresh: true });
                }
            }
            GuildAnswer::Result(result) => {
                if let Some(text) = result.message().and_then(|key| string(&board, key)) {
                    super::chat::system_note(&mut chat, fill(&text, &[result.name.as_str()]));
                }
            }
            GuildAnswer::Invite(offer) => {
                invite.write(GuildInviteRequest {
                    inviter: offer.inviter.clone(),
                    guild: offer.guild.clone(),
                });
            }
            GuildAnswer::Decline(name) => {
                if let Some(text) = string(&board, "ERR_GUILD_DECLINE_S") {
                    super::chat::system_note(&mut chat, fill(&text, &[name.as_str()]));
                }
            }
            GuildAnswer::Info(info) => {
                if let Some(text) = string(&board, "GUILD_NAME_TEMPLATE") {
                    super::chat::system_note(&mut chat, fill(&text, &[info.name.as_str()]));
                }
                if let Some(text) = string(&board, "GUILD_INFO_TEMPLATE") {
                    super::chat::system_note(&mut chat, fill_numbers(&text, &info_numbers(info)));
                }
            }
        }
    }
}

/// The `GlobalStrings.lua` key an event prints, or `None` for one that prints
/// nothing. `own` is the character's name: being removed has its own
/// sentence.
fn event_key(event: &GuildEvent, own: Option<&str>) -> Option<&'static str> {
    Some(match event.kind {
        EventKind::Promotion => "ERR_GUILD_PROMOTE_SSS",
        EventKind::Demotion => "ERR_GUILD_DEMOTE_SSS",
        EventKind::Joined => "ERR_GUILD_JOIN_S",
        EventKind::Left => "ERR_GUILD_LEAVE_S",
        EventKind::Removed => {
            let removed = event.params.first().map(String::as_str);
            if removed.is_some() && removed == own {
                "ERR_GUILD_REMOVE_SELF"
            } else {
                "ERR_GUILD_REMOVE_SS"
            }
        }
        EventKind::LeaderIs => "ERR_GUILD_LEADER_IS_S",
        EventKind::LeaderChanged => "ERR_GUILD_LEADER_CHANGED_SS",
        EventKind::Disbanded => "ERR_GUILD_DISBANDED",
        EventKind::SignedOn => "ERR_FRIEND_ONLINE_SS",
        EventKind::SignedOff => "ERR_FRIEND_OFFLINE_S",
        EventKind::Motd
        | EventKind::TabardChange
        | EventKind::RankNameChanged
        | EventKind::RosterChanged
        | EventKind::Unknown(_) => return None,
    })
}

/// The words an event's sentence takes, in order. `ERR_FRIEND_ONLINE_SS`
/// takes the one name twice: once inside the player link and once as its
/// text.
fn event_words(event: &GuildEvent) -> Vec<&str> {
    let mut words: Vec<&str> = event.params.iter().map(String::as_str).collect();
    if event.kind == EventKind::SignedOn {
        if let Some(name) = words.first().copied() {
            words = vec![name, name];
        }
    }
    words
}

/// Replace each `%s` in `template` with the next word. A `%s` with no word
/// left is removed.
fn fill(template: &str, words: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut words = words.iter();
    let mut rest = template;
    while let Some(at) = rest.find("%s") {
        out.push_str(&rest[..at]);
        out.push_str(words.next().copied().unwrap_or_default());
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// Replace each `%d` in `template` with the next number.
fn fill_numbers(template: &str, numbers: &[u32]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut numbers = numbers.iter();
    let mut rest = template;
    while let Some(at) = rest.find("%d") {
        out.push_str(&rest[..at]);
        if let Some(number) = numbers.next() {
            out.push_str(&number.to_string());
        }
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// The five numbers `GUILD_INFO_TEMPLATE` takes: the creation month, day and
/// year, then the member and account counts. The order is the one the
/// comment beside the key in `GlobalStrings.lua` states.
fn info_numbers(info: &GuildInfo) -> [u32; 5] {
    [info.month, info.day, info.year, info.members, info.accounts]
}

/// Raise the two events the board's counters stand for.
fn announce(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut last: Local<Option<(u32, u32)>>,
    mut roster: MessageWriter<GuildRosterUpdate>,
    mut membership: MessageWriter<PlayerGuildUpdate>,
) {
    let Some(host) = host else {
        // A new host has a new board, whose counters start again.
        *last = None;
        return;
    };
    let board = host.guild().borrow();
    let now = (board.roster_version, board.membership_version);
    let Some(before) = std::mem::replace(&mut *last, Some(now)) else {
        return;
    };
    if before.0 != now.0 {
        roster.write(GuildRosterUpdate { refresh: false });
    }
    if before.1 != now.1 {
        membership.write(PlayerGuildUpdate);
    }
}

/// Send the requests the interface queued, and a query for each guild whose
/// name was asked for and is not held.
fn send_verbs(host: Option<NonSendMut<crate::lua::host::LuaHost>>, session: Res<Session>) {
    let Some(mut host) = host else { return };
    let mut verbs = host.take_guild_verbs();
    verbs.extend(
        host.guild()
            .borrow_mut()
            .take_wanted()
            .into_iter()
            .map(GuildVerb::Query),
    );
    if verbs.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for verb in verbs {
        active.live.guild(verb);
    }
}

/// The arm [`crate::world::incoming`] calls for the seven guild events.
pub fn answer_of(event: &PlayerEvent) -> Option<GuildAnswer> {
    Some(match event {
        PlayerEvent::GuildQuery(query) => GuildAnswer::Query(query.clone()),
        PlayerEvent::GuildRoster(roster) => GuildAnswer::Roster(roster.clone()),
        PlayerEvent::GuildEvent(event) => GuildAnswer::Event(event.clone()),
        PlayerEvent::GuildCommandResult(result) => GuildAnswer::Result(result.clone()),
        PlayerEvent::GuildInvite(invite) => GuildAnswer::Invite(invite.clone()),
        PlayerEvent::GuildDecline(name) => GuildAnswer::Decline(name.clone()),
        PlayerEvent::GuildInfo(info) => GuildAnswer::Info(info.clone()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventKind, params: &[&str]) -> GuildEvent {
        GuildEvent {
            kind,
            params: params.iter().map(|s| (*s).to_string()).collect(),
            guid: None,
        }
    }

    /// The placeholders are filled in order, and a template with more
    /// placeholders than words loses the extra ones instead of printing
    /// `%s`.
    #[test]
    fn a_sentence_takes_its_words_in_order() {
        assert_eq!(
            fill("%s has promoted %s to %s.", &["Cade", "Bram", "Officer"]),
            "Cade has promoted Bram to Officer."
        );
        assert_eq!(fill("%s has left the guild.", &[]), " has left the guild.");
        assert_eq!(fill("Guild has been disbanded.", &["x"]), "Guild has been disbanded.");
        assert_eq!(
            fill_numbers("Guild created %d-%d-%d, %d players, %d accounts", &[3, 14, 2005, 40, 31]),
            "Guild created 3-14-2005, 40 players, 31 accounts"
        );
    }

    /// Being removed prints a different sentence to the member removed than
    /// to everyone else.
    #[test]
    fn a_removal_names_the_character_differently() {
        let removed = event(EventKind::Removed, &["Bram", "Cade"]);
        assert_eq!(event_key(&removed, Some("Bram")), Some("ERR_GUILD_REMOVE_SELF"));
        assert_eq!(event_key(&removed, Some("Adele")), Some("ERR_GUILD_REMOVE_SS"));
        assert_eq!(event_key(&removed, None), Some("ERR_GUILD_REMOVE_SS"));
    }

    /// A member logging in takes the name twice, for the player link, and the
    /// message of the day has no sentence of its own.
    #[test]
    fn a_log_in_takes_the_name_twice_and_the_motd_prints_nothing() {
        let on = event(EventKind::SignedOn, &["Bram"]);
        assert_eq!(event_words(&on), ["Bram", "Bram"]);
        assert_eq!(event_key(&on, None), Some("ERR_FRIEND_ONLINE_SS"));
        assert_eq!(event_key(&event(EventKind::Motd, &["hello"]), None), None);
    }

    /// The seven guild events are forwarded and nothing else is.
    #[test]
    fn only_guild_events_become_answers() {
        assert!(answer_of(&PlayerEvent::GuildDecline("Bram".into())).is_some());
        assert!(answer_of(&PlayerEvent::IgnoreList(Vec::new())).is_none());
    }
}
