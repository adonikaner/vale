//! What the character is carrying: the bags, their contents, and the worn
//! items.
//!
//! No packet lists the inventory. It is held entirely in update fields, in
//! three layers, and this module joins them:
//!
//! ```text
//! the player's own block   PLAYER_FIELD_INV_SLOT_HEAD   23 guids: 19 worn + 4 bags
//!                          PLAYER_FIELD_PACK_SLOT_1     16 guids: the backpack
//!                          PLAYER_FIELD_BANK_SLOT_1     24 guids: the bank's own squares
//!                          PLAYER_FIELD_BANKBAG_SLOT_1   6 guids: the bags in the bank
//!                          PLAYER_FIELD_KEYRING_SLOT_1  32 guids: the key ring
//! each bag is an object    CONTAINER_FIELD_NUM_SLOTS    how big it is
//!                          CONTAINER_FIELD_SLOT_1       36 guids: what is in it
//! each item is an object   OBJECT_FIELD_ENTRY           which item it is
//!                          ITEM_FIELD_STACK_COUNT       how many
//!                          ITEM_FIELD_DURABILITY        …and how worn out
//! ```
//!
//! A potion in the third slot of the second bag is therefore three objects
//! deep. The player names a container GUID, the container names an item GUID,
//! and the item names an entry. Only the entry can be turned into a name, an
//! icon or a tooltip, through `CMSG_ITEM_QUERY_SINGLE`, because `Item.dbc` is
//! not in the 1.12 archives. All three kinds of object arrive in the same
//! `SMSG_UPDATE_OBJECT` at login, as ordinary create blocks with no position
//! and no movement, so [`crate::state::objects::ObjectManager`] already holds
//! the whole inventory and this module reads it from there.
//!
//! ## Field indices, Lua slot ids and bag ids
//!
//! The field index is zero-based and the interface's slot id is one-based,
//! and the two do not cover the same set:
//!
//! ```text
//! field 0..18    worn:  head, neck, shoulder, shirt, chest, waist, legs, feet,
//!                       wrist, hands, finger x2, trinket x2, back, main hand,
//!                       off hand, ranged, tabard
//! field 19..22   the four bag slots
//! Lua 1..19      the worn slots           GetInventoryItemTexture("player", 1)
//! Lua 20..23     the four bag slots       CharacterBag0Slot:GetID() == 20
//! Lua 40..63     the bank's 24 squares    BankButtonIDToInvSlotID(1) == 40
//! Lua 64..69     the six bank bag slots   BankButtonIDToInvSlotID(1, 1) == 64
//! bag id 0       the backpack             GetContainerNumSlots(0)
//! bag id 1..4    the worn bags            …counted from the *first* bag slot
//! bag id -2      the key ring
//! bag id -1      the bank's own squares   BANK_CONTAINER in BankFrame.lua
//! bag id 5..10   the bags in the bank     ToggleBag(5) from BankFrameBag1
//! ```
//!
//! The bank is in this table because its items are in these fields. No
//! packet in the bank family carries an item (see [`crate::play::bank`]), so a
//! bank square is read here, addressed by the same three numberings, and moved
//! by the same swap packets. The banker only decides whether the server
//! accepts a move. Lua ids 24..39 are unused in 1.12: 24 is where the backpack
//! would be if it were a slot, and it is not a slot.
//!
//! Both numberings are the game's own, and neither can be computed from the
//! other by arithmetic alone: bag id 1 is Lua slot 20 is field 19.
//! `PaperDollItemFrame.dbc` states the Lua ids in 36 rows of
//! `(name, art, id)`, which `GetInventorySlotInfo` walks. This module keeps
//! the field mapping beside it so that no caller does the subtraction itself.
//!
//! ## Why the inventory is read as a snapshot
//!
//! [`Inventory::read`] walks the manager once and copies the result out. The
//! alternative, answering each `GetContainerItemInfo` by following three GUIDs
//! under the world lock, would add a hash lookup per bag slot on the hover
//! path and hold the lock while Lua ran. A bag frame asks about 16 to 36 slots
//! per refresh and refreshes on every `BAG_UPDATE`, so the snapshot is taken
//! when the fields change and read as many times as the interface needs.

use crate::bytes::Reader;
use crate::state::fields;
use crate::state::objects::{Entity, ObjectManager};
use crate::state::update::ObjectType;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The nineteen worn slots, in field order.
///
/// This is `EquipmentSlots` in vmangos, and the same order
/// `PLAYER_VISIBLE_ITEM_n` uses, so [`crate::state::objects::Entity::equipment`]
/// and this module agree slot for slot without either converting.
pub const EQUIPMENT_SLOTS: usize = 19;

/// The four bag slots that follow the worn slots in the same field block.
pub const BAG_SLOTS: usize = 4;

/// How many GUIDs `PLAYER_FIELD_INV_SLOT_HEAD` covers: the worn slots and the
/// bag slots together. 23 guids = 46 fields, which is the gap to
/// `PLAYER_FIELD_PACK_SLOT_1`.
pub const INVENTORY_SLOTS: usize = EQUIPMENT_SLOTS + BAG_SLOTS;

/// The backpack's sixteen slots. They live in the player's block rather than
/// in a container object. There is no backpack item, so
/// `GetContainerNumSlots(0)` cannot be answered by looking for one.
pub const BACKPACK_SLOTS: usize = 16;

/// The bank's own squares. `PLAYER_FIELD_BANK_SLOT_1` is 48 fields = 24
/// guids, and `BankFrame.xml` lays out `BankFrameItem1..24`. Like the
/// backpack, they are a run in the player's block with no container object
/// behind it.
pub const BANK_SLOTS: usize = 24;

/// The bag slots in the bank. `PLAYER_FIELD_BANKBAG_SLOT_1` is six guids,
/// `BankFrame.lua`'s `NUM_BANKBAGSLOTS` is 6, and
/// `BANK_SLOT_BAG_END - BANK_SLOT_BAG_START` is 6. How many of the six may be
/// used is stored in a byte; see [`Inventory::bank_bag_slots`].
pub const BANK_BAG_SLOTS: usize = 6;

/// The key ring's slots: 32 fields = 16 guids. The interface shows fewer than
/// exist: `GetKeyRingSize()` in `ContainerFrame.lua` is 4 under level 40 and
/// grows to 16 past 60, so the extra slots exist and are not drawn.
pub const KEYRING_SLOTS: usize = 16;

/// The largest a 1.12 bag can be: `MAX_CONTAINER_ITEMS` in
/// `ContainerFrame.lua`, and the width of `CONTAINER_FIELD_SLOT_1`.
pub const CONTAINER_SLOTS: usize = 36;

/// How many sold items a vendor holds for buyback: `BUYBACK_SLOT_END -
/// BUYBACK_SLOT_START`, and the interface's own `BUYBACK_ITEMS_PER_PAGE`.
///
/// Both sides use twelve: `PLAYER_FIELD_VENDORBUYBACK_SLOT_1` is 24 fields =
/// 12 guids, and `MerchantFrame.lua` lays out twelve buttons on its second
/// tab.
pub const BUYBACK_SLOTS: usize = 12;

/// `BUYBACK_SLOT_START`: where the buyback run begins in the server's flat
/// 118-entry slot numbering, and the number `CMSG_BUYBACK_ITEM` carries.
///
/// The 1.12.1 client reads the buyback items from slots 69 to 81 of the
/// player's guid array. `PLAYER_FIELD_INV_SLOT_HEAD + 69 * 2` is 624, which is
/// `PLAYER_FIELD_VENDORBUYBACK_SLOT_1`. The slot numbering and the field
/// numbering agree at this point, which confirms both values.
pub const BUYBACK_SLOT_START: u8 = 69;

/// `KEYRING_CONTAINER`, declared in `MainMenuBarBagButtons.lua`. It is
/// negative, so every bag id this client handles is an `i32` rather than a
/// count.
pub const KEYRING_CONTAINER: i32 = -2;

/// The backpack's bag id, which is what `ToggleBackpack` opens.
pub const BACKPACK_CONTAINER: i32 = 0;

/// `BANK_CONTAINER`, declared in `BankFrame.lua`: the bag id of the bank's
/// own twenty-four squares, which every `BankFrameItem` button passes to
/// `PickupContainerItem`, `UseContainerItem` and `GetContainerItemInfo`.
pub const BANK_CONTAINER: i32 = -1;

/// The bag id of the first bag in the bank. `BankFrameBag1`'s own id is 5
/// (`NUM_BAG_SLOTS + 1`, which `CloseBankBagFrames` and
/// `UpdateBagButtonHighlight` both count from), and `ToggleBag(5)` opens it
/// as a container frame like any worn bag.
pub const FIRST_BANK_BAG_ID: i32 = 5;

/// A GUID field is two update fields wide, little end first.
const GUID_STRIDE: u16 = 2;

// ---------------------------------------------------------------------------
// The server's (bag, slot) numbering, used by every outbound item packet
// ---------------------------------------------------------------------------
//
// The two numberings above are both the client's. Every outbound item packet
// addresses a slot as a `(bag, slot)` pair of server indices, and neither half
// matches either of them. The bag is `INVENTORY_SLOT_BAG_0` (255) for anything
// held in the player's own object, or the inventory slot of the bag itself
// for anything inside a container. The slot is a position in one flat
// 118-entry numbering (`PLAYER_SLOT_START..PLAYER_SLOT_END` in vmangos'
// `Player.h`), not a position in the bag.
//
// An off-by-one here names a neighbouring slot, which holds a different item
// and produces no error. The conversion is therefore done here, once, beside
// the other two, with `Player::GetItemByPos` as the reference.

/// `INVENTORY_SLOT_BAG_0`, meaning "not in a container". The worn slots, the
/// backpack and the key ring all live in the player's own object and are
/// addressed by this bag with a slot in the flat numbering below.
pub const SERVER_BAG_NONE: u8 = 255;

