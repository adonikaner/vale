use super::*;

/// `MCCV` stores each vertex colour as blue, green, red, alpha, and 0x7F
/// decodes to 1.0, not 0.5.
///
/// Neither fault shows on a neutral grey chunk. A decoder that read the bytes
/// in file order would swap red and blue on painted vertices; one that divided
/// by 255 would halve the brightness of every painted vertex and leave
/// unpainted ones unchanged.
///
/// The code that writes `MCCV` follows the same two rules.
#[test]
fn vertex_colours_are_bgra_and_centred_on_one() {
    // One vertex: blue 0x00, green 0x7F, red 0xFF, alpha 0x7F.
    let mut region = vec![COLOUR_NEUTRAL; HEIGHTS_PER_CHUNK * 4];
    region[0] = 0x00;
    region[1] = COLOUR_NEUTRAL;
    region[2] = 0xFF;
    region[3] = COLOUR_NEUTRAL;

    let decoded = decode_colours(&region);
    assert_eq!(decoded.len(), HEIGHTS_PER_CHUNK);
    let first = decoded[0];
    assert!((first[0] - 255.0 / 127.0).abs() < 1e-4, "red should be the third byte: {first:?}");
    assert!((first[1] - 1.0).abs() < 1e-4, "green is the middle and is neutral: {first:?}");
    assert!(first[2].abs() < 1e-4, "blue should be the first byte: {first:?}");

    // Every other vertex is neutral, and neutral is exactly one.
    for (i, colour) in decoded.iter().enumerate().skip(1) {
        assert_eq!(*colour, COLOUR_NONE, "vertex {i}");
    }

    // A short region answers neutral rather than reading off the end, which is
    // the same rule `decode_normals` follows for the same reason.
    assert_eq!(decode_colours(&[]).len(), HEIGHTS_PER_CHUNK);
    assert!(decode_colours(&[]).iter().all(|c| *c == COLOUR_NONE));
}


/// `placement_to_world` and `placement_from_world` are inverses. Loading tiles
/// uses only the first; code that writes a placement uses the second. A swapped
/// axis in the second would save a doodad far from where it was placed, in a
/// file that still parses.
#[test]
fn a_placement_survives_the_trip_out_to_the_world_and_back() {
    for record in [
        [1000.0f32, 47.5, 20000.0],
        [0.0, 0.0, 0.0],
        [MAP_ORIGIN, -12.25, MAP_ORIGIN],
    ] {
        let there = placement_to_world(record);
        let back = placement_from_world(there);
        for axis in 0..3 {
            assert!(
                (back[axis] - record[axis]).abs() < 1e-3,
                "{record:?} -> {there:?} -> {back:?}"
            );
        }
    }
}


#[test]
fn placement_positions_are_measured_back_from_the_map_origin() {
    // The raw triple is (westward, up, northward) from the map corner, so
    // the map origin itself is (MAP_ORIGIN, z, MAP_ORIGIN).
    let at_origin = placement_to_world([MAP_ORIGIN, 42.0, MAP_ORIGIN]);
    assert_eq!(at_origin, [0.0, 0.0, 42.0]);

    // The first doodad of Azeroth_34_51, a Duskwood tree. That tile's terrain
    // runs x -10666..-10133 and y -1600..-1066, and the tree must land on it,
    // allowing about 50 yards of overhang: an object is listed in every tile
    // it touches.
    let [x, y, z] = placement_to_world([18117.72, 30.206, 27210.664]);
    assert!((-10716.0..-10083.0).contains(&x), "x {x}");
    assert!((-1650.0..-1016.0).contains(&y), "y {y}");
    // Transposing the two horizontal axes would put this tree about ten
    // thousand yards away.
    assert!(x < -10000.0 && y > -2000.0);
    assert_eq!(z, 30.206);
}

#[test]
fn an_unrotated_placement_keeps_the_model_upright() {
    // With no rotation, the model's +Z maps to world +Z, multiplied by the
    // scale.
    let m = placement_matrix([10.0, 20.0, 30.0], [0.0, 0.0, 0.0], 2.0);
    assert_eq!(transform(&m, [0.0, 0.0, 1.0]), [10.0, 20.0, 32.0]);
    // Position is the fourth column.
    assert_eq!(transform(&m, [0.0, 0.0, 0.0]), [10.0, 20.0, 30.0]);
}

