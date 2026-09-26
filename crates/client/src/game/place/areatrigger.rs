//! **The client half of an instance portal — which is only the refusal.**
//!
//! Everything else about walking into a dungeon happens without this directory:
//! the volumes are `vale_assets::tables::areatrigger`, the 100 ms check and the
//! packet are `vale_protocol::play::areatrigger`'s and they run on the session
//! thread, and the *success* case is a `SMSG_NEW_WORLD` that moves the character
//! and the map under it through machinery that has existed for rounds. So the
//! only thing left for the game layer is the one outcome that has to be *said*.
//!
//! It earns a module rather than an arm in [`super::super::combat::action`] on the same terms
//! [`super::super::character::timers`] does: what arrives is news about the character's
//! environment that no button of ours started, and `drain_events` is only the
//! one reader of the queue it came in on.
//!
//! ## Why the refusal matters more than it looks
//!
//! A dungeon this account has zoned into five times in the last hour answers
//! `SMSG_TRANSFER_ABORTED` and does nothing else. Without this, that is
//! **byte-for-byte the same experience as the bug the rest of this subject
//! fixes**: you walk into the swirl, nothing happens, and there is no unhandled
//! opcode and no missing reply to find. It is the difference between "portals
//! are broken" and "you have entered too many instances recently".
//!
//! ## …and it goes to the chat frame, not `UIErrorsFrame`
//!
//! That is the reference's own routing rather than a preference.
//! `SMSG_TRANSFER_ABORTED`'s handler picks one of four `GlobalStrings.lua`
//! keys and passes it to the client's chat-message output with message class
//! **10** — its ordinary system output — where the red text at the
//! top of the screen is a different route entirely. So this uses
//! [`super::super::session::chat::system_note`], which puts a line in `ChatFrame1` through the
//! game's own `CHAT_MSG_SYSTEM` arm.
//!
//! **One thing is named rather than resolved**: class 10 is taken to be system
//! output from what it is used for; which `CHAT_MSG_*` name it maps to is not
//! confirmed.

use super::super::session::chat::system_note;
use super::super::events::ChatMessageReceived;
use super::super::messages::UiStrings;
use vale_protocol::play::areatrigger::TransferAbort;
use bevy::prelude::*;

/// `SMSG_TRANSFER_ABORTED`, forwarded off the session's event queue by
/// [`super::super::combat::action`]'s drain. The raw byte, because three of the codes are
/// deliberately silent and deciding that is this module's job.
#[derive(Message, Debug, Clone, Copy)]
pub struct TransferAborted(pub u8);

pub struct AreaTriggerPlugin;

impl Plugin for AreaTriggerPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TransferAborted>()
            .add_systems(Update, announce.in_set(super::super::GameSet));
    }
}

/// Say what the server said, in the game's own words.
fn announce(
    mut aborts: MessageReader<TransferAborted>,
    strings: Res<UiStrings>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    for TransferAborted(reason) in aborts.read().copied() {
        // `None` here is the client's own silence — reason 4, and anything
        // outside 1..=5 — rather than an unhandled case. See `TransferAbort`.
        let Some(abort) = TransferAbort::from_code(reason) else {
            continue;
        };
        // …and a key the shipped file does not carry draws nothing, which is
        // the rule every other message in this client keeps.
        let Some(text) = strings.get().and_then(|s| s.get(abort.message_key())) else {
            continue;
        };
        let text = text.to_string();
        system_note(&mut chat, text);
    }
}
