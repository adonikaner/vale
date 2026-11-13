//! A map that is one building and no terrain (most dungeons): where the
//! building stands, the tiles under it, and where the camera is put when the
//! map is opened.
//!
//! ```text
//! the box       the root file's bounds (`MOHD`) through the WDT's placement,
//!               in the world's axes; read with the map's layout
//!               (`super::tiles::read_building`)
//! the tiles     the 533-yard tiles the box covers, numbered as ADTs are.
//!               The map has no ADTs; these are the tiles the server's
//!               navmesh is built for (`crate::server::datadir`)
//! the arrival   where a character arrives: the lowest-numbered
//!               `areatrigger_teleport` row that sends a character to the
//!               map, as this project leaves it
//! ```
//!
//! ## Where the camera goes
//!
//! A map with no terrain has no ground for the camera to drop to, and the
//! building can be anywhere on the map, so a camera left where the previous
//! map had it, or at the map's origin, can look at nothing. When such a map
//! is opened, the camera is put at the arrival, or over the whole box when
//! no teleport sends a character there. It is left where it is when what
//! opened the map named a place inside the box (a bookmark, a trigger's
//! teleport target, a transport's stop), and when `--at` named the start.
//!
//! The teleports are read from the world database, which answers after the
//! map is open. Until it answers, the box is framed; when it answers, the
//! camera is moved to the arrival unless it has been moved since. A teleport
//! this project creates is found without the database. The map window's Go
//! to Building asks for the same placing again ([`Arrival::asked`]).

use bevy::prelude::*;

use crate::server::fresh::FreshReads;
use crate::session::EditSession;
use vale_mangos::row::{Edits, Life};
use vale_mangos::trigger::{self, Teleport};

/// The yards a tile is wide.
const TILE: f32 = vale_assets::world::adt::TILE_SIZE;

/// How far outside the box a place still counts as in it, in yards, for a
/// place a bookmark or a teleport target named at a door or on a balcony.
const NEAR_THE_BOX: f32 = 40.0;

/// The building's bounding box in the world's axes: x north, y west, z up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BuildingBox {
    pub(crate) lower: Vec3,
    pub(crate) upper: Vec3,
}

impl BuildingBox {
    /// The root file's box, in the building's own axes, through the
    /// placement's model-to-world matrix (`PlacedModel::matrix`): the box of
    /// its eight corners.
    pub(crate) fn of(root: [[f32; 3]; 2], matrix: &[f32; 16]) -> BuildingBox {
        let matrix = Mat4::from_cols_array(matrix);
        let (low, high) = (Vec3::from(root[0]), Vec3::from(root[1]));
        let mut lower = Vec3::splat(f32::INFINITY);
        let mut upper = Vec3::splat(f32::NEG_INFINITY);
        for corner in 0..8 {
            let pick = |bit: usize, axis: usize| match corner >> bit & 1 {
                0 => low[axis],
                _ => high[axis],
            };
            let at = matrix.transform_point3(Vec3::new(pick(0, 0), pick(1, 1), pick(2, 2)));
            lower = lower.min(at);
            upper = upper.max(at);
        }
        BuildingBox { lower, upper }
    }

    /// Whether `at` is in the box or within [`NEAR_THE_BOX`] of it.
    pub(crate) fn holds(&self, at: Vec3) -> bool {
        let margin = Vec3::splat(NEAR_THE_BOX);
        at.cmpge(self.lower - margin).all() && at.cmple(self.upper + margin).all()
    }

    /// The tiles under the box, first and last inclusive, as `(x, y)` in the
    /// ADT names' numbering: x falls as the world's y rises, and y as its x
    /// does. See `vale_assets::world::terrain::tile_for_position`.
    pub(crate) fn tile_range(&self) -> ((u32, u32), (u32, u32)) {
        let first = vale_assets::tile_for_position(self.upper.x, self.upper.y);
        let last = vale_assets::tile_for_position(self.lower.x, self.lower.y);
        (first, last)
    }

    /// Every tile under the box, row by row.
    pub(crate) fn tiles(&self) -> Vec<(u32, u32)> {
        let ((x0, y0), (x1, y1)) = self.tile_range();
        (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).collect()
    }

    /// The box in the map window's tile coordinates, fractional: the corner
    /// with the smaller x and y, and the opposite one.
    pub(crate) fn in_tiles(&self) -> ([f32; 2], [f32; 2]) {
        let origin = vale_assets::world::adt::MAP_ORIGIN;
        let x = |world_y: f32| (origin - world_y) / TILE;
        let y = |world_x: f32| (origin - world_x) / TILE;
        ([x(self.upper.y), y(self.upper.x)], [x(self.lower.y), y(self.lower.x)])
    }

