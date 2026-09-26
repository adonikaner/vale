use super::*;

fn encode(magic: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = vec![magic[3], magic[2], magic[1], magic[0]];
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    v
}

fn f32s(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A root with two materials sharing one texture, one group, one doodad set
/// and one doodad spawn.
fn build_root() -> Vec<u8> {
    let mut mohd = vec![0u8; 0x40];
    mohd[0x04..0x08].copy_from_slice(&1u32.to_le_bytes()); // nGroups
    // ambColor is a CArgb: r, g, b, a. Deliberately asymmetric values, so a
    // reader that used MOCV's BGRA layout here would be caught.
    mohd[0x1C..0x20].copy_from_slice(&[0x10, 0x20, 0x30, 0xFF]);
    mohd[0x20..0x24].copy_from_slice(&77u32.to_le_bytes()); // wmoId
    mohd[0x24..0x30].copy_from_slice(&f32s(&[-5.0, -5.0, 0.0]));
    mohd[0x30..0x3C].copy_from_slice(&f32s(&[5.0, 5.0, 10.0]));

    // MOTX: two paths, so the second material's offset is non-zero.
    let mut motx = Vec::new();
    motx.extend_from_slice(b"World\\wall.blp\0");
    let roof_at = motx.len() as u32;
    motx.extend_from_slice(b"World\\roof.blp\0");

    let mut momt = vec![0u8; MATERIAL_SIZE * 3];
    // 0: opaque wall, two-sided.
    momt[0x00..0x04].copy_from_slice(&material_flags::UNCULLED.to_le_bytes());
    momt[0x08..0x0C].copy_from_slice(&0u32.to_le_bytes());
    momt[0x0C..0x10].copy_from_slice(&0u32.to_le_bytes());
    // 1: alpha-keyed roof, unlit.
    let m1 = MATERIAL_SIZE;
    momt[m1..m1 + 4].copy_from_slice(&material_flags::UNLIT.to_le_bytes());
    momt[m1 + 0x08..m1 + 0x0C].copy_from_slice(&1u32.to_le_bytes());
    momt[m1 + 0x0C..m1 + 0x10].copy_from_slice(&roof_at.to_le_bytes());
    // 2: shares the wall texture, so the dedupe is observable.
    let m2 = MATERIAL_SIZE * 2;
    momt[m2 + 0x0C..m2 + 0x10].copy_from_slice(&0u32.to_le_bytes());

    let mogn = b"\0entrance\0".to_vec();

    let mut mogi = vec![0u8; GROUP_INFO_SIZE];
    mogi[0x1C..0x20].copy_from_slice(&1u32.to_le_bytes()); // name offset

    let mut mods = vec![0u8; DOODAD_SET_SIZE];
    mods[..7].copy_from_slice(b"Default");
    mods[20..24].copy_from_slice(&0u32.to_le_bytes());
    mods[24..28].copy_from_slice(&1u32.to_le_bytes());

    let modn = b"World\\Lamp.mdx\0".to_vec();

    let mut modd = vec![0u8; DOODAD_DEF_SIZE];
    modd[0x00..0x04].copy_from_slice(&0u32.to_le_bytes()); // name offset 0
    modd[0x04..0x10].copy_from_slice(&f32s(&[1.0, 2.0, 3.0]));
    modd[0x10..0x20].copy_from_slice(&f32s(&[0.0, 0.0, 0.0, 1.0])); // identity
    modd[0x20..0x24].copy_from_slice(&2.0f32.to_le_bytes());

    let mut buf = encode(b"MVER", &17u32.to_le_bytes());
    buf.extend(encode(b"MOHD", &mohd));
    buf.extend(encode(b"MOTX", &motx));
    buf.extend(encode(b"MOMT", &momt));
    buf.extend(encode(b"MOGN", &mogn));
    buf.extend(encode(b"MOGI", &mogi));
    buf.extend(encode(b"MODS", &mods));
    buf.extend(encode(b"MODN", &modn));
    buf.extend(encode(b"MODD", &modd));
    buf
}

/// An `MLIQ` payload: a 2x2 tile grid (so 3x3 vertices) at a stated base,
/// with `wet` deciding which tiles carry liquid.
///
/// The heights are deliberately all different, so a header read at 32 bytes
/// instead of 30 — which is what `sizeof` gives after the compiler pads the
/// trailing `u16` — comes out with every one of them wrong rather than with
/// a plausible surface.
fn build_mliq(liquid_type: u16, wet: [bool; 4]) -> Vec<u8> {
    let mut mliq = Vec::new();
    for count in [3u32, 3, 2, 2] {
        mliq.extend_from_slice(&count.to_le_bytes());
    }
    mliq.extend(f32s(&[10.0, 20.0, -3.0]));
    mliq.extend_from_slice(&liquid_type.to_le_bytes());
    assert_eq!(mliq.len(), LIQUID_HEADER_SIZE, "MLIQ's header is 30 bytes");

    for vertex in 0..9u32 {
        // Byte 0 is the union's first field — the *depth* for water, the low
        // half of an `s` coordinate for magma — and is the surface's
        // opacity. A recognisable multiple of ten, so a vertex that took
        // some other vertex's depth is obvious.
        mliq.push((vertex * 10) as u8);
        // The rest of the four bytes before the height, filled with a
        // pattern so a misread shows up as one of them rather than as zero.
        mliq.push(0xAD);
        mliq.extend_from_slice(&0xBEEFu16.to_le_bytes());
        mliq.extend_from_slice(&(vertex as f32).to_le_bytes());
    }
    // Low nibble 15 is "no liquid"; anything else is a type.
    mliq.extend(wet.iter().map(|&w| if w { 0x00 } else { 0x0F }));
    mliq
}

#[test]
fn mliq_reads_a_grid_of_heights_and_a_mask_of_wet_tiles() {
    let liquid = WmoLiquid::parse(&build_mliq(0, [true, false, true, true])).expect("MLIQ");
    assert_eq!((liquid.x_tiles, liquid.y_tiles), (2, 2));
    assert_eq!(liquid.base, [10.0, 20.0, -3.0]);
    // Nine vertices, in row-major order along +X first. If the header were
    // read at 32 bytes these would start at 0.5 of a vertex in and every
    // one would be wrong.
    assert_eq!(liquid.heights, (0..9).map(|v| v as f32).collect::<Vec<_>>());
    assert_eq!(liquid.height(0, 0), 0.0);
    assert_eq!(liquid.height(2, 0), 2.0);
    assert_eq!(liquid.height(0, 1), 3.0);
    assert_eq!(liquid.wet_tiles(), 3);
    assert!(liquid.has_liquid(0, 0));
    assert!(!liquid.has_liquid(1, 0));
}

/// The grid's spacing is stated nowhere in the file, so a vertex's position
/// is entirely this client's constant. Pinned here as well as measured by
/// `vale water`, which reports every real grid's corner as a whole
/// number of tiles from the origin.
#[test]
fn a_vertex_is_the_base_plus_whole_tiles() {
    let liquid = WmoLiquid::parse(&build_mliq(0, [true; 4])).expect("MLIQ");
    assert_eq!(
        liquid.vertex(2, 1),
        [10.0 + 2.0 * LIQUID_TILE_SIZE, 20.0 + LIQUID_TILE_SIZE, 5.0]
    );
}

/// **The liquid vertex is a union and the type picks the member.** Byte 0
/// is a depth for water and ocean — and therefore the surface's opacity —
/// but the low half of an `s` texture coordinate for magma and slime, which
/// would be a meaningless opacity walking with the texture.
///
/// This is measured rather than assumed, and the measurement is in the
/// files: over Stormwind, which is all water, byte 0 is 0 at every dry
/// vertex and plateaus at 86 over the pool. Over Ironforge, which is mostly
/// magma, it is spread flat over all 256 values with no plateau at all.
#[test]
fn only_water_takes_its_opacity_from_the_vertex_depth() {
    let liquid = WmoLiquid::parse(&build_mliq(0, [true; 4])).expect("MLIQ");
    assert_eq!(liquid.depths, (0..9).map(|v| (v * 10) as u8).collect::<Vec<_>>());

    // Water and ocean: the depth itself, so a shallow bank fades out.
    assert_eq!(liquid.opacity(0, Liquid::Water), 0);
    assert_eq!(liquid.opacity(5, Liquid::Water), 50);
    assert_eq!(liquid.opacity(5, Liquid::Ocean), 50);
    // Magma and slime: opaque, because byte 0 there is a coordinate. Their
    // textures agree — `lava` and `slime` carry no alpha channel at all.
    assert_eq!(liquid.opacity(5, Liquid::Magma), 255);
    assert_eq!(liquid.opacity(5, Liquid::Slime), 255);
    // A vertex off the end is opaque rather than invisible: a damaged tail
    // should cost the ripple, not the whole pool.
    assert_eq!(liquid.opacity(99, Liquid::Water), 255);
}

/// A vertex belongs to no tile of its own, so "is there liquid here" is a
/// question about the up-to-four tiles that meet at it. The corners of the
/// grid have one, the middle has four.
#[test]
fn a_vertex_is_wet_when_any_tile_touching_it_is() {
    // Only tile (0,0) is wet, so the four vertices of that tile are wet and
    // the rest of the 3x3 grid is not.
    let liquid = WmoLiquid::parse(&build_mliq(0, [true, false, false, false])).expect("MLIQ");
    let wet: Vec<bool> = (0..9).map(|i| liquid.vertex_is_wet(i)).collect();
    assert_eq!(
        wet,
        [true, true, false, true, true, false, false, false, false]
    );
}

/// **The regression this exists to prevent.** The surface's opacity is the
/// vertex colour's alpha, and it has to be `MLIQ`'s depth — the texture's
/// own alpha is a foam mask (`lake_a` means 54 of 255) and blending a canal
/// by it leaves it invisible, which is exactly what it did.
#[test]
fn a_liquid_surface_carries_its_depth_as_the_vertex_alpha() {
    let root = WmoRoot::parse(&build_root()).expect("root");
    let mut group = WmoGroup::parse(&build_group(0, 0)).expect("group");
    group.batches.clear();
    group.liquid = WmoLiquid::parse(&build_mliq(0, [true; 4]));
    let model = WmoModel::assemble("t.wmo", &root, std::slice::from_ref(&group));

    let draw = model
        .draws
        .iter()
        .find(|d| d.liquid == Some(Liquid::Water))
        .expect("the liquid draw");
    assert_eq!(
        draw.light,
        BatchLight::Sun,
        "water is lit by the sun, not by MOCV"
    );
    assert_eq!(draw.blend, 2, "water blends");

    // Four vertices per wet tile, pushed anticlockwise from the tile's own
    // corner. Tile (0,0) spans grid vertices (0,0), (1,0), (1,1), (0,1) —
    // which in the 3x3 grid are rows 0 and 1, i.e. depths 0, 10, 40, 30.
    // Getting the row stride wrong here reads a plausible neighbouring
    // vertex's depth, which is why the expected values are spelled out.
    let indices = &model.indices
        [draw.index_start as usize..(draw.index_start + draw.index_count) as usize];
    let base = indices[0] as usize;
    let first: Vec<u8> = (0..4).map(|corner| model.colours[base + corner][3]).collect();
    assert_eq!(first, [0, 10, 40, 30], "each corner takes its own depth");

    // And the RGB stays black — it is the identity for the additive light
    // term, and water is not lit by `MOCV`.
    for &index in indices {
        assert_eq!(model.colours[index as usize][..3], [0, 0, 0]);
    }
}

/// A grid whose vertex count does not agree with its tile count is not a
/// grid, and both products index into the payload.
#[test]
fn a_disagreeing_vertex_and_tile_count_is_refused() {
    let mut mliq = build_mliq(0, [true; 4]);
    mliq[0..4].copy_from_slice(&5u32.to_le_bytes());
    assert!(WmoLiquid::parse(&mliq).is_none());

    // …and so is a payload that stops in the middle of its heights.
    let short = build_mliq(0, [true; 4]);
    assert!(WmoLiquid::parse(&short[..short.len() - 5]).is_none());
}

/// The four steps of vmangos' fixup, each of which changes the answer.
#[test]
fn the_liquid_type_is_not_the_number_in_the_group_header() {
    let mut group = WmoGroup::parse(&build_group(0, 0)).expect("group");
    let root = WmoRoot::parse(&build_root()).expect("root");
    assert_eq!(group.liquid_kind(&root), None, "no MLIQ, no liquid");

    // Without the root's DBC-id flag the group's number is one *less* than
    // the entry, so 0 is water.
    group.liquid = WmoLiquid::parse(&build_mliq(0, [true; 4]));
    group.liquid_type = 0;
    assert_eq!(group.liquid_kind(&root), Some(Liquid::Water));

    // The water/ocean split is carried by a group flag and by nothing in
    // MLIQ at all — an ocean read as water is invisible to every number.
    group.flags = group_flags::LIQUID_IS_OCEAN;
    assert_eq!(group.liquid_kind(&root), Some(Liquid::Ocean));
    group.flags = 0;

    // Folded modulo 4, which is what keeps the table four liquids wide.
    group.liquid_type = 2;
    assert_eq!(group.liquid_kind(&root), Some(Liquid::Magma));
    group.liquid_type = 3;
    assert_eq!(group.liquid_kind(&root), Some(Liquid::Slime));

    // 15 means "not stated here": the answer comes from the first tile that
    // states one, so a group can declare nothing and still be full of lava.
    group.liquid_type = 15;
    group.liquid = WmoLiquid::parse(&build_mliq(0, [true; 4]));
    group.liquid.as_mut().unwrap().tile_flags = vec![0x0F, 0x0F, 0x02, 0x00];
    assert_eq!(group.liquid_kind(&root), Some(Liquid::Magma));
}

/// A room whose only content is the water standing in it draws nothing
/// else, so the "no batches, skip it" rule had to learn about `MLIQ`.
#[test]
fn a_group_with_only_liquid_still_produces_a_draw() {
    let root = WmoRoot::parse(&build_root()).expect("root");
    let mut group = WmoGroup::parse(&build_group(0, 0)).expect("group");
    group.batches.clear();
    group.liquid = WmoLiquid::parse(&build_mliq(0, [true, true, false, true]));

    let model = WmoModel::assemble("t.wmo", &root, std::slice::from_ref(&group));
    assert_eq!(model.draws.len(), 1, "the liquid surface");
    let draw = &model.draws[0];
    // Two triangles per *wet* tile, not per tile: the grid is a rectangle
    // and the pool is not.
    assert_eq!(draw.index_count, 3 * 2 * 3);
    assert!(draw.two_sided, "a water surface is seen from underneath");
    assert_eq!(draw.blend, 2, "water blends");
    assert_eq!(
        draw.liquid,
        Some(Liquid::Water),
        "and the kind is what the colour is looked up by"
    );
    // The texture is appended to the model's own list, so the renderer
    // loads it with the building's.
    let texture = model.textures[draw.texture.unwrap() as usize].as_str();
    assert_eq!(texture, Liquid::Water.texture(1));
    // And every index it names is inside the buffers it was appended to.
    let range = draw.index_start as usize..(draw.index_start + draw.index_count) as usize;
    assert!(model.indices[range]
        .iter()
        .all(|&i| (i as usize) < model.positions.len()));
    assert_eq!(model.colours.len(), model.positions.len());
}

/// A group file: three vertices, one triangle, one batch on material 1.
fn build_group(flags: u32, material: u8) -> Vec<u8> {
    let mut mogp = vec![0u8; MOGP_HEADER_SIZE];
    mogp[0x00..0x04].copy_from_slice(&1u32.to_le_bytes()); // name offset
    mogp[0x08..0x0C].copy_from_slice(&flags.to_le_bytes());
    mogp[0x0C..0x18].copy_from_slice(&f32s(&[0.0, 0.0, 0.0]));
    mogp[0x18..0x24].copy_from_slice(&f32s(&[2.0, 2.0, 2.0]));

    let movi: Vec<u8> = [0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
    let movt = f32s(&[0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 4.0]);
    let monr = f32s(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    let motv = f32s(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);

    let mut moba = vec![0u8; BATCH_SIZE];
    moba[0x0C..0x10].copy_from_slice(&0u32.to_le_bytes()); // index start
    moba[0x10..0x12].copy_from_slice(&3u16.to_le_bytes()); // index count
    moba[0x14..0x16].copy_from_slice(&2u16.to_le_bytes()); // vertex end
    moba[0x17] = material;

    // MOGP's declared size covers only its own header here, which is the
    // broken-but-common case the parser has to survive.
    let mut buf = encode(b"MVER", &17u32.to_le_bytes());
    buf.extend(encode(b"MOGP", &mogp));
    buf.extend(encode(b"MOPY", &[0x20u8, material]));
    buf.extend(encode(b"MOVI", &movi));
    buf.extend(encode(b"MOVT", &movt));
    buf.extend(encode(b"MONR", &monr));
    buf.extend(encode(b"MOTV", &motv));
    buf.extend(encode(b"MOBA", &moba));
    buf
}

/// Stamp a built group's own `MOGP` id at 0x38 — the third key into
/// `WMOAreaTable`. See [`super::WmoGroup::group_id`].
fn with_group_id(mut raw: Vec<u8>, id: u32) -> Vec<u8> {
    let mogp = raw.windows(4).position(|w| w == b"PGOM").expect("MOGP") + 8;
    raw[mogp + 0x38..mogp + 0x3C].copy_from_slice(&id.to_le_bytes());
    raw
}

/// Append a `MOCV` to a built group and set the transparency-batch count in
/// its `MOGP` header. `vertex_end` patches the batch the boundary is read
/// from, so a test can put the split part-way through the group.
fn add_mocv(mut raw: Vec<u8>, colours: &[[u8; 4]], trans_batches: u16, vertex_end: u16) -> Vec<u8> {
    // Chunk magics are stored reversed on disk, so MOGP is "PGOM".
    let mogp = raw.windows(4).position(|w| w == b"PGOM").expect("MOGP") + 8;
    raw[mogp + 0x28..mogp + 0x2A].copy_from_slice(&trans_batches.to_le_bytes());
    let moba = raw.windows(4).position(|w| w == b"ABOM").expect("MOBA") + 8;
    raw[moba + 0x14..moba + 0x16].copy_from_slice(&vertex_end.to_le_bytes());

    let mocv: Vec<u8> = colours.iter().flatten().copied().collect();
    raw.extend(encode(b"MOCV", &mocv));
    raw
}

/// **A `MODD` colour is BGRA like `MOCV` and not RGBA like the ambient**,
/// which is the third place in this one format where the two structs
/// disagree — and reading it the wrong way round produces a plausible light
/// of the wrong hue rather than an error.
#[test]
fn a_doodad_spawns_light_is_bgra_and_zero_means_none() {
    let spawn = |colour| WmoDoodad {
        path: "t.m2".into(),
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        colour,
        matrix: doodad_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0),
        exterior_lit: false,
    };
    // Bytes in the file: B=0x20, G=0x40, R=0x80, A=0xFF.
    let light = spawn(0xFF80_4020).light().expect("a light");
    assert!((light[0] - 0x80 as f32 / 255.0).abs() < 1e-6, "red {light:?}");
    assert!((light[1] - 0x40 as f32 / 255.0).abs() < 1e-6, "green {light:?}");
    assert!((light[2] - 0x20 as f32 / 255.0).abs() < 1e-6, "blue {light:?}");

    // The colour is what decides, not the alpha byte: every spawn in the
    // game has a non-zero one, so keying off it would light nothing.
    assert!(spawn(0xFF00_0000).light().is_none());
    assert!(spawn(0x0000_0001).light().is_some());
}

#[test]
fn mocv_is_bgra_and_the_ambient_colour_beside_it_is_rgba() {
    // The two colour structs in this format disagree about byte order, and
    // reading either with the other's layout swaps red and blue — which
    // renders as a plausible colour rather than as an error.
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    assert_eq!(root.ambient[0], 0x10 as f32 / 255.0, "ambient red is byte 0");
    assert_eq!(root.ambient[2], 0x30 as f32 / 255.0, "ambient blue is byte 2");

    // MOCV byte order is B, G, R, A — the reverse of the ambient's.
    let raw = add_mocv(build_group(0, 1), &[[0xB0, 0x60, 0x40, 0xFF]], 0, 2);
    let group = WmoGroup::parse(&raw).expect("valid group");
    assert_eq!(group.colours[0][0], 0x40 as f32 / 255.0, "red is byte 2");
    assert_eq!(group.colours[0][1], 0x60 as f32 / 255.0, "green is byte 1");
    assert_eq!(group.colours[0][2], 0xB0 as f32 / 255.0, "blue is byte 0");
}

#[test]
fn root_reads_materials_and_deduplicates_their_textures() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    assert_eq!(root.version, 17);
    assert_eq!(root.group_count, 1);
    assert_eq!(root.wmo_id, 77);

    // Three materials, two distinct textures: materials 0 and 2 share one.
    assert_eq!(root.materials.len(), 3);
    assert_eq!(root.textures, vec!["World\\wall.blp", "World\\roof.blp"]);
    assert_eq!(root.materials[0].texture, Some(0));
    assert_eq!(root.materials[1].texture, Some(1));
    assert_eq!(root.materials[2].texture, Some(0));

    assert!(root.materials[0].two_sided() && !root.materials[0].unlit());
    assert!(root.materials[1].unlit() && !root.materials[1].two_sided());
    assert_eq!(root.materials[1].blend_mode, 1);
}

#[test]
fn group_names_come_from_mogn_by_byte_offset() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    assert_eq!(root.group_info.len(), 1);
    // MOGN is "\0entrance\0", so offset 1 is the name and offset 0 is empty.
    assert_eq!(root.group_name(root.group_info[0].name_offset), "entrance");
    assert_eq!(root.group_name(0), "");
    assert_eq!(root.group_name(-1), "");
    assert_eq!(root.group_name(9999), "");
}

