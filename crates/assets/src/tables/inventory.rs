//! **The four shipped tables the bags and the paper doll are drawn from**, and
//! nothing about the wire.
//!
//! An item's *identity* comes from the server ([`vale_protocol::state::query`]);
//! what a slot is **called**, what art an empty one shows, where an icon lives
//! and what an item's class is called are all in the archives, and this module
//! is where they are read. None of it
//! needs a renderer or a session, so it is checkable with no window.
//!
//! ```text
//! PaperDollItemFrame.dbc   36 rows  (slotName, art, slotId)   GetInventorySlotInfo
//! StringLookups.dbc         9 rows  a directory per row       where an icon lives
//! ItemClass.dbc            16 rows  "Weapon", "Armor"         the tooltip's type line
//! ItemSubClass.dbc         72 rows  "Sword" / "Swords"        …and its right cell
//! ```
//!
//! ## The icon path is not a literal, and this is the one that surprises
//!
//! `ItemDisplayInfo.dbc`'s `inventoryIcon` is a **bare name** —
//! `"INV_Sword_39"` — and the client makes a path out of it. The directory it
//! prefixes is not compiled in: `GetContainerItemInfo` looks up index **3**
//! of a table loaded from `DBFilesClient\StringLookups.dbc`, takes that
//! row's second column, and formats `"%s%s%s"` with a `"\"`
//! separator that is dropped when the directory is empty. Row 3 of that table
//! is `Interface\Icons`, so the answer is
//! `Interface\Icons\INV_Sword_39` — but by way of a shipped table rather than
//! by a constant, which is why it is read here.
//!
//! ## `GetInventorySlotInfo` is a table walk, not a switch
//!
//! The client walks an array of `{name, art, slotId}` records, comparing the
//! caller's string case-insensitively against the first column and pushing
//! `(slotId, art, checkRelic)`. The array is loaded from
//! `PaperDollItemFrame.dbc`, three columns wide. `checkRelic`
//! is `1.0` when `slotId - 1 == 0x11`, i.e. **slot 18, the ranged slot**, and
//! `nil` otherwise; that is the only special case in the function.
//!
//! The table is what states the interface's slot numbering: 1..19 worn, 20..23
//! the four bag slots, 64..75 the bank bags, and **0 for `AmmoSlot`** — which
//! is why an ammo slot is declared and never filled. See
//! [`vale_protocol::play::items`], which carries the same numbering from the field
//! side.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// **The `GlobalStrings.lua` key for an `INVTYPE_*` value** — the word the
/// tooltip's type line and `GetItemInfo`'s `itemEquipLoc` are both built from.
///
/// A token rather than a word, because the *word* is in `GlobalStrings.lua` and
/// this crate does not read that file for anyone: `PaperDollFrame.lua` does
/// `getglobal(equipLoc)` itself, and so does [`crate::interface::strings`] on the client
/// side. Returning `"Two-Hand"` here would be inventing an English string.
///
/// **Four of the twenty-eight have no key at all** — `INVTYPE_AMMO`,
/// `INVTYPE_THROWN`, `INVTYPE_RANGEDRIGHT` and `INVTYPE_QUIVER` are absent from
/// the shipped `GlobalStrings.lua`, so they resolve to nothing and the type
/// line's left cell is blank. That is the client's own behaviour and the same
/// rule three of the 146 cast-failure reasons follow; it is not a gap to paper
/// over. The tokens are still returned, so an addon reading `itemEquipLoc` sees
/// the value the real client puts there.
///
/// `""` for `INVTYPE_NON_EQUIP` (0) and for anything past the enum, which is
/// what every consumer tests before drawing the line at all.
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
    /// The interface's own slot id — what `this:SetID(id)` is given, and what
    /// every `GetInventoryItem*` call is addressed by.
    pub id: u32,
    /// The empty-slot art, exactly as the table spells it (lower-case
    /// `interface\paperdoll\…`, `.blp` suffix included). Passed through
    /// unaltered: the archive lookup is case-insensitive and the real client
    /// hands the string straight to `SetTexture`.
    pub art: String,
}

/// **`RangedSlot`'s id**, which is the one value `GetInventorySlotInfo`
/// branches on: a totem, libram or idol goes in the ranged slot, so that slot
/// alone answers a third value and the paper doll swaps its art for
/// `UI-PaperDoll-Slot-Relic` when the class has one.
pub const RANGED_SLOT: u32 = 18;

/// **`AmmoSlot`'s id in the same table: 0.** The one square on the paper
/// doll that is not a slot — nothing is stored in it, `PLAYER_AMMO_ID` names
/// an entry the character is carrying, and every read of it is a read of that
/// entry. See `vale_protocol::play::items::set_ammo_body`.
pub const AMMO_SLOT: u32 = 0;

