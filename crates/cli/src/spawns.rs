//! `vale spawns` — the server's creatures, joined to the client's archives.
//!
//! The one check the creature tool can have without a window: every `creature`
//! row on a map, joined to the `creature_template` row vmangos would load for
//! it, joined to `CreatureDisplayInfo` and out to a model the archives either
//! hold or do not. Two sides measured against each other, and neither of them
//! is this project's own reading of the other.
//!
//! Three forms:
//!
//! ```text
//! vale spawns              which maps have spawns, and how many resolve
//! vale spawns Azeroth      one map: the census, and the ten nearest a tile
//! vale spawns 68           one creature entry traced: its winning template
//!                             row, its model, and where it stands
//! ```
//!
//! Read-only. It writes nothing and runs no statement that changes a row, which
//! is what makes it the thing to run first on a machine that has a server.
//!
//! ## What the numbers mean when they are wrong
//!
//! A spawn with no template row is a spawn the server will refuse to load. A
//! template whose `display_id1` resolves to nothing is a creature the client
//! draws as the fallback box. Both are reported per map rather than summed,
//! because a map where every one fails is a wrong patch or a wrong archive
//! chain and a map where three fail is three bad rows.

use crate::common::open_assets;
use vale_config::Config;
use vale_mangos::conn::{self, Db, Where};
use vale_mangos::creature::{self, RowValue};

/// Every form of the command.
pub fn cmd_spawns(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    let at = Where::find().ok_or_else(Where::absent)?;
    let patch = wow_patch();
    println!("world database: {}  (content patch {patch})", at.line());
    let mut db = Db::open(&at)?;

    match which {
        None => census(&mut db, patch),
        Some(word) => match word.parse::<u32>() {
            Ok(entry) => one(cfg, &mut db, patch, entry),
            Err(_) => map(cfg, &mut db, patch, word),
        },
    }
}

/// The content patch the rows are read at.
///
/// `VALE_MANGOSD` points at the conf that carries it, which is the same file
/// the connection came out of. With only `VALE_WORLDDB` set there is no conf
/// to read and the default stands — see [`conn::DEFAULT_WOW_PATCH`].
fn wow_patch() -> u32 {
    let Ok(at) = std::env::var("VALE_MANGOSD") else {
        return conn::DEFAULT_WOW_PATCH;
    };
    let at = std::path::PathBuf::from(at);
    let conf = match at.is_dir() {
        true => at.join("mangosd.conf"),
        false => at,
    };
    conn::wow_patch_from_conf(conf).unwrap_or(conn::DEFAULT_WOW_PATCH)
}