/// `INVENTORY_SLOT_BAG_START`: the flat slot of the first worn bag, which is
/// also the bag index a packet uses to name that bag's contents. It is the
/// same 19 the field run uses, because both count the bag slots from the end
/// of the equipment.
const SERVER_BAG_START: u8 = 19;

/// `INVENTORY_SLOT_ITEM_START`: where the backpack's sixteen slots begin in
/// the flat numbering.
const SERVER_BACKPACK_START: u8 = 23;

/// `KEYRING_SLOT_START`. It is 81, not 86: `KEYRING_SLOT_END` is 97 and the
/// ring is sixteen guids wide, which confirms the value.
const SERVER_KEYRING_START: u8 = 81;

/// `BANK_SLOT_ITEM_START`: the bank's twenty-four squares in the flat
/// numbering, 39..63, directly after the backpack's sixteen. The value is from
/// `Player.h`, and is the same 39 that `BankButtonIDToInvSlotID` adds to a
/// button id.
pub const SERVER_BANK_START: u8 = 39;

/// `BANK_SLOT_BAG_START`: the six bank bag slots, 63..69. It is also the bag
/// index a packet uses to name the contents of one of those bags, as
/// [`SERVER_BAG_START`] is for a worn bag.
pub const SERVER_BANK_BAG_START: u8 = 63;

/// Whether a wire position is in the bank. This is `Player::IsBankPos`, which
/// `HandleAutoStoreBankItemOpcode` uses to decide the direction of a move and
/// which the swap handlers test to require a banker in reach.
pub fn is_bank_position(bag: u8, slot: u8) -> bool {
    let bank_bags = SERVER_BANK_BAG_START..SERVER_BANK_BAG_START + BANK_BAG_SLOTS as u8;
    (bag == SERVER_BAG_NONE && (SERVER_BANK_START..SERVER_BANK_BAG_START).contains(&slot))
        || (bag == SERVER_BAG_NONE && bank_bags.contains(&slot))
        || bank_bags.contains(&bag)
}

/// A container slot as the wire addresses it: the `(bag, slot)` pair for
/// `CMSG_USE_ITEM`, `CMSG_AUTOEQUIP_ITEM` and every other packet that names an
/// item by its position.
///
/// Takes the interface's own pair: bag id 0 the backpack, 1..4 the worn bags,
/// [`KEYRING_CONTAINER`] the ring, and a one-based slot within it. A slot of
/// zero or one past the end of the numbering returns `None` rather than
/// wrapping onto the neighbouring slot.
pub fn server_container_slot(bag: i32, slot: usize) -> Option<(u8, u8)> {
    let index = u8::try_from(slot.checked_sub(1)?).ok()?;
    match bag {
        BACKPACK_CONTAINER if (index as usize) < BACKPACK_SLOTS => {
            Some((SERVER_BAG_NONE, SERVER_BACKPACK_START + index))
        }
        KEYRING_CONTAINER if (index as usize) < KEYRING_SLOTS => {
            Some((SERVER_BAG_NONE, SERVER_KEYRING_START + index))
        }
        // The bank's own squares are a run in the player's object, like the
        // backpack; a bag in the bank is named by its own slot, like a worn one.
        BANK_CONTAINER if (index as usize) < BANK_SLOTS => {
            Some((SERVER_BAG_NONE, SERVER_BANK_START + index))
        }
        1..=4 if (index as usize) < CONTAINER_SLOTS => {
            Some((SERVER_BAG_START + bag as u8 - 1, index))
        }
        5..=10 if (index as usize) < CONTAINER_SLOTS => Some((
            SERVER_BANK_BAG_START + (bag - FIRST_BANK_BAG_ID) as u8,
            index,
        )),
        _ => None,
    }
}

/// A worn slot as the wire addresses it, by the interface's own id: 1..19 the
/// equipment, 20..23 the four bag slots, 40..63 the bank's squares and 64..69
/// its bag slots.
///
/// The conversion is one subtraction with no bag, because everything in that
/// run is in the player's own object. It is here rather than at the call site
/// for the same reason as the other two conversions. Ids 24..39 map to
/// nothing.
pub fn server_inventory_slot(id: u32) -> Option<(u8, u8)> {
    let worn = 1..=INVENTORY_SLOTS as u32;
    let bank = FIRST_BANK_INVENTORY_SLOT..FIRST_BANK_BAG_INVENTORY_SLOT + BANK_BAG_SLOTS as u32;
    (worn.contains(&id) || bank.contains(&id)).then(|| (SERVER_BAG_NONE, (id - 1) as u8))
}

/// `CMSG_USE_ITEM`: `u8 bag, u8 slot, u8 spellIndex`, then a
/// `SpellCastTargets`.
///
/// The third byte is an index, not a count. `HandleUseItemOpcode` reads it as
/// `spellSlot`, uses it to index the item prototype's five spell blocks, and
/// refuses any block whose trigger is not `ITEM_SPELLTRIGGER_ON_USE`. A
/// bandage and a potion therefore send the same packet with a different value
/// in this byte, and a wrong value gets `EQUIP_ERR_ITEM_NOT_FOUND` for an item
/// that is in the bag. See [`crate::state::query::ItemSpell::trigger`], which
/// is where the index comes from.
///
/// The tail is the ordinary target block, [`crate::play::spells::CastTarget`]:
/// the same shapes `CMSG_CAST_SPELL` sends, written by the same function,
/// because the server reads both with `SpellCastTargets::ReadForCaster`.
pub fn use_item_body(bag: u8, slot: u8, spell_index: u8, target: crate::play::spells::CastTarget) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(bag).u8(slot).u8(spell_index);
    crate::play::spells::write_cast_target(&mut w, target);
    w.buf
}

/// `CMSG_AUTOEQUIP_ITEM`: `u8 srcBag, u8 srcSlot`, and nothing else.
///
/// This is the second packet a right-click can send. `HandleUseItemOpcode`
/// refuses outright when
/// `proto->InventoryType != INVTYPE_NON_EQUIP && !pItem->IsEquipped()`, so a
/// sword in a bag can never be used; the client has to recognise it as
/// equippable and send this packet instead. The server picks the slot itself
/// (`CanEquipItem(NULL_SLOT, …)`), which is how a ring goes to whichever
/// finger is free.
pub fn auto_equip_body(bag: u8, slot: u8) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(bag).u8(slot);
    w.buf
}

/// `CMSG_OPEN_ITEM`: `u8 bag, u8 slot`, and nothing else. These are the same
/// two bytes [`auto_equip_body`] sends, to a different opcode.
///
/// This is the third packet a right-click can send, besides using and
/// equipping: for a Blue Sack of Gems, a lockbox or a wrapped gift.
/// `HandleOpenItemOpcode` answers it with
/// `SendLoot(item->GetObjectGuid(), LOOT_CORPSE)`, so the reply is an ordinary
/// `SMSG_LOOT_RESPONSE` whose guid is the item's, and it opens the same window
/// a corpse opens. The rest of the loot path is unchanged.
///
/// The server refuses four cases before that: no such item, in flight, dead,
/// and a lock whose `Skill[0]` or `Skill[1]` is set without
/// [`item_dyn_flags::UNLOCKED`] on the copy. The last case is a strongbox a
/// rogue has not yet picked, answered with `EQUIP_ERR_ITEM_LOCKED`.
///
/// A wrapped item uses the same packet. The server reads `character_gifts`,
/// puts the real entry back on the object and sends nothing else, so the
/// unwrapping appears as an ordinary field change.
pub fn open_item_body(bag: u8, slot: u8) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(bag).u8(slot);
    w.buf
}

/// `CMSG_SET_AMMO`: `{u32 entry}`, with 0 to unload.
///
/// The ammo slot is the one paper-doll square that holds no item. Instead,
/// `PLAYER_AMMO_ID` (field 1223) names an item entry the character carries
/// somewhere in the bags, and the count the square draws is `GetItemCount` of
/// that entry. Loading it is therefore not a move and cannot be
/// `CMSG_AUTOEQUIP_ITEM`: `FindEquipSlot` has no case for `INVTYPE_AMMO` and
/// answers `EQUIP_ERR_ITEM_CANT_BE_EQUIPPED`. It is this packet, which
/// `HandleSetAmmoOpcode` answers with `SetAmmo` (`CanUseAmmo`: alive, the
/// entry carried, its `InventoryType` 24, else `SendEquipError`) or, for 0,
/// `RemoveAmmo`. Both end with the field changing, which redraws the square.
/// `INVTYPE_AMMO` is the only check: vmangos does not test the ranged weapon
/// here. `CheckAmmoCompatibility` is applied to the shot, not to the load.
pub fn set_ammo_body(entry: u32) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u32(entry);
    w.buf
}

/// `NULL_SLOT` / `NULL_BAG`, meaning "the server picks". It is the same 255 as
/// [`SERVER_BAG_NONE`] with a different meaning, so it has its own name.
///
/// `Player::IsValidPos(bag, NULL_SLOT, false)` returns true outright, so a
/// packet naming a destination bag and this slot means "anywhere in that
/// bag". `CMSG_AUTOSTORE_BAG_ITEM` uses this form.
pub const SERVER_SLOT_ANY: u8 = 255;

/// `CMSG_SWAP_INV_ITEM`: `u8 srcSlot, u8 dstSlot`. Both are positions in the
/// player's own object, and neither side has a bag.
///
/// This is the narrow form of a swap, and the one the paper doll uses.
/// Everything it can address (the nineteen worn slots, the four bag slots, the
/// backpack, the key ring) lives in the player's block, so the bag byte would
/// be [`SERVER_BAG_NONE`] on both sides and the server does not ask for it.
///
/// The source comes first. `HandleSwapInvItemOpcode` reads
/// `srcslot >> dstslot`, while [`swap_item_body`] below puts the destination
/// first. The two packets use opposite orders on the wire, and neither
/// packet's layout indicates it.
pub fn swap_inv_item_body(src_slot: u8, dst_slot: u8) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(src_slot).u8(dst_slot);
    w.buf
}

