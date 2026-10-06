//! The ground's own foliage tables: `GroundEffectTexture` joined to
//! `GroundEffectDoodad`, which name the grass tufts and wildflowers each
//! terrain texture plants.
//!
//! `MCLY`'s `effectId` has two other readers: the footstep sound
//! ([`crate::tables::sound::SoundBank::terrain_of_ground_effect`]) and
//! [`crate::world::adt::Adt::ground_effect_at`]. Here the same id names up to
//! four models and how many tufts to plant.
//!
//! ```text
//! GroundEffectTexture   12,742 rows x 7   id, doodad[4], density, terrainType
//! GroundEffectDoodad       443 rows x 3   id, ordinal, file name
//! ```
//!
//! Of the 12,742 texture rows, 730 name a doodad (the rest are
//! `-1, -1, -1, -1`); 706 of those name four, and 470 name four distinct ones.
//! Density runs 0..20, and 424 of the 730 rows state 8.
//!
//! 49 rows name only doodad ids `GroundEffectDoodad` does not carry (92 such
//! ids in all) and are dropped, so `vale foliage` reports 681 rows that plant
//! something. A wrong reading of the id columns would change that number. The
//! 7 rows that state a density of 0 are kept: the 1.12.1 client plants
//! [`DEFAULT_DENSITY`] tufts for them.
//!
//! ## What the density is
//!
//! The density is how many tufts one draw of a detail cell plants. The 1.12.1
//! client draws `frillDensity` cells per chunk at random, repeats allowed, and
//! plants each draw's density of tufts in that cell. Each tuft's model is the
//! row's column `(draw + tuft) & 3`, and an empty column plants nothing; so a
//! row that repeats an id is weighted toward it, and a row with empty columns
//! is sparser. See [`crate::world::foliage`], where the draws become
//! positions.
//!
//! ## The models are in `World\NoDXT\Detail\`, and 32 of them are missing
//!
//! The table names a bare file (`ElwFlo01.mdl`, `PlagueLandsFun01.mdx`) with no
//! directory and with the `.mdl`/`.mdx` extension the archive stores as `.m2`.
//! The directory is [`DETAIL_DIR`], found by listing the archives: 411 of the
//! 443 names resolve there and the other 32, the Tirisfal and Silverpine sets,
//! are in no archive in the chain under any name. They stay in the table: the
//! renderer's model cache reports a model that will not load once and skips
//! it, and dropping them here would hide a missing archive.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// The directory the ground's own doodads are in. Found by listing the chain;
/// the table carries only a bare file name.
pub const DETAIL_DIR: &str = "World\\NoDXT\\Detail\\";

/// The columns each of the two tables answers from.
pub mod fields {
    /// `GroundEffectTexture`: the four doodads a texture may plant, `-1` where
    /// the row names fewer than four.
    pub const DOODAD: [usize; 4] = [1, 2, 3, 4];
    /// …how many of them one draw of a detail cell plants; see
    /// [`super::GroundEffect::density`].
    pub const DENSITY: usize = 5;
    /// …and field 6, the `TerrainType` row a footstep is looked up through.
    /// It is [`crate::tables::sound`]'s column, listed here only so that the
    /// seventh field is accounted for.
    pub const TERRAIN_TYPE: usize = 6;

    /// `GroundEffectDoodad`: the bare file name, still with its `.mdl`/`.mdx`
    /// extension. Field 1 is a plain 0-based ordinal and is read by nothing.
    pub const PATH: usize = 2;
}

/// The most doodads one `GroundEffectTexture` row may name.
pub const CHOICES: usize = 4;

/// The tuft count the 1.12.1 client uses for a row whose density column is 0.
pub const DEFAULT_DENSITY: u8 = 8;

/// [`GroundEffect::slots`]' value for a slot the row leaves empty (`-1`) or
/// fills with an id `GroundEffectDoodad` does not carry.
pub const NO_SLOT: u16 = u16::MAX;

/// One texture layer's ground effect: which models, and how many tufts per
/// draw of a cell.
///
/// `Copy` and 20 bytes, because it is looked up once per draw and built once
/// per load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroundEffect {
    /// Indices into [`GroundEffects::models`], the first [`Self::choices`] of
    /// them real, in column order with the empty columns left out. Repeats
    /// are kept: a row naming one model four times weights the draw toward
    /// it.
    pub models: [u16; CHOICES],
    /// How many of [`Self::models`] the row named, 1..=4.
    pub choices: u8,
    /// The four doodad columns in the row's own order, as indices into
    /// [`GroundEffects::models`], [`NO_SLOT`] where a column names nothing
    /// usable. The 1.12.1 client picks a tuft's model by column, so an empty
    /// column is a tuft not planted rather than a weight on the others.
    pub slots: [u16; CHOICES],
    /// How many tufts one draw of a detail cell plants. Never 0: the client
    /// plants [`DEFAULT_DENSITY`] for a row that states 0.
    pub density: u8,
}

