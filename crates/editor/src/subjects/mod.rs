//! The server-row subjects that have no place in the world: each is a
//! workspace or a window rather than a pointer tool.
//!
//! ```text
//! items.rs       `item_template`, which has no place in the world: what an
//!                item is, what it does and what it looks like. A workspace
//!                like the client-table one in `crate::tables`, over the
//!                server's rows rather than a DBC, and the one server subject a
//!                `.reload` makes live
//! quests.rs      `quest_template` with the four relation tables beside it:
//!                what a quest asks for, says and gives, and who hands it out.
//!                A workspace of the same kind, and the list the creature
//!                tool's Quests window reads from the other direction
//! templates.rs   the Creatures and Game Objects workspaces: every
//!                `creature_template` or `gameobject_template` row as a list,
//!                read once and searched in memory, edited with the spawn
//!                tool's own form and state, written once over the spawn
//!                subject
//! listing.rs     a server table read whole and searched in memory: the list
//!                the two workspaces above draw, written once over any row type
//! loot.rs        the nine loot tables. No workspace and no rail entry: a set
//!                is reached from the creature, the object or the item whose
//!                column names it, in a window that follows the selection.
//!                What a set holds, read one set at a time, and what an edit
//!                to a row of it is
//! services.rs    what a creature sells and teaches: npc_vendor, npc_trainer
//!                and their template tables, in two windows that follow the
//!                selection. Both lists of a kind read together, and what an
//!                edit to a row is, refused where the server would skip it
//! addons.rs      how one spawn stands and what a creature holds:
//!                `creature_addon`, drawn as a section of the creature
//!                inspector, and `creature_equip_template`, the set a
//!                creature rolls a row of, in a window that follows the
//!                selection; what an edit to either is
//! formations.rs  which creature spawns walk and fight together and which
//!                answer another: `creature_groups` and the two linking
//!                tables, read whole, drawn as sections of the creature
//!                inspector and as lines in the viewport
//! pools.rs       which spawns the server keeps a few of at a time: the six
//!                pool tables, read whole, drawn as a section of both spawn
//!                inspectors and a window on one pool
//! game_events.rs what the world holds while an event is on: `game_event`
//!                and the four tables that name one, read whole, drawn as a
//!                section of both spawn inspectors and a window on one event
//! located.rs     where a spawn named by guid stands and what it is, read many
//!                at a time for the lists that name spawns on any map
//! keyed.rs       a keyed row with the project's edits over the database's,
//!                for the three subjects above, which read a table whole and
//!                draw a few rows of it
//! displays.rs    the display id picker's state: a creature's or a game
//!                object's model chosen from CreatureDisplayInfo or
//!                GameObjectDisplayInfo by picture, searched by model path.
//!                A dialog over the creature and game object forms
//! behaviour.rs   what a creature does: its `creature_ai_events` rows, the
//!                `creature_spells` list its template names, and the scripts
//!                either runs. No workspace and no rail entry: two windows
//!                opened from a selected creature and a third from either.
//!                Events and lists are rows of the store; a script is
//!                replaced whole, since its rows have no key
//! gossip.rs      what a creature says when spoken to: `gossip_menu`, the
//!                `gossip_menu_option` choices under it and the `npc_text`
//!                each menu row shows, read for the menus one creature leads
//!                to, and what an edit to each is
//! conditions.rs  the `conditions` table, read whole, and what an edit to a
//!                condition is. Any form whose column names a condition opens
//!                the window on it
//! windows.rs     the creature or game object template the windows above
//!                follow: the selected spawn's under a spawn tool, the open
//!                row's under its workspace
//! ```

pub(crate) mod addons;
pub(crate) mod behaviour;
pub(crate) mod conditions;
pub(crate) mod displays;
pub(crate) mod formations;
pub(crate) mod game_events;
pub(crate) mod gossip;
pub(crate) mod items;
pub(crate) mod keyed;
pub(crate) mod listing;
pub(crate) mod located;
pub(crate) mod loot;
pub(crate) mod pools;
pub(crate) mod quests;
pub(crate) mod services;
pub(crate) mod templates;
pub(crate) mod windows;
