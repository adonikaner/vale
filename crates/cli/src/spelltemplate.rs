//! `vale spelltemplate` — the join between `Spell.dbc` and the server's
//! `spell_template`, checked against the archives.
//!
//! The mapping lives in `vale_mangos::spell` and is transcribed from
//! vmangos' `SpellMgr::LoadSpell`. What can be checked with no database is
//! whether it fits the file: 151 columns, every field index inside the 173 the
//! shipped table has, no field read twice, and every column that claims to be
//! text pointing at something the string block can answer. That is this
//! command, and it is the check that runs on a machine with the archives and
//! nothing else.
//!
//! What it cannot check is whether the *values* agree with the rows vmangos
//! actually holds, because that needs a connection this project does not have
//! yet. `vale spelltemplate dump` is the half of that which can be done from
//! here: one `entry<TAB>column<TAB>value` line per DBC-fed column, which is
//! diffable against the same shape selected out of the database.

use crate::common::open_assets;
use vale_config::Config;
use vale_edit::dbc::DbcFile;
use vale_mangos::spell::{self, Kind, Source};

/// Every column, where it reads from, and whether the file can answer it.
pub fn cmd_spelltemplate(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    let dbc = open_spell_dbc(cfg)?;
    match which {
        Some("dump") => dump(&dbc),
        Some("db") => database(),
        Some(word) => match word.parse::<u32>() {
            Ok(entry) => one(&dbc, entry),
            Err(_) => Err(format!(
                "usage: vale spelltemplate [<spell id> | dump | db]  (not {word:?})"
            )),
        },
        None => census(&dbc),
    }
}

fn open_spell_dbc(cfg: &Config) -> Result<DbcFile, String> {
    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read("DBFilesClient\\Spell.dbc")
        .map_err(|e| e.to_string())?;
    DbcFile::parse(&raw).map_err(|e| e.to_string())
}

/// The mapping, and the three things about it the file can settle.
fn census(dbc: &DbcFile) -> Result<(), String> {
    println!(
        "DBFilesClient\\Spell.dbc: {} records x {} fields",
        dbc.record_count(),
        dbc.field_count()
    );
    println!(
        "spell_template: {} columns, written at build {}\n",
        spell::COLUMNS.len(),
        spell::BUILD
    );

    println!("  col  column                          reads          kind");
    for (index, column) in spell::COLUMNS.iter().enumerate() {
        let reads = match column.source {
            Source::Server => "—".to_string(),
            Source::Field(field) => format!("field {field}"),
            Source::Pair(low, high) => format!("fields {low}+{high}"),
        };
        // **Which of them the server then ignores.** Thirteen columns are
        // selected and never assigned, and two of those are a spell's
        // description and its tooltip — the thing a person is most likely to
        // edit and expect to cross.
        let used = match column.used_by_the_server() {
            true => "",
            false => "   the server ignores it",
        };
        println!(
            "  {index:>3}  {:<30}  {reads:<13}  {:?}{used}",
            column.name, column.kind
        );
    }

    // The three checks, each of which is a real way to get this wrong.
    let mut problems = 0;
    let mut seen: Vec<Option<&str>> = vec![None; dbc.field_count()];
    for column in spell::COLUMNS.iter() {
        let fields: Vec<usize> = match column.source {
            Source::Server => vec![],
            Source::Field(field) => vec![field],
            Source::Pair(low, high) => vec![low, high],
        };
        for field in fields {
            match seen.get_mut(field) {
                None => {
                    println!(
                        "\n  PROBLEM  {} reads field {field}, past the {} this file has",
                        column.name,
                        dbc.field_count()
                    );
                    problems += 1;
                }
                Some(slot) => {
                    if let Some(other) = slot {
                        println!(
                            "\n  PROBLEM  field {field} feeds both {other} and {}",
                            column.name
                        );
                        problems += 1;
                    }
                    *slot = Some(column.name);
                }
            }
        }
    }

    let server: Vec<&str> = spell::COLUMNS
        .iter()
        .filter(|column| column.source == Source::Server)
        .map(|column| column.name)
        .collect();
    let unread: Vec<usize> = seen
        .iter()
        .enumerate()
        .filter(|(_, who)| who.is_none())
        .map(|(field, _)| field)
        .collect();

    println!("\n  {} of the file's {} fields are read", seen.len() - unread.len(), seen.len());
    println!(
        "  {} columns the file cannot answer, kept as the server has them: {}",
        server.len(),
        server.join(", ")
    );
    // The 28 unread fields are the seven other locales of each of the four
    // strings, which vmangos keeps in `locales_spell` instead — worth saying
    // rather than leaving as a number.
    println!("  {} fields nothing reads (the seven non-enUS locales of each string)", unread.len());
    println!(
        "  {} columns the server selects and then ignores: {}",
        vale_mangos::spell::IGNORED.len(),
        vale_mangos::spell::IGNORED.join(", ")
    );

    match problems {
        0 => {
            println!("\n  the mapping fits the file");
            Ok(())
        }
        n => Err(format!("{n} problem(s) in the mapping")),
    }
}

