//! One M2, field by field, the way the renderer sees it — and the survey of
//! the whole spell-effect population behind it.
//!
//! **This is the instrument the effects round was missing.** `vale anim`
//! answers what a model can *play*, `vale particles` what it *sprays*, and
//! between them sat everything that decides what a spell effect actually looks
//! like on screen: which batch draws with which blend mode, which bone is a
//! camera billboard, whether a batch's texture is on a moving matrix, and
//! whether the model's visible half is geometry at all or a trail hanging off a
//! bone. Every one of those fails *plausibly* — a ribbon that is not parsed
//! draws nothing, a texture matrix that is not applied draws a smear, and a
//! wrongly billboarded bone draws a ring standing on edge. None of them
//! produces an error, and none of them is visible in any other command's
//! output.
//!
//! The survey is the layout check, on the same terms as `vale particles`:
//! the ribbon record is 0xDC bytes at a header offset nothing else in this
//! parser resolves, so the share of the spell-effect models carrying one, and
//! the physicality of the widths and lifetimes it reads, is what says the
//! stride is right.

use crate::common::*;
use vale_assets::world::m2::bone_flags as bone_flags_mask;
use vale_assets::world::m2::{Billboard, M2Ribbon, M2Track, M2};
use vale_config::Config;
use std::collections::{BTreeMap, BTreeSet};

/// [`vale_assets::world::m2::NotGround`], in words.
fn why_not(why: &vale_assets::world::m2::NotGround) -> String {
    use vale_assets::world::m2::NotGround as N;
    match why {
        N::NotTwoTriangles => "not two triangles".to_string(),
        N::NotFourCorners => "two triangles that do not share an edge".to_string(),
        N::Truncated => "an index past the end of the vertex arrays".to_string(),
        N::NotFlat(off) => format!("not flat — {off:.3}y off its own plane"),
        N::NotOneBone => "the four corners do not share one bone at full weight".to_string(),
        N::Degenerate => "no area in one direction".to_string(),
        N::NotARectangle => "coplanar, but not on the corners of its own box".to_string(),
    }
}

/// `M2Material::blending_mode`, named. The numbers are the file's.
fn blend_name(blend: u16) -> &'static str {
    match blend {
        0 => "opaque",
        1 => "alpha-key",
        2 => "alpha",
        3 => "additive",
        4 => "add-alpha",
        5 => "modulate",
        6 => "modulate2x",
        _ => "?",
    }
}

/// What one bit of `M2Bone::flags` is called — see
/// `vale_assets::world::m2::bone_flags`, which is where the reading lives.
fn bone_flag_name(flag: u32) -> &'static str {
    match flag {
        bone_flags_mask::IGNORE_PARENT_TRANSLATE => "ignore parent translate",
        bone_flags_mask::IGNORE_PARENT_SCALE => "ignore parent scale",
        bone_flags_mask::IGNORE_PARENT_ROTATE => "ignore parent rotate",
        bone_flags_mask::BILLBOARD_SPHERICAL => "billboard spherical",
        bone_flags_mask::BILLBOARD_LOCK_X => "billboard lock X",
        bone_flags_mask::BILLBOARD_LOCK_Y => "billboard lock Y",
        bone_flags_mask::BILLBOARD_LOCK_Z => "billboard lock Z",
        bone_flags_mask::TRANSFORMED => "transformed",
        _ => "",
    }
}

/// A track's shape in one line: how many keys, over what range, and whether it
/// runs on a global sequence. `None` prints the identity the renderer uses.
fn track_line(track: &Option<M2Track>, identity: &str) -> String {
    let Some(track) = track else {
        return format!("(none, {identity})");
    };
    if track.times.is_empty() {
        return format!("(keyless, {identity})");
    }
    let lo = track.values.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = track
        .values
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let gseq = if track.global_sequence >= 0 {
        format!(" gseq {}", track.global_sequence)
    } else {
        String::new()
    };
    format!(
        "{} keys {}..{} ms, {lo:.3}..{hi:.3}{gseq}",
        track.times.len(),
        track.times.first().copied().unwrap_or(0),
        track.times.last().copied().unwrap_or(0),
    )
}

/// Whether a tint track earns the per-instance word, in one word.
fn moving(tints: bool) -> &'static str {
    if tints {
        "(moves)"
    } else {
        "(identity — dropped)"
    }
}

