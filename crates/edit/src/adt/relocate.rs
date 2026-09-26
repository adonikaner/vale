//! Moving a whole tile to different coordinates.
//!
//! A tile's own file says where it is, twice over and in two different frames:
//! every one of its 256 map chunks carries a world position, and every `MDDF`
//! and `MODF` record carries a placement. Copying `Azeroth_32_48.adt` to
//! `Azeroth_33_48.adt` and leaving both alone gives a tile that loads, draws
//! *at the old coordinates*, and overlaps whatever is really there — the
//! streamer puts it in the new slot and the geometry insists on the old place.
//!
//! So a copy is a copy and a rewrite, and this is the rewrite.
//!
//! ## The two frames move by different numbers
//!
//! A chunk position is **world** space, where a tile's `x` is its *row* and its
//! `y` is its *column* — see [`super::blank`], where that is measured off a real
//! tile. A placement is `MDDF`/`MODF` space, which is an offset back from
//! [`MAP_ORIGIN`] with the axes swapped, and
//! [`vale_assets::world::adt::placement_from_world`] is the statement of it.
//! Move a tile one column east and the world `y` of every chunk falls by
//! [`TILE_SIZE`] while the *first* component of every placement rises by it.
//!
//! Getting that swap wrong is the failure this module exists to make
//! impossible: the ground lands correctly and every tree on it is a tile away,
//! which reads as "the doodads did not copy" rather than as an axis error.
//!
//! ## What is not touched
//!
//! **The heights, the textures, the alpha maps, the shadow, the water, the area
//! ids.** All of them are relative to the chunk rather than to the world, so
//! they move with it for free. The `MCSH` is the one that is arguably wrong
//! after a move — the shadow was baked for what stood around the old place — and
//! it is left alone rather than cleared, because clearing it would throw away a
//! correct bake for a tile copied one step sideways within the same forest. The
//! tile tool's own rebake is the answer when it matters.

use super::file::AdtFile;
use super::header::at;
use super::place::{Building, Doodad};
use vale_assets::world::adt::{MAP_ORIGIN, TILE_SIZE};

/// **Rewrite a tile to sit at different coordinates.**
///
/// `from` and `to` are ADT numbering — the `32` and `48` of `Azeroth_32_48`. A
/// move to where it already is does nothing and says so.
///
/// Returns how many records were moved: chunks, then doodads, then buildings.
pub fn relocate(tile: &mut AdtFile, from: (u32, u32), to: (u32, u32)) -> (usize, usize, usize) {
    if from == to {
        return (0, 0, 0);
    }

    // The world shift: a tile's x is its row and its y is its column, so the
    // two coordinates cross over. This is the whole of the arithmetic and every
    // other line here is bookkeeping.
    let shift_x = -((to.1 as f32 - from.1 as f32) * TILE_SIZE);
    let shift_y = -((to.0 as f32 - from.0 as f32) * TILE_SIZE);

    let mut chunks = 0usize;
    for chunk in &mut tile.chunks {
        let was = chunk.head().position();
        chunk
            .head_mut()
            .set_position([was[0] + shift_x, was[1] + shift_y, was[2]]);
        chunks += 1;
    }

    // …and the placements, in their own frame. `placement_from_world` is
    // `[MAP_ORIGIN - y, z, MAP_ORIGIN - x]`, so a world shift of `(dx, dy)`
    // is a placement shift of `(-dy, -dx)` on components 0 and 2 — which is the
    // swap the module comment is about, written once.
    let mut doodads: Vec<Doodad> = tile.doodad_list();
    for record in &mut doodads {
        record.position[0] -= shift_y;
        record.position[2] -= shift_x;
    }
    let moved_doodads = doodads.len();
    tile.set_doodad_list(&doodads);

    let mut buildings: Vec<Building> = tile.building_list();
    for record in &mut buildings {
        record.position[0] -= shift_y;
        record.position[2] -= shift_x;
        // A building's box is stated in the same frame and has to follow it, or
        // the culler rejects the building from everywhere it now is: a `MODF`
        // placement carries its own box.
        for corner in [&mut record.bounds_lower, &mut record.bounds_upper] {
            corner[0] -= shift_y;
            corner[2] -= shift_x;
        }
    }
    let moved_buildings = buildings.len();
    tile.set_building_list(&buildings);

    (chunks, moved_doodads, moved_buildings)
}