#[test]
fn doodad_spawns_resolve_their_names_and_extension() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    assert_eq!(root.doodads.len(), 1);
    // MODN names models .mdx; the archive holds .m2.
    assert_eq!(root.doodads[0].path, "World\\Lamp.m2");
    assert_eq!(root.doodads[0].position, [1.0, 2.0, 3.0]);
    assert_eq!(root.doodads[0].scale, 2.0);

    assert_eq!(root.doodad_sets.len(), 1);
    assert_eq!(root.doodad_sets[0].name, "Default");
    assert_eq!(root.doodads_in_set(0).len(), 1);
    // A MODF naming a set the root does not have must not panic.
    assert!(root.doodads_in_set(9).is_empty());
}

#[test]
fn a_doodad_matrix_places_and_scales_without_rotating() {
    let m = doodad_matrix([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], 2.0);
    assert_eq!(apply(&m, [0.0, 0.0, 0.0]), [1.0, 2.0, 3.0]);
    assert_eq!(apply(&m, [1.0, 0.0, 0.0]), [3.0, 2.0, 3.0]);
    assert_eq!(apply(&m, [0.0, 0.0, 1.0]), [1.0, 2.0, 5.0]);

    // A quarter turn about +Z, as the file stores it: (x, y, z, w).
    let q = (std::f32::consts::FRAC_PI_4).sin_cos();
    let m = doodad_matrix([0.0, 0.0, 0.0], [0.0, 0.0, q.0, q.1], 1.0);
    let f = apply(&m, [1.0, 0.0, 0.0]);
    assert!((f[0]).abs() < 1e-5 && (f[1] - 1.0).abs() < 1e-5, "{f:?}");

    // A degenerate quaternion is identity, not NaN.
    let m = doodad_matrix([0.0, 0.0, 0.0], [0.0; 4], 1.0);
    assert_eq!(apply(&m, [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
}

#[test]
fn matrices_compose_in_the_order_the_renderer_needs() {
    // A doodad two units up inside a building placed ten units north: the
    // composed matrix must put it at ten north and two up, not the reverse.
    let placement = crate::world::adt::placement_matrix([10.0, 0.0, 0.0], [0.0; 3], 1.0);
    let local = doodad_matrix([0.0, 0.0, 2.0], [0.0, 0.0, 0.0, 1.0], 1.0);
    let composed = mul4(&placement, &local);
    assert_eq!(apply(&composed, [0.0, 0.0, 0.0]), [10.0, 0.0, 2.0]);
}

#[test]
fn group_reads_geometry_through_a_mogp_whose_size_is_only_its_header() {
    let group = WmoGroup::parse(&build_group(group_flags::INDOOR, 1)).expect("valid group");
    assert_eq!(group.flags, group_flags::INDOOR);
    assert!(group.is_indoor() && !group.is_antiportal());
    assert_eq!(group.positions.len(), 3);
    assert_eq!(group.positions[2], [0.0, 2.0, 4.0]);
    assert_eq!(group.normals.len(), 3);
    assert_eq!(group.uvs.len(), 3);
    assert_eq!(group.indices, vec![0, 1, 2]);
    assert_eq!(group.batches.len(), 1);
    assert_eq!(group.batches[0].material, Some(1));
    assert_eq!(group.triangle_count(), 1);
}

#[test]
fn a_batch_indexing_past_the_index_buffer_is_dropped_not_drawn() {
    let mut raw = build_group(0, 1);
    // Point the batch at 300 indices when the group has 3. Drawing it would
    // read whatever geometry follows in the buffer.
    let at = raw
        .windows(4)
        .position(|w| w == [b'A', b'B', b'O', b'M'])
        .expect("MOBA is in the file")
        + 8;
    raw[at + 0x10..at + 0x12].copy_from_slice(&300u16.to_le_bytes());

    let group = WmoGroup::parse(&raw).expect("geometry is still fine");
    assert_eq!(group.positions.len(), 3, "the vertices survive");
    assert!(group.batches.is_empty(), "the impossible batch does not");
}

#[test]
fn a_material_index_of_0xff_means_no_material() {
    let group = WmoGroup::parse(&build_group(0, 0xFF)).expect("valid group");
    assert_eq!(group.batches[0].material, None);

    // ...and assembling drops that batch rather than drawing it untextured
    // with material 255's worth of garbage.
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    let model = WmoModel::assemble("x.wmo", &root, &[group]);
    assert!(model.draws.is_empty());
}

#[test]
fn assembling_offsets_each_groups_indices_into_one_buffer() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    let groups = vec![
        WmoGroup::parse(&build_group(group_flags::EXTERIOR, 0)).unwrap(),
        WmoGroup::parse(&build_group(group_flags::INDOOR, 1)).unwrap(),
    ];
    let model = WmoModel::assemble("Building.wmo", &root, &groups);

    assert_eq!(model.positions.len(), 6);
    // The second group's indices must point at its own vertices, 3..5 — not
    // at the first group's, which is what forgetting the offset looks like.
    assert_eq!(model.indices, vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(model.groups.len(), 2);
    assert!(!model.groups[0].indoor && model.groups[1].indoor);
    assert_eq!(model.groups[1].draw_start, 1);

    assert_eq!(model.draws.len(), 2);
    assert_eq!(model.draws[1].index_start, 3);
    assert_eq!(model.draws[1].group, 1);
    // Material 1 is the unlit alpha-keyed roof.
    assert_eq!(model.draws[1].texture, Some(1));
    assert_eq!(model.draws[1].blend, 1);
    assert!(model.draws[1].unlit);
    assert!(model.draws[0].two_sided);

    // Bounds cover every vertex the model will actually draw.
    assert_eq!(model.bounds, [[0.0, 0.0, 0.0], [2.0, 2.0, 4.0]]);
    assert!(model.radius > 2.4);
}

/// **Only the indoor groups are rooms**, which is the whole of the
/// interiority test: a character standing next to an exterior wall is
/// outdoors, and the exterior group's box covers exactly where they are
/// standing.
#[test]
fn only_the_indoor_groups_hand_back_a_box() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    let groups = vec![
        WmoGroup::parse(&build_group(group_flags::EXTERIOR, 0)).unwrap(),
        WmoGroup::parse(&build_group(group_flags::INDOOR, 1)).unwrap(),
    ];
    let model = WmoModel::assemble("Building.wmo", &root, &groups);

    let inside = model.interior_bounds();
    assert_eq!(inside.len(), 1, "one of the two groups is a room");
    // The box is the group's own geometry, which is what the sphere beside
    // it was derived from — not `MOHD`'s, which covers the whole building.
    assert_eq!(inside[0], [[0.0, 0.0, 0.0], [2.0, 2.0, 4.0]]);
    assert!(box_contains(&inside[0], [1.0, 1.0, 2.0]));
    assert!(!box_contains(&inside[0], [1.0, 1.0, 5.0]), "above the ceiling");
    assert!(!box_contains(&inside[0], [-0.1, 1.0, 2.0]), "outside the wall");
    // The boundary is inside: a character standing exactly on the floor of a
    // room is in it, and the floor is the box's own bottom face.
    assert!(box_contains(&inside[0], [0.0, 0.0, 0.0]));
}

/// **…and every group is an area, room or not.** The two lists are different
/// questions asked of the same boxes: a city's districts are `EXTERIOR` groups
/// and a tavern's rooms are `INDOOR` ones, and both have a place name.
///
/// The id is read at `MOGP` 0x38 and pinned here, because 0x34 beside it is the
/// liquid type — a wrong offset there resolves to nothing in `WMOAreaTable` and
/// looks exactly like a building the table does not carry.
#[test]
fn every_group_carries_its_own_id_and_a_box_to_find_it_by() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    let groups = vec![
        WmoGroup::parse(&with_group_id(build_group(group_flags::EXTERIOR, 0), 3558)).unwrap(),
        WmoGroup::parse(&with_group_id(build_group(group_flags::INDOOR, 1), 3559)).unwrap(),
    ];
    assert_eq!(groups[0].group_id, 3558);

    let model = WmoModel::assemble("Building.wmo", &root, &groups);
    assert_eq!(model.wmo_id, root.wmo_id, "the first key is the root's own");
    let areas = model.area_bounds();
    assert_eq!(areas.len(), 2, "both groups, where only one is a room");
    assert_eq!(model.interior_bounds().len(), 1);
    assert_eq!(areas[0].1, 3558);
    assert_eq!(areas[1].1, 3559);
    assert_eq!(areas[0].0, [[0.0, 0.0, 0.0], [2.0, 2.0, 4.0]]);
}

/// The room's light for an entity is the **mean** of what the file baked for
/// the things that do not move, and it is the named doodad set's — the same
/// slice the furniture itself comes from, so a building standing furnished
/// in one place and empty in another lights its visitors differently too.
#[test]
fn a_rooms_light_is_the_mean_of_its_furnitures_own() {
    let mut root = WmoRoot::parse(&build_root()).expect("valid root");
    let spawn = |bgr: u32| WmoDoodad {
        path: "lamp.m2".into(),
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        // BGRA in the file; `light()` reads R from the high byte of the
        // low three.
        colour: bgr,
        matrix: doodad_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0),
        exterior_lit: false,
    };
    // Two spawns in set 0 at 0x40 and 0x80 red, one in set 1 at 0xFF.
    root.doodads = vec![spawn(0x0040_0000), spawn(0x0080_0000), spawn(0x00FF_0000)];
    root.doodad_sets = vec![
        WmoDoodadSet { name: "a".into(), start: 0, count: 2 },
        WmoDoodadSet { name: "b".into(), start: 2, count: 1 },
    ];
    let model = WmoModel::assemble("Building.wmo", &root, &[]);

    let a = model.room_light(0);
    // (64 + 128) / 2 / 255.
    assert!((a[0] - 96.0 / 255.0).abs() < 1e-4, "the mean of 0x40 and 0x80: {a:?}");
    let b = model.room_light(1);
    assert!((b[0] - 1.0).abs() < 1e-3, "set 1 is its own slice: {b:?}");

    // **And a building whose spawns name no light falls back to `MOHD`'s
    // ambient**, which is the honest answer and is dark — see the note on
    // `room_light`. The fixture's ambient is 0x10/0x20/0x30.
    root.doodads = vec![spawn(0)];
    root.doodad_sets = vec![WmoDoodadSet { name: "a".into(), start: 0, count: 1 }];
    let model = WmoModel::assemble("Building.wmo", &root, &[]);
    assert_eq!(model.room_light(0), root.ambient);
    assert!(root.ambient[0] < 0.1, "and it is dark: {:?}", root.ambient);
}

