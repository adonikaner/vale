//! **Where you are when you are inside a building** — `WMOAreaTable.dbc`.
//!
//! [`crate::tables::area`] answers "where am I" from the ground under the character's
//! feet, which is `MCNK`'s own `areaId`. That is the whole answer in open
//! country and it is **not** the answer in a city or a tavern: the terrain under
//! Ironforge says Dun Morogh in all 194 of its chunks (measured on
//! `Azeroth_33_41`, which is the tile the city stands on), and the terrain under
//! the Lion's Pride Inn says Goldshire. Both were reported exactly that way.
//!
//! The place names for those are not in `AreaTable` at all — "The Great Forge"
//! and "Lion's Pride Inn" appear nowhere in its 1,081 rows. They are in this
//! table, keyed by the *building* rather than by the ground:
//!
//! ```text
//! WMOAreaTable.dbc   21,115 rows x 20 fields
//!   [ 0] id
//!   [ 1] rootId      MOHD 0x20      — which WMO
//!   [ 2] nameSetId   MODF 0x3C      — which dressing of it this placement is
//!   [ 3] groupId     MOGP 0x38      — which group, or -1 for the whole building
//!   [ 4] soundProviderPref     [ 5] …underwater
//!   [ 6] ambienceId            [ 7] zoneMusic          [ 8] introSound
//!   [ 9] flags
//!   [10] areaTableId           the `AreaTable` row this counts as, or 0
//!   [11] name                  enUS, first of the eight locale columns
//!   [19] nameFlags
//! ```
//!
//! The column block at 4..8 is the same shape `AreaTable` carries and the name
//! block is the same eight-locale one [`crate::tables::area`] reads at 11 — 12..18 are
//! zero in every row of the shipped file, which is what pins the block's start.
//!
//! ## Three halves of the answer, and they are separate
//!
//! * **`ambienceId` / `zoneMusic` / `introSound`** are what the building
//!   *sounds* like, and they are the reason a tavern sounds like a tavern.
//!   **6,441 of the table's rows state one**, and they are reachable no other
//!   way: the Lion's Pride Inn's `-1` row names ambience 170 and `ZoneMusic`
//!   **156, `Zone-TavernAlliance`**, while its `areaTableId` is 0 — so a client
//!   that reads only the area id gets Elwynn Forest's row and plays the forest
//!   at the bar, which is exactly how it was reported. See [`WmoAreas::sounds`],
//!   which is a different lookup from [`WmoAreas::lookup`] and says why.
//! * **`areaTableId`** is the zone this counts as. Ironforge's 104 groups all
//!   name 1537 ("Ironforge", a zone with no parent), which is how a city that
//!   stands on Dun Morogh's dirt is its own zone. **Zero is common and it means
//!   "the ground still knows best"** — the Lion's Pride Inn's rows say 0, and
//!   the inn really is in Goldshire.
//! * **`name`** is the sub-area, and it is this table's own string rather than
//!   an `AreaTable` name. That is the half that cannot be got any other way:
//!   "The Great Forge", "Tinker Town", "The Commerce Ward", "Lion's Pride Inn"
//!   are rows *here* and nowhere else.
//!
//! So a lookup answers a `(area, name)` pair where either half may be absent,
//! and the caller composes it with what the terrain said. See
//! [`WmoArea::area_or`].
//!
//! ## The `-1` row is the building's own
//!
//! Every WMO carries a row per group **plus** one with `groupId == -1`, which is
//! what a group with no row of its own falls back to: `ironforge.wmo`'s -1 row
//! is "City of Ironforge", and 42 of its groups name no place. A building whose
//! only row is the -1 (a small inn: 13 groups, one name) works entirely through
//! that fallback.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

mod fields {
    pub const ROOT_ID: usize = 1;
    pub const NAME_SET: usize = 2;
    pub const GROUP_ID: usize = 3;
    pub const AMBIENCE: usize = 6;
    pub const ZONE_MUSIC: usize = 7;
    pub const INTRO_SOUND: usize = 8;
    pub const AREA_TABLE_ID: usize = 10;
    pub const NAME: usize = 11;
}

/// The `groupId` of the row that stands for the whole building — see the module
/// comment. Stored as the `u32` the file holds it as.
const WHOLE_BUILDING: u32 = u32::MAX;

