//! The round trip, which is the only check that says a writer is lossless.
//!
//! A writer can be wrong in two ways and only one of them is visible. It can
//! produce a file nothing loads, which fails immediately; or it can produce a
//! file that loads with a region silently dropped, which fails the next time
//! somebody flies over that tile. Comparing the bytes catches both.

use super::*;
use crate::adt::file::{FIRST_REGION, REGIONS};
use crate::adt::heights;

/// The tiles the checks below run over, when the archives are on this machine.
/// Two maps, and one tile that is mostly water.
const TILES: [(&str, u32, u32); 5] = [
    ("Azeroth", 32, 48),
    ("Azeroth", 34, 51),
    ("Azeroth", 40, 23),
    ("Kalimdor", 30, 41),
    ("Kalimdor", 44, 32),
];

/// The archives, or `None` on a machine that has no install.
///
/// `VALE_GAMEDATA` moves the folder; otherwise it is the `Data\` beside this
/// repository, which is where the drop-in rule puts it.
fn archives() -> Option<vale_assets::Assets> {
    let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("Data")
            .to_string_lossy()
            .into_owned()
    });
    match vale_assets::Assets::open(&root) {
        Ok(assets) => Some(assets),
        Err(e) => {
            eprintln!("no archives at {root}: {e} — the tile checks did not run");
            None
        }
    }
}

/// Every tile in [`TILES`] the archives answer for, parsed.
fn real_tiles() -> Vec<(String, Vec<u8>)> {
    let Some(mut assets) = archives() else {
        return Vec::new();
    };
    TILES
        .iter()
        .filter_map(|&(map, x, y)| {
            let path = vale_assets::adt_path(map, x, y);
            assets.read(&path).ok().map(|bytes| (path, bytes))
        })
        .collect()
}

/// A tile with nothing in it, for the checks that are about one region rather
/// than about a real file.
pub(super) fn empty_tile() -> AdtFile {
    AdtFile {
        version: 18,
        header: [0; 16],
        textures: Vec::new(),
        models: Vec::new(),
        model_offsets: Vec::new(),
        buildings: Vec::new(),
        building_offsets: Vec::new(),
        doodads: Vec::new(),
        placements: Vec::new(),
        chunks: Vec::new(),
    }
}

/// **A region that changes length moves everything after it, and the indices
/// follow.**
///
/// This is the check the whole of [`crate::adt::alpha`] rests on and it is the
/// one thing the writer was built for and had never been made to do. Heights and
/// placements are edited *in place*: 580 bytes stay 580 bytes and the file's
/// shape never moves. Painting a fourth texture onto a chunk adds sixteen bytes
/// of `MCLY` and two kilobytes of `MCAL` in the middle of a two-megabyte file,
/// which moves the rest of that chunk's regions, all 255 chunks after it, every
/// `MCIN` offset and every region offset in every one of those headers.
///
/// A writer that got any of that wrong would produce a file that still parses —
/// the regions are found through the offsets that moved with them — and draws
/// the wrong thing, or nothing, from the first chunk after the edit. So the check
/// is a **full re-parse and a field-by-field comparison** rather than a byte
/// count: every chunk's heights, layer count and region set has to come back as
/// it went in, with only the painted chunk different.
#[test]
fn a_resized_alpha_region_moves_everything_after_it() {
    use crate::adt::alpha;
    use vale_assets::world::adt::{TextureLayer, ALPHA_LEN};

    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        // A chunk in the middle, so there are chunks on both sides of the edit.
        let which = 128.min(tile.chunks.len().saturating_sub(1));
        let was = alpha::paint(tile.chunk(which).expect("a chunk"));
        if was.is_empty() || was.len() >= alpha::MAX_LAYERS {
            continue;
        }
        let grew_by = was.stride;

        let mut painted = tile.clone();
        let texture = painted.name_texture("Tileset\\Generic\\Black.blp");
        let mut now = was.clone();
        now.layers.push(TextureLayer {
            texture_id: texture,
            flags: 0,
            alpha_offset: 0,
            effect_id: 0,
        });
        now.maps.push(vec![0x77u8; ALPHA_LEN]);
        alpha::set_paint(painted.chunk_mut(which).expect("a chunk"), &now);

        let written = painted.write();
        assert!(
            written.len() > bytes.len(),
            "{path}: a new layer has to make the file longer"
        );
        let back = AdtFile::parse(&written).unwrap_or_else(|e| panic!("{path}: {e}"));
        // **`MHDR`'s eight offsets are the writer's and the rest is the
        // tile's.** The in-memory header still holds the offsets the file was
        // read with, which is exactly the point: naming a texture lengthened
        // `MTEX`, so every chunk listed after it has to have moved by that much
        // and nothing else in the header may have changed at all.
        let grew = (painted.textures.len() - tile.textures.len()) as u32;
        assert!(grew > 0, "{path}");
        assert_eq!(back.header[0], painted.header[0], "{path}: MHDR flags");
        assert_eq!(back.header[9..], painted.header[9..], "{path}: MHDR tail");
        assert_eq!(back.header[2], painted.header[2], "{path}: MTEX did not move");
        for field in 3..=8 {
            assert_eq!(
                back.header[field],
                painted.header[field] + grew,
                "{path}: MHDR field {field}"
            );
        }
        assert_eq!(back.textures, painted.textures, "{path}: MTEX");
        assert_eq!(back.chunks.len(), painted.chunks.len(), "{path}");

        // The painted chunk carries the new layer, at the stride it had.
        let now = alpha::paint(back.chunk(which).expect("a chunk"));
        assert_eq!(now.len(), was.len() + 1, "{path}");
        assert_eq!(now.stride, was.stride, "{path}: the stride is the chunk's");
        assert_eq!(now.maps[was.len()], vec![0x77u8; ALPHA_LEN], "{path}");
        assert_eq!(back.chunk(which).unwrap().head().layer_count(), now.len() as u32);
        let mcal = back
            .chunk(which)
            .and_then(|c| c.region(Region::Alpha))
            .map(|sub| sub.data.len())
            .unwrap_or(0);
        assert_eq!(
            mcal,
            was.maps.len().saturating_sub(1) * grew_by + grew_by,
            "{path}: one map per layer above the base"
        );

        // …and the painted chunk kept everything that is not paint. A region
        // whose offset the writer recomputed to the wrong place comes back as
        // the bytes of whatever is next, which is why this is compared and not
        // merely re-read.
        for region in REGIONS {
            if matches!(region, Region::Layers | Region::Alpha) {
                continue;
            }
            assert_eq!(
                back.chunk(which).and_then(|c| c.region(region)),
                tile.chunk(which).and_then(|c| c.region(region)),
                "{path}: chunk {which}'s {region:?} did not survive being painted"
            );
        }

        // …and every other chunk is exactly what it was, byte for byte, which is
        // what says the offsets after the edit all moved together rather than
        // apart.
        for i in 0..tile.chunks.len() {
            if i == which {
                continue;
            }
            assert_eq!(
                back.chunk(i),
                tile.chunk(i),
                "{path}: chunk {i} moved when chunk {which} was painted"
            );
        }
    }
}

