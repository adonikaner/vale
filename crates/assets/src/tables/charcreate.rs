//! **What a character may be made of** — the whole of the character-creation
//! screen's data, and none of it is on the wire.
//!
//! ```text
//! ChrRaces          which races there are, in the order the buttons go
//! CharBaseInfo      …and which classes each of them may be
//! ChrClasses        …and what a class is called
//! FactionTemplate   …and which side a race is on
//!  + FactionGroup
//! CharSections      …and how many skins, faces and hair colours it has
//! CharHairGeosets   …and how many hairstyles
//! CharacterFacialHairStyles  …and how many beards, horns or piercings
//! CharStartOutfit   …and what it is *wearing* while you decide
//! ```
//!
//! `CMSG_CHAR_CREATE` carries seven bytes and the server validates them; every
//! decision *before* that press is the client's own, which is why this is a rule
//! module beside [`crate::tables::trainer`] and [`crate::tables::book`] rather than anything in
//! `crates/client`. `vale create` is the check.
//!
//! ## Three rules here are the client's and are in no file at all
//!
//! * **A race is playable unless `ChrRaces.flags & 1`**, tested over the
//!   whole table. In 5875
//!   the only row that sets it is **Goblin** (id 9), which is why the screen has
//!   eight buttons and the table has nine rows.
//! * **The buttons are Alliance first, then Horde**, and that is a *second pass*
//!   over the same table rather than a sort: the client runs its whole ChrRaces
//!   walk twice, taking rows
//!   whose faction group is `"Alliance"` on the first pass and
//!   `"Horde"` on the second. Which group a race is in is its
//!   `FactionTemplate`'s own `factionGroup` mask tested against `1 <<
//!   FactionGroup.MaskID`. So the order is Human, Dwarf, Night Elf, Gnome, Orc,
//!   Undead, Tauren, Troll — and `SetSelectedRace(id)` is an **index into that
//!   list**, not a race id.
//! * **A dwarf may not be a mage, and `CharBaseInfo.dbc` says they may.** The
//!   file really does carry a `(3, 8)` row — 41 records where the eight races'
//!   real combinations are 40 — and the client excludes race 3 with class 8
//!   explicitly. Both the count and the fill carry it. Reading the
//!   table straight puts a Mage button on the dwarf page, which is the exact
//!   shape of failure this project keeps recording: plausible rather than
//!   broken.
//!
//! ## What a "customization" is, and which table each of the five comes from
//!
//! `NUM_CHAR_CUSTOMIZATIONS` is 5 and `CycleCharCustomization(i, ±1)` walks one
//! of them. The reference reads all five out of one precomputed per-(race,
//! gender) index; this reads them from the tables that index was built from,
//! under one rule — **a geometry axis comes from a geoset table and a texture
//! axis from `CharSections`**:
//!
//! | # | axis | table | what varies |
//! |---|---|---|---|
//! | 1 | skin colour | `CharSections` section 0 | `colorIndex` |
//! | 2 | face | `CharSections` section 1 | `variationIndex` |
//! | 3 | hair style | `CharHairGeosets` | `variation` |
//! | 4 | hair colour | `CharSections` section 3 | `colorIndex` |
//! | 5 | facial hair | `CharacterFacialHairStyles` | `variation` |
//!
//! The two geometry axes have to come from the geoset tables rather than from
//! `CharSections`, and a tauren is why: its "facial hair" is **horns**, which
//! have no texture rows at all, so counting `CharSections` section 2 would give
//! a tauren one horn style out of the six it has. The other direction is real
//! too — human male hairstyle 0 is bald, geoset 0, and is a style.
//!
//! **Each axis is a list of ids rather than a count**, because nothing promises
//! a table's variations are `0..n`: a gap would otherwise be cycled onto and
//! resolve to no row, which draws a bald, faceless character rather than
//! failing.
//!
//! ## What `GetHairCustomization` is, and why it is a word
//!
//! `ChrRaces` fields 26, 27 and 28 are three strings — `"NORMAL"`,
//! `"PIERCINGS"`, `"HORNS"`, `"FEATURES"`, `"HAIR"`, `"TUSKS"` — and the screen
//! pastes them into a `GlueStrings.lua` key: `HAIR_<field 28>_STYLE`,
//! `HAIR_<field 28>_COLOR`, `FACIAL_HAIR_<field 26 + gender>`. So the label over
//! customization 5 reads "Facial Hair" for a human male, "Piercings" for a human
//! female and "Horns" for a tauren, and none of those words is written here.
//! The style word is field 28, and the facial-hair word is field
//! `26 + gender`.

use std::collections::BTreeMap;

use crate::tables::dbc::Dbc;

/// Field indices, measured with `vale dbc <table>` and named after what the
/// values in them resolve to. See the module comment for fields 26 to 28.
mod fields {
    /// `ChrRaces.dbc` — 29 fields, 9 records in 5875.
    pub mod races {
        pub const ID: usize = 0;
        /// Bit 0 is "not playable"; see the module comment.
        pub const FLAGS: usize = 1;
        /// A `FactionTemplate` id, which is how a race reaches its side.
        pub const FACTION_TEMPLATE: usize = 2;
        /// `clientFileString` — `"Human"`, `"Scourge"`. The model directory, and
        /// the key `RACE_INFO_*` and `RACE_ICON_TCOORDS` are built from.
        pub const FILE_STRING: usize = 15;
        /// `name_lang`, the enUS column of nine.
        pub const NAME: usize = 17;
        /// `facialHairCustomization[0]` and `[1]`; the gender indexes them.
        pub const FACIAL_HAIR_WORD: usize = 26;
        /// `hairCustomization`, one string for both genders.
        pub const HAIR_WORD: usize = 28;
    }

