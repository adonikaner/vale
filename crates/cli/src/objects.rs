//! The things the **server** places: what a `GAMEOBJECT_DISPLAYID` resolves to,
//! and whether you can walk through it.
//!
//! A game object is the one drawn population that is in no `MDDF` or `MODF` row
//! — a door, a portcullis, a chest, a mailbox, a campfire — so nothing in
//! `vale models` or `vale collision` covers it, and it is the population
//! the "you can walk through the Deadmines doors" report is about.
//!
//! The check that matters is the same one `vale npc` makes, one table over
//! and for a different reason: **a game object with no hull is not an error**,
//! it is the game saying *walk through me* (`crate::collision`'s own rule), so
//! the only way to tell a correct absence from a broken lookup is to count both
//! and see which files are in which bucket. A field index read one column out
//! resolves to a plausible model name and answers *no hull* for the entire
//! table, which is exactly the shape of failure that reads as "the fix did not
//! work" rather than as an error.

use crate::common::*;
use vale_config::Config;

/// Survey every `GameObjectDisplayInfo` row, trace one, or find the ones whose
/// model path contains a word.
///
/// The third form is not a convenience. The table is 1,638 rows of paths and no
/// names, so "is the Thunder Bluff elevator solid" is otherwise a question you
/// cannot ask of it at all — and that is the shape most questions about this
/// table take, because what the *server* calls a thing and what its model file
/// is called are different vocabularies.
pub fn cmd_objects(cfg: &Config, arg: Option<&str>) -> Result<(), String> {
    match arg.map(|a| (a, a.parse::<u32>())) {
        Some((_, Ok(id))) => survey(cfg, Some(id), None),
        Some((word, Err(_))) => survey(cfg, None, Some(word)),
        None => survey(cfg, None, None),
    }
}

