//! The landmark flags on the world map: `AreaPOI.dbc`, and the one entry
//! that is not in it.
//!
//! `WorldMapFrame.lua` draws this layer itself and asks two questions for it:
//!
//! ```text
//! GetNumMapLandmarks()        how many
//! GetMapLandmarkInfo(i)       name, description, icon, x, y
//! ```
//!
//! Both read one landmark list, which is filled again whenever the open map
//! changes: every row of this table projected onto the view, then the single
//! `SMSG_GOSSIP_POI` the session is holding.
//!
//! ## The table
//!
//! 339 records of 29 fields:
//!
//! ```text
//!   [0] id           [1] importance   [2] icon      the sheet cell; see below
//!   [4..7] x, y, z   [7] map          [8] flags
//!   [9] areaId       -> AreaTable, and the gate
//!   [10..19] name    [19..28] description          eight locales each
//! ```
//!
//! Field 9 is an `AreaTable` id, not a faction. The client looks it up in
//! `AreaTable` and tests that row's field 3, the `areaBit`, against
//! `PLAYER_EXPLORED_ZONES`, so a landmark appears once its area has been
//! explored. The test is skipped for a row whose area's field 10, the
//! exploration level, is negative, which is most of them.
//!
//! ## The icon is field 2, except on a continent
//!
//! The client takes field 2 when bit 7 of the row's flags is set. Otherwise the
//! view decides: a continent-level map answers 15 and a zone map answers field
//! 2. Both are cells of `Interface\Minimap\POIIcons.blp`, a 128x128 sheet of
//! 8x8 icons of 16x16, the grid `WORLDMAP_POI_TEXTURE_WIDTH = 128` and
//! `NUM_WORLDMAP_POI_COLUMNS = 8` state in `WorldMapFrame.lua`, which cuts the
//! cell itself.
//!
//! ## The coordinates are a fraction of the open map
//!
//! `GetMapLandmarkInfo` answers a fraction, and `WorldMapFrame.lua` line 83
//! scales it:
//!
//! ```lua
//! x = x * WorldMapButton:GetWidth();
//! y = -y * WorldMapButton:GetHeight();
//! worldMapPOI:SetPoint("CENTER", "WorldMapButton", "TOPLEFT", x, y );
//! ```
//!
//! These are the lines that scale the corpse (444) and a party dot (382), sign
//! included. A projection scaled on this side as well is scaled twice, and
//! puts a flag about 1002 widths to the right and 668 heights above the map:
//! off screen at every zoom, with no error. This client once did that, and a
//! guard's directions showed nothing while the corpse marker worked.
//!
//! The client's `GetMapLandmarkInfo` returns the position as the projection
//! wrote it, and that projection is the one `GetCorpseMapPosition` uses.
//!
//! ## A field of the landmark record that 1.12 never sets
//!
//! A landmark record has a field that marks it a `TaxiNodes` row drawn with
//! icon 6, and nothing in the client sets it: the two writers, for a table row
//! and for the gossip point, both leave it zero. Flight points are not
//! landmarks in this build. It is recorded because the reader tests it.

use crate::tables::area::Areas;
use crate::tables::dbc::Dbc;
use crate::tables::worldmap::{MapView, WorldMap};
use crate::AssetError;

mod fields {
    pub(crate) const IMPORTANCE: usize = 1;
    pub const ICON: usize = 2;
    pub const X: usize = 4;
    pub const Y: usize = 5;
    pub const MAP: usize = 7;
    pub const FLAGS: usize = 8;
    pub const AREA: usize = 9;
    pub const NAME: usize = 10;
    pub const DESCRIPTION: usize = 19;
}

/// `AreaPOI.dbc` field 8, bit 7 — this row's icon is its own whatever map is
/// open.
pub(crate) const ICON_IS_ABSOLUTE: u32 = 1 << 7;

