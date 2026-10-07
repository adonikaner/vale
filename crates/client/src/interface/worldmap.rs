//! **Where the character is, and which parchment the map is showing.**
//!
//! Two pieces of state and neither of them comes off the wire.
//!
//! * **The area.** The server says which *map* a character is on
//!   (`SMSG_LOGIN_VERIFY_WORLD`) and nothing else about where. Everything below
//!   that is the client reading the ground under its own feet: `MCNK`'s header
//!   carries an `areaId` per 33-yard chunk, and that id indexes
//!   `AreaTable.dbc`. So `GetZoneText()` is a terrain lookup, and every zone
//!   name in the interface — the minimap's title, the world map's, the chat
//!   line's `/say` scope text — is downstream of this one number.
//! * **…and the *building*, which overrules the ground.** The dirt under
//!   Ironforge says Dun Morogh in every one of `Azeroth_33_41`'s chunks and the
//!   dirt under the Lion's Pride Inn says Goldshire, which is what both of those
//!   places used to be called here. A WMO states its own area
//!   (`WMOAreaTable.dbc`, keyed by the building, its name set and the group the
//!   character is standing in — [`vale_assets::tables::wmoarea`]) and that is the only
//!   thing in any file that knows a city is a zone. It also carries the
//!   *sub-area's* name outright: "The Great Forge" and "Lion's Pride Inn" are
//!   rows there and appear nowhere in `AreaTable`.
//!
//!   **It is asked of the buildings that are spawned**, so a teleport into a
//!   city reads the dirt's answer for the second or two `stormwind.wmo` takes
//!   to arrive, and then corrects itself — the poll is every frame and the
//!   events are raised on the change, so nothing needs telling twice. That is
//!   the same window the floor takes to exist, and it is stated rather than
//!   worked around: the alternative is a second copy of the placement list that
//!   the streamer does not own.
//! * **The view.** Which of the cosmic map, a continent or a zone the world map
//!   panel is drawn on. The client keeps it as two integers
//!   ([`vale_assets::tables::worldmap::MapView`]) and every map function in
//!   `WorldMapFrame.lua` is a read of them.
//!
//! ## Why the area is polled rather than pushed
//!
//! There is nothing to push it. The mover walks the character locally
//! (`world::motion`), so no packet marks the moment a boundary is crossed — the
//! real client polls too, which is why 1.12 has a `ZONE_CHANGED` at all rather
//! than a `SMSG_ZONE_UPDATE`. The poll is one `HashMap` hit against a tile the
//! terrain cache already holds for the mover, and the events are raised **on a
//! change** the way [`super::vitals`]' are.
//!
//! ## The three names, and which one is which
//!
//! ```text
//! ZONE_CHANGED_NEW_AREA   the zone changed        Elwynn Forest -> Westfall
//! ZONE_CHANGED            the sub-area changed    Northshire Valley -> Northshire Abbey
//! MINIMAP_ZONE_CHANGED    either of the above     what the minimap's title redraws on
//! ```
//!
//! `ZONE_CHANGED_INDOORS` is the fourth and is **deliberately not raised**: it
//! marks entering a building, which this client decides geometrically against a
//! WMO's group boxes (`render::wmos`) and not from any table. Raising it off an
//! area change would be a name doing something other than what it says.

use vale_assets::tables::worldmap::MapView;
use bevy::prelude::*;

use super::events::{MinimapZoneChanged, WorldMapUpdate, ZoneChanged, ZoneChangedNewArea};
use crate::lua::panels::worldmap::MapRequest;
use crate::world::session::{Session, WorldStatus};

