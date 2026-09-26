//! **The flags on the world map** — `AreaPOI.dbc`, and the one entry that is not
//! in it.
//!
//! `WorldMapFrame.lua` draws this layer itself and asks the client two
//! questions for it:
//!
//! ```text
//! GetNumMapLandmarks()        how many
//! GetMapLandmarkInfo(i)       name, description, icon, x, y
//! ```
//!
//! Both read one landmark array, and **one function fills it** (re-run
//! whenever the open map changes):
//! every row of this table projected onto the view, then the single
//! `SMSG_GOSSIP_POI` the session is holding.
//!
//! ## The table
//!
//! 339 records of 29 fields:
//!
//! ```text
//!   [0] id           [1] importance   [2] icon      the sheet cell — see below
//!   [4..7] x, y, z   [7] map          [8] flags
//!   [9] areaId       -> AreaTable, and the gate
//!   [10..19] name    [19..28] description          eight locales apiece
//! ```
//!
//! `[9]` is an **`AreaTable` id and not a faction**, which is the one thing here
//! that reads plausibly wrong: the client looks it up in `AreaTable` and then
//! tests `[row + 0xc]` — field 3, the *areaBit* — against
//! `PLAYER_EXPLORED_ZONES`, which is a bit test over the
//! block at `PLAYER_FIELD_EXPLORED_ZONES_1`. So **a landmark appears when its
//! area has been explored**, and the gate is skipped entirely for a row
//! whose `[row + 0x28]` — field 10, the exploration level — is negative, which
//! is most of them.
//!
//! ## The icon is field 2, except on a continent
//!
//! The client reads the row's flags byte and takes field 2 when its bit 7 is
//! set. Otherwise the *view* decides: a continent-level map answers a flat
//! **15** and a zone map answers field 2. Both indices are cells of
//! `Interface\Minimap\POIIcons.blp`, which is 128x128 and an 8x8 grid of 16x16
//! icons — the same grid `WORLDMAP_POI_TEXTURE_WIDTH = 128` and
//! `NUM_WORLDMAP_POI_COLUMNS = 8` state in `WorldMapFrame.lua`, which cuts the
//! cell itself.
//!
//! ## …and the coordinates are a fraction of the open parchment
//!
//! **This was read backwards once and every flag in the game was drawn off the
//! map for it**, which is worth the paragraph: the claim was that
//! `GetMapLandmarkInfo` answers in `WorldMapButton` units because the file
//! passes them straight to `SetPoint`. The file does not. `WorldMapFrame.lua`
//! line 83 is
//!
//! ```lua
//! x = x * WorldMapButton:GetWidth();
//! y = -y * WorldMapButton:GetHeight();
//! worldMapPOI:SetPoint("CENTER", "WorldMapButton", "TOPLEFT", x, y );
//! ```
//!
//! — the same two lines it scales the corpse by (444) and a party dot by (382),
//! sign included. A projection applied on this side is therefore applied
//! **twice**, which puts a flag some 1002x further right and 668x above the
//! parchment's top edge: off screen at every zoom, on every map, with no error
//! anywhere. Reported as "the corpse marker works and a guard's directions put
//! up nothing".
//!
//! The client agrees: `GetMapLandmarkInfo` returns the landmark's position
//! exactly as the builder's projection wrote it, and that projection is the
//! same one `GetCorpseMapPosition` uses. One function, one unit, and the
//! corpse's is a fraction.
//!
//! ## One leg of the array is dead in 1.12
//!
//! A landmark record has a field that marks it a `TaxiNodes` row drawn with
//! icon 6, and **nothing in the client sets it** — the only two writers (for a
//! table row, and for the gossip point) both leave it zero. Flight points are not landmarks in this build; recorded because the
//! reader tests it and a reader written from the reader alone would look for
//! them.

use crate::tables::area::Areas;
use crate::tables::dbc::Dbc;
use crate::tables::worldmap::{MapView, WorldMap};
use crate::AssetError;

mod fields {
    pub const IMPORTANCE: usize = 1;
    pub const ICON: usize = 2;
    pub const X: usize = 4;
    pub const Y: usize = 5;
    pub const MAP: usize = 7;
    pub const FLAGS: usize = 8;
    pub const AREA: usize = 9;
    pub const NAME: usize = 10;
    pub const DESCRIPTION: usize = 19;
}

