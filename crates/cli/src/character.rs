//! A player's appearance: the composed skin, and the geosets it chooses.

use crate::common::*;
use vale_assets::Assets;
use vale_config::Config;


/// Check the player-skin composition against the archive.
///
/// This is the same kind of check as `vale npc`, and for the same reason: a
/// character whose skin does not resolve is drawn *magenta* or *grey*, which
/// reads as a rendering bug rather than as a missing lookup. Three questions,
/// none of which can be answered by looking at the screen:
///
/// 1. **Is the region layout right?** The composite's rectangles are hardcoded
///    (the DBC that states them arrives in Cataclysm), so the check is that
///    every texture in the table is *exactly* the size of the region it is
///    declared to fill. A wrong rectangle puts a face on a thigh, at plausible
///    coordinates, with no error anywhere.
/// 2. **Does every part resolve?** Over every appearance the table can
///    describe, not over the one character that happens to be logged in.
/// 3. **Does every file decode?** A full BLP decode, not an existence test —
///    the same reason `vale npc` decodes the bakes.
pub fn cmd_char(cfg: &Config, who: Option<&str>) -> Result<(), String> {
    use vale_assets::world::blp;
    use vale_assets::look::character::{Appearance, CharSections, Composite};
    use vale_assets::tables::dbc::dbc_path;

    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read(&dbc_path("CharSections"))
        .map_err(|e| e.to_string())?;
    let table = CharSections::parse(&raw).map_err(|e| e.to_string())?;
    println!("CharSections.dbc: {} records", table.len());

    // 1. Every declared region against the file that fills it.
    let mut sizes: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let mut wrong_size = 0u32;
    let mut undecodable = 0u32;
    let mut missing = 0u32;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, path, region) in table.declared_regions() {
        if !seen.insert(path.to_string()) {
            continue;
        }
        let Ok(bytes) = assets.read(path) else {
            println!("    !! not in the archive: {path}");
            missing += 1;
            continue;
        };
        let Ok(image) = blp::decode(&bytes) else {
            println!("    !! will not decode: {path}");
            undecodable += 1;
            continue;
        };
        *sizes
            .entry(format!("{}x{}", image.width, image.height))
            .or_insert(0) += 1;
        if image.width != region.width || image.height != region.height {
            if wrong_size < 8 {
                println!(
                    "    !! {path} is {}x{} but its region is {}x{}",
                    image.width, image.height, region.width, region.height
                );
            }
            wrong_size += 1;
        }
    }
    println!(
        "  {} distinct textures: {missing} not in the archive, {undecodable} will not decode, \
         {wrong_size} the wrong size for their region",
        seen.len()
    );

    // **…and the one file per hair row that the region survey above cannot
    // see.** A hair row's texture 0 dresses the hair *mesh* rather than a
    // rectangle of the composite, so it has no region and was never in the
    // list — which made it the only file in a 4,030-row table that nothing
    // checked, and the only one whose absence draws **magenta** instead of
    // drawing nothing. See [`vale_assets::look::character::CharSections::hair_meshes`].
    let mut hair_seen: std::collections::BTreeSet<&str> = Default::default();
    let mut hair_missing: Vec<&str> = Vec::new();
    let mut hair_undecodable: Vec<&str> = Vec::new();
    for path in table.hair_meshes() {
        if !hair_seen.insert(path) {
            continue;
        }
        match assets.read(path) {
            Err(_) => hair_missing.push(path),
            Ok(bytes) => {
                if blp::decode(&bytes).is_err() {
                    hair_undecodable.push(path);
                }
            }
        }
    }
    println!(
        "  {} distinct hair-mesh textures (M2 type 6): {} not in the archive, {} will not decode",
        hair_seen.len(),
        hair_missing.len(),
        hair_undecodable.len()
    );
    for path in hair_missing.iter().chain(&hair_undecodable).take(8) {
        println!("    !! {path}");
    }
    for (size, n) in &sizes {
        println!("    {n:>5} x {size}");
    }

    // 2. Every appearance the table can describe.
    let looks = table.every_appearance();
    let mut no_skin = 0u32;
    let mut no_face = 0u32;
    let mut no_hair = 0u32;
    let mut layers = 0u64;
    for look in &looks {
        let composed = table.skin(look);
        if composed.layers.is_empty() {
            no_skin += 1;
        }
        if !composed
            .layers
            .iter()
            .any(|l| l.region == vale_assets::look::character::regions::FACE_LOWER)
        {
            no_face += 1;
        }
        if composed.hair.is_none() {
            no_hair += 1;
        }
        layers += composed.layers.len() as u64;
    }
    println!(
        "  {} appearances: {no_skin} with no layers at all, {no_face} with no face, \
         {no_hair} with no hair texture; {:.1} layers each on average",
        looks.len(),
        layers as f64 / looks.len().max(1) as f64,
    );

    // 3. One character composed in full, so the numbers have a worked example
    //    behind them. Defaults to the first appearance in the table.
    let look = match who {
        Some(spec) => {
            let n: Vec<u8> = spec.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if n.len() < 2 {
                return Err(
                    "usage: vale char [race,gender[,skin,face,hairStyle,hairColour,facialHair]]"
                        .to_string(),
                );
            }
            Appearance {
                race: n[0],
                gender: n[1],
                skin: n.get(2).copied().unwrap_or(0),
                face: n.get(3).copied().unwrap_or(0),
                hair_style: n.get(4).copied().unwrap_or(0),
                hair_colour: n.get(5).copied().unwrap_or(0),
                facial_hair: n.get(6).copied().unwrap_or(0),
            }
        }
        None => looks.first().copied().unwrap_or_default(),
    };
    println!(
        "\n  race {} gender {} skin {} face {} hair {}/{} facial {}",
        look.race, look.gender, look.skin, look.face, look.hair_style, look.hair_colour,
        look.facial_hair
    );
    let composed = table.skin(&look);
    let mut composite = Composite::new();
    for layer in &composed.layers {
        let decoded = assets
            .read(&layer.path)
            .ok()
            .and_then(|b| blp::decode(&b).ok());
        match decoded {
            Some(image) => {
                println!(
                    "    {:>3},{:<3} {:>3}x{:<3}  {}  ({}x{})",
                    layer.region.x,
                    layer.region.y,
                    layer.region.width,
                    layer.region.height,
                    layer.path,
                    image.width,
                    image.height,
                );
                composite.paint(layer.region, image.width, image.height, &image.rgba);
            }
            None => println!("    !! {}", layer.path),
        }
    }
    match &composed.hair {
        Some(path) => println!("    hair mesh texture (M2 type 6): {path}"),
        None => println!("    no hair texture"),
    }

    // The composite must actually be covered. A hole reads as a transparent
    // hole in the character, and one that only shows on the back of the head is
    // not something a screenshot finds.
    let rgba = composite.rgba();
    let transparent = rgba.chunks_exact(4).filter(|p| p[3] == 0).count();
    println!(
        "  composed 256x256: {:.1}% of it still transparent",
        transparent as f64 * 100.0 / (rgba.len() / 4) as f64
    );

    // 4. The *geometry* half of an appearance, against the models that carry it.
    //
    //    The hair a character wears is a geoset and not a texture, and a geoset
    //    that is not in the model is not an error — it simply draws nothing, and
    //    a bald character is exactly what this client shipped for a whole
    //    milestone without anything saying so. So the check is against the
    //    models themselves: every hairstyle the table names must be a geoset the
    //    race's own M2 actually has.
    check_character_geosets(&mut assets, &table, &looks)
}