fn survey(cfg: &Config, display_id: Option<u32>, word: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::{dbc_path, Dbc};

    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let raw = assets
        .read(&dbc_path("GameObjectDisplayInfo"))
        .map_err(|e| e.to_string())?;
    let dbc = Dbc::parse(&raw).map_err(|e| e.to_string())?;

    if let Some(id) = display_id {
        return trace(&mut assets, &tables, id);
    }

    // Every row's id, then the client's own resolution of it — so this crosses
    // the table with the archives through the same function the renderer calls
    // rather than through a second copy of the rule.
    let ids: Vec<u32> = (0..dbc.record_count)
        .filter_map(|r| dbc.u32_at(r, 0))
        .collect();
    println!(
        "DBFilesClient\\GameObjectDisplayInfo.dbc: {} rows\n",
        ids.len()
    );

    if let Some(word) = word {
        let needle = word.to_ascii_lowercase();
        let mut found = 0;
        for id in &ids {
            let Some(model) = tables.game_object(*id) else {
                continue;
            };
            if !model.path.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            found += 1;
            let state = match assets.read(&model.path) {
                Err(_) if model.path.to_ascii_lowercase().ends_with(".wmo") => {
                    "a building (.wmo)".to_string()
                }
                Err(e) => format!("!! {e}"),
                Ok(bytes) => match vale_assets::world::m2::M2::parse(&bytes) {
                    Err(e) => format!("!! will not parse: {e}"),
                    Ok(m2) if m2.collision.is_empty() => "no hull".to_string(),
                    Ok(m2) => format!("{} tris", m2.collision.triangle_count()),
                },
            };
            println!("  {id:>5}  {state:>16}  {}", model.path);
        }
        println!("\n  {found} models whose path contains {word:?}");
        return Ok(());
    }

    let mut named = 0usize;
    let mut wmos = 0usize;
    let mut missing = 0usize;
    let mut unparsed = 0usize;
    let mut solid: Vec<(u32, String, usize)> = Vec::new();
    let mut hollow: Vec<(u32, String)> = Vec::new();

    for id in ids {
        let Some(model) = tables.game_object(id) else {
            continue;
        };
        named += 1;
        // A transport — a zeppelin, a boat, the Deeprun Tram — is a `.wmo` and
        // is drawn as nothing today, so it is solid as nothing too. Counted
        // apart rather than folded into "no hull", because the two are opposite
        // statements: one is the game saying walk through me and this is the
        // client saying it has not looked.
        if model.path.to_ascii_lowercase().ends_with(".wmo") {
            wmos += 1;
            continue;
        }
        let Ok(bytes) = assets.read(&model.path) else {
            missing += 1;
            hollow.push((id, model.path.clone()));
            continue;
        };
        match vale_assets::world::m2::M2::parse(&bytes) {
            Err(_) => {
                unparsed += 1;
                hollow.push((id, model.path.clone()));
            }
            Ok(m2) if m2.collision.is_empty() => hollow.push((id, model.path.clone())),
            Ok(m2) => solid.push((id, model.path.clone(), m2.collision.triangle_count())),
        }
    }

    let triangles: usize = solid.iter().map(|(_, _, t)| t).sum();
    let widest = solid.iter().map(|(_, _, t)| *t).max().unwrap_or(0);
    let narrowest = solid.iter().map(|(_, _, t)| *t).min().unwrap_or(0);
    println!("  resolve to a model     {named}");
    println!("  …of which .wmo         {wmos}   (a transport: drawn as nothing, solid as nothing)");
    println!("  M2 missing from the archive  {missing}");
    println!("  M2 will not parse            {unparsed}");
    println!(
        "\n  carry a hull           {}   {triangles} solid triangles, {narrowest}..{widest} each",
        solid.len()
    );
    println!(
        "  no hull                {}   (the game's own \"walk through me\")",
        hollow.len().saturating_sub(missing + unparsed)
    );

    // **The population the report is about.** A door with no hull is the bug;
    // a barrel with no hull is the game. Naming the doors separately is what
    // turns one aggregate into a claim about the thing that was reported.
    let is_door = |path: &str| {
        let p = path.to_ascii_lowercase();
        ["door", "gate", "portcullis"].iter().any(|w| p.contains(w))
    };
    let doors_solid = solid.iter().filter(|(_, p, _)| is_door(p)).count();
    let doors_hollow: Vec<&(u32, String)> = hollow.iter().filter(|(_, p)| is_door(p)).collect();
    println!(
        "\n  named a door, a gate or a portcullis: {} of which {doors_solid} are solid",
        doors_solid + doors_hollow.len()
    );
    for (id, path) in doors_hollow.iter().take(12) {
        println!("    !! {id:>5}  {path}  has no hull");
    }
    if doors_hollow.len() > 12 {
        println!("    … and {} more", doors_hollow.len() - 12);
    }

    println!("\n  the ten heaviest hulls:");
    solid.sort_by_key(|(_, _, t)| std::cmp::Reverse(*t));
    for (id, path, tris) in solid.iter().take(10) {
        println!("    {id:>5}  {tris:>6} tris  {path}");
    }

    locks(&mut assets);
    transports(&tables);
    Ok(())
}

