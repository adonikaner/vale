//! The flags that make the gestures of the creature and game object tools: a
//! spawn chosen, placed, removed or given a path, the picker, and the template
//! window with its display picker.

use super::Arity::*;
use super::{number, Flag};
use crate::tools::Tool;

/// The spawn tools' scripted gestures.
#[derive(Debug, Clone, Default)]
pub struct Spawns {
    /// `--spawn <guid>`: open the creature tool on that spawn, and move the
    /// camera to it.
    ///
    /// Same reason as `--row`: a spawn is chosen by clicking a creature in the
    /// viewport, and a scripted run has no pointer, so without this a scripted
    /// run cannot photograph the creature form, which is the tool's main
    /// content. It implies `--tool creatures`.
    ///
    /// The guid is the `creature` row's own, which `vale spawns <entry>`
    /// prints. A guid that is not on the open map selects nothing and prints
    /// that.
    pub spawn: Option<u64>,
    /// `--waypoints`: open the chosen spawn's path with it.
    ///
    /// Same reason as `--spawn`, one step further in: a path is opened by a
    /// button on a form that is itself reached by a click, so without this a
    /// scripted run cannot photograph or drive the waypoint window. It implies
    /// `--spawn`'s tool, and does nothing without a `--spawn` to open.
    pub waypoints: bool,
    /// `--waypoint-add <n>`: give the chosen spawn a ring of `n` points.
    ///
    /// Placing a point is a click on the ground, which a scripted run cannot
    /// make, so without this nothing that follows a placement can be checked
    /// with nobody at the keyboard: the store, the undo, the SQL, the apply
    /// and the revert. A ring around the creature is also the shape a person
    /// starts a patrol from and then drags into place.
    ///
    /// It implies `--waypoints`, and replaces whatever path the creature has.
    pub waypoint_add: Option<usize>,
    /// `--spawn-add <entry>`: place a spawn of that creature under the camera.
    ///
    /// `--spawn-delete <guid>`: mark that spawn for removal. These are the two
    /// gestures of the creature tool a scripted run cannot make: placing is a
    /// click on the ground with the placer armed, and removing is a press on a
    /// button under a selection. Without these flags nothing that follows
    /// either can be checked with nobody at the keyboard. Both imply
    /// `--tool creatures`.
    ///
    /// `--spawn-add` goes through the picker rather than bypassing it, so one
    /// run checks the search as well as the placement.
    pub spawn_add: Option<u32>,
    pub spawn_delete: Option<u64>,
    /// `--pick <entry>`: open the creature tool's Place half on that creature,
    /// and do nothing further.
    ///
    /// `--spawn-add` without the placement. It exists to photograph the
    /// picker, the preview pane and what a chosen creature looks like, which
    /// a scripted run cannot otherwise reach: the mode is a press on a
    /// segmented control, the search is typing, and the choice is a click on a
    /// row.
    ///
    /// It cannot show the translucent preview, and no flag can: the model on
    /// the cursor stands where the pointer meets the ground, and a scripted run
    /// has no pointer. The placement tools' own preview has the same limitation.
    pub pick: Option<u32>,
    /// `--template`: open the creature template window on `--spawn`'s creature.
    ///
    /// Same reason as `--lab` and `--browse`, for the creature tool. The window
    /// opens from a button two clicks in, so without this a scripted run cannot
    /// photograph the form that edits a creature.
    pub template: bool,
    /// `--template-new`: make a new creature template, choose it in the
    /// creature tool's picker and open the template window on it.
    ///
    /// The creature counterpart of `--item-new`: creating a template is a
    /// press on the picker, so without this a script cannot check a new
    /// creature. It implies `--tool creatures`.
    pub template_new: bool,
    /// `--template-entry <n>`: renumber the template the window is about,
    /// which is `--template-new`'s creature or `--spawn`'s.
    ///
    /// The one action on the template window that is not a column edit: it
    /// re-keys the project's claim on the row. See
    /// `crate::tools::spawns::creatures::Creatures::rekey_template`.
    pub template_entry: Option<u32>,
    /// `--pick-display [id]`: open the display picker on the template
    /// window's first display column (`display_id1` under the creature tool,
    /// `displayId` under the game object tool), and with an id choose it.
    ///
    /// `--item-display` for the other two templates, and stored the same way:
    /// the picker is a grid of pictures two clicks in, the bare flag opens it
    /// for a screenshot, and an id also writes the column, which checks the
    /// whole path with nobody at the keyboard. Two fields because 0 is a
    /// display id a row can hold. It does not choose a tool: `--spawn`,
    /// `--template-new`, `--object` or `--object-new` names the template.
    pub pick_display: bool,
    pub display_id: Option<u32>,
    /// `--object <guid>`: open the game-object tool on that spawn, and move the
    /// camera to it. It is `--spawn` for the `gameobject` table. `--template`
    /// and `--quests` open the same two windows on it. It implies
    /// `--tool objects`.
    pub object: Option<u64>,
    /// `--object-add <entry>`, `--object-delete <guid>` and `--object-pick
    /// <entry>`: `--spawn-add`, `--spawn-delete` and `--pick` for a game
    /// object, for the same reasons: placing is a click on the ground,
    /// removing is a press under a selection, and the picker is a mode, a
    /// search and a choice. Each implies `--tool objects`.
    pub object_add: Option<u32>,
    pub object_delete: Option<u64>,
    pub object_pick: Option<u32>,
    /// `--object-new`: make a new game object template, choose it in the
    /// picker and open the template window on it. `--object-entry <n>`:
    /// renumber the template the window is about, which is `--object-new`'s
    /// object or `--object`'s. They are `--template-new` and
    /// `--template-entry` for the `gameobject_template` table. Each implies
    /// `--tool objects`.
    pub object_new: bool,
    pub object_entry: Option<u32>,
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--spawn", One, |a, v| {
        a.spawns.spawn = number(v);
        // It names a creature spawn, so it selects the creature
        // tool whatever else was asked for. `--row` and `--table`
        // follow the same rule for the table browser.
        a.view.tool = Some(Tool::Creatures);
    }),
    Flag::new("--waypoint-add", One, |a, v| {
        a.spawns.waypoint_add = number(v);
        a.spawns.waypoints = true;
        a.view.tool = Some(Tool::Creatures);
    }),
    Flag::new("--spawn-add", One, |a, v| {
        a.spawns.spawn_add = number(v);
        a.view.tool = Some(Tool::Creatures);
    }),
    Flag::new("--pick", One, |a, v| {
        a.spawns.pick = number(v);
        a.view.tool = Some(Tool::Creatures);
    }),
    Flag::new("--spawn-delete", One, |a, v| {
        a.spawns.spawn_delete = number(v);
        a.view.tool = Some(Tool::Creatures);
    }),
    Flag::new("--waypoints", Switch, |a, _| {
        a.spawns.waypoints = true;
        a.view.tool = Some(Tool::Creatures);
    }),
    Flag::new("--template", Switch, |a, _| a.spawns.template = true),
    Flag::new("--template-new", Switch, |a, _| {
        a.spawns.template_new = true;
        a.view.tool = Some(Tool::Creatures);
    }),
    Flag::new("--template-entry", One, |a, v| {
        a.spawns.template_entry = number(v);
        a.view.tool = Some(Tool::Creatures);
    }),
    // The id is optional, so a following flag is not eaten and
    // the bare flag opens the picker without choosing.
    Flag::new("--pick-display", Optional, |a, v| {
        a.spawns.pick_display = true;
        if !v.is_empty() {
            a.spawns.display_id = number(v);
        }
    }),
    Flag::new("--object", One, |a, v| {
        a.spawns.object = number(v);
        a.view.tool = Some(Tool::GameObjects);
    }),
    Flag::new("--object-add", One, |a, v| {
        a.spawns.object_add = number(v);
        a.view.tool = Some(Tool::GameObjects);
    }),
    Flag::new("--object-delete", One, |a, v| {
        a.spawns.object_delete = number(v);
        a.view.tool = Some(Tool::GameObjects);
    }),
    Flag::new("--object-pick", One, |a, v| {
        a.spawns.object_pick = number(v);
        a.view.tool = Some(Tool::GameObjects);
    }),
    Flag::new("--object-new", Switch, |a, _| {
        a.spawns.object_new = true;
        a.view.tool = Some(Tool::GameObjects);
    }),
    Flag::new("--object-entry", One, |a, v| {
        a.spawns.object_entry = number(v);
        a.view.tool = Some(Tool::GameObjects);
    }),
];
