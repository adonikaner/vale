//! `vale weather` — **what the sky can do, checked against the archives.**
//!
//! Weather is one packet and no table, so there is nothing to census the way
//! `vale light` walks `Light.dbc`. What there is to check is the reference's
//! own inventory: four textures, three vertex shaders, nine ambience loops, and
//! the two rules the render pass is built on — the density curve and the ramp.
//! Every line here is either a file that must open or a number the renderer
//! uses.

use vale_config::Config;
use vale_protocol::play::weather::{Weather, DENSITY_FLOOR, RAMP_SECONDS_PER_UNIT};
use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::sound::SoundBank;

/// The reference's weather art.
const TEXTURES: [&str; 4] = [
    r"textures\Weather\RainDrop01.blp",
    r"textures\Weather\RainDropSplash01.blp",
    r"textures\Weather\SnowFlake01.blp",
    r"textures\Weather\SnowMist01.blp",
];

/// …and the three programs that draw it — the specification.
const SHADERS: [&str; 3] = [
    r"Shaders\Vertex\rain.bls",
    r"Shaders\Vertex\snowpoint.bls",
    r"Shaders\Vertex\sand.bls",
];

/// The nine loops vmangos sends — `Weather.cpp`'s `WeatherSounds` enum, "only
/// for 1.12" — light, medium and heavy of each kind.
const SOUNDS: [(&str, [u32; 3]); 3] = [
    ("rain", [8533, 8534, 8535]),
    ("snow", [8536, 8537, 8538]),
    ("sandstorm", [8556, 8557, 8558]),
];

pub fn cmd_weather(cfg: &Config, _arg: Option<&str>) -> Result<(), String> {
    let mut assets = crate::common::open_assets(cfg)?;

    println!("== the wire ==");
    println!("  SMSG_WEATHER  u32 type (0 fine, 1 rain, 2 snow, 3 storm)  f32 grade  u32 sound  u8 instant");
    println!();

    println!("== textures\\Weather ==");
    for path in TEXTURES {
        match assets.read(path) {
            Ok(raw) => match vale_assets::world::blp::decode(&raw) {
                Ok(blp) => println!("  {path:<44} {}x{}", blp.width, blp.height),
                Err(e) => println!("  {path:<44} ** will not decode: {e}"),
            },
            Err(e) => println!("  {path:<44} ** MISSING: {e}"),
        }
    }
    println!();

    println!("== Shaders\\Vertex ==");
    for path in SHADERS {
        match assets.read(path) {
            Ok(raw) => {
                let magic = String::from_utf8_lossy(&raw[..raw.len().min(4)]).to_string();
                let programs = raw.windows(9).filter(|w| w == b"!!ARBvp1.").count();
                println!("  {path:<44} {} bytes, magic {magic}, {programs} program(s)", raw.len());
            }
            Err(e) => println!("  {path:<44} ** MISSING: {e}"),
        }
    }
    println!();

    println!("== Sound\\Ambience\\Weather — the nine loops ==");
    let bank = SoundBank::load(|table| assets.read(&dbc_path(table)).ok())
        .map_err(|e| e.to_string())?;
    for (kind, ids) in SOUNDS {
        for (grade, id) in ["light", "medium", "heavy"].iter().zip(ids) {
            match bank.entry(id) {
                Some(entry) => {
                    let ok = entry
                        .files
                        .iter()
                        .filter(|(file, _)| assets.read(&entry.path(file)).is_ok())
                        .count();
                    println!(
                        "  {id}  {kind:<9} {grade:<6} \"{}\"  {} file(s), {ok} open",
                        entry.name,
                        entry.files.len()
                    );
                }
                None => println!("  {id}  {kind:<9} {grade:<6} ** not in SoundEntries.dbc"),
            }
        }
    }
    println!();

    println!("== the two rules (see play::weather) ==");
    println!("  density = max(0, (grade - {DENSITY_FLOOR}) * 4/3)");
    println!("  ramp    = |to - from| * {RAMP_SECONDS_PER_UNIT} s");
    println!("  grade   density   drops   flakes   mist");
    for step in 0..=8 {
        let grade = step as f32 / 8.0;
        let d = Weather::density(grade);
        println!(
            "  {grade:>5.3}   {d:>5.3}   {:>5}   {:>6}   {:>4}",
            (6500.0 * 0.66 * d) as u32,
            (1300.0 * 0.66 * d) as u32,
            (128.0 * d.max(0.25)) as u32,
        );
    }
    println!("  boxes: 44 x 44 x 25 yards around the camera, 128 mist");
    println!("  mist sizes: rain 1.2..5.0, snow 3.0..9.0; spread 20 deg");
    println!();

    storm_light(&mut assets)?;
    Ok(())
}