/// `CMSG_SWAP_ITEM`: `u8 dstBag, u8 dstSlot, u8 srcBag, u8 srcSlot`. This is
/// the general form, for a move with a bag on either end.
///
/// The destination comes first, the opposite of [`swap_inv_item_body`]'s
/// order; see `HandleSwapItem`.
pub fn swap_item_body(dst_bag: u8, dst_slot: u8, src_bag: u8, src_slot: u8) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(dst_bag).u8(dst_slot).u8(src_bag).u8(src_slot);
    w.buf
}

/// `CMSG_AUTOSTORE_BAG_ITEM`: `u8 srcBag, u8 srcSlot, u8 dstBag`. Stores the
/// item anywhere in the destination bag where it fits.
///
/// The destination has no slot: `HandleAutoStoreBagItemOpcode` checks
/// `IsValidPos(dstbag, NULL_SLOT, false)` and lets `CanStoreItem` choose. This
/// is what `PutItemInBag` and `PutItemInBackpack` send, because a click on a
/// bag button names no square. [`SERVER_BAG_NONE`] as the destination is the
/// backpack.
pub fn autostore_bag_item_body(src_bag: u8, src_slot: u8, dst_bag: u8) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(src_bag).u8(src_slot).u8(dst_bag);
    w.buf
}

/// `CMSG_SPLIT_ITEM`: `u8 srcBag, u8 srcSlot, u8 dstBag, u8 dstSlot, u8
/// count`. Moves part of a stack.
///
/// The source comes first here and the destination first in
/// [`swap_item_body`]; this is the third of the three byte orders in this
/// family of packets. The server drops a count of zero as a forged packet, so
/// the caller filters it out rather than sending it.
pub fn split_item_body(
    src_bag: u8,
    src_slot: u8,
    dst_bag: u8,
    dst_slot: u8,
    count: u8,
) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(src_bag).u8(src_slot).u8(dst_bag).u8(dst_slot).u8(count);
    w.buf
}

/// `CMSG_DESTROYITEM`: `u8 bag, u8 slot, u8 count`, then three more bytes.
///
/// `HandleDestroyItemOpcode` reads six bytes and uses three: `data1..data3`
/// are read and never referenced. They are written as zero because the packet
/// has a fixed length, and a short one would leave the next opcode misaligned
/// in the same buffer.
pub fn destroy_item_body(bag: u8, slot: u8, count: u8) -> Vec<u8> {
    let mut w = crate::bytes::Writer::new();
    w.u8(bag).u8(slot).u8(count).u8(0).u8(0).u8(0);
    w.buf
}

/// The Lua inventory slot id of the first bag slot: `CharacterBag0Slot`'s own
/// `GetID()`, from `PaperDollItemFrame.dbc`. The four bags are 20..23 and the
/// worn slots are 1..19, so this is 20, not 19.
pub const FIRST_BAG_INVENTORY_SLOT: u32 = 20;

/// The Lua inventory slot id of the first bank square:
/// `BankButtonIDToInvSlotID(1)` is `BANK_SLOT_ITEM_START + 1`, one-based over
/// the wire's 39, so the squares are 40..63.
pub const FIRST_BANK_INVENTORY_SLOT: u32 = SERVER_BANK_START as u32 + 1;

/// The Lua inventory slot id of the first bag slot in the bank:
/// `BankButtonIDToInvSlotID(1, 1)` is 64, and `BankFrameBag1..6` are 64..69.
pub const FIRST_BANK_BAG_INVENTORY_SLOT: u32 = SERVER_BANK_BAG_START as u32 + 1;

/// One occupied slot, flattened out of the item object in it.
///
/// A slot whose container this client has seen, but whose item it has not,
/// is `None` rather than a zeroed record. An empty square and a square holding
/// an item that cannot be named are different cases, and the interface draws
/// them differently.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemSlot {
    /// The item object's own GUID. It is unique to this stack, which
    /// distinguishes two identical stacks.
    pub guid: u64,
    /// `OBJECT_FIELD_ENTRY`: which item it is, and the only value
    /// `CMSG_ITEM_QUERY_SINGLE` takes.
    pub entry: u32,
    /// `ITEM_FIELD_STACK_COUNT`. It is 1 for anything that does not stack. The
    /// create block omits the field when it is zero (see [`Entity::created`]),
    /// so this is read with a floor rather than as an `Option`.
    pub count: u32,
    pub durability: u32,
    pub max_durability: u32,
    /// `ITEM_FIELD_FLAGS`, the whole word. The three bits this client reads
    /// are named in [`item_dyn_flags`] and read through the three accessors
    /// below.
    ///
    /// The field is kept as one word rather than one bool per bit because the
    /// bits are read in combination: `<Right Click to Open>` depends on a
    /// prototype flag and [`Self::unlocked`], or on a second prototype flag and
    /// [`Self::wrapped`]. Splitting the bits would turn that one test into
    /// three arguments instead of one.
    pub flags: u32,
    /// `ITEM_FIELD_ENCHANTMENT`: seven slots of `(id, duration, charges)`, in
    /// vmangos' `EnchantmentSlot` order: 0 permanent, 1 temporary, 2 unused
    /// in 1.12, 3..6 the random property's enchantments.
    pub enchantments: [ItemEnchant; ENCHANTMENT_SLOTS],
    /// `ITEM_FIELD_RANDOM_PROPERTIES_ID`: the `ItemRandomProperties.dbc` row
    /// that names the item's suffix ("of the Bear"), 0 for none.
    pub random_property: i32,
    /// `ITEM_FIELD_SPELL_CHARGES`: this copy's charges, one per prototype
    /// spell. Negative means the item is used up when they run out.
    pub spell_charges: [i32; SPELL_CHARGE_SLOTS],
    /// `ITEM_FIELD_CREATOR`: the player who made it, 0 for nobody.
    pub creator: u64,
    /// `ITEM_FIELD_GIFTCREATOR`: the player who wrapped it, 0 for nobody.
    pub gift_creator: u64,
    /// `ITEM_FIELD_ITEM_TEXT_ID`: text a player wrote into the item (a letter
    /// taken out of the mailbox), 0 for none.
    pub text_id: u32,
    /// When this copy expires, from `SMSG_ITEM_TIME_UPDATE`. `None` for an
    /// item that does not expire or whose time the server has not stated.
    pub expires: Option<Instant>,
}

/// How many enchantment slots an item has: vmangos' `MAX_ENCHANTMENT_SLOT`,
/// and the 21 fields of `ITEM_FIELD_ENCHANTMENT` at three per slot.
pub const ENCHANTMENT_SLOTS: usize = 7;

/// How many charge counters an item has: one per prototype spell, the five
/// fields of `ITEM_FIELD_SPELL_CHARGES`.
pub const SPELL_CHARGE_SLOTS: usize = 5;

/// One enchantment slot of an item object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemEnchant {
    /// The `SpellItemEnchantment.dbc` row, 0 for an empty slot. The 1.12.1
    /// client looks the row up by the absolute value and draws a negative id
    /// in red.
    pub id: i32,
    /// The field's own duration in milliseconds. The server writes it when the
    /// enchantment is applied and when the character is saved, so it is stale
    /// between the two; the time the plate prints comes from
    /// [`Self::expires`].
    pub duration_ms: u32,
    /// How many charges are left, 0 for an enchantment without charges.
    pub charges: u32,
    /// When the enchantment runs out, from `SMSG_ITEM_ENCHANT_TIME_UPDATE`.
    pub expires: Option<Instant>,
}

impl ItemEnchant {
    /// Milliseconds left at `now`, or `None` when no time was stated or it has
    /// passed.
    pub fn left_ms(&self, now: Instant) -> Option<u32> {
        left_ms(self.expires, now)
    }
}

/// Milliseconds from `now` to `expires`, `None` when there is no expiry or it
/// has passed. The 1.12.1 client prints no time for a clock that has reached
/// zero.
pub fn left_ms(expires: Option<Instant>, now: Instant) -> Option<u32> {
    let left = expires?.checked_duration_since(now)?;
    let ms = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
    (ms > 0).then_some(ms)
}

/// The clocks the server starts on carried items, keyed by item guid.
///
/// Neither clock is an update field. `SMSG_ITEM_ENCHANT_TIME_UPDATE` states how
/// many seconds a temporary enchantment has left and `SMSG_ITEM_TIME_UPDATE`
/// how many an expiring item has; the 1.12.1 client turns each into an expiry
/// time when the packet arrives and counts down itself. vmangos sends both when
/// the clock starts and again at login (`Player::SendEnchantmentDurations`,
/// `Player::SendItemDurations`). The login copies can arrive before the item
/// objects are created, so the clocks are kept by guid rather than on the
/// objects.
#[derive(Debug, Clone, Default)]
pub struct ItemTimers {
    enchantments: HashMap<(u64, u8), Instant>,
    items: HashMap<u64, Instant>,
}

impl ItemTimers {
    /// Record an enchantment's time as `SMSG_ITEM_ENCHANT_TIME_UPDATE` states
    /// it. Zero seconds clears the clock.
    pub fn note_enchantment(&mut self, time: EnchantTime, now: Instant) {
        let key = (time.item, time.slot);
        if time.seconds == 0 {
            self.enchantments.remove(&key);
        } else {
            self.enchantments
                .insert(key, now + Duration::from_secs(u64::from(time.seconds)));
        }
    }

