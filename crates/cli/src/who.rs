//! `vale who` — what a typed `/who` line means, with no window and no
//! server.
//!
//! The check behind [`vale_assets::interface::whoquery`], and it exists for
//! the reason [`crate::reputation`]'s does: the parse is a **client-side rule**.
//! Nothing on the wire states it and no file states it either — `SendWho` hands
//! the C side one line of text and `CMSG_WHO` carries eight parsed fields, and
//! everything between the two is the client's own parse.
//!
//! A rule like that does not fail when it is wrong. It sends a search that comes
//! back with the wrong people in it, which is why this command prints the parse
//! rather than a pass or a fail.
//!
//! ## What the census reports
//!
//! * **The five `WHO_TAG_*` prefixes as the running interface spells them**,
//!   read out of `GlobalStrings.lua` rather than written down — the parser looks
//!   all five up through the string table, so a build with different letters has
//!   a different syntax.
//! * **How many rows each of the three joins has**: `AreaTable.dbc`'s zones
//!   (parent zero, and *not* its sub-areas — the number to watch, because a
//!   count near the full table means the zone filter has stopped being one),
//!   `ChrRaces.dbc` and `ChrClasses.dbc`.
//! * **A sweep of the shapes**, each parsed against the real tables and printed
//!   with the field it set: the four level ranges, a quoted zone against an
//!   unquoted one, a zone nobody has heard of, two races unioned, and a line
//!   with more terms than the server will take.
//!
//! ## …and one line traced
//!
//! `vale who 'z-"Elwynn Forest" 5-10 c-warrior'` prints that line's parse
//! field by field, with the zone ids resolved back to names — so a `z-` that
//! matched nothing is visible as the single zone `0` it really is rather than as
//! an empty filter.

use crate::common::*;
use vale_assets::interface::whoquery::{self, Lookups, Tags, WhoQuery};
use vale_config::Config;