/// **The ones that move**, which is the population no other line in this survey
/// can see: an elevator is a game object like any other, right up to the moment
/// it is somewhere other than where it was spawned.
///
/// The number to look at is the **cycle**, and it is the one that would be got
/// wrong: `TransportAnimation.dbc` has no duration column, so the round trip is
/// the last node's own time (see
/// `vale_assets::tables::transport`, which reads that off the reference's own
/// divisor). A reader that took a fixed cycle, or the first node's time, would
/// print rows that look exactly like these and animate every platform in the
/// game at the wrong rate.
///
/// The travel is printed per axis on purpose. Every lift in the game moves in
/// **z alone** and that is what makes the client's frame conversion safe on the
/// reported population — see `crate::world::entities::transport`'s note in the
/// renderer, which rotates the offset by the object's facing and would show a
/// mistake only on something that moves horizontally.
fn transports(tables: &vale_assets::tables::dbc::DisplayTables) {
    let table = tables.transports();
    if table.is_empty() {
        println!(
            "
  TransportAnimation.dbc is not in the archives — every elevator stands still"
        );
        return;
    }
    println!(
        "
  TransportAnimation.dbc: {} gameobject entries that move, by entry",
        table.len()
    );
    println!("    entry   nodes   cycle      travel x / y / z (yards)");
    for entry in table.entries() {
        let nodes = table.nodes(entry).unwrap_or_default();
        let cycle = table.total_time(entry).unwrap_or(0);
        let span = |axis: usize| {
            let lo = nodes.iter().map(|n| n.offset[axis]).fold(f32::MAX, f32::min);
            let hi = nodes.iter().map(|n| n.offset[axis]).fold(f32::MIN, f32::max);
            hi - lo
        };
        println!(
            "    {entry:>6}  {:>5}   {:>6.1} s   {:>8.2} {:>8.2} {:>8.2}",
            nodes.len(),
            cycle as f32 / 1000.0,
            span(0),
            span(1),
            span(2),
        );
    }
    // …and one of them sampled, which is the only way to see that the
    // interpolation and the wrap are doing anything at all. 4170 is the Thunder
    // Bluff mesa lift, which is the report's own subject.
    const MESA: u32 = 4170;
    if let Some(cycle) = table.total_time(MESA) {
        println!("
  entry {MESA} (the mesa lift) sampled across its own cycle:");
        for step in 0..=8 {
            let phase = u64::from(cycle) * step / 8;
            let at = table.offset_at(MESA, phase).unwrap_or_default();
            let seq = table.sequence_at(MESA, phase).unwrap_or(0);
            println!(
                "    {:>6.1} s   z {:>8.2}   sequence {seq}",
                phase as f32 / 1000.0,
                at[2],
            );
        }
    }
}

/// **What it takes to open one** — `Lock.dbc`, `LockType.dbc` and the spells
/// that answer them.
///
/// The other half of a game object, and the half that decides what the pointer
/// says: an ore vein and a strongbox are the same `GAMEOBJECT_TYPE_CHEST` and
/// the only thing between them is the lock. See
/// `vale_assets::look::object`, whose whole rule is checkable from these
/// three files with no server.
///
/// **The number to look at is the last one.** Every `LockType` a lock names must
/// have a spell that opens it, because a chest is opened by *casting* and not by
/// `CMSG_GAMEOBJ_USE` — a lock type with no opener is a thing in the world that
/// nothing can ever open, and there should be none of those.
fn locks(assets: &mut vale_assets::Assets) {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::tables::lock::{KeyKind, Locks};
    use vale_assets::tables::spellbook::Spells;
    use std::collections::BTreeMap;

    let table = Locks::parse(
        &assets.read(&dbc_path("Lock")).unwrap_or_default(),
        &assets.read(&dbc_path("LockType")).unwrap_or_default(),
    );
    let (rows, names) = table.counts();
    if rows == 0 {
        println!("\n  Lock.dbc is not in the archives — every game object reads as unlocked");
        return;
    }
    let spells = Spells::parse(
        &assets.read(&dbc_path("Spell")).unwrap_or_default(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .ok();

    // Every lock type any lock names, with how many locks name it.
    let mut wanted: BTreeMap<u32, usize> = BTreeMap::new();
    let mut item_keys = 0usize;
    for id in 0..=u32::from(u16::MAX) {
        for key in table.keys(id) {
            match key.kind {
                KeyKind::Skill { lock_type, .. } => *wanted.entry(lock_type).or_default() += 1,
                KeyKind::Item(_) => item_keys += 1,
            }
        }
    }

    println!("\n  Lock.dbc: {rows} locks that name a way in, LockType.dbc: {names} named");
    println!("  {item_keys} slot(s) want an item; the skill slots are:");
    let mut unopenable = 0usize;
    for (lock_type, count) in &wanted {
        let name = table.lock_type_name(*lock_type).unwrap_or("??");
        let openers = spells.as_ref().map_or(&[][..], |s| s.open_lock_spells(*lock_type));
        if openers.is_empty() {
            unopenable += 1;
        }
        println!(
            "    {lock_type:>2}  {name:<24} {count:>3} lock(s)   {} spell(s) open it{}",
            openers.len(),
            match openers.first() {
                Some(first) => format!("  (e.g. {first})"),
                None => "  !! nothing can".to_string(),
            }
        );
    }
    println!("  {unopenable} lock type(s) that no spell opens");

    // **Column 0, which is the only one the plate reads** — see
    // `Locks::first_slot`. A lock that fills a later column and leaves this
    // one empty draws no "Requires" line in the reference at all, and a reader
    // that searched all eight wrote one under every such object.
    let mut filled = 0usize;
    let mut elsewhere = 0usize;
    let mut actions: BTreeMap<u32, usize> = BTreeMap::new();
    for id in 0..=u32::from(u16::MAX) {
        let keys = table.keys(id);
        if keys.is_empty() {
            continue;
        }
        if table.first_slot(id).is_some() {
            filled += 1;
        } else {
            elsewhere += 1;
        }
        for key in keys {
            *actions.entry(key.action).or_default() += 1;
        }
    }
    println!(
        "  column 0: {filled} lock(s) fill it and draw a requirement line, {elsewhere} leave it empty and draw none"
    );
    let actions: Vec<String> = actions
        .iter()
        .map(|(action, count)| format!("{action}: {count}"))
        .collect();
    println!("  actions over every filled slot (0 open, 1 unlock, 2 close): {}", actions.join(", "));
}

/// One display id in full: the model, the file, the hull, and where the hull
/// lands once the server's own position and facing are applied to it.
fn trace(
    assets: &mut vale_assets::Assets,
    tables: &vale_assets::tables::dbc::DisplayTables,
    id: u32,
) -> Result<(), String> {
    let model = tables
        .game_object(id)
        .ok_or_else(|| format!("display id {id} is not in GameObjectDisplayInfo"))?;
    println!("game object display {id}:");
    println!("  model  {}", model.path);
    if model.path.to_ascii_lowercase().ends_with(".wmo") {
        println!(
            "  a building rather than an M2 — a continent transport. Its route, its schedule and its hull are `vale ships`; this command reads M2s."
        );
        return Ok(());
    }

    let bytes = assets.read(&model.path).map_err(|e| e.to_string())?;
    println!("  file   {} bytes", bytes.len());
    let m2 = vale_assets::world::m2::M2::parse(&bytes).map_err(|e| e.to_string())?;
    let hull = &m2.collision;
    if hull.is_empty() {
        println!("  hull   none — the game's own \"walk through me\"");
        return Ok(());
    }
    println!(
        "  hull   {} triangles over {} vertices",
        hull.triangle_count(),
        hull.positions.len()
    );

    // Placed the way the client places one: at the origin, facing north, at
    // scale 1 — so the box printed is the model's own footprint in world yards,
    // which is the number to compare against a doorway.
    let matrix = vale_assets::world::collision::object_matrix([0.0; 3], 0.0, 1.0);
    let placed = vale_assets::world::collision::Collider::place(hull, &matrix);
    let [lo, hi] = placed.bounds();
    println!(
        "  box    [{:.2} {:.2} {:.2}] .. [{:.2} {:.2} {:.2}]   {:.2} x {:.2} x {:.2} yards",
        lo[0],
        lo[1],
        lo[2],
        hi[0],
        hi[1],
        hi[2],
        hi[0] - lo[0],
        hi[1] - lo[1],
        hi[2] - lo[2]
    );

    // How many of its triangles a stride would be stopped by, as opposed to
    // stood on: a door is nearly all wall, a bridge nearly all floor, and a
    // model that is all floor is one a character walks through and stands on.
    let walls = (0..placed.triangle_count())
        .filter(|&t| placed.triangle_is_wall(t))
        .count();
    println!(
        "  solid  {walls} of {} triangles block a stride; the rest are floor",
        placed.triangle_count()
    );
    Ok(())
}