/// The client's own two pieces of map state — see the module comment.
#[derive(Resource)]
pub struct WorldMapState {
    /// Which parchment the panel is on.
    pub view: MapView,
    /// The `AreaTable` id under the character, or 0 before the first poll and
    /// for a chunk that names no area. **0 is a real answer**, not an absence:
    /// the shipped tiles carry chunks with no area and the client shows nothing
    /// for them.
    pub area: u32,
    /// …and the enclosing zone's id, resolved once here rather than at every
    /// read.
    pub zone: u32,
    /// **Which map the four fields above are about.**
    ///
    /// Everything in this resource is a statement about a place, and a teleport
    /// makes every one of them false at once — but [`track_area`] answers
    /// "nothing here names an area yet" by leaving what it had, which is right
    /// *within* a map and wrong across one. So a character who walked into the
    /// Stockade kept Elwynn Forest on the minimap, permanently, because nothing
    /// in the instance would ever overwrite it: that is the report.
    ///
    /// 0 is "no map yet", and it is safe as a sentinel rather than lucky — map 0
    /// is Eastern Kingdoms, so the worst a collision could do is skip one reset
    /// on a login that lands there, and the first poll clears it anyway because
    /// `aimed` is false.
    map: u32,
    /// **The sub-area's name when a *building* is what names it**, and empty
    /// otherwise — see the module comment's third bullet.
    ///
    /// A string rather than another id because there is no id: "The Great
    /// Forge" and "Lion's Pride Inn" are `WMOAreaTable` rows and appear nowhere
    /// in `AreaTable`. When this is empty the sub-area is [`Self::area`]'s own
    /// name, which is the open-country case and most of the world.
    pub sub_name: String,
    /// **Whether the character is inside a building at all** — the same walk
    /// [`Self::sub_name`] comes out of, asked one question earlier.
    ///
    /// It is a different answer from `sub_name` being non-empty, and that is the
    /// whole reason it exists: 21,115 `WMOAreaTable` rows over the game's
    /// thousands of models is a few per building, so standing in an unnamed room
    /// is common and reads as open country. Nothing about the *zone* cares, and
    /// the minimap does not read it: [`super::minimap`] decides indoors from the
    /// floor under the feet, as [`Self::outdoors`] does.
    pub indoors: bool,
    /// **Whether the character is under open sky for the rules that care** —
    /// a mount, and every outdoor-only or indoor-only spell.
    ///
    /// **Not `!indoors`, and the difference was a bug report.** `indoors` is
    /// "inside some group's box", which is true on every street of Stormwind,
    /// because the city is one building whose districts are exterior groups;
    /// so the cast check refused every mount in the city. This is the server's
    /// own test instead — the group the floor underfoot belongs to, read for
    /// `MOGP` `0x8000` — see [`vale_assets::world::wmo::outdoors_at`], which
    /// states the rule and its two sources.
    pub outdoors: bool,
    /// **What this place sounds like** — the ambience, the zone music and the
    /// intro fanfare, already resolved through all three levels: the building's
    /// own columns, then the area's row, then the zone's, **a column at a
    /// time**. See [`track_area`], which is the one place that decides, and
    /// [`vale_assets::tables::sound::AreaSounds::or`] for why the fallback is per
    /// column rather than per row.
    ///
    /// It lives here rather than in `sound/` because the building is here: the
    /// `Interior` walk that says which room the character is in is this
    /// module's, and two copies of it is two places for the answer to drift.
    pub sounds: vale_assets::tables::sound::AreaSounds,
    /// Whether the view has ever been aimed at the character. `SetMapToCurrentZone`
    /// is the interface's own call and this is what makes the *first* one, at a
    /// login, land somewhere rather than leaving the cosmic map showing.
    aimed: bool,
    /// **How many places the character had explored at the last check**, and the
    /// whole of how the map learns it has a new picture on it.
    ///
    /// `PLAYER_EXPLORED_ZONES` is an update field with no event of its own, and
    /// the interface re-reads the overlays only on `WORLD_MAP_UPDATE` — so
    /// without this a zone map fills in silently and does not show it until
    /// something else moves the view. That was the "does not update until relog"
    /// half of the report.
    ///
    /// A **count** rather than the mask, because a bit is only ever set: the
    /// count moving is the same statement as the mask moving, in eight bytes
    /// instead of two hundred and fifty-six. `u32::MAX` means "not read yet", so
    /// the first reading is not itself a change.
    explored: u32,
}

impl Default for WorldMapState {
    fn default() -> Self {
        WorldMapState {
            view: MapView::default(),
            area: 0,
            zone: 0,
            map: 0,
            sub_name: String::new(),
            indoors: false,
            outdoors: true,
            sounds: vale_assets::tables::sound::AreaSounds::default(),
            aimed: false,
            // **Not zero** — see [`WorldMapState::explored`]. A fresh character
            // really has explored nothing, so zero would be a reading and the
            // first real one would look like a change.
            explored: u32::MAX,
        }
    }
}

/// **A place was discovered** — see [`crate::interface::action`], which composes the
/// name, and [`announce_discovery`], which says it.
///
/// A message rather than a direct chat write because the composition needs
/// `AreaTable` and the *saying* needs `GlobalStrings.lua`, and those are two
/// different resources in two different systems.
#[derive(Message, Debug, Clone)]
pub struct Discovered {
    pub name: String,
    pub experience: u32,
}

