use super::*;

/// **`MCCV` is blue, green, red, alpha — and 0x7F is 1.0, not 0.5.**
///
/// Two claims that are invisible on grey and wrong in the same picture: a
/// decoder that read the bytes in the order they are written would swap a
/// chunk's warm and cool halves, and one that divided by 255 would darken every
/// painted vertex by half while leaving an unpainted one alone. Both would look
/// like *the shading is a bit off* rather than like a bug.
///
/// See `vale_edit::adt::colours`, which is the writing half of the same two
/// claims.
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


/// **The two halves of the placement conversion are inverses.** Loading the
/// game's own tiles exercises one direction and nothing exercises the other,
/// which is the direction anything that *writes* a placement uses — so a
/// swapped axis there is a doodad saved a hundred yards from where it was put,
/// on a file that still parses.
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

    // Real numbers: the first doodad of Azeroth_34_51, a Duskwood tree.
    // That tile's terrain runs x -10666..-10133 and y -1600..-1066, and the
    // tree must land on it — allowing the overhang of objects listed in
    // every tile they touch, which on this tile reaches about 50 yards.
    let [x, y, z] = placement_to_world([18117.72, 30.206, 27210.664]);
    assert!((-10716.0..-10083.0).contains(&x), "x {x}");
    assert!((-1650.0..-1016.0).contains(&y), "y {y}");
    // Transposing the two horizontal axes is the classic failure and is not
    // subtle here: it would put this tree ten thousand yards away.
    assert!(x < -10000.0 && y > -2000.0);
    assert_eq!(z, 30.206);
}