/// One row, reduced to the three things anything asks of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WmoArea {
    /// The `AreaTable` row this counts as, **or 0 for "ask the ground"**.
    pub area: u32,
    /// The sub-area's own name, or empty. See the module comment on why this is
    /// not an `AreaTable` name.
    pub name: String,
    /// **The building's own ambience, music and intro**, each 0 for "not stated
    /// here" — see [`WmoAreas::sounds`], which is the question this answers and
    /// is deliberately *not* [`WmoAreas::lookup`]'s.
    pub sounds: crate::tables::sound::AreaSounds,
}

impl WmoArea {
    /// The area id to use, given what the terrain under the character said.
    ///
    /// The building wins where it states one, because it is the more specific
    /// statement — a WMO placed on Dun Morogh's dirt saying 1537 is the only
    /// thing in any file that knows Ironforge is a zone.
    pub fn area_or(&self, terrain: u32) -> u32 {
        if self.area == 0 { terrain } else { self.area }
    }

    /// A row that names no place and claims no area — **a real answer**, and the
    /// commonest one: 42 of Ironforge's 104 groups are stairwells and bridges
    /// the game leaves unnamed while keeping the city's zone. See
    /// [`WmoAreas::parse`] for the one thing this is used to decide.
    fn is_silent(&self) -> bool {
        self.area == 0 && self.name.is_empty()
    }
}

/// `WMOAreaTable.dbc`, parsed and keyed the way it is asked.
#[derive(Debug, Default)]
pub struct WmoAreas {
    by_key: HashMap<(u32, u32, u32), WmoArea>,
}

