//! `vale npc` — every display id: the model and skin it resolves to.

use crate::common::*;
use vale_config::Config;

/// Resolve display ids the way the renderer does, and check the result exists.
///
/// With an id, it traces one: the two DBC hops, the M2's texture table, and
/// which archive file each slot the client has to supply resolves to. Without
/// one, it surveys every row of `CreatureDisplayInfo` — which is the check that
/// matters, because the failure mode here is silent. A model that asks for
/// texture type 1 and gets nothing is drawn *grey*, not missing, so a wrong
/// field index in `CreatureDisplayInfoExtra` looks exactly like the bug it was
/// meant to fix.
pub fn cmd_npc(cfg: &Config, display_id: Option<u32>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::world::m2::M2;
    use std::collections::BTreeMap;

    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;

    if let Some(id) = display_id {
        let resolved = tables
            .creature(id)
            .ok_or_else(|| format!("display id {id} is not in CreatureDisplayInfo"))?;
        println!("display {id}:");
        println!("  model  {}  scale {:.2}", resolved.path, resolved.scale);
        for (slot, skin) in resolved.skins.iter().enumerate() {
            // Decoded, not merely present: a bake is a *generated* texture, and
            // one in a BLP variant `blp.rs` does not handle would fall back to
            // the same grey placeholder as no texture at all.
            let state = if skin.is_empty() {
                "(none)".to_string()
            } else {
                match assets.read(skin) {
                    Err(e) => format!("{skin}  !! {e}"),
                    Ok(bytes) => match vale_assets::world::blp::decode(&bytes) {
                        Ok(tex) => format!(
                            "{skin}  {}x{} {}",
                            tex.width,
                            tex.height,
                            describe_blp(&bytes)
                        ),
                        Err(e) => format!("{skin}  !! will not decode: {e}"),
                    },
                }
            };
            println!("  skin {slot}  {state}");
        }

        // **The geometry half of an NPC's gear**, which the bake cannot hold: a
        // pauldron is its own model and a bootleg is a geoset. Printed beside
        // the bake so the two halves can be compared — a row with six item
        // columns filled and no attachments here is an NPC wearing all of its
        // textures and none of its shapes.
        if !resolved.equipment.is_empty() {
            println!("  equipped ({} of ten columns filled):", resolved.equipment.len());
            for worn in &resolved.equipment {
                let mut note = String::new();
                if let Some(items) = tables.items() {
                    if let Some(look) = items.appearance(worn.display_id, 0) {
                        let models: Vec<&str> = look
                            .models
                            .iter()
                            .filter(|m| !m.is_empty())
                            .map(String::as_str)
                            .collect();
                        if !models.is_empty() {
                            note = format!("  models {models:?}");
                        }
                    }
                }
                println!("    {:<10} display {}{note}", format!("{:?}", worn.slot), worn.display_id);
            }
            if let (Some(look), Some(items)) = (resolved.appearance, tables.items()) {
                let attachments = vale_assets::tables::item::item_attachments(
                    items,
                    look.race,
                    look.gender,
                    &resolved.equipment,
                );
                println!("    -> {} attached model(s):", attachments.len());
                for a in &attachments {
                    println!("       point {:<3} {}", a.point, a.path);
                }
                let geosets = vale_assets::tables::item::item_geosets(items, &resolved.equipment);
                let worn: Vec<u16> = geosets.into_iter().filter(|g| *g != 0).collect();
                println!("    -> equipment geosets {worn:?}");
            }
        }

        // Which texture types the model actually asks the client for. Anything
        // other than 0 has to come from the tables above, and a type this
        // client has no slot for is a texture that will draw blank.
        let bytes = assets.read(&resolved.path).map_err(|e| e.to_string())?;
        let model = M2::parse(&bytes).map_err(|e| e.to_string())?;
        println!("  the model's texture table ({} entries):", model.textures.len());
        for (i, t) in model.textures.iter().enumerate() {
            let what = match t.kind {
                0 => format!("in the model: {}", t.file_name),
                1 => "type 1 - character body, from the bake -> skin 0".to_string(),
                2 => "type 2 - cape, NOT SUPPLIED".to_string(),
                6 => "type 6 - hair, NOT SUPPLIED".to_string(),
                8 => "type 8 - skin extra, NOT SUPPLIED".to_string(),
                11..=13 => format!("type {} - creature skin -> slot {}", t.kind, t.kind - 11),
                k => format!("type {k} - NOT SUPPLIED"),
            };
            println!("    [{i:>2}] {what}");
        }
        return Ok(());
    }

    // The survey. Every creature display id in the game, bucketed by where its
    // body texture comes from and whether that file is actually there.
    let creature_display = assets
        .read(&dbc_path("CreatureDisplayInfo"))
        .map_err(|e| e.to_string())?;
    let dbc = vale_assets::Dbc::parse(&creature_display).map_err(|e| e.to_string())?;
    println!(
        "CreatureDisplayInfo: {} records; resolving every one",
        dbc.record_count
    );

    let mut from_variation = 0usize;
    let mut from_bake = 0usize;
    let mut bake_missing: Vec<String> = Vec::new();
    let mut no_skin_at_all: BTreeMap<String, usize> = BTreeMap::new();
    let mut unresolved = 0usize;
    // Cache the answer per texture path — the read and decode are the slow part
    // and thousands of NPCs share a few hundred bakes. "Usable" is a decode and
    // not an existence check, because a BLP variant `blp.rs` cannot read draws
    // the same grey as no texture at all.
    let mut usable: BTreeMap<String, bool> = BTreeMap::new();
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();

    for record in 0..dbc.record_count {
        let Some(id) = dbc.u32_at(record, 0) else {
            continue;
        };
        let Some(resolved) = tables.creature(id) else {
            unresolved += 1;
            continue;
        };
        let skin = &resolved.skins[0];
        if skin.is_empty() {
            // Grey unless the model carries its own texture, which most
            // creatures do; only the character models are a problem.
            *no_skin_at_all.entry(resolved.path.clone()).or_insert(0) += 1;
            continue;
        }
        let present = match usable.get(skin) {
            Some(&known) => known,
            None => {
                let ok = match assets.read(skin) {
                    Ok(bytes) => {
                        let decoded = vale_assets::world::blp::decode(&bytes).is_ok();
                        if decoded {
                            *formats.entry(describe_blp(&bytes)).or_insert(0) += 1;
                        }
                        decoded
                    }
                    Err(_) => false,
                };
                usable.insert(skin.clone(), ok);
                ok
            }
        };
        if skin.starts_with("Textures\\BakedNpcTextures\\") {
            from_bake += 1;
            if !present {
                bake_missing.push(format!("{id}: {skin}"));
            }
        } else {
            from_variation += 1;
        }
    }

    println!("  {from_variation} take slot 0 from a CreatureDisplayInfo texture variation");
    println!(
        "  {from_bake} take it from a CreatureDisplayInfoExtra bake, {} of which will not decode",
        bake_missing.len()
    );
    println!("  distinct slot-0 textures by BLP format: {formats:?}");
    for m in bake_missing.iter().take(5) {
        println!("    !! {m}");
    }
    println!("  {} display ids do not resolve to a model at all", unresolved);

    // What is left: rows with no slot-0 texture from either table. Almost all
    // are ordinary creatures whose M2 names its own textures — the ones worth
    // seeing are any character models still in the list.
    let leftover: usize = no_skin_at_all.values().sum();
    let characters: Vec<(&String, &usize)> = no_skin_at_all
        .iter()
        .filter(|(path, _)| path.to_ascii_lowercase().starts_with("character\\"))
        .collect();
    let character_rows: usize = characters.iter().map(|(_, n)| **n).sum();
    println!(
        "  {leftover} have no slot-0 texture ({} distinct models); {character_rows} of those are character models",
        no_skin_at_all.len()
    );
    for (path, n) in characters.iter().take(5) {
        println!("    !! {n:>4} x {path}");
    }

    // The other side of the same coin: how much of the directory is reachable.
    let on_disk = assets.list_prefix("Textures\\BakedNpcTextures\\").len();
    println!("  Textures\\BakedNpcTextures\\ holds {on_disk} files in the archive chain");

    check_npc_hair(&mut assets, &tables, &dbc);

    // **What an NPC in a character model is wearing, and which column says so.**
    //
    // `CreatureDisplayInfoExtra`'s ten item columns are usually described as the
    // record of what the bake was computed from. True of the *texture* half of a
    // garment and false of the geometry half: a pauldron is a separate model and
    // a bootleg is a geoset, and neither can be painted into a body atlas. So
    // the columns have to be read, and which slot each one dresses has to be
    // right — a shoulder id read out of the shirt column hangs a shirt off a
    // bone.
    //
    // Measured the same way `Slot::components` was: a component texture is named
    // `<family>_<piece>_<column>`, so resolving every column of every row through
    // `ItemDisplayInfo` and counting the piece names prints the slot order
    // directly. Rows where the column is 0 say nothing and are skipped.
    let mut by_column: BTreeMap<usize, BTreeMap<String, u64>> = BTreeMap::new();
    let mut with_models = 0u64;
    for record in 0..dbc.record_count {
        let Some(id) = dbc.u32_at(record, 0) else {
            continue;
        };
        for (i, item) in tables.equipped_item_columns(id).into_iter().enumerate() {
            let (Some(items), true) = (tables.items(), item != 0) else {
                continue;
            };
            let piece = (0..8)
                .find_map(|c| items.texture_name(item, c))
                .and_then(|n| n.rsplit('_').nth(1).map(str::to_ascii_lowercase));
            // A row with a model rather than a texture is a pauldron or a helm —
            // the geometry this whole section exists for.
            if let Some(look) = items.appearance(item, 0) {
                if look.models.iter().any(|m| !m.is_empty()) {
                    with_models += 1;
                    *by_column
                        .entry(i)
                        .or_default()
                        .entry("<a model>".to_string())
                        .or_insert(0) += 1;
                }
            }
            if let Some(piece) = piece {
                *by_column.entry(i).or_default().entry(piece).or_insert(0) += 1;
            }
        }
    }
    println!(
        "\n  what each CreatureDisplayInfoExtra item column turns out to name \
         ({with_models} of them are attached models):"
    );
    for (column, pieces) in &by_column {
        let mut top: Vec<_> = pieces.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        let cells: Vec<String> = top.iter().take(4).map(|(p, n)| format!("{p} {n}")).collect();
        let claimed = vale_assets::tables::dbc::DisplayTables::EQUIPPED_SLOTS
            .get(*column)
            .map(|s| format!("{s:?}"))
            .unwrap_or_default();
        println!("    column {column} ({claimed})   {}", cells.join(", "));
    }

    shadow_blob(&mut assets);
    Ok(())
}