#[test]
fn an_unrotated_placement_keeps_the_model_upright() {
    // Whatever else the matrix does, a tree must not lie down: the model's
    // +Z has to come out as world +Z, and the scale has to reach it.
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
    // And it is still a rotation: up stays up.
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
    // 0xF scaled by 17, not 16: a layer meant to be opaque must actually
    // reach 255 or the layer beneath it shows through everywhere.
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

/// **Every value the decode can produce survives the encode.** The pair is
/// multiply-by-17 one way and nearest-nibble the other, so the check is that
/// they are inverses on the sixteen values that can actually be in a file — a
/// truncating encode turns 255 into 0xE and every fully-opaque layer into one
/// that leaves a ghost.
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

/// …and so does a whole chunk's `MCAL`, at either stride, with the layer
/// offsets the encoder hands back reading the maps out again.
///
/// **`alpha_stride` is asked rather than told**, because that is what a reader
/// of the written file does: the offsets the encoder produced have to be far
/// enough apart for the gap rule to measure the stride back.
#[test]
fn a_chunks_alpha_maps_survive_the_write() {
    for stride in [ALPHA_LEN / 2, ALPHA_LEN] {
        // Layer 0 is the base and is opaque by definition, so it is the one
        // map the pair cannot carry anything else in.
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

/// **A writer's decode is the file's bytes and a renderer's is not.** They
/// differ in the last row and column by construction — see `fix_alpha_edge` — so
/// a caller that decoded through the renderer's copy, changed nothing and wrote
/// it back would overwrite every chunk's far edge with its neighbour.
#[test]
fn the_writers_decode_leaves_the_far_edge_alone() {
    let mut map = vec![0u8; ALPHA_LEN];
    for row in 0..ALPHA_SIDE {
        for col in 0..ALPHA_SIDE {
            // 255 on the last row and column, 0 everywhere else: the edge the
            // renderer replaces is the only thing in the map.
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

/// **The seam-grid regression.** A 64x64 alpha map holds 63x63 of blend: the
/// pre-Cataclysm client puts texel 62's centre on the chunk's far edge and
/// never reads row or column 63, so nothing meaningful was ever written
/// there. Drawing it paints a line on the far edge of all 256 chunks in
/// every tile, and because it appears on a chunk's far edges but not its
/// near ones, it reads as a checkerboard rather than as an error.
///
/// Measured on real tiles by `vale textures` — see [`fix_alpha_edge`].
#[test]
fn the_unwritten_last_row_and_column_are_replaced_by_their_neighbours() {
    let layers = vec![layer(0, 0), layer(layer_flags::USE_ALPHA_MAP, 0)];
    // 4-bit source: every nibble 0xF (=> 255) except the last column and the
    // last row of the map, which are 0 — the shape of the real artefact.
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
    // And nothing else moved.
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
    // The cell is the chunk's *position in the list*, and `atlas_uv` has to
    // agree with `alpha_atlas` about that or every chunk is blended with some
    // other chunk's map — which looks like a wrong alpha decode, not like a
    // wrong lookup.
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
        // Sample where the middle of the chunk maps to, through the very
        // function the mesh uses.
        let [u, v] = atlas_uv(i, 0.5, 0.5);
        let (x, y) = (
            (u * ATLAS_SIDE as f32) as usize,
            (v * ATLAS_SIDE as f32) as usize,
        );
        let o = (y * ATLAS_SIDE + x) * 4;
        assert_eq!(atlas[o], i as u8 + 1, "chunk {i} at atlas ({x}, {y})");
    }

    // Chunk 16 starts the second row of cells — the wrap is the thing most
    // easily got backwards.
    let [u, v] = atlas_uv(16, 0.0, 0.0);
    assert!(u < 1.0 / CHUNKS_PER_SIDE as f32, "{u} should be in column 0");
    assert!(v > 1.0 / CHUNKS_PER_SIDE as f32, "{v} should be past row 0");
}

#[test]
fn an_atlas_lookup_stays_inside_its_own_cell() {
    // The whole cost of the atlas: a lookup exactly on a cell boundary has
    // bilinear filtering average in the neighbouring chunk's blend, which
    // paints a seam grid over the landscape. Both ends of the range have to
    // sit a half texel inside.
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

/// **The claim that kept the atlas unmipped for a milestone, tested.** It
/// was left at one level because "a level below the top averages across its
/// cells" — and it does not, not while a cell is wider than a level's texel.
/// A level-`n` texel is an aligned `2^n` block of level-0 texels and a cell
/// is 64 wide, so no block straddles two cells until a cell *is* one texel,
/// which is where [`ALPHA_ATLAS_MIPS`] stops.
///
/// Two neighbouring cells filled with values that could not be confused —
/// 255 and 0 — must therefore stay 255 and 0 the whole way down. Anything
/// else is a chunk being blended with its neighbour's map, which renders as
/// slightly wrong ground rather than as an error.
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
    // And the last level is one texel per chunk: 16x16 for a 16x16 tile.
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
    // The draw-call fix. Four chunks, two texture sets, and the result has to
    // be two draws over contiguous index ranges — contiguous because a draw
    // is one `index_start`/`index_count` pair and not a scan.
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
        // Deliberately interleaved: the grouping has to gather chunks that
        // are not adjacent in the file.
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

    // And every index still points at a vertex that exists — the reorder is
    // where that would go wrong.
    let vertices = mesh.positions.len() as u32;
    assert!(mesh.indices.iter().all(|&i| i < vertices));
}

#[test]
fn a_group_bounding_sphere_covers_every_chunk_in_it() {
    // Two chunks a tile apart sharing a texture set: the merged sphere has to
    // reach both, or half the group vanishes when the other half is on screen.
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
    // And the tile's own sphere, which is what the renderer culls by.
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
    // The renderer culls by these spheres, and a sphere that is too small
    // makes chunks blink out at the edge of the view — a bug that only
    // shows up while the camera is moving.
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
    // Two chunks with *different* texture sets, so they stay two draws and
    // the effect of the holes on the ranges is visible.
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
    // exactly where chunk 0 ended.
    assert_eq!(mesh.draws[0].index_count as usize, (64 - 4) * 4 * 3);
    assert_eq!(mesh.draws[1].index_start, mesh.draws[0].index_count);
    assert_eq!(mesh.draws[1].index_count as usize, 64 * 4 * 3);
}

#[test]
fn a_chunk_that_is_all_holes_gets_no_draw_at_all() {
    // 0xFFFF is every one of the 4x4 hole sub-cells set, so the chunk has no
    // geometry — and an empty range in a group would be a draw call for
    // nothing, which is the opposite of the point.
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

/// **`shadowed_at` samples the same texel the atlas puts under that
/// ground.** The mapping has two ways to be plausibly wrong — the two axes
/// swapped, or the texel taken from the wrong end of the chunk — and both
/// still return an in-range bit, so the test sets exactly one texel and
/// asks on both sides of both axes. The chunk origin is (0, 0) and the
/// chunk runs in *decreasing* x and y, like `to_mesh`'s vertices.
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

    // Outside the tile, and over a chunk with no MCSH at all, the answer
    // is "lit" — the conservative default for a sun scale.
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

    // A quarter of the way into the first cell should read 0.25 on every
    // one of the four wedges, which is the point of matching the topology.
    let q = -UNIT_SIZE * 0.25;
    for y in [q, -UNIT_SIZE * 0.75] {
        let h = adt.height_at(q, y).expect("inside");
        assert!((h - 0.25).abs() < 0.01, "got {h} at y={y}");
    }
}

/// **The slope a model is stood on comes off the same triangle its height
/// did.** A normal read from a different wedge — or from `MCNR`, whose smoothed
/// shading normals are the tempting alternative — tilts a mount by a number
/// that disagrees with the ground under its feet.
#[test]
fn the_ground_normal_is_the_slope_the_height_is_interpolated_on() {
    // Flat: straight up, and *exactly* so, since this is what every model in
    // the world that does not lean is measured against.
    let flat = one_chunk_tile(vec![5.0; HEIGHTS_PER_CHUNK], 0);
    assert_eq!(flat.normal_at(-1.0, -1.0), Some([0.0, 0.0, 1.0]));

    // A plane sloping one yard up per yard of *decreasing* x — walking away
    // from the chunk origin is walking uphill. The normal therefore leans
    // toward +x, and at 45° it is exactly `(1, 0, 1)/sqrt(2)`.
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
    // Every wedge of the cell, because the four are separate branches and a
    // sign that is right in one is not necessarily right in the others.
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

    // …and it answers nothing exactly where the height does: over a hole and
    // off the tile.
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
    // Positive x is off the chunk entirely — cells only run downwards.
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
    // 8x8 cells x 4 triangles.
    assert_eq!(adt.to_mesh().triangle_count(), 64 * 4);

    // One hole bit removes a 2x2 block of cells => 4 cells => 16 triangles.
    let holed = Adt {
        chunks: vec![test_chunk(heights, 0b1)],
        ..adt.clone()
    };
    assert_eq!(holed.to_mesh().triangle_count(), (64 - 4) * 4);
}

// -----------------------------------------------------------------------
// MCLQ — the terrain's own liquid
// -----------------------------------------------------------------------

/// One MCLQ block: min/max, 81 vertices, 64 tile bytes, the flow tail.
///
/// Byte 0 of each vertex is its own file index and the height is `height`,
/// so the re-order into the grid frame is visible in the assertions.
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
            b.push(if wet(row, col) { 0x00 } else { 0x0F });
        }
    }
    b.extend_from_slice(&0u32.to_le_bytes()); // nFlowvs
    b.extend_from_slice(&[0u8; 80]); // two flow vectors, always present
    assert_eq!(b.len(), MCLQ_BLOCK_SIZE);
    b
}

/// An MCNK payload whose MCLQ is written the way the real tools wrote it:
/// the sub-chunk's own size field is **zero**, and the honest extent is the
/// header's `sizeLiquid`.
fn mcnk_with_liquid(flags: u32, blocks: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = vec![0u8; MCNK_HEADER_SIZE];
    payload[0x00..0x04].copy_from_slice(&flags.to_le_bytes());
    payload[0x68..0x6C].copy_from_slice(&100f32.to_le_bytes());
    payload[0x6C..0x70].copy_from_slice(&200f32.to_le_bytes());
    payload[0x70..0x74].copy_from_slice(&7f32.to_le_bytes());
    let data: Vec<u8> = blocks.iter().flatten().copied().collect();
    payload[0x64..0x68].copy_from_slice(&((data.len() + 8) as u32).to_le_bytes());
    payload.extend_from_slice(b"QLCM");
    payload.extend_from_slice(&0u32.to_le_bytes()); // the famous lie
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

/// **`MCSH` unpacks LSB first, and nothing in the file says so.** The other
/// order is not a failure — it mirrors every group of eight texels inside
/// its own byte, which draws a shadow with a comb through it at a period of
/// 8 texels along one axis only. The fixture is one lit texel at the start
/// of a byte: under the wrong order it comes out at the *end* of that byte.
#[test]
fn a_shadow_map_unpacks_lsb_first() {
    // Byte 0 = 0b0000_0001: texel 0 shadowed, texels 1..7 lit.
    let chunk = parse_mcnk(&mcnk_with_shadow(mcnk_flags::HAS_MCSH, &[0x01]))
        .expect("parses");
    assert_eq!(chunk.shadow.len(), ALPHA_LEN);
    assert_eq!(chunk.shadow[0], 255, "bit 0 is texel 0, not texel 7");
    assert_eq!(&chunk.shadow[1..8], &[0; 7]);

    // And the high bit is the eighth texel, which is the other end.
    let high = parse_mcnk(&mcnk_with_shadow(mcnk_flags::HAS_MCSH, &[0x80]))
        .expect("parses");
    assert_eq!(&high.shadow[..7], &[0; 7]);
    assert_eq!(high.shadow[7], 255);
}

/// **The header flag is what says to read it.** The sub-chunk is written
/// whether or not anything is shadowed, so a chunk that declares no shadow
/// gets none however many bits are sitting in the file — and the empty
/// vector is what puts 0 in the atlas's alpha rather than a stale bitmap.
#[test]
fn a_chunk_that_declares_no_shadow_reads_none() {
    let chunk = parse_mcnk(&mcnk_with_shadow(0, &[0xFF; 64])).expect("parses");
    assert!(chunk.shadow.is_empty());

    // …and a chunk that declares one but has no sub-chunk is empty too,
    // rather than a panic on a missing block.
    let mut headerless = vec![0u8; MCNK_HEADER_SIZE];
    headerless[0x00..0x04].copy_from_slice(&mcnk_flags::HAS_MCSH.to_le_bytes());
    headerless[0x64..0x68].copy_from_slice(&8u32.to_le_bytes());
    assert!(parse_mcnk(&headerless).expect("parses").shadow.is_empty());
}

/// The last row and column are the neighbours' copies, exactly as an alpha
/// map's are: the client stretches a 64x64 map so texel 62's centre sits on
/// the chunk's edge and never reads 63. Drawing the unwritten strip puts a
/// shadow seam on the far edge of all 256 chunks in a tile.
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

/// The shadow rides in the atlas's alpha, in the same cell and at the same
/// texel as the chunk's blend maps — which is the whole reason it is in
/// there rather than in a texture of its own.
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

/// The blocks appear in flag-bit order and the broken size field is
/// ignored; a second block is read one whole stride — flow tail included —
/// after the first.
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
    // 255 — the union rule, unchanged from the WMO path.
    assert_eq!(chunk.liquids[1].1.opacity(40, Liquid::Magma), 255);
}

/// The file's rows run in *decreasing* world x from the chunk origin and
/// `WmoLiquid` runs +X from its base, so file vertex `(0, 0)` is grid
/// vertex `(8, 8)` — the chunk origin itself — and file tile `(0, 0)` is
/// grid tile `(7, 7)`. Getting this turn wrong still draws a plausible
/// lake; what it misplaces is every depth and every wet flag in it.
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
    // File vertex (1, 0) — one row down, index 9 — lands at grid (7, 8).
    assert_eq!(grid.opacity(8 * 9 + 7, Liquid::Water), 9);

    // Only file tile (0, 0) is wet, which is grid tile (7, 7).
    let wet: Vec<(usize, usize)> = (0..8)
        .flat_map(|y| (0..8).map(move |x| (x, y)))
        .filter(|&(x, y)| grid.has_liquid(x, y))
        .collect();
    assert_eq!(wet, [(7, 7)]);
}

/// The same block with a **sloping** surface: file vertex `(row, col)` stands
/// at `base + row`, so the height falls as world x rises (the file's rows run
/// in decreasing world x). Flat water cannot tell an interpolation from a
/// nearest-vertex lookup; this can.
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
    b.extend_from_slice(&[0u8; MCLQ_TILES]); // every tile wet
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&[0u8; 80]);
    assert_eq!(b.len(), MCLQ_BLOCK_SIZE);
    b
}

