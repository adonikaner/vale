//! What the mouse can hit: the two volumes a click is tested against.
//!
//! The measurement this exists for is the one that cannot be made any other way.
//! `vale_assets::look::pick` claims a byte layout — that each of a model's
//! animations carries its own bounding box at `+0x24` of its 68-byte record and
//! its own sphere at `+0x3C` — and a wrong offset there does not fail: it reads
//! four plausible floats out of the neighbouring fields and produces a sphere
//! somewhere near the model, which is exactly the failure mode that a picture
//! cannot show and a count can.
//!
//! So the survey is **self-checking**: pose each sequence, skin every vertex, and
//! ask whether the vertices of *that* animation stay inside the box *that*
//! animation declares. Nothing here comes from this client — the box is the
//! file's and the vertices are the file's — and a misread offset escapes it by
//! yards. It is `vale anim`'s check aimed one field over.
//!
//! The second number is the reason the round happened: how much **smaller** the
//! per-sequence sphere is than the model's own header sphere, which is what the
//! pick used to be built on.

use crate::common::*;
use vale_assets::world::m2::{M2, M2Skeleton, PoseLayers};
use vale_assets::look::pick::{PickMesh, hit_mesh, model_sphere, ray_sphere, sequence_sphere};
use vale_config::Config;

/// How many phases of a clip to sample. The same eight `vale anim` uses, for
/// the same reason: a box is authored to cover the whole window and a single
/// frame would pass on a box half the right size.
const PHASES: u32 = 8;

pub fn cmd_pick(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    match target {
        Some(target) => one(cfg, target),
        None => survey(cfg),
    }
}

/// One model in full: every sequence's own sphere, checked, and a ray fired at
/// the standing pose to show what the narrow phase does to it.
fn one(cfg: &Config, target: &str) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
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
    let header = model_sphere(&m2);
    println!(
        "  header box   [{:.2}, {:.2}, {:.2}] .. [{:.2}, {:.2}, {:.2}]  \
         centre [{:.2}, {:.2}, {:.2}]  radius {:.2}",
        m2.bounds[0][0],
        m2.bounds[0][1],
        m2.bounds[0][2],
        m2.bounds[1][0],
        m2.bounds[1][1],
        m2.bounds[1][2],
        header.centre[0],
        header.centre[1],
        header.centre[2],
        header.radius,
    );
    let mesh = PickMesh::of(&m2);
    println!(
        "  pick mesh    {} vertices, {} triangles",
        mesh.positions.len(),
        mesh.indices.len() / 3
    );

    let Some(sk) = m2.skeleton.as_ref() else {
        println!("  no skeleton: the header sphere is the whole broad phase");
        return Ok(());
    };
    println!("  sequences — each one's own sphere, and whether its own pose stays inside it:");
    for (i, seq) in sk.sequences.iter().enumerate() {
        let sphere = sequence_sphere(header, Some(seq));
        let (escape, _) = escapes(&m2, sk, i);
        println!(
            "    [{i:>2}] id {:>3}  box [{:>6.2},{:>6.2},{:>6.2}]..[{:>6.2},{:>6.2},{:>6.2}]  \
             r {:>6.2}{}  escapes {:.2}y",
            seq.id,
            seq.bounds[0][0],
            seq.bounds[0][1],
            seq.bounds[0][2],
            seq.bounds[1][0],
            seq.bounds[1][1],
            seq.bounds[1][2],
            sphere.radius,
            if seq.radius > 0.0 { " " } else { "*" },
            escape,
        );
    }
    println!("    (* = the sequence states no radius, so the header's is used)");

    // **The narrow phase, drawn as a column of hits and misses.** A ray fired
    // due north at the standing model from ten yards out, at a ladder of
    // heights: what the sphere alone would take, against what the triangles
    // take. The gap between the two columns is the whole point of stage two.
    let stand = sk.find_sequence(0).unwrap_or(0);
    let pose = sk.pose(stand, 0, 0, None, PoseLayers::default());
    let sphere = sequence_sphere(header, sk.sequences.get(stand));
    println!("  a ray from ten yards south, at each height — sphere against triangles:");
    let mut sphere_hits = 0usize;
    let mut mesh_hits = 0usize;
    for step in 0u8..=16 {
        let z = f32::from(step) * 0.25;
        let origin = [-10.0, 0.0, z];
        let north = [1.0, 0.0, 0.0];
        let broad = ray_sphere(origin, north, 100.0, sphere.centre, sphere.radius);
        let narrow = broad.and_then(|_| hit_mesh(&mesh, |v| mesh.skinned(v, &pose), origin, north));
        sphere_hits += usize::from(broad.is_some());
        mesh_hits += usize::from(narrow.is_some());
        println!(
            "    z {z:>5.2}  sphere {}  triangles {}",
            broad.map_or("  —  ".to_string(), |t| format!("{t:>5.2}")),
            narrow.map_or("  —  ".to_string(), |t| format!("{t:>5.2}")),
        );
    }
    println!("    {sphere_hits} of 17 heights hit the sphere, {mesh_hits} hit the model");
    Ok(())
}

