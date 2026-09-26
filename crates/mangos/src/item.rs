//! `item_template`: what an item is, what it does, and what it is worth.
//!
//! The third table in this crate with no client file behind it. `Item.dbc` is
//! not in the 1.12 archives at all — the client asks the server
//! (`CMSG_ITEM_QUERY_SINGLE`) and is told a display id, an inventory type, a
//! name and the rest of the plate — so the row in vmangos' database is the only
//! copy of an item there is, exactly as a creature's template is.
//!
//! What the *client* holds is the other half of one column.
//! `ItemDisplayInfo.dbc` is what `display_id` points at: the models a weapon or
//! a helm hangs on the wearer, the eight body textures a garment paints, the
//! icon in the bag. That file is `vale-assets`' and is read there
//! ([`vale_assets::tables::item`]); this module states only that the column
//! *is* a row of it, so a form can offer the picture rather than the number.
//!
//! ## 129 columns, in `LoadItemPrototypes`' own order
//!
//! [`TEMPLATE_COLUMNS`] is the `SELECT` at `ObjectMgr.cpp:3794`, column for
//! column and in its order, and the order is kept for the reason the creature
//! schema keeps its own: it is the list to compare against the source when
//! vmangos adds a column, and a schema that has drifted then shows up as a name
//! in the wrong place rather than as a name that is merely missing.
//!
//! ## The patch column is part of the key, exactly as it is for a creature
//!
//! `item_template`'s primary key is `(entry, patch)` and
//! `ObjectMgr::LoadItemPrototypes` takes, for each entry, the row with the
//! highest `patch` at or below the server's configured `WowPatch`:
//!
//! ```sql
//! WHERE `patch` = (SELECT max(`patch`) FROM `item_template` t2
//!                  WHERE t1.`entry` = t2.`entry` && `patch` <= %u)
//! ```
//!
//! So an `UPDATE … WHERE entry = 2589` with no patch would change every
//! content-patch version of that item, including the ones the running server is
//! not using — a change that looks applied, reloads cleanly, and does nothing.
//! [`winning_template_query`] is how the row the server would load is found, and
//! the key is a [`Key`] of two columns.
//!
//! ## An item is the first subject here that a reload makes live
//!
//! `creature_template` cannot be reloaded usefully and `creature` cannot be
//! reloaded at all — both need a restart, which is why the creature panel says
//! so instead of offering a button that would lie.
//!
//! `.reload item_template` is different and it is different in the one way that
//! matters. `HandleReloadItemTemplate` calls `ObjectMgr::LoadItemPrototypes`,
//! which **clears the whole map** before it reads (`ObjectMgr.cpp:3792`), and
//! `Item::GetProto` is a lookup by entry on every call
//! (`Item.cpp:560`) rather than a pointer held from when the item was created.
//! So an edited row is live for every item already in a bag, in the world and
//! on the auction house, with no restart and no relog.
//!
//! ## A removal is live on a restart and never on a reload
//!
//! The same reload is what makes a removal dangerous. An `Item` whose prototype
//! is gone answers `nullptr` from `GetProto()`, and the callers of it —
//! `isWeapon`, `IsBag`, `GetMaxStackCount` — dereference it without asking. A
//! reload after a removal leaves every copy of the item already loaded in the
//! world in that state, and an item in the bank of a character who is logged
//! in is enough.
//!
//! A restart does not. Each loader that builds an `Item` out of the
//! `characters` database asks for the prototype first and deletes the item
//! when there is none: `Player::_LoadInventory` (`Player.cpp:16811`), the
//! mailed items (`MasterPlayer.cpp:238`), `Item::LoadLootFromDB`
//! (`Item.cpp:534`) and the auction house (`AuctionHouseMgr.cpp:373`). So a
//! removed item is **deleted from every character who carries it** on the next
//! start, which is what removing an item means and is said on the button.
//!
//! [`can_live`] therefore allows a removal, [`reload_is_safe`] is what an apply
//! asks before it sends `.reload item_template`, and [`delete_statements`]
//! takes every content-patch version with the rows of [`DEPENDENTS`], which are
//! the rows that exist to describe the item or to hand it out.
//!
//! ## An item's `entry` may be changed, and what names it follows
//!
//! Renumbering an item is a thing people want: an entry read off a wiki, a
//! block of ids kept tidy, a row moved out of a range somebody else's patch is
//! about to use. To the project's store it is a re-key of the row's one claim,
//! which records where the database has the row — see
//! [`crate::row::RowEdit::from`]. To the database it is
//! [`crate::row::move_statements`]: the row, every other content-patch version
//! of it, and one `UPDATE` per entry of [`REFERENCES`].
//!
//! vmangos has no foreign keys and no cascade, so [`REFERENCES`] is the cascade:
//! every column of the world database that names an item by entry. **What it
//! cannot reach is stated rather than hidden**: the `characters` database
//! (`item_instance`, `character_inventory`, the mail and auction tables), which
//! is another connection, and `Spell.dbc`'s reagent and created-item fields,
//! which are a client file. A copy of a moved item that a character already
//! carries has no prototype after the move, exactly as it would after a
//! removal.
//!
//! ## What this module does not decide
//!
//! Which of the 129 columns the *client* would have an opinion about. An item's
//! quality colour, its class and subclass words, and the icon it is drawn with
//! are all in the archives (`ItemClass.dbc`, `ItemSubClass.dbc`,
//! `ItemDisplayInfo.dbc`), and reading them is `vale-assets`' job on this
//! crate's own rule — see the crate comment. The enumerations below are
//! vmangos' own headers, which is what the *server* believes, and they are
//! named as such.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{
    mask_words, millis_words, money_words, seconds_words, value_word, Bit, Column, Group, Kind,
    Row, RowValue, Value,
};

/// What an item is.
pub const TEMPLATE: &str = "item_template";

/// Every table this module writes, for a caller that has to resolve a name read
/// out of a file back to one of these constants — see
/// [`crate::creature::TABLES`], where the reason is.
pub const TABLES: [&str; 1] = [TEMPLATE];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

// ---------------------------------------------------------------------------
// The enumerations, each from vmangos' own header
// ---------------------------------------------------------------------------

/// `ItemQualities`, from `SharedDefines.h:192`. The colour a name is drawn in.
pub const QUALITIES: [Value; 7] = [
    Value { value: 0, name: "Poor (grey)" },
    Value { value: 1, name: "Common (white)" },
    Value { value: 2, name: "Uncommon (green)" },
    Value { value: 3, name: "Rare (blue)" },
    Value { value: 4, name: "Epic (purple)" },
    Value { value: 5, name: "Legendary (orange)" },
    Value { value: 6, name: "Artifact (yellow)" },
];

/// `ItemClass`, from `ItemPrototype.h:135`. The same values as `ItemClass.dbc`,
/// which is what the client draws the word from.
pub const CLASSES: [Value; 16] = [
    Value { value: 0, name: "Consumable" },
    Value { value: 1, name: "Container" },
    Value { value: 2, name: "Weapon" },
    Value { value: 3, name: "Gem" },
    Value { value: 4, name: "Armor" },
    Value { value: 5, name: "Reagent" },
    Value { value: 6, name: "Projectile" },
    Value { value: 7, name: "Trade Goods" },
    Value { value: 8, name: "Generic" },
    Value { value: 9, name: "Recipe" },
    Value { value: 10, name: "Money" },
    Value { value: 11, name: "Quiver" },
    Value { value: 12, name: "Quest" },
    Value { value: 13, name: "Key" },
    Value { value: 14, name: "Permanent" },
    Value { value: 15, name: "Junk" },
];

/// `ItemSubclassConsumable` — one value in 1.12; the rest arrive in 2.0.
pub const SUBCLASS_CONSUMABLE: [Value; 1] = [Value { value: 0, name: "Consumable" }];

/// `ItemSubclassContainer`, from `ItemPrototype.h:174`.
pub const SUBCLASS_CONTAINER: [Value; 4] = [
    Value { value: 0, name: "Bag" },
    Value { value: 1, name: "Soul Bag" },
    Value { value: 2, name: "Herb Bag" },
    Value { value: 3, name: "Enchanting Bag" },
];