/// **Where the water stands over a point** — the question the mover asks
/// twenty times a second, and the one thing that turns a lake from something
/// drawn into something swum in.
///
/// Three properties, and each has its own failure. A **dry tile answers
/// nothing**, which is what makes a pool a shape rather than a rectangle — a
/// lookup that ignored the low-nibble-15 flag would put an invisible surface
/// over the whole chunk and start the character swimming on dry land. The
/// surface is **interpolated across the tile**, matching the quad
/// [`Adt::liquid_surface`] emits, so a swimmer's water line and the one they
/// can see are the same ramp. And where two liquids stand over one point the
/// **higher wins**, because nothing in the file orders them and a river running
/// into the sea is the shipped case.
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
    // …and four tiles over is dry, however much water the grid declares.
    assert_eq!(chunk.liquid_at(100.0 - tile * 4.5, 200.0 - tile * 4.5), None);
    // Off the grid entirely.
    assert_eq!(chunk.liquid_at(500.0, 200.0), None);

    // The ramp: grid vertex x = 8 sits at world x = 100 and carries the base,
    // and each tile back along x is a yard higher.
    let sloped = parse_mcnk(&mcnk_with_liquid(mcnk_flags::LQ_RIVER, &[mclq_sloped(30.0)]))
        .expect("parses");
    let at = |x: f32| sloped.liquid_at(x, 200.0 - tile * 0.5).expect("wet").1;
    assert!((at(100.0 - tile * 0.001) - 30.0).abs() < 0.05, "{}", at(100.0));
    assert!((at(100.0 - tile) - 31.0).abs() < 0.01);
    // Half a tile in is half a yard — which is the whole of what interpolating
    // buys: a nearest-vertex read would answer 30.0 or 31.0 here.
    assert!((at(100.0 - tile * 0.5) - 30.5).abs() < 0.01, "{}", at(100.0 - tile * 0.5));

    // Two liquids over one point: the higher surface is the one you are in.
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