/// A bare group with the given flags and doodad references — no geometry,
/// which the ref walk must not care about: whether a room's batches resolve
/// has no bearing on where its furniture stands.
fn group_with_refs(flags: u32, refs: Vec<u16>) -> WmoGroup {
    WmoGroup {
        name: String::new(),
        flags,
        bounds: [[0.0; 3]; 2],
        name_offset: 0,
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        triangle_flags: Vec::new(),
        batches: Vec::new(),
        doodad_refs: refs,
        colours: Vec::new(),
        interior_start: 0,
        liquid_type: 0,
        group_id: 0,
        liquid: None,
    }
}

/// **Which spawns the sun lights is `MODR` crossed with the group flags.**
/// A spawn referenced only by an `EXTERIOR` group is street furniture; one
/// any `INDOOR` group claims keeps its baked colour whichever order the
/// groups are read in; one no group references keeps it too. This is the
/// rule that stops Stormwind's lamp-posts drawing at dusk at noon.
#[test]
fn a_spawn_referenced_only_by_an_exterior_group_takes_the_sun() {
    let mut root = WmoRoot::parse(&build_root()).expect("valid root");
    let spawn = |bgr: u32| WmoDoodad {
        path: "lamp.m2".into(),
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        colour: bgr,
        matrix: doodad_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0),
        exterior_lit: false,
    };
    root.doodads = vec![spawn(0x0040_0000); 4];
    root.doodad_sets = vec![WmoDoodadSet { name: "a".into(), start: 0, count: 4 }];

    // Spawn 0: street only. Spawn 1: room only. Spawn 2: a doorway rug both
    // reference — the room's answer wins, in either read order. Spawn 3:
    // referenced by nobody. The reference past the end must not panic.
    let street = group_with_refs(group_flags::EXTERIOR, vec![0, 2, 9]);
    let room = group_with_refs(group_flags::INDOOR, vec![1, 2]);

    for groups in [[&street, &room], [&room, &street]] {
        let groups: Vec<WmoGroup> = groups.into_iter().cloned().collect();
        let model = WmoModel::assemble("Building.wmo", &root, &groups);
        let lit: Vec<bool> = model.doodads.iter().map(|d| d.exterior_lit).collect();
        assert_eq!(lit, [true, false, false, false], "order-independent");

        // And the street lamp does not tint the room light handed to an
        // entity standing indoors — the mean is over the roofed spawns.
        assert_eq!(model.room_light(0)[0], 0x40 as f32 / 255.0);
    }
}