/// The geosets each of the 18 character models carries, keyed by race and
/// gender.
///
/// Race and gender reach a model path only through a display id: the NPCs that
/// wear a character model are the data's own statement of which M2 a race and
/// gender means, which beats matching the path by name.
///
/// Shared by `vale char` and `vale item`, because the question both of
/// them end up asking is the same one — *does the geoset this table names exist
/// on the model that would have to draw it?* — and a geoset that does not exist
/// draws nothing rather than failing.
pub fn character_model_geosets(
    assets: &mut Assets,
    tables: &vale_assets::tables::dbc::DisplayTables,
) -> std::collections::BTreeMap<(u8, u8), std::collections::BTreeSet<u16>> {
    use vale_assets::world::m2::M2;
    use std::collections::{BTreeMap, BTreeSet};

    let mut model_for: BTreeMap<(u8, u8), String> = BTreeMap::new();
    for id in 1..20_000u32 {
        let (Some(look), Some(display)) = (tables.appearance(id), tables.creature(id)) else {
            continue;
        };
        model_for.entry((look.race, look.gender)).or_insert(display.path);
    }

    let mut geosets: BTreeMap<(u8, u8), BTreeSet<u16>> = BTreeMap::new();
    for (key, path) in &model_for {
        let Some(m2) = assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) else {
            continue;
        };
        geosets.insert(*key, m2.batches.iter().map(|b| b.geoset).collect());
    }
    geosets
}

