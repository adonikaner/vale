//! `ItemDisplayInfo.dbc` — what a piece of equipment *looks* like.
//!
//! An item on a character is two things, and they are drawn by two different
//! mechanisms:
//!
//! * **textures**, painted into the same 256x256 body composite a player's skin
//!   is built in ([`crate::look::character`]). Eight of them, one per body component,
//!   and they land in the regions `character::regions` already names — a sleeve
//!   into the arm, a boot into the lower leg and the foot. This is why gloves
//!   and boots do not need geometry to be visible.
//! * **geosets**, which are the geometry that stands proud of the body — a
//!   cuff, a bootleg, the skirt of a robe. `geosetGroup[3]` names the variants
//!   and which groups they go in depends on where the item is worn.
//!
//! **The item's display id does not come from a DBC.** `Item.dbc` is not in the
//! 1.12 archives at all — the client asks the *server*
//! (`CMSG_ITEM_QUERY_SINGLE`), whose reply carries `DisplayInfoID` and
//! `InventoryType`. So equipment costs a round trip per distinct item, exactly
//! as a creature's name does, and this module starts from the display id that
//! comes back.
//!
//! Field indices are measured with `vale dbc ItemDisplayInfo`, and the
//! measurement is unusually easy to read: record 0 puts
//! `Generic_HuWk_01_Sleeve_AU`, `..._Sleeve_AL`, `..._Chest_TU`, `..._Chest_TL`,
//! `..._Pants_LU`, `..._Pant_LL` in fields 14, 15, 17, 18, 19 and 20 — the
//! suffixes name the components and the gap at 16 is the hands.

use crate::look::character::{regions, Region};
use crate::tables::dbc::Dbc;
use crate::AssetError;

/// Field indices in `ItemDisplayInfo.dbc`. 23 fields in 1.12; later versions
/// add `itemVisual` and `particleColorID` on the end.
// The whole row is named, including the fields nothing reads. The layout is the
// thing that was wrong — `geosetGroup` sat one field too high for a milestone —
// and a block of constants that accounts for all 23 fields is checkable
// (`the_row_layout_accounts_for_every_field`) where a block naming only the four
// that are used is not.
#[allow(dead_code)]
mod fields {
    /// `modelName[2]` — the *attached* models, for shoulders, weapons and
    /// helmets. Not drawn yet: an attachment needs `M2Attachment`, which is
    /// parsed past.
    pub const MODEL_NAME: usize = 1;
    pub const MODEL_TEXTURE: usize = 3;
    /// `inventoryIcon`, and **there is one of it in 1.12, not two.**
    ///
    /// This is the field the rest of the row hangs off, and getting it wrong
    /// shifts everything between here and the textures by exactly one. See
    /// [`GEOSET_GROUP`].
    pub const INVENTORY_ICON: usize = 5;
    /// Three variant numbers whose *groups* depend on where the item is worn.
    ///
    /// **Measured at 6, not 7, and the difference is a character's boots.** Both
    /// readings add up to the table's 23 fields — `inventoryIcon[2]` at 5..6 with
    /// the triple at 7..9 is exactly as arithmetically tidy as one icon at 5 with
    /// the triple at 6..8 — so the layout cannot settle it and the rows have to.
    /// Three things in the data do, and they agree:
    ///
    /// * **field 5 is a string in every row and field 6 is never one.** 6 holds
    ///   0, 1, 2 or 3, which are variant numbers and not string offsets; a second
    ///   icon would be a name.
    /// * **field 11 is the armour sound group**, 11 for every plate item in the
    ///   table and 7 for a tabard. That pins the tail of the block, and the tail
    ///   is what `helmetGeosetVis` and the textures are measured back from.
    /// * **field 6 is what varies with the garment.** `vale item` prints the
    ///   column-by-piece matrix: a boot moves column 0 and nothing else, a robe
    ///   moves column 2, a chest moves none.
    ///
    /// Read one high, every item's `geosetGroup[0]` is lost and `geosetGroup[2]`
    /// reads `flags`. Nothing fails: the value read is 0, 0 is a legal variant
    /// meaning "the default", and the default is the *bare body*. So a character
    /// in plate boots is drawn with bare feet, a robe has no skirt, and gloves
    /// have no cuffs — with every path resolving, every texture painted
    /// correctly onto the skin underneath, and no warning anywhere. It reads
    /// exactly like equipment geometry that was never implemented.
    pub const GEOSET_GROUP: usize = 6;
    /// `flags`, then `spellVisualID`, then the sound group — the three fields
    /// between the variants and the helmet masks. Named because they are what
    /// says the block ends where it does.
    pub const FLAGS: usize = 9;
    /// `spellVisualID` — the glow a weapon carries. Unread here.
    pub const SPELL_VISUAL: usize = 10;
    /// **`groupSoundIndex` — a row of `ItemGroupSounds.dbc`**, which is what
    /// this item sounds like when it is picked up and put down.
    ///
    /// **Measured at 11, and this constant said 10 for several rounds.** It was
    /// dead code, so nothing failed; what would have failed is the first reader,
    /// silently, because field 10 is `spellVisualID` and its values (224, 743,
    /// 2797…) are legal-looking numbers that simply miss every one of the 24
    /// rows. Three things pin it and they agree:
    ///
    /// * **the column's range is the table's.** Over all 29,604 rows field 11
    ///   holds exactly `{0, 3..24}` — 23 distinct values inside a 24-row table —
    ///   while field 10 holds 19 distinct values up to 4081.
    /// * **the client indexes it at `+0x2c`**, which is field 11 of a row whose
    ///   strings are one slot each in memory as they are on disk. The client
    ///   reads it, bounds-checks it against the group table's length and takes
    ///   the requested column of the row.
    /// * **8,274 rows read 7**, which is `PickUpCloth_Leather` — the largest
    ///   class of item in the game by a wide margin.
    pub const GROUP_SOUND: usize = 11;
    /// Which of the wearer's own geosets a helmet hides — hair, ears, facial
    /// hair.
    pub const HELMET_GEOSET_VIS: usize = 12;
    /// The eight body-component textures, in [`Component`] order.
    pub const TEXTURE: usize = 14;
    /// `itemVisual`, the last field: 14 + 8 = 22, and the table is 23 wide.
    pub const ITEM_VISUAL: usize = 22;
    /// How many fields the 1.12 table has, which every constant above is
    /// checked against.
    pub const COUNT: usize = 23;
}

/// One body component an item can paint, in the order `ItemDisplayInfo` lists
/// them.
///
/// The directory name is the component's own, and the region is where it lands
/// in the composite — the same rectangles a player's skin is composed in, which
/// is what makes a sleeve line up with the arm underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    ArmUpper,
    ArmLower,
    Hand,
    TorsoUpper,
    TorsoLower,
    LegUpper,
    LegLower,
    Foot,
}

impl Component {
    pub const ALL: [Component; 8] = [
        Component::ArmUpper,
        Component::ArmLower,
        Component::Hand,
        Component::TorsoUpper,
        Component::TorsoLower,
        Component::LegUpper,
        Component::LegLower,
        Component::Foot,
    ];

    /// This component's column, counting from `fields::TEXTURE` — which is also
    /// its position in [`Self::ALL`], because the enum is in the table's order.
    pub fn index(self) -> usize {
        Component::ALL
            .iter()
            .position(|c| *c == self)
            .expect("ALL lists every variant")
    }

