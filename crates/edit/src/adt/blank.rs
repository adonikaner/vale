//! A tile made from nothing.
//!
//! Every other file in this directory changes a tile that already exists. This
//! one builds one, which is the other half of what [`crate::wdt`] is for: a map
//! can claim a tile it has no ADT for, and a claim with no file behind it is a
//! hole in the world rather than new ground.
//!
//! ## What a tile needs to be, at a minimum
//!
//! Measured against `Azeroth_32_48` — 1,844,924 bytes, 256 map chunks — rather
//! than taken from a specification:
//!
//! ```text
//! MVER  18
//! MHDR  16 words; the writer fills eight of them
//! MCIN  256 entries of 16 bytes; the writer fills all of it
//! MTEX  the ground textures, NUL-separated — at least one, or nothing draws
//! MMDX  MMID  MWMO  MWID  MDDF  MODF   all legitimately empty
//! MCNK  x256, each: a 128-byte header, then MCVT, MCNR, MCLY, MCRF
//! ```
//!
//! The four regions are the floor of what a chunk can carry. `MCSH`, `MCAL`,
//! `MCLQ` and `MCSE` are all absent from chunks of real tiles — a dry, unshaded,
//! single-layer chunk has none of them — so leaving them out is the shipped
//! shape and not a shortcut.
//!
//! ## Where the chunk origins come from, measured
//!
//! A chunk's position is its **maximum** x and y corner, and the two indices in
//! its header run in the two world axes the opposite way round from how they
//! read. From `Azeroth_32_48`, chunk by chunk:
//!
//! ```text
//!    i  ix  iy         posX         posY
//!    0   0   0    -8533.334        0.000
//!    1   1   0    -8533.334      -33.334     ix advances -y
//!   16   0   1    -8566.666        0.000     iy advances -x
//!  255  15  15    -9033.332     -500.000
//! ```
//!
//! so `ix = i % 16` is the column and moves along **world y**, `iy = i / 16` is
//! the row and moves along **world x**, and the tile's own origin is
//! `MAP_ORIGIN - tile_y * TILE_SIZE` in x and `MAP_ORIGIN - tile_x * TILE_SIZE`
//! in y — which is `tile_for_position` run backwards, and is why that function
//! takes world y to get a tile x.
//!
//! Getting either of those swapped produces a tile that loads, draws, and sits
//! with its rows and columns transposed against the tiles beside it: a seam on
//! two edges and nothing that reports an error.

use super::file::{AdtFile, MapChunk, Region, SubChunk, MCNK_HEADER};
use super::header::at;
use vale_assets::world::adt::{
    CHUNKS_PER_SIDE, CHUNK_SIZE, HEIGHTS_PER_CHUNK, MAP_ORIGIN, TILE_SIZE,
};

/// How many map chunks a tile has.
pub const CHUNKS: usize = CHUNKS_PER_SIDE * CHUNKS_PER_SIDE;

/// Bytes of padding after `MCNR`, which no size field covers.
///
/// Thirteen, on every one of 1,280 measured chunks. Nothing in this project
/// reads them and the client finds the next region by the header's offset
/// rather than by walking — but a tile written without them is a tile shaped
/// unlike every tile the game ships, and the cost of matching is thirteen zero
/// bytes.
const MCNR_PADDING: usize = 13;

/// One `MCLY` entry: texture id, flags, offset into `MCAL`, effect id.
const LAYER_ENTRY: usize = 16;

/// The upward normal as `MCNR` encodes it — see [`super::heights`], where the
/// scale is measured: x, y, z each `n * 127`, and the third byte is up.
const UP: [i8; 3] = [0, 0, 127];

/// **A tile with flat ground at one height, painted with one texture.**
///
/// `tile_x` and `tile_y` are the ADT's own numbering — the `32` and `48` of
/// `Azeroth_32_48` — and decide where every chunk in it sits, so they are not
/// cosmetic: a tile built for the wrong pair is a tile in the wrong place.
///
/// `texture` is a path into the archives (`Tileset\Elwynn\ElwynnGrassBase.blp`).
/// One layer and no alpha map is the shipped shape for a chunk painted with a
/// single texture — layer 0 is the opaque base and has no `MCAL` entry, which
/// is the same rule `alpha.rs` writes under.
///
/// `height` is world z, flat across the whole tile. Flat is the honest starting
/// point: the alternative is inventing terrain, and the brush exists.
///
/// `area` is the `AreaTable.dbc` id every chunk is assigned. Zero is legitimate
/// and means *no area*, which is what an unassigned chunk of a real tile reads.
pub fn blank_tile(tile_x: u32, tile_y: u32, texture: &str, height: f32, area: u32) -> AdtFile {
    let mut textures = texture.as_bytes().to_vec();
    textures.push(0);

    AdtFile {
        version: 18,
        header: [0; 16],
        textures,
        models: Vec::new(),
        model_offsets: Vec::new(),
        buildings: Vec::new(),
        building_offsets: Vec::new(),
        doodads: Vec::new(),
        placements: Vec::new(),
        chunks: (0..CHUNKS)
            .map(|i| blank_chunk(tile_x, tile_y, i, height, area))
            .collect(),
    }
}

