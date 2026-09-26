//! **The ground's own foliage** — which of a chunk's 64 detail cells plants
//! something, and where each tuft of it stands.
//!
//! This is the *placement* half of the ground effects; the table join is
//! [`crate::tables::foliage`], which turns an `MCLY` `effectId` into up to four
//! models and a density. What is here is everything between that and a
//! position: which cell takes which texture layer, which cells the file says to
//! leave bare, how many tufts a cell gets and where in it they stand.
//!
//! ## Three inputs, all of them in the MCNK header
//!
//! ```text
//! 0x40  detail_layer[8]   which of the 4 texture layers wins in each cell
//! 0x50  no_detail         one bit per cell: grow nothing here
//! MCLY  effect_id         …and what that layer plants
//! ```
//!
//! A chunk is 33.3 yards and its detail grid is **8x8**, so a detail cell is
//! exactly one terrain cell — [`UNIT_SIZE`], 4.167 yards — which is why the
//! height under a tuft comes off the same four-triangle wedge a character walks
//! on ([`ChunkGround::height_at`]) rather than off a second interpolation.
//!
//! On `Azeroth_34_51`, of 16,384 cells: **6,560 suppressed** by `no_detail`,
//! 2,394 whose layer names an `effectId` that plants nothing, and **7,430 that
//! grow** — every one of them at density 8, from six models sharing one
//! texture. That last number is what makes the renderer's merge worth doing:
//! six models over a whole chunk, and one material.
//!
//! ## What is measured here and what is not
//!
//! **Measured**: the three fields above and their packing (see
//! [`Mcnk::detail_layer_at`], whose doc carries the check that fixes the bit
//! order), the cell size, and the ground the tuft stands on.
//!
//! **Not measured**: everything about the *arrangement* inside a cell. The
//! client's own placement rule was not checked, so the scatter is this
//! module's own: a hashed position uniform in the cell, a hashed yaw, a hashed
//! draw among the row's four slots, and **no size variation at all**. Each of
//! those is a guess in a different direction:
//!
//! * the **yaw** is nearly free to be wrong (a tuft has no front) and is very
//!   visibly wrong if omitted — 64 identical blades facing north is not
//!   something any client has ever drawn;
//! * the **scatter** is uniform rather than jittered off a sub-grid, which is
//!   the difference between a lawn that clumps and one that does not;
//! * the **scale** is left at 1.0 deliberately, because inventing a range would
//!   be inventing a number that is easy to state and impossible to check.
//!
//! **And two things here are for a renderer 5875 does not have.** Each tuft
//! carries the *ground's* shading normal ([`FoliageInstance::normal`]) because
//! the models' own normals are flat face normals that cannot light a two-sided
//! card, and a [`FoliageInstance::phase`] because this client sways its grass
//! and the reference does not. Both are marked where they are declared, and the
//! shader's own `wind.wgsl` carries the measurement that says 5875 stands
//! still.
//!
//! What is *not* a guess is that the arrangement is **stable**: it is a hash of
//! the chunk's own world position and the cell, so the same ground grows the
//! same grass every session, on every machine, whichever direction it was
//! walked into from. A renderer that generates a chunk's foliage lazily — which
//! is the only way to afford it, see `render::foliage` — cannot have grass that
//! moves when you look away.

use crate::tables::foliage::GroundEffects;
use crate::world::adt::{inner_index, ChunkGround, Mcnk, ALPHA_SIDE, INNER_SIDE, UNIT_SIZE};

/// The detail grid is the terrain grid: 8x8 cells of [`UNIT_SIZE`] yards.
pub const CELLS: usize = INNER_SIDE;

/// …so 64 of them per chunk.
pub const CELLS_PER_CHUNK: usize = CELLS * CELLS;

