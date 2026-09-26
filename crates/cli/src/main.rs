//! `vale` — headless driver for the protocol and asset layers.
//!
//! ```text
//! main.rs        the command table, and nothing else
//! common.rs      …and whatever more than one command needs: logging on,
//!                opening the archives, parsing the display tables
//!
//! the server:
//! connect.rs     SRP6 logon -> world handshake -> character list
//! login.rs       enter the world once and dump what is nearby
//! live.rs        …and stay in it, with a command line of its own
//! wdb.rs         …and what either of those wrote to WDB\: every cache file,
//!                its records, each body re-parsed
//! dress.rs       every player in view, dressed the way the renderer would —
//!                the one command that crosses the server with the archives
//!
//! the ground, tile by tile:
//! archives.rs    open the MPQ chain and report what mounted
//! extract.rs     any file in it, written out or hexdumped
//! catalogue.rs   …and the listing of what may be placed in the world, folder by
//!                folder, with the listed paths that do not open
//! maps.rs        every map that has a WDT, with its tile count
//! map.rs         …and the tile grid of one of them
//! tile.rs        one ADT: chunks, heights, and what it places
//! mcnk.rs        …its raw MCNK headers and height grids
//! textures.rs    …its layers, alpha maps, and the atlas round trip
//! models.rs      …every M2 it places: decode, place, verify
//! wmos.rs        …every WMO it places, against its own MODF record
//! collision.rs   …the solid half of those buildings
//! water.rs       …and the liquid standing in it
//! foliage.rs     …and what grows on it: the ground effects, cell by cell
//! light.rs       the world's own daylight, per map, band by band
//! weather.rs     …and what falls out of that sky: four textures, three shaders,
//!                nine loops and two rules, each against the archives
//! sun.rs         …and where the sun stands, from a tile's baked shadows
//! bake.rs        …and the two pictures an editor has to write back — a tile's
//!                minimap and its MCSH — each scored against the shipped one
//! sky.rs         …and what is drawn *in* it: the stars, the arcs, the sprites
//!
//! what a thing looks like:
//! npc.rs         every display id -> a model and a skin      (vale npc)
//! objects.rs     …and the server's own table: which are solid
//! character.rs   a player's composed skin, layer by layer    (vale char)
//! item.rs        a garment's components, regions and geosets
//! attach.rs      …and the models that hang off a bone instead
//! anim.rs        every creature model: bones, sequences, poses
//! emote.rs       Emotes.dbc -> AnimationData -> what a model can play
//! emotetext.rs   …and what /dance says: three tables joined, every token checked
//! bank.rs        what a bank bag slot costs, and the bank's three numberings crossed
//! spell.rs       Spell -> SpellVisual -> SpellVisualKit -> a caster's pose
//! particles.rs   every emitter two DBC populations name
//! model.rs       one M2 as the renderer sees it
//! pick.rs        …and what the mouse can hit of it
//! portrait.rs    …and where to stand to take its picture
//! pet.rs         what a pet *is*: the happiness bands, the families and
//!                their diets — four DBCs the wire never mentions
//! reputation.rs  what the reputation panel would draw, list and all
//! skills.rs      …and the skills panel, headings and all
//! talent.rs      …and the talent trees, whose whole shape is a client rule
//! tradeskill.rs  …and the profession windows: the openers, the thresholds,
//!                the difficulty formula, every reagent reference checked
//! cursor.rs      the mouse pointers, and where each one's point is
//! sound.rs       the sound tables against each other and the archives
//!
//! what the interface is made of:
//! bindings.rs    Bindings.xml: 234 key bindings and the verbs they call
//! messages.rs    …and what it *says*: the message table, key by key
//! combatlog.rs   …and what it says about a *fight*: every key the log can
//!                produce, every window it can land in, and a line of each
//! framexml.rs    the whole interface: load graph, markup, and the API gap
//! addons.rs      …and the folder's own additions to it: every addon under
//!                Interface\AddOns\, its files checked, the load order, and
//!                every character's AddOns.txt
//! channels.rs    …and the rooms it talks in: the six zone channels, and
//!                which of them a place is in
//! who.rs         …and the one line of typed text the client parses itself:
//!                what a /who means, against the three tables it joins
//! glue.rs        …and the screens before it: art, models, cameras
//! loading.rs     …and the picture between them: the field-38 join
//! mail.rs        …and the paper a letter is written on: five rows, one rule
//! spellbook.rs   what a spell is to a button                 (vale spellbook)
//! spelltemplate.rs  …and what a spell is to the *server*: the 151 columns of
//!                vmangos' spell_template, which Spell.dbc field feeds each,
//!                and the statements an edit becomes (vale spelltemplate)
//! itemtemplate.rs  …and what an *item* is to it: item_template's 129 columns,
//!                which have no client file behind them at all, and the
//!                appearance chain the archives do answer (vale itemtemplate)
//! questtemplate.rs …and what a *quest* is to it: quest_template's 131 columns
//!                and the four relation tables that say who gives and takes
//!                one (vale questtemplate)
//! spawns.rs      …and what an NPC is to it: creature_template and the
//!                creature rows that stand it in the world, each joined out to
//!                a model the archives either hold or do not (vale spawns)
//! waypoints.rs   …and the path one walks: creature_movement, every node
//!                against the ground the archives draw  (vale waypoints)
//! navmesh.rs     …and the surface the server walks it on: every mmaps tile
//!                read, by surface class               (vale navmesh)
//! book.rs        …and what the panel is: the tabs, in the client's own order
//! charcreate.rs  what a character may be made of             (vale create)
//! minimap.rs     the little map's index, and which way up a picture is
//! worldmap.rs    …and the big one: the zones, and what each highlights
//!                                                            (vale zones)
//! areatrigger.rs the volumes a dungeon portal is made of     (vale triggers)
//! taxi.rs        the flight map: three tables, the projection, the routes
//! ships.rs       …and the boats and zeppelins that share its table, whose
//!                position no packet ever states                (vale ships)
//! dbc.rs         every table's shape and round trip, and one  (vale dbc)
//!                table field by field
//! ```
//!
//! **One file per command**, which is what makes this list navigable: the four
//! names that differ from their command are the ones this repo shares with
//! `assets`, `protocol`, `game` and `lua`, and that parallel is worth more than
//! the exact match.
//!
//! Every subcommand is deliberately read-only; nothing here writes game data.
//!
//! **This is the project's verification harness, not a demo.** Almost every
//! fact the renderer relies on was established by one of these commands
//! measuring the game's files against something that did not come from this client — a
//! model's own declared bounding box, the game's own `MODF` placements, a
//! neighbouring chunk's alpha map, the size of the file that fills a region.
//! `dress` is the only one that crosses the server's answers with the archives.
//!
//! One module per subject; this file is the command table and nothing else.

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
/// The list used to be written out a second time inside the "unknown
/// subcommand" message. It fell eight commands behind the match — `extract`,
/// `glue`, `sky`, `zones`, `triggers`, `messages`, `combatlog` and
/// `tradeskill` all worked and none of them was named, so the only way to
/// discover them was to read this file. `every_subcommand_is_listed` compares
/// the two.
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
        // Two forms, on `foliage`'s own terms: one tile's layers in full, or —
        // with no tile — the `MCLY` flag census over a whole map, which is what
        // pinned the texture-animation bits. See `cli::textures`.
        Some("textures") if args.len() >= 4 => {
            tile(&args, |name, x, y| textures::cmd_textures(&cfg, name, x, y))
        }
        Some("textures") => textures::cmd_layer_flags(&cfg, arg(1)),
        Some("models") => tile(&args, |name, x, y| models::cmd_models(&cfg, name, x, y)),
        Some("wmos") => tile(&args, |name, x, y| wmos::cmd_wmos(&cfg, name, x, y)),
        // …and two more arguments turn the tile sweep into a question about
        // one point: `vale collision Azeroth 32 48 -8869.2 -163.2` says what
        // is under a creature that is drawn standing in the air.
        Some("collision") => match (args.get(4), args.get(5)) {
            (Some(px), Some(py)) => match (px.parse::<f32>(), py.parse::<f32>()) {
                // The z is optional and changes the answer: with one, each hull
                // is asked for the surface under *that* height — which for a
                // building is the floor the creature is on rather than its roof.
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
        // Two forms: the whole table against the archives, or one tile's own
        // ground cell by cell — see `cli::foliage`.
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
            // Bare, it is the census and the round trip over every table.
            None => dbc::cmd_dbc_all(&cfg),
        },
        Some("char") => character::cmd_char(&cfg, arg(1)),
        Some("item") => item::cmd_item(&cfg, id(1)),
        Some("attach") => attach::cmd_attach(&cfg, id(1)),
        Some("anim") => anim::cmd_anim(&cfg, arg(1)),
        Some("emote") => emote::cmd_emote(&cfg),
        Some("emotetext") => emotetext::cmd_emotetext(&cfg, arg(1)),
        Some("bank") => bank::cmd_bank(&cfg),
        Some("spelltemplate") => spelltemplate::cmd_spelltemplate(&cfg, arg(1)),
        // Four forms: no argument is the schema, a number is one item, and
        // `display` and `db` are the two censuses — see `cli::itemtemplate`.
        Some("itemtemplate") => itemtemplate::cmd_itemtemplate(&cfg, arg(1)),
        // Three forms: the schema, one quest traced, and `db` — see
        // `cli::questtemplate`.
        Some("questtemplate") => questtemplate::cmd_questtemplate(&cfg, arg(1)),
        // Three forms: no argument is every map, a name is one map, a number is
        // one creature entry traced — see `cli::spawns`.
        Some("spawns") => spawns::cmd_spawns(&cfg, arg(1)),
        // The same three forms, over the movement tables — see `cli::waypoints`.
        Some("waypoints") => waypoints::cmd_waypoints(&cfg, arg(1)),
        // Every tile of the server's navmesh, or one — see `cli::navmesh`.
        Some("navmesh") => navmesh::cmd_navmesh(&cfg, &args),
        Some("spell") => spell::cmd_spell(&cfg, id(1)),
        Some("sound") => sound::cmd_sound(&cfg, arg(1)),
        Some("weather") => weather::cmd_weather(&cfg, arg(1)),
        Some("spellbook") => spellbook::cmd_spellbook(&cfg, id(1)),
        Some("book") => book::cmd_book(&cfg, arg(1)),
        Some("create") => charcreate::cmd_create(&cfg, arg(1)),
        // Three forms: no argument is the whole table, a number traces one area,
        // and a map plus a tile is the *building* check — see `cli::worldmap`.
        Some("zones") if args.len() >= 4 => {
            tile(&args, |name, x, y| worldmap::cmd_zone_buildings(&cfg, name, x, y))
        }
        Some("zones") => worldmap::cmd_zones(&cfg, id(1)),
        // Three forms, on the same shape `zones` has: the whole index, one map,
        // or one tile — the last of which is the orientation check. See
        // `cli::minimap`.
        Some("minimap") if args.len() >= 4 => {
            tile(&args, |name, x, y| minimap::cmd_minimap(&cfg, Some(name), Some((x, y))))
        }
        Some("minimap") => minimap::cmd_minimap(&cfg, arg(1), None),
        // …and the two pictures an *editor* has to write, scored against the
        // ones the game shipped — see `cli::bake`, which says why these two are
        // the only parts of an edit that need an instrument.
        // …and the picture alone, over a sweep of looks, because the shadow
        // bake is two minutes and none of the minimap's settings is derivable
        // from a file — only measurable against the shipped one.
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
        // …and the *other* picture a map has, which is the one shown while it
        // is not there yet — see `cli::loading`.
        Some("loading") => loading::cmd_loading(&cfg),
        // …and the paper a letter is written on, which is the whole of what the
        // mailbox reads out of a file — see `cli::mail`.
        Some("mail") => mail::cmd_mail(&cfg, id(1)),
        // The volumes an instance portal is made of — see `cli::areatrigger`.
        Some("triggers") => areatrigger::cmd_triggers(&cfg, id(1)),
        // …and the flight map, which is the same kind of check — see `cli::taxi`.
        Some("taxi") => taxi::cmd_taxi(&cfg, id(1)),
        // …and the *other* thing `TaxiPath.dbc` describes: the boats and the
        // zeppelins, whose whole position is this arithmetic — see `cli::ships`.
        Some("ships") => ships::cmd_ships(&cfg, id(1)),
        Some("bindings") => bindings::cmd_bindings(&cfg, arg(1)),
        // …and what the client *says*: the message table, against the two
        // archive files its columns point into — see `cli::messages`.
        Some("messages") => messages::cmd_messages(&cfg, arg(1)),
        // …and the one body of text the client writes for *itself*: which
        // sentence a blow is, whose names go in it, and which of the 94
        // windows it lands in — see `cli::combatlog`.
        Some("combatlog") => combatlog::cmd_combatlog(&cfg, arg(1)),
        Some("framexml") => framexml::cmd_framexml(&cfg, arg(1)),
        Some("addons") => addons::cmd_addons(&cfg, arg(1)),
        // …and the one line of typed text the client parses *itself*, which is
        // the only rule in `assets` with no file behind it at all — see
        // `cli::who`.
        // **Every remaining argument, joined** rather than `arg(1)`: a `/who`
        // line has spaces in it and a shell splits on them, so taking only the
        // first word would parse `z-Durotar 5-10` as a zone and nothing else.
        // Quoting still works — a shell that keeps `z-"Elwynn Forest"` in one
        // argument produces the same string this does.
        Some("who") => who::cmd_who(&cfg, (args.len() > 1).then(|| args[1..].join(" "))),
        Some("channels") => channels::cmd_channels(&cfg, id(1)),
        Some("particles") => particles::cmd_particles(&cfg, arg(1)),
        Some("model") => model::cmd_model(&cfg, arg(1)),
        Some("pick") => pick::cmd_pick(&cfg, arg(1)),
        Some("portrait") => portrait::cmd_portrait(&cfg, arg(1)),
        // …and the *player's* own standing, which is the one panel whose whole
        // shape is a client-side rule — see `cli::reputation`.
        Some("reputation") => reputation::cmd_reputation(&cfg, arg(1)),
        // …and what it has *learned*, which is the same shape one panel over —
        // see `cli::skills`.
        Some("pet") => pet::cmd_pet(&cfg, arg(1)),
        Some("skills") => skills::cmd_skills(&cfg, arg(1)),
        // …and what it *chose*, which is the panel with the least wire under it
        // of any in the game — see `cli::talent`.
        Some("talent") => talent::cmd_talent(&cfg, arg(1)),
        // …and what it can *make*: the two profession windows, whose rows,
        // order and colours are all client-side rules — see `cli::tradeskill`.
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
/// It was written out five times, and the usage line it prints was therefore
/// wrong for whichever one had last been renamed — so the subcommand's own name
/// is taken from `args[0]` rather than repeated in the message.
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
    /// The names are read out of this file's own source rather than restated,
    /// because a restated list is the thing that went stale. `Some(other)` is
    /// the one arm with no literal and is skipped by the pattern.
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
