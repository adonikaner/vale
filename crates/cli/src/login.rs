//! `vale login` — enter the world once and dump what is nearby.

use crate::common::*;
use vale_config::Config;

/// Log a character into the world and report what the server says is around it.
///
/// This is the step that makes the project a *client* rather than a protocol
/// probe: after `CMSG_PLAYER_LOGIN` the server streams `SMSG_UPDATE_OBJECT`
/// blocks describing the player and every nearby entity, and we hold that state.
pub fn cmd_login(cfg: &Config, character: Option<&str>) -> Result<(), String> {
    use vale_assets::world::adt::{MAP_ORIGIN, TILE_SIZE};
    use vale_protocol::state::objects::ObjectManager;
    use vale_protocol::state::update::ObjectType;
    use std::time::Duration;

    let (mut session, chosen) = open_world(cfg, character)?;

    println!(
        "[login] entering world as {} (guid {}) on map {}",
        chosen.name, chosen.guid, chosen.map
    );
    session
        .player_login(chosen.guid)
        .map_err(|e| format!("[world] {e}"))?;

    let mut om = ObjectManager::new();
    let stats = session
        .pump(&mut om, Duration::from_secs(10), Duration::from_secs(3))
        .map_err(|e| format!("[world] {e}"))?;

    // The initial burst gives entries, not names. Ask about every distinct
    // entry, then pump again to collect the replies.
    let asked = session
        .request_unknown_creatures(&om)
        .map_err(|e| format!("[world] {e}"))?
        + session
            .request_unknown_gameobjects(&om)
            .map_err(|e| format!("[world] {e}"))?
        // Equipment: what a player is wearing is an item *entry*, and only the
        // server knows what it looks like — `Item.dbc` is not in the archives.
        + session
            .request_unknown_items(&om)
            .map_err(|e| format!("[world] {e}"))?;
    let q_stats = session
        .pump(&mut om, Duration::from_secs(6), Duration::from_secs(2))
        .map_err(|e| format!("[world] {e}"))?;
    println!(
        "[login] asked about {asked} entries, resolved {} creatures, {} game objects \
         and {} items",
        q_stats.creatures_resolved, q_stats.gameobjects_resolved, q_stats.items_resolved
    );

    println!(
        "\n[login] {} packets, {} object updates ({} compressed), {} monster moves",
        stats.packets, stats.updates, stats.compressed, stats.monster_moves
    );
    // **How much of the movement arrived in a bag** — see
    // `PumpStats::bagged_moves`. Zero is the ordinary quiet-server case and
    // says nothing; anything else is the state a client that ignores
    // `SMSG_COMPRESSED_MOVES` desyncs in, and the monster-move count above is
    // partly made of these.
    if stats.bagged_moves > 0 {
        println!(
            "         …of which {} arrived inside {} SMSG_COMPRESSED_MOVES bags",
            stats.bagged_packets, stats.bagged_moves
        );
    }
    for w in &stats.warnings {
        println!("  parse warning: {w}");
    }

    // **The world's clock, which arrives exactly once and is the index into the
    // light chain.** Printed here because it is the only check that it arrived
    // at all: a client that misses it does not fail, it stands at noon forever,
    // and a permanently sunlit world is the kind of wrong that looks deliberate.
    // Cross it with `vale light` for what the sky should be at this hour.
    match om.game_time {
        Some(t) => println!(
            "  world clock {:02}:{:02} on {}-{:02}-{:02}, {:.4} world minutes per second \
             ({} half-minutes into the day)",
            t.hour,
            t.minute,
            t.year,
            t.month + 1,
            t.day + 1,
            t.speed,
            t.half_minutes(),
        ),
        None => println!("  no SMSG_LOGIN_SETTIMESPEED — the world would be lit at noon"),
    }

    match om.player() {
        Some(p) => {
            let pos = p.position;
            println!("\n== in world ==");
            match pos {
                Some(pos) => {
                    println!(
                        "  player guid {} at ({:.1}, {:.1}, {:.1}) facing {:.2}",
                        p.low_guid(),
                        pos.x,
                        pos.y,
                        pos.z,
                        pos.orientation
                    );
                    // Which terrain tile are we standing on? This is the bridge
                    // between the protocol half and the asset half.
                    let tile_x = ((MAP_ORIGIN - pos.y) / TILE_SIZE).floor() as i32;
                    let tile_y = ((MAP_ORIGIN - pos.x) / TILE_SIZE).floor() as i32;
                    // Resolve the map's directory from Map.dbc rather than
                    // assuming 0 = Azeroth; this is the same lookup the GUI uses.
                    //
                    // **And from the map the login landed on rather than the
                    // one the character list named**, which are not always the
                    // same: a character whose instance was reset while it was
                    // logged out is relocated to the entrance inside
                    // `Player::LoadFromDB`, with no teleport packet of any
                    // kind. `SMSG_LOGIN_VERIFY_WORLD` is the only statement of
                    // it — see `state::movement::LoginVerifyWorld` — and this
                    // line printed `BlackrockDepths` for a character standing
                    // on Azeroth's Blackrock Mountain until it read it.
                    let map_id = stats.landed_on_map.unwrap_or(chosen.map);
                    let map_name = map_name_for(cfg, map_id)
                        .unwrap_or_else(|e| format!("<unresolved: {e}>"));
                    if map_id != chosen.map {
                        println!(
                            "  the character list said map {} — the instance was \
                             reset and the login landed on {map_id}",
                            chosen.map
                        );
                    }
                    println!(
                        "  standing on {} (map {}) tile {},{}",
                        map_name, map_id, tile_x, tile_y
                    );
                    println!("  → vale tile {map_name} {tile_x} {tile_y}");
                }
                None => println!("  player guid {} (no position yet)", p.low_guid()),
            }
            // **Ours is the other half of every reaction**, and a missing one is
            // invisible in a way a missing creature's is not: it makes *every*
            // unit in the world read neutral at once.
            match p.faction() {
                Some(f) => println!("  our faction template {f}"),
                None => println!("  our faction template is MISSING — everything will read neutral"),
            }
        }
        None => println!("\n  no object flagged UPDATEFLAG_SELF was received"),
    }

    println!("\n  {} entities known:", om.len());
    for (label, ty) in [
        ("players", ObjectType::Player),
        ("units", ObjectType::Unit),
        ("game objects", ObjectType::GameObject),
        ("items", ObjectType::Item),
        ("dynamic objects", ObjectType::DynamicObject),
        ("corpses", ObjectType::Corpse),
    ] {
        let n = om.of_type(ty).count();
        if n > 0 {
            println!("    {n:>4} {label}");
        }
    }

    // The archives, for the reaction column below. A failure here is a missing
    // column and not a failed login — this command's job is the *session*.
    let my_faction = om.player().and_then(|p| p.faction());
    let mut archives = open_assets(cfg)
        .map_err(|e| eprintln!("[assets] no archives: {e}"))
        .ok();
    let tables = archives
        .as_mut()
        .and_then(|assets| {
            open_display_tables(assets)
                .map_err(|e| eprintln!("[assets] no faction table: {e}"))
                .ok()
        });

    if let (Some(assets), Some(tables)) = (archives.as_mut(), tables.as_ref()) {
        solidity(&om, assets, tables);
    }

    let mut nearby: Vec<_> = om
        .iter()
        .filter(|e| e.position.is_some() && Some(e.guid) != om.player_guid)
        .collect();
    if let Some(me) = om.player().and_then(|p| p.position) {
        nearby.sort_by(|a, b| {
            let d = |p: &vale_protocol::state::update::Position| {
                (p.x - me.x).powi(2) + (p.y - me.y).powi(2) + (p.z - me.z).powi(2)
            };
            d(&a.position.unwrap())
                .partial_cmp(&d(&b.position.unwrap()))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        println!("\n  nearest entities:");
        for e in nearby.iter().take(12) {
            let p = e.position.unwrap();
            let dist = ((p.x - me.x).powi(2) + (p.y - me.y).powi(2) + (p.z - me.z).powi(2)).sqrt();
            let vitals = match (e.level(), e.health_label()) {
                (Some(level), Some(hp)) => format!("  level {level}, {hp}"),
                (Some(level), None) => format!("  level {level}"),
                _ => String::new(),
            };
            // **The faction template and what this client makes of it.** The
            // template is the one input to every friend-or-foe answer given
            // here — the sword cursor, the name plate's tint, Tab's pool, a
            // cast's binding — and the reaction beside it is the only place
            // where the server's answer and the archives are *crossed*. Neither
            // half is checkable from the other end: a wrong template reads as a
            // plausible neutral and so does a table that failed to load, and
            // both look like four separate interface bugs.
            let faction = match e.faction() {
                Some(f) => {
                    let reaction = tables
                        .as_ref()
                        .map(|t| t.reaction(my_faction, Some(f)))
                        .unwrap_or_default();
                    // **…and what the pointer would be**, which is the other
                    // half of the same cross: the reaction alone would put a
                    // sword over every neutral quest giver in the game, because
                    // `UNIT_NPC_FLAGS` is asked first. See
                    // `vale_assets::look::cursor`.
                    // **And the interaction gate**, which is the other half
                    // again: an enemy-faction vendor has a service and is not
                    // offered one. Both ranks come off the bare table here —
                    // this command holds no reputation and no group, so it is
                    // the base answer rather than the session's.
                    let towards = tables
                        .as_ref()
                        .map(|t| t.template_rank(my_faction, Some(f)))
                        .unwrap_or_default();
                    let back = tables
                        .as_ref()
                        .map(|t| t.template_rank(Some(f), my_faction))
                        .unwrap_or_default();
                    let cursor = vale_assets::look::cursor::over_unit(
                        e.npc_flags(),
                        vale_assets::look::cursor::can_interact(
                            e.npc_flags(),
                            e.unit_flags(),
                            towards,
                            back,
                        ),
                        vale_assets::tables::faction::can_attack(e.unit_flags(), reaction),
                        e.lootable(),
                    );
                    let cursor = match cursor {
                        Some(cursor) => format!("{cursor:?}"),
                        None => "Point".into(),
                    };
                    format!("  faction {f} -> {reaction:?}, cursor {cursor}")
                }
                None => "  faction -".into(),
            };
            println!(
                "    {:>7.1}y  {:<34}{}{}",
                dist,
                om.name_of(e),
                vitals,
                faction
            );
        }
    }

    if !stats.other.is_empty() {
        println!("\n  unhandled opcodes:");
        let mut rows: Vec<_> = stats.other.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1));
        for (name, count) in rows.iter().take(12) {
            println!("    {count:>4} x {name}");
        }
    }
    Ok(())
}