#[test]
fn yaw_turns_about_the_vertical_and_starts_half_a_turn_round() {
    // Model +X at zero yaw points along world -X: internal and world space
    // differ by a half turn about Z, and that term is part of the matrix.
    let m = placement_matrix([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
    let f = transform(&m, [1.0, 0.0, 0.0]);
    assert!((f[0] + 1.0).abs() < 1e-5 && f[1].abs() < 1e-5, "{f:?}");

    // 90 degrees of MDDF yaw is a quarter turn of that same vector, in the
    // direction a right-handed rotation about +Z goes (+X towards +Y).
    let m = placement_matrix([0.0, 0.0, 0.0], [0.0, 90.0, 0.0], 1.0);
    let f = transform(&m, [1.0, 0.0, 0.0]);
    assert!(f[0].abs() < 1e-5 && (f[1] + 1.0).abs() < 1e-5, "{f:?}");
    // The rotation leaves +Z unchanged.
    assert_eq!(transform(&m, [0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]);
}

/// Apply a column-major 4x4 to a point.
fn transform(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for row in 0..3 {
        out[row] = m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
        // Keep the assertions readable; these are exact quarter turns.
        if out[row].abs() < 1e-6 {
            out[row] = 0.0;
        }
    }
    out
}

/// Append a sub-chunk the way it appears inside an MCNK: reversed magic,
/// `u32` size, payload.
fn push_sub(out: &mut Vec<u8>, magic: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&[magic[3], magic[2], magic[1], magic[0]]);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

#[test]
fn mcnr_normals_are_read_as_north_west_up_in_that_order() {
    let mut payload = vec![0u8; MCNK_HEADER_SIZE];
    push_sub(&mut payload, b"MCVT", &vec![0u8; HEIGHTS_PER_CHUNK * 4]);

    // Sample 0 is the real triple measured on Azeroth_34_51 chunk 0, where
    // the MCVT grid independently implies (-0.23, -0.01, 0.97).
    let mut mcnr = vec![0u8; HEIGHTS_PER_CHUNK * 3];
    mcnr[0] = (-27i8) as u8;
    mcnr[1] = 0;
    mcnr[2] = 123;
    push_sub(&mut payload, b"MCNR", &mcnr);

    let chunk = parse_mcnk(&payload).expect("a header and two sub-chunks");
    let n = chunk.outer_normal(0, 0);
    assert!((n[0] + 0.21).abs() < 0.02, "north component of {n:?}");
    assert!(n[1].abs() < 0.02, "west component of {n:?}");
    assert!(n[2] > 0.95, "up component of {n:?}");
}

#[test]
fn a_normal_of_all_zeroes_becomes_up_rather_than_black() {
    // Padding, a truncated MCNR, or a genuinely zeroed sample would
    // otherwise normalise to NaN and light the cell black.
    let mut payload = vec![0u8; MCNK_HEADER_SIZE];
    push_sub(&mut payload, b"MCNR", &vec![0u8; HEIGHTS_PER_CHUNK * 3]);
    let chunk = parse_mcnk(&payload).expect("parses");
    assert_eq!(chunk.outer_normal(4, 4), [0.0, 0.0, 1.0]);
}

#[test]
fn a_tile_with_no_mcnr_still_has_upright_normals() {
    // The accessors fall back rather than returning a zero vector, which
    // would light the whole tile black.
    let c = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 0);
    assert_eq!(c.outer_normal(3, 4), [0.0, 0.0, 1.0]);
    assert_eq!(c.inner_normal(3, 4), [0.0, 0.0, 1.0]);

    let mesh = one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0).to_mesh();
    assert_eq!(mesh.normals.len(), mesh.positions.len());
    assert!(mesh.normals.iter().all(|n| *n == [0.0, 0.0, 1.0]));
}

#[test]
fn tile_geometry_constants_line_up() {
    assert!((TILE_SIZE - 1600.0 / 3.0).abs() < 0.001);
    assert!((CHUNK_SIZE * CHUNKS_PER_SIDE as f32 - TILE_SIZE).abs() < 0.01);
    assert!((UNIT_SIZE * INNER_SIDE as f32 - CHUNK_SIZE).abs() < 0.01);
    assert_eq!(HEIGHTS_PER_CHUNK, 145);
}

fn test_chunk(heights: Vec<f32>, holes: u16) -> Mcnk {
    Mcnk {
        index_x: 0,
        index_y: 0,
        area_id: 0,
        flags: 0,
        holes,
        position: [0.0, 0.0, 0.0],
        heights,
        normals: Vec::new(),
        colours: Vec::new(),
        layers: Vec::new(),
        alphas: Vec::new(),
        shadow: Vec::new(),
        liquids: Vec::new(),
        detail_layer: [0; 8],
        no_detail: 0,
    }
}

fn layer(flags: u32, alpha_offset: u32) -> TextureLayer {
    TextureLayer {
        texture_id: 0,
        flags,
        alpha_offset,
        effect_id: 0,
    }
}

#[test]
fn four_bit_alpha_unpacks_low_nibble_first_and_reaches_opaque() {
    let src = vec![0xF0u8; ALPHA_LEN / 2];
    let out = expand_4bit_alpha(&src);
    assert_eq!(out.len(), ALPHA_LEN);
    assert_eq!(out[0], 0, "low nibble comes first");
    // 0xF is scaled by 17, not 16, so an opaque layer reaches 255. At 240 the
    // layer beneath would show through everywhere.
    assert_eq!(out[1], 255);
}

#[test]
fn stride_is_measured_from_the_gap_between_layer_offsets() {
    let base = layer(0, 0);
    let four_bit = vec![
        base.clone(),
        layer(layer_flags::USE_ALPHA_MAP, 0),
        layer(layer_flags::USE_ALPHA_MAP, 2048),
    ];
    assert_eq!(alpha_stride(&four_bit, 4096), ALPHA_LEN / 2);

    let big = vec![
        base.clone(),
        layer(layer_flags::USE_ALPHA_MAP, 0),
        layer(layer_flags::USE_ALPHA_MAP, 4096),
    ];
    assert_eq!(alpha_stride(&big, 8192), ALPHA_LEN);

    // One alpha layer gives no gap to measure, so the total size decides.
    let lone_4bit = vec![base.clone(), layer(layer_flags::USE_ALPHA_MAP, 0)];
    assert_eq!(alpha_stride(&lone_4bit, 2048), ALPHA_LEN / 2);
    assert_eq!(alpha_stride(&lone_4bit, 4096), ALPHA_LEN);
}

#[test]
fn the_base_layer_is_opaque_and_has_no_map_in_the_file() {
    let layers = vec![layer(0, 0), layer(layer_flags::USE_ALPHA_MAP, 0)];
    // 4-bit: 2048 bytes of 0x00 => the second layer is fully transparent.
    let alphas = decode_alpha_maps(&layers, &vec![0u8; 2048]);
    assert_eq!(alphas.len(), 2);
    assert!(alphas[0].iter().all(|&a| a == 255), "base layer is opaque");
    assert!(alphas[1].iter().all(|&a| a == 0));
}

/// `pack_4bit_alpha` and `expand_4bit_alpha` are inverses on the sixteen values
/// a 4-bit map can hold. The decode multiplies by 17 and the encode rounds to
/// the nearest nibble. A truncating encode would turn 255 into 0xE, and a
/// fully opaque layer would no longer be opaque.
#[test]
fn the_four_bit_alpha_pair_round_trips() {
    let mut map = vec![0u8; ALPHA_LEN];
    for (i, texel) in map.iter_mut().enumerate() {
        *texel = ((i % 16) * 17) as u8;
    }
    let packed = pack_4bit_alpha(&map);
    assert_eq!(packed.len(), ALPHA_LEN / 2, "two texels to the byte");
    assert_eq!(expand_4bit_alpha(&packed), map);
}

/// A whole chunk's `MCAL` round-trips at either stride: the layer offsets that
/// `encode_alpha_maps` returns read the same maps back.
///
/// The test measures the stride with `alpha_stride` instead of passing it in,
/// as a reader of the written file does, so the encoder's offsets must be far
/// enough apart for the gap rule to recover the stride.
#[test]
fn a_chunks_alpha_maps_survive_the_write() {
    for stride in [ALPHA_LEN / 2, ALPHA_LEN] {
        // Layer 0 is the base layer. It is opaque by definition and has no map
        // in the file, so it can only hold 255.
        let mut maps: Vec<Vec<u8>> = vec![vec![255u8; ALPHA_LEN]];
        maps.extend((1..4).map(|which| {
            (0..ALPHA_LEN)
                .map(|i| (((i + which * 7) % 16) * 17) as u8)
                .collect::<Vec<u8>>()
        }));
        let (mcal, offsets) = encode_alpha_maps(&maps, stride);
        assert_eq!(mcal.len(), 3 * stride, "the base layer writes nothing");

        let layers: Vec<TextureLayer> = offsets
            .iter()
            .enumerate()
            .map(|(i, &offset)| match i {
                0 => layer(0, 0),
                _ => layer(layer_flags::USE_ALPHA_MAP, offset),
            })
            .collect();
        assert_eq!(alpha_stride(&layers, mcal.len()), stride);
        for (i, layer) in layers.iter().enumerate() {
            let back = decode_alpha_layer(i, layer, &mcal, stride);
            assert_eq!(back, maps[i], "layer {i} at stride {stride}");
        }
    }
}

/// `decode_alpha_layer` returns the file's bytes; `decode_alpha_maps` returns
/// the renderer's copy, whose last row and column are replaced by
/// `fix_alpha_edge`. A caller that decoded through the renderer's copy,
/// changed nothing and wrote it back would overwrite every chunk's far edge
/// with the row and column inside it.
#[test]
fn the_writers_decode_leaves_the_far_edge_alone() {
    let mut map = vec![0u8; ALPHA_LEN];
    for row in 0..ALPHA_SIDE {
        for col in 0..ALPHA_SIDE {
            // 255 on the last row and column, 0 everywhere else, so the map
            // holds nothing but the edge the renderer replaces.
            map[row * ALPHA_SIDE + col] = match row == ALPHA_SIDE - 1 || col == ALPHA_SIDE - 1 {
                true => 255,
                false => 0,
            };
        }
    }
    let (mcal, offsets) = encode_alpha_maps(&[vec![255u8; ALPHA_LEN], map.clone()], ALPHA_LEN);
    let layers = vec![layer(0, 0), layer(layer_flags::USE_ALPHA_MAP, offsets[1])];
    assert_eq!(decode_alpha_layer(1, &layers[1], &mcal, ALPHA_LEN), map);
    assert_eq!(
        decode_alpha_maps(&layers, &mcal)[1][ALPHA_LEN - 1],
        0,
        "the renderer's copy takes the far edge from the row inside it"
    );
}

#[test]
fn rle_alpha_handles_both_fill_and_copy_runs() {
    // A fill of 4 x 0x80, then a literal copy of 3 bytes.
    let src = [0x84u8, 0x80, 0x03, 1, 2, 3];
    let out = decompress_alpha_rle(&src);
    assert_eq!(&out[0..4], &[0x80, 0x80, 0x80, 0x80]);
    assert_eq!(&out[4..7], &[1, 2, 3]);
    // Anything the stream did not cover is zero, and the map is always
    // full length so the renderer never has to bounds-check it.
    assert_eq!(out.len(), ALPHA_LEN);
    assert_eq!(out[7], 0);
}

#[test]
fn a_truncated_alpha_stream_still_yields_a_full_map() {
    let layers = vec![layer(0, 0), layer(layer_flags::USE_ALPHA_MAP, 0)];
    let alphas = decode_alpha_maps(&layers, &[0xFFu8; 10]);
    assert_eq!(alphas[1].len(), ALPHA_LEN);
    assert_eq!(alphas[1][0], 255);
    assert_eq!(alphas[1][ALPHA_LEN - 1], 0, "past the data reads as empty");
}

/// The last row and column of an alpha map are replaced on decode. A 64x64
/// alpha map holds 63x63 texels of blend: the pre-Cataclysm client puts the
/// centre of texel 62 on the chunk's far edge and never reads row or column
/// 63, so the files hold no meaningful data there. Drawing those texels paints
/// a line on the far edges of all 256 chunks in every tile, which shows as a
/// grid of seams across the terrain.
///
/// Measured on real tiles by `vale textures`; see [`fix_alpha_edge`].
#[test]
fn the_unwritten_last_row_and_column_are_replaced_by_their_neighbours() {
    let layers = vec![layer(0, 0), layer(layer_flags::USE_ALPHA_MAP, 0)];
    // 4-bit source: every nibble 0xF (=> 255) except the last column and the
    // last row of the map, which are 0, as in the real files.
    let mut src = vec![0xFFu8; ALPHA_LEN / 2];
    for t in 0..ALPHA_SIDE {
        let zero = |src: &mut Vec<u8>, i: usize| {
            let (byte, shift) = (i / 2, (i % 2) * 4);
            src[byte] &= !(0x0F << shift);
        };
        zero(&mut src, t * ALPHA_SIDE + ALPHA_SIDE - 1);
        zero(&mut src, (ALPHA_SIDE - 1) * ALPHA_SIDE + t);
    }

    let map = &decode_alpha_maps(&layers, &src)[1];
    // Column 62 is untouched and column 63 now equals it, so the blend runs
    // to the chunk's own edge instead of collapsing to the base layer there.
    assert_eq!(map[10 * ALPHA_SIDE + 62], 255);
    assert_eq!(map[10 * ALPHA_SIDE + 63], 255, "last column was not fixed");
    assert_eq!(map[63 * ALPHA_SIDE + 10], 255, "last row was not fixed");
    // The corner takes the row fix, which itself took the column fix.
    assert_eq!(map[63 * ALPHA_SIDE + 63], 255);
    // Other texels are unchanged.
    assert_eq!(map[10 * ALPHA_SIDE + 5], 255);
    assert_eq!(map.len(), ALPHA_LEN);
}

#[test]
fn alpha_images_pack_layers_1_to_3_into_rgb() {
    let mut chunk = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 0);
    chunk.layers = vec![layer(0, 0); 3];
    chunk.alphas = vec![
        vec![255; ALPHA_LEN], // base, not written to any channel
        vec![10; ALPHA_LEN],
        vec![20; ALPHA_LEN],
    ];
    let adt = Adt {
        version: 18,
        texture_names: vec![],
        model_names: vec![],
        wmo_names: vec![],
        doodads: vec![],
        wmos: vec![],
        chunks: vec![chunk],
    };

    let atlas = adt.alpha_atlas();
    assert_eq!(atlas.len(), ATLAS_SIDE * ATLAS_SIDE * 4);
    // Alpha is the chunk's `MCSH`, and this one declares none.
    assert_eq!(&atlas[0..4], &[10, 20, 0, 0], "layer 1->R, 2->G, absent 3->B");
}

#[test]
fn each_chunk_lands_in_its_own_cell_of_the_alpha_atlas() {
    // The cell is chosen by the chunk's position in the list. `atlas_uv` and
    // `alpha_atlas` must agree on it, or every chunk is blended with another
    // chunk's map, which looks like a wrong alpha decode.
    let mut chunks = Vec::new();
    for i in 0..20u8 {
        let mut c = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 0);
        c.layers = vec![layer(0, 0); 2];
        // A distinct value per chunk, so a cell can be traced to its chunk.
        c.alphas = vec![vec![255; ALPHA_LEN], vec![i + 1; ALPHA_LEN]];
        chunks.push(c);
    }
    let adt = Adt { chunks, ..one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0) };
    let atlas = adt.alpha_atlas();

    for i in 0..20usize {
        // Sample the middle of the chunk through `atlas_uv`, the function the
        // mesh uses.
        let [u, v] = atlas_uv(i, 0.5, 0.5);
        let (x, y) = (
            (u * ATLAS_SIDE as f32) as usize,
            (v * ATLAS_SIDE as f32) as usize,
        );
        let o = (y * ATLAS_SIDE + x) * 4;
        assert_eq!(atlas[o], i as u8 + 1, "chunk {i} at atlas ({x}, {y})");
    }

    // Chunk 16 starts the second row of cells, which checks that the index
    // wraps into rows and not columns.
    let [u, v] = atlas_uv(16, 0.0, 0.0);
    assert!(u < 1.0 / CHUNKS_PER_SIDE as f32, "{u} should be in column 0");
    assert!(v > 1.0 / CHUNKS_PER_SIDE as f32, "{v} should be past row 0");
}