    /// `ChrClasses.dbc` — 17 fields, 9 records.
    pub mod classes {
        pub const ID: usize = 0;
        /// `name_lang`, enUS of nine.
        pub const NAME: usize = 5;
        /// `filename` — already upper case, which is why
        /// `CLASS_ICON_TCOORDS[classFileName]` is looked up without `strupper`.
        pub const FILE_NAME: usize = 14;
    }

    /// `FactionTemplate.dbc` — the `factionGroup` bit mask at field 3.
    pub const FACTION_TEMPLATE_GROUP: usize = 3;

    /// `FactionGroup.dbc` — id, **MaskID** (a bit *index*, not a mask),
    /// internalName, then `name_lang`.
    pub mod group {
        pub const MASK_ID: usize = 1;
        /// `"Alliance"`, `"Horde"`, `"Player"`, `"Monster"` — compared against
        /// literally by the race walk, and the key `FACTION_INFO_*` is built
        /// from once `strupper`'d.
        pub const INTERNAL_NAME: usize = 2;
        pub const NAME: usize = 3;
    }

    /// `CharSections.dbc` — the same columns [`crate::look::character`] reads, named
    /// again here rather than shared, because that module's are private and the
    /// two readers want different halves of the row.
    pub mod sections {
        pub const RACE: usize = 1;
        pub const GENDER: usize = 2;
        pub const BASE_SECTION: usize = 3;
        pub const VARIATION: usize = 4;
        pub const COLOUR: usize = 5;
    }

    /// `CharHairGeosets.dbc`: id, race, gender, variation, geoset, showScalp.
    pub mod hair {
        pub const RACE: usize = 1;
        pub const GENDER: usize = 2;
        pub const VARIATION: usize = 3;
    }

    /// `CharacterFacialHairStyles.dbc` — **no id column**, the key is the first
    /// three fields.
    pub mod facial {
        pub const RACE: usize = 0;
        pub const GENDER: usize = 1;
        pub const VARIATION: usize = 2;
    }

    /// `CharStartOutfit.dbc` — 82 rows, one per `(race, class, gender)`.
    ///
    /// **Its header lies about its width**: `fieldCount` says 41 and
    /// `recordSize` says **152**, which is 38 fields. Reading past field 37
    /// walks into the next record, and it is the *record size* that is right —
    /// the file's own length checks out at `20 + 82*152 + 1`. Only 0..=37 are
    /// read here.
    ///
    /// Field 1 is **four bytes rather than a number**: race, class, sex,
    /// outfitId. The reference compares them one byte at a time (byte 0
    /// against the selected race, then byte 1 against the class and byte 2
    /// against the sex) and **never looks at
    /// the outfit id at all**, which is why this does not either.
    pub mod outfit {
        /// `[1]`: byte 0 race, byte 1 class, byte 2 sex, byte 3 outfitId.
        pub const KEY: usize = 1;
        /// `[14..26]` — an `ItemDisplayInfo` id per slot, `0` or `-1` for none.
        /// **Not `[2..14]`**, which is the *item* id and needs a template this
        /// screen cannot fetch.
        pub const DISPLAY_ID: usize = 14;
        /// `[26..38]` — and its inventory type, 48 bytes further on.
        /// `0` is a non-equippable, which the outfits carry two
        /// of: rations and a hearthstone.
        pub const INVENTORY_TYPE: usize = 26;
        pub const SLOTS: usize = 12;
    }
}

/// The `baseSection` values the two texture axes live in — the same numbers
/// [`crate::look::character::section`] names, repeated so this module reads without
/// a jump.
mod section {
    pub const SKIN: u32 = 0;
    pub const FACE: u32 = 1;
    pub const HAIR: u32 = 3;
}

/// **The race is not playable.** `ChrRaces.flags` bit 0, and in 5875 the only
/// row that sets it is Goblin.
const RACE_FLAG_NOT_PLAYABLE: u32 = 1;

/// The two faction groups a playable race can be in, **in the order the race
/// buttons are filled** — see the module comment, where the two passes are.
const SIDES: [&str; 2] = ["Alliance", "Horde"];

/// `ChrRaces.dbc` id 3 may not be `ChrClasses.dbc` id 8, whatever
/// `CharBaseInfo.dbc` says.
const DWARF: u8 = 3;
const MAGE: u8 = 8;