/// Which maps have spawns, and how many of them the client can draw.
fn census(db: &mut Db, patch: u32) -> Result<(), String> {
    let totals = db.rows(
        "SELECT `map`, COUNT(*) AS `n`, COUNT(DISTINCT `id`) AS `kinds` \
         FROM `creature` GROUP BY `map` ORDER BY `n` DESC",
    )?;
    let templates = db.rows("SELECT COUNT(DISTINCT `entry`) AS `n` FROM `creature_template`")?;
    let entries = templates
        .first()
        .and_then(|row| row.integer("n"))
        .unwrap_or(0);
    println!("{entries} creature templates, spawned on {} maps\n", totals.len());
    println!("   map     spawns     kinds");
    let mut all = 0i64;
    for row in &totals {
        let map = row.integer("map").unwrap_or(-1);
        let n = row.integer("n").unwrap_or(0);
        all += n;
        println!("  {map:>4}  {n:>9}  {:>8}", row.integer("kinds").unwrap_or(0));
    }
    println!("\n  {all} spawns in all");

    // The one number the row counts cannot give: whether the join the server
    // makes at load actually lands. A spawn whose template row is missing at
    // this patch is a spawn vmangos drops with a DB error.
    let orphans = db.rows(&format!(
        "SELECT COUNT(*) AS `n` FROM `creature` c WHERE NOT EXISTS \
         (SELECT 1 FROM `creature_template` t WHERE t.`entry` = c.`id` AND t.`patch` <= {patch})"
    ))?;
    let orphans = orphans.first().and_then(|row| row.integer("n")).unwrap_or(0);
    match orphans {
        0 => println!("  every spawn has a template row at or below patch {patch}"),
        n => println!("  PROBLEM  {n} spawn(s) name a template with no row at or below patch {patch}"),
    }
    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// One map: what stands on it, and whether the client can draw it.
fn map(cfg: &Config, db: &mut Db, patch: u32, name: &str) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read(&vale_assets::tables::dbc::dbc_path("Map"))
        .map_err(|e| format!("Map.dbc: {e}"))?;
    let directories =
        vale_assets::tables::dbc::map_directories(&raw).map_err(|e| format!("Map.dbc: {e}"))?;
    let map_id = *directories
        .iter()
        .find(|(_, directory)| directory.eq_ignore_ascii_case(name))
        .map(|(id, _)| id)
        .ok_or_else(|| format!("no map directory called {name:?} in Map.dbc"))?;

    let tables = vale_assets::tables::dbc::DisplayTables::load(|table| {
        assets.read(&vale_assets::tables::dbc::dbc_path(table)).ok()
    })
    .map_err(|e| format!("the display tables: {e}"))?;

    let started = std::time::Instant::now();
    let rows = db.rows(&creature::spawns_on_map_query(map_id, patch))?;
    let read = started.elapsed();
    println!(
        "{name} (map {map_id}): {} spawn(s), read in {} ms\n",
        rows.len(),
        read.as_millis()
    );

    // Three ways the chain can end, counted separately: the server's join, the
    // display table's, and the archive's. Summing them would make one number out
    // of three different faults.
    //
    // **Resolved once per display id rather than once per spawn.** 24,610 spawns
    // share 3,355 creatures and far fewer models, and the archive question is
    // `exists` rather than `read`: reading each model whole took this command
    // from two seconds to over five minutes.
    let (mut no_template, mut no_display, mut no_model) = (0usize, 0usize, 0usize);
    let mut kinds: std::collections::BTreeMap<i64, (String, usize)> = std::collections::BTreeMap::new();
    let mut resolved: std::collections::HashMap<u32, Option<bool>> = std::collections::HashMap::new();
    for row in &rows {
        let entry = row.integer("id").unwrap_or(0);
        let Some(name) = row.text("name") else {
            no_template += 1;
            continue;
        };
        let count = kinds.entry(entry).or_insert_with(|| (name.to_string(), 0));
        count.1 += 1;
        let display = row.integer("display_id1").unwrap_or(0) as u32;
        let held = *resolved.entry(display).or_insert_with(|| {
            let model = tables.creature(display)?;
            Some(assets.exists(&model.path))
        });
        match held {
            None => no_display += 1,
            Some(false) => no_model += 1,
            Some(true) => {}
        }
    }

    println!("  {} distinct creature(s)", kinds.len());
    report("no creature_template row at this patch", no_template);
    report("no CreatureDisplayInfo row for display_id1", no_display);
    report("a model path the archives do not hold", no_model);

    println!("\n  the ten most spawned:");
    let mut by_count: Vec<(&i64, &(String, usize))> = kinds.iter().collect();
    by_count.sort_by_key(|(_, (_, count))| std::cmp::Reverse(*count));
    for (entry, (name, count)) in by_count.iter().take(10) {
        println!("    {entry:>7}  {count:>5}  {name}");
    }

    // Where they are, by tile, which is the number the editor's streaming
    // budget is decided by: a tool that draws the block around the camera
    // draws whatever the busiest tile in it holds.
    let mut per_tile: std::collections::HashMap<(u32, u32), usize> = std::collections::HashMap::new();
    for row in &rows {
        let (Some(x), Some(y)) = (row.number("position_x"), row.number("position_y")) else {
            continue;
        };
        *per_tile
            .entry(vale_assets::tile_for_position(x as f32, y as f32))
            .or_default() += 1;
    }
    let mut busiest: Vec<((u32, u32), usize)> = per_tile.into_iter().collect();
    busiest.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    println!("\n  the five busiest tiles:");
    for ((x, y), count) in busiest.iter().take(5) {
        println!("    {x:>2},{y:<2}  {count:>5} spawn(s)");
    }

    println!("\n  nothing was written: this command only reads");
    Ok(())
}