#[test]
fn an_atlas_lookup_stays_inside_its_own_cell() {
    // A lookup exactly on a cell boundary makes bilinear filtering average in
    // the neighbouring chunk's blend, which draws a grid of seams over the
    // terrain. Both ends of the range must sit half a texel inside the cell.
    let cell = 1.0 / CHUNKS_PER_SIDE as f32;
    let half_texel = 0.5 / ATLAS_SIDE as f32;
    for chunk_index in [0usize, 1, 15, 16, 255] {
        let lo = atlas_uv(chunk_index, 0.0, 0.0);
        let hi = atlas_uv(chunk_index, 1.0, 1.0);
        let cx = (chunk_index % CHUNKS_PER_SIDE) as f32;
        let cy = (chunk_index / CHUNKS_PER_SIDE) as f32;
        assert!((lo[0] - (cx * cell + half_texel)).abs() < 1e-6, "{lo:?}");
        assert!((lo[1] - (cy * cell + half_texel)).abs() < 1e-6, "{lo:?}");
        assert!((hi[0] - ((cx + 1.0) * cell - half_texel)).abs() < 1e-6, "{hi:?}");
        assert!((hi[1] - ((cy + 1.0) * cell - half_texel)).abs() < 1e-6, "{hi:?}");
    }
}

/// No mip level of the alpha atlas averages texels from two cells. A level-`n`
/// texel is an aligned `2^n` block of level-0 texels and a cell is 64 texels
/// wide, so no block straddles two cells until a cell is one texel wide, which
/// is the last level [`ALPHA_ATLAS_MIPS`] counts.
///
/// Neighbouring cells filled with 255 and 0 must therefore stay 255 and 0 at
/// every level. Any other value means a chunk is blended with its neighbour's
/// map, which renders as slightly wrong ground and raises no error.
#[test]
fn minifying_the_alpha_atlas_never_mixes_two_cells() {
    let mut atlas = vec![0u8; ATLAS_SIDE * ATLAS_SIDE * 4];
    for y in 0..ATLAS_SIDE {
        for x in 0..ATLAS_SIDE {
            let o = (y * ATLAS_SIDE + x) * 4;
            // Every other cell fully opaque, in a checkerboard, so a filter
            // that reached across a boundary in either axis would show.
            let cell = (x / ALPHA_SIDE + y / ALPHA_SIDE) % 2;
            atlas[o] = if cell == 0 { 255 } else { 0 };
            atlas[o + 3] = 255;
        }
    }

    let levels = alpha_atlas_mips(&atlas);
    assert_eq!(levels.len(), ALPHA_ATLAS_MIPS, "levels 1..=6");
    for (i, level) in levels.iter().enumerate() {
        let n = i + 1;
        let side = ATLAS_SIDE >> n;
        let cell_side = ALPHA_SIDE >> n;
        assert_eq!(level.len(), side * side * 4, "level {n} is {side}x{side}");
        assert!(cell_side >= 1, "level {n} has less than a texel per cell");
        for y in 0..side {
            for x in 0..side {
                let cell = (x / cell_side + y / cell_side) % 2;
                let want = if cell == 0 { 255 } else { 0 };
                assert_eq!(
                    level[(y * side + x) * 4],
                    want,
                    "level {n} texel ({x}, {y}) took part of the next cell"
                );
            }
        }
    }
    // The last level is one texel per chunk: 16x16 for a 16x16 tile.
    let last = levels.last().expect("a chain");
    assert_eq!(last.len(), CHUNKS_PER_SIDE * CHUNKS_PER_SIDE * 4);
}

#[test]
fn mesh_uvs_span_the_chunk_and_match_the_alpha_map_space() {
    let adt = one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0);
    let mesh = adt.to_mesh();
    assert_eq!(mesh.uvs.len(), mesh.positions.len());
    assert_eq!(mesh.alpha_uvs.len(), mesh.positions.len());
    // The chunk origin is cell (0,0)'s first corner, and the far corner of
    // the last cell is the opposite end of the alpha map.
    assert_eq!(mesh.uvs[0], [0.0, 0.0]);
    let (min, max) = mesh.uvs.iter().fold((1.0f32, 0.0f32), |(lo, hi), uv| {
        (lo.min(uv[0]).min(uv[1]), hi.max(uv[0]).max(uv[1]))
    });
    assert_eq!((min, max), (0.0, 1.0));

    // One draw, covering every index exactly once.
    assert_eq!(mesh.draws.len(), 1);
    assert_eq!(mesh.draws[0].index_start, 0);
    assert_eq!(mesh.draws[0].index_count as usize, mesh.indices.len());
}

#[test]
fn chunks_sharing_a_texture_set_share_one_draw() {
    // Chunks with the same texture set are grouped into one draw. Four chunks
    // with two texture sets must give two draws over contiguous index ranges:
    // a draw is one `index_start`/`index_count` pair, not a scan.
    let heights = vec![0.0; HEIGHTS_PER_CHUNK];
    let with_textures = |ids: &[u32]| {
        let mut c = test_chunk(heights.clone(), 0);
        c.layers = ids
            .iter()
            .map(|&id| TextureLayer { texture_id: id, ..layer(0, 0) })
            .collect();
        c
    };
    let adt = Adt {
        // Interleaved, so the grouping must gather chunks that are not
        // adjacent in the file.
        chunks: vec![
            with_textures(&[0, 1]),
            with_textures(&[2]),
            with_textures(&[0, 1]),
            with_textures(&[2]),
        ],
        ..one_chunk_tile(heights.clone(), 0)
    };
    let mesh = adt.to_mesh();

    assert_eq!(mesh.draws.len(), 2, "two texture sets, two draws");
    assert_eq!(mesh.draws[0].textures, vec![0, 1]);
    assert_eq!(mesh.draws[1].textures, vec![2]);
    for d in &mesh.draws {
        assert_eq!(d.chunks, 2, "each set was used by two chunks");
    }
    // Contiguous, in order, and covering the whole index buffer exactly once.
    assert_eq!(mesh.draws[0].index_start, 0);
    assert_eq!(mesh.draws[1].index_start, mesh.draws[0].index_count);
    let total: u32 = mesh.draws.iter().map(|d| d.index_count).sum();
    assert_eq!(total as usize, mesh.indices.len());

    // Every index still points at an existing vertex after the reorder.
    let vertices = mesh.positions.len() as u32;
    assert!(mesh.indices.iter().all(|&i| i < vertices));
}

#[test]
fn a_group_bounding_sphere_covers_every_chunk_in_it() {
    // Two chunks 400 yards apart on each axis share a texture set. The merged sphere must
    // contain both, or one chunk is culled while the other is on screen.
    let heights = vec![0.0; HEIGHTS_PER_CHUNK];
    let mut far = test_chunk(heights.clone(), 0);
    far.position = [400.0, 400.0, 0.0];
    let adt = Adt {
        chunks: vec![test_chunk(heights.clone(), 0), far],
        ..one_chunk_tile(heights, 0)
    };
    let mesh = adt.to_mesh();
    assert_eq!(mesh.draws.len(), 1);
    let d = &mesh.draws[0];
    for p in &mesh.positions {
        let dist = ((p[0] - d.centre[0]).powi(2)
            + (p[1] - d.centre[1]).powi(2)
            + (p[2] - d.centre[2]).powi(2))
        .sqrt();
        assert!(dist <= d.radius + 0.01, "{p:?} is {dist} from the centre");
    }
    // The tile's own sphere, which the renderer culls by, also contains
    // every vertex.
    for p in &mesh.positions {
        let dist = ((p[0] - mesh.centre[0]).powi(2)
            + (p[1] - mesh.centre[1]).powi(2)
            + (p[2] - mesh.centre[2]).powi(2))
        .sqrt();
        assert!(dist <= mesh.radius + 0.01, "{p:?} is {dist} from the tile centre");
    }
}

#[test]
fn every_chunk_vertex_is_inside_its_own_bounding_sphere() {
    // The renderer culls by these spheres. A sphere that is too small culls
    // chunks that are still partly in view, at the edge of the screen.
    let mut heights = vec![0.0; HEIGHTS_PER_CHUNK];
    heights[40] = 60.0; // one spike, so the sphere has to grow vertically
    heights[41] = -20.0;
    let adt = one_chunk_tile(heights, 0);
    let mesh = adt.to_mesh();
    let draw = &mesh.draws[0];

    for p in &mesh.positions {
        let d = ((p[0] - draw.centre[0]).powi(2)
            + (p[1] - draw.centre[1]).powi(2)
            + (p[2] - draw.centre[2]).powi(2))
        .sqrt();
        assert!(d <= draw.radius + 0.01, "{p:?} is {d} from the centre");
    }
}

#[test]
fn holes_shrink_a_draw_range_rather_than_shifting_its_neighbours() {
    // Two chunks with different texture sets, so they stay two draws and the
    // effect of the holes on each range can be checked.
    let heights = vec![0.0; HEIGHTS_PER_CHUNK];
    let mut holed = test_chunk(heights.clone(), 0b1);
    holed.layers = vec![TextureLayer { texture_id: 0, ..layer(0, 0) }];
    let mut whole = test_chunk(heights.clone(), 0);
    whole.layers = vec![TextureLayer { texture_id: 1, ..layer(0, 0) }];
    let adt = Adt {
        chunks: vec![holed, whole],
        ..one_chunk_tile(heights, 0)
    };
    let mesh = adt.to_mesh();
    // Chunk 0 lost a 2x2 block of cells; chunk 1 is whole and must start
    // exactly where chunk 0 ended. Both chunks share an origin, so neither
    // has a neighbour and each carries a skirt on all four sides: 12 indices
    // per edge segment, less the four segments along the holed block.
    let skirt = |segments: usize| segments * 12;
    assert_eq!(mesh.draws[0].index_count as usize, (64 - 4) * 4 * 3 + skirt(32 - 4));
    assert_eq!(mesh.draws[1].index_start, mesh.draws[0].index_count);
    assert_eq!(mesh.draws[1].index_count as usize, 64 * 4 * 3 + skirt(32));
}

#[test]
fn a_chunk_that_is_all_holes_gets_no_draw_at_all() {
    // 0xFFFF sets all 4x4 hole bits, so the chunk has no geometry. An empty
    // range in a group would be a draw call that draws nothing.
    let heights = vec![0.0; HEIGHTS_PER_CHUNK];
    let mut empty = test_chunk(heights.clone(), 0xFFFF);
    empty.layers = vec![TextureLayer { texture_id: 5, ..layer(0, 0) }];
    let adt = Adt {
        chunks: vec![empty],
        ..one_chunk_tile(heights, 0)
    };
    let mesh = adt.to_mesh();
    assert!(mesh.indices.is_empty());
    assert!(mesh.draws.is_empty());
}

#[test]
fn an_offset_past_the_end_of_mcal_is_empty_not_a_panic() {
    let layers = vec![layer(0, 0), layer(layer_flags::USE_ALPHA_MAP, 99_999)];
    let alphas = decode_alpha_maps(&layers, &[0u8; 2048]);
    assert!(alphas[1].iter().all(|&a| a == 0));
}

