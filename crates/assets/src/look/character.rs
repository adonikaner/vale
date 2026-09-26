//! Composing a player's skin, which the game does not ship and cannot.
//!
//! An NPC wearing a character model has its body texture **baked** — one file
//! per `CreatureDisplayInfoExtra` row, computed offline from a race, a face, a
//! hairstyle and ten equipped items — and [`crate::tables::dbc::DisplayTables`] simply
//! names it. A *player* has no such row: display ids 49..57 are the bare race
//! models, and the client composes the same texture at runtime from the
//! player's own appearance fields. That is why other players draw magenta
//! against a client that only knows how to look a name up.
//!
//! This module is the composition. It is two halves, deliberately separable:
//!
//! * [`CharSections`] turns an [`Appearance`] into a **recipe** — a list of
//!   archive paths and where each lands in the composite. Pure table lookup,
//!   no pixels, so it is unit-testable and the CLI can check every combination
//!   in the game against the archive without decoding anything.
//! * [`Composite`] does the pixels: alpha-blend each decoded layer into its
//!   region, in order.
//!
//! ## The composite is a 256x256 atlas of body parts
//!
//! A character M2's UVs address one texture for the whole body, and the pieces
//! are laid into fixed rectangles of it — the layout below, which is
//! WoWModelViewer's `regions[]` and is what the vanilla client hardcodes (the
//! `CharComponentTextureSections.dbc` that would state it arrives in
//! Cataclysm). **It is checked against the files rather than trusted**: every
//! source texture in the game must be exactly the size of the region it is
//! declared to fill, and `vale char` asserts that over the whole table. A
//! wrong rectangle does not fail — it puts a face on a thigh.
//!
//! ```text
//!   0,0                    128,0                  256,0
//!    +----------------------+----------------------+
//!    | arm upper   128x64   | torso upper  128x64  |
//!    +----------------------+----------------------+ 64
//!    | arm lower   128x64   | torso lower  128x32  |
//!    |                      +----------------------+ 96
//!    +----------------------+ leg upper    128x64  |
//!    | hand        128x32   |                      |
//!    +----------------------+----------------------+ 160
//!    | face upper  128x32   | leg lower    128x64  |
//!    +----------------------+                      |
//!    | face lower  128x64   |                      |
//!    |                      +----------------------+ 224
//!    |                      | foot         128x32  |
//!    +----------------------+----------------------+ 256
//! ```
//!
//! The base skin covers all 256x256 and everything else is painted over it.
//!
//! ## `CharSections.dbc`
//!
//! `id, race, gender, baseSection, variationIndex, colorIndex, texture[3],
//! flags` — **not** the layout most references give, which puts the three
//! textures at 4..6; here they are at 6..8. The measurement is self-checking,
//! because the filenames encode the two indices: record 413 is race 1 gender 1
//! section 1 variation 4 colour 9 and reads `HumanFemaleFaceLower04_09.blp` +
//! `HumanFemaleFaceUpper04_09.blp`.
//!
//! `baseSection` is 0 skin, 1 face, 2 facial hair, 3 hair, 4 underwear. Which
//! of the three texture columns means what depends on the section, and the
//! names say so: a face row's two are lower and upper, a hair row's first is
//! the *hair mesh's* own texture (the M2 asks for it as texture type 6, so it
//! is not composited at all) and its other two are the scalp, which is.
//!
//! Note that the variation digit in a filename is *not* always
//! `variationIndex`: facial-hair record 1283 is variation 4 but reads
//! `FacialLowerHair00_08`, because the style there is geometry — a geoset — and
//! only the colour is in the texture.

use crate::{AssetError, Dbc};

/// Side of the composed body texture. The character models' UVs are addressed
/// against this, so it is not a quality knob.
pub const COMPOSITE_SIDE: u32 = 256;

/// A rectangle of the composite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    const fn new(x: u32, y: u32, width: u32, height: u32) -> Region {
        Region {
            x,
            y,
            width,
            height,
        }
    }
}

/// Where each body part lives in the composite. See the module docs for the
/// picture and for why these are checked against the files rather than trusted.
pub mod regions {
    use super::Region;

    pub const BASE: Region = Region::new(0, 0, 256, 256);
    pub const ARM_UPPER: Region = Region::new(0, 0, 128, 64);
    pub const ARM_LOWER: Region = Region::new(0, 64, 128, 64);
    pub const HAND: Region = Region::new(0, 128, 128, 32);
    pub const FACE_UPPER: Region = Region::new(0, 160, 128, 32);
    pub const FACE_LOWER: Region = Region::new(0, 192, 128, 64);
    pub const TORSO_UPPER: Region = Region::new(128, 0, 128, 64);
    pub const TORSO_LOWER: Region = Region::new(128, 64, 128, 32);
    pub const LEG_UPPER: Region = Region::new(128, 96, 128, 64);
    pub const LEG_LOWER: Region = Region::new(128, 160, 128, 64);
    pub const FOOT: Region = Region::new(128, 224, 128, 32);
}

