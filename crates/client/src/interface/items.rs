//! The character's inventory, as the interface queries it.
//!
//! [`vale_protocol::play::items`] holds the rules: which field is which slot,
//! and which GUID resolves to which object. They live there for the same reason
//! the rules in `assets/dress.rs` live in `assets`: they need no renderer, so a
//! test with no session checks the same code the client runs. This module does
//! the three things that need a session:
//!
//! ```text
//! when to rebuild   inventory_version changes; the spell book uses the same latch
//! what to look up   an entry with no template yet -> want_item
//! who to tell       BAG_UPDATE per bag, UNIT_INVENTORY_CHANGED, PLAYER_MONEY
//! ```
//!
//! ## Money is compared outside the rebuild latch
//!
//! `PLAYER_FIELD_COINAGE` is not one of the slot runs
//! [`vale_protocol::state::objects`] counts as an inventory change, because
//! spending money moves nothing a bag square draws. The money therefore has its
//! own comparison before the rebuild latch, not inside it. When the comparison
//! was inside the latch, paying a trainer changed neither the version nor the
//! template count, so the gold on the trainer window and in every bag stayed
//! stale until the player moved an item, which does change the version.
//!
//! ## Item templates arrive after the slots
//!
//! An inventory read off the update fields is GUIDs, entries and counts. It has
//! no names, icons or tooltips, because `Item.dbc` is not in the 1.12 archives
//! and all three come back from `CMSG_ITEM_QUERY_SINGLE`. The bags are
//! therefore filled in two phases at every login: the slots appear with their
//! counts and no art, and each icon appears when its reply arrives. The 1.12.1
//! client behaves the same way with a cold item cache. [`Inventory::icon`]
//! answers `None` for a missing template and not a placeholder, because a
//! placeholder cannot be told apart from a wrong icon.
//!
//! The queries are sent by [`vale_protocol::state::objects::ObjectManager`]'s
//! query pass, which already walks every `Item` and `Container` object it
//! holds, so this module sends no packet for them. This module raises the
//! events again when a template arrives. Without that, a bag opened before the
//! replies arrived stays blank: `ContainerFrame_Update` runs on `BAG_UPDATE`
//! and on nothing else.
//!
//! ## `BAG_UPDATE` is raised per bag; `UNIT_INVENTORY_CHANGED` covers everything held
//!
//! `ContainerFrame_OnEvent` compares `arg1` against its own frame id, so a
//! `BAG_UPDATE` with the wrong number redraws nothing. The diff below is
//! therefore per container. The worn slots raise
//! `UNIT_INVENTORY_CHANGED("player")`, which is the event the twenty-four
//! paper-doll buttons listen for. Raising only one of the two events leaves
//! either the bags or the paper doll stale.
//!
//! `UNIT_INVENTORY_CHANGED` is not limited to equipment. It is raised for a
//! change to anything the character holds, bags included, because an action
//! button is a third listener: it registers this event and no other event a
//! bag change raises, and `ActionButton_Update` is the only function in the
//! directory that re-reads `GetActionCount`. When the event was raised for
//! equipment only, a potion drunk off the bar kept the count it was drawn with
//! while every other frame updated.

use bevy::prelude::*;

use super::events::{
    BagUpdate, PlayerMoney, PlayerbankbagslotsChanged, PlayerbankslotsChanged, UnitInventoryChanged,
};
use crate::assets::GameAssets;
use crate::world::session::Session;
use vale_protocol::play::items::{self, Inventory as Carried, ItemPlace, ItemSlot};
use vale_protocol::state::query::ItemInfo;
use std::collections::HashMap;

/// Every container id the interface can address, in the order the diff walks
/// them. This is [`vale_protocol::play::items::CONTAINERS_FOR_REPORT`], reused
/// so that this diff and the `bags` report of `vale live` cover the same set.
pub const CONTAINERS: [i32; 6] = items::CONTAINERS_FOR_REPORT;

/// The character's inventory and the item templates for it.
///
/// Empty before the first update block, which is the character screen, so
/// `GetContainerNumSlots` answers 0 there and not the last character's bags.
#[derive(Resource, Default)]
pub struct Inventory {
    /// The slots, as [`vale_protocol::play::items`] reads them.
    pub carried: Carried,
    /// `PLAYER_AMMO_ID`: the entry loaded in the ammo slot, or 0 for nothing.
    /// It is not a slot. It changes only through
    /// [`vale_protocol::play::items::set_ammo_body`], and
    /// [`Self::ammo_count`] is the number the square draws under it.
    pub ammo: u32,
    /// `PLAYER_FIELD_COINAGE`, in copper.
    pub money: u32,
    /// Item templates, by entry, copied out of the session's cache.
    ///
    /// A copy and not a borrow, because every read happens inside a scoped Lua
    /// call, where taking the world lock per `GetContainerItemInfo` would make
    /// a hover wait on the network thread. Entries are never removed: a
    /// template describes an item, not the fact of carrying one.
    templates: HashMap<u32, ItemInfo>,
    /// The `inventory_version` this was built from.
    built_from: Option<u32>,
    /// The size of the session's whole template cache when this was built. A
    /// reply that arrives after the slots changes this size, so the events are
    /// raised again; without it a bag opened on a cold cache stays blank (see
    /// the module comment). It counts the whole cache and not only this
    /// character's entries; [`rebuild`] explains why.
    templates_seen: usize,
}

impl Inventory {
    /// The template for an entry, or `None` while its reply is in flight.
    pub fn template(&self, entry: u32) -> Option<&ItemInfo> {
        self.templates.get(&entry)
    }

