//! `vale itemtemplate` — what an item is to the server, and the half of it
//! the archives can settle.
//!
//! `item_template` has **no client file behind it at all**. `Item.dbc` is not
//! in the 1.12 archives — the client asks the server and is told — so this is
//! not `vale spelltemplate`'s join between a DBC and a table. There is
//! nothing to line the columns up against.
//!
//! What *can* be checked without a database is everything except the values,
//! and it is worth having because that is where this kind of schema goes wrong:
//!
//! * **the column list is vmangos' own, in its own order.** A column inserted
//!   in the middle by an upstream update makes every name after it describe the
//!   column before it, and an editor built on that writes the wrong column
//!   silently. `vale_mangos::item`'s own test pins the list; this prints it
//!   so a person can read it beside `ObjectMgr.cpp:3794`.
//! * **every enumeration is complete and has no repeated value**, which is what
//!   decides whether a form offers a real choice or a menu with two entries
//!   called the same thing.
//! * **every reference names a table that can be resolved**, and for the one
//!   that matters — `display_id` — the archives answer outright: the table is
//!   open, the row is there or it is not, and what it names either opens or
//!   does not.
//!
//! Three forms:
//!
//! ```text
//! vale itemtemplate            the 129 columns, the enumerations, the checks
//! vale itemtemplate 2589       one item, from the database if there is one
//! vale itemtemplate display    every display id the archives hold, censused
//! vale itemtemplate db         whether there is a world database at all
//! ```

use crate::common::{open_assets, open_display_tables};
use vale_config::Config;
use vale_mangos::item::{self, Kind, RowValue};

/// The schema, one item, the appearance census, or the connection.
pub fn cmd_itemtemplate(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    match which {
        Some("db") => database(),
        Some("display") => displays(cfg),
        Some(word) => match word.parse::<u32>() {
            Ok(entry) => one(cfg, entry),
            Err(_) => Err(format!(
                "usage: vale itemtemplate [<item entry> | display | db]  (not {word:?})"
            )),
        },
        None => census(cfg),
    }
}

/// The table, column by column, with the three checks the archives can make.
fn census(cfg: &Config) -> Result<(), String> {
    println!(
        "item_template: {} columns, keyed by (entry, patch)\n",
        item::TEMPLATE_COLUMNS.len()
    );
    println!("  col  column                          group          kind");
    for (index, column) in item::TEMPLATE_COLUMNS.iter().enumerate() {
        println!(
            "  {index:>3}  {:<30}  {:<13}  {}",
            column.name,
            column.group.name(),
            kind_word(column.kind)
        );
    }

    let mut problems = 0;

    // **The enumerations.** A repeated value is a menu with two entries that do
    // the same thing, and a form offering one is a form that cannot be trusted
    // about any of them.
    println!("\nthe enumerations:");
    for (name, values) in named_choices() {
        let mut seen: Vec<u32> = values.iter().map(|value| value.value).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        if seen.len() != before {
            println!("  PROBLEM  {name} names one value twice");
            problems += 1;
        }
        println!("  {:<22} {:>3} value(s)", name, values.len());
    }

    // …and the subclass lists, which are the one enumeration that is a
    // function rather than an array.
    println!("\nthe subclass lists, one per class:");
    for class in item::CLASSES {
        let list = item::subclasses(class.value);
        if list.is_empty() {
            println!("  PROBLEM  class {} has no subclass list", class.name);
            problems += 1;
            continue;
        }
        println!("  {:<14} {:>3} subclass(es)", class.name, list.len());
    }

    // **The references.** Each names a table; which of them the archives can
    // answer is the useful half, because a form resolves those to names.
    println!("\nwhat the columns point at:");
    let mut refs: Vec<(&str, Vec<&str>)> = Vec::new();
    for column in item::TEMPLATE_COLUMNS.iter() {
        if let Kind::Ref(table) = column.kind {
            match refs.iter_mut().find(|(name, _)| *name == table) {
                Some((_, columns)) => columns.push(column.name),
                None => refs.push((table, vec![column.name])),
            }
        }
    }
    for (table, columns) in &refs {
        println!("  {:<20} {}", table, columns.join(", "));
    }

    // **And the one the archives settle**: `ItemDisplayInfo.dbc` is open here,
    // so "is a display id a row" is a question with an answer rather than a
    // convention.
    match open_assets(cfg) {
        Ok(mut assets) => {
            let tables = open_display_tables(&mut assets)?;
            match tables.items() {
                Some(displays) => println!(
                    "\nItemDisplayInfo.dbc: {} row(s) \u{2014} what display_id points at",
                    displays.len()
                ),
                None => {
                    println!("\n  PROBLEM  ItemDisplayInfo.dbc did not open");
                    problems += 1;
                }
            }
        }
        // The schema half of this command needs no archives at all, so a
        // machine without them still gets the columns and the enumerations.
        Err(e) => println!("\n  (no archives: {e})"),
    }

    println!();
    match problems {
        0 => println!("no problems"),
        n => println!("{n} problem(s)"),
    }
    Ok(())
}