impl GroundEffect {
    /// The model in column `n & 3`, or `None` for an empty column.
    pub fn slot(&self, n: u32) -> Option<u16> {
        Some(self.slots[(n & 3) as usize]).filter(|&index| index != NO_SLOT)
    }
}

/// The two tables, joined: an `effectId` to what it plants.
#[derive(Debug, Default)]
pub struct GroundEffects {
    effects: HashMap<u32, GroundEffect>,
    models: Vec<String>,
}

impl GroundEffects {
    /// Read both tables through `read`, which answers by bare table name: the
    /// same contract as [`crate::tables::dbc::DisplayTables::load`] and
    /// [`crate::tables::sound::SoundBank::load`], for the same reason.
    ///
    /// Never fails. A chain missing either table yields an empty set, and an
    /// empty set draws bare ground, a visible degradation rather than a crash.
    /// Neither table is required by anything else.
    pub fn load(mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> GroundEffects {
        let mut out = GroundEffects::default();

        // The doodads first, because the texture rows are resolved through
        // them: a row naming an id this table does not carry names nothing.
        let mut by_id: HashMap<u32, u16> = HashMap::new();
        if let Some(raw) = read("GroundEffectDoodad") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    let Some(name) = dbc.string_at(r, fields::PATH) else {
                        continue;
                    };
                    if name.is_empty() {
                        continue;
                    }
                    let index = out.models.len() as u16;
                    out.models.push(format!("{DETAIL_DIR}{}", as_m2(&name)));
                    by_id.insert(id, index);
                }
            }
        }

        if let Some(raw) = read("GroundEffectTexture") {
            if let Ok(dbc) = Dbc::parse(&raw) {
                for r in 0..dbc.record_count {
                    let Some(id) = dbc.u32_at(r, 0) else { continue };
                    // 0 is planted at the client's default. 255 is as many as
                    // a `u8` can hold; the table's own maximum is 20, so the
                    // clamp only matters for a patched file.
                    let density = match dbc.u32_at(r, fields::DENSITY).unwrap_or(0).min(255) as u8 {
                        0 => DEFAULT_DENSITY,
                        n => n,
                    };
                    let mut models = [0u16; CHOICES];
                    let mut slots = [NO_SLOT; CHOICES];
                    let mut choices = 0usize;
                    for (slot, &field) in fields::DOODAD.iter().enumerate() {
                        // `-1` is the table's "no doodad", and it arrives here
                        // as `0xFFFF_FFFF` because a DBC field has no sign.
                        let Some(doodad) = dbc.u32_at(r, field) else {
                            continue;
                        };
                        let Some(&index) = by_id.get(&doodad) else {
                            continue;
                        };
                        models[choices] = index;
                        slots[slot] = index;
                        choices += 1;
                    }
                    if choices == 0 {
                        continue;
                    }
                    out.effects.insert(
                        id,
                        GroundEffect {
                            models,
                            choices: choices as u8,
                            slots,
                            density,
                        },
                    );
                }
            }
        }

        out
    }

    /// What an `MCLY` `effectId` plants, or `None`, which is the answer for
    /// 12,012 of the 12,742 rows and for every id the table does not carry,
    /// including 0.
    pub fn effect(&self, effect_id: u32) -> Option<&GroundEffect> {
        self.effects.get(&effect_id)
    }

    /// The archive path of a model index, or `""` for an index from another
    /// load. The path is already `World\NoDXT\Detail\…​.m2`.
    pub fn model(&self, index: u16) -> &str {
        self.models.get(index as usize).map_or("", String::as_str)
    }

    /// Every model the table names, in index order.
    pub fn models(&self) -> &[String] {
        &self.models
    }

    /// How many texture rows plant anything: 681 on a shipped 1.12 chain.
    pub fn effect_count(&self) -> usize {
        self.effects.len()
    }

    /// Nothing loaded, which is a world with bare ground.
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }
}