/// What the decoded image actually contains — the half `describe_blp` cannot
/// say, and the half that decides whether a batch draws a shape or a slab.
///
/// **An effect texture's alpha is its silhouette.** These are drawn additively
/// over a quad the size of the whole effect, so what makes a shockwave a ring
/// rather than a 19-yard grey rectangle is the texture, and there are two ways
/// it can be one: black around the shape (which adds nothing) or transparent
/// around it. A file with `alphaDepth=0` has only the first, so its *dark
/// share* is the thing to measure — a mostly-black image is a shape and a
/// mostly-bright one is a slab.
fn decoded_shape(raw: &[u8]) -> String {
    let Ok(image) = vale_assets::world::blp::decode(raw) else {
        return "WILL NOT DECODE".to_string();
    };
    let pixels = image.rgba.len() / 4;
    if pixels == 0 {
        return "empty".to_string();
    }
    let mut dark = 0usize;
    let mut clear = 0usize;
    let mut sum = [0u64; 4];
    for p in image.rgba.chunks_exact(4) {
        let luma = u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
        if luma < 3 * 16 {
            dark += 1;
        }
        if p[3] < 16 {
            clear += 1;
        }
        for c in 0..4 {
            sum[c] += u64::from(p[c]);
        }
    }
    let pct = |n: usize| 100.0 * n as f32 / pixels as f32;
    format!(
        "{}x{} mean {}/{}/{} a{} — {:.0}% dark, {:.0}% transparent",
        image.width,
        image.height,
        sum[0] / pixels as u64,
        sum[1] / pixels as u64,
        sum[2] / pixels as u64,
        sum[3] / pixels as u64,
        pct(dark),
        pct(clear),
    )
}

fn print_ribbon(m2: &M2, i: usize, r: &M2Ribbon) {
    let texture = match r.texture.and_then(|t| m2.textures.get(t as usize)) {
        Some(t) if t.kind == 0 => t.file_name.clone(),
        Some(t) => format!("<client-supplied type {}>", t.kind),
        None => "<none>".to_string(),
    };
    println!(
        "  ribbon [{i}] bone {} at [{:.2}, {:.2}, {:.2}], blend {} {}{}",
        r.bone,
        r.position[0],
        r.position[1],
        r.position[2],
        r.blend,
        blend_name(r.blend),
        if r.two_sided { ", two-sided" } else { "" },
    );
    println!(
        "      texture: {texture}  atlas {}x{} cell {}",
        r.tile_rows, r.tile_cols, r.tex_slot
    );
    println!(
        "      {:.1} edges/s, edge life {:.2} s, gravity {:.2}, peak width {:.3} y",
        r.edges_per_second,
        r.edge_lifetime,
        r.gravity,
        r.peak_height(),
    );
    println!("      height above: {}", track_line(&r.height_above, "0"));
    println!("      height below: {}", track_line(&r.height_below, "0"));
    println!("      alpha:        {}", track_line(&r.alpha, "1.0"));
    println!("      colour:       {}", track_line(&r.color, "white"));
    println!(
        "      visibility:   {}",
        track_line(&r.visibility, "always on")
    );
}

