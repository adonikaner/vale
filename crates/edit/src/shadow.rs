//! `MCSH`, recomputed: what the world standing on a chunk throws across it.
//!
//! ## This is the only shadow 1.12 has
//!
//! The reference client casts nothing at run time — no shadow map, no
//! projection, nothing that moves with the sun. The shadows anyone remembers
//! under Stormwind's walls are a 64x64 bitmap per chunk that Blizzard's tools
//! baked once, at the same resolution and texel centres as an alpha map, drawn
//! by darkening the sun term where the bit is set. `vale_assets::world::adt`
//! is where that is written up and where the bits are read.
//!
//! Which means an editor that moves ground or places a building and does not
//! touch `MCSH` leaves a shadow from the old world lying on the new one. It is
//! the one thing in a tile that no other edit invalidates *visibly*: a raised
//! hill keeps the shadow of the trees that used to stand beside it, and nothing
//! reports anything.
//!
//! ## The sun is fixed, and that is not a simplification
//!
//! [`vale_assets::tables::light::SUN_TOWARD`] is a measured constant —
//! azimuth 40°, elevation 40°, recovered from the shadows Blizzard baked by
//! `vale sun`, which cross-correlates predicted canopy shadows against the
//! real bitmap over five tiles. A bake has to agree with the bakes on the tiles
//! beside it or the seam is a step in the shading, so the direction is not a
//! setting: it is whatever the shipped tiles were baked under. It is an argument
//! here rather than a constant only so that a caller can re-bake a whole map
//! under a different sun if it ever wants to, and so that the tests can use one
//! they can reason about.
//!
//! ## Where the geometry comes from, and why not from here
//!
//! This crate opens no archives and builds no meshes — see the crate root. What
//! casts a shadow is a doodad's `M2` and a building's `WMO`, which live in the
//! MPQ chain, so the caller supplies the answer to the only question this needs:
//! **is there anything between this point and the sun?** [`Occluder`] is that
//! question and nothing else.
//!
//! The caller decides what is in it, and **what the shipped files were baked
//! from is doodads and buildings rather than the ground**: see [`Ground`], where
//! the measurement that says so is. `vale_ide` puts the doodads and
//! buildings of the tile *and its eight neighbours* in, cut to [`casting_box`],
//! and leaves the terrain out. The neighbours are not optional: a building is
//! recorded by the tile its origin stands on and a shadow does not stop at a
//! seam, so a bake of the tile's own placements alone shadows a harbour on the
//! tile it belongs to and on none of the tiles it stands across.

use crate::adt::file::{AdtFile, Region, SubChunk};
use crate::adt::header::at;
use crate::adt::heights;
use vale_assets::world::adt::{mcnk_flags, ALPHA_SIDE, CHUNKS_PER_SIDE, CHUNK_SIZE};

/// Bytes an `MCSH` comes to: 64x64 bits, one per texel.
pub const SHADOW_BYTES: usize = ALPHA_SIDE * ALPHA_SIDE / 8;

/// How far from the ground to look for something in the way, in yards.
///
/// A shadow is only as long as the thing casting it: at the baked sun's 40° of
/// elevation, a hundred-yard tower reaches about 120 yards and a fifteen-yard
/// tree about eighteen. Two hundred covers every doodad in the game and the
/// walls of every building, and it bounds the work — a ray is tested against
/// every hull whose box it crosses, so the reach is the search radius.
pub const REACH: f32 = 200.0;

/// How far above the ground a ray starts.
///
/// Without it, a bake whose occluder includes the terrain has every ray leave
/// from a point on a triangle of the terrain and immediately hit it, and the
/// whole tile comes out in shadow. A tenth of a yard is far below what a texel
/// resolves (a texel is 0.52 yards) and far above the float error in a ground
/// height.
pub const LIFT: f32 = 0.1;