/// One planted doodad, in the file's own axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoliageInstance {
    /// World position, standing on the ground the character walks on.
    pub position: [f32; 3],
    /// Rotation about world `+Z`, radians. See the module doc: hashed, because
    /// the alternative is 64 identical blades all facing north.
    pub yaw: f32,
    /// **The ground's own shading normal under this tuft**, world axes — the
    /// `MCNR` normal at the centre of the cell it grows in, which is the same
    /// normal the terrain mesh is lit by there.
    ///
    /// It is here rather than left to the model because the models' own
    /// normals cannot light them: a tuft is two crossed cards and the tool that
    /// made them wrote **face** normals, so they lie in the ground plane
    /// (measured: `DskGra05`'s are exactly `(0,-1,0)` and `(1,0,0)`, and every
    /// grass model's `normal.z` is within 0.04 of zero — only the *rock*
    /// models have real ones). Lit literally, a two-sided card is lit on at
    /// most one side and every tuft's brightness becomes a lottery on its own
    /// yaw.
    pub normal: [f32; 3],
    /// A per-tuft phase for whatever animates it, 0..τ, hashed like the yaw.
    ///
    /// **Nothing in 1.12 reads this** — see the module doc — and it is produced
    /// here anyway because it is a property of the *placement* and belongs
    /// beside the yaw it is drawn with, not in the renderer that happens to
    /// sway with it.
    pub phase: f32,
    /// Index into [`GroundEffects::models`].
    pub model: u16,
    /// Whether the ground under it is inside the chunk's baked `MCSH` shadow —
    /// the bit that picks the instance's sun scale, exactly as it does for an
    /// `MDDF` doodad. Sampled per **cell** rather than per tuft: a cell is 4.2
    /// yards and the map is 64x64 per chunk, so this costs one sample per cell
    /// instead of one per instance and cannot differ by more than half a cell.
    pub shadowed: bool,
}

/// What one chunk plants, as a **plan** rather than as positions.
///
/// The distinction is the whole design. A tile's 256 chunks grow tens of
/// thousands of tufts and the camera can see a few hundred of them, so the
/// positions are generated when a chunk comes into range and dropped when it
/// leaves — see `render::foliage`. What has to survive the tile parse is only
/// this: the cell grid, the shadow bits, and the heights to stand on.
///
/// **About 920 bytes.** 580 of them are [`ChunkGround`]'s 145 floats, 256 the
/// cell ids, 192 the per-cell ground normals and 8 the shadow word. A whole 3x3
/// of tiles is a couple of megabytes, against the ~12 MB the instances
/// themselves would be.
#[derive(Debug, Clone)]
pub struct ChunkFoliage {
    /// The height field to stand things on, and the holes not to.
    pub ground: ChunkGround,
    /// Each detail cell's `GroundEffectTexture` id, **0 where nothing grows** —
    /// suppressed by the header, over a layer the chunk does not have, or over
    /// a texture with no ground effect at all. Row-major, `row * 8 + col`.
    pub cells: [u32; CELLS_PER_CHUNK],
    /// `MCSH` at each cell's centre, bit `row * 8 + col`. All zero on a chunk
    /// that declares no shadow, which is most open ground.
    pub shadow: u64,
    /// **The ground's shading normal at each cell's centre**, packed as the
    /// file's own signed bytes — see [`FoliageInstance::normal`], which is
    /// what it becomes.
    ///
    /// Per *cell* and not per tuft, which is exact rather than an
    /// approximation: a detail cell's centre is exactly an `MCNR` *inner*
    /// sample, so this is [`Mcnk::inner_normal`] read straight off, with no
    /// interpolation to disagree with the ground about. 192 bytes a chunk.
    pub normals: [[i8; 3]; CELLS_PER_CHUNK],
}