/// The shapes the sweep prints, each one a rule of the client's parse.
const SHAPES: [(&str, &str); 11] = [
    ("", "an unadorned /who: no filter at all"),
    ("57", "a bare number is a range of one"),
    ("57-", "…and a trailing dash reopens the top"),
    ("5-10", "…and both ends"),
    ("-40", "…and only the top"),
    (r#"z-"Elwynn Forest""#, "a quoted zone survives its space"),
    ("z-Elwynn Forest", "…and an unquoted one loses its second word"),
    ("z-Nowhere", "a zone nobody knows is a filter matching nothing"),
    ("r-orc r-tauren", "two races are a union"),
    ("c-warrior n-Bram g-Watch", "the other three tags"),
    ("a b c d e f", "at most four terms are kept"),
];

pub fn cmd_who(cfg: &Config, line: Option<String>) -> Result<(), String> {
    use vale_assets::tables::area::Areas;
    use vale_assets::tables::charcreate::CharCreate;
    use vale_assets::tables::dbc::dbc_path;

    let mut assets = open_assets(cfg)?;
    let strings = assets
        .read("Interface\\FrameXML\\GlobalStrings.lua")
        .map_err(|e| e.to_string())
        .map(|raw| vale_assets::interface::strings::Strings::parse(&raw))?;
    let tags = Tags::from_strings(&strings);

    let raw = assets.read(&dbc_path("AreaTable")).map_err(|e| e.to_string())?;
    let areas = Areas::parse(&raw).ok_or("AreaTable.dbc did not parse")?;
    let create = CharCreate::load(|table| assets.read(&dbc_path(table)).ok());

    // Built once here rather than per line, which is the opposite of what the
    // renderer does — see `whoquery::Lookups`, where the reason a `/who` can
    // afford to rebuild them is written down.
    let zones: Vec<(u32, &str)> = areas.zones().map(|a| (a.id, a.name.as_str())).collect();
    let races: Vec<(u8, &str)> = create.races().iter().map(|r| (r.id, r.name.as_str())).collect();
    let classes: Vec<(u8, &str)> = create.every_class().map(|c| (c.id, c.name.as_str())).collect();
    let lookups = Lookups {
        zones: &zones,
        races: &races,
        classes: &classes,
    };
    let name_of = |id: u32| -> String {
        zones
            .iter()
            .find(|(zone, _)| *zone == id)
            .map_or_else(|| "(no such zone)".to_string(), |(_, name)| (*name).to_string())
    };

    // --- one line traced ---
    if let Some(line) = line {
        let query = whoquery::parse(&line, &tags, &lookups);
        println!("{line:?}\n");
        print_query(&query, &races, &classes, name_of);
        return Ok(());
    }

    println!("the five tags, as GlobalStrings.lua spells them:");
    for (key, value) in [
        ("WHO_TAG_NAME", &tags.name),
        ("WHO_TAG_GUILD", &tags.guild),
        ("WHO_TAG_ZONE", &tags.zone),
        ("WHO_TAG_RACE", &tags.race),
        ("WHO_TAG_CLASS", &tags.class),
    ] {
        println!("  {key:<14} {value:?}");
    }

    println!(
        "\n  {} areas, of which {} are zones — the zone tag joins against the second\n  \
         {} races, {} classes",
        areas.count(),
        zones.len(),
        races.len(),
        classes.len(),
    );

    println!("\nthe shapes, parsed against those tables:\n");
    for (line, why) in SHAPES {
        let query = whoquery::parse(line, &tags, &lookups);
        println!("  {:<28} {why}", format!("{line:?}"));
        println!("    {}", summary(&query, &races, &classes, name_of));
    }

    println!(
        "\n  the server refuses a request over {} zones or {} terms by not answering, so \
         both are cut here",
        whoquery::MAX_ZONES,
        whoquery::MAX_TERMS
    );
    Ok(())
}

/// One line of the sweep — everything the query filters on, and nothing it does
/// not.
fn summary(
    query: &WhoQuery,
    races: &[(u8, &str)],
    classes: &[(u8, &str)],
    name_of: impl Fn(u32) -> String + Copy,
) -> String {
    let mut parts = vec![format!("level {}..{}", query.level_min, query.level_max)];
    if !query.name.is_empty() {
        parts.push(format!("name {:?}", query.name));
    }
    if !query.guild.is_empty() {
        parts.push(format!("guild {:?}", query.guild));
    }
    if !query.zones.is_empty() {
        let named: Vec<String> = query
            .zones
            .iter()
            .map(|id| format!("{id} {}", name_of(*id)))
            .collect();
        parts.push(format!("zones [{}]", named.join(", ")));
    }
    if query.race_mask != whoquery::MASK_ALL {
        parts.push(format!("races [{}]", named_mask(query.race_mask, races)));
    }
    if query.class_mask != whoquery::MASK_ALL {
        parts.push(format!("classes [{}]", named_mask(query.class_mask, classes)));
    }
    if !query.terms.is_empty() {
        parts.push(format!("terms {:?}", query.terms));
    }
    parts.join("  ")
}

/// A mask as the names its bits stand for, or `nobody` for a mask with none —
/// which is a real and different answer from "every one of them", and the whole
/// point of printing it this way.
fn named_mask(mask: u32, rows: &[(u8, &str)]) -> String {
    let named: Vec<&str> = rows
        .iter()
        .filter(|(id, _)| mask & (1 << *id) != 0)
        .map(|(_, name)| *name)
        .collect();
    if named.is_empty() {
        return "nobody".to_string();
    }
    named.join(", ")
}

/// The traced form: every field on its own line, so a wrong one is obvious.
fn print_query(
    query: &WhoQuery,
    races: &[(u8, &str)],
    classes: &[(u8, &str)],
    name_of: impl Fn(u32) -> String + Copy,
) {
    println!("  level     {}..{}", query.level_min, query.level_max);
    println!("  name      {:?}", query.name);
    println!("  guild     {:?}", query.guild);
    println!(
        "  races     0x{:08x}  {}",
        query.race_mask,
        if query.race_mask == whoquery::MASK_ALL {
            "every one".to_string()
        } else {
            named_mask(query.race_mask, races)
        }
    );
    println!(
        "  classes   0x{:08x}  {}",
        query.class_mask,
        if query.class_mask == whoquery::MASK_ALL {
            "every one".to_string()
        } else {
            named_mask(query.class_mask, classes)
        }
    );
    if query.zones.is_empty() {
        println!("  zones     (none — every zone)");
    } else {
        for id in &query.zones {
            println!("  zone      {id:<6} {}", name_of(*id));
        }
    }
    if query.terms.is_empty() {
        println!("  terms     (none)");
    } else {
        for term in &query.terms {
            println!("  term      {term:?}");
        }
    }
}