    /// The template for the item in a slot, or `None` while its reply is in
    /// flight.
    pub fn template_of(&self, item: &ItemSlot) -> Option<&ItemInfo> {
        self.templates.get(&item.entry)
    }

    /// The number of rounds of the loaded ammo, which is
    /// `GetInventoryItemCount("player", 0)`. That is `GetItemCount` of the
    /// loaded entry: the square counts what is in the bags, and shooting takes
    /// one from there. 0 with nothing loaded.
    pub fn ammo_count(&self) -> u32 {
        if self.ammo == 0 {
            0
        } else {
            self.carried.count_of(self.ammo)
        }
    }

    /// Insert a template directly, for a test that needs an item with a name.
    /// [`rebuild`] is the only writer in a real session and it needs a live
    /// one.
    #[cfg(test)]
    pub(crate) fn insert_template(&mut self, template: ItemInfo) {
        self.templates.insert(template.entry, template);
    }

    /// The icon path for a slot, or `None` for one whose template has not
    /// arrived. The module comment explains why this is not a placeholder.
    pub fn icon(&self, assets: &GameAssets, item: &ItemSlot) -> Option<String> {
        let tables = assets.display_tables().ok()?;
        tables.item_icon(self.template(item.entry)?.display_id)
    }

    /// How many slots a container has, by bag id. `0` for a bag id with no bag
    /// in it; `ToggleBag` does not open a bag with 0 slots.
    pub fn container_slots(&self, bag: i32) -> usize {
        self.carried.container(bag).map_or(0, <[_]>::len)
    }
}

/// The server's refusal of an item command: `SMSG_INVENTORY_CHANGE_FAILURE`,
/// forwarded off [`vale_protocol::socket::session::LiveSession::take_events`] by
/// [`super::action`], which is the only drain of that queue.
///
/// It is a message for the same reason the answers in `logout` and `death`
/// are: the queue has exactly one reader, and the subject belongs to this
/// module.
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemRefused(pub vale_protocol::play::items::InventoryFailure);

/// Use an item named by entry, wherever it is carried. The action bar can name
/// an item only this way.
///
/// `SMSG_ACTION_BUTTONS` stores an item slot as its entry and nothing else, so
/// pressing one takes two steps: find where that entry is, then do what a
/// right-click on it in the bag does. The search is
/// [`vale_protocol::play::items::Inventory::find_entry`], which follows the
/// 1.12.1 client's `UseAction`. The use is [`use_item`], the same body a click
/// goes through. This is a message and not a second copy of that body so that
/// the equip-or-use decision, the aiming and the cast bar have one
/// implementation.
///
/// Written by [`super::action::use_action`]; read below.
#[derive(Message, Debug, Clone, Copy)]
pub struct UseCarriedItem(pub u32);

/// The item whose spell raised the targeting cursor, kept so that the answer
/// can name it.
///
/// A sharpening stone right-clicked in the bag does not cast anything. It asks
/// which weapon, and the packet that is then sent is `CMSG_USE_ITEM` naming the
/// stone with `TARGET_FLAG_ITEM` naming the weapon, because the server reads
/// the stone's charges out of that packet. This resource keeps the stone's
/// place until the weapon is chosen.
///
/// It is empty when the cursor was raised by a spell pressed on the bar: that
/// is answered with `CMSG_CAST_SPELL` and has no source item. Both cases use
/// the single cursor in [`super::action::SpellTargeting`], and this resource is
/// the only thing that tells them apart.
#[derive(Resource, Default)]
pub struct PendingItemCast(Option<PendingItem>);

#[derive(Clone, Copy)]
pub struct PendingItem {
    /// The bag and slot in the server's numbering, already converted. This is
    /// the pair `CMSG_USE_ITEM` takes.
    pub bag: u8,
    pub slot: u8,
    /// Which of the template's five spells is being aimed.
    pub spell_index: u8,
}

impl PendingItemCast {
    fn arm(&mut self, item: PendingItem) {
        self.0 = Some(item);
    }

    /// Take the pending item and leave nothing, so it is answered once.
    ///
    /// `pub(crate)` because the trade window answers the same pending cast at
    /// a different square: an oil or a scroll aimed at the "will not be
    /// traded" slot is the same pending cast with a different target. See
    /// `crate::interface::trade`.
    pub(crate) fn take(&mut self) -> Option<PendingItem> {
        self.0.take()
    }

    /// Whether a cast from an item is waiting for a target.
    pub fn is_armed(&self) -> bool {
        self.0.is_some()
    }

    /// Drop the pending item unanswered, on Escape or a fresh press. Public
    /// because [`super::action`] stands down the cursor this belongs to, and
    /// the pending item must not outlive that cursor.
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

/// Raise the targeting cursor for a cast begun by an item.
///
/// A message and not a direct write, because `SpellTargeting` is owned by
/// [`super::action`] and this system already holds eight parameters. A
/// `ResMut` here would be a second writer of the cursor, which
/// `SpellTargetPicked` exists to avoid.
#[derive(Message, Debug, Clone, Copy)]
pub struct BeginItemTargeting(pub u32);

pub struct ItemsPlugin;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Inventory>()
            .add_message::<ItemRefused>()
            .add_message::<UseCarriedItem>()
            .add_message::<BeginItemTargeting>()
            .init_resource::<PendingItemCast>()
            .add_systems(
                Update,
                (rebuild, use_item, refusals, leave_world)
                    // After the action bar's systems, because a press writes
                    // [`UseCarriedItem`], and a message written after its
                    // reader has run is read a frame late. The bindings
                    // `use_item` reads directly are in the same order behind
                    // the same set.
                    .after(super::action::ActionSet)
                    .in_set(super::GameSet),
            );
    }
}

