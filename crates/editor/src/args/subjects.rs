//! The flags for the server-row subjects: the windows a selected spawn opens
//! (quests, loot, vendor, trainer, equipment, gossip, events, spells,
//! scripts, a condition), its addon row, and the item, quest, creature and
//! game object workspaces.

use super::Arity::*;
use super::{first, named, number, Flag};
use crate::tools::Tool;

/// The subjects' windows and workspaces.
#[derive(Debug, Clone, Default)]
pub(crate) struct Subjects {
    /// `--quests`: open the quest window on `--spawn`'s creature. It is the
    /// next button along from `--template`'s and exists for the same reason.
    pub(crate) creature_quests: bool,
    /// `--loot`: open the loot window on `--spawn`'s creature, `--object`'s
    /// object or `--item`'s item. It is the third button along, for the same
    /// reason. The window follows the selection, so it needs one of the three.
    pub(crate) loot_window: bool,
    /// `--loot-add <item>`: add that item to the set the loot window shows.
    /// A scripted run cannot make this gesture otherwise, and every later check
    /// needs it: the store, the SQL, the apply and the revert. Implies
    /// `--loot`.
    pub(crate) loot_add: Option<u32>,
    /// `--vendor` and `--trainer`: open the Vendor or the Trainer window on
    /// `--spawn`'s creature. Each follows the selection, so it needs
    /// `--spawn`.
    pub(crate) vendor_window: bool,
    pub(crate) trainer_window: bool,
    /// `--gossip`: open the gossip window on `--spawn`'s creature.
    pub(crate) gossip_window: bool,
    /// `--condition <id>`: open the condition window on one condition.
    pub(crate) condition: Option<u32>,
    /// `--vendor-add <item>` and `--trainer-add <spell>`: add a row to the
    /// list the window shows, which a scripted run cannot do otherwise. A
    /// spell that is not a teaching spell is replaced by the one that teaches
    /// it, as the window's own add does. Each implies its window.
    pub(crate) vendor_add: Option<u32>,
    pub(crate) trainer_add: Option<u32>,
    /// `--vendor-find <text>` and `--trainer-find <text>`: open the window's
    /// own picker for adding an item or a spell, with `text` searched. The
    /// picker is two clicks in, so a scripted run cannot reach it otherwise.
    /// Each implies its window.
    pub(crate) vendor_find: Option<String>,
    pub(crate) trainer_find: Option<String>,
    /// `--equipment`: open the Equipment window on `--spawn`'s creature.
    /// `--equipment-add <item>`: add a row holding the item in the main hand
    /// to the set the window shows, which implies the window. `--addon`:
    /// give `--spawn`'s creature a `creature_addon` row, which the Addon
    /// section's checkbox does. Each follows the selection, so it needs
    /// `--spawn`.
    pub(crate) equipment_window: bool,
    pub(crate) equipment_add: Option<u32>,
    pub(crate) addon_row: bool,
    /// `--formation-add <guid>`: that spawn joins the formation of
    /// `--spawn`'s creature where it stands, or a new one `--spawn` leads.
    /// `--link-add <guid>`: that spawn becomes a slave of `--spawn`'s. These
    /// are Add Selected and Link Selected as Slaves, which need a
    /// shift-click a scripted run cannot make.
    pub(crate) formation_add: Option<u64>,
    pub(crate) link_add: Option<u64>,
    /// `--pool <entry>`: open the Pool window on a pool. `--pool-add
    /// <entry>`: put `--spawn`'s creature, or `--object`'s game object, in
    /// that pool, or in a new one for 0, and open it.
    pub(crate) pool_window: Option<u32>,
    pub(crate) pool_add: Option<u32>,
    /// `--game-event <entry>`: open the Game Event window on an event.
    /// `--game-event-add <entry>`: add `--spawn`'s creature, or `--object`'s
    /// game object, to the event, or for a negative entry have the event
    /// remove it.
    pub(crate) game_event_window: Option<u32>,
    pub(crate) game_event_add: Option<i32>,
    /// `--event-view <all|outside|entry>`: the view bar's event filter, which
    /// is otherwise a menu a scripted run cannot open. `--others`: each spawn
    /// tool draws the other's spawns too, which is otherwise a checkbox.
    pub(crate) event_view: Option<crate::subjects::game_events::Phase>,
    pub(crate) others: bool,
    /// `--go-to-spawn <creature|object>:<guid>`: Go To on a spawn, as a
    /// pool's or a game event's list does it, once the spawn's map is read.
    pub(crate) go_to_spawn: Option<crate::tools::spawns::spawn::SpawnRef>,
    /// `--events`: open the events window on `--spawn`'s creature;
    /// `--event-add`: add an event to it, which is the gesture a scripted run
    /// cannot make and every check downstream needs. `--spells`: open the
    /// spells window. Each follows the selection, so it needs `--spawn`.
    pub(crate) events_window: bool,
    pub(crate) event_add: bool,
    pub(crate) spells_window: bool,
    /// `--script <table>:<id>`: open the script window on one script, with
    /// every step unfolded, which a scripted run cannot reach by a press on
    /// an event's action.
    pub(crate) script_window: Option<(&'static str, u32)>,
    /// `--find-event <text>`: open the events window and the chooser that
    /// adds an existing event, with `text` searched.
    pub(crate) find_event: Option<String>,
    /// `--item <entry or name>`: open the item workspace on that row.
    ///
    /// `--row` for items. It takes a name as well as an entry because items
    /// are known by name rather than by entry: `--item 'Linen Cloth'` and
    /// `--item 2589` are the same row. An exact name wins over a partial one; a
    /// name nothing matches opens nothing and prints that.
    ///
    /// It implies `--tool items`.
    pub(crate) item: Option<String>,
    /// `--item-new`: make a new item, and open it.
    ///
    /// The item counterpart of `--spawn-add`: creating a row is only a press on
    /// a panel, so without this a script cannot check a new item.
    pub(crate) item_new: bool,
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
    pub(crate) item_picker: bool,
    pub(crate) item_display: Option<u32>,
    /// `--item-entry <n>`: renumber the open item to that entry.
    ///
    /// This is the one action on the item form that is not a column edit: it
    /// re-keys the project's claim on the row. It needs its own flag to be
    /// checked with nobody at the keyboard. See
    /// `crate::subjects::items::Items::rekey`.
    pub(crate) item_entry: Option<u32>,
    /// `--item-remove`: mark the open item for removal, or give it up when this
    /// project created it. It is the item list's Remove button, for a scripted
    /// run. See `crate::subjects::items::Items::remove`.
    pub(crate) item_remove: bool,
    /// `--quest <entry or title>`: open the quest workspace on that row. It is
    /// `--item` for quests, and takes a title for the same reason. It implies
    /// `--tool quests`.
    pub(crate) quest: Option<String>,
    /// `--quest-new`: make a new quest, and open it.
    pub(crate) quest_new: bool,
    /// `--quest-entry <n>`: renumber the open quest to that entry. It is
    /// `--item-entry` for quests. See
    /// `crate::subjects::quests::Quests::rekey`.
    pub(crate) quest_entry: Option<u32>,
    /// `--quests-of <creature entry>`: open the quest workspace narrowed to the
    /// quests that creature gives or takes, on the first of them.
    ///
    /// This is what the Quests button on a selected creature does. Reaching it
    /// takes a click on a creature and then a press on its panel, two gestures
    /// a scripted run cannot make. It implies `--tool quests`.
    pub(crate) quests_of: Option<u32>,
    /// `--creature-data <entry>`: open the Creatures workspace on that
    /// `creature_template` row. It implies the workspace as the tool. See
    /// `crate::subjects::templates`.
    pub(crate) creature_data: Option<u32>,
    /// `--object-data <entry>`: the same for the Game Objects workspace and
    /// `gameobject_template`.
    pub(crate) object_data: Option<u32>,
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
    Flag::new("--equipment", Switch, |a, _| a.subjects.equipment_window = true),
    Flag::new("--equipment-add", One, |a, v| {
        a.subjects.equipment_add = number(v);
        a.subjects.equipment_window = true;
    }),
    Flag::new("--addon", Switch, |a, _| a.subjects.addon_row = true),
    Flag::new("--formation-add", One, |a, v| a.subjects.formation_add = number(v)),
    Flag::new("--link-add", One, |a, v| a.subjects.link_add = number(v)),
    Flag::new("--pool", One, |a, v| a.subjects.pool_window = number(v)),
    Flag::new("--pool-add", One, |a, v| a.subjects.pool_add = number(v)),
    Flag::new("--game-event", One, |a, v| a.subjects.game_event_window = number(v)),
    Flag::new("--game-event-add", One, |a, v| a.subjects.game_event_add = number(v)),
    Flag::new("--event-view", One, |a, v| {
        use crate::subjects::game_events::Phase;
        a.subjects.event_view = match first(v) {
            Some("all") => Some(Phase::All),
            Some("outside") => Some(Phase::Outside),
            other => other.and_then(|word| word.parse().ok()).map(Phase::During),
        };
    }),
    Flag::new("--others", Switch, |a, _| a.subjects.others = true),
    Flag::new("--go-to-spawn", One, |a, v| {
        use crate::tools::spawns::spawn::{SpawnKind, SpawnRef};
        a.subjects.go_to_spawn = first(v).and_then(|word| {
            let (kind, guid) = word.split_once(':')?;
            let kind = match kind {
                "creature" => SpawnKind::Creature,
                "object" => SpawnKind::GameObject,
                _ => return None,
            };
            Some(SpawnRef { kind, guid: guid.parse().ok()? })
        });
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