/// **The one place the server can name on the world map** —
/// `SMSG_GOSSIP_POI`, forwarded by [`crate::world::incoming`].
///
/// A message rather than a write straight into the resource for the reason
/// every other packet's answer is one: the parse runs on the session thread and
/// the state is a Bevy resource, and there is exactly one place the two meet.
#[derive(Message, Debug, Clone)]
pub struct PoiAnswer {
    pub poi: vale_assets::tables::areapoi::GossipPoi,
}

/// **The flag a guard's directions put up**, and the map it was named on.
///
/// One, replacing whatever was there — which is the reference's own shape: it
/// keeps a single synthetic `AreaPOI` row and overwrites it, so a second set of
/// directions moves the flag rather than adding one. See
/// [`vale_assets::tables::areapoi`].
///
/// **The map is remembered here because the packet does not carry one.** The
/// reference projects the point against whichever map is open, from the
/// character's own; a flag named in Stormwind and then carried to Kalimdor
/// would otherwise be drawn at the same yards on the wrong continent.
#[derive(Resource, Default, Debug, Clone)]
pub struct MapLandmarks {
    pub gossip: Option<vale_assets::tables::areapoi::GossipPoi>,
    /// The map the character was standing on when it arrived.
    pub map: u32,
}

/// Take the server's point, and stamp it with the map it was named on.
/// **…and tell the map**, which is the half this owed and did not have.
///
/// Reported as "the first set of directions shows and the next does not, until
/// you zoom out of the zone and back into it". That is exactly what the wiring
/// did: `WorldMapFrame`'s `OnShow` runs `SetMapToCurrentZone()` and **never
/// calls `WorldMapFrame_Update()` itself**, and `apply_requests` raises
/// `WORLD_MAP_UPDATE` only when the *view* actually moves. So the first flag
/// arrived while the view was still Cosmic, the open changed it to the zone, and
/// the rebuild that followed picked the flag up by luck. Every flag after that
/// found the view already on the zone, raised nothing, and left the old list on
/// the parchment — and zooming out and back in is two view changes, which is why
/// that worked.
///
/// The list this read answers has changed, so anything showing it must re-read:
/// the same event a view change raises, for the same reason.
fn take_poi(
    mut answers: MessageReader<PoiAnswer>,
    status: Res<crate::world::session::WorldStatus>,
    mut landmarks: ResMut<MapLandmarks>,
    mut map_update: MessageWriter<WorldMapUpdate>,
) {
    for answer in answers.read() {
        // Said once, because nothing on screen says the packet came: the flag
        // is only visible with the map open on the right parchment, and a
        // report of "no flag" needs to be told apart from "no packet".
        info!(
            "map flag: {:?} at ({:.1}, {:.1}) icon {} on map {}",
            answer.poi.name, answer.poi.position.0, answer.poi.position.1, answer.poi.icon, status.map_id
        );
        landmarks.gossip = Some(answer.poi.clone());
        landmarks.map = status.map_id;
        map_update.write(WorldMapUpdate);
    }
}

pub struct WorldMapPlugin;

impl Plugin for WorldMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldMapState>()
            .init_resource::<MapLandmarks>()
            .add_message::<Discovered>()
            .add_message::<PoiAnswer>()
            .add_systems(
            Update,
            // **The requests before the poll**, so that a `SetMapToCurrentZone`
            // made in a handler this frame is applied against the place the
            // poll is about to confirm rather than a frame behind it.
            (
                apply_requests,
                take_poi,
                track_area,
                track_exploration,
                announce_discovery,
                leave_world,
            )
                .chain()
                .in_set(super::GameSet),
        );
    }
}