/// **The box a tile's shadows are cast from**: the tile's own square widened
/// by `reach` on every side, at every height.
///
/// A ray leaves a texel of the tile and travels `reach` at most, so nothing
/// whose hull lies wholly outside this box can block one. It is what a caller
/// hands `vale_assets::world::collision::hulls_near` to keep the
/// neighbours' hulls to the ones that matter — and the neighbours *are*
/// wanted: a building is placed by the tile its origin is on and its walls
/// fall across the seam, so a bake fed only the tile's own file casts nothing
/// from a harbour that stands half on it. See that function's note.
pub fn casting_box(tile_x: u32, tile_y: u32, reach: f32) -> [[f32; 3]; 2] {
    use vale_assets::world::adt::TILE_SIZE;
    let centre = vale_assets::world::terrain::tile_centre(tile_x, tile_y);
    let half = TILE_SIZE / 2.0 + reach;
    [
        [centre[0] - half, centre[1] - half, f32::MIN],
        [centre[0] + half, centre[1] + half, f32::MAX],
    ]
}

/// **Is anything between this point and the sun?**
///
/// The one thing a bake needs and the one thing this crate cannot answer. The
/// caller builds it out of whatever it thinks casts a shadow — the tile's
/// doodads, its buildings, the neighbours' where they overhang, and the terrain
/// itself.
///
/// `from` is a point a little above the ground, `toward` is a unit vector at the
/// sun, and `max` is how far to look.
pub trait Occluder {
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool;
}

/// An occluder with nothing in it, which bakes an unshadowed tile.
///
/// Useful on its own: it is what *clears* a tile's shadows, and it is what a
/// caller that has not loaded any geometry yet should pass rather than skipping
/// the bake and leaving the old bits.
pub struct Nothing;

impl Occluder for Nothing {
    fn blocked(&self, _from: [f32; 3], _toward: [f32; 3], _max: f32) -> bool {
        false
    }
}

impl<F> Occluder for F
where
    F: Fn([f32; 3], [f32; 3], f32) -> bool,
{
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool {
        self(from, toward, max)
    }
}

/// **The tile's own ground, as something that casts — which the shipped bakes
/// appear not to have done.**
///
/// This was added on the reasoning that terrain must shadow terrain, since
/// `Azeroth_32_49` ships with 71% of its texels in shadow and no quantity of
/// trees accounts for that. `vale bake` then measured it against the shipped
/// files, both occluders over two tiles, and said otherwise:
///
/// ```text
///                                     32 49        34 51
/// shadow in the shipped file          70.9%        80.5%   <- the base rate
/// precision of hull-cast shadow       98.5%        98.3%
///   …as a multiple of the base rate   1.39x        1.22x
/// precision of what the ground adds   29.8%        72.7%
///   …as a multiple of the base rate   0.42x        0.90x
/// ```
///
/// A texel this marks at random would be right at the base rate. Hull-cast
/// shadow is well *above* it on both tiles, which is what says the hulls land
/// where the shipped shadow is. What the ground adds is at or below chance on
/// both — so there is no evidence terrain self-shadow is in `MCSH` at all, and
/// some evidence against.
///
/// That reading is consistent with what the client does: it already darkens a
/// slope through `MCNR`'s own normal, so baking the same term into `MCSH` would
/// darken it twice.
///
/// **So this is not in the default bake**, and a caller that wants it is asking
/// for something the reference tiles do not have. It is kept because the
/// measurement is worth being able to repeat, and because a map built for this
/// client rather than for 1.12 may well want it.
///
/// ## A height field is marched, not ray-traced
///
/// The ground is a function of x and y rather than a soup of triangles, so the
/// question *is anything between here and the sun* is answered by walking along
/// the ray and asking whether the ground is above it yet. That is one array
/// lookup per step against a triangle intersection per candidate, and at a
/// million texels a tile the difference is minutes.
///
/// It is sampled onto a regular grid once, at the alpha map's own resolution —
/// 1024 across a tile, a little over half a yard — because the alternative is a
/// chunk search per step. Building it is 1M height lookups and about a tenth of
/// a second.
///
/// ## What it does not have in it
///
/// **The neighbouring tiles.** A cliff on the tile next door throws a shadow
/// across this one and this knows nothing about it, so a bake is wrong in a band
/// along whichever edges have high ground beyond them. Fixing it means sampling
/// nine tiles into one field, which is nine times the memory and the same code;
/// it is not done because nothing has needed it yet, and it is written down here
/// because the seam is exactly where somebody will notice it first.
pub struct Ground {
    /// Samples across the tile, in both axes.
    side: usize,
    /// The tile's maximum corner, which is where sample `(0, 0)` sits.
    origin: [f32; 2],
    /// Yards between samples.
    step: f32,
    /// `side * side` heights, row-major: rows run along decreasing world x and
    /// columns along decreasing world y, the same frame as everything else here.
    z: Vec<f32>,
}