/// `AreaPOI.dbc` field 8, bit 7 — **this row's icon is its own whatever map is
/// open**.
pub const ICON_IS_ABSOLUTE: u32 = 1 << 7;

/// **Which zoom levels a row is drawn at**, as the client gates them — three
/// bits of field 8, one per parchment, tested before the row is projected at
/// all:
///
/// ```text
/// a zone map       requires flags & 0x04
/// a continent map  requires flags & 0x08
/// the cosmic map   requires flags & 0x10
/// ```
///
/// **This is the rule that keeps a continent from being covered in flags.**
/// Eight rows land inside Elwynn Forest — Goldshire, Northshire Abbey,
/// Stormwind, Mirror Lake, Echo Ridge Mine, Northshire Vineyards, Westbrook
/// Garrison, Jasperlode Mine — and all eight carry `0x04`, so all eight belong
/// on the zone map and the reference draws them too. Only *Stormwind* carries
/// `0x08`, so on the Eastern Kingdoms parchment it is the one of the eight that
/// survives. Without the gate the continent draws every row in the table at the
/// flat icon 15, which is a plausible picture and the wrong one.
///
/// **The gossip point is not gated by any of them.** It goes straight to the
/// projection and never through the gate — so a guard's directions show on
/// whichever parchment they land
/// on, whatever the packet's flags word says.
pub const ON_ZONE_MAP: u32 = 1 << 2;
pub const ON_CONTINENT_MAP: u32 = 1 << 3;
pub const ON_COSMIC_MAP: u32 = 1 << 4;

/// …and what a row without that bit answers on a continent map.
///
/// A cell of `POIIcons.blp` like any other; the reference falls back to it
/// because a continent is drawn small and most of the specific icons are
/// unreadable at that size.
pub const CONTINENT_ICON: u32 = 15;

/// **How near zero counts as "off the open map"** — `2^-22`, and the test is
/// `|u| < e && |v| < e` rather than an
/// equality against zero.
///
/// It matters only for a point that projects onto the parchment's very top-left
/// texel, which the reference declines to draw and so does this.
const OFF_MAP: f32 = 2.384_185_8e-7;

/// One row, reduced to what the two reads need.
#[derive(Debug, Clone, PartialEq)]
pub struct Poi {
    pub id: u32,
    /// Field 2 — a cell of `POIIcons.blp`, overridden on a continent map unless
    /// [`ICON_IS_ABSOLUTE`] is set. See the module comment.
    pub icon: u32,
    /// Field 1. Read and unused: nothing in `WorldMapFrame.lua` sorts on it and
    /// the fill walks the table in record order.
    pub importance: u32,
    pub flags: u32,
    /// World yards, and the map they are on.
    pub position: (f32, f32),
    pub map: u32,
    /// The `AreaTable` row that gates it — see the module comment. Measured
    /// over the shipped table: **171 rows name one, 98 are `-1` and 70 are
    /// `0`**, and both of the latter two are "always". The gate then drops out
    /// again for a named area whose exploration level is negative, so the count
    /// that actually gates is the survey's rather than this one.
    pub area: i32,
    pub name: String,
    pub description: String,
}

/// `AreaPOI.dbc`, in record order — which is the order the fill walks it and
/// therefore the order `GetMapLandmarkInfo`'s index runs in.
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

/// One entry of the array the two reads walk — a table row *or* the gossip
/// point, already projected.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub name: String,
    pub description: String,
    /// The cell of `POIIcons.blp`, after the continent substitution.
    pub icon: u32,
    /// **A fraction of the open parchment**, `0..1` from its top left and `y`
    /// growing *downward* — the same units `GetPlayerMapPosition` and
    /// `GetCorpseMapPosition` answer in, because the file scales all three the
    /// same way. See the module comment, which carries what happens when this
    /// is scaled here as well.
    pub at: (f32, f32),
    /// **The one the server sent**, which the file tooltips no differently but
    /// which is worth telling apart: it is the flag a guard's directions put up
    /// and it is replaced rather than accumulated.
    pub from_gossip: bool,
}

/// **The gossip point**, as `SMSG_GOSSIP_POI` states it.
///
/// The reference keeps exactly one, in a synthetic `AreaPOI` row — which is
/// why `GetMapLandmarkInfo` reads its name and icon the same way as a table
/// row's, and why a second one replaces the first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GossipPoi {
    pub flags: u32,
    /// World yards. The packet carries no map, so it is taken to be the one the
    /// character is standing on — which is what the server means by it.
    pub position: (f32, f32),
    pub icon: u32,
    pub data: u32,
    pub name: String,
}

