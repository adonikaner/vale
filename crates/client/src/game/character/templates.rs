//! **An item template landed — tell whoever was holding a blank.**
//!
//! This module is one system and one rule, and the rule is the client's own:
//! *a panel reads an item's name once, and something has to make it read again.*
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
//! An arrival raises an event *per open panel*, not one event per entry: a
//! login seeds hundreds of templates out of the on-disk cache and a fresh bag
//! walk resolves dozens, and `QuestFrameItems_Update` is not something to run a
//! hundred times in a frame. The whole batch collapses into at most one event
//! of each kind — which is also what the reference gets, since its callback
//! fires once per panel-visible read rather than once per record.

use bevy::prelude::*;

use super::super::events::{BagUpdate, MerchantUpdate, QuestItemUpdate};
use super::super::npc::loot::LootNamesLanded;
use crate::world::session::Session;

pub struct TemplatesPlugin;

impl Plugin for TemplatesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, wake_panels.in_set(super::super::GameSet));
    }
}

/// Which panels an arrival has to wake, given what is open.
///
/// A free function so the decision is testable with no world and no session —
/// the system below is the borrow and the drain, and this is the rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenPanels {
    pub quest: bool,
    pub merchant: bool,
    pub loot: bool,
    /// Whether anything the character *carries* could be named by an arrival.
    /// Always true while there is a session: the bags are open or not, but the
    /// paper doll, the tooltip and the action bar all read the same templates
    /// and none of them announces itself.
    pub bags: bool,
}

impl OpenPanels {
    /// Whether an arrival is worth raising anything at all for.
    pub fn any(self) -> bool {
        self.quest || self.merchant || self.loot || self.bags
    }
}

/// Drain the arrivals and raise one event per open panel.
#[allow(clippy::too_many_arguments)]
fn wake_panels(
    session: Res<Session>,
    quests: Res<super::super::npc::quest::Quests>,
    merchant: Res<super::super::npc::merchant::MerchantWindow>,
    loot: Res<super::super::npc::loot::LootWindow>,
    mut quest_item: MessageWriter<QuestItemUpdate>,
    mut merchant_update: MessageWriter<MerchantUpdate>,
    mut loot_named: MessageWriter<LootNamesLanded>,
    mut bag_update: MessageWriter<BagUpdate>,
    mut log_update: MessageWriter<super::super::events::QuestLogUpdate>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let arrived = {
        let world = active.live.world();
        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
        world.take_arrivals()
    };
    if arrived.is_empty() {
        return;
    }
    // **The log's own rows are a join too.** An objective line is a counter the
    // log carries and a name only the creature, game-object or item table has,
    // so a template landing is the only thing that can turn "  : 3/8" into
    // "Kobold Vermin slain: 3/8". `QuestLog_Update` runs off `QUEST_LOG_UPDATE`
    // and re-reads every row, which is the same redraw the titles already get
    // when a quest template lands.
    if arrived.names_a_quest_objective() {
        log_update.write(super::super::events::QuestLogUpdate);
    }
    let open = OpenPanels {
        quest: !matches!(quests.page(), super::super::npc::quest::Page::None),
        merchant: merchant.is_open(),
        loot: loot.get().is_some(),
        bags: true,
    };
    if open.quest {
        quest_item.write(QuestItemUpdate);
    }
    if open.merchant {
        merchant_update.write(MerchantUpdate);
    }
    // **The loot window is the one panel with no refresh event at all**, and
    // this arm used to re-raise `LOOT_OPENED` in the belief that it had one.
    // It does not: 5875 has no `LOOT_SLOT_CHANGED`, and `LootFrame_OnEvent`'s
    // `LOOT_OPENED` arm is `ShowUIPanel(this)`, which `UIParent.lua` returns
    // from immediately for a frame that is already visible. So the re-raise was a
    // no-op and a row that gained a name kept its blank for as long as the body
    // was held.
    //
    // Two things replace it, and neither is an event: the window is **held
    // shut** until its rows can be named ([`super::super::npc::loot::hold`]),
    // and if that hold ever times out, this message reaches the interface and
    // calls the frame's own `LootFrame_Update` where it stands. See
    // [`LootNamesLanded`].
    if open.loot {
        loot_named.write(LootNamesLanded);
    }
    // `arg1 = -1` is not a bag: `ContainerFrame_OnEvent` compares `arg1` against
    // each frame's own id and redraws on a match, and the paper doll and the
    // tooltip read the same templates without any id at all. Broadcasting to
    // every container is the honest answer for "some item somewhere is named
    // now", and it is one event rather than one per bag.
    if open.bags {
        for bag in 0..=4 {
            bag_update.write(BagUpdate(bag));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Nothing open is nothing raised.** The seed out of the on-disk item
    /// cache lands before anything is on screen and would otherwise be a batch
    /// of redraws for a panel that does not exist.
    #[test]
    fn an_arrival_with_nothing_open_wakes_nothing() {
        assert!(!OpenPanels::default().any());
        assert!(OpenPanels {
            bags: true,
            ..OpenPanels::default()
        }
        .any());
    }
}
