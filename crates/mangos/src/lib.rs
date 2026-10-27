//! What a row of the server's world database means.
//!
//! ```text
//! spell.rs     `spell_template`: its 151 columns, which `Spell.dbc` field feeds
//!              each of them, and the statements that make an edit live
//! taxi.rs      `taxi_nodes`, the server's copy of `TaxiNodes.dbc`: an edited
//!              flight node written as a row at build 5875, on the same terms
//!              as a spell
//! skills.rs    `skill_line_ability`, the server's copy of
//!              `SkillLineAbility.dbc`: an edited ability written as a row at
//!              build 5875, the only build the loader reads
//! trigger.rs   `areatrigger_template`, the server's copy of `AreaTrigger.dbc`
//!              written as a row at build 5875 on `taxi_nodes`' terms, and the
//!              three tables that say what a trigger does: a teleport, an inn,
//!              a quest's exploration objective. The three are live on reloads
//! graveyard.rs `game_graveyard_zone`, which safe place serves which zone and
//!              for which side, live on a reload, and `world_safe_locs_facing`,
//!              read at startup
//! map.rs       `map_template`, the server's maps: a new map's row, written at
//!              patch 0, since the server does not read `Map.dbc`
//! condition.rs `conditions`: the server's reusable yes-or-no tests, their
//!              63 types and what each value means, live on a reload
//! gossip.rs    what a creature says and offers when spoken to: `gossip_menu`,
//!              `gossip_menu_option` and `npc_text`, live on reloads
//! area.rs      `area_template`, the server's copy of `AreaTable.dbc`: an
//!              edited or new area written as a row keyed by its entry alone,
//!              nine columns of the file's 25, read at startup
//! faction.rs   `faction` and `faction_template`, the server's copies of
//!              `Faction.dbc` and `FactionTemplate.dbc`: an edited row of
//!              either written at build 5875 on `skill_line_ability`'s terms,
//!              both read at startup
//! addon.rs     `creature_addon`, how one spawn stands: its display, mount,
//!              equipment set, stand and sheath state, emote and auras over
//!              the template's; and `creature_equip_template`, the weapon
//!              rows a set holds, one rolled per spawn. Both read at startup
//! formation.rs `creature_groups`, which spawns walk and fight together and
//!              where each member stands from its leader, live on a reload;
//!              and `creature_linking` and `creature_linking_template`, what
//!              a slave does when its master acts, read at startup
//! pool.rs      the six pool tables: a pool, its creature, game object and
//!              pool members with their chances, and its members by entry.
//!              Read at startup
//! game_event.rs `game_event`, when an event is on, and the four tables of
//!              what it changes: the spawns it adds or removes, the creatures
//!              it changes, the quests it offers. Read at startup
//! creature.rs  two tables with no client file behind them: `creature_template`,
//!              what an NPC is, and `creature`, where one stands
//! gameobject.rs the matching pair for game objects: `gameobject_template`,
//!              whose 24 `data` columns are named by the row's type, and
//!              `gameobject`, whose facing and rotation quaternion must agree
//! item.rs      `item_template`: what an item is, what it does and what it is
//!              worth. 129 columns, and the one subject here a `.reload` makes
//!              live
//! quest.rs     `quest_template`, and the four relation tables that say who
//!              hands a quest out and who takes it. Rows are created, edited
//!              and removed, and each of those is live on a reload
//! loot.rs      the nine `*_loot_template` tables: one schema under nine names,
//!              keyed to a creature, an object, an item, a zone or a mail
//!              template. A row is five columns, and every table is live on its
//!              own reload, removals included
//! eventai.rs   `creature_ai_events`: what a creature running EventAI reacts
//!              to, with the 37 triggers' parameters named per type, and which
//!              scripts it runs. Live on a reload
//! scripts.rs   the eleven `*_scripts` tables: one schema under eleven names,
//!              the 94 commands with the columns each reads named per command,
//!              the 30 targets, and a script written whole under its id, since
//!              a row has no key of its own
//! creaturespells.rs `creature_spells`: the eight spells a creature casts in
//!              combat, one row of ninety columns per list. Live on a reload
//! broadcast.rs `broadcast_text`: the lines a Talk step says, yells or emotes.
//!              Read at start, so a change is live after a restart
//! vendor.rs    `npc_vendor` and `npc_vendor_template`: what a creature sells,
//!              from its own list and from the one its `vendor_id` names. Live
//!              on a reload, removals included
//! trainer.rs   `npc_trainer` and `npc_trainer_template`: what a creature
//!              teaches, as teaching spells, on the same terms
//! lists.rs     the server tables a template column names by id (gossip
//!              menus, equipment, spell lists, vendor, trainer and loot lists),
//!              and how a picker lists each by what is in it and who uses it
//! schema.rs    the vocabulary the row subjects (creature, gameobject, item,
//!              quest, loot, event, script, spell list, vendor, trainer) are
//!              described in: what a column is, how it is read, and which part
//!              of a form it is drawn in
//! row.rs       what an edit to a row is when there is no file to diff: the key,
//!              the assignment, the undo, the file a project keeps them in, and
//!              the table a row is in with the rules a store's row is planned by
//! path.rs      a waypoint path, the one subject whose edit is a set of rows
//!              rather than a set of columns. The server renumbers a path's
//!              points at every start, so a path is written whole
//! conn.rs      the only module that connects to the database: where the
//!              world database is, and running statements on it
//! sql.rs       how a value is written into one of those statements
//! migration.rs the form a set of those statements is handed to a server in:
//!              vmangos' own `add_migration` file, stamped and applied once;
//!              and the record a project keeps of what its last migration
//!              contained, so the next one carries the change since
//! datadir/     the part of the server's world that is files rather than rows:
//!              `DataDir`'s `maps`, `vmaps` and `mmaps`, one per tile, built
//!              from the client's archives by the four vmangos tools; and
//!              running those tools over the tiles a project changed, and an
//!              `mmaps` tile read as triangles
//! ```
//!
//! ## How this crate relates to `vale-assets`
//!
//! `vale-assets` is what a client file means and this crate is what a
//! server row means, on the same terms: no renderer, no window, no session, and
//! unit-tested against the real data. The server's world is a MySQL database;
//! the client's is a folder of archives; this crate is the half of the editor's
//! server work that needs neither a connection nor an app to be checked.
//!
//! This crate may not decide what a client file means. A `spell_template` row is
//! `Spell.dbc`'s row with the server's own columns beside it, and which field of
//! that DBC is which is `vale-assets`' fact — read through `vale-edit`'s
//! `DbcFile`, never restated here. This crate adds the other half of the join:
//! which column each of those fields lands in, which columns the client's file
//! has no answer for, and what a statement changing one looks like.
//!
//! ## Only `conn.rs` connects to the database
//!
//! [`spell`], [`taxi`] and [`sql`] have no connection in them. Every function
//! produces SQL text, which is what the project's `sql\` folder holds and what
//! a vmangos migration is made of, so those modules are checked by reading
//! their output. [`conn`] is the only module that finds the database and runs
//! statements on it, so the rest of the crate can be tested with nothing
//! installed.
//!
//! ## Nothing in `crates/client` may name this crate
//!
//! The rule is the same as the editor's: the renderer is a 1.12 client and
//! knows nothing about the server's database. The editor and the CLI are the
//! two hosts that use this crate.