/// **What the server put in view, and whether this client would let you walk
/// through it.**
///
/// The second command that crosses the server's answers with the archives, and
/// it exists for the same reason `dress` does: neither half is checkable from
/// the other end. `vale objects` says which *models* carry a hull and knows
/// nothing about what is spawned; a session says which game objects are here and
/// what state they are in, and knows nothing about their geometry. Only the join
/// answers the question that was actually reported — *is that door solid* — and
/// the failure it is aimed at is silent in both directions: a state read wrong
/// walls off an open doorway, and a display id resolved wrong reports a
/// confident "no hull" for a door that has one.
///
/// The three rules are the renderer's own, called rather than restated:
/// [`game_object_is_solid`] and [`M2::collision`], out of `vale_assets`.
///
/// [`game_object_is_solid`]: vale_assets::world::collision::game_object_is_solid
/// [`M2::collision`]: vale_assets::world::m2::M2
fn solidity(
    om: &vale_protocol::state::objects::ObjectManager,
    assets: &mut vale_assets::Assets,
    tables: &vale_assets::tables::dbc::DisplayTables,
) {
    use vale_assets::world::collision::game_object_is_solid;
    use vale_protocol::state::update::ObjectType;
    use std::collections::HashMap;

    let objects: Vec<_> = om.of_type(ObjectType::GameObject).collect();
    if objects.is_empty() {
        return;
    }
    // Cached per display id: a courtyard of thirty lamp posts is one M2.
    let mut hulls: HashMap<u32, Option<usize>> = HashMap::new();
    let (mut shut, mut solid, mut hollow, mut unresolved) = (0, 0, 0, 0);
    let mut walked: Vec<(String, u32)> = Vec::new();

    for e in &objects {
        if !game_object_is_solid(e.game_object_state()) {
            continue;
        }
        shut += 1;
        let Some(display_id) = e.display_id() else {
            unresolved += 1;
            continue;
        };
        let triangles = *hulls.entry(display_id).or_insert_with(|| {
            let model = tables.game_object(display_id)?;
            if model.path.to_ascii_lowercase().ends_with(".wmo") {
                return None;
            }
            let bytes = assets.read(&model.path).ok()?;
            let m2 = vale_assets::world::m2::M2::parse(&bytes).ok()?;
            (!m2.collision.is_empty()).then(|| m2.collision.triangle_count())
        });
        match triangles {
            Some(_) => solid += 1,
            None => {
                hollow += 1;
                walked.push((om.name_of(e), display_id));
            }
        }
    }

    println!(
        "\n  {} game objects in view: {shut} shut, of which {solid} are solid here",
        objects.len()
    );
    println!(
        "    {hollow} carry no hull (the game's own \"walk through me\"), {unresolved} name no display id"
    );
    // Named rather than counted, because "a hull-less door" is the bug and
    // "a hull-less campfire" is the game, and only the name tells them apart.
    walked.sort();
    walked.dedup();
    for (name, display_id) in walked.iter().take(8) {
        println!("      walk through: {name} (display {display_id})");
    }

    templates(om, tables, &objects);
}

