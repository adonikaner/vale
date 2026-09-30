//! The four shipped tables that the bags and the paper doll read. Nothing in
//! this module concerns the wire.
//!
//! An item's identity comes from the server ([`vale_protocol::state::query`]).
//! The archives hold the rest: what a slot is called, what art an empty slot
//! shows, where an icon lives and what an item's class is called. This module
//! reads them. None of it needs a renderer or a session, so it can be checked
//! without a window.
//!
//! ```text
//! PaperDollItemFrame.dbc   36 rows  (slotName, art, slotId)   GetInventorySlotInfo
//! StringLookups.dbc         9 rows  a directory per row       where an icon lives
//! ItemClass.dbc            16 rows  "Weapon", "Armor"         the tooltip's type line
//! ItemSubClass.dbc         72 rows  "Sword" / "Swords"        …and its right cell
//! ```
//!
//! ## How an icon name becomes a path
//!
//! `ItemDisplayInfo.dbc`'s `inventoryIcon` is a bare name, such as
//! `"INV_Sword_39"`, and the client builds a path from it. The directory is
//! not a constant in the client: `GetContainerItemInfo` takes it from row 3 of
//! `DBFilesClient\StringLookups.dbc`, second column, and joins directory and
//! name with a `\` separator. The separator is omitted when the directory is
//! empty. Row 3 of the shipped table is `Interface\Icons`, so the result is
//! `Interface\Icons\INV_Sword_39`. This module reads the directory from the
//! table for the same reason.
//!
//! ## `GetInventorySlotInfo` and `PaperDollItemFrame.dbc`
//!
//! `GetInventorySlotInfo(name)` matches `name` case-insensitively against the
//! first column of `PaperDollItemFrame.dbc`, which has three columns (name,
//! art, slot id), and returns `slotId`, `art` and `checkRelic`. `checkRelic`
//! is `1.0` for slot 18, the ranged slot, and `nil` for every other slot. No
//! other slot is treated differently.
//!
//! The table defines the interface's slot numbering: 1..19 worn, 20..23 the
//! four bag slots, 64..75 the bank bags, and 0 for `AmmoSlot`. Because
//! `AmmoSlot` is 0, the ammo slot is declared and never filled.
//! [`vale_protocol::play::items`] carries the same numbering on the update
//! field side.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// The `GlobalStrings.lua` key for an `INVTYPE_*` value. The tooltip's type
/// line and `GetItemInfo`'s `itemEquipLoc` are both built from it.
///
/// This returns a token rather than a word because the word is in
/// `GlobalStrings.lua`, which this crate does not read: `PaperDollFrame.lua`
/// calls `getglobal(equipLoc)` itself, and so does
/// [`crate::interface::strings`] on the client side. Returning `"Two-Hand"`
/// here would invent an English string.
///
/// Four of the twenty-eight tokens have no entry in the shipped
/// `GlobalStrings.lua`: `INVTYPE_AMMO`, `INVTYPE_THROWN`,
/// `INVTYPE_RANGEDRIGHT` and `INVTYPE_QUIVER`. They resolve to nothing, and
/// the left cell of the type line is blank. The 1.12.1 client shows the same
/// blank cell, and three of the 146 cast-failure reasons follow the same rule,
/// so this is not filled in. The tokens are still returned, so an addon
/// reading `itemEquipLoc` sees the value the 1.12.1 client puts there.
///
/// Returns `""` for `INVTYPE_NON_EQUIP` (0) and for any value past the enum.
/// Every consumer tests for `""` before drawing the type line.
pub fn inventory_type_key(inventory_type: u32) -> &'static str {
    match inventory_type {
        1 => "INVTYPE_HEAD",
        2 => "INVTYPE_NECK",
        3 => "INVTYPE_SHOULDER",
        4 => "INVTYPE_BODY",
        5 => "INVTYPE_CHEST",
        6 => "INVTYPE_WAIST",
        7 => "INVTYPE_LEGS",
        8 => "INVTYPE_FEET",
        9 => "INVTYPE_WRIST",
        10 => "INVTYPE_HAND",
        11 => "INVTYPE_FINGER",
        12 => "INVTYPE_TRINKET",
        13 => "INVTYPE_WEAPON",
        14 => "INVTYPE_SHIELD",
        15 => "INVTYPE_RANGED",
        16 => "INVTYPE_CLOAK",
        17 => "INVTYPE_2HWEAPON",
        18 => "INVTYPE_BAG",
        19 => "INVTYPE_TABARD",
        20 => "INVTYPE_ROBE",
        21 => "INVTYPE_WEAPONMAINHAND",
        22 => "INVTYPE_WEAPONOFFHAND",
        23 => "INVTYPE_HOLDABLE",
        24 => "INVTYPE_AMMO",
        25 => "INVTYPE_THROWN",
        26 => "INVTYPE_RANGEDRIGHT",
        27 => "INVTYPE_QUIVER",
        28 => "INVTYPE_RELIC",
        _ => "",
    }
}