/// One model, in full.
fn trace_model(assets: &mut vale_assets::Assets, path: &str) -> Result<(), String> {
    let bytes = assets.read(path).map_err(|e| e.to_string())?;
    let m2 = M2::parse(&bytes).map_err(|e| e.to_string())?;

    println!("{path}  (version {})", m2.version);
    println!(
        "  {} vertices, {} indices, {} batches, radius {:.2}, box [{:.1}, {:.1}, {:.1}]..[{:.1}, {:.1}, {:.1}]",
        m2.positions.len(),
        m2.indices.len(),
        m2.batches.len(),
        m2.bounding_radius,
        m2.bounds[0][0], m2.bounds[0][1], m2.bounds[0][2],
        m2.bounds[1][0], m2.bounds[1][1], m2.bounds[1][2],
    );

    println!("  textures:");
    for (i, t) in m2.textures.iter().enumerate() {
        if t.kind != 0 {
            println!("    [{i}] type {} — supplied by the client", t.kind);
            continue;
        }
        let state = match assets.read(&t.file_name) {
            Ok(raw) => format!("{}  {}", describe_blp(&raw), decoded_shape(&raw)),
            Err(_) => "MISSING from the archive chain".to_string(),
        };
        println!("    [{i}] {}  {state}", t.file_name);
    }

    println!("  batches:");
    let layers = vale_assets::world::m2::overlay_layers(&m2.batches);
    for (i, b) in m2.batches.iter().enumerate() {
        let mut flags = Vec::new();
        if b.unlit {
            flags.push("unlit");
        }
        if b.two_sided {
            flags.push("two-sided");
        }
        if b.no_depth_write {
            flags.push("no-depth-write");
        }
        let tint = match b.tint {
            Some(t) => format!(" tint(colour {:?}, alpha {:?})", t.color, t.transparency),
            None => String::new(),
        };
        let uv = match b.uv {
            Some(u) => format!(" uv-matrix {u}"),
            None => String::new(),
        };
        // **The ground quad, which is the one property of a batch that decides
        // which *pass* draws it.** A batch this names is re-rendered draped over
        // the terrain (`render::decals`) rather than as free geometry, so a
        // model whose spiral does not appear here is one that will clip through
        // the hillside it is cast on.
        //
        // **And a rejection says which test it missed**, which is the half that
        // was missing: the detection is strict on purpose, so a batch that is
        // very nearly a ground quad is drawn as free geometry and is *plausibly*
        // wrong rather than absent. `NotFlat` was Consecration's answer.
        let checked = m2.ground_quad_checked(b);
        let quad = checked.as_ref().ok().copied();
        let ground = match &checked {
            Ok(q) => format!(" GROUND QUAD (bone {}, z {:+.3})", q.bone, q.corners[0][2]),
            // Only worth a word for a batch that is at least the right size:
            // every ordinary mesh in the game is "not two triangles" and saying
            // so 1,300 times is noise.
            Err(vale_assets::world::m2::NotGround::NotTwoTriangles) => String::new(),
            Err(why) => format!(" not a ground quad: {}", why_not(why)),
        };
        // **Whether this batch is drawn at all**, or is an environment-map
        // layer of an earlier one folded into its material — see
        // `vale_assets::world::m2::overlay_layers`. Nearly every worn item in the
        // game is a base plus one or two of these, and each was its own draw
        // call in the sorted phase until the round that folded them.
        let layer = match layers.get(i).copied().flatten() {
            Some(base) => format!("  -> layer of [{base}], folded"),
            None => String::new(),
        };
        println!(
            "    [{i}] geoset {:4} tex {:?} blend {} {:<10} [{}]{tint}{uv}  {} tris{ground}{layer}",
            b.geoset,
            b.texture,
            b.blend,
            blend_name(b.blend),
            flags.join(" "),
            b.index_count / 3,
        );
        // **…and where that quad actually lands once its bone has posed it**,
        // which is the whole of what `render::decals` sees. The detection above
        // says the batch is the right *shape*; this says whether the shape is
        // where a projector could use it — a half-extent, a height off the
        // model's own ground plane, and both sampled through the clip, because a
        // quad that starts at scale zero and grows is indistinguishable from one
        // that never appears if only the first frame is looked at.
        if let Some(q) = quad {
            trace_ground_quad(&m2, &q);
        }
    }

    // **The bones, and only what makes one behave unlike a bone.** A spell
    // effect's whole appearance is often one billboarded quad, and a bone whose
    // billboard rule is not applied draws it edge-on — so the flags are the
    // point here, not the hierarchy. The rule is *named* rather than the bit
    // printed, because the four are four different constructions and only one
    // of them replaces the whole basis: `lock Z` is the one every worn aura in
    // the game uses and it keeps the bone's own vertical.
    match &m2.skeleton {
        None => println!("  no skeleton (drawn in its bind pose)"),
        Some(s) => {
            let mut billboards: Vec<String> = Vec::new();
            for (i, bone) in s.bones.iter().enumerate() {
                if let Some(kind) = Billboard::of(bone.flags) {
                    billboards.push(format!("{i} {kind:?}"));
                }
            }
            let other: BTreeSet<u32> = s
                .bones
                .iter()
                .map(|b| b.flags & !vale_assets::world::m2::bone_flags::BILLBOARD_ANY)
                .filter(|&f| f != 0)
                .collect();
            println!(
                "  {} bones, {} sequences, {} global sequences",
                s.bones.len(),
                s.sequences.len(),
                s.global_sequences.len(),
            );
            println!(
                "    billboarded: {}",
                if billboards.is_empty() {
                    "none".to_string()
                } else {
                    billboards.join(", ")
                }
            );
            if !other.is_empty() {
                let names: Vec<String> = other.iter().map(|f| format!("{f:#x}")).collect();
                println!("    other bone flags seen: {}", names.join(" "));
            }
            for (i, seq) in s.sequences.iter().enumerate() {
                println!(
                    "    seq [{i}] id {:3} {:5}..{:5} ms  flags {:#x} {}",
                    seq.id,
                    seq.start,
                    seq.end,
                    seq.flags,
                    if seq.flags & 1 == 0 {
                        "loops"
                    } else {
                        "one-shot"
                    },
                );
            }
        }
    }

    // **Where other things hang off this one**, which is the check on every
    // reading of what a numeric point *means*: point 0 is a shield's forearm on
    // a character, a plinth on a glue backdrop and the **saddle** on a mount,
    // and what tells the three apart is which models carry it. A creature
    // nobody rides carries no point 0 at all — `Creature\Tiger` is the standing
    // example — so this line is what makes that absence visible rather than
    // asserted in a doc comment.
    if m2.attachments.is_empty() {
        println!("  no attachment points");
    } else {
        let points: Vec<String> = m2
            .attachments
            .iter()
            .map(|p| {
                let [x, y, z] = p.position;
                format!("{}@bone {} ({x:.2}, {y:.2}, {z:.2})", p.id, p.bone)
            })
            .collect();
        println!("  {} attachment points:", m2.attachments.len());
        for point in points {
            println!("    {point}");
        }
    }

    // **…and the one of them the camera is about**, which is the check on
    // `vale_assets::look::anchor` against a real file. A creature model that falls
    // to the box fraction is saying it carries no point 17; the wisp, whose box
    // is a symmetric cube centred on its own origin, is the standing example of
    // why the fallback is the *height* and not the centre. See that module.
    let height = m2.bounds[1][2] - m2.bounds[0][2];
    let anchor = vale_assets::look::anchor::base(&m2.attachments, height);
    let source = if m2
        .attachments
        .iter()
        .any(|p| p.id == vale_assets::look::anchor::POINT)
    {
        format!("point {}", vale_assets::look::anchor::POINT)
    } else {
        format!(
            "{:.0}% of its {height:.2} height",
            vale_assets::look::anchor::BOX_FRACTION * 100.0
        )
    };
    // The clamp is applied in *world* yards, after the unit's own scale — so
    // this says where it would bite at scale 1 rather than pretending the
    // model-space number has already been through it.
    let note = if anchor < vale_assets::look::anchor::MIN {
        format!(
            " — under the {:.2} floor at scale 1",
            vale_assets::look::anchor::MIN
        )
    } else {
        String::new()
    };
    println!("  camera anchor {anchor:.2} model yards, from {source}{note}");
    if let Some(skeleton) = &m2.skeleton {
        if let Some(seated) = vale_assets::look::anchor::in_clip(
            &m2.attachments,
            skeleton,
            vale_assets::look::anchor::MOUNT_CLIP,
        ) {
            println!("    …and {seated:.2} in the Mount clip, which is what a rider is orbited at");
        }
    }

    // **What the tint slots actually hold**, which is the half the `tint(...)`
    // column above cannot say. Naming a slot and *fading* are different things:
    // every batch of every character model and of every item model in the game
    // names transparency track 0, and on almost all of them that track is one
    // key holding 1.0. `resolve_tint` drops those, so the `tint(...)` column is
    // now the batches that move — and this section is what says which slots
    // were there to be dropped, so the rule stays checkable rather than
    // invisible.
    if m2.tints.is_empty() {
        println!("  no colour or opacity tracks");
    } else {
        println!("  colour and opacity tracks:");
        for (i, c) in m2.tints.colors.iter().enumerate() {
            println!(
                "    colour [{i}] rgb {} {}  alpha {} {}",
                track_line(&c.color, "white"),
                moving(c.color.as_ref().is_some_and(|t| t.tints(3))),
                track_line(&c.alpha, "1"),
                moving(c.alpha.as_ref().is_some_and(|t| t.tints(1))),
            );
        }
        for (i, t) in m2.tints.transparencies.iter().enumerate() {
            println!(
                "    transparency [{i}] {} {}",
                track_line(&Some(t.clone()), "1"),
                moving(t.tints(1)),
            );
        }
        println!(
            "    {} of {} batches carry a tint the renderer will animate",
            m2.batches.iter().filter(|b| b.tint.is_some()).count(),
            m2.batches.len(),
        );
    }

    if m2.uv_anims.is_empty() {
        println!("  no texture matrices");
    } else {
        println!("  texture matrices:");
        for (i, t) in m2.uv_anims.transforms.iter().enumerate() {
            println!(
                "    [{i}] translate {}  rotate {}  scale {}",
                track_line(&t.translation, "0"),
                track_line(&t.rotation, "identity"),
                track_line(&t.scale, "1"),
            );
        }
    }

    // **What the model says lights the world around it**, which is the one
    // thing in an M2 that no other command prints and that
    // `crate::render::lamps` needs before it can believe anything: a lamppost's
    // glow is an additive quad on a billboarded bone, and the *light* beside it
    // is a `M2Light` record. Reading the quad and missing the record is how a
    // street of lit lanterns comes to light nothing at all.
    // **…and the lights it carries that are not light records**, which in this
    // game is all of them: `vale_assets::world::glow` reads an `unlit` batch
    // whose colour is not on a track as a lamp, and every lamppost, sconce,
    // candle and window glow in 1.12 is exactly that and nothing else. Printed
    // next to the `M2Light` block on purpose — the pair is what says which kind
    // a model is.
    let glows = vale_assets::world::glow::glows(&m2);
    if glows.is_empty() {
        println!("  no glow batches");
    } else {
        println!("  {} glow batch(es) — unlit, colour not animated:", glows.len());
        for (i, g) in glows.iter().enumerate() {
            let texture = g
                .texture
                .and_then(|t| m2.textures.get(t as usize))
                .map(|t| t.file_name.clone())
                .unwrap_or_else(|| "<unresolved>".into());
            println!(
                "    [{i}] at ({:.2}, {:.2}, {:.2})  extent {:.2}y  tex {texture}",
                g.at[0], g.at[1], g.at[2], g.extent,
            );
        }
    }

    if m2.lights.is_empty() {
        println!("  no lights");
    } else {
        println!("  {} light(s):", m2.lights.len());
        for (i, l) in m2.lights.iter().enumerate() {
            let byte = |c: [f32; 3]| {
                format!(
                    "{:3.0}/{:3.0}/{:3.0}",
                    c[0] * 255.0,
                    c[1] * 255.0,
                    c[2] * 255.0
                )
            };
            println!(
                "    [{i}] {}  bone {:>3}  at ({:.2}, {:.2}, {:.2})\n         \
                 diffuse {} x{:.2}   ambient {} x{:.2}   attenuation {:.1}..{:.1}y",
                if l.kind == 1 { "point      " } else { "directional" },
                l.bone,
                l.position[0],
                l.position[1],
                l.position[2],
                byte(l.diffuse),
                l.diffuse_intensity,
                byte(l.ambient),
                l.ambient_intensity,
                l.attenuation_start,
                l.attenuation_end,
            );
        }
    }

    if m2.ribbons.is_empty() {
        println!("  no ribbon emitters");
    } else {
        for (i, r) in m2.ribbons.iter().enumerate() {
            print_ribbon(&m2, i, r);
        }
    }
    println!(
        "  {} particle emitters (vale particles traces them)",
        m2.particles.len()
    );
    trace_flat(&m2);
    Ok(())
}

