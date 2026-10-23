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

pub mod behaviour;
pub mod conditions;
pub mod displays;
pub mod gossip;
pub mod items;
pub mod listing;
pub mod loot;
pub mod quests;
pub mod services;
pub mod templates;
pub mod windows;