/// **…and "a room" is the `0x48` pair being clear, not the `INDOOR` bit.**
///
/// The two answer different questions — `INDOOR` is the portal system's and
/// `0x48` is the light's — and a group carrying *both* is most of a city:
/// Stormwind's 115 building shells (`BH01`, `DW02`, `ClockTower`, and they name
/// themselves in `MOGN`) are exactly that shape, and they carry no `MOCV`,
/// which is the file saying the sun does their walls.
///
/// Asked as `INDOOR`, every barrel, tree, planter and torch those shells list
/// was drawn at a flat bake in the open air — dark at noon, bright at midnight,
/// because a bake does not move with the sun. **1,711 of Stormwind's 6,158
/// `MODD` spawns**, against the 42 the old rule found (`vale wmos`, which
/// names them: 517 barrels, 159 mushrooms, 149 planters, 96 trees).
#[test]
fn a_group_that_is_indoor_and_exterior_lit_lights_its_spawns_by_the_sun() {
    let mut root = WmoRoot::parse(&build_root()).expect("valid root");
    root.doodads = vec![WmoDoodad {
        path: "barrel.m2".into(),
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        colour: 0x0040_0000,
        matrix: doodad_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0),
        exterior_lit: false,
    }; 2];
    root.doodad_sets = vec![WmoDoodadSet { name: "a".into(), start: 0, count: 2 }];

    // A building shell: INDOOR for the portal graph, 0x40 for the light.
    let shell = group_with_refs(
        group_flags::INDOOR | group_flags::EXTERIOR_LIT,
        vec![0, 1],
    );
    // …and a real room, which claims the second spawn back.
    let room = group_with_refs(group_flags::INDOOR, vec![1]);

    for groups in [[&shell, &room], [&room, &shell]] {
        let groups: Vec<WmoGroup> = groups.into_iter().cloned().collect();
        let model = WmoModel::assemble("Building.wmo", &root, &groups);
        let lit: Vec<bool> = model.doodads.iter().map(|d| d.exterior_lit).collect();
        assert_eq!(
            lit,
            [true, false],
            "the shell's own spawn takes the sun; a real room still wins"
        );
    }

    // **And the same bit decides the *entity* test**, which is the other half
    // of the report: a shell's box is the outside of a house, so a character
    // walking past one was being handed the room light. Built through
    // `build_group` rather than the ref helper above, because a group with no
    // batches is never assembled and the assertion would pass vacuously.
    let drawn = |flags: u32| {
        let group = WmoGroup::parse(&build_group(flags, 0)).expect("valid group");
        WmoModel::assemble("Building.wmo", &root, &[group])
            .interior_bounds()
            .len()
    };
    assert_eq!(drawn(group_flags::INDOOR), 1, "a room is somewhere you are in");
    assert_eq!(
        drawn(group_flags::INDOOR | group_flags::EXTERIOR_LIT),
        0,
        "a building shell is not, whatever its INDOOR bit says"
    );
    assert_eq!(drawn(group_flags::EXTERIOR), 0);
}