/// Where a tile's own origin is — its maximum corner, in world axes.
///
/// The same numbers [`super::blank::chunk_origin`] gives for chunk 0, named
/// separately because a caller reasoning about whole tiles should not have to
/// know that chunk 0 is the corner one.
pub fn tile_origin(tile_x: u32, tile_y: u32) -> [f32; 2] {
    [
        MAP_ORIGIN - tile_y as f32 * TILE_SIZE,
        MAP_ORIGIN - tile_x as f32 * TILE_SIZE,
    ]
}

/// Set every chunk's `indexX`/`indexY` to match its slot.
///
/// Not part of [`relocate`], because the indices are *within* a tile and do not
/// change when the tile moves. It is here because it is the other thing a caller
/// building a tile out of parts has to get right, and because a tile assembled
/// with them wrong loads and draws transposed.
pub fn renumber(tile: &mut AdtFile) {
    use vale_assets::world::adt::CHUNKS_PER_SIDE;

    for (index, chunk) in tile.chunks.iter_mut().enumerate() {
        put_u32(&mut chunk.header, at::INDEX_X, (index % CHUNKS_PER_SIDE) as u32);
        put_u32(&mut chunk.header, at::INDEX_Y, (index / CHUNKS_PER_SIDE) as u32);
    }
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::blank::{blank_tile, chunk_origin};
    use vale_assets::world::adt::Adt;

    const GRASS: &str = r"Tileset\Elwynn\ElwynnGrassBase.blp";

    /// **The ground lands on the tile it was moved to.** The check that matters
    /// and the one a caller cannot do for itself: build a tile at one pair of
    /// coordinates, move it to another, and ask the crate that reads tiles which
    /// tile its chunks are in.
    #[test]
    fn a_moved_tile_is_where_it_was_moved_to() {
        for (from, to) in [
            ((32u32, 48u32), (33u32, 48u32)),
            ((32, 48), (32, 49)),
            ((32, 48), (40, 20)),
            ((0, 0), (63, 63)),
        ] {
            let mut tile = blank_tile(from.0, from.1, GRASS, 100.0, 0);
            relocate(&mut tile, from, to);
            let adt = Adt::parse(&tile.write()).expect("parses");
            for (index, chunk) in adt.chunks.iter().enumerate() {
                let want = chunk_origin(to.0, to.1, index);
                assert!(
                    (chunk.position[0] - want[0]).abs() < 0.01
                        && (chunk.position[1] - want[1]).abs() < 0.01,
                    "{from:?} -> {to:?} chunk {index}: {:?} is not {want:?}",
                    chunk.position
                );
            }
            // …and the tile a world point inside it resolves to.
            let corner = chunk_origin(to.0, to.1, 0);
            assert_eq!(
                vale_assets::tile_for_position(corner[0] - 1.0, corner[1] - 1.0),
                to
            );
        }
    }

    /// …and the heights come with it unchanged, since they are the chunk's own.
    #[test]
    fn a_moved_tile_keeps_its_shape() {
        let mut tile = blank_tile(32, 48, GRASS, 123.5, 7);
        relocate(&mut tile, (32, 48), (10, 10));
        let adt = Adt::parse(&tile.write()).unwrap();
        for chunk in &adt.chunks {
            assert!(chunk.heights.iter().all(|h| (h - 123.5).abs() < 0.01));
            assert_eq!(chunk.area_id, 7);
        }
    }

    /// **A doodad moves with the ground under it**, which is the half that can
    /// be plausibly wrong: the two frames cross over, so an implementation that
    /// applied the world shift to a placement puts every tree a tile away in the
    /// wrong axis — and the ground looks perfect.
    #[test]
    fn a_placement_moves_with_the_ground() {
        use vale_assets::world::adt::placement_from_world;

        let from = (32u32, 48u32);
        let to = (34u32, 51u32);
        let mut tile = blank_tile(from.0, from.1, GRASS, 0.0, 0);

        // A doodad standing a little inside the tile's own corner.
        let origin = chunk_origin(from.0, from.1, 0);
        let stood = [origin[0] - 40.0, origin[1] - 70.0, 12.0];
        let name_id = tile.name_model("world\tree.m2");
        tile.add_doodad(
            Doodad {
                name_id,
                unique_id: 1,
                position: placement_from_world(stood),
                rotation: [0.0; 3],
                scale: 1024,
                flags: 0,
            },
            5.0,
        );

        relocate(&mut tile, from, to);

        let adt = Adt::parse(&tile.write()).expect("parses");
        let placed = adt.placed_doodads();
        assert_eq!(placed.len(), 1, "the doodad did not survive");

        // Where it should now be: the same offset inside the new tile's corner.
        let corner = chunk_origin(to.0, to.1, 0);
        let want = [corner[0] - 40.0, corner[1] - 70.0, 12.0];
        for axis in 0..3 {
            assert!(
                (placed[0].position[axis] - want[axis]).abs() < 0.05,
                "axis {axis}: {:?} is not {want:?}",
                placed[0].position
            );
        }
        // …and it is on the tile it was moved to, which is the statement that
        // matters and the one a swapped axis fails.
        assert_eq!(
            vale_assets::tile_for_position(placed[0].position[0], placed[0].position[1]),
            to
        );
    }

    /// Moving a tile to where it already is changes nothing at all, so a caller
    /// can relocate unconditionally.
    #[test]
    fn moving_a_tile_nowhere_is_the_identity() {
        let made = blank_tile(32, 48, GRASS, 50.0, 0);
        let mut copy = made.clone();
        assert_eq!(relocate(&mut copy, (32, 48), (32, 48)), (0, 0, 0));
        assert_eq!(copy.write(), made.write());
    }

    /// Every chunk is moved, and the count says so.
    #[test]
    fn every_chunk_is_moved() {
        let mut tile = blank_tile(32, 48, GRASS, 0.0, 0);
        let (chunks, doodads, buildings) = relocate(&mut tile, (32, 48), (33, 48));
        assert_eq!(chunks, 256);
        assert_eq!((doodads, buildings), (0, 0));
    }

    /// `renumber` puts the indices back in step with the slots, which is what a
    /// tile assembled out of parts needs and what a transposed tile is missing.
    #[test]
    fn renumbering_puts_the_indices_in_step() {
        use vale_assets::world::adt::CHUNKS_PER_SIDE;

        let mut tile = blank_tile(32, 48, GRASS, 0.0, 0);
        // Scramble them the way a hand-assembled tile would be.
        for chunk in &mut tile.chunks {
            put_u32(&mut chunk.header, at::INDEX_X, 99);
            put_u32(&mut chunk.header, at::INDEX_Y, 99);
        }
        renumber(&mut tile);
        for (index, chunk) in tile.chunks.iter().enumerate() {
            assert_eq!(
                chunk.head().index(),
                ((index % CHUNKS_PER_SIDE) as u32, (index / CHUNKS_PER_SIDE) as u32)
            );
        }
    }

    #[test]
    fn a_tiles_origin_is_its_first_chunks() {
        for (x, y) in [(0u32, 0u32), (32, 48), (63, 63)] {
            let origin = tile_origin(x, y);
            let chunk = chunk_origin(x, y, 0);
            assert!((origin[0] - chunk[0]).abs() < 0.001);
            assert!((origin[1] - chunk[1]).abs() < 0.001);
        }
    }
}
