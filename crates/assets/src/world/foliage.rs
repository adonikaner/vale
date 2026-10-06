//! The ground's own foliage: which of a chunk's 64 detail cells plant
//! something, and where each tuft stands.
//!
//! This is the placement half of the ground effects. The table join is
//! [`crate::tables::foliage`], which turns an `MCLY` `effectId` into up to four
//! models and a density. This module goes from there to positions: which cell
//! takes which texture layer, which cells the file leaves bare, which cells
//! are drawn and where in them each tuft stands.
//!
//! ## Three inputs, all of them in the MCNK header
//!
//! ```text
//! 0x40  detail_layer[8]   which of the 4 texture layers wins in each cell
//! 0x50  no_detail         one bit per cell: grow nothing here
//! MCLY  effect_id         and what that layer plants
//! ```
//!
//! A chunk is 33.3 yards and its detail grid is 8x8, so a detail cell is one
//! terrain cell, [`UNIT_SIZE`] (4.167 yards). The height under a tuft comes off
//! the same four-triangle wedge a character walks on
//! ([`ChunkGround::height_at`]).
//!
//! On `Azeroth_34_51`, of 16,384 cells: 6,560 are suppressed by `no_detail`,
//! 2,394 take a layer whose `effectId` plants nothing, and 7,430 grow, every
//! one at density 8, from six models sharing one texture.
//!
//! ## The planting rule
//!
//! The 1.12.1 client plants a chunk by draws. It draws `frillDensity` cells
//! ([`DEFAULT_FRILL_DENSITY`] unless `Config.wtf` says otherwise), each a
//! random row and column, repeats allowed. A draw on a cell that grows nothing
//! (suppressed, a hole, or a layer with no ground effect) plants nothing.
//! Otherwise it plants the row's density of tufts in that cell, each with:
//!
//! * a position uniform in the cell;
//! * the model in the row's column `(draw + tuft) & 3`, and no tuft when that
//!   column is empty;
//! * a scale of `1.0 ± 0.1`, uniform;
//! * a yaw uniform over a full turn;
//! * the ground's baked `MCSH` shadow under the tuft itself.
//!
//! So a chunk grows at most `frillDensity x density` tufts, clumped in the
//! cells that were drawn: 128 at the default and density 8.
//!
//! The random draws are this module's own hash of the chunk's world position,
//! not the client's generator, so the cells drawn and the positions inside
//! them differ from the reference while the counts, the clumping and the
//! ranges are the same. The hash makes the lawn stable: the same ground grows
//! the same grass every session, whichever direction it was approached from,
//! which a renderer that builds a chunk's foliage when it comes into range and
//! drops it when it leaves (`render::foliage`) needs.
//!
//! ## Two additions the 1.12.1 client does not have
//!
//! Each tuft carries the ground's shading normal ([`FoliageInstance::normal`]),
//! because the models' own normals are flat face normals that cannot light a
//! two-sided card, and a [`FoliageInstance::phase`], because this client sways
//! its grass and the reference does not. The shader's `wind.wgsl` states that
//! 5875 does not sway it.

use crate::tables::foliage::GroundEffects;
use crate::world::adt::{
    inner_index, ChunkGround, Mcnk, ALPHA_SIDE, CHUNK_SIZE, INNER_SIDE, UNIT_SIZE,
};

/// The detail grid is the terrain grid: 8x8 cells of [`UNIT_SIZE`] yards.
pub const CELLS: usize = INNER_SIDE;

/// …so 64 of them per chunk.
pub const CELLS_PER_CHUNK: usize = CELLS * CELLS;

/// How many cells a chunk draws when `frillDensity` is not set: the value the
/// 1.12.1 client registers the CVar with.
pub const DEFAULT_FRILL_DENSITY: u32 = 16;

/// The range the 1.12.1 client accepts for `frillDensity`. It refuses a value
/// outside it with "Frill Density must be in range 1 - 256."
pub const FRILL_DENSITY_RANGE: std::ops::RangeInclusive<u32> = 1..=256;

/// How far a tuft's scale may be from 1.0, either way.
pub const SCALE_SPREAD: f32 = 0.1;