impl Ground {
    /// Sample a tile's ground onto a regular grid.
    ///
    /// Taken before the bake rather than during it, which is not only about
    /// speed: [`bake`] wants the tile mutably and this wants it shared, so the
    /// two cannot both hold it.
    pub fn of(tile: &AdtFile) -> Ground {
        let side = CHUNKS_PER_SIDE * ALPHA_SIDE;
        let step = CHUNK_SIZE / ALPHA_SIDE as f32;
        let mut z = vec![f32::MIN; side * side];
        let mut origin = [f32::MIN, f32::MIN];

        for (index, chunk) in tile.chunks.iter().enumerate().take(CHUNKS_PER_SIDE * CHUNKS_PER_SIDE)
        {
            let at = chunk.head().position();
            if index == 0 {
                origin = [at[0], at[1]];
            }
            let ground = heights::ground(chunk);
            let (cx, cy) = (index % CHUNKS_PER_SIDE, index / CHUNKS_PER_SIDE);
            for ty in 0..ALPHA_SIDE {
                for tx in 0..ALPHA_SIDE {
                    let x = at[0] - (ty as f32 + 0.5) * step;
                    let y = at[1] - (tx as f32 + 0.5) * step;
                    if let Some(h) = ground.height_at(x, y) {
                        z[(cy * ALPHA_SIDE + ty) * side + cx * ALPHA_SIDE + tx] = h;
                    }
                }
            }
        }
        Ground {
            side,
            origin,
            step,
            z,
        }
    }

    /// The ground at a world position, or `None` off the tile.
    fn at(&self, x: f32, y: f32) -> Option<f32> {
        let row = ((self.origin[0] - x) / self.step).floor();
        let col = ((self.origin[1] - y) / self.step).floor();
        if row < 0.0 || col < 0.0 {
            return None;
        }
        let (row, col) = (row as usize, col as usize);
        if row >= self.side || col >= self.side {
            return None;
        }
        match self.z[row * self.side + col] {
            v if v == f32::MIN => None,
            v => Some(v),
        }
    }
}

impl Occluder for Ground {
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool {
        // A ray going down cannot be blocked by ground it is already above, and
        // a ray straight up meets nothing. Neither happens with a real sun, and
        // both would loop or divide badly.
        if toward[2] <= 0.0 {
            return false;
        }
        let mut travelled = self.step;
        while travelled <= max {
            let x = from[0] + toward[0] * travelled;
            let y = from[1] + toward[1] * travelled;
            let z = from[2] + toward[2] * travelled;
            match self.at(x, y) {
                // Off the tile: there is nothing more this can say, and
                // marching on would sample the far edge for ever.
                None => return false,
                Some(ground) if ground > z => return true,
                Some(_) => {}
            }
            travelled += self.step;
        }
        false
    }
}

/// **Two occluders as one**, so a caller can put the ground and the buildings in
/// the same bake without either knowing about the other.
pub struct Either<'a>(pub &'a dyn Occluder, pub &'a dyn Occluder);

impl Occluder for Either<'_> {
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool {
        self.0.blocked(from, toward, max) || self.1.blocked(from, toward, max)
    }
}