/// **Which coin picture stands for an amount** — `GetCoinIcon(copper)`'s whole
/// content, and it is a five-way cascade rather than a formula.
///
/// Six pictures and five thresholds, in the order the client compares them:
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
/// **The numbering is not in order and that is the point of reading it rather
/// than guessing it**: `_05` and `_06` are the two smallest and `_01` and `_02`
/// the two largest, which is the opposite of what the names invite. A cascade
/// written from the names alone draws a pile of gold on a letter carrying four
/// copper.
///
/// The **bare name**, like every other icon in this crate — see
/// [`InventoryTables::icon_path`], which is the same `StringLookups` row 3 the
/// reference's own lookup fetches.
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

/// **Which row of `StringLookups.dbc` holds the icon directory.** The literal
/// `3` `GetContainerItemInfo` passes; see the module comment.
const ICON_DIRECTORY_ROW: u32 = 3;

/// The archive fallback for that directory, used when the table is absent.
///
/// Stated rather than silently empty: without a directory an icon name is a
/// bare `"INV_Sword_39"`, which resolves to nothing and draws an empty square
/// for every item in the game. The value is what row 3 of the shipped table
/// says, so the fallback and the reading agree by construction.
const ICON_DIRECTORY: &str = "Interface\\Icons";

/// The paper doll's slot table, `StringLookups`' directories, and the two class
/// name tables — everything about an item that is in a file rather than on the
/// wire.
#[derive(Debug, Default)]
pub struct ItemTables {
    /// Keyed by the **lower-cased** slot name, because
    /// `PaperDollItemSlotButton_OnLoad` passes `strsub(slotName, 10)` — the
    /// frame's own name minus `"Character"` — and the client compares
    /// case-insensitively.
    slots: HashMap<String, SlotInfo>,
    /// `StringLookups.dbc` row -> directory.
    directories: HashMap<u32, String>,
    /// `ItemClass.dbc`: class id -> name.
    classes: HashMap<u32, String>,
    /// `ItemSubClass.dbc`: (class, subclass) -> (singular, plural).
    subclasses: HashMap<(u32, u32), (String, String)>,
}

/// `PaperDollItemFrame.dbc` columns: the frame's name, its empty art, the id.
mod paperdoll_fields {
    pub const NAME: usize = 0;
    pub const ART: usize = 1;
    pub const ID: usize = 2;
    /// Three, and the client's loader checks exactly this and the 12-byte
    /// record size before reading a row.
    pub const COUNT: usize = 3;
}

/// `ItemClass.dbc` — id, then two ids nothing here reads, then the name.
mod class_fields {
    pub const ID: usize = 0;
    pub const NAME: usize = 3;
}

/// `ItemSubClass.dbc` — the class and subclass in the first two columns, the
/// **singular** name at 10 and the **plural** at 19.
///
/// Measured rather than named by the file: row (2, 2) reads `"Axe"` at 10 and
/// `"Axes"` at 19, (2, 18) reads `"Wand"` and `"Wands"`, (4, 8) reads `"Idol"`
/// and `"Idols"`. Which of the two the *client* puts on a tooltip is **not**
/// measured here — see [`ItemTables::subclass_name`], where the choice is
/// stated as a choice.
mod subclass_fields {
    pub const CLASS: usize = 0;
    pub const SUBCLASS: usize = 1;
    pub const NAME: usize = 10;
    pub const PLURAL: usize = 19;
}

