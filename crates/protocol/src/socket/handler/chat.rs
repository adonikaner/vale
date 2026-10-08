//! Packet handlers for what people say, the server included: chat lines,
//! channels, text emotes, the server's notices and `/roll`.
//!
//! One `pub(super) fn` per arm of [`super::apply_packet`]'s match. The parsing
//! is in `crate::play`, one directory up: `chat`, `channels`, `emotetext`,
//! `notices` and `randomroll`.

use super::{read, Incoming};
use crate::play::channels;
use crate::play::chat;
use crate::play::notices;
use crate::play::spells::PlayerEvent;
use crate::socket::world::Packet;

/// `SMSG_CHANNEL_NOTIFY`: something happened on a channel — you joined,
/// somebody left, a kick, a refusal. Thirty-two notices with seven tail
/// layouts; see [`crate::play::channels`]. A code past the enum is reported
/// through [`read`] rather than dropped, because a dropped notice leaves a
/// `/join` that fails with no message.
pub(super) fn channel_notify(ctx: &mut Incoming, pkt: &Packet) {
    let Some(notify) = read(ctx.stats, pkt, channels::parse_notify(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_event(PlayerEvent::ChannelNotify(Box::new(notify)));
}

/// `SMSG_TEXT_EMOTE`: somebody's `/dance`, whose sentence and voice line
/// the client composes — see [`crate::play::emotetext`].
pub(super) fn text_emote(ctx: &mut Incoming, pkt: &Packet) {
    let Some(emote) = read(ctx.stats, pkt, crate::play::emotetext::parse_text_emote(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_event(PlayerEvent::TextEmote(Box::new(emote)));
}

/// `SMSG_CHANNEL_LIST`: the answer to `/chatlist <channel>`.
pub(super) fn channel_list(ctx: &mut Incoming, pkt: &Packet) {
    let Some(list) = read(ctx.stats, pkt, channels::parse_list(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_event(PlayerEvent::ChannelList(Box::new(list)));
}

/// `SMSG_MESSAGECHAT`: a chat line, including lines from the server.
///
/// A GM command's reply arrives as `CHAT_MSG_SYSTEM` on this opcode, so this is
/// also how a `.tele` reports whether it worked.
pub(super) fn message(ctx: &mut Incoming, pkt: &Packet) {
    let Some(message) = read(ctx.stats, pkt, chat::parse_message(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_chat(message);
}

/// `SMSG_NOTIFICATION`: a bare string, which the server uses to explain a
/// refusal.
///
/// Examples are "You have not learned that language" and the anticheat's
/// warnings. If the packet is dropped, a rejected chat message looks the same
/// as one that was never sent, so it is added as a system line rather than
/// counted and discarded.
pub(super) fn notification(ctx: &mut Incoming, pkt: &Packet) {
    let Some(text) = read(ctx.stats, pkt, chat::parse_notification(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_chat(chat::ChatMessage {
        kind: chat::ChatType::System.code(),
        language: chat::Language::Universal.code(),
        sender: 0,
        sender_name: None,
        target: 0,
        channel: None,
        text,
        tag: 0,
    });
}

/// `SMSG_CHAT_PLAYER_NOT_FOUND`: the answer to a whisper addressed to a name
/// nobody is playing. Without it a misspelled whisper produces no reply. See
/// [`crate::play::notices`].
pub(super) fn player_not_found(ctx: &mut Incoming, pkt: &Packet) {
    let Some(name) = read(ctx.stats, pkt, notices::parse_player_not_found(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_event(PlayerEvent::PlayerNotFound { name });
}

/// `SMSG_SERVER_MESSAGE`: a shutdown or restart countdown, its cancellation,
/// or a free text from the server. See [`crate::play::notices`].
pub(super) fn server_message(ctx: &mut Incoming, pkt: &Packet) {
    let Some(message) = read(ctx.stats, pkt, notices::parse_server_message(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_event(PlayerEvent::ServerMessage(message));
}

/// `SMSG_ZONE_UNDER_ATTACK`: a guard in a zone was killed by the other team.
/// See [`crate::play::notices`].
pub(super) fn zone_under_attack(ctx: &mut Incoming, pkt: &Packet) {
    let Some(area) = read(ctx.stats, pkt, notices::parse_zone_under_attack(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_event(PlayerEvent::ZoneUnderAttack { area });
}

/// `SMSG_DEFENSE_MESSAGE`: an Eastern Plaguelands tower's announcement. See
/// [`crate::play::notices`].
pub(super) fn defense_message(ctx: &mut Incoming, pkt: &Packet) {
    let Some(message) = read(ctx.stats, pkt, notices::parse_defense_message(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_event(PlayerEvent::DefenseMessage(message));
}

/// `MSG_RANDOM_ROLL`: a `/roll` by a group member or by the player. The line
/// names the roller, so the roller's name is asked for if it is not held. See
/// [`crate::play::randomroll`].
pub(super) fn random_roll(ctx: &mut Incoming, pkt: &Packet) {
    let Some(roll) = read(ctx.stats, pkt, crate::play::randomroll::parse_random_roll(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.want_social_guid(roll.roller);
    ctx.world.note_event(PlayerEvent::RandomRoll(roll));
}
