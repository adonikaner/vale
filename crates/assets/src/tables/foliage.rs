//! **What grows on the ground** — `GroundEffectTexture` -> `GroundEffectDoodad`,
//! which is the join behind every tuft of grass and every wildflower in the
//! world.
//!
//! `MCLY`'s `effectId` is already read for two other reasons: it is what a
//! [footstep sounds like](crate::tables::sound::SoundBank::terrain_of_ground_effect)
//! and it is what [`crate::world::adt::Adt::ground_effect_at`] answers. This is
//! its third and largest reader — the same id, in the same column, naming up to
//! four models and how thickly to plant them.
//!
//! ```text
//! GroundEffectTexture   12,742 rows x 7   id, doodad[4], density, terrainType
//! GroundEffectDoodad       443 rows x 3   id, ordinal, file name
//! ```
//!
//! **Both shapes are measured, not assumed.** Of the 12,742 texture rows only
//! **730** name a doodad at all (the rest are `-1, -1, -1, -1`), 706 of those
//! name four and 470 name four *distinct* ones — a row repeating an id is the
//! file's own way of weighting a choice, and this module keeps the repeat
//! rather than deduplicating it. Density runs **0..20** with 8 by a landslide
//! (424 of 730).
//!
//! Two of those 730 plant nothing after all, and both are dropped here: **7**
//! state a density of 0, and **49** name only doodad ids `GroundEffectDoodad`
//! does not carry (92 such ids in all). So `vale foliage` reports **674**
//! rows that plant something, and that is the number to watch — an id table
//! read wrongly would move it.
//!
//! ## Density is a count, and that is the one interpretation here
//!
//! Everything else in this file is read off the table. That density is *the
//! number of doodads in one detail cell* is not: it is a small integer in a
//! column with no name, and the client's own placement rule was not
//! checked. What
//! supports it is the shape — integers 0..20, a hard mode at 8, a value of 0
//! meaning "nothing" — and what it produces on the ground: at 8 per cell a
//! detail cell of 4.17 yards square carries one tuft per ~2.2 square yards,
//! which is a lawn rather than a meadow or a bare field. See
//! [`crate::world::foliage`], which is where it becomes positions.
//!
//! ## The models are `World\NoDXT\Detail\`, and 32 of them are not there
//!
//! The table names a bare file (`ElwFlo01.mdl`, `PlagueLandsFun01.mdx`) with no
//! directory and the client's own `.mdl`/`.mdx` -> `.m2` swap still to do. The
//! directory is [`DETAIL_DIR`], measured by listing the archives: 411 of the
//! 443 names resolve there and the other **32 are simply absent** — the
//! Tirisfal and Silverpine sets, which are in no archive in the chain under any
//! name. They are left in the table rather than filtered out, because a model
//! that will not read is something the renderer's own cache reports once and
//! skips, and dropping them here would hide a missing archive as a missing row.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// Where the ground's own doodads live — measured by listing the chain, not
/// stated by the table, which carries a bare file name.
pub const DETAIL_DIR: &str = "World\\NoDXT\\Detail\\";

/// The columns each of the two tables answers from.
pub mod fields {
    /// `GroundEffectTexture`: the four doodads a texture may plant, `-1` where
    /// the row names fewer than four.
    pub const DOODAD: [usize; 4] = [1, 2, 3, 4];
    /// …how many of them go in one detail cell. See the module doc: this is
    /// the interpretation, not the measurement.
    pub const DENSITY: usize = 5;
    /// …and field 6, which is the `TerrainType` row a footstep is looked up
    /// through — [`crate::tables::sound`]'s column, listed here only so that
    /// the seventh field is accounted for.
    pub const TERRAIN_TYPE: usize = 6;

    /// `GroundEffectDoodad`: the bare file name, still with its `.mdl`/`.mdx`
    /// extension. Field 1 is a plain 0-based ordinal and is read by nothing.
    pub const PATH: usize = 2;
}

/// The most doodads one `GroundEffectTexture` row may name.
pub const CHOICES: usize = 4;

/// One texture layer's ground effect: which models, and how many per cell.
///
/// `Copy` and 12 bytes: a detail cell holds one of these per *cell*, so it is
/// looked up far more often than it is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroundEffect {
    /// Indices into [`GroundEffects::models`], the first [`Self::choices`] of
    /// them real. **Repeats are kept**: a row naming one model four times is
    /// the file weighting it, and a row naming two of one and two of another
    /// wants those odds.
    pub models: [u16; CHOICES],
    /// How many of [`Self::models`] the row actually named — 1..=4.
    pub choices: u8,
    /// How many doodads one detail cell gets. Never 0: a row that states 0 is
    /// not an effect at all and is dropped at load.
    pub density: u8,
}

impl GroundEffect {
    /// The model this cell's `n`th doodad is, by the caller's own draw.
    ///
    /// `pick` is any integer; it is reduced modulo the number of choices, so a
    /// caller hands in a hash and gets the file's own odds without knowing how
    /// many there were.
    pub fn model(&self, pick: u32) -> u16 {
        self.models[(pick % u32::from(self.choices.max(1))) as usize]
    }
}