/// One row of `PaperDollItemFrame.dbc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotInfo {
    /// The interface's slot id: the value passed to `this:SetID(id)`, and the
    /// id every `GetInventoryItem*` call takes.
    pub id: u32,
    /// The empty-slot art, spelled exactly as in the table (lower-case
    /// `interface\paperdoll\…`, `.blp` suffix included). It is passed through
    /// unaltered because the archive lookup is case-insensitive and the 1.12.1
    /// client passes the string to `SetTexture` unchanged.
    pub art: String,
}

/// `RangedSlot`'s id. It is the only slot for which `GetInventorySlotInfo`
/// returns a non-nil third value (`checkRelic`): a totem, libram or idol goes
/// in the ranged slot, and the paper doll swaps that slot's art for
/// `UI-PaperDoll-Slot-Relic` when the class uses a relic.
pub const RANGED_SLOT: u32 = 18;

/// `AmmoSlot`'s id in the same table: 0. It is the one square on the paper
/// doll that is not a slot. Nothing is stored in it: `PLAYER_AMMO_ID` names an
/// entry the character is carrying, and every read of the ammo slot reads that
/// entry. See `vale_protocol::play::items::set_ammo_body`.
pub const AMMO_SLOT: u32 = 0;

/// The coin icon that `GetCoinIcon(copper)` returns for an amount. It is a
/// cascade of five thresholds, not a formula.
///
/// Six icons and five thresholds, in the order the client tests them:
///
/// ```text
/// < 10        INV_Misc_Coin_05     a single copper
/// < 100       INV_Misc_Coin_06
/// < 1000      INV_Misc_Coin_03
/// < 10000     INV_Misc_Coin_04
/// < 100000    INV_Misc_Coin_01
/// otherwise   INV_Misc_Coin_02     a heap of gold
/// ```
///
/// The icon numbers do not follow the amounts: `_05` and `_06` are the two
/// smallest and `_01` and `_02` the two largest. A cascade written from the
/// names alone would draw a pile of gold on a letter carrying four copper.
///
/// Returns the bare name, like every other icon in this crate.
/// [`InventoryTables::icon_path`] turns it into a path using `StringLookups`
/// row 3, the same directory the 1.12.1 client uses.
pub fn coin_icon(copper: u32) -> &'static str {
    match copper {
        ..10 => "INV_Misc_Coin_05",
        ..100 => "INV_Misc_Coin_06",
        ..1_000 => "INV_Misc_Coin_03",
        ..10_000 => "INV_Misc_Coin_04",
        ..100_000 => "INV_Misc_Coin_01",
        _ => "INV_Misc_Coin_02",
    }
}

/// The row of `StringLookups.dbc` that holds the icon directory. It is the
/// row `GetContainerItemInfo` takes its directory from; see the module
/// comment.
const ICON_DIRECTORY_ROW: u32 = 3;

/// The archive fallback for that directory, used when the table is absent.
///
/// The fallback is a stated value rather than an empty string because an icon
/// name without a directory is a bare `"INV_Sword_39"`, which resolves to no
/// file and draws an empty square for every item in the game. The value is
/// what row 3 of the shipped table holds, so the fallback and the table agree.
const ICON_DIRECTORY: &str = "Interface\\Icons";

/// The paper doll's slot table, `StringLookups`' directories, and the two class
/// name tables: everything about an item that comes from a file rather than
/// from the wire.
#[derive(Debug, Default)]
pub struct ItemTables {
    /// Keyed by the lower-cased slot name, because
    /// `PaperDollItemSlotButton_OnLoad` passes `strsub(slotName, 10)` (the
    /// frame's name minus `"Character"`) and the client compares
    /// case-insensitively.
    slots: HashMap<String, SlotInfo>,
    /// `StringLookups.dbc` row -> directory.
    directories: HashMap<u32, String>,
    /// `ItemClass.dbc`: class id -> name.
    classes: HashMap<u32, String>,
    /// `ItemSubClass.dbc`: (class, subclass) -> (singular, plural).
    subclasses: HashMap<(u32, u32), (String, String)>,
    /// The `(class, subclass)` rows whose display flags have bit 0 set: the
    /// item plate draws no subclass word for them. See
    /// [`ItemTables::subclass_on_plate`].
    unnamed_on_plate: std::collections::HashSet<(u32, u32)>,
}