/// Apply what the interface asked of the view — see [`crate::lua::panels::worldmap`],
/// which is where the requests are recorded.
fn apply_requests(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    mut state: ResMut<WorldMapState>,
    mut map_update: MessageWriter<WorldMapUpdate>,
) {
    let Some(mut host) = host else { return };
    let requests = host.take_map_requests();
    if requests.is_empty() {
        return;
    }
    let map = session.active.as_ref().map_or(0, |active| active.map_id);
    for request in requests {
        match request {
            MapRequest::CurrentZone => {
                let zone = state.zone;
                aim_at_current_zone(&assets, &mut state, map, zone);
            }
            // **A click on the cosmic map zooms into the continent under it, and
            // one on a continent into the zone** — the level below whatever is
            // showing, which is the whole of what `ProcessMapClick` does. A
            // click on a zone map has nowhere to go.
            MapRequest::Click(u, v) => state.view = clicked(&assets, state.view, u, v),
            other => {
                let zones = |continent: usize| zone_count(&assets, continent);
                state.view = crate::lua::panels::worldmap::apply(state.view, other, zones);
            }
        }
    }
    // **Every request rebuilds, not only one that moves the view** — which is
    // the reference's own behaviour: `SetMapToCurrentZone` and `SetMapZoom`
    // both store the two indices and then refill the landmark array
    // **unconditionally**, with no comparison against what was showing.
    //
    // The change test that used to be here is why a guard's second set of
    // directions never appeared. `WorldMapFrame`'s `OnShow` runs
    // `SetMapToCurrentZone()` and nothing else rebuilds; `WorldMapFrame_OnEvent`
    // acts on `WORLD_MAP_UPDATE` only `if ( this:IsVisible() )`, so an event
    // raised while the map is shut is lost — and you talk to a guard with the
    // map shut. Opening it then found the view already on the zone, raised
    // nothing, and drew the list from before the conversation. Zooming out and
    // back in is two view changes, which is exactly why that worked.
    map_update.write(WorldMapUpdate);
}

/// How many zones a continent has, for [`crate::lua::panels::worldmap::apply`]'s clamp.
fn zone_count(assets: &crate::assets::GameAssets, continent: usize) -> usize {
    assets
        .display_tables()
        .ok()
        .and_then(|tables| Some(tables.world_map()?.continent(continent)?.zones.len()))
        .unwrap_or(0)
}

/// **One level in, from wherever the click landed.**
///
/// Needs the tables rather than only the view, which is why it is here and not
/// beside [`crate::lua::panels::worldmap::apply`]: the cosmic map's continents and a
/// continent's zones are both rectangles read out of `WorldMapArea`.
fn clicked(
    assets: &crate::assets::GameAssets,
    view: MapView,
    u: f32,
    v: f32,
) -> MapView {
    let Ok(tables) = assets.display_tables() else {
        return view;
    };
    let Some(world_map) = tables.world_map() else {
        return view;
    };
    match view {
        // The same test the highlight makes, which is one rule in the
        // reference and is one here too.
        MapView::Cosmic => world_map
            .continent_at(u, v)
            .map_or(view, MapView::Continent),
        MapView::Continent(continent) => world_map
            .zone_at(continent, u, v)
            .map_or(view, |zone| MapView::Zone(continent, zone)),
        MapView::Zone(_, _) => view,
    }
}

/// **How far above the character's feet the building test is taken**, in yards.
///
/// The same yard and for the same reason as `world::entities`' own
/// `INDOOR_PROBE`: a group's box is its own geometry,
/// so its top face is the outside of the roof and a character standing on one
/// would otherwise read as being in the room below.
const AREA_PROBE: f32 = 1.0;