    /// The directory under `Item\TextureComponents\`.
    pub fn directory(self) -> &'static str {
        match self {
            Component::ArmUpper => "ArmUpperTexture",
            Component::ArmLower => "ArmLowerTexture",
            Component::Hand => "HandTexture",
            Component::TorsoUpper => "TorsoUpperTexture",
            Component::TorsoLower => "TorsoLowerTexture",
            Component::LegUpper => "LegUpperTexture",
            Component::LegLower => "LegLowerTexture",
            Component::Foot => "FootTexture",
        }
    }

    /// Where it lands in the composite.
    pub fn region(self) -> Region {
        match self {
            Component::ArmUpper => regions::ARM_UPPER,
            Component::ArmLower => regions::ARM_LOWER,
            Component::Hand => regions::HAND,
            Component::TorsoUpper => regions::TORSO_UPPER,
            Component::TorsoLower => regions::TORSO_LOWER,
            Component::LegUpper => regions::LEG_UPPER,
            Component::LegLower => regions::LEG_LOWER,
            Component::Foot => regions::FOOT,
        }
    }
}

/// What one `ItemDisplayInfo` row says, resolved to archive paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemAppearance {
    /// One path per [`Component::ALL`], empty where the row names nothing.
    pub textures: [String; 8],
    /// `geosetGroup[3]`. A **0 means "the default variant"**, not "no geoset":
    /// the client draws `group * 100 + value + 1`, so an unequipped slot and an
    /// item that happens to use the default geometry come out the same, which
    /// is why the *item's* presence is what decides whether a group is touched.
    pub geoset_groups: [u32; 3],
    /// `modelName[2]` — shoulders, weapons, helmets. Kept so the count of what
    /// is *not* drawn is reportable rather than invisible.
    pub models: [String; 2],
    pub model_textures: [String; 2],
    pub helmet_geoset_vis: [u32; 2],
}

impl ItemAppearance {
    pub fn has_geometry(&self) -> bool {
        self.models.iter().any(|m| !m.is_empty())
    }

    /// The attached models this item hangs on the wearer, resolved to archive
    /// paths and to the attachment point each goes on.
    ///
    /// **`modelName[2]` is left-then-right, and the file names say so**:
    /// display 34256 is `["LShoulder_Plate_AhnQiraj_A_01.mdx",
    /// "RShoulder_Plate_AhnQiraj_A_01.mdx"]`, and `vale attach` checks that
    /// convention over every shoulder row in the table rather than taking it
    /// from one example. A single-model slot — a helm, a weapon — uses index 0
    /// and leaves index 1 empty.
    ///
    /// The model and its texture live in the *same* directory, which is the
    /// slot's: `Item\ObjectComponents\Shoulder\`, `...\Head\`, `...\Weapon\`.
    /// The DBC names the model `.mdx`, which means `.m2` in the archive — the
    /// same pre-1.0 extension `CreatureModelData` uses.
    pub fn attachments(&self, slot: Slot, race: u8, gender: u8) -> Vec<AttachedModel> {
        slot.attachment_points()
            .iter()
            .enumerate()
            .filter_map(|(i, point)| self.attachment_at(slot, i, *point, race, gender))
            .collect()
    }

    /// One of this item's models, hung from a point the **caller** chose.
    ///
    /// Armour never needs this — a helm hangs from the helm point and a pauldron
    /// from a shoulder, so [`Slot::attachment_points`] is the whole answer. A
    /// weapon does: where it hangs depends on whether it is drawn, and while it
    /// is not, on the item's own `Sheath` field. That is a property of the
    /// *item*, so the slot cannot state it. See [`sheath_point`].
    pub fn attachment_at(
        &self,
        slot: Slot,
        index: usize,
        point: u32,
        race: u8,
        gender: u8,
    ) -> Option<AttachedModel> {
        let directory = slot.object_directory()?;
        let i = index;
        let name = self.models.get(i).filter(|m| !m.is_empty())?;
        // **A helmet is cut for the head it sits on**, and the DBC does not
        // say so: the row names `Helm_Plate_D_04.mdx` and the archive holds
        // sixteen files, `helm_plate_d_04_hum.m2` through `..._trf.m2` —
        // eight race codes times two genders, ~112 helms each. Reading the
        // row verbatim finds nothing at all, which is how this was found:
        // 3,496 shoulder models resolved and 8 helms did.
        let stem = crate::world::m2::model_path(name);
        // An unknown race leaves the name as the row wrote it; that file is
        // not in the archive either, so the caller drops it exactly as it
        // drops any other missing model.
        let file = match (slot.cut_per_race(), race_code(race)) {
            (true, Some(code)) => format!(
                "{}_{code}{}.m2",
                stem.trim_end_matches(".m2"),
                if gender == 0 { "m" } else { "f" }
            ),
            _ => stem,
        };
        let texture = self
            .model_textures
            .get(i)
            .filter(|t| !t.is_empty())
            .map(|t| format!("Item\\ObjectComponents\\{directory}\\{t}.blp"));
        Some(AttachedModel {
            point,
            path: format!("Item\\ObjectComponents\\{directory}\\{file}"),
            texture,
        })
    }
}

/// `HelmetGeosetVisData.dbc` — which of the wearer's own geosets a helmet
/// hides.
///
/// Sixteen rows of `id` plus **five race masks**, one per group that a helmet
/// can hide: the hairstyle, the three facial-hair groups, and the ears. Bit `n`
/// set means "hide this group on race `n`".
///
/// **The masks are read as race bits because the data says so.** Row 248 is
/// `[!(1<<6), !(1<<5), !(1<<6), !0xE0, !0x110]`: everything but bit 6 for the
/// hair, and everything but bits 4 and 8 for the ears. Race 6 is the tauren,
/// whose mane and horns stay visible under a helmet; races 4 and 8 are the night
/// elf and the troll, whose ears famously stick out of one. No other reading of
/// those numbers produces that pattern, and it is the pattern the game is known
/// for.
///
/// The three middle columns are groups 1, 2 and 3 in order — the same order
/// [`crate::world::m2::CharacterGeosets::facial`] uses. That one is *not* pinned by a
/// signature like the ears; a swap would leave a moustache under a full helm,
/// which is why it is written down here rather than assumed to be checked.
pub struct HelmetVisibility {
    dbc: Dbc,
    by_id: std::collections::HashMap<u32, usize>,
}

/// Which of a wearer's own geosets the helmet they have on hides.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hides {
    pub hair: bool,
    /// Facial-hair groups 1, 2 and 3.
    pub facial: [bool; 3],
    /// Group 7, where "hidden" is the *other* variant rather than nothing: 701
    /// is the ears a helmet asks for and 702 is the ears themselves.
    pub ears: bool,
}

impl HelmetVisibility {
    pub fn parse(bytes: &[u8]) -> Result<HelmetVisibility, AssetError> {
        let dbc = Dbc::parse(bytes)?;
        let mut by_id = std::collections::HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            if let Some(id) = dbc.u32_at(record, 0) {
                by_id.insert(id, record);
            }
        }
        Ok(HelmetVisibility { dbc, by_id })
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// What row `id` hides on this race. An id of 0 — which most rows carry — is
    /// "hide nothing", and so is a row that is not in the table.
    pub fn hides(&self, id: u32, race: u8) -> Hides {
        let Some(&record) = self.by_id.get(&id) else {
            return Hides::default();
        };
        let hidden = |column: usize| {
            self.dbc
                .u32_at(record, column)
                .is_some_and(|mask| mask & (1 << race as u32) != 0)
        };
        Hides {
            hair: hidden(1),
            facial: [hidden(2), hidden(3), hidden(4)],
            ears: hidden(5),
        }
    }
}

