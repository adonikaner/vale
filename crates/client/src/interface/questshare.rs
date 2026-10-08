//! Sharing quests, the client's side: the push and its results, the party
//! quest offered to the rest of the group, and the decline a shared page owes.
//!
//! The wire is [`vale_protocol::play::questshare`]. What the 1.12.1 client does:
//!
//! * `GetQuestLogPushable()` answers 1 when the quest selected in the log is
//!   in the quest cache and its flags carry `QUEST_FLAGS_SHARABLE` (`0x8`), and
//!   nil otherwise. It tests nothing else; `QuestLogFrame.lua` adds the group
//!   test itself.
//! * `QuestLogPushQuest()` sends `CMSG_PUSHQUESTTOPARTY` with the selected
//!   quest when that quest is sharable and the character has at least one party
//!   member, and does nothing otherwise, with no message.
//! * `MSG_QUEST_PUSH_RESULT` is one message-table line, `ERR_QUEST_PUSH_*_S`,
//!   with the other player's name. A result above 8 shows nothing.
//! * `SMSG_QUEST_CONFIRM_ACCEPT` stores the quest id and raises
//!   `QUEST_ACCEPT_CONFIRM` with the accepting member's name and the quest's
//!   title, which `UIParent.lua` shows as the `QUEST_ACCEPT` popup.
//! * Both lines need the other player's name to be known already: the client
//!   asks for no name in this exchange, and shows nothing when the name is not
//!   there. This client does the same. A group member's name is normally known
//!   from the roster.
//! * `ConfirmAcceptQuest()` sends `CMSG_QUEST_CONFIRM_ACCEPT` with the stored
//!   quest id, without checks, and leaves it stored.
//! * Closing a quest page whose giver is a player sends `MSG_QUEST_PUSH_RESULT`
//!   with `Declined` to that player; see `interface::quest`. Without it the
//!   server keeps the share pending and answers the sharer's next push to this
//!   character with `ERR_QUEST_PUSH_BUSY_S`.

use bevy::prelude::*;

use vale_protocol::play::questshare::{ConfirmAccept, PushOutcome, PushResult};
use vale_protocol::play::spells::PlayerEvent;

use crate::input::bindings::{Binding, BindingPressed};
use crate::interface::events::QuestAcceptConfirm;
use crate::interface::messages::Announce;
use crate::interface::party::Party;
use crate::interface::quest::Quests;
use crate::world::session::Session;

/// `QUEST_FLAGS_SHARABLE` (vmangos `QuestDef.h`).
pub const QUEST_FLAGS_SHARABLE: u32 = 0x8;

/// What the session thread said. Written by [`crate::world::incoming`].
#[derive(Message, Debug, Clone)]
pub enum QuestShareAnswer {
    PushResult(PushOutcome),
    ConfirmAccept(Box<ConfirmAccept>),
}

/// The packet this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<QuestShareAnswer> {
    match event {
        PlayerEvent::QuestPushResult(outcome) => Some(QuestShareAnswer::PushResult(*outcome)),
        PlayerEvent::QuestConfirmAccept(offer) => Some(QuestShareAnswer::ConfirmAccept(offer.clone())),
        _ => None,
    }
}

/// The party quest last offered; 0 before any offer.
#[derive(Resource, Default, Debug)]
pub struct QuestShare {
    offered: u32,
}

/// `GetQuestLogPushable()`: whether the quest selected in the log may be
/// shared.
pub fn pushable(quests: &Quests) -> bool {
    selected_quest(quests)
        .and_then(|quest_id| quests.template(quest_id))
        .is_some_and(|template| template.flags & QUEST_FLAGS_SHARABLE != 0)
}

/// The quest id of the selected log row, `None` for no row or a heading.
fn selected_quest(quests: &Quests) -> Option<u32> {
    quests.at(quests.selected()).map(|slot| slot.quest_id).filter(|&id| id != 0)
}

pub struct QuestSharePlugin;

impl Plugin for QuestSharePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<QuestShareAnswer>()
            .init_resource::<QuestShare>()
            .add_systems(
                Update,
                (offers, results, press)
                    .chain()
                    .after(crate::input::bindings::BindingSet)
                    .in_set(super::GameSet),
            );
    }
}

/// `SMSG_QUEST_CONFIRM_ACCEPT`: store the quest and ask, when the accepting
/// member's name is known.
fn offers(
    session: Res<Session>,
    mut answers: MessageReader<QuestShareAnswer>,
    mut share: ResMut<QuestShare>,
    mut raise: MessageWriter<QuestAcceptConfirm>,
) {
    let world = session.active.as_ref().map(|active| active.live.world());
    for answer in answers.read() {
        let QuestShareAnswer::ConfirmAccept(offer) = answer else {
            continue;
        };
        share.offered = offer.quest_id;
        let name = world.and_then(|world| {
            world.lock().ok().and_then(|world| world.players.get(&offer.accepter).map(|p| p.name.clone()))
        });
        if let Some(name) = name {
            raise.write(QuestAcceptConfirm { name, title: offer.title.clone() });
        }
    }
}

/// `MSG_QUEST_PUSH_RESULT`: one line per result, when the member's name is
/// known.
fn results(session: Res<Session>, mut answers: MessageReader<QuestShareAnswer>, mut say: Announce) {
    let outcomes: Vec<PushOutcome> = answers
        .read()
        .filter_map(|answer| match answer {
            QuestShareAnswer::PushResult(outcome) => Some(*outcome),
            QuestShareAnswer::ConfirmAccept(_) => None,
        })
        .collect();
    if outcomes.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(world) = active.live.world().lock() else {
        return;
    };
    for outcome in outcomes {
        let (Some(result), Some(player)) =
            (PushResult::from_code(outcome.result), world.players.get(&outcome.member))
        else {
            continue;
        };
        say.formatted(result.key(), &player.name);
    }
}

/// `QuestLogPushQuest()` and `ConfirmAcceptQuest()` on the wire.
fn press(
    mut pressed: MessageReader<BindingPressed>,
    share: Res<QuestShare>,
    quests: Res<Quests>,
    party: Res<Party>,
    session: Res<Session>,
) {
    for BindingPressed(binding) in pressed.read() {
        let Some(active) = session.active.as_ref() else {
            continue;
        };
        match binding {
            Binding::QuestLogPushQuest => {
                if party.count() == 0 || !pushable(&quests) {
                    continue;
                }
                if let Some(quest_id) = selected_quest(&quests) {
                    active.live.push_quest(quest_id);
                }
            }
            Binding::ConfirmAcceptQuest => active.live.quest_confirm_accept(share.offered),
            _ => {}
        }
    }
}