/// Which zoom levels a row is drawn at, as the client tests them: three bits
/// of field 8, one per kind of map, tested before the row is projected:
///
/// ```text
/// a zone map       requires flags & 0x04   (and a single map)
/// a continent map  requires flags & 0x08
/// the cosmic map   requires flags & 0x10
/// ```
///
/// Eight rows land inside Elwynn Forest (Goldshire, Northshire Abbey,
/// Stormwind, Mirror Lake, Echo Ridge Mine, Northshire Vineyards, Westbrook
/// Garrison, Jasperlode Mine) and all eight carry `0x04`, so all eight are
/// drawn on the zone map, as in the 1.12.1 client. Only Stormwind carries
/// `0x08`, so it is the only one of the eight on the Eastern Kingdoms map.
/// Without the test a continent map draws every row in the table.
///
/// The gossip point is not tested. It goes straight to the projection, so a
/// guard's directions show on whichever map they land on, whatever the
/// packet's flags word says.
pub(crate) const ON_ZONE_MAP: u32 = 1 << 2;
pub(crate) const ON_CONTINENT_MAP: u32 = 1 << 3;
pub(crate) const ON_COSMIC_MAP: u32 = 1 << 4;

/// The icon a row without [`ICON_IS_ABSOLUTE`] answers on a continent map: a
/// cell of `POIIcons.blp` like any other.
pub(crate) const CONTINENT_ICON: u32 = 15;

/// How near zero counts as off the open map: `2^-22`, tested as
/// `|u| < e && |v| < e` rather than as equality with zero.
///
/// It matters only for a point that projects onto the map's top-left texel,
/// which the 1.12.1 client does not draw, and neither does this.
const OFF_MAP: f32 = 2.384_185_8e-7;

/// One row, reduced to what the two reads need.
#[derive(Debug, Clone, PartialEq)]
pub struct Poi {
    pub id: u32,
    /// Field 2: a cell of `POIIcons.blp`, overridden on a continent map unless
    /// [`ICON_IS_ABSOLUTE`] is set. See the module comment.
    pub icon: u32,
    /// Field 1. Read and unused: nothing in `WorldMapFrame.lua` sorts on it and
    /// the fill walks the table in record order.
    pub(crate) importance: u32,
    pub flags: u32,
    /// World yards, and the map they are on.
    pub position: (f32, f32),
    pub map: u32,
    /// The `AreaTable` row that gates it; see the module comment. In the
    /// shipped table 171 rows name one, 98 are `-1` and 70 are `0`, and both
    /// of the last two are always shown. A named area whose exploration level
    /// is negative is not tested either, so the number of rows that are gated
    /// is the survey's, not 171.
    pub area: i32,
    pub name: String,
    pub description: String,
}

/// `AreaPOI.dbc`, in record order: the order the list is filled in, and so the
/// order `GetMapLandmarkInfo`'s index runs in.
#[derive(Debug, Clone, Default)]
pub struct AreaPois(Vec<Poi>);

impl AreaPois {
    pub fn parse(raw: &[u8]) -> Result<AreaPois, AssetError> {
        let dbc = Dbc::parse(raw)?;
        let mut rows = Vec::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            let float = |f: usize| dbc.u32_at(record, f).map_or(0.0, f32::from_bits);
            rows.push(Poi {
                id,
                icon: dbc.u32_at(record, fields::ICON).unwrap_or(0),
                importance: dbc.u32_at(record, fields::IMPORTANCE).unwrap_or(0),
                flags: dbc.u32_at(record, fields::FLAGS).unwrap_or(0),
                position: (float(fields::X), float(fields::Y)),
                map: dbc.u32_at(record, fields::MAP).unwrap_or(0),
                area: dbc.u32_at(record, fields::AREA).unwrap_or(0) as i32,
                name: dbc.string_at(record, fields::NAME).unwrap_or_default(),
                description: dbc
                    .string_at(record, fields::DESCRIPTION)
                    .unwrap_or_default(),
            });
        }
        Ok(AreaPois(rows))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every row, for the survey.
    pub fn rows(&self) -> &[Poi] {
        &self.0
    }
}