/// The two-letter code a race's own files are named with.
///
/// Read off `Item\ObjectComponents\Head\`, which holds exactly sixteen suffixes
/// — `dwf dwm gnf gnm huf hum nif nim orf orm scf scm taf tam trf trm`, ~112
/// helms each. `sc` is the undead, whose race is still called Scourge in the
/// file names; that one is the reason to measure rather than transcribe.
pub fn race_code(race: u8) -> Option<&'static str> {
    Some(match race {
        1 => "hu",
        2 => "or",
        3 => "dw",
        4 => "ni",
        5 => "sc",
        6 => "ta",
        7 => "gn",
        8 => "tr",
        _ => return None,
    })
}

/// One model an item hangs off the wearer's skeleton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedModel {
    /// The [`crate::world::m2::attach`] id of the point it hangs from.
    pub point: u32,
    pub path: String,
    /// The skin the *client* supplies for it: an item model declares its
    /// texture as type 2 with no filename, exactly as a creature declares its
    /// body as type 11. `None` leaves whatever the model names itself.
    pub texture: Option<String>,
}

/// `ItemDisplayInfo.dbc`, indexed by display id.
pub struct ItemDisplays {
    dbc: Dbc,
    by_id: std::collections::HashMap<u32, usize>,
}

impl ItemDisplays {
    pub fn parse(bytes: &[u8]) -> Result<ItemDisplays, AssetError> {
        let dbc = Dbc::parse(bytes)?;
        let mut by_id = std::collections::HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            if let Some(id) = dbc.u32_at(record, 0) {
                by_id.insert(id, record);
            }
        }
        Ok(ItemDisplays { dbc, by_id })
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Every display id in the table, for the survey.
    pub fn ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.by_id.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// One item's appearance, with its textures resolved for a wearer of this
    /// gender.
    ///
    /// **The gender suffix is a fallback chain, not a choice.** The files are
    /// named `<component>_<suffix>.blp` with `M`, `F` and `U`: a piece cut
    /// differently for the two bodies ships as two files, and one that is not
    /// ships once as `U`. `Item\TextureComponents\ArmUpperTexture\` holds
    /// `cloth_a_01black_sleeve_au_f`, `..._m` **and** `..._u`, so trying only
    /// one of the three silently loses armour that is in the archive — hence
    /// [`Self::texture_candidates`], and hence `vale item` counting which
    /// suffix answered.
    pub fn appearance(&self, display_id: u32, gender: u8) -> Option<ItemAppearance> {
        let record = *self.by_id.get(&display_id)?;
        let text = |i: usize| {
            self.dbc
                .string_at(record, i)
                .filter(|s| !s.is_empty())
                .unwrap_or_default()
        };
        let number = |i: usize| self.dbc.u32_at(record, i).unwrap_or(0);

        let mut textures: [String; 8] = Default::default();
        for (i, component) in Component::ALL.iter().enumerate() {
            let name = text(fields::TEXTURE + i);
            if name.is_empty() {
                continue;
            }
            // The first candidate that could exist; the caller checks the
            // archive, because only it has one.
            textures[i] = texture_candidates(*component, &name, gender)
                .into_iter()
                .next()
                .unwrap_or_default();
        }

        Some(ItemAppearance {
            textures,
            geoset_groups: [
                number(fields::GEOSET_GROUP),
                number(fields::GEOSET_GROUP + 1),
                number(fields::GEOSET_GROUP + 2),
            ],
            models: [text(fields::MODEL_NAME), text(fields::MODEL_NAME + 1)],
            model_textures: [text(fields::MODEL_TEXTURE), text(fields::MODEL_TEXTURE + 1)],
            helmet_geoset_vis: [
                number(fields::HELMET_GEOSET_VIS),
                number(fields::HELMET_GEOSET_VIS + 1),
            ],
        })
    }

    /// One raw field of a row, as a number and as the string it would be if it
    /// were an offset into the string block.
    ///
    /// Exists for the survey rather than for the renderer. The meaning of a DBC
    /// column is not in the file, so the only honest way to establish that
    /// `geosetGroup` starts at 6 and not at 7 is to look at what the columns
    /// hold across the whole table — a column of small integers is not a column
    /// of icon names, and `vale item` says so in one line.
    pub fn raw_field(&self, display_id: u32, field: usize) -> Option<(u32, Option<String>)> {
        let record = *self.by_id.get(&display_id)?;
        let number = self.dbc.u32_at(record, field)?;
        let text = self
            .dbc
            .string_at(record, field)
            .filter(|s| !s.is_empty());
        Some((number, text))
    }

    /// **Which `ItemGroupSounds` row this item belongs to** — field 11, the
    /// number the client bounds-checks before it plays anything.
    ///
    /// `Some(0)` is never returned: a group of zero is "this item makes no
    /// noise", which is the same answer as a display id with no row, so both
    /// come back `None` and no caller has to know the sentinel.
    pub fn group_sound(&self, display_id: u32) -> Option<u32> {
        let record = *self.by_id.get(&display_id)?;
        self.dbc
            .u32_at(record, fields::GROUP_SOUND)
            .filter(|group| *group != 0)
    }

    /// **The bare icon name a display row carries** — `"INV_Sword_39"`, with no
    /// directory and no suffix.
    ///
    /// Bare on purpose: the folder it goes in is a *shipped table* rather than
    /// a constant, so the path is composed by
    /// [`crate::tables::inventory::ItemTables::icon_path`], which reads it. Returning a
    /// path from here would put the same string in two places and hide where it
    /// came from.
    ///
    /// `None` for a row with no icon at all, which draws an empty square rather
    /// than a missing-texture one.
    pub fn inventory_icon(&self, display_id: u32) -> Option<String> {
        let record = *self.by_id.get(&display_id)?;
        self.dbc
            .string_at(record, fields::INVENTORY_ICON)
            .filter(|s| !s.is_empty())
    }

    /// The raw component name a row gives, before any suffix.
    pub fn texture_name(&self, display_id: u32, component: usize) -> Option<String> {
        let record = *self.by_id.get(&display_id)?;
        self.dbc
            .string_at(record, fields::TEXTURE + component)
            .filter(|s| !s.is_empty())
    }
}

/// The paths one component name could resolve to, most specific first.
///
/// `M`/`F` are the gendered cuts and `U` the unisex one. The caller tries them
/// in order against the archive and takes the first that is there.
pub fn texture_candidates(component: Component, name: &str, gender: u8) -> Vec<String> {
    let sex = if gender == 0 { 'M' } else { 'F' };
    ['\u{0}', sex, 'U']
        .into_iter()
        .filter(|c| *c != '\u{0}')
        .map(|suffix| {
            format!(
                "Item\\TextureComponents\\{}\\{}_{}.blp",
                component.directory(),
                name,
                suffix
            )
        })
        .collect()
}

/// Where an item is worn, from `ItemPrototype::InventoryType` — the value the
/// server sends in `SMSG_ITEM_QUERY_SINGLE_RESPONSE`.
///
/// Transcribed from vmangos `ItemPrototype.h`. Only the slots that change how a
/// character is drawn are named; the rest are `Other` and are ignored rather
/// than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Head,
    Shoulders,
    Shirt,
    Chest,
    Waist,
    Legs,
    Feet,
    Wrists,
    Hands,
    Back,
    Tabard,
    Robe,
    /// A weapon in the right hand, and one in the left.
    ///
    /// These are **not** reachable from `from_inventory_type`: which hand a
    /// weapon goes in is the *equipment slot* it is worn in, not the item's
    /// inventory type — a one-hand sword is `INVTYPE_WEAPON` whichever hand
    /// holds it. The caller knows the hand (it is which of the three virtual
    /// item slots the display id came out of, or which equipment slot for a
    /// player) and names it here.
    MainHand,
    OffHand,
    /// A shield or an off-hand held item, which lives in its own directory.
    Shield,
    Other,
}

