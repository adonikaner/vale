//! Opening things, and naming things: the helpers every subcommand needs.
//!
//! Nothing here is a command. What lands in this module is whatever more than
//! one of them does — logging on, opening the archive chain, parsing the
//! display tables — and the point of it being one copy is the same as
//! everywhere else in this project: the realm choice and the missing-port
//! fallback cannot drift apart between `login`, `live` and `dress` if there is
//! only one of them.

use vale_assets::Assets;
use vale_config::Config;
use vale_protocol::{socket::auth, state::query, socket::world};

/// Race and class names come from the protocol crate, which needs the same
/// tables to render `SMSG_NAME_QUERY_RESPONSE` for other players.
pub fn race_name(r: u8) -> &'static str {
    or_unknown(query::race_name(r as u32))
}

pub fn class_name(c: u8) -> &'static str {
    or_unknown(query::class_name(c as u32))
}

pub fn or_unknown(name: &'static str) -> &'static str {
    if name.is_empty() {
        "?"
    } else {
        name
    }
}


/// Log on, connect to the world server, and pick a character.
///
/// The three protocol subcommands all start this way; keeping it in one place
/// means the realm choice and the missing-port fallback cannot drift apart
/// between them.
pub fn open_world(
    cfg: &Config,
    character: Option<&str>,
) -> Result<(world::WorldSession, world::CharListEntry), String> {
    check_credentials(cfg)?;
    let outcome = auth::login(&cfg.host, &cfg.account, &cfg.password)
        .map_err(|e| format!("[auth] {e}"))?;
    let world_addr = normalize_addr(&outcome.realms.first().ok_or("no realms returned")?.address);

    let mut session = world::WorldSession::connect(&world_addr, &cfg.account, outcome.session_key)
        .map_err(|e| format!("[world] {e}"))?;
    let chars = session.char_enum().map_err(|e| format!("[world] {e}"))?;

    let chosen = match character {
        Some(name) => chars
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                let names: Vec<&str> = chars.iter().map(|c| c.name.as_str()).collect();
                format!("no character named {name:?}; have: {}", names.join(", "))
            })?,
        None => chars.first().ok_or("account has no characters")?,
    };
    Ok((session, chosen.clone()))
}


/// Map id -> the directory under `World\Maps\`, straight from `Map.dbc`.
pub fn map_name_for(cfg: &Config, map_id: u32) -> Result<String, String> {
    use vale_assets::tables::dbc::{dbc_path, map_directories};
    let mut assets = open_assets(cfg)?;
    let raw = assets.read(&dbc_path("Map")).map_err(|e| e.to_string())?;
    let dirs = map_directories(&raw).map_err(|e| e.to_string())?;
    dirs.get(&map_id)
        .cloned()
        .ok_or_else(|| format!("map {map_id} not in Map.dbc"))
}

/// **Refuse before the socket** when the folder has not named an account or the
/// environment has not supplied a password.
///
/// This project has been bitten twice by realmd's own refusal codes pointing the
/// wrong way — `VERSION_INVALID` blamed the client build when the password had
/// already passed, and `WOW_FAIL_UNKNOWN_ACCOUNT` blamed the credentials for a
/// byte-count bug. An empty password produces one of those, and the cause is
/// neither: it is a headless run with nowhere to have got one from. Saying so
/// here costs one `if` and saves the whole dig.
pub fn check_credentials(cfg: &Config) -> Result<(), String> {
    if cfg.account.is_empty() {
        return Err(format!(
            "no account: set {} or log in once so `accountName` is saved to WTF/Config.wtf",
            vale_config::ACCOUNT_ENV
        ));
    }
    if cfg.password.is_empty() {
        return Err(format!(
            "no password: set {} — the reference client stores none, so a headless \
             run has to be told (see crates/config/src/lib.rs)",
            vale_config::PASSWORD_ENV
        ));
    }
    Ok(())
}

/// Realm addresses sometimes omit the port; default to the vmangos world port.
pub fn normalize_addr(addr: &str) -> String {
    if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{addr}:8085")
    }
}

// ------------------------------------------------------------------ assets --

/// The four display DBCs plus the two geoset tables, as the renderer opens them.
pub fn open_display_tables(
    assets: &mut Assets,
) -> Result<vale_assets::tables::dbc::DisplayTables, String> {
    use vale_assets::tables::dbc::{dbc_path, DisplayTables};
    let mut tables =
        DisplayTables::load(|table| assets.read(&dbc_path(table)).ok()).map_err(|e| e.to_string())?;
    // …and the world map's one file, which is not a table — see
    // `DisplayTables::load_zone_grids`.
    tables.load_zone_grids(|path| assets.read(path).ok());
    Ok(tables)
}

/// The archive chain, with the install folder's `Interface\AddOns\` read under
/// it — see `Assets::with_loose_root`.
pub fn open_assets(cfg: &Config) -> Result<Assets, String> {
    let assets = Assets::open(&cfg.gamedata)
        .map_err(|e| e.to_string())?
        .with_loose_root(&cfg.root);
    for (name, why) in &assets.failures {
        eprintln!("[assets] WARNING could not open {name}: {why}");
    }
    Ok(assets)
}


/// `compression/alphaDepth/alphaType` straight out of the BLP header, for the
/// summary above — this is the classification the decoder branches on.
pub fn describe_blp(buf: &[u8]) -> String {
    if buf.len() < 12 || &buf[0..4] != b"BLP2" {
        return "not-blp2".to_string();
    }
    let kind = match buf[8] {
        1 => "palettized",
        2 => "dxt",
        3 => "raw-bgra",
        other => return format!("compression {other}"),
    };
    format!("{kind} alphaDepth={} alphaType={}", buf[9], buf[10])
}

/// **What a unit is holding, as the dressing rule wants it.**
///
/// Shared because `login` and `dress` both ask the same question of the same
/// two fields and a second copy of the mapping is a second answer to "which
/// hand is this in".
pub fn held(item: vale_protocol::state::objects::HeldItem) -> vale_assets::tables::item::Weapon {
    vale_assets::tables::item::Weapon {
        display_id: item.display_id,
        class: item.class,
        subclass: item.subclass,
        inventory_type: item.inventory_type,
        sheath: item.sheath,
        material: item.material,
    }
}