/// `CharSections.baseSection`.
pub mod section {
    pub const SKIN: u32 = 0;
    pub const FACE: u32 = 1;
    pub const FACIAL_HAIR: u32 = 2;
    pub const HAIR: u32 = 3;
    pub const UNDERWEAR: u32 = 4;
}

/// Field indices in `CharSections.dbc`. Measured — see the module docs.
mod fields {
    pub const RACE: usize = 1;
    pub const GENDER: usize = 2;
    pub const BASE_SECTION: usize = 3;
    pub const VARIATION_INDEX: usize = 4;
    pub const COLOR_INDEX: usize = 5;
    pub const TEXTURE: usize = 6;
}

/// Everything the server tells the client about how a character looks, out of
/// `UNIT_FIELD_BYTES_0` (race, gender) and `PLAYER_BYTES` / `PLAYER_BYTES_2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Appearance {
    pub race: u8,
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_colour: u8,
    pub facial_hair: u8,
}

impl Appearance {
    /// Unpack the three update fields vmangos packs these into.
    ///
    /// `Unit::GetRace` and friends are `GetByteValue(field, offset)`, which is
    /// `((uint8*)&value)[offset]` — so offset 0 is the *least* significant byte
    /// and this is a plain little-endian split. Getting the order backwards
    /// gives a human male orc, which is a texture path that does not exist
    /// rather than a wrong-looking one, and would read as a missing file.
    pub fn from_fields(bytes_0: u32, player_bytes: u32, player_bytes_2: u32) -> Appearance {
        Appearance {
            race: bytes_0 as u8,
            gender: (bytes_0 >> 16) as u8,
            skin: player_bytes as u8,
            face: (player_bytes >> 8) as u8,
            hair_style: (player_bytes >> 16) as u8,
            hair_colour: (player_bytes >> 24) as u8,
            facial_hair: player_bytes_2 as u8,
        }
    }
}

/// One texture to paint into the composite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinLayer {
    pub path: String,
    pub region: Region,
}

/// What to draw a character with: the body composite's ingredients, and the
/// separate texture the hair *mesh* wears.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CharacterSkin {
    /// In paint order — the base skin first, everything else over it.
    pub layers: Vec<SkinLayer>,
    /// The hair mesh's own texture, which the M2 asks for as type 6 and which
    /// is **not** part of the composite: it dresses separate geometry.
    pub hair: Option<String>,
}

/// Field indices in the two geoset tables. Measured the same way as the rest —
/// `vale dbc <Table>` and read the columns.
mod geoset_fields {
    /// `CharHairGeosets.dbc`: id, race, gender, variation, geoset, showScalp.
    ///
    /// Record 0 is race 1 gender 0 variation 0 -> geoset **0**, showScalp 1, and
    /// record 1 is variation 1 -> geoset 2. So the hairstyle index the server
    /// sends is this table's `variation`, the geoset is a group-0 id, and zero
    /// is a real answer rather than a missing row: human male style 0 is bald,
    /// and the scalp the composite paints is what is seen instead.
    pub mod hair {
        pub const RACE: usize = 1;
        pub const GENDER: usize = 2;
        pub const VARIATION: usize = 3;
        pub const GEOSET: usize = 4;
        pub const SHOW_SCALP: usize = 5;
    }

    /// `CharacterFacialHairStyles.dbc`: race, gender, variation, three fields
    /// that are not geosets, then the three that are.
    ///
    /// **There is no id column** — the key is the first three fields — and
    /// fields 3..5 hold the same three numbers (80190984, 0, 1960) in every one
    /// of the 136 rows, which is what says they are not per-style data. The
    /// three at 6..8 vary with the variation and are the geoset *variants* for
    /// groups 1, 3 and 2 in that order (WoWModelViewer's `geoset100`,
    /// `geoset300`, `geoset200`).
    ///
    /// **That order is measured, not taken on authority.** `vale char`
    /// checks every style's geosets against the geosets the race's own model
    /// carries, and of the six ways the three columns could map onto the three
    /// groups this one is the only one that fits: 27 of 408 name a geoset the
    /// model lacks, where the next best is 71 and the worst 115. The 27 are
    /// rows shipped without the geometry to draw them, the same kind of gap as
    /// the 44 `CharSections` textures that are not in the archive.
    pub mod facial {
        pub const RACE: usize = 0;
        pub const GENDER: usize = 1;
        pub const VARIATION: usize = 2;
        /// Groups 1, 3, 2 — not 1, 2, 3.
        pub const GEOSETS: [usize; 3] = [6, 7, 8];
        pub const GROUPS: [usize; 3] = [1, 3, 2];
    }
}