/// One entry of the list the two reads walk: a table row or the gossip point,
/// already projected.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub name: String,
    pub description: String,
    /// The cell of `POIIcons.blp`, after the continent substitution.
    pub icon: u32,
    /// A fraction of the open map, `0..1` from its top left with `y` growing
    /// downward: the units `GetPlayerMapPosition` and `GetCorpseMapPosition`
    /// answer in, because the file scales all three the same way. See the
    /// module comment.
    pub at: (f32, f32),
    /// Whether this is the point the server sent: the flag a guard's
    /// directions put up, which a second one replaces. The file shows its
    /// tooltip as any other.
    pub from_gossip: bool,
}

/// The gossip point, as `SMSG_GOSSIP_POI` states it.
///
/// The 1.12.1 client keeps one, in a synthetic `AreaPOI` row, so
/// `GetMapLandmarkInfo` reads its name and icon as a table row's, and a second
/// one replaces the first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GossipPoi {
    pub flags: u32,
    /// World yards. The packet carries no map; the server means the map the
    /// character is standing on.
    pub position: (f32, f32),
    pub icon: u32,
    pub data: u32,
    pub name: String,
}

/// Every landmark on the open map, in the order the client fills them.
///
/// `explored` answers whether one `AreaTable` bit is set; `None` skips the
/// gate, which is what a caller with no character has to do and what the CLI's
/// survey does.
///
/// `gossip_map` is the map the character is on, for the gossip point, whose
/// packet carries no map.
pub fn landmarks(
    pois: &AreaPois,
    world_map: &WorldMap,
    areas: Option<&Areas>,
    view: MapView,
    explored: Option<&dyn Fn(u32) -> bool>,
    gossip: Option<&GossipPoi>,
    gossip_map: u32,
) -> Vec<Landmark> {
    let continent = matches!(view, MapView::Continent(_) | MapView::Cosmic);
    let mut out = Vec::new();
    for poi in &pois.0 {
        // Tested before the projection, as the 1.12.1 client tests it.
        if !shown_on(view, poi.flags) {
            continue;
        }
        let Some(at) = place(world_map, view, poi.map, poi.position) else {
            continue;
        };
        if !revealed(poi, areas, explored) {
            continue;
        }
        out.push(Landmark {
            name: poi.name.clone(),
            description: poi.description.clone(),
            icon: icon_for(poi.flags, poi.icon, continent),
            at,
            from_gossip: false,
        });
    }
    // The gossip point comes after the table, so it is always the last
    // index. Like a row, it is dropped when it does not land on the open map.
    if let Some(gossip) = gossip {
        if let Some(at) = place(world_map, view, gossip_map, gossip.position) {
            out.push(Landmark {
                name: gossip.name.clone(),
                description: String::new(),
                // Never substituted: the client tests for the gossip
                // record first, so the gossip point keeps the icon the
                // packet named on every map.
                icon: gossip.icon,
                at,
                from_gossip: true,
            });
        }
    }
    out
}

/// The row's own cell, or 15 on a continent map unless the row carries
/// [`ICON_IS_ABSOLUTE`].
fn icon_for(flags: u32, icon: u32, continent: bool) -> u32 {
    if flags & ICON_IS_ABSOLUTE != 0 || !continent {
        icon
    } else {
        CONTINENT_ICON
    }
}

/// Whether a row is drawn at this zoom at all. See [`ON_ZONE_MAP`] for the
/// bits and an example.
fn shown_on(view: MapView, flags: u32) -> bool {
    let wanted = match view {
        // The client gates a single map's rows as it gates a zone map's.
        MapView::Zone(_, _) | MapView::Single(_) => ON_ZONE_MAP,
        MapView::Continent(_) => ON_CONTINENT_MAP,
        MapView::Cosmic => ON_COSMIC_MAP,
    };
    flags & wanted != 0
}