fn report(what: &str, count: usize) {
    match count {
        0 => println!("    none {what}"),
        n => println!("    {n:>5}  {what}"),
    }
}

/// One creature entry: the row the server would load, the model it draws as,
/// and where it stands.
fn one(cfg: &Config, db: &mut Db, patch: u32, entry: u32) -> Result<(), String> {
    let row = db
        .row(&creature::winning_template_query(entry, patch))?
        .ok_or_else(|| format!("no creature_template row for entry {entry} at or below patch {patch}"))?;

    let winning = row.integer("patch").unwrap_or(0);
    println!(
        "creature {entry} — {}  (patch {winning} of the rows at or below {patch})\n",
        row.text("name").unwrap_or("(no name)")
    );
    println!("  the key an edit is written under: {}", creature::template_key(entry, winning as u32).where_clause());
    println!();

    for column in creature::TEMPLATE_COLUMNS {
        // Three answers and not two. A column holding SQL NULL and a column the
        // table does not have at all read the same through `text`, and they are
        // different faults: `subname` is NULL for most creatures, where a column
        // missing from a `SELECT *` means this schema has drifted from vmangos'.
        let value = match (row.contains_key(column.name), row.text(column.name)) {
            (true, Some(value)) => named(&column, value),
            (true, None) => "NULL".to_string(),
            (false, _) => "(not a column of this table)".to_string(),
        };
        println!("  {:<28}  {value}", column.name);
    }

    // The client's half of the same row: what `display_id1` resolves to.
    let mut assets = open_assets(cfg)?;
    let tables = vale_assets::tables::dbc::DisplayTables::load(|table| {
        assets.read(&vale_assets::tables::dbc::dbc_path(table)).ok()
    })
    .map_err(|e| format!("the display tables: {e}"))?;
    let display = row.integer("display_id1").unwrap_or(0) as u32;
    println!("\n  display {display}:");
    match tables.creature(display) {
        Some(model) => {
            let held = assets.read(&model.path).is_ok();
            println!("    {}  {}", model.path, match held {
                true => "in the archives",
                false => "NOT in the archives",
            });
            println!("    scale {}", model.scale);
            for (slot, skin) in model.skins.iter().enumerate() {
                if !skin.is_empty() {
                    println!("    skin {slot}  {skin}");
                }
            }
        }
        None => println!("    no CreatureDisplayInfo row"),
    }

    // …and every spawn of it, which is the other direction of the same join.
    let spawns = db.rows(&format!(
        "SELECT `guid`, `map`, `position_x`, `position_y`, `position_z` FROM `creature` \
         WHERE `id` = {entry} OR `id2` = {entry} OR `id3` = {entry} OR `id4` = {entry} \
         OR `id5` = {entry} ORDER BY `guid`"
    ))?;
    println!("\n  {} spawn(s):", spawns.len());
    for spawn in spawns.iter().take(20) {
        println!(
            "    guid {:>8}  map {:>3}  {:.1}, {:.1}, {:.1}",
            spawn.integer("guid").unwrap_or(0),
            spawn.integer("map").unwrap_or(0),
            spawn.number("position_x").unwrap_or(0.0),
            spawn.number("position_y").unwrap_or(0.0),
            spawn.number("position_z").unwrap_or(0.0),
        );
    }
    if spawns.len() > 20 {
        println!("    …and {} more", spawns.len() - 20);
    }

    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// A column's value with its bits or its enumeration named, where the schema
/// names them. The number is kept beside the words, because the number is what
/// goes into a statement.
fn named(column: &creature::Column, value: &str) -> String {
    let Ok(number) = value.parse::<u32>() else {
        return value.to_string();
    };
    match column.kind {
        creature::Kind::Flags(bits) => {
            format!("{number:<12}  {}", creature::mask_words(bits, number))
        }
        creature::Kind::Choice(values) => {
            format!("{number:<12}  {}", creature::value_word(values, number))
        }
        _ => value.to_string(),
    }
}