    /// The camera over the whole box: its centre, from a distance that shows
    /// most of it, between [`FRAME_NEAREST`] and [`FRAME_FARTHEST`].
    pub(crate) fn framing(&self) -> crate::camera::FlyTo {
        let size = self.upper - self.lower;
        crate::camera::FlyTo {
            at: (self.lower + self.upper) * 0.5,
            distance: Some((size.length() * 0.6).clamp(FRAME_NEAREST, FRAME_FARTHEST)),
        }
    }
}

/// The nearest and farthest the camera stands to frame a building, in yards.
/// The farthest is about what the editor's camera may be asked for.
const FRAME_NEAREST: f32 = 60.0;
const FRAME_FARTHEST: f32 = 1500.0;

/// How far the camera stands from the arrival, in yards: a room's width.
const AT_THE_ARRIVAL: f32 = 40.0;

/// Where a character arrives on map `map`: the target of the lowest-numbered
/// teleport row that sends a character there, as the project leaves the
/// rows. `held` is the rows the database holds; a row the project marks for
/// removal is left out, and a row it creates is counted.
pub(crate) fn arrival_on(held: &[Teleport], edits: &Edits, map: u32) -> Option<Vec3> {
    let mut found: Vec<Teleport> = Vec::new();
    for teleport in held {
        let shown = crate::tools::places::triggers::shown(edits, trigger::TELEPORT, &teleport.key(), Some(&teleport.assignments()));
        if let Some((row, _)) = shown.filter(|(_, life)| *life != Life::Delete) {
            found.extend(Teleport::from_row(&row));
        }
    }
    for (table, key, claim) in edits.rows() {
        if table != trigger::TELEPORT || claim.life != Life::Insert || held.iter().any(|teleport| teleport.key() == *key) {
            continue;
        }
        if let Some((row, _)) = crate::tools::places::triggers::shown(edits, trigger::TELEPORT, key, None) {
            found.extend(Teleport::from_row(&row));
        }
    }
    found
        .into_iter()
        .filter(|teleport| teleport.target_map == map)
        .min_by_key(|teleport| teleport.id)
        .map(|teleport| Vec3::from(teleport.target))
}

/// The camera at the arrival.
fn at_the_arrival(at: Vec3) -> crate::camera::FlyTo {
    crate::camera::FlyTo { at, distance: Some(AT_THE_ARRIVAL) }
}

/// Where the camera is put on a building's map, and the teleports it is put
/// by. See the module comment.
#[derive(Resource, Default)]
pub(crate) struct Arrival {
    /// Every `areatrigger_teleport` row the database holds at the server's
    /// patch, read while a building's map is open.
    teleports: FreshReads<(), Vec<Teleport>>,
    /// The map the camera was last placed for, and whether that was the
    /// first map of the run.
    placed_for: Option<String>,
    /// Where the camera was framed while the teleports were being read, so
    /// the answer moves it only when it has not been moved since.
    waiting: Option<Vec3>,
    /// Go to Building in the map window: place the camera again.
    pub(crate) asked: bool,
}

/// Put the camera on the open map's building when the map is opened, and
/// when the map window asks. Nothing on a map with terrain.
pub(crate) fn place_camera(
    session: Option<Res<EditSession>>,
    tiles: Res<super::tiles::Tiles>,
    settings: Res<crate::server::settings::ServerSettings>,
    args: Res<crate::args::Args>,
    mut camera: ResMut<crate::camera::EditorCamera>,
    mut arrival: ResMut<Arrival>,
) {
    let Some(session) = session else { return };
    // The box is read with the map's layout, a frame or more after the map
    // is opened.
    if !tiles.building_for.eq_ignore_ascii_case(&session.map) {
        return;
    }
    let building = tiles.building_box;
    let writes = session.database_writes;
    arrival.teleports.renew(writes);
    if let Some(Err(why)) = arrival.teleports.poll() {
        warn!("building: the teleports were not read: {why}");
        arrival.waiting = None;
    }
    if building.is_some() {
        let patch = crate::server::settings::server_patch(&settings);
        arrival.teleports.want(writes, (), || {
            let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
            Ok(crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                Ok(db.rows(&trigger::teleports_query(patch))?.iter().filter_map(Teleport::from_row).collect())
            }))
        });
    }
    let held: &[Teleport] = arrival.teleports.value().map_or(&[], Vec::as_slice);
    let found = arrival_on(held, &session.rows.edits, session.map_id);

    let opened = arrival.placed_for.as_deref().is_none_or(|map| !map.eq_ignore_ascii_case(&session.map));
    if opened {
        let first = arrival.placed_for.is_none();
        arrival.placed_for = Some(session.map.clone());
        arrival.waiting = None;
        let Some(building) = building else { return };
        // A place the opener named: `--at` at the start, or anything inside
        // the box afterwards.
        let named = match first {
            true => args.view.at.is_some(),
            false => building.holds(camera.target),
        };
        if named {
            camera.wants_the_ground = false;
            return;
        }
    } else if !std::mem::take(&mut arrival.asked) {
        // The teleports answered after the box was framed: the arrival,
        // unless the camera has been moved since.
        let Some(framed) = arrival.waiting else { return };
        if arrival.teleports.value().is_none() && arrival.teleports.trouble().is_none() {
            return;
        }
        arrival.waiting = None;
        if let Some(at) = found.filter(|_| camera.target == framed) {
            go(&mut camera, at_the_arrival(at));
        }
        return;
    }
    let Some(building) = building else { return };
    match found {
        Some(at) => go(&mut camera, at_the_arrival(at)),
        None => {
            go(&mut camera, building.framing());
            let reading = arrival.teleports.value().is_none() && arrival.teleports.trouble().is_none();
            arrival.waiting = reading.then_some(camera.target);
        }
    }
}