/// **A moved placement survives being written and read again**, and moving it
/// changes nothing else about the tile.
///
/// This is the check the doodad tool leans on and it is a different one from the
/// round trip above. That one says a tile written back unchanged is unchanged;
/// this says a tile written back with **one** record changed differs in that
/// record and in nothing else — which is the failure a placement edit actually
/// has, because `MDDF` sits between two other lists and above 256 chunks of
/// references into it.
///
/// It also pins the two conversions the editor's panel is written in terms of.
/// A record is edited in *world* coordinates and stored in the file's own, so a
/// swapped axis is a doodad that lands somewhere else entirely — and the
/// direction that is not exercised by simply loading a tile is exactly the one
/// an edit uses.
#[test]
fn a_moved_placement_survives_the_write_and_moves_nothing_else() {
    use vale_assets::world::adt::{placement_from_world, placement_to_world};

    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        let before = tile.doodad_list();
        if before.is_empty() {
            continue;
        }

        // The first placement, moved five yards north and one yard up — through
        // the world, which is how the panel does it.
        let mut moved = tile;
        let mut list = before.clone();
        let world = placement_to_world(list[0].position);
        list[0].position = placement_from_world([world[0] + 5.0, world[1], world[2] + 1.0]);
        list[0].rotation[1] = (list[0].rotation[1] + 90.0).rem_euclid(360.0);
        list[0].scale = list[0].scale.saturating_add(256);
        let wanted = list[0];
        moved.set_doodad_list(&list);

        // Written, read again, and compared record by record.
        let again = AdtFile::parse(&moved.write()).expect("the edited tile parses");
        let after = again.doodad_list();
        assert_eq!(
            after.len(),
            before.len(),
            "{path}: the list changed length"
        );
        let there = placement_to_world(after[0].position);
        let asked = placement_to_world(wanted.position);
        for axis in 0..3 {
            assert!(
                (there[axis] - asked[axis]).abs() < 1e-2,
                "{path}: moved to {there:?} rather than {asked:?}"
            );
        }
        assert_eq!(after[0].rotation, wanted.rotation, "{path}: the turn was lost");
        assert_eq!(after[0].scale, wanted.scale, "{path}: the scale was lost");
        assert_eq!(
            after[0].unique_id, before[0].unique_id,
            "{path}: the id changed, so the renderer would no longer know it"
        );
        for (index, (was, now)) in before.iter().zip(&after).enumerate().skip(1) {
            assert_eq!(was, now, "{path}: placement {index} moved and was not asked to");
        }

        // …and every other region is byte for byte what it was, which is the
        // half that catches a writer that renumbered something it should not
        // have. `MDDF` is the one that is allowed to differ.
        let original = AdtFile::parse(&bytes).expect("a shipped tile parses");
        for chunk in 0..original.chunks.len() {
            let (a, b) = (original.chunk(chunk), again.chunk(chunk));
            let (Some(a), Some(b)) = (a, b) else { continue };
            assert_eq!(
                a.region(Region::Refs).map(|r| &r.data),
                b.region(Region::Refs).map(|r| &r.data),
                "{path}: chunk {chunk}'s references were rewritten by a move"
            );
        }
        assert_eq!(
            original.models, again.models,
            "{path}: the model name list was rewritten by a move"
        );
    }
}

/// **No shipped 1.12 tile carries an `MCCV`**, and the header slot its offset
/// lives in is empty on every chunk.
///
/// The measurement the whole vertex-shading subject rests on. If a shipped tile
/// did carry one, adding the region would be editing something the reference
/// reads and the deviation could not be stated the way it is; because none does,
/// every chunk this crate shades is a chunk the region is *added* to, and a tile
/// nobody has shaded is untouched.
#[test]
fn no_shipped_tile_has_vertex_shading() {
    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        for (i, chunk) in tile.chunks.iter().enumerate() {
            assert!(
                chunk.region(Region::Colours).is_none(),
                "{path} chunk {i} carries an MCCV"
            );
            let head = chunk.head();
            assert_eq!(
                head.flags() & crate::adt::file::FLAG_HAS_COLOURS,
                0,
                "{path} chunk {i} claims an MCCV in its flags"
            );
        }
        // …and the three dwords 3.x puts `ofsMCCV`, `ofsMCLV` and a spare in are
        // zero, which is what makes writing the first of them safe.
        for chunk in &tile.chunks {
            for at in [0x74usize, 0x78, 0x7c] {
                let word = u32::from_le_bytes(chunk.header[at..at + 4].try_into().unwrap());
                assert_eq!(word, 0, "{path}: header 0x{at:02x} is not empty");
            }
        }
    }
}

/// **Shading a chunk survives the write, and shading it back to neutral undoes
/// the file as well as the picture.**
///
/// Three claims in one test because they are one property: the region can be
/// added, it reads back as what was written, and taking it away leaves the tile
/// byte for byte what it was. The last of those is the one that matters — a
/// stroke that is undone should not leave 580 bytes of 0x7F behind on a tile
/// that shipped without them.
#[test]
fn shading_a_chunk_round_trips_and_clearing_it_undoes_the_file() {
    use crate::adt::colours;

    for (path, bytes) in real_tiles() {
        let original = AdtFile::parse(&bytes).expect("a shipped tile parses");
        let mut tile = AdtFile::parse(&bytes).expect("a shipped tile parses");

        // A colour with three different channels, because the file's order is
        // blue, green, red and a swap is invisible on grey.
        let painted: Vec<colours::Colour> = (0..vale_assets::world::adt::HEIGHTS_PER_CHUNK)
            .map(|i| [10 + (i % 7) as u8, 90, 200, colours::NEUTRAL])
            .collect();
        colours::set_all(tile.chunk_mut(3).expect("a tile has 256 chunks"), &painted);

        let written = tile.write();
        let again = AdtFile::parse(&written).expect("the shaded tile parses");
        let chunk = again.chunk(3).expect("still there");
        assert!(
            chunk.region(Region::Colours).is_some(),
            "{path}: the region did not survive the write"
        );
        assert_eq!(
            chunk.head().flags() & crate::adt::file::FLAG_HAS_COLOURS,
            crate::adt::file::FLAG_HAS_COLOURS,
            "{path}: the flag did not follow the region"
        );
        assert_eq!(
            colours::colours(chunk),
            painted,
            "{path}: the shading came back different"
        );
        // Every other chunk is untouched, which is what says the added region
        // did not disturb the offsets of anything around it.
        for i in 0..again.chunks.len() {
            if i == 3 {
                continue;
            }
            assert!(
                again.chunk(i).and_then(|c| c.region(Region::Colours)).is_none(),
                "{path}: chunk {i} grew an MCCV it was not given"
            );
        }
        for region in REGIONS {
            if region == Region::Colours {
                continue;
            }
            assert_eq!(
                original.chunk(3).and_then(|c| c.region(region)),
                again.chunk(3).and_then(|c| c.region(region)),
                "{path}: {} changed when only the shading was asked for",
                String::from_utf8_lossy(&region.magic()[..])
            );
        }

        // …and taking it away puts the file back exactly.
        let mut undone = AdtFile::parse(&written).expect("the shaded tile parses");
        colours::clear(undone.chunk_mut(3).expect("still there"));
        assert_eq!(
            undone.write(),
            bytes,
            "{path}: a cleared tile is not the tile that shipped"
        );
    }
}