/// `ItemSubclassWeapon`, from `ItemPrototype.h:190`. 9, 11 and 12 are the
/// table's own `obsolete` and `exotic` entries and are kept, because a row
/// holding one is a row worth being able to see as itself.
pub const SUBCLASS_WEAPON: [Value; 21] = [
    Value { value: 0, name: "Axe (one-hand)" },
    Value { value: 1, name: "Axe (two-hand)" },
    Value { value: 2, name: "Bow" },
    Value { value: 3, name: "Gun" },
    Value { value: 4, name: "Mace (one-hand)" },
    Value { value: 5, name: "Mace (two-hand)" },
    Value { value: 6, name: "Polearm" },
    Value { value: 7, name: "Sword (one-hand)" },
    Value { value: 8, name: "Sword (two-hand)" },
    Value { value: 9, name: "obsolete" },
    Value { value: 10, name: "Staff" },
    Value { value: 11, name: "Exotic" },
    Value { value: 12, name: "Exotic 2" },
    Value { value: 13, name: "Fist Weapon" },
    Value { value: 14, name: "Miscellaneous" },
    Value { value: 15, name: "Dagger" },
    Value { value: 16, name: "Thrown" },
    Value { value: 17, name: "Spear" },
    Value { value: 18, name: "Crossbow" },
    Value { value: 19, name: "Wand" },
    Value { value: 20, name: "Fishing Pole" },
];

/// `ItemSubclassGem` is a single value in 1.12; the colours arrive in 2.0.
pub const SUBCLASS_GEM: [Value; 1] = [Value { value: 0, name: "Gem" }];

/// `ItemSubclassArmor`, from `ItemPrototype.h:233`.
pub const SUBCLASS_ARMOR: [Value; 10] = [
    Value { value: 0, name: "Miscellaneous" },
    Value { value: 1, name: "Cloth" },
    Value { value: 2, name: "Leather" },
    Value { value: 3, name: "Mail" },
    Value { value: 4, name: "Plate" },
    Value { value: 5, name: "Buckler" },
    Value { value: 6, name: "Shield" },
    Value { value: 7, name: "Libram" },
    Value { value: 8, name: "Idol" },
    Value { value: 9, name: "Totem" },
];

/// `ItemSubclassReagent`.
pub const SUBCLASS_REAGENT: [Value; 1] = [Value { value: 0, name: "Reagent" }];

/// `ItemSubclassProjectile`, from `ItemPrototype.h:256`. 0, 1 and 4 are marked
/// `ABS` — obsolete — in the header.
pub const SUBCLASS_PROJECTILE: [Value; 5] = [
    Value { value: 0, name: "Wand (obsolete)" },
    Value { value: 1, name: "Bolt (obsolete)" },
    Value { value: 2, name: "Arrow" },
    Value { value: 3, name: "Bullet" },
    Value { value: 4, name: "Thrown (obsolete)" },
];

/// `ItemSubclassTradeGoods`, from `ItemPrototype.h:267`.
pub const SUBCLASS_TRADE_GOODS: [Value; 4] = [
    Value { value: 0, name: "Trade Goods" },
    Value { value: 1, name: "Parts" },
    Value { value: 2, name: "Explosives" },
    Value { value: 3, name: "Devices" },
];

/// `ItemSubclassGeneric`.
pub const SUBCLASS_GENERIC: [Value; 1] = [Value { value: 0, name: "Generic" }];

/// `ItemSubclassRecipe`, from `ItemPrototype.h:295`.
pub const SUBCLASS_RECIPE: [Value; 10] = [
    Value { value: 0, name: "Book" },
    Value { value: 1, name: "Leatherworking Pattern" },
    Value { value: 2, name: "Tailoring Pattern" },
    Value { value: 3, name: "Engineering Schematic" },
    Value { value: 4, name: "Blacksmithing Plans" },
    Value { value: 5, name: "Cooking Recipe" },
    Value { value: 6, name: "Alchemy Recipe" },
    Value { value: 7, name: "First Aid Manual" },
    Value { value: 8, name: "Enchanting Formula" },
    Value { value: 9, name: "Fishing Manual" },
];

/// `ItemSubclassMoney`.
pub const SUBCLASS_MONEY: [Value; 1] = [Value { value: 0, name: "Money" }];

/// `ItemSubclassQuiver`, from `ItemPrototype.h:318`.
pub const SUBCLASS_QUIVER: [Value; 4] = [
    Value { value: 0, name: "Quiver (obsolete)" },
    Value { value: 1, name: "Quiver (obsolete)" },
    Value { value: 2, name: "Quiver" },
    Value { value: 3, name: "Ammo Pouch" },
];

/// `ItemSubclassQuest`.
pub const SUBCLASS_QUEST: [Value; 1] = [Value { value: 0, name: "Quest" }];

/// `ItemSubclassKey`, from `ItemPrototype.h:335`.
pub const SUBCLASS_KEY: [Value; 2] = [
    Value { value: 0, name: "Key" },
    Value { value: 1, name: "Lockpick" },
];

/// `ItemSubclassPermanent`.
pub const SUBCLASS_PERMANENT: [Value; 1] = [Value { value: 0, name: "Permanent" }];

/// `ItemSubclassJunk`.
pub const SUBCLASS_JUNK: [Value; 1] = [Value { value: 0, name: "Junk" }];

/// **Which subclass list applies to a class** — see [`Kind::Subclass`].
///
/// `MaxItemSubclassValues` (`ItemPrototype.h:357`) is vmangos' own version of
/// this, as a count per class; these are the same lists with their names on.
/// A class the game does not have answers an empty slice, and a form draws the
/// number, which is what the column is.
pub fn subclasses(class: u32) -> &'static [Value] {
    match class {
        0 => &SUBCLASS_CONSUMABLE,
        1 => &SUBCLASS_CONTAINER,
        2 => &SUBCLASS_WEAPON,
        3 => &SUBCLASS_GEM,
        4 => &SUBCLASS_ARMOR,
        5 => &SUBCLASS_REAGENT,
        6 => &SUBCLASS_PROJECTILE,
        7 => &SUBCLASS_TRADE_GOODS,
        8 => &SUBCLASS_GENERIC,
        9 => &SUBCLASS_RECIPE,
        10 => &SUBCLASS_MONEY,
        11 => &SUBCLASS_QUIVER,
        12 => &SUBCLASS_QUEST,
        13 => &SUBCLASS_KEY,
        14 => &SUBCLASS_PERMANENT,
        15 => &SUBCLASS_JUNK,
        _ => &[],
    }
}

/// `InventoryType`, from `ItemPrototype.h:100`. **Where it is worn**, which is
/// also what decides the slot its display id's models hang from — see
/// `vale_assets::tables::item::Slot::from_inventory_type`.
pub const INVENTORY_TYPES: [Value; 29] = [
    Value { value: 0, name: "Not equipped" },
    Value { value: 1, name: "Head" },
    Value { value: 2, name: "Neck" },
    Value { value: 3, name: "Shoulders" },
    Value { value: 4, name: "Shirt" },
    Value { value: 5, name: "Chest" },
    Value { value: 6, name: "Waist" },
    Value { value: 7, name: "Legs" },
    Value { value: 8, name: "Feet" },
    Value { value: 9, name: "Wrists" },
    Value { value: 10, name: "Hands" },
    Value { value: 11, name: "Finger" },
    Value { value: 12, name: "Trinket" },
    Value { value: 13, name: "One-hand" },
    Value { value: 14, name: "Shield" },
    Value { value: 15, name: "Bow" },
    Value { value: 16, name: "Back" },
    Value { value: 17, name: "Two-hand" },
    Value { value: 18, name: "Bag" },
    Value { value: 19, name: "Tabard" },
    Value { value: 20, name: "Robe" },
    Value { value: 21, name: "Main hand" },
    Value { value: 22, name: "Off hand" },
    Value { value: 23, name: "Held in off-hand" },
    Value { value: 24, name: "Ammo" },
    Value { value: 25, name: "Thrown" },
    Value { value: 26, name: "Ranged right" },
    Value { value: 27, name: "Quiver" },
    Value { value: 28, name: "Relic" },
];

/// `ItemBondingType`, from `ItemPrototype.h:49`.
pub const BONDING: [Value; 6] = [
    Value { value: 0, name: "No bind" },
    Value { value: 1, name: "Binds when picked up" },
    Value { value: 2, name: "Binds when equipped" },
    Value { value: 3, name: "Binds when used" },
    Value { value: 4, name: "Quest item" },
    Value { value: 5, name: "Quest item (unused)" },
];

