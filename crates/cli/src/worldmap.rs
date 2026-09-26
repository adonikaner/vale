//! `vale zones` — **where the world map says you are**, checked with no
//! session.
//!
//! Two tables and a page of arithmetic decide the whole panel
//! ([`vale_assets::tables::worldmap`]), and none of it crosses the wire — so the only
//! way to be wrong about it is to be wrong quietly. This is what makes it loud:
//! it builds the continent table the way the client builds it, prints every
//! parchment and how many zones it holds, and then asks each zone's own
//! rectangle three questions whose answers are known in advance.
//!
//! ```text
//! vale zones              every continent, its zones, and the three checks
//! vale zones 12           trace one area: its zone, its map, its rectangle
//! vale zones Azeroth 33 41   …and what the *buildings* on a tile are called
//! ```
//!
//! ## The third form is the check on `WMOAreaTable.dbc`
//!
//! The ground is not the whole answer — see [`vale_assets::tables::wmoarea`]. The join
//! that fixes that is three keys deep and every one of them is a byte offset in
//! a different file (`MOHD` 0x20, `MODF` 0x3C, `MOGP` 0x38), so a wrong one
//! resolves to *nothing* and looks exactly like a building the table says
//! nothing about — which is the honest answer for most of the game's models.
//! Reading it back off the archives is the only thing that tells the two apart:
//! `vale zones Azeroth 33 41` walks Ironforge's own 104 groups and names
//! them.
//!
//! **The checks are shape checks, in the sense the light command's are.** None
//! of them needs to know where Durotar is; each is a property the table must
//! have if it is being read as a table at all:
//!
//! * a zone's rectangle is **oriented** — `locLeft > locRight` and
//!   `locTop > locBottom`, because both axes of this game's coordinates run
//!   backwards. A transposed pair reads as a rectangle of negative width, and
//!   every position on it comes back outside 0..1 and therefore as no position
//!   at all;
//! * a zone's **centre lands on its own map**, which is the round trip through
//!   [`vale_assets::tables::worldmap::MapArea::position`] and back;
//! * a zone's centre also lands **inside its continent's own rectangle**, which
//!   is the only thing that crosses the two branches of the position mapping —
//!   the zone lerp and the cosmic placement are computed from different columns,
//!   so a sign error in either shows up here and nowhere else;
//! * a zone's centre **highlights that zone**, which is the round trip through
//!   the `.zmp` grid and back and is the check that pins its indexing: get the
//!   row and column the wrong way round and the map flips north to south while
//!   still answering a zone for every point on it;
//! * …and the quad it answers with **stays on the parchment**, and its sheet is
//!   actually in the archives — the directory is a string column, so a wrong one
//!   resolves to a missing file rather than to an error.

use crate::common::{open_assets, open_display_tables};
use vale_assets::tables::worldmap::MapView;
use vale_config::Config;