/// Show the reason the server refused an item command, as a `GlobalStrings.lua`
/// line.
///
/// Every refusal of an item command is `SMSG_INVENTORY_CHANGE_FAILURE`. When
/// nothing read it, a right-click the server refused showed nothing, the same
/// as a right-click on empty ground. A potion eleven levels too high is one
/// case: `Player::CanUseItem` refuses before a spell is prepared, so there is
/// no `SMSG_CAST_RESULT` to carry the reason and no other packet is sent.
///
/// The code is an index and the text is the game's, as for a cast failure:
/// [`vale_protocol::play::items::inventory_failure_key`] answers a
/// `GlobalStrings.lua` key, and [`super::messages::UiErrors`] is the only way
/// to turn a key into a line. A code with no string shows nothing. One of the
/// sixty-seven codes has no string by design, and `HandleUseItemOpcode` sends
/// that code before the real reason on every failed use.
fn refusals(mut refused: MessageReader<ItemRefused>, mut say: super::messages::Announce) {
    for ItemRefused(failure) in refused.read() {
        let Some(key) = vale_protocol::play::items::inventory_failure_key(failure.code) else {
            continue;
        };
        // One key in the family takes an argument: "You must reach level %d to
        // use that item." The level is the item's. It is in the packet because
        // the client may not know it: the prototype may not have arrived.
        if failure.code == vale_protocol::play::items::EQUIP_ERR_CANT_EQUIP_LEVEL_I {
            say.formatted(key, &failure.required_level.to_string());
        } else {
            say.key(key);
        }
    }
}

/// The three things a click on an item can open, bundled.
///
/// Grouped for the reason [`crate::world::incoming::NpcAnswers`] is: `use_item`
/// had reached Bevy's sixteen-parameter limit and the page window was the
/// seventeenth. None of the three is state. Each is a window or a mode that
/// the click hands off to, not something the click reads.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Opens<'w> {
    /// The answer to the spell cursor: the item the player chose.
    picked: MessageWriter<'w, super::action::SpellItemPicked>,
    /// The item's own spell needs a target, so the cursor is raised.
    begin: MessageWriter<'w, BeginItemTargeting>,
    /// The item is a book, a note or a scroll. See
    /// [`crate::interface::pagetext`].
    read_page: MessageWriter<'w, crate::interface::pagetext::ReadPage>,
}