/// `SheathTypes`, from `SharedDefines.h:215`. **Where the weapon hangs while it
/// is not drawn**, which is a property of the item and not of the slot — see
/// `vale_assets::look::sheath`, which maps it to an attachment point.
pub const SHEATH_TYPES: [Value; 8] = [
    Value { value: 0, name: "None" },
    Value { value: 1, name: "Main hand" },
    Value { value: 2, name: "Off hand" },
    Value { value: 3, name: "Large weapon, left" },
    Value { value: 4, name: "Large weapon, right" },
    Value { value: 5, name: "Hip weapon, left" },
    Value { value: 6, name: "Hip weapon, right" },
    Value { value: 7, name: "Shield" },
];

/// `ItemModType`, from `ItemPrototype.h:27`. The seven stats an item can carry
/// in 1.12, plus the 2 the header skips — a row holding 2 is a row worth seeing
/// as an unnamed number rather than as the stat above or below it.
pub const STAT_TYPES: [Value; 8] = [
    Value { value: 0, name: "Mana" },
    Value { value: 1, name: "Health" },
    Value { value: 3, name: "Agility" },
    Value { value: 4, name: "Strength" },
    Value { value: 5, name: "Intellect" },
    Value { value: 6, name: "Spirit" },
    Value { value: 7, name: "Stamina" },
    // The client draws nothing for a stat it does not know; 0 with a value of 0
    // is how an unused slot is written, which is why "Mana" is also what an
    // empty slot reads as. `stat_value` being 0 is the real "no stat".
    Value { value: 2, name: "unused in 1.12" },
];

/// `ItemSpelltriggerType`, from `ItemPrototype.h:40`. **How an item's spell is
/// set off**, which is the difference between a potion and a trinket.
pub const SPELL_TRIGGERS: [Value; 3] = [
    Value { value: 0, name: "On use" },
    Value { value: 1, name: "On equip" },
    Value { value: 2, name: "Chance on hit" },
];

/// The schools a weapon's damage can be, which is a row of `Resistances.dbc` —
/// `_ItemDamage::DamageType` says so (`ItemPrototype.h:427`), and it is the
/// same order the resistance columns are in.
pub const DAMAGE_SCHOOLS: [Value; 7] = [
    Value { value: 0, name: "Physical" },
    Value { value: 1, name: "Holy" },
    Value { value: 2, name: "Fire" },
    Value { value: 3, name: "Nature" },
    Value { value: 4, name: "Frost" },
    Value { value: 5, name: "Shadow" },
    Value { value: 6, name: "Arcane" },
];

/// `BagFamily`, from `ItemPrototype.h:86`. **What a bag may hold**, and on an
/// ordinary item what kind of bag it will go in.
pub const BAG_FAMILIES: [Value; 10] = [
    Value { value: 0, name: "Any bag" },
    Value { value: 1, name: "Arrows" },
    Value { value: 2, name: "Bullets" },
    Value { value: 3, name: "Soul shards" },
    Value { value: 4, name: "unknown 1" },
    Value { value: 5, name: "unknown 2" },
    Value { value: 6, name: "Herbs" },
    Value { value: 7, name: "Enchanting supplies" },
    Value { value: 8, name: "Engineering supplies" },
    Value { value: 9, name: "Keys" },
];

/// `ItemPrototypeFlags`, from `ItemPrototype.h:62`. The header's own comments
/// are kept, including the two that say a flag is not used at all in 1.12.
pub const FLAGS: [Bit; 19] = [
    Bit { bit: 0x00000001, name: "NO_PICKUP", about: "not used" },
    Bit { bit: 0x00000002, name: "CONJURED", about: "vanishes on logout" },
    Bit { bit: 0x00000004, name: "LOOTABLE", about: "right click to open" },
    Bit { bit: 0x00000008, name: "EXOTIC", about: "not used before 3.x" },
    Bit { bit: 0x00000010, name: "DEPRECATED", about: "drawn as if broken" },
    Bit { bit: 0x00000020, name: "INDESTRUCTIBLE", about: "cannot be destroyed; used for totems" },
    Bit { bit: 0x00000040, name: "PLAYERCAST", about: "usable" },
    Bit { bit: 0x00000080, name: "NO_EQUIP_COOLDOWN", about: "no cooldown when equipped" },
    Bit { bit: 0x00000100, name: "INTBONUSINSTEAD", about: "" },
    Bit { bit: 0x00000200, name: "WRAPPER", about: "wraps another item" },
    Bit { bit: 0x00000400, name: "IGNORE_BAG_SPACE", about: "created whether or not there is room" },
    Bit { bit: 0x00000800, name: "PARTY_LOOT", about: "group loot rather than personal" },
    Bit { bit: 0x00001000, name: "BRIEFSPELLEFFECTS", about: "not used before 3.x" },
    Bit { bit: 0x00002000, name: "CHARTER", about: "guild charter" },
    Bit { bit: 0x00004000, name: "HAS_TEXT", about: "readable" },
    Bit { bit: 0x00008000, name: "NO_DISENCHANT", about: "cannot be disenchanted" },
    Bit { bit: 0x00010000, name: "REAL_DURATION", about: "the duration runs while logged out" },
    Bit { bit: 0x00020000, name: "NO_CREATOR", about: "last flag 1.12.1 uses" },
    Bit { bit: 0x00080000, name: "UNIQUE_EQUIPPED", about: "vmangos' own; 2.x has it client-side" },
];

/// `ItemExtraFlags`, from `ItemPrototype.h:394`. vmangos' own, above the
/// client's.
pub const EXTRA_FLAGS: [Bit; 3] = [
    Bit { bit: 0x01, name: "MAIL_STATIONERY", about: "used as mail art" },
    Bit { bit: 0x02, name: "IGNORE_QUEST_STATUS", about: "drops whatever the quest state is" },
    Bit { bit: 0x04, name: "NOT_OBTAINABLE", about: "never obtainable in vanilla" },
];

/// **Who may equip it**, as a class mask. `Classes` from `SharedDefines.h`, as
/// bits: the value stored is `1 << (class - 1)`, and `-1` is everybody.
pub const CLASS_MASK: [Bit; 9] = [
    Bit { bit: 0x001, name: "Warrior", about: "" },
    Bit { bit: 0x002, name: "Paladin", about: "" },
    Bit { bit: 0x004, name: "Hunter", about: "" },
    Bit { bit: 0x008, name: "Rogue", about: "" },
    Bit { bit: 0x010, name: "Priest", about: "" },
    Bit { bit: 0x020, name: "unused (Death Knight)", about: "not a 1.12 class" },
    Bit { bit: 0x040, name: "Shaman", about: "" },
    Bit { bit: 0x080, name: "Mage", about: "" },
    Bit { bit: 0x100, name: "Warlock", about: "" },
];

/// …and the same for race — `1 << (race - 1)`, with `-1` for everybody.
pub const RACE_MASK: [Bit; 8] = [
    Bit { bit: 0x001, name: "Human", about: "" },
    Bit { bit: 0x002, name: "Orc", about: "" },
    Bit { bit: 0x004, name: "Dwarf", about: "" },
    Bit { bit: 0x008, name: "Night Elf", about: "" },
    Bit { bit: 0x010, name: "Undead", about: "" },
    Bit { bit: 0x020, name: "Tauren", about: "" },
    Bit { bit: 0x040, name: "Gnome", about: "" },
    Bit { bit: 0x080, name: "Troll", about: "" },
];

/// `ReputationRank`, from `SharedDefines.h`. What `required_reputation_rank`
/// counts in.
pub const REPUTATION_RANKS: [Value; 8] = [
    Value { value: 0, name: "Hated" },
    Value { value: 1, name: "Hostile" },
    Value { value: 2, name: "Unfriendly" },
    Value { value: 3, name: "Neutral" },
    Value { value: 4, name: "Friendly" },
    Value { value: 5, name: "Honored" },
    Value { value: 6, name: "Revered" },
    Value { value: 7, name: "Exalted" },
];

/// **What a pet will eat**, as a mask of `CreatureFamily.dbc`'s own diet bits —
/// which is the same mask `vale pet` decodes from the other side. A hunter's
/// pet eats an item when `food_type` and its family's diet share a bit.
pub const FOOD_TYPES: [Value; 9] = [
    Value { value: 0, name: "Not food" },
    Value { value: 1, name: "Meat" },
    Value { value: 2, name: "Fish" },
    Value { value: 3, name: "Cheese" },
    Value { value: 4, name: "Bread" },
    Value { value: 5, name: "Fungus" },
    Value { value: 6, name: "Fruit" },
    Value { value: 7, name: "Raw meat" },
    Value { value: 8, name: "Raw fish" },
];