/// **A shading stroke moves the vertices under it and no others**, and it
/// creates the region on the way.
///
/// The brush's own check, over a real tile rather than a fixture: what is easy
/// to get wrong here is not the arithmetic but the *reach* — a stroke that used
/// the wrong vertex positions would shade a neat circle somewhere else, which
/// looks entirely deliberate.
#[test]
fn a_shading_stroke_darkens_what_is_under_it() {
    use crate::adt::colours;
    use crate::ops::{Shade, Shading, Working};

    let Some((path, bytes)) = real_tiles().into_iter().next() else {
        return;
    };
    let mut tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
    let origin = tile.chunk(0).expect("a tile has chunks").head().position();
    // The middle of chunk 0, which is a quarter of a chunk in from its origin
    // along both axes — the axes run *away* from it.
    let at = [origin[0] - 8.0, origin[1] - 8.0];

    let brush = Shading {
        radius: 10.0,
        strength: 4.0,
        mode: Shade::Darken,
        ..Shading::default()
    };
    let mut working = Working::default();
    // A second of stroke at four times the remaining distance a second: enough
    // to be unmistakably darker without relying on how many frames it took.
    let edits = brush.stroke(&mut tile, at, 1.0, &mut working, None);
    assert!(!edits.is_empty(), "{path}: the stroke changed nothing");

    let chunk = tile.chunk(0).expect("still there");
    assert!(
        chunk.region(Region::Colours).is_some(),
        "{path}: the stroke did not create the region"
    );
    let after = colours::colours(chunk);
    let mut darkened = 0;
    for (i, colour) in after.iter().enumerate() {
        let (dx, dy) = crate::adt::heights::vertex_offset(i);
        let (x, y) = (origin[0] - dx, origin[1] - dy);
        let far = ((x - at[0]).powi(2) + (y - at[1]).powi(2)).sqrt();
        match far <= brush.radius {
            // Inside the brush: darker, or at worst unchanged right on the rim
            // where the falloff is zero.
            true => {
                assert!(
                    colour[0] <= colours::NEUTRAL,
                    "{path}: vertex {i} at {far:.1} yd got brighter"
                );
                if colour[0] < colours::NEUTRAL {
                    darkened += 1;
                }
            }
            // Outside it: untouched, which is the half that catches a brush
            // aimed at the wrong vertices.
            false => assert_eq!(
                *colour,
                colours::NEUTRAL_COLOUR,
                "{path}: vertex {i} at {far:.1} yd is outside a {} yd brush",
                brush.radius
            ),
        }
    }
    assert!(darkened > 4, "{path}: only {darkened} vertices moved");
}