/// The right-click on an item: `UseContainerItem(bag, slot)` and its
/// paper-doll equivalent, turned into the packet the item's prototype calls
/// for.
///
/// The whole decision is made here because the click carries a slot and
/// nothing else. What a click on that slot means depends on the item
/// ([`vale_protocol::state::query::ItemInfo`]), which arrives a round trip
/// after the slot does.
///
/// ```text
/// StartQuest != 0                  the quest page, and no packet before it
/// in a bag,  InventoryType == 24   CMSG_SET_AMMO
/// in a bag,  InventoryType != 0    CMSG_AUTOEQUIP_ITEM   the server picks the slot
/// wrapped                          CMSG_OPEN_ITEM        the server unwraps it
/// PageText != 0                    the page window, and no packet before it
/// ITEM_FLAG_LOOTABLE               CMSG_OPEN_ITEM        the server answers with a loot window
/// has an ON_USE spell              CMSG_USE_ITEM         with that spell's index
/// none of those                    nothing at all
/// ```
///
/// The last line is the ordinary case: a stack of linen, a quest token. The
/// 1.12.1 client sends nothing for one. `HandleUseItemOpcode` refuses anything
/// whose named spell block is not `ON_USE`, so a packet would produce
/// `EQUIP_ERR_ITEM_NOT_FOUND` in the error frame for an item that is in the
/// bag.
///
/// The order is the 1.12.1 client's. `UseContainerItem` equips an item whose
/// `InventoryType != 0`, unless `StartQuest != 0`, in which case it uses the
/// item instead. A quest starter that is also a garment therefore opens its
/// page and is not worn. Every other check applies only to an inventory type
/// of zero. In order, they are: a bag does nothing (`OBJECT_FIELD_TYPE` bit
/// 2), the wrapper pair, `StartQuest`, the usability check, `PageText`, the
/// item's own `ITEM_FIELD_ITEM_TEXT_ID`, and `ITEM_FLAG_LOOTABLE` before the
/// search for an on-use spell.
///
/// This client lacks one check from that list, the letter:
/// `ITEM_FIELD_ITEM_TEXT_ID` on the item object, which opens the same
/// `ItemTextFrame` the mailbox does through `CMSG_ITEM_TEXT_QUERY`. It needs a
/// field on `ItemSlot` and an entry point into [`crate::interface::mail`]'s
/// reader, and nothing else.
///
/// The equip row applies to a bag click only, as the first column of the table
/// says. An item that is already worn passes the server's `IsEquipped()` gate,
/// so right-clicking a trinket on the paper doll uses it. Right-clicking a
/// worn garment takes it off, and that is `CMSG_AUTOSTORE_BAG_ITEM`, a packet
/// this client does not send. It therefore sends nothing, because the server
/// would answer a `CMSG_AUTOEQUIP_ITEM` by re-equipping the item where it
/// already is.
///
/// A slot whose template has not arrived does nothing. Both the equip and the
/// use decision need the prototype. Assuming "use" would send an equippable as
/// `CMSG_USE_ITEM` and get the same refusal. The templates arrive within a
/// second of login (see the module comment), and a click before that is a
/// click on a square with no icon on it.
///
/// A use also starts the cast locally. An item whose spell has a cast time, a
/// bandage or a scroll, raises the cast bar, and its cooldown swirl starts at
/// the press and not a round trip later. See
/// [`super::action::begin_item_cast`], which holds the three local effects.
///
/// The action bar is a third source of uses. It names an item by entry and not
/// by place; see [`UseCarriedItem`]. The entry is resolved to a place first
/// and then goes through this body, so a mount on the bar and the same mount
/// right-clicked in the bag are one code path.
#[allow(clippy::too_many_arguments)]
fn use_item(
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    mut wanted: MessageReader<UseCarriedItem>,
    inventory: Res<Inventory>,
    selection: Res<super::target::Selection>,
    // The two windows that change what the click means. See the sell and
    // deposit branches below, and the module notes of [`super::merchant`] and
    // [`super::bank`].
    merchant: Res<super::merchant::MerchantWindow>,
    bank: Res<super::bank::BankWindow>,
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    mut cooldowns: ResMut<super::action::Cooldowns>,
    mut events: super::action::ActionEvents,
    // The spell cursor, which makes this click an answer to it. See the branch
    // below and `SpellTargeting::wants_item`.
    targeting: Res<super::action::SpellTargeting>,
    mut pending: ResMut<PendingItemCast>,
    // The three things the click can open, bundled. See [`Opens`].
    mut opens: Opens,
    // The one error line the client raises itself. See the cooldown refusal
    // below, which is the same message `cast_known_spell` sends for a spell.
    mut errors: super::messages::UiErrors,
    // The caster, for the pre-cast checks that `cast_known_spell` makes. See
    // [`super::api::caster_conditions`].
    player: Query<&crate::world::session::WorldEntity, With<crate::world::session::LocalPlayer>>,
    // The time of day, the weather and the reputation list, which those checks
    // need and the caster's own snapshot does not carry. One param because
    // this system is at fifteen and three `Res` would not fit. See
    // [`super::api::Surroundings`].
    around: super::api::Surroundings,
) {
    // The pending item must not outlive the cursor it belongs to. Escape, a
    // right click and a fresh press all stand `SpellTargeting` down without
    // touching `PendingItemCast`, so it is cleared here instead of making those
    // paths a second writer. The cost is one comparison a frame.
    if !targeting.wants_item() {
        pending.clear();
    }
    // The two bindings differ in which of the three numberings the slot
    // arrived in (both are converted to the server's below, once) and in
    // whether the item is already worn, which decides the equip branch.
    let places = pressed
        .read()
        .filter_map(|press| match press.0 {
            crate::input::bindings::Binding::UseContainerItem { bag, slot } => Some(ItemPlace::Carried {
                bag,
                slot: usize::from(slot),
            }),
            crate::input::bindings::Binding::UseInventoryItem(slot) => Some(ItemPlace::Worn(slot)),
            _ => None,
        })
        // The action bar's uses, which name an entry that has to be looked up.
        // An entry the character no longer carries resolves to nothing and
        // does nothing. That is a bar slot holding a potion that has been
        // drunk.
        .chain(
            wanted
                .read()
                .filter_map(|UseCarriedItem(entry)| inventory.carried.find_entry(*entry)),
        )
        .collect::<Vec<_>>();
    for place in places {
        let (item, at, carried) = match place {
            ItemPlace::Carried { bag, slot } => (
                inventory.carried.container_item(bag, slot),
                items::server_container_slot(bag, slot),
                true,
            ),
            ItemPlace::Worn(slot) => (
                inventory.carried.inventory_slot(slot),
                items::server_inventory_slot(slot),
                false,
            ),
        };
        let (Some(item), Some((bag, slot))) = (item, at) else {
            continue;
        };
        // While the spell cursor is waiting for an item, the click chooses
        // that item and does not use it. 1.12 has no separate Lua function for
        // this: the bag square's `OnClick` is a plain
        // `UseContainerItem(bag, slot)` (`ContainerFrame.lua:596`), and the
        // client decides which of the two it is. Enchanting formulas, poisons
        // and sharpening stones work this way. See
        // `combat::action::SpellTargeting::wants_item`.
        //
        // This is tested before the merchant branch below: with a spell
        // waiting and a vendor open, clicking the character's own weapon must
        // enchant it and not sell it.
        if targeting.wants_item() {
            // The cursor has two possible sources, and they end in different
            // packets. A spell pressed on the bar answers with
            // `CMSG_CAST_SPELL`. An item whose own spell wants an item, a
            // sharpening stone or a poison, answers with `CMSG_USE_ITEM`
            // naming the stone, because the server reads the stone's charges
            // out of that packet. See [`PendingItemCast`].
            match pending.take() {
                Some(from) => {
                    if let Some(active) = session.active.as_ref() {
                        active.live.use_item(
                            from.bag,
                            from.slot,
                            Some(from.spell_index),
                            vale_protocol::play::spells::CastTarget::Item(item.guid),
                        );
                    }
                }
                None => {
                    opens.picked.write(super::action::SpellItemPicked { guid: item.guid });
                }
            }
            continue;
        }
        // With the bank open, the same click is a deposit, or a withdrawal
        // when the square is a bank square. It is one command; the server
        // chooses the direction from the source position, in
        // `WorldSession::bank_item`. It applies to a bag click only, as the
        // sale below does: the 1.12.1 client has no gesture that banks a worn
        // item. It is tested before the merchant because the two windows
        // cannot be open at once and the bank test is cheaper.
        if carried && bank.is_open() {
            if let Some(active) = session.active.as_ref() {
                active.live.bank_item(bag, slot);
            }
            continue;
        }
        // With a merchant window open, a bag right-click sells. It is the same
        // gesture with a different packet, as in the 1.12.1 client, where
        // `UseContainerItem` sells and there is no sell button. The item is
        // named by its object guid with a count of 0, which the server reads
        // as the whole stack. Nothing local changes until the update block
        // arrives, as for every other item command. A worn item is not sold
        // this way: the 1.12.1 client's gesture is bag-only.
        if carried && merchant.is_open() {
            if let (Some(active), Some(vendor)) = (session.active.as_ref(), merchant.guid()) {
                active.live.npc(vale_protocol::socket::session::NpcVerb::Sell {
                    vendor,
                    item: item.guid,
                    count: 0,
                });
            }
            continue;
        }
        let Some(template) = inventory.template(item.entry) else {
            continue;
        };
        // An item that begins a quest opens the quest page and does nothing
        // else. `ItemPrototype::StartQuest` is a quest id, and this is the
        // only code that acts on it. The tooltip prints `ITEM_STARTS_QUEST`
        // off the same field.
        //
        // The packet is `CMSG_QUESTGIVER_QUERY_QUEST` with the item's own guid
        // as the giver. `HandleQuestgiverQueryQuestOpcode` looks the guid up
        // through `TYPEMASK_CREATURE_GAMEOBJECT_PLAYER_ITEM`, so the server
        // treats an item as an ordinary quest giver, and the exchange that
        // follows (the details page, `Accept`, the log update) is the one
        // `interface::quest` already drives for an NPC. Accepting destroys the
        // item, and the server does that.
        //
        // This is tested before the equip and use branches. The 1.12.1 client
        // uses an equippable item whose `StartQuest` is set instead of
        // equipping it, so an equippable quest starter opens its page and is
        // not worn.
        if template.start_quest != 0 {
            if let Some(active) = session.active.as_ref() {
                active.live.quest(vale_protocol::socket::session::QuestVerb::Details {
                    guid: item.guid,
                    quest_id: template.start_quest,
                });
            }
            continue;
        }
        // Ammo in a bag is loaded, not worn. `INVTYPE_AMMO` is the one
        // equippable type `FindEquipSlot` has no case for, so the right-click
        // that equips a sword sends `CMSG_SET_AMMO` for a stack of arrows.
        // See [`vale_protocol::play::items::set_ammo_body`].
        if carried && template.inventory_type == vale_protocol::state::query::INVTYPE_AMMO {
            if let Some(active) = session.active.as_ref() {
                active.live.set_ammo(item.entry);
            }
            continue;
        }
        // A garment in a bag is worn, not used. See
        // [`vale_protocol::state::query::ItemInfo::is_equippable`], which
        // quotes the server-side refusal that requires the split. `None` here
        // makes the command send `CMSG_AUTOEQUIP_ITEM`.
        //
        // The three checks in the `else` apply only to an item that is used.
        // The 1.12.1 client makes them only when the inventory type is zero,
        // which is why they are inside the `else`.
        let spell_index = if carried && template.is_equippable() {
            None
        } else {
            // A wrapped item is unwrapped by the packet that opens a sack. The
            // two are told apart by a bit on the object, not by anything in
            // the prototype: `ITEM_FLAG_WRAPPER` with `ITEM_DYNFLAG_WRAPPED`
            // is a gift to open. Without the dynamic flag the same item is
            // blank paper, whose right-click raises a wrapping cursor this
            // client has no gesture for.
            //
            // The server answers by putting the real entry back on the object
            // and sending nothing else, so the unwrapping arrives as an
            // ordinary field change.
            if item.wrapped() {
                if let Some(active) = session.active.as_ref() {
                    active.live.open_item(bag, slot);
                }
                continue;
            }
            // A guild charter opens the petition window. The request names
            // the item, and the server answers with the signatures; see
            // [`crate::interface::petition`]. The tooltip prints
            // `<Right Click for Details>` for the same items.
            if template.flags & vale_protocol::state::query::item_flags::CHARTER != 0 {
                if let Some(active) = session.active.as_ref() {
                    active.live.petition(
                        vale_protocol::socket::session::PetitionVerb::ShowSignatures(item.guid),
                    );
                }
                continue;
            }
            // A readable item (a book, a note, a scroll) opens its page and
            // does nothing else. `ItemPrototype::PageText` is the page id, and
            // the client opens the window: nothing is sent before the page
            // query. The tooltip prints `<Right Click to Read>` for the same
            // items. See [`crate::interface::pagetext`].
            if template.is_readable() {
                opens.read_page.write(crate::interface::pagetext::ReadPage {
                    title: template.name.clone(),
                    page_id: template.page_text,
                    material: template.page_material,
                    guid: None,
                });
                continue;
            }
            // A sack, a pouch or a strongbox is opened, not used:
            // `ITEM_FLAG_LOOTABLE`, `CMSG_OPEN_ITEM`, and the answer is an
            // ordinary `SMSG_LOOT_RESPONSE` whose guid is the item's. Item
            // 17962, the Blue Sack of Gems, is an example: it has no on-use
            // spell and no inventory type, so without this branch every other
            // branch falls through to `continue` and the right-click does
            // nothing.
            //
            // This is tested before the search for an on-use spell, as the
            // 1.12.1 client does. The order matters: a lootable item carrying
            // an on-use spell would otherwise be cast instead of opened.
            //
            // The lock is not checked here, as in the 1.12.1 client. A
            // strongbox that has not been picked is sent and comes back
            // `EQUIP_ERR_ITEM_LOCKED`, which shows the player a message. The
            // lock does gate the tooltip's `<Right Click to Open>` line; see
            // `ItemInfo::says_right_click_to_open`.
            if template.is_openable() {
                if let Some(active) = session.active.as_ref() {
                    active.live.open_item(bag, slot);
                }
                continue;
            }
            match template.on_use_spell() {
                Some(index) => Some(index),
                None => continue,
            }
        };
        // The use is aimed the way a cast is, which for an item is usually at
        // nobody: `SpellCastTargets` with an empty mask lets the server fill
        // the targeting in from the spell, and that is correct for every
        // potion and food. A selection is passed only when there is one, so a
        // bandage used with a friend selected lands on the friend. That is the
        // same `TARGET_FLAG_UNIT` block `CMSG_CAST_SPELL` sends.
        let aim = match selection.guid.filter(|_| spell_index.is_some()) {
            Some(guid) => vale_protocol::play::spells::CastTarget::Unit(guid),
            None => vale_protocol::play::spells::CastTarget::SelfImplicit,
        };
        // If the item's spell wants an item as its target, this click raises
        // the cursor and sends nothing yet. A sharpening stone right-clicked
        // in the bag waits for a weapon; sending it now fails with "Invalid
        // target", because `Spell::CheckCast` has no item to enchant. The
        // stone's place is remembered so the answer can name it. See
        // [`PendingItemCast`] and the branch above.
        if let Some((index, spell)) = spell_index.zip(
            spell_index
                .and_then(|index| template.spells.get(usize::from(index)))
                .map(|s| s.spell_id)
                .filter(|id| *id != 0)
                .and_then(|id| {
                    let tables = assets.display_tables().ok()?;
                    tables.spellbook()?.info(id)
                }),
        ) {
            if matches!(
                vale_assets::tables::spellbook::resolve_aim(&spell, None, None, false, None),
                vale_assets::tables::spellbook::CastAim::WantsItem
            ) {
                pending.arm(PendingItem { bag, slot, spell_index: index });
                opens.begin.write(BeginItemTargeting(spell.id));
                continue;
            }
        }
        // The spell is resolved before the send, because it decides whether
        // the packet is sent at all as well as starting the local cooldown.
        //
        // A `CMSG_AUTOEQUIP_ITEM` is not a cast, and an item whose spell the
        // catalogue does not carry gets no cast bar, since the bar would have
        // no name. Both answer `None` here and fall through to the plain send.
        let info = spell_index
            .and_then(|index| template.spells.get(usize::from(index)))
            .map(|spell| spell.spell_id)
            .filter(|id| *id != 0)
            .and_then(|id| {
                let tables = assets.display_tables().ok()?;
                tables.spellbook()?.info(id)
            });
        // A press while the item is still on cooldown is refused here, with
        // the client's own message, as `cast_known_spell` does for a spell.
        // The server does not answer a too-early `CMSG_USE_ITEM`, so without
        // this check a potion pressed repeatedly on cooldown sends packets and
        // shows no feedback.
        if let Some(info) = info.as_ref() {
            if !cooldowns.ready(info) {
                errors.key(
                    vale_assets::tables::spellbook::failure_override(
                        "SPELL_FAILED_NOT_READY",
                        0,
                    )
                    .unwrap_or("SPELL_FAILED_NOT_READY"),
                );
                continue;
            }
            // Every other local refusal `cast_known_spell` makes is made here
            // through the same two functions. This is where a mount is refused
            // while moving: every mount in 1.12 is an item, so a check made
            // only in `cast_known_spell` does not cover one. `distance` and
            // the target's state are not measured here, because an item's aim
            // is the selection or nobody and the server range-checks it. The
            // checks added are on the caster's own state: dead, moving, and
            // the cost.
            if let Ok(me) = player.single() {
                let conditions =
                    super::api::caster_conditions(me, info, None, None, around.cast_world());
                if let Some(key) = vale_assets::tables::spellbook::check_cast(info, &conditions) {
                    errors.key(
                        vale_assets::tables::spellbook::failure_override(key, info.power_type)
                            .unwrap_or(key),
                    );
                    events.cast_failed.write(super::events::SpellcastFailed);
                    continue;
                }
            }
        }
        if let Some(active) = session.active.as_ref() {
            active.live.use_item(bag, slot, spell_index, aim);
        }
        let Some(info) = info else {
            continue;
        };
        super::action::begin_item_cast(&info, &mut cooldowns, &mut events);
    }
}