/// **The parchment a readable item is drawn on** — a row of
/// `PageTextMaterial.dbc`, which is what the client looks the background up in.
pub const PAGE_MATERIALS: [Value; 5] = [
    Value { value: 0, name: "None" },
    Value { value: 1, name: "Parchment" },
    Value { value: 2, name: "Stone" },
    Value { value: 3, name: "Marble" },
    Value { value: 4, name: "Silver" },
];

/// **What a readable item is written in** — a row of `Languages.dbc`. `0` is
/// the universal tongue, which is what every ordinary book and letter uses.
pub const LANGUAGES: [Value; 10] = [
    Value { value: 0, name: "Universal" },
    Value { value: 1, name: "Orcish" },
    Value { value: 2, name: "Darnassian" },
    Value { value: 3, name: "Taurahe" },
    Value { value: 6, name: "Dwarvish" },
    Value { value: 7, name: "Common" },
    Value { value: 8, name: "Demonic" },
    Value { value: 9, name: "Titan" },
    Value { value: 10, name: "Thalassian" },
    Value { value: 11, name: "Draconic" },
];

/// **What the item is made of**, as `ItemPrototype.h:484` states it: a row of
/// `Material.dbc`. It is what the client picks a pick-up and put-down sound by
/// when the display row names no sound group of its own.
///
/// `-1` is a consumable, which is the one value that is not a row id at all.
pub const MATERIALS: [Value; 10] = [
    Value { value: 0, name: "Not defined" },
    Value { value: 1, name: "Metal" },
    Value { value: 2, name: "Wood" },
    Value { value: 3, name: "Liquid" },
    Value { value: 4, name: "Jewelry" },
    Value { value: 5, name: "Chain" },
    Value { value: 6, name: "Plate" },
    Value { value: 7, name: "Cloth" },
    Value { value: 8, name: "Leather" },
    // Stored as -1 and read back as a signed column; named here so a form that
    // has the number in hand can still say what it is.
    Value { value: u32::MAX, name: "Consumable (-1)" },
];

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// **`item_template`'s 129 columns**, in the order
/// `ObjectMgr::LoadItemPrototypes` selects them (`ObjectMgr.cpp:3794`).
///
/// `patch` is not among them for the reason `creature_template`'s is not: it is
/// half the key rather than a column an edit may write, and [`template_key`] is
/// where it goes.
pub const TEMPLATE_COLUMNS: [Column; 129] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the item id" },
    Column { name: "class", kind: Kind::Choice(&CLASSES), group: Group::Identity, about: "ItemClass.dbc: what kind of thing it is" },
    Column { name: "subclass", kind: Kind::Subclass, group: Group::Identity, about: "ItemSubClass.dbc: which kind of that kind — the list depends on the class" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "what the tooltip's first line says" },
    Column { name: "description", kind: Kind::Text, group: Group::Text, about: "the yellow line in quotes at the foot of the tooltip" },
    Column { name: "display_id", kind: Kind::Ref("ItemDisplayInfo"), group: Group::Appearance, about: "ItemDisplayInfo.dbc: the icon, the models and the textures it paints" },
    Column { name: "quality", kind: Kind::Choice(&QUALITIES), group: Group::Identity, about: "the colour the name is drawn in" },
    Column { name: "flags", kind: Kind::Flags(&FLAGS), group: Group::Advanced, about: "ItemPrototypeFlags: conjured, lootable, readable, deprecated" },
    Column { name: "buy_count", kind: Kind::Unsigned, group: Group::Economy, about: "how many a vendor sells at once" },
    Column { name: "buy_price", kind: Kind::Money, group: Group::Economy, about: "what a vendor charges for buy_count of them" },
    Column { name: "sell_price", kind: Kind::Money, group: Group::Economy, about: "what a vendor pays for one; 0 means it cannot be sold" },
    Column { name: "inventory_type", kind: Kind::Choice(&INVENTORY_TYPES), group: Group::Appearance, about: "which slot it is worn in, and so which models its display id hangs" },
    Column { name: "allowable_class", kind: Kind::Flags(&CLASS_MASK), group: Group::Requirements, about: "which classes may equip it; -1 is all of them" },
    Column { name: "allowable_race", kind: Kind::Flags(&RACE_MASK), group: Group::Requirements, about: "which races may equip it; -1 is all of them" },
    Column { name: "item_level", kind: Kind::Unsigned, group: Group::Stats, about: "what the item counts as, for the stat budget and for disenchanting" },
    Column { name: "required_level", kind: Kind::Unsigned, group: Group::Requirements, about: "the character level needed to equip it" },
    Column { name: "required_skill", kind: Kind::Ref("SkillLine"), group: Group::Requirements, about: "SkillLine.dbc id the wearer must have" },
    Column { name: "required_skill_rank", kind: Kind::Unsigned, group: Group::Requirements, about: "…and how much of it" },
    Column { name: "required_spell", kind: Kind::Ref("Spell"), group: Group::Requirements, about: "a spell the wearer must know — a proficiency, or a recipe already learnt" },
    Column { name: "required_honor_rank", kind: Kind::Unsigned, group: Group::Requirements, about: "the PvP rank needed, 1..14" },
    Column { name: "required_city_rank", kind: Kind::Unsigned, group: Group::Requirements, about: "unused in 1.12" },
    Column { name: "required_reputation_faction", kind: Kind::Ref("Faction"), group: Group::Requirements, about: "Faction.dbc id the standing is with" },
    Column { name: "required_reputation_rank", kind: Kind::Choice(&REPUTATION_RANKS), group: Group::Requirements, about: "…and the standing needed with it" },
    Column { name: "max_count", kind: Kind::Unsigned, group: Group::Economy, about: "how many a character may own; 0 is unlimited" },
    Column { name: "stackable", kind: Kind::Unsigned, group: Group::Economy, about: "how many fit in one bag square" },
    Column { name: "container_slots", kind: Kind::Unsigned, group: Group::Container, about: "how many squares this bag has" },
    Column { name: "stat_type1", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "which stat the first line adds" },
    Column { name: "stat_value1", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type2", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the second stat line" },
    Column { name: "stat_value2", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type3", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the third" },
    Column { name: "stat_value3", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type4", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the fourth" },
    Column { name: "stat_value4", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type5", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the fifth" },
    Column { name: "stat_value5", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type6", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the sixth" },
    Column { name: "stat_value6", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type7", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the seventh" },
    Column { name: "stat_value7", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type8", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the eighth" },
    Column { name: "stat_value8", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type9", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the ninth" },
    Column { name: "stat_value9", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "stat_type10", kind: Kind::Choice(&STAT_TYPES), group: Group::Stats, about: "the tenth, which nothing in 1.12 uses" },
    Column { name: "stat_value10", kind: Kind::Signed, group: Group::Stats, about: "…and by how much" },
    Column { name: "delay", kind: Kind::Millis, group: Group::Combat, about: "the swing timer: milliseconds between attacks" },
    Column { name: "range_mod", kind: Kind::Float, group: Group::Combat, about: "how far it reaches, as a multiplier of the class's own" },
    Column { name: "ammo_type", kind: Kind::Choice(&SUBCLASS_PROJECTILE), group: Group::Combat, about: "which projectile subclass this weapon fires" },
    Column { name: "dmg_min1", kind: Kind::Float, group: Group::Combat, about: "the low end of the first damage line" },
    Column { name: "dmg_max1", kind: Kind::Float, group: Group::Combat, about: "…and the high end" },
    Column { name: "dmg_type1", kind: Kind::Choice(&DAMAGE_SCHOOLS), group: Group::Combat, about: "…and its school, from Resistances.dbc" },
    Column { name: "dmg_min2", kind: Kind::Float, group: Group::Combat, about: "a second damage line" },
    Column { name: "dmg_max2", kind: Kind::Float, group: Group::Combat, about: "…and the high end" },
    Column { name: "dmg_type2", kind: Kind::Choice(&DAMAGE_SCHOOLS), group: Group::Combat, about: "…and its school" },
    Column { name: "dmg_min3", kind: Kind::Float, group: Group::Combat, about: "a third damage line" },
    Column { name: "dmg_max3", kind: Kind::Float, group: Group::Combat, about: "…and the high end" },
    Column { name: "dmg_type3", kind: Kind::Choice(&DAMAGE_SCHOOLS), group: Group::Combat, about: "…and its school" },
    Column { name: "dmg_min4", kind: Kind::Float, group: Group::Combat, about: "a fourth damage line" },
    Column { name: "dmg_max4", kind: Kind::Float, group: Group::Combat, about: "…and the high end" },
    Column { name: "dmg_type4", kind: Kind::Choice(&DAMAGE_SCHOOLS), group: Group::Combat, about: "…and its school" },
    Column { name: "dmg_min5", kind: Kind::Float, group: Group::Combat, about: "a fifth damage line" },
    Column { name: "dmg_max5", kind: Kind::Float, group: Group::Combat, about: "…and the high end" },
    Column { name: "dmg_type5", kind: Kind::Choice(&DAMAGE_SCHOOLS), group: Group::Combat, about: "…and its school" },
    Column { name: "block", kind: Kind::Unsigned, group: Group::Combat, about: "how much damage a shield blocks" },
    Column { name: "armor", kind: Kind::Unsigned, group: Group::Stats, about: "the armour it gives" },
    Column { name: "holy_res", kind: Kind::Signed, group: Group::Stats, about: "holy resistance" },
    Column { name: "fire_res", kind: Kind::Signed, group: Group::Stats, about: "fire resistance" },
    Column { name: "nature_res", kind: Kind::Signed, group: Group::Stats, about: "nature resistance" },
    Column { name: "frost_res", kind: Kind::Signed, group: Group::Stats, about: "frost resistance" },
    Column { name: "shadow_res", kind: Kind::Signed, group: Group::Stats, about: "shadow resistance" },
    Column { name: "arcane_res", kind: Kind::Signed, group: Group::Stats, about: "arcane resistance" },
    Column { name: "spellid_1", kind: Kind::Ref("Spell"), group: Group::Effects, about: "the first spell it carries" },
    Column { name: "spelltrigger_1", kind: Kind::Choice(&SPELL_TRIGGERS), group: Group::Effects, about: "…and what sets it off" },
    Column { name: "spellcharges_1", kind: Kind::Signed, group: Group::Effects, about: "…how many uses; negative destroys the item at zero" },
    Column { name: "spellppmrate_1", kind: Kind::Float, group: Group::Effects, about: "…procs per minute, for a chance-on-hit" },
    Column { name: "spellcooldown_1", kind: Kind::Millis, group: Group::Effects, about: "…this spell's own cooldown" },
    Column { name: "spellcategory_1", kind: Kind::Unsigned, group: Group::Effects, about: "…SpellCategory.dbc id it shares a cooldown with" },
    Column { name: "spellcategorycooldown_1", kind: Kind::Millis, group: Group::Effects, about: "…and that category's cooldown" },
    Column { name: "spellid_2", kind: Kind::Ref("Spell"), group: Group::Effects, about: "the second spell it carries" },
    Column { name: "spelltrigger_2", kind: Kind::Choice(&SPELL_TRIGGERS), group: Group::Effects, about: "…and what sets it off" },
    Column { name: "spellcharges_2", kind: Kind::Signed, group: Group::Effects, about: "…how many uses" },
    Column { name: "spellppmrate_2", kind: Kind::Float, group: Group::Effects, about: "…procs per minute" },
    Column { name: "spellcooldown_2", kind: Kind::Millis, group: Group::Effects, about: "…its own cooldown" },
    Column { name: "spellcategory_2", kind: Kind::Unsigned, group: Group::Effects, about: "…its category" },
    Column { name: "spellcategorycooldown_2", kind: Kind::Millis, group: Group::Effects, about: "…and that category's cooldown" },
    Column { name: "spellid_3", kind: Kind::Ref("Spell"), group: Group::Effects, about: "the third spell it carries" },
    Column { name: "spelltrigger_3", kind: Kind::Choice(&SPELL_TRIGGERS), group: Group::Effects, about: "…and what sets it off" },
    Column { name: "spellcharges_3", kind: Kind::Signed, group: Group::Effects, about: "…how many uses" },
    Column { name: "spellppmrate_3", kind: Kind::Float, group: Group::Effects, about: "…procs per minute" },
    Column { name: "spellcooldown_3", kind: Kind::Millis, group: Group::Effects, about: "…its own cooldown" },
    Column { name: "spellcategory_3", kind: Kind::Unsigned, group: Group::Effects, about: "…its category" },
    Column { name: "spellcategorycooldown_3", kind: Kind::Millis, group: Group::Effects, about: "…and that category's cooldown" },
    Column { name: "spellid_4", kind: Kind::Ref("Spell"), group: Group::Effects, about: "the fourth spell it carries" },
    Column { name: "spelltrigger_4", kind: Kind::Choice(&SPELL_TRIGGERS), group: Group::Effects, about: "…and what sets it off" },
    Column { name: "spellcharges_4", kind: Kind::Signed, group: Group::Effects, about: "…how many uses" },
    Column { name: "spellppmrate_4", kind: Kind::Float, group: Group::Effects, about: "…procs per minute" },
    Column { name: "spellcooldown_4", kind: Kind::Millis, group: Group::Effects, about: "…its own cooldown" },
    Column { name: "spellcategory_4", kind: Kind::Unsigned, group: Group::Effects, about: "…its category" },
    Column { name: "spellcategorycooldown_4", kind: Kind::Millis, group: Group::Effects, about: "…and that category's cooldown" },
    Column { name: "spellid_5", kind: Kind::Ref("Spell"), group: Group::Effects, about: "the fifth spell it carries" },
    Column { name: "spelltrigger_5", kind: Kind::Choice(&SPELL_TRIGGERS), group: Group::Effects, about: "…and what sets it off" },
    Column { name: "spellcharges_5", kind: Kind::Signed, group: Group::Effects, about: "…how many uses" },
    Column { name: "spellppmrate_5", kind: Kind::Float, group: Group::Effects, about: "…procs per minute" },
    Column { name: "spellcooldown_5", kind: Kind::Millis, group: Group::Effects, about: "…its own cooldown" },
    Column { name: "spellcategory_5", kind: Kind::Unsigned, group: Group::Effects, about: "…its category" },
    Column { name: "spellcategorycooldown_5", kind: Kind::Millis, group: Group::Effects, about: "…and that category's cooldown" },
    Column { name: "bonding", kind: Kind::Choice(&BONDING), group: Group::Requirements, about: "when it becomes soulbound" },
    Column { name: "page_text", kind: Kind::Unsigned, group: Group::Text, about: "page_text id: what reading it shows" },
    Column { name: "page_language", kind: Kind::Choice(&LANGUAGES), group: Group::Text, about: "Languages.dbc id it is written in" },
    Column { name: "page_material", kind: Kind::Choice(&PAGE_MATERIALS), group: Group::Text, about: "PageTextMaterial.dbc id: the parchment it is drawn on" },
    Column { name: "start_quest", kind: Kind::Ref("quest_template"), group: Group::Requirements, about: "the quest this item starts when it is right-clicked" },
    Column { name: "lock_id", kind: Kind::Ref("Lock"), group: Group::Requirements, about: "Lock.dbc id: what it takes to open it" },
    Column { name: "material", kind: Kind::Choice(&MATERIALS), group: Group::Appearance, about: "Material.dbc id: what it sounds like when picked up" },
    Column { name: "sheath", kind: Kind::Choice(&SHEATH_TYPES), group: Group::Appearance, about: "where the weapon hangs while it is not drawn" },
    Column { name: "random_property", kind: Kind::Unsigned, group: Group::Loot, about: "ItemRandomProperties.dbc id: the 'of the Bear' suffix table" },
    Column { name: "set_id", kind: Kind::Unsigned, group: Group::Stats, about: "ItemSet.dbc id: which set it belongs to" },
    Column { name: "max_durability", kind: Kind::Unsigned, group: Group::Stats, about: "how much it takes before it breaks; 0 never breaks" },
    Column { name: "area_bound", kind: Kind::Ref("AreaTable"), group: Group::Requirements, about: "AreaTable.dbc id it only works in" },
    Column { name: "map_bound", kind: Kind::Ref("Map"), group: Group::Requirements, about: "Map.dbc id it only works on" },
    Column { name: "duration", kind: Kind::Seconds, group: Group::Economy, about: "seconds before it vanishes; 0 lasts for ever" },
    Column { name: "bag_family", kind: Kind::Choice(&BAG_FAMILIES), group: Group::Container, about: "which special bag holds it, or which this bag holds" },
    Column { name: "disenchant_id", kind: Kind::Unsigned, group: Group::Loot, about: "disenchant_loot_template entry" },
    Column { name: "food_type", kind: Kind::Choice(&FOOD_TYPES), group: Group::Requirements, about: "which pet diet this feeds" },
    Column { name: "min_money_loot", kind: Kind::Money, group: Group::Loot, about: "least copper inside it when it is opened" },
    Column { name: "max_money_loot", kind: Kind::Money, group: Group::Loot, about: "most copper inside it" },
    Column { name: "wrapped_gift", kind: Kind::Ref(TEMPLATE), group: Group::Economy, about: "the item this one becomes when it is wrapped" },
    Column { name: "extra_flags", kind: Kind::Flags(&EXTRA_FLAGS), group: Group::Advanced, about: "vmangos' own flags, above the client's" },
    Column { name: "other_team_entry", kind: Kind::Ref(TEMPLATE), group: Group::Advanced, about: "the other faction's version of this item" },
];