/// **The landmark layer**, checked the way every other table here is: every row
/// against the map it claims, and every projection against the parchment it
/// lands on.
///
/// Four numbers worth reading, and the fourth is the one that would catch a
/// wrong field index: a row whose `map` is not a map with a parchment, or whose
/// yards fall outside its own zone rectangle, projects to nothing — so a
/// **large** "off the map" count means the coordinate columns are being read at
/// the wrong offset rather than that the game ships stray rows.
fn landmarks(tables: &vale_assets::tables::dbc::DisplayTables) {
    use vale_assets::tables::worldmap::MapView;
    let (Some(pois), Some(map), Some(areas)) =
        (tables.area_pois(), tables.world_map(), tables.areas())
    else {
        println!();
        println!("== AreaPOI.dbc is not in the archives — the map has no landmarks ==");
        return;
    };
    println!();
    println!("== AreaPOI.dbc ==");
    println!("  {} rows", pois.len());

    let mut by_map: std::collections::BTreeMap<u32, usize> = Default::default();
    let mut gated = 0;
    let mut named = 0;
    let mut described = 0;
    let mut icons: std::collections::BTreeSet<u32> = Default::default();
    for poi in pois.rows() {
        *by_map.entry(poi.map).or_default() += 1;
        icons.insert(poi.icon);
        named += usize::from(!poi.name.is_empty());
        described += usize::from(!poi.description.is_empty());
        // The exploration gate, as `areapoi::landmarks` applies it.
        if poi.area > 0 {
            if let Some(area) = areas.get(poi.area as u32) {
                gated += usize::from(area.explore_level >= 0);
            }
        }
    }
    println!(
        "  {named} named, {described} with a description, {gated} gated on exploration"
    );
    println!(
        "  icons {}..{} over {} distinct cells of POIIcons.blp (8x8 of 16x16)",
        icons.iter().next().copied().unwrap_or(0),
        icons.iter().next_back().copied().unwrap_or(0),
        icons.len()
    );
    for (id, count) in &by_map {
        println!("    map {id:<4} {count:>4} rows  {}", tables.map_name(*id));
    }

    // **Every row projected onto its own zone's parchment**, which is the check
    // the counts above cannot make: a coordinate column read at the wrong
    // offset still counts, still names a map, and lands nowhere.
    let mut placed = 0;
    let mut off = Vec::new();
    for poi in pois.rows() {
        let view = map
            .continents()
            .iter()
            .position(|c| c.map == poi.map)
            .map_or(MapView::Cosmic, MapView::Continent);
        if map.position(view, poi.map, poi.position.0, poi.position.1).is_some() {
            placed += 1;
        } else if off.len() < 6 {
            off.push(poi);
        }
    }
    // **The rest are on a map with no continent row**, which is every
    // battleground: `WorldMapContinent.dbc` has two entries and a battleground
    // is neither, so there is no continent parchment to project onto and the
    // zone one is what the game opens. A count that is anything *but* the two
    // continents' rows is the interesting failure — it would mean a coordinate
    // column read at the wrong offset.
    println!(
        "  {placed} of {} land on their own continent's parchment",
        pois.len()
    );
    println!("    (the rest are on a map with no WorldMapContinent row — the battlegrounds)");
    for poi in off {
        println!(
            "    off: {:<28} map {} at {:.0}, {:.0}",
            poi.name, poi.map, poi.position.0, poi.position.1
        );
    }
}