/// **Every display id the archives hold**, and which kind of item each is.
///
/// The picker's own question asked from the command line: how many appearances
/// carry geometry the wearer hangs on, how many only paint the wearer's skin,
/// and how many name neither — which is the population a grid of pictures has
/// to draw, and the reason a cell is a model or an icon rather than always one
/// of the two.
fn displays(cfg: &Config) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let displays = tables
        .items()
        .ok_or_else(|| "ItemDisplayInfo.dbc did not open".to_string())?;

    let ids = displays.ids();
    let (mut with_models, mut painted_only, mut icon_only, mut no_icon) = (0, 0, 0, 0);
    for id in &ids {
        let Some(appearance) = displays.appearance(*id, 0) else {
            continue;
        };
        let paints = appearance.textures.iter().any(|path| !path.is_empty());
        match (appearance.has_geometry(), paints) {
            (true, _) => with_models += 1,
            (false, true) => painted_only += 1,
            (false, false) => icon_only += 1,
        }
        if displays.inventory_icon(*id).is_none() {
            no_icon += 1;
        }
    }
    println!("ItemDisplayInfo.dbc: {} row(s)", ids.len());
    println!("  {with_models:>6}  hang a model on the wearer \u{2014} drawn as a model");
    println!("  {painted_only:>6}  paint the wearer's skin only \u{2014} drawn as an icon");
    println!("  {icon_only:>6}  name neither \u{2014} an icon and nothing else");
    println!("  {no_icon:>6}  carry no icon at all \u{2014} an empty square");

    // **What the editor's slot filter leaves**, which is the number that says
    // whether a picker over this table is usable at all: 29,604 rows at ninety-
    // six a page is three hundred pages, and nobody chooses an appearance that
    // way. The rule is the editor's `tools::items::fits_slot`, counted here
    // over the whole table.
    // **Which directory each row's model is actually in**, which is what makes
    // the geometry slots a real narrowing rather than "everything with a
    // model": a helm and a sword both carry one, and the row does not say which
    // it is. The archives do — `Item\ObjectComponents\<slot>\` holds the file —
    // so this is one listing and a hash lookup per row.
    let held = object_components(&mut assets);
    let mut placed = 0usize;
    let mut unplaced = 0usize;
    let facts: Vec<(u32, Option<&'static str>, bool, [bool; 8])> = ids
        .iter()
        .filter_map(|id| {
            let appearance = displays.appearance(*id, 0)?;
            let directory = directory_of(&held, &appearance.models[0]);
            // A cloak is the wearer's own cape geoset with a texture the item
            // names: no model, and `Cape\` holds no `.m2` at all.
            let cloak = !appearance.model_textures[0].is_empty() && !appearance.has_geometry();
            if appearance.has_geometry() {
                match directory.is_some() {
                    true => placed += 1,
                    false => unplaced += 1,
                }
            }
            let mut paints = [false; 8];
            for (at, path) in appearance.textures.iter().enumerate() {
                paints[at] = !path.is_empty();
            }
            Some((*id, directory, cloak, paints))
        })
        .collect();
    println!(
        "\n  of the {} rows that carry a model, {placed} are in a directory the \
         archives hold and {unplaced} are in none",
        placed + unplaced
    );
    for (directory, files) in &held {
        println!("    {:<10} {:>5} file(s)", directory, files.len());
    }

    println!("\n  what `fits this slot` leaves, per slot:");
    for worn in item::INVENTORY_TYPES {
        let slot = slot_of(worn.value);
        let left = facts
            .iter()
            .filter(|(_, directory, cloak, paints)| fits_slot(*directory, *cloak, paints, slot))
            .count();
        let pages = left.div_ceil(96);
        println!("    {:<18} {left:>6}  ({pages} page(s) of 96)", worn.name);
    }
    Ok(())
}