/// A pathless `MODD` row stays in the list — `MODS` and `MODR` both index
/// it by position, so filtering it out would shift every spawn after it
/// onto its neighbour's slot — and is skipped only where it would be drawn.
#[test]
fn a_pathless_spawn_keeps_its_slot_and_is_not_drawable() {
    let named = WmoDoodad {
        path: "lamp.m2".into(),
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        colour: 0,
        matrix: doodad_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0),
        exterior_lit: false,
    };
    let mut pathless = named.clone();
    pathless.path = String::new();
    assert!(named.drawable());
    assert!(!pathless.drawable());
}

#[test]
fn the_colour_fixup_splits_at_the_transparency_batch_boundary() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");

    // Three white vertices at full alpha, with the boundary after the first:
    // MOBA's vertex_end is 0, so interior_start is 1.
    let white = [[0xFF, 0xFF, 0xFF, 0xFF]; 3];
    let group = WmoGroup::parse(&add_mocv(build_group(0, 1), &white, 1, 0)).unwrap();
    assert_eq!(group.interior_start, 1);

    let shaded = group.shaded_colours(&root);
    // Vertex 0 is in the transition batch. It loses the ambient — which the
    // shader adds back — and **nothing else**: its alpha is the weight
    // [`BatchLight::Blend`] fades it toward the daylight by, so folding
    // `(1 - a)` into the colour the way Noggit does takes a vertex the file
    // marks *fully outdoors* to black. That fold is what put a step of
    // darkness in the mouth of every building in the game.
    assert!(shaded[0][0] > 0.9, "{:?}", shaded[0]);
    assert_eq!(shaded[0][3], 1.0, "the fade weight survives the fixup");
    // Vertices 1 and 2 take the interior branch, which subtracts the same
    // ambient and adds the gain — white minus a dark ambient is still nearly
    // white — and keeps its alpha as the emissive mask
    // `MapObjOverbright.bls` multiplies the batch by (`tex * MOCV * (1 + 4a)`).
    assert!(shaded[1][0] > 0.9, "{:?}", shaded[1]);
    assert!(shaded[1][2] < shaded[1][0], "blue loses more to the ambient");
    assert_eq!(shaded[2], shaded[1], "same branch, same result");
    assert_eq!(shaded[1][3], 1.0, "an interior vertex keeps its mask");
    // The two branches are still not the same arithmetic, even though both
    // survive here: the interior one carries the `c * a / 64` gain and the
    // transition one must not, because that alpha is not a brightness. Both
    // clamp at 1.0, so the difference shows on the channel the ambient has
    // taken furthest from white.
    assert!(
        shaded[1][2] > shaded[0][2],
        "the interior gain is interior-only: {:?} vs {:?}",
        shaded[1],
        shaded[0]
    );
}

