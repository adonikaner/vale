//! What a row of the server's world database means.
//!
//! ```text
//! spell.rs     `spell_template`: its 151 columns, which `Spell.dbc` field feeds
//!              each of them, and the statements that make an edit live
//! taxi.rs      `taxi_nodes`, the server's copy of `TaxiNodes.dbc`: an edited
//!              flight node written as a row at build 5875, on the same terms
//!              as a spell
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
//! schema.rs    the vocabulary the row subjects (creature, gameobject, item,
//!              quest, loot, event, script, spell list) are described in:
//!              what a column is, how it is read, and which part of a form it
//!              is drawn in
//! row.rs       what an edit to a row is when there is no file to diff: the key,
//!              the assignment, the undo, and the file a project keeps them in
//! path.rs      a waypoint path, the one subject whose edit is a set of rows
//!              rather than a set of columns. The server renumbers a path's
//!              points at every start, so a path is written whole
//! conn.rs      the only module that connects to anything: where the world
//!              database is, and running statements on it
//! sql.rs       how a value is written into one of those statements
//! migration.rs the form a set of those statements is handed to a server in:
//!              vmangos' own `add_migration` file, stamped and applied once;
//!              and the record a project keeps of what its last migration
//!              contained, so the next one carries the change since
//! datadir.rs   the part of the server's world that is files rather than rows:
//!              `DataDir`'s `maps`, `vmaps` and `mmaps`, one per tile, built
//!              from the client's archives by the four vmangos tools; and
//!              running those tools over the tiles a project changed
//! navmesh.rs   an `mmaps` tile read as triangles in the world's axes, grouped
//!              by what their flags mean to the server's path queries
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

pub mod conn;
pub mod creature;
pub mod creaturespells;
pub mod datadir;
pub mod eventai;
pub mod gameobject;
pub mod item;
pub mod loot;
pub mod migration;
pub mod navmesh;
pub mod path;
pub mod quest;
pub mod row;
pub mod schema;
pub mod scripts;
pub mod spell;
pub mod sql;
pub mod taxi;