pub fn cmd_zones(cfg: &Config, area: Option<u32>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let areas = tables
        .areas()
        .ok_or_else(|| "AreaTable.dbc is not in the archives".to_string())?;
    let map = tables
        .world_map()
        .ok_or_else(|| "WorldMapArea.dbc or WorldMapContinent.dbc is not in the archives")?;

    println!("== AreaTable.dbc ==");
    println!(
        "  {} areas, {} of them zones ({} sub-areas)",
        areas.count(),
        areas.zone_count(),
        areas.count() - areas.zone_count()
    );

    // …and the other table that answers the same question, for the places the
    // ground does not know about.
    match tables.wmo_areas() {
        Some(wmo_areas) => println!(
            "  WMOAreaTable.dbc: {} rows, {} naming a place, {} naming an area",
            wmo_areas.count(),
            wmo_areas.named_count(),
            wmo_areas.with_area_count()
        ),
        None => println!("  WMOAreaTable.dbc is not in the archives — no building names a place"),
    }

    if let Some(id) = area {
        return trace(&tables, id);
    }

    landmarks(&tables);

    let (rows, continents, _) = map.counts();
    println!();
    println!("== WorldMapArea.dbc ==");
    println!("  {rows} rows, {continents} of them continents");

    let (mut oriented, mut on_own, mut on_continent, mut zones) = (0, 0, 0, 0);
    let (mut named_back, mut inside, mut has_art) = (0, 0, 0);
    let mut elsewhere: Vec<(String, String)> = Vec::new();
    for (index, continent) in map.continents().iter().enumerate() {
        let (left, top, right, bottom) = continent.rect_on_cosmic();
        println!();
        println!(
            "  {} (map {}) — {} zones, {} on the world map at u {:.3}..{:.3}, v {:.3}..{:.3}",
            tables.map_name(continent.map),
            continent.map,
            continent.zones.len(),
            map.directory(MapView::Continent(index)).unwrap_or("?"),
            left,
            right,
            top,
            bottom
        );
        for row in &continent.zones {
            let Some(zone) = map.area(*row) else { continue };
            zones += 1;
            if zone.left > zone.right && zone.top > zone.bottom {
                oriented += 1;
            }
            // The middle of the zone, in world yards — which by construction is
            // dead centre of its own parchment.
            let (x, y) = (
                (zone.top + zone.bottom) / 2.0,
                (zone.left + zone.right) / 2.0,
            );
            if zone.position(x, y).is_some() {
                on_own += 1;
            }
            if let Some((u, v)) = map.position(MapView::Cosmic, continent.map, x, y) {
                if (left..=right).contains(&u) && (top..=bottom).contains(&v) {
                    on_continent += 1;
                }
            }
            // **…and what the pointer over that centre would highlight**, which
            // is the fourth check and the only one that reaches the archives:
            // the directory is a string column, so a wrong one resolves to a
            // file that is not there rather than to an error.
            if let Some((u, v)) = map.position(MapView::Continent(index), continent.map, x, y) {
                if let Some(hit) = map.highlight(MapView::Continent(index), u, v) {
                    if hit.directory == zone.directory {
                        named_back += 1;
                    } else {
                        // **Not necessarily wrong** — see the note below the
                        // checks, which is why these are named rather than
                        // counted.
                        elsewhere.push((zone.directory.clone(), hit.directory.clone()));
                    }
                    let (right_edge, bottom_edge) =
                        (hit.offset.0 + hit.size.0, hit.offset.1 + hit.size.1);
                    if hit.offset.0 >= 0.0
                        && hit.offset.1 >= 0.0
                        && right_edge <= 1.0
                        && bottom_edge <= 1.0
                    {
                        inside += 1;
                    }
                    if sheet_exists(&mut assets, &hit.directory) {
                        has_art += 1;
                    }
                }
            }
            println!(
                "      {:<32} area {:<5} {}",
                areas
                    .get(zone.area)
                    .map_or("(no AreaTable row)", |area| area.name.as_str()),
                zone.area,
                zone.directory
            );
        }
    }

    println!();
    println!("== the shape checks ==");
    let grids = map
        .continents()
        .iter()
        .filter(|continent| continent.has_zone_grid())
        .count();
    println!(
        "  {grids} of {continents} continents have their .zmp grid{}",
        if grids == continents {
            ""
        } else {
            "   (the rest fall back to the rectangles — see WorldMap::zone_at)"
        }
    );
    let line = |name: &str, count: usize| {
        println!(
            "  {count} of {zones} {name}{}",
            if count == zones { "" } else { "   <- MISMATCH" }
        );
    };
    line("rectangles run the right way round", oriented);
    line("zone centres land on their own map", on_own);
    line("…and inside their continent's rectangle", on_continent);
    println!(
        "  {named_back} of {zones} …and highlight themselves when hovered",
    );
    // **The exceptions are the answer rather than an error.** The `.zmp` grid
    // says what is *on the surface* at a point, so a city under one — Ironforge
    // under Dun Morogh, Undercity under Tirisfal — is never what a hover over
    // its own map rectangle names. The real client does the same; its city maps
    // are reached from the drop-down.
    for (zone, named) in &elsewhere {
        println!("      ({zone} is under {named}, which is what the grid says)");
    }
    line("…with a quad that stays on the parchment", inside);
    println!(
        "  {has_art} of {zones} have a highlight sheet in the archives\
         {}",
        if has_art == zones {
            ""
        } else {
            "   (a zone with none draws no highlight, which is 1.12's own)"
        }
    );

    // The cosmic branch of the same function, which is a different arm and two
    // rows rather than fifty.
    println!();
    for (index, continent) in map.continents().iter().enumerate() {
        let (left, top, right, bottom) = continent.rect_on_cosmic();
        let middle = ((left + right) / 2.0, (top + bottom) / 2.0);
        match map.highlight(MapView::Cosmic, middle.0, middle.1) {
            Some(hit) => println!(
                "  the cosmic map highlights {:<10} at u {:.3}..{:.3}, v {:.3}..{:.3}, {:.0}% x {:.0}% of its sheet{}",
                hit.directory,
                hit.offset.0,
                hit.offset.0 + hit.size.0,
                hit.offset.1,
                hit.offset.1 + hit.size.1,
                hit.tex_percentage.0 * 100.0,
                hit.tex_percentage.1 * 100.0,
                if sheet_exists(&mut assets, &hit.directory) {
                    ""
                } else {
                    "   <- no sheet in the archives"
                }
            ),
            None => println!(
                "  the cosmic map highlights nothing over {}   <- MISMATCH",
                map.directory(MapView::Continent(index)).unwrap_or("?")
            ),
        }
    }

    instance_zones(&mut assets, &tables);
    overlays(&mut assets, &tables)?;
    Ok(())
}

