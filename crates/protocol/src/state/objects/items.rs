//! What the character carries, read off the object manager.
//!
//! The item layout, the slot numberings and the packets are in
//! [`crate::play::items`]. This file walks the three layers of update fields
//! (the player names a bag, the bag names an item, the item names an entry)
//! and copies them into [`Inventory`], [`Bag`] and [`ItemSlot`].

use crate::play::items::{
    Bag, BuybackSlot, Inventory, ItemEnchant, ItemSlot, BACKPACK_SLOTS, BANK_SLOTS,
    BUYBACK_SLOTS, BUYBACK_SLOT_START, CONTAINER_SLOTS, EQUIPMENT_SLOTS, INVENTORY_SLOTS,
    KEYRING_SLOTS,
};
use crate::state::fields;
use crate::state::objects::{Entity, ObjectManager};
use crate::state::update::ObjectType;

/// A GUID field is two update fields wide, little end first.
const GUID_STRIDE: u16 = 2;

impl ItemSlot {
    /// Read one item object. Returns `None` for a GUID that names no known
    /// object. That is a normal case, not an error, because a bag's contents
    /// stream in one block behind the bag itself.
    pub(crate) fn read(world: &ObjectManager, guid: u64) -> Option<ItemSlot> {
        if guid == 0 {
            return None;
        }
        let entity = world.get(guid)?;
        Some(ItemSlot {
            guid,
            entry: entity.entry().unwrap_or(0),
            // `max(1)` is required. `_SetCreateBits` omits every zero field,
            // and a single non-stacking item is written with a stack count of
            // 1. A values block that has never set the field leaves it absent,
            // and a zero count draws no number and makes any division by the
            // count useless. The same omission of zero fields is what made a
            // corpse read as alive.
            count: entity.field(fields::item::STACK_COUNT).unwrap_or(1).max(1),
            durability: entity.field(fields::item::DURABILITY).unwrap_or(0),
            max_durability: entity.field(fields::item::MAXDURABILITY).unwrap_or(0),
            flags: entity.field(fields::item::FLAGS).unwrap_or(0),
            enchantments: std::array::from_fn(|slot| {
                let base = fields::item::ENCHANTMENT + (slot as u16) * 3;
                ItemEnchant {
                    id: entity.field(base).unwrap_or(0) as i32,
                    duration_ms: entity.field(base + 1).unwrap_or(0),
                    charges: entity.field(base + 2).unwrap_or(0),
                    expires: world.item_timers.enchantment(guid, slot as u8),
                }
            }),
            random_property: entity.field(fields::item::RANDOM_PROPERTIES_ID).unwrap_or(0) as i32,
            spell_charges: std::array::from_fn(|slot| {
                entity
                    .field(fields::item::SPELL_CHARGES + slot as u16)
                    .unwrap_or(0) as i32
            }),
            creator: guid_field(entity, fields::item::CREATOR),
            gift_creator: guid_field(entity, fields::item::GIFTCREATOR),
            text_id: entity.field(fields::item::ITEM_TEXT_ID).unwrap_or(0),
            expires: world.item_timers.item(guid),
        })
    }
}

/// A two-field guid, low word first, with 0 for an absent half.
fn guid_field(entity: &Entity, index: u16) -> u64 {
    let low = entity.field(index).unwrap_or(0);
    let high = entity.field(index + 1).unwrap_or(0);
    u64::from(low) | (u64::from(high) << 32)
}