/// A dry chunk is `sizeLiquid == 8`; a chunk with no liquid flag reads
/// nothing whatever the size claims; a truncated block costs the blocks,
/// not the chunk.
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

/// The assembled surface is the WMO emission on the terrain's grids: two
/// up-facing triangles per wet tile, the depth byte as the vertex alpha,
/// merged into one draw per kind.
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

    // And the winding faces +Z, like a WMO pool's.
    let p = |i: usize| {
        let v = draw.positions[draw.indices[i] as usize];
        (v[0], v[1], v[2])
    };
    let (a, b, c) = (p(0), p(1), p(2));
    let cross_z = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    assert!(cross_z > 0.0, "the surface faces up, not into the lake");
}

/// **A building the caller has not staged is a hole in the answer, not a
/// licence to use the ground.** `MODF` states the box before anything reads the
/// `.wmo`, so the tile that answers a height also knows a building stands over
/// it — and Stormwind's terrain is far below Stormwind's floors, so the caller
/// that takes it in the meantime falls through the world.
///
/// The point is the whole of what the client tests before it waits for the
/// object to load, so this checks all three ways it can be wrong:
/// inside, outside, and inside-but-already-settled.
#[test]
fn a_point_inside_an_unstaged_modf_box_has_no_floor_yet() {
    // A ten-yard cube around the map origin, in the file's own
    // (westward, up, northward) frame — so its world box is x/y -5..5 and
    // z 20..30.
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
    // The conversion is the one `vale wmos` checks the built geometry
    // against, and it re-sorts: a lower corner in the file is not a lower
    // corner in the world on the two axes that are measured backwards.
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
    // …and once the caller has decided about it, the ground is the answer
    // again — which is what stops a building with no hull at all being a
    // permanent hole.
    assert!(!adt.awaiting_building([0.0, 0.0, 25.0], |_| false), "settled");
    // The predicate is asked about the placement's own id and no other.
    assert!(adt.awaiting_building([0.0, 0.0, 25.0], |id| id == 77));
    assert!(!adt.awaiting_building([0.0, 0.0, 25.0], |id| id == 78));

    // **…and the same boxes asked the other question**, which is the one that
    // does *not* stop being true when the building lands: a point inside a
    // `MODF` box is inside a building whatever its hull has decided. That is
    // what stops the mover taking the mountain over Ironforge as a floor at a
    // spot the city's own hull happens to answer nothing for — see
    // `Standing::standable`.
    assert!(adt.inside_building([0.0, 0.0, 25.0]), "inside, hull or no hull");
    assert!(!adt.inside_building([0.0, 0.0, 40.0]), "over the roof");
    assert!(!adt.inside_building([50.0, 0.0, 25.0]), "beside it");
    // The difference between the two, stated: settling the placement retires
    // the first question and leaves the second exactly where it was.
    assert!(!adt.awaiting_building([0.0, 0.0, 25.0], |_| false));
    assert!(adt.inside_building([0.0, 0.0, 25.0]));
}

