//! What the windows that follow a selection are about: [`Focus`], the
//! creature or game object template whose quests, loot, behaviour, vendor,
//! trainer and gossip windows are open. The windows are drawn by
//! `crate::ui::subjects::windows`; each window's subject is built from a
//! `Focus` by its own module (`crate::subjects::loot::Window::for_creature`,
//! `crate::subjects::behaviour::About::from_focus` and the others).

use crate::tools::spawns::creatures::Creatures;
use crate::tools::spawns::gameobjects::{GameObjects, Known};
use crate::tools::Tool;
use vale_mangos::row::{Edits, Key};

/// The creature or game object template the windows that follow a selection
/// (quests, loot, events, spells, gossip, vendor, trainer) are about.
///
/// Under the creature and game object tools it is the selected spawn's
/// template; under the Creatures and Game Objects workspaces it is the open
/// row. The windows' subjects are built from this, so each window works the
/// same from either.
#[derive(Clone)]
pub(crate) struct Focus {
    pub(crate) entry: u32,
    pub(crate) label: String,
    pub(crate) template_key: Key,
    /// `npc_flags`, for the creature's quest and service windows.
    pub(crate) npc_flags: u32,
    /// Where the spawn stands, for the gossip window. `None` for a template
    /// opened in the workspace.
    pub(crate) at: Option<[f32; 2]>,
    /// The game object's brief row, for the quest window's type check and the
    /// loot window's loot column. `None` for a creature, and for a spawn
    /// whose template row is missing.
    pub(crate) object: Option<Known>,
}

impl Focus {
    /// The creature the windows are about under `tool`, with this project's
    /// edits over it, or `None` under any other tool.
    pub(crate) fn creature(tool: Tool, creatures: &Creatures, edits: &Edits) -> Option<Focus> {
        match tool {
            Tool::Creatures => creatures.chosen_edited(Some(edits)).map(|spawn| Focus {
                entry: spawn.entry,
                label: spawn.label(),
                template_key: spawn.template_key(),
                npc_flags: spawn.npc_flags,
                at: Some([spawn.at.x, spawn.at.y]),
                object: None,
            }),
            Tool::CreatureData => creatures.listed.open_row(edits).map(|known| Focus {
                entry: known.entry,
                label: known.label(),
                template_key: known.key(),
                npc_flags: known.npc_flags,
                at: None,
                object: None,
            }),
            _ => None,
        }
    }

    /// The game object the windows are about under `tool`, or `None` under
    /// any other tool.
    pub(crate) fn object(tool: Tool, objects: &GameObjects, edits: &Edits) -> Option<Focus> {
        match tool {
            Tool::GameObjects => objects.chosen_edited(Some(edits)).map(|spawn| Focus {
                entry: spawn.entry,
                label: spawn.label(),
                template_key: spawn.template_key(),
                npc_flags: 0,
                at: Some([spawn.at.x, spawn.at.y]),
                object: spawn.known.clone(),
            }),
            Tool::GameObjectData => objects.listed.open_row(edits).map(|known| Focus {
                entry: known.entry,
                label: known.label(),
                template_key: known.key(),
                npc_flags: 0,
                at: None,
                object: Some(known),
            }),
            _ => None,
        }
    }
}