/// **What each visible game object *is*** — its type, its lock, and which
/// pointer that comes out as.
///
/// This is the only check the `data[24]` tail of `SMSG_GAMEOBJECT_QUERY_RESPONSE`
/// can have, and it needs one: the number of empty alternate names between the
/// name and the union differs between server builds, and **a reader that is off
/// by one still parses** — every word of the template comes back shifted by a
/// byte, which is a lock id in the hundreds of millions rather than an error.
/// See [`vale_protocol::state::query::parse_gameobject_response`], which
/// counts the tail backwards from the end for exactly that reason.
///
/// So the thing to look at is the **lock** column: a Copper Vein must read 38
/// and a Deadmines door 85, and both are two-digit numbers that a shifted read
/// cannot produce by accident. The `type` column is the same check one field
/// earlier and is the coarser of the two — every ore vein in the game is type 3.
fn templates(
    om: &vale_protocol::state::objects::ObjectManager,
    tables: &vale_assets::tables::dbc::DisplayTables,
    objects: &[&vale_protocol::state::objects::Entity],
) {
    use vale_assets::look::object::{over_object, Kind};
    use std::collections::BTreeMap;

    // One row per *entry*, not per placement: a corridor of eight identical
    // doors is one template and one thing to check.
    let mut rows: BTreeMap<u32, (String, u32, u32, usize)> = BTreeMap::new();
    let mut unresolved = 0usize;
    for e in objects {
        let Some(info) = om.gameobject_of(e) else {
            unresolved += 1;
            continue;
        };
        let kind = Kind::of(info.object_type);
        let lock = kind
            .lock_word()
            .and_then(|word| info.data.get(word).copied())
            .unwrap_or(0);
        rows.entry(info.entry)
            .or_insert_with(|| (info.name.clone(), info.object_type, lock, 0))
            .3 += 1;
    }
    if rows.is_empty() {
        return;
    }
    println!(
        "
  {} distinct templates in view ({unresolved} still waiting on CMSG_GAMEOBJECT_QUERY):",
        rows.len()
    );
    println!("    entry  type  lock  n  pointer        name");
    for (entry, (name, object_type, lock, count)) in rows.iter().take(24) {
        let kind = Kind::of(*object_type);
        let pointer = match over_object(kind, tables.locks(), *lock) {
            Some(cursor) => format!("{cursor:?}"),
            None => "-".to_string(),
        };
        let requires = tables
            .locks()
            .skill_lock(*lock)
            .and_then(|(lock_type, rank)| {
                Some(format!("  requires {} {rank}", tables.locks().lock_type_name(lock_type)?))
            })
            .unwrap_or_default();
        println!("    {entry:>5}  {object_type:>4}  {lock:>4}  {count:>1}  {pointer:<13}  {name}{requires}");
    }
    if rows.len() > 24 {
        println!("    … and {} more", rows.len() - 24);
    }
    let (locks, lock_types) = tables.locks().counts();
    println!("    Lock.dbc: {locks} locks, LockType.dbc: {lock_types} named");
}