/// One planted doodad, in the file's own axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoliageInstance {
    /// World position, standing on the ground the character walks on.
    pub position: [f32; 3],
    /// Rotation about world `+Z`, radians, 0..τ.
    pub yaw: f32,
    /// The model's scale, `1.0 ± SCALE_SPREAD`.
    pub scale: f32,
    /// The ground's shading normal under this tuft, in world axes: the `MCNR`
    /// normal at the centre of the cell it grows in, which is the normal the
    /// terrain mesh is lit by there.
    ///
    /// The models' own normals cannot light them. A tuft is two crossed cards
    /// and its normals are the cards' face normals, which lie in the ground
    /// plane: `DskGra05`'s are exactly `(0,-1,0)` and `(1,0,0)`, and every
    /// grass model's `normal.z` is within 0.04 of zero (only the rock models
    /// have real ones). Lit with those, a two-sided card is lit on at most one
    /// side and each tuft's brightness depends on its yaw.
    pub normal: [f32; 3],
    /// A per-tuft phase for the sway, 0..τ. The 1.12.1 client does not sway
    /// its grass; the value is produced here because it is a property of the
    /// placement, drawn with the yaw.
    pub phase: f32,
    /// Index into [`GroundEffects::models`].
    pub model: u16,
    /// Whether the ground under the tuft is in the chunk's baked `MCSH`
    /// shadow, which picks the instance's sun scale as it does for an `MDDF`
    /// doodad.
    pub shadowed: bool,
}

/// What one chunk plants, as a plan rather than as positions.
///
/// A tile's 256 chunks grow tens of thousands of tufts and the camera sees a
/// few hundred of them, so the positions are generated when a chunk comes
/// into range and dropped when it leaves (see `render::foliage`). The plan is
/// what has to survive the tile parse: the cell grid, the shadow map, the
/// normals and the heights to stand on.
///
/// About 1.5 KB: 580 bytes of [`ChunkGround`]'s 145 floats, 256 of cell ids,
/// 512 of shadow map and 192 of per-cell normals.
#[derive(Debug, Clone)]
pub struct ChunkFoliage {
    /// The height field to stand things on, and the holes not to.
    pub ground: ChunkGround,
    /// Each detail cell's `GroundEffectTexture` id, 0 where nothing grows:
    /// suppressed by the header, over a layer the chunk does not have, or over
    /// a texture with no ground effect. Row-major, `row * 8 + col`.
    pub cells: [u32; CELLS_PER_CHUNK],
    /// The chunk's `MCSH` map, one `u64` per texel row with bit `col` set
    /// where the texel is shadowed. All zero on a chunk that declares no
    /// shadow, which is most open ground.
    pub shadow: [u64; ALPHA_SIDE],
    /// The ground's shading normal at each cell's centre, packed as the
    /// file's own signed bytes; see [`FoliageInstance::normal`].
    ///
    /// Per cell rather than per tuft, and exact: a detail cell's centre is an
    /// `MCNR` inner sample, so this is [`Mcnk::inner_normal`] with no
    /// interpolation.
    pub normals: [[i8; 3]; CELLS_PER_CHUNK],
}

impl ChunkFoliage {
    /// The plan for one chunk, or `None` when nothing grows on it.
    ///
    /// `None` rather than an empty plan, because a city tile is mostly such
    /// chunks and an empty plan would still be stored and walked every time
    /// the camera crosses it.
    pub fn plan(chunk: &Mcnk, effects: &GroundEffects) -> Option<ChunkFoliage> {
        if effects.is_empty() {
            return None;
        }
        let mut cells = [0u32; CELLS_PER_CHUNK];
        let mut normals = [[0i8, 0, 127]; CELLS_PER_CHUNK];
        let mut any = false;
        for row in 0..CELLS {
            for col in 0..CELLS {
                let Some(layer) = chunk.detail_layer_at(row, col) else {
                    continue;
                };
                if effects.effect(layer.effect_id).is_none() {
                    continue;
                }
                cells[row * CELLS + col] = layer.effect_id;
                any = true;
                // A detail cell's centre is an `MCNR` inner sample, so this is
                // the ground's own shading normal there.
                let n = chunk.inner_normal(row, col);
                normals[row * CELLS + col] =
                    [pack_normal(n[0]), pack_normal(n[1]), pack_normal(n[2])];
            }
        }
        any.then(|| ChunkFoliage {
            ground: chunk.ground(),
            cells,
            shadow: shadow_rows(chunk),
            normals,
        })
    }

