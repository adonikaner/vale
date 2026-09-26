//! `vale light` — a map's daylight, band by band, plus its zone spheres.

use vale_assets::tables::dbc::MapKind;
use vale_assets::tables::light::{
    Atmosphere, LightTables, DAY, FLOAT_BAND_NAMES, FLOAT_BAND_NOTES, INT_BAND_NAMES,
    INT_BAND_NOTES, NOON, SKY_STOPS, YARDS_PER_UNIT,
};
use vale_config::Config;

/// The whole survey, or one map's day.
pub fn cmd_light(cfg: &Config, map: Option<&str>) -> Result<(), String> {
    let mut assets = crate::common::open_assets(cfg)?;
    let raw = assets
        .read(&vale_assets::tables::dbc::dbc_path("Map"))
        .map_err(|e| e.to_string())?;
    let maps = vale_assets::tables::dbc::map_directories(&raw).map_err(|e| e.to_string())?;
    let kinds = vale_assets::tables::dbc::map_kinds(&raw).map_err(|e| e.to_string())?;
    let tables = crate::common::open_display_tables(&mut assets)?;
    let light = tables
        .light()
        .ok_or("no light chain in this archive — every liquid draws white and the world takes the placeholder")?;

    match map {
        Some(name) => {
            let (&id, _) = maps
                .iter()
                .find(|(_, dir)| dir.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no map named {name:?} in Map.dbc"))?;
            one_map(light, id, name)
        }
        None => {
            survey(light, &maps, &kinds);
            positional_survey(&mut assets, light, &maps);
            schema_census(&mut assets);
        }
    }
    Ok(())
}

/// **The five light tables against the schema that describes them.**
///
/// [`vale_assets::tables::schema`] carries a column count, a type per column
/// and, on several columns, a measurement written as prose. A schema is what an
/// editor draws a form from, so a column typed wrongly is a number somebody
/// edits into nonsense and a count off by one pairs every band key with the
/// wrong value. None of that fails at load — a DBC states its own width and the
/// reader takes it — so this is the check.
///
/// It re-measures rather than restating: every line below is computed from the
/// file here and now, and the schema's claim is what it is compared against.
fn schema_census(assets: &mut vale_assets::Assets) {
    use vale_assets::tables::dbc::{dbc_path, Dbc};
    use vale_assets::tables::light::{BAND_KEYS, BAND_TIME_FIELD, BAND_VALUE_FIELD, DAY};
    use vale_assets::tables::schema;

    /// The fill an authoring tool left in the slots past `EntryCount`.
    const UNINITIALISED: u32 = 0xCCCC_CCCC;

    let mut open = |name: &str| {
        let raw = assets.read(&dbc_path(name)).ok()?;
        Dbc::parse(&raw).ok()
    };

    println!("\nthe light chain against `tables::schema`:");
    for name in [
        "Light",
        "LightParams",
        "LightIntBand",
        "LightFloatBand",
        "LightSkybox",
    ] {
        let Some(described) = schema::for_table(name) else {
            println!("  {name:<16} no schema   <- LOOK");
            continue;
        };
        let Some(dbc) = open(name) else {
            println!("  {name:<16} not in the archives   <- LOOK");
            continue;
        };
        let fits = dbc.field_count == described.columns.len();
        println!(
            "  {name:<16} {:5} records x {:2} fields; schema has {:2}{}",
            dbc.record_count,
            dbc.field_count,
            described.columns.len(),
            if fits { "" } else { "   <- LOOK" },
        );
    }

    // **The band hop is arithmetic, so the row counts are what check it.**
    // 18 int bands and 6 float bands per `LightParams` row. A count that does
    // not divide would read one light's fog as another's, which is a plausible
    // wrong colour rather than a failure.
    let (Some(params), Some(ints), Some(floats), Some(light)) = (
        open("LightParams"),
        open("LightIntBand"),
        open("LightFloatBand"),
        open("Light"),
    ) else {
        return;
    };
    let exact = params.record_count != 0
        && ints.record_count == params.record_count * 18
        && floats.record_count == params.record_count * 6;
    println!(
        "  bands per params: {} int, {} float, over {} params rows{}",
        ints.record_count.checked_div(params.record_count).unwrap_or(0),
        floats.record_count.checked_div(params.record_count).unwrap_or(0),
        params.record_count,
        if exact { "" } else { "   <- LOOK: not an exact division" },
    );

    // Every live key's time falling inside the day is what pins fields 2..17
    // as the times rather than the values. The slots past `EntryCount` are
    // counted rather than read: they hold whatever was in the authoring tool's
    // memory, and a writer that cleaned them would rewrite rows nobody edited.
    for (label, dbc) in [("LightIntBand", &ints), ("LightFloatBand", &floats)] {
        let (mut live, mut in_day, mut ascending, mut padded) = (0usize, 0usize, 0usize, 0usize);
        for record in 0..dbc.record_count {
            let count = dbc.u32_at(record, 1).unwrap_or(0).min(BAND_KEYS as u32) as usize;
            let times: Vec<u32> = (0..count)
                .filter_map(|n| dbc.u32_at(record, BAND_TIME_FIELD + n))
                .collect();
            live += times.len();
            in_day += times.iter().filter(|&&t| t < DAY).count();
            ascending += times.windows(2).all(|w| w[0] < w[1]) as usize;
            padded += (count..BAND_KEYS)
                .filter(|&n| dbc.u32_at(record, BAND_TIME_FIELD + n) == Some(UNINITIALISED))
                .count();
        }
        let mark = if in_day == live { "" } else { "   <- LOOK" };
        println!(
            "  {label:<16} {live} live keys, {in_day} inside the {DAY}-half-minute day{mark}"
        );
        println!(
            "                   {ascending} of {} rows ascend strictly; {padded} unused \
             time slots hold 0x{UNINITIALISED:08X}",
            dbc.record_count,
        );
    }

    // An int band's colours are `0x00RRGGBB`. A set top byte is a hand-authored
    // row and it rides through a write untouched — see `schema::Kind::Colour`.
    let mut top_set = 0usize;
    for record in 0..ints.record_count {
        let count = ints.u32_at(record, 1).unwrap_or(0).min(BAND_KEYS as u32) as usize;
        top_set += (0..count)
            .filter(|&n| ints.u32_at(record, BAND_VALUE_FIELD + n).unwrap_or(0) >> 24 != 0)
            .count();
    }
    println!("  LightIntBand     {top_set} live colours carry a non-zero top byte");

    // `LightParams.Skybox` is a reference the schema names, and 1.12 uses it
    // for one thing. Counted here because the claim is about the population
    // rather than about the column: the rows that carry a skybox are reached
    // by `Light`'s death column and by nothing else.
    let carriers: Vec<u32> = (0..params.record_count)
        .filter(|&r| params.u32_at(r, 2).unwrap_or(0) != 0)
        .filter_map(|r| params.u32_at(r, 0))
        .collect();
    let named_by = |fields: std::ops::Range<usize>| {
        (0..light.record_count)
            .flat_map(|record| fields.clone().map(move |f| (record, f)))
            .filter(|&(record, f)| carriers.contains(&light.u32_at(record, f).unwrap_or(0)))
            .count()
    };
    println!(
        "  LightParams      {} rows carry a skybox: {} references from Light's four living \
         columns, {} from its death column",
        carriers.len(),
        named_by(7..11),
        named_by(11..12),
    );
}

/// **The positional half of `Light.dbc`, checked against the ground it stands
/// on.**
///
/// A map's default light is one row; every other row is a sphere of its own
/// daylight over it, and where those spheres *are* is the whole question — the
/// three floats carry no unit, no origin and no axis names, and every reading
/// of them yields a position. What separates the right one is that only the
/// right one puts the lights **on land**: a zone light sits over a zone, so its
/// centre falls on a tile the map actually ships, and the map ships tiles over
/// a third of its grid at most.
///
/// So this prints, for each of the four readings the columns admit, how many
/// centres land on an existing tile. It is not a formality — the wrong ones
/// score near the base rate, and the base rate is the last line.
/// (The corroborating measurement, against vmangos' `game_tele`, is recorded
/// on `vale_assets::tables::light::WORLD_CORNER`; this is the half that needs no
/// server running.)
///
/// **A map with no ADT tiles is left out of the totals, and so is one whose
/// tiles fill its grid.** The first can answer nothing — a WMO-only instance
/// has no ground for a centre to land on, so every reading scores zero and
/// the four are told apart by nothing. The second answers *yes* to everything:
/// `EmeraldDream` is 256 tiles in one solid block and any position inside it
/// hits. Both are printed with their own counts and marked, because the
/// exclusion is a judgement and a reader should see what it removed.
fn positional_survey(
    assets: &mut vale_assets::archive::Assets,
    light: &LightTables,
    maps: &std::collections::HashMap<u32, String>,
) {
    let mut ids: Vec<u32> = maps.keys().copied().collect();
    ids.sort_unstable();

    println!("\npositional lights — the spheres laid over each map's default:");
    println!("  per map, how many centres land on one of that map's own tiles, by reading:");
    let mut total = 0usize;
    let (mut checked, mut on_land) = (0usize, [0usize; 4]);
    let (mut land_tiles, mut maps_checked) = (0usize, 0usize);
    for id in ids {
        let count = light.positional(id).count();
        total += count;
        if count == 0 {
            continue;
        }
        let wdt = assets
            .read(&vale_assets::wdt_path(&maps[&id]))
            .ok()
            .and_then(|raw| vale_assets::world::wdt::Wdt::parse(&raw).ok());
        let Some(wdt) = wdt else {
            println!("  {id:4} {:<22} {count:4} lights   (no WDT to check against)", maps[&id]);
            continue;
        };
        let tiles = wdt.tile_count();
        let mut hits = [0usize; 4];
        for row in light.positional(id) {
            for (reading, hit) in hits.iter_mut().enumerate() {
                if let Some((tx, ty)) = tile_of(row.raw, reading) {
                    *hit += usize::from(wdt.has_tile[ty][tx]);
                }
            }
        }
        // Only a map with *some* ground and *not all* of it can separate the
        // four readings; see the note above.
        let discriminates = tiles > 0 && tiles < 64 * 64;
        if discriminates {
            checked += count;
            land_tiles += tiles;
            maps_checked += 1;
            for (sum, hit) in on_land.iter_mut().zip(hits) {
                *sum += hit;
            }
        }
        println!(
            "  {id:4} {:<22} {count:4} lights  {:4} {:4} {:4} {:4}  of {tiles} tiles{}",
            maps[&id],
            hits[0],
            hits[1],
            hits[2],
            hits[3],
            if discriminates { "" } else { "   (not counted)" },
        );
    }
    println!("\n  {total} positional rows in all, {checked} on the {maps_checked} maps that can tell the readings apart:");
    for (reading, hit) in on_land.iter().enumerate() {
        let mark = if reading == 0 { "   <- the one this client reads" } else { "" };
        println!("    {hit:4}/{checked}  {:<38}{mark}", READINGS[reading]);
    }
    // The number that makes the four above mean something: a centre dropped at
    // random on those grids lands on land this often. A reading that scores
    // near it has told you nothing.
    if maps_checked > 0 {
        let base = land_tiles as f32 / (maps_checked as f32 * 64.0 * 64.0);
        println!(
            "    {:4.0}/{checked}  what randomly placed centres would score ({:.0}% of the grid is land)",
            base * checked as f32,
            base * 100.0,
        );
    }
}

/// The four ways the three stored floats can be read as a horizontal position,
/// in the order [`READINGS`] names them.
const READINGS: [&str; 4] = [
    "corner - z, corner - x  (placements')",
    "corner - x, corner - z  (transposed)",
    "z - corner, x - corner  (mirrored)",
    "x - corner, z - corner  (both)",
];

/// Which tile one reading of a row's raw floats lands on, or `None` for a
/// position outside the 64x64 grid. See [`READINGS`].
///
/// **Deliberately not `tile_for_position`**, which clamps: a clamped position
/// off the west of the map lands on column 0 and would be *counted* wherever
/// column 0 has land, which hands the wrong readings a score they have not
/// earned. Off the map is the answer here, and it is the interesting one.
fn tile_of(raw: [f32; 3], reading: usize) -> Option<(usize, usize)> {
    let (a, b) = (raw[0] * YARDS_PER_UNIT, raw[2] * YARDS_PER_UNIT);
    let origin = vale_assets::world::adt::MAP_ORIGIN;
    let (x, y) = match reading {
        0 => (origin - b, origin - a),
        1 => (origin - a, origin - b),
        2 => (b - origin, a - origin),
        _ => (a - origin, b - origin),
    };
    let tile = |world: f32| {
        let index = ((origin - world) / vale_assets::world::adt::TILE_SIZE).floor();
        (index >= 0.0 && index < 64.0).then_some(index as usize)
    };
    // Same axis swap `tile_for_position` makes: +Y gives the column, +X the row.
    Some((tile(y)?, tile(x)?))
}

/// Every map's daylight at noon, and the shape checks over all of them.
fn survey(
    light: &LightTables,
    maps: &std::collections::HashMap<u32, String>,
    kinds: &std::collections::HashMap<u32, MapKind>,
) {
    let mut ids: Vec<u32> = maps.keys().copied().collect();
    ids.sort_unstable();
    let own: Vec<u32> = ids.iter().copied().filter(|&m| light.has_own_light(m)).collect();

    println!(
        "Map.dbc has {} maps; {} name a light row of their own, {} borrow map 0's",
        ids.len(),
        own.len(),
        ids.len() - own.len(),
    );
    println!(
        "  (borrowing is a documented degradation, not an error — see LightTables::atmosphere)\n"
    );

    // The six float bands, surveyed the way the eighteen int bands were. This
    // is what names them: one of the six is a distance in the hundreds and one
    // never leaves 0..1, and no other pair in the table has that contrast.
    println!("the six float bands over {} lights at noon — the range is the naming:", own.len());
    for band in 0..6u32 {
        let mut values: Vec<f32> = own
            .iter()
            .filter_map(|&m| light.params_for(m))
            .filter_map(|p| light.band_value(p, band, NOON))
            .collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if values.is_empty() {
            println!("  band {band}   (absent)");
            continue;
        }
        let median = values[values.len() / 2];
        let (min, max) = (values[0], values[values.len() - 1]);
        // A band that never moves across nineteen zones is not a parameter this
        // version uses; one in the thousands is a distance in some unit; one
        // that goes negative cannot be either.
        let shape = if min == max {
            "  <- flat across all 19: unused in 1.12".to_string()
        } else if max > 1.0 {
            format!(
                "  <- /36 = {:.0}..{:.0} yards",
                min * YARDS_PER_UNIT,
                max * YARDS_PER_UNIT
            )
        } else if min < 0.0 {
            "  <- goes negative: a scaler, not a distance".to_string()
        } else {
            String::new()
        };
        println!("  band {band}   min {min:8.2}  median {median:8.2}  max {max:8.2}{shape}");
    }

    println!("\nnoon, per map:");
    let sky_of = |id: &u32| kinds.get(id).copied().unwrap_or(MapKind::World).has_sky();
    let (mut outdoors, mut interiors) = (Checks::default(), Checks::default());
    for &id in &own {
        let sky = light.atmosphere(id, NOON);
        if sky_of(&id) {
            outdoors.count(&sky);
        } else {
            interiors.count(&sky);
        }
        // The params id is on the line because zones *share* rows, and sharing
        // is what explains an odd one out: a battleground reading like a
        // dungeon is the game pointing two maps at one row, not a misread.
        println!(
            "  {id:4} {:<22} {:<12} params {:3}  sun {}  fill {}  zenith {}  fog {} {:5.0}..{:.0}y",
            maps[&id],
            format!("{:?}", kinds.get(&id).copied().unwrap_or(MapKind::World)).to_lowercase(),
            light.params_for(id).unwrap_or(0),
            rgb(sky.diffuse),
            rgb(sky.ambient),
            rgb(sky.sky[0]),
            rgb(sky.fog()),
            sky.fog_start,
            sky.fog_end,
        );
    }

    // **Split, because three of these checks are statements about daylight and
    // half the table is underground.** Scarlet Monastery's band 0 is torchlight
    // and its band 2 is a ceiling; asking a corridor to have a sky darker
    // overhead than at eye level is asking the wrong question, and answering it
    // in one pooled count is what turns a correct reading into a "LOOK".
    outdoors.report(own.iter().filter(|id| sky_of(id)).count(), "under a sky");
    interiors.report(
        own.iter().filter(|id| !sky_of(id)).count(),
        "under a ceiling — these are expected to miss",
    );
}

/// One map across its whole day, plus its raw bands — the trace half, for when
/// the survey says a zone is the odd one out.
fn one_map(light: &LightTables, id: u32, name: &str) {
    let Some(params) = light.params_for(id) else {
        println!("map {id} {name}: no light row and no map 0 to borrow from");
        return;
    };
    println!(
        "map {id} {name}: LightParams {params}{}",
        if light.has_own_light(id) {
            ""
        } else {
            "  (borrowed from map 0 — this map names no light of its own)"
        }
    );

    // Eight times of day rather than one, because the whole table is a function
    // of the hour and a client fixed at noon reads only one column of it. The
    // wrap across midnight is the part worth seeing move.
    println!(
        "\n  across the day — sun, fill, band 17, zenith, horizon/fog, fog distance:"
    );
    for hour in [0, 3, 6, 9, 12, 15, 18, 21] {
        let time = hour * DAY / 24;
        let sky = light.atmosphere(id, time);
        println!(
            "    {hour:02}:00  sun {}  fill {}  band17 {}  zenith {}  fog {} {:5.0}..{:.0}y",
            rgb(sky.diffuse),
            rgb(sky.ambient),
            rgb(sky.shadow),
            rgb(sky.sky[0]),
            rgb(sky.fog()),
            sky.fog_start,
            sky.fog_end,
        );
    }

    println!("\n  the dome at noon, zenith to horizon — must brighten downward:");
    let noon = light.atmosphere(id, NOON);
    for stop in 0..SKY_STOPS {
        let c = noon.sky[stop];
        let bar = "#".repeat(((c[0] + c[1] + c[2]) / 3.0 * 40.0) as usize);
        println!("    stop {stop}  band {:2}  {}  {bar}", stop + 2, rgb(c));
    }

    println!("\n  all 18 int bands at noon (10..12 are the cloud art this client does not draw):");
    for band in 0..18u32 {
        let named = labelled(
            INT_BAND_NAMES[band as usize],
            INT_BAND_NOTES[band as usize],
        );
        match light.band(params, band, NOON) {
            Some(c) => println!("    band {band:2}  {}  {named}", rgb(c)),
            None => println!("    band {band:2}  (missing row)  {named}"),
        }
    }

    println!("\n  all 6 float bands at noon:");
    for band in 0..6u32 {
        match light.band_value(params, band, NOON) {
            Some(v) => println!(
                "    band {band}  {v:10.3}  {}",
                labelled(
                    FLOAT_BAND_NAMES[band as usize],
                    FLOAT_BAND_NOTES[band as usize]
                )
            ),
            None => println!("    band {band}  (missing row)"),
        }
    }

    // **The zones inside the map**, which is what the default above is only the
    // background of. Widest first, which is the order they are laid over each
    // other in — see `LightTables::lights_at`.
    let rows: Vec<_> = light.positional(id).copied().collect();
    if rows.is_empty() {
        println!("\n  no positional lights: this map is its default light everywhere");
        return;
    }
    println!(
        "\n  {} positional lights, widest first — sun and fog of each against the default's {} {:.0}y:",
        rows.len(),
        rgb(noon.diffuse),
        noon.fog_end,
    );
    for row in &rows {
        let sky = light.atmosphere_at(id, row.at, NOON);
        let (tile_x, tile_y) = vale_assets::world::terrain::tile_for_position(row.at[0], row.at[1]);
        println!(
            "    light {:4} params {:3}  ({:7.0},{:7.0},{:5.0}) tile {tile_x:2},{tile_y:2}  r {:5.0}..{:<5.0}  sun {}  fog {} {:.0}y",
            row.id,
            row.params,
            row.at[0],
            row.at[1],
            row.at[2],
            row.falloff_start,
            row.falloff_end,
            rgb(sky.diffuse),
            rgb(sky.fog()),
            sky.fog_end,
        );
    }
}

/// The shape checks, counted over every light rather than asserted on one — a
/// transposition that survives one zone does not survive nineteen.
#[derive(Default)]
struct Checks {
    warm_sun: usize,
    cool_fill: usize,
    fill_dimmer: usize,
    dome_brightens: usize,
    fog_ordered: usize,
    shadow_dimmer: usize,
}

impl Checks {
    fn count(&mut self, sky: &Atmosphere) {
        let warmth = |c: [f32; 3]| c[0] - c[2];
        let sum = |c: [f32; 3]| c[0] + c[1] + c[2];
        self.warm_sun += (warmth(sky.diffuse) > 0.0) as usize;
        self.cool_fill += (warmth(sky.ambient) < 0.0) as usize;
        self.fill_dimmer += (sum(sky.ambient) < sum(sky.diffuse)) as usize;
        // The zenith against the four sky bands below it — not a monotonic ramp
        // (see `band::SKY_TOP`), and the fog at stop 5 is not sky at all.
        self.dome_brightens +=
            (1..SKY_STOPS - 1).all(|s| sum(sky.sky[s]) > sum(sky.sky[0])) as usize;
        self.fog_ordered += (sky.fog_start <= sky.fog_end && sky.fog_end > 0.0) as usize;
        // **A measurement about band 17, kept apart from any reading of it.**
        // It is never brighter than the fill, in every light surveyed. That was
        // once taken as naming it the colour ground in a baked `MCSH` shadow is
        // lit by; `Shaders\Pixel\terrain1.bls` applies `MCSH` as a flat
        // `shadow * 0.3 + 0.7` scalar with no colour in it, so the shape holds
        // and the conclusion does not. River-far is the open candidate — see
        // `tables::light`'s `band::SHADOW`. Nothing in the renderer reads the
        // band; this check exists so that a change to it is noticed.
        self.shadow_dimmer += (sum(sky.shadow) <= sum(sky.ambient)) as usize;
    }

    fn report(&self, total: usize, group: &str) {
        println!("\nthe shape checks over the {total} lights {group}:");
        let line = |n: usize, what: &str| {
            let mark = if n == total { "" } else { "   <- LOOK" };
            println!("  {n:3}/{total}  {what}{mark}");
        };
        line(self.warm_sun, "the sun is warm (band 0 red > blue)");
        line(self.cool_fill, "the fill is cool (band 1 red < blue)");
        line(self.fill_dimmer, "the fill never outshines the sun");
        line(
            self.dome_brightens,
            "the zenith is the darkest of the sky bands (2 against 3..6)",
        );
        line(self.fog_ordered, "the fog starts before it ends");
        line(
            self.shadow_dimmer,
            "band 17 is never brighter than the fill",
        );
    }
}

fn rgb(c: [f32; 3]) -> String {
    let byte = |v: f32| (v * 255.0).round() as i32;
    format!("{:3}/{:3}/{:3}", byte(c[0]), byte(c[1]), byte(c[2]))
}

/// A band's name, and what is known about it beside the name.
///
/// Both come from `tables::light`, which is the one place they are written.
/// This file kept its own copy of the eighteen and it drifted: its note on
/// band 17 still carried the shadow-colour reading `terrain1.bls` disproved.
fn labelled(name: &str, note: &str) -> String {
    match note.is_empty() {
        true => name.to_string(),
        false => format!("{name} — {note}"),
    }
}