/// Parsing a tile and writing it back produces the bytes it was given.
#[test]
fn a_real_tile_round_trips_byte_for_byte() {
    let tiles = real_tiles();
    if tiles.is_empty() {
        return;
    }
    for (path, bytes) in &tiles {
        let tile = AdtFile::parse(bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        let written = tile.write();
        assert_eq!(
            written.len(),
            bytes.len(),
            "{path}: wrote {} bytes for a tile of {}",
            written.len(),
            bytes.len()
        );
        if written != *bytes {
            let at = written
                .iter()
                .zip(bytes.iter())
                .position(|(a, b)| a != b)
                .expect("the lengths match and the contents do not");
            panic!("{path}: first difference at byte {at}");
        }
    }
}

/// **A height stroke leaves the placements the same, and a moved doodad does
/// not** — which is what decides whether the server's vmap half is rebuilt
/// for a tile (`AdtFile::same_placements`).
#[test]
fn a_terrain_edit_keeps_the_placements_and_a_moved_doodad_does_not() {
    let tiles = real_tiles();
    let Some((path, bytes)) = tiles.iter().find(|(_, bytes)| {
        AdtFile::parse(bytes).is_ok_and(|tile| !tile.doodads.is_empty())
    }) else {
        return;
    };
    let shipped = AdtFile::parse(bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut raised = shipped.clone();
    let chunk = raised.chunk_mut(0).expect("a chunk");
    let mut heights = super::heights::heights(chunk);
    for h in heights.iter_mut() {
        *h += 5.0;
    }
    super::heights::set_heights(chunk, &heights);
    assert_ne!(raised.write(), shipped.write(), "{path}: the stroke changed the tile");
    assert!(shipped.same_placements(&raised), "{path}: a height stroke places nothing");

    let mut moved = shipped.clone();
    moved.doodads[4] ^= 0x01;
    assert!(!shipped.same_placements(&moved), "{path}: a moved doodad is a placement change");
}

/// The layout every map chunk in those tiles has, which the writer assumes.
///
/// The order is not the order the header lists the offsets in, and `MCNR` is
/// followed by thirteen bytes no size field covers. Both are the reason
/// [`super::file::parse_chunk`] places regions by their offsets and never by
/// walking sub-chunk sizes.
#[test]
fn every_map_chunk_has_the_regions_in_one_order_and_one_padding() {
    let tiles = real_tiles();
    if tiles.is_empty() {
        return;
    }
    let mut chunks = 0;
    for (path, bytes) in &tiles {
        let tile = AdtFile::parse(bytes).unwrap();
        assert_eq!(tile.chunks.len(), 256, "{path}");
        for (i, chunk) in tile.chunks.iter().enumerate() {
            // **The eight regions a 1.12 chunk is made of**, which is every
            // region but the one this crate adds: no shipped tile has an
            // `MCCV`, and `no_shipped_tile_has_vertex_shading` is where that is
            // asserted on its own terms.
            let shipped: Vec<Region> = REGIONS
                .into_iter()
                .filter(|&r| r != Region::Colours)
                .collect();
            let present: Vec<Region> = shipped
                .iter()
                .copied()
                .filter(|&r| chunk.region(r).is_some())
                .collect();
            assert_eq!(present, shipped, "{path} chunk {i}");
            for region in shipped.iter().copied() {
                let sub = chunk.region(region).unwrap();
                let want = if region == Region::Normals { 13 } else { 0 };
                assert_eq!(
                    sub.padding.len(),
                    want,
                    "{path} chunk {i}: {:?} padding",
                    region
                );
            }
            let head = chunk.head();
            assert_eq!(head.index(), ((i % 16) as u32, (i / 16) as u32), "{path}");
            assert_eq!(
                head.layer_count() as usize * 16,
                chunk.region(Region::Layers).unwrap().data.len(),
                "{path} chunk {i}: nLayers against MCLY"
            );
            chunks += 1;
        }
    }
    assert!(chunks >= 256, "only {chunks} chunks were checked");
}

/// The first region begins immediately after the header, on every chunk.
#[test]
fn the_first_region_follows_the_header() {
    let tiles = real_tiles();
    if tiles.is_empty() {
        return;
    }
    for (path, bytes) in &tiles {
        let tile = AdtFile::parse(bytes).unwrap();
        for (i, chunk) in tile.chunks.iter().enumerate() {
            let offset = u32::from_le_bytes(
                chunk.header[Region::Heights.offset_field()
                    ..Region::Heights.offset_field() + 4]
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(offset, FIRST_REGION, "{path} chunk {i}");
        }
    }
}

/// A height edit changes the heights and the length of nothing.
///
/// `MCVT` is a fixed 580 bytes, so raising ground moves no other region and the
/// tile comes out the same size. That is what makes the height brush the safest
/// tool to write first.
#[test]
fn raising_a_chunk_changes_its_heights_and_not_its_length() {
    let tiles = real_tiles();
    if tiles.is_empty() {
        return;
    }
    let (path, bytes) = &tiles[0];
    let mut tile = AdtFile::parse(bytes).unwrap();
    let before = heights::heights(tile.chunk(70).unwrap());
    let raised: Vec<f32> = before.iter().map(|h| h + 5.0).collect();
    heights::set_heights(tile.chunk_mut(70).unwrap(), &raised);

    let written = tile.write();
    assert_eq!(written.len(), bytes.len(), "{path}");
    let again = AdtFile::parse(&written).unwrap();
    let after = heights::heights(again.chunk(70).unwrap());
    for (i, (a, b)) in raised.iter().zip(after.iter()).enumerate() {
        assert!(
            (a - b).abs() < 1e-3,
            "{path} vertex {i}: wrote {a}, read back {b}"
        );
    }
    // …and every other chunk is untouched.
    assert_eq!(
        heights::heights(again.chunk(71).unwrap()),
        heights::heights(AdtFile::parse(bytes).unwrap().chunk(71).unwrap())
    );
}

/// Flat ground gets a normal pointing straight up, and a slope gets one leaning
/// away from the rise.
#[test]
fn recomputed_normals_point_away_from_the_slope() {
    let tiles = real_tiles();
    if tiles.is_empty() {
        return;
    }
    let (_, bytes) = &tiles[0];
    let mut tile = AdtFile::parse(bytes).unwrap();

    // Flat: every vertex at one height.
    let flat = vec![100.0f32; heights::VERTICES];
    heights::set_heights(tile.chunk_mut(70).unwrap(), &flat);
    heights::recompute_normals(&mut tile, 70);
    let normals = &tile.chunk(70).unwrap().region(Region::Normals).unwrap().data;
    // A vertex on the edge of the chunk takes half its gradient from the chunk
    // next door, which was not flattened, so only the interior is asserted. That
    // reach across the boundary is the point of it: without it a flattened chunk
    // would be lit as though its neighbours were flat too.
    for row in 1..8 {
        for col in 1..8 {
            let i = heights::outer(row, col).unwrap();
            let n = [
                normals[i * 3] as i8,
                normals[i * 3 + 1] as i8,
                normals[i * 3 + 2] as i8,
            ];
            assert_eq!(n, [0, 0, 127], "vertex {i} of flat ground is not facing up");
        }
    }

    // A ramp rising toward decreasing world x: the ground climbs as the row
    // index grows, so the normal leans toward increasing x, which is +x.
    let ramp: Vec<f32> = (0..heights::VERTICES)
        .map(|i| 100.0 + heights::vertex_offset(i).0)
        .collect();
    heights::set_heights(tile.chunk_mut(70).unwrap(), &ramp);
    heights::recompute_normals(&mut tile, 70);
    let normals = &tile.chunk(70).unwrap().region(Region::Normals).unwrap().data;
    let centre = heights::outer(4, 4).unwrap();
    let n = [
        normals[centre * 3] as i8,
        normals[centre * 3 + 1] as i8,
        normals[centre * 3 + 2] as i8,
    ];
    assert!(n[0] > 0, "a ramp climbing away from +x leans back toward it: {n:?}");
    assert!(n[1].abs() <= 1, "the ramp has no slope across y: {n:?}");
    assert!(n[2] > 0, "the normal points up: {n:?}");
}

/// Removing a placement renumbers the references above it in every chunk.
#[test]
fn removing_a_doodad_renumbers_the_references() {
    let tiles = real_tiles();
    if tiles.is_empty() {
        return;
    }
    let (_, bytes) = &tiles[0];
    let mut tile = AdtFile::parse(bytes).unwrap();
    let before = tile.doodad_list();
    assert!(before.len() > 3, "the tile has too few doodads to check");

    // The chunk that references the doodad after the one being removed, so that
    // the renumbering has somewhere to show.
    let removed = 1usize;
    let watched: Vec<usize> = (0..256)
        .filter(|&i| tile.refs(i).0.contains(&2))
        .collect();

    let gone = tile.remove_doodad(removed).unwrap();
    assert_eq!(gone, before[removed]);
    assert_eq!(tile.doodad_list().len(), before.len() - 1);
    for chunk in watched {
        assert!(
            tile.refs(chunk).0.contains(&1),
            "chunk {chunk} referenced doodad 2 and should now reference 1"
        );
        assert!(!tile.refs(chunk).0.contains(&(before.len() as u32 - 1)));
    }
    // …and the file it writes still reads back as what it says.
    let again = AdtFile::parse(&tile.write()).unwrap();
    assert_eq!(again.doodad_list(), tile.doodad_list());
    for i in 0..256 {
        assert_eq!(again.refs(i), tile.refs(i), "chunk {i}");
    }
}

/// The vertex layout agrees with `vale_assets`: nine outer rows interleaved
/// with eight inner ones, running along decreasing world x and y.
#[test]
fn the_vertex_grid_is_nine_by_nine_interleaved_with_eight_by_eight() {
    use vale_assets::world::adt::UNIT_SIZE;
    assert_eq!(heights::VERTICES, 145);
    assert_eq!(heights::outer(0, 0), Some(0));
    assert_eq!(heights::inner(0, 0), Some(9));
    assert_eq!(heights::outer(1, 0), Some(17));
    assert_eq!(heights::outer(8, 8), Some(144));
    assert_eq!(heights::outer(9, 0), None);
    assert_eq!(heights::inner(8, 0), None);

    assert_eq!(heights::vertex_offset(0), (0.0, 0.0));
    let (dx, dy) = heights::vertex_offset(heights::inner(0, 0).unwrap());
    assert!((dx - 0.5 * UNIT_SIZE).abs() < 1e-3 && (dy - 0.5 * UNIT_SIZE).abs() < 1e-3);
    let (dx, dy) = heights::vertex_offset(heights::outer(8, 8).unwrap());
    assert!((dx - 8.0 * UNIT_SIZE).abs() < 1e-3 && (dy - 8.0 * UNIT_SIZE).abs() < 1e-3);
}

/// The three size fields, and which of them counts the sub-chunk header.
#[test]
fn only_the_alpha_and_liquid_sizes_count_their_own_header() {
    assert_eq!(Region::Alpha.size_field(), Some((0x28, true)));
    assert_eq!(Region::Shadow.size_field(), Some((0x30, false)));
    assert_eq!(Region::Liquid.size_field(), Some((0x64, true)));
    assert_eq!(Region::Heights.size_field(), None);
}

/// A file this writer could not put back together is refused rather than
/// half-read.
#[test]
fn an_unknown_top_level_chunk_is_refused() {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"REVM");
    buf.extend_from_slice(&4u32.to_le_bytes());
    buf.extend_from_slice(&18u32.to_le_bytes());
    buf.extend_from_slice(b"O2HM");
    buf.extend_from_slice(&0u32.to_le_bytes());
    let error = AdtFile::parse(&buf).unwrap_err().to_string();
    assert!(error.contains("MH2O"), "{error}");
}

/// Every point over a tile answers a height, including the last two
/// centimetres of it.
///
/// The chunk origins in the file step by 33.332 yards where `CHUNK_SIZE` is
/// 33.3333, so sixteen of them span 533.314 against the tile's 533.333 and a
/// strict containment test refuses the strip at the far edge. What that looks
/// like in the editor is a brush ring whose vertices jump to an unrelated
/// height wherever they cross it. See [`heights::height_at`]'s `REACH`.
#[test]
fn the_whole_tile_answers_a_height() {
    let tiles = real_tiles();
    if tiles.is_empty() {
        return;
    }
    let (path, bytes) = &tiles[0];
    let tile = AdtFile::parse(bytes).unwrap();
    let origin = tile.chunk(0).unwrap().head().position();
    let size = vale_assets::world::adt::TILE_SIZE;
    // The corners of the tile, and its two far edges, which is where the gap is.
    let mut refused = Vec::new();
    for step in 0..=64 {
        let t = step as f32 / 64.0;
        for (x, y) in [
            (origin[0] - size + 0.001, origin[1] - t * size),
            (origin[0] - t * size, origin[1] - size + 0.001),
            (origin[0] - 0.001, origin[1] - t * size),
            (origin[0] - t * size, origin[1] - 0.001),
        ] {
            if heights::height_at(&tile, x, y).is_none() {
                refused.push((x, y));
            }
        }
    }
    assert!(
        refused.is_empty(),
        "{path}: {} points over the tile answered no height, first {:?}",
        refused.len(),
        refused.first()
    );
}

/// **One placement reads the same as its row in the whole list**, which is what
/// lets the two things that ask about a single record every frame — a panel
/// checking it is still in step, and every edit reading the value it replaces —
/// avoid parsing all 1,400 of them to do it.
#[test]
fn one_placement_reads_the_same_as_the_whole_list() {
    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        let list = tile.doodad_list();
        for (index, record) in list.iter().enumerate() {
            assert_eq!(tile.doodad_at(index).as_ref(), Some(record), "{path} {index}");
        }
        assert_eq!(tile.doodad_at(list.len()), None, "{path}: past the end");
    }
}

/// **A placement moved to another tile comes out of one file and into the
/// other, keeping its id**, and both files survive the write.
///
/// This is the file half of `vale_ide::tools::rehome`, and the reason it
/// exists is a rule in the renderer: **the tile containing a placement's origin
/// is the tile that draws it**, because a model touching two tiles is listed in
/// both with one `unique_id` and exactly one of them has to claim it. So a
/// placement dragged past a border whose record stayed behind is a placement
/// nobody draws — the tile that holds it no longer claims it, and the tile that
/// would has no record to claim.
///
/// The two halves are different operations and each renumbers something:
/// removing renumbers every `MCRF` reference above it in 256 chunks of one file,
/// and adding appends to `MMDX`, `MMID` and `MDDF` in the other under *that*
/// tile's own numbering — a `name_id` means nothing outside the file it is in.
#[test]
fn a_placement_moved_between_tiles_keeps_its_id_and_both_files_survive() {
    let tiles = real_tiles();
    let (Some((from_path, from_bytes)), Some((to_path, to_bytes))) =
        (tiles.first(), tiles.get(1))
    else {
        return;
    };
    let mut from = AdtFile::parse(from_bytes).expect("a shipped tile parses");
    let mut to = AdtFile::parse(to_bytes).expect("a shipped tile parses");
    if from.doodad_list().is_empty() {
        return;
    }

    let record = from.doodad_at(0).expect("a placement");
    let path = from.model_names()[record.name_id as usize].clone();
    let was_from = from.doodad_list().len();
    let was_to = to.doodad_list().len();
    let names_to = to.model_names().len();

    // Out of one…
    assert_eq!(from.remove_doodad(0), Some(record), "{from_path}");
    assert_eq!(from.doodad_list().len(), was_from - 1);
    assert!(
        !from.doodad_list().iter().any(|d| d.unique_id == record.unique_id),
        "{from_path}: it is gone from the list it left"
    );

    // …and into the other, under that tile's own numbering.
    let mut moved = record;
    moved.name_id = to.name_model(&path);
    let index = to.add_doodad(moved, 20.0);
    assert_eq!(to.doodad_list().len(), was_to + 1, "{to_path}");
    let landed = to.doodad_at(index).expect("the new row");
    assert_eq!(landed.unique_id, record.unique_id, "the id is kept");
    assert_eq!(
        to.model_names()[landed.name_id as usize],
        path,
        "{to_path}: and it resolves to the same model"
    );
    // The name list grew by at most one — by nothing at all if that tile already
    // named the model, which for a tree on a seam it usually does.
    assert!(to.model_names().len() <= names_to + 1);

    // Both files still parse, and every chunk's references still point inside
    // the lists they index — which is what the renumbering is for and the one
    // thing a mistake here does not otherwise show.
    for (path, tile) in [(from_path, &from), (to_path, &to)] {
        let written = tile.write();
        let back = AdtFile::parse(&written).unwrap_or_else(|e| panic!("{path}: {e}"));
        // **Not `back == tile`.** Every offset and size in `MHDR`, `MCIN` and
        // the 256 chunk headers is the *writer's*, and the in-memory copy still
        // holds the ones the file was read with — which is the point, since both
        // lists have changed length. Writing what came back is the comparison
        // that means anything: it says the file describes itself. What is
        // compared beside it is the content those offsets lead to.
        assert_eq!(back.write(), written, "{path}: does not survive the write");
        assert_eq!(back.doodad_list(), tile.doodad_list(), "{path}: MDDF");
        assert_eq!(back.model_names(), tile.model_names(), "{path}: MMDX");
        let doodads = back.doodad_list().len() as u32;
        let buildings = back.building_list().len() as u32;
        for chunk in 0..back.chunks.len() {
            let (refs, wmos) = back.refs(chunk);
            assert!(
                refs.iter().all(|&at| at < doodads),
                "{path}: chunk {chunk} references an MDDF entry that is not there"
            );
            assert!(
                wmos.iter().all(|&at| at < buildings),
                "{path}: chunk {chunk} references an MODF entry that is not there"
            );
        }
    }
}

/// **A minted id is above everything the game ships and above every other
/// minted one.**
///
/// It matters more than it used to. The rule that keeps a placement from being
/// drawn twice is that one id names one object, so two objects sharing an id are
/// one object as far as `rehome` is concerned — it would collapse them. Counting
/// up from the largest id in the tiles a tool happens to have open would hand out
/// a number a tile over the horizon is already using.
#[test]
fn a_minted_id_is_above_the_shipped_ones() {
    use crate::adt::place::MINTED_BASE;

    for (path, bytes) in real_tiles() {
        let mut tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        // Nothing the game ships is in the minted range…
        assert_eq!(tile.highest_minted_id(), None, "{path}");
        let shipped = tile.next_unique_id();
        assert!(shipped < MINTED_BASE, "{path}: {shipped} is in the minted range");

        // …and a placement given a minted id is found by it, whichever list it
        // went into, because the range is shared between the two.
        let Some(first) = tile.doodad_at(0) else { continue };
        let mut minted = first;
        minted.unique_id = MINTED_BASE;
        tile.add_doodad(minted, 10.0);
        assert_eq!(tile.highest_minted_id(), Some(MINTED_BASE), "{path}");

        if let Some(building) = tile.building_at(0) {
            let mut minted = building;
            minted.unique_id = MINTED_BASE + 5;
            tile.add_building(minted);
            assert_eq!(
                tile.highest_minted_id(),
                Some(MINTED_BASE + 5),
                "{path}: the range is shared between MDDF and MODF"
            );
        }
    }
}

/// **A placed doodad survives the write and is referenced by the chunks under
/// it.** `MCRF` is the list the reference client culls by, and this client never
/// reads it — so a placement added without it draws here and is invisible there.
#[test]
fn a_placed_doodad_is_written_and_referenced() {
    use crate::adt::place::MINTED_BASE;

    for (path, bytes) in real_tiles() {
        let mut tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        let Some(model) = tile.model_names().first().cloned() else {
            continue;
        };
        let was = tile.doodad_list().len();

        // In the middle of chunk 128, which is the middle of the tile.
        let origin = tile.chunk(128).expect("a chunk").head().position();
        let world = [origin[0] - 16.0, origin[1] - 16.0, 50.0];
        let record = Doodad {
            name_id: tile.name_model(&model),
            unique_id: MINTED_BASE,
            position: vale_assets::world::adt::placement_from_world(world),
            rotation: [0.0, 90.0, 0.0],
            scale: 1024,
            flags: 0,
        };
        let index = tile.add_doodad(record, 12.0);
        assert_eq!(index, was, "it goes on the end");
        assert_eq!(tile.doodad_at(index), Some(record));

        // The chunk it stands on references it, which is the half `MCRF` is for.
        let refs = tile.refs(128).0;
        assert!(
            refs.contains(&(index as u32)),
            "{path}: chunk 128 does not reference the placement standing on it"
        );

        // …and the file still describes itself.
        let written = tile.write();
        let back = AdtFile::parse(&written).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(back.write(), written, "{path}");
        assert_eq!(back.doodad_at(index), Some(record), "{path}");
        assert_eq!(
            back.model_names()[record.name_id as usize],
            model,
            "{path}: and it still resolves to the model it was given"
        );
    }
}

/// **A hole is two bytes, and two bytes are what changes.**
///
/// The smallest edit in this crate, over real tiles, and the check is
/// deliberately the strict one: the written file must differ from the original
/// in exactly the chunk that was cut and in nothing else. A mask written a
/// quadrant out, or written to the wrong chunk, still parses and still draws —
/// it just takes the ground out somewhere nobody asked — so byte equality is the
/// only thing that reports it.
#[test]
fn a_cut_hole_survives_the_write_and_changes_nothing_else() {
    use crate::adt::holes;

    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        let was: Vec<u16> = (0..tile.chunks.len())
            .filter_map(|i| holes::holes(&tile, i))
            .collect();

        // Chunk 70, which is in the middle of every tile, and the bit under a
        // point three quarters of the way across it — so the arithmetic is
        // exercised rather than bit 0 being right by accident.
        let mut cut = tile;
        let origin = cut.chunk(70).expect("256 chunks").head().position();
        let at = [origin[0] - 25.0, origin[1] - 8.0];
        let wanted = holes::with_bit_at(&cut, 70, at[0], at[1], true)
            .expect("the point is over the chunk it was taken from");
        assert_ne!(wanted, was[70], "{path}: the point was already a hole");
        // **Both directions, and both idempotent.** The tool records nothing
        // when the mask comes back unchanged, which is what keeps a drag across
        // ground that is already gone from putting an entry on the stack per
        // frame.
        assert_eq!(
            holes::with_bit_at(&cut, 70, at[0], at[1], false),
            Some(was[70]),
            "{path}: patching where there is no hole changed the mask"
        );
        // A point off the chunk it was taken from names no bit at all rather
        // than the nearest, which would cut a square somewhere else entirely.
        assert_eq!(holes::with_bit_at(&cut, 70, origin[0] + 1.0, origin[1], true), None);
        holes::set_holes(cut.chunk_mut(70).expect("chunk 70"), wanted);

        let again = AdtFile::parse(&cut.write()).expect("the edited tile parses");
        let now: Vec<u16> = (0..again.chunks.len())
            .filter_map(|i| holes::holes(&again, i))
            .collect();
        assert_eq!(now.len(), was.len(), "{path}: the chunk count changed");
        for (index, (had, has)) in was.iter().zip(&now).enumerate() {
            match index == 70 {
                true => assert_eq!(*has, wanted, "{path}: chunk 70's mask is not what was asked"),
                false => assert_eq!(has, had, "{path}: chunk {index}'s holes changed"),
            }
        }

        // …and the bytes: a two-byte field, so a writer that moved anything
        // else has moved it for no reason.
        let written = cut.write();
        assert_eq!(written.len(), bytes.len(), "{path}: the file changed length");
        let differ: Vec<usize> = written
            .iter()
            .zip(&bytes)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(at, _)| at)
            .collect();
        assert!(
            differ.len() <= 2 && differ.windows(2).all(|w| w[1] == w[0] + 1),
            "{path}: {} bytes differ, at {:?}",
            differ.len(),
            &differ[..differ.len().min(8)]
        );

        // Putting it back gives the file it started as, byte for byte.
        holes::set_holes(cut.chunk_mut(70).expect("chunk 70"), was[70]);
        assert_eq!(cut.write(), bytes, "{path}: patching the hole did not restore the tile");
    }
}

