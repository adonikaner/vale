//! The message that an item, creature or game-object template has arrived,
//! and the rule it serves, which is the client's own: a panel reads an item's
//! name once, and something has to make it read again.
//!
//! ## Why the name is blank the first time and only the first time
//!
//! Nothing in `SMSG_LOOT_RESPONSE`, `SMSG_LIST_INVENTORY` or
//! `SMSG_QUESTGIVER_QUEST_DETAILS` carries an item's *name*. Each carries an
//! entry (and, for loot and quests, a display id, which is why the **icon** is
//! right while the name is empty — the shape of the screenshot this module
//! exists for). The name costs a `CMSG_ITEM_QUERY_SINGLE` round trip, and the
//! window has already drawn by the time it lands.
//!
//! The reference's own machinery for this is a **completion callback on the
//! item cache**: a lookup that hits returns the record, and one that misses
//! files the callback, sends the query and returns nothing. The quest panel's
//! item read passes such a callback, and on arrival it reads again with no
//! callback and raises `QUEST_ITEM_UPDATE`.
//!
//! So on a miss the read really does answer an empty string — the reference
//! draws the same blank this client does — and what fixes it a moment later is
//! an *event*, raised by the cache, that makes the panel run its update again.
//! `QuestFrame_OnLoad` registers `QUEST_ITEM_UPDATE` and its handler re-runs
//! `QuestFrameItems_Update` for whichever of the three panels is visible;
//! `MerchantFrame` does the same on `MERCHANT_UPDATE`, `LootFrame` on
//! `LOOT_OPENED`, `ContainerFrame` on `BAG_UPDATE`.
//!
//! So there is **no departure here**: this client raises `QUEST_ITEM_UPDATE`
//! and so does the reference.
//!
//! ## Only what is open
//!
//! An arrival raises an event per open panel, not one event per entry: a
//! login seeds hundreds of templates out of the on-disk cache and a fresh bag
//! walk resolves dozens, and `QuestFrameItems_Update` is not something to run a
//! hundred times in a frame. [`crate::world::incoming::drain_queues`] writes
//! one [`TemplatesArrived`] per frame for the whole batch, and each panel that
//! reads it raises at most one event of its own, only while it is open:
//!
//! ```text
//! interface/quest.rs      QUEST_ITEM_UPDATE, and QUEST_LOG_UPDATE for the
//!                         log's objective names
//! interface/merchant.rs   MERCHANT_UPDATE
//! interface/loot.rs       LootNamesLanded (the loot window has no refresh event)
//! interface/items.rs      BAG_UPDATE for bags 0 to 4
//! interface/mail.rs       MAIL_INBOX_UPDATE for a parcel or stationery item
//! interface/tradeskill.rs a rebuild, then TRADE_SKILL_UPDATE or CRAFT_UPDATE
//! interface/stable.rs     PET_STABLE_UPDATE for a stabled pet's family
//! ```

use bevy::prelude::*;
use vale_protocol::state::objects::Arrivals;

/// The templates that arrived since the last frame: item, creature and
/// game-object entries. Written once per frame at most, and only when the
/// batch is not empty.
#[derive(Message, Debug, Clone)]
pub(crate) struct TemplatesArrived(pub(crate) Arrivals);

impl TemplatesArrived {
    /// Whether any item entry in `wanted` arrived in this batch.
    pub(crate) fn names_an_item_in(&self, wanted: &[u32]) -> bool {
        self.0.items.iter().any(|entry| wanted.contains(entry))
    }
}