#[test]
fn outer_and_inner_grids_interleave_correctly() {
    // Sample i gets value i, so index arithmetic is directly checkable.
    let heights: Vec<f32> = (0..HEIGHTS_PER_CHUNK).map(|i| i as f32).collect();
    let c = test_chunk(heights, 0);

    // Row 0: 9 outer samples at 0..8, then 8 inner at 9..16.
    assert_eq!(c.outer_height(0, 0), 0.0);
    assert_eq!(c.outer_height(0, 8), 8.0);
    assert_eq!(c.inner_height(0, 0), 9.0);
    // Row 1 outer starts at 17.
    assert_eq!(c.outer_height(1, 0), 17.0);
    // Final outer row starts at 8 * 17 = 136 and ends at 144.
    assert_eq!(c.outer_height(8, 8), 144.0);
}

#[test]
fn holes_mask_maps_to_2x2_blocks() {
    let heights = vec![0.0; HEIGHTS_PER_CHUNK];
    let c = test_chunk(heights, 0b1); // bit 0 => top-left 2x2 block
    assert!(c.is_hole(0, 0));
    assert!(c.is_hole(1, 1));
    assert!(!c.is_hole(2, 0));
    assert!(!c.is_hole(0, 2));
}

fn one_chunk_tile(heights: Vec<f32>, holes: u16) -> Adt {
    Adt {
        version: 18,
        texture_names: vec![],
        model_names: vec![],
        wmo_names: vec![],
        doodads: vec![],
        wmos: vec![],
        chunks: vec![test_chunk(heights, holes)],
    }
}

/// `shadowed_at` samples the same `MCSH` texel the atlas puts under that
/// ground. Swapped axes, or a texel taken from the wrong end of the chunk,
/// would still return an in-range bit, so the test sets one texel and checks
/// both sides of it on both axes. The chunk origin is (0, 0) and the chunk
/// runs in decreasing x and y, like the vertices from `to_mesh`.
#[test]
fn a_placements_shadow_bit_is_the_texel_under_it() {
    let mut chunk = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 0);
    let mut shadow = vec![0u8; ALPHA_LEN];
    // Row 3 (along -x), column 10 (along -y).
    shadow[3 * ALPHA_SIDE + 10] = 255;
    chunk.shadow = shadow;
    let adt = Adt {
        chunks: vec![chunk],
        ..one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0)
    };

    let texel = CHUNK_SIZE / ALPHA_SIDE as f32;
    let at = |row: f32, col: f32| adt.shadowed_at(-row * texel, -col * texel);
    assert!(at(3.5, 10.5), "the set texel reads shadowed");
    assert!(!at(10.5, 3.5), "the transposed texel does not");
    assert!(!at(2.5, 10.5), "one row up is lit");
    assert!(!at(3.5, 11.5), "one column over is lit");

    // Outside the tile, and over a chunk with no MCSH, the answer is lit,
    // which is the default for a sun scale.
    assert!(!adt.shadowed_at(100.0, 100.0));
    let bald = one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0);
    assert!(!bald.shadowed_at(-1.0, -1.0));
}

#[test]
fn height_lookup_hits_the_sample_grid_exactly() {
    // Height = the sample index, so every lookup is checkable by hand.
    let heights: Vec<f32> = (0..HEIGHTS_PER_CHUNK).map(|i| i as f32).collect();
    let adt = one_chunk_tile(heights, 0);

    // The chunk origin is (0, 0) and cells run in decreasing x and y, so
    // the origin itself is outer sample (0, 0).
    assert_eq!(adt.height_at(0.0, 0.0), Some(0.0));
    // One unit west is outer (0, 1) = sample 1.
    assert_eq!(adt.height_at(0.0, -UNIT_SIZE), Some(1.0));
    // One unit north-ish (decreasing x) is outer (1, 0) = sample 17.
    assert_eq!(adt.height_at(-UNIT_SIZE, 0.0), Some(17.0));
    // The centre of cell (0, 0) is the inner vertex = sample 9.
    let mid = -UNIT_SIZE / 2.0;
    let h = adt.height_at(mid, mid).expect("inside the chunk");
    assert!((h - 9.0).abs() < 0.01, "got {h}");
}

#[test]
fn height_lookup_interpolates_a_slope_linearly() {
    // A plane sloping in x: outer row r has height r, and the inner samples
    // sit exactly half way between their neighbouring rows.
    let mut heights = vec![0.0; HEIGHTS_PER_CHUNK];
    for r in 0..OUTER_SIDE {
        for c in 0..OUTER_SIDE {
            heights[r * 17 + c] = r as f32;
        }
    }
    for r in 0..INNER_SIDE {
        for c in 0..INNER_SIDE {
            heights[r * 17 + 9 + c] = r as f32 + 0.5;
        }
    }
    let adt = one_chunk_tile(heights, 0);

    // A quarter of the way into the first cell reads 0.25 on each of the four
    // triangles, because the lookup uses the same triangles as the mesh.
    let q = -UNIT_SIZE * 0.25;
    for y in [q, -UNIT_SIZE * 0.75] {
        let h = adt.height_at(q, y).expect("inside");
        assert!((h - 0.25).abs() < 0.01, "got {h} at y={y}");
    }
}

/// `normal_at` returns the normal of the triangle that `height_at`
/// interpolates on. A normal taken from a different triangle, or from the
/// smoothed shading normals in `MCNR`, would tilt a model to a slope that
/// does not match the ground under it.
#[test]
fn the_ground_normal_is_the_slope_the_height_is_interpolated_on() {
    // Flat ground gives exactly (0, 0, 1), so a model on level ground is not
    // tilted at all.
    let flat = one_chunk_tile(vec![5.0; HEIGHTS_PER_CHUNK], 0);
    assert_eq!(flat.normal_at(-1.0, -1.0), Some([0.0, 0.0, 1.0]));

    // A plane rising one yard per yard of decreasing x, so the ground rises
    // away from the chunk origin. The normal leans toward +x, and at 45° it
    // is `(1, 0, 1)/sqrt(2)`.
    let mut heights = vec![0.0; HEIGHTS_PER_CHUNK];
    for r in 0..OUTER_SIDE {
        for c in 0..OUTER_SIDE {
            heights[r * 17 + c] = r as f32 * UNIT_SIZE;
        }
    }
    for r in 0..INNER_SIDE {
        for c in 0..INNER_SIDE {
            heights[r * 17 + 9 + c] = (r as f32 + 0.5) * UNIT_SIZE;
        }
    }
    let slope = one_chunk_tile(heights, 0);
    let root_half = std::f32::consts::FRAC_1_SQRT_2;
    // All four triangles of the cell are checked: each is a separate branch,
    // and a sign can be right in one and wrong in another.
    for (x, y) in [
        (-UNIT_SIZE * 0.25, -UNIT_SIZE * 0.25),
        (-UNIT_SIZE * 0.75, -UNIT_SIZE * 0.25),
        (-UNIT_SIZE * 0.25, -UNIT_SIZE * 0.75),
        (-UNIT_SIZE * 0.75, -UNIT_SIZE * 0.75),
    ] {
        let n = slope.normal_at(x, y).expect("inside the chunk");
        assert!(n[2] > 0.0, "the normal points into the ground at ({x}, {y})");
        assert!(
            (n[0] - root_half).abs() < 0.01 && n[1].abs() < 0.01 && (n[2] - root_half).abs() < 0.01,
            "at ({x}, {y}): {n:?}"
        );
    }

    // `normal_at` returns `None` where `height_at` does: over a hole and off
    // the tile.
    let holed = one_chunk_tile(vec![5.0; HEIGHTS_PER_CHUNK], 0b1);
    assert_eq!(holed.normal_at(0.0, 0.0), None);
    assert_eq!(flat.normal_at(10.0, 0.0), None);
}

#[test]
fn holes_and_positions_outside_the_tile_have_no_height() {
    let heights = vec![5.0; HEIGHTS_PER_CHUNK];
    let holed = one_chunk_tile(heights.clone(), 0b1);
    assert_eq!(holed.height_at(0.0, 0.0), None, "cell (0,0) is a hole");

    let solid = one_chunk_tile(heights, 0);
    // Positive x is off the chunk, because cells run in decreasing x.
    assert_eq!(solid.height_at(10.0, 0.0), None);
    assert_eq!(solid.height_at(-CHUNK_SIZE - 1.0, 0.0), None);
}

#[test]
fn mesh_emits_four_triangles_per_cell_and_skips_holes() {
    let heights = vec![0.0; HEIGHTS_PER_CHUNK];
    let adt = Adt {
        version: 18,
        texture_names: vec![],
        model_names: vec![],
        wmo_names: vec![],
        doodads: vec![],
        wmos: vec![],
        chunks: vec![test_chunk(heights.clone(), 0)],
    };
    // 8x8 cells x 4 triangles, and a lone chunk's skirt: 32 edge segments,
    // four triangles each (two per side of the strip).
    assert_eq!(adt.to_mesh().triangle_count(), 64 * 4 + 32 * 4);

    // One hole bit removes a 2x2 block of cells => 4 cells => 16 triangles,
    // and the four skirt segments along the block's two outer sides.
    let holed = Adt {
        chunks: vec![test_chunk(heights, 0b1)],
        ..adt.clone()
    };
    assert_eq!(holed.to_mesh().triangle_count(), (64 - 4) * 4 + (32 - 4) * 4);
}

// -----------------------------------------------------------------------
// MCLQ: liquid stored in the terrain chunks
// -----------------------------------------------------------------------

/// One MCLQ block: min/max, 81 vertices, 64 tile bytes, the flow tail.
///
/// Byte 0 of each vertex is its own file index and the height is `height`,
/// so the assertions can check where each vertex lands in the grid frame.
fn mclq_block(height: f32, wet: impl Fn(usize, usize) -> bool) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&height.to_le_bytes()); // min
    b.extend_from_slice(&height.to_le_bytes()); // max
    for i in 0..MCLQ_VERTS {
        b.push(i as u8); // depth / low byte of s
        b.extend_from_slice(&[0, 0, 0]);
        b.extend_from_slice(&height.to_le_bytes());
    }
    for row in 0..INNER_SIDE {
        for col in 0..INNER_SIDE {
            // Water's type, 4, on a wet cell; a cell of type 0 reads as dry.
            b.push(if wet(row, col) { 0x04 } else { 0x0F });
        }
    }
    b.extend_from_slice(&0u32.to_le_bytes()); // nFlowvs
    b.extend_from_slice(&[0u8; 80]); // two flow vectors, always present
    assert_eq!(b.len(), MCLQ_BLOCK_SIZE);
    b
}