impl Inventory {
    /// Walk the manager and copy the whole inventory out.
    ///
    /// Returns the default for a world with no local player, which is what a
    /// character screen and a logged-out session both are.
    pub fn read(world: &ObjectManager) -> Inventory {
        let Some(player) = world.player() else {
            return Inventory::default();
        };
        let mut out = Inventory {
            backpack: vec![None; BACKPACK_SLOTS],
            keyring: vec![None; KEYRING_SLOTS],
            bank: vec![None; BANK_SLOTS],
            ..Default::default()
        };

        for (slot, worn) in out.equipped.iter_mut().enumerate() {
            *worn = ItemSlot::read(world, inventory_guid(player, slot));
        }
        for (index, bag) in out.bags.iter_mut().enumerate() {
            let guid = inventory_guid(player, EQUIPMENT_SLOTS + index);
            *bag = read_bag(world, guid);
        }
        for (slot, held) in out.backpack.iter_mut().enumerate() {
            let guid = guid_at(player, fields::player::PACK_SLOT_1, slot);
            *held = ItemSlot::read(world, guid);
        }
        for (slot, held) in out.keyring.iter_mut().enumerate() {
            let guid = guid_at(player, fields::player::KEYRING_SLOT_1, slot);
            *held = ItemSlot::read(world, guid);
        }
        for (slot, held) in out.bank.iter_mut().enumerate() {
            let guid = guid_at(player, fields::player::BANK_SLOT_1, slot);
            *held = ItemSlot::read(world, guid);
        }
        for (index, bag) in out.bank_bags.iter_mut().enumerate() {
            let guid = guid_at(player, fields::player::BANKBAG_SLOT_1, index);
            *bag = read_bag(world, guid);
        }
        // `GetBankBagSlotCount`: `GetByteValue(PLAYER_BYTES_2, 2)`.
        out.bank_bag_slots = player
            .field(fields::player::BYTES_2)
            .map_or(0, |bytes| ((bytes >> 16) & 0xff) as u8);
        out.buyback = read_buyback(world, player);
        out
    }
}

/// A GUID out of a run of GUID fields.
fn guid_at(entity: &Entity, base: u16, index: usize) -> u64 {
    let index = base + (index as u16) * GUID_STRIDE;
    let low = entity.field(index).unwrap_or(0) as u64;
    let high = entity.field(index + 1).unwrap_or(0) as u64;
    low | (high << 32)
}

/// One worn slot's item guid, read without walking the bags.
///
/// [`Inventory::read`] answers the same question but reads everything the
/// character carries to do it. That suits a panel and is too much work for a
/// query made every time a spell is pressed. The one caller is the aiming
/// rule's main hand; see `vale_assets::tables::spellbook::CastAim::Item`, and
/// [`EQUIPMENT_SLOTS`](crate::play::items::EQUIPMENT_SLOTS) for what a slot index means.
///
/// `None` for an empty slot, which is a zero guid on the wire.
pub fn equipped_guid(player: &Entity, slot: usize) -> Option<u64> {
    if slot >= EQUIPMENT_SLOTS {
        return None;
    }
    match inventory_guid(player, slot) {
        0 => None,
        guid => Some(guid),
    }
}

/// One slot's item guid from `PLAYER_FIELD_INV_SLOT_HEAD`, which covers the
/// worn slots and the bag slots in one run.
fn inventory_guid(player: &Entity, slot: usize) -> u64 {
    debug_assert!(slot < INVENTORY_SLOTS);
    guid_at(player, fields::player::INV_SLOT_HEAD, slot)
}