/// The two DBCs that say which *geometry* a character's own appearance selects.
///
/// The skin is a texture and is composed ([`CharSections`]); the hair and the
/// beard are geosets, and this is where they come from. Both tables are keyed by
/// race, gender and a variation index the server sends in `PLAYER_BYTES`, and
/// both differ between two characters sharing one M2 — which is why the batch
/// list a model draws cannot be baked per file.
pub struct CharGeosets {
    hair: Vec<HairRow>,
    facial: Vec<FacialRow>,
}

struct HairRow {
    race: u8,
    gender: u8,
    variation: u8,
    geoset: u16,
    show_scalp: bool,
}

struct FacialRow {
    race: u8,
    gender: u8,
    variation: u8,
    geosets: [u16; 3],
}

/// A hairstyle resolved: which geoset to draw, and whether the scalp under it
/// is meant to show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hair {
    pub geoset: u16,
    /// `showScalp`. A style that covers the head sets this to 0 — the composite
    /// paints the scalp textures regardless today, which is invisible under
    /// hair and correct without it.
    pub show_scalp: bool,
}

impl CharGeosets {
    /// Both tables. Either may be missing: the cost is bald characters or
    /// beardless ones, and neither is worth failing an entity over.
    pub fn parse(hair: &[u8], facial: &[u8]) -> CharGeosets {
        use geoset_fields::{facial as ff, hair as hf};

        let mut hair_rows = Vec::new();
        if let Ok(dbc) = Dbc::parse(hair) {
            for record in 0..dbc.record_count {
                let field = |i: usize| dbc.u32_at(record, i).unwrap_or(0);
                hair_rows.push(HairRow {
                    race: field(hf::RACE) as u8,
                    gender: field(hf::GENDER) as u8,
                    variation: field(hf::VARIATION) as u8,
                    geoset: field(hf::GEOSET) as u16,
                    show_scalp: field(hf::SHOW_SCALP) != 0,
                });
            }
        }

        let mut facial_rows = Vec::new();
        if let Ok(dbc) = Dbc::parse(facial) {
            for record in 0..dbc.record_count {
                let field = |i: usize| dbc.u32_at(record, i).unwrap_or(0);
                let mut geosets = [0u16; 3];
                for (i, &column) in ff::GEOSETS.iter().enumerate() {
                    // Into the group's own slot, so the caller does not have to
                    // know that the columns are ordered 1, 3, 2.
                    geosets[ff::GROUPS[i] - 1] = field(column) as u16;
                }
                facial_rows.push(FacialRow {
                    race: field(ff::RACE) as u8,
                    gender: field(ff::GENDER) as u8,
                    variation: field(ff::VARIATION) as u8,
                    geosets,
                });
            }
        }

        CharGeosets {
            hair: hair_rows,
            facial: facial_rows,
        }
    }

    /// How many rows each table gave, for the survey.
    pub fn len(&self) -> (usize, usize) {
        (self.hair.len(), self.facial.len())
    }

    /// Every hairstyle the table knows: race, gender, variation, the geoset it
    /// names, and `showScalp`. For the check that those geosets exist in the
    /// models that are meant to draw them.
    pub fn hair_styles(&self) -> Vec<(u8, u8, u8, u16, bool)> {
        self.hair
            .iter()
            .map(|r| (r.race, r.gender, r.variation, r.geoset, r.show_scalp))
            .collect()
    }

    /// Every facial-hair style: race, gender, variation, and the three group
    /// variants, already in group order.
    pub fn facial_styles(&self) -> Vec<(u8, u8, u8, [u16; 3])> {
        self.facial
            .iter()
            .map(|r| (r.race, r.gender, r.variation, r.geosets))
            .collect()
    }

    /// The hairstyle row for one appearance.
    pub fn hair(&self, look: &Appearance) -> Option<Hair> {
        self.hair
            .iter()
            .find(|r| {
                r.race == look.race && r.gender == look.gender && r.variation == look.hair_style
            })
            .map(|r| Hair {
                geoset: r.geoset,
                show_scalp: r.show_scalp,
            })
    }