/// An MCNK payload whose MCLQ is laid out as in the shipped files: the
/// sub-chunk's own size field is zero, and the real extent is the header's
/// `sizeLiquid`.
fn mcnk_with_liquid(flags: u32, blocks: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = vec![0u8; MCNK_HEADER_SIZE];
    payload[0x00..0x04].copy_from_slice(&flags.to_le_bytes());
    payload[0x68..0x6C].copy_from_slice(&100f32.to_le_bytes());
    payload[0x6C..0x70].copy_from_slice(&200f32.to_le_bytes());
    payload[0x70..0x74].copy_from_slice(&7f32.to_le_bytes());
    let data: Vec<u8> = blocks.iter().flatten().copied().collect();
    payload[0x64..0x68].copy_from_slice(&((data.len() + 8) as u32).to_le_bytes());
    payload.extend_from_slice(b"QLCM");
    payload.extend_from_slice(&0u32.to_le_bytes()); // size field, always 0
    payload.extend_from_slice(&data);
    payload
}

/// An MCNK carrying one `MCSH` sub-chunk and whatever header flags are
/// passed. 512 bytes of shadow bits, laid out as the file lays them out.
fn mcnk_with_shadow(flags: u32, bits: &[u8]) -> Vec<u8> {
    let mut payload = vec![0u8; MCNK_HEADER_SIZE];
    payload[0x00..0x04].copy_from_slice(&flags.to_le_bytes());
    // No liquid: `sizeLiquid` is the bare header.
    payload[0x64..0x68].copy_from_slice(&8u32.to_le_bytes());
    let mut mcsh = bits.to_vec();
    mcsh.resize(ALPHA_LEN / 8, 0);
    payload.extend_from_slice(b"HSCM");
    payload.extend_from_slice(&(mcsh.len() as u32).to_le_bytes());
    payload.extend_from_slice(&mcsh);
    payload
}

/// `MCSH` unpacks least significant bit first; the file does not record the
/// order. The other order raises no error: it mirrors each group of eight
/// texels within its byte, which draws a shadow striped at a period of 8
/// texels along one axis. The fixture sets the texel at the start of a byte;
/// under the wrong order it comes out at the end of that byte.
#[test]
fn a_shadow_map_unpacks_lsb_first() {
    // Byte 0 = 0b0000_0001: texel 0 shadowed, texels 1..7 lit.
    let chunk = parse_mcnk(&mcnk_with_shadow(mcnk_flags::HAS_MCSH, &[0x01]))
        .expect("parses");
    assert_eq!(chunk.shadow.len(), ALPHA_LEN);
    assert_eq!(chunk.shadow[0], 255, "bit 0 is texel 0, not texel 7");
    assert_eq!(&chunk.shadow[1..8], &[0; 7]);

    // The high bit is the eighth texel.
    let high = parse_mcnk(&mcnk_with_shadow(mcnk_flags::HAS_MCSH, &[0x80]))
        .expect("parses");
    assert_eq!(&high.shadow[..7], &[0; 7]);
    assert_eq!(high.shadow[7], 255);
}

/// `MCSH` is read only when the header flag `HAS_MCSH` is set. The sub-chunk
/// is written whether or not anything is shadowed, so a chunk without the
/// flag gets no shadow, whatever bits the file holds. The empty vector puts 0
/// in the atlas's alpha channel.
#[test]
fn a_chunk_that_declares_no_shadow_reads_none() {
    let chunk = parse_mcnk(&mcnk_with_shadow(0, &[0xFF; 64])).expect("parses");
    assert!(chunk.shadow.is_empty());

    // A chunk with the flag but no sub-chunk also reads an empty shadow,
    // and does not panic.
    let mut headerless = vec![0u8; MCNK_HEADER_SIZE];
    headerless[0x00..0x04].copy_from_slice(&mcnk_flags::HAS_MCSH.to_le_bytes());
    headerless[0x64..0x68].copy_from_slice(&8u32.to_le_bytes());
    assert!(parse_mcnk(&headerless).expect("parses").shadow.is_empty());
}

/// The last row and column of a shadow map are copies of the row and column
/// beside them, as with an alpha map: the client stretches a 64x64 map so the
/// centre of texel 62 sits on the chunk's edge, and never reads texel 63.
/// Drawing the unwritten strip puts a shadow seam on the far edge of all 256
/// chunks in a tile.
#[test]
fn a_shadow_maps_unwritten_edge_is_the_one_beside_it() {
    // Every bit set except the whole last row and column, which the file
    // leaves at zero.
    let mut bits = vec![0u8; ALPHA_LEN / 8];
    for texel in 0..ALPHA_LEN {
        let (x, y) = (texel % ALPHA_SIDE, texel / ALPHA_SIDE);
        if x < ALPHA_SIDE - 1 && y < ALPHA_SIDE - 1 {
            bits[texel / 8] |= 1 << (texel % 8);
        }
    }
    let chunk =
        parse_mcnk(&mcnk_with_shadow(mcnk_flags::HAS_MCSH, &bits)).expect("parses");
    let last = ALPHA_SIDE - 1;
    assert_eq!(chunk.shadow[last * ALPHA_SIDE + last], 255, "the corner");
    assert_eq!(chunk.shadow[3 * ALPHA_SIDE + last], 255, "the last column");
    assert_eq!(chunk.shadow[last * ALPHA_SIDE + 3], 255, "the last row");
}

/// The shadow is stored in the atlas's alpha channel, in the same cell and at
/// the same texel as the chunk's blend maps. That is why it shares the atlas
/// instead of having a texture of its own.
#[test]
fn the_atlas_carries_the_shadow_in_its_alpha() {
    let chunks: Vec<Mcnk> = (0..2)
        .map(|i| {
            let mut c = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 0);
            c.layers = vec![layer(0, 0); 2];
            c.alphas = vec![vec![255; ALPHA_LEN], vec![7; ALPHA_LEN]];
            // Chunk 1 declares no shadow, and reads as unshadowed rather
            // than as whatever is in its neighbour's cell.
            if i == 0 {
                c.shadow = (0..ALPHA_LEN).map(|t| (t % 256) as u8).collect();
            }
            c
        })
        .collect();
    let adt = Adt {
        chunks,
        ..one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0)
    };

    let atlas = adt.alpha_atlas();
    let at = |cell: usize, texel: usize| {
        let (cx, cy) = (cell % CHUNKS_PER_SIDE, cell / CHUNKS_PER_SIDE);
        let x = cx * ALPHA_SIDE + texel % ALPHA_SIDE;
        let y = cy * ALPHA_SIDE + texel / ALPHA_SIDE;
        atlas[(y * ATLAS_SIDE + x) * 4 + 3]
    };
    assert_eq!(at(0, 0), 0);
    assert_eq!(at(0, 200), (200 % 256) as u8);
    assert_eq!(at(1, 200), 0, "no MCSH is no shadow");
}

/// The blocks appear in flag-bit order and the zero size field is ignored. A
/// second block is read one whole stride after the first, flow tail included.
#[test]
fn mclq_reads_one_block_per_flag_bit_past_the_zero_size_field() {
    let payload = mcnk_with_liquid(
        mcnk_flags::LQ_RIVER | mcnk_flags::LQ_MAGMA,
        &[mclq_block(30.0, |_, _| true), mclq_block(99.0, |_, _| true)],
    );
    let chunk = parse_mcnk(&payload).expect("parses");
    assert_eq!(chunk.liquids.len(), 2);
    assert_eq!(chunk.liquids[0].0, Liquid::Water);
    assert_eq!(chunk.liquids[1].0, Liquid::Magma);
    assert_eq!(chunk.liquids[0].1.height(4, 4), 30.0);
    assert_eq!(chunk.liquids[1].1.height(4, 4), 99.0, "the stride skipped the flow tail");

    // Magma's byte 0 is a texture coordinate, so its opacity is a constant
    // 255. The same rule applies to WMO liquid.
    assert_eq!(chunk.liquids[1].1.opacity(40, Liquid::Magma), 255);
}

/// The file's rows run in decreasing world x from the chunk origin and
/// `WmoLiquid` runs +X from its base, so file vertex `(0, 0)` is grid vertex
/// `(8, 8)`, the chunk origin, and file tile `(0, 0)` is grid tile `(7, 7)`.
/// A wrong mapping still draws a plausible lake, but misplaces every depth
/// and every wet flag in it.
#[test]
fn mclq_lands_in_world_space_with_the_file_frame_turned_around() {
    let payload = mcnk_with_liquid(
        mcnk_flags::LQ_RIVER,
        &[mclq_block(30.0, |row, col| row == 0 && col == 0)],
    );
    let chunk = parse_mcnk(&payload).expect("parses");
    let (kind, grid) = &chunk.liquids[0];
    assert_eq!(*kind, Liquid::Water);

    // The chunk origin is (100, 200); its far corner is one CHUNK_SIZE down
    // each axis, and that is the grid's base.
    let origin = grid.vertex(8, 8);
    assert!((origin[0] - 100.0).abs() < 1e-3, "{origin:?}");
    assert!((origin[1] - 200.0).abs() < 1e-3, "{origin:?}");
    assert_eq!(origin[2], 30.0, "MCLQ heights are absolute, not chunk-relative");

    // File vertex (0, 0) carried depth byte 0; it must land at grid (8, 8).
    assert_eq!(grid.opacity(8 * 9 + 8, Liquid::Water), 0);
    // File vertex (1, 0), one row down at index 9, lands at grid (7, 8).
    assert_eq!(grid.opacity(8 * 9 + 7, Liquid::Water), 9);

    // Only file tile (0, 0) is wet, which is grid tile (7, 7).
    let wet: Vec<(usize, usize)> = (0..8)
        .flat_map(|y| (0..8).map(move |x| (x, y)))
        .filter(|&(x, y)| grid.has_liquid(x, y))
        .collect();
    assert_eq!(wet, [(7, 7)]);
}

/// The same block with a sloping surface: file vertex `(row, col)` is at
/// height `base + row`, so the height falls as world x rises (the file's rows
/// run in decreasing world x). Flat water gives the same answer under
/// interpolation and under a nearest-vertex lookup; a slope does not.
fn mclq_sloped(base: f32) -> Vec<u8> {
    let side = INNER_SIDE + 1;
    let mut b = Vec::new();
    b.extend_from_slice(&base.to_le_bytes());
    b.extend_from_slice(&(base + 8.0).to_le_bytes());
    for i in 0..MCLQ_VERTS {
        let row = i / side;
        b.push(0);
        b.extend_from_slice(&[0, 0, 0]);
        b.extend_from_slice(&(base + row as f32).to_le_bytes());
    }
    b.extend_from_slice(&[0x04u8; MCLQ_TILES]); // every tile wet, water's type
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&[0u8; 80]);
    assert_eq!(b.len(), MCLQ_BLOCK_SIZE);
    b
}