/// **Every NPC in a character model, and whether its hair resolves.**
///
/// This is the check that was missing, and its absence was a false all-clear:
/// `vale npc` verified the **bake** — slot 0, the body atlas —
/// over all 10,534 display ids and reported clean, while the hair on a character
/// model is not in the bake at all. The hairstyle is *geometry* dressed by its
/// own texture (the M2 asks for it as type 6), and geometry with no texture
/// bound draws **magenta**, which is the reported bug.
///
/// The two tables that have to agree are keyed differently, which is the whole
/// mechanism: `CharHairGeosets` is `(race, gender, style)` and `CharSections` is
/// `(race, gender, style, colour)`. A `CreatureDisplayInfoExtra` row states both
/// numbers and nothing checks them against each other, so a colour index with no
/// row for that style still gets a geoset — hair with nothing on it.
fn check_npc_hair(
    assets: &mut vale_assets::Assets,
    tables: &vale_assets::tables::dbc::DisplayTables,
    dbc: &vale_assets::tables::dbc::Dbc,
) {
    use vale_assets::look::character::CharSections;
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::world::m2::M2;
    use std::collections::BTreeMap;

    let Some(sections) = assets
        .read(&dbc_path("CharSections"))
        .ok()
        .and_then(|raw| CharSections::parse(raw.as_ref()).ok())
    else {
        println!("\n  CharSections.dbc will not read — no hair check");
        return;
    };

    let mut characters = 0u32;
    let mut dressed = 0u32;
    let mut bald = 0u32;
    let mut hairless = 0u32;
    let mut magenta: BTreeMap<(u8, u8, u8, u8), Vec<u32>> = BTreeMap::new();
    // **Does this model have a hair slot at all?** Cached per path, because the
    // 6,984 rows share eighteen models. A model with no type-6 texture cannot
    // draw one however the tables answer — `GoblinMale.m2` declares two
    // textures, both body — so counting those as magenta would be a check
    // reporting a bug that cannot happen.
    let mut has_hair_slot: BTreeMap<String, bool> = BTreeMap::new();
    for record in 0..dbc.record_count {
        let Some(id) = dbc.u32_at(record, 0) else {
            continue;
        };
        let Some(look) = tables.appearance(id) else {
            continue;
        };
        characters += 1;
        let Some(model) = tables.creature(id) else {
            continue;
        };
        let slot = match has_hair_slot.get(&model.path) {
            Some(&known) => known,
            None => {
                let known = assets
                    .read(&model.path)
                    .ok()
                    .and_then(|b| M2::parse(&b).ok())
                    .is_some_and(|m2| m2.textures.iter().any(|t| t.kind == 6));
                has_hair_slot.insert(model.path.clone(), known);
                known
            }
        };
        if !slot {
            hairless += 1;
            continue;
        }
        let geoset = tables
            .char_geosets()
            .hair(&look)
            .map(|h| h.geoset)
            .unwrap_or(0);
        if geoset == 0 {
            bald += 1;
        } else if sections.skin(&look).hair.is_some() {
            dressed += 1;
        } else {
            magenta
                .entry((look.race, look.gender, look.hair_style, look.hair_colour))
                .or_default()
                .push(id);
        }
    }
    let ids: usize = magenta.values().map(Vec::len).sum();
    println!(
        "\n  {characters} display ids wear a character model: {dressed} with hair that resolves, \
         {bald} bald, {hairless} on a model with no hair slot, \
         {ids} drawing a hair mesh with no texture ({} distinct appearances)",
        magenta.len()
    );
    for ((race, gender, style, colour), ids) in magenta.iter().take(8) {
        println!(
            "    !! race {race} gender {gender} style {style} colour {colour}: \
             {} display ids, e.g. {}",
            ids.len(),
            ids[0]
        );
    }
}