/// **A flat model, as an interface widget would draw it** — and for the one
/// that matters, the sweep of the cooldown clock.
///
/// `Interface\Cooldown\UI-Cooldown-Indicator` is a `<Model>` on every action
/// button in the game (`Cooldown.xml`), and the whole of what makes it a *clock*
/// rather than a still picture is arithmetic no other command here exercises:
/// four quadrant quads sampling one 32x32 sheet through four texture matrices
/// that turn 90 degrees and translate a tile, each over its own quarter of
/// sequence 0.
///
/// So this prints what `vale_assets::interface::uimodel::flatten` produces at five
/// phases, with **the share of each batch that lands on the dark half of its
/// texture** beside it. Passing looks like `100 100 100 100` at the start and
/// `0 0 0 0` at the end, one quadrant clearing per quarter — which is a clock
/// hand, and is the reading that measured the composition order of the texture
/// matrix (see `vale_assets::world::m2::M2TextureAnims::matrix`).
fn trace_flat(m2: &M2) {
    if !vale_assets::interface::uimodel::is_flat(m2) {
        return;
    }
    let sequence = m2.skeleton.as_ref().and_then(|s| s.sequences.first());
    let Some(seq) = sequence else { return };
    let length = seq.end.saturating_sub(seq.start);
    println!("  flat: drawable as an interface <Model>; sequence 0 is {length} ms");
    println!("    phase     batches  coverage per batch (% on the dark half)");
    for step in 0..=4 {
        let t = length * step / 4;
        let batches = vale_assets::interface::uimodel::flatten(m2, sequence, t, t);
        let drawn: Vec<_> = batches.iter().filter(|b| !b.is_invisible()).collect();
        let shares: Vec<String> = drawn
            .iter()
            .map(|b| format!("{:3.0}", dark_share(b) * 100.0))
            .collect();
        println!("    {t:5} ms  {:5}      {}", drawn.len(), shares.join(" "));
    }
}