/// The `_s` name goes **before** the extension and not after it, which is the
/// difference between a file the archives hold and one they never will.
#[test]
fn the_specular_variant_is_named_before_the_extension() {
    assert_eq!(
        specular_texture(r"Tileset\Duskwood\DuskwoodCobblestone.blp"),
        r"Tileset\Duskwood\DuskwoodCobblestone_s.blp"
    );
    // The *last* dot, since a path may carry others.
    assert_eq!(specular_texture(r"a.b\c.blp"), r"a.b\c_s.blp");
    // No extension at all: the client walks back to a dot it does not find
    // and appends, which is the same string this returns.
    assert_eq!(specular_texture("bare"), "bare_s");
    // And it is not idempotent by accident — asking twice is a name nothing
    // ships, which is what makes a double application show up as a magenta
    // ground rather than as nothing.
    assert_eq!(specular_texture("a_s.blp"), "a_s_s.blp");
}

/// **A cell written into a built atlas lands exactly where the builder put it,
/// at every level of the mip chain.**
///
/// This is what lets a host change one chunk's blend without rebuilding the
/// tile, and the failure it guards against is silent: a level indexed at the
/// wrong stride writes a chunk's blend over its neighbour's, which reads as
/// "the paint appears one chunk along" at distance and is correct up close.
#[test]
fn a_cell_written_into_an_atlas_lands_where_the_builder_put_it() {
    let levels = alpha_atlas_levels();
    assert_eq!(levels.len(), 1 + ALPHA_ATLAS_MIPS);
    assert_eq!(levels[0], (0, ATLAS_SIDE));
    assert_eq!(levels[1], (ATLAS_SIDE * ATLAS_SIDE * 4, ATLAS_SIDE / 2));
    // The last level is where a cell has become one texel, which is where the
    // chain stops.
    assert_eq!(levels[ALPHA_ATLAS_MIPS].1, ATLAS_SIDE / ALPHA_SIDE);

    let total: usize = levels.iter().map(|&(_, side)| side * side * 4).sum();
    let mut atlas = vec![0u8; total];
    // One cell in the middle of the grid, filled with a value no other cell
    // has, so anything landing outside it is visible as a stray byte.
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

/// **The three `MDDF` angles are turns about the world's z, y and x, in that
/// order and with those signs**, and the pair that says so round trips.
///
/// The check is against [`placement_matrix`] itself rather than against the
/// arithmetic: turning one angle has to move the model the way the named world
/// axis says, or a panel that labels a field "about z" is lying about what
/// dragging it does.
/// **A lean stands the model's up on the normal, whatever the turn**, checked
/// through the matrix itself rather than through the derivation: the record
/// [`lean_to_normal`] produces is pushed through [`placement_matrix`] and the
/// model's own `+Z` has to come out along the normal it was asked for.
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
    // Straight up leans nothing, and a normal under the horizon is refused
    // rather than standing the model on its head.
    assert_eq!(lean_to_normal([0.0, 0.0, 1.0], 45.0), [0.0, 0.0, 45.0]);
    assert_eq!(lean_to_normal([0.0, 0.0, -1.0], 45.0), [0.0, 0.0, 45.0]);
}