/// **The ground a hole came out of is still described by the file**, which is
/// the whole of how the hole tool aims.
///
/// A hole takes cells out of the *mesh*; `MCVT` is untouched. So `height_at`
/// refuses over a cut square — right for anything that asks where a character
/// stands — and `solid_height_at` answers the height the missing cells were at.
/// Without the second one the pointer falls through the moment a square is cut
/// and a held drag walks off and cuts a trail of squares nobody asked for.
#[test]
fn a_cut_square_still_has_a_height_to_aim_at() {
    use crate::adt::{heights, holes};

    for (path, bytes) in real_tiles() {
        let mut tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        // The middle of chunk 70's first square, which every tile has ground at.
        let origin = tile.chunk(70).expect("256 chunks").head().position();
        let at = [origin[0] - 4.0, origin[1] - 4.0];
        let Some(was) = heights::height_at(&tile, at[0], at[1]) else {
            continue;
        };
        assert!(
            heights::solid_height_at(&tile, at[0], at[1])
                .is_some_and(|now| (now - was).abs() < 1e-3),
            "{path}: the two disagree where there is no hole"
        );

        let bit = vale_assets::world::adt::hole_bit(origin, at[0], at[1]).expect("over it");
        let mask = holes::holes(&tile, 70).expect("chunk 70");
        holes::set_holes(tile.chunk_mut(70).expect("chunk 70"), mask | (1 << bit));

        assert_eq!(
            heights::height_at(&tile, at[0], at[1]),
            None,
            "{path}: the drawn ground still answers over a hole"
        );
        assert!(
            heights::solid_height_at(&tile, at[0], at[1])
                .is_some_and(|now| (now - was).abs() < 1e-3),
            "{path}: the ground the hole came out of moved when the hole was cut"
        );
    }
}