impl Slot {
    pub fn from_inventory_type(inventory_type: u32) -> Slot {
        match inventory_type {
            1 => Slot::Head,
            3 => Slot::Shoulders,
            4 => Slot::Shirt,
            5 => Slot::Chest,
            6 => Slot::Waist,
            7 => Slot::Legs,
            8 => Slot::Feet,
            9 => Slot::Wrists,
            10 => Slot::Hands,
            16 => Slot::Back,
            19 => Slot::Tabard,
            20 => Slot::Robe,
            _ => Slot::Other,
        }
    }

    /// Where this slot's textures sit in the paint order of the composite.
    ///
    /// **Later covers earlier, and that is the whole content of the number.**
    /// A sleeve goes under a glove, a trouser leg under a boot, and a tabard
    /// over the chest it hangs on — so the order is body-outwards rather than
    /// anything the DBC states. The real client derives the same thing from a
    /// per-section layer table that arrives in a later expansion; this is that
    /// table's shape for the slots 1.12 has, and it is checked the only way it
    /// can be, which is by looking at a dressed character.
    pub fn layer(self) -> u8 {
        match self {
            Slot::Shirt => 1,
            Slot::Legs => 2,
            Slot::Chest | Slot::Robe => 3,
            Slot::Feet => 4,
            Slot::Wrists => 5,
            Slot::Waist => 6,
            Slot::Hands => 7,
            Slot::Tabard => 8,
            // Head, shoulders and back paint nothing onto the body.
            _ => 0,
        }
    }

    /// Which body components this slot may paint.
    ///
    /// **A row's eight texture columns are not one garment — they are the whole
    /// armour set**, and the slot picks the pieces that belong to *this* item.
    /// `Leggings of Polarity` (display 35514) names
    /// `Cloth_B_03RedPhoenix_Sleeve_AU/_AL` and `..._Chest_TU/_TL` beside its own
    /// `Cloth_B_03BlackSilver_Pant_LU/_LL`: those first four are the tier set's
    /// chest, copied into every row of the set, and painting them puts the chest
    /// piece's colour on the wearer's torso *over whatever they are actually
    /// wearing*. That is not a subtle wrongness — a mage in a blue robe and this
    /// set's leggings comes out red from the neck down, which reads as a
    /// composition bug and is a filtering one.
    ///
    /// The names are what say which columns a garment fills, and they are
    /// unambiguous: the second-to-last token of a component texture is the piece
    /// (`Sleeve`, `Chest`, `Pant`, `Robe`, `Belt`, `Boot`, `Glove`, `Bracer`,
    /// `Tabard`) and the last is the component it fills. `vale item` prints
    /// that matrix over the whole table, and it is where these sets come from —
    /// `Boot` fills only leg-lower and foot in all 3,732 of its rows, `Bracer`
    /// only arm-lower, `Belt` only leg-upper.
    pub fn components(self) -> &'static [Component] {
        use Component::*;
        match self {
            // Sleeves and the chest itself.
            Slot::Shirt | Slot::Chest => &[ArmUpper, ArmLower, TorsoUpper, TorsoLower],
            // A robe is a chest that continues down the legs.
            Slot::Robe => &[ArmUpper, ArmLower, TorsoUpper, TorsoLower, LegUpper, LegLower],
            Slot::Legs => &[LegUpper, LegLower],
            // A boot covers the shin as well as the foot.
            Slot::Feet => &[LegLower, Foot],
            // A glove's cuff runs onto the forearm.
            Slot::Hands => &[ArmLower, Hand],
            Slot::Wrists => &[ArmLower],
            // A belt sits on the hips, which is the leg-upper region.
            Slot::Waist => &[LegUpper],
            Slot::Tabard => &[TorsoUpper, TorsoLower],
            // Head, shoulders and back are geometry or nothing: their rows carry
            // the set's textures too, and none of them belongs to the body.
            _ => &[],
        }
    }

    /// The directory under `Item\ObjectComponents\` this slot's *models* live
    /// in, or `None` for a slot that is all texture.
    ///
    /// The model and its skin share the directory, which is what makes this one
    /// answer rather than two.
    pub fn object_directory(self) -> Option<&'static str> {
        match self {
            Slot::Head => Some("Head"),
            Slot::Shoulders => Some("Shoulder"),
            // A cloak names no model — it is the wearer's own group-15 geoset —
            // but its *skin* lives here, and the M2 asks for it as texture
            // type 2 exactly as a creature asks for its body as type 11.
            Slot::Back => Some("Cape"),
            // Read off the archive rather than assumed: `Item\ObjectComponents\`
            // holds nine subdirectories, and `vale attach` prints the count in
            // each — `weapon` 1,937 models, `shield` 192, beside `head` 2,196 and
            // `shoulder` 602.
            Slot::MainHand | Slot::OffHand => Some("Weapon"),
            Slot::Shield => Some("Shield"),
            _ => None,
        }
    }

    /// The cloak texture this slot supplies to the wearer's own cape geoset,
    /// which is the one piece of equipment that is geometry the *character*
    /// already has and a texture the item names.
    ///
    /// `modelTexture[0]`, from `Item\ObjectComponents\Cape\`: display 35444 is
    /// `Cape_Naxxramas_03Red`, and the row carries no model and no components at
    /// all. Without it the geoset arrives and draws magenta — the same honest
    /// signal the hair gave when its texture was a round behind it.
    pub fn is_cloak(self) -> bool {
        matches!(self, Slot::Back)
    }

    /// Whether this slot's model is cut per race and gender.
    ///
    /// Only the head is: a helm has to fit a tauren's muzzle and a gnome's
    /// skull, so the archive holds sixteen of each. Shoulders sit outside the
    /// body and ship once — 3,496 of them resolve from the row's own name.
    pub fn cut_per_race(self) -> bool {
        matches!(self, Slot::Head)
    }

    /// Which attachment points this slot's `modelName[2]` hang from, in order.
    ///
    /// Left first, then right — the order `modelName` is written in, which the
    /// file names give away and `vale attach` checks over the whole table.
    pub fn attachment_points(self) -> &'static [u32] {
        use crate::world::m2::attach;
        match self {
            Slot::Head => &[attach::HELM],
            Slot::Shoulders => &[attach::SHOULDER_LEFT, attach::SHOULDER_RIGHT],
            // **Drawn weapons only** — where a *sheathed* one hangs is the
            // item's own sheath type, through [`sheath_point`].
            //
            // The right hand is attachment 1 and the left is 2, and every
            // character model carries both. **A shield is neither**: it hangs
            // off the forearm at attachment 0, which the client picks in the
            // same branch that picks its directory.
            Slot::MainHand => &[attach::HAND_RIGHT],
            Slot::OffHand => &[attach::HAND_LEFT],
            Slot::Shield => &[attach::SHIELD],
            _ => &[],
        }
    }

    /// Which geoset groups this slot's `geosetGroup[3]` fill, in order.
    ///
    /// **The three numbers are not the same three groups for every slot** — a
    /// robe's first is its sleeves and its third is its skirt, where a boot's
    /// first is its bootleg and it has no others.
    ///
    /// The table that would state this (`CharComponentTextureLayouts`) is a
    /// later expansion's, so this is measured against the models rather than
    /// transcribed, the way the facial-hair column order was: `vale item`
    /// crosses every row's variant numbers with the geosets the 18 character
    /// models actually carry and prints which groups could hold each column. The
    /// arms and the legs come out **paired**, which is what makes the reading
    /// structural rather than a fit:
    ///
    /// ```text
    ///   group  8   variants 2,3     group  9   variants 2,3    the frills
    ///   group 10   variant  2       group 11   variant  2      the garment
    ///   group 13   variants 1,2                                the skirt
    /// ```
    ///
    /// So a chest is `[8, 10, 13]` — cuff, doublet, skirt — and a pair of
    /// trousers is `[9, 11, 13]` in exactly the same order one group over.
    /// Chest and robe are identical here: a long tunic states its skirt in the
    /// third column and `x01` in group 13 is the *bare legs*, so a garment with
    /// no skirt asks for the default and gets it.
    ///
    /// Note that groups 8 to 12 have **no `x01` at all** — their unequipped
    /// state is genuinely nothing, which is why an item can claim one without
    /// having to hide anything underneath.
    pub fn geoset_groups(self) -> &'static [u16] {
        match self {
            // Cuffs and the doublet. A shirt has no skirt.
            Slot::Shirt => &[8, 10],
            // Cuff, doublet, skirt — a robe and a long tunic are the same three.
            Slot::Chest | Slot::Robe => &[8, 10, 13],
            // The same three one group over: kneepad, trouser, skirt.
            Slot::Legs => &[9, 11, 13],
            Slot::Feet => &[5],
            Slot::Hands => &[4],
            // A bracer shares the arm's frill group with a sleeve and, being
            // painted over it, wins — see
            // [`crate::world::m2::CharacterGeosets::equip`], which keeps one variant
            // per group in paint order.
            Slot::Wrists => &[8],
            Slot::Back => &[15],
            Slot::Tabard => &[12],
            _ => &[],
        }
    }
}