    /// Put a plan back on ground that has moved, without re-planning it.
    ///
    /// A height edit changes where a tuft stands and how it is lit, not
    /// whether one grows. The cells, the models and the `MCSH` map are left as
    /// they are and only the heights and normals are rewritten, which keeps
    /// the lawn identical across the edit, since [`Self::grow`] hashes the
    /// chunk's world position and nothing else.
    ///
    /// `normals` is the chunk's `MCNR` in the file's own order, all 145; the
    /// inner ones are taken by [`inner_index`]. A short slice leaves the cells
    /// it does not reach lit as they were.
    ///
    /// Nothing in this crate calls it: a shipped chunk's heights never move. It
    /// is for a caller that has moved them under a tile that is already built.
    pub fn reground(&mut self, ground: ChunkGround, normals: &[[f32; 3]]) {
        self.ground = ground;
        for row in 0..CELLS {
            for col in 0..CELLS {
                let Some(n) = normals.get(inner_index(row, col)) else {
                    continue;
                };
                self.normals[row * CELLS + col] =
                    [pack_normal(n[0]), pack_normal(n[1]), pack_normal(n[2])];
            }
        }
    }

    /// Every tuft this chunk grows with `frill_density` cell draws, appended
    /// to `out`. See the module doc for the rule.
    ///
    /// Deterministic: the same chunk answers identically every call, on any
    /// machine, because every draw is a hash of the chunk's world position.
    /// Takes the sink rather than returning a `Vec` so a caller building a
    /// whole chunk's merged mesh allocates once.
    pub fn grow(&self, effects: &GroundEffects, frill_density: u32, out: &mut Vec<FoliageInstance>) {
        let seed = self.seed();
        self.each_draw(effects, frill_density, |draw, row, col, effect| {
            let packed = self.normals[row * CELLS + col];
            let normal = unpack_normal(packed);
            for tuft in 0..u32::from(effect.density) {
                let Some(model) = effect.slot(draw + tuft) else {
                    continue;
                };
                let bits = mix(seed | u64::from(draw) << 8 | u64::from(tuft));
                let more = mix(bits);
                // Cells run along decreasing world x and y from the chunk
                // origin, as the mesh, the alpha maps and the shadow map do.
                let x = self.ground.position[0] - (row as f32 + unit(bits)) * UNIT_SIZE;
                let y = self.ground.position[1] - (col as f32 + unit(bits >> 16)) * UNIT_SIZE;
                // A hole is ground that is not drawn, so nothing grows over
                // one.
                let Some(z) = self.ground.height_at(x, y) else {
                    continue;
                };
                out.push(FoliageInstance {
                    position: [x, y, z],
                    yaw: unit(bits >> 32) * std::f32::consts::TAU,
                    scale: 1.0 + (unit(more) * 2.0 - 1.0) * SCALE_SPREAD,
                    normal,
                    // Independent of the yaw, so tufts facing one way do not
                    // sway in step.
                    phase: unit(bits >> 48) * std::f32::consts::TAU,
                    model,
                    shadowed: self.shadowed_at(x, y),
                });
            }
        });
    }

    /// How many tufts [`Self::grow`] produces with `frill_density` draws,
    /// without producing them.
    ///
    /// An upper bound: it does not test the holes, which `grow` does. `vale
    /// foliage` reports it per tile so the population can be checked against
    /// the archives without building a vertex.
    pub fn count(&self, effects: &GroundEffects, frill_density: u32) -> usize {
        let mut count = 0;
        self.each_draw(effects, frill_density, |draw, _, _, effect| {
            count += (0..u32::from(effect.density))
                .filter(|tuft| effect.slot(draw + tuft).is_some())
                .count();
        });
        count
    }

    /// How many of the 64 cells grow anything.
    pub fn planted_cells(&self) -> usize {
        self.cells.iter().filter(|&&id| id != 0).count()
    }