    /// Everything a character's appearance chooses, in the form the model
    /// filter wants.
    ///
    /// A row this table does not have costs that piece and nothing else — no
    /// hair row means bald, which is what the client did for every character
    /// before these tables were read at all.
    pub fn geosets(&self, look: &Appearance) -> crate::world::m2::CharacterGeosets {
        let facial = self
            .facial
            .iter()
            .find(|r| {
                r.race == look.race && r.gender == look.gender && r.variation == look.facial_hair
            })
            .map(|r| r.geosets)
            .unwrap_or_default();
        crate::world::m2::CharacterGeosets {
            hair: self.hair(look).map(|h| h.geoset).unwrap_or(0),
            facial,
            // Both filled by the caller from the wearer's equipment, which is a
            // server answer rather than a DBC one.
            equipment: [0; 12],
            hide_ears: false,
        }
    }
}

/// `CharSections.dbc`, indexed by the five things that select a row.
pub struct CharSections {
    rows: Vec<Row>,
}

struct Row {
    race: u8,
    gender: u8,
    section: u32,
    variation: u8,
    colour: u8,
    textures: [String; 3],
}

impl CharSections {
    pub fn parse(bytes: &[u8]) -> Result<CharSections, AssetError> {
        let dbc = Dbc::parse(bytes)?;
        let mut rows = Vec::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let text = |i: usize| {
                dbc.string_at(record, fields::TEXTURE + i)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default()
            };
            rows.push(Row {
                race: dbc.u32_at(record, fields::RACE).unwrap_or(0) as u8,
                gender: dbc.u32_at(record, fields::GENDER).unwrap_or(0) as u8,
                section: dbc.u32_at(record, fields::BASE_SECTION).unwrap_or(0),
                variation: dbc.u32_at(record, fields::VARIATION_INDEX).unwrap_or(0) as u8,
                colour: dbc.u32_at(record, fields::COLOR_INDEX).unwrap_or(0) as u8,
                textures: [text(0), text(1), text(2)],
            });
        }
        Ok(CharSections { rows })
    }

    /// How many rows the table holds. For the survey.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn find(&self, race: u8, gender: u8, section: u32, variation: u8, colour: u8) -> Option<&Row> {
        self.rows.iter().find(|r| {
            r.race == race
                && r.gender == gender
                && r.section == section
                && r.variation == variation
                && r.colour == colour
        })
    }

    /// The recipe for one character.
    ///
    /// Every part is optional and a missing one costs only itself: a face
    /// variation the table does not have leaves the base skin showing through,
    /// which is a featureless but correctly *coloured* character rather than a
    /// magenta one. That is the whole point of building this bottom-up — the
    /// base skin alone is already a large improvement, and each further layer
    /// is independent of the rest.
    pub fn skin(&self, look: &Appearance) -> CharacterSkin {
        let (race, gender) = (look.race, look.gender);
        let mut layers = Vec::new();
        let mut push = |path: &str, region: Region| {
            if !path.is_empty() {
                layers.push(SkinLayer {
                    path: path.to_string(),
                    region,
                });
            }
        };

        // The base skin covers the whole texture; its *variation* is always 0
        // and the colour index is the skin tone.
        if let Some(row) = self.find(race, gender, section::SKIN, 0, look.skin) {
            push(&row.textures[0], regions::BASE);
        }

        // Underwear, before the face, because it is body and the face is not:
        // a bra and pants over the bare skin. Male rows carry only the pelvis.
        if let Some(row) = self.find(race, gender, section::UNDERWEAR, 0, look.skin) {
            push(&row.textures[0], regions::LEG_UPPER);
            push(&row.textures[1], regions::TORSO_UPPER);
        }

        // The face is keyed by the face id *and* the skin tone, which is why
        // there are (faces x skins) rows of it and not just faces.
        if let Some(row) = self.find(race, gender, section::FACE, look.face, look.skin) {
            push(&row.textures[0], regions::FACE_LOWER);
            push(&row.textures[1], regions::FACE_UPPER);
        }

        // The scalp goes on before facial hair, so a beard is drawn over it.
        // A hair row's first texture is the hair *mesh's*, and is not painted
        // into the composite at all.
        let hair = self
            .find(race, gender, section::HAIR, look.hair_style, look.hair_colour)
            .and_then(|row| {
                push(&row.textures[1], regions::FACE_LOWER);
                push(&row.textures[2], regions::FACE_UPPER);
                (!row.textures[0].is_empty()).then(|| row.textures[0].clone())
            });

        // Facial hair is coloured by the *hair* colour, not the skin's.
        if let Some(row) = self.find(
            race,
            gender,
            section::FACIAL_HAIR,
            look.facial_hair,
            look.hair_colour,
        ) {
            push(&row.textures[0], regions::FACE_LOWER);
            push(&row.textures[1], regions::FACE_UPPER);
        }

        CharacterSkin { layers, hair }
    }

    /// Every row's `(section, texture path, the region it is declared to fill)`,
    /// for the survey that checks those declarations against the files.
    ///
    /// Only the columns this client actually paints appear — a hair row's first
    /// texture dresses the hair mesh and has no region.
    pub fn declared_regions(&self) -> Vec<(u32, &str, Region)> {
        let mut out = Vec::new();
        for row in &self.rows {
            let mut add = |i: usize, region: Region| {
                if !row.textures[i].is_empty() {
                    out.push((row.section, row.textures[i].as_str(), region));
                }
            };
            match row.section {
                section::SKIN => add(0, regions::BASE),
                section::UNDERWEAR => {
                    add(0, regions::LEG_UPPER);
                    add(1, regions::TORSO_UPPER);
                }
                section::FACE | section::FACIAL_HAIR => {
                    add(0, regions::FACE_LOWER);
                    add(1, regions::FACE_UPPER);
                }
                section::HAIR => {
                    add(1, regions::FACE_LOWER);
                    add(2, regions::FACE_UPPER);
                }
                _ => {}
            }
        }
        out
    }

    /// **Every hair-mesh texture the table names**, which is the one file per
    /// row that [`Self::declared_regions`] deliberately leaves out.
    ///
    /// A hair row's texture 0 dresses *geometry* — the M2 asks for it as
    /// replaceable texture type 6 — so it has no region in the composite and the
    /// region survey has nothing to say about it. That is exactly why it needed
    /// its own accessor: it was the only file in this table that nothing
    /// checked against the archive, and a hair mesh with no texture bound draws
    /// **magenta** rather than drawing nothing.
    pub fn hair_meshes(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|row| row.section == section::HAIR && !row.textures[0].is_empty())
            .map(|row| row.textures[0].as_str())
            .collect()
    }

    /// Every distinct `(race, gender, skin, face, hairStyle, hairColour,
    /// facialHair)` the table can describe, for the survey. Built from the rows
    /// rather than from a hardcoded range of races, so a table with custom rows
    /// in it is covered too.
    pub fn every_appearance(&self) -> Vec<Appearance> {
        let mut out = Vec::new();
        let mut races: Vec<(u8, u8)> = self.rows.iter().map(|r| (r.race, r.gender)).collect();
        races.sort_unstable();
        races.dedup();
        for (race, gender) in races {
            let of = |section: u32| -> Vec<(u8, u8)> {
                let mut v: Vec<(u8, u8)> = self
                    .rows
                    .iter()
                    .filter(|r| r.race == race && r.gender == gender && r.section == section)
                    .map(|r| (r.variation, r.colour))
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            };
            let skins: Vec<u8> = of(section::SKIN).into_iter().map(|(_, c)| c).collect();
            let faces = of(section::FACE);
            let hair = of(section::HAIR);
            let beards = of(section::FACIAL_HAIR);
            for &skin in &skins {
                for &(face, _) in faces.iter().filter(|(_, c)| *c == skin) {
                    for &(hair_style, hair_colour) in &hair {
                        let facial_hair = beards
                            .iter()
                            .find(|(_, c)| *c == hair_colour)
                            .map(|(v, _)| *v)
                            .unwrap_or(0);
                        out.push(Appearance {
                            race,
                            gender,
                            skin,
                            face,
                            hair_style,
                            hair_colour,
                            facial_hair,
                        });
                    }
                }
            }
        }
        out
    }
}