    /// Record an item's own time, from `SMSG_ITEM_TIME_UPDATE`.
    pub fn note_item(&mut self, time: ItemTime, now: Instant) {
        if time.seconds == 0 {
            self.items.remove(&time.item);
        } else {
            self.items
                .insert(time.item, now + Duration::from_secs(u64::from(time.seconds)));
        }
    }

    /// When one enchantment slot of an item runs out.
    pub fn enchantment(&self, item: u64, slot: u8) -> Option<Instant> {
        self.enchantments.get(&(item, slot)).copied()
    }

    /// When an item runs out.
    pub fn item(&self, item: u64) -> Option<Instant> {
        self.items.get(&item).copied()
    }
}

/// `SMSG_ITEM_ENCHANT_TIME_UPDATE`: `{u64 item, u32 slot, u32 seconds, u64
/// player}`, as vmangos' `WorldSession::SendItemEnchantTimeUpdate` writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnchantTime {
    pub item: u64,
    pub slot: u8,
    pub seconds: u32,
    pub player: u64,
}

/// Read `SMSG_ITEM_ENCHANT_TIME_UPDATE`. `None` for a short body or a slot
/// past [`ENCHANTMENT_SLOTS`].
pub fn parse_item_enchant_time_update(body: &[u8]) -> Option<EnchantTime> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4 + 4) {
        return None;
    }
    let item = r.u64();
    let slot = r.u32();
    let seconds = r.u32();
    // The player guid closes the packet. The clock needs only the three
    // fields before it.
    let player = if r.has(8) { r.u64() } else { 0 };
    Some(EnchantTime {
        item,
        slot: u8::try_from(slot)
            .ok()
            .filter(|s| usize::from(*s) < ENCHANTMENT_SLOTS)?,
        seconds,
        player,
    })
}

/// `SMSG_ITEM_TIME_UPDATE`: `{u64 item, u32 seconds}`, from vmangos'
/// `Item::SendTimeUpdate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemTime {
    pub item: u64,
    pub seconds: u32,
}

/// Read `SMSG_ITEM_TIME_UPDATE`. `None` for a short body.
pub fn parse_item_time_update(body: &[u8]) -> Option<ItemTime> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    Some(ItemTime {
        item: r.u64(),
        seconds: r.u32(),
    })
}

/// `ITEM_DYNFLAG_*`: the bits of an item object's `ITEM_FIELD_FLAGS`.
///
/// These bits are per copy and change during a session.
/// [`crate::state::query::item_flags`] is the per-prototype word, which does
/// not change. The two are easy to confuse and a mix-up produces no error, so
/// both modules state the distinction.
pub mod item_dyn_flags {
    /// Bound to this character now, as opposed to binding when picked up. The
    /// tooltip's first grey line.
    pub const BOUND: u32 = 0x0000_0001;
    /// This copy's lock has been picked. `HandleOpenItemOpcode` refuses an
    /// item with a `proto->LockID` unless this bit is set, and the tooltip
    /// hides its `<Right Click to Open>` line for the same reason.
    pub const UNLOCKED: u32 = 0x0000_0004;
    /// This copy is inside wrapping paper: its entry and flags are the
    /// wrapper's, and opening it is what puts the real item back.
    pub const WRAPPED: u32 = 0x0000_0008;
}

impl ItemSlot {
    /// Read one item object. Returns `None` for a GUID that names no known
    /// object. That is a normal case, not an error, because a bag's contents
    /// stream in one block behind the bag itself.
    fn read(world: &ObjectManager, guid: u64) -> Option<ItemSlot> {
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

    /// The enchantment in one slot, or `None` for an empty slot.
    pub fn enchantment(&self, slot: usize) -> Option<&ItemEnchant> {
        self.enchantments.get(slot).filter(|e| e.id != 0)
    }

    /// Whether this item is broken, with its durability worn down to zero.
    /// This answers `GetInventoryItemBroken`, which paints the slot's icon
    /// red.
    pub fn broken(&self) -> bool {
        self.max_durability > 0 && self.durability == 0
    }

    /// Bound to this character already: [`item_dyn_flags::BOUND`]. The
    /// tooltip's first grey line.
    pub fn soulbound(&self) -> bool {
        self.flags & item_dyn_flags::BOUND != 0
    }

    /// This copy's lock has been picked: [`item_dyn_flags::UNLOCKED`].
    pub fn unlocked(&self) -> bool {
        self.flags & item_dyn_flags::UNLOCKED != 0
    }

    /// This copy is wrapped: [`item_dyn_flags::WRAPPED`].
    pub fn wrapped(&self) -> bool {
        self.flags & item_dyn_flags::WRAPPED != 0
    }
}

/// A two-field guid, low word first, with 0 for an absent half.
fn guid_field(entity: &Entity, index: u16) -> u64 {
    let low = entity.field(index).unwrap_or(0);
    let high = entity.field(index + 1).unwrap_or(0);
    u64::from(low) | (u64::from(high) << 32)
}

/// A worn bag and what is in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bag {
    /// The bag's own item slot. Its entry names the bag and gives it an icon,
    /// so a bag is an item as well as a container.
    pub item: ItemSlot,
    /// The slots, `CONTAINER_FIELD_NUM_SLOTS` of them. The count is the
    /// container object's, not the prototype's: the two agree, but only the
    /// container object's value has arrived when the bag first appears.
    pub slots: Vec<Option<ItemSlot>>,
}

impl Bag {
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// One item a vendor holds for buyback, as the buyback tab draws it.
///
/// This is not part of what the character carries. The item object exists
/// and is owned, but it is in no bag, and the comment in
/// `Player::DurabilityRepairAll` ("bank, buyback and keys not repaired") shows
/// the server does not treat it as carried either. It is therefore its own
/// list beside the four in [`Inventory`], not a sixth container.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuybackSlot {
    /// The item itself, read out of the object the guid names.
    pub item: ItemSlot,
    /// `PLAYER_FIELD_BUYBACK_PRICE_1 + n`: what buying it back costs, in
    /// copper. It also marks occupancy: the 1.12.1 client skips a slot whose
    /// price is zero, which is how an emptied slot is recognised without a
    /// second flag.
    pub price: u32,
    /// `PLAYER_FIELD_BUYBACK_TIMESTAMP_1 + n`: when the item was sold, in the
    /// server's `time() - loginTime + 30h` units. It is never displayed and is
    /// carried only as the sort key.
    pub sold_at: u32,
    /// Which of the twelve wire slots this is, 69..80. The interface addresses
    /// a row by its position in this list, and `CMSG_BUYBACK_ITEM` needs the
    /// slot. Once all twelve are full the server evicts the oldest item rather
    /// than the last, so the position and the slot no longer agree.
    pub wire_slot: u32,
}

