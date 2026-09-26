//! `vale loading` — **what is on the screen while a map loads**, checked
//! against the archives.
//!
//! The join it exercises is the one thing about the loading screen that could
//! be wrong without failing: `Map.dbc` field 38 is not a column any of this
//! project's authorities names — vmangos' `MapEntryfmt` skips it, and it is
//! read here because the client reads it, at record offset `+0x98`. A wrong field index there
//! does not error. It resolves *some* row of `LoadingScreens.dbc`, or none, and
//! the client then shows a plausible parchment that is the wrong one, or the
//! generic one, for ever.
//!
//! So the check is built around the two things that would catch that:
//!
//! * **every map's picture is named**, and the five whose answer is known by
//!   name are asserted outright — Azeroth and Kalimdor are the two continents,
//!   and three instances whose rows say their own names; and
//! * **every path resolves in the archive chain**, both from the map side and
//!   over the whole of `LoadingScreens.dbc`, because a column that happened to
//!   land on a *different* valid table would give ids that mostly miss.
//!
//! Plus the three files that are not in any table at all — the fallback and the
//! two bar strips, whose paths are fixed in the client — and the bar's own
//! geometry, which is arithmetic and is checked in
//! [`vale_assets::tables::loading`]'s own tests rather than here. What this prints
//! of it is where it lands on a 1600x900 window, because a rectangle is easier
//! to disbelieve than four fractions.

use crate::common::*;
use vale_assets::tables::dbc::{dbc_path, map_directories, map_display_names};
use vale_assets::tables::loading::{LoadingScreens, BAR, FALLBACK};
use vale_config::Config;
use std::collections::BTreeMap;

/// The five maps whose picture is known from its own row name, and which
/// therefore pin the join rather than merely exercising it.
///
/// Two continents and three instances, chosen to span the id range: 3 and 4 are
/// the only single-digit screens, 104 is a battleground and 195/204 are at the
/// far end of the table.
const KNOWN: [(u32, &str); 5] = [
    (0, "Azeroth"),
    (1, "Kalimdor"),
    (30, "InstanceAlteracValley"),
    (33, "InstanceShadowfangKeep"),
    (389, "InstanceRagefireChasm"),
];