/// The composed body texture, built one layer at a time.
///
/// Starts fully transparent rather than black: the base skin covers all of it
/// in practice, but a race whose base row is missing should show the model's
/// own geometry through rather than a black silhouette.
pub struct Composite {
    rgba: Vec<u8>,
}

impl Default for Composite {
    fn default() -> Self {
        Composite::new()
    }
}

impl Composite {
    pub fn new() -> Composite {
        Composite {
            rgba: vec![0u8; (COMPOSITE_SIDE * COMPOSITE_SIDE * 4) as usize],
        }
    }

    /// Alpha-blend one decoded RGBA layer into its region.
    ///
    /// The source is scaled to the region by nearest sampling, which in the
    /// game's own data is never actually a scale — every file is exactly its
    /// region's size, and `vale char` checks that over the whole table. It
    /// is here so that a patched archive with a higher-resolution replacement
    /// lands in the right rectangle rather than a corner of it.
    pub fn paint(&mut self, region: Region, width: u32, height: u32, rgba: &[u8]) {
        if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
            return;
        }
        for y in 0..region.height {
            let dy = region.y + y;
            if dy >= COMPOSITE_SIDE {
                break;
            }
            let sy = y * height / region.height;
            for x in 0..region.width {
                let dx = region.x + x;
                if dx >= COMPOSITE_SIDE {
                    break;
                }
                let sx = x * width / region.width;
                let s = ((sy * width + sx) * 4) as usize;
                let d = ((dy * COMPOSITE_SIDE + dx) * 4) as usize;

                // Straight source-over. The layers above the base are mostly
                // transparent — a face is a patch on a 128x64 rectangle — so
                // ignoring alpha would paint rectangles of blank texture over
                // the skin, which is what "the face is a floating box" looks
                // like.
                let a = rgba[s + 3] as u32;
                if a == 0 {
                    continue;
                }
                for c in 0..3 {
                    let src = rgba[s + c] as u32;
                    let dst = self.rgba[d + c] as u32;
                    self.rgba[d + c] = ((src * a + dst * (255 - a)) / 255) as u8;
                }
                self.rgba[d + 3] = (a + (self.rgba[d + 3] as u32) * (255 - a) / 255).min(255) as u8;
            }
        }
    }

    /// The finished 256x256 RGBA texture.
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// A box-filtered mip chain for a texture this client generated.
///
/// The BLPs in the archive carry Blizzard's own mips and nothing here has to
/// invent any — but a *composed* skin has no file to take them from, and a
/// character with one mip level shimmers exactly the way the ground did.
///
/// Each level averages a 2x2 block of the one above **weighted by alpha**, so a
/// transparent texel's colour cannot bleed into its neighbours; a straight
/// average darkens every edge towards whatever the unused colour under the
/// transparency happens to be.
///
/// That weighting is only right for a texture whose alpha *is* transparency. See
/// [`box_mips`] for the other kind.
pub fn generate_mips(width: u32, height: u32, top: &[u8]) -> Vec<Vec<u8>> {
    mips(width, height, top, true)
}