/// Poll the ground for an area id and raise the three names on a change.
///
/// **…and the building over the top of it**, which is the more specific
/// statement and wins where it makes one — see the module comment.
pub(super) fn track_area(
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    status: Res<WorldStatus>,
    mut state: ResMut<WorldMapState>,
    // The placed buildings of the loaded 3x3, exactly as
    // `world::entities::light_entities` takes them: each rejects on its own box
    // before walking its groups, so a city's three hundred is only walked by
    // somebody standing in the city.
    interiors: Query<&crate::render::wmos::Interior>,
    // …and the buildings' solid triangles, for the one question the boxes
    // cannot answer: which group's floor the character is standing on. See
    // [`WorldMapState::outdoors`].
    solids: Res<crate::world::session::Solids>,
    mut new_area: MessageWriter<ZoneChangedNewArea>,
    mut changed: MessageWriter<ZoneChanged>,
    mut minimap: MessageWriter<MinimapZoneChanged>,
    mut map_update: MessageWriter<WorldMapUpdate>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    if !status.in_world {
        return;
    }
    let position = status.position;
    let map = active.map_id;
    // **A teleport makes every field here false at once**, and until this
    // existed nothing said so: the resolution below answers "not known yet" by
    // leaving what it had, which is right while walking around one map and is
    // the whole of "the minimap says whatever the last zone was" across two.
    // Clearing rather than holding is also what re-aims the world map, since a
    // fresh state has `aimed` false. See [`WorldMapState::map`].
    if state.map != map {
        *state = WorldMapState { map, ..WorldMapState::default() };
    }
    // The mover's own terrain, so the tile is already resident and this costs a
    // cache hit rather than a parse. See `world::session`.
    //
    // **`Option`, not a precondition, and that is the whole of one report.**
    // Twenty of the game's forty-three maps have no ADT at all, so on every one
    // of them this is `None` for ever — and it used to return here, which meant
    // the building below was never even looked at. The consequence is the whole
    // of "you are in an empty zone with no minimap name": no area, therefore no
    // zone, no subzone, no ambience, no zone music and nothing on the minimap,
    // permanently, inside every dungeon in the game. The ground is a
    // *fallback* for what the building says, and on those maps the building is
    // the only thing that says anything at all.
    let ground = active.terrain_area(map, position.x, position.y);
    let tables = assets.display_tables().ok();
    let probe = [position.x, position.y, position.z + AREA_PROBE];
    // **Open sky, by the server's rule** — asked every frame and written
    // without a change test of its own, because nothing raises an event on it:
    // the cast check reads it when a button is pressed. One cell of one hull's
    // grid per building under the point, which is a few dozen triangles.
    let probe_z = position.z + AREA_PROBE;
    let outdoors = vale_assets::world::wmo::outdoors_at(
        solids.0.building_floor(map, position.x, position.y, probe_z),
        active.terrain_height(map, position.x, position.y),
        probe_z,
    );
    // Written only on a change, so the resource is not marked changed every
    // frame for a reader that might one day filter on it.
    if state.outdoors != outdoors {
        state.outdoors = outdoors;
    }
    let (inside, building_sounds, indoors) = tables
        .as_ref()
        .and_then(|tables| Some(building_here(tables.wmo_areas()?, &interiors, probe)))
        .unwrap_or_default();
    // **…and the map's own zone, which is the whole answer on a map with no
    // ground — and that is measured rather than assumed.**
    //
    // `vale zones`' last section reads every one of the twenty maps that are
    // one building and no ADT, resolves the building, and asks `WMOAreaTable`
    // what it names: **not one row of any of them states an `AreaTable` area**.
    // Sunken Temple has 127 rows and Dire Maul 111, so this is the table saying
    // nothing rather than the check failing to find it — those rows carry the
    // *room* names ("The Pit of Refuse") and leave the zone to something else.
    // With no `MCNK` under the character either, the only thing left in any
    // shipped file that knows a dungeon is a place is `AreaTable`'s own `mapId`
    // column, and it answers for 12 of the 20.
    //
    // Lazy, because it is a scan of 1,081 rows and every outdoor map answers
    // from the ground on the line above.
    let of_map = || {
        tables
            .as_ref()
            .and_then(|tables| Some(tables.areas()?.zone_of_map(map)?.id))
    };
    // A building that states no area of its own still names the room — an inn
    // is in the village it stands in — so the name below is taken either way.
    let area = match (inside.as_ref(), ground) {
        (Some(found), Some(ground)) => found.area_or(ground),
        // `area_or(0)` is the building's own id, or 0 for a group that states
        // none — which on a WMO-only map is *every* group, per the census
        // above. The map's own zone is what stands behind it.
        (Some(found), None) => match found.area_or(0) {
            0 => match of_map() {
                Some(area) => area,
                None => return,
            },
            area => area,
        },
        (None, Some(ground)) => ground,
        // Neither the ground nor a building knows: a WMO-only map whose one
        // building has not arrived yet, or one the table does not name either
        // (Blackrock Depths and Blackrock Spire put their rows on map 0). The
        // state has already been cleared for this map, so returning here shows
        // nothing rather than the last map's name.
        (None, None) => match of_map() {
            Some(area) => area,
            None => return,
        },
    };
    let sub_name = inside.map_or("", |found| found.name.as_str());

    let zone = tables.as_ref().map_or(0, |tables| {
        tables
            .areas()
            .and_then(|areas| areas.zone_of(area))
            .map_or(0, |area| area.id)
    });
    // **The building, then the area, then the zone — a column at a time.**
    // Falling back a whole *row* at a time is what left 306 of the game's
    // subzones with no ambience at all and every tavern playing the forest it
    // stands in; see `AreaSounds::or` and `WmoAreas::sounds`, which carry the
    // measurements.
    let bank = assets.sounds();
    let row = |id: u32| bank.area_sounds(id).copied().unwrap_or_default();
    let sounds = building_sounds.or(row(area)).or(row(zone));

    // **The sounds are part of the change test**, because two rooms of one
    // building can share a name and an area and still not sound alike — the
    // inn's own rows do exactly that. Testing the name alone froze the bed at
    // whichever room was entered first.
    // **The indoor flag is part of the change test for the same reason the
    // sounds are**: crossing a threshold into an unnamed room moves nothing else
    // here — same area, same name, often the same sounds — and the minimap's
    // radius halves on it.
    if area == state.area
        && sub_name == state.sub_name
        && sounds == state.sounds
        && indoors == state.indoors
        && state.aimed
    {
        return;
    }

    let zone_moved = zone != state.zone;
    // …before the write below, which is what makes it a comparison.
    let area_moved = area != state.area;
    state.area = area;
    state.zone = zone;
    state.sounds = sounds;
    state.sub_name = sub_name.to_string();
    state.indoors = indoors;

    // **The first poll aims the map at the character**, which is what
    // `SetMapToCurrentZone` does at a login — and without it the panel opens on
    // the cosmic parchment for a session that never leaves one zone.
    if !state.aimed {
        state.aimed = true;
        aim_at_current_zone(&assets, &mut state, map, zone);
        map_update.write(WorldMapUpdate);
    }

    // **…and the map redraws, because the overlays may have moved.** A
    // sub-region is revealed by *standing in it* — the server sets the bit in
    // `PLAYER_EXPLORED_ZONES` off the same area id this poll just read — so the
    // area changing is exactly the moment a zone map can gain a picture. The
    // event costs nothing when the panel is shut: `WorldMapFrame_OnEvent`
    // answers `WORLD_MAP_UPDATE` with `if ( this:IsVisible() )`.
    if area_moved || zone_moved {
        map_update.write(WorldMapUpdate);
    }
    if zone_moved {
        new_area.write(ZoneChangedNewArea);
    }
    changed.write(ZoneChanged);
    minimap.write(MinimapZoneChanged);
}