/// **What names a map that has no ground** — the census behind
/// `game::place::worldmap`'s last fallback.
///
/// Twenty of the game's forty-three maps are one building and no ADT, so
/// `MCNK`'s `areaId` — where every zone name in this client otherwise comes
/// from — does not exist on them at all. The building's own `WMOAreaTable` rows
/// are meant to answer instead, and where they do nothing here is needed. Where
/// they do **not**, the only thing in any shipped file that still names the
/// place is `AreaTable`'s own `mapId` column, and this is the count that says
/// whether reading it is a rule or a guess.
///
/// The number to look at is the middle column: **one** zone row naming a map is
/// an answer, and anything else is why [`vale_assets::tables::area::Areas::zone_of_map`]
/// refuses rather than picking.
fn instance_zones(
    assets: &mut vale_assets::Assets,
    tables: &vale_assets::tables::dbc::DisplayTables,
) {
    let Some(areas) = tables.areas() else { return };
    let directories = assets
        .read(&vale_assets::tables::dbc::dbc_path("Map"))
        .ok()
        .and_then(|raw| vale_assets::tables::dbc::map_directories(&raw).ok())
        .unwrap_or_default();
    println!();
    println!("== the maps with no ground: what names them, and what does not ==");
    let mut ids: Vec<u32> = tables.map_ids().collect();
    ids.sort_unstable();
    let (mut both, mut building_only, mut table_only, mut neither) = (0, 0, 0, 0);
    for map in ids {
        // A map with a tile grid is not this section's subject: its ground
        // answers, and `zone_of_map` is never asked. Reading the `WDT` is what
        // says which is which, and it is one small file per map.
        let Some(directory) = directories.get(&map) else {
            continue;
        };
        let Some(wdt) = assets
            .read(&vale_assets::wdt_path(directory))
            .ok()
            .and_then(|raw| vale_assets::world::wdt::Wdt::parse(&raw).ok())
        else {
            continue;
        };
        if !wdt.is_wmo_only() {
            continue;
        }

        // What the *building* says, which is the primary answer and the one
        // this section exists to check rather than assume.
        let placements = wdt.placed_global_wmos();
        //
        // **Every row of it, not only the group under the character**: the
        // question here is whether the building states an area *anywhere*, which
        // is the honest form of "would the primary path ever answer on this
        // map". Asking only the whole-building `-1` row would under-report, and
        // it is the first thing this census got wrong.
        //
        // **The root id and the row count are printed beside the answer**,
        // because "the building states no area" and "this census never found the
        // building" produce the same dash and are not the same claim. A `-` with
        // a root id and a row count behind it is a measurement; a `-` with
        // `wmo ?` behind it is a hole in the check.
        let building = placements.first().and_then(|placement| {
            let root = assets
                .read(&placement.path)
                .ok()
                .and_then(|raw| vale_assets::world::wmo::WmoRoot::parse(&raw).ok())?;
            let rows = tables.wmo_areas()?.areas_named_by(root.wmo_id);
            Some((root.wmo_id, tables.wmo_areas()?.rows_for(root.wmo_id), rows))
        });
        let from_building = building
            .as_ref()
            .and_then(|(_, _, named)| named.first().copied());
        // …and what the table says, which is the fallback.
        let from_table = areas.zone_of_map(map).map(|zone| zone.id);

        match (from_building.is_some(), from_table.is_some()) {
            (true, true) => both += 1,
            (true, false) => building_only += 1,
            (false, true) => table_only += 1,
            (false, false) => neither += 1,
        }
        let name = |id: Option<u32>| match id {
            Some(id) => format!("{} ({id})", areas.zone_name(id)),
            None => "-".to_string(),
        };
        let provenance = match &building {
            Some((root, rows, _)) => format!("wmo {root}, {rows} row(s)"),
            None => "wmo ?   <- the building was not read".to_string(),
        };
        println!(
            "  map {:<4} {:<24} building {:<26} AreaTable {:<26} ({provenance})",
            map,
            tables.map_name(map),
            name(from_building),
            name(from_table),
        );
    }
    println!(
        "  {both} answered by both, {building_only} by the building alone, \
         {table_only} by AreaTable alone, {neither} by neither"
    );
}