/// The area test, which most rows skip.
fn revealed(poi: &Poi, areas: Option<&Areas>, explored: Option<&dyn Fn(u32) -> bool>) -> bool {
    if poi.area <= 0 {
        return true;
    }
    let (Some(areas), Some(explored)) = (areas, explored) else {
        // Shown when there is no character to ask: the survey wants the whole
        // table, and a client with no exploration mask yet has not been told
        // what is explored.
        return true;
    };
    let Some(area) = areas.get(poi.area as u32) else {
        return true;
    };
    // A negative exploration level is not tested.
    if area.explore_level < 0 {
        return true;
    }
    explored(area.explore_bit)
}

/// World yards to a fraction of the open map, or `None` for a point off it,
/// with the client's [`OFF_MAP`] test.
///
/// Nothing is scaled here. The file scales a landmark as it scales the corpse
/// and the party dots; see the module comment.
fn place(world_map: &WorldMap, view: MapView, map: u32, at: (f32, f32)) -> Option<(f32, f32)> {
    let (u, v) = world_map.position(view, map, at.0, at.1)?;
    if u.abs() < OFF_MAP && v.abs() < OFF_MAP {
        return None;
    }
    Some((u, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A landmark is in the units of the corpse and the party dots, a
    /// fraction of the open map. See the module comment.
    ///
    /// Checked against [`WorldMap::position`] rather than a fixed number: the
    /// property is that `place` passes the projection through, so any scale
    /// factor added here fails it.
    #[test]
    fn a_landmark_is_a_fraction_of_the_parchment_like_the_corpse() {
        let world_map = crate::tables::worldmap::tests::built();
        let view = MapView::Zone(0, 0);
        // Inside the fixture's first zone rectangle (x within its top/bottom
        // pair and y within its left/right one, both axes running backwards as
        // this game's coordinates do), taken through the call
        // `GetCorpseMapPosition` uses.
        let (map, x, y) = (1, 0.0, -4000.0);
        let projected = world_map
            .position(view, map, x, y)
            .expect("the fixture's zone covers this point");
        assert_eq!(
            place(&world_map, view, map, (x, y)),
            Some(projected),
            "the projection is handed back unscaled"
        );
        let (u, v) = projected;
        assert!(
            (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v),
            "a fraction, not frame units: {projected:?}"
        );
    }

    /// Each kind of map has its own flag bit, and a row with none of them is
    /// drawn nowhere.
    #[test]
    fn a_row_is_drawn_at_the_zooms_its_flags_name() {
        let zone = MapView::Zone(0, 0);
        assert!(shown_on(zone, ON_ZONE_MAP));
        assert!(!shown_on(zone, ON_CONTINENT_MAP | ON_COSMIC_MAP));
        assert!(shown_on(MapView::Continent(0), ON_CONTINENT_MAP));
        assert!(!shown_on(MapView::Continent(0), ON_ZONE_MAP));
        assert!(shown_on(MapView::Cosmic, ON_COSMIC_MAP));
        assert!(!shown_on(MapView::Cosmic, ON_ZONE_MAP | ON_CONTINENT_MAP));
        assert!(!shown_on(zone, 0), "a row with no zoom flags is drawn nowhere");
        // A single map takes the zone map's bit.
        assert!(shown_on(MapView::Single(401), ON_ZONE_MAP));
        assert!(!shown_on(MapView::Single(401), ON_CONTINENT_MAP | ON_COSMIC_MAP));

        // Two shipped rows: Goldshire is 5 and shows
        // on its zone and not on the continent; Stormwind is 29 and shows on
        // both. See [`ON_ZONE_MAP`].
        assert!(shown_on(zone, 5) && !shown_on(MapView::Continent(0), 5));
        assert!(shown_on(zone, 29) && shown_on(MapView::Continent(0), 29));
    }

    /// The icon is the row's own on a zone map and 15 on a continent, and a
    /// row with the flag keeps its own everywhere.
    #[test]
    fn a_continent_map_flattens_every_icon_that_does_not_insist() {
        assert_eq!(icon_for(0, 6, false), 6, "a zone map takes the row's cell");
        assert_eq!(icon_for(0, 6, true), CONTINENT_ICON);
        assert_eq!(
            icon_for(ICON_IS_ABSOLUTE, 6, true),
            6,
            "bit 7 keeps the cell on a continent too"
        );
    }
}