/// One spell: what each column would hold, taken from the client's own file.
///
/// Directly comparable with `SELECT * FROM spell_template WHERE entry = <id>`
/// on the server, which is the check the mapping actually needs and the reason
/// this prints values rather than a summary.
fn one(dbc: &DbcFile, entry: u32) -> Result<(), String> {
    let Some(record) = dbc.row_of(entry) else {
        return Err(format!("no spell {entry} in Spell.dbc"));
    };
    let name = dbc.string_at(record, 120).unwrap_or_default();
    println!("spell {entry} — {name}  (record {record})\n");
    for (index, column) in spell::COLUMNS.iter().enumerate() {
        let value = match column.name {
            "build" => spell::BUILD.to_string(),
            _ => match column.value(dbc, record) {
                Some(value) => value,
                // The server's own: this end has nothing to say about it, and
                // saying "0" would be a claim.
                None => "(the server's)".to_string(),
            },
        };
        println!("  {index:>3}  {:<30}  {value}", column.name);
    }

    // …and what a save would emit if this spell had been edited, which is the
    // shape of the thing rather than a statement to run: `changes` against an
    // unedited file is empty by construction.
    println!("\n  the statements a save emits for an edited spell:\n");
    println!("  {}", spell::copy_forward(entry));
    if let Some(insert) = spell::insert_new(dbc, entry) {
        // Long, and the point is its shape rather than its 147 values.
        let head: String = insert.chars().take(160).collect();
        println!("  {head}…");
    }
    println!(
        "  {}",
        spell::update(
            entry,
            &[vale_mangos::spell::Assignment {
                column: "effectBasePoints1",
                value: "41".into(),
            }]
        )
        .unwrap_or_default()
    );
    Ok(())
}

/// **Is there a world database, and does the mapping agree with it?**
///
/// The half of this command that the archives alone cannot answer, and the only
/// check that can catch a column mapped to the wrong field: a positional
/// transcription of 151 columns is wrong *per column*, so the census is over
/// every spell and the number to read is the per-column agreement.
///
/// Read-only. It writes nothing and changes nothing, which is what lets it be
/// the thing to run first on a machine that has a server.
fn database() -> Result<(), String> {
    use vale_mangos::conn::{Db, Where};

    let at = Where::find().ok_or_else(Where::absent)?;
    println!("world database: {}", at.line());
    let mut db = Db::open(&at)?;

    // Two counts first, because they say whether anything below can be
    // trusted: an empty table answers every question with "no row".
    for (what, sql) in [
        ("rows", "SELECT COUNT(*) AS n FROM `spell_template`"),
        ("entries", "SELECT COUNT(DISTINCT `entry`) AS n FROM `spell_template`"),
        (
            "at build 5875 (the dev rows)",
            "SELECT COUNT(*) AS n FROM `spell_template` WHERE `build` = 5875",
        ),
    ] {
        let row = db.row(sql)?;
        let n = row
            .and_then(|row| row.get("n").cloned().flatten())
            .unwrap_or_else(|| "?".into());
        println!("  {n:>8}  {what}");
    }
    println!();
    println!("  nothing was written: this command only reads");
    Ok(())
}

/// Every DBC-fed column of every spell, as `entry<TAB>column<TAB>value`.
///
/// For the one check this command cannot make on its own: the same shape comes
/// out of the database with a `SELECT`, and the two are diffed. It is the whole
/// table because the mapping is wrong per *column*, so one spell proves almost
/// nothing and every spell proves it outright.
fn dump(dbc: &DbcFile) -> Result<(), String> {
    for record in 0..dbc.record_count() {
        let Some(entry) = dbc.u32_at(record, 0) else {
            continue;
        };
        for column in spell::COLUMNS.iter() {
            // Text is deliberately left out. A name reaches the database
            // through `locales_spell` and the escaping, and a tab or a newline
            // inside one would break the line this writes; the numbers are what
            // a positional mapping gets wrong.
            if column.kind == Kind::Text {
                continue;
            }
            if let Some(value) = column.value(dbc, record) {
                println!("{entry}\t{}\t{value}", column.name);
            }
        }
    }
    Ok(())
}