/// The two tables, joined: an `effectId` to what it plants.
#[derive(Debug, Default)]
pub struct GroundEffects {
    effects: HashMap<u32, GroundEffect>,
    models: Vec<String>,
}

impl GroundEffects {
    /// Read both tables off `read`, which answers by **bare table name** — the
    /// same contract as [`crate::tables::dbc::DisplayTables::load`] and
    /// [`crate::tables::sound::SoundBank::load`], for the same reason.
    ///
    /// **Never fails.** A chain missing either table yields an empty set, and
    /// an empty set is a world with bare ground — which is what this client was
    /// before this module existed, and is a visible degradation rather than a
    /// crash. Neither table is required by anything else.
    pub fn load(mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> GroundEffects {
        let mut out = GroundEffects::default();

        // **The doodads first**, because the texture rows are resolved through
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
                    // 0 plants nothing, and 255 is as many as a `u8` can say —
                    // the table's own maximum is 20, so the clamp never fires
                    // on a shipped file and is here for a patched one.
                    let density = dbc.u32_at(r, fields::DENSITY).unwrap_or(0).min(255) as u8;
                    if density == 0 {
                        continue;
                    }
                    let mut models = [0u16; CHOICES];
                    let mut choices = 0usize;
                    for &field in &fields::DOODAD {
                        // `-1` is the table's "no doodad", and it arrives here
                        // as `0xFFFF_FFFF` because a DBC field has no sign.
                        let Some(doodad) = dbc.u32_at(r, field) else {
                            continue;
                        };
                        let Some(&index) = by_id.get(&doodad) else {
                            continue;
                        };
                        models[choices] = index;
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
                            density,
                        },
                    );
                }
            }
        }

        out
    }

    /// What an `MCLY` `effectId` plants, or `None` — which is the answer for
    /// **12,012 of the 12,742 rows** and for every id the table does not carry
    /// at all, including 0.
    pub fn effect(&self, effect_id: u32) -> Option<&GroundEffect> {
        self.effects.get(&effect_id)
    }

    /// The archive path of a model index, or `""` for an index from another
    /// load. Already `World\NoDXT\Detail\…​.m2`.
    pub fn model(&self, index: u16) -> &str {
        self.models.get(index as usize).map_or("", String::as_str)
    }

    /// Every model the table names, in index order.
    pub fn models(&self) -> &[String] {
        &self.models
    }

    /// How many texture rows plant anything — 730 on a shipped 1.12 chain.
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
/// The same swap `MDDF`'s model names need — 1.12 ships `.m2` and every table
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
                // The same doodad four times — the file's own way of weighting.
                vec![101, 7, 7, 7, 7, 3, 0],
                // Density 0: not an effect.
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
        // `.mdx` swaps exactly as `.mdl` does — the table uses both.
        assert_eq!(tables.model(effect.models[1]), "World\\NoDXT\\Detail\\Fern.m2");
    }

    /// **Two ways of naming nothing, and both are dropped.** A row with density
    /// 0 plants nothing however many models it names, and a row whose doodad
    /// ids are in no `GroundEffectDoodad` names nothing however dense it says
    /// it is. Neither may come back as an effect with an empty model list,
    /// which would be a cell that costs a lookup and draws nothing.
    #[test]
    fn a_row_that_plants_nothing_is_not_an_effect() {
        let tables = tables();
        assert!(tables.effect(102).is_none(), "density 0 plants nothing");
        assert!(tables.effect(103).is_none(), "no resolvable doodad");
        assert!(tables.effect(0).is_none(), "id 0 is not a row");
        assert!(tables.effect(999).is_none());
        assert_eq!(tables.effect_count(), 2);
    }

    /// **A repeated id is a weight and is kept.** Row 101 names one model four
    /// times, so every draw on it must come out as that model — deduplicating
    /// the slots would silently reweight every row in the table that uses the
    /// idiom, which is 260 of the 730.
    #[test]
    fn a_repeated_doodad_is_a_weight_and_not_a_duplicate() {
        let tables = tables();
        let effect = tables.effect(101).expect("row 101 is an effect");
        assert_eq!(effect.choices, 4);
        for pick in 0..16 {
            assert_eq!(tables.model(effect.model(pick)), "World\\NoDXT\\Detail\\ElwGra01.m2");
        }
    }

    /// The draw is reduced modulo the choices the row really made, so a caller
    /// hands in a hash and never reads slot 2 of a two-model row — which would
    /// be model index 0, i.e. the wrong plant, silently, on half of every cell.
    #[test]
    fn a_draw_only_ever_lands_on_a_slot_the_row_named() {
        let tables = tables();
        let effect = tables.effect(100).expect("row 100");
        let mut seen = [false; 2];
        for pick in 0..64 {
            let model = effect.model(pick);
            let slot = effect.models.iter().position(|&m| m == model).unwrap();
            assert!(slot < 2, "draw landed on unnamed slot {slot}");
            seen[slot] = true;
        }
        assert_eq!(seen, [true, true], "both named slots are reachable");
    }

    /// **An absent table is bare ground, not a failure.** Neither DBC is
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