/// One of the eight race buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceChoice {
    /// The `ChrRaces` id, which is what goes on the wire. **Not** the button's
    /// index — see [`CharCreate::races`].
    pub id: u8,
    /// `name_lang` — "Human", "Night Elf", "Undead".
    pub name: String,
    /// `clientFileString` — "Human", "NightElf", **"Scourge"**. The model
    /// directory and the stem of every `RACE_INFO_*` and `ABILITY_INFO_*` key.
    pub file_string: String,
    /// `FactionGroup.internalName` — "Alliance" or "Horde". The screen tints its
    /// three backdrops from this and builds `FACTION_INFO_<upper>` out of it.
    pub side: String,
    /// …and the same group's `name_lang`, which is what the label says.
    pub side_name: String,
    /// `hairCustomization`, the stem of `HAIR_<word>_STYLE` and `_COLOR`.
    pub hair_word: String,
    /// `facialHairCustomization[gender]`, the stem of `FACIAL_HAIR_<word>`.
    /// **`"NONE"` hides customization 5 outright**, which is
    /// `CharacterCreate_UpdateFacialHairCustomization`'s own first branch.
    pub facial_hair_word: [String; 2],
}

/// One of the class buttons under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassChoice {
    /// The `ChrClasses` id, which is what goes on the wire.
    pub id: u8,
    /// `name_lang` — "Warrior".
    pub name: String,
    /// `filename`, already upper case — "WARRIOR". `CLASS_ICON_TCOORDS`' key,
    /// and the stem of `CLASS_<name>`.
    pub file_name: String,
}

/// **The five axes a character's look is cycled along**, each as the list of ids
/// that actually have a row — see the module comment for why a list and not a
/// count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Looks {
    pub skins: Vec<u8>,
    pub faces: Vec<u8>,
    pub hair_styles: Vec<u8>,
    pub hair_colours: Vec<u8>,
    pub facial_hairs: Vec<u8>,
}

impl Looks {
    /// The list for one of the five, **one-based** as
    /// `CycleCharCustomization(id, ±1)` numbers them.
    pub fn axis(&self, which: usize) -> &[u8] {
        match which {
            1 => &self.skins,
            2 => &self.faces,
            3 => &self.hair_styles,
            4 => &self.hair_colours,
            5 => &self.facial_hairs,
            _ => &[],
        }
    }
}

/// **What a character being made is wearing** — one slot of its starting outfit.
///
/// A display id and an inventory type is exactly the pair
/// `SMSG_CHAR_ENUM` carries per slot, which is what lets the create screen's
/// character and the character-select plinth be dressed by the same call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutfitPiece {
    /// An `ItemDisplayInfo` id.
    pub display_id: u32,
    /// `INVTYPE_*`. Never 0 — a non-equippable is dropped at the source, see
    /// [`read_outfits`].
    pub inventory_type: u8,
}

impl OutfitPiece {
    /// **Which hand this piece is held in**, or `None` for something worn.
    ///
    /// `0` is the main hand and `1` the off hand, which are the two slots
    /// the dressing loop clears before it dresses (15 and 16) and therefore the two it
    /// can fill. Everything else in the outfit goes on the body.
    ///
    /// **A ranged weapon is worn by nobody here**, and that is the reference's
    /// own line rather than a shortcut: the create screen's clear touches the
    /// twelve visible slots and hands 15 and 16 and *not* 17, and character
    /// select skips slot 17 outright one instruction into its loop. So a hunter
    /// makes their character without a bow on it, in both places.
    pub fn hand(self) -> Option<usize> {
        /// `INVTYPE_WEAPON`, `INVTYPE_2HWEAPON`, `INVTYPE_WEAPONMAINHAND`.
        const MAIN: [u8; 3] = [13, 17, 21];
        /// `INVTYPE_SHIELD`, `INVTYPE_WEAPONOFFHAND`, `INVTYPE_HOLDABLE`.
        const OFF: [u8; 3] = [14, 22, 23];
        if MAIN.contains(&self.inventory_type) {
            Some(0)
        } else if OFF.contains(&self.inventory_type) {
            Some(1)
        } else {
            None
        }
    }

    /// …and whether it is drawn at all: a ranged weapon is not. See
    /// [`Self::hand`], where the reference's two skips are.
    pub fn is_ranged(self) -> bool {
        /// `INVTYPE_RANGED`, `_THROWN`, `_RANGEDRIGHT`, `_RELIC`.
        const RANGED: [u8; 4] = [15, 25, 26, 28];
        RANGED.contains(&self.inventory_type)
    }
}

/// Everything the character-creation screen is drawn from.
#[derive(Debug, Default)]
pub struct CharCreate {
    races: Vec<RaceChoice>,
    /// Race id -> its classes, in `CharBaseInfo`'s own row order.
    classes: BTreeMap<u8, Vec<ClassChoice>>,
    /// `(race, gender)` -> the five axes.
    looks: BTreeMap<(u8, u8), Looks>,
    /// `(race, class, gender)` -> what it starts in.
    outfits: BTreeMap<(u8, u8, u8), Vec<OutfitPiece>>,
}