    /// The chunk's own place in the world as a seed. Its position is an exact
    /// multiple of the chunk size in every shipped tile; rounding rather than
    /// truncating keeps a position a float epsilon off from seeding a
    /// different lawn.
    fn seed(&self) -> u64 {
        let kx = (self.ground.position[0] / CHUNK_SIZE).round() as i64;
        let ky = (self.ground.position[1] / CHUNK_SIZE).round() as i64;
        (kx as u64 & 0xffff) << 48 | (ky as u64 & 0xffff) << 32
    }

    /// Call `plant(draw, row, col, effect)` for each of the `frill_density`
    /// draws that lands on a cell that grows something.
    fn each_draw(
        &self,
        effects: &GroundEffects,
        frill_density: u32,
        mut plant: impl FnMut(u32, usize, usize, &crate::tables::foliage::GroundEffect),
    ) {
        let seed = self.seed();
        let draws = frill_density.clamp(*FRILL_DENSITY_RANGE.start(), *FRILL_DENSITY_RANGE.end());
        for draw in 0..draws {
            // Bit 31 keeps the cell draws apart from the tuft draws, which
            // hash `draw << 8 | tuft` under the same seed.
            let bits = mix(seed | 1 << 31 | u64::from(draw));
            let row = (bits & 7) as usize;
            let col = (bits >> 3 & 7) as usize;
            let effect_id = self.cells[row * CELLS + col];
            if effect_id == 0 {
                continue;
            }
            if let Some(effect) = effects.effect(effect_id) {
                plant(draw, row, col, effect);
            }
        }
    }

    /// The `MCSH` bit under a world position inside this chunk. The texel
    /// derivation is [`crate::world::adt::Adt::shadowed_at`]'s.
    fn shadowed_at(&self, x: f32, y: f32) -> bool {
        let texel = |d: f32| (((d / CHUNK_SIZE) * ALPHA_SIDE as f32).max(0.0) as usize).min(ALPHA_SIDE - 1);
        let row = texel(self.ground.position[0] - x);
        let col = texel(self.ground.position[1] - y);
        self.shadow[row] >> col & 1 != 0
    }
}

/// The chunk's `MCSH` map packed one `u64` per texel row. The map is 64x64
/// row-major with the column minor, as [`crate::world::adt::Adt::shadowed_at`]
/// reads it; a chunk with no map is all lit.
fn shadow_rows(chunk: &Mcnk) -> [u64; ALPHA_SIDE] {
    let mut rows = [0u64; ALPHA_SIDE];
    for (i, &texel) in chunk.shadow.iter().enumerate().take(ALPHA_SIDE * ALPHA_SIDE) {
        if texel != 0 {
            rows[i / ALPHA_SIDE] |= 1 << (i % ALPHA_SIDE);
        }
    }
    rows
}