/// The five directories a worn model can be in, and what each holds.
///
/// Lower case with the extension off, which is what a row's `modelName` is once
/// `m2::model_path` has turned `.mdx` into `.m2`.
fn object_components(
    assets: &mut vale_assets::Assets,
) -> Vec<(&'static str, std::collections::HashSet<String>)> {
    const DIRECTORIES: [&str; 5] = ["Head", "Shoulder", "Weapon", "Shield", "Cape"];
    let listed = assets.list_prefix("item\\objectcomponents\\");
    let mut out: Vec<(&'static str, std::collections::HashSet<String>)> = DIRECTORIES
        .iter()
        .map(|directory| (*directory, std::collections::HashSet::new()))
        .collect();
    for path in listed {
        if !path.ends_with(".m2") {
            continue;
        }
        let Some(rest) = path.strip_prefix("item\\objectcomponents\\") else {
            continue;
        };
        let Some((directory, file)) = rest.split_once('\\') else {
            continue;
        };
        if let Some((_, held)) = out
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(directory))
        {
            held.insert(file.trim_end_matches(".m2").to_string());
        }
    }
    out
}

/// Which of them holds one row's model, trying the sixteen per-race cuts a
/// helmet ships as.
fn directory_of(
    held: &[(&'static str, std::collections::HashSet<String>)],
    model: &str,
) -> Option<&'static str> {
    if model.is_empty() {
        return None;
    }
    let stem = vale_assets::world::m2::model_path(model)
        .trim_end_matches(".m2")
        .to_ascii_lowercase();
    for (directory, files) in held {
        if files.contains(&stem) {
            return Some(directory);
        }
    }
    for race in 1..=8u8 {
        let Some(code) = vale_assets::tables::item::race_code(race) else {
            continue;
        };
        for sex in ['m', 'f'] {
            let cut = format!("{stem}_{code}{sex}");
            for (directory, files) in held {
                if files.contains(&cut) {
                    return Some(directory);
                }
            }
        }
    }
    None
}

/// Which slot an inventory type's appearance is chosen for.
///
/// **Written twice on purpose.** `vale-ide` has the same six lines in
/// `tools::items::slot_of`, and the CLI does not call it: that crate is a Bevy
/// application, and depending on it to reach a `match` would put five hundred
/// crates behind `vale itemtemplate`. What keeps the two honest is that both
/// are transcriptions of one list in `vale_assets::tables::item::Slot` — the
/// weapons, which `from_inventory_type` deliberately leaves out because which
/// hand a weapon goes in is the equipment slot rather than the item's type —
/// and that the editor's copy has a test naming every inventory type on it.
fn slot_of(inventory_type: u32) -> vale_assets::tables::item::Slot {
    use vale_assets::tables::item::Slot;
    match inventory_type {
        13 | 15 | 17 | 21 | 25 | 26 => Slot::MainHand,
        22 => Slot::OffHand,
        14 | 23 => Slot::Shield,
        other => Slot::from_inventory_type(other),
    }
}

/// …and whether one appearance could be worn in one slot, on the same terms.
fn fits_slot(
    directory: Option<&'static str>,
    cloak: bool,
    paints: &[bool; 8],
    slot: vale_assets::tables::item::Slot,
) -> bool {
    if slot.is_cloak() {
        return cloak;
    }
    if let Some(wanted) = slot.object_directory() {
        return match directory {
            Some(had) => had == wanted,
            // A model the archives cannot place falls back to the loose test,
            // which is the editor's own rule: a row whose file is missing is
            // still offered rather than disappearing from every slot.
            None => false,
        };
    }
    let wanted = slot.components();
    if wanted.is_empty() {
        return true;
    }
    wanted.iter().all(|component| paints[component.index()])
}

/// One item: its row as the server would load it, and the statements an edit
/// becomes.
fn one(cfg: &Config, entry: u32) -> Result<(), String> {
    use vale_mangos::conn::{Db, Where};

    let mut assets = open_assets(cfg).ok();
    let Some(at) = Where::find() else {
        println!("item {entry}: there is no world database to read it from.");
        println!("{}", Where::absent());
        println!("\nWhat this command can say without one is the schema, which is");
        println!("`vale itemtemplate` with no argument.");
        return Ok(());
    };
    println!("world database: {}", at.line());
    let mut db = Db::open(&at)?;
    let patch = wow_patch();
    let Some(row) = db.row(&item::winning_template_query(entry, patch))? else {
        return Err(format!(
            "item_template has no row for entry {entry} at or below patch {patch}"
        ));
    };
    let at_patch = row.integer("patch").unwrap_or(0) as u32;
    println!(
        "\n{} ({entry}) \u{2014} patch {at_patch} of {patch}\n",
        row.text("name").unwrap_or("?")
    );

    // **Group by group rather than in the table's order**, because a group's
    // columns are not contiguous in it: `quality` sits between `display_id` and
    // `flags`, so walking the table and printing a heading whenever the group
    // changed printed `[Identity]` four times for one item.
    let class = row.integer("class").unwrap_or(0) as u32;
    let mut seen: Vec<item::Group> = Vec::new();
    for column in item::TEMPLATE_COLUMNS.iter() {
        if !seen.contains(&column.group) {
            seen.push(column.group);
        }
    }
    for group in seen {
        let columns: Vec<&item::Column> = item::TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.group == group)
            .filter(|column| {
                // The zeros are most of a row and say nothing; the key is
                // printed whatever it holds.
                let Some(value) = RowValue::text(&row, column.name) else {
                    return false;
                };
                column.kind == Kind::Key
                    || (value != "0" && !value.is_empty() && value != "-1")
            })
            .collect();
        if columns.is_empty() {
            continue;
        }
        println!("  [{}]", group.name());
        for column in columns {
            let value = RowValue::text(&row, column.name).unwrap_or_default();
            println!(
                "    {:<30} {:<12} {}",
                column.name,
                value,
                means(column.kind, value, class)
            );
        }
    }

    // …and what the appearance it names is, which is the archives' half.
    let display_id = row.integer("display_id").unwrap_or(0) as u32;
    let inventory_type = row.integer("inventory_type").unwrap_or(0) as u32;
    if let (Some(assets), true) = (assets.as_mut(), display_id != 0) {
        let tables = open_display_tables(assets)?;
        match tables.items().and_then(|displays| {
            displays
                .appearance(display_id, 0)
                .map(|appearance| (displays.inventory_icon(display_id), appearance))
        }) {
            Some((icon, appearance)) => {
                println!("\n  [Appearance]");
                println!(
                    "    icon                           {}",
                    icon.unwrap_or_else(|| "none".into())
                );
                // **The weapons are not in `from_inventory_type`**: which hand
                // a weapon goes in is the equipment slot rather than the item's
                // type, so a caller has to choose one — see [`slot_of`].
                let slot = slot_of(inventory_type);
                for attached in appearance.attachments(slot, 1, 0) {
                    println!("    model                          {}", attached.path);
                }
                let painted = appearance
                    .textures
                    .iter()
                    .filter(|path| !path.is_empty())
                    .count();
                println!("    body textures                  {painted}");
            }
            None => println!("\n  [Appearance]  display {display_id} is in no row of ItemDisplayInfo.dbc"),
        }
    }

    // **And the statements an edit becomes**, which is the whole of what the
    // editor writes — printed here so the shape can be read with no window.
    let key = item::template_key(entry, at_patch);
    let changes = vec![vale_mangos::row::Assignment {
        column: "quality",
        value: "4".to_string(),
    }];
    println!("\n  an edit to `quality` would be:");
    for statement in item::statements(
        item::TEMPLATE,
        &key,
        vale_mangos::row::Life::Update,
        &changes,
    ) {
        println!("    {statement}");
    }
    println!("\n  nothing was written: this command only reads");
    Ok(())
}

