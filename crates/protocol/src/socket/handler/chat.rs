//! What people say — including the server.
//!
//! One `pub(super) fn` per arm of [`super::apply_packet`]'s match. The parsing
//! itself is `crate::play::chat`, one directory up.

use super::{read, Incoming};
use crate::play::channels;
use crate::play::chat;
use crate::play::spells::PlayerEvent;
use crate::socket::world::Packet;

/// `SMSG_CHANNEL_NOTIFY`: something happened on a channel — you joined,
/// somebody left, a kick, a refusal. Thirty-two notices with seven tail
/// layouts; see [`crate::play::channels`]. A code past the enum is reported
/// through [`read`] rather than dropped, since a silent one is a `/join`
/// that never says why it did nothing.
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

/// `SMSG_MESSAGECHAT`.
///
/// Including the server: a GM command's reply comes back as `CHAT_MSG_SYSTEM`
/// on this opcode, so this is also how a `.tele` says whether it worked.
pub(super) fn message(ctx: &mut Incoming, pkt: &Packet) {
    let Some(message) = read(ctx.stats, pkt, chat::parse_message(&pkt.body)) else {
        return;
    };
    ctx.stats.chat += 1;
    ctx.world.note_chat(message);
}

/// `SMSG_NOTIFICATION`: a bare string, and the channel the server explains a
/// *refusal* on.
///
/// "You have not learned that language", and the anticheat's own warnings.
/// Dropped, a rejected chat message is indistinguishable from one that was
/// never sent — so it is folded in as a system line rather than counted and
/// discarded.
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