/// **The half of weather that is not particles**: `Light.dbc`'s third
/// `LightParams` column, which is what the world is lit by while it rains.
///
/// The particles are the loud half and the light is the one a player actually
/// reads — a downpour under Elwynn's noon sun is the picture this check was
/// written for. Every map's clear row and its storm row at noon, side by side,
/// so that "the sky darkens" is a pair of numbers rather than an impression.
fn storm_light(assets: &mut vale_assets::Assets) -> Result<(), String> {
    use vale_assets::tables::light::{Weather, NOON};

    println!("== the light a storm is drawn in — Light.dbc column 2 ==");
    let raw = assets
        .read(&dbc_path("Map"))
        .map_err(|e| format!("Map.dbc: {e}"))?;
    let maps = vale_assets::tables::dbc::map_directories(&raw).map_err(|e| e.to_string())?;
    let tables = crate::common::open_display_tables(assets)?;
    let Some(light) = tables.light() else {
        println!("  no light chain: the sky does not change when it rains");
        return Ok(());
    };

    println!("  at noon; the sun and the fill are bands 9 and 10, the fog is where the world ends");
    println!(
        "  {:<22} {:>5} {:>5}   {:<27}  {:<27}  {}",
        "map", "clear", "storm", "sun clear -> storm", "fill clear -> storm", "fog"
    );
    let mut ids: Vec<u32> = maps.keys().copied().collect();
    ids.sort_unstable();
    let (mut own, mut named, mut darker, mut closer) = (0usize, 0usize, 0usize, 0usize);
    for id in ids {
        if !light.has_own_light(id) {
            continue;
        }
        let (Some(clear_id), Some(storm_id)) = (
            light.params_for_weather(id, Weather::Clear),
            light.params_for_weather(id, Weather::Storm),
        ) else {
            continue;
        };
        own += 1;
        if storm_id != clear_id {
            named += 1;
        }
        let clear = light.atmosphere_weather(id, NOON, Weather::Clear);
        let storm = light.atmosphere_weather(id, NOON, Weather::Storm);
        let byte = |c: [f32; 3]| {
            let c = c.map(|v| (v * 255.0) as i32);
            format!("{:>3},{:>3},{:>3}", c[0], c[1], c[2])
        };
        let sum = |c: [f32; 3]| c[0] + c[1] + c[2];
        if sum(storm.diffuse) < sum(clear.diffuse) {
            darker += 1;
        }
        if storm.fog_end < clear.fog_end {
            closer += 1;
        }
        println!(
            "  {:<22} {clear_id:>5} {storm_id:>5}   {} -> {}  {} -> {}  {:>4.0} -> {:.0}",
            maps[&id],
            byte(clear.diffuse),
            byte(storm.diffuse),
            byte(clear.ambient),
            byte(storm.ambient),
            clear.fog_end,
            storm.fog_end,
        );
    }
    println!();
    println!(
        "  {own} maps name a light of their own, {named} of them a storm row that is not the clear one"
    );
    println!("  {darker} are darker in a storm and {closer} fog in closer");
    println!("  what is between the two rows is the grade, which is this client's reading —");
    println!("  see LightTables::atmosphere_in_storm; the rows themselves are the table's");
    Ok(())
}