/// Say what was discovered, in the game's own words.
fn announce_discovery(
    mut found: MessageReader<Discovered>,
    strings: Res<super::messages::UiStrings>,
    mut say: super::messages::Announce,
) {
    for place in found.read() {
        let Some(table) = strings.get() else { continue };
        // **The two keys go to two different windows, and that is the table's
        // decision rather than this one's.**
        //
        // ```text
        // ERR_ZONE_EXPLORED     id 314  UI_INFO_MESSAGE  yellow, centre screen
        // ERR_ZONE_EXPLORED_XP  id 315  CHAT_MSG_SYSTEM  the chat frame
        // ```
        //
        // Which was a **guess** here for several rounds — both were sent to the
        // chat frame with a comment saying the destination column had not been
        // read out. It has been now (`vale messages` prints the column), and
        // half the guess was wrong: a discovery that pays nothing — every one of
        // them at max level, and every sub-area of a zone already explored —
        // said its line into a scrolling window instead of across the middle of
        // the screen.
        //
        // They still cannot share a call: `ERR_ZONE_EXPLORED_XP` takes two
        // arguments and `ERR_ZONE_EXPLORED` one. A key the file does not carry
        // says nothing, which is this client's rule everywhere else.
        let (key, line) = if place.experience > 0 {
            (
                "ERR_ZONE_EXPLORED_XP",
                table.get("ERR_ZONE_EXPLORED_XP").map(|text| {
                    text.replacen("%s", &place.name, 1)
                        .replacen("%d", &place.experience.to_string(), 1)
                }),
            )
        } else {
            (
                "ERR_ZONE_EXPLORED",
                table
                    .get("ERR_ZONE_EXPLORED")
                    .map(|text| text.replacen("%s", &place.name, 1)),
            )
        };
        if let Some(line) = line {
            say.composed(key, line);
        }
    }
}

