//! The client half of an area trigger: what the server says back.
//!
//! Everything else about walking into a dungeon happens outside this module.
//! The volumes are `vale_assets::tables::areatrigger`, the 100 ms test and the
//! packet are `vale_protocol::play::areatrigger`'s and run on the session
//! thread, and a teleport that goes ahead is a `SMSG_NEW_WORLD`, which moves
//! the character and the map through the ordinary transfer path. What is left
//! for the game layer is the server's answers that have to be shown.
//!
//! It is a module rather than an arm in [`crate::interface::action`] on
//! [`crate::interface::timers`]' terms: what arrives is news no button of ours
//! started, and `drain_events` is only the reader of the queue it came on.
//!
//! ## Why the refusal has to be shown
//!
//! A dungeon this account has entered five times in the last hour answers
//! `SMSG_TRANSFER_ABORTED` and nothing else. Without a message, that looks
//! the same as a portal that was never reported: the character walks into it
//! and nothing happens. With it, the player reads "you have entered too many
//! instances recently".
//!
//! ## A teleport's own refusal goes to `UIErrorsFrame`
//!
//! A teleport the character is too low for, or fails the condition of,
//! answers `SMSG_AREA_TRIGGER_MESSAGE` with the row's text. The 1.12.1 client
//! raises that text as `UI_INFO_MESSAGE`, the yellow line in `UIErrorsFrame`,
//! and shows nothing for an empty text. The text is the server's, not a
//! `GlobalStrings.lua` key, so this module writes [`UiInfoMessage`] itself
//! rather than through [`crate::interface::messages::UiErrors`], which takes
//! keys only.
//!
//! ## The refusal goes to the chat frame
//!
//! The 1.12.1 client shows a transfer refusal as a system line in the chat
//! frame, not in `UIErrorsFrame`: it passes one of four `GlobalStrings.lua`
//! keys to its chat output with message class 10, its ordinary system output.
//! This module uses [`crate::interface::chat::system_note`], which adds a line
//! to `ChatFrame1` through the game's own `CHAT_MSG_SYSTEM` arm. Which
//! `CHAT_MSG_*` name class 10 maps to is taken from what it is used for and is
//! not confirmed.

use crate::interface::chat::system_note;
use crate::interface::events::{ChatMessageReceived, UiInfoMessage};
use crate::interface::messages::UiStrings;
use vale_protocol::play::areatrigger::TransferAbort;
use bevy::prelude::*;

/// `SMSG_TRANSFER_ABORTED`, forwarded off the session's event queue by
/// [`crate::interface::action`]'s drain. The raw byte, because three of the codes are
/// deliberately silent and deciding that is this module's job.
#[derive(Message, Debug, Clone, Copy)]
pub struct TransferAborted(pub u8);

/// `SMSG_AREA_TRIGGER_MESSAGE`'s text, forwarded the same way.
#[derive(Message, Debug, Clone)]
pub struct AreaTriggerMessage(pub String);

pub struct AreaTriggerPlugin;

impl Plugin for AreaTriggerPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TransferAborted>()
            .add_message::<AreaTriggerMessage>()
            .add_systems(Update, (announce, show_refusal_text).in_set(crate::interface::GameSet));
    }
}

/// Say what the server said, in the game's own words.
fn announce(
    mut aborts: MessageReader<TransferAborted>,
    strings: Res<UiStrings>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    for TransferAborted(reason) in aborts.read().copied() {
        // `None` is a reason the 1.12.1 client shows nothing for (4, and
        // anything outside 1..=5), not an unhandled case. See `TransferAbort`.
        let Some(abort) = TransferAbort::from_code(reason) else {
            continue;
        };
        // …and a key the shipped file does not carry shows nothing, the rule
        // every other message in this client keeps.
        let Some(text) = strings.get().and_then(|s| s.get(abort.message_key())) else {
            continue;
        };
        let text = text.to_string();
        system_note(&mut chat, text);
    }
}

/// Show a teleport's refusal text as a `UI_INFO_MESSAGE`. An empty text shows
/// nothing.
fn show_refusal_text(mut texts: MessageReader<AreaTriggerMessage>, mut info: MessageWriter<UiInfoMessage>) {
    for AreaTriggerMessage(text) in texts.read() {
        if text.is_empty() {
            continue;
        }
        info.write(UiInfoMessage(text.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_message::<AreaTriggerMessage>()
            .add_message::<UiInfoMessage>()
            .add_systems(Update, show_refusal_text);
        app
    }

    fn shown(app: &mut App) -> Vec<String> {
        app.world_mut()
            .resource_mut::<Messages<UiInfoMessage>>()
            .drain()
            .map(|UiInfoMessage(text)| text)
            .collect()
    }

    /// The text reaches `UI_INFO_MESSAGE` unchanged, and an empty one is not
    /// shown.
    #[test]
    fn a_refusal_text_is_a_yellow_line_and_an_empty_one_is_nothing() {
        let mut app = app();
        app.world_mut().write_message(AreaTriggerMessage("You must be level 50 to enter.".to_string()));
        app.world_mut().write_message(AreaTriggerMessage(String::new()));
        app.update();
        assert_eq!(shown(&mut app), vec!["You must be level 50 to enter.".to_string()]);
    }
}
