//! The server's notices, the client's side: four packets that become one chat
//! line each and change nothing else.
//!
//! The wire is [`vale_protocol::play::notices`]. Where each line goes is the
//! 1.12.1 client's rule:
//!
//! * `SMSG_CHAT_PLAYER_NOT_FOUND` is message-table line
//!   `ERR_CHAT_PLAYER_NOT_FOUND_S` with the name, which the table sends to the
//!   chat frame as `CHAT_MSG_SYSTEM`. It goes through
//!   [`super::messages::Announce`].
//! * `SMSG_SERVER_MESSAGE` is the packet type's row of `ServerMessages.dbc`
//!   with the packet's text, as `CHAT_MSG_SYSTEM`. See
//!   [`vale_assets::tables::servermessage`].
//! * `SMSG_ZONE_UNDER_ATTACK` is `ZONE_UNDER_ATTACK` from `GlobalStrings.lua`
//!   with the zone's name, and `SMSG_DEFENSE_MESSAGE` is the packet's own
//!   text. Each is a `CHAT_MSG_CHANNEL` line, with no sender, in every joined
//!   defense channel that hears it; see [`vale_assets::tables::defense`]. A
//!   character in neither defense channel sees nothing.
//!
//! `ChatFrame.lua` registers an event named `ZONE_UNDER_ATTACK`, and the 1.12.1
//! client does not raise it; the line arrives on the channel instead, so this
//! module raises no such event either.
//!
//! `ServerMessages.dbc` is read here on first need rather than through
//! `DisplayTables`, because nothing else reads it.

use std::sync::Arc;

use bevy::prelude::*;

use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::servermessage::ServerMessages;
use vale_protocol::play::chat::ChatType;
use vale_protocol::play::spells::PlayerEvent;

use super::events::ChatMessageReceived;
use super::messages::{Announce, UiStrings};

/// What the session thread said. Written by [`crate::world::incoming`].
#[derive(Message, Debug, Clone)]
pub enum NoticeAnswer {
    PlayerNotFound(String),
    ServerMessage { kind: u32, text: String },
    ZoneUnderAttack { area: u32 },
    DefenseMessage { zone: u32, text: String },
}

/// The packet this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<NoticeAnswer> {
    Some(match event {
        PlayerEvent::PlayerNotFound { name } => NoticeAnswer::PlayerNotFound(name.clone()),
        PlayerEvent::ServerMessage(message) => NoticeAnswer::ServerMessage {
            kind: message.kind,
            text: message.text.clone(),
        },
        PlayerEvent::ZoneUnderAttack { area } => NoticeAnswer::ZoneUnderAttack { area: *area },
        PlayerEvent::DefenseMessage(message) => NoticeAnswer::DefenseMessage {
            zone: message.zone,
            text: message.text.clone(),
        },
        _ => return None,
    })
}

pub struct NoticesPlugin;

impl Plugin for NoticesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NoticeAnswer>().add_systems(
            Update,
            (say_not_found, say_lines).in_set(super::GameSet),
        );
    }
}

/// `SMSG_CHAT_PLAYER_NOT_FOUND`, through the message table. A system of its
/// own because [`Announce`] holds a chat writer, and [`say_lines`] needs one
/// too.
fn say_not_found(mut answers: MessageReader<NoticeAnswer>, mut say: Announce) {
    for answer in answers.read() {
        if let NoticeAnswer::PlayerNotFound(name) = answer {
            say.formatted("ERR_CHAT_PLAYER_NOT_FOUND_S", name);
        }
    }
}

/// The other three: a system line, or a line in each defense channel that
/// hears it.
fn say_lines(
    assets: Res<crate::assets::GameAssets>,
    strings: Res<UiStrings>,
    place: Option<Res<super::worldmap::WorldMapState>>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut answers: MessageReader<NoticeAnswer>,
    mut server_messages: Local<Option<Arc<ServerMessages>>>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    for answer in answers.read() {
        match answer {
            NoticeAnswer::PlayerNotFound(_) => {}
            NoticeAnswer::ServerMessage { kind, text } => {
                let table = server_messages.get_or_insert_with(|| Arc::new(load_server_messages(&assets)));
                chat.write(ChatMessageReceived {
                    event: super::chat::event_name(ChatType::System),
                    text: table.compose(*kind, text),
                    ..Default::default()
                });
            }
            NoticeAnswer::ZoneUnderAttack { area } => {
                let Some(host) = host.as_deref() else { continue };
                let board = host.channels().borrow();
                let Some(areas) = board.areas.clone() else { continue };
                // An area the table does not carry shows nothing.
                let (Some(zone), Some(row)) =
                    (vale_assets::tables::defense::zone_of_line(&areas, *area), areas.get(*area))
                else {
                    continue;
                };
                let Some(format) = strings.get().and_then(|s| s.get("ZONE_UNDER_ATTACK")) else {
                    continue;
                };
                let text = format.replacen("%s", &row.name, 1);
                for line in defense_lines(&board, zone, place.as_deref(), &text) {
                    chat.write(line);
                }
            }
            NoticeAnswer::DefenseMessage { zone, text } => {
                let Some(host) = host.as_deref() else { continue };
                let board = host.channels().borrow();
                // The packet's zone, or its parent when the table has one.
                let zone = board
                    .areas
                    .as_deref()
                    .and_then(|areas| vale_assets::tables::defense::zone_of_line(areas, *zone))
                    .unwrap_or(*zone);
                for line in defense_lines(&board, zone, place.as_deref(), text) {
                    chat.write(line);
                }
            }
        }
    }
}

/// One `CHAT_MSG_CHANNEL` line per joined channel that hears a line about
/// `zone`. The zone the zone channels were joined for is the zone of the area
/// the character stands in, which is the area `interface::channels` joins
/// them for.
fn defense_lines(
    board: &crate::lua::panels::channels::Channels,
    zone: u32,
    place: Option<&super::worldmap::WorldMapState>,
    text: &str,
) -> Vec<ChatMessageReceived> {
    use crate::lua::panels::channels::Channels;
    let Some(table) = board.table.as_deref() else {
        return Vec::new();
    };
    let joined_zone = match (board.areas.as_deref(), place) {
        (Some(areas), Some(place)) => areas.zone_of(place.area).map(|row| row.id),
        _ => None,
    };
    board
        .joined()
        .filter(|(_, slot)| vale_assets::tables::defense::hears(table, slot.zone_id, zone, joined_zone))
        .map(|(number, slot)| ChatMessageReceived {
            event: super::chat::event_name(ChatType::Channel),
            text: text.to_string(),
            channel: Channels::display(number, &slot.name),
            channel_name: slot.name.clone(),
            zone_channel: slot.zone_id,
            number,
            instance: slot.instance,
            ..Default::default()
        })
        .collect()
}

/// `ServerMessages.dbc`, or an empty table when the archives do not have it,
/// in which case every message is shown as `[<type>]: <text>`.
fn load_server_messages(assets: &crate::assets::GameAssets) -> ServerMessages {
    assets
        .with_archive(|archive| Ok(archive.read(&dbc_path("ServerMessages")).ok()))
        .ok()
        .flatten()
        .and_then(|raw| ServerMessages::parse(&raw).ok())
        .unwrap_or_default()
}