impl ChunkFoliage {
    /// The plan for one chunk, or `None` when nothing at all grows on it.
    ///
    /// `None` rather than an empty plan on purpose: three of `Azeroth_34_51`'s
    /// 256 chunks grow nothing, and a city tile is mostly such chunks — a plan
    /// that exists is 730 bytes and a walk over 64 cells every time the camera
    /// crosses it.
    pub fn plan(chunk: &Mcnk, effects: &GroundEffects) -> Option<ChunkFoliage> {
        if effects.is_empty() {
            return None;
        }
        let mut cells = [0u32; CELLS_PER_CHUNK];
        let mut normals = [[0i8, 0, 127]; CELLS_PER_CHUNK];
        let mut shadow = 0u64;
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
                if shadow_at_cell(chunk, row, col) {
                    shadow |= 1 << (row * CELLS + col);
                }
                // A detail cell's centre *is* an `MCNR` inner sample, so this
                // is the ground's own shading normal there and not a
                // resampling of it.
                let n = chunk.inner_normal(row, col);
                normals[row * CELLS + col] =
                    [pack_normal(n[0]), pack_normal(n[1]), pack_normal(n[2])];
            }
        }
        any.then(|| ChunkFoliage {
            ground: chunk.ground(),
            cells,
            shadow,
            normals,
        })
    }

    /// **Put a plan back on ground that has moved**, without re-planning it.
    ///
    /// A height edit changes where a tuft stands and how it is lit; it does not
    /// change whether one grows. The cells, the models and the `MCSH` bits are
    /// therefore untouched and only the two fields the heights decide are
    /// rewritten — which is also what keeps the lawn *identical* across the
    /// edit, since [`Self::grow`] hashes the chunk's world position and nothing
    /// else. A re-plan would draw the same tufts anyway and cost a `MCLY` walk
    /// and a `GroundEffects` lookup per cell.
    ///
    /// `normals` is the chunk's `MCNR` in the file's own order, all 145 of
    /// them; the inner ones are taken out of it by [`inner_index`], because a
    /// detail cell's centre is exactly an inner sample. A short slice leaves
    /// the cell it could not reach lit as it was.
    ///
    /// **Nothing in this crate calls it** — the parse produces a plan once and
    /// a shipped chunk's heights never move. It is here for a caller that has
    /// moved them under a tile that is already built.
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

    /// Every tuft this chunk grows, appended to `out`.
    ///
    /// Deterministic: the same chunk on the same tile answers identically every
    /// call, on any machine, because the draw is a hash of the chunk's world
    /// position and nothing else. Takes the sink rather than returning a `Vec`
    /// so that a caller building a whole chunk's merged mesh allocates once.
    pub fn grow(&self, effects: &GroundEffects, out: &mut Vec<FoliageInstance>) {
        // The chunk's own place in the world, as the seed. Its position is an
        // exact multiple of the chunk size in every shipped tile; rounding
        // rather than truncating keeps a file that is a float epsilon off from
        // seeding a different lawn.
        let kx = (self.ground.position[0] / (UNIT_SIZE * CELLS as f32)).round() as i64;
        let ky = (self.ground.position[1] / (UNIT_SIZE * CELLS as f32)).round() as i64;
        let chunk_seed = (kx as u64 & 0xffff) << 48 | (ky as u64 & 0xffff) << 32;

        for row in 0..CELLS {
            for col in 0..CELLS {
                let effect_id = self.cells[row * CELLS + col];
                if effect_id == 0 {
                    continue;
                }
                let Some(effect) = effects.effect(effect_id) else {
                    continue;
                };
                let shadowed = self.shadow >> (row * CELLS + col) & 1 != 0;
                let packed = self.normals[row * CELLS + col];
                let normal = unpack_normal(packed);
                let cell_seed =
                    chunk_seed | (row as u64) << 28 | (col as u64) << 24;
                for n in 0..u32::from(effect.density) {
                    let bits = mix(cell_seed | u64::from(n));
                    // Four independent draws off one hash. Splitmix's output
                    // bits are all avalanched, so slicing is as good as four
                    // calls and costs a quarter as much.
                    let u = unit(bits) ;
                    let v = unit(bits >> 16);
                    let yaw = unit(bits >> 32) * std::f32::consts::TAU;

                    // Cells run along *decreasing* world x and y from the
                    // chunk origin — the same direction the mesh, the alpha
                    // maps and the shadow map all run.
                    let x = self.ground.position[0] - (row as f32 + u) * UNIT_SIZE;
                    let y = self.ground.position[1] - (col as f32 + v) * UNIT_SIZE;
                    // A hole is ground that is not drawn, so nothing grows over
                    // one — the same refusal a character's footing gets.
                    let Some(z) = self.ground.height_at(x, y) else {
                        continue;
                    };
                    out.push(FoliageInstance {
                        position: [x, y, z],
                        yaw,
                        normal,
                        // A second, independent draw off the same hash: a tuft
                        // whose phase tracked its yaw would sway in step with
                        // whichever way it happened to face.
                        phase: unit(bits >> 48) * std::f32::consts::TAU,
                        model: effect.model((bits >> 56) as u32),
                        shadowed,
                    });
                }
            }
        }
    }

    /// How many tufts [`Self::grow`] will produce, without producing them.
    ///
    /// An upper bound rather than the exact count — it does not test the holes,
    /// which `grow` does — and it is what `vale foliage` reports per tile so
    /// that the population can be checked against the archives without building
    /// a single vertex.
    pub fn count(&self, effects: &GroundEffects) -> usize {
        self.cells
            .iter()
            .filter_map(|&id| effects.effect(id))
            .map(|effect| usize::from(effect.density))
            .sum()
    }

    /// How many of the 64 cells grow anything.
    pub fn planted_cells(&self) -> usize {
        self.cells.iter().filter(|&&id| id != 0).count()
    }
}

