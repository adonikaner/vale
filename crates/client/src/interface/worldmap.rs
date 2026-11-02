//! Where the character is, and which map the world map panel is showing.
//!
//! Neither piece of state comes from the server.
//!
//! * The area. The server says which map a character is on
//!   (`SMSG_LOGIN_VERIFY_WORLD`) and nothing else about where. The client reads
//!   the rest from the ground under the character: each `MCNK` header carries an
//!   `areaId` for its 33-yard chunk, and the id indexes `AreaTable.dbc`. So
//!   `GetZoneText()` is a terrain lookup, and every zone name in the interface
//!   (the minimap's title, the world map's, the `/say` scope text) comes from
//!   this one number.
//! * The building, which overrules the ground. The ground under Ironforge says
//!   Dun Morogh in every chunk of `Azeroth_33_41`, and the ground under the
//!   Lion's Pride Inn says Goldshire. A WMO states its own area in
//!   `WMOAreaTable.dbc`, keyed by the building, its name set and the group the
//!   character stands in ([`vale_assets::tables::wmoarea`]); it is the only data
//!   that says a city is a zone. It also carries sub-area names outright: "The
//!   Great Forge" and "Lion's Pride Inn" are rows there and appear nowhere in
//!   `AreaTable`.
//!
//!   Only spawned buildings are asked, so after a teleport into a city the
//!   ground's answer stands for the second or two `stormwind.wmo` takes to
//!   arrive, and is then corrected: the poll runs every frame and the events are
//!   raised on a change. The floor takes the same time to exist. Avoiding the
//!   delay would need a second copy of the placement list, outside the streamer.
//! * The view: which of the cosmic map, a continent, a zone or a single map the
//!   panel is drawn on. The client keeps it as two integers
//!   ([`vale_assets::tables::worldmap::MapView`]) and every map function in
//!   `WorldMapFrame.lua` reads them.
//!
//! ## Why the area is polled
//!
//! No packet marks the moment a boundary is crossed: the mover walks the
//! character locally (`world::motion`). The 1.12.1 client polls too, which is
//! why 1.12 raises `ZONE_CHANGED` itself and has no `SMSG_ZONE_UPDATE`. The poll
//! is one `HashMap` lookup in a tile the terrain cache already holds for the
//! mover, and the events are raised on a change, as [`super::vitals`]' are.
//!
//! ## The three events
//!
//! ```text
//! ZONE_CHANGED_NEW_AREA   the zone changed        Elwynn Forest -> Westfall
//! ZONE_CHANGED            the sub-area changed    Northshire Valley -> Northshire Abbey
//! MINIMAP_ZONE_CHANGED    either of the above     what the minimap's title redraws on
//! ```
//!
//! `ZONE_CHANGED_INDOORS` is not raised. It marks entering a building, which
//! this client decides from a WMO's group boxes (`render::wmos`) and not from a
//! table, so raising it on an area change would raise it at the wrong times.

use vale_assets::tables::worldmap::MapView;
use bevy::prelude::*;

use super::events::{MinimapZoneChanged, WorldMapUpdate, ZoneChanged, ZoneChangedNewArea};
use crate::lua::panels::worldmap::MapRequest;
use crate::world::session::Session;
use crate::world::status::WorldStatus;

