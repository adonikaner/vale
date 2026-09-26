//! `vale questtemplate` — what a quest is to the server: `quest_template`'s
//! 131 columns, and the four relation tables that say who gives and takes it.
//!
//! A quest has no client file behind it, so, as with `vale itemtemplate`,
//! what can be checked without a database is the schema — the column list in
//! `LoadQuests`' own order, the enumerations, and which table each reference
//! names. With a database it checks the values as well, and it is the only
//! place the queries `vale_mangos::quest` builds are run outside the editor.
//!
//! ```text
//! vale questtemplate        the columns by group, the enumerations, the checks
//! vale questtemplate 783    one quest: its row, what each id resolves to, who
//!                              gives and takes it, and the statements an edit and
//!                              a removal become
//! vale questtemplate db     the table's counts, the relation tables', and the
//!                              three censuses the editor's rules rest on
//! ```

use vale_config::Config;
use vale_mangos::conn::{Db, Where};
use vale_mangos::quest::{self, Group, Kind, RowValue};
use vale_mangos::row::{Assignment, Life};

/// The schema, one quest, or the database's own counts.
pub fn cmd_questtemplate(_cfg: &Config, which: Option<&str>) -> Result<(), String> {
    match which {
        Some("db") => database(),
        Some(word) => match word.parse::<u32>() {
            Ok(entry) => one(entry),
            Err(_) => Err(format!(
                "usage: vale questtemplate [<quest entry> | db]  (not {word:?})"
            )),
        },
        None => census(),
    }
}

/// The table, group by group, and the checks that need no database.
fn census() -> Result<(), String> {
    println!(
        "quest_template: {} columns, keyed by (entry, patch)\n",
        quest::TEMPLATE_COLUMNS.len()
    );
    let mut groups: Vec<Group> = Vec::new();
    for column in quest::TEMPLATE_COLUMNS.iter() {
        if !groups.contains(&column.group) {
            groups.push(column.group);
        }
    }
    for group in &groups {
        let columns: Vec<_> = quest::TEMPLATE_COLUMNS
            .iter()
            .enumerate()
            .filter(|(_, column)| column.group == *group)
            .collect();
        println!("  {} ({})", group.name(), columns.len());
        for (index, column) in columns {
            println!("    {index:>3}  {:<26} {}", column.name, column.kind.describe());
        }
    }

    let mut problems = 0usize;
    println!("\n  enumerations");
    for (name, values) in [("Method", &quest::METHODS[..]), ("Type", &quest::TYPES[..])] {
        let mut seen: Vec<u32> = Vec::new();
        for value in values {
            if seen.contains(&value.value) {
                println!("    {name}: {} is named twice", value.value);
                problems += 1;
            }
            seen.push(value.value);
        }
        println!("    {name:<14} {} value(s)", values.len());
    }
    for (name, bits) in [("QuestFlags", &quest::FLAGS[..]), ("SpecialFlags", &quest::SPECIAL_FLAGS[..])] {
        let mut all = 0u32;
        for bit in bits {
            if all & bit.bit != 0 {
                println!("    {name}: bit {:#x} is named twice", bit.bit);
                problems += 1;
            }
            all |= bit.bit;
        }
        println!("    {name:<14} {} bit(s), {all:#06x} named", bits.len());
    }

    println!("\n  references");
    let mut tables: Vec<(String, usize)> = Vec::new();
    for column in quest::TEMPLATE_COLUMNS.iter() {
        let named: Vec<&str> = match column.kind {
            Kind::Ref(table) => vec![table],
            Kind::Either(positive, negative) => vec![positive, negative],
            _ => Vec::new(),
        };
        for table in named {
            match tables.iter_mut().find(|(had, _)| had == table) {
                Some((_, count)) => *count += 1,
                None => tables.push((table.to_string(), 1)),
            }
        }
    }
    for (table, count) in &tables {
        println!("    {table:<22} {count} column(s)");
    }

    println!("\n  relation tables, keyed by (id, quest) and filtered by patch_min..patch_max");
    for table in quest::RELATIONS {
        println!("    {table}");
    }
    println!("\n  removed with a quest");
    for (table, column) in quest::DEPENDENTS {
        println!("    {table}.{column}");
    }
    match problems {
        0 => println!("\n  no problems"),
        n => println!("\n  {n} problem(s)"),
    }
    Ok(())
}