/// What a bake changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Baked {
    /// Chunks whose bits are not what they were.
    pub changed: usize,
    /// …and how many of the 256 came out with any shadow on them at all.
    pub shadowed: usize,
}

/// **Recompute one chunk's `MCSH`.** Returns the 512 packed bits.
///
/// The texel grid is the alpha map's: row `ty` runs along decreasing world x and
/// column `tx` along decreasing world y, from the chunk's origin, which is its
/// maximum corner in both. That is the layout `Adt::alpha_atlas` assumes, the
/// one `vale textures`' seam check pins, and the one `vale sun` reads
/// shadows back through — a bake written under any other convention measures
/// correct and lands rotated.
///
/// **Texel 63 of each axis is computed and never read.** The pre-Cataclysm
/// client stretches a 64x64 map so texels 0 and 62 sit on the chunk's edges, so
/// `decode_shadow_map` copies 62 into 63 on the way in. Writing an honest value
/// there costs nothing and means the file says what the geometry says rather
/// than what one reader happens to do with it.
pub fn bake_chunk(
    tile: &AdtFile,
    chunk_index: usize,
    sun: [f32; 3],
    occluder: &dyn Occluder,
    reach: f32,
) -> Vec<u8> {
    let mut bits = vec![0u8; SHADOW_BYTES];
    let Some(chunk) = tile.chunks.get(chunk_index) else {
        return bits;
    };
    let origin = chunk.head().position();
    let ground = heights::ground(chunk);
    let texel = CHUNK_SIZE / ALPHA_SIDE as f32;
    let sun = normalise(sun);

    for ty in 0..ALPHA_SIDE {
        for tx in 0..ALPHA_SIDE {
            // The texel's centre, in the chunk's own decreasing axes.
            let x = origin[0] - (ty as f32 + 0.5) * texel;
            let y = origin[1] - (tx as f32 + 0.5) * texel;
            let Some(z) = ground.height_at(x, y) else {
                continue;
            };
            let from = [x, y, z + LIFT];
            if occluder.blocked(from, sun, reach) {
                let i = ty * ALPHA_SIDE + tx;
                // LSB first within each byte — the order `decode_shadow_map`
                // reads, and the order `MCAL`'s nibbles run in. Backwards, this
                // mirrors every group of eight texels inside its own byte,
                // which reads as a comb rather than as an error.
                bits[i / 8] |= 1 << (i % 8);
            }
        }
    }
    bits
}

/// **Recompute every chunk's `MCSH`, and put the header in step with it.**
///
/// A chunk that comes out with no shadow at all **loses the region and the
/// flag**, rather than carrying 64 bytes of zeroes. That is the same rule
/// [`crate::adt::colours`] follows for `MCCV` and it is the same argument: an
/// all-zero `MCSH` and no `MCSH` draw identically, and of the two only the
/// second is what the shipped files do for an unshadowed chunk. Without it a
/// tile bakes once and is permanently larger.
///
/// Both directions matter. The region is *added* to a chunk that had none the
/// moment something is placed to cast on it, and the flag with it — and the flag
/// is what the reader keys off, so writing the region without it is a bake
/// nothing draws.
pub fn bake(
    tile: &mut AdtFile,
    sun: [f32; 3],
    occluder: &dyn Occluder,
    reach: f32,
) -> Baked {
    bake_watching(tile, sun, occluder, reach, &|_| {})
}