/// **An area id is four bytes, and four bytes are what changes.**
///
/// The same strict check the hole mask gets, for the same reason: an id written
/// to the wrong chunk still parses, still draws, and moves the zone boundary
/// somewhere nobody asked. Byte equality is the only thing that reports it.
///
/// It also pins the two things the tool rests on: that `0` survives as a value
/// rather than being treated as "unset", and that a tile's census sees every
/// chunk.
#[test]
fn an_area_id_survives_the_write_and_changes_nothing_else() {
    use crate::adt::area;

    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        let was: Vec<u32> = (0..tile.chunks.len())
            .filter_map(|i| area::area(&tile, i))
            .collect();
        assert_eq!(was.len(), tile.chunks.len(), "{path}: a chunk had no area");
        assert_eq!(
            area::census(&tile).values().sum::<usize>(),
            tile.chunks.len(),
            "{path}: the census lost a chunk"
        );

        // A row id nothing in the shipped table uses, so the change is
        // unmistakable, on a chunk in the middle of the file.
        let mut edited = tile;
        let wanted = 0x00AB_CDEFu32;
        area::set_area(edited.chunk_mut(70).expect("chunk 70"), wanted);

        let again = AdtFile::parse(&edited.write()).expect("the edited tile parses");
        let now: Vec<u32> = (0..again.chunks.len())
            .filter_map(|i| area::area(&again, i))
            .collect();
        for (index, (had, has)) in was.iter().zip(&now).enumerate() {
            match index == 70 {
                true => assert_eq!(*has, wanted, "{path}: chunk 70's area is not what was asked"),
                false => assert_eq!(has, had, "{path}: chunk {index}'s area changed"),
            }
        }

        let written = edited.write();
        assert_eq!(written.len(), bytes.len(), "{path}: the file changed length");
        let differ: Vec<usize> = written
            .iter()
            .zip(&bytes)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(at, _)| at)
            .collect();
        assert!(
            differ.len() <= 4 && differ.windows(2).all(|w| w[1] == w[0] + 1),
            "{path}: {} bytes differ, at {:?}",
            differ.len(),
            &differ[..differ.len().min(8)]
        );

        // **Zero is a real value, not "unset".** The shipped tiles carry chunks
        // with no area at all and a tool has to be able to write one back.
        area::set_area(edited.chunk_mut(70).expect("chunk 70"), 0);
        assert_eq!(area::area(&edited, 70), Some(0), "{path}");
        area::set_area(edited.chunk_mut(70).expect("chunk 70"), was[70]);
        assert_eq!(
            edited.write(),
            bytes,
            "{path}: putting the area back did not restore the tile"
        );
    }
}