/// The vendor's twelve buyback slots, compacted and sorted in the 1.12.1
/// client's order.
///
/// The data is three parallel runs on the player's own object, with no
/// packet: `PLAYER_FIELD_VENDORBUYBACK_SLOT_1` (12 guids), `..._BUYBACK_PRICE_1`
/// and `..._BUYBACK_TIMESTAMP_1` (12 each). A sale does not destroy the item;
/// it re-parents it, so the object the guid names is still in the world and
/// still carries its entry and its stack count.
///
/// Each of these rules matches the 1.12.1 client and changes what the panel
/// shows:
///
/// * A slot is occupied when its price is non-zero, not when its guid is.
///   Both are cleared together, so the two agree in practice. The client
///   tests the price, and a zero-price row could not be bought anyway.
/// * The list is compacted, so `GetBuybackItemInfo(1)` is the first occupied
///   slot and not slot 69. Below twelve items the two coincide, so an
///   implementation that skipped this step would pass a test with fewer than
///   twelve items.
/// * The list is sorted by timestamp, ascending. This makes the newest sale
///   the last entry, which is what `MerchantFrame.lua` shows on its first
///   tab: `GetBuybackItemInfo(GetNumBuybackItems())`. Once the twelve are full
///   the server replaces the oldest slot (`Player::AddItemToBuyBackSlot`), so
///   without the sort the most recent sale would appear in the middle of the
///   list and the first tab would show a different item.
fn read_buyback(world: &ObjectManager, player: &Entity) -> Vec<BuybackSlot> {
    let mut out: Vec<BuybackSlot> = (0..BUYBACK_SLOTS)
        .filter_map(|slot| {
            let price = player.field(fields::player::BUYBACK_PRICE_1 + slot as u16)?;
            if price == 0 {
                return None;
            }
            let item = ItemSlot::read(
                world,
                guid_at(player, fields::player::VENDORBUYBACK_SLOT_1, slot),
            )?;
            Some(BuybackSlot {
                item,
                price,
                sold_at: player
                    .field(fields::player::BUYBACK_TIMESTAMP_1 + slot as u16)
                    .unwrap_or(0),
                wire_slot: u32::from(BUYBACK_SLOT_START) + slot as u32,
            })
        })
        .collect();
    out.sort_by_key(|slot| slot.sold_at);
    out
}