/// The client's own two pieces of map state — see the module comment.
#[derive(Resource)]
pub struct WorldMapState {
    /// Which map the panel is on.
    pub view: MapView,
    /// The `AreaTable` id under the character, or 0 before the first poll and
    /// for a chunk that names no area. 0 is a real answer: shipped tiles have
    /// chunks with no area, and the client shows nothing for them.
    pub area: u32,
    /// The enclosing zone's id, resolved once here rather than at every read.
    pub zone: u32,
    /// Which map the fields above are about.
    ///
    /// A teleport makes every one of them false at once, and [`track_area`]
    /// keeps what it had when nothing names an area yet, which is right within
    /// a map and wrong across two. Without this a character who walked into the
    /// Stockade kept Elwynn Forest on the minimap, because nothing in the
    /// instance overwrote it.
    ///
    /// 0 means no map yet. Map 0 is Eastern Kingdoms, so a login that lands
    /// there skips one reset, and the first poll clears the state anyway because
    /// `aimed` is false.
    map: u32,
    /// The sub-area's name when a building names it, and empty otherwise; see
    /// the module comment.
    ///
    /// A string rather than an id, because there is no id: "The Great Forge"
    /// and "Lion's Pride Inn" are `WMOAreaTable` rows and appear nowhere in
    /// `AreaTable`. When this is empty the sub-area is [`Self::area`]'s own
    /// name, which is the case in open country.
    pub sub_name: String,
    /// Whether the character is inside a building at all, from the same walk
    /// [`Self::sub_name`] comes from.
    ///
    /// It differs from `sub_name` being non-empty: 21,115 `WMOAreaTable` rows
    /// over the game's thousands of models is a few per building, so many rooms
    /// have no name. The zone does not depend on it, and neither does the
    /// minimap: [`super::minimap`] decides indoors from the floor under the
    /// feet, as [`Self::outdoors`] does.
    pub(crate) indoors: bool,
    /// Whether the character is under open sky, for the rules that depend on
    /// it: mounting, and every outdoor-only or indoor-only spell.
    ///
    /// This is not `!indoors`. `indoors` means inside some group's box, which is
    /// true on every street of Stormwind, because the city is one building
    /// whose districts are exterior groups; the cast check that read it refused
    /// every mount in the city. This is the server's test instead: the group the
    /// floor underfoot belongs to, read for `MOGP` flag `0x8000`. See
    /// [`vale_assets::world::wmo::outdoors_at`], which states the rule and its
    /// two sources.
    pub(crate) outdoors: bool,
    /// What this place sounds like: the ambience, the zone music and the intro
    /// music, resolved through three levels a column at a time: the building's
    /// own columns, then the area's row, then the zone's. [`track_area`] decides
    /// it; [`vale_assets::tables::sound::AreaSounds::or`] explains why the
    /// fallback is per column rather than per row.
    ///
    /// It is here rather than in `sound/` because the building walk that says
    /// which room the character is in is this module's.
    pub sounds: vale_assets::tables::sound::AreaSounds,
    /// Whether the view has been aimed at the character. `SetMapToCurrentZone`
    /// is the interface's own call; this makes the first one, at a login, land
    /// on the character's map rather than leaving the cosmic map showing.
    aimed: bool,
    /// How many places the character had explored at the last check, which is
    /// how the map learns that it has a new picture to draw.
    ///
    /// `PLAYER_EXPLORED_ZONES` is an update field with no event of its own, and
    /// the interface reads the overlays again only on `WORLD_MAP_UPDATE`.
    /// Without this a zone map did not show a new overlay until something else
    /// moved the view, or until the next login.
    ///
    /// A count rather than the mask, because bits are only ever set, so a change
    /// of count is a change of mask. `u32::MAX` means not read yet, so the first
    /// reading is not taken as a change.
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
            // Not zero; see [`WorldMapState::explored`]. A new character has explored
            // nothing, so a zero would be a reading and the first real one would look
            // like a change.
            explored: u32::MAX,
        }
    }
}

/// A place was discovered. [`crate::interface::action`] composes the name
/// and [`announce_discovery`] says it.
///
/// A message rather than a direct chat write because the name needs
/// `AreaTable` and the line needs `GlobalStrings.lua`, two resources in two
/// systems.
#[derive(Message, Debug, Clone)]
pub(crate) struct Discovered {
    pub name: String,
    pub experience: u32,
}

/// The one place the server can name on the world map: `SMSG_GOSSIP_POI`,
/// forwarded by [`crate::world::incoming`].
///
/// A message because the packet is parsed on the session thread and the
/// state is a Bevy resource; [`crate::world::incoming`] is where the two
/// meet.
#[derive(Message, Debug, Clone)]
pub(crate) struct PoiAnswer {
    pub poi: vale_assets::tables::areapoi::GossipPoi,
}

/// The flag a guard's directions put up, and the map it was named on.
///
/// There is one, and a new one replaces it, as in the 1.12.1 client, which
/// keeps a single synthetic `AreaPOI` row and overwrites it. See
/// [`vale_assets::tables::areapoi`].
///
/// The map is kept here because the packet does not carry one. The client
/// projects the point onto whichever map is open, so without the map a flag
/// named in Stormwind and carried to Kalimdor would be drawn at the same
/// yards on the wrong continent.
#[derive(Resource, Default, Debug, Clone)]
pub(crate) struct MapLandmarks {
    pub gossip: Option<vale_assets::tables::areapoi::GossipPoi>,
    /// The map the character was standing on when it arrived.
    pub map: u32,
}