/// The whole bestiary: does the layout hold, and how much does stage one
/// actually save?
fn survey(cfg: &Config) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use std::collections::BTreeSet;

    let mut assets = open_assets(cfg)?;
    let table = assets
        .read(&dbc_path("CreatureModelData"))
        .map_err(|e| format!("CreatureModelData.dbc: {e}"))?;
    let dbc = vale_assets::Dbc::parse(&table).map_err(|e| e.to_string())?;
    let paths: BTreeSet<String> = (0..dbc.record_count)
        .filter_map(|r| dbc.string_at(r, 2))
        .filter(|p| !p.is_empty())
        .map(|p| vale_assets::world::m2::model_path(&p))
        .collect();
    println!(
        "CreatureModelData: {} distinct models; checking every sequence's own sphere",
        paths.len()
    );

    let mut decoded = 0usize;
    let mut animated = 0usize;
    let mut sequences = 0usize;
    let mut with_radius = 0usize;
    // The structural check — see `union_overrun`.
    let mut union_checked = 0usize;
    let mut union_clean = 0usize;
    let mut union_worst = (0.0f32, String::new());
    // …and the pose check, against its own control.
    let mut posed = 0usize;
    let mut worse_than_the_header = 0usize;
    let mut worst = (0.0f32, String::new());
    // The saving: the header sphere against the standing sphere, per model.
    let mut ratio_total = 0.0f64;
    let mut ratio_count = 0usize;
    let mut biggest_saving = (1.0f32, String::new());
    let mut triangles = 0usize;

    for path in &paths {
        let Ok(bytes) = assets.read(path) else { continue };
        let Ok(m2) = M2::parse(&bytes) else { continue };
        decoded += 1;
        triangles += m2.indices.len() / 3;
        let Some(sk) = m2.skeleton.as_ref() else {
            continue;
        };
        animated += 1;
        for seq in &sk.sequences {
            sequences += 1;
            with_radius += usize::from(seq.radius > 0.0);
        }

        // **The check that pins the offset**, and it touches no pose maths at
        // all: every sequence box has to sit inside the header box.
        if let Some(over) = union_overrun(&m2, sk) {
            union_checked += 1;
            union_clean += usize::from(over <= 0.01);
            if over > union_worst.0 {
                union_worst = (over, path.clone());
            }
        }

        // **The pose check, against its own control.** A sequence's box is
        // tighter than the header's, so a model whose *pose* already leaves the
        // header box leaves the tighter one by at least as much — and 6,320 of
        // 10,511 sequences do, which says nothing about this field. What is
        // worth counting is the sequences that do materially *worse* against
        // their own box than the same pose does against the model's.
        for (i, seq) in sk.sequences.iter().enumerate() {
            if seq.radius <= 0.0 || seq.end <= seq.start {
                continue;
            }
            posed += 1;
            let (own, header) = escapes(&m2, sk, i);
            let excess = own - header;
            if excess > 0.25 {
                worse_than_the_header += 1;
            }
            if excess > worst.0 {
                worst = (excess, format!("{path} seq {i} (id {})", seq.id));
            }
        }

        // The standing clip is the one a unit is in whenever anybody is
        // pointing at it, so it is the honest one to price the saving on.
        let stand = sk.find_sequence(0).unwrap_or(0);
        let sphere = sequence_sphere(model_sphere(&m2), sk.sequences.get(stand));
        if m2.bounding_radius > 0.01 && sphere.radius > 0.0 {
            let ratio = sphere.radius / m2.bounding_radius;
            ratio_total += f64::from(ratio);
            ratio_count += 1;
            if ratio < biggest_saving.0 {
                biggest_saving = (ratio, path.clone());
            }
        }
    }

    println!("  {decoded} decoded, {animated} with a skeleton, {sequences} sequences");
    println!(
        "  {with_radius} of {sequences} sequences state their own radius; \
         the rest fall back to the model's"
    );
    println!(
        "  {union_clean} of {union_checked} models have every sequence box inside the header box\
         {}",
        if union_clean == union_checked {
            " — the offset holds"
        } else {
            ""
        }
    );
    if !union_worst.1.is_empty() {
        println!("    worst overrun: {:.2}y — {}", union_worst.0, union_worst.1);
    }
    println!(
        "  {worse_than_the_header} of {posed} posed sequences leave their own box by more than \
         0.25y beyond what the same pose already does to the header box"
    );
    if !worst.1.is_empty() {
        println!("    worst excess: {:.2}y — {}", worst.0, worst.1);
    }
    if ratio_count > 0 {
        println!(
            "  the standing sphere is on average {:.0}% of the header sphere over {ratio_count} \
             models; tightest {:.0}% — {}",
            100.0 * ratio_total / ratio_count as f64,
            100.0 * biggest_saving.0,
            biggest_saving.1,
        );
    }
    println!("  {triangles} triangles across the bestiary, which is what stage two walks");
    Ok(())
}