/// Copy the inventory out of the session when the latch changes, and raise the
/// events for what changed.
fn rebuild(
    session: Res<Session>,
    mut inventory: ResMut<Inventory>,
    mut bags: MessageWriter<BagUpdate>,
    mut worn: MessageWriter<UnitInventoryChanged>,
    mut coins: MessageWriter<PlayerMoney>,
    // The bank's two events, which are the only way its window learns that
    // something moved: the bank is update fields and never a packet. See
    // [`super::bank`].
    mut bank: MessageWriter<PlayerbankslotsChanged>,
    mut bank_slots: MessageWriter<PlayerbankbagslotsChanged>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // The lock is taken once, to read the version, the money, and (if anything
    // changed) the slots and the templates that have arrived. Everything after
    // this block is local.
    let (version, cache_size, money, ammo, moved) = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        let version = world.inventory_version;
        // The second half of the latch is over-sensitive by design.
        // `world.items` is keyed by entry and holds every template the session
        // has resolved, another player's breastplate as well as this
        // character's linen, so a reply about somebody else's gear changes
        // this count and rebuilds the bags. That costs one unneeded inventory
        // read. Avoiding it would need the set of this character's entries
        // before reading them, which is what the rebuild computes. The
        // opposite error, missing a reply about a carried entry, would leave a
        // bag blank.
        let templates_seen = world.items.len();
        // The money is read on every frame, outside the latch.
        // `PLAYER_FIELD_COINAGE` is field 1176, which is outside both of the
        // slot runs `touches_inventory` tests, because spending money moves
        // nothing a bag square draws. Paying a trainer therefore changes no
        // `inventory_version` and adds no template, and money read only when
        // the latch changes would stay stale until an item moved.
        let money = world
            .player()
            .and_then(|p| p.field(vale_protocol::state::fields::player::COINAGE))
            .unwrap_or(0);
        let ammo = world
            .player()
            .and_then(|p| p.field(vale_protocol::state::fields::player::AMMO_ID))
            .unwrap_or(0);
        let moved = (inventory.built_from != Some(version)
            || inventory.templates_seen != templates_seen)
            .then(|| {
                let carried = Carried::read(&world);
                // Only the entries this character carries are copied, not the
                // whole cache. In a city that map holds every distinct item
                // worn by everyone in sight, and cloning it once per arriving
                // template would be quadratic in the number of players
                // nearby.
                let templates: HashMap<u32, ItemInfo> = carried
                    .entries()
                    .into_iter()
                    // The loaded ammo's entry is added, because it may no
                    // longer be in a bag: the square still names it with a
                    // count of 0.
                    .chain(std::iter::once(ammo))
                    .filter_map(|entry| Some((entry, world.items.get(&entry)?.clone())))
                    .collect();
                (carried, templates)
            });
        (version, templates_seen, money, ammo, moved)
    };

    // The money is assigned only when it changed, for Bevy's change detection.
    // `ResMut` marks the resource changed on any mutable deref, so writing the
    // same value back every frame made `Inventory::is_changed()` true on every
    // frame, and two systems depend on it. `merchant::templates_landed` raised
    // `MERCHANT_UPDATE` sixty times a second at an open vendor (which redraws
    // the buyback tab as well as the item list), and `price_repairs` re-read
    // two DBCs over forty slots on each of those frames.
    let money_changed = inventory.money != money;
    if money_changed {
        inventory.money = money;
    }
    // The ammo slot redraws on the same event the worn slots do:
    // `CharacterAmmoSlot` is a `PaperDollItemSlotButton`, and
    // `UNIT_INVENTORY_CHANGED` is the only refresh it registers.
    let ammo_changed = inventory.ammo != ammo;
    if ammo_changed {
        inventory.ammo = ammo;
    }
    // Money that changed with no inventory change still raises its event.
    // `MoneyFrame_OnEvent` and `ClassTrainerMoneyFrame` both depend on it:
    // `SmallMoneyFrameTemplate` registers `PLAYER_MONEY` and re-reads
    // `GetMoney()` on it and on nothing else.
    let Some((carried, templates)) = moved else {
        if money_changed {
            coins.write(PlayerMoney);
        }
        if ammo_changed {
            worn.write(UnitInventoryChanged(super::api::UnitId::Player));
        }
        return;
    };

    // What changed, in terms of the events the interface listens for.
    //
    // The four bag items count as equipment, because they are inventory slots
    // 20..23 and `CharacterBag0Slot` is a paper-doll button like any other:
    // swapping a bag has to redraw its button as well as its contents.
    let equipment_changed = inventory.carried.equipped != carried.equipped
        || worn_bags(&inventory.carried) != worn_bags(&carried);
    let changed_bags: Vec<i32> = CONTAINERS
        .iter()
        .copied()
        .filter(|bag| inventory.carried.container(*bag) != carried.container(*bag))
        .collect();
    // The bank has its own events. Its squares and the six bag items redraw on
    // `PLAYERBANKSLOTS_CHANGED`, which every bank button registers and no bag
    // frame does. A bag in the bank is a container frame like any other and
    // takes `BAG_UPDATE` with its id, 5..10. The number of bought bag slots is
    // a third event, read from a byte no slot run covers.
    let bank_moved = first_bank_change(&inventory.carried, &carried);
    let changed_bank_bags: Vec<i32> = items::BANK_CONTAINERS_FOR_REPORT
        .iter()
        .copied()
        .filter(|bag| *bag != items::BANK_CONTAINER)
        .filter(|bag| inventory.carried.container(*bag) != carried.container(*bag))
        .collect();
    let bank_slots_changed = inventory.carried.bank_bag_slots != carried.bank_bag_slots;
    // A template arriving counts as a change to every bag holding that entry:
    // the slot is unchanged, but what can be drawn of it is not.
    //
    // The test is "is there an entry resolved now that was not before" and not
    // a count. A reply arriving on the frame a stack is used up leaves the
    // count unchanged, and in that case a bag would keep an icon with no name.
    // The comparison is against this character's resolved set, not against the
    // latch above, which counts the whole cache.
    let templates_grew = templates
        .keys()
        .any(|entry| !inventory.templates.contains_key(entry));

    inventory.built_from = Some(version);
    inventory.templates_seen = cache_size;
    inventory.carried = carried;
    inventory.templates = templates;

    // The event is raised for everything the character holds, not only what is
    // worn. See [`UnitInventoryChanged`], which gives the reason.
    // `ActionButton_Update` is the only function that re-reads
    // `GetActionCount`, and this is the only event it registers that a bag
    // change raises. When the condition tested equipment only, a stack of
    // potions on the bar kept its old count.
    if equipment_changed || templates_grew || !changed_bags.is_empty() || ammo_changed {
        worn.write(UnitInventoryChanged(super::api::UnitId::Player));
    }
    for bag in if templates_grew {
        CONTAINERS.to_vec()
    } else {
        changed_bags
    } {
        bags.write(BagUpdate(bag));
    }
    // The bank's events follow the same rule: a template arriving is a change
    // to every square that can now be drawn.
    if let Some(slot) = bank_moved.or_else(|| templates_grew.then_some(items::FIRST_BANK_INVENTORY_SLOT)) {
        bank.write(PlayerbankslotsChanged(slot));
    }
    for bag in if templates_grew {
        items::BANK_CONTAINERS_FOR_REPORT
            .iter()
            .copied()
            .filter(|bag| *bag != items::BANK_CONTAINER)
            .collect()
    } else {
        changed_bank_bags
    } {
        bags.write(BagUpdate(bag));
    }
    if bank_slots_changed {
        bank_slots.write(PlayerbankbagslotsChanged);
    }
    if money_changed {
        coins.write(PlayerMoney);
    }
}