/// Take the server's point, record the map it was named on, and raise
/// `WORLD_MAP_UPDATE`.
///
/// `WorldMapFrame`'s `OnShow` runs `SetMapToCurrentZone()` and does not call
/// `WorldMapFrame_Update()` itself, so the list is drawn again only on
/// `WORLD_MAP_UPDATE`. Without the event here, the first set of directions
/// showed (the open moved the view, which raised the event) and every later
/// one did not until the view was moved again.
fn take_poi(
    mut answers: MessageReader<PoiAnswer>,
    status: Res<crate::world::status::WorldStatus>,
    mut landmarks: ResMut<MapLandmarks>,
    mut map_update: MessageWriter<WorldMapUpdate>,
) {
    for answer in answers.read() {
        // Logged, because nothing on screen shows that the packet came: the flag
        // is visible only with the map open on the right picture, and a report of
        // no flag has to be told apart from no packet.
        info!(
            "map flag: {:?} at ({:.1}, {:.1}) icon {} on map {}",
            answer.poi.name, answer.poi.position.0, answer.poi.position.1, answer.poi.icon, status.map_id
        );
        landmarks.gossip = Some(answer.poi.clone());
        landmarks.map = status.map_id;
        map_update.write(WorldMapUpdate);
    }
}

pub(crate) struct WorldMapPlugin;

impl Plugin for WorldMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldMapState>()
            .init_resource::<MapLandmarks>()
            .add_message::<Discovered>()
            .add_message::<PoiAnswer>()
            .add_systems(
            Update,
            // The requests before the poll, so a `SetMapToCurrentZone` made in a
            // handler this frame is applied against the place the poll is about to
            // confirm rather than a frame late.
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
    let Some(host) = host else { return };
    let requests = crate::lua::panels::worldmap::take(&host);
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
            // A click on the cosmic map zooms into the continent under it, and one on
            // a continent into the zone under it: one level in, which is what
            // `ProcessMapClick` does. A click on a zone or single map does nothing.
            MapRequest::Click(u, v) => state.view = clicked(&assets, state.view, u, v),
            other => {
                let zones = |continent: usize| zone_count(&assets, continent);
                state.view = crate::lua::panels::worldmap::apply(state.view, other, zones);
            }
        }
    }
    // Every request raises `WORLD_MAP_UPDATE`, not only one that moves the
    // view. The 1.12.1 client does the same: `SetMapToCurrentZone` and
    // `SetMapZoom` both store the two indices and then refill the landmark list
    // without comparing against what was showing.
    //
    // `WorldMapFrame_OnEvent` acts on `WORLD_MAP_UPDATE` only
    // `if ( this:IsVisible() )`, so an event raised while the map is shut is
    // lost, and a guard is talked to with the map shut. With a change test
    // here, opening the map found the view already on the zone, raised
    // nothing, and drew the list from before the conversation.
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

/// One level in, from wherever the click landed.
///
/// It needs the tables as well as the view, which is why it is here and not
/// beside [`crate::lua::panels::worldmap::apply`]: the cosmic map's
/// continents and a continent's zones are rectangles in `WorldMapArea`.
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
        // The same test the highlight makes.
        MapView::Cosmic => world_map
            .continent_at(u, v)
            .map_or(view, MapView::Continent),
        MapView::Continent(continent) => world_map
            .zone_at(continent, u, v)
            .map_or(view, |zone| MapView::Zone(continent, zone)),
        // The 1.12.1 client ignores a click on a single map.
        MapView::Zone(_, _) | MapView::Single(_) => view,
    }
}

/// How far above the character's feet the building test is taken, in yards.
///
/// The same yard as `world::entities`' `INDOOR_PROBE`, for the same reason: a
/// group's box is its own geometry, so its top face is the outside of the
/// roof, and a character standing on a roof would otherwise read as being in
/// the room below.
const AREA_PROBE: f32 = 1.0;