/// The client build as a literal, for a statement written as a `&'static str`
/// with [`concat!`]. Everything else names [`CLIENT_BUILD`].
macro_rules! client_build {
    () => {
        5875
    };
}

/// The client build a vmangos for 1.12.1 serves: `SUPPORTED_CLIENT_BUILD`.
///
/// It names the folder under `DataDir` the server reads its DBC files from,
/// and it is the build a row copied from a client file is written at: each
/// loader of such a table takes the highest row at or below this build, so a
/// row written here wins over every shipped one, and one written above it is
/// never read.
pub(crate) const CLIENT_BUILD: u32 = client_build!();

/// How far from a map's centre a point may be on either axis, in yards: half
/// of 64 grids of 533⅓ yards, less half a yard. vmangos' `IsValidMapCoord`
/// (`GridDefines.h`) refuses a coordinate outside it.
pub(crate) const MAP_REACH: f32 = 64.0 * 533.333_3 / 2.0 - 0.5;

pub mod addon;
pub mod area;
pub mod broadcast;
pub mod condition;
pub mod conn;
pub mod creature;
pub mod creaturespells;
pub mod datadir;
pub mod eventai;
pub mod faction;
pub mod formation;
pub mod game_event;
pub mod gameobject;
pub mod gossip;
pub mod graveyard;
pub mod item;
pub mod lists;
pub mod loot;
pub mod map;
pub mod migration;
pub mod path;
pub mod pool;
pub mod quest;
pub mod row;
pub mod schema;
pub mod scripts;
pub mod skills;
pub mod spell;
pub mod sql;
pub mod taxi;
pub mod trainer;
pub mod trigger;
pub mod vendor;