/// The columns of the table, or an empty slice for a name this module does not
/// know.
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        TEMPLATE => &TEMPLATE_COLUMNS,
        _ => &[],
    }
}

/// One column, by name.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The key an edit to an item row is written under — see the module comment on
/// why the patch is half of it.
pub fn template_key(entry: u32, patch: u32) -> Key {
    Key::two(("entry", entry as u64), ("patch", patch as u64))
}

/// **Where an item this project creates gets its entry from.**
///
/// `item_template.entry` is `mediumint unsigned`, so the ceiling is
/// [`MAX_ENTRY`] — the same number a creature guid has, arrived at from the
/// column's own width rather than from a packed object guid.
///
/// **Measured rather than assumed, and the measurement was a surprise.** The
/// reference install's shipped items stop in the tens of thousands, but its
/// highest `entry` is **1,000,006**: somebody's own items are already sitting
/// at a round million, which is exactly where a first draft of this constant
/// would have put its own. So the base is two million, and
/// `crate::item::MAX_ENTRY_QUERY` is read beside it — the base is a floor and
/// the table's own maximum is the other half, because a reserved range means
/// nothing if the range is already in use.
pub const RESERVED_ENTRY_BASE: u32 = 2_000_000;

/// …and the highest entry the column can hold — `mediumint unsigned`.
pub const MAX_ENTRY: u32 = 0x00FF_FFFF;