/// **Every batch of a building is one of three lights, and the group alone
/// cannot say which.** `MOGP` `+0x28`/`+0x2A` partition the `MOBA` list into
/// transition, interior and exterior runs, and inside an indoor group those are
/// the seam, the room and the daylight respectively. Before this was read, all
/// three took the room — so the mouth of a building was drawn at the same flat
/// bake as its cellar, which is the step of darkness the reports named.
///
/// Measured against real files by `vale wmos`, which crosses the sections
/// with the group flags: over five Elwynn tiles **not one** transition or
/// interior batch lands in a group the `0x48` mask calls outdoor.
#[test]
fn each_batch_takes_the_light_its_moba_section_names() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");

    // One indoor group, three batches over the same triangle, declared
    // 1 transition + 1 interior + (the rest) exterior.
    let mut raw = build_group(group_flags::INDOOR, 0);
    let moba = raw.windows(4).position(|w| w == b"ABOM").expect("MOBA") + 8;
    let one = raw[moba..moba + BATCH_SIZE].to_vec();
    raw.splice(moba + BATCH_SIZE..moba + BATCH_SIZE, one.repeat(2));
    let size = raw.windows(4).position(|w| w == b"ABOM").expect("MOBA") + 4;
    raw[size..size + 4].copy_from_slice(&((BATCH_SIZE * 3) as u32).to_le_bytes());
    let mogp = raw.windows(4).position(|w| w == b"PGOM").expect("MOGP") + 8;
    raw[mogp + 0x28..mogp + 0x2A].copy_from_slice(&1u16.to_le_bytes());
    raw[mogp + 0x2A..mogp + 0x2C].copy_from_slice(&1u16.to_le_bytes());
    let raw = add_mocv(raw, &[[0x80, 0x80, 0x80, 0x40]; 3], 1, 0);

    let group = WmoGroup::parse(&raw).expect("valid group");
    assert_eq!(group.batches.len(), 3);
    let sections: Vec<BatchSection> = group.batches.iter().map(|b| b.section).collect();
    assert_eq!(
        sections,
        [BatchSection::Trans, BatchSection::Int, BatchSection::Ext]
    );

    let model = WmoModel::assemble("t.wmo", &root, std::slice::from_ref(&group));
    let lights: Vec<BatchLight> = model.draws.iter().map(|d| d.light).collect();
    assert_eq!(
        lights,
        [BatchLight::Blend, BatchLight::Bake, BatchLight::Sun],
        "the seam fades, the room bakes, and the file's own exterior run takes \
         the sun even though the group around it is a room"
    );
}