/// Everything the character is carrying, as one snapshot.
///
/// It is empty before `SMSG_UPDATE_OBJECT` has described the player, which is
/// the correct state at a character screen, and makes `GetContainerNumSlots`
/// answer 0 there rather than a stale count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    /// The nineteen worn slots, in field order: `equipped[0]` is the head.
    pub equipped: [Option<ItemSlot>; EQUIPMENT_SLOTS],
    /// The four worn bags. `None` for an empty bag slot, which is different
    /// from a bag with no room left in it.
    pub bags: [Option<Bag>; BAG_SLOTS],
    /// The backpack, always sixteen entries once the player exists.
    pub backpack: Vec<Option<ItemSlot>>,
    /// The key ring, always sixteen slots. See [`KEYRING_SLOTS`] for why the
    /// interface shows fewer.
    pub keyring: Vec<Option<ItemSlot>>,
    /// The bank's own twenty-four squares, always that many once the player
    /// exists. Like the backpack, they are a run in the player's block. See
    /// [`crate::play::bank`] for why no packet carries them.
    pub bank: Vec<Option<ItemSlot>>,
    /// The six bags in the bank, `None` for an empty slot. The slot does not
    /// say whether it has been bought.
    pub bank_bags: [Option<Bag>; BANK_BAG_SLOTS],
    /// How many of the six bank bag slots may be used: the third byte of
    /// `PLAYER_BYTES_2`, which `GetNumBankSlots()` returns and a purchase
    /// increments. 0 for a character who has never bought one.
    pub bank_bag_slots: u8,
    /// The items the vendor holds, oldest first. See [`read_buyback`] for why
    /// this is the 1.12.1 client's order.
    ///
    /// Only occupied slots are included, so this has 0..12 entries rather than
    /// twelve `Option`s: `GetNumBuybackItems()` is its length and
    /// `GetBuybackItemInfo(i)` is a one-based index into it.
    pub buyback: Vec<BuybackSlot>,
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

    /// Every item a repair-all would touch, in the server's order.
    ///
    /// `Player::DurabilityRepairAll` walks `EQUIPMENT_SLOT_START ..
    /// INVENTORY_SLOT_ITEM_END` (the worn slots, the four bag containers
    /// themselves and the backpack) and then the contents of each bag. Its
    /// comment names what it leaves out: "bank, buyback and keys not
    /// repaired". The key ring is therefore absent here. A client that walked
    /// every container it knows would quote a higher price than the server
    /// charges.
    ///
    /// This is here rather than in the renderer because it walks this module's
    /// layout, for the same reason `container` is here. The price it is used
    /// for is a rule from a data file and lives in
    /// `vale_assets::tables::repair`.
    pub fn repairable(&self) -> impl Iterator<Item = &ItemSlot> {
        self.equipped
            .iter()
            .flatten()
            .chain(self.bags.iter().flatten().map(|bag| &bag.item))
            .chain(self.backpack.iter().flatten())
            .chain(self.bags.iter().flatten().flat_map(|bag| bag.slots.iter().flatten()))
    }

    /// The slots of one container, by the bag id the interface uses: 0 the
    /// backpack, 1..4 the worn bags, -2 the key ring, -1 the bank's own
    /// squares, 5..10 the bags in the bank.
    ///
    /// Returns `None` for a bag id with no bag in it, which
    /// `GetContainerNumSlots` answers with 0. An empty slice would instead be a
    /// bag with no room. A bank square is returned with no banker nearby,
    /// because the fields are present whether or not one is. The game's
    /// `BankFrame` reads them the same way, and only the actions that move
    /// items are gated.
    pub fn container(&self, bag: i32) -> Option<&[Option<ItemSlot>]> {
        match bag {
            KEYRING_CONTAINER => Some(&self.keyring),
            BACKPACK_CONTAINER => Some(&self.backpack),
            BANK_CONTAINER => Some(&self.bank),
            1..=4 => self.bags[bag as usize - 1].as_ref().map(|b| b.slots.as_slice()),
            5..=10 => self.bank_bags[(bag - FIRST_BANK_BAG_ID) as usize]
                .as_ref()
                .map(|b| b.slots.as_slice()),
            _ => None,
        }
    }

    /// How many squares a container has, or `None` for a bag id with nothing
    /// in it. This is `GetContainerNumSlots` before its `unwrap_or(0)`.
    pub fn container_num(&self, bag: i32) -> Option<usize> {
        self.container(bag).map(<[_]>::len)
    }

    /// One slot of one container, one-based, which is how every
    /// `GetContainerItem*` call addresses it.
    pub fn container_item(&self, bag: i32, slot: usize) -> Option<&ItemSlot> {
        let slots = self.container(bag)?;
        slots.get(slot.checked_sub(1)?)?.as_ref()
    }

    /// The bag item in a bag id, for the name and icon of the bag itself. The
    /// backpack has none, because it is a field block rather than an object,
    /// so `GetBagName(0)` falls back to the game's own `BACKPACK_TOOLTIP`.
    pub fn bag_item(&self, bag: i32) -> Option<&ItemSlot> {
        match bag {
            1..=4 => self.bags[bag as usize - 1].as_ref().map(|b| &b.item),
            5..=10 => self.bank_bags[(bag - FIRST_BANK_BAG_ID) as usize]
                .as_ref()
                .map(|b| &b.item),
            _ => None,
        }
    }

    /// One worn slot by the interface's id: 1..19 worn, 20..23 the bags,
    /// 40..63 the bank's squares, 64..69 the bags in the bank. The last two
    /// ranges are what `BankButtonIDToInvSlotID` passes to every
    /// `GetInventoryItem*` call the bank frame makes.
    ///
    /// This is the one place the two numberings are converted, so that no
    /// caller subtracts one itself. An id outside the set answers `None`: 0
    /// (the ammo slot, which 1.12 declares and never fills) or the gap at
    /// 24..39.
    pub fn inventory_slot(&self, id: u32) -> Option<&ItemSlot> {
        match id {
            1..=19 => self.equipped[id as usize - 1].as_ref(),
            20..=23 | 64..=69 => self.bag_item(bag_id_of_inventory_slot(id)?),
            40..=63 => self.bank[(id - FIRST_BANK_INVENTORY_SLOT) as usize].as_ref(),
            _ => None,
        }
    }

    /// How many of an entry are carried, across every container and the worn
    /// slots. `GetItemCount`'s answer.
    pub fn count_of(&self, entry: u32) -> u32 {
        let containers = [KEYRING_CONTAINER, 0, 1, 2, 3, 4];
        let carried: u32 = containers
            .iter()
            .filter_map(|bag| self.container(*bag))
            .flatten()
            .flatten()
            .filter(|item| item.entry == entry)
            .map(|item| item.count)
            .sum();
        let worn: u32 = self
            .equipped
            .iter()
            .flatten()
            .filter(|item| item.entry == entry)
            .map(|item| item.count)
            .sum();
        carried + worn
    }

    /// Every item entry this inventory holds, bags included, without
    /// duplicates. This is the work list for `CMSG_ITEM_QUERY_SINGLE`.
    pub fn entries(&self) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        let mut push = |entry: u32| {
            if entry != 0 && !out.contains(&entry) {
                out.push(entry);
            }
        };
        for item in self.equipped.iter().flatten() {
            push(item.entry);
        }
        for bag in self.bags.iter().flatten().chain(self.bank_bags.iter().flatten()) {
            push(bag.item.entry);
            for item in bag.slots.iter().flatten() {
                push(item.entry);
            }
        }
        for item in self
            .backpack
            .iter()
            .flatten()
            .chain(self.keyring.iter().flatten())
            .chain(self.bank.iter().flatten())
        {
            push(item.entry);
        }
        out
    }
}

/// Where an item is, in whichever of the interface's two numberings addresses
/// it. [`Inventory::find_entry`] returns this, and every item action in the
/// client takes it.
///
/// There are two variants rather than one pair because the wire addresses the
/// two cases differently: a worn slot is `(SERVER_BAG_NONE, flat)` and a
/// carried one is `(the bag's own slot, position)`. [`server_inventory_slot`]
/// and [`server_container_slot`] are the two conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemPlace {
    /// A worn slot, by the interface's own id: 1..19 the equipment, 20..23 the
    /// four bag slots. [`Inventory::inventory_slot`]'s numbering.
    Worn(u32),
    /// A container slot: the bag id (0 the backpack, 1..4 the worn bags, -2 the
    /// ring) and a one-based position in it, which is how every
    /// `GetContainerItem*` call addresses one.
    Carried { bag: i32, slot: usize },
}

impl Inventory {
    /// Where the first copy of an entry is, in the 1.12.1 client's search
    /// order. An item on the action bar is resolved through this, because
    /// `SMSG_ACTION_BUTTONS` stores an item as its entry and nothing else.
    ///
    /// The order is the one `UseAction` uses. The client checks twice: first
    /// whether the entry is worn (the inventory walk limited to the
    /// equipment), and only if it is not does it search for a position to use.
    /// The general walk runs over the server's flat numbering in ascending
    /// order: 0..18 the equipment, 19..22 the bag slots, 23..38 the backpack,
    /// the ring last. It descends into a bag when it reaches that bag's own
    /// slot, so a bag's contents come before the backpack rather than after
    /// it.
    ///
    /// The order only decides which copy is used when a character carries
    /// several, so an error in it shows only in that case. A trinket that is
    /// both worn and carried is the case that matters: using the carried copy
    /// would try to equip a second one.
    ///
    /// `None` is an entry the character is not carrying at all, which is what
    /// a bar slot looks like when its item has been used up or sold.
    pub fn find_entry(&self, entry: u32) -> Option<ItemPlace> {
        if entry == 0 {
            return None;
        }
        let holds = |slot: &Option<ItemSlot>| slot.as_ref().is_some_and(|i| i.entry == entry);
        // The equipment first, on its own; see the two checks above.
        if let Some(id) = self.equipped.iter().position(holds) {
            return Some(ItemPlace::Worn(id as u32 + 1));
        }
        // Then each bag slot, and immediately after it whatever is inside.
        for (index, bag) in self.bags.iter().enumerate() {
            let Some(bag) = bag else { continue };
            if bag.item.entry == entry {
                return Some(ItemPlace::Worn(FIRST_BAG_INVENTORY_SLOT + index as u32));
            }
            if let Some(slot) = bag.slots.iter().position(holds) {
                return Some(ItemPlace::Carried {
                    bag: index as i32 + 1,
                    slot: slot + 1,
                });
            }
        }
        // Then the backpack, and the ring after it.
        for (bag, slots) in [
            (BACKPACK_CONTAINER, &self.backpack),
            (KEYRING_CONTAINER, &self.keyring),
        ] {
            if let Some(slot) = slots.iter().position(holds) {
                return Some(ItemPlace::Carried {
                    bag,
                    slot: slot + 1,
                });
            }
        }
        None
    }

    /// Whether this entry is worn. This answers `IsEquippedAction`, is the
    /// equipment-only half of the search above, and decides the green border
    /// round a bar button.
    pub fn is_equipped(&self, entry: u32) -> bool {
        entry != 0
            && self
                .equipped
                .iter()
                .flatten()
                .any(|item| item.entry == entry)
    }
}

/// Every container id, in the order a report walks them.
///
/// The key ring comes first because it is the one with a negative id and the
/// easiest to leave out of a range by accident. The list is here rather than
/// in a caller because both the renderer's diff and `vale live`'s `bags` walk
/// this set, and a second copy could leave one out.
pub const CONTAINERS_FOR_REPORT: [i32; 6] = [KEYRING_CONTAINER, BACKPACK_CONTAINER, 1, 2, 3, 4];

/// The bank's container ids, in report order: its own squares first, then the
/// six bags. Kept apart from [`CONTAINERS_FOR_REPORT`] because the two halves
/// raise different events: a bank square raises `PLAYERBANKSLOTS_CHANGED` and
/// a bag in the bank raises an ordinary `BAG_UPDATE`. A walker that wants both
/// chains the two lists.
pub const BANK_CONTAINERS_FOR_REPORT: [i32; 7] = [BANK_CONTAINER, 5, 6, 7, 8, 9, 10];

/// The bag id a bag slot corresponds to: Lua slot 20 is bag 1, and 64 (the
/// first bag slot in the bank) is bag 5.
///
/// This is `BagSlotButton_OnClick`'s arithmetic,
/// `id - CharacterBag0Slot:GetID() + 1`, written once here rather than at each
/// of its four call sites. The bank half is `BankFrameItemButtonBag_OnClick`'s
/// `ToggleBag(this:GetID())` on a button whose id is `NUM_BAG_SLOTS + n`.
pub fn bag_id_of_inventory_slot(id: u32) -> Option<i32> {
    let worn = FIRST_BAG_INVENTORY_SLOT..FIRST_BAG_INVENTORY_SLOT + BAG_SLOTS as u32;
    let bank = FIRST_BANK_BAG_INVENTORY_SLOT..FIRST_BANK_BAG_INVENTORY_SLOT + BANK_BAG_SLOTS as u32;
    if worn.contains(&id) {
        Some((id - FIRST_BAG_INVENTORY_SLOT + 1) as i32)
    } else if bank.contains(&id) {
        Some((id - FIRST_BANK_BAG_INVENTORY_SLOT) as i32 + FIRST_BANK_BAG_ID)
    } else {
        None
    }
}