/// Where map chunk `index` of tile `(tile_x, tile_y)` sits — see the module
/// comment, where the measurement is.
pub fn chunk_origin(tile_x: u32, tile_y: u32, index: usize) -> [f32; 2] {
    let ix = (index % CHUNKS_PER_SIDE) as f32;
    let iy = (index / CHUNKS_PER_SIDE) as f32;
    [
        MAP_ORIGIN - tile_y as f32 * TILE_SIZE - iy * CHUNK_SIZE,
        MAP_ORIGIN - tile_x as f32 * TILE_SIZE - ix * CHUNK_SIZE,
    ]
}

fn blank_chunk(tile_x: u32, tile_y: u32, index: usize, height: f32, area: u32) -> MapChunk {
    let mut header = [0u8; MCNK_HEADER];
    let origin = chunk_origin(tile_x, tile_y, index);

    put_u32(&mut header, at::INDEX_X, (index % CHUNKS_PER_SIDE) as u32);
    put_u32(&mut header, at::INDEX_Y, (index / CHUNKS_PER_SIDE) as u32);
    put_u32(&mut header, at::LAYER_COUNT, 1);
    put_u32(&mut header, at::AREA_ID, area);
    put_f32(&mut header, at::POSITION, origin[0]);
    put_f32(&mut header, at::POSITION + 4, origin[1]);
    // **The chunk's own z, and the heights are relative to it.** Putting the
    // height here and zeroes in `MCVT` is the same ground as zero here and the
    // height repeated 145 times, and it is what the shipped files do — every
    // chunk of `Azeroth_32_48` carries a non-zero z and small offsets.
    put_f32(&mut header, at::POSITION + 8, height);

    // Flags stay zero: no `MCSH` (nothing casts yet), no liquid, no shading.
    let mut regions: [Option<SubChunk>; 9] = Default::default();
    regions[Region::Heights as usize] = Some(SubChunk::new(vec![0u8; HEIGHTS_PER_CHUNK * 4]));
    regions[Region::Normals as usize] = Some(SubChunk {
        data: UP.iter()
            .cycle()
            .take(HEIGHTS_PER_CHUNK * 3)
            .map(|&b| b as u8)
            .collect(),
        padding: vec![0u8; MCNR_PADDING],
        declared: (HEIGHTS_PER_CHUNK * 3) as u32,
    });
    regions[Region::Layers as usize] = Some(SubChunk::new(vec![0u8; LAYER_ENTRY]));
    // An empty `MCRF`: this chunk stands over nothing. Present rather than
    // absent because every chunk of every shipped tile has one, and the
    // header's `nDoodadRefs`/`nMapObjRefs` already say it is empty.
    regions[Region::Refs as usize] = Some(SubChunk::new(Vec::new()));

    MapChunk {
        header,
        regions,
        index_extra: [0; 2],
    }
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(buf: &mut [u8], at: usize, value: f32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::adt::Adt;

    const GRASS: &str = r"Tileset\Elwynn\ElwynnGrassBase.blp";

    /// **The check that matters: a tile this crate invented is a tile the
    /// crate that reads tiles can read.** Everything else here is a detail of
    /// that.
    #[test]
    fn a_blank_tile_parses_as_a_real_one() {
        let raw = blank_tile(32, 48, GRASS, 100.0, 12).write();
        let adt = Adt::parse(&raw).expect("a blank tile must parse");
        assert_eq!(adt.chunks.len(), CHUNKS);
        assert_eq!(adt.texture_names, vec![GRASS.to_string()]);
        assert!(adt.placed_doodads().is_empty());
        assert!(adt.placed_wmos().is_empty());
    }

    /// …and it round-trips through this crate's own container unchanged, which
    /// is what says the writer and the parser agree about what it built.
    #[test]
    fn a_blank_tile_round_trips() {
        let made = blank_tile(32, 48, GRASS, 100.0, 12);
        let raw = made.write();
        let read = AdtFile::parse(&raw).expect("a blank tile must re-parse");
        assert_eq!(read.write(), raw);
        assert_eq!(read.chunks.len(), CHUNKS);
    }

    /// **The chunk origins are the ones the game's own tile has.** These four
    /// numbers are read off `Azeroth_32_48` and are the whole of why the module
    /// comment exists: swapping the two indices produces a tile that loads and
    /// draws transposed, with a seam on two edges and no error anywhere.
    #[test]
    fn the_chunk_origins_match_the_shipped_tiles() {
        // (index, posX, posY), measured.
        for (index, x, y) in [
            (0usize, -8533.334f32, 0.0f32),
            (1, -8533.334, -33.334),
            (16, -8566.666, 0.0),
            (255, -9033.332, -500.0),
        ] {
            let got = chunk_origin(32, 48, index);
            assert!(
                (got[0] - x).abs() < 0.01 && (got[1] - y).abs() < 0.01,
                "chunk {index}: {got:?} is not [{x}, {y}]"
            );
        }
    }

    /// …and the header carries the same pair, so a reader that trusts the
    /// indices and a reader that trusts the position agree.
    #[test]
    fn the_header_indices_and_the_position_agree() {
        let tile = blank_tile(32, 48, GRASS, 0.0, 0);
        for (i, chunk) in tile.chunks.iter().enumerate() {
            let head = chunk.head();
            assert_eq!(
                head.index(),
                ((i % CHUNKS_PER_SIDE) as u32, (i / CHUNKS_PER_SIDE) as u32)
            );
            let origin = chunk_origin(32, 48, i);
            let position = head.position();
            assert!((position[0] - origin[0]).abs() < 0.01, "chunk {i} x");
            assert!((position[1] - origin[1]).abs() < 0.01, "chunk {i} y");
        }
    }

    /// **A tile lands where `tile_for_position` says it should.** The join that
    /// cannot be checked from inside the file: build a tile, take a point
    /// inside one of its chunks, and ask the reader which tile that is.
    #[test]
    fn a_blank_tile_lands_on_its_own_coordinates() {
        for (tx, ty) in [(32u32, 48u32), (0, 0), (63, 63), (34, 51)] {
            // A point a little inside the tile's maximum corner, since the
            // origin itself is the boundary between two tiles.
            let origin = chunk_origin(tx, ty, 0);
            let inside = (origin[0] - 1.0, origin[1] - 1.0);
            assert_eq!(
                vale_assets::tile_for_position(inside.0, inside.1),
                (tx, ty),
                "tile {tx},{ty}"
            );
        }
    }

    /// The ground is flat, at the height asked for, everywhere.
    #[test]
    fn the_ground_is_flat_at_the_height_given() {
        let raw = blank_tile(32, 48, GRASS, 123.5, 0).write();
        let adt = Adt::parse(&raw).unwrap();
        for chunk in &adt.chunks {
            assert_eq!(chunk.heights.len(), HEIGHTS_PER_CHUNK);
            for &h in &chunk.heights {
                assert!((h - 123.5).abs() < 0.001, "height {h} is not 123.5");
            }
        }
    }

    /// …and the normals point up, so it shades as flat ground rather than as
    /// black ground. A tile with zeroed `MCNR` draws unlit and looks like a
    /// hole; this is the one field that cannot be left at its default.
    #[test]
    fn the_normals_point_up() {
        let raw = blank_tile(32, 48, GRASS, 0.0, 0).write();
        let adt = Adt::parse(&raw).unwrap();
        for chunk in &adt.chunks {
            for n in &chunk.normals {
                assert!(n[2] > 0.99, "{n:?} does not point up");
            }
        }
    }

    /// One layer, no alpha map, no shadow, no liquid — the shipped shape for a
    /// chunk painted with a single texture, and the reason the file is small.
    #[test]
    fn a_blank_chunk_carries_only_the_four_regions_it_needs() {
        let tile = blank_tile(32, 48, GRASS, 0.0, 0);
        let chunk = &tile.chunks[0];
        for present in [Region::Heights, Region::Normals, Region::Layers, Region::Refs] {
            assert!(chunk.region(present).is_some(), "{present:?} is missing");
        }
        for absent in [
            Region::Shadow,
            Region::Alpha,
            Region::Liquid,
            Region::Emitters,
            Region::Colours,
        ] {
            assert!(chunk.region(absent).is_none(), "{absent:?} should be absent");
        }
        assert_eq!(chunk.head().layer_count(), 1);
        assert_eq!(chunk.head().doodad_ref_count(), 0);
    }

    /// The area id reaches every chunk, since that is what a zone tool would
    /// otherwise have to set 256 times on a tile somebody just made.
    #[test]
    fn every_chunk_takes_the_area_given() {
        let raw = blank_tile(32, 48, GRASS, 0.0, 12).write();
        let adt = Adt::parse(&raw).unwrap();
        assert!(adt.chunks.iter().all(|c| c.area_id == 12));
    }
}