/// **Every column of a new item**, as the values a row created here starts
/// from.
///
/// The `entry` and the `patch` are not among them: they are the key, and
/// [`crate::row::insert`] writes a key's own columns first. Everything else is
/// named, because a column an `INSERT` leaves out takes the table's default
/// rather than a value somebody chose — and three of those defaults make a row
/// the server loads and the client refuses to draw.
///
/// The values are the ones that produce an item a character can actually be
/// given, which is what "new item" has to mean:
///
/// * `class` **15** and `subclass` **0**, which is Junk — the one class with no
///   requirement of any kind attached to it. A row of zeros is class 0,
///   *Consumable*, whose tooltip says "Use:" and whose spell list is empty.
/// * `quality` **1** and `display_id` **0**. Zero is the one display id
///   `vale_assets::tables::item` documents as meaning *nothing worn*, so a
///   new item draws the empty-slot icon until one is picked, which is the
///   honest picture of a row that has not chosen an appearance yet.
/// * `stackable` **1** and `buy_count` **1**. The DDL's `buy_count` default is
///   1 and its `stackable` default is 1; zero in either is an item a vendor
///   sells none of and a bag square that holds none of.
/// * `max_durability` **0**, which is "never breaks" rather than "breaks at
///   once".
/// * `allowable_class` and `allowable_race` **-1**, which is everybody.
pub fn new_item(name: &str) -> Vec<Assignment> {
    let mut out: Vec<(&'static str, String)> = vec![
        ("class", "15".to_string()),
        ("subclass", "0".to_string()),
        ("name", crate::sql::text(name)),
        ("description", crate::sql::text("")),
        ("display_id", "0".to_string()),
        ("quality", "1".to_string()),
        ("buy_count", "1".to_string()),
        ("stackable", "1".to_string()),
        ("allowable_class", "-1".to_string()),
        ("allowable_race", "-1".to_string()),
        ("max_durability", "0".to_string()),
        ("material", "0".to_string()),
    ];
    // Everything else is zero, which for every remaining column is the value
    // that means "nothing": no spell, no stat, no requirement, no page of text.
    let already: Vec<&str> = out.iter().map(|(column, _)| *column).collect();
    for column in TEMPLATE_COLUMNS.iter().filter(|column| column.editable()) {
        if already.contains(&column.name) {
            continue;
        }
        out.push((
            column.name,
            match column.kind {
                Kind::Text => crate::sql::text(""),
                Kind::Float => "0".to_string(),
                _ => "0".to_string(),
            },
        ));
    }
    out.into_iter()
        .map(|(column, value)| Assignment { column, value })
        .collect()
}

/// **The statements a save emits for one row this project claims.**
///
/// One `UPDATE` for a row it edits and a `DELETE`/`INSERT` pair for a row it
/// creates — the pair for [`crate::creature::statements`]' reason, which is
/// that applying twice then means the same as applying once.
///
/// **A removal is [`delete_statements`]**: every version of the item and the
/// rows of [`DEPENDENTS`]. It is safe on a restart and not on a reload — see
/// the module comment and [`reload_is_safe`].
pub fn statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    if !can_live(table, life) {
        return Vec::new();
    }
    match life {
        Life::Update => crate::row::update(table, key, changes).into_iter().collect(),
        Life::Insert => vec![
            crate::row::delete(table, key),
            match crate::row::insert(table, key, changes) {
                Some(statement) => statement,
                None => return Vec::new(),
            },
        ],
        Life::Delete => match key.first() {
            Some(entry) => delete_statements(entry),
            None => Vec::new(),
        },
    }
}

/// **The rows removed with an item**, as the column each names it in.
///
/// Two kinds, and both are rows whose only subject is the item: what describes
/// it (`locales_item`, `item_required_target`, `forbidden_items`, and
/// `item_loot_template` keyed by the item, which is what the item contains),
/// and what exists to hand it out (a loot row while `mincountOrRef` is
/// positive, a vendor's line, the auction bot's line, a starting or premade
/// item). A loader meeting one of these after the item has gone logs it and
/// skips it, so leaving them would be a start-up log of every place the item
/// used to be.
///
/// **Left alone**, because each is a column of a row about something else:
/// `creature_equip_template`'s three items, `player_factionchange_items`'
/// pairs, and `quest_template`'s and `spell_template`'s item columns. A quest
/// that asks for or rewards a removed item still names it, and the server
/// reports it at start.
pub const DEPENDENTS: [crate::row::Reference; 18] = {
    use crate::row::Reference as R;
    const LOOT: &str = "`mincountOrRef` > 0";
    [
        R::new("locales_item", "entry"),
        R::new("item_required_target", "entry"),
        R::new("forbidden_items", "entry"),
        R::new("item_loot_template", "entry"),
        R::only("creature_loot_template", "item", LOOT),
        R::only("disenchant_loot_template", "item", LOOT),
        R::only("fishing_loot_template", "item", LOOT),
        R::only("gameobject_loot_template", "item", LOOT),
        R::only("item_loot_template", "item", LOOT),
        R::only("mail_loot_template", "item", LOOT),
        R::only("pickpocketing_loot_template", "item", LOOT),
        R::only("reference_loot_template", "item", LOOT),
        R::only("skinning_loot_template", "item", LOOT),
        R::new("npc_vendor", "item"),
        R::new("npc_vendor_template", "item"),
        R::new("auctionhousebot", "item"),
        R::new("playercreateinfo_item", "itemid"),
        R::new("player_premade_item", "item"),
    ]
};

/// **Everything a removed item takes with it**, as statements: every version
/// of the template, with the patch left out of the `WHERE` so the server does
/// not fall back to an older one, and then each of [`DEPENDENTS`].
pub fn delete_statements(entry: u64) -> Vec<String> {
    let mut out = vec![format!(
        "DELETE FROM {} WHERE `entry` = {entry};",
        crate::sql::name(TEMPLATE)
    )];
    out.extend(DEPENDENTS.iter().map(|dependent| dependent.delete(entry)));
    out
}

/// **Whether `.reload item_template` may follow an apply of these rows.** Not
/// when any of them removes an item — see the module comment. The server has
/// to be restarted instead, and until it is, it still holds the prototype.
pub fn reload_is_safe<'a>(lives: impl IntoIterator<Item = &'a Life>) -> bool {
    lives.into_iter().all(|life| *life != Life::Delete)
}

