//! Particle emitters: what the models ask to spray, and whether the record
//! layout holds against the whole population.
//!
//! The counterpart of `vale anim` for emitters, and the check is the same
//! shape: a wrongly read 0x1F8-byte record does not fail — it produces
//! plausible numbers — so the survey reads every model two populations can
//! name and measures the parse against things that did not come from this
//! client. Three of them:
//!
//! * **the spell-effect models exist to be particles.** `SpellVisualEffectName`
//!   names 782 models whose whole job is glows and sprays; a wrong stride
//!   validates their tables away to nothing, so the share of them carrying
//!   emitters is the layout check.
//! * **every emitter names a texture inside its own model's table**, and the
//!   type-0 ones name files that must exist in the archive — the same
//!   cross-check `vale npc` runs on skins.
//! * **the values are physical.** Lifespans in seconds, rates per second,
//!   scales in yards: a shifted field reads as a lifespan of 10^30.

use crate::common::*;
use vale_config::Config;
use vale_assets::world::m2::{particle_flags, M2Particle, M2Track, M2};
use std::collections::{BTreeMap, BTreeSet};

/// The first key of an emitter property track, or the fallback the client
/// uses when the track has no keys.
fn first(track: &Option<M2Track>, fallback: f32) -> f32 {
    track.as_ref().map(|t| t.first()).unwrap_or(fallback)
}

