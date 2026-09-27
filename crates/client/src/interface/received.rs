//! **"You receive loot: [Linen Cloth]."** — the one packet that says an item
//! arrived, and the four lines it makes.
//!
//! `SMSG_ITEM_PUSH_RESULT` is the only statement on the wire that something
//! *entered* a bag. Everything else about the inventory is update fields, and a
//! field says what is true now rather than what happened: a stack that grew by
//! three carries nothing about whether it was looted, bought, crafted, mailed or
//! traded. `Player::SendNewItem` has twenty call sites and covers all of them.
//!
//! ## The line is the C client's, not the interface's
//!
//! Six `GlobalStrings` keys — `LOOT_ITEM_SELF`, `LOOT_ITEM_PUSHED_SELF`,
//! `LOOT_ITEM_CREATED_SELF` and their `_MULTIPLE` twins — take exactly the
//! arguments this packet carries, and **nothing in the ninety files
//! `FrameXML.toc` loads mentions any of them**. So the composition is the C
//! side's and the line reaches the chat frame as an ordinary `CHAT_MSG_LOOT`,
//! which `ChatFrame_OnEvent` already handles and this client already routes.
//! What is *not* measured is the address of the handler that does it; the key
//! choice is [`ItemPush::line_key`]'s and is written down there.
//!
//! ## Two things it has to wait for
//!
//! **The name.** An item's name is a `CMSG_ITEM_QUERY_SINGLE` round trip away,
//! and the first of anything a character ever picks up arrives before its
//! template does. A line composed without one would read "You receive loot:
//! []", so a push whose entry is not yet known is **held** and the query asked
//! for — the same deferral the reference makes for the quest-accepted chime,
//! and the same one [`super::loot`] makes for a body's rows.
//!
//! **Whose bag it was.** A loot is broadcast to the whole group
//! (`LootHandler.cpp:236` passes `broadcast = true`), so the guid in the body is
//! not necessarily ours. Without that check every party member's pickup is
//! announced as the player's own.
//!
//! ## What it replaces
//!
//! A purchase used to say `ERR_RECEIVE_ITEM_S` — *"%s received."* — raised by
//! [`super::merchant`] off `SMSG_BUY_ITEM`, because that was the
//! only packet this client read. It fired on a purchase whether or not the bag
//! had room and on nothing else. The stand-in is gone: a purchase now says the
//! game's own *"You receive item: [x]."* off the packet the reference learns
//! from, and loot, quest rewards, mail, trades and crafts say theirs for the
//! first time.

use bevy::prelude::*;

use vale_protocol::play::items::ItemPush;

use crate::world::session::Session;

/// Pushes waiting on a name.
///
/// A `Vec` rather than a map: it holds nothing at all in the ordinary case, a
/// handful for the first minute of a fresh character, and the order is the
/// order the items arrived in — which is the order the lines should be said in.
#[derive(Resource, Default)]
pub struct PendingPushes(Vec<ItemPush>);

/// How many pushes may queue up waiting for a name.
///
/// A ceiling rather than a promise. Every entry in here has a query out for it
/// and the answers come back in one round trip, so the queue is short by
/// construction — but a session whose queries are being dropped would otherwise
/// grow this without bound, and a line said a minute late is worse than one not
/// said at all.
const MAX_PENDING: usize = 64;

pub struct ReceivedPlugin;

impl Plugin for ReceivedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingPushes>()
            .add_message::<ItemReceived>()
            .add_systems(Update, announce);
    }
}

/// One `SMSG_ITEM_PUSH_RESULT`, handed on from [`crate::world::incoming`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemReceived(pub ItemPush);

fn announce(
    mut arrived: MessageReader<ItemReceived>,
    mut pending: ResMut<PendingPushes>,
    session: Res<Session>,
    inventory: Res<super::items::Inventory>,
    mut say: super::messages::Announce,
) {
    // Drained whether or not anything can be said with it, exactly as every
    // other reader in this directory is: an unread message would surface on
    // whatever frame this system next ran.
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

    // **Ask for every name that is missing before saying anything**, in one
    // pass under one lock — and take our own guid off the same borrow. The
    // queries go out on the next flush either way, and taking the lock per push
    // would put a network round trip's worth of contention on a frame that is
    // otherwise free.
    let ours = {
        let world = active.live.world();
        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
        for push in &waiting {
            if inventory.template(push.item_id).is_none() {
                world.want_item(push.item_id);
            }
        }
        world.player_guid
    };
    let Some(ours) = ours else {
        // No character yet: the pushes are held rather than said, because a
        // line about somebody else's loot cannot be told from one about ours
        // without the guid to compare against.
        pending.0 = waiting;
        pending.0.truncate(MAX_PENDING);
        return;
    };

    for push in waiting {
        // **Not ours**: a loot is broadcast to the group, and the guid is the
        // only thing that says whose bag it went into.
        if push.guid != ours {
            continue;
        }
        let Some(key) = push.line_key() else {
            // `showInChat = false`, which is what a quest reward is sent with:
            // the reward window has already shown the item and the reference
            // does not double it up.
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
        // **The count is substituted here rather than by `Announce`**, because
        // the `_MULTIPLE` keys are `"%sx%d"` and the table's own formatter fills
        // one argument. The singular keys take the link alone, so the same
        // replacement covers both: a key with no `%d` in it is untouched.
        let line = say
            .string(key)
            .map(|format| format.replacen("%s", &link, 1).replacen("%d", &push.count.to_string(), 1));
        if let Some(line) = line {
            say.loot(line);
        }
    }
}