/// One thing a character is wearing, as the composite and the geoset filter
/// need it: what it looks like, and where it is worn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Equipped {
    /// `ItemDisplayInfo` row, from the server's `SMSG_ITEM_QUERY_SINGLE_RESPONSE`.
    pub display_id: u32,
    pub slot: Slot,
}

/// What a unit has in one hand, as far as drawing and animating it goes.
///
/// **The same five facts arrive by two different routes, and this is where they
/// stop being two.** For a *creature* they are already on the wire:
/// `Creature::SetVirtualItem` writes the display id into
/// `UNIT_VIRTUAL_ITEM_SLOT_DISPLAY` and packs the class, subclass, material and
/// inventory type into `UNIT_VIRTUAL_ITEM_INFO`'s first word with the **sheath
/// type** in its second. For a *player* the wire carries an item **entry** and
/// all of it comes back from `SMSG_ITEM_QUERY_SINGLE_RESPONSE` a round trip
/// later. Everything downstream — which point it hangs from, which swing the
/// wielder plays — is one code path over this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Weapon {
    /// `ItemDisplayInfo` row. **Zero is an empty hand**, and is the only test
    /// for one: an unarmed unit's whole record is zeroes.
    pub display_id: u32,
    /// `ItemPrototype::Class` — 2 is a weapon, 4 is armour (which in a hand
    /// means a shield or an off-hand held item).
    pub class: u8,
    /// `ItemPrototype::SubClass` — which weapon, within the class.
    pub subclass: u8,
    /// `ItemPrototype::InventoryType`.
    pub inventory_type: u8,
    /// `ItemPrototype::Sheath` — **where it hangs when it is put away**. See
    /// [`sheath_point`].
    pub sheath: u8,
    /// `ItemPrototype::Material` — **the only field here that is never drawn**,
    /// and the one thing that decides what a blow with this weapon *sounds*
    /// like.
    ///
    /// `WeaponImpactSounds.dbc` carries two rows per subclass split by a metal
    /// flag, and the pair is not cosmetic: subclass 4 is `Mace1H_ArmorFlesh`
    /// non-metal and `Mace1HMetal_ArmorFlesh` metal, and the same split runs
    /// through the two-handed maces, the polearms, the staves, the misc
    /// weapons and the fishing poles. The bladed subclasses carry the *same*
    /// id in both rows, which is why keying the table on the subclass alone
    /// looked right for a sword and flipped a mace at random.
    ///
    /// 1 is metal ([`MATERIAL_METAL`]); the rest are wood, cloth, leather and
    /// the ones no weapon is. `Creature::SetVirtualItem` packs it at byte 2 of
    /// `UNIT_VIRTUAL_ITEM_INFO`'s first word — beside the class and the
    /// subclass, and this client read past it for eight rounds.
    pub material: u8,
}

/// `ITEM_MATERIAL_METAL` — the one value of [`Weapon::material`] that picks
/// `WeaponImpactSounds`' metal row.
pub const MATERIAL_METAL: u8 = 1;

impl Weapon {
    /// Whether anything is in this hand at all.
    pub fn is_empty(&self) -> bool {
        self.display_id == 0
    }

    /// A shield — the one worn item with its own directory and its own
    /// attachment: `Item\ObjectComponents\Shield\`, hung off the forearm.
    ///
    /// `INVTYPE_SHIELD` **exactly**, which is the reference's own test — the
    /// character-select dressing loop passes the weapon attacher an
    /// `isShield` computed as the bare `inventoryType == 14` — and it is a
    /// measurement here too, because this method once said `14 or 23` and that
    /// made every off-hand *holdable* in the game invisible. A tome, an orb, a
    /// torch (`INVTYPE_HOLDABLE`, 23) is held in the left hand and its model
    /// lives under `Weapon\` with every other held thing: probed over all
    /// **222 holdable items vmangos ships — 71 distinct models, 71 of them
    /// under `Weapon\` and 0 under `Shield\`** — so the `| 23` sent each one
    /// to a directory where none of them are ("`Shield\Hand_1H_AhnQiraj_D_01`
    /// will not read") and hung the miss from the wrong point besides.
    pub fn is_shield(&self) -> bool {
        const ITEM_CLASS_ARMOR: u8 = 4;
        const INVTYPE_SHIELD: u8 = 14;
        self.class == ITEM_CLASS_ARMOR && self.inventory_type == INVTYPE_SHIELD
    }