/// **Raise `WORLD_MAP_UPDATE` when the exploration mask moves.**
///
/// The discovery *message* comes off `SMSG_EXPLORATION_EXPERIENCE`
/// ([`super::action`] forwards it), and this is the other half: the field
/// arrives on its own schedule, so the packet and the bit are not the same
/// instant, and the map has to redraw off whichever lands second. Watching the
/// field covers both — and covers `.explorecheat`, which sets a thousand bits
/// and sends no packet at all.
///
/// One `count_ones` over 64 words per frame, and only while there is a player.
fn track_exploration(
    units: super::api::Units,
    mut state: ResMut<WorldMapState>,
    mut map_update: MessageWriter<WorldMapUpdate>,
) {
    let Some(explored) = units.explored(super::api::UnitId::Player) else {
        return;
    };
    let now = explored.count();
    let before = std::mem::replace(&mut state.explored, now);
    // **The first reading is not a change**, or every login would raise one.
    if before != u32::MAX && before != now {
        map_update.write(WorldMapUpdate);
    }
}

/// **What the building a point is standing in calls that point, and what it
/// sounds like** — `None` and no sounds for a point in no building, or in one
/// the table says nothing about, which is most of them: 21,115 rows over the
/// game's thousands of models is a few per building, and a fence has none.
///
/// The first building whose box holds the point wins, on the same terms as the
/// group choice inside it ([`crate::render::wmos::Interior::group_at`]): WMOs
/// overlap where a city has an entrance building, and no file states a priority.
///
/// **The two answers are asked separately of the same walk**, because the table
/// answers them differently — see [`vale_assets::tables::wmoarea::WmoAreas::sounds`],
/// which is not `lookup`. A room that states a sound and no name is 2,954 of the
/// shipped rows, and it must not lose its sound to the building's just because
/// it borrows the building's name.
fn building_here<'a>(
    areas: &'a vale_assets::tables::wmoarea::WmoAreas,
    interiors: &Query<&crate::render::wmos::Interior>,
    point: [f32; 3],
) -> (
    Option<&'a vale_assets::tables::wmoarea::WmoArea>,
    vale_assets::tables::sound::AreaSounds,
    bool,
) {
    let mut place = None;
    let mut sounds = vale_assets::tables::sound::AreaSounds::default();
    // **The third answer is the walk's own, before either table is asked.**
    // Being in a room is a geometric fact and stays true for the 2,954 rows that
    // name no place and the many buildings with no row at all — so it cannot be
    // read off either of the two above without under-reporting every unnamed
    // interior in the game. See [`WorldMapState::indoors`].
    let mut indoors = false;
    for interior in interiors {
        let Some(group) = interior.group_at(point) else {
            continue;
        };
        indoors = true;
        let (root, name_set) = (interior.wmo_id, u32::from(interior.name_set));
        if place.is_none() {
            place = areas.lookup(root, name_set, group);
        }
        if sounds == vale_assets::tables::sound::AreaSounds::default() {
            sounds = areas.sounds(root, name_set, group);
        }
        if place.is_some() && sounds != vale_assets::tables::sound::AreaSounds::default() {
            break;
        }
    }
    (place, sounds, indoors)
}

/// **`SetMapToCurrentZone`** — point the view at the zone the character is in,
/// or at its continent when the zone has no map of its own.
///
/// Public because the interface calls the verb: `WorldMapFrame`'s own
/// `ToggleWorldMap` runs it every time the panel is opened.
pub fn aim_at_current_zone(
    assets: &crate::assets::GameAssets,
    state: &mut WorldMapState,
    map: u32,
    zone: u32,
) {
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let Some(world_map) = tables.world_map() else {
        return;
    };
    let Some(continent) = world_map
        .continents()
        .iter()
        .position(|continent| continent.map == map)
    else {
        // A map with no continent of its own — an instance. The real client
        // shows the cosmic parchment there, which is what a dungeon's world map
        // looks like in 1.12.
        state.view = MapView::Cosmic;
        return;
    };
    let zone_index = world_map.continents()[continent]
        .zones
        .iter()
        .position(|id| world_map.area(*id).is_some_and(|row| row.area == zone));
    state.view = match zone_index {
        Some(index) => MapView::Zone(continent, index),
        // **A zone with no `WorldMapArea` row shows its continent**, not
        // nothing: 51 rows cover far fewer than the 1,081 areas, so this is the
        // ordinary case for a sub-zone and for every instance interior.
        None => MapView::Continent(continent),
    };
}

/// Let go of the last character's place — see [`super`]'s note on why each
/// module resets its own.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut state: ResMut<WorldMapState>,
) {
    if leaving.read().next().is_some() {
        *state = WorldMapState::default();
    }
}