/// `PaperDollItemFrame.dbc` columns: the frame's name, its empty art, the id.
mod paperdoll_fields {
    pub const NAME: usize = 0;
    pub const ART: usize = 1;
    pub const ID: usize = 2;
    /// Three. The 1.12.1 client accepts the file only with exactly three
    /// fields and a 12-byte record.
    pub const COUNT: usize = 3;
}

/// `ItemClass.dbc` columns: the id, two ids nothing here reads, then the name.
mod class_fields {
    pub const ID: usize = 0;
    pub const NAME: usize = 3;
}

/// `ItemSubClass.dbc`: 28 fields. The class and subclass are in the first two
/// columns, the display flags at 5, the singular name at 10 and the plural at
/// 19.
///
/// The file does not name its columns; these indices were measured. Row
/// (2, 2) reads `"Bow"` at 10 and `"Bows"` at 19, (2, 18) reads `"Crossbow"`
/// and `"Crossbows"`, and (4, 8) reads `"Idol"` and `"Idols"`. The item plate
/// prints the singular. Bit 0 of the
/// display flags is set on the rows the plate prints no subclass for:
/// Consumable (0, 0), Miscellaneous armour (4, 0), Miscellaneous weapons
/// (2, 14), Trade Goods (7, 0), every recipe row and Quest (12, 0).
mod subclass_fields {
    pub const CLASS: usize = 0;
    pub const SUBCLASS: usize = 1;
    pub const DISPLAY_FLAGS: usize = 5;
    pub const NAME: usize = 10;
    pub const PLURAL: usize = 19;
    /// The display-flags bit that hides the subclass on the item plate.
    pub const NO_PLATE_NAME: u32 = 0x1;
}

impl ItemTables {
    /// Reads all four tables. Each is optional, and each missing table affects
    /// a different part: without `PaperDollItemFrame` the twenty-four
    /// paper-doll buttons have no id and no empty art; without `StringLookups`
    /// icon paths use the fallback directory above; without the class tables
    /// the tooltip has no type line.
    pub fn parse(
        paperdoll: &[u8],
        string_lookups: &[u8],
        item_class: &[u8],
        item_subclass: &[u8],
    ) -> ItemTables {
        let mut out = ItemTables::default();

        if let Ok(dbc) = Dbc::parse(paperdoll) {
            if dbc.field_count >= paperdoll_fields::COUNT {
                for record in 0..dbc.record_count {
                    let (Some(name), Some(id)) = (
                        dbc.string_at(record, paperdoll_fields::NAME),
                        dbc.u32_at(record, paperdoll_fields::ID),
                    ) else {
                        continue;
                    };
                    if name.is_empty() {
                        continue;
                    }
                    out.slots.insert(
                        name.to_ascii_lowercase(),
                        SlotInfo {
                            id,
                            art: dbc.string_at(record, paperdoll_fields::ART).unwrap_or_default(),
                        },
                    );
                }
            }
        }

        if let Ok(dbc) = Dbc::parse(string_lookups) {
            for record in 0..dbc.record_count {
                let (Some(id), Some(path)) = (dbc.u32_at(record, 0), dbc.string_at(record, 1))
                else {
                    continue;
                };
                if !path.is_empty() {
                    out.directories.insert(id, path);
                }
            }
        }

        if let Ok(dbc) = Dbc::parse(item_class) {
            for record in 0..dbc.record_count {
                let (Some(id), Some(name)) = (
                    dbc.u32_at(record, class_fields::ID),
                    dbc.string_at(record, class_fields::NAME),
                ) else {
                    continue;
                };
                if !name.is_empty() {
                    out.classes.insert(id, name);
                }
            }
        }

        if let Ok(dbc) = Dbc::parse(item_subclass) {
            for record in 0..dbc.record_count {
                let (Some(class), Some(subclass)) = (
                    dbc.u32_at(record, subclass_fields::CLASS),
                    dbc.u32_at(record, subclass_fields::SUBCLASS),
                ) else {
                    continue;
                };
                // Row 0 is the `(-1, -1)` "no subclass" placeholder and would
                // otherwise key on two enormous numbers.
                if class == u32::MAX || subclass == u32::MAX {
                    continue;
                }
                let name = dbc.string_at(record, subclass_fields::NAME).unwrap_or_default();
                let plural = dbc.string_at(record, subclass_fields::PLURAL).unwrap_or_default();
                out.subclasses.insert((class, subclass), (name, plural));
                let flags = dbc
                    .u32_at(record, subclass_fields::DISPLAY_FLAGS)
                    .unwrap_or(0);
                if flags & subclass_fields::NO_PLATE_NAME != 0 {
                    out.unnamed_on_plate.insert((class, subclass));
                }
            }
        }
        out
    }