/// The reverse of [`bag_id_of_inventory_slot`]: bag id 1 is inventory slot 20,
/// and bag id 5 is 64.
pub fn inventory_slot_of_bag_id(bag: i32) -> Option<u32> {
    if (1..=BAG_SLOTS as i32).contains(&bag) {
        Some(FIRST_BAG_INVENTORY_SLOT + bag as u32 - 1)
    } else if (FIRST_BANK_BAG_ID..FIRST_BANK_BAG_ID + BANK_BAG_SLOTS as i32).contains(&bag) {
        Some(FIRST_BANK_BAG_INVENTORY_SLOT + (bag - FIRST_BANK_BAG_ID) as u32)
    } else {
        None
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
/// [`EQUIPMENT_SLOTS`] for what a slot index means.
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

/// `EQUIPMENT_SLOT_MAINHAND`: 15, the slot every weapon imbue, poison and
/// sharpening stone applies to. vmangos names the same constant and comments
/// the client flag beside it: "Client automatically selects item from
/// mainhand slot as a cast target".
pub const EQUIPMENT_SLOT_MAINHAND: usize = 15;

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

// ---------------------------------------------------------------------------
// SMSG_INVENTORY_CHANGE_FAILURE: the reason an item action was refused
// ---------------------------------------------------------------------------

/// `SMSG_INVENTORY_CHANGE_FAILURE`: the refusal shared by every item action.
///
/// ```text
/// u8  code                       an index into INVENTORY_FAILURE_KEYS
/// u32 requiredLevel              CANT_EQUIP_LEVEL_I only
/// u64 item, u64 otherItem        the guids the refusal is about
/// u8  bagSubclass                the two WRONG_BAG_TYPE codes only
/// ```
///
/// The whole body after the first byte is absent when the code is
/// `EQUIP_ERR_OK`, and the level word is present for exactly one code. See
/// `Player::SendEquipError`, which sizes the packet at 22, 18 or 1 bytes for
/// the three cases. Reading the tail unconditionally makes a one-byte packet a
/// parse failure, and a refusal that fails to parse leaves the player with no
/// feedback, which is the problem the packet is sent to solve.
///
/// This packet is the only thing on the wire that answers a right-click the
/// server has rejected. A potion eleven levels too high goes
/// `HandleUseItemOpcode` -> `Player::CanUseItem` ->
/// `EQUIP_ERR_CANT_EQUIP_LEVEL_I`. No cast is prepared, so no
/// `SMSG_CAST_RESULT` is sent and nothing else in the session changes.
/// Without handling this packet, the click looks the same as a click on empty
/// space.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InventoryFailure {
    /// `InventoryResult`, and an index into [`INVENTORY_FAILURE_KEYS`].
    pub code: u8,
    /// The level the item requires. Only [`EQUIP_ERR_CANT_EQUIP_LEVEL_I`] sets
    /// it; its string is the only one in the family with a `%d`.
    pub required_level: u32,
    /// The item the refusal is about, or 0. It is not used to name the item,
    /// because the message text is the interface's job, but it distinguishes
    /// two refusals about two stacks in the same second.
    pub item: u64,
    pub other_item: u64,
}

/// `EQUIP_ERR_OK`: sent as a one-byte packet, and not an error.
///
/// `HandleUseItemOpcode`'s failure path sends `EQUIP_ERR_NONE` first ("free
/// gray item after use fail"), which is a different code with no string. This
/// code is the success acknowledgement that several actions end with.
pub const EQUIP_ERR_OK: u8 = 0;

/// The one code that carries a number, and the reason this packet has two
/// shapes.
pub const EQUIP_ERR_CANT_EQUIP_LEVEL_I: u8 = 1;

/// `InventoryResult` in declaration order, as `GlobalStrings.lua` keys.
///
/// This has the same shape as [`crate::play::spells::CAST_FAILURE_KEYS`], for
/// the same reason: the wire carries a byte that indexes an enum, and the
/// enum's entries name keys in a file the archives ship. An off-by-one here
/// does not fail; it shows "That bag is full." when the server said "You must
/// reach level 45 to use that item."
///
/// Transcribed from vmangos' `InventoryResult` (`Objects/ItemDefines.h`),
/// whose comment column gives the key for each entry. Every `#if` guard in
/// that header is `> CLIENT_BUILD_1_x` for an x below 1.12, so 1.12.1 gets all
/// 67 entries and declaration order is the wire value.
///
/// Two entries differ from vmangos' comment on purpose.
/// `EQUIP_ERR_BANK_FULL` (51) is commented `ERR_BAG_FULL` there and is
/// `ERR_BANK_FULL` here: `GlobalStrings.lua` ships a separate `ERR_BANK_FULL`
/// as "Your bank is full", so the vmangos comment mislabels an enumerator
/// whose own name says otherwise.
/// `EQUIP_ERR_NONE` (59) is `ERR_CANT_BE_DISENCHANTED`, which
/// `GlobalStrings.lua` does not carry, so it displays nothing. That is
/// correct: the server sends this code to release a grey item after a failed
/// use, immediately before the real reason.
pub const INVENTORY_FAILURE_KEYS: [&str; 67] = [
    "",                                // 0  EQUIP_ERR_OK, not a failure
    "ERR_CANT_EQUIP_LEVEL_I",          // 1  You must reach level %d to use that item.
    "ERR_CANT_EQUIP_SKILL",            // 2  You aren't skilled enough to use that item.
    "ERR_WRONG_SLOT",                  // 3  That item does not go in that slot.
    "ERR_BAG_FULL",                    // 4
    "ERR_BAG_IN_BAG",                  // 5  Can't put non-empty bags in other bags.
    "ERR_TRADE_EQUIPPED_BAG",          // 6
    "ERR_AMMO_ONLY",                   // 7
    "ERR_PROFICIENCY_NEEDED",          // 8
    "ERR_NO_SLOT_AVAILABLE",           // 9
    "ERR_CANT_EQUIP_EVER",             // 10
    "ERR_CANT_EQUIP_EVER",             // 11 the enum has two of these
    "ERR_NO_SLOT_AVAILABLE",           // 12
    "ERR_2HANDED_EQUIPPED",            // 13
    "ERR_2HSKILLNOTFOUND",             // 14 You cannot dual-wield
    "ERR_WRONG_BAG_TYPE",              // 15
    "ERR_WRONG_BAG_TYPE",              // 16
    "ERR_ITEM_MAX_COUNT",              // 17
    "ERR_NO_SLOT_AVAILABLE",           // 18
    "ERR_CANT_STACK",                  // 19
    "ERR_NOT_EQUIPPABLE",              // 20
    "ERR_CANT_SWAP",                   // 21
    "ERR_SLOT_EMPTY",                  // 22
    "ERR_ITEM_NOT_FOUND",              // 23
    "ERR_DROP_BOUND_ITEM",             // 24
    "ERR_OUT_OF_RANGE",                // 25
    "ERR_TOO_FEW_TO_SPLIT",            // 26
    "ERR_SPLIT_FAILED",                // 27
    "ERR_SPELL_FAILED_REAGENTS_GENERIC", // 28
    "ERR_NOT_ENOUGH_MONEY",            // 29
    "ERR_NOT_A_BAG",                   // 30
    "ERR_DESTROY_NONEMPTY_BAG",        // 31
    "ERR_NOT_OWNER",                   // 32
    "ERR_ONLY_ONE_QUIVER",             // 33
    "ERR_NO_BANK_SLOT",                // 34
    "ERR_NO_BANK_HERE",                // 35
    "ERR_ITEM_LOCKED",                 // 36
    "ERR_GENERIC_STUNNED",             // 37
    "ERR_PLAYER_DEAD",                 // 38
    "ERR_CLIENT_LOCKED_OUT",           // 39
    "ERR_INTERNAL_BAG_ERROR",          // 40
    "ERR_ONLY_ONE_BOLT",               // 41
    "ERR_ONLY_ONE_AMMO",               // 42
    "ERR_CANT_WRAP_STACKABLE",         // 43
    "ERR_CANT_WRAP_EQUIPPED",          // 44
    "ERR_CANT_WRAP_WRAPPED",           // 45
    "ERR_CANT_WRAP_BOUND",             // 46
    "ERR_CANT_WRAP_UNIQUE",            // 47
    "ERR_CANT_WRAP_BAGS",              // 48
    "ERR_LOOT_GONE",                   // 49
    "ERR_INV_FULL",                    // 50
    "ERR_BANK_FULL",                   // 51 see the note above
    "ERR_VENDOR_SOLD_OUT",             // 52
    "ERR_BAG_FULL",                    // 53
    "ERR_ITEM_NOT_FOUND",              // 54
    "ERR_CANT_STACK",                  // 55
    "ERR_BAG_FULL",                    // 56
    "ERR_VENDOR_SOLD_OUT",             // 57
    "ERR_OBJECT_IS_BUSY",              // 58
    "ERR_CANT_BE_DISENCHANTED",        // 59 not in GlobalStrings: shows nothing
    "ERR_NOT_IN_COMBAT",               // 60
    "ERR_NOT_WHILE_DISARMED",          // 61
    "ERR_BAG_FULL",                    // 62
    "ERR_CANT_EQUIP_RANK",             // 63
    "ERR_CANT_EQUIP_REPUTATION",       // 64
    "ERR_TOO_MANY_SPECIAL_BAGS",       // 65
    "ERR_LOOT_CANT_LOOT_THAT_NOW",     // 66
];

/// The `GlobalStrings.lua` key for one of those codes, or `None` for the
/// success code and for anything the table does not carry.
///
/// A code past the end maps to `ERR_BAG_FULL`. That is the 1.12.1 client's
/// behaviour, not a fallback chosen here: `ItemDefines.h` ends with
/// `// any greater values show as "bag full"`. vmangos and this client are two
/// halves of a table that grew in later builds, so an unknown code is a newer
/// code, and the client already defines how to show one.
pub fn inventory_failure_key(code: u8) -> Option<&'static str> {
    if code == EQUIP_ERR_OK {
        return None;
    }
    let key = INVENTORY_FAILURE_KEYS
        .get(usize::from(code))
        .copied()
        .unwrap_or("ERR_BAG_FULL");
    (!key.is_empty()).then_some(key)
}