/// …and the *group* half of that answer is the `0x48` pair, not `0x08` alone.
///
/// The client tests `flags & 0x48` everywhere it asks. A group carrying only
/// `0x40` is a
/// street or a courtyard; read as indoor it is drawn at a bake authored to be
/// multiplied by daylight, which is a city in permanent dusk. Stormwind's own
/// tile carries **115** of them.
#[test]
fn the_second_exterior_bit_is_exterior_too() {
    let lit = WmoGroup::parse(&build_group(group_flags::EXTERIOR_LIT, 0)).unwrap();
    assert!(lit.is_exterior(), "0x40 alone is outdoor geometry");
    let plain = WmoGroup::parse(&build_group(group_flags::EXTERIOR, 0)).unwrap();
    assert!(plain.is_exterior());
    let neither = WmoGroup::parse(&build_group(group_flags::INDOOR, 0)).unwrap();
    assert!(!neither.is_exterior());
}

/// The emissive mask survives at its authored value, not merely at the
/// endpoints: a hearth's half-bright rim is alpha 0x80 and has to come out
/// as 0x80, because the ×(1 + 4a) gain is read per vertex in the shader.
#[test]
fn an_interior_vertexs_emissive_alpha_is_the_authored_byte() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    let group = WmoGroup::parse(&add_mocv(
        build_group(group_flags::INDOOR, 1),
        &[[0x40, 0x60, 0x80, 0x80]; 3],
        0,
        2,
    ))
    .unwrap();
    let shaded = group.shaded_colours(&root);
    assert!(shaded.iter().all(|c| c[3] == 0x80 as f32 / 255.0), "{shaded:?}");
}

#[test]
fn the_boundary_is_resolved_before_an_impossible_batch_is_dropped() {
    // The batch the boundary is read from can be one the retain removes. If
    // that happened first, interior_start would fall back to 0 and every
    // vertex would silently take the wrong branch.
    let mut raw = add_mocv(build_group(0, 1), &[[0x80, 0x80, 0x80, 0xFF]; 3], 1, 1);
    let at = raw.windows(4).position(|w| w == b"ABOM").expect("MOBA") + 8;
    raw[at + 0x10..at + 0x12].copy_from_slice(&300u16.to_le_bytes());

    let group = WmoGroup::parse(&raw).expect("geometry survives");
    assert!(group.batches.is_empty(), "the impossible batch is dropped");
    assert_eq!(group.interior_start, 2, "but the boundary it stated is not");
}

#[test]
fn the_do_not_fix_flag_leaves_the_colours_alone_and_only_forces_alpha() {
    let mut raw = build_root();
    let at = raw.windows(4).position(|w| w == b"DHOM").expect("MOHD") + 8;
    raw[at + 0x3C..at + 0x3E]
        .copy_from_slice(&(root_flags::DO_NOT_FIX_VERTEX_COLOUR_ALPHA as u16).to_le_bytes());
    let root = WmoRoot::parse(&raw).expect("valid root");
    assert_eq!(root.flags, root_flags::DO_NOT_FIX_VERTEX_COLOUR_ALPHA);

    let group =
        WmoGroup::parse(&add_mocv(build_group(0, 1), &[[0x40, 0x60, 0x80, 0x20]; 3], 0, 2))
            .unwrap();
    let shaded = group.shaded_colours(&root);
    // RGB is untouched — no ambient subtracted, no alpha gain applied.
    assert_eq!(shaded[0][0], 0x80 as f32 / 255.0);
    assert_eq!(shaded[0][2], 0x40 as f32 / 255.0);
    // ...and the alpha says "not lit from outside", the group not being one.
    assert_eq!(shaded[0][3], 0.0);
}