impl ItemTables {
    /// Read all four. Every one of them is optional and each absence is its own
    /// degradation: no `PaperDollItemFrame` is twenty-four paper-doll buttons
    /// with no id and no empty art, no `StringLookups` is the stated fallback
    /// above, and no class table is a tooltip missing its type line.
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
            }
        }
        out
    }

    /// `GetInventorySlotInfo(name)` — the id, the empty art, and whether this is
    /// the slot a relic can go in.
    ///
    /// `None` for a name the table does not carry, which is what the real
    /// client's walk falls out of the bottom of: it pushes nothing at all and
    /// the caller's `this:SetID(id)` gets nil.
    pub fn slot(&self, name: &str) -> Option<(&SlotInfo, bool)> {
        let info = self.slots.get(&name.to_ascii_lowercase())?;
        Some((info, info.id == RANGED_SLOT))
    }

    /// How many slot rows were read — the number `vale` checks report.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// **A full icon path from an `ItemDisplayInfo` icon name.**
    ///
    /// The composition the client makes: directory, a backslash if the
    /// directory is non-empty, then the name. An empty name answers `None`
    /// rather than a bare directory — an item whose display row carries no icon
    /// draws nothing, which is different from drawing the folder.
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

    /// `ItemClass.dbc`'s word for a class — "Weapon", "Armor", "Consumable".
    /// Empty for a class the table does not carry, which draws a shorter
    /// tooltip rather than a wrong one.
    pub fn class_name(&self, class: u32) -> &str {
        self.classes.get(&class).map_or("", String::as_str)
    }

    /// …and `ItemSubClass.dbc`'s, in the **singular** — "Sword", "Cloth",
    /// "Bow".
    ///
    /// **The singular is a choice, and it is recorded as one.** The table
    /// carries both forms and this client has not established which column the
    /// tooltip builder reads; the singular is chosen because it
    /// is what the retail plate shows beside a weapon's `INVTYPE_*` word
    /// ("One-Hand / Sword"), and [`Self::subclass_plural`] is beside it for the
    /// other use.
    pub fn subclass_name(&self, class: u32, subclass: u32) -> &str {
        self.subclasses
            .get(&(class, subclass))
            .map_or("", |(name, _)| name.as_str())
    }

    /// The plural form of the same row — "Swords", "Bows" — which is what
    /// `GetItemInfo`'s `itemSubType` is documented as answering.
    pub fn subclass_plural(&self, class: u32, subclass: u32) -> &str {
        self.subclasses
            .get(&(class, subclass))
            .map_or("", |(_, plural)| plural.as_str())
    }

    /// How many class and subclass rows were read.
    pub fn class_counts(&self) -> (usize, usize) {
        (self.classes.len(), self.subclasses.len())
    }

    /// **The word a refused cast puts in its own `%s`** — "Sword", "Weapon".
    ///
    /// `SMSG_CAST_RESULT`'s three equipped-item refusals quote the spell's own
    /// `EquippedItemClass` and `EquippedItemSubClassMask`, and
    /// `GlobalStrings.lua` spells all three of them with a placeholder
    /// (`"Must have a %s equipped in the main hand"`). Without this the
    /// placeholder is what the player reads, which is what the warrior report
    /// was. See `vale_protocol::play::spells::EquipRequirement`.
    ///
    /// **The two branches are not the same kind of claim.** A mask with exactly
    /// one bit names one subclass row and there is nothing to choose: the spell
    /// wants a sword and the row says "Sword". A mask with *several* — the
    /// ordinary shape for "any melee weapon" — has no single row to name, and
    /// falling back to the class name ("Weapon") is a **choice**, not a
    /// measurement: what the reference composes there is not established, and
    /// the honest options were this or the placeholder. `None` when no table
    /// was loaded or the row is absent, which draws nothing at all rather than
    /// a sentence with a hole in it.
    pub fn requirement_name(&self, class: u32, subclass_mask: u32) -> Option<&str> {
        // `count_ones() == 1` rather than a loop: the bit index *is* the
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

    /// **The five thresholds, at their edges** — and the reason this is a test
    /// rather than a glance is that the picture numbering runs backwards
    /// against the amounts, so an off-by-one here is a plausible coin rather
    /// than a broken path.
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
        // Row 3 is the icon directory; row 1 is something else entirely, which
        // is the point of keying by id rather than by position.
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

    /// **The slot lookup is by name and case-insensitive**, because the
    /// interface passes a substring of a frame name and the client compares
    /// with a case-folding compare.
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

    /// `checkRelic` is the ranged slot and only the ranged slot — the one
    /// branch in the client's slot lookup.
    #[test]
    fn only_the_ranged_slot_answers_check_relic() {
        let tables = tables();
        assert!(tables.slot("RangedSlot").expect("ranged").1);
    }

    /// **The icon directory comes out of the table, not out of a literal.**
    #[test]
    fn an_icon_name_becomes_a_path_through_string_lookups() {
        let tables = tables();
        assert_eq!(
            tables.icon_path("INV_Sword_39").as_deref(),
            Some("Interface\\Icons\\INV_Sword_39")
        );
        assert_eq!(tables.icon_path("").as_deref(), None, "no icon is not the folder");
    }

    /// …and without the table at all it is still a path, by the stated
    /// fallback — an item with a bare name draws nothing at all.
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

    /// **The word that fills a refused cast's `%s`.** One bit names its own
    /// subclass row; several have no row to name and fall back to the class,
    /// which is the branch the doc records as a choice rather than a
    /// measurement.
    #[test]
    fn a_requirement_names_its_subclass_when_the_mask_names_exactly_one() {
        let tables = tables();

        // Bit 7 set and nothing else: the subclass id *is* the bit index.
        assert_eq!(tables.requirement_name(2, 1 << 7), Some("Sword"));

        // Several bits — "any melee weapon" — has no single row, so the class.
        assert_eq!(tables.requirement_name(2, (1 << 7) | (1 << 0)), Some("Weapon"));

        // An empty mask is not one bit either.
        assert_eq!(tables.requirement_name(2, 0), Some("Weapon"));

        // A single bit whose row is absent still falls back rather than
        // answering the empty string, which would draw "Must have a  equipped".
        assert_eq!(tables.requirement_name(2, 1 << 9), Some("Weapon"));

        // …and with no tables at all there is no sentence to draw.
        let none = ItemTables::parse(&[], &[], &[], &[]);
        assert_eq!(none.requirement_name(2, 1 << 7), None);
    }
}
