//! `vale sound` — the sound tables, checked against each other and against
//! the archives.
//!
//! The survey is structural on purpose: every column [`vale_assets::tables::sound`]
//! claims to understand is validated by what its values resolve *in* — a sound
//! column against the `SoundEntries` id set, the footstep column against the
//! lookup's own key set, every referenced file against the archive chain, and
//! every WAV's `fmt` tag against the one encoding the client's decoder speaks
//! (plain PCM). A wrong field index here would not error; it would resolve a
//! plausible-looking nothing, which is exactly the failure the census exists
//! to catch.

use crate::common;
use vale_config::Config;
use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::sound::SoundBank;
use std::collections::{HashMap, HashSet};

pub fn cmd_sound(cfg: &Config, arg: Option<&str>) -> Result<(), String> {
    let mut assets = common::open_assets(cfg)?;
    let bank = SoundBank::load(|table| assets.read(&dbc_path(table)).ok())
        .map_err(|e| e.to_string())?;
    match arg {
        None => survey(&bank, &mut assets),
        Some(arg) => trace(&bank, &mut assets, arg),
    }
}

fn survey(bank: &SoundBank, assets: &mut vale_assets::Assets) -> Result<(), String> {
    let counts = bank.counts();
    println!(
        "SoundEntries: {} rows, {} named; ZoneMusic {}, SoundAmbience {}, \
         ZoneIntroMusicTable {}, CreatureSoundData {}, FootstepTerrainLookup {}, \
         GroundEffectTexture (with sound) {}, areas with sound {}, spells with sound {}",
        counts.entries,
        counts.named,
        counts.zone_music,
        counts.ambience,
        counts.intro_music,
        counts.creatures,
        counts.footsteps,
        counts.ground_terrain,
        counts.area_sounds,
        counts.spells,
    );

    // --- what an id is: every referenced file, against the archives ---
    let mut files = 0usize;
    let mut missing = 0usize;
    let mut by_ext: HashMap<String, usize> = HashMap::new();
    let mut wav_pcm = 0usize;
    let mut undecodable: Vec<String> = Vec::new();
    let started = std::time::Instant::now();
    for entry in bank.entries_iter() {
        for (file, _) in &entry.files {
            files += 1;
            let path = entry.path(file);
            let ext = file
                .rsplit('.')
                .next()
                .unwrap_or("none")
                .to_ascii_lowercase();
            *by_ext.entry(ext.clone()).or_default() += 1;
            if !assets.exists(&path) {
                missing += 1;
                if missing <= 5 {
                    println!("  MISSING  entry {} -> {}", entry.id, path);
                }
                continue;
            }
            // The question that decides whether the client can play the file,
            // asked with **the same decoder at the same version** — because
            // bevy_audio panics the whole app on one it cannot eat, this list
            // is the difference between a measured degradation and a crash on
            // whatever zone references the file. (The WAV fmt tag is folded
            // in: every failure seen so far is an MP3 whose frames symphonia
            // rejects, and every readable WAV is plain PCM.)
            if let Ok(bytes) = assets.read(&path) {
                if ext == "wav"
                    && bytes.get(20..22).map(|b| u16::from_le_bytes([b[0], b[1]])) == Some(1)
                {
                    wav_pcm += 1;
                }
                if rodio::Decoder::new(std::io::Cursor::new(bytes)).is_err() {
                    undecodable.push(format!("entry {} -> {}", entry.id, path));
                }
            }
        }
    }
    let mut exts: Vec<_> = by_ext.iter().collect();
    exts.sort_by(|a, b| b.1.cmp(a.1));
    let exts: Vec<String> = exts.iter().map(|(e, n)| format!("{n} {e}")).collect();
    println!(
        "files: {files} referenced ({}) — {} resolve in the archives, {missing} missing  \
         [{:.1}s]",
        exts.join(", "),
        files - missing,
        started.elapsed().as_secs_f32()
    );
    println!(
        "decodable: {} of {} present decode with the client's own decoder ({wav_pcm} PCM wavs); \
         {} DO NOT and are skipped at play time:",
        files - missing - undecodable.len(),
        files - missing,
        undecodable.len()
    );
    for path in &undecodable {
        println!("  UNDECODABLE  {path}");
    }

    // --- the columns, pinned by what they resolve in ---
    let resolves = |id: u32| id == 0 || bank.entry(id).is_some();
    let mut creature_cols: [(usize, usize); 12] = [(0, 0); 12];
    let mut footstep_keys: HashSet<u32> = HashSet::new();
    let mut terrain_keys: HashSet<u32> = HashSet::new();
    for (&(creature, terrain), _) in bank.footsteps_iter() {
        footstep_keys.insert(creature);
        terrain_keys.insert(terrain);
    }
    let mut footstep_col = (0usize, 0usize);
    // **`impact_type` is pinned differently from every column beside it**, and
    // that is the point of listing it here: it holds a *material*, not a sound
    // id, so what it has to land inside is the ten slots of a
    // `WeaponImpactSounds` row rather than the `SoundEntries` id set. A column
    // that can exceed its supposed key space is not that key space — which is
    // how the `TerrainType` hop was found — so this counts the rows inside it.
    let mut impact_type = (0usize, 0usize);
    let mut impact_type_values: HashMap<u32, usize> = HashMap::new();
    for (_, sounds) in bank.creatures_iter() {
        let cols = [
            sounds.exertion,
            sounds.wound,
            sounds.wound_critical,
            sounds.wound_crushing,
            sounds.death,
            sounds.aggro,
            sounds.alert,
            sounds.custom_attack[0],
            sounds.custom_attack[1],
            // **The three a pet speaks from**, which no other unit in the game
            // uses: the two `SMSG_PET_ACTION_SOUND` routes to and the one
            // `SMSG_PET_DISMISS_SOUND` plays. Four rows state them — the
            // warlock's demons — so a count that is not 4 is a wrong column.
            sounds.pet_attack,
            sounds.pet_order,
            sounds.pet_dismiss,
        ];
        for (i, id) in cols.into_iter().enumerate() {
            if id != 0 {
                creature_cols[i].1 += 1;
                creature_cols[i].0 += usize::from(bank.entry(id).is_some());
            }
        }
        if sounds.footstep != 0 {
            footstep_col.1 += 1;
            footstep_col.0 += usize::from(footstep_keys.contains(&sounds.footstep));
        }
        impact_type.1 += 1;
        impact_type.0 +=
            usize::from((sounds.impact_type as usize) < vale_assets::tables::sound::impact_slot::COUNT);
        *impact_type_values.entry(sounds.impact_type).or_default() += 1;
    }
    let names = [
        "exertion",
        "wound",
        "wound-crit",
        "wound-crushing",
        "death",
        "aggro",
        "alert",
        "attack1",
        "attack2",
        "pet-attack",
        "pet-order",
        "pet-dismiss",
    ];
    let cols: Vec<String> = names
        .iter()
        .zip(creature_cols)
        .map(|(name, (ok, all))| format!("{name} {ok}/{all}"))
        .collect();
    println!("creature sound columns resolve in SoundEntries: {}", cols.join(", "));
    // **A range check on a column of small integers is nearly vacuous**, so the
    // spread is printed beside it: a column that is all zeroes would pass the
    // bound and mean nothing at all.
    let mut spread: Vec<(u32, usize)> = impact_type_values.into_iter().collect();
    spread.sort_unstable();
    println!(
        "CreatureImpactType lands inside a weapon row's ten slots: {}/{}, spread {:?}",
        impact_type.0, impact_type.1, spread
    );

    // **The impact table's own key**, which is two things and not one. Printed
    // as the count of subclasses carrying *both* rows, because that number is
    // the size of the bug a one-part key has: every one of them loses a row.
    let mut by_subclass: HashMap<u32, usize> = HashMap::new();
    let (mut impact_rows, mut impact_ids) = (0usize, (0usize, 0usize));
    for (&(subclass, _), row) in bank.impacts_iter() {
        impact_rows += 1;
        *by_subclass.entry(subclass).or_default() += 1;
        for id in row.hit.iter().chain(row.crit.iter()) {
            if *id != 0 {
                impact_ids.1 += 1;
                impact_ids.0 += usize::from(resolves(*id));
            }
        }
    }
    let two_rows = by_subclass.values().filter(|&&n| n > 1).count();
    println!(
        "WeaponImpactSounds: {impact_rows} rows over {} subclasses, {two_rows} of which state \
         both a metal and a non-metal row; their {} slot ids resolve {}/{} — and the unarmed \
         row ({}) is {:?}",
        by_subclass.len(),
        impact_ids.1,
        impact_ids.0,
        impact_ids.1,
        vale_assets::tables::sound::UNARMED_SUBCLASS,
        bank.impact(
            vale_assets::tables::sound::UNARMED_SUBCLASS,
            false,
            vale_assets::tables::sound::impact_slot::ARMOR_FLESH,
            false,
        )
        .and_then(|id| bank.entry(id))
        .map(|e| e.name.as_str())
        .unwrap_or("MISSING"),
    );
    for (name, two_handed) in [
        (vale_assets::tables::sound::COMBAT_MISS_1H, false),
        (vale_assets::tables::sound::COMBAT_MISS_2H, true),
    ] {
        match bank.miss_whoosh(two_handed) {
            Some(id) => println!("  miss whoosh {name:?} -> entry {id}"),
            None => println!("  miss whoosh {name:?} -> MISSING, every miss is silent"),
        }
    }
    println!(
        "creature footstep column resolves in FootstepTerrainLookup keys: {}/{} \
         ({} keys, {} terrains: {:?})",
        footstep_col.0,
        footstep_col.1,
        footstep_keys.len(),
        terrain_keys.len(),
        {
            let mut t: Vec<u32> = terrain_keys.iter().copied().collect();
            t.sort_unstable();
            t
        }
    );

    // **How many of the game's units have a voice at all**, which is the
    // question a report of "many combat sounds do not play" turns into. The
    // columns above resolve at 263/263 and say nothing about it: the join runs
    // display -> (its own override, else its model's own) -> a
    // `CreatureSoundData` row, and a display that reaches no row is mute in
    // every one of the five columns no matter how clean the table is.
    let mut displays = (0usize, 0usize, 0usize); // total, via override, via model
    let mut voiced: [usize; 4] = [0; 4]; // exertion, wound, death, aggro
    let mut with_row = 0usize;
    for (display, over, _model) in bank.displays_iter() {
        displays.0 += 1;
        let Some(sounds) = bank.unit_sounds(display) else {
            continue;
        };
        with_row += 1;
        if over != 0 {
            displays.1 += 1;
        } else {
            displays.2 += 1;
        }
        for (i, id) in [sounds.exertion, sounds.wound, sounds.death, sounds.aggro]
            .into_iter()
            .enumerate()
        {
            voiced[i] += usize::from(id != 0);
        }
    }
    println!(
        "display ids that reach a CreatureSoundData row: {with_row}/{} \
         ({} through the display's own override, {} through its model's) — \
         of those, exertion {} wound {} death {} aggro {}",
        displays.0, displays.1, displays.2, voiced[0], voiced[1], voiced[2], voiced[3],
    );

    // **Checked through the hop, not against the raw column.** A ground
    // effect's field 6 is a `TerrainType` *row id*; the lookup is keyed on that
    // row's own field 4. Checking the row id straight against the key set is
    // what let grass resolve as wood for a round — both are small integers over
    // overlapping ranges, so it passed at 1,827 of 1,830 while being one along
    // for every texture in the game. The three that failed were the `None` rows
    // at id 10, which is one past the largest key the lookup has.
    let mut ground: (usize, usize) = (0, 0);
    for (&effect, _) in bank.ground_terrain_iter() {
        ground.1 += 1;
        ground.0 += usize::from(terrain_keys.contains(&bank.terrain_of_ground_effect(effect)));
    }
    println!(
        "ground-effect textures resolve a footstep key through TerrainType: {}/{}",
        ground.0, ground.1
    );

    let mut footstep_sounds = (0usize, 0usize);
    for (_, cell) in bank.footsteps_iter() {
        for id in [cell.sound, cell.splash] {
            if id != 0 {
                footstep_sounds.1 += 1;
                footstep_sounds.0 += usize::from(resolves(id));
            }
        }
    }
    println!(
        "footstep cells resolve in SoundEntries: {}/{}",
        footstep_sounds.0, footstep_sounds.1
    );

    item_group_sounds(bank, assets);

    // --- the zone chain ---
    let (mut amb, mut music, mut intro) = ((0, 0), (0, 0), (0, 0));
    for (_, sounds) in bank.area_sounds_iter() {
        for (id, (ok, all), table) in [
            (sounds.ambience, &mut amb, 0),
            (sounds.zone_music, &mut music, 1),
            (sounds.intro_music, &mut intro, 2),
        ] {
            if id == 0 {
                continue;
            }
            *all += 1;
            let hit = match table {
                0 => bank.ambience(id).is_some(),
                1 => bank.zone_music(id).is_some(),
                _ => bank.intro_music(id).is_some(),
            };
            *ok += usize::from(hit);
        }
    }
    println!(
        "AreaTable sound columns resolve: ambience {}/{}, music {}/{}, intro {}/{}",
        amb.0, amb.1, music.0, music.1, intro.0, intro.1
    );
    buildings_and_their_sounds(bank, assets);
    let mut day_night = (0usize, 0usize);
    for (_, m) in bank.zone_music_iter() {
        for i in [0, 1] {
            if m.sounds[i] != 0 {
                day_night.1 += 1;
                day_night.0 += usize::from(resolves(m.sounds[i]));
            }
        }
    }
    for (_, a) in bank.ambience_iter() {
        for i in [0, 1] {
            if a.sounds[i] != 0 {
                day_night.1 += 1;
                day_night.0 += usize::from(resolves(a.sounds[i]));
            }
        }
    }
    for (_, i) in bank.intro_music_iter() {
        if i.sound != 0 {
            day_night.1 += 1;
            day_night.0 += usize::from(resolves(i.sound));
        }
    }
    println!(
        "zone music/ambience/intro entries resolve in SoundEntries: {}/{}",
        day_night.0, day_night.1
    );

    // --- the spell chain ---
    let mut spell_ids = (0usize, 0usize);
    for (_, sounds) in bank.spell_sounds_iter() {
        for id in [
            sounds.precast,
            sounds.cast,
            sounds.impact,
            sounds.channel,
            sounds.missile,
        ] {
            if id != 0 {
                spell_ids.1 += 1;
                spell_ids.0 += usize::from(resolves(id));
            }
        }
    }
    println!(
        "spell kit/missile sounds resolve in SoundEntries: {}/{}",
        spell_ids.0, spell_ids.1
    );
    Ok(())
}