/// `liquid_at` returns the liquid and its surface height over a point. The
/// movement code calls it twenty times a second to decide whether the
/// character is swimming.
///
/// The test checks three properties:
///
/// * A dry tile returns `None`. A lookup that ignored the low-nibble-15 flag
///   would put an invisible surface over the whole chunk and make the
///   character swim on dry land.
/// * The surface is interpolated across the tile, matching the quad
///   [`Adt::liquid_surface`] emits, so the water line used for swimming is
///   the one drawn.
/// * Where two liquids cover one point, the higher surface is returned. The
///   file does not order them, and shipped tiles have rivers that run into
///   the sea.
#[test]
fn liquid_at_answers_the_surface_over_a_point_and_nothing_over_a_dry_tile() {
    let tile = crate::world::wmo::LIQUID_TILE_SIZE;
    // The fixture's chunk origin is (100, 200) and its grid runs one
    // CHUNK_SIZE back along each axis from there.
    let payload = mcnk_with_liquid(
        mcnk_flags::LQ_RIVER,
        &[mclq_block(30.0, |row, col| row == 0 && col == 0)],
    );
    let chunk = parse_mcnk(&payload).expect("parses");

    // File tile (0, 0) is grid tile (7, 7), the corner at the chunk origin.
    let inside = chunk.liquid_at(100.0 - tile * 0.5, 200.0 - tile * 0.5);
    assert_eq!(inside, Some((Liquid::Water, 30.0)));
    // Four tiles over is dry, although the grid covers it.
    assert_eq!(chunk.liquid_at(100.0 - tile * 4.5, 200.0 - tile * 4.5), None);
    // Off the grid entirely.
    assert_eq!(chunk.liquid_at(500.0, 200.0), None);

    // The slope: grid vertex x = 8 is at world x = 100 and has the base
    // height, and each tile back along x is a yard higher.
    let sloped = parse_mcnk(&mcnk_with_liquid(mcnk_flags::LQ_RIVER, &[mclq_sloped(30.0)]))
        .expect("parses");
    let at = |x: f32| sloped.liquid_at(x, 200.0 - tile * 0.5).expect("wet").1;
    assert!((at(100.0 - tile * 0.001) - 30.0).abs() < 0.05, "{}", at(100.0));
    assert!((at(100.0 - tile) - 31.0).abs() < 0.01);
    // Half a tile in is half a yard higher. A nearest-vertex lookup would
    // return 30.0 or 31.0 here.
    assert!((at(100.0 - tile * 0.5) - 30.5).abs() < 0.01, "{}", at(100.0 - tile * 0.5));

    // Two liquids over one point: the higher surface is returned.
    let both = parse_mcnk(&mcnk_with_liquid(
        mcnk_flags::LQ_RIVER | mcnk_flags::LQ_OCEAN,
        &[mclq_block(30.0, |_, _| true), mclq_block(44.0, |_, _| true)],
    ))
    .expect("parses");
    assert_eq!(
        both.liquid_at(100.0 - tile * 0.5, 200.0 - tile * 0.5),
        Some((Liquid::Ocean, 44.0))
    );
}

/// A dry chunk has `sizeLiquid == 8`. A chunk with no liquid flag reads no
/// liquid, whatever the size field says. A truncated block drops the liquid
/// blocks and keeps the rest of the chunk.
#[test]
fn mclq_absence_and_damage_cost_the_liquid_and_nothing_else() {
    let dry = mcnk_with_liquid(mcnk_flags::LQ_RIVER, &[]);
    assert!(parse_mcnk(&dry).expect("parses").liquids.is_empty());

    let unflagged = mcnk_with_liquid(0, &[mclq_block(30.0, |_, _| true)]);
    assert!(parse_mcnk(&unflagged).expect("parses").liquids.is_empty());

    let mut short = mclq_block(30.0, |_, _| true);
    short.truncate(400);
    let truncated = mcnk_with_liquid(mcnk_flags::LQ_RIVER, &[short]);
    let chunk = parse_mcnk(&truncated).expect("parses");
    assert!(chunk.liquids.is_empty(), "a torn block is dropped");
    assert_eq!(chunk.heights.len(), HEIGHTS_PER_CHUNK, "the terrain survives");
}

/// A cell whose low nibble is 0 is dry, whatever its high nibble, and a cell
/// of any other type is wet; the high nibble is kept on a wet cell.
#[test]
fn mclq_cell_of_type_zero_is_dry() {
    let mut block = mclq_block(30.0, |_, _| true);
    let tiles = 8 + MCLQ_VERTS * LIQUID_VERTEX_SIZE;
    block[tiles] = 0x40;
    block[tiles + 1] = 0xC1;
    let chunk = parse_mcnk(&mcnk_with_liquid(mcnk_flags::LQ_RIVER, &[block])).expect("parses");
    let (_, grid) = &chunk.liquids[0];
    // File cell (row 0, col 0) is grid tile x 7, y 7, and file cell (row 0,
    // col 1) is x 7, y 6.
    assert!(!grid.has_liquid(7, 7), "type 0 is dry");
    assert!(grid.has_liquid(7, 6), "type 1 is wet");
    assert_eq!(grid.tile_flags[6 * INNER_SIDE + 7], 0xC1);
    assert!(grid.has_liquid(0, 0), "the rest of the block is still wet");
}

/// The terrain liquid surface is built the same way as WMO liquid: two
/// up-facing triangles per wet tile, the depth byte as the vertex alpha, and
/// one draw per liquid type.
#[test]
fn terrain_liquid_surface_faces_up_and_carries_depth_as_alpha() {
    let grid = WmoLiquid {
        x_tiles: INNER_SIDE,
        y_tiles: INNER_SIDE,
        base: [0.0, 0.0, 5.0],
        liquid_type: 0,
        heights: vec![5.0; MCLQ_VERTS],
        depths: (0..MCLQ_VERTS).map(|i| i as u8).collect(),
        tile_flags: (0..MCLQ_TILES)
            .map(|i| if i == 3 * INNER_SIDE + 2 { 0x00 } else { 0x0F })
            .collect(),
    };
    let mut chunk = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 0);
    chunk.liquids = vec![(Liquid::Water, grid)];
    let adt = Adt {
        version: 18,
        texture_names: vec![],
        model_names: vec![],
        wmo_names: vec![],
        doodads: vec![],
        wmos: vec![],
        chunks: vec![chunk],
    };

    let draws = adt.liquid_surface();
    assert_eq!(draws.len(), 1);
    let draw = &draws[0];
    assert_eq!(draw.kind, Liquid::Water);
    assert_eq!(draw.triangle_count(), 2, "two per wet tile");
    assert_eq!(draw.positions.len(), 4);
    assert_eq!(draw.colours.len(), draw.positions.len(), "buffers stay parallel");

    // Wet tile (2, 3): its four corners' depth bytes, in emission order.
    let alphas: Vec<u8> = draw.colours.iter().map(|c| c[3]).collect();
    assert_eq!(alphas, [3 * 9 + 2, 3 * 9 + 3, 4 * 9 + 3, 4 * 9 + 2]);
    assert!(draw.colours.iter().all(|c| c[0] == 0 && c[1] == 0 && c[2] == 0));

    // The winding faces +Z, as WMO liquid does.
    let p = |i: usize| {
        let v = draw.positions[draw.indices[i] as usize];
        (v[0], v[1], v[2])
    };
    let (a, b, c) = (p(0), p(1), p(2));
    let cross_z = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    assert!(cross_z > 0.0, "the surface faces up, not into the lake");
}

/// A point inside the `MODF` box of a building the caller has not loaded yet
/// has no floor, instead of taking the terrain height. `MODF` gives the box
/// before the `.wmo` is read, so the tile that answers a height also knows a
/// building stands over that point. Stormwind's terrain is far below its
/// floors, so a caller that used the terrain height in the meantime would
/// fall through the world.
///
/// The box test is all the client checks before it waits for the object to
/// load. This test covers a point inside, a point outside, and a point inside
/// a building the caller has already settled.
#[test]
fn a_point_inside_an_unstaged_modf_box_has_no_floor_yet() {
    // A ten-yard cube around the map origin, in the file's (westward, up,
    // northward) frame, so its world box is x/y -5..5 and z 20..30.
    let placement = WmoPlacement {
        name_id: 0,
        unique_id: 77,
        position: [MAP_ORIGIN, 25.0, MAP_ORIGIN],
        rotation: [0.0; 3],
        bounds_lower: [MAP_ORIGIN - 5.0, 20.0, MAP_ORIGIN - 5.0],
        bounds_upper: [MAP_ORIGIN + 5.0, 30.0, MAP_ORIGIN + 5.0],
        flags: 0,
        doodad_set: 0,
        name_set: 0,
    };
    // `vale wmos` checks the built geometry against this conversion. It
    // re-sorts the corners: on the two axes measured backwards, the file's
    // lower corner is not the world's lower corner.
    let world = modf_world_box(&placement);
    assert_eq!(world, [[-5.0, -5.0, 20.0], [5.0, 5.0, 30.0]]);

    let adt = Adt {
        wmos: vec![placement],
        ..one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0)
    };

    let pending = |_: u32| true;
    assert!(adt.awaiting_building([0.0, 0.0, 25.0], pending), "inside");
    assert!(!adt.awaiting_building([0.0, 0.0, 40.0], pending), "over the roof");
    assert!(!adt.awaiting_building([50.0, 0.0, 25.0], pending), "beside it");
    // Once the caller has settled the placement, the terrain height applies
    // again, so a building with no hull does not leave a permanent hole.
    assert!(!adt.awaiting_building([0.0, 0.0, 25.0], |_| false), "settled");
    // The predicate is asked about the placement's own id and no other.
    assert!(adt.awaiting_building([0.0, 0.0, 25.0], |id| id == 77));
    assert!(!adt.awaiting_building([0.0, 0.0, 25.0], |id| id == 78));

    // `inside_building` asks the same boxes a different question, and its
    // answer does not change when the building loads: a point inside a
    // `MODF` box is inside a building whatever its hull answers. This stops
    // the movement code from taking the mountain over Ironforge as a floor
    // where the city's hull has no surface; see `Standing::standable`.
    assert!(adt.inside_building([0.0, 0.0, 25.0]), "inside, hull or no hull");
    assert!(!adt.inside_building([0.0, 0.0, 40.0]), "over the roof");
    assert!(!adt.inside_building([50.0, 0.0, 25.0]), "beside it");
    // Settling the placement changes `awaiting_building` and leaves
    // `inside_building` unchanged.
    assert!(!adt.awaiting_building([0.0, 0.0, 25.0], |_| false));
    assert!(adt.inside_building([0.0, 0.0, 25.0]));
}