/// Which geosets a character model carries, and whether the tables agree.
///
/// Three questions, none of which the screen answers:
///
/// 1. **Does every hairstyle exist?** `CharHairGeosets` names a group-0 geoset
///    per race, gender and style. One the model does not have draws nothing.
/// 2. **Does every facial-hair style exist?** Same, for groups 1..3 — and on a
///    tauren those geosets are the horns, so an empty one is not subtle.
/// 3. **What is in group 7?** Every other equipment group defaults to `x01`, but
///    ears invert it: 701 is the hidden ears a helmet asks for and 702 is the
///    ears. Taking `x01` there leaves every character earless. Printed rather
///    than asserted, because it is the *reason* for a constant.
/// 4. **Do the two hair tables agree?** The geoset comes out of
///    `CharHairGeosets` and the texture that dresses it out of `CharSections`,
///    and *neither table mentions the other*. A style with geometry and no
///    texture is the one combination that cannot be seen as an absence: the
///    hair mesh draws with an unbound sampler, which is **magenta**. See
///    [`check_hair_pairs`].
fn check_character_geosets(
    assets: &mut Assets,
    sections: &vale_assets::look::character::CharSections,
    looks: &[vale_assets::look::character::Appearance],
) -> Result<(), String> {
    use std::collections::{BTreeMap, BTreeSet};

    let tables = open_display_tables(assets)?;

    let geosets = character_model_geosets(assets, &tables);
    println!(
        "\n  {} character models, one per race and gender that a display id names",
        geosets.len()
    );

    let (hair_rows, facial_rows) = tables.char_geosets().len();
    let mut hair_missing = 0u32;
    let mut hair_bald = 0u32;
    for (race, gender, variation, geoset, _scalp) in tables.char_geosets().hair_styles() {
        let Some(have) = geosets.get(&(race, gender)) else {
            continue;
        };
        if geoset == 0 {
            // A real answer: human male style 0 is bald, and the scalp the
            // composite paints is what is seen instead.
            hair_bald += 1;
        } else if !have.contains(&geoset) {
            if hair_missing < 8 {
                println!("    !! race {race} gender {gender} style {variation} wants geoset {geoset}, which the model does not have");
            }
            hair_missing += 1;
        }
    }
    println!(
        "  CharHairGeosets: {hair_rows} styles, {hair_bald} of them bald, \
         {hair_missing} naming a geoset their model lacks"
    );

    let mut facial_missing = 0u32;
    let mut facial_none = 0u32;
    for (race, gender, _variation, groups) in tables.char_geosets().facial_styles() {
        let Some(have) = geosets.get(&(race, gender)) else {
            continue;
        };
        for (i, &variant) in groups.iter().enumerate() {
            if variant == 0 {
                facial_none += 1;
            } else if !have.contains(&((i as u16 + 1) * 100 + variant)) {
                facial_missing += 1;
            }
        }
    }
    println!(
        "  CharacterFacialHairStyles: {facial_rows} styles x 3 groups, \
         {facial_none} empty, {facial_missing} naming a geoset their model lacks"
    );

    // What group 7 actually holds, which is the whole argument for `EARS_SHOWN`.
    let mut ears: BTreeMap<Vec<u16>, u32> = BTreeMap::new();
    for have in geosets.values() {
        let group: Vec<u16> = have.iter().copied().filter(|g| g / 100 == 7).collect();
        *ears.entry(group).or_insert(0) += 1;
    }
    for (group, n) in &ears {
        println!("  group 7 (ears): {n} models carry {group:?}");
    }

    // **Every group a character model carries, and how many variants are in
    // it.** This is the other half of `Slot::geoset_groups`: that table says
    // which group a slot's variant numbers land in, and it can only be right if
    // the group exists and has enough variants for the numbers the wardrobe
    // asks for. A group with a single `x01` is a group nothing can be equipped
    // into; a group with five is one where an item asking for variant 3 has
    // somewhere to go.
    let mut variants: BTreeMap<u16, (u32, BTreeSet<u16>)> = BTreeMap::new();
    for have in geosets.values() {
        let mut groups_here: BTreeSet<u16> = BTreeSet::new();
        for geoset in have {
            groups_here.insert(geoset / 100);
            variants
                .entry(geoset / 100)
                .or_insert((0, BTreeSet::new()))
                .1
                .insert(geoset % 100);
        }
        for group in groups_here {
            variants.entry(group).or_insert((0, BTreeSet::new())).0 += 1;
        }
    }
    println!("  geoset groups the character models carry (models, and every variant seen):");
    for (group, (models, seen)) in &variants {
        let list: Vec<String> = seen.iter().map(u16::to_string).collect();
        println!("    group {group:>2}   {models:>2} models   {}", list.join(","));
    }

    check_hair_pairs(&tables, sections, looks);
    Ok(())
}