/// **The buildings' own sound columns, and the two ways a naive join loses
/// them.**
///
/// Both numbers here are the *reason* a tavern was reported playing the forest
/// outside it, so they are printed rather than assumed:
///
/// * **rows that state a sound while stating no `areaTableId`** — every one of
///   those is unreachable through `AreaTable` and can only be had by reading
///   this table's own columns;
/// * **`AreaTable` rows that state one column and not another** — every one of
///   those is silenced in the missing column by a fallback that works a row at
///   a time, which is what [`vale_assets::tables::sound::AreaSounds::or`] replaced.
/// **What an item sounds like changing hands** — `ItemGroupSounds` against
/// `SoundEntries`, and against the display table that names it.
///
/// Three numbers, and the middle one is the whole reason this check exists.
/// This repo recorded the table's columns as "**not** `SoundEntries` ids", off
/// the six rows that carry 273/274/275 — which resolve to nothing, and which
/// nothing in the shipped `ItemDisplayInfo` names. The other eighteen carry
/// 1183..1221, every one of them a `PickUp*`/`PutDown*` row. So the honest
/// report is per column and beside the *reach*: how many display rows point at
/// a row at all, and how many of those land on a sound that is in the archives.
fn item_group_sounds(bank: &SoundBank, assets: &mut vale_assets::Assets) {
    let Ok(raw) = assets.read(&dbc_path("ItemGroupSounds")) else {
        println!("ItemGroupSounds: not in the archives");
        return;
    };
    let groups = vale_assets::tables::itemsound::ItemGroupSounds::parse(&raw);
    let ids = groups.ids();
    let mut columns = (0usize, 0usize);
    for id in &ids {
        for sound in [groups.pick_up(*id), groups.put_down(*id)] {
            let Some(sound) = sound else { continue };
            columns.1 += 1;
            columns.0 += usize::from(bank.entry(sound).is_some());
        }
    }
    println!(
        "ItemGroupSounds: {} rows, {}/{} of their pick-up/put-down columns resolve in SoundEntries",
        ids.len(),
        columns.0,
        columns.1
    );

    let Ok(raw) = assets.read(&dbc_path("ItemDisplayInfo")) else {
        return;
    };
    let Ok(displays) = vale_assets::tables::item::ItemDisplays::parse(&raw) else {
        return;
    };
    let (mut named, mut landed) = (0usize, 0usize);
    let mut used: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for display in displays.ids() {
        let Some(group) = displays.group_sound(display) else {
            continue;
        };
        named += 1;
        used.insert(group);
        landed += usize::from(groups.pick_up(group).is_some_and(|id| bank.entry(id).is_some()));
    }
    println!(
        "  …and {named} of {} ItemDisplayInfo rows name one of {} groups; {landed} reach a pick-up sound that exists",
        displays.len(),
        used.len()
    );
}