/// **Every landmark on the open map**, in the order the client fills them.
///
/// `explored` answers whether one `AreaTable` bit is set; `None` skips the
/// gate, which is what a caller with no character has to do and what the CLI's
/// survey does.
///
/// `map` is the map the *character* is on, for the gossip point — the packet
/// carries no map of its own.
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
        // **Before the projection, as the reference tests it** — the row is
        // refused on its flags and never reaches the arithmetic.
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
    // The gossip point is appended after the whole table, so it is
    // always the last index — and it is dropped, like a row, when it does not
    // land on the open map.
    if let Some(gossip) = gossip {
        if let Some(at) = place(world_map, view, gossip_map, gossip.position) {
            out.push(Landmark {
                name: gossip.name.clone(),
                description: String::new(),
                // **Never substituted**: the client tests the gossip record
                // before it reaches the continent branch, so a gossip
                // point keeps the icon the packet named on every map.
                icon: gossip.icon,
                at,
                from_gossip: true,
            });
        }
    }
    out
}

/// The row's own cell, or **15** on a continent unless the row
/// insists.
fn icon_for(flags: u32, icon: u32, continent: bool) -> u32 {
    if flags & ICON_IS_ABSOLUTE != 0 || !continent {
        icon
    } else {
        CONTINENT_ICON
    }
}

/// The client's three tests: is this row drawn at this zoom at all? See
/// [`ON_ZONE_MAP`], which carries the bits and the worked example.
fn shown_on(view: MapView, flags: u32) -> bool {
    let wanted = match view {
        MapView::Zone(_, _) => ON_ZONE_MAP,
        MapView::Continent(_) => ON_CONTINENT_MAP,
        MapView::Cosmic => ON_COSMIC_MAP,
    };
    flags & wanted != 0
}

/// The area gate, and it is skipped by most rows.
fn revealed(poi: &Poi, areas: Option<&Areas>, explored: Option<&dyn Fn(u32) -> bool>) -> bool {
    if poi.area <= 0 {
        return true;
    }
    let (Some(areas), Some(explored)) = (areas, explored) else {
        // **Shown rather than hidden** with no character to ask, which is the
        // direction that cannot lose a landmark: the survey wants the whole
        // table and a client with no exploration mask yet has not lost one, it
        // has not been told.
        return true;
    };
    let Some(area) = areas.get(poi.area as u32) else {
        return true;
    };
    // A negative exploration level is no gate at all.
    if area.explore_level < 0 {
        return true;
    }
    explored(area.explore_bit)
}

/// World yards to a fraction of the open parchment, or `None` for a point off
/// it — the builder's own test, epsilon and all.
///
/// **Nothing is scaled here.** The file does it, for a landmark exactly as for
/// the corpse and the party dots; see the module comment, which is about the
/// round this was wrong.
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

    /// **A landmark is in the same units as the corpse and the party dots**,
    /// which is a fraction of the open parchment — the whole of the bug that
    /// drew every flag off the map. See the module comment.
    ///
    /// Checked against [`WorldMap::position`] itself rather than against a
    /// remembered number: the property is that `place` passes its projection
    /// through, and a scale factor reintroduced here would fail it whatever the
    /// factor was.
    #[test]
    fn a_landmark_is_a_fraction_of_the_parchment_like_the_corpse() {
        let world_map = crate::tables::worldmap::tests::built();
        let view = MapView::Zone(0, 0);
        // Inside the fixture's first zone rectangle — x within its
        // top/bottom pair and y within its left/right one, both axes running
        // backwards as this game's coordinates do — taken through the same call
        // `GetCorpseMapPosition` goes through.
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

    /// **Each parchment has its own flag bit**, and a row with none of them is
    /// drawn nowhere — the rule that keeps a continent map from being covered.
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

        // The shipped rows this was checked against: Goldshire is 5 and shows
        // on its zone and not on the continent; Stormwind is 29 and shows on
        // both. See [`ON_ZONE_MAP`].
        assert!(shown_on(zone, 5) && !shown_on(MapView::Continent(0), 5));
        assert!(shown_on(zone, 29) && shown_on(MapView::Continent(0), 29));
    }

    /// **The icon is the row's own on a zone map and 15 on a continent**, and a
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