/// Carry out a placing, as `crate::camera::fly_to` carries out a request.
fn go(camera: &mut crate::camera::EditorCamera, to: crate::camera::FlyTo) {
    camera.target = to.at;
    camera.wants_the_ground = false;
    if let Some(distance) = to.distance {
        camera.distance = distance;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stormwind Stockade's box as the client's reader places it (see
    /// `vale_assets::world::wdt::Wdt::placed_global_wmos`): the placement at
    /// the map's origin with no turn, so the box is the root's own, turned a
    /// half turn about z.
    #[test]
    fn a_box_is_its_corners_through_the_placement() {
        let matrix = vale_assets::world::adt::placement_matrix([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
        let placed = BuildingBox::of([[-197.0, -151.0, -40.0], [8.0, 150.0, 30.0]], &matrix);
        let close = |a: Vec3, b: Vec3| (a - b).abs().max_element() < 1e-3;
        assert!(close(placed.lower, Vec3::new(-8.0, -150.0, -40.0)), "{placed:?}");
        assert!(close(placed.upper, Vec3::new(197.0, 151.0, 30.0)), "{placed:?}");
        assert!(placed.holds(Vec3::new(100.0, 0.0, 0.0)));
        assert!(placed.holds(Vec3::new(230.0, 0.0, 0.0)), "within the margin");
        assert!(!placed.holds(Vec3::new(-9000.0, 500.0, 0.0)));
    }

    /// The tiles under a box are those the client's tile lookup gives its
    /// corners, and the map window's fractional box covers them.
    #[test]
    fn the_tiles_under_a_box() {
        let building = BuildingBox { lower: Vec3::new(-8.0, -150.0, -40.0), upper: Vec3::new(197.0, 151.0, 30.0) };
        assert_eq!(building.tile_range(), ((31, 31), (32, 32)));
        assert_eq!(building.tiles(), vec![(31, 31), (32, 31), (31, 32), (32, 32)]);
        let (from, to) = building.in_tiles();
        assert!(from[0] > 31.0 && from[0] < 32.0 && to[0] > 32.0 && to[0] < 33.0, "{from:?} {to:?}");
        assert!(from[1] > 31.0 && from[1] < 32.0 && to[1] > 32.0 && to[1] < 33.0, "{from:?} {to:?}");
        let framed = building.framing();
        assert_eq!(framed.at, Vec3::new(94.5, 0.5, -5.0));
        assert!(framed.distance.is_some_and(|d| (FRAME_NEAREST..=FRAME_FARTHEST).contains(&d)));
    }

    /// The arrival is the lowest-numbered teleport to the map, as the project
    /// leaves the rows: a row it removes is not one, and a row it creates is.
    #[test]
    fn the_arrival_is_the_first_teleport_to_the_map() {
        let held = vec![
            Teleport::new(12, 34, [50.0, 1.0, 2.0]),
            Teleport::new(7, 0, [-9000.0, 300.0, 50.0]),
            Teleport::new(30, 34, [60.0, 1.0, 2.0]),
        ];
        let mut edits = Edits::default();
        assert_eq!(arrival_on(&held, &edits, 34), Some(Vec3::new(50.0, 1.0, 2.0)));
        edits.set_life(trigger::TELEPORT, &held[0].key(), Life::Delete);
        assert_eq!(arrival_on(&held, &edits, 34), Some(Vec3::new(60.0, 1.0, 2.0)), "a removed row is not an arrival");
        let made = Teleport::new(3, 34, [70.0, 1.0, 2.0]);
        let created = crate::subjects::services::creation(&made.assignments());
        edits.set_row_line(trigger::TELEPORT, &made.key(), Some(&created.to_line()));
        assert_eq!(arrival_on(&held, &edits, 34), Some(Vec3::new(70.0, 1.0, 2.0)), "a created row is");
        assert_eq!(arrival_on(&[], &Edits::default(), 34), None);
    }
}