/// The same chain with every tap weighted equally.
///
/// **For a four-channel texture that is four independent maps rather than a
/// colour and its transparency.** The terrain's alpha atlas is the one this
/// client has: three blend weights and, since shadows, `MCSH` in the alpha — and
/// [`generate_mips`]' weighting would fade a chunk's *blend* out wherever its
/// **shadow** does, which is a plausible-looking ground rather than a failure.
pub fn box_mips(width: u32, height: u32, top: &[u8]) -> Vec<Vec<u8>> {
    mips(width, height, top, false)
}

fn mips(width: u32, height: u32, top: &[u8], weighted: bool) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let (mut w, mut h) = (width, height);
    let mut level = top.to_vec();
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut colour = [0u32; 3];
                let mut alpha = 0u32;
                let mut weight = 0u32;
                let mut taps = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let s = ((sy * w + sx) * 4) as usize;
                        let a = level[s + 3] as u32;
                        let tap = if weighted { a } else { 1 };
                        for c in 0..3 {
                            colour[c] += level[s + c] as u32 * tap;
                        }
                        alpha += a;
                        weight += tap;
                        taps += 1;
                    }
                }
                let d = ((y * nw + x) * 4) as usize;
                for c in 0..3 {
                    next[d + c] = if weight > 0 {
                        (colour[c] / weight) as u8
                    } else {
                        0
                    };
                }
                next[d + 3] = (alpha / taps.max(1)) as u8;
            }
        }
        out.push(next.clone());
        level = next;
        w = nw;
        h = nh;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DBC_MAGIC: &[u8; 4] = b"WDBC";

    /// A `CharSections.dbc` from rows of ten fields.
    fn build(rows: &[Vec<u32>], strings: &[u8]) -> Vec<u8> {
        let fields = 10usize;
        let mut v = Vec::new();
        v.extend_from_slice(DBC_MAGIC);
        v.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        v.extend_from_slice(&(fields as u32).to_le_bytes());
        v.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        v.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for r in rows {
            for f in 0..fields {
                v.extend_from_slice(&r.get(f).copied().unwrap_or(0).to_le_bytes());
            }
        }
        v.extend_from_slice(strings);
        v
    }

    fn row(
        id: u32,
        race: u8,
        gender: u8,
        section: u32,
        variation: u8,
        colour: u8,
        textures: [u32; 3],
    ) -> Vec<u32> {
        let mut r = vec![0u32; 10];
        r[0] = id;
        r[fields::RACE] = race as u32;
        r[fields::GENDER] = gender as u32;
        r[fields::BASE_SECTION] = section;
        r[fields::VARIATION_INDEX] = variation as u32;
        r[fields::COLOR_INDEX] = colour as u32;
        for (i, t) in textures.into_iter().enumerate() {
            r[fields::TEXTURE + i] = t;
        }
        r
    }

    /// vmangos packs these with `GetByteValue(field, offset)`, which indexes the
    /// value's bytes — so offset 0 is the low byte. Reading them the other way
    /// round turns a human into an orc, which resolves to a path that is simply
    /// not in the archive and therefore reads as a missing file rather than as
    /// a wrong one.
    #[test]
    fn the_appearance_fields_unpack_little_endian() {
        // race 1, class 5, gender 1, powerType 0
        let bytes_0 = 1 | (5 << 8) | (1 << 16);
        // skin 4, face 3, hairStyle 2, hairColour 7
        let player_bytes = 4 | (3 << 8) | (2 << 16) | (7 << 24);
        let look = Appearance::from_fields(bytes_0, player_bytes, 6);
        assert_eq!(look.race, 1);
        assert_eq!(look.gender, 1);
        assert_eq!(look.skin, 4);
        assert_eq!(look.face, 3);
        assert_eq!(look.hair_style, 2);
        assert_eq!(look.hair_colour, 7);
        assert_eq!(look.facial_hair, 6);
    }

    /// The recipe, end to end. Each part is keyed by a different pair of the
    /// appearance's fields and getting one wrong picks a real row for the wrong
    /// character — a plausible face, not an error.
    #[test]
    fn a_recipe_takes_each_part_from_its_own_row() {
        let strings = b"\0skin.blp\0facelo.blp\0faceup.blp\0hair.blp\0scalplo.blp\0beardlo.blp\0pelvis.blp\0";
        let at = |needle: &[u8]| {
            strings
                .windows(needle.len())
                .position(|w| w == needle)
                .expect("string present") as u32
        };
        let (skin, facelo, faceup) = (at(b"skin.blp"), at(b"facelo.blp"), at(b"faceup.blp"));
        let (hair, scalplo, beardlo, pelvis) = (
            at(b"hair.blp"),
            at(b"scalplo.blp"),
            at(b"beardlo.blp"),
            at(b"pelvis.blp"),
        );

        let table = CharSections::parse(&build(
            &[
                // Base skin: variation is always 0, colour is the tone.
                row(1, 1, 0, section::SKIN, 0, 3, [skin, 0, 0]),
                // A face for the *same* tone, and a decoy for another one.
                row(2, 1, 0, section::FACE, 2, 3, [facelo, faceup, 0]),
                row(3, 1, 0, section::FACE, 2, 9, [at(b"skin.blp"), 0, 0]),
                // Hair: the first column dresses the mesh, the rest is scalp.
                row(4, 1, 0, section::HAIR, 5, 7, [hair, scalplo, 0]),
                // Facial hair takes the *hair* colour, not the skin's.
                row(5, 1, 0, section::FACIAL_HAIR, 4, 7, [beardlo, 0, 0]),
                row(6, 1, 0, section::UNDERWEAR, 0, 3, [pelvis, 0, 0]),
            ],
            strings,
        ))
        .expect("parsed");

        let look = Appearance {
            race: 1,
            gender: 0,
            skin: 3,
            face: 2,
            hair_style: 5,
            hair_colour: 7,
            facial_hair: 4,
        };
        let composed = table.skin(&look);

        assert_eq!(
            composed.hair.as_deref(),
            Some("hair.blp"),
            "the hair mesh's texture is not part of the composite"
        );
        let painted: Vec<(&str, Region)> = composed
            .layers
            .iter()
            .map(|l| (l.path.as_str(), l.region))
            .collect();
        assert_eq!(
            painted,
            vec![
                ("skin.blp", regions::BASE),
                ("pelvis.blp", regions::LEG_UPPER),
                ("facelo.blp", regions::FACE_LOWER),
                ("faceup.blp", regions::FACE_UPPER),
                ("scalplo.blp", regions::FACE_LOWER),
                ("beardlo.blp", regions::FACE_LOWER),
            ],
            "wrong parts, or the wrong order"
        );
    }

    /// The failure mode that matters: a face the table does not have must leave
    /// a correctly coloured body, not a magenta one. Every layer is optional and
    /// independent.
    #[test]
    fn a_missing_part_costs_only_itself() {
        let strings = b"\0skin.blp\0";
        let table = CharSections::parse(&build(
            &[row(1, 1, 0, section::SKIN, 0, 0, [1, 0, 0])],
            strings,
        ))
        .expect("parsed");

        let composed = table.skin(&Appearance {
            race: 1,
            gender: 0,
            face: 9,
            hair_style: 9,
            ..Default::default()
        });
        assert_eq!(composed.layers.len(), 1, "the base skin still resolves");
        assert_eq!(composed.layers[0].region, regions::BASE);
        assert_eq!(composed.hair, None);
    }

    /// A race the table says nothing about yields nothing, rather than the
    /// first row it happened to find.
    #[test]
    fn an_unknown_race_yields_no_layers() {
        let strings = b"\0skin.blp\0";
        let table = CharSections::parse(&build(
            &[row(1, 1, 0, section::SKIN, 0, 0, [1, 0, 0])],
            strings,
        ))
        .expect("parsed");
        let composed = table.skin(&Appearance {
            race: 8,
            ..Default::default()
        });
        assert!(composed.layers.is_empty());
    }

    /// The regions must tile the composite without overlapping, or two body
    /// parts share texels and one of them is drawn on the other.
    #[test]
    fn the_regions_tile_the_composite_exactly_once() {
        let parts = [
            regions::ARM_UPPER,
            regions::ARM_LOWER,
            regions::HAND,
            regions::FACE_UPPER,
            regions::FACE_LOWER,
            regions::TORSO_UPPER,
            regions::TORSO_LOWER,
            regions::LEG_UPPER,
            regions::LEG_LOWER,
            regions::FOOT,
        ];
        let mut covered = vec![0u8; (COMPOSITE_SIDE * COMPOSITE_SIDE) as usize];
        for r in parts {
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    covered[(y * COMPOSITE_SIDE + x) as usize] += 1;
                }
            }
        }
        assert!(
            covered.iter().all(|&n| n == 1),
            "the body-part regions must exactly partition the 256x256 base"
        );
        // And the base is the whole of it.
        assert_eq!(regions::BASE.width * regions::BASE.height, covered.len() as u32);
    }

    /// A transparent layer must not paint. Half the area of a face texture is
    /// blank, and ignoring alpha covers the skin with rectangles of nothing —
    /// which is what "the face is a floating box" looks like.
    #[test]
    fn a_transparent_texel_leaves_what_is_under_it() {
        let mut composite = Composite::new();
        composite.paint(regions::BASE, 1, 1, &[10, 20, 30, 255]);
        // One fully transparent texel over the face region.
        composite.paint(regions::FACE_LOWER, 1, 1, &[200, 0, 0, 0]);
        let at = |x: u32, y: u32| {
            let o = ((y * COMPOSITE_SIDE + x) * 4) as usize;
            &composite.rgba()[o..o + 4]
        };
        assert_eq!(at(regions::FACE_LOWER.x, regions::FACE_LOWER.y), &[10, 20, 30, 255]);

        // And an opaque one does paint, in its own rectangle only.
        let mut composite = Composite::new();
        composite.paint(regions::BASE, 1, 1, &[10, 20, 30, 255]);
        composite.paint(regions::FACE_LOWER, 1, 1, &[200, 0, 0, 255]);
        let at = |x: u32, y: u32| {
            let o = ((y * COMPOSITE_SIDE + x) * 4) as usize;
            composite.rgba()[o..o + 4].to_vec()
        };
        assert_eq!(at(regions::FACE_LOWER.x, regions::FACE_LOWER.y), vec![200, 0, 0, 255]);
        // The torso is on the other half of the texture and must be untouched.
        assert_eq!(at(regions::TORSO_UPPER.x, regions::TORSO_UPPER.y), vec![10, 20, 30, 255]);
    }

    /// A composed skin has no file to take mips from, so they are generated —
    /// and the chain has to have exactly the shape the uploader expects, or the
    /// sampler reads one level's bytes as another's.
    #[test]
    fn generated_mips_halve_to_one_by_one() {
        let top = vec![255u8; (4 * 4 * 4) as usize];
        let mips = generate_mips(4, 4, &top);
        assert_eq!(mips.len(), 2, "4x4 -> 2x2 -> 1x1");
        assert_eq!(mips[0].len(), 2 * 2 * 4);
        assert_eq!(mips[1].len(), 4);
        assert_eq!(mips[1], vec![255, 255, 255, 255], "a flat white stays white");
    }

    /// A transparent texel's colour must not bleed into the average, or every
    /// edge in the composite darkens towards whatever was under the alpha.
    #[test]
    fn mip_averaging_is_weighted_by_alpha() {
        // 2x1: one opaque white, one transparent black.
        let top = vec![255, 255, 255, 255, 0, 0, 0, 0];
        let mips = generate_mips(2, 1, &top);
        assert_eq!(mips.len(), 1);
        assert_eq!(&mips[0][0..3], &[255, 255, 255], "the black must not count");
        assert_eq!(mips[0][3], 127, "but the alpha is still averaged");
    }
}