/// The `_s` suffix goes before the extension, not after it. The archives hold
/// `_s.blp` files; a name with the suffix after the extension matches none.
#[test]
fn the_specular_variant_is_named_before_the_extension() {
    assert_eq!(
        specular_texture(r"Tileset\Duskwood\DuskwoodCobblestone.blp"),
        r"Tileset\Duskwood\DuskwoodCobblestone_s.blp"
    );
    // The suffix goes before the last dot, since a path may contain others.
    assert_eq!(specular_texture(r"a.b\c.blp"), r"a.b\c_s.blp");
    // With no extension, the client appends `_s` to the whole name, and so
    // does this function.
    assert_eq!(specular_texture("bare"), "bare_s");
    // The function is not idempotent. Applying it twice gives a name no
    // archive holds, so a double application shows as magenta ground
    // instead of passing unnoticed.
    assert_eq!(specular_texture("a_s.blp"), "a_s_s.blp");
}

/// `write_alpha_atlas_cell` writes a cell into a built atlas at the same place
/// `alpha_atlas` put it, at every level of the mip chain.
///
/// This lets a host change one chunk's blend without rebuilding the tile. A
/// level indexed at the wrong stride writes a chunk's blend over its
/// neighbour's: up close the blend is correct, and at a distance it appears
/// one chunk along. No error is raised.
#[test]
fn a_cell_written_into_an_atlas_lands_where_the_builder_put_it() {
    let levels = alpha_atlas_levels();
    assert_eq!(levels.len(), 1 + ALPHA_ATLAS_MIPS);
    assert_eq!(levels[0], (0, ATLAS_SIDE));
    assert_eq!(levels[1], (ATLAS_SIDE * ATLAS_SIDE * 4, ATLAS_SIDE / 2));
    // The chain stops at the level where a cell is one texel.
    assert_eq!(levels[ALPHA_ATLAS_MIPS].1, ATLAS_SIDE / ALPHA_SIDE);

    let total: usize = levels.iter().map(|&(_, side)| side * side * 4).sum();
    let mut atlas = vec![0u8; total];
    // One cell in the middle of the grid, filled with values no other cell
    // has, so a byte written outside it shows as a nonzero value.
    let which = 5 * CHUNKS_PER_SIDE + 9;
    let cell = alpha_atlas_cell(
        &[
            vec![255u8; ALPHA_LEN],
            vec![200u8; ALPHA_LEN],
            vec![100u8; ALPHA_LEN],
            vec![50u8; ALPHA_LEN],
        ],
        &vec![10u8; ALPHA_LEN],
    );
    assert_eq!(&cell[..4], &[200, 100, 50, 10], "layers 1..3 in RGB, MCSH in A");
    write_alpha_atlas_cell(&mut atlas, which, &cell);

    // A flat cell survives a box filter unchanged, so every level reads the
    // same four bytes inside the cell and zero outside it.
    for (level, &(offset, side)) in levels.iter().enumerate() {
        let cell_side = ALPHA_SIDE >> level;
        let (x0, y0) = (
            which % CHUNKS_PER_SIDE * cell_side,
            which / CHUNKS_PER_SIDE * cell_side,
        );
        let at = |x: usize, y: usize| -> [u8; 4] {
            let o = offset + (y * side + x) * 4;
            [atlas[o], atlas[o + 1], atlas[o + 2], atlas[o + 3]]
        };
        assert_eq!(at(x0, y0), [200, 100, 50, 10], "level {level}, first texel");
        assert_eq!(
            at(x0 + cell_side - 1, y0 + cell_side - 1),
            [200, 100, 50, 10],
            "level {level}, last texel"
        );
        if x0 > 0 {
            assert_eq!(at(x0 - 1, y0), [0; 4], "level {level} leaked left");
        }
        if y0 > 0 {
            assert_eq!(at(x0, y0 - 1), [0; 4], "level {level} leaked up");
        }
        assert_eq!(at(x0 + cell_side, y0), [0; 4], "level {level} leaked right");
    }
}

/// [`lean_to_normal`] tilts the model so its `+Z` lies along the given normal,
/// for any turn. The test passes the record it produces through
/// [`placement_matrix`] and checks where the model's `+Z` lands, instead of
/// checking the derivation.
#[test]
fn a_lean_carries_the_models_up_onto_the_normal() {
    let up_of = |world: [f32; 3]| {
        let m = placement_matrix([0.0; 3], placement_euler_from_world(world), 1.0);
        // Column 2 of the basis is where the model's +Z lands.
        [m[8], m[9], m[10]]
    };
    let normals: [[f32; 3]; 6] = [
        [0.0, 0.0, 1.0],
        [0.3, 0.0, 0.954],
        [0.0, -0.5, 0.866],
        [-0.4, 0.4, 0.825],
        [0.6, -0.2, 0.775],
        [0.7, 0.7, 0.14],
    ];
    for normal in normals {
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        let unit = [normal[0] / len, normal[1] / len, normal[2] / len];
        for turn in [0.0f32, 37.0, 90.0, 180.0, 265.0, 359.0] {
            let world = lean_to_normal(unit, turn);
            assert_eq!(world[2], turn, "the turn is kept");
            let up = up_of(world);
            for axis in 0..3 {
                assert!(
                    (up[axis] - unit[axis]).abs() < 1e-3,
                    "normal {unit:?} at turn {turn}: up landed at {up:?}"
                );
            }
        }
    }
    // A normal straight up gives no lean. A normal below the horizon is
    // refused and also gives no lean, instead of turning the model upside
    // down.
    assert_eq!(lean_to_normal([0.0, 0.0, 1.0], 45.0), [0.0, 0.0, 45.0]);
    assert_eq!(lean_to_normal([0.0, 0.0, -1.0], 45.0), [0.0, 0.0, 45.0]);
}

/// The three `MDDF` angles are turns about the world's z, y and x, in that
/// order and with those signs, and `placement_euler_to_world` and
/// `placement_euler_from_world` are inverses.
///
/// The test checks [`placement_matrix`] itself, not the arithmetic: turning
/// one angle must move the model about the named world axis, or a control
/// labelled "about z" would turn the model about a different axis.
#[test]
fn the_placement_angles_are_turns_about_the_worlds_axes() {
    let there_and_back = |rot: [f32; 3]| {
        placement_euler_from_world(placement_euler_to_world(rot))
    };
    assert_eq!(there_and_back([10.0, 20.0, 30.0]), [10.0, 20.0, 30.0]);
    assert_eq!(placement_euler_to_world([10.0, 20.0, 30.0]), [-30.0, -10.0, 20.0]);

    // A 90° turn about the world's z moves the model's forward axis to the
    // next axis. Compare the basis a right-handed 90° turn about z produces
    // with the one the file's angle produces.
    let column = |m: &[f32; 16], col: usize| [m[col * 4], m[col * 4 + 1], m[col * 4 + 2]];
    let flat = placement_matrix([0.0; 3], [0.0; 3], 1.0);
    let about_z = placement_matrix(
        [0.0; 3],
        placement_euler_from_world([0.0, 0.0, 90.0]),
        1.0,
    );
    // Rz(90) maps (x, y, z) to (-y, x, z), so the first column of the turned
    // basis is the flat one rotated that way.
    let was = column(&flat, 0);
    let now = column(&about_z, 0);
    assert!((now[0] - -was[1]).abs() < 1e-4, "{was:?} -> {now:?}");
    assert!((now[1] - was[0]).abs() < 1e-4, "{was:?} -> {now:?}");
    assert!((now[2] - was[2]).abs() < 1e-4, "{was:?} -> {now:?}");
}

/// The bit `hole_bit` returns for a point is the bit the mesher skips there.
///
/// `hole_bit` saves a tool from computing the mask again from the chunk
/// origin. A hole cut one square away from the pointer is still a valid hole
/// in a file that still parses, so nothing reports it. The test therefore
/// checks against the two existing readers: `Mcnk::is_hole`, and the ground
/// returning no height over a hole.
#[test]
fn the_bit_a_hole_tool_cuts_is_the_bit_the_mesher_skips() {
    let origin = [100.0f32, 200.0, 0.0];
    for bit in 0..16usize {
        let (high, low) = hole_square(origin, bit);
        let middle = [(high[0] + low[0]) * 0.5, (high[1] + low[1]) * 0.5];
        assert_eq!(
            hole_bit(origin, middle[0], middle[1]),
            Some(bit),
            "the middle of bit {bit}'s own square is bit {bit}"
        );

        // `is_hole` reports the same square, over every cell it covers.
        let mut chunk = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 1 << bit);
        chunk.position = origin;
        for row in 0..INNER_SIDE {
            for col in 0..INNER_SIDE {
                let covered = (row / 2) * 4 + col / 2 == bit;
                assert_eq!(chunk.is_hole(row, col), covered, "bit {bit} at {row},{col}");
            }
        }
        // The ground returns no height in the square.
        assert_eq!(
            chunk.ground().height_at(middle[0], middle[1]),
            None,
            "bit {bit} is a hole in the ground"
        );
    }

    // A point off the chunk on either axis has no bit, not the nearest one.
    assert_eq!(hole_bit(origin, origin[0] + 1.0, origin[1] - 1.0), None);
    assert_eq!(hole_bit(origin, origin[0] - 1.0, origin[1] + 1.0), None);
    assert_eq!(hole_bit(origin, origin[0] - CHUNK_SIZE - 1.0, origin[1] - 1.0), None);
}

/// One real tile from the game data, or `None` when it is not available.
///
/// Most tests in this file use synthetic bytes. The two tests that call this
/// check what the shipped files contain, so they need a real tile.
fn real_tile(map: &str, x: u32, y: u32) -> Option<Adt> {
    let dir = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| "../../Data".into());
    let mut assets = crate::Assets::open(&dir).ok()?;
    let raw = assets.read(&crate::adt_path(map, x, y)).ok()?;
    Adt::parse(&raw).ok()
}