/// The inventory slot id of the first bank square that differs: 40..63 for the
/// squares and 64..69 for the bag slots. This is the argument of
/// `PLAYERBANKSLOTS_CHANGED`. `None` when the bank's own thirty slots are
/// unchanged. The contents of a bag in the bank are not compared here; they
/// are reported by that bag's `BAG_UPDATE`.
fn first_bank_change(before: &Carried, after: &Carried) -> Option<u32> {
    let square = before
        .bank
        .iter()
        .zip(&after.bank)
        .position(|(a, b)| a != b)
        .map(|index| items::FIRST_BANK_INVENTORY_SLOT + index as u32);
    square.or_else(|| {
        before
            .bank_bags
            .iter()
            .zip(&after.bank_bags)
            .position(|(a, b)| a.as_ref().map(|bag| &bag.item) != b.as_ref().map(|bag| &bag.item))
            .map(|index| items::FIRST_BANK_BAG_INVENTORY_SLOT + index as u32)
    })
}

/// The item in each of the four bag slots: the bag itself, which is worn, as
/// opposed to its contents.
fn worn_bags(carried: &Carried) -> [Option<ItemSlot>; items::BAG_SLOTS] {
    let mut out = [None; items::BAG_SLOTS];
    for (slot, bag) in out.iter_mut().zip(carried.bags.iter()) {
        *slot = bag.as_ref().map(|bag| bag.item);
    }
    out
}