/// **The explored half of a zone map**, checked against the archives and against
/// itself.
///
/// `WorldMapOverlay.dbc`'s field layout is the one the client's
/// `GetMapOverlayInfo` reads ([`vale_assets::tables::worldmap`] quotes the
/// offsets), and this is the measurement beside it — because a field index
/// that is *nearly* right here does not error, it draws a picture in the wrong
/// place or draws nothing at all:
///
/// * every overlay's **hit rectangle lies inside its own picture**, which is
///   what a wrong `offset`/`hitRect` pair breaks and nothing else would notice:
///   the label would name a place a hand's width from where the art is;
/// * every overlay's picture **stays on the parchment** (1002 x 668), which is
///   what a transposed width/height or x/y pair breaks;
/// * every overlay names a **`WorldMapArea` row that exists** and at least one
///   **`AreaTable` row that exists**, which is what pins fields 1 and 2..5 — a
///   wrong index resolves in neither table;
/// * and every **piece of every picture is in the archives**, at the path
///   the client's format string builds, cut the way `WorldMapFrame_Update` cuts
///   it. That last is the check that cannot be argued with: 526 rows over the
///   game's 46 zone directories, and a wrong texture column would resolve to
///   nothing at all.
fn overlays(
    assets: &mut vale_assets::Assets,
    tables: &vale_assets::tables::dbc::DisplayTables,
) -> Result<(), String> {
    let Some(map) = tables.world_map() else {
        return Ok(());
    };
    let all = map.all_overlays();
    println!();
    println!("== WorldMapOverlay.dbc ==");
    if all.is_empty() {
        println!("  not in the archives — every zone map draws blank, which is an");
        println!("  unexplored map rather than a wrong one");
        return Ok(());
    }
    let (mut on_area, mut named, mut hit_inside, mut on_parchment) = (0, 0, 0, 0);
    let (mut pieces, mut present) = (0, 0);
    let mut missing: Vec<String> = Vec::new();
    let mut ragged: Vec<String> = Vec::new();
    let mut zones_with_art = std::collections::BTreeSet::new();
    for overlay in all {
        let row = map.area(overlay.map_area);
        if let Some(row) = row {
            on_area += 1;
            zones_with_art.insert(row.id);
        }
        if overlay
            .areas
            .iter()
            .any(|id| tables.areas().and_then(|areas| areas.get(*id)).is_some())
        {
            named += 1;
        }
        let (top, left, bottom, right) = overlay.hit;
        if left >= overlay.offset_x
            && top >= overlay.offset_y
            && right <= overlay.offset_x + overlay.width
            && bottom <= overlay.offset_y + overlay.height
        {
            hit_inside += 1;
        } else {
            ragged.push(format!(
                "{} hit {left},{top}..{right},{bottom} against art {},{}..{},{}",
                overlay.texture,
                overlay.offset_x,
                overlay.offset_y,
                overlay.offset_x + overlay.width,
                overlay.offset_y + overlay.height,
            ));
        }
        if (overlay.offset_x + overlay.width) as f32 <= vale_assets::tables::worldmap::PARCHMENT_WIDTH
            && (overlay.offset_y + overlay.height) as f32
                <= vale_assets::tables::worldmap::PARCHMENT_HEIGHT
        {
            on_parchment += 1;
        } else {
            ragged.push(format!(
                "{} runs to {},{} of the 1002x668 parchment",
                overlay.texture,
                overlay.offset_x + overlay.width,
                overlay.offset_y + overlay.height,
            ));
        }
        // …and the art itself. Only for rows whose directory resolves; one with
        // no `WorldMapArea` row has no directory to build a path out of.
        let Some(row) = row else { continue };
        let (across, down) = overlay.tiles();
        for n in 1..=across * down {
            pieces += 1;
            let path = format!("{}.blp", overlay.piece(&row.directory, n));
            if assets.read(&path).is_ok() {
                present += 1;
            } else if missing.len() < 8 {
                missing.push(path);
            }
        }
    }
    let rows = all.len();
    // **The two geometry checks are the ones with authoring exceptions**, so
    // they are reported with the offenders named rather than as a mismatch: a
    // *field-index* error would put every row on the wrong side of them, and a
    // handful is the table having a handful of ragged rows. See the note below
    // the call.
    let line = |name: &str, count: usize| {
        println!(
            "  {count} of {rows} {name}{}",
            if count == rows { "" } else { "   <- MISMATCH" }
        );
    };
    let soft = |name: &str, count: usize| {
        println!(
            "  {count} of {rows} {name}{}",
            if count == rows {
                String::new()
            } else {
                format!("   ({} ragged in the shipped table)", rows - count)
            }
        );
    };
    println!("  {rows} overlays over {} zone maps", zones_with_art.len());
    line("name a WorldMapArea row that exists", on_area);
    line("name at least one AreaTable row that exists", named);
    soft("…with a hit rectangle inside their own picture", hit_inside);
    soft("…and a picture that stays on the parchment", on_parchment);
    for line in &ragged {
        println!("      {line}");
    }
    println!(
        "  {present} of {pieces} art pieces are in the archives{}",
        if present == pieces {
            ""
        } else {
            "   <- MISMATCH"
        }
    );
    for path in &missing {
        println!("      missing: {path}");
    }
    Ok(())
}