impl CharCreate {
    /// Read every table this needs out of `read`, which answers by **bare table
    /// name** — the same contract [`crate::tables::dbc::DisplayTables::load`] takes, and
    /// for the same reason: eight byte slices in a row is precisely the shape
    /// that produced this repo's field-index bugs.
    ///
    /// **Nothing here is required.** A chain missing `ChrRaces` answers no races
    /// at all, which is a character-create screen with no buttons rather than a
    /// client that will not start — and every other absence costs its own axis:
    /// no `CharSections` is a character with one skin and one face, no
    /// `CharBaseInfo` is a race with no classes. Each is a screen that visibly
    /// has nothing on it, which is the degradation this crate prefers to a
    /// fabricated default.
    pub fn load(mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> CharCreate {
        let table = |read: &mut dyn FnMut(&str) -> Option<Vec<u8>>, name: &str| {
            read(name).and_then(|raw| Dbc::parse(&raw).ok())
        };
        let races = table(&mut read, "ChrRaces");
        let classes = table(&mut read, "ChrClasses");
        let base = table(&mut read, "CharBaseInfo");
        let templates = table(&mut read, "FactionTemplate");
        let groups = table(&mut read, "FactionGroup");
        let sections = table(&mut read, "CharSections");
        let hair = table(&mut read, "CharHairGeosets");
        let facial = table(&mut read, "CharacterFacialHairStyles");
        let outfits = read_outfits(table(&mut read, "CharStartOutfit").as_ref());

        let races = read_races(races.as_ref(), templates.as_ref(), groups.as_ref());
        let classes = read_classes(base.as_ref(), classes.as_ref());
        let mut looks = BTreeMap::new();
        for race in &races {
            for gender in 0..2u8 {
                looks.insert(
                    (race.id, gender),
                    read_looks(
                        sections.as_ref(),
                        hair.as_ref(),
                        facial.as_ref(),
                        race.id,
                        gender,
                    ),
                );
            }
        }
        CharCreate {
            races,
            classes,
            looks,
            outfits,
        }
    }

    /// **Build one from resolved rows rather than from bytes.**
    ///
    /// The reason this is public rather than a test helper: the *consumer* of
    /// this type is `crate::lua::charcreate` one crate over, and its own tests
    /// are about what happens to an index when the body under it changes — a
    /// tauren's nineteen skins becoming a gnome's five. Expressing that through
    /// hand-built DBC bytes would put a second, subtly different fixture builder
    /// in the client crate and test this module's parser twice instead of
    /// testing the thing in question once.
    ///
    /// `looks` is keyed `(race id, gender)`, as [`Self::looks`] reads it.
    pub fn from_parts(
        races: Vec<RaceChoice>,
        classes: BTreeMap<u8, Vec<ClassChoice>>,
        looks: BTreeMap<(u8, u8), Looks>,
        outfits: BTreeMap<(u8, u8, u8), Vec<OutfitPiece>>,
    ) -> CharCreate {
        CharCreate {
            races,
            classes,
            looks,
            outfits,
        }
    }

    /// **The race buttons, in the order they are drawn** — Alliance then Horde.
    ///
    /// `GetSelectedRace()` and `SetSelectedRace(id)` are one-based *indices into
    /// this list*, which is why the button's own `SetID` is never a race id and
    /// why the wire value has to come back through [`RaceChoice::id`].
    pub fn races(&self) -> &[RaceChoice] {
        &self.races
    }

    /// The classes a race may take, in `CharBaseInfo`'s own order — minus the
    /// dwarf mage.
    pub fn classes_for(&self, race: u8) -> &[ClassChoice] {
        self.classes.get(&race).map_or(&[], Vec::as_slice)
    }

    /// The five axes for one body. Empty for a race and gender the tables do not
    /// describe, which cycles nothing rather than cycling onto a missing row.
    pub fn looks(&self, race: u8, gender: u8) -> Option<&Looks> {
        self.looks.get(&(race, gender))
    }

    /// **What this character starts in**, in `CharStartOutfit`'s own slot order.
    ///
    /// Empty for a combination the table does not describe, which is a character
    /// in its underwear — the same thing an absent table gives, and visibly so.
    pub fn outfit(&self, race: u8, class: u8, gender: u8) -> &[OutfitPiece] {
        self.outfits
            .get(&(race, class, gender))
            .map_or(&[], Vec::as_slice)
    }

    /// **Every class the tables named**, deduplicated by id and in id order.
    ///
    /// [`Self::classes_for`] is the character-create screen's question — which
    /// buttons a race gets — and this is the other one: what a `ChrClasses` id
    /// is *called*, which a friends list and a `/who` both need for somebody
    /// whose race they may not know. Built by walking the per-race lists because
    /// `ChrClasses` is only ever read through them.
    pub fn every_class(&self) -> impl Iterator<Item = &ClassChoice> {
        let mut seen = std::collections::BTreeMap::new();
        for choice in self.classes.values().flatten() {
            seen.entry(choice.id).or_insert(choice);
        }
        seen.into_values()
    }

    /// The race with this `ChrRaces` id, whatever position it is in.
    pub fn race(&self, id: u8) -> Option<&RaceChoice> {
        self.races.iter().find(|r| r.id == id)
    }

    /// How many races, `(race, gender)` bodies and starting outfits were
    /// described, for the census.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.races.len(), self.looks.len(), self.outfits.len())
    }
}

/// `ChrRaces`, filtered and ordered — the two rules the module comment states.
fn read_races(
    races: Option<&Dbc>,
    templates: Option<&Dbc>,
    groups: Option<&Dbc>,
) -> Vec<RaceChoice> {
    use fields::races as f;
    let Some(races) = races else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // **Two passes over the whole table, one per side** — see the module
    // comment. Written as the reference writes it rather than as a sort, because
    // a sort would have to invent a comparator for a race in neither group and
    // the reference simply leaves one out.
    for side in SIDES {
        for record in 0..races.record_count {
            let field = |i: usize| races.u32_at(record, i).unwrap_or(0);
            if field(f::FLAGS) & RACE_FLAG_NOT_PLAYABLE != 0 {
                continue;
            }
            let Some((internal, name)) = faction_group(templates, groups, field(f::FACTION_TEMPLATE))
            else {
                continue;
            };
            if internal != side {
                continue;
            }
            let text = |i: usize| races.string_at(record, i).unwrap_or_default();
            out.push(RaceChoice {
                id: field(f::ID) as u8,
                name: text(f::NAME),
                file_string: text(f::FILE_STRING),
                side: internal,
                side_name: name,
                hair_word: text(f::HAIR_WORD),
                facial_hair_word: [text(f::FACIAL_HAIR_WORD), text(f::FACIAL_HAIR_WORD + 1)],
            });
        }
    }
    out
}