/// One quest, read from the database.
fn one(entry: u32) -> Result<(), String> {
    let Some(at) = Where::find() else {
        println!("quest {entry}: there is no world database to read it from.");
        println!("{}", Where::absent());
        return Ok(());
    };
    println!("world database: {}", at.line());
    let mut db = Db::open(&at)?;
    let patch = wow_patch();
    let Some(row) = db.row(&quest::winning_template_query(entry, patch))? else {
        return Err(format!(
            "quest_template has no row for entry {entry} at or below patch {patch}"
        ));
    };
    let at_patch = row.integer("patch").unwrap_or(0) as u32;
    println!(
        "\nquest {entry} at patch {at_patch} (the server is at {patch}): {}",
        row.text("Title").unwrap_or("")
    );

    // The ids the row names, resolved in two queries rather than one per column.
    let mut items: Vec<u32> = Vec::new();
    let mut creatures: Vec<u32> = Vec::new();
    let mut objects: Vec<u32> = Vec::new();
    for column in quest::TEMPLATE_COLUMNS.iter() {
        let value = row.integer(column.name).unwrap_or(0);
        match column.kind {
            Kind::Ref(table) if table == vale_mangos::item::TEMPLATE && value > 0 => {
                items.push(value as u32)
            }
            Kind::Either(_, "gameobject_template") if value > 0 => creatures.push(value as u32),
            Kind::Either(_, "gameobject_template") if value < 0 => objects.push(-value as u32),
            _ => {}
        }
    }
    let mut names: std::collections::HashMap<(u8, u32), String> = Default::default();
    for (which, sql) in [
        (0u8, quest::item_names_query(&items, patch)),
        (1, quest::holder_names_query(true, &creatures, patch)),
        (2, quest::holder_names_query(false, &objects, patch)),
    ] {
        let Some(sql) = sql else { continue };
        for found in db.rows(&sql)? {
            if let (Some(id), Some(name)) = (found.integer("entry"), found.text("name")) {
                names.insert((which, id as u32), name.to_string());
            }
        }
    }

    // By group rather than by position: the table's order is the `Quest`
    // constructor's, which selects `MaxLevel` and `RewXP` a hundred columns
    // after the ones they belong beside.
    let mut groups: Vec<Group> = Vec::new();
    for column in quest::TEMPLATE_COLUMNS.iter() {
        if !groups.contains(&column.group) {
            groups.push(column.group);
        }
    }
    let ordered = groups.iter().flat_map(|group| {
        quest::TEMPLATE_COLUMNS
            .iter()
            .filter(move |column| column.group == *group)
    });
    let mut group = None;
    for column in ordered {
        let Some(value) = row.text(column.name) else {
            continue;
        };
        // A quest is mostly zeros; the ones that say something are what is read.
        if value == "0" || value.is_empty() {
            continue;
        }
        if group != Some(column.group) {
            group = Some(column.group);
            println!("\n  {}", column.group.name());
        }
        let number: i64 = value.trim().parse().unwrap_or(0);
        let means = match column.kind {
            Kind::Choice(values) => quest::value_word(values, number as u32),
            Kind::Flags(bits) => quest::mask_words(bits, number as u32),
            Kind::Money => quest::money_words(number.max(0) as u64),
            Kind::SignedMoney => match number < 0 {
                true => format!("costs {}", quest::money_words(number.unsigned_abs())),
                false => format!("pays {}", quest::money_words(number as u64)),
            },
            Kind::Seconds => quest::seconds_words(number, 0),
            Kind::Ref(table) if table == vale_mangos::item::TEMPLATE => names
                .get(&(0, number as u32))
                .cloned()
                .unwrap_or_else(|| "no such item".to_string()),
            Kind::Either(_, "gameobject_template") => match number > 0 {
                true => names
                    .get(&(1, number as u32))
                    .map(|name| format!("creature: {name}"))
                    .unwrap_or_else(|| "no such creature".to_string()),
                false => names
                    .get(&(2, number.unsigned_abs() as u32))
                    .map(|name| format!("game object: {name}"))
                    .unwrap_or_else(|| "no such game object".to_string()),
            },
            Kind::Either(positive, negative) => match number > 0 {
                true => format!("{positive} {number}"),
                false => format!("{negative} {}", -number),
            },
            _ => String::new(),
        };
        let shown: String = value.chars().take(70).collect();
        let shown = shown.replace('\n', " ");
        match means.is_empty() {
            true => println!("    {:<26} {shown}", column.name),
            false => println!("    {:<26} {shown}  ({means})", column.name),
        }
    }

    println!("\n  who gives and takes it");
    for table in quest::RELATIONS {
        let sql = format!(
            "SELECT `id` FROM `{table}` WHERE `quest` = {entry} AND {patch} BETWEEN `patch_min` AND `patch_max`"
        );
        let ids: Vec<u32> = db
            .rows(&sql)?
            .iter()
            .filter_map(|found| found.integer("id"))
            .map(|id| id as u32)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let creatures = table.starts_with("creature");
        let mut words: Vec<String> = Vec::new();
        if let Some(sql) = quest::holder_names_query(creatures, &ids, patch) {
            for found in db.rows(&sql)? {
                words.push(format!(
                    "{} ({})",
                    found.text("name").unwrap_or("?"),
                    found.integer("entry").unwrap_or(0)
                ));
            }
        }
        println!("    {table:<28} {}", words.join(", "));
    }

    let key = quest::template_key(entry, at_patch);
    println!("\n  the statement an edit to its level becomes");
    for statement in quest::statements(
        quest::TEMPLATE,
        &key,
        Life::Update,
        &[Assignment { column: "QuestLevel", value: "60".to_string() }],
    ) {
        println!("    {statement}");
    }
    println!("  …and a removal, which names the entry alone so that no older version is left to load");
    for statement in quest::statements(quest::TEMPLATE, &key, Life::Delete, &[]) {
        println!("    {statement}");
    }
    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// The table's counts, and the censuses behind three of the editor's rules.
fn database() -> Result<(), String> {
    let at = Where::find().ok_or_else(Where::absent)?;
    println!("world database: {}", at.line());
    let mut db = Db::open(&at)?;
    let patch = wow_patch();

    let mut count = |what: &str, sql: String| -> Result<(), String> {
        let n = db
            .row(&sql)?
            .and_then(|row| row.get("n").cloned().flatten())
            .unwrap_or_else(|| "?".into());
        println!("  {n:>10}  {what}");
        Ok(())
    };
    count("rows", "SELECT COUNT(*) AS n FROM `quest_template`".into())?;
    count("entries", "SELECT COUNT(DISTINCT `entry`) AS n FROM `quest_template`".into())?;
    count(
        "entries with more than one content-patch version, which is why a removal names the entry alone",
        "SELECT COUNT(*) AS n FROM (SELECT `entry` FROM `quest_template` GROUP BY `entry` \
         HAVING COUNT(*) > 1) x"
            .into(),
    )?;
    count("the highest entry", "SELECT MAX(`entry`) AS n FROM `quest_template`".into())?;
    count(
        "at or above the reserved base (this editor's own)",
        format!(
            "SELECT COUNT(*) AS n FROM `quest_template` WHERE `entry` >= {}",
            quest::RESERVED_ENTRY_BASE
        ),
    )?;
    count(
        "filed under a QuestSort rather than an area (ZoneOrSort < 0)",
        "SELECT COUNT(*) AS n FROM `quest_template` WHERE `ZoneOrSort` < 0".into(),
    )?;
    count(
        "with a game object among their objectives (ReqCreatureOrGOId < 0)",
        "SELECT COUNT(*) AS n FROM `quest_template` WHERE `ReqCreatureOrGOId1` < 0 \
         OR `ReqCreatureOrGOId2` < 0 OR `ReqCreatureOrGOId3` < 0 OR `ReqCreatureOrGOId4` < 0"
            .into(),
    )?;
    println!();
    for table in quest::RELATIONS {
        count(table, format!("SELECT COUNT(*) AS n FROM `{table}`"))?;
    }

    // The queries the editor's tool runs, run once each so a syntax error is
    // found here rather than in a window.
    println!();
    let quests = db.rows(&quest::all_quests_query(patch))?.len();
    println!("  {quests:>10}  quests the server would load at patch {patch}");
    for table in quest::RELATIONS {
        let rows = db.rows(&quest::relations_query(table, patch))?.len();
        println!("  {rows:>10}  {table} rows inside their patch band");
    }
    for creatures in [true, false] {
        let rows = db.rows(&quest::relation_names_query(creatures, patch))?.len();
        let what = match creatures {
            true => "creatures",
            false => "game objects",
        };
        println!("  {rows:>10}  {what} that give or take a quest");
    }
    let found = db.rows(&quest::holder_search_query(true, "marshal", patch, 50))?.len();
    println!("  {found:>10}  creatures named like 'marshal' (capped at 50)");
    let found = db.rows(&quest::item_search_query("linen", patch, 50))?.len();
    println!("  {found:>10}  items named like 'linen' (capped at 50)");
    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// The content patch a row is read at — `crate::itemtemplate`'s own helper.
fn wow_patch() -> u32 {
    use vale_mangos::conn;
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
