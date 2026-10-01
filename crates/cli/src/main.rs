//! `vale`: a headless driver for the protocol and asset layers.
//!
//! ```text
//! main.rs        the command table only
//! common.rs      code more than one command uses: logging on, opening the
//!                archives, parsing the display tables
//!
//! the server:
//! connect.rs     SRP6 logon -> world handshake -> character list
//! login.rs       enter the world once and dump what is nearby
//! live.rs        enter the world and stay, with a command line of its own
//! wdb.rs         the WDB\ cache that login and live write: every cache file,
//!                its records, each body re-parsed
//! dress.rs       every player in view, dressed as the renderer would dress
//!                them; the only command that combines server data with the
//!                archives
//!
//! the ground, tile by tile:
//! archives.rs    open the MPQ chain and report what mounted
//! extract.rs     any file in the MPQ chain, written out or hexdumped
//! catalogue.rs   the listing of what may be placed in the world, folder by
//!                folder, with the listed paths that do not open
//! maps.rs        every map that has a WDT, with its tile count
//! map.rs         the tile grid of one map
//! tile.rs        one ADT: chunks, heights, and what it places
//! mcnk.rs        one ADT's raw MCNK headers and height grids
//! textures.rs    one ADT's texture layers, alpha maps, and the atlas round trip
//! models.rs      every M2 an ADT places: decode, place, verify
//! wmos.rs        every WMO an ADT places, checked against its MODF record
//! collision.rs   the collision geometry of those WMOs
//! water.rs       the liquid in an ADT
//! foliage.rs     an ADT's ground effects, cell by cell
//! light.rs       the world's daylight, per map, band by band
//! weather.rs     weather: four textures, three shaders, nine loops and two
//!                rules, each checked against the archives
//! sun.rs         the sun's position, derived from a tile's baked shadows
//! bake.rs        the two images an editor has to write back, a tile's minimap
//!                and its MCSH, each scored against the shipped one
//! sky.rs         what is drawn in the sky: the stars, the arcs, the sprites
//!
//! what a thing looks like:
//! npc.rs         every display id -> a model and a skin      (vale npc)
//! objects.rs     the server's game object table: which objects are solid
//! character.rs   a player's composed skin, layer by layer    (vale char)
//! item.rs        a garment's components, regions and geosets
//! itemset.rs     an item plate's set block, enchantment lines and random
//!                suffixes, from ItemSet, SpellItemEnchantment and
//!                ItemRandomProperties
//! itemvisual.rs  the glows and flames on held items, from ItemVisuals,
//!                ItemVisualEffects and the two columns that name them
//! attach.rs      the item models attached to a bone rather than drawn as
//!                part of the skin
//! anim.rs        every creature model: bones, sequences, poses
//! emote.rs       Emotes.dbc -> AnimationData -> what a model can play
//! emotetext.rs   the text of an emote such as /dance: three tables joined,
//!                every token checked
//! bank.rs        what a bank bag slot costs, and the bank's three slot
//!                numberings compared
//! spell.rs       Spell -> SpellVisual -> SpellVisualKit -> a caster's pose
//! particles.rs   every emitter that two DBC populations name
//! model.rs       one M2 as the renderer sees it
//! pick.rs        the parts of an M2 the mouse can hit
//! portrait.rs    the camera position for an M2's portrait
//! pet.rs         pet data the server never sends: the happiness bands, the
//!                families and their diets, from four DBCs
//! reputation.rs  what the reputation panel draws, including the faction list
//! skills.rs      the skills panel, including its headings
//! talent.rs      the talent trees, whose layout is entirely a client rule
//! tradeskill.rs  the profession windows: the openers, the thresholds, the
//!                difficulty formula, every reagent reference checked
//! cursor.rs      the mouse pointers, and each one's hot spot
//! sound.rs       the sound tables checked against each other and the archives
//!
//! what the interface is made of:
//! bindings.rs    Bindings.xml: 234 key bindings and what each one calls
//! messages.rs    the interface message table, key by key
//! combatlog.rs   the combat log text: every key the log can produce, every
//!                window a line can land in, and a sample line of each
//! framexml.rs    the whole interface: load graph, markup, and the API gap
//! addons.rs      every addon under Interface\AddOns\: its files checked, the
//!                load order, and every character's AddOns.txt
//! channels.rs    the six zone chat channels, and which of them a place is in
//! who.rs         the /who query, the one line of typed text the client parses
//!                itself, checked against the three tables it joins
//! glue.rs        the screens shown before the world: art, models, cameras
//! loading.rs     the loading screen shown between them: the field-38 join
//! mail.rs        mail stationery: five rows, one rule
//! spellbook.rs   which spells become action buttons          (vale spellbook)
//! spelltemplate.rs  vmangos' spell_template: its 151 columns, the Spell.dbc
//!                field that feeds each, and the statements an edit becomes
//!                (vale spelltemplate)
//! itemtemplate.rs  vmangos' item_template: its 129 columns, which no client
//!                file backs, and the appearance chain the archives do hold
//!                (vale itemtemplate)
//! questtemplate.rs vmangos' quest_template: its 131 columns and the four
//!                relation tables that say who gives and takes a quest
//!                (vale questtemplate)
//! spawns.rs      vmangos' creature_template and the creature rows that place
//!                NPCs in the world, each joined to its model and whether the
//!                archives hold it                              (vale spawns)
//! waypoints.rs   creature_movement: an NPC's path, every node checked against
//!                the terrain the archives draw               (vale waypoints)
//! navmesh.rs     the server's mmaps navmesh: every tile read, by surface
//!                class                                         (vale navmesh)
//! book.rs        the spellbook panel: the tabs, in the client's order
//! charcreate.rs  what a character may be made of             (vale create)
//! minimap.rs     the minimap's tile index, and the orientation of each image
//! worldmap.rs    the world map: the zones, and what each highlights
//!                                                            (vale zones)
//! areatrigger.rs the volumes a dungeon portal is made of     (vale triggers)
//! taxi.rs        the flight map: three tables, the projection, the routes
//! ships.rs       the boats and zeppelins that share the taxi path table,
//!                whose position no packet states               (vale ships)
//! dbc.rs         every table's shape and round trip, and one  (vale dbc)
//!                table field by field
//! ```
//!
//! Each command has one file. Four files are named differently from their
//! command (`character`, `charcreate`, `worldmap`, `areatrigger`); they use the
//! module names that `assets`, `protocol`, `game` and `lua` use for the same
//! subject, so a subject has the same name in every crate.
//!
//! Every subcommand is read-only; nothing here writes game data.
//!
//! This crate is the project's verification harness. Most facts the renderer
//! relies on were established by one of these commands, which measure the
//! game's files against a source that did not come from this client: a model's
//! declared bounding box, the game's `MODF` placements, a neighbouring chunk's
//! alpha map, the size of the file that fills a region. `dress` is the only
//! command that combines the server's answers with the archives.
//!
//! There is one module per subject; this file holds only the command table.