/// Which side a `FactionTemplate` id is on: `(internalName, name_lang)`.
///
/// The template's `factionGroup` is a **mask** and a group's `MaskID` is a bit
/// *index* into it (`1 << MaskID`), which is the client's inner loop.
///
/// **`MaskID` 0 is skipped, and without that every race answers "Player".**
/// `FactionGroup.dbc` has four rows — Player 0, Alliance 1, Horde 2, Monster 3 —
/// and a playable race's template carries **both** its side's bit and Player's:
/// template 1 (human) is `factionGroup = 3`, which is `Player | Alliance`. So the
/// first matching row in file order is Player for every race in the game, and a
/// reader that takes it puts nobody on either side and draws a screen with no
/// race buttons at all. The reference skips a zero `MaskID` before testing
/// the template's `factionGroup` mask.
fn faction_group(
    templates: Option<&Dbc>,
    groups: Option<&Dbc>,
    template_id: u32,
) -> Option<(String, String)> {
    let (templates, groups) = (templates?, groups?);
    let mask = (0..templates.record_count)
        .find(|&r| templates.u32_at(r, 0) == Some(template_id))
        .and_then(|r| templates.u32_at(r, fields::FACTION_TEMPLATE_GROUP))?;
    (0..groups.record_count).find_map(|r| {
        let bit = groups.u32_at(r, fields::group::MASK_ID).filter(|b| *b != 0)?;
        (mask & (1u32 << bit) != 0).then(|| {
            (
                groups
                    .string_at(r, fields::group::INTERNAL_NAME)
                    .unwrap_or_default(),
                groups.string_at(r, fields::group::NAME).unwrap_or_default(),
            )
        })
    })
}

/// `CharBaseInfo` joined onto `ChrClasses`, in the file's own row order.
fn read_classes(base: Option<&Dbc>, classes: Option<&Dbc>) -> BTreeMap<u8, Vec<ClassChoice>> {
    let mut out: BTreeMap<u8, Vec<ClassChoice>> = BTreeMap::new();
    let (Some(base), Some(classes)) = (base, classes) else {
        return out;
    };
    // **`CharBaseInfo` is two *bytes* per record**, a race and a class, which is
    // narrower than a DBC field — so it is read through
    // [`Dbc::record_bytes`] rather than by column. Reading it as a `u32`
    // silently drops the **last** row, because the four-byte read runs two bytes
    // past the block; that row is `(Troll, Mage)`, and the screen it produces
    // has 39 combinations and looks entirely reasonable.
    for record in 0..base.record_count {
        let Some(&[race, class]) = base.record_bytes(record) else {
            continue;
        };
        if race == DWARF && class == MAGE {
            continue;
        }
        let Some(row) = (0..classes.record_count)
            .find(|&r| classes.u32_at(r, fields::classes::ID) == Some(u32::from(class)))
        else {
            continue;
        };
        out.entry(race).or_default().push(ClassChoice {
            id: class,
            name: classes
                .string_at(row, fields::classes::NAME)
                .unwrap_or_default(),
            file_name: classes
                .string_at(row, fields::classes::FILE_NAME)
                .unwrap_or_default(),
        });
    }
    out
}

/// `CharStartOutfit.dbc`, keyed the way the reference keys it.
///
/// **The client's dressing loop is worth reading whole**, because it
/// says both what is drawn and what is *not*: it clears the twelve visible slots
/// and both hands (15 and 16),
/// finds the row whose bytes 4, 5 and 6 are the selected race, class and sex,
/// and then walks **twelve display ids from field 14** with their inventory
/// types 48 bytes further on, skipping any id that is not positive. So a
/// character being made wears its starting outfit, and the outfit id — byte 3 of
/// the key — is never compared.
///
/// Two things are dropped here that the reference passes on and its equipper
/// ignores: a **non-equippable** (inventory type 0, which every outfit has two
/// of — rations and a hearthstone) and a display id of 0, which sits mid-list
/// rather than at the end. Keeping either would put a hearthstone in a wardrobe
/// slot.
fn read_outfits(dbc: Option<&Dbc>) -> BTreeMap<(u8, u8, u8), Vec<OutfitPiece>> {
    use fields::outfit as f;
    let mut out: BTreeMap<(u8, u8, u8), Vec<OutfitPiece>> = BTreeMap::new();
    let Some(dbc) = dbc else { return out };
    for record in 0..dbc.record_count {
        let Some(key) = dbc.u32_at(record, f::KEY) else {
            continue;
        };
        let (race, class, gender) = (key as u8, (key >> 8) as u8, (key >> 16) as u8);
        let mut pieces = Vec::new();
        for slot in 0..f::SLOTS {
            // **Signed, because "no item" is -1 here and 0 elsewhere in the same
            // row.** Reading it unsigned turns the empty slots into display id
            // 4,294,967,295, which resolves to nothing and would be invisible.
            let display = dbc.u32_at(record, f::DISPLAY_ID + slot).unwrap_or(0) as i32;
            let kind = dbc.u32_at(record, f::INVENTORY_TYPE + slot).unwrap_or(0) as i32;
            if display <= 0 || kind <= 0 {
                continue;
            }
            pieces.push(OutfitPiece {
                display_id: display as u32,
                inventory_type: kind as u8,
            });
        }
        out.insert((race, class, gender), pieces);
    }
    out
}

