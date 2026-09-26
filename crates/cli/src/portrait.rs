//! `vale portrait` — where the game says to stand to take a unit's picture.
//!
//! The check behind `SetPortraitTexture`, and it exists because the alternative
//! was to invent a framing. A portrait drawn from the wrong camera is a picture
//! of a chest or a pair of knees: it renders perfectly and is wrong, which is
//! exactly the class of rendering bug this project keeps a rule about. So the
//! framing comes out of the file (`vale_assets::look::portrait`) and this command
//! is what says the file really carries one and that it really points at a head.
//!
//! Two numbers decide it, over every model any creature display id can name:
//!
//! * **how many carry a camera of kind 0** — the portrait camera — against how
//!   many have to be framed off their own bounding box instead;
//! * **where each of those cameras aims, as a fraction of the model's height.**
//!   A head shot lands near the top. The kind-1 camera, which is what taking
//!   `cameras[0]` on trust would have used, lands near the middle — so the two
//!   populations are separable by measurement rather than by the type code's
//!   name, and this prints both so they can be compared.

use crate::common::*;
use vale_config::Config;

/// The census, or one model traced.
pub fn cmd_portrait(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::world::m2::M2;
    use vale_assets::look::portrait::{self, Source};
    use std::collections::BTreeSet;

    let mut assets = open_assets(cfg)?;

    // --- one model traced ---
    if let Some(target) = target {
        // A display id or a path, the same two forms `vale anim` takes and
        // for the same reason: an id is what a session hands you and a path is
        // what a file does.
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
        println!(
            "  declared box [{:.2}, {:.2}, {:.2}] .. [{:.2}, {:.2}, {:.2}]",
            m2.bounds[0][0], m2.bounds[0][1], m2.bounds[0][2],
            m2.bounds[1][0], m2.bounds[1][1], m2.bounds[1][2]
        );
        let bind = portrait::bind_bounds(&m2);
        println!(
            "  bind pose   [{:.2}, {:.2}, {:.2}] .. [{:.2}, {:.2}, {:.2}]   <- what a portrait sees",
            bind[0][0], bind[0][1], bind[0][2], bind[1][0], bind[1][1], bind[1][2]
        );
        // **Every camera, not only the one chosen**, because the whole point of
        // the reading is that the kinds are different framings and the
        // difference is visible in the numbers.
        if m2.cameras.is_empty() {
            println!("  no cameras at all");
        }
        for (index, camera) in m2.cameras.iter().enumerate() {
            let word = match camera.kind {
                0 => "portrait",
                1 => "character-info",
                _ => "other",
            };
            let framing = portrait::Framing {
                eye: camera.position,
                aim: camera.target,
                fov: camera.fov,
                near: camera.near_clip,
                far: camera.far_clip,
                source: Source::Camera,
            };
            let height = framing
                .aim_height(bind)
                .map_or("--".to_string(), |h| format!("{:.0}%", h * 100.0));
            println!(
                "  [{index}] kind {} ({word:14}) fov {:.3}  eye [{:.2}, {:.2}, {:.2}]  \
                 aim [{:.2}, {:.2}, {:.2}]  {:.2}y away, {height} up the body",
                camera.kind,
                camera.fov,
                camera.position[0], camera.position[1], camera.position[2],
                camera.target[0], camera.target[1], camera.target[2],
                framing.distance(),
            );
        }
        let chosen = portrait::framing(&m2);
        println!(
            "  -> {:?}: fov {:.3}, {:.2}y away, aim [{:.2}, {:.2}, {:.2}]",
            chosen.source,
            chosen.fov,
            chosen.distance(),
            chosen.aim[0], chosen.aim[1], chosen.aim[2]
        );
        return Ok(());
    }

    // --- the census ---
    //
    // The same population `vale anim` walks: every distinct model any
    // creature display id can name. That is the set a unit frame can ever be
    // asked to draw, plus the ones only a spell effect names.
    let raw = assets
        .read(&dbc_path("CreatureModelData"))
        .map_err(|e| format!("CreatureModelData.dbc: {e}"))?;
    let dbc = vale_assets::Dbc::parse(&raw).map_err(|e| e.to_string())?;
    let paths: BTreeSet<String> = (0..dbc.record_count)
        .filter_map(|r| dbc.string_at(r, 2))
        .filter(|p| !p.is_empty())
        .map(|p| vale_assets::world::m2::model_path(&p))
        .collect();
    println!(
        "CreatureModelData: {} records, {} distinct models",
        dbc.record_count,
        paths.len()
    );

    let mut decoded = 0usize;
    let mut unreadable = 0usize;
    let mut with_portrait = 0usize;
    let mut derived = 0usize;
    // …and the same two counts for the *body* framing, which is what a
    // `<PlayerModel>` frame is drawn through — see
    // `vale_assets::look::portrait::body_framing` and `render::paperdoll`.
    let mut with_body = 0usize;
    let mut body_derived = 0usize;
    let mut body_distances: Vec<f32> = Vec::new();
    let mut body_fovs: Vec<f32> = Vec::new();
    // **Whether the body framing actually holds the body**, which is the one
    // check on the derivation that the type code cannot make: the height the
    // frustum spans at the eye's distance, as a multiple of the model's own.
    // Under 1.0 is a paper doll with its feet or its head cut off.
    let mut body_coverage: Vec<f32> = Vec::new();
    // …and the ones that do not fit, with where their framing came from. A
    // derived one that crops is this client's bug; a file's own that crops is
    // the file, and the file wins.
    let mut body_cropped: Vec<(String, f32, Source)> = Vec::new();
    // Where the two kinds of camera aim, as a fraction of the bind-pose height.
    // Buckets rather than an average, because the claim is about the *shape* of
    // the two populations and an average of a bimodal set says nothing.
    let mut portrait_aims: Vec<f32> = Vec::new();
    let mut character_aims: Vec<f32> = Vec::new();
    // The one that would be a bug: a portrait camera aiming below the waist.
    let mut low: Vec<(String, f32)> = Vec::new();
    let mut fovs: Vec<f32> = Vec::new();
    let mut distances: Vec<f32> = Vec::new();
    // **How often `cameras[0]` is the wrong one.** This is the number that says
    // whether choosing by kind is load-bearing or merely tidy: every model in
    // this bucket would have been framed from four yards away at chest height.
    let mut first_is_not_portrait: Vec<String> = Vec::new();
    // …and the models with no portrait camera at all, named rather than
    // counted, since seven is a list.
    let mut derived_models: Vec<String> = Vec::new();

    for path in &paths {
        let Some(m2) = assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) else {
            unreadable += 1;
            continue;
        };
        decoded += 1;
        let bind = portrait::bind_bounds(&m2);
        for camera in &m2.cameras {
            let framing = portrait::Framing {
                eye: camera.position,
                aim: camera.target,
                fov: camera.fov,
                near: camera.near_clip,
                far: camera.far_clip,
                source: Source::Camera,
            };
            let Some(height) = framing.aim_height(bind) else {
                continue;
            };
            match camera.kind {
                0 => portrait_aims.push(height),
                1 => character_aims.push(height),
                _ => {}
            }
        }
        // The body framing, counted apart from the head one. A model may carry
        // either camera, both or neither, so these are four populations and not
        // two — which is the whole reason the two functions exist.
        let body = portrait::body_framing(&m2);
        match body.source {
            Source::Camera => with_body += 1,
            Source::Derived => body_derived += 1,
        }
        body_fovs.push(body.fov);
        body_distances.push(body.distance());
        let height = bind[1][2] - bind[0][2];
        if height > f32::EPSILON {
            // The vertical opening at the panel's own shape, taken the way the
            // renderer takes it: `M2Camera::fov` is a diagonal angle, and
            // `CharacterModelFrame` is 233 by 224.
            let aspect: f32 = 233.0 / 224.0;
            let vertical = body.fov / (aspect * aspect + 1.0).sqrt();
            let span = 2.0 * body.distance() * (vertical * 0.5).tan();
            let coverage = span / height;
            body_coverage.push(coverage);
            if coverage < 1.0 {
                body_cropped.push((path.clone(), coverage, body.source));
            }
        }

        let framing = portrait::framing(&m2);
        match framing.source {
            Source::Camera => {
                with_portrait += 1;
                fovs.push(framing.fov);
                distances.push(framing.distance());
                if m2.cameras.first().is_some_and(|c| c.kind != 0) {
                    first_is_not_portrait.push(path.clone());
                }
                if let Some(height) = framing.aim_height(bind) {
                    if height < 0.5 && low.len() < 8 {
                        low.push((path.clone(), height));
                    }
                }
            }
            Source::Derived => {
                derived += 1;
                derived_models.push(path.clone());
            }
        }
    }

    println!("  {decoded} decoded, {unreadable} unreadable");
    println!(
        "  {with_portrait} carry a portrait camera, {derived} are framed off their own box"
    );
    println!(
        "  {} would have been framed by the wrong camera if the first were taken on trust",
        first_is_not_portrait.len()
    );
    for path in first_is_not_portrait.iter().take(6) {
        println!("    {path}");
    }
    for path in &derived_models {
        println!("    derived: {path}");
    }
    println!(
        "  {with_body} carry a character-info camera, {body_derived} are framed off their own box"
    );
    report("portrait cameras aim", &portrait_aims);
    report("character cameras aim", &character_aims);
    report("portrait field of view", &fovs);
    report("portrait eye distance", &distances);
    report("body field of view", &body_fovs);
    report("body eye distance", &body_distances);
    // **The check on the body framing**, and the one number on this page that
    // is about the picture rather than about the file: how many of the model's
    // own heights the frame spans. Below 1.0 the character does not fit in the
    // panel. See `render::paperdoll`.
    report("body coverage (x model height)", &body_coverage);
    let derived_crops = body_cropped
        .iter()
        .filter(|(_, _, source)| *source == Source::Derived)
        .count();
    println!(
        "  {} of {} would not fit in the character panel — {derived_crops} of them derived",
        body_cropped.len(),
        body_coverage.len()
    );
    body_cropped.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    for (path, coverage, source) in body_cropped.iter().take(8) {
        println!("    {:.0}%  {source:?}  {path}", coverage * 100.0);
    }
    if low.is_empty() {
        println!("  no portrait camera aims below half the body's height");
    } else {
        println!("  {} aim below the waist:", low.len());
        for (path, height) in &low {
            println!("    {:.0}%  {path}", height * 100.0);
        }
    }
    Ok(())
}

/// Median and range of a sample, printed the way every other census here does.
///
/// The **median**, not the mean: the interesting statement about these
/// populations is where the bulk of them sits, and one model with a camera
/// twenty yards out would drag a mean without moving the fact.
fn report(what: &str, values: &[f32]) {
    if values.is_empty() {
        println!("  {what}: nothing measured");
        return;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    println!(
        "  {what}: {} samples, median {:.3}, {:.3} .. {:.3}",
        sorted.len(),
        median,
        sorted[0],
        sorted[sorted.len() - 1]
    );
}