/// **What the shipped files say about `MCLQ`, measured rather than assumed.**
///
/// Four facts, all of them unanimous over the 1,280 chunks of five real tiles,
/// and every one of them is something a writer would otherwise have to guess:
///
/// * **every chunk has an `MCLQ` region**, wet or dry — a dry one is the bare
///   eight-byte sub-chunk header and no payload;
/// * **its own size field is zero on every one of them**, which is why the
///   header's `sizeLiquid` is the authority and why
///   `SubChunk::set_keeping_size` exists;
/// * **the block count is exactly the popcount of the liquid flag bits**, which
///   is the whole of how a reader knows how many there are;
/// * and a block is **804 bytes**.
///
/// A writer that got the second one wrong would produce a file that still loads
/// here and carries a number the game never writes.
#[test]
fn the_shipped_liquid_regions_all_agree_about_their_own_shape() {
    use crate::adt::liquid;
    use vale_assets::world::adt::mcnk_flags;

    let mut looked_at = 0;
    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        for index in 0..tile.chunks.len() {
            let chunk = tile.chunk(index).expect("in range");
            let flags = chunk.head().flags();
            let sub = chunk
                .region(Region::Liquid)
                .unwrap_or_else(|| panic!("{path}: chunk {index} has no MCLQ at all"));
            assert_eq!(sub.declared, 0, "{path}: chunk {index}'s MCLQ declares a size");
            assert_eq!(
                sub.data.len(),
                (flags & mcnk_flags::LQ_ANY).count_ones() as usize * liquid::BLOCK,
                "{path}: chunk {index}'s MCLQ is not one block per liquid flag"
            );
            looked_at += 1;
        }
    }
    assert!(looked_at == 0 || looked_at >= 256, "{looked_at} chunks");
}