pub fn cmd_particles(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;

    let mut assets = open_assets(cfg)?;

    if let Some(target) = target {
        // A display id or a model path, exactly as `vale anim` takes one.
        let path = match target.parse::<u32>() {
            Ok(display_id) => {
                let tables = open_display_tables(&mut assets)?;
                tables
                    .creature(display_id)
                    .ok_or_else(|| format!("display id {display_id} is not in CreatureDisplayInfo"))?
                    .path
            }
            Err(_) => vale_assets::world::m2::model_path(target),
        };
        let bytes = assets.read(&path).map_err(|e| e.to_string())?;
        let m2 = M2::parse(&bytes).map_err(|e| e.to_string())?;
        println!("{path}");
        if m2.particles.is_empty() {
            println!("  no particle emitters (or the table did not validate)");
            return Ok(());
        }
        for (i, e) in m2.particles.iter().enumerate() {
            trace_emitter(&mut assets, &m2, i, e);
        }
        trace_jets(&m2);
        return Ok(());
    }

    // The survey: every model the two populations name.
    let paths_of = |assets: &mut vale_assets::Assets, table: &str, field: usize| {
        let bytes = assets
            .read(&dbc_path(table))
            .map_err(|e| format!("{table}.dbc: {e}"))?;
        let dbc = vale_assets::Dbc::parse(&bytes).map_err(|e| e.to_string())?;
        Ok::<BTreeSet<String>, String>(
            (0..dbc.record_count)
                .filter_map(|r| dbc.string_at(r, field))
                .filter(|p| !p.is_empty())
                .map(|p| vale_assets::world::m2::model_path(&p))
                .collect(),
        )
    };
    let spells = paths_of(&mut assets, "SpellVisualEffectName", 2)?;
    let creatures = paths_of(&mut assets, "CreatureModelData", 2)?;

    let mut all_emitters: Vec<(String, M2Particle, usize)> = Vec::new();
    let mut texture_missing = 0usize;
    let mut texture_unresolved = 0usize;
    let mut client_supplied = 0usize;

    for (label, paths) in [("spell effects", &spells), ("creatures", &creatures)] {
        let mut read = 0usize;
        let mut unreadable = 0usize;
        let mut with = 0usize;
        let mut emitters = 0usize;
        // **The other thing these two populations animate**, and it is not
        // emitters: a texture matrix slides a batch's UVs across geometry that
        // does not move. A swirl, a beam, a rune circle and a waterfall are all
        // one of these, so the share here is what says whether skipping the
        // block costs a handful of models or a whole visual vocabulary.
        let mut with_uv = 0usize;
        let mut uv_batches = 0usize;
        let mut uv_of = 0usize;
        for path in paths.iter() {
            // The two populations overlap on nothing in practice, but a model
            // in both would be counted per population and measured once.
            let Ok(bytes) = assets.read(path) else {
                unreadable += 1;
                continue;
            };
            let Ok(m2) = M2::parse(&bytes) else {
                unreadable += 1;
                continue;
            };
            read += 1;
            uv_of += m2.batches.len();
            uv_batches += m2.batches.iter().filter(|b| b.uv.is_some()).count();
            if !m2.uv_anims.is_empty() {
                with_uv += 1;
            }
            if m2.particles.is_empty() {
                continue;
            }
            with += 1;
            emitters += m2.particles.len();
            for e in &m2.particles {
                match m2.textures.get(e.texture as usize) {
                    None => texture_unresolved += 1,
                    Some(t) if t.kind == 0 => {
                        if !t.file_name.is_empty() && assets.exists(&t.file_name) {
                            // resolved
                        } else {
                            texture_missing += 1;
                        }
                    }
                    // A client-supplied slot (a creature skin) — legal, rare.
                    Some(_) => client_supplied += 1,
                }
                all_emitters.push((path.clone(), e.clone(), m2.textures.len()));
            }
        }
        println!(
            "{label}: {} distinct models, {read} read, {unreadable} unreadable; \
             {with} carry emitters, {emitters} emitters in all",
            paths.len()
        );
        println!(
            "  …and {with_uv} state a texture matrix, moving {uv_batches} of {uv_of} batches"
        );
    }

    // The layout check: models that exist to be particles carry them.
    let spell_with = all_emitters
        .iter()
        .filter(|(p, _, _)| spells.contains(p))
        .map(|(p, _, _)| p)
        .collect::<BTreeSet<_>>()
        .len();
    println!(
        "\nthe layout check: {spell_with} of {} readable spell-effect models carry emitters \
         (a wrong stride validates these tables away to nothing)",
        spells.len()
    );

    // Shape of the population.
    let mut by_type: BTreeMap<u16, usize> = BTreeMap::new();
    let mut by_blend: BTreeMap<u16, usize> = BTreeMap::new();
    let mut by_head_tail: BTreeMap<u16, usize> = BTreeMap::new();
    let mut atlases = 0usize;
    let mut keyed_rates = 0usize;
    let mut gated = 0usize;
    let mut boned = 0usize;
    for (_, e, _) in &all_emitters {
        *by_type.entry(e.emitter_type).or_default() += 1;
        *by_blend.entry(e.blend).or_default() += 1;
        *by_head_tail.entry(e.head_tail).or_default() += 1;
        if e.rows * e.columns > 1 {
            atlases += 1;
        }
        if e.emission_rate.as_ref().is_some_and(|t| t.times.len() > 1) {
            keyed_rates += 1;
        }
        if e.enabled.is_some() {
            gated += 1;
        }
        if e.bone != 0 {
            boned += 1;
        }
    }
    let show = |name: &str, map: &BTreeMap<u16, usize>| {
        let parts: Vec<String> = map.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        println!("  {name}: {}", parts.join(", "));
    };
    println!("\nover all {} emitters:", all_emitters.len());
    show("emitter type (1 plane, 2 sphere, 3 spline)", &by_type);
    show("blend (the material table)", &by_blend);
    show("head/tail (0 head, 1 tail, 2 both)", &by_head_tail);
    println!(
        "  {atlases} use a texture atlas, {keyed_rates} animate their rate, \
         {gated} carry an enable gate, {boned} ride a bone other than 0"
    );

    let named_flags: [(&str, u32); 12] = [
        ("fogged", particle_flags::FOGGED),
        ("model-space", particle_flags::MODEL_SPACE),
        ("scale-by-instance", particle_flags::SCALE_BY_INSTANCE),
        ("inherit-velocity", particle_flags::INHERIT_VELOCITY),
        ("kill-outbound", particle_flags::KILL_OUTBOUND),
        ("sphere-up", particle_flags::SPHERE_UP),
        ("tumble-sign", particle_flags::TUMBLE_RANDOM_SIGN),
        ("tail-grows", particle_flags::TAIL_GROWS),
        ("xy-quad", particle_flags::XY_QUAD),
        ("ground-snap", particle_flags::GROUND_SNAP),
        ("follow-emitter", particle_flags::FOLLOW_EMITTER),
        ("burst", particle_flags::BURST),
    ];
    let flag_counts: Vec<String> = named_flags
        .iter()
        .map(|(name, bit)| {
            let n = all_emitters.iter().filter(|(_, e, _)| e.flags & bit != 0).count();
            format!("{name} {n}")
        })
        .collect();
    println!("  flags: {}", flag_counts.join(", "));

    // The physical check. A shifted field does not read as garbage, it reads
    // as a lifespan of 1e30 — so the extremes are printed, named.
    let mut worst_life = (0.0f32, String::new());
    let mut worst_rate = (0.0f32, String::new());
    let mut worst_scale = (0.0f32, String::new());
    for (path, e, _) in &all_emitters {
        let life = first(&e.lifespan, 0.0);
        let rate = first(&e.emission_rate, 0.0);
        let scale = e.scales.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        if life > worst_life.0 {
            worst_life = (life, path.clone());
        }
        if rate > worst_rate.0 {
            worst_rate = (rate, path.clone());
        }
        if scale > worst_scale.0 {
            worst_scale = (scale, path.clone());
        }
    }
    println!(
        "\nthe physical check — the extremes over every emitter:\n\
         \x20 longest lifespan {:.1} s ({})\n\
         \x20 highest rate {:.0}/s ({})\n\
         \x20 largest scale {:.1} y ({})",
        worst_life.0, worst_life.1, worst_rate.0, worst_rate.1, worst_scale.0, worst_scale.1
    );

    println!(
        "\nthe texture check: {texture_missing} emitters name a file the archive lacks, \
         {texture_unresolved} an index outside their model's table, \
         {client_supplied} a client-supplied slot"
    );

    // **The geometry-model check**, and it is the one that decides whether a
    // family of spells is a cloud or a slab. An emitter naming a model at
    // +0x18 does not draw a billboard at all — each of its particles is an
    // instance of that file — and its own `texture` slot is never read. Cone
    // of Cold's eight cloud emitters and Evocation's five are *all* of this
    // population, which is why no blend, alpha or size rule ever fixed them.
    let mut geometry: BTreeMap<String, usize> = BTreeMap::new();
    let mut recursion: BTreeMap<String, usize> = BTreeMap::new();
    for (_, e, _) in &all_emitters {
        if let Some(model) = &e.geometry_model {
            *geometry.entry(model.clone()).or_default() += 1;
        }
        if let Some(model) = &e.recursion_model {
            *recursion.entry(model.clone()).or_default() += 1;
        }
    }
    let geometry_emitters: usize = geometry.values().sum();
    let geometry_missing = geometry.keys().filter(|m| !assets.exists(m)).count();
    println!(
        "\nthe geometry-model check: {geometry_emitters} emitters draw a model rather than a \
         quad, naming {} distinct files, {geometry_missing} of which the archive lacks\n\
         \x20 …and {} name a recursion model (child emitters), {} distinct",
        geometry.len(),
        recursion.values().sum::<usize>(),
        recursion.len()
    );
    Ok(())
}

