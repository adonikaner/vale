//! Which tables each workspace reaches: the tabs, the tables a subject opens
//! beside its own, and a table named in any case.

use vale_assets::tables::schema;

/// The tables a subject needs in addition to its own root table.
///
/// There are three chains (the spell chain, the light chain and the flight
/// path tables) and the tables a skill table or an item set is labelled
/// from. A spell's form can be drawn from `Spell.dbc` alone, but its
/// storyboard cannot, and neither can a followed reference or a row labelled
/// from the rows it references. These tables are opened one per frame after
/// the subject's own; together they are smaller than `Spell.dbc`'s string
/// block, so opening them has no noticeable cost.
pub fn chain_for(table: &str) -> &'static [&'static str] {
    match table {
        "Spell" => &[
            "SpellVisual",
            "SpellVisualKit",
            "SpellVisualEffectName",
            "AnimationData",
            "SpellIcon",
            "SpellCastTimes",
            "SpellDuration",
            "SpellRadius",
            "SpellRange",
            "SoundEntries",
            "SpellEffectCameraShakes",
            // The skill tables, which are tabs of the spell workspace. With
            // `SkillLineAbility` open, a spell's reverse references list the
            // skill lines it is in.
            "SkillLine",
            "SkillLineAbility",
            "SkillRaceClassInfo",
            "SkillLineCategory",
        ],
        "Light" => &[
            "LightParams",
            "LightIntBand",
            "LightFloatBand",
            "LightSkybox",
            // `Map` is not part of the light chain but is opened with it. It is
            // not edited here and has no tab; it is read so a light's label
            // can show `Azeroth` instead of `map 0`. `Light` rows have no
            // names, so the map is most of what the label shows.
            "Map",
        ],
        // The paths and their points, which a node's form lists, and `Map` for
        // the same reason the light chain opens it.
        "TaxiNodes" => &["TaxiPath", "TaxiPathNode", "Map"],
        // A set's bonuses are spells and its requirement is a skill line.
        "ItemSet" => &["Spell", "SkillLine"],
        // The creature display chain: what a display's model and extra
        // columns name, which are its two other tabs, and the items an extra
        // row dresses the creature in.
        "CreatureDisplayInfo" | "CreatureDisplayInfoExtra" | "CreatureModelData" => &[
            "CreatureDisplayInfo",
            "CreatureDisplayInfoExtra",
            "CreatureModelData",
            "ItemDisplayInfo",
        ],
        // A game object's ten sounds.
        "GameObjectDisplayInfo" => &["SoundEntries"],
        // The visual an item's glow is.
        "ItemDisplayInfo" => &["SpellVisual"],
        // The skill tables, for when one is opened from the Tables
        // workspace and the spell chain is not open: each is labelled from
        // the others.
        "SkillLine" => &["SkillLineCategory", "SpellIcon"],
        "SkillLineAbility" | "SkillRaceClassInfo" => &["SkillLine", "Spell"],
        // `Map`, which an area's label names, and the five tables its sound
        // and liquid columns refer to, so each reference is drawn as a name.
        //
        // `SoundEntries` as well, which the music, intro and ambience rows
        // name, so a sound's play button says which sound it plays.
        "AreaTable" => &[
            "Map",
            "ZoneMusic",
            "ZoneIntroMusicTable",
            "SoundAmbience",
            "SoundProviderPreferences",
            "LiquidType",
            "SoundEntries",
        ],
        _ => &[],
    }
}

/// The flight path tables, in the order a flight is read: the node, the path
/// from it, the points of the path.
pub const TAXI_TABS: [(&str, &str); 3] = [
    ("Nodes", "TaxiNodes"),
    ("Paths", "TaxiPath"),
    ("Points", "TaxiPathNode"),
];

/// The trigger tool's one table. It keeps the viewport, so no tab is drawn.
pub const TRIGGER_TABS: [(&str, &str); 1] = [("Triggers", "AreaTrigger")];

/// The graveyard tool's one table. It also keeps the viewport, so no tab is
/// drawn.
pub const SAFE_LOC_TABS: [(&str, &str); 1] = [("Graveyards", "WorldSafeLocs")];

/// The spell workspace's tabs: the spell chain in the order a cast reads it,
/// then the three skill tables. The workspace draws them [`TAB_ROW`] to a
/// row, so the chain is the first row and the skill tables the second.
/// Callers read the tabs through [`crate::tools::Tool::tabs`], which chooses
/// between this, [`LIGHT_TABS`], [`TAXI_TABS`] and [`SET_TABS`].
pub const SPELL_TABS: [(&str, &str); 7] = [
    ("Spells", "Spell"),
    ("Visuals", "SpellVisual"),
    ("Kits", "SpellVisualKit"),
    ("Effects", "SpellVisualEffectName"),
    ("Skill lines", "SkillLine"),
    ("Abilities", "SkillLineAbility"),
    ("Race & class", "SkillRaceClassInfo"),
];

/// How many tabs the middle workspace draws to a row.
pub const TAB_ROW: usize = 4;

/// The Sets part of the Items workspace: one table. A single tab is not
/// drawn; the strip above it switches between the workspace's parts.
pub const SET_TABS: [(&str, &str); 1] = [("Sets", "ItemSet")];

/// The Displays part of the Creatures workspace: a display, the extra
/// appearance a character-model display names, and the model data a display
/// names, in the order a display is resolved.
pub const CREATURE_DISPLAY_TABS: [(&str, &str); 3] = [
    ("Display Info", "CreatureDisplayInfo"),
    ("Extra", "CreatureDisplayInfoExtra"),
    ("Model Data", "CreatureModelData"),
];

/// The Displays part of the Game Objects workspace: one table, so no tab is
/// drawn.
pub const OBJECT_DISPLAY_TABS: [(&str, &str); 1] = [("Display Info", "GameObjectDisplayInfo")];

/// The Displays part of the Items workspace: one table, so no tab is drawn.
pub const ITEM_DISPLAY_TABS: [(&str, &str); 1] = [("Display Info", "ItemDisplayInfo")];

/// The Zones workspace: one table, so no tab is drawn. The zones and their
/// sub-areas are rows of the same table.
pub const ZONE_TABS: [(&str, &str); 1] = [("Areas", "AreaTable")];

/// The table the Tables workspace starts on: none. Its list is the tables
/// themselves until one is chosen. See [`crate::tools::Tool::Tables`].
pub const ANY: &str = "";

/// The one name of the table `asked` names in any case, or `None` when it is
/// neither described nor a file of `DBFilesClient\`.
pub fn table_named(asked: &str) -> Option<&'static str> {
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
pub const LIGHT_TABS: [(&str, &str); 5] = [
    ("Lights", "Light"),
    ("Params", "LightParams"),
    ("Colours", "LightIntBand"),
    ("Numbers", "LightFloatBand"),
    ("Skyboxes", "LightSkybox"),
];