    /// `GetInventorySlotInfo(name)`: the id, the empty art, and whether this is
    /// the slot a relic can go in.
    ///
    /// `None` for a name the table does not carry. The 1.12.1 client returns
    /// nothing for such a name, so the caller's `this:SetID(id)` receives nil.
    pub fn slot(&self, name: &str) -> Option<(&SlotInfo, bool)> {
        let info = self.slots.get(&name.to_ascii_lowercase())?;
        Some((info, info.id == RANGED_SLOT))
    }

    /// How many slot rows were read. `vale` checks report this number.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// A full icon path from an `ItemDisplayInfo` icon name.
    ///
    /// The path is built the way the client builds it: the directory, a
    /// backslash if the directory is non-empty, then the name. An empty name
    /// returns `None` rather than the bare directory, because an item whose
    /// display row has no icon draws nothing, not the folder.
    pub fn icon_path(&self, icon_name: &str) -> Option<String> {
        if icon_name.is_empty() {
            return None;
        }
        let directory = self
            .directories
            .get(&ICON_DIRECTORY_ROW)
            .map_or(ICON_DIRECTORY, String::as_str);
        Some(if directory.is_empty() {
            icon_name.to_string()
        } else {
            format!("{directory}\\{icon_name}")
        })
    }

    /// `ItemClass.dbc`'s word for a class: "Weapon", "Armor", "Consumable".
    /// Empty for a class the table does not carry, so the tooltip is shorter
    /// rather than wrong.
    pub fn class_name(&self, class: u32) -> &str {
        self.classes.get(&class).map_or("", String::as_str)
    }

    /// `ItemSubClass.dbc`'s word for a subclass, in the singular: "Sword",
    /// "Cloth", "Bow". This is the column the item plate prints; see
    /// [`Self::subclass_on_plate`] for the rows it leaves out.
    pub fn subclass_name(&self, class: u32, subclass: u32) -> &str {
        self.subclasses
            .get(&(class, subclass))
            .map_or("", |(name, _)| name.as_str())
    }

    /// The subclass word the item plate prints beside the slot: the singular,
    /// or empty for a row whose display flags hide it. A potion's plate has no
    /// "Consumable" and a ring's no "Miscellaneous" for this reason. The
    /// 1.12.1 client also prints no subclass for a cloak (`INVTYPE_CLOAK`),
    /// which is decided by the caller from the inventory type.
    pub fn subclass_on_plate(&self, class: u32, subclass: u32) -> &str {
        if self.unnamed_on_plate.contains(&(class, subclass)) {
            return "";
        }
        self.subclass_name(class, subclass)
    }

    /// The plural form of the same row ("Swords", "Bows"). `GetItemInfo`'s
    /// `itemSubType` is documented as returning this form.
    pub fn subclass_plural(&self, class: u32, subclass: u32) -> &str {
        self.subclasses
            .get(&(class, subclass))
            .map_or("", |(_, plural)| plural.as_str())
    }

    /// How many class and subclass rows were read.
    pub fn class_counts(&self) -> (usize, usize) {
        (self.classes.len(), self.subclasses.len())
    }