fn trace_emitter(
    assets: &mut vale_assets::Assets,
    m2: &M2,
    i: usize,
    e: &M2Particle,
) {
    let kind = match e.emitter_type {
        1 => "plane",
        2 => "sphere",
        3 => "spline",
        _ => "?",
    };
    let ht = match e.head_tail {
        0 => "head",
        1 => "tail",
        2 => "head+tail",
        _ => "?",
    };
    let texture = match m2.textures.get(e.texture as usize) {
        Some(t) if t.kind == 0 => {
            let status = if assets.exists(&t.file_name) { "ok" } else { "MISSING" };
            format!("{} ({status})", t.file_name)
        }
        Some(t) => format!("client-supplied type {}", t.kind),
        None => format!("index {} out of range", e.texture),
    };
    println!(
        "  [{i}] {kind}, {ht}, blend {}, bone {}, at [{:.2}, {:.2}, {:.2}]",
        e.blend, e.bone, e.position[0], e.position[1], e.position[2]
    );
    // A geometry model replaces the quad outright, so it is printed *instead*
    // of the texture line: naming both would read as "a textured billboard,
    // and also a model", which is exactly the misreading this field cost.
    match &e.geometry_model {
        Some(model) => {
            let status = if assets.exists(model) { "ok" } else { "MISSING" };
            println!(
                "      MODEL particles: {model} ({status})  \
                 — the quad and its texture ({texture}) are not drawn"
            );
            if let Some(child) = &e.recursion_model {
                println!("      recursion model: {child} (child emitters, not implemented)");
            }
            let tumble = e.tumble_max.iter().zip(e.tumble_min).any(|(a, b)| *a != b);
            println!(
                "      tumble {:?}..{:?}{}",
                e.tumble_min,
                e.tumble_max,
                if tumble { "" } else { " (still)" }
            );
        }
        None => println!("      texture: {texture}  atlas {}x{}", e.rows, e.columns),
    }
    println!(
        "      rate {:.1}/s (keys {}), life {:.2} s, speed {:.2} ±{:.0}%, gravity {:.2}, \
         drag {:.2}, spin {:.2}",
        first(&e.emission_rate, 0.0),
        e.emission_rate.as_ref().map_or(0, |t| t.times.len()),
        first(&e.lifespan, 0.0),
        first(&e.emission_speed, 0.0),
        first(&e.speed_variation, 0.0) * 100.0,
        first(&e.gravity, 0.0),
        e.drag,
        e.spin,
    );
    println!(
        "      area {:.2} x {:.2}, vertical {:.2}, horizontal {:.2}, z-source {:.2}, \
         tail {:.2} s",
        first(&e.area_length, 0.0),
        first(&e.area_width, 0.0),
        first(&e.vertical_range, 0.0),
        first(&e.horizontal_range, 0.0),
        first(&e.z_source, 0.0),
        e.tail_time,
    );
    println!(
        "      over life (mid {:.2}): colour {:?} -> {:?} -> {:?}, scale {:.2} -> {:.2} -> {:.2}, \
         cells {:?}",
        e.mid_point,
        e.colors[0],
        e.colors[1],
        e.colors[2],
        e.scales[0],
        e.scales[1],
        e.scales[2],
        e.cells,
    );
    let mut named = Vec::new();
    for (name, bit) in [
        ("fogged", particle_flags::FOGGED),
        ("model-space", particle_flags::MODEL_SPACE),
        ("scale-by-instance", particle_flags::SCALE_BY_INSTANCE),
        ("inherit-velocity", particle_flags::INHERIT_VELOCITY),
        ("kill-outbound", particle_flags::KILL_OUTBOUND),
        ("sphere-up", particle_flags::SPHERE_UP),
        ("tumble-sign", particle_flags::TUMBLE_RANDOM_SIGN),
        ("tail-grows", particle_flags::TAIL_GROWS),
        ("xy-quad", particle_flags::XY_QUAD),
        ("ground-snap", particle_flags::GROUND_SNAP),
        ("follow-emitter", particle_flags::FOLLOW_EMITTER),
        ("burst", particle_flags::BURST),
    ] {
        if e.flags & bit != 0 {
            named.push(name);
        }
    }
    println!(
        "      flags 0x{:X} [{}]{}",
        e.flags,
        named.join(", "),
        if e.enabled.is_some() { "  gated by animation" } else { "" }
    );
}