/// Poll the ground for an area id, and the building over it, which wins
/// where it names one (see the module comment). Raise the three events on a
/// change.
pub(super) fn track_area(
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    status: Res<WorldStatus>,
    mut state: ResMut<WorldMapState>,
    // The placed buildings of the loaded 3x3, as
    // `world::entities::light_entities` takes them: each tests its own box
    // before walking its groups, so a city's three hundred are walked only
    // when the character is in the city.
    interiors: Query<&crate::render::wmos::Interior>,
    // The buildings' solid triangles, for the one question the boxes cannot
    // answer: which group's floor the character is standing on. See
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
    // A teleport makes every field here false at once. The resolution below
    // keeps what it had when nothing is known yet, which is right within one
    // map and wrong across two, so the state is cleared on a change of map.
    // Clearing also re-aims the world map, since a new state has `aimed`
    // false. See [`WorldMapState::map`].
    if state.map != map {
        *state = WorldMapState { map, ..WorldMapState::default() };
    }
    // The mover's own terrain, so the tile is already resident and this is a
    // cache lookup rather than a parse. See `world::session`.
    //
    // An `Option`, not a precondition. Twenty of the game's forty-three maps
    // have no ADT at all, so on them this is always `None`. Returning here on
    // `None` skipped the building below too, and left every dungeon with no
    // area, no zone, no sub-zone, no ambience, no zone music and no minimap
    // name. The ground is a fallback for what the building says, and on those
    // maps the building is the only source.
    let ground = active.terrain_area(map, position.x, position.y);
    let tables = assets.display_tables().ok();
    let probe = [position.x, position.y, position.z + AREA_PROBE];
    // Open sky, by the server's rule. Asked every frame and written without a
    // change test of its own, because no event depends on it: the cast check
    // reads it when a button is pressed. It costs one cell of one hull's grid
    // per building under the point, a few dozen triangles.
    let probe_z = position.z + AREA_PROBE;
    let outdoors = vale_assets::world::wmo::outdoors_at(
        solids.0.building_floor(map, position.x, position.y, probe_z),
        active.terrain_height(map, position.x, position.y),
        probe_z,
    );
    // Written only on a change, so the resource is not marked changed every
    // frame.
    if state.outdoors != outdoors {
        state.outdoors = outdoors;
    }
    let (inside, building_sounds, indoors) = tables
        .as_ref()
        .and_then(|tables| Some(building_here(tables.wmo_areas()?, &interiors, probe)))
        .unwrap_or_default();
    // The map's own zone, which is the only answer on a map with no ground.
    //
    // `vale zones`' last section reads every one of the twenty maps that are
    // one building and no ADT, resolves the building, and asks `WMOAreaTable`
    // what it names: no row of any of them states an `AreaTable` area. Sunken
    // Temple has 127 rows and Dire Maul 111, so the rows were found; they carry
    // room names ("The Pit of Refuse") and leave the zone out. With no `MCNK`
    // under the character either, the only shipped data that places a dungeon
    // is `AreaTable`'s own `mapId` column, which answers for 12 of the 20.
    //
    // Lazy, because it scans 1,081 rows and every outdoor map answers from the
    // ground above.
    let of_map = || {
        tables
            .as_ref()
            .and_then(|tables| Some(tables.areas()?.zone_of_map(map)?.id))
    };
    // A building that states no area of its own still names the room (an inn
    // is in the village it stands in), so the name below is taken either way.
    let area = match (inside.as_ref(), ground) {
        (Some(found), Some(ground)) => found.area_or(ground),
        // `area_or(0)` is the building's own id, or 0 for a group that states
        // none, which on a WMO-only map is every group (see above). The map's own
        // zone stands behind it.
        (Some(found), None) => match found.area_or(0) {
            0 => match of_map() {
                Some(area) => area,
                None => return,
            },
            area => area,
        },
        (None, Some(ground)) => ground,
        // Neither the ground nor a building knows: a WMO-only map whose building
        // has not arrived yet, or one the table does not name either (Blackrock
        // Depths and Blackrock Spire put their rows on map 0). The state has
        // already been cleared for this map, so returning shows nothing rather
        // than the last map's name.
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
    // The building, then the area, then the zone, a column at a time. Falling
    // back a whole row at a time left 306 of the game's sub-zones with no
    // ambience and every tavern playing the forest it stands in; see
    // `AreaSounds::or` and `WmoAreas::sounds`, which carry the measurements.
    let bank = assets.sounds();
    let row = |id: u32| bank.area_sounds(id).copied().unwrap_or_default();
    let sounds = building_sounds.or(row(area)).or(row(zone));

    // The sounds are part of the change test, because two rooms of one
    // building can share a name and an area and not sound alike; the inn's
    // rows do. Testing the name alone kept the sounds of the first room
    // entered.
    //
    // The indoor flag is part of it for the same reason: crossing into an
    // unnamed room changes nothing else here, and the minimap's radius halves
    // on it.
    if area == state.area
        && sub_name == state.sub_name
        && sounds == state.sounds
        && indoors == state.indoors
        && state.aimed
    {
        return;
    }

    let zone_moved = zone != state.zone;
    // Compared before the write below.
    let area_moved = area != state.area;
    state.area = area;
    state.zone = zone;
    state.sounds = sounds;
    state.sub_name = sub_name.to_string();
    state.indoors = indoors;

    // The first poll aims the map at the character, as `SetMapToCurrentZone`
    // does at a login. Without it the panel opens on the cosmic map for a
    // session that never leaves one zone.
    if !state.aimed {
        state.aimed = true;
        aim_at_current_zone(&assets, &mut state, map, zone);
        map_update.write(WorldMapUpdate);
    }

    // The map redraws, because the overlays may have changed. A sub-area is
    // revealed by standing in it (the server sets the bit in
    // `PLAYER_EXPLORED_ZONES` from the same area id this poll read), so a
    // change of area is when a zone map can gain an overlay. The event costs
    // nothing with the panel shut: `WorldMapFrame_OnEvent` answers
    // `WORLD_MAP_UPDATE` only `if ( this:IsVisible() )`.
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
        // The two keys go to two windows, as the message table says:
        //
        // ```text
        // ERR_ZONE_EXPLORED     id 314  UI_INFO_MESSAGE  yellow, centre screen
        // ERR_ZONE_EXPLORED_XP  id 315  CHAT_MSG_SYSTEM  the chat frame
        // ```
        //
        // `vale messages` prints the destination column. A discovery that pays no
        // experience (every one at maximum level, and every sub-area of a zone
        // already explored) goes across the middle of the screen.
        //
        // They cannot share a call: `ERR_ZONE_EXPLORED_XP` takes two arguments and
        // `ERR_ZONE_EXPLORED` one. A key the file does not carry says nothing, as
        // everywhere else in this client.
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

/// Raise `WORLD_MAP_UPDATE` when the exploration mask changes.
///
/// The discovery message comes from `SMSG_EXPLORATION_EXPERIENCE`
/// ([`super::action`] forwards it). The field arrives on its own schedule, so
/// the packet and the bit are not the same instant, and the map has to redraw
/// on whichever lands second. Watching the field covers both, and covers
/// `.explorecheat`, which sets a thousand bits and sends no packet.
///
/// One `count_ones` over 64 words per frame, only while there is a player.
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
    // The first reading is not a change, or every login would raise one.
    if before != u32::MAX && before != now {
        map_update.write(WorldMapUpdate);
    }
}

/// What the building a point is in calls that point, and what it sounds
/// like. `None` and no sounds for a point in no building, or in one the
/// table says nothing about, which is most: 21,115 rows over the game's
/// thousands of models is a few per building, and a fence has none.
///
/// The first building whose box holds the point wins, as in the group choice
/// inside it ([`crate::render::wmos::Interior::group_at`]): WMOs overlap where
/// a city has an entrance building, and no file states a priority.
///
/// The name and the sounds are asked separately of the same walk, because the
/// table answers them differently; see
/// [`vale_assets::tables::wmoarea::WmoAreas::sounds`], which is not `lookup`.
/// 2,954 shipped rows state a sound and no name, and such a room must keep its
/// own sound while it borrows the building's name.
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
    // The third answer comes from the walk itself, before either table is
    // asked. Being in a room is a geometric fact, true also for the 2,954 rows
    // that name no place and the buildings with no row, so it cannot be read
    // from the two answers above. See [`WorldMapState::indoors`].
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

/// `SetMapToCurrentZone`: point the view at the zone the character is in,
/// at its continent when the zone has no map of its own, and on a map that is
/// no continent's at that map's own row, or the cosmic map when it has none.
///
/// Public because the interface calls it: `WorldMapFrame`'s `ToggleWorldMap`
/// runs it every time the panel is opened.
pub(crate) fn aim_at_current_zone(
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
        // A map that is no continent's map. The 1.12.1 client shows the first
        // row on it on its own, which is how a battleground has a map, and the
        // cosmic map on a map with no row, which is every dungeon.
        state.view = world_map.single_for(map).map_or(MapView::Cosmic, MapView::Single);
        return;
    };
    let zone_index = world_map.continents()[continent]
        .zones
        .iter()
        .position(|id| world_map.area(*id).is_some_and(|row| row.area == zone));
    state.view = match zone_index {
        Some(index) => MapView::Zone(continent, index),
        // A zone with no `WorldMapArea` row shows its continent. 51 rows cover
        // far fewer than the 1,081 areas, so this is the usual case for a sub-zone
        // and for every instance interior.
        None => MapView::Continent(continent),
    };
}

/// Let go of the last character's place. See [`super`]'s note on why each
/// module resets its own state.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut state: ResMut<WorldMapState>,
) {
    if leaving.read().next().is_some() {
        *state = WorldMapState::default();
    }
}