pub fn cmd_loading(cfg: &Config) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let screens = LoadingScreens::load(|table| assets.read(&dbc_path(table)).ok());
    let raw = assets
        .read(&dbc_path("Map"))
        .map_err(|e| format!("Map.dbc: {e}"))?;
    let directories = map_directories(&raw).map_err(|e| e.to_string())?;
    let names = map_display_names(&raw);

    if screens.maps() == 0 {
        return Err("Map.dbc named no loading screens at all — is field 38 still the column?".into());
    }
    println!(
        "LoadingScreens.dbc: {} row(s); Map.dbc: {} map(s), {} of which carry field 38",
        screens.screens().count(),
        directories.len(),
        screens.maps(),
    );

    // ---------------------------------------------------------- the maps --
    println!(
        "\n  {:<5} {:<22} {:<26} {:>6}  picture",
        "map", "directory", "display name", "screen"
    );
    let mut resolved = 0;
    let mut generic = 0;
    let mut orphan = Vec::new();
    let mut missing = Vec::new();
    let mut used: BTreeMap<u32, usize> = BTreeMap::new();
    for (map, directory) in directories.iter().collect::<BTreeMap<_, _>>() {
        let screen = screens.screen_of(*map);
        let path = screens.picture(*map);
        // The archive question is asked once per *path* below; here it is asked
        // per map so that the line says what this map would actually show.
        let present = assets.read(&file_of(path)).is_ok();
        if let Some(id) = screen {
            if id != 0 && screens.screen(id).is_none() {
                orphan.push((*map, id));
            }
            if screens.screen(id).is_some() {
                *used.entry(id).or_default() += 1;
            }
        }
        if path == FALLBACK {
            generic += 1;
        } else if present {
            resolved += 1;
        } else {
            missing.push((*map, path.to_string()));
        }
        println!(
            "  {:<5} {:<22} {:<26} {:>6}  {}{}",
            map,
            directory,
            names.get(map).map_or("-", String::as_str),
            screen.map_or_else(|| "-".to_string(), |id| id.to_string()),
            short(path),
            if present { "" } else { "   MISSING" },
        );
    }

    // -------------------------------------------------- and the table's --
    let mut unnamed = Vec::new();
    let mut unreadable = Vec::new();
    let mut rows: Vec<(u32, String, String)> = screens
        .screens()
        .map(|(id, name, path)| (id, name.to_string(), path.to_string()))
        .collect();
    rows.sort_by_key(|(id, _, _)| *id);
    for (id, name, path) in &rows {
        if assets.read(&file_of(path)).is_err() {
            unreadable.push((*id, path.clone()));
        }
        // A row nothing points at is not a fault — the shipped table has more
        // screens than 1.12 has maps — but it is the number that would move if
        // the join were reading the wrong column, so it is counted.
        if !used.contains_key(id) {
            unnamed.push((*id, name.clone()));
        }
    }

    println!(
        "\n  maps:      {resolved} show their own picture, {generic} the generic one, \
         {} name a row LoadingScreens.dbc does not have{}",
        orphan.len(),
        list(&orphan.iter().map(|(map, id)| format!("map {map} -> {id}")).collect::<Vec<_>>()),
    );
    println!(
        "  files:     {} of {} rows are in the archives{}",
        rows.len() - unreadable.len(),
        rows.len(),
        list(&unreadable.iter().map(|(id, path)| format!("{id} {path}")).collect::<Vec<_>>()),
    );
    if !missing.is_empty() {
        println!(
            "  MISSING:   {} map(s) name a picture the archives do not carry{}",
            missing.len(),
            list(&missing.iter().map(|(map, path)| format!("map {map} {path}")).collect::<Vec<_>>()),
        );
    }
    println!(
        "  unused:    {} row(s) no map points at{}",
        unnamed.len(),
        list(&unnamed.iter().map(|(id, name)| format!("{id} {name}")).collect::<Vec<_>>()),
    );

    // ------------------------------------------------------- the pinning --
    // The check that would fail if field 38 were the wrong column, and the one
    // that cannot be satisfied by any *other* plausible index: the five answers
    // are known from outside the file.
    let mut wrong = Vec::new();
    for (map, expected) in KNOWN {
        let actual = screens
            .screen_of(map)
            .and_then(|id| screens.screen(id))
            .map(|(name, _)| name.to_string());
        if actual.as_deref() != Some(expected) {
            wrong.push(format!(
                "map {map} -> {} (expected {expected})",
                actual.unwrap_or_else(|| "nothing".into())
            ));
        }
    }
    println!(
        "  join:      {} of {} known maps land on the row their own name says{}",
        KNOWN.len() - wrong.len(),
        KNOWN.len(),
        list(&wrong),
    );

    // ------------------------------------------- the three files off-table --
    println!("\n  the three paths that are fixed in the client rather than table rows:");
    for path in [FALLBACK, BAR[0].path, BAR[1].path] {
        match assets.read(&file_of(path)) {
            Ok(raw) => println!(
                "    {:<52} {:>7} bytes  {}",
                short(path),
                raw.len(),
                describe_blp(&raw)
            ),
            Err(e) => println!("    {:<52} MISSING ({e})", short(path)),
        }
    }

    // …and where the bar lands, which is the half of this subject a number
    // cannot make obvious. 1600x900 because it is the shape of a window
    // somebody is looking at, not because anything depends on it.
    println!("\n  the bar on a 1600x900 window, at three progresses:");
    for progress in [0.0, 0.5, 1.0] {
        let parts: Vec<String> = BAR
            .iter()
            .map(|piece| {
                let [left, bottom, right, top] = piece.rect(progress);
                format!(
                    "{}: x {:.0}..{:.0}, y {:.0}..{:.0} from the bottom",
                    if piece.fills { "fill" } else { "frame" },
                    left * 1600.0,
                    right * 1600.0,
                    bottom * 900.0,
                    top * 900.0,
                )
            })
            .collect();
        println!("    {progress:>4.1}  {}", parts.join("   "));
    }

    if !wrong.is_empty() || !missing.is_empty() {
        return Err("the loading-screen join does not hold — see above".into());
    }
    Ok(())
}

/// **The file an `Interface\` path names.** The forty table rows carry `.blp`
/// and the three fixed paths do not — the client appends it, which
/// is the same rule `ui::framexml`'s `decode_rgba` keeps. Without this the
/// fallback reads as missing from the archives it is plainly in, which is the
/// one way this check can lie in the reassuring direction's opposite.
fn file_of(path: &str) -> String {
    if path.to_ascii_lowercase().ends_with(".blp") {
        path.to_string()
    } else {
        format!("{path}.blp")
    }
}

/// `Interface\Glues\LoadingScreens\LoadScreenKalimdor.blp` -> the leaf, which is
/// the only part that differs between the forty rows.
fn short(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// ` — a, b, c` for a non-empty list and nothing at all for an empty one, so a
/// clean run's line ends where it says zero.
fn list(items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!(" — {}", items.join(", "))
    }
}