/// **Where each emitter stands and which way it sprays, through its own clip.**
///
/// The half of a cloud that no field of the record states. Every kernel's
/// direction is a cone about the emitter's local `+Z`, so what a spell's spray
/// actually *does* is the product of that constant and the emitter bone's own
/// animation — and a bone that is not posed, or is posed in the wrong frame,
/// produces a jet that is the right speed, the right size and the right colour
/// and points somewhere absurd. Nothing else in this CLI can see that: `vale
/// particles` prints the record and `vale anim` measures the pose against the
/// *vertices*, which an emitter-only model has none of.
///
/// Printed in **model space** (WoW axes: +X forward, +Y left, +Z up), so a spell
/// thrown from the hand should read as a `+X` jet and one poured on the floor as
/// `-Z`.
fn trace_jets(m2: &M2) {
    let Some(skeleton) = &m2.skeleton else {
        println!("  no skeleton: every emitter sprays along the model's own +Z");
        return;
    };
    let Some(sequence) = skeleton.find_sequence(0).or(Some(0)) else {
        return;
    };
    let Some(window) = skeleton.sequences.get(sequence) else {
        return;
    };
    let length = window.end.saturating_sub(window.start).max(1);
    println!("  where each emitter sprays, model space (+X forward, +Y left, +Z up):");
    for (i, e) in m2.particles.iter().enumerate() {
        let mut line = format!("    [{i}] bone {:2}", e.bone);
        for step in 0..=4 {
            let at = length * step / 4;
            // The **two** clocks a bone track can run on: the sequence's own, and
            // the free-running wall clock a global sequence uses. Advanced
            // together, because a bone whose rotation is on a global sequence is
            // motionless against the first one alone.
            let pose = skeleton.pose(sequence, at, at, None, Default::default());
            let Some(bone) = pose.get(e.bone as usize) else {
                continue;
            };
            let apply = |v: [f32; 3], w: f32| {
                [
                    bone[0] * v[0] + bone[1] * v[1] + bone[2] * v[2] + bone[3] * w,
                    bone[4] * v[0] + bone[5] * v[1] + bone[6] * v[2] + bone[7] * w,
                    bone[8] * v[0] + bone[9] * v[1] + bone[10] * v[2] + bone[11] * w,
                ]
            };
            let at_pos = apply(e.position, 1.0);
            // The kernels' own axis, and the sign the emission speed gives it.
            let speed = e
                .emission_speed
                .as_ref()
                .and_then(|t| t.values.first().copied())
                .unwrap_or(0.0);
            let d = apply([0.0, 0.0, 1.0], 0.0);
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
            let sign = if speed < 0.0 { -1.0 } else { 1.0 };
            line += &format!(
                "  {}ms at [{:.2},{:.2},{:.2}] -> [{:+.2},{:+.2},{:+.2}]",
                at,
                at_pos[0],
                at_pos[1],
                at_pos[2],
                sign * d[0] / len,
                sign * d[1] / len,
                sign * d[2] / len,
            );
        }
        println!("{line}");
    }
}
