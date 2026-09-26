//! `vale glue` — what the login and character-select screens are made of.
//!
//! The renderer's own `glue.rs` draws those screens in egui as a placeholder, and
//! the round that replaces it with the game's own art needs to start from a list
//! of what is actually in the archive chain rather than from a list of paths
//! somebody remembers. So this **enumerates** `Interface\Glues\` instead of
//! probing guessed names, decodes every model it finds, and says which of their
//! textures resolve.
//!
//! It is the same shape as every other check here: a thing that is missing from
//! the archives is reported as a number, because a login screen with a texture
//! it cannot resolve does not fail — it draws magenta, or nothing at all.

use crate::common::*;
use vale_assets::world::m2::M2;
use vale_config::Config;

/// Everything under this prefix belongs to a screen shown before the world.
const GLUES: &str = "interface\\glues\\";

pub fn cmd_glue(cfg: &Config) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let files = assets.list_prefix(GLUES);
    if files.is_empty() {
        return Err(format!("no files under {GLUES} — is the archive chain right?"));
    }
    println!("{} file(s) under {GLUES}", files.len());

    // By subdirectory, which is how the game groups them: `Common` is the
    // furniture every glue screen shares, `LoadingScreens` the zone art,
    // `Models` the 3D scenes, `CharacterCreate` and `CharacterSelect` their own.
    let mut groups: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for path in &files {
        let rest = &path[GLUES.len()..];
        let group = rest.split('\\').next().unwrap_or("").to_string();
        let entry = groups.entry(group).or_insert((0, 0));
        entry.0 += 1;
        if is_model(path) {
            entry.1 += 1;
        }
    }
    println!("\n  {:<24} {:>6}  {:>6}", "group", "files", "models");
    for (group, (count, models)) in &groups {
        println!("  {group:<24} {count:>6}  {models:>6}");
    }

    // The 3D scenes. This is the half that is not a texture and therefore the
    // half that decides whether the real screens are a rendering job or a
    // texture-blitting job.
    let models: Vec<&String> = files.iter().filter(|p| is_model(p)).collect();
    println!("\n{} model(s):", models.len());
    let mut missing_textures = 0;
    let mut decoded = 0;
    // **The cameras, which are the whole framing of the two screens.** Counted
    // and printed rather than merely parsed, because a wrong `M2Camera` offset
    // does not fail — it reads as a plausible eye and a plausible aim, and the
    // login screen then draws the same geometry from the wrong place. See
    // `vale_assets::world::m2::M2Camera`, whose guards this is the check for.
    let mut cameras = 0;
    let mut with_camera = 0;
    // **The plinths, which are where the *character* stands.** Same argument as
    // the cameras one line up, one step further: nothing in any file says that
    // attachment 0 of `UI_Orc.m2` is a pair of feet, and a client that guessed a
    // position instead would draw a character standing somewhere plausible in a
    // scene authored around a spot it is not on. What makes it checkable is that
    // the artists put the camera *on* it: the point should lie on the camera's
    // own axis and about a body's-eye-height below the aim.
    let mut plinths = 0;
    let mut worst_lateral: f32 = 0.0;
    // **The lights, which are the whole of how these scenes are shaded.** Same
    // argument as the two above: nothing in the world reads an `M2Light`, so
    // this block was unparsed for the life of the project and the login screen
    // was lit by `Light.dbc`'s daylight for a map nobody is on — which draws
    // pale stone flat where the file asks for dark stone with a fire on it.
    // Counted, and the *animated* ones counted separately, because the parser
    // samples each track's first key and only the count says whether that is a
    // simplification or the whole truth.
    let mut lights = 0;
    let mut animated = 0;
    let mut off_pivot = 0;
    let mut directional = 0;
    for path in &models {
        let Ok(bytes) = assets.read(path) else {
            println!("  {path}  — in the listing and not readable");
            continue;
        };
        let Ok(m2) = M2::parse(&bytes) else {
            println!("  {path}  — {} bytes, will not parse", bytes.len());
            continue;
        };
        decoded += 1;
        cameras += m2.cameras.len();
        with_camera += usize::from(!m2.cameras.is_empty());
        let named: Vec<&str> = m2
            .textures
            .iter()
            .filter(|t| t.kind == 0 && !t.file_name.is_empty())
            .map(|t| t.file_name.as_str())
            .collect();
        let resolved = named
            .iter()
            .filter(|name| assets.exists(&name.to_ascii_lowercase()))
            .count();
        missing_textures += named.len() - resolved;
        println!(
            "  {path}\n      {} vertices, {} batches, {} texture(s) — {resolved} resolve{}{}",
            m2.positions.len(),
            m2.batches.len(),
            named.len(),
            if m2.skeleton.is_some() { ", animated" } else { "" },
            if m2.textures.iter().any(|t| t.kind != 0) {
                ", and one the client must supply"
            } else {
                ""
            },
        );
        for name in named.iter().filter(|n| !assets.exists(&n.to_ascii_lowercase())) {
            println!("      missing: {name}");
        }
        for (index, camera) in m2.cameras.iter().enumerate() {
            let [ex, ey, ez] = camera.position;
            let [tx, ty, tz] = camera.target;
            let reach = ((ex - tx).powi(2) + (ey - ty).powi(2) + (ez - tz).powi(2)).sqrt();
            println!(
                "      camera {index}: type {}, fov {:.1}deg, eye ({ex:.1}, {ey:.1}, {ez:.1}) \
                 -> ({tx:.1}, {ty:.1}, {tz:.1}), {reach:.1} away, clip {:.1}..{:.0}",
                camera.kind,
                camera.fov.to_degrees(),
                camera.near_clip,
                camera.far_clip,
            );
        }
        for (index, light) in m2.lights.iter().enumerate() {
            lights += 1;
            animated += usize::from(light.is_animated());
            // **Is the light's own position the whole of where it is?** It is
            // written in its *bone's* frame, so in general it needs the pose —
            // and in these files it does not, because every light bone is a
            // parentless root whose pivot is that same position and which
            // carries no translation track. `crate::render::glue` places the
            // lamps by the position alone on the strength of that, so the
            // moment one of these disagrees the scene is lit from the wrong
            // place with nothing else to say so.
            if let Some(bone) = m2
                .skeleton
                .as_ref()
                .and_then(|s| s.bones.get(light.bone as usize))
            {
                let off = (0..3)
                    .map(|k| (bone.pivot[k] - light.position[k]).abs())
                    .fold(0.0f32, f32::max);
                if off > 1e-3 || bone.parent >= 0 || bone.translation.is_some() {
                    off_pivot += 1;
                    println!(
                        "      !! light {index} is not its bone's pivot: {off:.3} off, \
                         parent {}, translation {}",
                        bone.parent,
                        if bone.translation.is_some() { "keyed" } else { "none" },
                    );
                }
            }
            let [ar, ag, ab] = light.ambient_colour();
            let [dr, dg, db] = light.diffuse_colour();
            // **Where it is, or where it shines *from*** — two different
            // questions, and the light's own `type` decides which one it
            // answers. `UI_Human`'s directionals are 92 and 127 yards from the
            // character they light, so reading one as a placed lamp is a
            // character lit by its ambient alone. See
            // `vale_assets::world::m2::M2Light::kind`.
            let (kind, place) = if light.is_point() {
                let [x, y, z] = light.position;
                ("point", format!("at ({x:.2}, {y:.2}, {z:.2})"))
            } else {
                let [x, y, z] = light.direction;
                directional += 1;
                ("dirn ", format!("from ({x:.2}, {y:.2}, {z:.2})"))
            };
            println!(
                "      light {index}: {kind} {} on bone {} {place} — \
                 ambient ({ar:.3}, {ag:.3}, {ab:.3}), diffuse ({dr:.3}, {dg:.3}, {db:.3}), \
                 authored range {:.2}..{:.2} (unread){}",
                if light.is_lamp() { "lamp" } else { "fill" },
                light.bone,
                light.attenuation_start,
                light.attenuation_end,
                if light.is_animated() { ", ANIMATED" } else { "" },
            );
        }
        // …and the two attachment points, against the first camera. Every
        // backdrop carries exactly two and `UI_MainMenu` carries none, which is
        // itself the check: the login screen is the one glue scene with no
        // character in it.
        for point in &m2.attachments {
            let [px, py, pz] = point.position;
            let framing = match m2.cameras.first() {
                Some(camera) => {
                    let (along, lateral, below) = against_camera(camera, point.position);
                    if point.id == PLINTH_POINT {
                        plinths += 1;
                        worst_lateral = worst_lateral.max(lateral);
                    }
                    format!(
                        " — {along:.2} along the camera's axis, {lateral:.2} off it, \
                         {below:.2} below the aim"
                    )
                }
                None => String::new(),
            };
            println!(
                "      attachment {} on bone {} at ({px:.2}, {py:.2}, {pz:.2}){framing}",
                point.id, point.bone,
            );
        }
    }

    println!(
        "\n{decoded} of {} models decode, {missing_textures} texture(s) named but absent",
        models.len()
    );
    // **The number that says the camera offset is right.** A `<Model>` widget
    // calls `SetCamera(0)` and gets nothing when this is zero — which draws the
    // scene from the renderer's own fallback and is exactly as plausible as the
    // real thing until you compare the two.
    println!("{cameras} camera(s) over {with_camera} of {decoded} models");
    // **The number that says point 0 is the plinth.** Six backdrops, six points,
    // every one of them within a fraction of a yard of the camera's own axis —
    // which is not something an arbitrary attachment would be. If this ever
    // reports a lateral offset in yards, the reading is wrong and
    // `render::glue::PLINTH_POINT` is standing the character somewhere else.
    println!(
        "{plinths} plinth point(s), the furthest {worst_lateral:.2} yards off its camera's axis"
    );
    // **The number that says the first-key sampling is not a simplification.**
    // `crate::assets::m2::M2Light` reads one key per track; if `animated` is
    // ever anything but zero, one of these scenes has a lamp that moves and the
    // renderer is holding its first frame for ever.
    println!(
        "{lights} light(s), {directional} directional, {animated} animated, \n         {off_pivot} not standing on their own bone's pivot"
    );
    Ok(())
}