/// **A pool read out and written back is the bytes it came from**, and a pool
/// changed changes only its own chunk.
///
/// The round trip that says the encoder and the decoder agree — including the
/// parts this crate does not understand, which is the point of carrying the flow
/// tail rather than zeroing it. A block re-encoded without it would still parse,
/// still draw, and have quietly dropped something the game may read.
#[test]
fn a_pool_survives_being_read_out_and_written_back() {
    use crate::adt::liquid;

    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        let wet = (0..tile.chunks.len()).find(|&i| !liquid::pools(&tile, i).is_empty());
        let Some(wet) = wet else { continue };

        let mut same = tile.clone();
        let pools = liquid::pools(&same, wet);
        assert!(!pools.is_empty(), "{path}");
        liquid::set_pools(same.chunk_mut(wet).expect("in range"), &pools);
        // **The region first**, so a failure prints 804 bytes rather than two
        // megabytes — and then the file, which is what says nothing else moved.
        assert_eq!(
            same.chunk(wet).and_then(|chunk| chunk.region(Region::Liquid)),
            tile.chunk(wet).and_then(|chunk| chunk.region(Region::Liquid)),
            "{path}: chunk {wet}'s MCLQ did not survive a read and a write"
        );
        assert!(
            same.write() == bytes,
            "{path}: chunk {wet}'s liquid round trip moved something else"
        );

        // …and now change it: dry the whole chunk, which is the edit that takes
        // the header flag away and the region with it.
        let mut dried = tile.clone();
        let mut pools = liquid::pools(&dried, wet);
        for pool in pools.iter_mut() {
            for row in 0..8 {
                for col in 0..8 {
                    pool.set_wet(row, col, false);
                }
            }
        }
        liquid::set_pools(dried.chunk_mut(wet).expect("in range"), &pools);
        let again = AdtFile::parse(&dried.write()).expect("the edited tile parses");
        assert!(
            liquid::pools(&again, wet).is_empty(),
            "{path}: the chunk still declares a liquid"
        );
        assert_eq!(
            again
                .chunk(wet)
                .and_then(|chunk| chunk.region(Region::Liquid))
                .map(|sub| sub.data.len()),
            Some(0),
            "{path}: the region was left behind"
        );
        // Every other chunk is untouched, which is what says the region's new
        // length moved the offsets after it and nothing else.
        for index in 0..tile.chunks.len() {
            if index == wet {
                continue;
            }
            assert_eq!(
                liquid::pools(&again, index),
                liquid::pools(&tile, index),
                "{path}: chunk {index}'s liquid changed"
            );
        }
    }
}

/// **Water painted onto a dry chunk comes back as water**, at the level it was
/// given and only where the brush reached.
#[test]
fn a_painted_pool_lands_where_the_brush_was_and_nowhere_else() {
    use crate::adt::liquid;
    use crate::ops::WaterBrush;
    use vale_assets::world::adt::cell_at;

    for (path, bytes) in real_tiles() {
        let tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
        // A chunk with no liquid of its own, so what comes back is the brush's.
        let Some(dry) = (0..tile.chunks.len()).find(|&i| liquid::pools(&tile, i).is_empty())
        else {
            continue;
        };

        let mut flooded = tile.clone();
        let origin = flooded.chunk(dry).expect("in range").head().position();
        // The middle of the chunk, and a radius under one cell, so exactly the
        // cell under the point is wet.
        let at = [origin[0] - 16.0, origin[1] - 16.0];
        let (row, col) = cell_at(origin, at[0], at[1]).expect("over the chunk");
        let brush = WaterBrush {
            radius: 1.0,
            level: 123.5,
            kind: vale_assets::world::wmo::Liquid::Water,
            cell_flags: liquid::FISHABLE,
        };
        let done = brush.stroke(&mut flooded, at, crate::ops::WaterAction::Flood, |_, _| Some(100.0));
        assert_eq!(done.cells, 1, "{path}: one cell, not {}", done.cells);
        assert_eq!(done.edits.len(), 1, "{path}");

        let again = AdtFile::parse(&flooded.write()).expect("the edited tile parses");
        let pools = liquid::pools(&again, dry);
        assert_eq!(pools.len(), 1, "{path}: one liquid");
        let pool = &pools[0];
        assert_eq!(pool.kind, vale_assets::world::wmo::Liquid::Water, "{path}");
        assert!(pool.is_wet(row, col), "{path}: the cell under the brush is dry");
        assert_eq!(
            (0..8).flat_map(|r| (0..8).map(move |c| (r, c)))
                .filter(|&(r, c)| pool.is_wet(r, c))
                .count(),
            1,
            "{path}: more than the one cell is wet"
        );
        assert!(
            (pool.height(row, col) - 123.5).abs() < 1e-3,
            "{path}: the surface is not at the level it was given"
        );
        // …and the depth byte came off the ground rather than a flat 255, which
        // is what gives a pool its shallows. 23.5 yards above at eight a yard.
        assert_eq!(pool.depth(row, col), 188, "{path}");

        // Drying it again puts the chunk back exactly as it was.
        let mut dried = flooded.clone();
        let done = brush.stroke(&mut dried, at, crate::ops::WaterAction::Drain, |_, _| None);
        assert_eq!(done.cells, 1, "{path}");
        assert_eq!(dried.write(), bytes, "{path}: drying did not restore the tile");
    }
}