/// How much of a flattened batch samples the **right** of `u = 0.125`.
///
/// That number is `cooldown.blp`'s own transparency edge — its left four
/// columns of 32 are clear and the rest is half-black — so for the cooldown
/// this is the fraction of the quadrant that is still darkened. For any other
/// flat model it is a statistic about its UVs, which is why the line above says
/// what it is measuring.
fn dark_share(batch: &vale_assets::interface::uimodel::UiBatch) -> f32 {
    let (lo, hi) = batch
        .vertices
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), v| {
            (lo.min(v.u), hi.max(v.u))
        });
    if hi <= lo {
        return 0.0;
    }
    ((hi - 0.125) / (hi - lo)).clamp(0.0, 1.0)
}

/// Where one ground quad lands once its own bone has posed it, sampled through
/// the model's first sequence.
///
/// **The half-extent and the height are the two numbers the projector acts on**
/// (`crate::render::decals`): it refuses a quad whose horizontal half-extent has
/// collapsed, and it drops any sample whose ground is further from the quad's own
/// plane than twice that half-extent. So a quad that poses far off `z = 0`, or
/// one that never grows, is a spiral that is authored correctly and drawn
/// nowhere — which no count of the batches can show.
fn trace_ground_quad(m2: &vale_assets::world::m2::M2, quad: &vale_assets::world::m2::GroundQuad) {
    let Some(skeleton) = &m2.skeleton else {
        return;
    };
    let Some(sequence) = skeleton.sequences.first() else {
        return;
    };
    let span = sequence.end.saturating_sub(sequence.start);
    let mut line = String::new();
    for step in 0..=4 {
        let elapsed = span * step / 4;
        let pose = skeleton.pose(0, elapsed, elapsed, None, Default::default());
        let Some(m) = pose.get(usize::from(quad.bone)) else {
            return;
        };
        // The bone's row-major 3x4 applied to each corner, in the model's own
        // axes — the same arithmetic `M2::skin_position` does for one vertex at
        // full weight, which is what these four are.
        let posed = quad.corners.map(|c| {
            [
                m[0] * c[0] + m[1] * c[1] + m[2] * c[2] + m[3],
                m[4] * c[0] + m[5] * c[1] + m[6] * c[2] + m[7],
                m[8] * c[0] + m[9] * c[1] + m[10] * c[2] + m[11],
            ]
        });
        let ex = [posed[1][0] - posed[0][0], posed[1][1] - posed[0][1]];
        let ez = [posed[2][0] - posed[0][0], posed[2][1] - posed[0][1]];
        let half = ex[0].hypot(ex[1]).max(ez[0].hypot(ez[1])) * 0.5;
        let z = posed.iter().map(|p| p[2]).sum::<f32>() / 4.0;
        line.push_str(&format!(" {elapsed}ms half {half:.2} z {z:+.2} |"));
    }
    println!("        posed:{line}");
}