/// Read a container object and everything in it.
///
/// The number of slots comes from the container's own field. A container that
/// states no slot count is still a bag: its create block has not arrived in
/// full. Answering "no slots" for it matches the 1.12.1 client
/// (`ContainerFrame_OnShow` compares `> 0`).
fn read_bag(world: &ObjectManager, guid: u64) -> Option<Bag> {
    let item = ItemSlot::read(world, guid)?;
    let entity = world.get(guid)?;
    // An object that is not a `Container` is not a bag: the field indices
    // below mean something else on every other object type, and the per-type
    // field modules exist to keep those overlapping indices apart.
    if entity.object_type != Some(ObjectType::Container) {
        return Some(Bag {
            item,
            slots: Vec::new(),
        });
    }
    let count = entity
        .field(fields::container::NUM_SLOTS)
        .unwrap_or(0)
        .min(CONTAINER_SLOTS as u32) as usize;
    let slots = (0..count)
        .map(|slot| ItemSlot::read(world, guid_at(entity, fields::container::SLOT_1, slot)))
        .collect();
    Some(Bag { item, slots })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::items::*;
    use crate::state::update::{ObjectUpdate, UpdateBlock, ValuesUpdate};

    /// Build a create block for one object with the given fields.
    fn create(guid: u64, kind: ObjectType, fields: &[(u16, u32)]) -> UpdateBlock {
        UpdateBlock::Create {
            guid,
            object_type: Some(kind),
            movement: Default::default(),
            values: ValuesUpdate {
                fields: fields.to_vec(),
            },
            is_new: false,
        }
    }

    /// A player carrying a bag with one stack in it, plus a worn helmet and one
    /// item in the backpack. It covers every level of the three-object chain,
    /// which is what this module reads.
    fn world_with_a_bag() -> ObjectManager {
        const PLAYER: u64 = 0x1;
        const HELMET: u64 = 0x10;
        const BAG: u64 = 0x20;
        const POTION: u64 = 0x21;
        const LINEN: u64 = 0x30;

        let mut player_fields = vec![];
        // Worn slot 0 (head) -> the helmet.
        player_fields.push((fields::player::INV_SLOT_HEAD, HELMET as u32));
        player_fields.push((fields::player::INV_SLOT_HEAD + 1, 0));
        // Bag slot 0 -> the bag. Field index 19 of the same run.
        let bag_field = fields::player::INV_SLOT_HEAD + (EQUIPMENT_SLOTS as u16) * GUID_STRIDE;
        player_fields.push((bag_field, BAG as u32));
        player_fields.push((bag_field + 1, 0));
        // Backpack slot 0 -> the linen.
        player_fields.push((fields::player::PACK_SLOT_1, LINEN as u32));
        player_fields.push((fields::player::PACK_SLOT_1 + 1, 0));

        let mut world = ObjectManager::new();
        world.apply(&ObjectUpdate {
            has_transport: false,
            warning: None,
            blocks: vec![
                create(PLAYER, ObjectType::Player, &player_fields),
                create(
                    HELMET,
                    ObjectType::Item,
                    &[
                        (fields::object::ENTRY, 12640),
                        (fields::item::DURABILITY, 55),
                        (fields::item::MAXDURABILITY, 60),
                    ],
                ),
                create(
                    BAG,
                    ObjectType::Container,
                    &[
                        (fields::object::ENTRY, 4500),
                        (fields::container::NUM_SLOTS, 6),
                        (fields::container::SLOT_1 + 2 * GUID_STRIDE, POTION as u32),
                    ],
                ),
                create(
                    POTION,
                    ObjectType::Item,
                    &[(fields::object::ENTRY, 858), (fields::item::STACK_COUNT, 5)],
                ),
                create(
                    LINEN,
                    ObjectType::Item,
                    &[(fields::object::ENTRY, 2589), (fields::item::STACK_COUNT, 20)],
                ),
            ],
        });
        // `apply` only learns the player GUID from a `SELF` movement flag; the
        // fixture sets it directly because none of this needs a position.
        world.player_guid = Some(PLAYER);
        world
    }

    /// The three-object chain: the player names the bag, the bag names the
    /// stack, the stack names the entry.
    #[test]
    fn a_bag_and_its_contents_are_read_through_three_objects() {
        let inventory = Inventory::read(&world_with_a_bag());

        let bag = inventory.bags[0].as_ref().expect("the bag slot is filled");
        assert_eq!(bag.item.entry, 4500, "the bag is itself an item");
        assert_eq!(bag.len(), 6, "CONTAINER_FIELD_NUM_SLOTS");

        // The potion is in the third slot, and the interface counts from one.
        let potion = inventory.container_item(1, 3).expect("the third slot");
        assert_eq!(potion.entry, 858);
        assert_eq!(potion.count, 5);
        assert!(inventory.container_item(1, 1).is_none(), "slot one is empty");
        assert!(inventory.container_item(1, 7).is_none(), "past the end");
    }

    /// The backpack is a field block rather than an object, and so is the key
    /// ring. Neither has a container object to follow.
    #[test]
    fn the_backpack_lives_in_the_players_own_fields() {
        let inventory = Inventory::read(&world_with_a_bag());
        assert_eq!(inventory.container(0).map(<[_]>::len), Some(BACKPACK_SLOTS));
        let linen = inventory.container_item(0, 1).expect("the first slot");
        assert_eq!(linen.entry, 2589);
        assert_eq!(linen.count, 20);
        assert_eq!(inventory.container(KEYRING_CONTAINER).map(<[_]>::len), Some(KEYRING_SLOTS));
    }

    /// The two numberings. Bag id 1 is inventory slot 20 is field 19, and
    /// getting any of the three wrong reads a neighbouring slot.
    #[test]
    fn the_interfaces_slot_ids_and_the_field_indices_are_crossed_here() {
        let inventory = Inventory::read(&world_with_a_bag());
        assert_eq!(inventory.inventory_slot(1).map(|i| i.entry), Some(12640), "head");
        assert_eq!(inventory.inventory_slot(20).map(|i| i.entry), Some(4500), "bag 1");
        assert_eq!(bag_id_of_inventory_slot(20), Some(1));
        assert_eq!(bag_id_of_inventory_slot(23), Some(4));
        assert_eq!(bag_id_of_inventory_slot(19), None, "19 is the tabard, not a bag");
        assert_eq!(inventory_slot_of_bag_id(1), Some(20));
        assert_eq!(inventory_slot_of_bag_id(0), None, "the backpack is not worn");
    }

    /// A count is a sum over everything carried, which is what `GetItemCount`
    /// answers. A stack whose create block stated no count counts as one.
    #[test]
    fn a_count_sums_every_stack_and_an_absent_count_is_one() {
        let inventory = Inventory::read(&world_with_a_bag());
        assert_eq!(inventory.count_of(858), 5);
        assert_eq!(inventory.count_of(2589), 20);
        assert_eq!(inventory.count_of(12640), 1, "the helmet stated no stack count");
        assert_eq!(inventory.count_of(1), 0);
    }

    /// Durability, and the broken test the paper doll paints a slot red on.
    #[test]
    fn durability_comes_off_the_item_object() {
        let inventory = Inventory::read(&world_with_a_bag());
        let helmet = inventory.inventory_slot(1).expect("worn");
        assert_eq!((helmet.durability, helmet.max_durability), (55, 60));
        assert!(!helmet.broken());
        assert!(
            !ItemSlot::default().broken(),
            "a thing with no durability at all is not broken"
        );
        assert!(ItemSlot {
            max_durability: 60,
            ..Default::default()
        }
        .broken());
    }

    /// A world with no player answers the empty inventory rather than
    /// panicking. That is the state at a character screen, and in every frame
    /// before the first update block.
    #[test]
    fn no_player_is_an_empty_inventory() {
        let inventory = Inventory::read(&ObjectManager::new());
        assert_eq!(inventory, Inventory::default());
        // The backpack is still a container, with no slots, which is the size
        // `ToggleBackpack` refuses to open on (`size > 0`). `None` here would
        // mean "there is no backpack", which is never true of a character.
        assert_eq!(inventory.container(0).map(<[_]>::len), Some(0));
        assert!(inventory.container(1).is_none(), "and no bag in slot one");
        assert!(inventory.entries().is_empty());
    }

    /// The inventory version (the latch), which is the only signal an
    /// inventory change produces.
    ///
    /// A stack count changing must increment it and a position change must
    /// not. The local player's block carries both, and a version that changed
    /// on every step would rebuild the bags sixty times a second and signal
    /// nothing useful.
    #[test]
    fn the_version_moves_for_an_item_and_not_for_a_step() {
        let mut world = world_with_a_bag();
        // Without this line the player half of the test passes vacuously.
        // `touches_inventory` checks the entity's own `is_self`, which only a
        // `SELF` movement flag sets. A fixture that only writes `player_guid`
        // makes every assertion below about the player's block pass for the
        // wrong reason, including the two about health and coinage.
        world.get_mut(0x1).expect("the fixture's player").is_self = true;
        let before = world.inventory_version;

        world.apply(&ObjectUpdate {
            has_transport: false,
            warning: None,
            blocks: vec![UpdateBlock::Values {
                guid: 0x21,
                values: ValuesUpdate {
                    fields: vec![(fields::item::STACK_COUNT, 4)],
                },
            }],
        });
        assert_eq!(world.inventory_version, before + 1, "a stack was used");
        assert_eq!(Inventory::read(&world).container_item(1, 3).unwrap().count, 4);

        let after = world.inventory_version;
        world.apply(&ObjectUpdate {
            has_transport: false,
            warning: None,
            blocks: vec![UpdateBlock::Values {
                guid: 0x1,
                values: ValuesUpdate {
                    fields: vec![(fields::unit::HEALTH, 900)],
                },
            }],
        });
        assert_eq!(world.inventory_version, after, "health is not inventory");

        // The coinage is not inventory either, which is why money has its own
        // latch one crate up. `PLAYER_FIELD_COINAGE` sits past the slot runs,
        // and paying a trainer changes nothing a bag square draws, so anything
        // watching only this version never learns that gold was spent. See
        // `vale_client::game::items`: reading the coinage under this latch
        // instead of its own made the gold display update only when an item
        // moved.
        world.apply(&ObjectUpdate {
            has_transport: false,
            warning: None,
            blocks: vec![UpdateBlock::Values {
                guid: 0x1,
                values: ValuesUpdate {
                    fields: vec![(fields::player::COINAGE, 1234)],
                },
            }],
        });
        assert_eq!(world.inventory_version, after, "the purse is not inventory");
    }

    /// A destroyed item clears its slot on the packet that destroys it.
    ///
    /// `SMSG_DESTROY_OBJECT` is a bare guid handled outside `apply`, so the
    /// latch that the `OUT_OF_RANGE` case sets was never reached, and items are
    /// not in the grid, so no out-of-range block ever names one. The version
    /// did not change on a destroy, and the client's snapshot kept the old row
    /// until another field changed: a stack merged into another showed in both
    /// squares until the next bag update.
    ///
    /// The two assertions test the two parts: the version changes, and the
    /// read taken after it is empty.
    #[test]
    fn destroying_an_item_moves_the_version_and_empties_the_square() {
        let mut world = world_with_a_bag();
        assert_eq!(Inventory::read(&world).container_item(0, 1).unwrap().entry, 2589);
        let before = world.inventory_version;

        // All that `world::destroy` does: one guid, no block, no fields.
        world.remove(0x30);

        assert_eq!(world.inventory_version, before + 1, "the destroy is a change");
        assert!(
            Inventory::read(&world).container_item(0, 1).is_none(),
            "the backpack square the linen was in is empty"
        );

        // A creature dying is not an inventory change, so the latch does not
        // change on every despawn in the world.
        let after = world.inventory_version;
        world.remove(0x1234);
        assert_eq!(world.inventory_version, after, "an absent guid changes nothing");
    }

    /// The buyback list differs from a straight walk of the twelve slots in
    /// three ways, all matching the 1.12.1 client.
    ///
    /// Occupancy is decided by the price, the list is compacted, and it is
    /// sorted by timestamp, so the newest sale is last. That is the entry
    /// `MerchantFrame_UpdateMerchantInfo` draws on the first tab.
    #[test]
    fn the_buyback_is_compacted_and_sorted_oldest_first() {
        let mut world = world_with_a_bag();
        let player = 0x1u64;
        // Slots 0 and 2, out of order and with a gap: slot 0 holds the newer
        // sale and slot 2 the older, which is what happens once the twelve are
        // full and the server starts replacing the oldest.
        let mut fields = vec![];
        for (slot, guid, price, when) in [(0usize, 0x40u64, 55u32, 300u32), (2, 0x41, 70, 100)] {
            let base = fields::player::VENDORBUYBACK_SLOT_1 + slot as u16 * GUID_STRIDE;
            fields.push((base, guid as u32));
            fields.push((base + 1, 0));
            fields.push((fields::player::BUYBACK_PRICE_1 + slot as u16, price));
            fields.push((fields::player::BUYBACK_TIMESTAMP_1 + slot as u16, when));
        }
        // Slot 5 has a guid and no price, which is what an emptied slot looks
        // like if only half of it is cleared. It must not appear.
        let base = fields::player::VENDORBUYBACK_SLOT_1 + 5 * GUID_STRIDE;
        fields.push((base, 0x42));
        fields.push((base + 1, 0));

        world.apply(&ObjectUpdate {
            has_transport: false,
            warning: None,
            blocks: vec![
                UpdateBlock::Values {
                    guid: player,
                    values: ValuesUpdate { fields },
                },
                create(0x40, ObjectType::Item, &[(fields::object::ENTRY, 2589), (fields::item::STACK_COUNT, 4)]),
                create(0x41, ObjectType::Item, &[(fields::object::ENTRY, 858)]),
                create(0x42, ObjectType::Item, &[(fields::object::ENTRY, 1180)]),
            ],
        });

        let buyback = Inventory::read(&world).buyback;
        assert_eq!(buyback.len(), 2, "the priceless slot is not held");
        // Sorted by `sold_at`: slot 2 (100) before slot 0 (300).
        assert_eq!(buyback[0].item.entry, 858);
        assert_eq!(buyback[0].wire_slot, BUYBACK_SLOT_START as u32 + 2);
        assert_eq!(buyback[1].item.entry, 2589);
        assert_eq!(buyback[1].item.count, 4);
        assert_eq!(buyback[1].price, 55);
        // The sort exists for this assertion: the last entry is the newest
        // sale, which is `GetBuybackItemInfo(GetNumBuybackItems())`.
        assert_eq!(buyback.last().unwrap().wire_slot, BUYBACK_SLOT_START as u32);
    }

    /// A sale changes only the buyback runs, and those must count as an
    /// inventory change, or the vendor's second tab fills one packet late.
    #[test]
    fn the_buyback_runs_move_the_version() {
        let mut world = world_with_a_bag();
        // Set `is_self`, not only `player_guid`. `touches_inventory` checks the
        // entity, because the field runs it checks mean something else on
        // every other player in the world. The fixture sets the guid directly
        // because nothing else in it needs a position.
        world.get_mut(0x1).expect("the fixture's player").is_self = true;
        for field in [
            fields::player::VENDORBUYBACK_SLOT_1,
            fields::player::BUYBACK_PRICE_1,
            fields::player::BUYBACK_TIMESTAMP_1,
        ] {
            let before = world.inventory_version;
            world.apply(&ObjectUpdate {
                has_transport: false,
                warning: None,
                blocks: vec![UpdateBlock::Values {
                    guid: 0x1,
                    values: ValuesUpdate {
                        fields: vec![(field, 7)],
                    },
                }],
            });
            assert_eq!(world.inventory_version, before + 1, "field {field} is a change");
        }
    }

    /// Every entry carried, once each: the work list the query pass sends.
    #[test]
    fn the_entry_list_covers_every_layer_and_deduplicates() {
        let mut entries = Inventory::read(&world_with_a_bag()).entries();
        entries.sort_unstable();
        assert_eq!(entries, vec![858, 2589, 4500, 12640]);
    }

    /// An entry resolves to a place, in the client's search order. An item on
    /// the action bar is only an entry, because `SMSG_ACTION_BUTTONS` carries
    /// the entry and nothing else.
    #[test]
    fn an_entry_is_found_where_it_is() {
        let inventory = Inventory::read(&world_with_a_bag());

        // The worn helmet, by the interface's 1..19.
        assert_eq!(inventory.find_entry(12640), Some(ItemPlace::Worn(1)));
        // The bag itself is a worn slot too, and it is 20 rather than 19; see
        // `FIRST_BAG_INVENTORY_SLOT`.
        assert_eq!(inventory.find_entry(4500), Some(ItemPlace::Worn(20)));
        // Inside that bag, one-based.
        assert_eq!(
            inventory.find_entry(858),
            Some(ItemPlace::Carried { bag: 1, slot: 3 })
        );
        // The backpack.
        assert_eq!(
            inventory.find_entry(2589),
            Some(ItemPlace::Carried {
                bag: BACKPACK_CONTAINER,
                slot: 1
            })
        );
        // An entry nobody is carrying, and the zero that means "no item": an
        // empty slot must not resolve to the first empty square.
        assert_eq!(inventory.find_entry(99999), None);
        assert_eq!(inventory.find_entry(0), None);
    }

    /// The equipment is checked first and separately. This is the part of the
    /// order that changes what a press does: a trinket that is worn and also
    /// spare in a bag is used from the paper doll, where using the carried one
    /// would try to equip a second.
    ///
    /// A bag's contents come before the backpack, because the walk descends
    /// into a container when it reaches that container's own slot, not after
    /// the player's own slots are exhausted.
    #[test]
    fn the_search_order_is_the_clients_own() {
        let mut inventory = Inventory::read(&world_with_a_bag());
        let same = |guid: u64| ItemSlot {
            guid,
            entry: 858,
            count: 1,
            ..ItemSlot::default()
        };
        // The same entry in three places at once.
        inventory.equipped[12] = Some(same(0x100));
        inventory.backpack[0] = Some(same(0x101));
        assert_eq!(
            inventory.find_entry(858),
            Some(ItemPlace::Worn(13)),
            "worn beats carried"
        );
        assert!(inventory.is_equipped(858));

        inventory.equipped[12] = None;
        assert_eq!(
            inventory.find_entry(858),
            Some(ItemPlace::Carried { bag: 1, slot: 3 }),
            "a bag's contents come before the backpack"
        );
        assert!(!inventory.is_equipped(858));
        assert!(!inventory.is_equipped(0), "nothing is not worn");
    }

    // --- the bank -------------------------------------------------------

    /// A player with a stack in the third bank square, a bag in the second
    /// bank bag slot holding one thing, and two bag slots bought.
    fn world_with_a_bank() -> ObjectManager {
        const PLAYER: u64 = 0x1;
        const LINEN: u64 = 0x30;
        const BAG: u64 = 0x40;
        const ORE: u64 = 0x41;

        let bank_field = fields::player::BANK_SLOT_1 + 2 * GUID_STRIDE;
        let bag_field = fields::player::BANKBAG_SLOT_1 + GUID_STRIDE;
        let player_fields = vec![
            (bank_field, LINEN as u32),
            (bank_field + 1, 0),
            (bag_field, BAG as u32),
            (bag_field + 1, 0),
            // Byte 2 of PLAYER_BYTES_2: two slots bought.
            (fields::player::BYTES_2, 2 << 16),
        ];
        let mut world = ObjectManager::new();
        world.apply(&ObjectUpdate {
            has_transport: false,
            warning: None,
            blocks: vec![
                create(PLAYER, ObjectType::Player, &player_fields),
                create(
                    LINEN,
                    ObjectType::Item,
                    &[(fields::object::ENTRY, 2589), (fields::item::STACK_COUNT, 20)],
                ),
                create(
                    BAG,
                    ObjectType::Container,
                    &[
                        (fields::object::ENTRY, 4500),
                        (fields::container::NUM_SLOTS, 6),
                        (fields::container::SLOT_1, ORE as u32),
                    ],
                ),
                create(ORE, ObjectType::Item, &[(fields::object::ENTRY, 2770)]),
            ],
        });
        world.player_guid = Some(PLAYER);
        world
    }

    #[test]
    fn the_bank_reads_off_its_own_fields_by_every_numbering() {
        let inventory = Inventory::read(&world_with_a_bank());
        assert_eq!(inventory.bank.len(), BANK_SLOTS);
        assert_eq!(inventory.bank_bag_slots, 2);
        // The bank's own squares: bag id -1, and Lua ids 40..63.
        assert_eq!(inventory.container_item(BANK_CONTAINER, 3).map(|i| i.entry), Some(2589));
        assert_eq!(inventory.inventory_slot(42).map(|i| i.count), Some(20));
        assert_eq!(inventory.container_num(BANK_CONTAINER), Some(BANK_SLOTS));
        // The bag in the second bank bag slot: bag id 6, Lua id 65.
        assert_eq!(inventory.bag_item(6).map(|i| i.entry), Some(4500));
        assert_eq!(inventory.inventory_slot(65).map(|i| i.entry), Some(4500));
        assert_eq!(inventory.container_item(6, 1).map(|i| i.entry), Some(2770));
        assert_eq!(inventory.container(5), None, "an empty bank bag slot is no bag");
        // Every entry is queried, so the squares get names.
        let entries = inventory.entries();
        assert!(entries.contains(&2589) && entries.contains(&4500) && entries.contains(&2770));
        // The action bar's walk does not descend into the bank.
        assert_eq!(inventory.find_entry(2589), None);
    }
}
