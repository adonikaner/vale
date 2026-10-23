//! The flags for the server-row subjects: the windows a selected spawn opens
//! (quests, loot, vendor, trainer, gossip, events, spells, scripts, a
//! condition) and the item, quest, creature and game object workspaces.

use super::Arity::*;
use super::{first, named, number, Flag};
use crate::tools::Tool;

/// The subjects' windows and workspaces.
#[derive(Debug, Clone, Default)]
pub struct Subjects {
    /// `--quests`: open the quest window on `--spawn`'s creature. It is the
    /// next button along from `--template`'s and exists for the same reason.
    pub creature_quests: bool,
    /// `--loot`: open the loot window on `--spawn`'s creature, `--object`'s
    /// object or `--item`'s item. It is the third button along, for the same
    /// reason. The window follows the selection, so it needs one of the three.
    pub loot_window: bool,
    /// `--loot-add <item>`: add that item to the set the loot window shows.
    /// A scripted run cannot make this gesture otherwise, and every later check
    /// needs it: the store, the SQL, the apply and the revert. Implies
    /// `--loot`.
    pub loot_add: Option<u32>,
    /// `--vendor` and `--trainer`: open the Vendor or the Trainer window on
    /// `--spawn`'s creature. Each follows the selection, so it needs
    /// `--spawn`.
    pub vendor_window: bool,
    pub trainer_window: bool,
    /// `--gossip`: open the gossip window on `--spawn`'s creature.
    pub gossip_window: bool,
    /// `--condition <id>`: open the condition window on one condition.
    pub condition: Option<u32>,
    /// `--vendor-add <item>` and `--trainer-add <spell>`: add a row to the
    /// list the window shows, which a scripted run cannot do otherwise. A
    /// spell that is not a teaching spell is replaced by the one that teaches
    /// it, as the window's own add does. Each implies its window.
    pub vendor_add: Option<u32>,
    pub trainer_add: Option<u32>,
    /// `--vendor-find <text>` and `--trainer-find <text>`: open the window's
    /// own picker for adding an item or a spell, with `text` searched. The
    /// picker is two clicks in, so a scripted run cannot reach it otherwise.
    /// Each implies its window.
    pub vendor_find: Option<String>,
    pub trainer_find: Option<String>,
    /// `--events`: open the events window on `--spawn`'s creature;
    /// `--event-add`: add an event to it, which is the gesture a scripted run
    /// cannot make and every check downstream needs. `--spells`: open the
    /// spells window. Each follows the selection, so it needs `--spawn`.
    pub events_window: bool,
    pub event_add: bool,
    pub spells_window: bool,
    /// `--script <table>:<id>`: open the script window on one script, with
    /// every step unfolded, which a scripted run cannot reach by a press on
    /// an event's action.
    pub script_window: Option<(&'static str, u32)>,
    /// `--find-event <text>`: open the events window and the chooser that
    /// adds an existing event, with `text` searched.
    pub find_event: Option<String>,
    /// `--item <entry or name>`: open the item workspace on that row.
    ///
    /// `--row` for items. It takes a name as well as an entry because items
    /// are known by name rather than by entry: `--item 'Linen Cloth'` and
    /// `--item 2589` are the same row. An exact name wins over a partial one; a
    /// name nothing matches opens nothing and prints that.
    ///
    /// It implies `--tool items`.
    pub item: Option<String>,
    /// `--item-new`: make a new item, and open it.
    ///
    /// The item counterpart of `--spawn-add`: creating a row is only a press on
    /// a panel, so without this a script cannot check a new item.
    pub item_new: bool,
    /// `--item-display <id>`: open the appearance picker, and choose that id
    /// for the open item.
    ///
    /// The picker is a grid of pictures two clicks in, and choosing what an
    /// item looks like is a click on one of them. The bare flag opens the
    /// picker, for a screenshot; with an id it also writes the column, which
    /// checks the whole path with nobody at the keyboard.
    ///
    /// It is stored in two fields, because `0` is a display id a row can
    /// legitimately hold (`vale_assets::tables::item` documents it as
    /// "nothing worn"), so it cannot also mean "no id was given".
    pub item_picker: bool,
    pub item_display: Option<u32>,
    /// `--item-entry <n>`: renumber the open item to that entry.
    ///
    /// This is the one action on the item form that is not a column edit: it
    /// re-keys the project's claim on the row. It needs its own flag to be
    /// checked with nobody at the keyboard. See
    /// `crate::subjects::items::Items::rekey`.
    pub item_entry: Option<u32>,
    /// `--item-remove`: mark the open item for removal, or give it up when this
    /// project created it. It is the item list's Remove button, for a scripted
    /// run. See `crate::subjects::items::Items::remove`.
    pub item_remove: bool,
    /// `--quest <entry or title>`: open the quest workspace on that row. It is
    /// `--item` for quests, and takes a title for the same reason. It implies
    /// `--tool quests`.
    pub quest: Option<String>,
    /// `--quest-new`: make a new quest, and open it.
    pub quest_new: bool,
    /// `--quest-entry <n>`: renumber the open quest to that entry. It is
    /// `--item-entry` for quests. See
    /// `crate::subjects::quests::Quests::rekey`.
    pub quest_entry: Option<u32>,
    /// `--quests-of <creature entry>`: open the quest workspace narrowed to the
    /// quests that creature gives or takes, on the first of them.
    ///
    /// This is what the Quests button on a selected creature does. Reaching it
    /// takes a click on a creature and then a press on its panel, two gestures
    /// a scripted run cannot make. It implies `--tool quests`.
    pub quests_of: Option<u32>,
    /// `--creature-data <entry>`: open the Creatures workspace on that
    /// `creature_template` row. It implies the workspace as the tool. See
    /// `crate::subjects::templates`.
    pub creature_data: Option<u32>,
    /// `--object-data <entry>`: the same for the Game Objects workspace and
    /// `gameobject_template`.
    pub object_data: Option<u32>,
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--quests", Switch, |a, _| a.subjects.creature_quests = true),
    Flag::new("--loot", Switch, |a, _| a.subjects.loot_window = true),
    Flag::new("--loot-add", One, |a, v| {
        a.subjects.loot_add = number(v);
        a.subjects.loot_window = true;
    }),
    Flag::new("--vendor", Switch, |a, _| a.subjects.vendor_window = true),
    Flag::new("--gossip", Switch, |a, _| a.subjects.gossip_window = true),
    Flag::new("--condition", One, |a, v| a.subjects.condition = number(v)),
    Flag::new("--trainer", Switch, |a, _| a.subjects.trainer_window = true),
    Flag::new("--vendor-add", One, |a, v| {
        a.subjects.vendor_add = number(v);
        a.subjects.vendor_window = true;
    }),
    Flag::new("--trainer-add", One, |a, v| {
        a.subjects.trainer_add = number(v);
        a.subjects.trainer_window = true;
    }),
    Flag::new("--vendor-find", One, |a, v| {
        a.subjects.vendor_find = named(v);
        a.subjects.vendor_window = true;
    }),
    Flag::new("--trainer-find", One, |a, v| {
        a.subjects.trainer_find = named(v);
        a.subjects.trainer_window = true;
    }),
    Flag::new("--events", Switch, |a, _| a.subjects.events_window = true),
    Flag::new("--event-add", Switch, |a, _| {
        a.subjects.event_add = true;
        a.subjects.events_window = true;
    }),
    Flag::new("--spells", Switch, |a, _| a.subjects.spells_window = true),
    Flag::new("--find-event", One, |a, v| {
        a.subjects.find_event = named(v);
        a.subjects.events_window = a.subjects.find_event.is_some() || a.subjects.events_window;
    }),
    Flag::new("--script", One, |a, v| {
        a.subjects.script_window = first(v).and_then(|v| {
            let (table, id) = v.split_once(':')?;
            Some((vale_mangos::scripts::table_named(table.trim())?, id.trim().parse().ok()?))
        });
    }),
    Flag::new("--item", One, |a, v| {
        a.subjects.item = named(v);
        // It names an item, so it selects the item workspace
        // whatever else was asked for, by the same rule as
        // `--spawn`.
        a.view.tool = Some(Tool::Items);
    }),
    Flag::new("--item-new", Switch, |a, _| {
        a.subjects.item_new = true;
        a.view.tool = Some(Tool::Items);
    }),
    // The id is optional, so a following flag is not eaten and
    // the bare flag opens the picker without choosing.
    Flag::new("--item-display", Optional, |a, v| {
        a.subjects.item_picker = true;
        if !v.is_empty() {
            a.subjects.item_display = number(v);
        }
        a.view.tool = Some(Tool::Items);
    }),
    Flag::new("--item-entry", One, |a, v| {
        a.subjects.item_entry = number(v);
        a.view.tool = Some(Tool::Items);
    }),
    Flag::new("--item-remove", Switch, |a, _| {
        a.subjects.item_remove = true;
        a.view.tool = Some(Tool::Items);
    }),
    Flag::new("--quest", One, |a, v| {
        a.subjects.quest = named(v);
        a.view.tool = Some(Tool::Quests);
    }),
    Flag::new("--quest-new", Switch, |a, _| {
        a.subjects.quest_new = true;
        a.view.tool = Some(Tool::Quests);
    }),
    Flag::new("--quest-entry", One, |a, v| {
        a.subjects.quest_entry = number(v);
        a.view.tool = Some(Tool::Quests);
    }),
    Flag::new("--quests-of", One, |a, v| {
        a.subjects.quests_of = number(v);
        a.view.tool = Some(Tool::Quests);
    }),
    Flag::new("--creature-data", One, |a, v| {
        a.subjects.creature_data = number(v);
        a.view.tool = Some(Tool::CreatureData);
    }),
    Flag::new("--object-data", One, |a, v| {
        a.subjects.object_data = number(v);
        a.view.tool = Some(Tool::GameObjectData);
    }),
];