/// `MCSH` at the centre of one detail cell.
///
/// The texel derivation is [`crate::world::adt::Adt::shadowed_at`]'s: the map
/// is 64x64 row-major with the column minor, and both axes run along decreasing
/// world coordinates — so a detail cell is exactly 8x8 texels of it and its
/// centre is texel 4.
fn shadow_at_cell(chunk: &Mcnk, row: usize, col: usize) -> bool {
    let per_cell = ALPHA_SIDE / CELLS;
    let texel_row = row * per_cell + per_cell / 2;
    let texel_col = col * per_cell + per_cell / 2;
    chunk
        .shadow
        .get(texel_row * ALPHA_SIDE + texel_col)
        .copied()
        .unwrap_or(0)
        != 0
}

/// `MCNR`'s own encoding: a unit component as a signed byte over 127.
fn pack_normal(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

/// …and back, renormalised — the rounding above costs up to half a byte per
/// component and a normal that is not unit shades a shade too bright or too
/// dark.
fn unpack_normal(n: [i8; 3]) -> [f32; 3] {
    let v = [f32::from(n[0]), f32::from(n[1]), f32::from(n[2])];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-6 {
        return [0.0, 0.0, 1.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

/// SplitMix64's finalizer — the whole of the scatter's randomness.
///
/// An integer hash rather than a PRNG because the draw has to be a *function*
/// of where the tuft is: a chunk generated, dropped and generated again as the
/// camera walks past it twice must come back identical, and a sequence cannot
/// promise that without being stored.
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
    use crate::world::adt::{TextureLayer, CHUNK_SIZE, HEIGHTS_PER_CHUNK};

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

    /// One doodad, one texture row of density 2 on `effectId` 100.
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
        let textures = dbc(&[vec![100, 7, u32::MAX, u32::MAX, u32::MAX, 2, 0]], &[0]);
        GroundEffects::load(|table| match table {
            "GroundEffectDoodad" => Some(doodads.clone()),
            "GroundEffectTexture" => Some(textures.clone()),
            _ => None,
        })
    }

    fn grown(chunk: &Mcnk) -> Vec<FoliageInstance> {
        let effects = effects();
        let mut out = Vec::new();
        if let Some(plan) = ChunkFoliage::plan(chunk, &effects) {
            plan.grow(&effects, &mut out);
        }
        out
    }

    /// **Every tuft lands inside the chunk it was planted on, on its ground.**
    /// The cell axes run along *decreasing* world x and y, which is the one
    /// thing here that is easy to get backwards — and getting it backwards
    /// grows a chunk's grass on its neighbour, which looks like nothing at all
    /// until a tile at the edge of the loaded world has bald patches.
    #[test]
    fn a_chunk_plants_inside_itself_and_on_the_ground() {
        let chunk = chunk(100, 42.0);
        let grown = grown(&chunk);
        assert_eq!(grown.len(), 64 * 2, "64 cells at density 2");
        for tuft in &grown {
            let dx = chunk.position[0] - tuft.position[0];
            let dy = chunk.position[1] - tuft.position[1];
            assert!((0.0..CHUNK_SIZE).contains(&dx), "x {dx} outside the chunk");
            assert!((0.0..CHUNK_SIZE).contains(&dy), "y {dy} outside the chunk");
            assert!((tuft.position[2] - 42.0).abs() < 1e-3, "off the ground");
            assert!((0.0..std::f32::consts::TAU).contains(&tuft.yaw));
        }
    }

    /// **Regrounding moves the lawn and leaves it otherwise identical.** This is
    /// what a terrain brush needs from the plan: a tuft that was standing on the
    /// old ground stands on the new one, at the same place in the chunk, from
    /// the same model, with the same yaw. Anything else and the grass
    /// rearranges itself every time the brush is released.
    #[test]
    fn regrounding_a_plan_lifts_every_tuft_and_changes_nothing_else() {
        let effects = effects();
        let low = chunk(100, 10.0);
        let mut plan = ChunkFoliage::plan(&low, &effects).expect("the chunk plants");
        let before: Vec<FoliageInstance> = {
            let mut out = Vec::new();
            plan.grow(&effects, &mut out);
            out
        };

        let mut high = low.clone();
        high.heights = vec![15.0; HEIGHTS_PER_CHUNK];
        plan.reground(high.ground(), &high.normals);
        let mut after = Vec::new();
        plan.grow(&effects, &mut after);

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
            assert_eq!(was.model, now.model);
            assert_eq!(was.shadowed, now.shadowed);
        }
    }

    /// …and the per-cell shading normal comes with it. A chunk regrounded onto a
    /// slope whose `MCNR` was left behind is a hill lit from the wrong side: the
    /// shape is new and the light on it is the old shape's.
    #[test]
    fn regrounding_takes_the_new_shading_normals() {
        let effects = effects();
        let flat = chunk(100, 0.0);
        let mut plan = ChunkFoliage::plan(&flat, &effects).expect("the chunk plants");

        let mut tilted = flat.clone();
        // Leaning a little way along +x, in the file's own order.
        tilted.normals = vec![[0.6, 0.0, 0.8]; HEIGHTS_PER_CHUNK];
        plan.reground(tilted.ground(), &tilted.normals);

        let mut grown = Vec::new();
        plan.grow(&effects, &mut grown);
        assert!(!grown.is_empty());
        for tuft in &grown {
            assert!((tuft.normal[0] - 0.6).abs() < 0.02, "{:?}", tuft.normal);
            assert!((tuft.normal[2] - 0.8).abs() < 0.02, "{:?}", tuft.normal);
        }
    }

    /// **The scatter is a function of the place and not of a sequence.** A
    /// chunk generated twice — which is what walking past it twice does — must
    /// come back identical, or the grass moves whenever the camera looks away.
    #[test]
    fn the_same_ground_grows_the_same_grass_every_time() {
        let chunk = chunk(100, 0.0);
        assert_eq!(grown(&chunk), grown(&chunk));
    }

    /// …and two *different* chunks do not grow the same lawn, which is the
    /// other half: a seed that ignored the chunk would tile one cell's scatter
    /// across the whole world in a visible 33-yard grid.
    #[test]
    fn two_chunks_do_not_grow_the_same_lawn() {
        let mut far = chunk(100, 0.0);
        far.position = [CHUNK_SIZE * 9.0, CHUNK_SIZE * 4.0, 0.0];
        let here = grown(&chunk(100, 0.0));
        let there = grown(&far);
        assert_eq!(here.len(), there.len());
        // Compare the *offsets within the chunk*, so this fails on a shared
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
            "every chunk in the world grows the same 64 tufts"
        );
    }

    /// **The header's three refusals, each on its own.** A suppressed cell, a
    /// layer the chunk does not have and a texture with no ground effect all
    /// mean bare ground — and the first of them is what keeps grass out of the
    /// roads, which is 40% of a shipped tile's cells.
    #[test]
    fn the_header_can_refuse_a_cell_three_ways() {
        // Suppressed: the whole top row, bits 0..8.
        let mut suppressed = chunk(100, 0.0);
        suppressed.no_detail = 0xff;
        assert_eq!(grown(&suppressed).len(), 56 * 2, "the top row is bare");

        // A 2-bit field can always name layer 3; a chunk can always have one
        // layer. Naming a layer that is not there grows nothing rather than
        // panicking or wrapping round to layer 0.
        let mut absent = chunk(100, 0.0);
        absent.detail_layer = [0xffff; 8];
        assert!(grown(&absent).is_empty(), "layer 3 of a one-layer chunk");

        // …and a texture whose `effectId` is in no `GroundEffectTexture` row.
        assert!(grown(&chunk(999, 0.0)).is_empty(), "no ground effect row");
        assert!(grown(&chunk(0, 0.0)).is_empty(), "effectId 0 plants nothing");
    }

    /// Nothing grows over a hole — the same refusal the character's footing
    /// gets from [`ChunkGround::height_at`], and for the same reason: there is
    /// no ground there to stand on.
    #[test]
    fn nothing_grows_over_a_hole() {
        let mut holed = chunk(100, 0.0);
        // Bit 0 of the 4x4 mask is the 2x2 block of cells at (0..2, 0..2).
        holed.holes = 1;
        assert_eq!(grown(&holed).len(), 60 * 2, "four cells are a hole");
    }

    /// The plan is `None` — not an empty plan — for a chunk that grows nothing,
    /// which is what keeps a city tile from carrying 256 dead 730-byte plans
    /// and walking 16,384 empty cells on every crossing.
    #[test]
    fn a_barren_chunk_has_no_plan_at_all() {
        assert!(ChunkFoliage::plan(&chunk(999, 0.0), &effects()).is_none());
        // …and neither does anything, when the tables did not load.
        let empty = GroundEffects::load(|_| None);
        assert!(ChunkFoliage::plan(&chunk(100, 0.0), &empty).is_none());
    }

    /// **The shadow is read per cell and lands on the tufts in it.** A tuft's
    /// sun scale comes off this bit, so a whole chunk reading "lit" under a
    /// building is the same visible fault a tree taking full sun in its shade
    /// was.
    #[test]
    fn a_cell_in_the_baked_shadow_plants_shadowed_tufts() {
        let mut shaded = chunk(100, 0.0);
        // The whole map dark: every cell centre samples 255.
        shaded.shadow = vec![255u8; ALPHA_SIDE * ALPHA_SIDE];
        assert!(grown(&shaded).iter().all(|t| t.shadowed));
        // …and the default chunk declares no shadow at all, which is lit.
        assert!(grown(&chunk(100, 0.0)).iter().all(|t| !t.shadowed));
    }

    /// The count the CLI reports is the count `grow` produces, on ground with
    /// no holes in it — the two must not drift, because the check is the only
    /// thing that says the population is what the file asked for.
    #[test]
    fn the_reported_count_is_the_grown_count() {
        let effects = effects();
        let chunk = chunk(100, 0.0);
        let plan = ChunkFoliage::plan(&chunk, &effects).expect("plants");
        let mut out = Vec::new();
        plan.grow(&effects, &mut out);
        assert_eq!(plan.count(&effects), out.len());
        assert_eq!(plan.planted_cells(), 64);
    }
}