impl WmoAreas {
    /// Parse the table. `None` for bytes that are not a DBC, which costs every
    /// building's own place name and nothing else — the ground's answer is
    /// unaffected, so the degradation is exactly the behaviour this client had
    /// before the table was read at all.
    pub fn parse(raw: &[u8]) -> Option<WmoAreas> {
        let dbc = Dbc::parse(raw).ok()?;
        let mut by_key: HashMap<(u32, u32, u32), WmoArea> =
            HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(root) = dbc.u32_at(record, fields::ROOT_ID) else {
                continue;
            };
            let name_set = dbc.u32_at(record, fields::NAME_SET).unwrap_or(0);
            let group = dbc.u32_at(record, fields::GROUP_ID).unwrap_or(WHOLE_BUILDING);
            let row = WmoArea {
                area: dbc.u32_at(record, fields::AREA_TABLE_ID).unwrap_or(0),
                name: dbc.string_at(record, fields::NAME).unwrap_or_default(),
                sounds: crate::tables::sound::AreaSounds {
                    ambience: dbc.u32_at(record, fields::AMBIENCE).unwrap_or(0),
                    zone_music: dbc.u32_at(record, fields::ZONE_MUSIC).unwrap_or(0),
                    intro_music: dbc.u32_at(record, fields::INTRO_SOUND).unwrap_or(0),
                },
            };
            // **Ten keys are stated twice and three of those disagree**, all of
            // them one building: `wmoId` 556 name set 1, where one row is
            // "Stonehearth Outpost" in area 2958 and the other is blank. The
            // file states no order between them, so keeping whichever came last
            // would be a rule read off the record order rather than off the
            // game. A row that says something outranks one that says nothing,
            // which is the only asymmetry there is.
            match by_key.entry((root, name_set, group)) {
                std::collections::hash_map::Entry::Occupied(mut held) => {
                    if held.get().is_silent() && !row.is_silent() {
                        held.insert(row);
                    }
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(row);
                }
            }
        }
        Some(WmoAreas { by_key })
    }

    /// **What one group of one placement is called** — its own row, else the
    /// building's `-1` row, else nothing.
    ///
    /// **A row that says nothing at all falls through too**, and that is not a
    /// tidying-up: the Lion's Pride Inn has a row for each of its twelve groups
    /// and every one of them is blank, with the name on the `-1` row alone. A
    /// lookup that stopped at the first row *found* would answer nothing for
    /// every point inside the building it is named after.
    ///
    /// It costs nothing where a group does say something. Ironforge's 42 unnamed
    /// groups — the stairwells and the bridges — carry area 1537 and so are not
    /// silent, which is what keeps them in the city while leaving them unnamed
    /// rather than relabelling all of them "City of Ironforge".
    pub fn lookup(&self, root: u32, name_set: u32, group: u32) -> Option<&WmoArea> {
        let own = self.by_key.get(&(root, name_set, group));
        if own.is_some_and(|row| !row.is_silent()) {
            return own;
        }
        self.by_key
            .get(&(root, name_set, WHOLE_BUILDING))
            .filter(|row| !row.is_silent())
            .or(own)
    }

    /// **What a building sounds like** — its own three ids, a column at a time,
    /// over the whole-building `-1` row's.
    ///
    /// This is a *different question* from [`Self::lookup`] and is deliberately
    /// not answered through it, which took one attempt to get wrong. That
    /// lookup picks **one** row by whether it names a place, and 2,954 of the
    /// table's rows state a sound while naming nothing at all — so a group that
    /// carries its own ambience and no name falls through to the building's row
    /// there and would be answered with a different room's sound. The Lion's
    /// Pride Inn is that case in both directions: `wmoId` 53's `-1` row holds
    /// the name *and* ambience 170 / music 156 (`Zone-TavernAlliance`), while
    /// group 903 holds ambience 27 / music 183 and no name, so the name comes
    /// from the building and the sound from the room.
    ///
    /// **The columns are why the inn was reported silent**: its `-1` row states
    /// `areaTableId` **0**, so every join through `AreaTable` lands on Elwynn
    /// Forest and plays Elwynn's music. Nothing but this table knows a tavern
    /// has tavern music.
    pub fn sounds(&self, root: u32, name_set: u32, group: u32) -> crate::tables::sound::AreaSounds {
        let own = self
            .by_key
            .get(&(root, name_set, group))
            .map(|row| row.sounds)
            .unwrap_or_default();
        let whole = self
            .by_key
            .get(&(root, name_set, WHOLE_BUILDING))
            .map(|row| row.sounds)
            .unwrap_or_default();
        own.or(whole)
    }

    /// How many distinct `(root, name set, group)` keys the table carries —
    /// **21,105 of the file's 21,115 rows**, the ten missing being the duplicate
    /// keys [`Self::parse`] collapses.
    pub fn count(&self) -> usize {
        self.by_key.len()
    }

    /// Every row, for a census — the CLI's, which is the only caller.
    pub fn rows(&self) -> impl Iterator<Item = &WmoArea> {
        self.by_key.values()
    }

    /// **Every `AreaTable` row this building names**, over all its name sets and
    /// all its groups, in no order and without repeats.
    ///
    /// A census question rather than a lookup — [`Self::lookup`] answers for the
    /// group the character is standing in, and this answers whether the building
    /// states an area *anywhere*. The two are far apart on a dungeon: the twenty
    /// maps that are one building and no ground state **nothing at all** here,
    /// which is why `game::place::worldmap` needs a fallback and why that
    /// fallback is a measurement rather than a precaution. See `vale zones`.
    pub fn areas_named_by(&self, root: u32) -> Vec<u32> {
        let mut found: Vec<u32> = self
            .by_key
            .iter()
            .filter(|((key_root, _, _), row)| *key_root == root && row.area != 0)
            .map(|(_, row)| row.area)
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// **How many rows this building has at all**, named or not — the
    /// provenance beside [`Self::areas_named_by`]'s answer.
    ///
    /// "The building states no area" and "the census never found the building"
    /// are the same empty answer and are not the same claim, and a check that
    /// cannot tell them apart is one that reports a measurement when it has a
    /// hole. One caller, `vale zones`.
    pub fn rows_for(&self, root: u32) -> usize {
        self.by_key
            .keys()
            .filter(|(key_root, _, _)| *key_root == root)
            .count()
    }

    /// How many rows name a place, which is the check that the name column is
    /// being read as a column: 12..18 are empty in every shipped row, so reading
    /// one of those answers a plausible nothing everywhere.
    pub fn named_count(&self) -> usize {
        self.by_key.values().filter(|a| !a.name.is_empty()).count()
    }

    /// How many name an `AreaTable` row. The two counts overlap and neither
    /// contains the other — see the module comment.
    pub fn with_area_count(&self) -> usize {
        self.by_key.values().filter(|a| a.area != 0).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// `\0City of Ironforge\0The Great Forge\0`
    const NAMES: &[u8] = b"\0City of Ironforge\0The Great Forge\0";

    fn row(root: u32, name_set: u32, group: u32, area: u32, name: u32) -> Vec<u32> {
        let mut row = vec![0u32; 20];
        row[0] = 1;
        row[fields::ROOT_ID] = root;
        row[fields::NAME_SET] = name_set;
        row[fields::GROUP_ID] = group;
        row[fields::AREA_TABLE_ID] = area;
        row[fields::NAME] = name;
        row
    }

    fn sound_row(
        root: u32,
        name_set: u32,
        group: u32,
        area: u32,
        name: u32,
        sounds: [u32; 3],
    ) -> Vec<u32> {
        let mut row = row(root, name_set, group, area, name);
        row[fields::AMBIENCE] = sounds[0];
        row[fields::ZONE_MUSIC] = sounds[1];
        row[fields::INTRO_SOUND] = sounds[2];
        row
    }

    fn areas() -> WmoAreas {
        WmoAreas::parse(&dbc(
            &[
                // The building's own row, and one group that names a place.
                row(208, 0, WHOLE_BUILDING, 1537, 1),
                row(208, 0, 3610, 1537, 19),
                // A named group with no area at all — the tavern case.
                row(53, 2, WHOLE_BUILDING, 0, 19),
                // The same building, dressed differently: a *different* place.
                row(53, 1, WHOLE_BUILDING, 2104, 1),
            ],
            20,
            NAMES,
        ))
        .expect("the table parses")
    }

    #[test]
    fn a_group_answers_with_its_own_name_and_its_buildings_zone() {
        let areas = areas();
        let forge = areas.lookup(208, 0, 3610).expect("the group's own row");
        assert_eq!(forge.name, "The Great Forge");
        assert_eq!(forge.area, 1537);
    }

    /// **A group with no row of its own is the building's**, which is how a
    /// twelve-group inn with one row names every room in it.
    #[test]
    fn a_group_with_no_row_falls_back_to_the_building() {
        let areas = areas();
        let stair = areas.lookup(208, 0, 9999).expect("the -1 row");
        assert_eq!(stair.name, "City of Ironforge");
        assert_eq!(stair.area, 1537);
    }

    /// **…and so is a group whose row is blank**, which is the Lion's Pride
    /// Inn's real shape: twelve group rows, all empty, and the name on the
    /// building's row alone. This is the case that decides whether standing in
    /// a tavern says anything at all.
    ///
    /// A group that says *something* keeps its own answer, even where that is
    /// an area with no name — Ironforge's stairwells are in Ironforge and are
    /// not "City of Ironforge".
    #[test]
    fn a_blank_group_row_falls_through_to_the_building_too() {
        let areas = WmoAreas::parse(&dbc(
            &[
                row(53, 2, WHOLE_BUILDING, 0, 19),
                row(53, 2, 893, 0, 0),
                row(208, 0, WHOLE_BUILDING, 1537, 1),
                row(208, 0, 3556, 1537, 0),
            ],
            20,
            NAMES,
        ))
        .expect("the table parses");
        assert_eq!(
            areas.lookup(53, 2, 893).expect("row").name,
            "The Great Forge",
            "a blank group row is the building's"
        );
        let stair = areas.lookup(208, 0, 3556).expect("row");
        assert_eq!(stair.name, "", "…but an area with no name is an answer");
        assert_eq!(stair.area, 1537);
    }

    /// **A tavern's music comes from this table and nowhere else.** The Lion's
    /// Pride Inn states `areaTableId` 0 — so every join through `AreaTable`
    /// lands on Elwynn Forest — and states `ZoneMusic` 156 on its own row.
    /// Reading the area id and stopping is what played the forest at the bar.
    #[test]
    fn a_buildings_own_sound_columns_are_the_only_place_a_tavern_states_one() {
        let areas = WmoAreas::parse(&dbc(
            &[sound_row(53, 2, WHOLE_BUILDING, 0, 19, [170, 156, 0])],
            20,
            NAMES,
        ))
        .expect("the table parses");
        let sounds = areas.sounds(53, 2, 893);
        assert_eq!(sounds.ambience, 170);
        assert_eq!(sounds.zone_music, 156, "Zone-TavernAlliance");
        assert_eq!(sounds.intro_music, 0, "and the inn has no fanfare");
        assert_eq!(
            areas.lookup(53, 2, 893).expect("row").area,
            0,
            "…while the area id says nothing at all, which is the point"
        );
    }

    /// **A room's own column beats the building's, one column at a time**, and
    /// the row that supplies it may name no place — 2,954 of the shipped rows
    /// are exactly that, which is why this is not [`WmoAreas::lookup`]'s answer.
    #[test]
    fn a_rooms_own_sound_wins_column_by_column_over_the_buildings() {
        let areas = WmoAreas::parse(&dbc(
            &[
                sound_row(53, 0, WHOLE_BUILDING, 0, 19, [170, 156, 4]),
                // A blank room that states an ambience and nothing else.
                sound_row(53, 0, 903, 0, 0, [27, 0, 0]),
            ],
            20,
            NAMES,
        ))
        .expect("the table parses");
        let room = areas.sounds(53, 0, 903);
        assert_eq!(room.ambience, 27, "the room's own");
        assert_eq!(room.zone_music, 156, "…and the building's music under it");
        assert_eq!(room.intro_music, 4);
        assert_eq!(
            areas.lookup(53, 0, 903).expect("row").name,
            "The Great Forge",
            "the *name* still falls through to the building, which is a \
             different question and stays one"
        );
        assert_eq!(
            areas.sounds(53, 0, 1),
            crate::tables::sound::AreaSounds {
                ambience: 170,
                zone_music: 156,
                intro_music: 4,
            },
            "a room with no row of its own is the building's outright"
        );
    }

    /// A building nothing in the table mentions states nothing, rather than
    /// answering the first row it finds.
    #[test]
    fn a_building_with_no_rows_states_no_sound() {
        let areas = areas();
        assert_eq!(
            areas.sounds(9999, 0, 1),
            crate::tables::sound::AreaSounds::default()
        );
    }

    /// **The name set is part of the key, not decoration.** One inn model is
    /// placed all over the world and each placement is a different tavern; a
    /// lookup that ignored it would call every one of them by the first one's
    /// name.
    #[test]
    fn the_name_set_picks_which_dressing_of_a_building_this_is() {
        let areas = areas();
        assert_eq!(areas.lookup(53, 2, 1).expect("row").name, "The Great Forge");
        assert_eq!(
            areas.lookup(53, 1, 1).expect("row").name,
            "City of Ironforge"
        );
        assert_eq!(areas.lookup(53, 7, 1), None, "a dressing with no rows");
    }

    /// **Area 0 means the ground still knows best**, which is most of the
    /// taverns in the game — and a stated area wins over it, which is the whole
    /// of how Ironforge stops being Dun Morogh.
    #[test]
    fn a_row_with_no_area_defers_to_the_terrain() {
        let areas = areas();
        assert_eq!(areas.lookup(53, 2, 1).expect("row").area_or(87), 87);
        assert_eq!(areas.lookup(208, 0, 3610).expect("row").area_or(1), 1537);
    }

    #[test]
    fn a_building_nothing_carries_answers_nothing() {
        assert_eq!(areas().lookup(9999, 0, 1), None);
    }

    /// **A key stated twice keeps the row that says something.** Ten keys in
    /// the shipped file are, and three of those disagree — see [`WmoAreas::parse`].
    /// Checked in both orders, because the whole point is that the record order
    /// must not decide it.
    #[test]
    fn a_duplicate_key_keeps_the_row_that_says_something() {
        let named = row(556, 1, WHOLE_BUILDING, 2958, 1);
        let blank = row(556, 1, WHOLE_BUILDING, 0, 0);
        for pair in [[named.clone(), blank.clone()], [blank, named]] {
            let areas = WmoAreas::parse(&dbc(&pair, 20, NAMES)).expect("the table parses");
            let found = areas.lookup(556, 1, WHOLE_BUILDING).expect("the row");
            assert_eq!(found.name, "City of Ironforge");
            assert_eq!(found.area, 2958);
            assert_eq!(areas.count(), 1, "one key, whichever order it arrived in");
        }
    }

    #[test]
    fn the_counts_split_the_named_from_the_placed() {
        let areas = areas();
        assert_eq!(areas.count(), 4);
        assert_eq!(areas.named_count(), 4);
        assert_eq!(areas.with_area_count(), 3);
    }
}