#[test]
fn the_placement_angles_are_turns_about_the_worlds_axes() {
    let there_and_back = |rot: [f32; 3]| {
        placement_euler_from_world(placement_euler_to_world(rot))
    };
    assert_eq!(there_and_back([10.0, 20.0, 30.0]), [10.0, 20.0, 30.0]);
    assert_eq!(placement_euler_to_world([10.0, 20.0, 30.0]), [-30.0, -10.0, 20.0]);

    // A 90° turn about the world's z takes the model's own forward round to the
    // next axis, and the matrix is what says which. Compare the basis a
    // right-handed 90° about z produces against the one the file's own angle
    // produces.
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

/// **The bit a hole tool cuts is the bit the mesher skips.**
///
/// `hole_bit` exists so that a tool does not work the mask out again from the
/// chunk origin, and the failure it guards against is the quiet one: a quadrant
/// away from the pointer is still a hole, on a file that still parses, so
/// nothing anywhere says it went in the wrong place. The check is therefore
/// against the two readings that already exist — `Mcnk::is_hole`, and the ground
/// itself refusing a height where a hole is.
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

        // …and the same square is the one `is_hole` answers for, over every
        // cell it covers.
        let mut chunk = test_chunk(vec![0.0; HEIGHTS_PER_CHUNK], 1 << bit);
        chunk.position = origin;
        for row in 0..INNER_SIDE {
            for col in 0..INNER_SIDE {
                let covered = (row / 2) * 4 + col / 2 == bit;
                assert_eq!(chunk.is_hole(row, col), covered, "bit {bit} at {row},{col}");
            }
        }
        // The ground refuses a height in the square and answers outside it,
        // which is the property a person cutting a hole is actually asking for.
        assert_eq!(
            chunk.ground().height_at(middle[0], middle[1]),
            None,
            "bit {bit} is a hole in the ground"
        );
    }

    // Off the chunk in either axis is no bit at all rather than the nearest.
    assert_eq!(hole_bit(origin, origin[0] + 1.0, origin[1] - 1.0), None);
    assert_eq!(hole_bit(origin, origin[0] - 1.0, origin[1] + 1.0), None);
    assert_eq!(hole_bit(origin, origin[0] - CHUNK_SIZE - 1.0, origin[1] - 1.0), None);
}