/// Is the sheet a highlight names actually in the archives?
///
/// `Interface\WorldMap\<dir>\<dir>Highlight.blp`, which is the path
/// `WorldMapButton_OnUpdate` builds by hand out of the second return value —
/// so this is that concatenation and not a second convention.
fn sheet_exists(assets: &mut vale_assets::Assets, directory: &str) -> bool {
    assets
        .read(&format!(
            "Interface\\WorldMap\\{directory}\\{directory}Highlight.blp"
        ))
        .is_ok()
}

/// **What the buildings on one tile are called** — the `WMOAreaTable` join,
/// walked back off the archives.
///
/// Per placement: which of its groups resolve to a row, what the distinct names
/// are, and what area the building claims against what the ground under it says.
/// A building with no rows at all is the ordinary case and is counted rather
/// than listed — a fence has none.
pub fn cmd_zone_buildings(cfg: &Config, map_name: &str, x: u32, y: u32) -> Result<(), String> {
    use vale_assets::world::wmo;
    use std::collections::{BTreeMap, BTreeSet};

    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let areas = tables
        .areas()
        .ok_or_else(|| "AreaTable.dbc is not in the archives".to_string())?;
    let wmo_areas = tables
        .wmo_areas()
        .ok_or_else(|| "WMOAreaTable.dbc is not in the archives".to_string())?;

    let path = format!("World\\Maps\\{map_name}\\{map_name}_{x}_{y}.adt");
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    let adt = vale_assets::Adt::parse(&raw).map_err(|e| e.to_string())?;
    println!("{path}");

    // The ground's own answer, for the comparison that is the whole point: a
    // building that agrees with the dirt it stands on has changed nothing.
    let ground: BTreeSet<u32> = adt.chunks.iter().map(|c| c.area_id).collect();
    let ground_names: Vec<String> = ground
        .iter()
        .map(|id| format!("{} ({id})", areas.zone_name(*id)))
        .collect();
    println!("  the ground says: {}", ground_names.join(", "));

    let mut models: BTreeMap<String, wmo::WmoModel> = BTreeMap::new();
    let (mut placed, mut answered, mut silent) = (0usize, 0usize, 0usize);
    for placement in &adt.wmos {
        let Some(name) = adt.wmo_names.get(placement.name_id as usize).cloned() else {
            continue;
        };
        placed += 1;
        if !models.contains_key(&name) {
            match wmo::load(&mut assets, &name) {
                Ok((model, _)) => {
                    models.insert(name.clone(), model);
                }
                Err(e) => {
                    println!("  !! {name}: {e}");
                    continue;
                }
            }
        }
        let Some(model) = models.get(&name) else {
            continue;
        };

        // One lookup per group, exactly as `game::worldmap` makes it — the same
        // three keys, off the same two files.
        let name_set = u32::from(placement.name_set);
        let mut found = 0usize;
        let mut places: BTreeMap<String, usize> = BTreeMap::new();
        let mut claimed: BTreeSet<u32> = BTreeSet::new();
        for (_, group) in model.area_bounds() {
            let Some(row) = wmo_areas.lookup(model.wmo_id, name_set, group) else {
                continue;
            };
            found += 1;
            if row.area != 0 {
                claimed.insert(row.area);
            }
            if !row.name.is_empty() {
                *places.entry(row.name.clone()).or_default() += 1;
            }
        }
        if found == 0 {
            silent += 1;
            continue;
        }
        answered += 1;
        let zones: Vec<String> = claimed
            .iter()
            .map(|id| format!("{} ({id})", areas.zone_name(*id)))
            .collect();
        println!();
        println!(
            "  {name}  (wmoId {}, name set {name_set})",
            model.wmo_id
        );
        println!(
            "    {found} of {} groups resolve; zone: {}",
            model.groups.len(),
            if zones.is_empty() {
                "(the ground's)".to_string()
            } else {
                zones.join(", ")
            }
        );
        for (place, count) in &places {
            println!("      {place:<28} {count} group(s)");
        }
    }

    println!();
    println!("== the checks ==");
    println!("  {placed} placements: {answered} name a place, {silent} say nothing");
    println!(
        "  (a building with no rows is the ordinary case — {} of the table's {} rows are named)",
        wmo_areas.named_count(),
        wmo_areas.count()
    );
    Ok(())
}

