//! `vale attach` — attached models: directories, sides, skins, points.

use crate::common::*;
use vale_config::Config;

/// The half of equipment that is a *model*: where each one lives, what it is
/// skinned with, and which point on the wearer it hangs from.
///
/// Three claims are checked here rather than assumed, because all three fail
/// silently — a wrong directory is a file that is not there, a wrong texture
/// slot draws magenta, and a wrong attachment point puts a pauldron on the
/// wrong shoulder, which is *in range* and wrong:
///
/// * every `modelName` resolves under `Item\ObjectComponents\<slot>\`, with the
///   slot's directory taken from [`Slot::object_directory`];
/// * `modelName[0]` is the **left** piece and `[1]` the right, which the file
///   names give away (`LShoulder_…`, `RShoulder_…`);
/// * the model's own textures are type 2 with no filename, so `modelTexture`
///   is what fills them, out of the same directory.
///
/// And on the wearer's side: every character model carries attachment points 5,
/// 6 and 11, and the right shoulder really is on the right — model space is
/// +Y **left**, so `SHOULDER_RIGHT` must have negative Y.
pub fn cmd_attach(cfg: &Config, display_id: Option<u32>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::tables::item::{ItemDisplays, Slot};
    use vale_assets::world::m2::{attach, M2};
    use std::collections::BTreeMap;

    let mut assets = open_assets(cfg)?;
    let table = ItemDisplays::parse(
        &assets
            .read(&dbc_path("ItemDisplayInfo"))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    // The wearer's side first: the points themselves, on the models players
    // actually use. `vale npc` already established that display ids 49..57
    // are the bare race models, so those are the ones a character wears.
    let tables = open_display_tables(&mut assets)?;
    println!("attachment points on the character models:");
    // Race and gender reach a model path only through a display id, exactly as
    // in `vale char`: the NPCs wearing a character model are the data's own
    // statement of which M2 a race and gender means.
    let mut character_models: Vec<String> = (1..20_000u32)
        .filter(|id| tables.appearance(*id).is_some())
        .filter_map(|id| tables.creature(id))
        .map(|d| d.path.clone())
        .collect();
    character_models.sort();
    character_models.dedup();
    let mut without = 0;
    // A cloak is the one piece of "attached" equipment that is not attached at
    // all: it is the wearer's own group-15 geoset wearing the item's texture.
    // Counted here because the two failure modes look identical on screen —
    // a model with no cape geoset draws no cloak, and so does a cloak whose
    // texture never arrived.
    let mut with_cape = 0;
    let mut cape_surprises = 0;
    // Each model's helm point height — what the renderer's camera orbits.
    let mut heads: Vec<(String, f32)> = Vec::new();
    // The sheath points every character model carries, and where they sit.
    let mut sheath_points: BTreeMap<u32, Vec<(String, [f32; 3])>> = BTreeMap::new();
    let file_name = |p: &str| p.rsplit('\\').next().unwrap_or(p).to_string();
    for path in &character_models {
        let Some(model) = assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) else {
            continue;
        };
        let capes: Vec<u16> = {
            let mut ids: Vec<u16> = model
                .batches
                .iter()
                .map(|b| b.geoset)
                .filter(|g| g / 100 == 15)
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        if !capes.is_empty() {
            with_cape += 1;
        }
        // **Which of the cape geosets is a cloak, and which is the bare back.**
        // The texture *type* answers it: type 1 is the composed body skin and
        // type 2 is the object skin an item supplies, so a group-15 batch
        // wearing type 1 is a piece of the character and not a cloak at all.
        // Every model puts that one at `x01`, which is why an item's variant 0
        // has to mean `x02` — see `item::first_variant`.
        for batch in model.batches.iter().filter(|b| b.geoset / 100 == 15) {
            let kind = batch
                .texture
                .and_then(|t| model.textures.get(t as usize))
                .map(|t| t.kind);
            let bare = batch.geoset % 100 == 1;
            if bare != (kind == Some(1)) {
                println!(
                    "    !! {path}: geoset {} has texture type {kind:?}",
                    batch.geoset
                );
                cape_surprises += 1;
            }
        }
        let point = |id: u32| model.attachment(id).map(|a| a.position);
        let (left, right, helm) = (
            point(attach::SHOULDER_LEFT),
            point(attach::SHOULDER_RIGHT),
            point(attach::HELM),
        );
        let sided = match (left, right) {
            // +Y is left in model space, so the two shoulders must straddle
            // zero in that axis with the right one negative.
            (Some(l), Some(r)) => l[1] > 0.0 && r[1] < 0.0,
            _ => false,
        };
        // **The hands, on the same terms as the shoulders.** A drawn weapon
        // hangs on point 1 or 2, and the right hand must be on the right — if
        // the two were the other way round every character in the world would
        // fight left-handed and nothing would say so. Checked here because the
        // renderer now depends on both points existing, where until this round
        // it only needed 5, 6 and 11.
        //
        // **Stated as an ordering, not as a sign.** The shoulders straddle the
        // centreline and can be tested against zero; the hands cannot. A dwarf
        // female's right hand sits at y = +0.004 in her bind pose — on the
        // *left* of the midline by four millimetres, because her arms are short
        // and hang inward — so a sign test calls a perfectly ordinary model
        // broken. What has to be true is only that the right hand is to the
        // right of the left one.
        let (hand_r, hand_l) = (point(attach::HAND_RIGHT), point(attach::HAND_LEFT));
        let handed = match (hand_l, hand_r) {
            (Some(l), Some(r)) => r[1] < l[1],
            _ => false,
        };
        if !handed {
            println!(
                "    !! {path}: hands {:?}/{:?} — a weapon would be hung wrong",
                hand_l.map(|p| p[1]),
                hand_r.map(|p| p[1])
            );
        }
        // **A shield is not held in the left hand, and point 0 is the proof.**
        // The client picks attachment 0 for it in the same branch that picks
        // `Item\ObjectComponents\Shield\`, and the two numbers say why: the
        // shield point sits further out and further back than the palm does,
        // because a shield is strapped to the forearm rather than gripped.
        // Hanging one off the palm draws it through the arm holding it, which
        // is exactly what was on screen.
        match (point(attach::SHIELD), hand_l) {
            (Some(s), Some(l)) if s[1] <= l[1] => println!(
                "    !! {path}: shield point y {:+.2} is not outboard of the left hand's {:+.2}",
                s[1], l[1]
            ),
            (None, _) => println!("    !! {path}: no shield point — a shield would be dropped"),
            _ => {}
        }
        // **The helm point is also the camera's head height.** `head_height` in
        // the renderer reads its z straight out — an attachment position is
        // written in its bone's frame and a bone matrix maps bind-pose model
        // space onto the posed model, so the recorded point already *is* a
        // bind-pose model-space position, and model space is Z-up from the feet.
        // A number that says otherwise would put the camera through the floor or
        // above the character's head with nothing on screen to name the cause,
        // so it is checked against the model's own declared box.
        if let Some(h) = helm {
            heads.push((path.clone(), h[2]));
            if h[2] <= 0.0 || h[2] > model.bounds[1][2] {
                println!(
                    "    !! {path}: helm point at z {:.2}, outside the model's box 0.00..{:.2}",
                    h[2], model.bounds[1][2]
                );
            }
        }
        // **Every point a weapon can hang from, measured rather than named.**
        //
        // The *table* — which sheath type goes to which of these — is not
        // measurable, and trying to infer it from the names is what put every
        // one-handed weapon in the game high on the back for a round: vmangos'
        // seven-value `SHEATHETYPE_*` enum and the attachment enum share their
        // names, the join looks forced, and the client's own enum turns out to
        // have four values with the side coming from the hand instead. That is
        // settled in `item::sheath_point`.
        //
        // What these numbers are for is checking the *result* is somewhere a
        // weapon could be. Model space is X **forward**, Y **left**, Z **up**
        // from the feet, so "on the back", "on the left hip" and "off the left
        // forearm" are numbers, and a table that hangs a one-hander somewhere
        // with positive X is hanging it through the character's stomach.
        for id in attach::WEAPON_POINTS {
            if let Some(p) = point(id) {
                sheath_points
                    .entry(id)
                    .or_insert_with(Vec::new)
                    .push((file_name(path), p));
            }
        }
        if left.is_none() || right.is_none() || helm.is_none() || !sided {
            without += 1;
            println!(
                "    !! {path}: {} points, shoulders {:?}/{:?}, helm {:?}",
                model.attachments.len(),
                left.map(|p| p[1]),
                right.map(|p| p[1]),
                helm.is_some()
            );
        }
    }
    println!(
        "  {} character models, {} carry shoulders + helm with the right one on the right, \
         {with_cape} carry a cape geoset ({cape_surprises} batches where the body/cloak \
         split is not x01/x02)",
        character_models.len(),
        character_models.len() - without
    );
    // **Where each point sits, averaged over the models that carry it.**
    // Printed in full rather than reduced to a verdict, because the reader has
    // to be able to disagree with the reading: forward/back, left/right and
    // height are the three facts, and which sheath type lands on which of them
    // is the inference — one this project got wrong once already.
    if !sheath_points.is_empty() {
        println!("\n  weapon points (model space: +X forward, +Y left, +Z up from the feet):");
        for (id, found) in &sheath_points {
            let n = found.len() as f32;
            let mean = |axis: usize| found.iter().map(|(_, p)| p[axis]).sum::<f32>() / n;
            let (x, y, z) = (mean(0), mean(1), mean(2));
            println!(
                "    [{id:>2}] {:>2} of {} models   x {x:+.2}  y {y:+.2}  z {z:.2}   {}",
                found.len(),
                character_models.len(),
                // The words the numbers say, so a wrong table is legible without
                // reading three columns of floats.
                format!(
                    "{}, {}, {}",
                    if x < -0.03 { "behind" } else if x > 0.03 { "in front" } else { "centred" },
                    if y > 0.03 { "left" } else if y < -0.03 { "right" } else { "midline" },
                    if z > 1.0 { "high" } else { "low" }
                )
            );
        }
    }

    heads.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("finite"));
    let file_of = |p: &str| p.rsplit('\\').next().unwrap_or(p).to_string();
    if let (Some(low), Some(high)) = (heads.first(), heads.last()) {
        println!(
            "  head height (helm point z, what the camera orbits): {:.2} {} .. {:.2} {}",
            low.1,
            file_of(&low.0),
            high.1,
            file_of(&high.0)
        );
    }

    // **Which directories `Item\ObjectComponents\` actually has**, and how many
    // models are in each. The slot -> directory mapping is a client-side rule
    // that no table states, so it is read off the archive rather than guessed:
    // a weapon named `Axe_2H_War_B_01.mdx` in a display row is a file somewhere
    // under here, and which subdirectory is the whole question.
    {
        let all = assets.list_prefix("item\\objectcomponents\\");
        let mut dirs: BTreeMap<String, u64> = BTreeMap::new();
        for path in &all {
            if let Some(rest) = path.strip_prefix("item\\objectcomponents\\") {
                if let Some((dir, _)) = rest.split_once('\\') {
                    *dirs.entry(dir.to_string()).or_insert(0) += 1;
                }
            }
        }
        println!("\n  Item\\ObjectComponents\\ subdirectories:");
        for (dir, n) in &dirs {
            println!("    {dir:<16} {n}");
        }
    }

    // What the head directory actually holds, since a helmet is the one
    // attached model whose file name is not the one the DBC gives.
    let head: Vec<String> = assets.list_prefix("item\\objectcomponents\\head\\");
    println!(
        "\n  Item\\ObjectComponents\\Head\\ holds {} files; a sample:",
        head.len()
    );
    for path in head.iter().filter(|p| p.contains("plate_d_04")).take(12) {
        println!("    {path}");
    }
    // The suffix set is the race-and-gender table, read off the files rather
    // than transcribed: a helm is cut for the head it sits on.
    let mut suffixes: BTreeMap<String, u64> = BTreeMap::new();
    for path in &head {
        if let Some(stem) = path.strip_suffix(".m2") {
            *suffixes
                .entry(stem.chars().rev().take(3).collect::<String>().chars().rev().collect())
                .or_insert(0) += 1;
        }
    }
    let common: Vec<String> = suffixes
        .iter()
        .filter(|(_, n)| **n > 20)
        .map(|(s, n)| format!("{s} {n}"))
        .collect();
    println!("    suffixes: {}", common.join("  "));

    // One item in full, when asked for.
    if let Some(id) = display_id {
        let look = table
            .appearance(id, 0)
            .ok_or_else(|| format!("display id {id} is not in ItemDisplayInfo"))?;
        println!("\n  display {id}  models {:?}  (as a human male)", look.models);
        // The cloak, which is the wearer's own geoset and the item's texture.
        if let Some(cloak) = vale_assets::tables::item::cloak_texture(
            &table,
            &[vale_assets::tables::item::Equipped {
                display_id: id,
                slot: Slot::Back,
            }],
        ) {
            println!(
                "    worn on the back: {cloak}{}",
                if assets.exists(&cloak) {
                    ""
                } else {
                    "   !! not in the archive"
                }
            );
        }
        for slot in [Slot::Head, Slot::Shoulders] {
            for attached in look.attachments(slot, 1, 0) {
                let there = assets.exists(&attached.path);
                println!(
                    "    {:?} point {:<3} {}{}",
                    slot,
                    attached.point,
                    attached.path,
                    if there { "" } else { "   !! not in the archive" }
                );
                if let Some(texture) = &attached.texture {
                    println!(
                        "        skin {texture}{}",
                        if assets.exists(texture) {
                            ""
                        } else {
                            "   !! not in the archive"
                        }
                    );
                }
                if there {
                    if let Some(model) = assets.read(&attached.path).ok().and_then(|b| M2::parse(&b).ok())
                    {
                        println!(
                            "        {} batches, texture types {:?}",
                            model.batches.len(),
                            model.textures.iter().map(|t| t.kind).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    // The whole table: which directory each model is in, and whether the
    // left/right convention holds.
    let mut by_directory: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut missing_model = 0u64;
    let mut missing_texture = 0u64;
    let mut wrong_side = 0u64;
    let mut kinds: BTreeMap<u32, u64> = BTreeMap::new();
    let mut parsed = 0u64;
    for (n, id) in table.ids().iter().enumerate() {
        let Some(look) = table.appearance(*id, 0) else {
            continue;
        };
        if !look.has_geometry() {
            continue;
        }
        // The slot is not in this table — it is in the item, which comes from
        // the server — so the directory is *probed*, which is also what checks
        // that `object_directory` names the right ones. Weapons and shields are
        // in the list for exactly that: a row whose model is not under the
        // directory its slot claims simply fails to resolve, and the
        // per-directory counts below are the answer.
        for slot in [Slot::Head, Slot::Shoulders, Slot::MainHand, Slot::Shield] {
            // A helm is cut per race, so the survey has to name a wearer: a
            // human male stands for all sixteen cuts, and the count below says
            // whether that assumption ever costs a helmet.
            let attached = look.attachments(slot, 1, 0);
            if attached.is_empty() || !assets.exists(&attached[0].path) {
                continue;
            }
            *by_directory
                .entry(slot.object_directory().unwrap_or("?"))
                .or_insert(0) += attached.len() as u64;
            // Left first, right second: the names say so, and a row that
            // disagrees would swap a character's pauldrons.
            if slot == Slot::Shoulders && attached.len() == 2 {
                let name = |p: &str| p.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
                if !name(&attached[0].path).starts_with('l') || !name(&attached[1].path).starts_with('r')
                {
                    if wrong_side < 6 {
                        println!("    !! not L-then-R: {:?}", look.models);
                    }
                    wrong_side += 1;
                }
            }
            for one in &attached {
                if !assets.exists(&one.path) {
                    missing_model += 1;
                    continue;
                }
                match &one.texture {
                    Some(t) if !assets.exists(t) => missing_texture += 1,
                    None => missing_texture += 1,
                    _ => {}
                }
                // Every fiftieth model parsed in full: the point is the texture
                // *types*, which is what says the client has to supply the skin.
                if n % 50 == 0 {
                    if let Some(model) = assets.read(&one.path).ok().and_then(|b| M2::parse(&b).ok()) {
                        parsed += 1;
                        for texture in &model.textures {
                            *kinds.entry(texture.kind).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    println!("\n  attached models that resolve, by directory:");
    for (directory, n) in &by_directory {
        println!("    {n:>6} x Item\\ObjectComponents\\{directory}\\");
    }
    println!("  {missing_model} name a model no directory holds, {missing_texture} have no skin");
    println!("  {wrong_side} shoulder rows are not left-then-right");
    println!("  {parsed} models parsed as a sample; their texture types:");
    for (kind, n) in &kinds {
        println!(
            "    {n:>6} x type {kind}{}",
            match kind {
                0 => "  (the model names the file itself)",
                2 => "  (object skin — the client supplies it)",
                _ => "",
            }
        );
    }
    Ok(())
}