/// …and the same, saying how far it has got.
///
/// `watch` is called with the number of chunks finished, after each one. A bake
/// is about a minute and there are 256 of them, so this is the difference
/// between a window that has stopped answering and one that is visibly working
/// — see `vale_ide::tools::tiles`, which runs it on a task pool and draws
/// the count on the status line.
///
/// It is a separate entry point rather than an `Option` on [`bake`] because
/// every existing caller wants neither, and because the closure has to be
/// `Send` for the pool while [`bake`]'s callers are on the main thread.
pub fn bake_watching(
    tile: &mut AdtFile,
    sun: [f32; 3],
    occluder: &dyn Occluder,
    reach: f32,
    watch: &(dyn Fn(usize) + Sync),
) -> Baked {
    let mut baked = Baked::default();
    for index in 0..tile.chunks.len().min(CHUNKS_PER_SIDE * CHUNKS_PER_SIDE) {
        let bits = bake_chunk(tile, index, sun, occluder, reach);
        let any = bits.iter().any(|&b| b != 0);
        if any {
            baked.shadowed += 1;
        }
        if set_shadow(tile, index, any.then_some(bits.as_slice())) {
            baked.changed += 1;
        }
        watch(index + 1);
    }
    baked
}

/// How many chunks a bake walks, for a caller sizing a progress bar.
pub const CHUNKS: usize = CHUNKS_PER_SIDE * CHUNKS_PER_SIDE;

/// Put a chunk's `MCSH` bits in place, or take the region away.
///
/// Returns whether anything moved, so a caller can skip writing a tile nothing
/// changed on.
pub fn set_shadow(tile: &mut AdtFile, chunk_index: usize, bits: Option<&[u8]>) -> bool {
    let Some(chunk) = tile.chunks.get_mut(chunk_index) else {
        return false;
    };
    let was = chunk
        .region(Region::Shadow)
        .map(|sub| sub.data.clone());
    let now = bits.map(|b| b.to_vec());
    let flags = u32_at(&chunk.header, at::FLAGS);
    let wanted_flags = match now.is_some() {
        true => flags | mcnk_flags::HAS_MCSH,
        false => flags & !mcnk_flags::HAS_MCSH,
    };
    if was == now && flags == wanted_flags {
        return false;
    }

    chunk.regions[Region::Shadow as usize] = now.map(SubChunk::new);
    put_u32(&mut chunk.header, at::FLAGS, wanted_flags);
    true
}

