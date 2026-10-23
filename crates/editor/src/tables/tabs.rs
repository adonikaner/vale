//! Which tables each workspace reaches: the tabs, and a table named in any
//! case. The tables opened beside a subject's own are
//! `vale_assets::tables::schema::chain_for`'s.

use vale_assets::tables::schema;


/// The flight path tables, in the order a flight is read: the node, the path
/// from it, the points of the path.
pub(crate) const TAXI_TABS: [(&str, &str); 3] = [
    ("Nodes", "TaxiNodes"),
    ("Paths", "TaxiPath"),
    ("Points", "TaxiPathNode"),
];

/// The trigger tool's one table. It keeps the viewport, so no tab is drawn.
pub(crate) const TRIGGER_TABS: [(&str, &str); 1] = [("Triggers", "AreaTrigger")];

/// The graveyard tool's one table. It also keeps the viewport, so no tab is
/// drawn.
pub(crate) const SAFE_LOC_TABS: [(&str, &str); 1] = [("Graveyards", "WorldSafeLocs")];

/// The spell workspace's tabs: the spell chain in the order a cast reads it,
/// then the three skill tables. The workspace draws them [`TAB_ROW`] to a
/// row, so the chain is the first row and the skill tables the second.
/// Callers read the tabs through [`crate::tools::Tool::tabs`], which chooses
/// between this, [`LIGHT_TABS`], [`TAXI_TABS`] and [`SET_TABS`].
pub(crate) const SPELL_TABS: [(&str, &str); 7] = [
    ("Spells", "Spell"),
    ("Visuals", "SpellVisual"),
    ("Kits", "SpellVisualKit"),
    ("Effects", "SpellVisualEffectName"),
    ("Skill lines", "SkillLine"),
    ("Abilities", "SkillLineAbility"),
    ("Race & class", "SkillRaceClassInfo"),
];

/// How many tabs the middle workspace draws to a row.
pub(crate) const TAB_ROW: usize = 4;

/// The Sets part of the Items workspace: one table. A single tab is not
/// drawn; the strip above it switches between the workspace's parts.
pub(crate) const SET_TABS: [(&str, &str); 1] = [("Sets", "ItemSet")];

/// The Displays part of the Creatures workspace: a display, the extra
/// appearance a character-model display names, and the model data a display
/// names, in the order a display is resolved.
pub(crate) const CREATURE_DISPLAY_TABS: [(&str, &str); 3] = [
    ("Display Info", "CreatureDisplayInfo"),
    ("Extra", "CreatureDisplayInfoExtra"),
    ("Model Data", "CreatureModelData"),
];

/// The Displays part of the Game Objects workspace: one table, so no tab is
/// drawn.
pub(crate) const OBJECT_DISPLAY_TABS: [(&str, &str); 1] = [("Display Info", "GameObjectDisplayInfo")];

/// The Displays part of the Items workspace: one table, so no tab is drawn.
pub(crate) const ITEM_DISPLAY_TABS: [(&str, &str); 1] = [("Display Info", "ItemDisplayInfo")];

/// The Zones workspace: one table, so no tab is drawn. The zones and their
/// sub-areas are rows of the same table.
pub(crate) const ZONE_TABS: [(&str, &str); 1] = [("Areas", "AreaTable")];

/// The table the Tables workspace starts on: none. Its list is the tables
/// themselves until one is chosen. See [`crate::tools::Tool::Tables`].
pub(crate) const ANY: &str = "";

/// The one name of the table `asked` names in any case, or `None` when it is
/// neither described nor a file of `DBFilesClient\`.
pub(crate) fn table_named(asked: &str) -> Option<&'static str> {
    schema::for_table(asked).map(|schema| schema.table).or_else(|| {
        schema::TABLE_NAMES
            .iter()
            .find(|known| known.eq_ignore_ascii_case(asked.trim()))
            .copied()
    })
}

/// The light chain's tabs, in the order it resolves: a light names params, and
/// params own bands.
///
/// `LightIntBand` and `LightFloatBand` have tabs although no column references
/// them. A band's id is computed from a params row rather than stored in a
/// reference column, so the browser cannot follow one, and the tab is the only
/// way to open those tables.
pub(crate) const LIGHT_TABS: [(&str, &str); 5] = [
    ("Lights", "Light"),
    ("Params", "LightParams"),
    ("Colours", "LightIntBand"),
    ("Numbers", "LightFloatBand"),
    ("Skyboxes", "LightSkybox"),
];