fn buildings_and_their_sounds(bank: &SoundBank, assets: &mut vale_assets::Assets) {
    let Ok(raw) = assets.read(&dbc_path("WMOAreaTable")) else {
        println!("WMOAreaTable: not in the archives");
        return;
    };
    let Some(areas) = vale_assets::tables::wmoarea::WmoAreas::parse(&raw) else {
        println!("WMOAreaTable: will not parse");
        return;
    };
    let (mut with_sound, mut unreachable, mut resolve) = (0usize, 0usize, (0usize, 0usize));
    for row in areas.rows() {
        let s = row.sounds;
        if s == vale_assets::tables::sound::AreaSounds::default() {
            continue;
        }
        with_sound += 1;
        unreachable += usize::from(row.area == 0);
        for (id, hit) in [
            (s.ambience, bank.ambience(s.ambience).is_some()),
            (s.zone_music, bank.zone_music(s.zone_music).is_some()),
            (s.intro_music, bank.intro_music(s.intro_music).is_some()),
        ] {
            if id != 0 {
                resolve.1 += 1;
                resolve.0 += usize::from(hit);
            }
        }
    }
    println!(
        "WMOAreaTable: {with_sound} rows state a sound, {unreachable} of them state no area \
         at all (reachable only here); their columns resolve {}/{}",
        resolve.0, resolve.1
    );
    let mut partial = (0usize, 0usize);
    for (_, s) in bank.area_sounds_iter() {
        partial.1 += 1;
        partial.0 += usize::from(s.ambience == 0 || s.zone_music == 0);
    }
    println!(
        "AreaTable: {} of {} rows with any sound state one column and not another — \
         a per-row fallback silences every one of them",
        partial.0, partial.1
    );
}

/// One entry in full — by id, or by the name `PlaySound` would use.
fn trace(bank: &SoundBank, assets: &mut vale_assets::Assets, arg: &str) -> Result<(), String> {
    let entry = match arg.parse::<u32>() {
        Ok(id) => bank.entry(id),
        Err(_) => bank.entry_named(arg),
    }
    .ok_or_else(|| format!("no sound entry {arg:?} (by id or by name)"))?;
    println!(
        "entry {}  {:?}  type {}  volume {:.2}  distance {:.0}..{:.0}  flags {:#x}",
        entry.id,
        entry.name,
        entry.kind,
        entry.volume,
        entry.min_distance,
        entry.cutoff_distance,
        entry.flags
    );
    println!("  directory {:?}", entry.directory);
    for (file, weight) in &entry.files {
        let path = entry.path(file);
        let held = if assets.exists(&path) { "ok" } else { "MISSING" };
        println!("  weight {weight}  {path}  [{held}]");
    }
    Ok(())
}