/// The table's `.mdl`/`.mdx` name as the file that is actually in the archive.
///
/// The same swap `MDDF`'s model names need: 1.12 ships `.m2` and every table
/// in the game still names the authoring extension.
fn as_m2(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) => format!("{}.m2", &name[..dot]),
        None => format!("{name}.m2"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `WDBC` with `fields` u32 columns and one string block.
    fn dbc(rows: &[Vec<u32>], strings: &[u8]) -> Vec<u8> {
        let fields = rows.first().map_or(0, Vec::len);
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for row in rows {
            for value in row {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.extend_from_slice(strings);
        out
    }

    /// `\0ElwGra01.mdl\0Fern.mdx\0` — offset 1 and offset 14.
    fn doodad_strings() -> Vec<u8> {
        let mut s = vec![0u8];
        s.extend_from_slice(b"ElwGra01.mdl\0");
        s.extend_from_slice(b"Fern.mdx\0");
        s
    }

    fn tables() -> GroundEffects {
        let doodads = dbc(&[vec![7, 0, 1], vec![9, 1, 14]], &doodad_strings());
        let textures = dbc(
            &[
                // Four slots, two named: grass, fern, nothing, nothing.
                vec![100, 7, 9, u32::MAX, u32::MAX, 8, 5],
                // The same doodad four times, which is how the file weights a
                // model.
                vec![101, 7, 7, 7, 7, 3, 0],
                // Density 0: planted at the client's default.
                vec![102, 7, 9, u32::MAX, u32::MAX, 0, 5],
                // Names only ids the doodad table does not carry.
                vec![103, 4242, u32::MAX, u32::MAX, u32::MAX, 8, 5],
            ],
            &[0],
        );
        GroundEffects::load(|table| match table {
            "GroundEffectDoodad" => Some(doodads.clone()),
            "GroundEffectTexture" => Some(textures.clone()),
            _ => None,
        })
    }

    /// The join, and the directory and extension the table does not state.
    #[test]
    fn a_row_resolves_to_the_files_the_archive_actually_holds() {
        let tables = tables();
        let effect = tables.effect(100).expect("row 100 plants two models");
        assert_eq!(effect.choices, 2);
        assert_eq!(effect.density, 8);
        assert_eq!(
            tables.model(effect.models[0]),
            "World\\NoDXT\\Detail\\ElwGra01.m2"
        );
        // `.mdx` swaps exactly as `.mdl` does; the table uses both.
        assert_eq!(tables.model(effect.models[1]), "World\\NoDXT\\Detail\\Fern.m2");
    }

    /// A row whose doodad ids are in no `GroundEffectDoodad` names nothing and
    /// is dropped. A row with density 0 is kept at [`DEFAULT_DENSITY`], which
    /// is what the 1.12.1 client plants for it.
    #[test]
    fn a_row_that_names_no_model_is_not_an_effect() {
        let tables = tables();
        let zero = tables.effect(102).expect("density 0 is still an effect");
        assert_eq!(zero.density, DEFAULT_DENSITY);
        assert!(tables.effect(103).is_none(), "no resolvable doodad");
        assert!(tables.effect(0).is_none(), "id 0 is not a row");
        assert!(tables.effect(999).is_none());
        assert_eq!(tables.effect_count(), 3);
    }

    /// A repeated id is kept: row 101 names one model four times, so every
    /// column answers that model. Deduplicating would reweight every row
    /// that repeats an id, 260 of the 730.
    #[test]
    fn a_repeated_doodad_is_a_weight_and_not_a_duplicate() {
        let tables = tables();
        let effect = tables.effect(101).expect("row 101 is an effect");
        assert_eq!(effect.choices, 4);
        for column in 0..16 {
            let model = effect.slot(column).expect("every column is named");
            assert_eq!(tables.model(model), "World\\NoDXT\\Detail\\ElwGra01.m2");
        }
    }

    /// A tuft's model is its row's column `n & 3`, and an empty column plants
    /// nothing: row 100 names two of its four columns.
    #[test]
    fn an_empty_column_answers_no_model() {
        let tables = tables();
        let effect = tables.effect(100).expect("row 100");
        let answered: Vec<bool> = (0..8).map(|n| effect.slot(n).is_some()).collect();
        assert_eq!(answered, [true, true, false, false, true, true, false, false]);
        assert_eq!(effect.slot(0), Some(effect.models[0]));
        assert_eq!(effect.slot(1), Some(effect.models[1]));
    }

    /// An absent table means bare ground, not a failure. Neither DBC is
    /// required by anything else in the client, so a chain without them has to
    /// load and answer nothing.
    #[test]
    fn no_tables_is_an_empty_set_and_not_an_error() {
        let tables = GroundEffects::load(|_| None);
        assert!(tables.is_empty());
        assert!(tables.effect(100).is_none());
        assert_eq!(tables.models().len(), 0);
        assert_eq!(tables.model(0), "");
    }
}