/// `MCNR`'s own encoding: a unit component as a signed byte over 127.
fn pack_normal(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

/// …and back, renormalised: the rounding costs up to half a byte per
/// component, and a normal that is not unit length shades too bright or too
/// dark.
fn unpack_normal(n: [i8; 3]) -> [f32; 3] {
    let v = [f32::from(n[0]), f32::from(n[1]), f32::from(n[2])];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-6 {
        return [0.0, 0.0, 1.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

/// SplitMix64's finalizer, the source of every draw.
///
/// An integer hash rather than a generator because each draw has to be a
/// function of where it is: a chunk generated, dropped and generated again as
/// the camera passes it twice must come back identical, and a sequence cannot
/// do that without being stored.
fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Sixteen bits of a hash as `0.0..1.0`.
fn unit(bits: u64) -> f32 {
    (bits & 0xffff) as f32 / 65536.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::adt::{TextureLayer, HEIGHTS_PER_CHUNK};

    /// A chunk whose every cell takes layer 0, at a stated height.
    fn chunk(effect_id: u32, height: f32) -> Mcnk {
        Mcnk {
            index_x: 0,
            index_y: 0,
            area_id: 0,
            flags: 0,
            holes: 0,
            position: [CHUNK_SIZE, CHUNK_SIZE, 0.0],
            heights: vec![height; HEIGHTS_PER_CHUNK],
            normals: Vec::new(),
            colours: Vec::new(),
            layers: vec![TextureLayer {
                texture_id: 0,
                flags: 0,
                alpha_offset: 0,
                effect_id,
            }],
            alphas: Vec::new(),
            shadow: Vec::new(),
            liquids: Vec::new(),
            detail_layer: [0; 8],
            no_detail: 0,
        }
    }

    /// One doodad. `effectId` 100 names it in all four columns at density 2;
    /// 101 names it in column 0 only, at density 4.
    fn effects() -> GroundEffects {
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
        let mut names = vec![0u8];
        names.extend_from_slice(b"Gra01.mdl\0");
        let doodads = dbc(&[vec![7, 0, 1]], &names);
        let textures = dbc(
            &[
                vec![100, 7, 7, 7, 7, 2, 0],
                vec![101, 7, u32::MAX, u32::MAX, u32::MAX, 4, 0],
            ],
            &[0],
        );
        GroundEffects::load(|table| match table {
            "GroundEffectDoodad" => Some(doodads.clone()),
            "GroundEffectTexture" => Some(textures.clone()),
            _ => None,
        })
    }

    fn grown_with(chunk: &Mcnk, frill_density: u32) -> Vec<FoliageInstance> {
        let effects = effects();
        let mut out = Vec::new();
        if let Some(plan) = ChunkFoliage::plan(chunk, &effects) {
            plan.grow(&effects, frill_density, &mut out);
        }
        out
    }

    fn grown(chunk: &Mcnk) -> Vec<FoliageInstance> {
        grown_with(chunk, DEFAULT_FRILL_DENSITY)
    }

    /// The cell a tuft stands in, as `(row, col)`.
    fn cell_of(chunk: &Mcnk, tuft: &FoliageInstance) -> (usize, usize) {
        let row = ((chunk.position[0] - tuft.position[0]) / UNIT_SIZE) as usize;
        let col = ((chunk.position[1] - tuft.position[1]) / UNIT_SIZE) as usize;
        (row, col)
    }

    /// Every tuft lands inside its chunk, on its ground. The cell axes run
    /// along decreasing world x and y; getting that backwards grows a chunk's
    /// grass on its neighbour.
    #[test]
    fn a_chunk_plants_inside_itself_and_on_the_ground() {
        let chunk = chunk(100, 42.0);
        let grown = grown(&chunk);
        assert_eq!(grown.len(), 16 * 2, "sixteen draws at density 2");
        for tuft in &grown {
            let dx = chunk.position[0] - tuft.position[0];
            let dy = chunk.position[1] - tuft.position[1];
            assert!((0.0..CHUNK_SIZE).contains(&dx), "x {dx} outside the chunk");
            assert!((0.0..CHUNK_SIZE).contains(&dy), "y {dy} outside the chunk");
            assert!((tuft.position[2] - 42.0).abs() < 1e-3, "off the ground");
            assert!((0.0..std::f32::consts::TAU).contains(&tuft.yaw));
        }
    }

    /// `frillDensity` is the number of cell draws, so it scales the count.
    #[test]
    fn the_frill_density_is_the_number_of_draws() {
        let chunk = chunk(100, 0.0);
        assert_eq!(grown_with(&chunk, 48).len(), 48 * 2);
        assert_eq!(grown_with(&chunk, 1).len(), 2);
        // Outside the client's range, the nearest end of it.
        assert_eq!(grown_with(&chunk, 0).len(), 2);
        assert_eq!(grown_with(&chunk, 1000).len(), 256 * 2);
    }

    /// One draw's tufts all stand in the drawn cell, so the lawn clumps.
    #[test]
    fn a_draws_tufts_share_one_cell() {
        let chunk = chunk(100, 0.0);
        let grown = grown(&chunk);
        for pair in grown.chunks(2) {
            assert_eq!(cell_of(&chunk, &pair[0]), cell_of(&chunk, &pair[1]));
        }
    }

    /// A tuft's model is the row's column `(draw + tuft) & 3`, and an empty
    /// column plants nothing: with only column 0 named and density 4, each
    /// draw plants exactly one tuft.
    #[test]
    fn an_empty_column_plants_nothing() {
        let chunk = chunk(101, 0.0);
        assert_eq!(grown(&chunk).len(), 16);
    }

    /// Scale varies by up to a tenth either way, and not every tuft is the
    /// same size.
    #[test]
    fn the_scale_varies_by_a_tenth() {
        let grown = grown_with(&chunk(100, 0.0), 64);
        assert!(grown.iter().all(|t| (0.9..=1.1).contains(&t.scale)));
        assert!(grown.iter().any(|t| (t.scale - grown[0].scale).abs() > 1e-3));
    }

    /// Regrounding moves the lawn and leaves it otherwise identical: the same
    /// place in the chunk, the same model, the same yaw and scale. Otherwise
    /// the grass rearranges itself every time a terrain brush is released.
    #[test]
    fn regrounding_a_plan_lifts_every_tuft_and_changes_nothing_else() {
        let effects = effects();
        let low = chunk(100, 10.0);
        let mut plan = ChunkFoliage::plan(&low, &effects).expect("the chunk plants");
        let mut before = Vec::new();
        plan.grow(&effects, DEFAULT_FRILL_DENSITY, &mut before);

        let mut high = low.clone();
        high.heights = vec![15.0; HEIGHTS_PER_CHUNK];
        plan.reground(high.ground(), &high.normals);
        let mut after = Vec::new();
        plan.grow(&effects, DEFAULT_FRILL_DENSITY, &mut after);

        assert_eq!(before.len(), after.len());
        for (was, now) in before.iter().zip(&after) {
            assert_eq!(was.position[0], now.position[0], "moved sideways");
            assert_eq!(was.position[1], now.position[1], "moved sideways");
            assert!(
                (now.position[2] - was.position[2] - 5.0).abs() < 1e-3,
                "{} should be five yards over {}",
                now.position[2],
                was.position[2]
            );
            assert_eq!(was.yaw, now.yaw, "a different lawn");
            assert_eq!(was.scale, now.scale);
            assert_eq!(was.model, now.model);
            assert_eq!(was.shadowed, now.shadowed);
        }
    }

    /// The per-cell shading normal moves with a regrounded plan. A chunk
    /// regrounded onto a slope with its old `MCNR` is a hill lit as the old
    /// shape was.
    #[test]
    fn regrounding_takes_the_new_shading_normals() {
        let effects = effects();
        let flat = chunk(100, 0.0);
        let mut plan = ChunkFoliage::plan(&flat, &effects).expect("the chunk plants");

        let mut tilted = flat.clone();
        // Leaning along +x, in the file's own order.
        tilted.normals = vec![[0.6, 0.0, 0.8]; HEIGHTS_PER_CHUNK];
        plan.reground(tilted.ground(), &tilted.normals);

        let mut grown = Vec::new();
        plan.grow(&effects, DEFAULT_FRILL_DENSITY, &mut grown);
        assert!(!grown.is_empty());
        for tuft in &grown {
            assert!((tuft.normal[0] - 0.6).abs() < 0.02, "{:?}", tuft.normal);
            assert!((tuft.normal[2] - 0.8).abs() < 0.02, "{:?}", tuft.normal);
        }
    }

    /// The draws are a function of the place, not of a sequence: a chunk
    /// generated twice comes back identical.
    #[test]
    fn the_same_ground_grows_the_same_grass_every_time() {
        let chunk = chunk(100, 0.0);
        assert_eq!(grown(&chunk), grown(&chunk));
    }

    /// Two different chunks do not grow the same lawn; a seed that ignored
    /// the chunk would repeat one pattern across the world in a 33-yard grid.
    #[test]
    fn two_chunks_do_not_grow_the_same_lawn() {
        let mut far = chunk(100, 0.0);
        far.position = [CHUNK_SIZE * 9.0, CHUNK_SIZE * 4.0, 0.0];
        let here = grown(&chunk(100, 0.0));
        let there = grown(&far);
        assert_eq!(here.len(), there.len());
        // Compare the offsets within the chunk, so this fails on a shared
        // pattern rather than on the two being in different places.
        let offsets = |chunk: &Mcnk, grown: &[FoliageInstance]| -> Vec<(i32, i32)> {
            grown
                .iter()
                .map(|t| {
                    (
                        ((chunk.position[0] - t.position[0]) * 100.0) as i32,
                        ((chunk.position[1] - t.position[1]) * 100.0) as i32,
                    )
                })
                .collect()
        };
        assert_ne!(
            offsets(&chunk(100, 0.0), &here),
            offsets(&far, &there),
            "every chunk in the world grows the same tufts"
        );
    }

    /// The header's three refusals: a suppressed cell, a layer the chunk does
    /// not have, and a texture with no ground effect all mean bare ground.
    /// The first keeps grass off the roads, which is 40% of a shipped tile's
    /// cells.
    #[test]
    fn the_header_can_refuse_a_cell_three_ways() {
        // Suppressed: the whole first row, bits 0..8.
        let mut suppressed = chunk(100, 0.0);
        suppressed.no_detail = 0xff;
        let grown_suppressed = grown_with(&suppressed, 256);
        assert!(!grown_suppressed.is_empty());
        assert!(grown_suppressed.iter().all(|t| cell_of(&suppressed, t).0 != 0));

        // A 2-bit field can name layer 3 of a one-layer chunk; that grows
        // nothing rather than panicking or wrapping round to layer 0.
        let mut absent = chunk(100, 0.0);
        absent.detail_layer = [0xffff; 8];
        assert!(grown(&absent).is_empty(), "layer 3 of a one-layer chunk");

        // A texture whose `effectId` is in no `GroundEffectTexture` row.
        assert!(grown(&chunk(999, 0.0)).is_empty(), "no ground effect row");
        assert!(grown(&chunk(0, 0.0)).is_empty(), "effectId 0 plants nothing");
    }

    /// Nothing grows over a hole: there is no ground to stand on.
    #[test]
    fn nothing_grows_over_a_hole() {
        let mut holed = chunk(100, 0.0);
        // Bit 0 of the 4x4 mask is the 2x2 block of cells at (0..2, 0..2).
        holed.holes = 1;
        let grown = grown_with(&holed, 256);
        assert!(!grown.is_empty());
        assert!(grown.iter().all(|t| {
            let (row, col) = cell_of(&holed, t);
            row >= 2 || col >= 2
        }));
    }

    /// A chunk that grows nothing has no plan, rather than an empty one.
    #[test]
    fn a_barren_chunk_has_no_plan_at_all() {
        assert!(ChunkFoliage::plan(&chunk(999, 0.0), &effects()).is_none());
        // Nor does anything when the tables did not load.
        let empty = GroundEffects::load(|_| None);
        assert!(ChunkFoliage::plan(&chunk(100, 0.0), &empty).is_none());
    }

    /// The shadow is read under each tuft. A tuft's sun scale comes off this
    /// bit, so a lawn reading lit in a building's shadow is the same fault as
    /// a tree taking full sun in its shade.
    #[test]
    fn a_tuft_takes_the_shadow_under_itself() {
        let mut shaded = chunk(100, 0.0);
        shaded.shadow = vec![255u8; ALPHA_SIDE * ALPHA_SIDE];
        assert!(grown(&shaded).iter().all(|t| t.shadowed));
        // The default chunk declares no shadow at all, which is lit.
        assert!(grown(&chunk(100, 0.0)).iter().all(|t| !t.shadowed));

        // Only the first half of the texel rows shadowed: a tuft is shadowed
        // exactly when it stands in that half of the chunk.
        let mut half = chunk(100, 0.0);
        half.shadow = (0..ALPHA_SIDE * ALPHA_SIDE)
            .map(|i| if i / ALPHA_SIDE < ALPHA_SIDE / 2 { 255 } else { 0 })
            .collect();
        let grown = grown_with(&half, 256);
        for tuft in &grown {
            let near = half.position[0] - tuft.position[0] < CHUNK_SIZE / 2.0;
            assert_eq!(tuft.shadowed, near, "{:?}", tuft.position);
        }
    }

    /// The count the CLI reports is the count `grow` produces on ground with
    /// no holes. The check is what says the population is what the file asked
    /// for, so the two must not drift.
    #[test]
    fn the_reported_count_is_the_grown_count() {
        let effects = effects();
        for id in [100, 101] {
            let chunk = chunk(id, 0.0);
            let plan = ChunkFoliage::plan(&chunk, &effects).expect("plants");
            for frill in [1, DEFAULT_FRILL_DENSITY, 48] {
                let mut out = Vec::new();
                plan.grow(&effects, frill, &mut out);
                assert_eq!(plan.count(&effects, frill), out.len());
            }
            assert_eq!(plan.planted_cells(), 64);
        }
    }
}