    /// **What a `SMSG_CHAR_ENUM` slot makes of a weapon** — the character-select
    /// screen's whole knowledge of one, and, as it turns out, all the client
    /// asks for there.
    ///
    /// That packet carries a display id and an inventory type per slot and
    /// nothing else, and there is no world to send a `CMSG_ITEM_QUERY_SINGLE`
    /// into, so the class, the subclass and the sheath type are simply absent.
    /// The obvious conclusion — that a character on the plinth therefore cannot
    /// be given its weapons — is **wrong**, and it stood in this project's
    /// sources for several rounds. The client's character-select dressing
    /// loop runs over exactly the twenty `(display id, inventory type)` pairs
    /// the packet carries, and for equipment slots **15 and 16** it calls the
    /// weapon attacher with
    ///
    /// ```text
    /// ranged-weapon-in-the-right-hand   no
    /// isShield                          inventoryType == 14
    /// "put away"                        no -> the point is the HAND, not a sheath point
    /// sheathType                        0  -> never consulted, because of the line above
    /// the equipment slot
    /// ```
    ///
    /// — so the plinth's character holds its weapons rather than wearing them,
    /// and the only question asked of the item is `inventoryType == 14`. Slot
    /// **17, the ranged weapon, is skipped outright** at the top of the
    /// loop body, which is why no bow, gun or wand is ever on
    /// that screen.
    ///
    /// This fills in the one field the rules downstream read that the packet
    /// does not carry — [`Self::is_shield`] wants `ITEM_CLASS_ARMOR` — and
    /// leaves the rest at zero, which is not a placeholder: a sheath type of 0
    /// is "hangs nowhere", and nothing here is ever sheathed.
    ///
    /// An off-hand *holdable* — a tome, an orb, a torch, `INVTYPE_HOLDABLE` —
    /// fails the reference's `== 14` test, takes the left hand and is looked
    /// for under `Weapon\`, **which is where every one of its models lives**
    /// (see [`Self::is_shield`], where the census is), so the plinth draws it
    /// exactly as the world does. An earlier reading of this file said the
    /// holdables lived under `Shield\` and drew nothing here — the premise
    /// was never measured, and it was wrong both ways.
    pub fn from_char_enum(display_id: u32, inventory_type: u8) -> Weapon {
        const ITEM_CLASS_ARMOR: u8 = 4;
        const INVTYPE_SHIELD: u8 = 14;
        Weapon {
            display_id,
            class: if inventory_type == INVTYPE_SHIELD {
                ITEM_CLASS_ARMOR
            } else {
                0
            },
            subclass: 0,
            inventory_type,
            sheath: 0,
            // Nothing on the character screen ever swings, so the one field
            // that only decides a noise stays at "not stated".
            material: 0,
        }
    }
}

/// Which family of attack, ready and parry animations a weapon belongs to.
///
/// **The names are the models' own.** Every character model carries
/// `Attack1H`/`Attack2H`/`Attack2HL` and `AttackBow`/`AttackRifle`/
/// `AttackThrown` as separate sequences, so the choice is real geometry rather
/// than a refinement — a character holding a two-hander and playing the unarmed
/// swing punches the air beside the weapon.
/// **The three questions do not partition the same way**, which is why this is
/// a family rather than a sequence id: a fist weapon holds the one-handed ready
/// stance and throws an *unarmed* punch, and a fishing pole is swung two-handed
/// from an unarmed guard. Every split here follows the client's own
/// `(class, subclass)` tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponAnim {
    Unarmed,
    /// An axe, mace, sword, exotic or "miscellaneous" one-hander.
    OneHand,
    /// **A dagger, which stabs rather than swings**: `Attack1HPierce` (85) and
    /// `AttackOffPierce` (88) are sequences of their own that every character
    /// model carries, and drawing a rogue's daggers as one-handed swings was a
    /// visible loss on the class that swings most often.
    Dagger,
    /// **A fist weapon, which is a punch with a prop on it.** Held in the
    /// one-handed guard and thrown as `AttackUnarmed`; the models have no
    /// separate sequence and the client does not ask for one.
    Fist,
    /// A two-handed sword, axe or mace: swung from over the shoulder.
    TwoHand,
    /// A polearm, staff or spear — `Attack2HL`, a distinct sequence with a
    /// distinct grip. The models keep them apart, so this does too.
    TwoHandLarge,
    /// **The game's own joke, and it is a joke with its own row.** A fishing
    /// pole is *swung* as a long two-hander and *held* in the unarmed guard —
    /// the client's ready table has 0x14 falling through to `ReadyUnarmed`
    /// where its attack table sends it to `Attack2HL`.
    FishingPole,
    Bow,
    /// A gun or a crossbow.
    Rifle,
    Thrown,
}

/// `ItemSubclassWeapon`, transcribed from vmangos `ItemPrototype.h`. Only the
/// ones that pick a different animation are named.
mod weapon_subclass {
    pub const AXE: u8 = 0;
    pub const AXE2: u8 = 1;
    pub const BOW: u8 = 2;
    pub const GUN: u8 = 3;
    pub const MACE: u8 = 4;
    pub const MACE2: u8 = 5;
    pub const POLEARM: u8 = 6;
    pub const SWORD: u8 = 7;
    pub const SWORD2: u8 = 8;
    pub const STAFF: u8 = 10;
    pub const EXOTIC: u8 = 11;
    pub const EXOTIC2: u8 = 12;
    pub const FIST: u8 = 13;
    pub const MISC: u8 = 14;
    pub const DAGGER: u8 = 15;
    pub const THROWN: u8 = 16;
    pub const SPEAR: u8 = 17;
    pub const CROSSBOW: u8 = 18;
    pub const WAND: u8 = 19;
    pub const FISHING_POLE: u8 = 20;
}

impl WeaponAnim {
    /// Which family this weapon is swung as.
    ///
    /// An empty hand, a shield, and anything that is not `ITEM_CLASS_WEAPON`
    /// are all [`WeaponAnim::Unarmed`]: a character holding a shield swings the
    /// hand that is *not* holding it.
    pub fn of(weapon: &Weapon) -> WeaponAnim {
        use weapon_subclass as sub;
        const ITEM_CLASS_WEAPON: u8 = 2;
        if weapon.is_empty() || weapon.class != ITEM_CLASS_WEAPON {
            return WeaponAnim::Unarmed;
        }
        match weapon.subclass {
            sub::AXE | sub::MACE | sub::SWORD | sub::MISC | sub::EXOTIC => WeaponAnim::OneHand,
            sub::DAGGER => WeaponAnim::Dagger,
            sub::FIST => WeaponAnim::Fist,
            sub::AXE2 | sub::MACE2 | sub::SWORD2 | sub::EXOTIC2 => WeaponAnim::TwoHand,
            // A polearm, a staff and a spear are held in two hands at arm's
            // length; the fishing pole is swung the same way and *held*
            // differently, which is why it is not in this arm.
            sub::POLEARM | sub::STAFF | sub::SPEAR => WeaponAnim::TwoHandLarge,
            sub::FISHING_POLE => WeaponAnim::FishingPole,
            sub::BOW => WeaponAnim::Bow,
            sub::GUN | sub::CROSSBOW => WeaponAnim::Rifle,
            sub::THROWN => WeaponAnim::Thrown,
            // A wand is pointed rather than swung, and the models have no
            // sequence for it; the caster animations cover what it does.
            sub::WAND => WeaponAnim::Unarmed,
            _ => WeaponAnim::OneHand,
        }
    }

    /// The `AnimationData.dbc` id of this family's **main-hand** swing.
    pub fn attack(self) -> u16 {
        use crate::world::m2::anim;
        match self {
            // A fist weapon throws a punch: the client's table sends 0xd to
            // `AttackUnarmed` outright and no model carries anything else.
            WeaponAnim::Unarmed | WeaponAnim::Fist => anim::ATTACK_UNARMED,
            WeaponAnim::OneHand => anim::ATTACK_1H,
            WeaponAnim::Dagger => anim::ATTACK_1H_PIERCE,
            WeaponAnim::TwoHand => anim::ATTACK_2H,
            WeaponAnim::TwoHandLarge | WeaponAnim::FishingPole => anim::ATTACK_2HL,
            WeaponAnim::Bow => anim::ATTACK_BOW,
            WeaponAnim::Rifle => anim::ATTACK_RIFLE,
            WeaponAnim::Thrown => anim::ATTACK_THROWN,
        }
    }