/// Clear the last character's inventory. [`super`]'s module note explains why
/// each module resets its own state.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut inventory: ResMut<Inventory>,
) {
    if leaving.read().next().is_some() {
        *inventory = Inventory::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::events::UiErrorMessage;
    use crate::interface::messages::UiStrings;
    use vale_protocol::play::items::InventoryFailure;
    use bevy::ecs::message::Messages as MessageQueue;
    use std::sync::Arc;

    /// Push one refusal through the real system and collect the line it put on
    /// the error frame.
    fn shown(table: &str, failure: InventoryFailure) -> Vec<String> {
        let mut app = App::new();
        app.insert_resource(UiStrings(Some(Arc::new(
            vale_assets::interface::strings::Strings::parse(table.as_bytes()),
        ))));
        app.add_message::<ItemRefused>()
            .add_systems(Update, refusals);
        crate::interface::messages::Announce::register(&mut app);
        app.world_mut().write_message(ItemRefused(failure));
        app.update();
        let queue = app.world().resource::<MessageQueue<UiErrorMessage>>();
        let mut cursor = queue.get_cursor();
        cursor.read(queue).map(|m| m.0.clone()).collect()
    }

    /// A level refusal, from the packet to the line on the error frame. A
    /// potion the character is too low for is refused by `Player::CanUseItem`
    /// before a spell is prepared, so this packet is the only thing that gives
    /// the reason. The level in it is the item's, filled into the
    /// `GlobalStrings.lua` sentence.
    #[test]
    fn a_level_refusal_says_which_level_in_the_games_own_words() {
        let table = r#"
            ERR_CANT_EQUIP_LEVEL_I = "You must reach level %d to use that item.";
            ERR_INV_FULL = "Inventory is full.";
        "#;
        assert_eq!(
            shown(
                table,
                InventoryFailure {
                    code: vale_protocol::play::items::EQUIP_ERR_CANT_EQUIP_LEVEL_I,
                    required_level: 45,
                    ..InventoryFailure::default()
                }
            ),
            vec!["You must reach level 45 to use that item."]
        );
        // Every other code shows the key's string with no argument.
        assert_eq!(
            shown(
                table,
                InventoryFailure {
                    code: 50,
                    ..InventoryFailure::default()
                }
            ),
            vec!["Inventory is full."]
        );
    }

    /// Two codes show no line, for different reasons. `EQUIP_ERR_OK` is the
    /// success acknowledgement. The grey-item release that
    /// `HandleUseItemOpcode` sends before the real reason names a key
    /// `GlobalStrings.lua` does not carry.
    #[test]
    fn the_two_codes_that_are_not_messages_say_nothing() {
        let table = r#"ERR_INV_FULL = "Inventory is full.";"#;
        assert!(shown(table, InventoryFailure::default()).is_empty());
        assert!(shown(
            table,
            InventoryFailure {
                code: 59,
                ..InventoryFailure::default()
            }
        )
        .is_empty());
    }
}