/// Whether there is a world database, and what it holds of this table.
fn database() -> Result<(), String> {
    use vale_mangos::conn::{Db, Where};

    let at = Where::find().ok_or_else(Where::absent)?;
    println!("world database: {}", at.line());
    let mut db = Db::open(&at)?;

    for (what, sql) in [
        ("rows", "SELECT COUNT(*) AS n FROM `item_template`".to_string()),
        (
            "entries",
            "SELECT COUNT(DISTINCT `entry`) AS n FROM `item_template`".to_string(),
        ),
        ("the highest entry", item::MAX_ENTRY_QUERY.replace("`entry`) AS `entry`", "`entry`) AS n")),
        (
            "at or above the reserved base (this editor's own)",
            format!(
                "SELECT COUNT(*) AS n FROM `item_template` WHERE `entry` >= {}",
                item::RESERVED_ENTRY_BASE
            ),
        ),
    ] {
        let row = db.row(&sql)?;
        let n = row
            .and_then(|row| row.get("n").cloned().flatten())
            .unwrap_or_else(|| "?".into());
        println!("  {n:>10}  {what}");
    }
    println!();
    println!("  nothing was written: this command only reads");
    Ok(())
}

/// The content patch the row is read at.
///
/// `crate::spawns`' own helper, and the same sentence: `VALE_MANGOSD` points
/// at the conf that carries it, and with only `VALE_WORLDDB` set there is no
/// conf to read and the default stands.
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