/// **Every column of the world database that names an item by entry**, read
/// off the reference install's `information_schema` and checked against the
/// loaders that read each one.
///
/// A loot table's `item` is an item only while `mincountOrRef` is positive;
/// below zero the row points at a `reference_loot_template` and `item` is a
/// label. `item_loot_template.entry` is the container the loot is inside.
pub const REFERENCES: [crate::row::Reference; 55] = {
    use crate::row::Reference as R;
    const LOOT: &str = "`mincountOrRef` > 0";
    [
        R::only("creature_loot_template", "item", LOOT),
        R::only("disenchant_loot_template", "item", LOOT),
        R::only("fishing_loot_template", "item", LOOT),
        R::only("gameobject_loot_template", "item", LOOT),
        R::only("item_loot_template", "item", LOOT),
        R::only("mail_loot_template", "item", LOOT),
        R::only("pickpocketing_loot_template", "item", LOOT),
        R::only("reference_loot_template", "item", LOOT),
        R::only("skinning_loot_template", "item", LOOT),
        R::new("item_loot_template", "entry"),
        R::new("npc_vendor", "item"),
        R::new("npc_vendor_template", "item"),
        R::new("creature_equip_template", "item1"),
        R::new("creature_equip_template", "item2"),
        R::new("creature_equip_template", "item3"),
        R::new("playercreateinfo_item", "itemid"),
        R::new("player_premade_item", "item"),
        R::new("player_factionchange_items", "alliance_id"),
        R::new("player_factionchange_items", "horde_id"),
        R::new("locales_item", "entry"),
        R::new("item_required_target", "entry"),
        R::new("forbidden_items", "entry"),
        R::new("auctionhousebot", "item"),
        R::new("quest_template", "SrcItemId"),
        R::new("quest_template", "ReqItemId1"),
        R::new("quest_template", "ReqItemId2"),
        R::new("quest_template", "ReqItemId3"),
        R::new("quest_template", "ReqItemId4"),
        R::new("quest_template", "ReqSourceId1"),
        R::new("quest_template", "ReqSourceId2"),
        R::new("quest_template", "ReqSourceId3"),
        R::new("quest_template", "ReqSourceId4"),
        R::new("quest_template", "RewChoiceItemId1"),
        R::new("quest_template", "RewChoiceItemId2"),
        R::new("quest_template", "RewChoiceItemId3"),
        R::new("quest_template", "RewChoiceItemId4"),
        R::new("quest_template", "RewChoiceItemId5"),
        R::new("quest_template", "RewChoiceItemId6"),
        R::new("quest_template", "RewItemId1"),
        R::new("quest_template", "RewItemId2"),
        R::new("quest_template", "RewItemId3"),
        R::new("quest_template", "RewItemId4"),
        R::new("spell_template", "effectItemType1"),
        R::new("spell_template", "effectItemType2"),
        R::new("spell_template", "effectItemType3"),
        R::new("spell_template", "reagent1"),
        R::new("spell_template", "reagent2"),
        R::new("spell_template", "reagent3"),
        R::new("spell_template", "reagent4"),
        R::new("spell_template", "reagent5"),
        R::new("spell_template", "reagent6"),
        R::new("spell_template", "reagent7"),
        R::new("spell_template", "reagent8"),
        R::new("spell_template", "totem1"),
        R::new("spell_template", "totem2"),
    ]
};

/// **Whether this table can hold a row that is created or removed at all.**
///
/// Both, for `item_template`. A removal is written only as the whole item —
/// see [`delete_statements`] — and is safe on a restart only: see
/// [`reload_is_safe`].
pub fn can_live(table: &str, life: Life) -> bool {
    match life {
        Life::Update => true,
        Life::Insert | Life::Delete => table == TEMPLATE,
    }
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// **The columns a list row, an icon and a heading need**, as the `SELECT` the
/// browse and search queries share.
///
/// Not `*`: the whole table is 129 columns over 24,000 rows, and the browser
/// reads every row at once so that searching is a walk of memory rather than a
/// round trip per keystroke. These eleven are what a row of the list draws.
const BRIEF: &str = "t.`entry`, t.`patch`, t.`name`, t.`class`, t.`subclass`, t.`quality`, \
     t.`display_id`, t.`inventory_type`, t.`item_level`, t.`required_level`, t.`flags`";

/// …and the join that picks the row the server would load, which is
/// `LoadItemPrototypes`' correlated sub-select written as a derived table.
///
/// The rows it picks are the same rows. The shape is
/// [`crate::creature::spawns_on_map_query`]'s and it is there for the same
/// measured reason: grouping the table by entry once is a fraction of the cost
/// of a sub-select per row.
fn winning_join(wow_patch: u32) -> String {
    format!(
        "FROM `item_template` t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `item_template` \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch`"
    )
}

/// **Every item the server would load**, briefly — what the browser reads once.
pub fn all_items_query(wow_patch: u32) -> String {
    format!(
        "SELECT {BRIEF} {} ORDER BY t.`entry`",
        winning_join(wow_patch)
    )
}

/// …and the same for one entry, for a row a list has not got.
pub fn brief_query(entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT {BRIEF} {} WHERE t.`entry` = {entry}",
        winning_join(wow_patch)
    )
}

/// **The whole row the server would load for one entry**, with its `patch`.
///
/// The `patch` is selected rather than assumed, because it is half the key an
/// edit is written under — see the module comment.
pub fn winning_template_query(entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT * FROM `item_template` t1 WHERE `entry` = {entry} AND `patch` = \
         (SELECT MAX(t2.`patch`) FROM `item_template` t2 \
          WHERE t2.`entry` = t1.`entry` AND t2.`patch` <= {wow_patch})"
    )
}

/// The row a key names — what an undo is taken from.
pub fn row_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT * FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// **Whether a row is already there**, which is what an apply asks before it
/// creates one — see [`crate::creature::exists_query`], where the reason is.
pub fn exists_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT 1 FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// **The highest entry the table holds**, which is where a new one is numbered
/// from when the reserved base is already in use.
pub const MAX_ENTRY_QUERY: &str = "SELECT MAX(`entry`) AS `entry` FROM `item_template`";

