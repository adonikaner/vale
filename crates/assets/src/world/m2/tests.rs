use super::*;

/// A minimal but *valid* vanilla M2: three vertices, one triangle, one
/// texture, one submesh, one batch.
///
/// Built rather than loaded from `Data/` so the test runs without the
/// 5 GB of archives; the real files are exercised by `vale doodads`.
fn build_model() -> Vec<u8> {
    let mut buf = vec![0u8; 0x200];
    buf[0..4].copy_from_slice(M2_MAGIC);
    buf[4..8].copy_from_slice(&256u32.to_le_bytes());

    let put_u32 = |buf: &mut Vec<u8>, at: usize, v: u32| {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    };
    let put_u16 = |buf: &mut Vec<u8>, at: usize, v: u16| {
        buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
    };
    let put_f32 = |buf: &mut Vec<u8>, at: usize, v: f32| {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    };
    let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: u32| {
        buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
        buf[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    };

    // name
    let name_at = 0x200;
    buf.extend_from_slice(b"Test.m2\0");
    put_arr(&mut buf, offsets::NAME, 8, name_at as u32);

    // three vertices at (i, 0, 0)
    let verts_at = buf.len();
    buf.resize(verts_at + 3 * VERTEX_STRIDE, 0);
    for i in 0..3 {
        let o = verts_at + i * VERTEX_STRIDE;
        put_f32(&mut buf, o, i as f32);
        put_f32(&mut buf, o + 28, 1.0); // normal +Z
        put_f32(&mut buf, o + 32, i as f32 / 2.0); // u
    }
    put_arr(&mut buf, offsets::VERTICES, 3, verts_at as u32);
    put_f32(&mut buf, offsets::BOUNDING_RADIUS, 2.5);

    // texture table: one type-0 texture with a path
    let tex_name_at = buf.len();
    buf.extend_from_slice(b"World\\Test\\bark.blp\0");
    let tex_at = buf.len();
    buf.resize(tex_at + 16, 0);
    put_u32(&mut buf, tex_at, 0);
    put_u32(&mut buf, tex_at + 8, 20);
    put_u32(&mut buf, tex_at + 12, tex_name_at as u32);
    put_arr(&mut buf, offsets::TEXTURES, 1, tex_at as u32);

    // materials: one, blend 1 (alpha key), two-sided
    let mat_at = buf.len();
    buf.resize(mat_at + 4, 0);
    put_u16(&mut buf, mat_at, 0x04);
    put_u16(&mut buf, mat_at + 2, 1);
    put_arr(&mut buf, offsets::RENDER_FLAGS, 1, mat_at as u32);

    // texture lookup: slot 0 -> texture 0
    let lookup_at = buf.len();
    buf.resize(lookup_at + 2, 0);
    put_arr(&mut buf, offsets::TEXTURE_LOOKUP, 1, lookup_at as u32);

    // view 0: vertex lookup [2, 1, 0], triangles [0, 1, 2] — so the
    // indirection is observable, the resolved indices come out reversed.
    let vlookup_at = buf.len();
    for v in [2u16, 1, 0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let tris_at = buf.len();
    for v in [0u16, 1, 2] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let sub_at = buf.len();
    buf.resize(sub_at + SUBMESH_SIZE, 0);
    put_u16(&mut buf, sub_at, 401); // geoset
    put_u16(&mut buf, sub_at + 8, 0); // indexStart
    put_u16(&mut buf, sub_at + 10, 3); // indexCount
    let batch_at = buf.len();
    buf.resize(batch_at + BATCH_SIZE, 0);
    put_u16(&mut buf, batch_at + 4, 0); // skinSectionIndex
    put_u16(&mut buf, batch_at + 0x0A, 0); // materialIndex
    put_u16(&mut buf, batch_at + 0x10, 0); // textureComboIndex

    let view_at = buf.len();
    buf.resize(view_at + VIEW_SIZE, 0);
    put_arr(&mut buf, view_at, 3, vlookup_at as u32);
    put_arr(&mut buf, view_at + 8, 3, tris_at as u32);
    put_arr(&mut buf, view_at + 24, 1, sub_at as u32);
    put_arr(&mut buf, view_at + 32, 1, batch_at as u32);
    put_arr(&mut buf, offsets::VIEWS, 1, view_at as u32);

    buf
}

/// [`build_model`] with a collision hull bolted on: four vertices and two
/// triangles, so `nBoundingTriangles` is **6** and not 2.
fn build_solid_model() -> Vec<u8> {
    let mut buf = build_model();
    let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: u32| {
        buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
        buf[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    };

    let verts_at = buf.len();
    for v in [
        [0.0f32, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [4.0, 4.0, 0.0],
        [0.0, 4.0, 0.0],
    ] {
        for c in v {
            buf.extend_from_slice(&c.to_le_bytes());
        }
    }
    let tris_at = buf.len();
    for i in [0u16, 1, 2, 0, 2, 3] {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    put_arr(&mut buf, offsets::COLLISION_VERTICES, 4, verts_at as u32);
    put_arr(&mut buf, offsets::COLLISION_INDICES, 6, tris_at as u32);
    buf
}

/// [`build_model`] with the colour and transparency blocks bolted on: one
/// `M2Color` whose RGB goes white -> red and whose alpha goes 1 -> 0 over a
/// second, and one texture weight that stays at half. The batch names both.
///
/// The two are deliberately different shapes: the alpha and the weight are
/// separate answers that **multiply**, so a reader that takes one and stops
/// comes out at 0.5 or at 0.0 where the file says neither.
fn build_fading_model() -> Vec<u8> {
    let mut buf = build_model();
    let put_u16 = |buf: &mut Vec<u8>, at: usize, v: u16| {
        buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
    };
    let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: u32| {
        buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
        buf[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    };
    // One track header: interpolation 1 (linear), no global sequence, then
    // the ranges/times/values triple.
    let track = |buf: &mut Vec<u8>, at: usize, times: u32, times_at: u32, values_at: u32| {
        buf[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
        buf[at + 2..at + 4].copy_from_slice(&(-1i16).to_le_bytes());
        put_arr(buf, at + 0x0C, times, times_at);
        put_arr(buf, at + 0x14, times, values_at);
    };

    // Two keys at 0 ms and 1000 ms, shared by every track below.
    let times_at = buf.len();
    for t in [0u32, 1000] {
        buf.extend_from_slice(&t.to_le_bytes());
    }
    // RGB: white -> red.
    let rgb_at = buf.len();
    for v in [1.0f32, 1.0, 1.0, 1.0, 0.0, 0.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    // Alpha, **fixed 16**: opaque -> gone.
    let alpha_at = buf.len();
    for v in [32767i16, 0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    // The texture weight: flat half, so it can only be read as a multiplier.
    let weight_at = buf.len();
    for v in [16384i16, 16384] {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    let color_at = buf.len();
    buf.resize(color_at + COLOR_SIZE, 0);
    track(&mut buf, color_at, 2, times_at as u32, rgb_at as u32);
    track(
        &mut buf,
        color_at + TRACK_SIZE,
        2,
        times_at as u32,
        alpha_at as u32,
    );
    put_arr(&mut buf, offsets::COLORS, 1, color_at as u32);

    let weights_at = buf.len();
    buf.resize(weights_at + TRACK_SIZE, 0);
    track(&mut buf, weights_at, 2, times_at as u32, weight_at as u32);
    put_arr(&mut buf, offsets::TEXTURE_WEIGHTS, 1, weights_at as u32);

    // The lookup: slot 1 -> weight 0, so a reader that skips the
    // indirection lands on slot 1 and finds nothing.
    let wlookup_at = buf.len();
    for slot in [0xFFFFu16, 0] {
        buf.extend_from_slice(&slot.to_le_bytes());
    }
    put_arr(
        &mut buf,
        offsets::TEXTURE_WEIGHT_LOOKUP,
        2,
        wlookup_at as u32,
    );

    // Point the one batch at both.
    let view_at = array(&buf, offsets::VIEWS).offset;
    let batch_at = array(&buf, view_at + 32).offset;
    put_u16(&mut buf, batch_at + 0x08, 0); // colorIndex
    put_u16(&mut buf, batch_at + 0x14, 1); // textureWeightComboIndex
    buf
}

/// **An effect fades because two blocks say so, and neither is a float
/// array.** The alpha and the texture weight are `fixed16` — read as `f32`s
/// they decode to denormals and every effect in the game draws at zero
/// opacity, which is indistinguishable from the block not being read.
#[test]
fn a_batch_fades_on_its_colour_and_its_transparency_together() {
    let m2 = M2::parse(&build_fading_model()).expect("parses");
    let tint = m2.batches[0].tint.expect("the batch names both blocks");
    assert_eq!(tint.color, Some(0));
    assert_eq!(
        tint.transparency,
        Some(0),
        "the transparency index went through the lookup, or did not"
    );

    // At the start: white, and the colour block's 1.0 times the weight's 0.5.
    let at = |t| m2.tints.sample(tint, t, 0, 1000, 0);
    let start = at(0);
    assert_eq!([start[0], start[1], start[2]], [1.0, 1.0, 1.0]);
    assert!((start[3] - 0.5).abs() < 1e-3, "alpha {} at t=0", start[3]);

    // Halfway: half red, and half of the half.
    let mid = at(500);
    assert!((mid[1] - 0.5).abs() < 1e-3, "green {} at t=500", mid[1]);
    assert!((mid[3] - 0.25).abs() < 1e-3, "alpha {} at t=500", mid[3]);

    // The end: red and gone.
    let end = at(1000);
    assert_eq!([end[0], end[1], end[2]], [1.0, 0.0, 0.0]);
    assert!(end[3] < 1e-3, "alpha {} at t=1000", end[3]);
}

/// The overwhelming majority of the world names no colour block at all, and
/// it has to come back as `None` rather than as an index into an empty
/// array — a batch that "has a tint" is a batch the renderer gives a
/// per-instance tag and a second material variant to.
#[test]
fn a_batch_that_names_no_tint_has_none() {
    let m2 = M2::parse(&build_model()).expect("parses");
    assert_eq!(m2.batches[0].tint, None);
    assert!(m2.tints.is_empty());

    // …and so does one whose colour slot exists but carries no keys, which
    // is the same answer drawn the same way.
    let mut buf = build_fading_model();
    let at = array(&buf, offsets::COLORS).offset;
    for track in [at, at + TRACK_SIZE] {
        buf[track + 0x0C..track + 0x10].copy_from_slice(&0u32.to_le_bytes());
    }
    let wat = array(&buf, offsets::TEXTURE_WEIGHTS).offset;
    buf[wat + 0x0C..wat + 0x10].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(M2::parse(&buf).expect("parses").batches[0].tint, None);
}

/// **A slot whose keys never leave white and opaque is the same answer as no
/// slot**, and this is the case that was costing the whole character
/// population.
///
/// Every batch of every character model and every item model in the game names
/// transparency track 0 — `HumanMale.m2` reads `1 keys 0..0 ms, 1.000..1.000`
/// across all 56 of its batches, and so do a plate helm and a pauldron. Read as
/// "tinted", the batch's material sets `is_tint`, which the renderer's
/// `material_for` uses to drop `vertex_lit` — so no character and no worn item
/// could ever be lit by the room it was standing in — and the batch then spends
/// the one per-instance word there is on a constant that `animate` rewrites
/// sixty times a second.
///
/// The discrimination is the point and is measured: over the 453 readable
/// spell-effect models this drops **57 of 1,008 batches** and keeps 951,
/// including all six of `ArcaneExplosion_Base`'s.
#[test]
fn a_tint_that_never_leaves_white_is_not_a_tint() {
    let mut buf = build_fading_model();
    // Where each track's values live: `put_arr` wrote (count, offset) at
    // +0x14, so the offset word is at +0x18.
    let values_at = |buf: &[u8], track_at: usize| u32_at(buf, track_at + 0x18) as usize;

    let colours = array(&buf, offsets::COLORS).offset;
    // RGB: white at both keys instead of white -> red.
    let rgb = values_at(&buf, colours);
    for i in 0..6 {
        let at = rgb + i * 4;
        buf[at..at + 4].copy_from_slice(&1.0f32.to_le_bytes());
    }
    // Alpha and the texture weight: fully opaque at both keys.
    let alpha = values_at(&buf, colours + TRACK_SIZE);
    let weight = values_at(&buf, array(&buf, offsets::TEXTURE_WEIGHTS).offset);
    for base in [alpha, weight] {
        for i in 0..2 {
            let at = base + i * 2;
            buf[at..at + 2].copy_from_slice(&32767i16.to_le_bytes());
        }
    }
    let m2 = M2::parse(&buf).expect("parses");
    assert_eq!(
        m2.batches[0].tint, None,
        "a batch whose colour and opacity never move is not tinted",
    );
    // The tracks themselves are still parsed and still there — what changed is
    // only whether the batch claims them.
    assert!(!m2.tints.is_empty(), "the blocks are still read");

    // …and a *constant but not white* tint is kept, because it changes the
    // picture even though it never moves: half opacity is half opacity.
    for i in 0..2 {
        let at = weight + i * 2;
        buf[at..at + 2].copy_from_slice(&16384i16.to_le_bytes());
    }
    let tint = M2::parse(&buf).expect("parses").batches[0]
        .tint
        .expect("a flat 0.5 opacity is still a tint");
    assert_eq!(tint.color, None, "the colour block is all identity");
    assert_eq!(tint.transparency, Some(0));
}

/// The hull is a different block from the drawn geometry, and the count in
/// it is **`u16`s, not triangles** — vmangos' extractor says so in a comment
/// beside the field. Reading it as triangles asks for three times as many
/// indices as exist, which this parser then validates away to an empty hull:
/// a world of trees you walk straight through, with nothing said anywhere.
#[test]
fn the_collision_block_is_read_as_indices_and_not_as_triangles() {
    let m2 = M2::parse(&build_solid_model()).expect("parses");
    assert_eq!(m2.collision.triangle_count(), 2);
    assert_eq!(m2.collision.positions.len(), 4);
    // No `MOPY` and no per-triangle flag: the block exists for collision, so
    // every triangle in it is solid and the two numbers are equal by
    // construction. A WMO's hull is a fraction of its declared geometry.
    assert_eq!(m2.collision.declared_triangles, 2);
    // And it is genuinely a *different* set from what is drawn: three
    // vertices and one triangle.
    assert_eq!(m2.triangle_count(), 1);
    assert_eq!(m2.positions.len(), 3);
}

/// Most of the world has no hull at all, and that is an answer rather than a
/// failure: it is how the game says *walk through me*. vmangos' `Model::open`
/// drops exactly these before they reach a vmap.
#[test]
fn a_model_with_no_hull_parses_and_is_walked_through() {
    let m2 = M2::parse(&build_model()).expect("parses");
    assert!(m2.collision.is_empty());
}

/// A wrong offset in this block does not read as garbage — it reads as
/// plausible coordinates — so an index past the vertices drops the hull
/// whole. Two thirds of a hull is a shape the file never described, and it
/// stops a stride from one direction and not another.
#[test]
fn a_hull_indexing_past_its_own_vertices_is_dropped_whole() {
    let mut buf = build_solid_model();
    let at = array(&buf, offsets::COLLISION_INDICES).offset;
    buf[at..at + 2].copy_from_slice(&9u16.to_le_bytes());
    assert!(M2::parse(&buf).expect("parses").collision.is_empty());

    // …and so is one whose index count is not a whole number of triangles.
    let mut buf = build_solid_model();
    buf[offsets::COLLISION_INDICES..offsets::COLLISION_INDICES + 4]
        .copy_from_slice(&5u32.to_le_bytes());
    assert!(M2::parse(&buf).expect("parses").collision.is_empty());
}

/// [`build_model`] with one particle emitter bolted on: a plane emitter
/// with an animated emission rate, an atlas texture and a colour ramp, so
/// every part of the record layout is observable.
fn build_particle_model() -> Vec<u8> {
    let mut buf = build_model();
    let put_u16 = |buf: &mut Vec<u8>, at: usize, v: u16| {
        buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
    };
    let put_u32 = |buf: &mut Vec<u8>, at: usize, v: u32| {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    };
    let put_f32 = |buf: &mut Vec<u8>, at: usize, v: f32| {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    };
    let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: u32| {
        buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
        buf[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    };

    // One key apiece for the rate and the lifespan.
    let rate_times = buf.len();
    buf.extend_from_slice(&0u32.to_le_bytes());
    let rate_values = buf.len();
    buf.extend_from_slice(&12.0f32.to_le_bytes());
    let life_times = buf.len();
    buf.extend_from_slice(&0u32.to_le_bytes());
    let life_values = buf.len();
    buf.extend_from_slice(&1.5f32.to_le_bytes());
    // The enable gate: a *byte* track, off at 0 ms and on at 500.
    let en_times = buf.len();
    for t in [0u32, 500] {
        buf.extend_from_slice(&t.to_le_bytes());
    }
    let en_values = buf.len();
    buf.extend_from_slice(&[0u8, 1]);

    let at = buf.len();
    buf.resize(at + PARTICLE_SIZE, 0);
    put_u32(&mut buf, at + 0x04, 0x8); // flags: fogged
    put_f32(&mut buf, at + 0x08, 0.25); // position
    put_f32(&mut buf, at + 0x0C, 0.5);
    put_f32(&mut buf, at + 0x10, 1.5);
    put_u16(&mut buf, at + 0x14, 0); // bone
    put_u16(&mut buf, at + 0x16, 0); // texture
    buf[at + 0x28] = 3; // blend: additive — a u8, not a u16
    put_u16(&mut buf, at + 0x2A, 1); // emitter: plane
    put_u16(&mut buf, at + 0x2C, 2); // head and tail both
    put_u16(&mut buf, at + 0x30, 2); // rows
    put_u16(&mut buf, at + 0x32, 2); // columns
    // Track 6 (emission rate) and track 5 (lifespan), 0x1C apart from 0x34.
    let rate_at = at + 0x34 + 6 * TRACK_SIZE;
    put_arr(&mut buf, rate_at + 0x0C, 1, rate_times as u32);
    put_arr(&mut buf, rate_at + 0x14, 1, rate_values as u32);
    let life_at = at + 0x34 + 5 * TRACK_SIZE;
    put_arr(&mut buf, life_at + 0x0C, 1, life_times as u32);
    put_arr(&mut buf, life_at + 0x14, 1, life_values as u32);
    // Over-life: midpoint, a red-to-transparent ramp (file bytes are
    // BGRA), three scales, the two atlas-cell ranges.
    let p = at + 0x14C;
    put_f32(&mut buf, p, 0.5);
    buf[p + 4..p + 8].copy_from_slice(&[0, 0, 255, 255]); // birth: red
    buf[p + 8..p + 12].copy_from_slice(&[0, 128, 255, 128]); // mid: orange
    buf[p + 12..p + 16].copy_from_slice(&[0, 0, 0, 0]); // death: gone
    put_f32(&mut buf, p + 16, 1.0);
    put_f32(&mut buf, p + 20, 2.0);
    put_f32(&mut buf, p + 24, 0.5);
    put_u16(&mut buf, p + 28, 0); // first-half cells 0..1
    put_u16(&mut buf, p + 30, 1);
    put_u16(&mut buf, p + 34, 2); // second-half cells 2..3
    put_u16(&mut buf, p + 36, 3);
    put_f32(&mut buf, at + 0x17C, 0.25); // tail time
    put_f32(&mut buf, at + 0x190, 1.0); // inherit scale
    put_f32(&mut buf, at + 0x194, 0.75); // drag
    put_f32(&mut buf, at + 0x198, 2.0); // spin
    let en_at = at + 0x1DC;
    put_arr(&mut buf, en_at + 0x0C, 2, en_times as u32);
    put_arr(&mut buf, en_at + 0x14, 2, en_values as u32);
    put_f32(&mut buf, at + 0x1B0, -10.0); // tumble max, Z

    put_arr(&mut buf, offsets::PARTICLE_EMITTERS, 1, at as u32);
    buf
}

/// [`build_particle_model`] with a **geometry model** named at +0x18 and the
/// lone `\0` at +0x20 every emitter carries in the empty case.
fn build_model_particle_model() -> Vec<u8> {
    let mut buf = build_particle_model();
    let at = array(&buf, offsets::PARTICLE_EMITTERS).offset;
    let name = b"Spells\\ConeofCold_Geo.mdx\0";
    let name_at = buf.len();
    buf.extend_from_slice(name);
    // The empty array is not absent: it is one terminator, which is why the
    // parse tests the *count* rather than the string.
    let empty_at = buf.len();
    buf.push(0);
    let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: u32| {
        buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
        buf[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    };
    put_arr(&mut buf, at + 0x18, name.len() as u32, name_at as u32);
    put_arr(&mut buf, at + 0x20, 1, empty_at as u32);
    buf
}

/// **An emitter's geometry model is read, and `.mdx` means `.m2`.**
///
/// The field decides whether a particle is a camera-facing quad or a
/// three-dimensional instance of another file, and it was unread for the
/// whole life of the particle system — which is what drew Cone of Cold and
/// Evocation as slabs of `SPELLS\CLOUDS.BLP`, a texture the client never puts
/// on screen for those emitters at all. 55 emitters over 13 files carry one.
///
/// The empty case is the other half: every emitter in the game holds *both*
/// arrays and the unused ones are a single `\0`, so an absence is a count of
/// one rather than a missing array — reading it as a name gives every emitter
/// in the world a one-character model.
#[test]
fn an_emitter_names_the_model_its_particles_are() {
    let m2 = M2::parse(&build_model_particle_model()).expect("parses");
    let e = &m2.particles[0];
    assert_eq!(
        e.geometry_model.as_deref(),
        Some("Spells\\ConeofCold_Geo.m2"),
        ".mdx in the record means .m2 in the archive"
    );
    assert_eq!(e.recursion_model, None, "a lone terminator is an absence");
    assert_eq!(
        e.tumble_max,
        [0.0, 0.0, -10.0],
        "and the tumble band beside it, which is what turns the body"
    );

    // The ordinary emitter is a quad and says so.
    let plain = M2::parse(&build_particle_model()).expect("parses");
    assert_eq!(plain.particles[0].geometry_model, None);
}

#[test]
fn a_particle_emitter_reads_every_field_it_carries() {
    let m2 = M2::parse(&build_particle_model()).expect("parses");
    assert_eq!(m2.particles.len(), 1);
    let e = &m2.particles[0];
    assert_eq!(e.flags, particle_flags::FOGGED);
    assert_eq!(e.position, [0.25, 0.5, 1.5]);
    assert_eq!((e.blend, e.emitter_type, e.head_tail), (3, 1, 2));
    assert_eq!((e.rows, e.columns), (2, 2));
    // The tracks keep their keys rather than a first-sample snapshot.
    assert_eq!(e.emission_rate.as_ref().expect("rate").values, vec![12.0]);
    assert_eq!(e.lifespan.as_ref().expect("life").values, vec![1.5]);
    // The colour ramp is unpacked from the file's BGRA to RGBA.
    assert_eq!(e.colors[0], [255, 0, 0, 255]);
    assert_eq!(e.colors[1], [255, 128, 0, 128]);
    assert_eq!(e.colors[2], [0, 0, 0, 0]);
    assert_eq!(e.scales, [1.0, 2.0, 0.5]);
    assert_eq!(e.mid_point, 0.5);
    assert_eq!(e.cells, [[0, 1], [2, 3]]);
    assert_eq!((e.tail_time, e.inherit_scale), (0.25, 1.0));
    assert_eq!((e.drag, e.spin), (0.75, 2.0));
    // The enable gate is a byte track, promoted to floats like every other.
    let en = e.enabled.as_ref().expect("enable gate");
    assert_eq!(en.times, vec![0, 500]);
    assert_eq!(en.values, vec![0.0, 1.0]);
}

/// One bad record condemns the table, not the model: a blend mode past the
/// table means the offset is wrong, and every other field is then noise.
#[test]
fn a_bad_particle_record_drops_the_table_and_keeps_the_model() {
    let mut buf = build_particle_model();
    let at = array(&buf, offsets::PARTICLE_EMITTERS).offset;
    buf[at + 0x28] = 99;
    let m2 = M2::parse(&buf).expect("geometry is still fine");
    assert!(m2.particles.is_empty(), "the untrustworthy table is dropped");
    assert_eq!(m2.triangle_count(), 1);
}

/// [`build_model`] with two bones, two animations and three tracks bolted
/// on: bone 0 is the root and moves, bone 1 hangs off it and moves as well,
/// so a pose has something to compose. The keys deliberately straddle both
/// animations, because a vanilla track is one flat timeline and the window
/// logic is the part that is easy to get wrong.
fn build_animated_model() -> Vec<u8> {
    let mut buf = build_model();
    let put_u16 = |buf: &mut Vec<u8>, at: usize, v: u16| {
        buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
    };
    let put_u32 = |buf: &mut Vec<u8>, at: usize, v: u32| {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    };
    let put_f32 = |buf: &mut Vec<u8>, at: usize, v: f32| {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    };
    let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: u32| {
        buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
        buf[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    };

    // Skin vertex 0 to bone 0 and vertices 1 and 2 to bone 1, at full weight.
    let verts = array(&buf, offsets::VERTICES).offset;
    for (v, bone) in [(0usize, 0u8), (1, 1), (2, 1)] {
        let o = verts + v * VERTEX_STRIDE;
        buf[o + 12] = 255;
        buf[o + 16] = bone;
    }

    // Two animations sharing one timeline: Stand over [0, 1000] and Run
    // over [1000, 2000].
    let seq_at = buf.len();
    buf.resize(seq_at + 2 * SEQUENCE_SIZE, 0);
    for (i, (id, start, end)) in [(0u16, 0u32, 1000u32), (5, 1000, 2000)].iter().enumerate() {
        let o = seq_at + i * SEQUENCE_SIZE;
        put_u16(&mut buf, o, *id);
        put_u32(&mut buf, o + 4, *start);
        put_u32(&mut buf, o + 8, *end);
        put_f32(&mut buf, o + 0x0C, 2.5); // move speed
    }
    put_arr(&mut buf, offsets::SEQUENCES, 2, seq_at as u32);

    // Bone 0's translation: keys at 0 and 2000 only, so during Run the last
    // key at or before `t` belongs to Stand and must not be lerped from.
    let root_times = buf.len();
    for t in [0u32, 2000] {
        buf.extend_from_slice(&t.to_le_bytes());
    }
    let root_values = buf.len();
    for v in [0.0f32, 0.0, 0.0, 5.0, 0.0, 0.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    // Bone 1's translation: a key per animation boundary.
    let child_times = buf.len();
    for t in [0u32, 1000, 2000] {
        buf.extend_from_slice(&t.to_le_bytes());
    }
    let child_values = buf.len();
    for v in [0.0f32, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 20.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    let bones_at = buf.len();
    buf.resize(bones_at + 2 * BONE_SIZE, 0);
    // bone 0: root, pivot at the origin
    let o = bones_at;
    put_u16(&mut buf, o + 8, u16::MAX); // parent -1
    put_u16(&mut buf, o + 0x0C, 1); // linear interpolation
    put_u16(&mut buf, o + 0x0E, u16::MAX); // no global sequence
    put_arr(&mut buf, o + 0x0C + 0x0C, 2, root_times as u32);
    put_arr(&mut buf, o + 0x0C + 0x14, 2, root_values as u32);
    // bone 1: child of bone 0, pivot one yard along +X
    let o = bones_at + BONE_SIZE;
    put_u16(&mut buf, o + 8, 0); // parent 0
    put_u16(&mut buf, o + 0x0C, 1);
    put_u16(&mut buf, o + 0x0E, u16::MAX);
    put_arr(&mut buf, o + 0x0C + 0x0C, 3, child_times as u32);
    put_arr(&mut buf, o + 0x0C + 0x14, 3, child_values as u32);
    put_f32(&mut buf, o + 0x60, 1.0);
    put_arr(&mut buf, offsets::BONES, 2, bones_at as u32);
    buf
}

/// The attachment table, and the guard that drops it whole.
///
/// A wrong offset here reads as a plausible id at a plausible position and
/// hangs a pauldron off a bone chosen at random — there is no error to
/// notice, so the validation is what stands in for one.
#[test]
fn reads_the_points_other_models_hang_from() {
    let mut buf = build_model();
    let at = buf.len();
    buf.resize(at + 2 * ATTACHMENT_SIZE, 0);
    let put = |buf: &mut Vec<u8>, o: usize, id: u32, bone: u16, p: [f32; 3]| {
        buf[o..o + 4].copy_from_slice(&id.to_le_bytes());
        buf[o + 4..o + 6].copy_from_slice(&bone.to_le_bytes());
        for (i, v) in p.iter().enumerate() {
            buf[o + 8 + i * 4..o + 12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
    };
    // +Y is left in model space, so the right shoulder sits at negative Y.
    put(&mut buf, at, attach::SHOULDER_LEFT, 3, [0.0, 0.4, 1.5]);
    put(&mut buf, at + ATTACHMENT_SIZE, attach::SHOULDER_RIGHT, 4, [0.0, -0.4, 1.5]);
    buf[offsets::ATTACHMENTS..offsets::ATTACHMENTS + 4].copy_from_slice(&2u32.to_le_bytes());
    buf[offsets::ATTACHMENTS + 4..offsets::ATTACHMENTS + 8]
        .copy_from_slice(&(at as u32).to_le_bytes());

    let m2 = M2::parse(&buf).expect("valid model");
    assert_eq!(m2.attachments.len(), 2);
    let right = m2.attachment(attach::SHOULDER_RIGHT).expect("a right shoulder");
    assert_eq!(right.bone, 4);
    assert!(right.position[1] < 0.0, "the right shoulder is on the right");
    assert!(m2.attachment(attach::HELM).is_none(), "a point it does not have");

    // A position no model could hold means the offset is not a table, and
    // the whole thing is dropped rather than half-trusted.
    let mut broken = buf.clone();
    broken[at + 8..at + 12].copy_from_slice(&1.0e6f32.to_le_bytes());
    assert!(M2::parse(&broken).expect("still a model").attachments.is_empty());

    // **…but a model that is a *place* keeps its points.** The guard was 100
    // yards, which is true of every character and creature and false of the
    // character-select backdrops: `UI_Human.m2` is authored where its scene
    // stands on the map, and both of its attachments — one of which is the
    // plinth the character stands on — are at about (-225, -80). It cost that
    // one screen its character, silently, because a dropped optional block is
    // the documented degradation for five other reasons.
    let mut distant = buf;
    distant[at + 8..at + 12].copy_from_slice(&(-225.18f32).to_le_bytes());
    distant[at + 12..at + 16].copy_from_slice(&(-80.84f32).to_le_bytes());
    let scene = M2::parse(&distant).expect("still a model");
    assert_eq!(scene.attachments.len(), 2, "a scene's points are not garbage");
    assert!((scene.attachments[0].position[0] + 225.18).abs() < 1e-3);
}

#[test]
fn reads_bones_sequences_and_the_tracks_that_join_them() {
    let m2 = M2::parse(&build_animated_model()).expect("valid model");
    let sk = m2.skeleton.as_ref().expect("a skeleton");
    assert_eq!(sk.bones.len(), 2);
    assert_eq!(sk.bones[0].parent, -1);
    assert_eq!(sk.bones[1].parent, 0);
    assert_eq!(sk.bones[1].pivot, [1.0, 0.0, 0.0]);
    assert!(sk.bones[0].rotation.is_none() && sk.bones[0].scale.is_none());

    let track = sk.bones[1].translation.as_ref().expect("a translation track");
    assert_eq!(track.times, vec![0, 1000, 2000]);
    assert_eq!(track.dim, 3);
    assert_eq!(track.global_sequence, -1);

    assert_eq!(sk.sequences.len(), 2);
    assert_eq!(sk.sequences[1].id, anim::RUN);
    assert_eq!((sk.sequences[1].start, sk.sequences[1].end), (1000, 2000));
    assert_eq!(sk.sequences[0].move_speed, 2.5);
    assert_eq!(sk.find_sequence(anim::RUN), Some(1));
    assert_eq!(sk.find_sequence(anim::WALK), None);
    // Walk is missing, so "walk, else run, else anything" lands on Run.
    assert_eq!(sk.best_sequence(&[anim::WALK, anim::RUN]), Some(1));

    // The vertex block's other half, skipped until there were bones to
    // point it at.
    assert_eq!(m2.bone_weights[1], [255, 0, 0, 0]);
    assert_eq!(m2.bone_indices[1], [1, 0, 0, 0]);
}

#[test]
fn a_pose_composes_a_child_onto_its_parent() {
    let m2 = M2::parse(&build_animated_model()).expect("valid model");
    let sk = m2.skeleton.as_ref().expect("a skeleton");

    // Half way through Run (t = 1500 on the shared timeline). Bone 0's only
    // keys are at 0 and 2000: the one at 0 belongs to Stand, so the sample
    // holds the *next* key rather than interpolating across the boundary —
    // which would have given 3.75 instead of 5.
    let pose = sk.pose(1, 500, 0, None, PoseLayers::default());
    assert_eq!(pose.len(), 2);
    assert!((pose[0][3] - 5.0).abs() < 1e-5, "root x: {}", pose[0][3]);

    // Bone 1's keys straddle Run exactly, so it does interpolate: half way
    // between 10 and 20 is 15 — and its parent's 5 yards along X carry
    // through, which is the composition being checked.
    assert!((pose[1][3] - 5.0).abs() < 1e-5, "child x: {}", pose[1][3]);
    assert!((pose[1][11] - 15.0).abs() < 1e-5, "child z: {}", pose[1][11]);

    // At the start of Stand nothing has moved yet.
    let rest = sk.pose(0, 0, 0, None, PoseLayers::default());
    assert!(rest[1][3].abs() < 1e-5 && rest[1][11].abs() < 1e-5);
}

/// A skeleton of two bones — a root carrying `flags` and a child of it, both
/// with no tracks at all, so the pose is the identity plus whatever the
/// billboard does to it. One sequence, so `pose` does not bail.
///
/// Built by hand rather than through the byte builder because the subject is
/// the *rule*, and a flags field threaded through 0x6c bytes of file layout
/// would test the parser instead.
fn billboard_skeleton(flags: u32, pivot: [f32; 3]) -> M2Skeleton {
    let bone = |flags: u32, parent: i16, pivot: [f32; 3]| M2Bone {
        key_bone: -1,
        flags,
        parent,
        pivot,
        translation: None,
        rotation: None,
        scale: None,
    };
    M2Skeleton {
        bones: vec![bone(flags, -1, pivot), bone(0, 0, pivot)],
        sequences: vec![M2Sequence {
            id: 0,
            variation: 0,
            start: 0,
            end: 1000,
            move_speed: 0.0,
            flags: 0,
            probability: 0x7fff,
            bounds: [[0.0; 3]; 2],
            radius: 0.0,
        }],
        global_sequences: Vec::new(),
        plan: std::sync::OnceLock::new(),
    }
}

/// **A child stored before its parent still composes onto it.** Nothing in the
/// format promises parent-first order ([`M2Bone::parent`]'s own doc), and the
/// pose plan freezes a visit order at first use — so this pins that the order
/// is derived from the hierarchy and not from the file: bone 0's parent is
/// bone 1, and bone 0 must still inherit bone 1's translation.
#[test]
fn a_child_listed_before_its_parent_still_rides_it() {
    let track = M2Track {
        interpolation: 0,
        global_sequence: -1,
        times: vec![0],
        values: vec![5.0, 0.0, 0.0],
        dim: 3,
    };
    let bone = |parent: i16, translation: Option<M2Track>| M2Bone {
        key_bone: -1,
        flags: 0,
        parent,
        pivot: [0.0; 3],
        translation,
        rotation: None,
        scale: None,
    };
    let sk = M2Skeleton {
        // Bone 0 is the *child*; its parent, bone 1, carries the movement.
        bones: vec![bone(1, None), bone(-1, Some(track))],
        sequences: vec![M2Sequence {
            id: 0,
            variation: 0,
            start: 0,
            end: 1000,
            move_speed: 0.0,
            flags: 0,
            probability: 0x7fff,
            bounds: [[0.0; 3]; 2],
            radius: 0.0,
        }],
        global_sequences: Vec::new(),
        plan: std::sync::OnceLock::new(),
    };
    let pose = sk.pose(0, 0, 0, None, PoseLayers::default());
    assert!((pose[1][3] - 5.0).abs() < 1e-6, "parent x: {}", pose[1][3]);
    assert!(
        (pose[0][3] - 5.0).abs() < 1e-6,
        "child listed first must still inherit its parent's translation: {}",
        pose[0][3]
    );
}

/// The columns of a row-major 3x4 — the images of the local axes, which is what
/// every billboard rule is written in terms of.
fn columns(m: &[f32; 12]) -> [[f32; 3]; 3] {
    [
        [m[0], m[4], m[8]],
        [m[1], m[5], m[9]],
        [m[2], m[6], m[10]],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// **Which rule a bone's flags name, and that a combination names none.**
///
/// The client's switch masks with 0x78 and matches the result
/// *exactly* against four values through a case table, so `0x8 | 0x40` falls to
/// the default and is not billboarded at all. A `flags & 0x8 != 0` test — which
/// is what this client used to do — would spherically billboard it.
#[test]
fn a_bone_carrying_two_billboard_bits_names_no_rule() {
    use crate::world::m2::bone_flags::*;
    assert_eq!(Billboard::of(0), None);
    assert_eq!(Billboard::of(TRANSFORMED), None);
    assert_eq!(Billboard::of(BILLBOARD_SPHERICAL), Some(Billboard::Spherical));
    assert_eq!(Billboard::of(BILLBOARD_LOCK_X), Some(Billboard::LockX));
    assert_eq!(Billboard::of(BILLBOARD_LOCK_Y), Some(Billboard::LockY));
    // The one the archives use, and it comes with `TRANSFORMED` on every bone
    // that carries it — 0x240, which must not stop it resolving.
    assert_eq!(Billboard::of(BILLBOARD_LOCK_Z), Some(Billboard::LockZ));
    assert_eq!(
        Billboard::of(BILLBOARD_LOCK_Z | TRANSFORMED),
        Some(Billboard::LockZ)
    );
    assert_eq!(Billboard::of(BILLBOARD_SPHERICAL | BILLBOARD_LOCK_Z), None);
    assert_eq!(Billboard::of(BILLBOARD_LOCK_X | BILLBOARD_LOCK_Y), None);
}

/// **A spherical billboard is the camera's whole basis**, which is the rule
/// that already worked and is here as the guard on the other three: local X
/// away from the eye, local Y the camera's right, local Z its up.
#[test]
fn a_spherical_billboard_takes_the_camera_basis_whole() {
    let sk = billboard_skeleton(crate::world::m2::bone_flags::BILLBOARD_SPHERICAL, [0.0, 0.0, 0.0]);
    let right = [0.0, 1.0, 0.0];
    let up = [0.0, 0.0, 1.0];
    let pose = sk.pose(0, 0, 0, Some((right, up)), PoseLayers::default());
    let [x, y, z] = columns(&pose[0]);
    // back = right x up = (1, 0, 0), and local X takes its negation.
    assert!((x[0] + 1.0).abs() < 1e-5, "local X: {x:?}");
    assert!((y[1] - 1.0).abs() < 1e-5, "local Y: {y:?}");
    assert!((z[2] - 1.0).abs() < 1e-5, "local Z: {z:?}");
}

/// **A lock-Z billboard keeps the bone's own vertical and turns the other two
/// axes toward the eye** — the client's rule, and the whole of why every
/// worn aura in the game is a flat sheet that faces you.
///
/// The check is what the construction *means* rather than the numbers it
/// happens to produce: the locked axis is untouched, the companion axis is
/// square to both the locked axis and the view direction (which is what puts
/// the sheet's width across the screen), and the basis stays orthonormal.
/// A sheet whose plane is unturned — this client's behaviour until now — fails
/// the second of those from every angle but one.
#[test]
fn a_lock_z_billboard_keeps_its_axle_and_turns_about_it() {
    let sk = billboard_skeleton(
        crate::world::m2::bone_flags::BILLBOARD_LOCK_Z | crate::world::m2::bone_flags::TRANSFORMED,
        [0.0, 0.0, 0.0],
    );
    // A camera looking along +X, then the same camera swung 90° to look along
    // +Y: the sheet must turn with it and the axle must not move.
    for (right, up) in [([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]), ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0])] {
        let pose = sk.pose(0, 0, 0, Some((right, up)), PoseLayers::default());
        let [x, y, z] = columns(&pose[0]);
        let back = [
            right[1] * up[2] - right[2] * up[1],
            right[2] * up[0] - right[0] * up[2],
            right[0] * up[1] - right[1] * up[0],
        ];
        // The axle: untouched, still the model's own +Z.
        assert!(
            (z[0]).abs() < 1e-5 && (z[1]).abs() < 1e-5 && (z[2] - 1.0).abs() < 1e-5,
            "the locked axis moved: {z:?}"
        );
        // The width of the sheet lies across the screen: square to the view.
        assert!(dot(y, back).abs() < 1e-5, "local Y is not in the screen plane: {y:?}");
        assert!(dot(y, z).abs() < 1e-5, "local Y is not square to the axle: {y:?}");
        // …and it is a basis, not a collapse.
        assert!((dot(y, y) - 1.0).abs() < 1e-5 && (dot(x, x) - 1.0).abs() < 1e-5);
        assert!(dot(x, y).abs() < 1e-5 && dot(x, z).abs() < 1e-5);
    }
}

/// **The turn reaches the bones hung under it.**
///
/// The client billboards inside the same walk that composes the hierarchy
/// (between the parent multiply and the next bone), so a child of a
/// billboarded bone is composed against the *turned* parent. This client used
/// to do it as a pass over the finished pose, which leaves every such child
/// behind — and `Spells\ManaShield_State_Base.m2` is exactly that shape: bone 0
/// is the lock-Z root and three bones hang under it.
#[test]
fn a_child_of_a_billboarded_bone_is_carried_by_the_turn() {
    let sk = billboard_skeleton(crate::world::m2::bone_flags::BILLBOARD_LOCK_Z, [0.0, 0.0, 0.0]);
    let pose = sk.pose(0, 0, 0, Some(([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0])), PoseLayers::default());
    let parent = columns(&pose[0]);
    let child = columns(&pose[1]);
    for axis in 0..3 {
        for c in 0..3 {
            assert!(
                (parent[axis][c] - child[axis][c]).abs() < 1e-5,
                "the child kept the untuned basis: {child:?} against {parent:?}"
            );
        }
    }
}

/// **A billboard turns the bone and does not move it**, which is what the pivot
/// term is for: the point the hierarchy put the bone's origin at has to survive
/// the basis being replaced, or every aura in the game jumps as the camera
/// swings.
#[test]
fn a_billboard_leaves_its_pivot_where_the_hierarchy_put_it() {
    let pivot = [0.0, 0.0, 2.0];
    for flags in [
        crate::world::m2::bone_flags::BILLBOARD_SPHERICAL,
        crate::world::m2::bone_flags::BILLBOARD_LOCK_Z,
    ] {
        let sk = billboard_skeleton(flags, pivot);
        for (right, up) in [([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]), ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0])] {
            let pose = sk.pose(0, 0, 0, Some((right, up)), PoseLayers::default());
            let m = &pose[0];
            let at = [
                m[0] * pivot[0] + m[1] * pivot[1] + m[2] * pivot[2] + m[3],
                m[4] * pivot[0] + m[5] * pivot[1] + m[6] * pivot[2] + m[7],
                m[8] * pivot[0] + m[9] * pivot[1] + m[10] * pivot[2] + m[11],
            ];
            for i in 0..3 {
                assert!((at[i] - pivot[i]).abs() < 1e-5, "{flags:#x} moved the pivot: {at:?}");
            }
        }
    }
}

/// **The counter-twist: how far each key bone turns, and that it carries its
/// subtree with it.**
///
/// The split is the client's and the arithmetic closes on its own — half the
/// gap on the spine capped at 45°, the remainder on the head capped at 45°, so
/// a pure 90° strafe puts the hips on the strafe heading, the shoulders half
/// way back and the head exactly on the aim.
#[test]
fn the_body_twist_splits_the_gap_between_the_spine_and_the_head() {
    use crate::world::m2::BodyTwist;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    // A quarter turn is the pure strafe, and it closes exactly: 45 + 45.
    let (spine, head) = BodyTwist { gap: FRAC_PI_2 }.split();
    assert!((spine - FRAC_PI_4).abs() < 1e-6, "{spine}");
    assert!((head - FRAC_PI_4).abs() < 1e-6, "{head}");
    assert!((spine + head - FRAC_PI_2).abs() < 1e-6, "the head must land on the aim");

    // A diagonal's 45° is half and half, both under the cap.
    let (spine, head) = BodyTwist { gap: FRAC_PI_4 }.split();
    assert!((spine - FRAC_PI_4 / 2.0).abs() < 1e-6 && (head - FRAC_PI_4 / 2.0).abs() < 1e-6);

    // Mirrored, and capped: nothing twists past 45° whatever it is handed.
    let (spine, head) = BodyTwist { gap: -3.0 }.split();
    assert!(spine >= -FRAC_PI_4 - 1e-6 && head >= -FRAC_PI_4 - 1e-6, "{spine} {head}");
    assert!(spine < 0.0 && head < 0.0);

    // Nothing to twist, nothing twisted — the case every walking character in
    // the world is in.
    assert_eq!(BodyTwist::default().split(), (0.0, 0.0));
}

/// The twist is applied to the bone's **local**, so everything under it comes
/// along — which is the whole point: turning the spine has to take the arms,
/// the shoulders and the head with it, and turning the head must not move the
/// spine.
#[test]
fn a_twisted_bone_takes_its_children_with_it() {
    use crate::world::m2::{key_bone, BodyTwist};
    use std::f32::consts::FRAC_PI_2;

    let mut m2 = M2::parse(&build_animated_model()).expect("valid model");
    // Bone 0 is the root and bone 1 hangs off it, a yard along X at rest — so
    // naming the root `SpineLow` makes bone 1 the subtree under test.
    let sk = m2.skeleton.as_mut().expect("a skeleton");
    sk.bones[0].key_bone = key_bone::SPINE_LOW;
    let sk = m2.skeleton.as_ref().expect("a skeleton");

    let plain = sk.pose(0, 0, 0, None, PoseLayers::default());
    let twisted = sk.pose(0, 0, 0, None, PoseLayers { twist: Some(BodyTwist { gap: FRAC_PI_2 }), ..PoseLayers::default() });

    // Where the child bone's own pivot ends up — a matrix is not a position,
    // and the point is what the vertices under it ride on.
    let at = |m: &[f32; 12], p: [f32; 3]| {
        [
            m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
            m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        ]
    };
    let pivot = sk.bones[1].pivot;
    let [px, py] = at(&plain[1], pivot);
    let [dx, dy] = at(&twisted[1], pivot);
    assert!(px > 0.5 && py.abs() < 1e-5, "the fixture moved: {px} {py}");
    // 45° on the spine swings the child a yard out along X round toward Y, and
    // its distance from the twist's own pivot is unchanged.
    assert!(dy > 0.1, "the child did not come round with its parent: {dx} {dy}");
    assert!(
        ((dx * dx + dy * dy).sqrt() - (px * px + py * py).sqrt()).abs() < 1e-4,
        "the twist changed the model's proportions"
    );
    assert!(
        (dy.atan2(dx) - std::f32::consts::FRAC_PI_4).abs() < 1e-4,
        "half of a quarter turn is 45°, drew {}",
        dy.atan2(dx)
    );

    // And a model with neither key bone is untouched, which is the client's own
    // gate: a wolf has no SpineLow and nothing to twist.
    let bare = M2::parse(&build_animated_model()).expect("valid model");
    let bare = bare.skeleton.as_ref().expect("a skeleton");
    assert_eq!(
        bare.pose(0, 0, 0, None, PoseLayers { twist: Some(BodyTwist { gap: FRAC_PI_2 }), ..PoseLayers::default() }),
        bare.pose(0, 0, 0, None, PoseLayers::default())
    );
}

/// **The second track: the torso plays one sequence while the legs play
/// another.**
///
/// This is how 1.12 swings, emotes and casts on the move, and the thing being
/// pinned is that the mask is a *subtree* — the bone named `SpineLow` and
/// everything under it takes the overlay, and nothing else moves at all. A
/// client with one track plays the swing over the whole body and stops a
/// running character dead in its stride, which is what this one did.
#[test]
fn a_masked_overlay_takes_the_spine_subtree_and_leaves_the_rest_on_the_base() {
    use crate::world::m2::{key_bone, Overlay};

    let mut m2 = M2::parse(&build_animated_model()).expect("valid model");
    // Bone 1 hangs off bone 0, so naming *it* `SpineLow` makes bone 0 the legs
    // and bone 1 the torso — the shape of a real skeleton, one level down.
    m2.skeleton.as_mut().expect("a skeleton").bones[1].key_bone = key_bone::SPINE_LOW;
    let sk = m2.skeleton.as_ref().expect("a skeleton");

    let run = Overlay { sequence: 1, elapsed_ms: 500 };
    let posed = sk.pose(0, 0, 0, None, PoseLayers { overlay: Some(run), ..PoseLayers::default() });

    // The legs are still at the start of Stand, where nothing has moved: the
    // root did *not* take Run's 5 yards along X.
    assert!(posed[0][3].abs() < 1e-5, "the base track moved: {}", posed[0][3]);
    // …and the torso is half way through Run, 15 up, which is the whole point.
    assert!((posed[1][11] - 15.0).abs() < 1e-5, "the overlay did not play: {}", posed[1][11]);
    assert!(posed[1][3].abs() < 1e-5, "the torso took the base's travel: {}", posed[1][3]);

    // Rooted at bone 0 the mask covers everything, so a masked play there is
    // the same thing as a full-body one — which is the client's own degenerate
    // case and a check that `subtree` reaches children at all.
    let mut whole = M2::parse(&build_animated_model()).expect("valid model");
    whole.skeleton.as_mut().expect("a skeleton").bones[0].key_bone = key_bone::SPINE_LOW;
    let whole = whole.skeleton.as_ref().expect("a skeleton");
    assert_eq!(
        whole.pose(0, 0, 0, None, PoseLayers { overlay: Some(run), ..PoseLayers::default() }),
        whole.pose(1, 500, 0, None, PoseLayers::default())
    );
}

/// **A gait is played at the speed the unit is actually going**, not at the
/// speed it was authored for — and the id is what decides, not the declared
/// number.
///
/// The fixture declares 2.5 y/s on *both* its sequences, which is what makes it
/// worth checking: Stand is id 0 and outside the rate-scaled set, so it plays
/// at 1× however fast the character is moving, where Run at id 5 does not.
#[test]
fn a_locomotion_clip_is_played_at_speed_over_its_authored_speed() {
    let m2 = M2::parse(&build_animated_model()).expect("valid model");
    let sk = m2.skeleton.as_ref().expect("a skeleton");

    // Run (index 1, id 5) authored at 2.5, travelled at 5.0: twice as fast.
    assert!((sk.playback_rate(1, 5.0) - 2.0).abs() < 1e-6);
    // …and the 10% the real case is: 7.6 against `HumanMale`'s 6.9-ish.
    assert!((sk.playback_rate(1, 2.75) - 1.1).abs() < 1e-6);

    // Stand declares the same speed and is not a gait, so it is never scaled.
    assert_eq!(sk.playback_rate(0, 5.0), 1.0);
    // A unit that is not moving, and a sequence that does not exist.
    assert_eq!(sk.playback_rate(1, 0.0), 1.0);
    assert_eq!(sk.playback_rate(99, 5.0), 1.0);
}

/// Two ways an overlay is **no** overlay, and both are ordinary: a model with
/// no `SpineLow` has nothing to mask at — a wolf has a `Head` and no spine —
/// and a sequence the model does not carry is nothing to play.
#[test]
fn an_overlay_with_nowhere_to_go_leaves_the_pose_alone() {
    use crate::world::m2::Overlay;

    let m2 = M2::parse(&build_animated_model()).expect("valid model");
    let sk = m2.skeleton.as_ref().expect("a skeleton");
    let plain = sk.pose(0, 0, 0, None, PoseLayers::default());

    // No key bone anywhere in the fixture.
    assert_eq!(
        sk.pose(0, 0, 0, None, PoseLayers { overlay: Some(Overlay { sequence: 1, elapsed_ms: 500 }), ..PoseLayers::default() }),
        plain
    );

    // …and a sequence index the model does not have, on a model that *could*
    // mask.
    let mut spined = M2::parse(&build_animated_model()).expect("valid model");
    spined.skeleton.as_mut().expect("a skeleton").bones[1].key_bone = crate::world::m2::key_bone::SPINE_LOW;
    let spined = spined.skeleton.as_ref().expect("a skeleton");
    assert_eq!(
        spined.pose(0, 0, 0, None, PoseLayers { overlay: Some(Overlay { sequence: 99, elapsed_ms: 0 }), ..PoseLayers::default() }),
        plain
    );
}

/// **A clock run past the end of its window holds the last frame; it does
/// not come back round to the first.** Looping is [`M2Skeleton::phase`]'s
/// job, i.e. the caller's, because only the caller knows whether it is
/// drawing a gait or a corpse.
///
/// This is the bug that made every corpse in the world stand back up. The
/// renderer asked for `min(elapsed, duration)` precisely so that Death would
/// stop at its end — and this function wrapped it, `duration % duration`
/// being 0, which is frame zero of Death: the creature upright, exactly as
/// it was before it fell over. So the death animation played through and the
/// body then stood up and froze there for the rest of the session, with
/// nothing failing and no count moving anywhere.
///
/// Run's window is 1000..2000 and bone 1's keys are 10 at 1000 and 20 at
/// 2000, so the two readings are 10 and 20 — the first frame against the
/// last, which is the whole difference between standing and lying down.
/// **A batch's texture matrix, end to end** — the block, its lookup, the
/// batch's own field, and the matrix that comes out.
///
/// The failure this pins is not a crash. The vanilla header carries a
/// `texture_flipbooks` array at `0x6C` that every later version drops, so a
/// layout copied forward reads the *flipbooks* as transforms: the record
/// parses, the lookup resolves, and every scroll in the game stands still —
/// which looks exactly like a client that never read the block at all.
#[test]
fn a_batch_scrolls_its_texture_on_its_own_matrix() {
    let mut buf = build_model();
    let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: u32| {
        buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
        buf[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
    };
    let put_u16 = |buf: &mut Vec<u8>, at: usize, v: u16| {
        buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
    };

    // Two keys a second apart, sliding the texture a whole tile along u —
    // `ArcaneExplosion_Base`'s own shape, at a thousandth of its duration.
    let times_at = buf.len();
    for t in [0u32, 1000] {
        buf.extend_from_slice(&t.to_le_bytes());
    }
    let values_at = buf.len();
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    // Two records, so the lookup is observably an indirection: record 0 is
    // empty and record 1 is the scroll.
    let transforms_at = buf.len();
    buf.resize(transforms_at + 2 * TEXTURE_TRANSFORM_SIZE, 0);
    let translation = transforms_at + TEXTURE_TRANSFORM_SIZE;
    put_u16(&mut buf, translation, 1); // linear
    put_u16(&mut buf, translation + 2, u16::MAX); // no global sequence
    put_arr(&mut buf, translation + 0x0C, 2, times_at as u32);
    put_arr(&mut buf, translation + 0x14, 2, values_at as u32);
    put_arr(&mut buf, offsets::TEXTURE_TRANSFORMS, 2, transforms_at as u32);

    // The lookup: slot 0 -> record 1, slot 1 -> the file's own "none".
    let uv_lookup_at = buf.len();
    for slot in [1u16, u16::MAX] {
        buf.extend_from_slice(&slot.to_le_bytes());
    }
    put_arr(
        &mut buf,
        offsets::TEXTURE_TRANSFORM_LOOKUP,
        2,
        uv_lookup_at as u32,
    );

    let batch_at = u32_at(&buf, u32_at(&buf, offsets::VIEWS + 4) as usize + 36) as usize;
    put_u16(&mut buf, batch_at + 0x16, 1); // textureTransformComboIndex
    let unanimated = M2::parse(&buf).expect("valid model");
    assert_eq!(
        unanimated.batches[0].uv, None,
        "a batch whose lookup slot says 0xFFFF was given a matrix anyway"
    );

    put_u16(&mut buf, batch_at + 0x16, 0);
    let m2 = M2::parse(&buf).expect("valid model");

    assert_eq!(
        m2.uv_anims.transforms.len(),
        2,
        "the block did not read — check 0x74 against the vanilla flipbook array at 0x6C"
    );
    assert_eq!(
        m2.batches[0].uv,
        Some(1),
        "the lookup was skipped, so the batch took the empty record"
    );
    let uv = m2.batches[0].uv.expect("a matrix");

    // Half way along, half a tile — and the *rest* of the matrix untouched,
    // which is what says the pivot round trip cancels when nothing rotates.
    let half = m2.uv_anims.matrix(uv, 500, 0, 1000, 0);
    assert!((half[2] - 0.5).abs() < 1e-5, "u offset {half:?}");
    assert_eq!([half[0], half[1], half[3], half[4], half[5]], [1.0, 0.0, 0.0, 1.0, 0.0]);

    // Record 0 has no keys at all, and an empty scale track sampled as a
    // value would collapse the texture to a point rather than leaving it
    // alone — which is why each track is taken only when it has keys.
    assert_eq!(m2.uv_anims.matrix(0, 500, 0, 1000, 0), UV_IDENTITY);
    assert_eq!(m2.uv_anims.matrix(99, 500, 0, 1000, 0), UV_IDENTITY);
    assert!(!m2.uv_anims.is_global(uv), "the scroll named no global sequence");
}

#[test]
fn a_clock_past_the_end_holds_the_last_frame_rather_than_wrapping() {
    let m2 = M2::parse(&build_animated_model()).expect("valid model");
    let sk = m2.skeleton.as_ref().expect("a skeleton");

    let held = sk.pose(1, 1000, 0, None, PoseLayers::default());
    assert!((held[1][11] - 20.0).abs() < 1e-5, "held z: {}", held[1][11]);
    // …and still there much later, rather than playing again.
    let later = sk.pose(1, 60_000, 0, None, PoseLayers::default());
    assert!((later[1][11] - 20.0).abs() < 1e-5, "later z: {}", later[1][11]);

    // Wrapping is available, and is what a gait asks for: one full cycle in
    // is frame zero again.
    assert_eq!(sk.phase(1, 1000), 0);
    assert_eq!(sk.phase(1, 1500), 500);
    assert_eq!(sk.phase(1, 2500), 500);
    let looped = sk.pose(1, sk.phase(1, 1000), 0, None, PoseLayers::default());
    assert!((looped[1][11] - 10.0).abs() < 1e-5, "looped z: {}", looped[1][11]);
    // A sequence that does not exist cannot be wrapped onto anything.
    assert_eq!(sk.phase(99, 1234), 1234);
}

#[test]
fn a_cross_fade_walks_from_one_pose_to_the_other() {
    let m2 = M2::parse(&build_animated_model()).expect("valid model");
    let sk = m2.skeleton.as_ref().expect("a skeleton");

    // The two ends of the fade, posed on their own: the start of Stand,
    // where nothing has moved, and half way through Run.
    let stand = sk.pose(0, 0, 0, None, PoseLayers::default());
    let run = sk.pose(1, 500, 0, None, PoseLayers::default());
    let fade = |weight| {
        sk.pose(
            0,
            0,
            0,
            None,
            PoseLayers {
                blend: Some(Blend { sequence: 1, elapsed_ms: 500, weight }),
                ..PoseLayers::default()
            },
        )
    };

    // Weight 0 and weight 1 are the two ends exactly — a fade that has not
    // started, and one that has not moved off its first frame.
    for e in 0..12 {
        assert!((fade(0.0)[1][e] - stand[1][e]).abs() < 1e-5, "element {e}");
        assert!((fade(1.0)[1][e] - run[1][e]).abs() < 1e-5, "element {e}");
    }

    // And half way is half way. Worth asserting on the *translation*
    // specifically: the child's z is 0 in one pose and 15 in the other, so a
    // fade that silently did nothing would still pass the two ends above.
    assert!((fade(0.5)[1][11] - 7.5).abs() < 1e-5, "{}", fade(0.5)[1][11]);
}

#[test]
fn a_fade_out_of_the_animation_already_playing_is_not_a_fade() {
    // Two of these cases arise every time an entity's animation is chosen:
    // a model with no Run resolves both Run and Stand to the same sequence,
    // and a fade that has run its course is still passed one more frame.
    // Both must leave the pose exactly as the unblended one, or every such
    // entity is quietly posed through a no-op blend for ever.
    let m2 = M2::parse(&build_animated_model()).expect("valid model");
    let sk = m2.skeleton.as_ref().expect("a skeleton");
    let plain = sk.pose(1, 500, 0, None, PoseLayers::default());

    let same = Blend { sequence: 1, elapsed_ms: 0, weight: 1.0 };
    let spent = Blend { sequence: 0, elapsed_ms: 0, weight: 0.0 };
    let missing = Blend { sequence: 99, elapsed_ms: 0, weight: 1.0 };
    for blend in [same, spent, missing] {
        let posed = sk.pose(1, 500, 0, None, PoseLayers { blend: Some(blend), ..PoseLayers::default() });
        for b in 0..2 {
            for e in 0..12 {
                assert!(
                    (posed[b][e] - plain[b][e]).abs() < 1e-6,
                    "{blend:?} changed bone {b} element {e}"
                );
            }
        }
    }
}

#[test]
fn a_wrong_sequence_size_costs_the_skeleton_not_the_geometry() {
    let mut buf = build_animated_model();
    // Vanilla's M2Sequence is 68 bytes and later versions' are not. Reading
    // it at the wrong stride puts the *next* record's fields in this one's
    // start and end, which comes out backwards — the check that catches it.
    let seq = array(&buf, offsets::SEQUENCES).offset;
    buf[seq + 4..seq + 8].copy_from_slice(&9000u32.to_le_bytes());

    let m2 = M2::parse(&buf).expect("geometry is still fine");
    assert_eq!(m2.triangle_count(), 1);
    assert!(m2.skeleton.is_none(), "a half-trusted skeleton is worse than none");
}

#[test]
fn a_vertex_weighted_to_a_bone_that_does_not_exist_drops_the_skeleton() {
    let mut buf = build_animated_model();
    let verts = array(&buf, offsets::VERTICES).offset;
    buf[verts + 16] = 9; // bone 9 of 2
    assert!(M2::parse(&buf).expect("still geometry").skeleton.is_none());
}

#[test]
fn a_quaternion_takes_the_short_way_between_two_keys() {
    // The same rotation written with opposite signs: nlerped naively, the
    // bone spins the long way round through 270 degrees. The hemisphere
    // correction makes the midpoint the identity's neighbour instead.
    let track = M2Track {
        interpolation: 1,
        global_sequence: -1,
        times: vec![0, 1000],
        values: vec![0.0, 0.0, 0.0, 1.0, -0.0, -0.0, -0.0, -1.0],
        dim: 4,
    };
    let q = track.sample(500, 0, 1000, &[], 0);
    assert!((q[3].abs() - 1.0).abs() < 1e-5, "not a unit rotation: {q:?}");
    assert!(q[0].abs() < 1e-5 && q[1].abs() < 1e-5 && q[2].abs() < 1e-5);
}

#[test]
fn a_static_model_has_no_skeleton() {
    let m2 = M2::parse(&build_model()).expect("valid");
    assert!(m2.skeleton.is_none(), "a doodad has nothing to play");
}

#[test]
fn reads_geometry_textures_and_one_batch() {
    let m2 = M2::parse(&build_model()).expect("valid model");
    assert_eq!(m2.name, "Test.m2");
    assert_eq!(m2.version, 256);
    assert_eq!(m2.positions.len(), 3);
    assert_eq!(m2.positions[1], [1.0, 0.0, 0.0]);
    assert_eq!(m2.normals[0], [0.0, 0.0, 1.0]);
    assert_eq!(m2.uvs[2], [1.0, 0.0]);
    assert_eq!(m2.triangle_count(), 1);
    assert_eq!(m2.bounding_radius, 2.5);

    assert_eq!(m2.textures.len(), 1);
    assert_eq!(m2.textures[0].file_name, "World\\Test\\bark.blp");
    assert_eq!(m2.texture_paths().collect::<Vec<_>>(), ["World\\Test\\bark.blp"]);

    assert_eq!(m2.batches.len(), 1);
    let b = &m2.batches[0];
    assert_eq!((b.index_start, b.index_count), (0, 3));
    assert_eq!(b.texture, Some(0));
    assert_eq!(b.blend, 1);
    assert!(b.two_sided && !b.unlit && !b.no_depth_write);
}

#[test]
fn triangles_are_resolved_through_the_views_vertex_lookup() {
    // The view lists triangle indices 0,1,2 against a lookup of [2,1,0],
    // so a renderer must receive 2,1,0. Skipping the indirection draws the
    // wrong vertices, and on a real model it draws a shredded mess.
    let m2 = M2::parse(&build_model()).expect("valid model");
    assert_eq!(m2.indices, vec![2, 1, 0]);
}

#[test]
fn rejects_non_m2_and_post_classic_versions() {
    assert!(M2::parse(b"short").is_err());
    let mut buf = build_model();
    buf[0] = b'X';
    assert!(M2::parse(&buf).is_err());

    let mut buf = build_model();
    buf[4..8].copy_from_slice(&272u32.to_le_bytes());
    assert!(M2::parse(&buf).is_err());
}

#[test]
fn a_bad_material_table_costs_the_batches_not_the_geometry() {
    let mut buf = build_model();
    // Blend modes run 0..7; 99 means the offset is not a material table.
    let mat_arr = array(&buf, offsets::RENDER_FLAGS);
    buf[mat_arr.offset + 2..mat_arr.offset + 4].copy_from_slice(&99u16.to_le_bytes());

    let m2 = M2::parse(&buf).expect("geometry is still fine");
    assert_eq!(m2.triangle_count(), 1);
    assert!(m2.batches.is_empty(), "the untrustworthy table is dropped");
}

#[test]
fn an_emitter_only_model_parses_to_no_geometry() {
    let mut buf = build_model();
    // Particle-only models (spell glows) have no vertices and no view.
    buf[offsets::VERTICES..offsets::VERTICES + 4].copy_from_slice(&0u32.to_le_bytes());
    let m2 = M2::parse(&buf).expect("not an error");
    assert!(m2.positions.is_empty() && m2.indices.is_empty());
}

/// …but it keeps its **skeleton**: an emitter-only model's bones are what
/// carry its emitters — `Spells\ConeofCold_Hand.m2` is eleven emitters on
/// eleven animated bones and not one vertex, and the sweep of those bones *is*
/// the cone. Dropping the skeleton with the mesh anchored every such effect to
/// its attachment root, unrotated and unswept, which drew Cone of Cold as a
/// pile of clouds fired into the ground.
#[test]
fn an_emitter_only_model_keeps_its_skeleton() {
    let mut buf = build_animated_model();
    buf[offsets::VERTICES..offsets::VERTICES + 4].copy_from_slice(&0u32.to_le_bytes());
    let m2 = M2::parse(&buf).expect("not an error");
    assert!(m2.positions.is_empty(), "still no geometry");
    let sk = m2.skeleton.as_ref().expect("the bones survive the missing mesh");
    assert_eq!(sk.bones.len(), 2);
    assert_eq!(sk.sequences.len(), 2, "…and so do the clip windows");
}

#[test]
fn creature_geosets_keep_the_lowest_variant_in_each_group() {
    let batch = |geoset| M2Batch {
        geoset,
        index_start: 0,
        index_count: 3,
        texture: None,
        blend: 0,
        unlit: false,
        two_sided: false,
        no_depth_write: false,
        tint: None,
        uv: None,
    };
    let mut m2 = M2::parse(&build_model()).expect("valid");
    m2.batches = vec![
        batch(0),
        batch(1),
        batch(2),
        batch(101),
        batch(102),
        batch(402),
        batch(403),
        batch(701),
        batch(702),
        batch(1301),
    ];

    // Creature: body, plus the lowest variant of every other group. The
    // hairstyles (geosets 1 and 2) are dropped.
    let creature: Vec<u16> = m2
        .visible_batches(Dress::Creature)
        .iter()
        .map(|b| b.geoset)
        .collect();
    assert_eq!(creature, vec![0, 101, 402, 701, 1301]);

    // Character: body, the hairstyle *asked for* rather than the first one,
    // the beard the facial-hair table named, ears, and the x01 "nothing
    // equipped" variants — so group 4's 402/403 gloves are both dropped and
    // 401 is absent because this model has no bare hands.
    let look = CharacterGeosets {
        hair: 2,
        facial: [2, 0, 0],
        equipment: [0; 12],
        hide_ears: false,
    };
    let character: Vec<u16> = m2
        .visible_batches(Dress::Character(look))
        .iter()
        .map(|b| b.geoset)
        .collect();
    assert_eq!(character, vec![0, 2, 102, 702, 1301]);

    // **Bald is a real answer**, not a missing lookup: `CharHairGeosets`
    // gives human male style 0 exactly geoset 0, and asking for it must not
    // fall back to some other hairstyle.
    let bald: Vec<u16> = m2
        .visible_batches(Dress::Character(CharacterGeosets::default()))
        .iter()
        .map(|b| b.geoset)
        .collect();
    assert_eq!(bald, vec![0, 702, 1301]);
}

#[test]
fn a_model_with_one_geoset_is_drawn_whole() {
    let m2 = M2::parse(&build_model()).expect("valid");
    // Every batch is geoset 401 and there is no 400, so "lowest in group"
    // keeps it. A doodad (all geoset 0) is likewise untouched.
    assert_eq!(m2.visible_batches(Dress::Creature).len(), 1);
    assert_eq!(
        m2.visible_batches(Dress::Character(CharacterGeosets::default()))
            .len(),
        1
    );
}

#[test]
fn dbc_model_paths_become_archive_paths() {
    // Every creature in the game is named .mdx in the DBC and stored as .m2.
    assert_eq!(model_path("Creature\\Wolf\\Wolf.mdx"), "Creature\\Wolf\\Wolf.m2");
    assert_eq!(model_path("Creature\\Wolf\\Wolf.MDX"), "Creature\\Wolf\\Wolf.m2");
    assert_eq!(model_path("x\\y.mdl"), "x\\y.m2");
    assert_eq!(model_path("x\\y.m2"), "x\\y.m2");
    assert_eq!(model_path("x\\y"), "x\\y.m2");
}

/// **The ground-quad shape, and the things that are not it.**
///
/// The detection decides which *pass* a batch is drawn by (see
/// `M2::ground_quad` and `crate::render::decals` on the client side), and it is
/// deliberately strict — a near-miss stretched through a projector is worse than
/// a flat quad. So the negatives matter as much as the positive: 12 of the
/// spell-effect population's 132 flat batches are rejected by this and stay on
/// the ordinary path.
#[test]
fn a_flat_axis_aligned_rectangle_is_a_ground_quad_and_nothing_else_is() {
    // The corners in the file's own order, deliberately shuffled: the detection
    // slots each vertex into its bilinear position rather than trusting the
    // order they were authored in.
    let quad = |z: [f32; 4]| {
        let mut m2 = M2 {
            positions: vec![
                [1.0, 2.0, z[0]],
                [-1.0, -2.0, z[1]],
                [1.0, -2.0, z[2]],
                [-1.0, 2.0, z[3]],
            ],
            uvs: vec![[1.0, 1.0], [0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![1, 2, 0, 1, 0, 3],
            bone_weights: vec![[255, 0, 0, 0]; 4],
            bone_indices: vec![[7, 0, 0, 0]; 4],
            ..empty_model()
        };
        m2.batches = vec![flat_batch()];
        m2
    };

    let m2 = quad([0.0; 4]);
    let found = m2.ground_quad(&m2.batches[0]).expect("a ground quad");
    assert_eq!(found.bone, 7);
    // Rectangle order: (min x, min y), (max x, min y), (min x, max y), (max…).
    assert_eq!(found.corners[0], [-1.0, -2.0, 0.0]);
    assert_eq!(found.corners[1], [1.0, -2.0, 0.0]);
    assert_eq!(found.corners[2], [-1.0, 2.0, 0.0]);
    assert_eq!(found.corners[3], [1.0, 2.0, 0.0]);
    // …and the UVs travel with their own corners.
    assert_eq!(found.uvs[0], [0.0, 0.0]);
    assert_eq!(found.uvs[3], [1.0, 1.0]);

    // One vertex lifted off the plane: not flat, not a ground quad.
    let tilted = quad([0.0, 0.0, 0.0, 0.5]);
    assert!(tilted.ground_quad(&tilted.batches[0]).is_none());

    // Two bones: the projector poses through one joint and could not follow it.
    let mut split = quad([0.0; 4]);
    split.bone_indices[2] = [8, 0, 0, 0];
    assert!(split.ground_quad(&split.batches[0]).is_none());

    // A partial weight, likewise.
    let mut soft = quad([0.0; 4]);
    soft.bone_weights[1] = [128, 127, 0, 0];
    assert!(soft.ground_quad(&soft.batches[0]).is_none());

    // Not a rectangle: a vertex between the extremes on x.
    let mut skew = quad([0.0; 4]);
    skew.positions[0][0] = 0.25;
    assert!(skew.ground_quad(&skew.batches[0]).is_none());

    // A boneless model still qualifies — an effect with no skeleton rides its
    // attachment root, which is the same single frame.
    let mut boneless = quad([0.0; 4]);
    boneless.bone_weights.clear();
    boneless.bone_indices.clear();
    assert_eq!(boneless.ground_quad(&boneless.batches[0]).map(|q| q.bone), Some(0));

    // Six vertices is a strip, not a quad, however flat it is.
    let mut strip = quad([0.0; 4]);
    strip.indices = vec![0, 1, 2, 0, 2, 3, 1, 2, 3];
    strip.batches[0].index_count = 9;
    assert!(strip.ground_quad(&strip.batches[0]).is_none());
}

/// A batch covering the whole of a four-vertex model, for the test above.
fn flat_batch() -> M2Batch {
    M2Batch {
        geoset: 0,
        index_start: 0,
        index_count: 6,
        texture: Some(0),
        blend: 4,
        unlit: true,
        two_sided: false,
        no_depth_write: true,
        tint: None,
        uv: None,
    }
}

/// An `M2` with nothing in it, so a test can fill in only what it is about.
fn empty_model() -> M2 {
    M2 {
        name: String::new(),
        version: 256,
        global_flags: 0,
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        bone_weights: Vec::new(),
        bone_indices: Vec::new(),
        textures: Vec::new(),
        batches: Vec::new(),
        attachments: Vec::new(),
        events: Vec::new(),
        particles: Vec::new(),
        ribbons: Vec::new(),
        cameras: Vec::new(),
        lights: Vec::new(),
        bounding_radius: 0.0,
        bounds: [[0.0; 3]; 2],
        collision: crate::world::collision::CollisionMesh::default(),
        skeleton: None,
        tints: M2Tints::default(),
        uv_anims: M2TextureAnims::default(),
    }
}

/// **The texture matrix translates first and rotates second**, which is the
/// order the cooldown clock measures — see [`M2TextureAnims::matrix`].
///
/// Built here rather than read out of the archive so the arithmetic is pinned
/// with no file: a 90° turn and a translation of a whole tile along v, which is
/// exactly the state each of `UI-Cooldown-Indicator`'s four quadrants ends its
/// own quarter in. Under `R · T` the `v` translation comes out along **u**,
/// which is what takes the sampled square off the dark half of the sheet and
/// clears the quadrant; under `T · R` it stays along v and the quadrant is dark
/// for ever.
#[test]
fn the_texture_matrix_translates_before_it_rotates() {
    let track = |dim: usize, values: Vec<f32>| M2Track {
        interpolation: 0,
        global_sequence: -1,
        times: vec![0],
        values,
        dim,
    };
    let anims = M2TextureAnims {
        transforms: vec![M2TextureTransform {
            // A quarter turn about Z, in the direction the cooldown's own
            // quadrants turn: (x, y, z, w) = (0, 0, -sin45, cos45), which
            // `2 * atan2(z, w)` reads back as -90°.
            rotation: Some(track(
                4,
                vec![0.0, 0.0, -std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2],
            )),
            translation: Some(track(3, vec![0.0, -0.91, 0.0])),
            scale: None,
        }],
        global_sequences: Vec::new(),
    };

    let m = anims.matrix(0, 0, 0, 1000, 0);
    // `u' = a*u + b*v + tx`: a 90° turn sends v into u, and the *translation*
    // has to arrive turned as well.
    let map = |u: f32, v: f32| (m[0] * u + m[1] * v + m[2], m[3] * u + m[4] * v + m[5]);
    let (u, _) = map(0.5, 0.5);
    assert!(
        (u - (0.5 - 0.91)).abs() < 1e-4,
        "the middle of the tile should land at v - 0.91 along u, not at {u}"
    );
    // …and the whole of the cooldown's own quad, which must end up left of the
    // texture's transparency edge at u = 0.125.
    for (u, v) in [(0.146, 0.095), (0.146, 1.003), (1.054, 0.095), (1.054, 1.003)] {
        let (u, _) = map(u, v);
        assert!(u < 0.125, "a finished quadrant still samples the dark half at u = {u}");
    }
}

/// A batch that plots the same triangles as an earlier opaque one, does not
/// write depth and blends, is a **layer of it** — see [`overlay_layers`]. This
/// is the shape nearly every worn item in the game is authored in.
#[test]
fn an_env_map_layer_folds_into_the_batch_under_it() {
    let base = M2Batch {
        geoset: 0,
        index_start: 0,
        index_count: 300,
        texture: Some(0),
        blend: 0,
        unlit: false,
        two_sided: false,
        no_depth_write: false,
        tint: None,
        uv: None,
    };
    // opaque base, modulate2x sheen, alpha glow — the helm's own shape.
    let batches = vec![
        base.clone(),
        M2Batch { blend: 6, no_depth_write: true, texture: Some(1), ..base.clone() },
        M2Batch { blend: 2, no_depth_write: true, texture: Some(2), ..base.clone() },
    ];
    assert_eq!(overlay_layers(&batches), vec![None, Some(0), Some(0)]);

    // …and a third layer is left to draw for itself, because a material has
    // two slots — see [`MAX_OVERLAYS`].
    let mut four = batches.clone();
    four.push(M2Batch { blend: 4, no_depth_write: true, texture: Some(1), ..base.clone() });
    assert_eq!(overlay_layers(&four), vec![None, Some(0), Some(0), None]);
}

/// **Every one of the conditions is a way the fold would change the picture**,
/// so each is checked on its own — a fold that quietly took one of these would
/// be a wrong pixel nothing warns about.
#[test]
fn a_layer_that_would_change_the_picture_is_not_folded() {
    let base = M2Batch {
        geoset: 0,
        index_start: 0,
        index_count: 300,
        texture: Some(0),
        blend: 0,
        unlit: false,
        two_sided: false,
        no_depth_write: false,
        tint: None,
        uv: None,
    };
    let over = M2Batch { blend: 6, no_depth_write: true, texture: Some(1), ..base.clone() };
    let refused = |b: M2Batch, o: M2Batch| assert_eq!(overlay_layers(&[b, o])[1], None);
    // A different triangle range is different geometry.
    refused(base.clone(), M2Batch { index_count: 120, ..over.clone() });
    refused(base.clone(), M2Batch { index_start: 12, ..over.clone() });
    // A depth-writing overlay occludes; a base that does not write is not one.
    refused(base.clone(), M2Batch { no_depth_write: false, ..over.clone() });
    refused(M2Batch { no_depth_write: true, ..base.clone() }, over.clone());
    // A translucent base has no settled output for the layer to read.
    refused(M2Batch { blend: 2, ..base.clone() }, over.clone());
    // Different geosets can be selected apart by a dressing.
    refused(base.clone(), M2Batch { geoset: 1, ..over.clone() });
    // One lighting term and one face test serve the whole stack.
    refused(base.clone(), M2Batch { unlit: true, ..over.clone() });
    refused(base.clone(), M2Batch { two_sided: true, ..over.clone() });
    // An animated batch's value rides its own material or its own tag.
    refused(base.clone(), M2Batch { uv: Some(0), ..over.clone() });
    let tinted = Some(BatchTint { color: Some(0), transparency: None });
    refused(base.clone(), M2Batch { tint: tinted, ..over.clone() });
    refused(M2Batch { tint: tinted, ..base.clone() }, over.clone());
    // …and the pattern still holds when nothing is wrong with it.
    assert_eq!(overlay_layers(&[base, over])[1], Some(0));
}

/// **A batch authored invisible for its whole timeline is told apart from one
/// that fades in from nothing**, which is the whole of
/// [`M2Tints::never_visible`]'s job.
///
/// The first is Ironforge's `lavasteam.m2`, whose one blend-0 batch carries a
/// single transparency key of 0.000 and exists only to hold eight particle
/// emitters. Drawn, it is a 60-yard slab standing in the Great Forge. The
/// second is every fade-in in the game, and dropping one of those would take a
/// spell effect off the screen.
#[test]
fn a_batch_that_is_never_visible_is_told_apart_from_one_that_fades_in() {
    let track = |values: Vec<f32>| M2Track {
        interpolation: 1,
        global_sequence: -1,
        times: (0..values.len() as u32).map(|i| i * 500).collect(),
        values,
        dim: 1,
    };
    let tints = M2Tints {
        colors: vec![M2Color { color: None, alpha: Some(track(vec![0.0, 0.0])) }],
        transparencies: vec![
            track(vec![0.0]),
            track(vec![0.0, 1.0]),
            track(vec![1.0]),
        ],
        global_sequences: Vec::new(),
    };

    let weight = |i: u16| BatchTint { color: None, transparency: Some(i) };
    assert!(tints.never_visible(weight(0)), "one key of zero: the lava steam");
    assert!(!tints.never_visible(weight(1)), "a fade in from nothing still draws");
    assert!(!tints.never_visible(weight(2)), "and an opaque batch certainly does");

    // The colour block's own alpha is the other half, and either one flat at
    // zero is enough — `sample` multiplies them.
    assert!(tints.never_visible(BatchTint { color: Some(0), transparency: Some(2) }));

    // **A batch naming no track at all is opaque, not invisible.** The identity
    // an absent track answers is white and 1.0, and this is the great majority
    // of every model in the game.
    assert!(!tints.never_visible(BatchTint { color: None, transparency: None }));

    // A track index the file does not have is not a reason to drop geometry.
    assert!(!tints.never_visible(weight(99)));
}

/// **A bone with no track poses exactly as one with constant identity keys.**
///
/// `pose` skips the sampling for a trackless bone and copies its parent's
/// matrix — see the still-bone shortcut in [`M2Skeleton::pose`] — on the
/// argument that its local is the identity bit for bit. This is that argument
/// checked against the long path: two skeletons, one whose child carries no
/// tracks and one whose child carries a single key of zero translation, the
/// unit quaternion and unit scale, posed under an animated parent, under a
/// cross-fade, under the overlay and under a billboard, must agree exactly.
/// The twist bones stay on the long path and are checked to.
#[test]
fn a_trackless_bone_is_its_parent_bit_for_bit() {
    use crate::world::m2::{key_bone, M2Bone, M2Track, Blend, BodyTwist, Overlay};
    let constant = |dim: usize, value: &[f32]| -> Option<M2Track> {
        Some(M2Track {
            interpolation: 1,
            global_sequence: -1,
            times: vec![0],
            values: value.to_vec(),
            dim,
        })
    };
    let moving_root = |t: [f32; 3]| M2Bone {
        key_bone: -1,
        flags: 0,
        parent: -1,
        pivot: [0.3, -0.2, 1.7],
        translation: Some(M2Track {
            interpolation: 1,
            global_sequence: -1,
            times: vec![0, 1000, 2000, 3000],
            values: vec![0.0, 0.0, 0.0, t[0], t[1], t[2], 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            dim: 3,
        }),
        rotation: Some(M2Track {
            interpolation: 1,
            global_sequence: -1,
            times: vec![0, 1000, 2000, 3000],
            values: vec![
                0.0, 0.0, 0.0, 1.0, //
                0.0, 0.0, 0.7071, 0.7071, //
                0.0, 0.7071, 0.0, 0.7071, //
                0.0, 0.0, 0.0, 1.0,
            ],
            dim: 4,
        }),
        scale: None,
    };
    let sequences = vec![
        M2Sequence { id: 0, variation: 0, start: 0, end: 1000, move_speed: 0.0, flags: 0, probability: 0x7fff, bounds: [[0.0; 3]; 2], radius: 0.0 },
        M2Sequence { id: 4, variation: 0, start: 1000, end: 2000, move_speed: 0.0, flags: 0, probability: 0x7fff, bounds: [[0.0; 3]; 2], radius: 0.0 },
        M2Sequence { id: 5, variation: 0, start: 2000, end: 3000, move_speed: 0.0, flags: 0, probability: 0x7fff, bounds: [[0.0; 3]; 2], radius: 0.0 },
    ];
    let skeleton = |child_tracks: bool, flags: u32, key: i16| {
        let (translation, rotation, scale) = if child_tracks {
            (
                constant(3, &[0.0, 0.0, 0.0]),
                constant(4, &[0.0, 0.0, 0.0, 1.0]),
                constant(3, &[1.0, 1.0, 1.0]),
            )
        } else {
            (None, None, None)
        };
        M2Skeleton::new(
            vec![
                moving_root([2.0, 3.0, 4.0]),
                M2Bone { key_bone: key, flags, parent: 0, pivot: [0.5, 0.25, 0.125], translation, rotation, scale },
                // …and a grandchild under the still one, which composes onto
                // whatever the shortcut left there.
                M2Bone { key_bone: -1, flags: 0, parent: 1, pivot: [0.0, 0.0, 0.0], translation: constant(3, &[0.1, 0.2, 0.3]), rotation: None, scale: None },
            ],
            sequences.clone(),
            Vec::new(),
        )
    };
    let camera = Some(([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]));
    let layers = [
        PoseLayers::default(),
        PoseLayers { blend: Some(Blend { sequence: 1, elapsed_ms: 400, weight: 0.3 }), ..Default::default() },
        PoseLayers { overlay: Some(Overlay { sequence: 2, elapsed_ms: 250 }), ..Default::default() },
        PoseLayers { twist: Some(BodyTwist { gap: 0.4 }), ..Default::default() },
    ];
    for (flags, key) in [(0, -1), (crate::world::m2::bone_flags::BILLBOARD_LOCK_Z, -1), (0, key_bone::SPINE_LOW), (0, key_bone::HEAD)] {
        let still = skeleton(false, flags, key);
        let keyed = skeleton(true, flags, key);
        for layers in layers {
            for camera in [None, camera] {
                for (sequence, elapsed) in [(0, 0), (0, 600), (1, 500), (2, 999)] {
                    let a = still.pose(sequence, elapsed, 0, camera, layers);
                    let b = keyed.pose(sequence, elapsed, 0, camera, layers);
                    assert_eq!(
                        a, b,
                        "flags {flags:#x} key {key} sequence {sequence} at {elapsed} with {layers:?}, camera {}",
                        camera.is_some()
                    );
                }
            }
        }
    }
}

/// **A cue fires once, in the frame that crosses it**, and the first frame
/// of a sequence takes its very first millisecond; a loop that wrapped takes
/// the tail and the head both.
#[test]
fn a_sound_cue_lands_in_its_sequence_and_fires_in_the_frame_that_crosses_it() {
    use crate::world::m2::{M2Event, SoundCues};
    let sequences = vec![
        M2Sequence { id: 0, variation: 0, start: 0, end: 1000, move_speed: 0.0, flags: 0, probability: 0x7fff, bounds: [[0.0; 3]; 2], radius: 0.0 },
        M2Sequence { id: 77, variation: 0, start: 5000, end: 6000, move_speed: 0.0, flags: 0, probability: 0x7fff, bounds: [[0.0; 3]; 2], radius: 0.0 },
    ];
    let events = vec![
        M2Event { id: *b"$CSD", data: 6921, bone: 1, position: [0.0; 3], times: vec![5000, 5500] },
        M2Event { id: *b"$CSD", data: 6576, bone: 1, position: [0.0; 3], times: vec![200, 9999] },
        M2Event { id: *b"$CAH", data: 0, bone: 2, position: [0.0; 3], times: vec![5100] },
        M2Event { id: *b"$SND", data: 0, bone: 2, position: [0.0; 3], times: vec![5200] },
    ];
    let cues = SoundCues::from_events(&events, &sequences);
    assert_eq!(cues.of(1), &[(0, 6921), (500, 6921)]);
    assert_eq!(cues.of(0), &[(200, 6576)], "9999 is in no window; $CAH and a zero $SND are not sounds");
    let fired = |from, to, fresh| cues.in_window(1, from, to, fresh).collect::<Vec<_>>();
    assert_eq!(fired(0, 16, true), vec![6921], "the first frame takes millisecond zero");
    assert_eq!(fired(0, 16, false), Vec::<u32>::new(), "and a later frame at the same clock does not");
    assert_eq!(fired(480, 520, false), vec![6921]);
    assert_eq!(fired(520, 560, false), Vec::<u32>::new());
    assert_eq!(fired(950, 30, false), vec![6921], "a wrap crosses the head");
    assert!(cues.in_window(7, 0, 100, true).next().is_none(), "no such sequence");
    assert!(!cues.is_empty());
}