/// `SMSG_ITEM_PUSH_RESULT`: an item has arrived in a bag. It is the one packet
/// that says so, whatever the source.
///
/// Everything else about the inventory is update fields, and update fields
/// state the current value, not the event that changed it. A stack that grew
/// by three is `ITEM_FIELD_STACK_COUNT` changing, with nothing to say whether
/// the items were looted, bought, crafted, mailed or traded. This packet is
/// the event, and `Player::SendNewItem` has twenty callers covering all of
/// those cases.
///
/// ```text
/// u64 playerGuid       whose bag — a group broadcast names the other player
/// u32 received         0 looted, 1 from an NPC
/// u32 created          0 received, 1 created
/// u32 showInChat
/// u8  bagSlot
/// u32 slot             0xFFFFFFFF when it went onto an existing stack
/// u32 itemId
/// u32 suffixFactor
/// u32 randomPropertyId
/// u32 count
/// ```
///
/// The last two fields exist only in builds above 1.10.2, which 5875 is: the
/// `#if SUPPORTED_CLIENT_BUILD` guards in `SendNewItem` bracket the slot and
/// the count. A reader written against the older layout is two fields short
/// and reports every stack as one item.
///
/// For loot the packet is broadcast to the group, with `broadcast = true` at
/// `LootHandler.cpp:236`. That is why the guid is in the body, and why it has
/// to be checked: a client that skips the check announces every party
/// member's loot as its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPush {
    /// Whose bag it landed in. Not necessarily the local player's; see above.
    pub guid: u64,
    /// `0` looted, `1` handed over by an NPC. Chooses between the game's
    /// `LOOT_ITEM_SELF` and `LOOT_ITEM_PUSHED_SELF` lines.
    pub received: bool,
    /// `1` when the item was made rather than given: `LOOT_ITEM_CREATED_SELF`.
    /// Takes precedence over [`Self::received`], which a crafted item also
    /// has.
    pub created: bool,
    /// The server's "show this in the chat frame" switch. Quest rewards turn
    /// it off (`Player.cpp:14381` passes `showInChat = false`), which is why
    /// the quest-reward window does not also produce a loot line.
    pub show_in_chat: bool,
    pub bag_slot: u8,
    /// The slot inside that bag, or `None` when the item went onto an existing
    /// stack (the wire's `0xFFFFFFFF`).
    pub slot: Option<u32>,
    pub item_id: u32,
    pub suffix_factor: u32,
    pub random_property: u32,
    /// How many arrived. The `_MULTIPLE` line is the one to use above 1.
    pub count: u32,
}

pub fn parse_item_push_result(body: &[u8]) -> Option<ItemPush> {
    let mut r = crate::bytes::Reader::new(body);
    if !r.has(8 + 4 + 4 + 4 + 1 + 4 + 4 + 4 + 4 + 4) {
        return None;
    }
    let guid = r.u64();
    let received = r.u32() != 0;
    let created = r.u32() != 0;
    let show_in_chat = r.u32() != 0;
    let bag_slot = r.u8();
    let slot = match r.u32() {
        u32::MAX => None,
        slot => Some(slot),
    };
    Some(ItemPush {
        guid,
        received,
        created,
        show_in_chat,
        bag_slot,
        slot,
        item_id: r.u32(),
        suffix_factor: r.u32(),
        random_property: r.u32(),
        count: r.u32(),
    })
}

impl ItemPush {
    /// The `GlobalStrings` key for the chat line this item produces, or `None`
    /// when the server asked for no line.
    ///
    /// There are six keys, three cases times singular and plural, and the game
    /// ships all six:
    ///
    /// ```text
    /// LOOT_ITEM_CREATED_SELF   "You create: %s."
    /// LOOT_ITEM_PUSHED_SELF    "You receive item: %s."
    /// LOOT_ITEM_SELF           "You receive loot: %s."
    /// ```
    ///
    /// No file in `Interface\FrameXML\` uses any of them. The strings exist,
    /// they take exactly the arguments this body carries, and none of the
    /// ninety Lua files the `.toc` loads mentions them, so the 1.12.1 client
    /// composes the line itself from this packet.
    pub fn line_key(&self) -> Option<&'static str> {
        if !self.show_in_chat {
            return None;
        }
        let multiple = self.count > 1;
        Some(match (self.created, self.received, multiple) {
            (true, _, false) => "LOOT_ITEM_CREATED_SELF",
            (true, _, true) => "LOOT_ITEM_CREATED_SELF_MULTIPLE",
            (false, true, false) => "LOOT_ITEM_PUSHED_SELF",
            (false, true, true) => "LOOT_ITEM_PUSHED_SELF_MULTIPLE",
            (false, false, false) => "LOOT_ITEM_SELF",
            (false, false, true) => "LOOT_ITEM_SELF_MULTIPLE",
        })
    }
}

/// Parse `SMSG_INVENTORY_CHANGE_FAILURE`.
///
/// A one-byte body is the success acknowledgement and parses to an
/// [`InventoryFailure`] holding [`EQUIP_ERR_OK`], not to `None`. In this crate
/// `None` means "the packet was malformed", and it is counted as a read
/// failure on the HUD.
pub fn parse_inventory_change_failure(body: &[u8]) -> Option<InventoryFailure> {
    let mut r = crate::bytes::Reader::new(body);
    let code = r.has(1).then(|| r.u8())?;
    let mut failure = InventoryFailure {
        code,
        ..InventoryFailure::default()
    };
    if code == EQUIP_ERR_OK {
        return Some(failure);
    }
    if code == EQUIP_ERR_CANT_EQUIP_LEVEL_I {
        if !r.has(4) {
            return None;
        }
        failure.required_level = r.u32();
    }
    // The guids are optional here, though always present on the wire. vmangos
    // always writes them, but they are the tail and this client uses neither,
    // so a short body still yields the code, which is the whole message.
    if r.has(16) {
        failure.item = r.u64();
        failure.other_item = r.u64();
    }
    Some(failure)
}


/// `SMSG_ENCHANTMENTLOG`: an item was enchanted, or an enchantment faded.
///
/// `{u64 caster, u64 owner, u32 item_entry, u32 spell_id, u8 show_affiliation}`,
/// with full guids rather than packed ones. The last byte is only read when
/// there is a caster.
///
/// A zero caster means a fade, not a bad packet. vmangos' comment on the field
/// is "enchanter; empty means enchant has faded", and the two cases use
/// different `GlobalStrings` keys: `ITEMENCHANTMENTADD*` for an application
/// and `ITEMENCHANTMENTREMOVE*` for a fade. A reader that treats the guid as
/// mandatory loses every expiry line.
///
/// This is the only packet that reports an enchantment being applied. The
/// item's `ITEM_FIELD_ENCHANTMENT` fields change in an update block with no
/// announcement, and the trade window's echo only reports what is proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnchantmentLog {
    /// Zero means the enchant faded rather than that somebody applied it.
    pub caster: u64,
    pub owner: u64,
    pub item_entry: u32,
    /// The enchanting spell, whose name the line prints.
    pub spell_id: u32,
    /// Only meaningful when [`Self::caster`] is set.
    pub show_affiliation: bool,
}

impl EnchantmentLog {
    /// Whether this is an enchant fading rather than one being applied.
    pub fn faded(&self) -> bool {
        self.caster == 0
    }
}