#[test]
fn every_vertex_gets_a_colour_even_when_its_group_has_no_mocv() {
    // The colour buffer is parallel to the positions across the whole
    // concatenation. A group that contributed nothing would shift every
    // later group's colours by its own vertex count — which lights the
    // wrong room and reads as a rendering bug, not a parsing one.
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    let groups = vec![
        WmoGroup::parse(&build_group(group_flags::EXTERIOR, 0)).unwrap(),
        WmoGroup::parse(&add_mocv(
            build_group(group_flags::INDOOR, 1),
            &[[0x00, 0x00, 0xFF, 0x00]; 3],
            0,
            2,
        ))
        .unwrap(),
    ];
    let model = WmoModel::assemble("Building.wmo", &root, &groups);

    assert_eq!(model.colours.len(), model.positions.len());
    // The group without MOCV contributes transparent black: the identity
    // for a term the shader adds to the ambient — and for the emissive
    // mask in the alpha, where 255 would be a ×5 boost on an unlit pad.
    assert_eq!(&model.colours[..3], &[[0, 0, 0, 0]; 3]);
    assert!(model.colours[3][0] > 200, "{:?}", model.colours[3]);

    // Only the group that has colours and is not exterior is vertex-lit.
    assert!(!model.groups[0].vertex_lit && model.groups[1].vertex_lit);
    // `build_group` writes no batch-section counts, so every batch reads
    // exterior — which inside an indoor group is `Sun`, and is the reason the
    // second group's draw is checked for `Bake` through its section below
    // rather than here. What this pins is the *group's* half of the answer.
    assert_eq!(model.draws[0].light, BatchLight::Sun);
    assert_eq!(model.ambient[0], 0x10 as f32 / 255.0);
}

#[test]
fn antiportal_groups_are_not_drawn() {
    let root = WmoRoot::parse(&build_root()).expect("valid root");
    let groups = vec![WmoGroup::parse(&build_group(group_flags::ANTIPORTAL, 0)).unwrap()];
    let model = WmoModel::assemble("Building.wmo", &root, &groups);
    assert!(model.groups.is_empty() && model.draws.is_empty());
    assert_eq!(model.bounds, [[0.0; 3]; 2]);
}

#[test]
fn group_paths_insert_the_index_before_the_extension() {
    assert_eq!(
        group_path("World\\wmo\\Azeroth\\Buildings\\Human_Farm\\Farm.wmo", 0),
        "World\\wmo\\Azeroth\\Buildings\\Human_Farm\\Farm_000.wmo"
    );
    assert_eq!(group_path("Farm.WMO", 12), "Farm_012.wmo");
    assert_eq!(group_path("Farm", 3), "Farm_003.wmo");
}

#[test]
fn a_non_wmo_and_a_later_version_are_both_errors() {
    assert!(WmoRoot::parse(b"not a wmo at all").is_err());

    let mut buf = build_root();
    // Version 17 is 1.12; TBC's WMOs change MOMT and must not be read as if
    // they had not.
    let at = buf.windows(4).position(|w| w == b"REVM").unwrap() + 8;
    buf[at..at + 4].copy_from_slice(&21u32.to_le_bytes());
    assert!(WmoRoot::parse(&buf).is_err());
}

#[test]
fn a_group_with_no_vertices_is_an_error_not_an_empty_model() {
    let mut buf = encode(b"MVER", &17u32.to_le_bytes());
    buf.extend(encode(b"MOGP", &vec![0u8; MOGP_HEADER_SIZE]));
    assert!(WmoGroup::parse(&buf).is_err());
    // And a file with no MOGP at all.
    assert!(WmoGroup::parse(&encode(b"MVER", &17u32.to_le_bytes())).is_err());
}

/// Apply a column-major 4x4 to a point, rounding away the noise that makes
/// exact quarter turns unreadable.
fn apply(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for row in 0..3 {
        out[row] = m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
        if out[row].abs() < 1e-6 {
            out[row] = 0.0;
        }
    }
    out
}

/// **A root claiming more groups than any building has is refused.**
///
/// `MOHD`'s group count is the one field of a WMO header a caller *acts* on:
/// `load` reads that many archive files, one per index. An unbounded one is an
/// unbounded run of lookups and, before that, an allocation of whatever the
/// damaged bytes said — which is the four-gigabyte spike and the stall that
/// were reported from the window.
///
/// The number is measured rather than chosen: over all 816 `.wmo` roots of a
/// 1.12 install the largest is 306, `stormwind.wmo`.
#[test]
fn a_root_claiming_impossible_groups_is_refused() {
    let mut raw = build_root();
    // The `MOHD` payload begins eight bytes into the chunk, and `nGroups` is at
    // 0x04 of it. `build_root` puts MVER first, so find the chunk rather than
    // counting: its magic is stored reversed.
    let at = raw
        .windows(4)
        .position(|w| w == b"DHOM")
        .expect("the root has a MOHD");
    let groups = at + 8 + 0x04;
    raw[groups..groups + 4].copy_from_slice(&4_067_989_298u32.to_le_bytes());
    assert!(
        WmoRoot::parse(&raw).is_err(),
        "a root claiming four billion groups is a damaged header, not a building"
    );

    // ...and the largest thing the game actually ships still opens.
    raw[groups..groups + 4].copy_from_slice(&306u32.to_le_bytes());
    let root = WmoRoot::parse(&raw).expect("stormwind's own group count is fine");
    assert_eq!(root.group_count, 306);
}

/// **The server's outdoors rule, case by case** — `TerrainInfo::IsOutdoors`.
///
/// The case that matters is the second: a street group carries `OUTDOOR` and a
/// building's rooms do not, so Stormwind reads as open sky everywhere but
/// indoors. Before this rule the client asked whether the point was inside
/// *any* group's box, which is true of every street in the city.
#[test]
fn outdoors_is_the_floor_group_bit_unless_the_ground_covers_it() {
    let street = group_flags::OUTDOOR | group_flags::EXTERIOR;
    let bank = group_flags::INDOOR;
    // No building under the probe at all: open country.
    assert!(outdoors_at(None, Some(40.0), 51.0));
    // Standing on a street in a city WMO.
    assert!(outdoors_at(Some((50.0, street)), Some(10.0), 51.0));
    // Standing in a room: the refusal a mount gets in the bank.
    assert!(!outdoors_at(Some((50.0, bank)), Some(10.0), 51.0));
    // A room whose floor the terrain lies over, inside the two-yard allowance
    // above the probe: the building does not count, whatever its group says.
    assert!(outdoors_at(Some((45.0, bank)), Some(52.0), 51.0));
    // …and terrain more than two yards over the probe is a hill above a
    // cellar, not ground the character is on: the room still decides.
    assert!(!outdoors_at(Some((45.0, bank)), Some(60.0), 51.0));
    // Terrain below the floor is the ordinary case and changes nothing.
    assert!(!outdoors_at(Some((50.0, bank)), Some(49.0), 51.0));
    // No terrain under the point (a hole, or a map with no ground): the room
    // decides alone.
    assert!(!outdoors_at(Some((50.0, bank)), None, 51.0));
}