/// What a column's kind is called in the census.
fn kind_word(kind: Kind) -> String {
    kind.describe()
}

/// …and what one value of it means, for the one-item form.
fn means(kind: Kind, value: &str, class: u32) -> String {
    use vale_mangos::schema;
    let number: i64 = value.trim().parse().unwrap_or(0);
    match kind {
        Kind::Choice(values) => schema::value_word(values, number as u32),
        // The one column whose list is another column's: 7 is *Sword* on a
        // weapon and *Cloth* on a piece of armour.
        Kind::Subclass => schema::value_word(item::subclasses(class), number as u32),
        Kind::Flags(bits) => schema::mask_words(bits, number as u32),
        Kind::Money => schema::money_words(number.max(0) as u64),
        Kind::Millis => schema::millis_words(number),
        Kind::Seconds => schema::seconds_words(number, 0),
        _ => String::new(),
    }
}

/// Every enumeration the schema names, for the census.
fn named_choices() -> Vec<(&'static str, &'static [vale_mangos::schema::Value])> {
    vec![
        ("QUALITIES", &item::QUALITIES),
        ("CLASSES", &item::CLASSES),
        ("INVENTORY_TYPES", &item::INVENTORY_TYPES),
        ("BONDING", &item::BONDING),
        ("SHEATH_TYPES", &item::SHEATH_TYPES),
        ("STAT_TYPES", &item::STAT_TYPES),
        ("SPELL_TRIGGERS", &item::SPELL_TRIGGERS),
        ("DAMAGE_SCHOOLS", &item::DAMAGE_SCHOOLS),
        ("BAG_FAMILIES", &item::BAG_FAMILIES),
        ("REPUTATION_RANKS", &item::REPUTATION_RANKS),
        ("FOOD_TYPES", &item::FOOD_TYPES),
        ("PAGE_MATERIALS", &item::PAGE_MATERIALS),
        ("LANGUAGES", &item::LANGUAGES),
        ("MATERIALS", &item::MATERIALS),
    ]
}