pub fn parse_enchantment_log(body: &[u8]) -> Option<EnchantmentLog> {
    let mut r = Reader::new(body);
    if !r.has(8 + 8 + 4 + 4) {
        return None;
    }
    let caster = r.u64();
    let owner = r.u64();
    let item_entry = r.u32();
    let spell_id = r.u32();
    // The trailing byte is the last field, and a packet without it is not
    // broken: read it if present.
    let show_affiliation = r.has(1) && r.u8() != 0;
    Some(EnchantmentLog { caster, owner, item_entry, spell_id, show_affiliation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::objects::ObjectManager;
    use crate::state::update::{ObjectType, ObjectUpdate, UpdateBlock, ValuesUpdate};

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

    /// The server's (bag, slot) numbering, checked against
    /// `Player::GetItemByPos`.
    ///
    /// Bag id 1's first slot is `(19, 0)` and the backpack's is `(255, 23)`:
    /// two different bags and two different bases, which is why the conversion
    /// is done here rather than at a call site. Getting either half wrong names
    /// a neighbouring item, and the server accepts it.
    #[test]
    fn a_container_slot_crosses_into_the_servers_own_pair() {
        // The backpack: no bag, and the flat run starts at INVENTORY_SLOT_ITEM_START.
        assert_eq!(server_container_slot(BACKPACK_CONTAINER, 1), Some((255, 23)));
        assert_eq!(server_container_slot(BACKPACK_CONTAINER, 16), Some((255, 38)));
        assert_eq!(
            server_container_slot(BACKPACK_CONTAINER, 17),
            None,
            "one past the backpack is the first bank slot, not a seventeenth pocket"
        );
        // A worn bag: the bag is its own inventory slot, and the slot inside
        // it counts from zero.
        assert_eq!(server_container_slot(1, 1), Some((19, 0)));
        assert_eq!(server_container_slot(4, 3), Some((22, 2)));
        // The key ring, whose base is 81: KEYRING_SLOT_END is 97 and the ring
        // is sixteen guids wide.
        assert_eq!(server_container_slot(KEYRING_CONTAINER, 1), Some((255, 81)));
        assert_eq!(server_container_slot(KEYRING_CONTAINER, 16), Some((255, 96)));
        // Zero is not a slot: the interface counts from one, and a `checked_sub`
        // rather than a saturating one is what keeps a stray 0 off the wire.
        assert_eq!(server_container_slot(0, 0), None);
        assert_eq!(server_container_slot(11, 1), None, "four worn bags and six in the bank");
    }

    /// The worn run, which is one subtraction and always the same bag.
    #[test]
    fn a_worn_slot_is_the_players_own_object() {
        assert_eq!(server_inventory_slot(1), Some((255, 0)), "the head");
        assert_eq!(server_inventory_slot(19), Some((255, 18)), "the tabard");
        assert_eq!(server_inventory_slot(20), Some((255, 19)), "the first bag");
        assert_eq!(server_inventory_slot(23), Some((255, 22)));
        assert_eq!(server_inventory_slot(0), None, "the ammo slot is never filled");
        assert_eq!(set_ammo_body(2512), vec![0xd0, 0x09, 0, 0], "CMSG_SET_AMMO is one u32");
        assert_eq!(set_ammo_body(0), vec![0, 0, 0, 0], "…and zero unloads");
        assert_eq!(server_inventory_slot(24), None);
    }

    /// `CMSG_USE_ITEM`'s three bytes and its target block, checked against
    /// `HandleUseItemOpcode`'s read order.
    #[test]
    fn a_use_names_the_slot_the_spell_index_and_whom() {
        // A potion in the backpack's first slot, no target: three bytes and an
        // empty mask, which is what lets the server aim it from the spell.
        assert_eq!(
            use_item_body(255, 23, 0, crate::play::spells::CastTarget::SelfImplicit),
            vec![255, 23, 0, 0, 0]
        );
        // A bandage on the selection, which uses the ordinary unit block.
        let aimed = use_item_body(19, 2, 1, crate::play::spells::CastTarget::Unit(0xF130_0000_0001_2345));
        assert_eq!(&aimed[..3], &[19, 2, 1]);
        assert_eq!(
            &aimed[3..5],
            &crate::play::spells::target_flag::UNIT.to_le_bytes(),
            "the same mask CMSG_CAST_SPELL sends"
        );
        // The equip form carries the pair and stops.
        assert_eq!(auto_equip_body(255, 25), vec![255, 25]);
        // `CMSG_OPEN_ITEM` sends the same two bytes to a different opcode,
        // which is why the two are easy to confuse.
        assert_eq!(open_item_body(255, 25), vec![255, 25]);
        assert_eq!(open_item_body(19, 3), vec![19, 3]);
    }

    /// Every entry carried, once each: the work list the query pass sends.
    #[test]
    fn the_entry_list_covers_every_layer_and_deduplicates() {
        let mut entries = Inventory::read(&world_with_a_bag()).entries();
        entries.sort_unstable();
        assert_eq!(entries, vec![858, 2589, 4500, 12640]);
    }

    /// `SMSG_INVENTORY_CHANGE_FAILURE` has three shapes, and one of them is a
    /// single byte. `Player::SendEquipError` sizes the packet at 22, 18 or 1,
    /// and reading the tail unconditionally turns the success acknowledgement
    /// into a parse failure. That reaches the HUD as a warning about a packet
    /// that was well formed.
    #[test]
    fn a_refusal_reads_its_one_optional_word_and_no_more() {
        // The success form: one byte, no tail.
        let ok = parse_inventory_change_failure(&[EQUIP_ERR_OK]).expect("a one-byte body parses");
        assert_eq!(ok.code, EQUIP_ERR_OK);
        assert_eq!(ok.required_level, 0);

        // The level form, which is the only one that carries a number.
        let mut w = crate::bytes::Writer::new();
        w.u8(EQUIP_ERR_CANT_EQUIP_LEVEL_I).u32(45).u64(7).u64(0);
        let level = parse_inventory_change_failure(&w.buf).expect("the 22-byte form parses");
        assert_eq!(level.required_level, 45, "the *item's* level, not ours");
        assert_eq!(level.item, 7);

        // Every other code: the body starts with the guids. Reading the level
        // word here would take the low half of the item guid for it.
        let mut w = crate::bytes::Writer::new();
        w.u8(23).u64(0xABCD).u64(0);
        let other = parse_inventory_change_failure(&w.buf).expect("the 18-byte form parses");
        assert_eq!(other.required_level, 0);
        assert_eq!(other.item, 0xABCD);

        assert_eq!(parse_inventory_change_failure(&[]), None);
    }

    /// The table is indexed by code, and its first and last entries confirm
    /// the indexing.
    ///
    /// An off-by-one here shows "That bag is full." where the server said "You
    /// must reach level 45 to use that item."
    #[test]
    fn a_refusal_code_names_the_games_own_string() {
        assert_eq!(inventory_failure_key(0), None, "success is not a message");
        assert_eq!(
            inventory_failure_key(EQUIP_ERR_CANT_EQUIP_LEVEL_I),
            Some("ERR_CANT_EQUIP_LEVEL_I")
        );
        assert_eq!(inventory_failure_key(23), Some("ERR_ITEM_NOT_FOUND"));
        assert_eq!(inventory_failure_key(50), Some("ERR_INV_FULL"));
        // The last entry of the 1.12 enum, which checks that the four `#if`
        // guards were all resolved the way 5875 resolves them.
        assert_eq!(
            inventory_failure_key(66),
            Some("ERR_LOOT_CANT_LOOT_THAT_NOW")
        );
        // Past the end is "bag full", which is the client's own rule rather
        // than a fallback chosen here: `ItemDefines.h` says so in its last
        // line, and an unknown code is a code from a later build.
        assert_eq!(inventory_failure_key(200), Some("ERR_BAG_FULL"));
        // The one code with no string, which is how the client shows nothing
        // for the grey-item release that precedes a real reason.
        assert_eq!(
            INVENTORY_FAILURE_KEYS[59], "ERR_CANT_BE_DISENCHANTED",
            "not in GlobalStrings.lua, and deliberately so"
        );
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

    #[test]
    fn the_bank_crosses_into_the_wire_beside_the_bags() {
        assert_eq!(server_container_slot(BANK_CONTAINER, 1), Some((SERVER_BAG_NONE, 39)));
        assert_eq!(server_container_slot(BANK_CONTAINER, 24), Some((SERVER_BAG_NONE, 62)));
        assert_eq!(server_container_slot(BANK_CONTAINER, 25), None);
        assert_eq!(server_container_slot(5, 1), Some((63, 0)));
        assert_eq!(server_container_slot(10, 2), Some((68, 1)));
        assert_eq!(server_inventory_slot(40), Some((SERVER_BAG_NONE, 39)));
        assert_eq!(server_inventory_slot(69), Some((SERVER_BAG_NONE, 68)));
        assert_eq!(server_inventory_slot(24), None, "the gap between the bags and the bank");
        assert_eq!(server_inventory_slot(70), None);
        assert_eq!(bag_id_of_inventory_slot(64), Some(5));
        assert_eq!(bag_id_of_inventory_slot(69), Some(10));
        assert_eq!(bag_id_of_inventory_slot(63), None);
        assert_eq!(inventory_slot_of_bag_id(5), Some(64));
        assert_eq!(inventory_slot_of_bag_id(10), Some(69));
        assert_eq!(inventory_slot_of_bag_id(11), None);
        // `IsBankPos`, all three of its clauses.
        assert!(is_bank_position(SERVER_BAG_NONE, 39));
        assert!(is_bank_position(SERVER_BAG_NONE, 68));
        assert!(is_bank_position(65, 3));
        assert!(!is_bank_position(SERVER_BAG_NONE, 38));
        assert!(!is_bank_position(SERVER_BAG_NONE, 69));
        assert!(!is_bank_position(19, 0));
    }
    /// Two full guids, two dwords and a byte. A zero caster is a fade rather
    /// than a bad packet. See [`EnchantmentLog`].
    #[test]
    fn an_enchantment_log_tells_an_application_from_a_fade() {
        let body = |caster: u64, owner: u64, entry: u32, spell: u32, show: u8| {
            let mut out = Vec::new();
            out.extend_from_slice(&caster.to_le_bytes());
            out.extend_from_slice(&owner.to_le_bytes());
            out.extend_from_slice(&entry.to_le_bytes());
            out.extend_from_slice(&spell.to_le_bytes());
            out.push(show);
            out
        };
        let applied = parse_enchantment_log(&body(0x11, 0x22, 12345, 7218, 1)).expect("applied");
        assert_eq!((applied.caster, applied.owner), (0x11, 0x22));
        assert_eq!((applied.item_entry, applied.spell_id), (12345, 7218));
        assert!(applied.show_affiliation);
        assert!(!applied.faded());
        let faded = parse_enchantment_log(&body(0, 0x22, 12345, 7218, 0)).expect("faded");
        assert!(faded.faded(), "an empty caster is the expiry line");
        // The trailing byte is the last field, and a packet without it is not
        // broken. This is the one length decision in this parser.
        let short = &body(0x11, 0x22, 1, 2, 0)[..24];
        assert!(parse_enchantment_log(short).is_some());
        assert!(parse_enchantment_log(&short[..23]).is_none());
    }

}