/// `layer_flags::scroll` turns a layer's animation bits into a UV velocity,
/// and returns `None` unless `ANIMATION_ENABLED` is set.
///
/// Each of three properties would still draw plausibly if it were wrong:
///
/// * Direction 0 scrolls along +V, not +U. The two look the same on a
///   tileset with no grain and differ visibly on lava.
/// * Each speed step doubles the previous one. The field is three bits and
///   covers rates from a drifting shadow to a running waterfall; a linear
///   reading would put seven steps inside a factor of eight.
/// * `ANIMATION_ENABLED` gates both direction and speed. A layer with a
///   direction and no enable bit is still, and Azeroth has 42 more of those
///   than moving layers.
#[test]
fn a_layers_animation_bits_become_a_uv_velocity() {
    use layer_flags as f;
    let flags = |turn: u32, rate: u32, on: bool| {
        (turn & 7) | ((rate & 7) << f::ANIMATION_SPEED_SHIFT) | if on { f::ANIMATION_ENABLED } else { 0 }
    };

    assert_eq!(f::scroll(flags(3, 4, false)), None, "the switch gates it");
    assert_eq!(f::scroll(0), None);
    assert_eq!(
        f::scroll(f::USE_ALPHA_MAP | f::OVERBRIGHT),
        None,
        "the bits above the animation say nothing about it"
    );

    let (u, v) = f::scroll(flags(0, 0, true)).expect("on");
    assert!(u.abs() < 1e-6, "direction 0 is not along u: {u}");
    assert!((v - f::ANIMATION_BASE_RATE).abs() < 1e-6, "…and is +v at the base rate: {v}");

    let (u, v) = f::scroll(flags(2, 0, true)).expect("on");
    assert!((u - f::ANIMATION_BASE_RATE).abs() < 1e-6, "direction 2 is +u: {u}");
    assert!(v.abs() < 1e-6, "…and nothing along v: {v}");

    let (u, v) = f::scroll(flags(4, 0, true)).expect("on");
    assert!(u.abs() < 1e-6 && (v + f::ANIMATION_BASE_RATE).abs() < 1e-6, "4 is the reverse of 0");

    // Each step doubles, and the eighth is 128 times the first.
    let rate = |step: u32| {
        let (u, v) = f::scroll(flags(0, step, true)).expect("on");
        u.hypot(v)
    };
    for step in 1..8 {
        assert!(
            (rate(step) - rate(step - 1) * 2.0).abs() < 1e-5,
            "speed {step} is not twice speed {}", step - 1
        );
    }
    assert!((rate(7) / rate(0) - 128.0).abs() < 1e-3);
}

/// A layer's scroll velocity reaches its draw group, which is how the shader
/// receives it.
///
/// A still chunk must not share a draw group with a scrolling one, because
/// the velocity is one value per draw group. Grouping by textures alone
/// would make every chunk that shares the lava's tileset scroll, including
/// the dry rock beside it. `vale textures` reports 46 animated layers on
/// Azeroth 36 46, a tile that has both still and scrolling chunks.
#[test]
fn a_scrolling_layer_reaches_its_own_draw_group() {
    let Some(adt) = real_tile("Azeroth", 36, 46) else {
        eprintln!("no archives; skipping");
        return;
    };
    let mesh = adt.to_mesh();
    let moving: Vec<&TerrainDraw> = mesh
        .draws
        .iter()
        .filter(|d| d.scrolls.iter().any(|s| *s != [0.0, 0.0]))
        .collect();
    assert!(
        !moving.is_empty(),
        "the tile vale textures counts 46 animated layers on has no moving draw group"
    );
    assert!(
        mesh.draws.iter().any(|d| d.scrolls.iter().all(|s| *s == [0.0, 0.0])),
        "…and the dry ground on the same tile is not moving"
    );
    // Every draw group's scroll list is as long as its texture list.
    // Otherwise the pairing into the material's two vectors puts one layer's
    // velocity on another layer.
    for draw in &mesh.draws {
        assert_eq!(draw.scrolls.len(), draw.textures.len());
    }
    // The same textures with different scrolls are different groups.
    for a in &moving {
        for b in &mesh.draws {
            if a.textures == b.textures && a.scrolls != b.scrolls {
                assert_ne!(a.index_start, b.index_start, "two groups share a range");
            }
        }
    }
}

/// Two chunks side by side whose files disagree about their shared edge, as
/// the shipped and edited tiles do: an origin rounded away from the lattice by
/// a fraction of a millimetre, and edge heights a few millimetres apart. Every
/// vertex the two meshes share comes out as the same `f32` triple, so no gap
/// is left between them.
#[test]
fn two_chunks_share_their_edge_vertices_exactly() {
    let left = Mcnk {
        position: [-8533.333_984, 0.0, 236.7],
        ..test_chunk((0..HEIGHTS_PER_CHUNK).map(|i| (i % 17) as f32 * 0.37).collect(), 0)
    };
    // The next chunk across (decreasing y), its origin off the lattice as
    // the files have it, and its column-0 heights 4 mm off its neighbour's
    // column 8.
    let mut heights: Vec<f32> = (0..HEIGHTS_PER_CHUNK).map(|i| (i % 17) as f32 * 0.21).collect();
    for row in 0..9 {
        heights[row * 17] = left.heights[row * 17 + 8] + 0.004;
    }
    let right = Mcnk {
        position: [-8533.333_984, -33.333_984, 236.7],
        ..test_chunk(heights, 0)
    };
    let adt = Adt {
        chunks: vec![left, right],
        ..one_chunk_tile(vec![0.0; HEIGHTS_PER_CHUNK], 0)
    };
    let mesh = adt.to_mesh();
    let (start, count) = adt.chunk_vertex_span(1);
    let first: std::collections::HashSet<[u32; 3]> = mesh.positions[..start]
        .iter()
        .map(|p| p.map(f32::to_bits))
        .collect();
    let mut shared = 0;
    for p in &mesh.positions[start..start + count] {
        // On the shared edge by lattice x/y: the left chunk's column 8 is
        // the right chunk's column 0.
        let near = mesh.positions[..start]
            .iter()
            .find(|q| (q[0] - p[0]).abs() < 0.01 && (q[1] - p[1]).abs() < 0.01);
        if let Some(q) = near {
            shared += 1;
            assert_eq!(q.map(f32::to_bits), p.map(f32::to_bits), "{q:?} against {p:?}");
            assert!(first.contains(&p.map(f32::to_bits)));
        }
    }
    // Nine vertices on the edge, each emitted by two cells on each side.
    assert!(shared >= 9, "{shared} shared vertices found");
}

/// On a shipped tile, every vertex position that two chunks' meshes share is
/// the same `f32` triple in both. Vertex x and y come from the map's vertex
/// lattice; computed from each chunk's own origin, every chunk edge on
/// `Azeroth_32_48` was 0.65 mm from its neighbour's.
#[test]
fn a_real_tiles_chunks_share_their_edges_exactly() {
    let Some(adt) = real_tile("Azeroth", 32, 48) else {
        eprintln!("no archives: the shared-edge check did not run");
        return;
    };
    assert_eq!(seam_mismatches(&adt), 0);
}

/// How many vertex positions of a tile mesh lie within a tenth of a yard of
/// another chunk's vertex without being the same `f32` triple.
fn seam_mismatches(adt: &Adt) -> usize {
    let mesh = adt.to_mesh();
    let mut by_cell: std::collections::HashMap<(i64, i64), Vec<([u32; 3], usize)>> =
        std::collections::HashMap::new();
    for chunk in 0..adt.chunks.len() {
        let (start, count) = adt.chunk_vertex_span(chunk);
        for p in &mesh.positions[start..start + count] {
            let key = ((p[0] * 10.0).round() as i64, (p[1] * 10.0).round() as i64);
            by_cell.entry(key).or_default().push((p.map(f32::to_bits), chunk));
        }
    }
    by_cell
        .values()
        .filter(|vertices| {
            vertices
                .iter()
                .any(|(bits, chunk)| vertices.iter().any(|(b, c)| c != chunk && b != bits))
        })
        .count()
}

/// The vertex layout: nine outer rows interleaved with eight inner ones,
/// running along decreasing world x and y from the chunk origin.
#[test]
fn the_vertex_grid_is_nine_by_nine_interleaved_with_eight_by_eight() {
    assert_eq!(HEIGHTS_PER_CHUNK, 145);
    assert_eq!(outer_index(0, 0), 0);
    assert_eq!(inner_index(0, 0), 9);
    assert_eq!(outer_index(1, 0), 17);
    assert_eq!(outer_index(8, 8), 144);

    assert_eq!(vertex_offset(0), (0.0, 0.0));
    let (dx, dy) = vertex_offset(inner_index(0, 0));
    assert!((dx - 0.5 * UNIT_SIZE).abs() < 1e-3 && (dy - 0.5 * UNIT_SIZE).abs() < 1e-3);
    let (dx, dy) = vertex_offset(outer_index(8, 8));
    assert!((dx - 8.0 * UNIT_SIZE).abs() < 1e-3 && (dy - 8.0 * UNIT_SIZE).abs() < 1e-3);
    let (dx, dy) = vertex_offset(inner_index(3, 5));
    assert!((dx - 3.5 * UNIT_SIZE).abs() < 1e-3 && (dy - 5.5 * UNIT_SIZE).abs() < 1e-3);

    for row in 0..OUTER_SIDE {
        for col in 0..OUTER_SIDE {
            assert_eq!(outer_row_col(outer_index(row, col)), Some((row, col)));
        }
    }
    assert_eq!(outer_row_col(inner_index(2, 3)), None);
}

/// Chunk numbers run along `indexX` first: chunk 1 is `(1, 0)` and chunk 16
/// is `(0, 1)`. A step off the grid has no chunk.
#[test]
fn a_chunk_number_is_y_times_sixteen_plus_x() {
    assert_eq!(chunk_xy(0), (0, 0));
    assert_eq!(chunk_xy(1), (1, 0));
    assert_eq!(chunk_xy(16), (0, 1));
    assert_eq!(chunk_xy(255), (15, 15));
    for index in 0..CHUNKS_PER_SIDE * CHUNKS_PER_SIDE {
        let (x, y) = chunk_xy(index);
        assert_eq!(chunk_index(x, y), index);
        assert_eq!(chunk_on_grid(x as i32, y as i32), Some(index));
    }
    assert_eq!(chunk_on_grid(-1, 0), None);
    assert_eq!(chunk_on_grid(0, 16), None);
}

/// [`encode_shadow_map`] packs in the order [`decode_shadow_map`] unpacks.
/// The decoder copies texel 62 into 63 on each axis, so the fixture leaves
/// the last row and column lit and the round trip is exact.
#[test]
fn a_shadow_map_encodes_in_the_order_it_decodes() {
    let mut texels = vec![0u8; ALPHA_LEN];
    for (i, texel) in texels.iter_mut().enumerate() {
        let (row, col) = (i / ALPHA_SIDE, i % ALPHA_SIDE);
        if row < ALPHA_SIDE - 1 && col < ALPHA_SIDE - 1 && (row * 7 + col * 3) % 5 == 0 {
            *texel = 255;
        }
    }
    let bits = encode_shadow_map(&texels);
    assert_eq!(bits.len(), SHADOW_MAP_BYTES);
    assert_eq!(encode_shadow_map(&[255])[0], 0x01, "texel 0 is bit 0");
    let mut back = decode_shadow_map(&bits);
    let last = ALPHA_SIDE - 1;
    for t in 0..ALPHA_SIDE {
        back[t * ALPHA_SIDE + last] = 0;
        back[last * ALPHA_SIDE + t] = 0;
    }
    assert_eq!(back, texels);
}