    /// **The shot, for a weapon that is fired rather than swung** — and `None`
    /// for everything else, which is the half that matters.
    ///
    /// [`Self::attack`] answers *something* for every family, because every
    /// family swings; this one is asked of the **ranged slot** and the honest
    /// answer for a slot holding nothing, or a wand, is nothing at all. A wand
    /// is `WeaponAnim::Unarmed` here for the reason it is there — the character
    /// models carry no wand sequence — and firing one must not therefore make
    /// the caster throw a punch.
    ///
    /// The three ids are the three families and the models keep them apart:
    /// `AttackBow` (46), `AttackRifle` (49) and `AttackThrown` (107), which are
    /// also exactly the three the client exempts from the empty-hands stow
    /// while ranged-drawn (`crate::look::sheath::RANGED_EXEMPT`, whose nine members
    /// are these three plus each one's load and hold).
    pub fn ranged_attack(self) -> Option<u16> {
        use crate::world::m2::anim;
        Some(match self {
            WeaponAnim::Bow => anim::ATTACK_BOW,
            WeaponAnim::Rifle => anim::ATTACK_RIFLE,
            WeaponAnim::Thrown => anim::ATTACK_THROWN,
            _ => return None,
        })
    }

    /// The **off**-hand swing for whatever is in the left hand.
    ///
    /// A different partition from [`Self::attack`], and a much coarser one: the
    /// game ships three off-hand sequences and no more. Anything that is not an
    /// equipped weapon — an empty hand, a shield, an off-hand tome — punches,
    /// which is what makes [`WeaponAnim::Unarmed`] the right answer for a
    /// sword-and-board fighter's left hand.
    pub fn off_attack(self) -> u16 {
        use crate::world::m2::anim;
        match self {
            WeaponAnim::Dagger => anim::ATTACK_OFF_PIERCE,
            WeaponAnim::Unarmed => anim::ATTACK_UNARMED_OFF,
            _ => anim::ATTACK_OFF,
        }
    }

    /// The between-swings stance for this family.
    pub fn ready(self) -> u16 {
        use crate::world::m2::anim;
        match self {
            // …and the fishing pole is held like nothing at all, which is the
            // one place the ready table and the attack table disagree.
            WeaponAnim::Unarmed | WeaponAnim::FishingPole => anim::READY_UNARMED,
            // A dagger and a fist weapon are both **held** one-handed; only
            // the blow differs.
            WeaponAnim::OneHand | WeaponAnim::Dagger | WeaponAnim::Fist => anim::READY_1H,
            WeaponAnim::TwoHand => anim::READY_2H,
            WeaponAnim::TwoHandLarge => anim::READY_2HL,
            WeaponAnim::Bow => anim::READY_BOW,
            WeaponAnim::Rifle => anim::READY_RIFLE,
            WeaponAnim::Thrown => anim::READY_THROWN,
        }
    }

    /// The parry for this family. A ranged weapon parries nothing, so it takes
    /// the unarmed one — which is what the models offer.
    ///
    /// **Not confirmed against the client**, unlike the three tables above it:
    /// the client's parry pick is not known, and this follows the grip rather
    /// than the blow because a parry is made with the weapon held rather than
    /// swung.
    pub fn parry(self) -> u16 {
        use crate::world::m2::anim;
        match self {
            WeaponAnim::OneHand | WeaponAnim::Dagger | WeaponAnim::Fist => anim::PARRY_1H,
            WeaponAnim::TwoHand => anim::PARRY_2H,
            WeaponAnim::TwoHandLarge | WeaponAnim::FishingPole => anim::PARRY_2HL,
            _ => anim::PARRY_UNARMED,
        }
    }
}

/// Where a **sheathed** weapon hangs: the item's `Sheath` field and which hand
/// it belongs to, against the client's own table of attachment points.
///
/// **This rule never crosses the wire and no DBC states it**, and every other
/// authority gets it wrong. The client's rule is
/// `(sheathType, isMainHand) -> attachmentId`:
///
/// ```text
///   sheath > 4          -> -1, draw nothing
///   1  MAINHAND         -> 26 SheathMainHand    / 27 SheathOffHand
///   2  LARGEWEAPON      -> 30 LargeWeaponLeft   / 31 LargeWeaponRight
///   3  HIPWEAPON        -> 32 HipWeaponLeft     / 33 HipWeaponRight
///   4  SHIELD           -> 28 SheathShield, whichever hand
///   0  NONE             -> -1, draw nothing
/// ```
///
/// The left variant is the **main hand's** — for the main hand the client
/// takes `base - 1`.
///
/// **The client's `SheathTypes` enum has five values, not eight**, and that is
/// the whole bug this replaced. vmangos (and TrinityCore, and this project for
/// a round) name seven — `MAINHAND, OFFHAND, LARGEWEAPONLEFT, LARGEWEAPONRIGHT,
/// HIPWEAPONLEFT, HIPWEAPONRIGHT, SHIELD` — and joining *those* names to the
/// attachment enum's identical names is arithmetically tidy, leaves the two
/// values 1.12 never uses stranded on the two points nothing else claims, and
/// is wrong: the client's enum is `MAINHAND, LARGEWEAPON, HIPWEAPON, SHIELD`,
/// where the side is not in the value at all because it comes from the hand.
/// A sheath type names a **place**, and the left/right of that place is decided
/// separately.
///
/// What the two readings differ by is exactly what was seen on screen. Under
/// the seven-name join every one-handed weapon in the game (`Sheath` 3) hangs
/// from `LargeWeaponLeft`, high on the back at z 1.50; under the client's it
/// hangs from `HipWeaponLeft` at z 1.12, which is where a sword belongs and
/// where the retail client puts it. And a shield (`Sheath` 4) moves from the
/// right of the back to the midline of it.
///
/// Three things then agree that were previously in tension:
///
/// * **the hips are used after all.** `HipSheath` (`AnimationData` 90) is on
///   every one of the 18 character models, and under the old table nothing in
///   1.12 ever played it. It is the draw from the hip, and every one-handed
///   weapon and every dagger in `item_template` carries `Sheath` 3.
/// * **`LargeWeapon` holds large weapons.** `Sheath` 2 is staves and polearms,
///   which is what the point's name says, where the old join gave it the
///   one-handers and gave the greatswords the shoulder.
/// * **the value the client refuses is the one the game does not draw.**
///   `item_template` also uses 7 — fist weapons and off-hand holdables — and
///   the client's `> 4` test drops it, which is the well-known vanilla
///   behaviour that a sheathed fist weapon is invisible.
///
/// `None` means "draw nothing", which is also the right answer for sheath type
/// 0: bows, guns, wands and thrown weapons all carry it, and they are drawn
/// only while the ranged sheath state has them out.
pub fn sheath_point(sheath: u8, main_hand: bool) -> Option<u32> {
    use crate::world::m2::attach;
    // The pairs are `(main hand, off hand)`, which in the client is one
    // subtraction off a single base rather than a pair — 26/27, 30/31 and 32/33
    // are consecutive for that reason.
    Some(match sheath {
        SHEATHETYPE_MAINHAND if main_hand => attach::SHEATH_MAIN,
        SHEATHETYPE_MAINHAND => attach::SHEATH_OFF,
        SHEATHETYPE_LARGEWEAPON if main_hand => attach::LARGE_WEAPON_LEFT,
        SHEATHETYPE_LARGEWEAPON => attach::LARGE_WEAPON_RIGHT,
        SHEATHETYPE_HIPWEAPON if main_hand => attach::HIP_WEAPON_LEFT,
        SHEATHETYPE_HIPWEAPON => attach::HIP_WEAPON_RIGHT,
        // The one place with no side: a shield is on the midline of the back
        // whichever hand it came out of.
        SHEATHETYPE_SHIELD => attach::SHEATH_SHIELD,
        // 0 is `SHEATHETYPE_NONE`, and so is everything above 4 — the client
        // tests `sheath > 4` first and returns no attachment at all, which is
        // what makes a sheathed fist weapon invisible.
        _ => return None,
    })
}