mod addons;
mod anim;
mod archives;
mod areatrigger;
mod attach;
mod bake;
mod bindings;
mod book;
mod character;
mod charcreate;
mod collision;
mod common;
mod connect;
mod cursor;
mod dbc;
mod dress;
mod emote;
mod emotetext;
mod bank;
mod extract;
mod foliage;
mod framexml;
mod glue;
mod item;
mod light;
mod live;
mod wdb;
mod loading;
mod mail;
mod login;
mod map;
mod catalogue;
mod maps;
mod mcnk;
mod combatlog;
mod messages;
mod minimap;
mod model;
mod models;
mod npc;
mod objects;
mod particles;
mod pet;
mod pick;
mod portrait;
mod reputation;
mod skills;
mod talent;
mod sky;
mod sound;
mod spell;
mod spellbook;
mod spawns;
mod waypoints;
mod navmesh;
mod itemset;
mod itemvisual;
mod itemtemplate;
mod questtemplate;
mod spelltemplate;
mod sun;
mod ships;
mod taxi;
mod textures;
mod tradeskill;
mod tile;
mod water;
mod weather;
mod who;
mod channels;
mod wmos;
mod worldmap;
use vale_config::Config;

/// Every subcommand the match below accepts, in the order the arms appear.
///
/// The "unknown subcommand" message prints this list. It previously kept its
/// own copy, which fell eight commands behind the match: `extract`, `glue`,
/// `sky`, `zones`, `triggers`, `messages`, `combatlog` and `tradeskill` were
/// accepted but not named, so they could only be found by reading this file.
/// `every_subcommand_is_listed` checks that this list and the match agree.
const SUBCOMMANDS: &[&str] = &[
    "connect",
    "login",
    "live",
    "wdb",
    "dress",
    "archives",
    "extract",
    "glue",
    "cursor",
    "maps",
    "catalogue",
    "map",
    "tile",
    "mcnk",
    "textures",
    "models",
    "wmos",
    "collision",
    "water",
    "foliage",
    "light",
    "sky",
    "sun",
    "npc",
    "objects",
    "dbc",
    "char",
    "item",
    "itemset",
    "itemvisual",
    "attach",
    "anim",
    "emote",
    "emotetext",
    "bank",
    "spell",
    "sound",
    "weather",
    "spellbook",
    "spelltemplate",
    "itemtemplate",
    "questtemplate",
    "spawns",
    "waypoints",
    "navmesh",
    "book",
    "create",
    "zones",
    "minimap",
    "bake",
    "loading",
    "mail",
    "triggers",
    "taxi",
    "ships",
    "bindings",
    "messages",
    "combatlog",
    "framexml",
    "addons",
    "who",
    "channels",
    "particles",
    "pet",
    "model",
    "pick",
    "portrait",
    "reputation",
    "skills",
    "talent",
    "tradeskill",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = Config::load();
    let arg = |n: usize| args.get(n).map(String::as_str);
    let id = |n: usize| args.get(n).and_then(|s| s.parse().ok());

    let result = match args.first().map(String::as_str) {
        None | Some("connect") => connect::cmd_connect(&cfg),
        Some("login") => login::cmd_login(&cfg, arg(1)),
        Some("live") => live::cmd_live(&cfg, arg(1)),
        Some("wdb") => wdb::cmd_wdb(&cfg),
        Some("dress") => dress::cmd_dress(&cfg, arg(1)),

        Some("archives") => archives::cmd_archives(&cfg),
        Some("extract") => match arg(1) {
            Some(path) => extract::cmd_extract(&cfg, path, arg(2)),
            None => Err("usage: vale extract <archive\\path> [out-file]".to_string()),
        },
        Some("glue") => glue::cmd_glue(&cfg),
        Some("cursor") => cursor::cmd_cursor(&cfg, arg(1)),
        Some("maps") => maps::cmd_maps(&cfg),
        Some("catalogue") => catalogue::cmd_catalogue(&cfg, arg(1)),
        Some("map") => match arg(1) {
            Some(name) => map::cmd_map(&cfg, name),
            None => Err("usage: vale map <MapName>".to_string()),
        },
        Some("tile") => tile(&args, |name, x, y| tile::cmd_tile(&cfg, name, x, y)),
        Some("mcnk") => tile(&args, |name, x, y| mcnk::cmd_mcnk(&cfg, name, x, y)),
        // Two forms, as for `foliage`: with a tile, that tile's layers in full;
        // without one, the `MCLY` flag census over a whole map, which is how the
        // texture-animation bits were identified. See `cli::textures`.
        Some("textures") if args.len() >= 4 => {
            tile(&args, |name, x, y| textures::cmd_textures(&cfg, name, x, y))
        }
        Some("textures") => textures::cmd_layer_flags(&cfg, arg(1)),
        Some("models") => tile(&args, |name, x, y| models::cmd_models(&cfg, name, x, y)),
        Some("wmos") => tile(&args, |name, x, y| wmos::cmd_wmos(&cfg, name, x, y)),
        // Two more arguments after the tile restrict the query to one world
        // position: `vale collision Azeroth 32 48 -8869.2 -163.2` reports the
        // surfaces under that point, for example under a creature drawn
        // standing in the air.
        Some("collision") => match (args.get(4), args.get(5)) {
            (Some(px), Some(py)) => match (px.parse::<f32>(), py.parse::<f32>()) {
                // The z is optional. With it, each hull returns the surface
                // below that height, which for a building is the floor the
                // creature stands on rather than the roof.
                (Ok(px), Ok(py)) => match args.get(6).map(|z| z.parse::<f32>()) {
                    Some(Err(_)) => Err("the height must be a number".to_string()),
                    pz => {
                        let pz = pz.map(|z| z.unwrap_or_default());
                        tile(&args, |name, x, y| {
                            collision::cmd_collision(&cfg, name, x, y, Some((px, py, pz)))
                        })
                    }
                },
                _ => Err("the world position must be two numbers".to_string()),
            },
            _ => tile(&args, |name, x, y| collision::cmd_collision(&cfg, name, x, y, None)),
        },
        Some("water") => tile(&args, |name, x, y| water::cmd_water(&cfg, name, x, y)),
        // Two forms: the whole table checked against the archives, or one
        // tile's ground effects cell by cell. See `cli::foliage`.
        Some("foliage") if args.len() >= 4 => {
            tile(&args, |name, x, y| foliage::cmd_foliage_tile(&cfg, name, x, y))
        }
        Some("foliage") => foliage::cmd_foliage(&cfg),
        Some("light") => light::cmd_light(&cfg, arg(1)),
        Some("sky") => sky::cmd_sky(&cfg),
        Some("sun") => tile(&args, |name, x, y| sun::cmd_sun(&cfg, name, x, y)),

        Some("npc") => npc::cmd_npc(&cfg, id(1)),
        Some("objects") => objects::cmd_objects(&cfg, arg(1)),
        Some("dbc") => match arg(1) {
            Some(table) => dbc::cmd_dbc(&cfg, table, id(2)),
            // With no table name: the census and round trip over every table.
            None => dbc::cmd_dbc_all(&cfg),
        },
        Some("char") => character::cmd_char(&cfg, arg(1)),
        Some("item") => item::cmd_item(&cfg, id(1)),
        // Three forms: the census, one set, or `suffix <id>` for one random
        // property. See `cli::itemset`.
        Some("itemset") => itemset::cmd_itemset(&cfg, arg(1), arg(2)),
        // The census, or one display id with an optional enchantment. See
        // `cli::itemvisual`.
        Some("itemvisual") => itemvisual::cmd_itemvisual(&cfg, arg(1), arg(2)),
        Some("attach") => attach::cmd_attach(&cfg, id(1)),
        Some("anim") => anim::cmd_anim(&cfg, arg(1)),
        Some("emote") => emote::cmd_emote(&cfg),
        Some("emotetext") => emotetext::cmd_emotetext(&cfg, arg(1)),
        Some("bank") => bank::cmd_bank(&cfg),
        Some("spelltemplate") => spelltemplate::cmd_spelltemplate(&cfg, arg(1)),
        // Four forms: no argument prints the schema, a number prints one item,
        // and `display` and `db` run the two censuses. See `cli::itemtemplate`.
        Some("itemtemplate") => itemtemplate::cmd_itemtemplate(&cfg, arg(1)),
        // Three forms: the schema, one quest followed through its tables, or
        // `db`. See `cli::questtemplate`.
        Some("questtemplate") => questtemplate::cmd_questtemplate(&cfg, arg(1)),
        // Three forms: no argument covers every map, a name covers one map, and
        // a number follows one creature entry through its tables. See
        // `cli::spawns`.
        Some("spawns") => spawns::cmd_spawns(&cfg, arg(1)),
        // The same three forms as `spawns`, over the movement tables. See
        // `cli::waypoints`.
        Some("waypoints") => waypoints::cmd_waypoints(&cfg, arg(1)),
        // Every tile of the server's navmesh, or one tile. See `cli::navmesh`.
        Some("navmesh") => navmesh::cmd_navmesh(&cfg, &args),
        Some("spell") => spell::cmd_spell(&cfg, id(1)),
        Some("sound") => sound::cmd_sound(&cfg, arg(1)),
        Some("weather") => weather::cmd_weather(&cfg, arg(1)),
        Some("spellbook") => spellbook::cmd_spellbook(&cfg, id(1)),
        Some("book") => book::cmd_book(&cfg, arg(1)),
        Some("create") => charcreate::cmd_create(&cfg, arg(1)),
        // Three forms: no argument prints the whole table, a number follows one
        // area through its tables, and a map plus a tile runs the building
        // check. See `cli::worldmap`.
        Some("zones") if args.len() >= 4 => {
            tile(&args, |name, x, y| worldmap::cmd_zone_buildings(&cfg, name, x, y))
        }
        Some("zones") => worldmap::cmd_zones(&cfg, id(1)),
        // Three forms, as for `zones`: the whole index, one map, or one tile.
        // The one-tile form is the orientation check. See `cli::minimap`.
        Some("minimap") if args.len() >= 4 => {
            tile(&args, |name, x, y| minimap::cmd_minimap(&cfg, Some(name), Some((x, y))))
        }
        Some("minimap") => minimap::cmd_minimap(&cfg, arg(1), None),
        // The two images an editor has to write (a tile's minimap and its
        // MCSH), scored against the ones the game shipped. `cli::bake` explains
        // why these two are the only parts of an edit that need a measurement.
        // The `minimap` form bakes the minimap image alone over a sweep of
        // settings. The shadow bake takes two minutes, and none of the
        // minimap's settings can be derived from a file; they can only be
        // measured against the shipped image.
        Some("bake") if args.len() >= 5 && args[4] == "minimap" => {
            let dump = args.get(5).map(String::as_str);
            tile(&args, |name, x, y| bake::cmd_bake_minimap(&cfg, name, x, y, dump))
        }
        Some("bake") if args.len() >= 4 => {
            tile(&args, |name, x, y| bake::cmd_bake_tile(&cfg, name, x, y))
        }
        Some("bake") => match arg(1) {
            Some(map) => bake::cmd_bake_map(&cfg, map),
            None => Err("usage: vale bake <MapName> [<x> <y>]".into()),
        },
        // The loading screen image a map shows while it loads. See
        // `cli::loading`.
        Some("loading") => loading::cmd_loading(&cfg),
        // Mail stationery, which is the only data the mailbox reads from a
        // file. See `cli::mail`.
        Some("mail") => mail::cmd_mail(&cfg, id(1)),
        // The volumes an instance portal is made of. See `cli::areatrigger`.
        Some("triggers") => areatrigger::cmd_triggers(&cfg, id(1)),
        // The flight map, checked the same way as `triggers`. See `cli::taxi`.
        Some("taxi") => taxi::cmd_taxi(&cfg, id(1)),
        // The boats and zeppelins, which `TaxiPath.dbc` also describes. Their
        // position is computed entirely from that table. See `cli::ships`.
        Some("ships") => ships::cmd_ships(&cfg, id(1)),
        Some("bindings") => bindings::cmd_bindings(&cfg, arg(1)),
        // The interface message table, checked against the two archive files
        // its columns point into. See `cli::messages`.
        Some("messages") => messages::cmd_messages(&cfg, arg(1)),
        // The combat log, the one body of text the client composes itself:
        // which sentence an event becomes, whose names go in it, and which of
        // the 94 windows it lands in. See `cli::combatlog`.
        Some("combatlog") => combatlog::cmd_combatlog(&cfg, arg(1)),
        Some("framexml") => framexml::cmd_framexml(&cfg, arg(1)),
        Some("addons") => addons::cmd_addons(&cfg, arg(1)),
        // The `/who` query, the one line of typed text the client parses
        // itself. It is the only rule in `assets` that no file backs. See
        // `cli::who`.
        // The query is every remaining argument joined with spaces, not
        // `arg(1)`: a `/who` line contains spaces and the shell splits on
        // them, so taking only the first word would parse `z-Durotar 5-10` as
        // a zone alone. Quoting still works: a shell that keeps
        // `z-"Elwynn Forest"` in one argument produces the same string.
        Some("who") => who::cmd_who(&cfg, (args.len() > 1).then(|| args[1..].join(" "))),
        Some("channels") => channels::cmd_channels(&cfg, id(1)),
        Some("particles") => particles::cmd_particles(&cfg, arg(1)),
        Some("model") => model::cmd_model(&cfg, arg(1)),
        Some("pick") => pick::cmd_pick(&cfg, arg(1)),
        Some("portrait") => portrait::cmd_portrait(&cfg, arg(1)),
        // The player's reputation standings. The panel's layout is entirely a
        // client-side rule. See `cli::reputation`.
        Some("reputation") => reputation::cmd_reputation(&cfg, arg(1)),
        // `skills`: the player's learned skills, a panel with the same
        // structure as the reputation panel. See `cli::skills`.
        Some("pet") => pet::cmd_pet(&cfg, arg(1)),
        Some("skills") => skills::cmd_skills(&cfg, arg(1)),
        // The player's talent choices. The talent panel depends on less
        // server data than any other panel in the game. See `cli::talent`.
        Some("talent") => talent::cmd_talent(&cfg, arg(1)),
        // What the player can craft: the two profession windows, whose rows,
        // order and colours are all client-side rules. See `cli::tradeskill`.
        Some("tradeskill") => tradeskill::cmd_tradeskill(&cfg, arg(1)),

        Some(other) => Err(format!(
            "unknown subcommand {other:?}; try: {}",
            SUBCOMMANDS.join(", ")
        )),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// The `<MapName> <x> <y>` argument shape, which five subcommands share.
///
/// When this parsing was written out five times, the usage line of a renamed
/// subcommand still printed the old name. The subcommand name is therefore
/// read from `args[0]` rather than written into the message.
fn tile(
    args: &[String],
    run: impl FnOnce(&str, u32, u32) -> Result<(), String>,
) -> Result<(), String> {
    let usage = || {
        format!(
            "usage: vale {} <MapName> <x> <y>",
            args.first().map(String::as_str).unwrap_or("tile")
        )
    };
    let (Some(name), Some(x), Some(y)) = (args.get(1), args.get(2), args.get(3)) else {
        return Err(usage());
    };
    match (x.parse::<u32>(), y.parse::<u32>()) {
        (Ok(x), Ok(y)) => run(name, x, y),
        _ => Err("tile coordinates must be numbers".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::SUBCOMMANDS;
    use std::collections::BTreeSet;

    /// Every `Some("name")` arm of `main`'s match appears in [`SUBCOMMANDS`],
    /// and nothing else does.
    ///
    /// The names are read from this file's source rather than restated,
    /// because the earlier restated list went out of date. `Some(other)` is
    /// the one arm with no literal, and the pattern skips it.
    #[test]
    fn every_subcommand_is_listed() {
        let source = include_str!("main.rs");
        let mut arms = BTreeSet::new();
        for line in source.lines() {
            let line = line.trim_start();
            let Some(rest) = line.strip_prefix("Some(\"") else {
                continue;
            };
            let Some(end) = rest.find('"') else { continue };
            arms.insert(&rest[..end]);
        }
        // `connect` is reached through `None | Some("connect")`, which the
        // pattern above does not match.
        arms.insert("connect");

        let listed: BTreeSet<&str> = SUBCOMMANDS.iter().copied().collect();
        assert_eq!(
            arms.difference(&listed).collect::<Vec<_>>(),
            Vec::<&&str>::new(),
            "subcommands the match accepts and SUBCOMMANDS does not name"
        );
        assert_eq!(
            listed.difference(&arms).collect::<Vec<_>>(),
            Vec::<&&str>::new(),
            "names in SUBCOMMANDS that the match does not accept"
        );
    }
}