/// **The geoset and the texture that dresses it, against each other.**
///
/// A hairstyle is two answers out of two tables that never mention each other:
/// `CharHairGeosets` says which group-0 geoset to draw and `CharSections` says
/// what to paint it with. Three of the four combinations are fine and one is
/// not:
///
/// | geoset | texture | what is on the screen |
/// |---|---|---|
/// | 0 | none | bald, and the composite's scalp is what is seen — correct |
/// | 0 | a file | a texture nothing wears — costs a load and nothing else |
/// | a geoset | a file | hair |
/// | **a geoset** | **none** | **the mesh draws with no texture bound: magenta** |
///
/// The last row is the "untextured, pink hair" report, and it is the one state
/// that cannot present as an absence — a missing geoset draws nothing and a
/// missing texture on nothing is nothing, but geometry with no texture is a
/// bright magenta head of hair. It is checked over **every appearance the table
/// can describe** rather than over the styles alone, because the texture is
/// chosen by the hair *colour* as well as the style and the colours are not
/// square: a style may resolve at colour 0 and not at colour 7.
fn check_hair_pairs(
    tables: &vale_assets::tables::dbc::DisplayTables,
    sections: &vale_assets::look::character::CharSections,
    looks: &[vale_assets::look::character::Appearance],
) {
    let mut untextured = 0u32;
    let mut unworn = 0u32;
    let mut shown = 0u32;
    let mut hairy = 0u32;
    let mut unworn_styles: std::collections::BTreeSet<(u8, u8, u8)> = Default::default();
    for look in looks {
        let geoset = tables
            .char_geosets()
            .hair(look)
            .map(|h| h.geoset)
            .unwrap_or(0);
        let texture = sections.skin(look).hair;
        match (geoset, &texture) {
            (0, None) => {}
            (0, Some(_)) => {
                unworn_styles.insert((look.race, look.gender, look.hair_style));
                unworn += 1;
            }
            (_, Some(_)) => hairy += 1,
            (_, None) => {
                if shown < 8 {
                    println!(
                        "    !! race {} gender {} style {} colour {} draws geoset {geoset} with no texture",
                        look.race, look.gender, look.hair_style, look.hair_colour
                    );
                    shown += 1;
                }
                untextured += 1;
            }
        }
    }
    println!(
        "  hair geoset against hair texture over {} appearances: {hairy} dressed, \
         {unworn} a texture with no geometry, {untextured} geometry with no texture (magenta)",
        looks.len()
    );
    // **The bald styles that ship a hair texture anyway**, which is what the
    // middle number is: they are drawn bald, correctly, and the file the table
    // names for them is the *scalp* the composite paints rather than a mesh.
    // Listed rather than counted because a race appearing here that should not
    // is the "empty hair" half of the same report.
    let styles: Vec<String> = unworn_styles
        .iter()
        .map(|(race, gender, style)| format!("{race}/{gender} style {style}"))
        .collect();
    if !styles.is_empty() {
        println!("    bald styles carrying a texture: {}", styles.join(", "));
    }
}