    /// The word a refused cast substitutes for its `%s`, such as "Sword" or
    /// "Weapon".
    ///
    /// `SMSG_CAST_RESULT`'s three equipped-item refusals quote the spell's
    /// `EquippedItemClass` and `EquippedItemSubClassMask`, and
    /// `GlobalStrings.lua` writes all three messages with a placeholder
    /// (`"Must have a %s equipped in the main hand"`). Without this word the
    /// player reads the raw placeholder, as a warrior bug report showed. See
    /// `vale_protocol::play::spells::EquipRequirement`.
    ///
    /// The two branches rest on different evidence. A mask with exactly one
    /// bit names one subclass row: the spell wants a sword and the row says
    /// "Sword". A mask with several bits, the usual shape for "any melee
    /// weapon", names no single row. Falling back to the class name ("Weapon")
    /// there is a choice, not a measurement: what the 1.12.1 client shows in
    /// that case is not established, and the only alternative was to leave the
    /// placeholder. `None` when no table was loaded or the row is absent; the
    /// caller then draws no sentence rather than one with a gap in it.
    pub fn requirement_name(&self, class: u32, subclass_mask: u32) -> Option<&str> {
        // `count_ones() == 1` rather than a loop: the bit index is the
        // subclass id, so a single-bit mask resolves with no search.
        if subclass_mask.count_ones() == 1 {
            let name = self.subclass_name(class, subclass_mask.trailing_zeros());
            if !name.is_empty() {
                return Some(name);
            }
        }
        let name = self.class_name(class);
        (!name.is_empty()).then_some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five thresholds, tested at their edges. The icon numbering does not
    /// follow the amounts, so an off-by-one error still produces a valid coin
    /// icon rather than a broken path, and only a test catches it.
    #[test]
    fn the_coin_picture_steps_at_ten_and_then_by_tens() {
        assert_eq!(coin_icon(0), "INV_Misc_Coin_05");
        assert_eq!(coin_icon(9), "INV_Misc_Coin_05");
        assert_eq!(coin_icon(10), "INV_Misc_Coin_06");
        assert_eq!(coin_icon(99), "INV_Misc_Coin_06");
        assert_eq!(coin_icon(100), "INV_Misc_Coin_03");
        assert_eq!(coin_icon(999), "INV_Misc_Coin_03");
        assert_eq!(coin_icon(1_000), "INV_Misc_Coin_04");
        assert_eq!(coin_icon(9_999), "INV_Misc_Coin_04");
        assert_eq!(coin_icon(10_000), "INV_Misc_Coin_01");
        assert_eq!(coin_icon(99_999), "INV_Misc_Coin_01");
        assert_eq!(coin_icon(100_000), "INV_Misc_Coin_02");
        assert_eq!(coin_icon(u32::MAX), "INV_Misc_Coin_02");
    }
    

    /// Build a DBC with `fields` columns from `rows`, where a value is either a
    /// number or an offset into `strings`.
    fn dbc(rows: &[Vec<u32>], fields: usize, strings: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for row in rows {
            for field in 0..fields {
                out.extend_from_slice(&row.get(field).copied().unwrap_or(0).to_le_bytes());
            }
        }
        out.extend_from_slice(strings);
        out
    }

    fn tables() -> ItemTables {
        // "\0HeadSlot\0interface\\paperdoll\\Head.blp\0"
        let paperdoll_strings = b"\0HeadSlot\0art-head\0RangedSlot\0art-ranged\0".to_vec();
        let paperdoll = dbc(
            &[vec![1, 10, 1], vec![19, 31, RANGED_SLOT]],
            3,
            &paperdoll_strings,
        );
        // Row 3 is the icon directory and row 1 is a different directory.
        // Rows are keyed by id, not by position, so row 1 must not be used.
        let lookup_strings = b"\0Interface\\Cursor\0Interface\\Icons\0".to_vec();
        let lookups = dbc(&[vec![1, 1], vec![3, 18]], 2, &lookup_strings);
        let class_strings = b"\0Weapon\0".to_vec();
        let classes = dbc(&[vec![2, 0, 0, 1]], 4, &class_strings);
        let sub_strings = b"\0Sword\0Swords\0".to_vec();
        let mut sub_row = vec![0u32; 20];
        sub_row[subclass_fields::CLASS] = 2;
        sub_row[subclass_fields::SUBCLASS] = 7;
        sub_row[subclass_fields::NAME] = 1;
        sub_row[subclass_fields::PLURAL] = 7;
        let placeholder = vec![u32::MAX; 20];
        let subclasses = dbc(&[placeholder, sub_row], 20, &sub_strings);
        ItemTables::parse(&paperdoll, &lookups, &classes, &subclasses)
    }

    /// The slot lookup is by name and case-insensitive, because the interface
    /// passes a substring of a frame name and the client ignores case when
    /// matching it.
    #[test]
    fn a_slot_is_found_by_the_name_the_frame_carries() {
        let tables = tables();
        let (head, relic) = tables.slot("HeadSlot").expect("HeadSlot");
        assert_eq!(head.id, 1);
        assert_eq!(head.art, "art-head");
        assert!(!relic, "a helmet slot takes no relic");
        assert!(tables.slot("headslot").is_some(), "case does not matter");
        assert!(tables.slot("PocketSlot").is_none());
    }

    /// `checkRelic` is set for the ranged slot and for no other slot.
    #[test]
    fn only_the_ranged_slot_answers_check_relic() {
        let tables = tables();
        assert!(tables.slot("RangedSlot").expect("ranged").1);
    }

    /// The icon directory comes from `StringLookups.dbc`, not from a literal.
    #[test]
    fn an_icon_name_becomes_a_path_through_string_lookups() {
        let tables = tables();
        assert_eq!(
            tables.icon_path("INV_Sword_39").as_deref(),
            Some("Interface\\Icons\\INV_Sword_39")
        );
        assert_eq!(tables.icon_path("").as_deref(), None, "no icon is not the folder");
    }

    /// Without `StringLookups.dbc` the icon path still gets the fallback
    /// directory. A bare name would resolve to no file and draw nothing.
    #[test]
    fn an_absent_lookup_table_falls_back_rather_than_dropping_the_folder() {
        let tables = ItemTables::parse(&[], &[], &[], &[]);
        assert_eq!(
            tables.icon_path("INV_Sword_39").as_deref(),
            Some("Interface\\Icons\\INV_Sword_39")
        );
        assert_eq!(tables.slot_count(), 0);
        assert_eq!(tables.class_name(2), "");
    }

    /// The two class tables, and the placeholder row that must not become a key.
    #[test]
    fn the_class_names_read_and_the_minus_one_row_is_skipped() {
        let tables = tables();
        assert_eq!(tables.class_name(2), "Weapon");
        assert_eq!(tables.subclass_name(2, 7), "Sword");
        assert_eq!(tables.subclass_plural(2, 7), "Swords");
        assert_eq!(tables.class_counts(), (1, 1), "the (-1, -1) row is not a subclass");
        assert_eq!(tables.subclass_name(2, 99), "");
    }

    /// A subclass row with bit 0 of its display flags set keeps its name but
    /// prints none on the item plate.
    #[test]
    fn a_flagged_subclass_prints_nothing_on_the_plate() {
        let strings = b"\0Consumable\0Sword\0".to_vec();
        let mut consumable = vec![0u32; 28];
        consumable[subclass_fields::NAME] = 1;
        consumable[subclass_fields::DISPLAY_FLAGS] = 1;
        let mut sword = vec![0u32; 28];
        sword[subclass_fields::CLASS] = 2;
        sword[subclass_fields::SUBCLASS] = 7;
        sword[subclass_fields::NAME] = 12;
        sword[subclass_fields::DISPLAY_FLAGS] = 2;
        let tables = ItemTables::parse(&[], &[], &[], &dbc(&[consumable, sword], 28, &strings));
        assert_eq!(tables.subclass_name(0, 0), "Consumable");
        assert_eq!(tables.subclass_on_plate(0, 0), "");
        assert_eq!(tables.subclass_on_plate(2, 7), "Sword", "bit 1 alone is not the hiding bit");
    }

    /// The word that fills a refused cast's `%s`. A one-bit mask names its
    /// subclass row. A mask with several bits names no row and falls back to
    /// the class; the doc comment on `requirement_name` records that fallback
    /// as a choice, not a measurement.
    #[test]
    fn a_requirement_names_its_subclass_when_the_mask_names_exactly_one() {
        let tables = tables();

        // Only bit 7 set: the subclass id is the bit index.
        assert_eq!(tables.requirement_name(2, 1 << 7), Some("Sword"));

        // Several bits ("any melee weapon") name no single row, so the class.
        assert_eq!(tables.requirement_name(2, (1 << 7) | (1 << 0)), Some("Weapon"));

        // An empty mask is not one bit either.
        assert_eq!(tables.requirement_name(2, 0), Some("Weapon"));

        // A single bit whose row is absent still falls back rather than
        // answering the empty string, which would draw "Must have a  equipped".
        assert_eq!(tables.requirement_name(2, 1 << 9), Some("Weapon"));

        // With no tables loaded there is no word, so no sentence is drawn.
        let none = ItemTables::parse(&[], &[], &[], &[]);
        assert_eq!(none.requirement_name(2, 1 << 7), None);
    }
}