/// **Which items point at a display id**, for the picker's own question: is
/// this appearance already in use, and by what.
///
/// Capped, because a common appearance is worn by hundreds of rows and the
/// answer a person wants is "these, and how many more".
pub fn users_of_display_query(display_id: u32, wow_patch: u32, limit: usize) -> String {
    format!(
        "SELECT {BRIEF} {} WHERE t.`display_id` = {display_id} \
         ORDER BY t.`quality` DESC, t.`item_level` DESC LIMIT {limit}",
        winning_join(wow_patch)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The schema is `LoadItemPrototypes`' `SELECT`, column for column.**
    ///
    /// The list in the test is the query at `ObjectMgr.cpp:3794` with the
    /// backticks taken off, and it is the check that catches the one way this
    /// file goes wrong: a column inserted in the middle by an upstream update,
    /// which makes every name after it describe the column before it.
    #[test]
    fn every_column_is_the_servers_own_in_the_servers_own_order() {
        const SELECTED: [&str; 129] = [
            "entry", "class", "subclass", "name", "description", "display_id", "quality", "flags",
            "buy_count", "buy_price", "sell_price", "inventory_type", "allowable_class",
            "allowable_race", "item_level", "required_level", "required_skill",
            "required_skill_rank", "required_spell", "required_honor_rank", "required_city_rank",
            "required_reputation_faction", "required_reputation_rank", "max_count", "stackable",
            "container_slots", "stat_type1", "stat_value1", "stat_type2", "stat_value2",
            "stat_type3", "stat_value3", "stat_type4", "stat_value4", "stat_type5", "stat_value5",
            "stat_type6", "stat_value6", "stat_type7", "stat_value7", "stat_type8", "stat_value8",
            "stat_type9", "stat_value9", "stat_type10", "stat_value10", "delay", "range_mod",
            "ammo_type", "dmg_min1", "dmg_max1", "dmg_type1", "dmg_min2", "dmg_max2", "dmg_type2",
            "dmg_min3", "dmg_max3", "dmg_type3", "dmg_min4", "dmg_max4", "dmg_type4", "dmg_min5",
            "dmg_max5", "dmg_type5", "block", "armor", "holy_res", "fire_res", "nature_res",
            "frost_res", "shadow_res", "arcane_res", "spellid_1", "spelltrigger_1",
            "spellcharges_1", "spellppmrate_1", "spellcooldown_1", "spellcategory_1",
            "spellcategorycooldown_1", "spellid_2", "spelltrigger_2", "spellcharges_2",
            "spellppmrate_2", "spellcooldown_2", "spellcategory_2", "spellcategorycooldown_2",
            "spellid_3", "spelltrigger_3", "spellcharges_3", "spellppmrate_3", "spellcooldown_3",
            "spellcategory_3", "spellcategorycooldown_3", "spellid_4", "spelltrigger_4",
            "spellcharges_4", "spellppmrate_4", "spellcooldown_4", "spellcategory_4",
            "spellcategorycooldown_4", "spellid_5", "spelltrigger_5", "spellcharges_5",
            "spellppmrate_5", "spellcooldown_5", "spellcategory_5", "spellcategorycooldown_5",
            "bonding", "page_text", "page_language", "page_material", "start_quest", "lock_id",
            "material", "sheath", "random_property", "set_id", "max_durability", "area_bound",
            "map_bound", "duration", "bag_family", "disenchant_id", "food_type", "min_money_loot",
            "max_money_loot", "wrapped_gift", "extra_flags", "other_team_entry",
        ];
        let ours: Vec<&str> = TEMPLATE_COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(ours, SELECTED.to_vec());
    }

    /// **`entry` is the only key column in the list**, and `patch` is not in it
    /// at all: it is the other half of the key and [`template_key`] is where it
    /// goes. A `patch` among the columns would be a column an `INSERT` named
    /// twice.
    #[test]
    fn the_key_is_the_entry_and_the_patch_is_not_a_column() {
        let keys: Vec<&str> = TEMPLATE_COLUMNS
            .iter()
            .filter(|column| !column.editable())
            .map(|column| column.name)
            .collect();
        assert_eq!(keys, vec!["entry"]);
        assert!(column(TEMPLATE, "patch").is_none());
        let key = template_key(2589, 0);
        assert_eq!(key.text(), "entry=2589;patch=0");
        assert_eq!(key.where_clause(), "`entry` = 2589 AND `patch` = 0");
    }

    /// **A new item names every column the table has**, which is what the
    /// editor's plan refuses an `INSERT` without.
    #[test]
    fn a_new_item_names_every_editable_column() {
        let made = new_item("Test Item");
        let named: Vec<&str> = made.iter().map(|change| change.column).collect();
        for column in TEMPLATE_COLUMNS.iter().filter(|column| column.editable()) {
            assert!(named.contains(&column.name), "{} is missing", column.name);
        }
        assert_eq!(named.len(), TEMPLATE_COLUMNS.len() - 1, "one per editable column, once");
    }

    /// …and that it is a row the server loads and the client draws: junk, of
    /// common quality, stacking to one, worn by anybody.
    #[test]
    fn a_new_item_starts_from_values_somebody_chose() {
        let made = new_item("Test Item");
        let of = |name: &str| {
            made.iter()
                .find(|change| change.column == name)
                .map(|change| change.value.as_str())
                .unwrap_or("")
        };
        assert_eq!(of("class"), "15");
        assert_eq!(of("quality"), "1");
        assert_eq!(of("stackable"), "1");
        assert_eq!(of("buy_count"), "1");
        assert_eq!(of("allowable_class"), "-1");
        assert_eq!(of("allowable_race"), "-1");
        assert_eq!(of("name"), "'Test Item'");
    }

    /// **A removal names the entry alone**, so every content-patch version
    /// goes and the server cannot fall back to an older one, and it takes the
    /// rows that hand the item out with it.
    #[test]
    fn a_removed_item_takes_every_version_and_its_dependents() {
        assert!(can_live(TEMPLATE, Life::Delete));
        let sql = statements(TEMPLATE, &template_key(835, 10), Life::Delete, &[]);
        assert_eq!(sql.len(), 1 + DEPENDENTS.len());
        assert_eq!(sql[0], "DELETE FROM `item_template` WHERE `entry` = 835;");
        assert!(sql.contains(&"DELETE FROM `npc_vendor` WHERE `item` = 835;".to_string()));
        assert!(sql.contains(
            &"DELETE FROM `creature_loot_template` WHERE `item` = 835 AND `mincountOrRef` > 0;"
                .to_string()
        ));
        // A column of a row about something else is not a dependent.
        assert!(!sql.iter().any(|statement| statement.contains("quest_template")));
    }

    /// **Every dependent is a column the cascade list already names**, which is
    /// the list that was checked against the reference install's
    /// `information_schema`.
    #[test]
    fn every_dependent_is_a_known_reference() {
        for dependent in DEPENDENTS {
            assert!(
                REFERENCES.iter().any(|known| known.table == dependent.table
                    && known.column == dependent.column
                    && known.only == dependent.only),
                "{}.{}",
                dependent.table,
                dependent.column
            );
        }
    }

    /// **A reload may not follow a removal**, and may follow anything else.
    #[test]
    fn a_removal_forbids_the_reload() {
        assert!(reload_is_safe(&[Life::Update, Life::Insert]));
        assert!(!reload_is_safe(&[Life::Update, Life::Delete]));
    }

    /// A creation is a `DELETE` and an `INSERT`, so applying twice means the
    /// same as applying once.
    #[test]
    fn a_creation_is_idempotent() {
        let key = template_key(2_000_000, 0);
        let changes = new_item("Test Item");
        let sql = statements(TEMPLATE, &key, Life::Insert, &changes);
        assert_eq!(sql.len(), 2);
        assert!(sql[0].starts_with("DELETE FROM `item_template` WHERE `entry` = 2000000"), "{}", sql[0]);
        assert!(sql[1].starts_with("INSERT INTO `item_template` (`entry`, `patch`,"), "{}", sql[1]);
    }

    /// An edit is an `UPDATE` naming the whole key.
    #[test]
    fn an_edit_names_both_key_columns() {
        let key = template_key(2589, 0);
        let changes = vec![Assignment { column: "quality", value: "3".to_string() }];
        let sql = statements(TEMPLATE, &key, Life::Update, &changes);
        assert_eq!(
            sql,
            vec!["UPDATE `item_template` SET `quality` = 3 WHERE `entry` = 2589 AND `patch` = 0;"]
        );
    }

    /// No reference is listed twice, which would be the same `UPDATE` twice
    /// and an undo that is one statement too long.
    #[test]
    fn no_reference_is_listed_twice() {
        for (index, reference) in REFERENCES.iter().enumerate() {
            let again = REFERENCES[index + 1..]
                .iter()
                .any(|other| other.table == reference.table && other.column == reference.column);
            assert!(!again, "{}.{} is listed twice", reference.table, reference.column);
        }
    }

    /// **The subclass list is the class's**, which is the whole of why
    /// [`Kind::Subclass`] exists: 0 is three different words in three classes.
    #[test]
    fn a_subclass_is_read_through_its_class() {
        assert_eq!(value_word(subclasses(2), 0), "Axe (one-hand)");
        assert_eq!(value_word(subclasses(4), 0), "Miscellaneous");
        assert_eq!(value_word(subclasses(4), 1), "Cloth");
        assert_eq!(value_word(subclasses(9), 5), "Cooking Recipe");
        // A class the game does not have draws the number.
        assert_eq!(value_word(subclasses(99), 3), "3");
    }

    /// Every class in [`CLASSES`] has a subclass list, and no list is empty —
    /// the pairing `MaxItemSubclassValues` states from the other side.
    #[test]
    fn every_class_has_its_subclasses() {
        for class in CLASSES {
            assert!(
                !subclasses(class.value).is_empty(),
                "{} has no subclass list",
                class.name
            );
        }
    }

    /// The reserved base is inside the column's own ceiling, with room above it.
    #[test]
    fn a_new_entry_is_clear_of_upstream_and_inside_the_column() {
        assert!(RESERVED_ENTRY_BASE < MAX_ENTRY);
        assert!(MAX_ENTRY - RESERVED_ENTRY_BASE > 10_000_000 / 2);
    }

    /// The browse query reads the row the server would load, and says so in the
    /// join rather than in a sub-select per row.
    #[test]
    fn the_browse_query_picks_the_winning_patch() {
        let sql = all_items_query(10);
        assert!(sql.contains("MAX(`patch`) AS `patch`"), "{sql}");
        assert!(sql.contains("WHERE `patch` <= 10"), "{sql}");
        assert!(sql.contains("ORDER BY t.`entry`"), "{sql}");
    }
}