/// One real tile out of the install, or `None` where there is not one.
///
/// The same arrangement `vale_edit`'s tests use: most of this file works on
/// invented bytes, and the two checks below cannot — what they are about is
/// what the shipped files actually carry.
fn real_tile(map: &str, x: u32, y: u32) -> Option<Adt> {
    let dir = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| "../../Data".into());
    let mut assets = crate::Assets::open(&dir).ok()?;
    let raw = assets.read(&crate::adt_path(map, x, y)).ok()?;
    Adt::parse(&raw).ok()
}

/// **The animation bits come out as a UV velocity, and only when the switch is
/// on.**
///
/// Three properties, each of which would draw plausibly if it were wrong.
/// Direction 0 must crawl along +V and *not* +U: the two are indistinguishable
/// on a tileset with no grain and obvious on lava. Each speed step must double
/// the last, because the field is three bits covering a drifting shadow and a
/// running waterfall and a linear reading would put seven steps inside a factor
/// of eight. And the switch has to gate both, because a layer with a direction
/// and no switch is a still layer and there are 42 more of those in Azeroth
/// than there are moving ones.
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

/// …and it reaches a draw group, which is what decides whether the shader ever
/// sees it.
///
/// **A still chunk must not be folded in with a crawling one**, because the
/// velocity is one number per draw group: a grouping that keyed on the textures
/// alone would make every chunk sharing the lava's tileset crawl, including the
/// dry rock beside it. `vale textures` says Azeroth 36 46 carries 46
/// animated layers, which is the tile with both on it.
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
    // Every draw group's scroll list is as long as its texture list, or the
    // pairing into the material's two vectors puts one layer's velocity on
    // another layer.
    for draw in &mesh.draws {
        assert_eq!(draw.scrolls.len(), draw.textures.len());
    }
    // …and the same textures with different scrolls are different groups.
    for a in &moving {
        for b in &mesh.draws {
            if a.textures == b.textures && a.scrolls != b.scrolls {
                assert_ne!(a.index_start, b.index_start, "two groups share a range");
            }
        }
    }
}