/// Attachment 0 on a character model is a shield; on these six it is a pair of
/// feet. See `crate::render::glue::PLINTH_POINT` in the client, which is the
/// consumer this number is checked for.
const PLINTH_POINT: u32 = 0;

/// A point in the scene, measured **in the first camera's own frame**:
/// `(distance along the aim, distance off the axis, distance below the aim)`.
///
/// The whole of the plinth reading. A pair of feet in front of a camera reads
/// as "several yards along, nothing off the axis, about a yard below"; anything
/// else in the scene reads as neither.
fn against_camera(camera: &vale_assets::world::m2::M2Camera, point: [f32; 3]) -> (f32, f32, f32) {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let len = |a: [f32; 3]| dot(a, a).sqrt();

    let aim = sub(camera.target, camera.position);
    let reach = len(aim);
    if reach < 1e-6 {
        return (0.0, 0.0, 0.0);
    }
    let dir = [aim[0] / reach, aim[1] / reach, aim[2] / reach];
    let to_point = sub(point, camera.position);
    let along = dot(to_point, dir);
    let off = [
        to_point[0] - dir[0] * along,
        to_point[1] - dir[1] * along,
        to_point[2] - dir[2] * along,
    ];
    // Model space is Z-up, so the vertical part of that residue is how far below
    // the aim the point sits and the rest is how far to the side.
    let below = -off[2];
    let lateral = (off[0] * off[0] + off[1] * off[1]).sqrt();
    (along, lateral, below)
}

/// `.m2` in the archive, `.mdx` in a DBC — and the glue directory holds both
/// spellings, so both count.
fn is_model(path: &str) -> bool {
    path.ends_with(".m2") || path.ends_with(".mdx")
}