/// One area, traced — what the interface would say standing on it.
fn trace(tables: &vale_assets::tables::dbc::DisplayTables, id: u32) -> Result<(), String> {
    let areas = tables.areas().expect("checked by the caller");
    let map = tables.world_map().expect("checked by the caller");
    let Some(area) = areas.get(id) else {
        return Err(format!("no area {id} in AreaTable.dbc"));
    };
    let zone = areas.zone_of(id).expect("the area resolves");
    println!();
    println!("== area {id} ==");
    println!("  name            {}", area.name);
    println!("  map             {} ({})", tables.map_name(area.map), area.map);
    println!("  GetZoneText     {}", areas.zone_name(id));
    println!("  GetSubZoneText  {:?}", areas.sub_zone_name(id));

    // …and where the world map would open, which is the join between the two
    // tables: `SetMapToCurrentZone` looks for a `WorldMapArea` row whose own
    // `areaId` is this area's *zone*.
    let found = map.continents().iter().enumerate().find_map(|(index, c)| {
        let zone_index = c
            .zones
            .iter()
            .position(|row| map.area(*row).is_some_and(|row| row.area == zone.id))?;
        Some((index, zone_index))
    });
    match found {
        Some((continent, zone_index)) => {
            let view = MapView::Zone(continent, zone_index);
            println!(
                "  the map opens   continent {}, zone {} — {}",
                view.continent_index(),
                view.zone_index(),
                map.directory(view).unwrap_or("?")
            );
        }
        // The ordinary case for an instance and for the handful of zones with
        // no parchment of their own — see `game::worldmap::aim_at_current_zone`.
        None => println!("  the map opens   its continent: no WorldMapArea row for this zone"),
    }
    Ok(())
}