/// A chunk's `MCSH` as one byte per texel, `255` in shadow — the same shape
/// `vale_assets`' reader answers with, for a caller comparing the two.
///
/// `None` when the chunk declares no shadow, which is the honest answer and not
/// an empty grid: *no shadow declared* and *a shadow that happens to be empty*
/// are the same picture and different files.
pub fn shadow_texels(tile: &AdtFile, chunk_index: usize) -> Option<Vec<u8>> {
    let chunk = tile.chunks.get(chunk_index)?;
    if u32_at(&chunk.header, at::FLAGS) & mcnk_flags::HAS_MCSH == 0 {
        return None;
    }
    let bits = &chunk.region(Region::Shadow)?.data;
    Some(
        (0..ALPHA_SIDE * ALPHA_SIDE)
            .map(|i| match bits.get(i / 8) {
                Some(&byte) if byte >> (i % 8) & 1 != 0 => 255,
                _ => 0,
            })
            .collect(),
    )
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    match len > 1.0e-6 {
        true => [v[0] / len, v[1] / len, v[2] / len],
        false => [0.0, 0.0, 1.0],
    }
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(word)
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::blank::{blank_tile, chunk_origin};
    use vale_assets::tables::light::SUN_TOWARD;

    const GRASS: &str = r"Tileset\Elwynn\ElwynnGrassBase.blp";

    fn tile() -> AdtFile {
        blank_tile(32, 48, GRASS, 100.0, 0)
    }

    /// An occluder that blocks everything: the whole tile is in shadow.
    struct Everything;
    impl Occluder for Everything {
        fn blocked(&self, _: [f32; 3], _: [f32; 3], _: f32) -> bool {
            true
        }
    }

    /// **Nothing casting means no region and no flag**, rather than a region of
    /// zeroes. A blank tile has neither to begin with, so this is also the check
    /// that a bake over empty ground leaves the file alone.
    #[test]
    fn an_unoccluded_tile_carries_no_shadow_at_all() {
        let mut tile = tile();
        let baked = bake(&mut tile, SUN_TOWARD, &Nothing, REACH);
        assert_eq!(baked.shadowed, 0);
        assert_eq!(baked.changed, 0, "nothing should have moved");
        for chunk in &tile.chunks {
            assert!(chunk.region(Region::Shadow).is_none());
            assert_eq!(chunk.head().flags() & mcnk_flags::HAS_MCSH, 0);
        }
    }

    /// …and everything casting sets the region, the flag and every bit.
    #[test]
    fn a_fully_occluded_tile_is_all_shadow() {
        let mut tile = tile();
        let baked = bake(&mut tile, SUN_TOWARD, &Everything, REACH);
        assert_eq!(baked.shadowed, 256);
        assert_eq!(baked.changed, 256);
        for (i, chunk) in tile.chunks.iter().enumerate() {
            let sub = chunk.region(Region::Shadow).expect("region");
            assert_eq!(sub.data.len(), SHADOW_BYTES);
            assert!(sub.data.iter().all(|&b| b == 0xFF), "chunk {i}");
            assert_ne!(chunk.head().flags() & mcnk_flags::HAS_MCSH, 0);
        }
    }

    /// **And baking it back to nothing takes the region away again**, so a tile
    /// whose buildings were deleted writes back as a tile with no shadow rather
    /// than 16 KB of zeroes.
    #[test]
    fn clearing_a_bake_removes_the_region_and_the_flag() {
        let mut tile = tile();
        bake(&mut tile, SUN_TOWARD, &Everything, REACH);
        let baked = bake(&mut tile, SUN_TOWARD, &Nothing, REACH);
        assert_eq!(baked.changed, 256);
        assert_eq!(baked.shadowed, 0);
        assert!(tile.chunks.iter().all(|c| c.region(Region::Shadow).is_none()));
        assert!(tile
            .chunks
            .iter()
            .all(|c| c.head().flags() & mcnk_flags::HAS_MCSH == 0));
        // …and the file is the one it started as.
        assert_eq!(tile.write(), self::tests::tile().write());
    }

    /// A second bake with the same occluder changes nothing, which is what lets
    /// a caller bake on every edit without rewriting the file every time.
    #[test]
    fn baking_twice_is_idempotent() {
        let mut tile = tile();
        bake(&mut tile, SUN_TOWARD, &Everything, REACH);
        let again = bake(&mut tile, SUN_TOWARD, &Everything, REACH);
        assert_eq!(again.changed, 0);
    }

    /// **The bits land on the texels the geometry is over.** An occluder that
    /// blocks only rays starting past a world x cuts the tile in half, and the
    /// half it cuts has to be the half the texel mapping says — which is the one
    /// thing here that a correct-looking result can get wrong, because a
    /// transposed or mirrored map is still a plausible shadow.
    #[test]
    fn a_half_blocked_chunk_shadows_the_right_half() {
        let mut tile = tile();
        // Chunk 0's origin is its maximum corner; x decreases along rows.
        let origin = chunk_origin(32, 48, 0);
        let cut = origin[0] - CHUNK_SIZE / 2.0;
        let half = move |from: [f32; 3], _: [f32; 3], _: f32| from[0] < cut;
        bake(&mut tile, SUN_TOWARD, &half, REACH);

        let texels = shadow_texels(&tile, 0).expect("chunk 0 has a shadow");
        for ty in 0..ALPHA_SIDE {
            for tx in 0..ALPHA_SIDE {
                let lit = texels[ty * ALPHA_SIDE + tx] == 0;
                // Rows past the middle are the ones with the smaller x.
                let expected_lit = ty < ALPHA_SIDE / 2;
                assert_eq!(lit, expected_lit, "texel ({tx}, {ty})");
            }
        }
    }

    /// …and the same in the other axis, since one test can only pin one of the
    /// two ways a grid transposes.
    #[test]
    fn a_half_blocked_chunk_shadows_the_right_columns() {
        let mut tile = tile();
        let origin = chunk_origin(32, 48, 0);
        let cut = origin[1] - CHUNK_SIZE / 2.0;
        let half = move |from: [f32; 3], _: [f32; 3], _: f32| from[1] < cut;
        bake(&mut tile, SUN_TOWARD, &half, REACH);

        let texels = shadow_texels(&tile, 0).unwrap();
        for ty in 0..ALPHA_SIDE {
            for tx in 0..ALPHA_SIDE {
                let lit = texels[ty * ALPHA_SIDE + tx] == 0;
                assert_eq!(lit, tx < ALPHA_SIDE / 2, "texel ({tx}, {ty})");
            }
        }
    }

    /// **The bit order is the reader's.** A bake packed MSB-first decodes as a
    /// comb at a period of eight texels along one axis, which looks like lace
    /// rather than like an error — so this compares against the reader's own
    /// decode rather than against an expectation written here.
    #[test]
    fn the_packed_bits_are_the_order_the_reader_unpacks() {
        let mut tile = tile();
        // One texel in eight, so a mirrored byte is unmistakable.
        let every_eighth = |from: [f32; 3], _: [f32; 3], _: f32| {
            let origin = chunk_origin(32, 48, 0);
            let texel = CHUNK_SIZE / ALPHA_SIDE as f32;
            let tx = ((origin[1] - from[1]) / texel) as usize;
            tx % 8 == 0
        };
        bake(&mut tile, SUN_TOWARD, &every_eighth, REACH);

        let raw = tile.write();
        let adt = vale_assets::world::adt::Adt::parse(&raw).unwrap();
        let shadow = &adt.chunks[0].shadow;
        assert_eq!(shadow.len(), ALPHA_SIDE * ALPHA_SIDE);
        // Column 0 of every row is shadowed and column 1 is not. The reader's
        // `fix_alpha_edge` rewrites the last row and column, so the check stops
        // short of them.
        for ty in 0..ALPHA_SIDE - 1 {
            assert_ne!(shadow[ty * ALPHA_SIDE], 0, "row {ty} column 0");
            assert_eq!(shadow[ty * ALPHA_SIDE + 1], 0, "row {ty} column 1");
            assert_ne!(shadow[ty * ALPHA_SIDE + 8], 0, "row {ty} column 8");
        }
    }

    /// A tile that has been baked is still a tile the reader reads, which is
    /// the end-to-end the other tests approach one field at a time.
    #[test]
    fn a_baked_tile_still_parses() {
        let mut tile = tile();
        bake(&mut tile, SUN_TOWARD, &Everything, REACH);
        let raw = tile.write();
        let adt = vale_assets::world::adt::Adt::parse(&raw).unwrap();
        assert_eq!(adt.chunks.len(), 256);
        for chunk in &adt.chunks {
            assert_eq!(chunk.shadow.len(), ALPHA_SIDE * ALPHA_SIDE);
            assert!(chunk.shadow.iter().all(|&t| t == 255));
        }
        // …and this crate can read it back too.
        assert_eq!(AdtFile::parse(&raw).unwrap().write(), raw);
    }

    /// **Flat ground shadows nothing**, which is the identity the marcher has to
    /// have: every sample is at the same height as the ray's start and the ray
    /// climbs away from it.
    #[test]
    fn flat_ground_casts_no_shadow_on_itself() {
        let mut tile = tile();
        let ground = Ground::of(&tile);
        let baked = bake(&mut tile, SUN_TOWARD, &ground, REACH);
        assert_eq!(baked.shadowed, 0, "flat ground shadowed itself");
    }

    /// …and a wall of ground does. Raise one half of the tile far above the
    /// other and the low half has to come out shadowed on the side the sun is
    /// on — which is the check that the marcher walks the right way.
    #[test]
    fn a_step_in_the_ground_shadows_what_is_behind_it() {
        let mut tile = tile();
        // The sun is at azimuth 40 from +x toward +y, so it is in the +x/+y
        // direction: ground at *higher* x shadows ground at lower x.
        for index in 0..tile.chunks.len() {
            let origin = chunk_origin(32, 48, index);
            // Chunks in the half nearer the tile's maximum x get raised.
            if origin[0] > chunk_origin(32, 48, 128)[0] {
                let at = tile.chunks[index].head().position();
                tile.chunks[index]
                    .head_mut()
                    .set_position([at[0], at[1], at[2] + 300.0]);
            }
        }
        let ground = Ground::of(&tile);
        let baked = bake(&mut tile, SUN_TOWARD, &ground, REACH);
        assert!(
            baked.shadowed > 0,
            "a 300-yard step cast nothing at all"
        );
        // **The chunk immediately below the step**, which is row 8 — the first
        // unraised row, with a 300-yard wall along its high-x edge. Further
        // down the tile the wall is past `REACH` and the ground is legitimately
        // lit, which is what the first draft of this test asserted on and why it
        // failed: a shadow is only as long as the reach allows.
        let low = shadow_texels(&tile, 128).expect("the chunk under the step has a shadow");
        let shadowed = low.iter().filter(|&&t| t != 0).count();
        assert!(
            shadowed > low.len() / 4,
            "only {shadowed} of {} texels under a 300-yard wall are shadowed",
            low.len()
        );
        // …and the raised half, which nothing stands over, is not.
        assert!(
            shadow_texels(&tile, 0).is_none(),
            "the top of the step shadowed itself"
        );
    }

    /// A ray that leaves the tile stops rather than marching against the edge
    /// for ever — which without the `None` arm is an infinite band of shadow
    /// along two edges.
    #[test]
    fn a_ray_off_the_tile_is_not_blocked() {
        let tile = tile();
        let ground = Ground::of(&tile);
        // Well outside the tile, aimed away from it.
        let outside = [-20000.0, -20000.0, 500.0];
        assert!(!ground.blocked(outside, normalise(SUN_TOWARD), REACH));
    }

    /// **Two occluders compose**, which is how a caller puts the ground and the
    /// buildings in one bake.
    #[test]
    fn either_is_blocked_when_either_is() {
        let yes = Everything;
        let no = Nothing;
        let from = [0.0, 0.0, 0.0];
        let up = [0.0, 0.0, 1.0];
        assert!(Either(&yes, &no).blocked(from, up, 10.0));
        assert!(Either(&no, &yes).blocked(from, up, 10.0));
        assert!(!Either(&no, &no).blocked(from, up, 10.0));
    }

    /// **The watcher is told about every chunk, once, in order.** A progress
    /// bar that skipped or repeated would read as a bake that had stalled.
    #[test]
    fn a_bake_reports_every_chunk_once() {
        let mut tile = tile();
        let seen = std::sync::Mutex::new(Vec::new());
        bake_watching(&mut tile, SUN_TOWARD, &Nothing, REACH, &|done| {
            seen.lock().unwrap().push(done);
        });
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), CHUNKS);
        assert_eq!(seen.first(), Some(&1));
        assert_eq!(seen.last(), Some(&CHUNKS));
        assert!(seen.windows(2).all(|w| w[1] == w[0] + 1), "out of order");
    }

    /// The ray leaves from above the ground rather than on it. Without the lift
    /// a bake whose occluder includes the terrain has every ray hit the triangle
    /// it started on and the whole world comes out black.
    #[test]
    fn rays_start_above_the_ground() {
        let tile = tile();
        let seen = std::cell::RefCell::new(Vec::new());
        let record = |from: [f32; 3], _: [f32; 3], _: f32| {
            seen.borrow_mut().push(from[2]);
            false
        };
        bake_chunk(&tile, 0, SUN_TOWARD, &record, REACH);
        let seen = seen.into_inner();
        assert_eq!(seen.len(), ALPHA_SIDE * ALPHA_SIDE);
        assert!(seen.iter().all(|&z| (z - (100.0 + LIFT)).abs() < 0.001));
    }
}