/// The five axes for one `(race, gender)`.
fn read_looks(
    sections: Option<&Dbc>,
    hair: Option<&Dbc>,
    facial: Option<&Dbc>,
    race: u8,
    gender: u8,
) -> Looks {
    // Every axis is "the distinct values of one column over the rows matching
    // this body", sorted — a `BTreeMap` key set in all but name, and small
    // enough (at most a few dozen) that the linear scan is not worth avoiding.
    let distinct = |values: Vec<u8>| {
        let mut v = values;
        v.sort_unstable();
        v.dedup();
        v
    };

    let from_sections = |want: u32, column: usize| -> Vec<u8> {
        use fields::sections as f;
        let Some(dbc) = sections else {
            return Vec::new();
        };
        (0..dbc.record_count)
            .filter(|&r| {
                dbc.u32_at(r, f::RACE) == Some(u32::from(race))
                    && dbc.u32_at(r, f::GENDER) == Some(u32::from(gender))
                    && dbc.u32_at(r, f::BASE_SECTION) == Some(want)
            })
            .filter_map(|r| dbc.u32_at(r, column).map(|v| v as u8))
            .collect()
    };

    let from_geosets = |dbc: Option<&Dbc>, race_at: usize, gender_at: usize, at: usize| {
        let Some(dbc) = dbc else { return Vec::new() };
        (0..dbc.record_count)
            .filter(|&r| {
                dbc.u32_at(r, race_at) == Some(u32::from(race))
                    && dbc.u32_at(r, gender_at) == Some(u32::from(gender))
            })
            .filter_map(|r| dbc.u32_at(r, at).map(|v| v as u8))
            .collect()
    };

    Looks {
        skins: distinct(from_sections(section::SKIN, fields::sections::COLOUR)),
        faces: distinct(from_sections(section::FACE, fields::sections::VARIATION)),
        hair_styles: distinct(from_geosets(
            hair,
            fields::hair::RACE,
            fields::hair::GENDER,
            fields::hair::VARIATION,
        )),
        hair_colours: distinct(from_sections(section::HAIR, fields::sections::COLOUR)),
        facial_hairs: distinct(from_geosets(
            facial,
            fields::facial::RACE,
            fields::facial::GENDER,
            fields::facial::VARIATION,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DBC of `u32` fields with a string block.
    fn dbc(rows: &[Vec<u32>], fields: usize, strings: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for row in rows {
            for i in 0..fields {
                out.extend_from_slice(&row.get(i).copied().unwrap_or(0).to_le_bytes());
            }
        }
        out.extend_from_slice(strings);
        out
    }

    /// …and `CharBaseInfo`, whose record is **two bytes**.
    fn char_base_info(pairs: &[(u8, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for (race, class) in pairs {
            out.push(*race);
            out.push(*class);
        }
        out.push(0);
        out
    }

    /// Offsets into the shared string block below.
    const S: &[u8] = b"\0Human\0Dwarf\0Goblin\0Alliance\0Horde\0NORMAL\0PIERCINGS\0Warrior\0WARRIOR\0Mage\0MAGE\0Orc\0Player\0";
    const HUMAN: u32 = 1;
    const DWARF_S: u32 = 7;
    const GOBLIN: u32 = 13;
    const ALLIANCE: u32 = 20;
    const HORDE: u32 = 29;
    const NORMAL: u32 = 35;
    const PIERCINGS: u32 = 42;
    const WARRIOR: u32 = 52;
    const WARRIOR_F: u32 = 60;
    const MAGE_S: u32 = 68;
    const MAGE_F: u32 = 73;
    const ORC: u32 = 78;
    const PLAYER: u32 = 82;

    /// Three races: a human (Alliance), a dwarf (Alliance, declared *after* an
    /// orc so the two-pass order is actually exercised), an orc (Horde), and a
    /// goblin flagged unplayable.
    fn chr_races() -> Vec<u8> {
        let row = |id: u32, flags: u32, template: u32, file: u32, name: u32| {
            let mut r = vec![0u32; 29];
            r[0] = id;
            r[1] = flags;
            r[2] = template;
            r[15] = file;
            r[17] = name;
            r[26] = NORMAL;
            r[27] = PIERCINGS;
            r[28] = NORMAL;
            r
        };
        dbc(
            &[
                row(1, 12, 1, HUMAN, HUMAN),
                // The orc sits between the two Alliance rows on purpose.
                row(2, 12, 2, ORC, ORC),
                row(3, 12, 1, DWARF_S, DWARF_S),
                row(9, 1, 1, GOBLIN, GOBLIN),
            ],
            29,
            S,
        )
    }

    /// **The real table's shape, Player row and all.** Template 1 is
    /// `Player | Alliance` = 3 and template 2 is `Player | Horde` = 5, which is
    /// what 5875 actually holds — so a reader that does not skip `MaskID` 0
    /// answers "Player" for both and this fixture's races vanish.
    fn faction_tables() -> (Vec<u8>, Vec<u8>) {
        let templates = dbc(&[vec![1, 0, 0, 3], vec![2, 0, 0, 5]], 14, b"\0");
        // id, MaskID, internalName, name_lang — Player first, as it is in file.
        let groups = dbc(
            &[
                vec![1, 0, PLAYER, PLAYER],
                vec![2, 1, ALLIANCE, ALLIANCE],
                vec![3, 2, HORDE, HORDE],
            ],
            12,
            S,
        );
        (templates, groups)
    }

    fn chr_classes() -> Vec<u8> {
        let row = |id: u32, name: u32, file: u32| {
            let mut r = vec![0u32; 17];
            r[0] = id;
            r[5] = name;
            r[14] = file;
            r
        };
        dbc(&[row(1, WARRIOR, WARRIOR_F), row(8, MAGE_S, MAGE_F)], 17, S)
    }

    fn tables(name: &str) -> Option<Vec<u8>> {
        let (templates, groups) = faction_tables();
        Some(match name {
            "ChrRaces" => chr_races(),
            "ChrClasses" => chr_classes(),
            // Every race gets a warrior and a mage; the dwarf's mage is the one
            // the client's own rule takes away. **The orc's mage is deliberately
            // the last row**, because a record two bytes wide puts the final
            // pair against the end of the block — see [`Dbc::record_bytes`],
            // which exists because reading it as a `u32` drops that row.
            "CharBaseInfo" => char_base_info(&[(1, 1), (1, 8), (3, 1), (3, 8), (2, 1), (2, 8)]),
            "FactionTemplate" => templates,
            "FactionGroup" => groups,
            _ => return None,
        })
    }

    /// **Goblin is out and the order is Alliance first**, which are the two
    /// halves of the client's double pass — and the second is what makes
    /// `SetSelectedRace(2)` mean the dwarf rather than the orc.
    #[test]
    fn the_race_buttons_are_alliance_then_horde_and_the_goblin_is_not_one() {
        let created = CharCreate::load(tables);
        let order: Vec<u8> = created.races().iter().map(|r| r.id).collect();
        assert_eq!(order, vec![1, 3, 2], "human, dwarf, then the orc");
        assert!(
            created.race(9).is_none(),
            "ChrRaces flag bit 0 is not playable"
        );
        assert_eq!(created.races()[0].side, "Alliance");
        assert_eq!(created.races()[2].side, "Horde");
        // The words the two `GlueStrings.lua` keys are pasted out of.
        assert_eq!(created.races()[0].hair_word, "NORMAL");
        assert_eq!(
            created.races()[0].facial_hair_word,
            ["NORMAL".to_string(), "PIERCINGS".to_string()]
        );
    }

    /// **A dwarf may not be a mage even though `CharBaseInfo.dbc` says so** —
    /// the client's explicit exclusion, and the one rule on this screen
    /// that no file states. The human keeps the same combination, which is what
    /// says the exclusion is about the pair and not about the class.
    #[test]
    fn the_dwarf_mage_row_in_the_file_is_the_clients_own_exception() {
        let created = CharCreate::load(tables);
        let dwarf: Vec<u8> = created.classes_for(3).iter().map(|c| c.id).collect();
        assert_eq!(dwarf, vec![1], "the dwarf's mage row is dropped");
        let human: Vec<u8> = created.classes_for(1).iter().map(|c| c.id).collect();
        assert_eq!(human, vec![1, 8], "…and only the dwarf's");
        // …and the *last* row in the file is read at all, which a four-byte read
        // over a two-byte record is not: in 5875 that row is the troll's mage.
        let orc: Vec<u8> = created.classes_for(2).iter().map(|c| c.id).collect();
        assert_eq!(orc, vec![1, 8], "the final record is a record");
        // `filename` is already upper case, which is why the screen looks it up
        // in `CLASS_ICON_TCOORDS` without `strupper`.
        assert_eq!(created.classes_for(1)[1].file_name, "MAGE");
        assert_eq!(created.classes_for(1)[1].name, "Mage");
    }

    /// **The outfit's key is four bytes and its display ids start at field 14.**
    ///
    /// Three separate ways to get this wrong, and none of them fails loudly: a
    /// transposed race and class dresses everybody as somebody else, reading the
    /// *item* ids at field 2 instead of the display ids at 14 resolves to nothing
    /// at all, and reading the empty slots unsigned turns -1 into display id
    /// 4,294,967,295 — which also resolves to nothing, so a client that made both
    /// mistakes would look exactly like one that made neither.
    #[test]
    fn a_starting_outfit_is_keyed_by_four_bytes_and_read_from_the_display_ids() {
        // One row: race 3, class 5, sex 1, outfit 0 — and the layout's own three
        // blocks, with the two shapes of "nothing" the real table mixes.
        let mut row = vec![0u32; 38];
        row[0] = 7;
        row[1] = u32::from_le_bytes([3, 5, 1, 0]);
        // items — deliberately different numbers, so reading the wrong block shows
        for (i, v) in [38u32, 25, 0].into_iter().enumerate() {
            row[2 + i] = v;
        }
        for (i, v) in [9891u32, 1542, u32::MAX].into_iter().enumerate() {
            row[14 + i] = v;
        }
        for (i, v) in [4u32, 21, u32::MAX].into_iter().enumerate() {
            row[26 + i] = v;
        }
        // …and a fourth slot that is a *hearthstone*: a real display id with an
        // inventory type of 0, which the reference's equipper ignores.
        row[17] = 6418;
        row[29] = 0;

        let created = CharCreate::load(|name| match name {
            "CharStartOutfit" => Some(dbc(&[row.clone()], 38, b"\0")),
            other => tables(other),
        });
        let worn = created.outfit(3, 5, 1);
        assert_eq!(
            worn,
            [
                OutfitPiece { display_id: 9891, inventory_type: 4 },
                OutfitPiece { display_id: 1542, inventory_type: 21 },
            ],
            "the display ids, and neither kind of empty"
        );
        // The key is bytes, not a number: nothing else answers.
        assert!(created.outfit(5, 3, 1).is_empty(), "race and class are not swapped");
        assert!(created.outfit(3, 5, 0).is_empty(), "the sex is the third byte");

        // …and where each piece goes, which is the reference's two cleared hands.
        assert_eq!(worn[0].hand(), None, "a shirt is worn");
        assert_eq!(worn[1].hand(), Some(0), "INVTYPE_WEAPONMAINHAND");
        let shield = OutfitPiece { display_id: 1, inventory_type: 14 };
        assert_eq!(shield.hand(), Some(1), "a shield is the off hand");
        let bow = OutfitPiece { display_id: 1, inventory_type: 15 };
        assert!(bow.is_ranged(), "…and a bow is drawn nowhere on this screen");
        assert_eq!(bow.hand(), None);
    }

    /// **A missing table costs its own axis and nothing else.** A chain with no
    /// `ChrRaces` is a screen with no buttons, which is visible; a client that
    /// refused to start would not be.
    #[test]
    fn every_table_here_is_optional() {
        let empty = CharCreate::load(|_| None);
        assert!(empty.races().is_empty());
        assert!(empty.classes_for(1).is_empty());
        assert_eq!(empty.looks(1, 0), None);

        // …and the races without the classes, which is the more likely half.
        let no_classes = CharCreate::load(|name| match name {
            "CharBaseInfo" | "ChrClasses" => None,
            other => tables(other),
        });
        assert_eq!(no_classes.races().len(), 3);
        assert!(no_classes.classes_for(1).is_empty());
    }

    /// **The five axes are lists of ids and not counts**, so a table with a gap
    /// in it cycles onto the ids that exist rather than onto the ones that
    /// would be there if the column were dense.
    #[test]
    fn a_customization_axis_is_the_ids_that_have_a_row() {
        // CharSections: race 1, gender 0 — skins 0 and 2 (no 1), faces 0 and 1.
        let section_row = |section: u32, variation: u32, colour: u32| {
            let mut r = vec![0u32; 10];
            r[1] = 1;
            r[2] = 0;
            r[3] = section;
            r[4] = variation;
            r[5] = colour;
            r
        };
        let sections = dbc(
            &[
                section_row(section::SKIN, 0, 0),
                section_row(section::SKIN, 0, 2),
                section_row(section::FACE, 0, 0),
                section_row(section::FACE, 1, 0),
                section_row(section::FACE, 1, 2),
                section_row(section::HAIR, 0, 5),
                section_row(section::HAIR, 1, 5),
                section_row(section::HAIR, 1, 6),
                // Another body entirely, which must not leak into the answer.
                {
                    let mut r = section_row(section::SKIN, 0, 9);
                    r[2] = 1;
                    r
                },
            ],
            10,
            b"\0",
        );
        // CharHairGeosets: id, race, gender, variation, geoset, showScalp.
        let hair = dbc(
            &[vec![0, 1, 0, 0, 0, 1], vec![1, 1, 0, 3, 2, 1]],
            6,
            b"\0",
        );
        // CharacterFacialHairStyles: race, gender, variation, then six more.
        let facial = dbc(&[vec![1, 0, 0], vec![1, 0, 1], vec![1, 1, 0]], 9, b"\0");

        let created = CharCreate::load(|name| match name {
            "CharSections" => Some(sections.clone()),
            "CharHairGeosets" => Some(hair.clone()),
            "CharacterFacialHairStyles" => Some(facial.clone()),
            other => tables(other),
        });
        let looks = created.looks(1, 0).expect("the human male");
        assert_eq!(looks.skins, vec![0, 2], "the gap is not filled in");
        assert_eq!(looks.faces, vec![0, 1], "distinct, not one per row");
        assert_eq!(looks.hair_styles, vec![0, 3], "geometry, not texture");
        assert_eq!(looks.hair_colours, vec![5, 6]);
        assert_eq!(looks.facial_hairs, vec![0, 1], "the female row is not ours");
        // …and the one-based numbering `CycleCharCustomization` uses.
        assert_eq!(looks.axis(1), &[0, 2]);
        assert_eq!(looks.axis(5), &[0, 1]);
        assert!(looks.axis(0).is_empty(), "there is no customization 0");
        assert!(looks.axis(6).is_empty());
    }
}