/// How far the **union of every sequence box** sticks out of the model's own
/// header box, or `None` for a model that states neither.
///
/// **This is the check that pins the field offset, and it involves no pose maths
/// at all** — the header box is a union over every sequence *and* every emitter
/// the file has, so a correctly-read sequence box is a subset of it, exactly,
/// on every model. Four floats read out of the neighbouring fields would not be:
/// a sequence record's `+0x24` is bracketed by `blend_time` and the sphere, so a
/// stride or an offset off by anything at all reads timestamps and radii as
/// coordinates and lands nowhere near.
fn union_overrun(m2: &M2, sk: &M2Skeleton) -> Option<f32> {
    let (lo, hi) = (m2.bounds[0], m2.bounds[1]);
    if (0..3).all(|a| (hi[a] - lo[a]).abs() < 1e-6) {
        return None;
    }
    let mut worst: Option<f32> = None;
    for seq in &sk.sequences {
        // A box with no volume states nothing — see `escapes`.
        if (0..3).all(|a| (seq.bounds[1][a] - seq.bounds[0][a]).abs() < 1e-6) {
            continue;
        }
        let mut over = 0.0f32;
        for a in 0..3 {
            over = over
                .max(lo[a] - seq.bounds[0][a])
                .max(seq.bounds[1][a] - hi[a]);
        }
        worst = Some(worst.map_or(over, |w: f32| w.max(over)));
    }
    worst
}

/// How far one sequence's posed vertices leave **its own** box, and how far the
/// same vertices leave the **model's** box — the measurement and its control, in
/// one walk.
///
/// The control is the whole reason both numbers come back together. `vale
/// anim` already reports that plenty of models' poses escape the header box —
/// this client's `pose` is an approximation of the reference's in several
/// documented ways — so a sequence box being escaped is not evidence of
/// anything on its own. What is evidence is the *difference*: a box read out of
/// the wrong four floats is escaped by yards more than the union it sits inside.
fn escapes(m2: &M2, sk: &M2Skeleton, sequence: usize) -> (f32, f32) {
    let Some(seq) = sk.sequences.get(sequence) else {
        return (0.0, 0.0);
    };
    let (lo, hi) = (seq.bounds[0], seq.bounds[1]);
    // A box with no volume states nothing; treat it as no claim rather than as
    // a claim every vertex escapes.
    if (0..3).all(|a| (hi[a] - lo[a]).abs() < 1e-6) {
        return (0.0, 0.0);
    }
    let duration = (seq.end - seq.start).max(1);
    let (mut own, mut header) = (0.0f32, 0.0f32);
    for phase in 0..PHASES {
        let pose = sk.pose(sequence, duration * phase / PHASES, 0, None, PoseLayers::default());
        for v in 0..m2.positions.len() {
            let p = m2.skin_position(v, &pose);
            for axis in 0..3 {
                if !p[axis].is_finite() {
                    return (f32::INFINITY, f32::INFINITY);
                }
                own = own.max((lo[axis] - p[axis]).max(p[axis] - hi[axis]));
                header = header
                    .max((m2.bounds[0][axis] - p[axis]).max(p[axis] - m2.bounds[1][axis]));
            }
        }
    }
    (own, header)
}