/// `enum SheathTypes` **as the 1.12 client has it** — four values and a none.
///
/// Not vmangos' seven-value enum, which names a left and a right for two of
/// them; the side comes from the hand instead. See [`sheath_point`], where the
/// difference is the whole difficulty and is what put every one-handed weapon
/// in the game on the wrong part of a character's back.
const SHEATHETYPE_MAINHAND: u8 = 1;
const SHEATHETYPE_LARGEWEAPON: u8 = 2;
const SHEATHETYPE_HIPWEAPON: u8 = 3;
const SHEATHETYPE_SHIELD: u8 = 4;

/// The texture layers a set of equipment paints over a character's skin, in
/// paint order.
///
/// `exists` answers whether a path is in the archive, which is how the gender
/// fallback is resolved — two thirds of the wardrobe ships as `_U` and only a
/// third as `_M`/`_F`, so trying one suffix and stopping loses most of it. The
/// check is a closure rather than an `Assets` so this is testable without an
/// archive and callable from the loader thread, which owns one.
///
/// A piece whose file is not there costs its own component and nothing else —
/// the same rule as a missing `CharSections` layer. What shows is the skin
/// underneath, which is a bare arm rather than a hole.
pub fn item_layers(
    table: &ItemDisplays,
    gender: u8,
    items: &[Equipped],
    mut exists: impl FnMut(&str) -> bool,
) -> Vec<crate::look::character::SkinLayer> {
    let mut wearing: Vec<&Equipped> = items.iter().filter(|i| i.slot.layer() > 0).collect();
    // Stable by layer, so two items in the same layer keep the order the
    // caller listed them in rather than an arbitrary one.
    wearing.sort_by_key(|i| i.slot.layer());

    let mut layers = Vec::new();
    for item in wearing {
        // Only the components this slot wears — see [`Slot::components`]. The
        // other columns hold the rest of the armour set and belong to the pieces
        // that are worn elsewhere.
        for component in item.slot.components() {
            let Some(name) = table.texture_name(item.display_id, component.index()) else {
                continue;
            };
            let Some(path) = texture_candidates(*component, &name, gender)
                .into_iter()
                .find(|p| exists(p))
            else {
                continue;
            };
            layers.push(crate::look::character::SkinLayer {
                path,
                region: component.region(),
            });
        }
    }
    layers
}

/// The models a set of equipment hangs off the wearer's own skeleton —
/// pauldrons and a helm — with the point each one goes on.
///
/// This is the half of equipment that is *geometry rather than paint*: 9,263 of
/// the table's 29,604 display ids carry one. It is separate from
/// [`item_layers`] because nothing about it touches the composite: an attached
/// model is its own M2, with its own texture, drawn as a child of the wearer.
pub fn item_attachments(
    table: &ItemDisplays,
    race: u8,
    gender: u8,
    items: &[Equipped],
) -> Vec<AttachedModel> {
    let mut out = Vec::new();
    for item in items {
        let Some(look) = table.appearance(item.display_id, gender) else {
            continue;
        };
        out.extend(look.attachments(item.slot, race, gender));
    }
    out
}

/// What the wearer's helmet hides of their own head.
///
/// `helmetGeosetVis[2]` in `ItemDisplayInfo` is indexed by **gender** and names
/// a `HelmetGeosetVisData` row; that row is indexed by **race**. Both halves
/// matter: the same helm hides a human's hair and leaves a tauren's, and a
/// female row is not a male one.
pub fn helmet_hides(
    table: &ItemDisplays,
    visibility: &HelmetVisibility,
    race: u8,
    gender: u8,
    items: &[Equipped],
) -> Hides {
    let mut out = Hides::default();
    for item in items.iter().filter(|i| matches!(i.slot, Slot::Head)) {
        let Some(look) = table.appearance(item.display_id, gender) else {
            continue;
        };
        let row = look.helmet_geoset_vis[usize::from(gender != 0)];
        let hides = visibility.hides(row, race);
        out.hair |= hides.hair;
        out.ears |= hides.ears;
        for (a, b) in out.facial.iter_mut().zip(hides.facial) {
            *a |= b;
        }
    }
    out
}

/// The cloak texture, if the wearer has a cloak on.
///
/// The wearer's *own* group-15 geoset is the cape; the item supplies only the
/// skin for it, which the character M2 asks for as texture type 2.
pub fn cloak_texture(table: &ItemDisplays, items: &[Equipped]) -> Option<String> {
    let cloak = items.iter().find(|i| i.slot.is_cloak())?;
    let look = table.appearance(cloak.display_id, 0)?;
    let name = look.model_textures.first().filter(|t| !t.is_empty())?;
    Some(format!("Item\\ObjectComponents\\Cape\\{name}.blp"))
}

/// The geosets a set of equipment asks the wearer to draw.
///
/// `geosetGroup[n]` is a *variant number* and the group it belongs to comes
/// from the slot, not from the table — so a robe's three numbers are its
/// sleeves, its chest trim and its skirt, while a boot's one number is its
/// bootleg. The drawn id is `group * 100 + value + 1`, because the table counts
/// from the default: **0 means the default variant, which is `x01`**, and the
/// item still claims the group so that the bare-body geometry underneath is
/// dropped.
pub fn item_geosets(table: &ItemDisplays, items: &[Equipped]) -> [u16; 12] {
    let mut out = crate::world::m2::CharacterGeosets::default();
    // Paint order, so that where two garments claim one group the outer one
    // wins — a sleeve over a bracer's cuff, a robe's skirt over trousers. See
    // `CharacterGeosets::equip`, which keeps one variant per group.
    let mut wearing: Vec<&Equipped> = items.iter().collect();
    wearing.sort_by_key(|i| i.slot.layer());
    for item in wearing {
        let Some(look) = table.appearance(item.display_id, 0) else {
            continue;
        };
        for (group, value) in item.slot.geoset_groups().iter().zip(look.geoset_groups) {
            out.equip(group * 100 + value as u16 + first_variant(*group));
        }
    }
    out.equipment
}

/// The cape group, whose `x01` is the *bare back* rather than the first cloak.
const CAPE_GROUP: u16 = 15;

/// Which variant an item's `geosetGroup` value of 0 means, per group.
///
/// Normally `x01`: the table counts from the group's unequipped default, so a
/// glove asking for 0 is 401 and a boot 501. **The cape counts from `x02`**,
/// because 1501 is not a cape at all — it is the back of the body, and the
/// model says so: on `HumanMale.m2` the group-15 batches are
/// `(1501, type 1) (1502..1506, type 2)`, and type 1 is the composed body skin
/// where type 2 is the object skin an *item* supplies. Drawing 1501 for a
/// character wearing a cloak therefore draws their bare back, correctly
/// textured, and no cloak — which looks exactly like equipment that has not been
/// implemented rather than like an off-by-one.
///
/// It is the same inversion group 7 has, where 701 is the *hidden* ears.
fn first_variant(group: u16) -> u16 {
    if group == CAPE_GROUP {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests;