/// **The shadow every unit in the world stands on**, measured rather than
/// assumed.
///
/// 1.12 casts no runtime shadow from a character: the client pairs
/// `Textures\ShadowBlob.blp` with two CVars — `shadowBias` ("Unit shadow depth
/// bias") and `shadowLOD` ("Unit shadow LOD") — and the *map* shadows are a
/// separate switch (`mapShadows`, which is this client's `MCSH`). One texture,
/// one bias, one distance: a blob decal under each unit, and nothing that
/// projects geometry.
///
/// So the only open question is **how to draw it**, and that is a property of
/// the file: a black image with an alpha ramp blends, and a white-on-black one
/// modulates. It is the same question `lake_a` answered wrongly for two rounds
/// by nobody measuring the texture's own colour, so it is measured here — the
/// peak channel and the alpha coverage, which between them name the blend mode.
fn shadow_blob(assets: &mut vale_assets::Assets) {
    const BLOB: &str = "Textures\\ShadowBlob.blp";
    println!("\n  the shadow a unit stands on ({BLOB}):");
    let Ok(bytes) = assets.read(BLOB) else {
        println!("    !! not in the archive chain");
        return;
    };
    let Ok(blob) = vale_assets::world::blp::decode(&bytes) else {
        println!("    !! will not decode ({})", describe_blp(&bytes));
        return;
    };
    let mut peak = 0u8;
    let mut alpha_total = 0u64;
    let mut opaque = 0u64;
    let mut opaque_total = 0u64;
    for texel in blob.rgba.chunks_exact(4) {
        peak = peak.max(texel[0]).max(texel[1]).max(texel[2]);
        alpha_total += texel[3] as u64;
        if texel[3] == 255 {
            opaque += 1;
            opaque_total += texel[0].max(texel[1]).max(texel[2]) as u64;
        }
    }
    let texels = (blob.rgba.len() / 4).max(1) as u64;
    let at = |x: u32, y: u32| -> u8 {
        let i = ((y * blob.width + x) * 4) as usize;
        blob.rgba.get(i).copied().unwrap_or(0)
    };
    println!("    {}x{} {}", blob.width, blob.height, describe_blp(&bytes));
    println!(
        "    peak colour channel {peak}/255, mean alpha {}/255, {opaque} of {texels} texels opaque",
        alpha_total / texels,
    );
    println!(
        "    centre {}/255, mean over the opaque texels {}/255",
        at(blob.width / 2, blob.height / 2),
        opaque_total / opaque.max(1),
    );
    // **And what the file puts under the alpha, which a modulate blend cannot
    // ignore.** Multiplying is not blending: the corners of the square are cut
    // out by a one-bit alpha that no blend factor here reads, so whatever colour
    // the palette left there is what the ground gets multiplied by. Black
    // corners would print a black square around every character.
    let cut: Vec<u8> = blob
        .rgba
        .chunks_exact(4)
        .filter(|t| t[3] != 255)
        .map(|t| t[0].max(t[1]).max(t[2]))
        .collect();
    let cut_mean = cut.iter().map(|&c| c as u64).sum::<u64>() / (cut.len().max(1) as u64);
    println!(
        "    under the cut-out alpha: {} texels, mean {cut_mean}/255",
        cut.len(),
    );
    // **The shape, as eight rows of one digit** — because "a dark blob with an
    // alpha ramp" and "a light ring in a dark square" produce the same three
    // numbers above and want opposite blend modes. Luminance 0..9, a dot where
    // the texel is transparent.
    let step = (blob.width.max(blob.height) / 8).max(1);
    for y in (0..blob.height).step_by(step as usize) {
        let row: String = (0..blob.width)
            .step_by(step as usize)
            .map(|x| {
                let i = ((y * blob.width + x) * 4) as usize;
                match blob.rgba.get(i + 3) {
                    Some(255) => char::from_digit((at(x, y) as u32 * 9) / 255, 10).unwrap_or('?'),
                    _ => '.',
                }
            })
            .collect();
        println!("      {row}");
    }
    // The verdict, stated rather than left to the reader: this is what the
    // renderer's blend mode is chosen from.
    let verdict = if peak < 32 {
        "dark throughout -> blend mode 2, the darkness being the texel's own colour"
    } else if cut_mean > 250 {
        "dark in the middle, white at the rim and white under the cut -> blend mode 5 \
         (modulate), and it needs no fixup: the corners multiply by one"
    } else {
        "dark in the middle and not white under the cut -> modulate would print a \
         square; whiten the cut-out texels first"
    };
    println!("    {verdict}");
}