pub fn cmd_model(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    if let Some(target) = target {
        let path = vale_assets::world::m2::model_path(target);
        return trace_model(&mut assets, &path);
    }

    // ---- the survey: every model the spell chain can name ----
    //
    // The same population `vale particles` uses, and the same argument:
    // these are the models that exist to *be* effects, so a block this parser
    // reads wrongly shows up as a share that is absurd rather than as a
    // failure.
    let bytes = assets
        .read(&vale_assets::tables::dbc::dbc_path("SpellVisualEffectName"))
        .map_err(|e| format!("SpellVisualEffectName.dbc: {e}"))?;
    let dbc = vale_assets::Dbc::parse(&bytes).map_err(|e| e.to_string())?;
    let paths: BTreeSet<String> = (0..dbc.record_count)
        .filter_map(|r| dbc.string_at(r, 2))
        .filter(|p| !p.is_empty())
        .map(|p| vale_assets::world::m2::model_path(&p))
        .collect();

    let mut read = 0usize;
    let mut with_ribbons = 0usize;
    let mut ribbons = 0usize;
    let mut ribbon_keyed_height = 0usize;
    let mut ribbon_zero_first_key = 0usize;
    let mut ribbon_gated = 0usize;
    let mut ribbon_no_texture = 0usize;
    let mut ribbon_blends = [0usize; 8];
    let mut with_uv = 0usize;
    let mut uv_batches = 0usize;
    let mut billboarded = 0usize;
    // **Every bone flag the population carries, and how many bones carry it.**
    // The renderer acts on exactly one of them (0x8), so this is the line that
    // says whether that is the whole of the subject or a subset of it — a
    // cylindrical billboard left unturned draws a quad edge-on in exactly the
    // way a spherical one does, and neither errors.
    let mut bone_flags: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    let mut billboard_combinations = 0usize;
    let mut sequence_idiom: BTreeMap<&str, usize> = BTreeMap::new();
    let mut batches_by_blend = [0usize; 8];
    let mut widest: Vec<(f32, String)> = Vec::new();
    // The ground-quad population: models with at least one, quads in total, and
    // the batches that are four vertices lying flat but are *not* the shape —
    // the third number is the one that says whether the detection is too strict.
    let mut with_ground = 0usize;
    let mut ground_quads = 0usize;
    let mut stacked_ground = 0usize;
    let mut ground_depth_writers = 0usize;
    let mut ground_rejects: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut flat_but_not_quads = 0usize;

    for path in &paths {
        let Ok(bytes) = assets.read(path) else {
            continue;
        };
        let Ok(m2) = M2::parse(&bytes) else {
            continue;
        };
        read += 1;
        for b in &m2.batches {
            if let Some(slot) = batches_by_blend.get_mut(b.blend as usize) {
                *slot += 1;
            }
            if b.uv.is_some() {
                uv_batches += 1;
            }
        }
        if m2.batches.iter().any(|b| b.uv.is_some()) {
            with_uv += 1;
        }
        let mut ground_here = 0usize;
        let mut heights_here: Vec<f32> = Vec::new();
        for b in &m2.batches {
            match m2.ground_quad_checked(b) {
                Ok(q) => {
                    ground_here += 1;
                    heights_here.push(q.corners[0][2]);
                    // **Whether any of them writes depth**, which is what
                    // decides whether stacking them matters at all: an
                    // additive batch that does not write depth adds wherever it
                    // lands, so two quads draped onto the same surface compose
                    // rather than fight.
                    if !b.no_depth_write {
                        ground_depth_writers += 1;
                    }
                }
                // Not even the right size — every ordinary mesh in the game.
                Err(vale_assets::world::m2::NotGround::NotTwoTriangles) => {}
                Err(why) => {
                    // Bucketed by *kind*: `why_not` carries the measurement,
                    // which is what a single model wants and what a histogram
                    // over 629 of them must not have.
                    let bucket = match why {
                        vale_assets::world::m2::NotGround::NotFlat(_) => {
                            "not flat".to_string()
                        }
                        other => why_not(&other),
                    };
                    *ground_rejects.entry(bucket).or_insert(0usize) += 1;
                    // Flat by geometry but rejected by the shape test — a
                    // triangle fan, a non-rectangle, a multi-bone skin. These
                    // stay on the ordinary path deliberately; the count is what
                    // says how many.
                    let range = b.index_start as usize..(b.index_start + b.index_count) as usize;
                    if let Some(indices) = m2.indices.get(range) {
                        let flat = !indices.is_empty()
                            && indices.iter().all(|&i| {
                                m2.positions
                                    .get(usize::from(i))
                                    .is_some_and(|p| p[2].abs() <= 0.01)
                            });
                        if flat {
                            flat_but_not_quads += 1;
                        }
                    }
                }
            }
        }
        if ground_here > 0 {
            with_ground += 1;
            ground_quads += ground_here;
            // **How many models stack their floor quads at different heights**,
            // which is the one thing the widened flatness test could cost: a
            // projector drapes every one of them onto the same surface, so two
            // quads a yard apart in the file land coplanar and fight. Zero is
            // the number that says the widening is free; anything else is a
            // population to look at before trusting it.
            let lo = heights_here.iter().cloned().fold(f32::MAX, f32::min);
            let hi = heights_here.iter().cloned().fold(f32::MIN, f32::max);
            if hi - lo > 0.05 {
                stacked_ground += 1;
            }
        }
        if m2
            .skeleton
            .as_ref()
            .is_some_and(|s| s.bones.iter().any(|b| Billboard::of(b.flags).is_some()))
        {
            billboarded += 1;
        }
        if let Some(s) = &m2.skeleton {
            let mut here: BTreeSet<u32> = BTreeSet::new();
            for bone in &s.bones {
                for bit in 0..32 {
                    let flag = 1u32 << bit;
                    if bone.flags & flag != 0 {
                        bone_flags.entry(flag).or_default().0 += 1;
                        here.insert(flag);
                    }
                }
                // The client's switch matches `flags & 0x78` **exactly**, so a
                // bone carrying two billboard bits is not billboarded at all.
                // This is the count that says whether that branch is reachable.
                if bone.flags & bone_flags_mask::BILLBOARD_ANY != 0
                    && Billboard::of(bone.flags).is_none()
                {
                    billboard_combinations += 1;
                }
            }
            for flag in here {
                bone_flags.entry(flag).or_default().1 += 1;
            }
            // **How many of these models are a three-act effect** — `Stand`
            // (the birth, one-shot), `Hold` (158, the loop while the state
            // lasts) and `Decay` (159, the exit). An attached model here plays
            // its `Stand` and holds the last frame, so anything counted below
            // the first line is an effect whose loop is never reached: a Mana
            // Shield bubble that comes up and then stops pulsing. What the
            // reference does about it is **not** established, so
            // this is the size of the question rather than a fault count.
            let ids: BTreeSet<u16> = s.sequences.iter().map(|q| q.id).collect();
            let has = |id: u16| ids.contains(&id);
            let tag = match (has(158), has(159)) {
                (true, true) => "birth, hold and decay",
                (true, false) => "a hold and no decay",
                (false, true) => "a decay and no hold",
                (false, false) => "one act only",
            };
            *sequence_idiom.entry(tag).or_default() += 1;
        }
        if !m2.ribbons.is_empty() {
            with_ribbons += 1;
        }
        for r in &m2.ribbons {
            ribbons += 1;
            if let Some(slot) = ribbon_blends.get_mut(r.blend as usize) {
                *slot += 1;
            }
            if r.texture.is_none() {
                ribbon_no_texture += 1;
            }
            // **Gated *off* somewhere**, not merely keyed. Almost every ribbon
            // in the game carries a visibility track and almost every one of
            // them is a single key saying "on"; what is worth counting is the
            // trails that are dark in some sequence, because those are the
            // ones a spawn site that ignored the track would draw wrongly —
            // the thrown dagger's flight trail hanging off it in the hand.
            if r.visibility
                .as_ref()
                .is_some_and(|t| t.values.iter().any(|&v| v <= 0.5))
            {
                ribbon_gated += 1;
            }
            let keyed = r.height_above.as_ref().is_some_and(|t| t.times.len() > 1);
            if keyed {
                ribbon_keyed_height += 1;
                // The `values[0]` trap: a slash authored from zero. Counted
                // because it is the one that draws *nothing* when baked.
                if r.height_above
                    .as_ref()
                    .and_then(|t| t.values.first().copied())
                    == Some(0.0)
                    && r.peak_height() > 0.0
                {
                    ribbon_zero_first_key += 1;
                }
            }
            widest.push((r.peak_height(), path.clone()));
        }
    }

    println!(
        "SpellVisualEffectName names {} models, {read} of them readable",
        paths.len()
    );
    println!("  batches by blend mode:");
    for (blend, count) in batches_by_blend.iter().enumerate() {
        if *count > 0 {
            println!("    {blend} {:<10} {count}", blend_name(blend as u16));
        }
    }
    println!("  {billboarded} models carry a billboarded bone");
    println!("  bone flags over the whole population (bones / models):");
    for (flag, (bones, models)) in &bone_flags {
        println!(
            "    {flag:#06x} {:<24} {bones:6} bones in {models:4} models",
            bone_flag_name(*flag)
        );
    }
    println!("    {billboard_combinations} bones carry two billboard bits (the client's switch refuses those)");
    println!("  of the models with a skeleton, how many acts each one has:");
    for (idiom, count) in &sequence_idiom {
        println!("    {count:4} with {idiom}");
    }
    println!(
        "  {with_ground} models author {ground_quads} flat ground quads \
         ({flat_but_not_quads} other flat batches stay on the ordinary path)"
    );
    println!(
        "    {stacked_ground} of those stack theirs more than 0.05y apart, and {ground_depth_writers} of the {ground_quads} write depth — \n         a projector drapes a stack onto one surface, which composes for an additive batch and fights for an opaque one"
    );
    if !ground_rejects.is_empty() {
        println!("    four-corner batches the shape test refuses, and why:");
        let mut rows: Vec<(&String, &usize)> = ground_rejects.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1));
        for (why, count) in rows {
            println!("      {count:5}  {why}");
        }
    }
    println!("  {with_uv} models put a texture matrix on {uv_batches} batches");
    println!("  {with_ribbons} models carry {ribbons} ribbon emitters");
    if ribbons > 0 {
        println!(
            "    {ribbon_keyed_height} have a keyed height, {ribbon_zero_first_key} of those start at zero \
             — the ones a values[0] bake would silently never draw"
        );
        println!("    {ribbon_gated} carry a per-sequence visibility gate; {ribbon_no_texture} name no texture");
        let blends: Vec<String> = ribbon_blends
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(b, c)| format!("{c} {}", blend_name(b as u16)))
            .collect();
        println!("    blends: {}", blends.join(", "));
        widest.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("    widest trails:");
        for (width, path) in widest.iter().take(5) {
            println!("      {width:.2} y  {path}");
        }
    }
    Ok(())
}
