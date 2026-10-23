//! The chat line for an item entering a bag ("You receive loot: [Linen
//! Cloth]."), from `SMSG_ITEM_PUSH_RESULT`, and the four forms it takes.
//!
//! `SMSG_ITEM_PUSH_RESULT` is the only packet that states an item entered a
//! bag. The rest of the inventory is update fields, which state the current
//! value and not the event: a stack that grew by three does not say whether it
//! was looted, bought, crafted, mailed or traded. vmangos'
//! `Player::SendNewItem` has twenty call sites and covers all of these.
//!
//! ## The line is composed outside the interface code
//!
//! Six `GlobalStrings` keys (`LOOT_ITEM_SELF`, `LOOT_ITEM_PUSHED_SELF`,
//! `LOOT_ITEM_CREATED_SELF` and their `_MULTIPLE` forms) take the arguments
//! this packet carries, and none of the ninety files `FrameXML.toc` loads
//! mentions any of them. The 1.12.1 client therefore composes the line itself
//! and sends it to the chat frame as an ordinary `CHAT_MSG_LOOT`, which
//! `ChatFrame_OnEvent` handles and this client already routes. What is not
//! measured is the address of the handler that does it; the key choice is
//! [`ItemPush::line_key`]'s and is documented there.
//!
//! ## What a line waits for
//!
//! The item's name. A name is one `CMSG_ITEM_QUERY_SINGLE` round trip away,
//! and the first of any item a character picks up arrives before its
//! template. A line composed without the name would read "You receive loot:
//! []", so a push whose entry is not known yet is held and the query is
//! queued. The 1.12.1 client defers the quest-accepted sound the same way,
//! and [`super::loot`] defers a body's rows the same way.
//!
//! Whose bag it was. A loot is broadcast to the whole group
//! (`LootHandler.cpp:236` passes `broadcast = true`), so the guid in the body
//! need not be the player's. Without the check, every party member's pickup
//! would be announced as the player's own.
//!
//! ## Purchases
//!
//! A purchase says the game's own "You receive item: [x]." from this packet.
//! [`super::merchant`] does not raise `ERR_RECEIVE_ITEM_S` ("%s received.")
//! from `SMSG_BUY_ITEM`: that line fired on every purchase whether or not the
//! bag had room, and on nothing else. Loot, quest rewards, mail, trades and
//! crafts take their lines from this packet too.

use bevy::prelude::*;

use vale_protocol::play::items::ItemPush;

use crate::world::session::Session;

/// Pushes waiting on a name.
///
/// A `Vec` rather than a map: it is empty in the ordinary case and holds a
/// handful for the first minute of a new character, and its order is the order
/// the items arrived in, which is the order the lines are said in.
#[derive(Resource, Default)]
pub(crate) struct PendingPushes(Vec<ItemPush>);

/// How many pushes may queue up waiting for a name.
///
/// Every entry here has a query out for it and the answers arrive in one round
/// trip, so the queue is normally short. The limit bounds it for a session
/// whose queries are dropped; a line said a minute late is worse than no line.
const MAX_PENDING: usize = 64;

pub(crate) struct ReceivedPlugin;

impl Plugin for ReceivedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingPushes>()
            .add_message::<ItemReceived>()
            .add_systems(Update, announce);
    }
}

/// One `SMSG_ITEM_PUSH_RESULT`, handed on from [`crate::world::incoming`].
#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct ItemReceived(pub(crate) ItemPush);

fn announce(
    mut arrived: MessageReader<ItemReceived>,
    mut pending: ResMut<PendingPushes>,
    session: Res<Session>,
    inventory: Res<super::items::Inventory>,
    mut say: super::messages::Announce,
) {
    // Drained whether or not anything can be said, as every other reader in
    // this directory is: an unread message would be read on whatever frame
    // this system next ran.
    let fresh: Vec<ItemPush> = arrived.read().map(|received| received.0).collect();
    let Some(active) = session.active.as_ref() else {
        pending.0.clear();
        return;
    };
    if fresh.is_empty() && pending.0.is_empty() {
        return;
    }

    let mut waiting = std::mem::take(&mut pending.0);
    waiting.extend(fresh);

    // Every item named by a push is queued for a template query before
    // anything is said. `LiveSession::want_items` takes the world lock once
    // for the batch, and `want_item` skips an entry the session already knows.
    // The queries go out on the next flush either way; taking the lock per push
    // would put a network round trip's worth of contention on a frame that is
    // otherwise free.
    active.live.want_items(waiting.iter().map(|push| push.item_id));
    let ours = {
        let world = active.live.read();
        world.player_guid
    };
    let Some(ours) = ours else {
        // No character yet: the pushes are held, because without the player's
        // guid a push to somebody else's bag cannot be told from one to the
        // player's.
        pending.0 = waiting;
        pending.0.truncate(MAX_PENDING);
        return;
    };

    for push in waiting {
        // Another character's push: a loot is broadcast to the group, and the
        // guid is the only field that says whose bag the item went into.
        if push.guid != ours {
            continue;
        }
        let Some(key) = push.line_key() else {
            // `showInChat = false`, which a quest reward is sent with: the
            // reward window has already shown the item, and the 1.12.1 client
            // does not print a second line for it.
            continue;
        };
        let Some(template) = inventory.template(push.item_id) else {
            if pending.0.len() < MAX_PENDING {
                pending.0.push(push);
            }
            continue;
        };
        let link = crate::lua::panels::container::item_link(
            template.entry,
            template.quality,
            &template.name,
        );
        // The count is substituted here rather than by `Announce`, because the
        // `_MULTIPLE` keys are `"%sx%d"` and the table's formatter fills one
        // argument. The singular keys take the link alone, so the same
        // replacement covers both: a key with no `%d` is left unchanged.
        let line = say
            .string(key)
            .map(|format| format.replacen("%s", &link, 1).replacen("%d", &push.count.to_string(), 1));
        if let Some(line) = line {
            say.loot(line);
        }
    }
}
