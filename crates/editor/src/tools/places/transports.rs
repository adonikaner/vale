//! Transports: the boats and zeppelins that sail a path of the flight path
//! tool, with their schedules, their periods on the server, and the commands
//! that make and remove one.
//!
//! A transport is three things in three places:
//!
//! ```text
//! the route      a TaxiPath row and its TaxiPathNode points, which the flight
//!                path tool draws and edits (`super::flightpaths`)
//! the template   a gameobject_template row of type 15: the hull's display,
//!                and data0 the path, data1 the speed, data2 the acceleration
//! the period     a transports row: milliseconds one round takes
//! ```
//!
//! The server makes a ship of every type 15 template whose path it can read,
//! so the template is the transport and there is no spawn row. This file reads
//! the templates and the periods from the world database, puts the project's
//! claims over them, and builds each ship's schedule with the client's own
//! schedule code (`vale_assets::tables::shiptransport`).
//!
//! The flight path tool has two halves, Flights and Transports
//! (`Flightpaths::transports`), and this is the second. Its toolbar and
//! inspector are `crate::ui::tools::places::transports`.
//!
//! ## A route is its legs
//!
//! The client's schedule cuts a route where the map changes and after a point
//! with the teleport flag ([`runs`]). Each part is a [`Leg`]: the ship comes
//! into view at its first point, sails through the others, waits at the
//! stops, and leaves at its last; it is then moved to the start of the next
//! leg, and from the last leg to the first. A leg's first and last point are
//! curve handles and not places the ship goes.
//!
//! A leg is drawn by clicking the places the ship sails through
//! ([`Draft`], [`press`], [`keys`]), and [`leg_points`] adds the two
//! handles. A new transport is its first leg, made with its path, the two
//! nodes a path needs, its template and its period as one undo entry
//! ([`finish`]). A leg on another map is drawn after going there.
//!
//! ## Going to another map
//!
//! The selected route ([`Transports::route`]) is kept while its map is not
//! the open one. [`Go`] is a request to look at a place on a map, which
//! [`travel`] answers by opening the map first when it is not the open one,
//! as an area trigger's teleport target is picked. A leg, a stop and the
//! ship's place in the preview each have one, so a route over two maps is
//! edited without choosing each map by hand. [`sync`] keeps the route
//! selected in the flight path tool's own lists across the change.
//!
//! ## The period follows the route
//!
//! Neither end is told where a ship is. Both compute it from the route and
//! from how far into the round the ship is, taken modulo the round's length,
//! so the two lengths have to be the same integer. The server's own
//! arithmetic gives a slightly different total from the client's, and it
//! reads `transports.period` in place of it. So every change to a route's
//! points, stops or delays, or to a template's speed or acceleration, needs
//! the period written again from the client's schedule.
//!
//! [`collect`] does that. When an edit takes a ship that was in step out of
//! step, it writes the new period into the project's row at build 5875 on the
//! same undo entry as the edit ([`EditSession::amend_server_row`]), so one
//! undo takes back both. It writes nothing when the edit on top of the stack
//! is not about a route or a template, or when the last thing that happened
//! was an undo: the ship is then shown as out of step and
//! [`update_period`], the panel's button, writes the period as an entry of its
//! own.
//!
//! The rows are read while the three taxi tables are open in the session, and
//! the ships are built again while the flight path tool is chosen or a client
//! table changed. A template's speed edited in the Game Objects workspace with
//! the flight path tool closed is therefore found the next time the tool is
//! opened, and is the button's case.
//!
//! ## What is not handled
//!
//! A template of another type that the project changes to type 15 is not
//! listed: its other columns are the database's and are not read here. A
//! transport the database holds cannot be removed, because a template row is
//! never deleted (`vale_mangos::gameobject`).

use super::flightpaths::Flightpaths;
use crate::marks::{Look, Marks};
use crate::server::fresh::FreshReads;
use crate::server::store::Gesture;
use crate::session::EditSession;
use crate::subjects::services::{creation, write_column};
use crate::tools::Tool;
use bevy::prelude::*;
use std::collections::HashMap;
use vale_assets::tables::shiptransport::{MoTransport, Route as Schedule, Spline};
use vale_assets::tables::taxi::TaxiWaypoint;
use vale_client::render::axes;
use vale_edit::dbc::taxi::{self, Point};
use vale_mangos::gameobject;
use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};
use vale_mangos::transport::{self, Period, RoutePoint, Template};

/// What the world database holds about transports.
#[derive(Debug, Clone, Default)]
pub(crate) struct Held {
    /// The type 15 templates the server would load.
    pub(crate) templates: Vec<Template>,
    /// Every `transports` row the loader could read.
    pub(crate) periods: Vec<Period>,
    /// The highest `gameobject_template.entry` at any patch, for numbering a
    /// new template.
    pub(crate) max_entry: Option<u32>,
}

/// A transport's period, on each side of an Apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PeriodState {
    /// The row the server uses now: the highest build the database holds.
    pub(crate) database: Option<u32>,
    /// The period the project's row at build 5875 states.
    pub(crate) claimed: Option<u32>,
    /// Whether the database holds a row at build 5875 that the project does
    /// not create.
    pub(crate) dev_row_held: bool,
}

impl PeriodState {
    /// The period the server uses once the project is applied.
    pub(crate) fn server(&self) -> Option<u32> {
        self.claimed.or(self.database)
    }
}

/// One place a ship waits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stop {
    /// Milliseconds into the round the ship arrives.
    pub(crate) arrive_ms: u32,
    pub(crate) wait_ms: u32,
    pub(crate) map: u32,
    pub(crate) at: [f32; 3],
    /// The `TaxiPathNode` id of the point the stop is at.
    pub(crate) point: Option<u32>,
}

/// One transport as the project leaves it.
#[derive(Debug, Clone)]
pub(crate) struct Ship {
    /// The template with the project's edits over the database's row.
    pub(crate) template: Template,
    /// The key the project's claim on the template is under.
    pub(crate) key: Key,
    /// The database's row, for an edit typed back to its value to be
    /// cleared. `None` for a template the project creates.
    pub(crate) in_database: Option<Template>,
    pub(crate) period: PeriodState,
    /// The route as the client schedules it. `None` when the path has no
    /// points or the template's speed or acceleration is 0.
    pub(crate) schedule: Option<Schedule>,
    pub(crate) stops: Vec<Stop>,
    /// What is wrong with the route, each as a sentence. The ones that stop
    /// the server are first.
    pub(crate) findings: Vec<String>,
}

impl Ship {
    pub(crate) fn created(&self) -> bool {
        self.in_database.is_none()
    }

    /// The length of one round as the client computes it, in milliseconds.
    pub(crate) fn computed(&self) -> Option<u32> {
        self.schedule.as_ref().map(Schedule::period).filter(|period| *period > 0)
    }

    /// Whether the server will use the period the client computes. A ship
    /// with no schedule has nothing to be in step with.
    pub(crate) fn in_step(&self) -> bool {
        match self.computed() {
            Some(computed) => self.period.server() == Some(computed),
            None => true,
        }
    }

    /// Where the ship is `ms` into its round.
    pub(crate) fn placement(&self, ms: f64) -> Option<vale_assets::tables::shiptransport::Placement> {
        let schedule = self.schedule.as_ref()?;
        let period = f64::from(schedule.period());
        if period <= 0.0 {
            return None;
        }
        schedule.at(ms.rem_euclid(period) as u32)
    }
}

/// The clock the panel's schedule and the viewport's ship markers share.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Preview {
    /// Milliseconds since the start of a round.
    pub(crate) ms: f64,
    pub(crate) playing: bool,
    /// How many times faster than the game the clock runs.
    pub(crate) rate: f32,
}

impl Default for Preview {
    fn default() -> Preview {
        Preview { ms: 0.0, playing: false, rate: 10.0 }
    }
}

/// The fields of the panel's Create Transport form.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewShip {
    pub(crate) name: String,
    /// `GameObjectDisplayInfo.dbc` id of the hull.
    pub(crate) display: u32,
    pub(crate) speed: u32,
    pub(crate) accel: u32,
}

impl Default for NewShip {
    fn default() -> NewShip {
        NewShip {
            name: "New transport".to_string(),
            display: HULLS[0].0,
            speed: transport::SPEED,
            accel: transport::ACCEL,
        }
    }
}

/// The two hulls the shipped boats and zeppelins use, by
/// `GameObjectDisplayInfo.dbc` id.
pub(crate) const HULLS: [(u32, &str); 2] = [(3015, "Ship"), (3031, "Zeppelin")];

/// One leg of a route: a run of its points on one map. The ship enters at
/// the leg's first point, sails through the others, and leaves at its last;
/// it is then moved to the start of the next leg, and from the last leg to
/// the first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Leg {
    /// Which of the route's points are the leg's.
    pub(crate) points: std::ops::Range<usize>,
    pub(crate) map: u32,
    /// How many of its points the ship waits at.
    pub(crate) stops: usize,
    /// A place on the leg to look at: its first stop, or else its middle
    /// point.
    pub(crate) at: [f32; 3],
}

/// One route a ship sails or could sail: a path a transport's template
/// names, or one that starts or ends at a node with no mounts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sailing {
    /// The `TaxiPath` id.
    pub(crate) path: u32,
    /// Every point of the path, on every map, in order.
    pub(crate) points: Vec<Point>,
    pub(crate) legs: Vec<Leg>,
}

/// The legs of a route's points; see [`runs`].
pub(crate) fn legs_of(points: &[Point]) -> Vec<Leg> {
    runs(points)
        .into_iter()
        .map(|run| {
            let part = &points[run.clone()];
            let travelled = |n: usize| n > 0 && n + 1 < part.len();
            let stops: Vec<&Point> = part.iter().enumerate().filter(|(n, point)| point.action_flag == 2 && travelled(*n)).map(|(_, point)| point).collect();
            Leg {
                map: part[0].map,
                stops: stops.len(),
                at: stops.first().map_or(part[part.len() / 2].at, |stop| stop.at),
                points: run,
            }
        })
        .collect()
}

/// What a leg being drawn is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Drawing {
    /// A new transport: the leg is its route's first.
    New,
    /// One more leg at the end of a route.
    Leg { path: u32 },
}

/// A leg being drawn, before it is rows: the places the ship sails through
/// on the open map, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Draft {
    pub(crate) what: Drawing,
    /// Each place, and whether the ship waits there.
    pub(crate) points: Vec<(Vec3, bool)>,
    /// The height, in world yards, the next point is placed at. `None` until
    /// the first click, which sets it [`AIR_CLEARANCE`] above the ground
    /// there.
    pub(crate) height: Option<f32>,
}

/// Yards above the first click a leg's height starts at when it is not
/// given: a zeppelin's. A ship's leg starts at 0, the sea's level.
pub(crate) const AIR_CLEARANCE: f32 = 60.0;

/// Seconds a new stop waits, which is what the shipped docks wait.
pub(crate) const DEFAULT_WAIT: u32 = 60;

/// A request to look at a place, opening its map first when it is not the
/// open one. See [`travel`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Go {
    pub(crate) map: u32,
    /// Where to look. `None` leaves the camera where it is over the map.
    pub(crate) at: Option<Vec3>,
}

/// The points a leg drawn through `places` is written as: the places, with
/// a curve handle before the first and after the last, each as far beyond
/// its end as the next place in. A leg is sailed from its second point to
/// its second-last, so the handles are what lets the ship reach every place
/// drawn.
pub(crate) fn leg_points(map: u32, places: &[(Vec3, bool)]) -> Vec<taxi::PointSpec> {
    let (Some(first), Some(last)) = (places.first(), places.last()) else {
        return Vec::new();
    };
    let second = places.get(1).map_or(first.0, |place| place.0);
    let before_last = places.len().checked_sub(2).map_or(last.0, |at| places[at].0);
    let handle = |at: Vec3| taxi::PointSpec::flying(map, at.to_array());
    let mut out = vec![handle(first.0 * 2.0 - second)];
    out.extend(places.iter().map(|(at, stop)| taxi::PointSpec {
        map,
        at: at.to_array(),
        action_flag: if *stop { 2 } else { 0 },
        delay: if *stop { DEFAULT_WAIT } else { 0 },
    }));
    out.push(handle(last.0 * 2.0 - before_last));
    out
}

/// The transport half of the flight path tool's state.
#[derive(Resource, Debug, Default)]
pub(crate) struct Transports {
    /// Every route a ship sails or could sail, on any map, by path id.
    pub(crate) routes: Vec<Sailing>,
    /// The selected route, by path id. It is kept while its map is not the
    /// open one, which the flight path tool's own selection is not.
    pub(crate) route: Option<u32>,
    /// Select [`Self::route`] in the flight path tool when its lists next
    /// hold it: after it was chosen from the list or the map was changed.
    reselect: bool,
    /// The leg being drawn.
    pub(crate) drawing: Option<Draft>,
    /// The toolbar's Finish was pressed; [`keys`] finishes the leg.
    pub(crate) finish: bool,
    /// The model picker a hull is chosen in, when it is open: on a ship's
    /// `displayId`, or on the New Transport form's hull, which is kept and
    /// not written. See `crate::subjects::displays`.
    pub(crate) hull_pick: Option<crate::subjects::displays::DisplayPick>,
    /// The model each hull's display id resolves to, memoised, and the
    /// `GameAssets::tables_generation` it was resolved at.
    models: HashMap<u32, Option<String>>,
    models_at: u64,
    /// A place to go to, taken by [`travel`].
    pub(crate) travel: Option<Go>,
    /// The templates and periods, read at once and kept until an apply moves
    /// `EditSession::database_writes`.
    reads: FreshReads<(), Held>,
    /// The content patch the server loads templates at, read with the rows.
    patch: u32,
    /// Every transport, by entry.
    pub(crate) ships: Vec<Ship>,
    /// What the ships were last built from.
    built: Option<(u64, u64, u64, bool)>,
    /// The session's row revision the ships were last built at.
    rows_at: u64,
    /// Whether each ship was in step when the ships were last built, by
    /// entry. See the module comment.
    settled: HashMap<u32, bool>,
    /// A number that moves when the set of paths the ships sail changes, for
    /// the flight path tool's own lists.
    pub(crate) sailed_revision: u64,
    pub(crate) preview: Preview,
    pub(crate) new_ship: NewShip,
}

impl Transports {
    pub(crate) fn held(&self) -> Option<&Held> {
        self.reads.value()
    }

    /// Why the last read of the database answered nothing, when it did.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.reads.trouble()
    }

    pub(crate) fn ship(&self, entry: u32) -> Option<&Ship> {
        self.ships.iter().find(|ship| ship.template.entry == entry)
    }

    /// The ships that sail a path.
    pub(crate) fn of_path(&self, path: u32) -> impl Iterator<Item = &Ship> {
        self.ships.iter().filter(move |ship| ship.template.path == path)
    }

    /// Build the ships again on the next frame.
    pub(crate) fn stale(&mut self) {
        self.built = None;
    }

    /// The model a hull's `GameObjectDisplayInfo.dbc` id names, memoised: a
    /// card asks on every frame it is drawn.
    pub(crate) fn hull_model(&mut self, assets: &vale_client::assets::GameAssets, display: u32) -> Option<String> {
        let generation = assets.tables_generation();
        if self.models_at != generation {
            self.models.clear();
            self.models_at = generation;
        }
        if display == 0 {
            return None;
        }
        if let Some(had) = self.models.get(&display) {
            return had.clone();
        }
        let found = assets.display_tables().ok().and_then(|tables| tables.game_object(display)).map(|display| display.path);
        self.models.insert(display, found.clone());
        found
    }

    /// Open the model picker on a hull: a ship's, whose choice the picker
    /// writes to the template's `displayId`, or with no ship the New
    /// Transport form's, whose choice is kept for [`Self::set_new_hull`].
    pub(crate) fn pick_hull(&mut self, ship: Option<&Ship>, display: u32) {
        use crate::subjects::displays::{DisplayPick, Table};
        self.hull_pick = Some(match ship {
            Some(ship) => DisplayPick::open(
                Table::Object,
                crate::subjects::quests::ColumnTarget {
                    table: gameobject::TEMPLATE,
                    key: ship.key.clone(),
                    column: "displayId",
                    in_database: ship.in_database.as_ref().map(|held| held.display.to_string()),
                    label: "Edit transport",
                    subject: format!("{} {} displayId", gameobject::TEMPLATE, ship.key.text()),
                },
                display,
            ),
            None => DisplayPick::open_kept(Table::Object, "displayId", display),
        });
    }

    /// Set the New Transport form's hull. A leg being drawn for it with no
    /// point placed yet takes the hull's height: the sea's level for a ship.
    pub(crate) fn set_new_hull(&mut self, display: u32) {
        self.new_ship.display = display;
        let ship = display == HULLS[0].0;
        if let Some(draft) = self.drawing.as_mut().filter(|draft| draft.what == Drawing::New && draft.points.is_empty()) {
            draft.height = ship.then_some(0.0);
        }
    }

    /// One route, by path id.
    pub(crate) fn sailing(&self, path: u32) -> Option<&Sailing> {
        self.routes.iter().find(|route| route.path == path)
    }

    /// Select a route, in the flight path tool too when its points are on
    /// the open map.
    pub(crate) fn select(&mut self, flights: &mut Flightpaths, path: u32) {
        self.route = Some(path);
        self.reselect = true;
        flights.point = None;
        match flights.route(path).is_some() {
            true => flights.select_path(path),
            false => flights.path = None,
        }
    }

    /// Start drawing a new transport's first leg. A ship's is drawn at the
    /// sea's level.
    pub(crate) fn start_new(&mut self) {
        let ship = self.new_ship.display == HULLS[0].0;
        self.drawing = Some(Draft { what: Drawing::New, points: Vec::new(), height: ship.then_some(0.0) });
    }

    /// Start drawing one more leg of a route, at the height of the route's
    /// first leg.
    pub(crate) fn start_leg(&mut self, path: u32) {
        let height = self.sailing(path).and_then(|route| route.points.get(1)).map(|point| point.at[2]);
        self.drawing = Some(Draft { what: Drawing::Leg { path }, points: Vec::new(), height });
    }
}

/// The transports the project leaves: the database's with the project's
/// claims over them, then the ones the project creates. Each is the template,
/// the key its claim is under, and the database's row.
///
/// A template the project creates is listed once, as a creation, when the
/// database also holds it because the creation was applied. A database
/// template the project changes to another type is left out.
pub(crate) fn listed(held: Option<&Held>, edits: &Edits) -> Vec<(Template, Key, Option<Template>)> {
    // Where the database has each row the project claims: the claim's own
    // key unless the project changed the row's id.
    let claims: Vec<(&Key, &RowEdit)> = edits.table(gameobject::TEMPLATE).collect();
    let claim_at = |at: &Key| {
        claims
            .iter()
            .find(|(key, row)| row.from.as_ref().unwrap_or(key) == at)
            .copied()
    };
    let number = |row: &RowEdit, column: &str| {
        row.columns.get(column).and_then(|value| value.trim().parse::<i64>().ok()).map(|value| value.max(0) as u32)
    };
    let name = |row: &RowEdit| row.columns.get("name").map(|value| vale_mangos::sql::unquote(value));
    let mut out: Vec<(Template, Key, Option<Template>)> = Vec::new();
    for template in held.map_or(&[][..], |held| &held.templates) {
        let at = template.key();
        match claim_at(&at) {
            None => out.push((template.clone(), at, Some(template.clone()))),
            Some((_, row)) if row.life != Life::Update => {}
            Some((key, row)) => {
                if number(row, "type").unwrap_or(transport::TYPE) != transport::TYPE {
                    continue;
                }
                let shown = Template {
                    entry: key.first().map_or(template.entry, |entry| entry as u32),
                    patch: key.patch().unwrap_or(template.patch),
                    name: name(row).unwrap_or_else(|| template.name.clone()),
                    display: number(row, "displayId").unwrap_or(template.display),
                    path: number(row, "data0").unwrap_or(template.path),
                    speed: number(row, "data1").unwrap_or(template.speed),
                    accel: number(row, "data2").unwrap_or(template.accel),
                };
                out.push((shown, key.clone(), Some(template.clone())));
            }
        }
    }
    for (key, row) in &claims {
        if row.life != Life::Insert || number(row, "type") != Some(transport::TYPE) {
            continue;
        }
        let Some(entry) = key.first() else { continue };
        out.push((
            Template {
                entry: entry as u32,
                patch: key.patch().unwrap_or(0),
                name: name(row).unwrap_or_default(),
                display: number(row, "displayId").unwrap_or(0),
                path: number(row, "data0").unwrap_or(0),
                speed: number(row, "data1").unwrap_or(0),
                accel: number(row, "data2").unwrap_or(0),
            },
            (*key).clone(),
            None,
        ));
    }
    out.sort_by_key(|(template, _, _)| template.entry);
    out
}

/// A transport's period on each side of an Apply.
pub(crate) fn period_state(held: Option<&Held>, edits: &Edits, entry: u32) -> PeriodState {
    let key = transport::key(entry);
    let claim = edits.row(transport::TABLE, &key);
    let periods = held.map_or(&[][..], |held| &held.periods);
    let creates = claim.is_some_and(|claim| claim.life == Life::Insert);
    let dev = periods.iter().find(|row| row.entry == entry && row.build == transport::DEV_BUILD);
    // The database's rows as they stand without the project's own creation,
    // which it holds once the creation has been applied.
    let own: Vec<Period> = periods
        .iter()
        .filter(|row| row.entry == entry && !(creates && row.build == transport::DEV_BUILD))
        .cloned()
        .collect();
    let removes = claim.is_some_and(|claim| claim.life == Life::Delete);
    let database = own
        .iter()
        .filter(|row| !(removes && row.build == transport::DEV_BUILD))
        .max_by_key(|row| row.build)
        .map(|row| row.period)
        .filter(|period| *period > 0);
    let claimed = claim
        .filter(|claim| claim.life != Life::Delete)
        .and_then(|claim| claim.columns.get("period"))
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|period| *period > 0);
    PeriodState { database, claimed, dev_row_held: dev.is_some() && !creates }
}

/// A path's points as the client's schedule reads them.
fn waypoints(points: &[Point]) -> Vec<TaxiWaypoint> {
    points
        .iter()
        .map(|point| TaxiWaypoint {
            index: point.index,
            map: point.map,
            pos: point.at,
            action_flag: point.action_flag,
            delay: point.delay,
        })
        .collect()
}

/// The schedule of a route sailed at a template's speed and acceleration.
pub(crate) fn schedule(points: &[Point], template: &Template) -> Option<Schedule> {
    let sailed = MoTransport::read(transport::TYPE, &[template.path, template.speed, template.accel])?;
    Schedule::build(&waypoints(points), sailed)
}

/// The runs a route is cut into, as ranges of `points`: a new run starts
/// where the map changes and after a point that carries the teleport flag,
/// which is where the client's schedule cuts a leg.
pub(crate) fn runs(points: &[Point]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for at in 1..points.len() {
        if points[at].map != points[at - 1].map || points[at - 1].action_flag & 1 != 0 {
            out.push(start..at);
            start = at;
        }
    }
    if start < points.len() {
        out.push(start..points.len());
    }
    out
}

/// About how many yards of a leg's curve one straight piece of the drawn
/// line covers, and how many pieces a leg is drawn with at most.
const CURVE_STEP: f32 = 6.0;
const CURVE_PIECES: usize = 24;

/// The line a ship sails along each leg of a path, sampled into short
/// straight pieces: one entry per pair of neighbouring points, on
/// `super::flightpaths::flown`'s terms.
///
/// A ship's curve is not a flight's. Within a run of points on one map the
/// first and the last point are not places the ship goes: they shape the
/// curve at the second and the second-last, and the ship travels from the
/// second to the second-last. Their two legs are given as straight lines, so
/// the points are still drawn joined and can be picked. A pair that spans two
/// runs is empty.
pub(crate) fn sailed(points: &[Point]) -> Vec<Vec<Vec3>> {
    let mut legs: Vec<Vec<Vec3>> = vec![Vec::new(); points.len().saturating_sub(1)];
    for run in runs(points) {
        let places: Vec<[f32; 3]> = points[run.clone()].iter().map(|point| point.at).collect();
        let spline = Spline::new(&places);
        for (j, pair) in places.windows(2).enumerate() {
            let (a, b) = (Vec3::from(pair[0]), Vec3::from(pair[1]));
            let travelled = j >= 1 && j <= spline.segments();
            legs[run.start + j] = match travelled {
                false => vec![a, b],
                true => {
                    let pieces = ((b - a).length() / CURVE_STEP).ceil().clamp(1.0, CURVE_PIECES as f32) as usize;
                    (0..=pieces)
                        .map(|step| Vec3::from(spline.at(j - 1, step as f64 / pieces as f64)))
                        .collect()
                }
            };
        }
    }
    legs
}

/// What is wrong with a route for a ship, each as a sentence: first what
/// stops the server (`vale_mangos::transport::route_faults`), then what the
/// client's schedule does not use. `known_map` answers whether `Map.dbc` has
/// a map.
pub(crate) fn findings(points: &[Point], template: &Template, known_map: impl Fn(u32) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if points.is_empty() {
        out.push(format!("TaxiPath {} has no points; the server stops at startup on a transport with no route", template.path));
        return out;
    }
    let route: Vec<RoutePoint> = points.iter().map(|point| RoutePoint { map: point.map, action_flag: point.action_flag }).collect();
    for fault in transport::route_faults(&route) {
        out.push(format!("The server stops at startup: {fault}."));
    }
    if template.speed == 0 || template.accel == 0 {
        out.push("Speed or acceleration is 0: neither side can schedule the route.".to_string());
    }
    let mut maps: Vec<u32> = points.iter().map(|point| point.map).collect();
    maps.dedup();
    maps.sort_unstable();
    maps.dedup();
    for map in maps.into_iter().filter(|map| !known_map(*map)) {
        out.push(format!("Map {map} is not in Map.dbc."));
    }
    let mut stops = 0;
    for run in runs(points) {
        let part = &points[run.clone()];
        if part.len() < 4 {
            out.push(format!(
                "Points {} to {} are {} point(s) on map {}: the ship travels between the second and the \
                 second-last point of each part of the route, so a part needs four or more.",
                part[0].index,
                part[part.len() - 1].index,
                part.len(),
                part[0].map
            ));
            continue;
        }
        for (n, point) in part.iter().enumerate() {
            if point.action_flag != 2 {
                continue;
            }
            match n == 0 || n + 1 == part.len() {
                true => out.push(format!(
                    "Point {} is a Stop on the first or last point of its part of the route, which the ship does not travel to; the stop is not used.",
                    point.index
                )),
                false => stops += 1,
            }
        }
    }
    if stops == 0 {
        out.push("No point is a Stop: the ship never waits at a dock.".to_string());
    }
    out
}

/// The places a schedule waits at, in the order of the round, each matched
/// to the point it is at.
fn stops_of(schedule: &Schedule, points: &[Point]) -> Vec<Stop> {
    let mut out = Vec::new();
    for leg in schedule.legs() {
        for stop in leg.stops().iter().filter(|stop| stop.delay_ms > 0) {
            let at = leg.spline().along(stop.dist);
            let point = points
                .iter()
                .filter(|point| point.map == leg.map() && point.action_flag == 2)
                .map(|point| (taxi::distance(point.at, at), point.id))
                .filter(|(off, _)| *off < 2.0)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, id)| id);
            out.push(Stop { arrive_ms: stop.time_ms, wait_ms: stop.delay_ms, map: leg.map(), at, point });
        }
    }
    out
}

/// One ship, built from its template, the route's points and the periods.
fn build(
    template: Template,
    key: Key,
    in_database: Option<Template>,
    points: &[Point],
    period: PeriodState,
    known_map: impl Fn(u32) -> bool,
) -> Ship {
    let schedule = schedule(points, &template);
    Ship {
        stops: schedule.as_ref().map_or_else(Vec::new, |schedule| stops_of(schedule, points)),
        findings: findings(points, &template, known_map),
        schedule,
        period,
        in_database,
        key,
        template,
    }
}

/// The claim that states a ship's period: the project's row with the period
/// set, an edit of a row the database holds at build 5875, or else a new row
/// at that build named after the ship.
pub(crate) fn period_row(edits: &Edits, ship: &Ship, period: u32) -> RowEdit {
    let key = transport::key(ship.template.entry);
    match edits.row(transport::TABLE, &key) {
        Some(claim) if claim.life != Life::Delete => {
            let mut row = claim.clone();
            row.columns.insert("period".to_string(), period.to_string());
            row
        }
        _ if ship.period.dev_row_held => {
            let mut row = RowEdit::default();
            row.columns.insert("period".to_string(), period.to_string());
            row
        }
        _ => creation(&Period::new(ship.template.entry, &ship.template.name, period).assignments()),
    }
}

/// Whether an undo entry is about a route or a transport's template: it
/// changes a taxi table, or a row of `gameobject_template`.
fn about_a_route(change: &vale_edit::undo::Change) -> bool {
    change.tables().iter().any(|table| taxi::TABLES.contains(&table.as_str()))
        || change.server_steps(true).any(|entry| entry.table == gameobject::TEMPLATE)
}

fn active(tool: &Tool, state: &crate::playtest::Playtest) -> bool {
    *tool == Tool::Flightpaths && state.editing()
}

/// Read the templates and the periods while the taxi tables are open, and
/// again after the database is written.
fn read_the_rows(
    mut transports: ResMut<Transports>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let Some(session) = session else { return };
    if session.table(taxi::POINTS).is_none() {
        return;
    }
    let writes = session.database_writes;
    if transports.reads.renew(writes) {
        transports.stale();
    }
    match transports.reads.poll() {
        Some(Ok(())) => {
            if let Some(held) = transports.held() {
                info!("transports: {} template(s), {} period row(s)", held.templates.len(), held.periods.len());
            }
            transports.stale();
        }
        Some(Err(e)) => warn!("transports: {e}"),
        None => {}
    }
    if transports.reads.is_read(&()) || transports.reads.is_busy() || transports.reads.trouble().is_some() {
        return;
    }
    let patch = crate::server::settings::server_patch(&settings);
    transports.patch = patch;
    transports.reads.want(writes, (), || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            use vale_mangos::schema::RowValue;
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let templates = db.rows(&transport::templates_query(patch))?.iter().filter_map(Template::from_row).collect();
            let periods = db.rows(&transport::periods_query())?.iter().filter_map(Period::from_row).collect();
            let max_entry = db
                .row(gameobject::MAX_ENTRY_QUERY)?
                .and_then(|row| row.text("entry").and_then(|entry| entry.trim().parse::<u32>().ok()));
            Ok(Held { templates, periods, max_entry })
        }))
    });
}

/// Build the ships when the taxi tables, the project's rows or the reading
/// changed, and write the period of a ship an edit has just taken out of
/// step. See the module comment.
fn collect(
    mut transports: ResMut<Transports>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    let Some(session) = session.as_mut() else { return };
    follow(&mut transports, session, active(&tool, &state));
}

/// [`collect`], given whether the flight path tool is chosen.
fn follow(transports: &mut Transports, session: &mut EditSession, in_tool: bool) {
    if session.table(taxi::POINTS).is_none() {
        return;
    }
    // The project's rows move on every frame of a drag in any tool. They are
    // followed only while this tool is chosen. An edit to a DBC is followed
    // whichever tool made it.
    if in_tool {
        transports.rows_at = session.rows.revision;
    }
    let key = (session.table_revision, transports.rows_at, session.database_writes, transports.held().is_some());
    if transports.built == Some(key) {
        return;
    }
    let read_before = transports.built.is_some_and(|built| built.3 && built.2 == key.2);
    rebuild(transports, session);
    // Once per reading: every ship and whether the server holds its period,
    // which is the check that the schedule here is the one the server was
    // given.
    if key.3 && !read_before {
        for ship in &transports.ships {
            info!(
                "transports: {} {} path {}: period {:?} ms, server {:?} ms{}{}",
                ship.template.entry,
                ship.template.name,
                ship.template.path,
                ship.computed(),
                ship.period.server(),
                match ship.in_step() {
                    true => "",
                    false => ", OUT OF STEP",
                },
                match ship.findings.len() {
                    0 => String::new(),
                    n => format!(", {n} finding(s): {}", ship.findings.join(" | ")),
                }
            );
        }
    }

    let fresh = session.history.next_undo().is_some_and(about_a_route);
    let mut wrote = false;
    for at in 0..transports.ships.len() {
        let ship = &transports.ships[at];
        let entry = ship.template.entry;
        let (Some(computed), false) = (ship.computed(), ship.in_step()) else {
            continue;
        };
        if !fresh || transports.settled.get(&entry) != Some(&true) {
            continue;
        }
        let row = period_row(&session.rows.edits, ship, computed);
        if session.amend_server_row(transport::TABLE, &transport::key(entry), Some(&row)) {
            transports.ships[at].period.claimed = Some(computed);
            wrote = true;
        }
    }
    transports.settled = transports.ships.iter().map(|ship| (ship.template.entry, ship.in_step())).collect();
    if wrote && in_tool {
        transports.rows_at = session.rows.revision;
    }
    transports.built = Some((session.table_revision, transports.rows_at, session.database_writes, transports.held().is_some()));
}

/// The ships, from the reading, the project's rows and the taxi tables.
fn rebuild(transports: &mut Transports, session: &EditSession) {
    let Some(table) = session.table(taxi::POINTS) else { return };
    let by_path = taxi::points_by_path(table);
    let none: Vec<Point> = Vec::new();
    let known_map = |map: u32| session.maps.is_empty() || session.maps.iter().any(|(id, _)| *id == map);
    let before: Vec<u32> = sailed_paths(&transports.ships);
    let held = transports.reads.value();
    let ships: Vec<Ship> = listed(held, &session.rows.edits)
        .into_iter()
        .map(|(template, key, in_database)| {
            let points = by_path.get(&template.path).unwrap_or(&none);
            let period = period_state(held, &session.rows.edits, template.entry);
            build(template, key, in_database, points, period, known_map)
        })
        .collect();
    // The routes: every path a ship sails, and every path with an end at a
    // node that has no mounts, which is a route with no ship yet.
    let mut paths = sailed_paths(&ships);
    if let (Some(nodes), Some(path_table)) = (session.table(taxi::NODES), session.table(taxi::PATHS)) {
        let docks: Vec<u32> = taxi::nodes(nodes).into_iter().filter(|node| node.mounts == [0, 0]).map(|node| node.id).collect();
        paths.extend(
            taxi::paths(path_table)
                .into_iter()
                .filter(|path| docks.contains(&path.from) || docks.contains(&path.to))
                .map(|path| path.id),
        );
    }
    paths.sort_unstable();
    paths.dedup();
    let routes: Vec<Sailing> = paths
        .into_iter()
        .map(|path| {
            let points = by_path.get(&path).cloned().unwrap_or_default();
            Sailing { path, legs: legs_of(&points), points }
        })
        .collect();
    transports.ships = ships;
    transports.routes = routes;
    if sailed_paths(&transports.ships) != before {
        transports.sailed_revision += 1;
    }
}

/// The paths a list of ships sails, in order, each once.
fn sailed_paths(ships: &[Ship]) -> Vec<u32> {
    let mut paths: Vec<u32> = ships.iter().map(|ship| ship.template.path).collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// The database's template as the columns a ship's form writes, for
/// `write_column` to clear an edit typed back to the database's value.
fn held_columns(template: &Template) -> Vec<Assignment> {
    vec![
        Assignment { column: "name", value: vale_mangos::sql::text(&template.name) },
        Assignment { column: "displayId", value: template.display.to_string() },
        Assignment { column: "data0", value: template.path.to_string() },
        Assignment { column: "data1", value: template.speed.to_string() },
        Assignment { column: "data2", value: template.accel.to_string() },
    ]
}

/// Set one column of a ship's template. `value` is a SQL literal. A change
/// to the path, the speed or the acceleration takes the ship out of step,
/// and [`collect`] writes its period on the same undo entry.
pub(crate) fn set_column(session: &mut EditSession, ship: &Ship, column: &'static str, value: String, now: f64) {
    let label = format!("{} {} {column}", gameobject::TEMPLATE, ship.key.text());
    let gesture = Gesture { label: "Edit transport", subject: &label, now };
    let held = ship.in_database.as_ref().map(held_columns);
    write_column(session, gameobject::TEMPLATE, &ship.key, held.as_deref(), column, value, gesture);
}

/// Write a ship's period from its schedule, as an undo entry of its own.
/// Answers the status line.
pub(crate) fn update_period(session: &mut EditSession, transports: &mut Transports, entry: u32, now: f64) -> String {
    let Some(ship) = transports.ship(entry) else {
        return format!("no transport {entry}");
    };
    let Some(computed) = ship.computed() else {
        return "the route has no schedule to take a period from".to_string();
    };
    let key = transport::key(entry);
    let row = period_row(&session.rows.edits, ship, computed);
    let label = format!("{} {}", transport::TABLE, key.text());
    let gesture = Gesture { label: "Update transport period", subject: &label, now };
    session.set_server_row(transport::TABLE, &key, Some(&row), Some(gesture));
    transports.stale();
    format!("transport {entry}: period {computed} ms")
}

/// Make a transport that sails `path`: its template and its period row, as
/// one undo entry. Answers the new entry, or why nothing was made.
pub(crate) fn create(session: &mut EditSession, transports: &mut Transports, path: u32, now: f64) -> Result<u32, String> {
    let new = new_rows(session, transports, path)?;
    let subject = format!("{} {}", gameobject::TEMPLATE, new.entry);
    session.as_one("New transport", &subject, |session| {
        let gesture = Gesture { label: "New transport", subject: &subject, now };
        session.set_server_row(gameobject::TEMPLATE, &new.key, Some(&new.template), Some(gesture));
        if let Some(period) = &new.period {
            session.set_server_row(transport::TABLE, &transport::key(new.entry), Some(period), Some(gesture));
        }
    });
    transports.stale();
    Ok(new.entry)
}

/// The rows a new transport on a path is: its template, and its period when
/// the path's points give a schedule.
struct NewRows {
    entry: u32,
    key: Key,
    template: RowEdit,
    period: Option<RowEdit>,
}

/// The rows of a new transport that sails `path`, from the form's fields,
/// numbered above everything the database and the project hold.
fn new_rows(session: &mut EditSession, transports: &Transports, path: u32) -> Result<NewRows, String> {
    let new = transports.new_ship.clone();
    if new.speed == 0 || new.accel == 0 {
        return Err("a transport needs a speed and an acceleration above 0".to_string());
    }
    let points = session
        .table(taxi::POINTS)
        .map(|table| taxi::points_by_path(table).remove(&path).unwrap_or_default())
        .ok_or_else(|| "TaxiPathNode.dbc is not open".to_string())?;
    let floor = gameobject::NUMBERING.next_entry(transports.held().and_then(|held| held.max_entry), &session.rows.edits);
    let entry = session.numbers.soft_u32(&vale_edit::numbers::row(gameobject::TEMPLATE, "entry"), floor);
    let patch = match transports.held().is_some() {
        true => transports.patch,
        false => vale_mangos::conn::DEFAULT_WOW_PATCH,
    };
    let template = Template {
        entry,
        patch,
        name: new.name.clone(),
        display: new.display,
        path,
        speed: new.speed,
        accel: new.accel,
    };
    let period = schedule(&points, &template).map(|schedule| schedule.period()).filter(|period| *period > 0);
    Ok(NewRows {
        entry,
        key: template.key(),
        template: creation(&transport::new_template(&new.name, new.display, path, new.speed, new.accel)),
        period: period.map(|period| creation(&Period::new(entry, &new.name, period).assignments())),
    })
}

/// Join a later operation's edits to an earlier one's, so the two are one
/// undo entry. Both add records only, so the order within a table holds.
fn joined(mut done: taxi::Edits, more: taxi::Edits) -> taxi::Edits {
    done.rows.extend(more.rows);
    done.cells.extend(more.cells);
    done.made = more.made.or(done.made);
    done
}

/// Make a transport from the leg drawn: a node for its route to start at,
/// the path through [`leg_points`], the template and the period row, as one
/// undo entry. Answers the path and the entry.
///
/// A path needs two different nodes. The first is a new node with no mounts
/// at the leg's first place, named after the transport. The second is any
/// node with no mounts whose name starts with `Generic`, as the shipped
/// zeppelin routes end at one; one is made when the table has none. A ship
/// goes to neither.
fn new_transport(
    session: &mut EditSession,
    flights: &mut Flightpaths,
    transports: &mut Transports,
    places: &[(Vec3, bool)],
) -> Result<(u32, u32), String> {
    let map = session.map_id;
    let name = transports.new_ship.name.clone();
    let specs = leg_points(map, places);
    let first = places.first().map(|place| place.0.to_array()).ok_or("no points were placed")?;
    let generic = taxi::nodes(session.table(taxi::NODES).ok_or("TaxiNodes.dbc is not open")?)
        .into_iter()
        .find(|node| node.mounts == [0, 0] && node.name.starts_with("Generic"))
        .map(|node| node.id);
    let mut done = taxi::new_node(&mut session.tables, &mut session.numbers, map, first, &format!("Transport, {name}"), [0, 0])
        .map_err(|e| e.to_string())?;
    // From here every failure still records what was made, so it can be
    // undone.
    let made = (|| -> Result<u32, String> {
        let from = done.made.ok_or("the node was given no id")?;
        let to = match generic {
            Some(id) => id,
            None => {
                let more = taxi::new_node(&mut session.tables, &mut session.numbers, map, first, "Generic, transport route end", [0, 0])
                    .map_err(|e| e.to_string())?;
                let id = more.made.ok_or("the node was given no id")?;
                done = joined(std::mem::take(&mut done), more);
                id
            }
        };
        let more = taxi::new_path(&mut session.tables, &mut session.numbers, from, to, 0, &specs).map_err(|e| e.to_string())?;
        let path = more.made.ok_or("the path was given no id")?;
        done = joined(std::mem::take(&mut done), more);
        Ok(path)
    })();
    super::flightpaths::record(session, "New transport", done);
    flights.stale();
    transports.stale();
    let path = made?;
    let new = new_rows(session, transports, path)?;
    session.amend_server_row(gameobject::TEMPLATE, &new.key, Some(&new.template));
    if let Some(period) = &new.period {
        session.amend_server_row(transport::TABLE, &transport::key(new.entry), Some(period));
    }
    Ok((path, new.entry))
}

/// Add the leg drawn to the end of a route, on the open map, as one undo
/// entry. The route's period follows through [`follow`].
fn add_leg(
    session: &mut EditSession,
    flights: &mut Flightpaths,
    transports: &mut Transports,
    path: u32,
    places: &[(Vec3, bool)],
) -> Result<(), String> {
    let map = session.map_id;
    let last = transports.sailing(path).and_then(|route| route.points.last()).map(|point| point.map);
    if last == Some(map) {
        return Err("the route's last leg is already on this map; open the map the next leg is on, or add points to the leg here".to_string());
    }
    let done = taxi::append_points(&mut session.tables, &mut session.numbers, path, &leg_points(map, places)).map_err(|e| e.to_string())?;
    super::flightpaths::record(session, "Add transport leg", done);
    flights.stale();
    transports.stale();
    Ok(())
}

/// Finish the leg being drawn: a new transport, or one more leg of a route.
/// Answers the status line. A leg of fewer than two places is not made, and
/// stays being drawn.
pub(crate) fn finish(session: &mut EditSession, flights: &mut Flightpaths, transports: &mut Transports) -> String {
    let Some(draft) = transports.drawing.clone() else {
        return "no leg is being drawn".to_string();
    };
    if draft.points.len() < 2 {
        return "place at least two points for the ship to sail between".to_string();
    }
    let done = match draft.what {
        Drawing::New => new_transport(session, flights, transports, &draft.points).map(|(path, entry)| {
            transports.select(flights, path);
            format!("transport {entry} made on path {path}; set where it waits on its points")
        }),
        Drawing::Leg { path } => add_leg(session, flights, transports, path, &draft.points).map(|()| {
            transports.select(flights, path);
            format!("leg of {} points added to path {path}", draft.points.len())
        }),
    };
    match done {
        Ok(line) => {
            transports.drawing = None;
            line
        }
        Err(why) => why,
    }
}

/// Remove one leg of a route, as one undo entry. Refused for a route's only
/// leg. Answers the status line.
pub(crate) fn remove_leg(session: &mut EditSession, flights: &mut Flightpaths, transports: &mut Transports, path: u32, leg: usize) -> String {
    let Some(route) = transports.sailing(path) else {
        return format!("no route {path}");
    };
    let Some(range) = route.legs.get(leg).map(|leg| leg.points.clone()) else {
        return format!("route {path} has no leg {}", leg + 1);
    };
    if route.legs.len() < 2 {
        return "a route keeps one leg; remove the route to remove it".to_string();
    }
    let first = route.points[range.start].index;
    match taxi::remove_points(&mut session.tables, path, first, range.len() as u32) {
        Ok(done) => {
            super::flightpaths::record(session, "Remove transport leg", done);
            flights.point = None;
            flights.stale();
            transports.stale();
            format!("leg {} of path {path} removed", leg + 1)
        }
        Err(e) => e.to_string(),
    }
}

/// Remove a route with no transport, or one whose transports this project
/// creates, with them, as one undo entry. Refused while a transport the
/// database holds names it: that template would then name a path with no
/// points, which stops the server. Answers the status line.
pub(crate) fn remove_route(session: &mut EditSession, flights: &mut Flightpaths, transports: &mut Transports, path: u32) -> String {
    let ships: Vec<Ship> = transports.of_path(path).cloned().collect();
    if let Some(held) = ships.iter().find(|ship| !ship.created()) {
        return format!("transport {} is the database's and sails this route; the route is kept", held.template.entry);
    }
    match taxi::remove_path(&mut session.tables, path) {
        Ok(done) => {
            super::flightpaths::record(session, "Remove transport route", done);
            for ship in &ships {
                session.amend_server_row(transport::TABLE, &transport::key(ship.template.entry), None);
                session.amend_server_row(gameobject::TEMPLATE, &ship.key, None);
            }
            transports.route = None;
            flights.path = None;
            flights.point = None;
            flights.stale();
            transports.stale();
            format!("route {path} removed")
        }
        Err(e) => e.to_string(),
    }
}

/// How far a curve handle may be from the place [`leg_points`] would put it
/// and still be moved with its neighbours, in yards.
const HANDLE_SLACK: f32 = 1.0;

/// Move a leg's curve handle with the point beside it.
///
/// A leg's first and last point are curve handles. One that stands where
/// [`leg_points`] would put it, as far beyond the leg's end as the next point
/// in, is kept there when either of those two points is moved to `to`, on the
/// move's own undo entry. A handle that has been placed by hand is left.
pub(crate) fn carry_handles(session: &mut EditSession, flights: &Flightpaths, id: u32, to: Vec3, now: f64) {
    use vale_assets::tables::taxi::path_node_fields as wf;
    let Some((route, _)) = flights.point(id) else { return };
    if !route.transport {
        return;
    }
    let points = &route.points;
    let Some(moved) = points.iter().position(|point| point.id == id) else { return };
    let at = |index: usize| Vec3::from(points[index].at);
    let mut writes: Vec<(usize, Vec3)> = Vec::new();
    for run in runs(points) {
        if !run.contains(&moved) || run.len() < 4 {
            continue;
        }
        for (handle, end, inner) in [(run.start, run.start + 1, run.start + 2), (run.end - 1, run.end - 2, run.end - 3)] {
            if moved != end && moved != inner {
                continue;
            }
            if (at(handle) - (at(end) * 2.0 - at(inner))).length() > HANDLE_SLACK {
                continue;
            }
            let now_at = |index: usize| if index == moved { to } else { at(index) };
            writes.push((points[handle].record, now_at(end) * 2.0 - now_at(inner)));
        }
    }
    for (record, place) in writes {
        crate::tables::fields::set_fields(
            session,
            taxi::POINTS,
            record,
            &[(wf::X, place.x.to_bits()), (wf::Y, place.y.to_bits()), (wf::Z, place.z.to_bits())],
            "Move flight point",
            &format!("TaxiPathNode {id} position"),
            now,
        );
    }
}

/// Run the preview clock.
fn advance(mut transports: ResMut<Transports>, time: Res<Time>, tool: Res<Tool>, state: Res<crate::playtest::Playtest>) {
    if !active(&tool, &state) || !transports.preview.playing {
        return;
    }
    let step = f64::from(time.delta_secs()) * f64::from(transports.preview.rate) * 1000.0;
    // Wrapped at a day, which is past every period, so the clock stays exact.
    transports.preview.ms = (transports.preview.ms + step).rem_euclid(86_400_000.0);
}

/// Keep the route selected here and the path selected in the flight path
/// tool the same, while the transports half is shown.
///
/// A click on a route in the world selects the path there, and this takes it
/// as the route. A route chosen from the list, or kept across a change of
/// map, is selected there once the tool's lists hold it. A path deselected
/// there with Escape deselects the route, unless the route is on another
/// map, where it stays selected so its card stays open.
fn sync(mut transports: ResMut<Transports>, mut flights: ResMut<Flightpaths>, tool: Res<Tool>, state: Res<crate::playtest::Playtest>) {
    if !active(&tool, &state) || !flights.transports {
        return;
    }
    match (flights.path, transports.route) {
        (Some(path), route) if route != Some(path) => {
            transports.route = Some(path);
            transports.reselect = false;
        }
        (Some(_), _) => transports.reselect = false,
        (None, Some(route)) if flights.route(route).is_some() => match transports.reselect {
            true => {
                flights.select_path(route);
                transports.reselect = false;
            }
            false => transports.route = None,
        },
        _ => {}
    }
}

/// `--transport <entry>`: once the ships are read, show the transports half,
/// select that transport's route and go to its first leg.
fn scripted(
    mut transports: ResMut<Transports>,
    mut flights: ResMut<Flightpaths>,
    args: Res<crate::args::Args>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut done: Local<bool>,
) {
    let Some(entry) = args.tools.transport else { return };
    if *done || !active(&tool, &state) {
        return;
    }
    // The database's ships are listed once its reading has arrived, and the
    // project's own without one.
    let read = transports.built.is_some_and(|built| built.3) || (transports.built.is_some() && transports.trouble().is_some());
    if !read {
        return;
    }
    *done = true;
    flights.transports = true;
    let Some(path) = transports.ship(entry).map(|ship| ship.template.path) else {
        warn!("--transport {entry}: no transport has that entry");
        return;
    };
    transports.select(&mut flights, path);
    if args.tools.pick_hull {
        if let Some(ship) = transports.ship(entry).cloned() {
            transports.pick_hull(Some(&ship), ship.template.display);
        }
    }
    let legs = transports.sailing(path).map(|route| route.legs.clone()).unwrap_or_default();
    info!(
        "--transport: {entry} on path {path}, {} leg(s) on map(s) {:?}",
        legs.len(),
        legs.iter().map(|leg| leg.map).collect::<Vec<_>>()
    );
    if let Some(leg) = legs.first() {
        transports.travel = Some(Go { map: leg.map, at: Some(leg.at.into()) });
    }
}

/// Go where [`Transports::travel`] asks: open the map when it is not the
/// open one, and put the camera over the place. The selected route is kept
/// and selected again on the new map.
fn travel(
    mut transports: ResMut<Transports>,
    mut session: Option<ResMut<EditSession>>,
    mut camera: ResMut<crate::camera::EditorCamera>,
) {
    let Some(go) = transports.travel.take() else { return };
    let Some(session) = session.as_mut() else { return };
    if go.map != session.map_id {
        let Some((id, name)) = session.maps.iter().find(|(id, _)| *id == go.map).cloned() else {
            session.status = format!("map {} is not in Map.dbc", go.map);
            return;
        };
        session.switch_map(name, id, &mut camera);
        transports.reselect = true;
    }
    if let Some(at) = go.at {
        // The height is kept: a zeppelin's stop is far above the ground.
        camera.target = at;
        camera.wants_the_ground = false;
    }
}

/// A click while a leg is being drawn: place the next point above the click,
/// at the draft's height. With Shift held the ship waits there.
#[allow(clippy::too_many_arguments)]
fn press(
    mut transports: ResMut<Transports>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cursor: Res<crate::pick::Cursor>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let (Some(draft), Some(surface)) = (transports.drawing.as_mut(), cursor.surface) else {
        return;
    };
    let height = *draft.height.get_or_insert(surface.z + AIR_CLEARANCE);
    let stop = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    draft.points.push((surface.truncate().extend(height), stop));
    if let Some(session) = session.as_mut() {
        session.status = match stop {
            true => format!("stop placed, point {}", draft.points.len()),
            false => format!("point {} placed", draft.points.len()),
        };
    }
}

/// The keys of a leg being drawn: Enter finishes it, Backspace removes the
/// last point, Escape discards it. The toolbar's Finish button asks through
/// [`Transports::finish`], since the toolbar has no session.
fn keys(
    mut transports: ResMut<Transports>,
    mut flights: ResMut<Flightpaths>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut session: Option<ResMut<EditSession>>,
) {
    let asked = std::mem::take(&mut transports.finish);
    if !active(&tool, &state) || transports.drawing.is_none() {
        return;
    }
    let Some(session) = session.as_mut() else { return };
    if asked {
        session.status = finish(session, &mut flights, &mut transports);
        return;
    }
    if wants.wants_keyboard_input() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        transports.drawing = None;
        session.status = "the leg was discarded".to_string();
    } else if keys.just_pressed(KeyCode::Backspace) {
        if let Some(draft) = transports.drawing.as_mut() {
            draft.points.pop();
            session.status = format!("{} point(s) placed", draft.points.len());
        }
    } else if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        session.status = finish(session, &mut flights, &mut transports);
    }
}

/// Draw the leg being drawn: its points joined in order, a stop larger and
/// green, and the point the next click places.
fn draw_draft(
    mut marks: ResMut<Marks>,
    transports: Res<Transports>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    cursor: Res<crate::pick::Cursor>,
) {
    if !active(&tool, &state) {
        return;
    }
    let Some(draft) = transports.drawing.as_ref() else { return };
    let bevy = |p: Vec3| axes::to_bevy(p.to_array());
    let drawn = Color::srgb(0.5, 1.0, 0.5);
    for pair in draft.points.windows(2) {
        marks.line(bevy(pair[0].0), bevy(pair[1].0), drawn, Look::Ghosted);
    }
    for (at, stop) in &draft.points {
        let (size, colour) = match stop {
            true => (2.4, Color::srgb(0.4, 0.9, 0.5)),
            false => (1.2, drawn),
        };
        let radius = marks.visible_radius(bevy(*at), size);
        marks.sphere(bevy(*at), radius, colour, Look::Ghosted);
    }
    let Some(surface) = cursor.surface else { return };
    let next = surface.truncate().extend(draft.height.unwrap_or(surface.z + AIR_CLEARANCE));
    let faint = Color::srgb(1.0, 0.85, 0.5);
    if let Some((last, _)) = draft.points.last() {
        marks.line(bevy(*last), bevy(next), faint.with_alpha(0.7), Look::Ghosted);
    }
    let radius = marks.visible_radius(bevy(next), 1.2);
    marks.sphere(bevy(next), radius, faint, Look::Ghosted);
    marks.line(bevy(next), bevy(surface), Color::WHITE.with_alpha(0.5), Look::Ghosted);
}

/// How long a ship's marker is, in yards: about the length of the shipped
/// hulls.
const MARKER_LENGTH: f32 = 40.0;

/// Draw each ship of the open map where the preview clock puts it, and the
/// selected path's stops.
///
/// A ship is a cone pointing the way it travels. The ships of the selected
/// path are always drawn; the others are drawn with the transport routes,
/// when Show Transports is on.
fn draw(
    mut marks: ResMut<Marks>,
    transports: Res<Transports>,
    flights: Res<Flightpaths>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !active(&tool, &state) {
        return;
    }
    if !flights.transports {
        return;
    }
    let bevy = |p: [f32; 3]| axes::to_bevy(p);
    for ship in &transports.ships {
        let selected = transports.route == Some(ship.template.path);
        let Some(placed) = ship.placement(transports.preview.ms).filter(|placed| placed.map == flights.map) else {
            continue;
        };
        // The stored facing is half a turn from the direction of travel.
        let ahead = Vec3::new(-placed.facing.cos(), -placed.facing.sin(), 0.0);
        let at = Vec3::from(placed.pos);
        let colour = match selected {
            true => Color::srgb(1.0, 0.9, 0.3),
            false => Color::srgba(0.8, 0.8, 0.9, 0.8),
        };
        let stern = at - ahead * MARKER_LENGTH * 0.5;
        let bow = at + ahead * MARKER_LENGTH * 0.5;
        marks.cone(bevy(stern.to_array()), bevy(bow.to_array()), MARKER_LENGTH * 0.2, colour, Look::Ghosted);
        let mast = at + Vec3::Z * MARKER_LENGTH * 0.5;
        marks.line(bevy(at.to_array()), bevy(mast.to_array()), colour, Look::Ghosted);
    }
}

pub(crate) struct TransportPlugin;

impl Plugin for TransportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Transports>()
            .add_systems(
                Update,
                (read_the_rows, collect, scripted, travel, sync, press, keys, advance, draw, draw_draft)
                    .chain()
                    .after(crate::pick::aim),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(entry: u32, path: u32) -> Template {
        Template { entry, patch: 0, name: format!("Ship {entry}"), display: 3015, path, speed: 30, accel: 1 }
    }

    fn point(index: u32, map: u32, x: f32, flag: u32, delay: u32) -> Point {
        Point { id: 100 + index, record: index as usize, path: 1, index, map, at: [x, 0.0, 0.0], action_flag: flag, delay }
    }

    /// Twelve points along x on one map, 100 yards apart, with a stop at the
    /// third and the tenth.
    fn loop_of_twelve() -> Vec<Point> {
        (0..12)
            .map(|i| {
                let stop = i == 2 || i == 9;
                point(i, 0, i as f32 * 100.0, if stop { 2 } else { 0 }, if stop { 60 } else { 0 })
            })
            .collect()
    }

    /// The database's transports are listed with the project's column edits
    /// over them, a created template is listed once, and a template changed
    /// to another type is left out.
    #[test]
    fn the_ships_are_the_databases_with_the_projects_claims_over_them() {
        let held = Held { templates: vec![template(20808, 241), template(164871, 302)], ..Held::default() };
        let mut edits = Edits::default();
        edits.set(gameobject::TEMPLATE, &template(20808, 241).key(), "data1", Some("45".into()));
        edits.set(gameobject::TEMPLATE, &template(164871, 302).key(), "type", Some("5".into()));
        let made = creation(&transport::new_template("Test Ferry", 3031, 900, 20, 2));
        edits.set_row_line(gameobject::TEMPLATE, &gameobject::template_key(2_000_000, 10), Some(&made.to_line()));
        let ships = listed(Some(&held), &edits);
        assert_eq!(ships.len(), 2);
        assert_eq!((ships[0].0.entry, ships[0].0.speed, ships[0].0.path), (20808, 45, 241));
        assert!(ships[0].2.is_some());
        let created = &ships[1];
        assert_eq!((created.0.entry, created.0.patch, created.0.path), (2_000_000, 10, 900));
        assert_eq!((created.0.name.as_str(), created.0.display, created.0.speed, created.0.accel), ("Test Ferry", 3031, 20, 2));
        assert!(created.2.is_none());

        // Once the creation is applied the database holds it too.
        let held = Held { templates: vec![Template { entry: 2_000_000, patch: 10, ..template(0, 900) }], ..Held::default() };
        let ships = listed(Some(&held), &edits);
        assert_eq!(ships.iter().filter(|ship| ship.0.entry == 2_000_000).count(), 1);
        // With no reading, the project's own creation is still listed.
        assert_eq!(listed(None, &edits).len(), 1);
    }

    /// The server's period is the highest build's, and the project's row at
    /// build 5875 is read in place of it.
    #[test]
    fn the_projects_period_is_read_over_the_databases() {
        let held = Held {
            periods: vec![
                Period { entry: 20808, build: 0, name: String::new(), period: 339_575 },
                Period { entry: 20808, build: 4695, name: String::new(), period: 350_818 },
            ],
            ..Held::default()
        };
        let mut edits = Edits::default();
        let state = period_state(Some(&held), &edits, 20808);
        assert_eq!((state.database, state.claimed, state.server()), (Some(350_818), None, Some(350_818)));
        let row = creation(&Period::new(20808, "Ratchet", 351_000).assignments());
        edits.set_row_line(transport::TABLE, &transport::key(20808), Some(&row.to_line()));
        let state = period_state(Some(&held), &edits, 20808);
        assert_eq!((state.claimed, state.server(), state.dev_row_held), (Some(351_000), Some(351_000), false));
        // Applied, the database holds the project's row; it is still the
        // project's creation and not the database's own.
        let mut applied = held.clone();
        applied.periods.push(Period::new(20808, "Ratchet", 351_000));
        let state = period_state(Some(&applied), &edits, 20808);
        assert_eq!((state.database, state.dev_row_held), (Some(350_818), false));
        assert_eq!(period_state(None, &Edits::default(), 7).server(), None);
    }

    /// A ship whose route gives the period the server holds is in step, and
    /// moving a point takes it out of step. The claim that puts it back is a
    /// new row at build 5875 the first time and the same row after.
    #[test]
    fn a_moved_point_takes_a_ship_out_of_step() {
        let points = loop_of_twelve();
        let template = template(20808, 1);
        let computed = schedule(&points, &template).expect("a schedule").period();
        assert!(computed > 120_000, "two waits of a minute and the travel: {computed}");
        let held = Held { periods: vec![Period { entry: 20808, build: 0, name: String::new(), period: computed }], ..Held::default() };
        let mut edits = Edits::default();
        let state = period_state(Some(&held), &edits, 20808);
        let ship = build(template.clone(), template.key(), Some(template.clone()), &points, state, |_| true);
        assert!(ship.in_step());
        assert_eq!(ship.stops.len(), 2);
        assert_eq!(ship.stops[0].point, Some(102));

        let mut moved = points.clone();
        moved[5].at[1] = 80.0;
        let ship = build(template.clone(), template.key(), Some(template.clone()), &moved, state, |_| true);
        assert!(!ship.in_step());
        let longer = ship.computed().expect("a schedule");
        assert!(longer > computed);
        let row = period_row(&edits, &ship, longer);
        assert_eq!(row.life, Life::Insert);
        assert_eq!(row.columns.get("period").map(String::as_str), Some(longer.to_string().as_str()));
        edits.set_row_line(transport::TABLE, &transport::key(20808), Some(&row.to_line()));
        let state = period_state(Some(&held), &edits, 20808);
        let ship = build(template.clone(), template.key(), Some(template), &moved, state, |_| true);
        assert!(ship.in_step());
        let again = period_row(&edits, &ship, longer + 5);
        assert_eq!(again.life, Life::Insert);
        assert_eq!(again.columns.get("name"), row.columns.get("name"));
    }

    /// A route is cut where the map changes and after a teleport, and a ship's
    /// curve is drawn between the second and the second-last point of each
    /// part.
    #[test]
    fn a_ships_line_is_drawn_between_the_inner_points_of_each_part() {
        let mut points = loop_of_twelve();
        for point in points.iter_mut().skip(6) {
            point.map = 1;
        }
        points[5].action_flag = 1;
        assert_eq!(runs(&points), vec![0..6, 6..12]);
        let legs = sailed(&points);
        assert_eq!(legs.len(), 11);
        // The first and last leg of a part are straight lines to its handles.
        assert_eq!(legs[0].len(), 2);
        assert_eq!(legs[4].len(), 2);
        // The inner legs are sampled curves from point to point.
        assert!(legs[1].len() > 2);
        assert!((legs[1][0] - Vec3::from(points[1].at)).length() < 1e-2);
        assert!((*legs[1].last().unwrap() - Vec3::from(points[2].at)).length() < 1e-2);
        // The pair that spans the two maps has no line.
        assert!(legs[5].is_empty());
        assert!(legs[7].len() > 2);
    }

    #[test]
    fn a_route_is_checked_for_the_server_and_for_the_schedule() {
        let fine = findings(&loop_of_twelve(), &template(1, 1), |_| true);
        assert!(fine.is_empty(), "{fine:?}");
        // No stop, an unknown map, and a part too short to travel.
        let short: Vec<Point> = (0..3).map(|i| point(i, 9, i as f32 * 100.0, 0, 0)).collect();
        let found = findings(&short, &template(1, 1), |map| map != 9);
        assert!(found.iter().any(|line| line.contains("Map 9")), "{found:?}");
        assert!(found.iter().any(|line| line.contains("four or more")), "{found:?}");
        assert!(found.iter().any(|line| line.contains("never waits")), "{found:?}");
        // A route the server cannot read is named first.
        let found = findings(&short[..2], &template(1, 1), |_| true);
        assert!(found[0].starts_with("The server stops at startup"), "{found:?}");
        assert_eq!(findings(&[], &template(1, 7), |_| true).len(), 1);
        // A stop on a part's first point is not used.
        let mut ends = loop_of_twelve();
        ends[0].action_flag = 2;
        let found = findings(&ends, &template(1, 1), |_| true);
        assert!(found.iter().any(|line| line.contains("Point 0 is a Stop")), "{found:?}");
    }

    /// A `TaxiPathNode.dbc` holding `points`.
    fn points_table(points: &[Point]) -> vale_edit::dbc::DbcFile {
        use vale_assets::tables::taxi::path_node_fields as wf;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        bytes.extend_from_slice(&(points.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&9u32.to_le_bytes());
        bytes.extend_from_slice(&36u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for point in points {
            let mut row = [0u32; 9];
            row[wf::ID] = point.id;
            row[wf::PATH] = point.path;
            row[wf::INDEX] = point.index;
            row[wf::MAP] = point.map;
            row[wf::X] = point.at[0].to_bits();
            row[wf::Y] = point.at[1].to_bits();
            row[wf::Z] = point.at[2].to_bits();
            row[wf::ACTION_FLAG] = point.action_flag;
            row[wf::DELAY] = point.delay;
            for field in row {
                bytes.extend_from_slice(&field.to_le_bytes());
            }
        }
        bytes.push(0);
        vale_edit::dbc::DbcFile::parse(&bytes).expect("a table this test builds")
    }

    /// A point moved in a route takes its ship out of step, and the period
    /// is written on the same undo entry: one entry holds the table's cell and
    /// the `transports` row, and taking it back takes back both. An edit made
    /// before the ship was ever in step, or one that is not about a route,
    /// writes nothing.
    #[test]
    fn a_route_edit_and_its_period_are_one_undo_entry() {
        use vale_assets::tables::taxi::path_node_fields as wf;
        let install = std::env::temp_dir().join(format!("vale-transport-follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Follow").unwrap());
        let points = loop_of_twelve();
        session.tables.insert(taxi::POINTS.to_string(), points_table(&points));
        let ship = template(20808, 1);
        let shipped = schedule(&points, &ship).expect("a schedule").period();
        let mut transports = Transports::default();
        transports.reads.insert(
            (),
            Held {
                templates: vec![ship.clone()],
                periods: vec![Period { entry: 20808, build: 0, name: "Test".to_string(), period: shipped }],
                max_entry: Some(182_121),
            },
        );
        follow(&mut transports, &mut session, true);
        assert_eq!(transports.ships.len(), 1);
        assert!(transports.ships[0].in_step());
        assert!(session.rows.edits.is_empty());

        // An edit that is not about a route, then a moved point.
        crate::tables::fields::set_fields(&mut session, taxi::POINTS, 5, &[(wf::Y, 80.0f32.to_bits())], "Move flight point", "TaxiPathNode 105 position", 1.0);
        let entries = session.history.depth_done();
        follow(&mut transports, &mut session, true);
        assert_eq!(session.history.depth_done(), entries, "the period is on the edit's own entry");
        let longer = transports.ships[0].computed().expect("a schedule");
        assert!(longer > shipped);
        assert!(transports.ships[0].in_step());
        let key = transport::key(20808);
        let claim = session.rows.edits.row(transport::TABLE, &key).expect("the period row").clone();
        assert_eq!(claim.life, Life::Insert);
        assert_eq!(claim.columns.get("period").map(String::as_str), Some(longer.to_string().as_str()));
        let top = session.history.next_undo().expect("the edit");
        assert_eq!(top.tables(), vec![taxi::POINTS.to_string()]);
        assert_eq!(top.server_steps(true).count(), 1);

        // Undone, the row goes with the point.
        let change = session.history.undo().expect("the edit");
        session.rows.step(&change, false);
        assert!(session.rows.edits.row(transport::TABLE, &key).is_none());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A ship that is out of step when it is first seen is left for the
    /// panel's button: no edit made it so, and there is no entry to fold a
    /// write into.
    #[test]
    fn a_ship_first_seen_out_of_step_is_not_written() {
        let install = std::env::temp_dir().join(format!("vale-transport-seen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Seen").unwrap());
        session.tables.insert(taxi::POINTS.to_string(), points_table(&loop_of_twelve()));
        let mut transports = Transports::default();
        transports.reads.insert(
            (),
            Held {
                templates: vec![template(20808, 1)],
                periods: vec![Period { entry: 20808, build: 0, name: "Test".to_string(), period: 1_000 }],
                max_entry: None,
            },
        );
        follow(&mut transports, &mut session, true);
        assert!(!transports.ships[0].in_step());
        assert!(session.rows.edits.is_empty());
        // The button writes it, as an entry of its own.
        let line = update_period(&mut session, &mut transports, 20808, 1.0);
        assert!(line.contains("period"), "{line}");
        assert_eq!(session.history.depth_done(), 1);
        follow(&mut transports, &mut session, true);
        assert!(transports.ships[0].in_step());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A new transport is its template and its period row, on one undo
    /// entry, and removing it takes both claims back.
    #[test]
    fn a_new_transport_is_a_template_and_a_period() {
        let install = std::env::temp_dir().join(format!("vale-transport-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "New").unwrap());
        let points = loop_of_twelve();
        session.tables.insert(taxi::POINTS.to_string(), points_table(&points));
        let mut transports = Transports::default();
        transports.new_ship.name = "Test Ferry".to_string();
        let entry = create(&mut session, &mut transports, 1, 1.0).expect("a transport");
        assert!(entry >= gameobject::RESERVED_ENTRY_BASE);
        assert_eq!(session.history.depth_done(), 1);
        follow(&mut transports, &mut session, true);
        let ship = transports.ship(entry).expect("the ship").clone();
        assert!(ship.created() && ship.in_step());
        assert_eq!(ship.template.name, "Test Ferry");
        assert_eq!(ship.period.claimed, schedule(&points, &ship.template).map(|schedule| schedule.period()));
        let plan = crate::server::subject::plan_of(&crate::server::gameobjects::GameObjects, &session.rows.edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.counts(), (2, 0, 0));

        let _ = std::fs::remove_dir_all(&install);
    }

    /// A `TaxiNodes.dbc` and a `TaxiPath.dbc` with no rows.
    fn empty_table(fields: usize) -> vale_edit::dbc::DbcFile {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(fields as u32).to_le_bytes());
        bytes.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(0);
        vale_edit::dbc::DbcFile::parse(&bytes).expect("a table this test builds")
    }

    /// A leg drawn through places is those places with a curve handle before
    /// the first and after the last, each as far out as the next place in,
    /// and a stop waits the default time.
    #[test]
    fn a_drawn_leg_gets_a_handle_at_each_end() {
        let places = [(Vec3::new(0.0, 0.0, 5.0), false), (Vec3::new(100.0, 0.0, 5.0), true), (Vec3::new(200.0, 50.0, 5.0), false)];
        let points = leg_points(7, &places);
        assert_eq!(points.len(), 5);
        assert_eq!(points[0].at, [-100.0, 0.0, 5.0]);
        assert_eq!(points[4].at, [300.0, 100.0, 5.0]);
        assert!(points.iter().all(|point| point.map == 7));
        assert_eq!((points[2].action_flag, points[2].delay), (2, DEFAULT_WAIT));
        assert_eq!((points[1].action_flag, points[0].action_flag, points[4].action_flag), (0, 0, 0));
        assert!(leg_points(7, &[]).is_empty());
    }

    /// A new transport drawn on one map and given a leg on another is one
    /// route of two legs with its ship, each step one undo entry, and its
    /// period follows the second leg. Removing a leg and then the route takes
    /// the points and the project's rows back.
    #[test]
    fn a_transport_is_drawn_a_leg_at_a_time() {
        use vale_assets::tables::taxi::{node_fields as nf, path_fields as pf, path_node_fields as wf};
        let install = std::env::temp_dir().join(format!("vale-transport-legs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Legs").unwrap());
        session.tables.insert(taxi::NODES.to_string(), empty_table(nf::COUNT));
        session.tables.insert(taxi::PATHS.to_string(), empty_table(pf::COUNT));
        session.tables.insert(taxi::POINTS.to_string(), empty_table(wf::COUNT));
        session.map_id = 0;
        let mut flights = Flightpaths::default();
        flights.transports = true;
        let mut transports = Transports::default();
        transports.new_ship.name = "Test Ferry".to_string();

        transports.start_new();
        assert_eq!(transports.drawing.as_ref().and_then(|draft| draft.height), Some(0.0), "a ship is drawn at the sea's level");
        let draft = transports.drawing.as_mut().unwrap();
        for (x, stop) in [(0.0, false), (300.0, false), (600.0, true), (900.0, false)] {
            draft.points.push((Vec3::new(x, 0.0, 0.0), stop));
        }
        let line = finish(&mut session, &mut flights, &mut transports);
        assert!(line.contains("made"), "{line}");
        assert!(transports.drawing.is_none());
        assert_eq!(session.history.depth_done(), 1, "the nodes, the path, the template and the period are one entry");
        follow(&mut transports, &mut session, true);
        let path = transports.route.expect("the new route is selected");
        let route = transports.sailing(path).expect("the route").clone();
        assert_eq!(route.points.len(), 6);
        assert_eq!(route.legs.len(), 1);
        assert_eq!((route.legs[0].map, route.legs[0].stops), (0, 1));
        let ship = transports.of_path(path).next().expect("the ship").clone();
        assert!(ship.created() && ship.in_step());
        assert!(ship.findings.is_empty(), "{:?}", ship.findings);
        let one_leg = ship.computed().expect("a schedule");
        // Two nodes with no mounts were made: the route's own and a shared
        // end.
        assert_eq!(taxi::nodes(session.table(taxi::NODES).unwrap()).len(), 2);

        // A second leg on another map.
        session.map_id = 1;
        transports.start_leg(path);
        let draft = transports.drawing.as_mut().unwrap();
        assert_eq!(draft.height, Some(0.0), "the route's own height");
        for (x, stop) in [(5000.0, false), (5300.0, true), (5600.0, false)] {
            draft.points.push((Vec3::new(x, 0.0, 0.0), stop));
        }
        let line = finish(&mut session, &mut flights, &mut transports);
        assert!(line.contains("leg"), "{line}");
        assert_eq!(session.history.depth_done(), 2);
        follow(&mut transports, &mut session, true);
        assert_eq!(session.history.depth_done(), 2, "the period is on the leg's own entry");
        let route = transports.sailing(path).expect("the route").clone();
        assert_eq!(route.legs.iter().map(|leg| (leg.map, leg.points.len(), leg.stops)).collect::<Vec<_>>(), vec![(0, 6, 1), (1, 5, 1)]);
        let ship = transports.of_path(path).next().expect("the ship").clone();
        assert!(ship.in_step());
        assert!(ship.computed().expect("a schedule") > one_leg);
        assert_eq!(ship.stops.len(), 2);
        // A third leg on the map the route ends on is refused, and stays
        // being drawn.
        transports.start_leg(path);
        transports.drawing.as_mut().unwrap().points.extend([(Vec3::ZERO, false), (Vec3::X, false)]);
        let line = finish(&mut session, &mut flights, &mut transports);
        assert!(line.contains("already on this map"), "{line}");
        assert!(transports.drawing.is_some());
        transports.drawing = None;

        let line = remove_leg(&mut session, &mut flights, &mut transports, path, 1);
        assert!(line.contains("removed"), "{line}");
        follow(&mut transports, &mut session, true);
        assert_eq!(transports.sailing(path).map(|route| route.legs.len()), Some(1));
        assert_eq!(transports.of_path(path).next().and_then(Ship::computed), Some(one_leg));
        assert!(remove_leg(&mut session, &mut flights, &mut transports, path, 0).contains("keeps one leg"));

        let line = remove_route(&mut session, &mut flights, &mut transports, path);
        assert!(line.contains("removed"), "{line}");
        assert!(session.rows.edits.is_empty(), "the template and the period went with the route");
        follow(&mut transports, &mut session, true);
        assert!(transports.sailing(path).is_none());
        assert!(transports.ships.is_empty());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A route a database transport sails is not removed.
    #[test]
    fn a_database_transports_route_is_kept() {
        let install = std::env::temp_dir().join(format!("vale-transport-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Kept").unwrap());
        session.tables.insert(taxi::POINTS.to_string(), points_table(&loop_of_twelve()));
        let mut transports = Transports::default();
        transports.reads.insert((), Held { templates: vec![template(20808, 1)], ..Held::default() });
        follow(&mut transports, &mut session, true);
        let mut flights = Flightpaths::default();
        let line = remove_route(&mut session, &mut flights, &mut transports, 1);
        assert!(line.contains("is kept"), "{line}");
        assert_eq!(session.history.depth_done(), 0);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A curve handle that stands where a drawn leg puts it moves with the
    /// point beside it, and one placed by hand does not.
    #[test]
    fn a_handle_follows_the_point_beside_it() {
        use super::super::flightpaths::Route;
        let install = std::env::temp_dir().join(format!("vale-transport-handle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Handle").unwrap());
        let specs = leg_points(0, &[(Vec3::ZERO, false), (Vec3::new(100.0, 0.0, 0.0), false), (Vec3::new(200.0, 0.0, 0.0), false)]);
        let points: Vec<Point> = specs
            .iter()
            .enumerate()
            .map(|(n, spec)| Point { id: 100 + n as u32, record: n, path: 1, index: n as u32, map: 0, at: spec.at, action_flag: 0, delay: 0 })
            .collect();
        session.tables.insert(taxi::POINTS.to_string(), points_table(&points));
        let mut flights = Flightpaths::default();
        flights.routes.push(Route {
            path: taxi::Path { id: 1, record: 0, from: 1, to: 2, cost: 0 },
            points: points.clone(),
            length: 0.0,
            flown: Vec::new(),
            bounds: Vec::new(),
            revision: 0,
            transport: true,
        });
        // The first travelled point moves 50 yards west: its handle goes to
        // the place a drawn leg would put it.
        carry_handles(&mut session, &flights, 101, Vec3::new(0.0, 50.0, 0.0), 1.0);
        let table = session.table(taxi::POINTS).unwrap();
        let read = taxi::points_by_path(table).remove(&1).unwrap();
        assert_eq!(read[0].at, [-100.0, 100.0, 0.0]);
        assert_eq!(read[4].at, points[4].at, "the far handle is not beside the moved point");
        // A handle dragged away from that place is left where it is.
        flights.routes[0].points[0].at = [-400.0, 0.0, 0.0];
        session.tables.insert(taxi::POINTS.to_string(), points_table(&flights.routes[0].points));
        carry_handles(&mut session, &flights, 101, Vec3::new(0.0, 50.0, 0.0), 2.0);
        let read = taxi::points_by_path(session.table(taxi::POINTS).unwrap()).remove(&1).unwrap();
        assert_eq!(read[0].at, [-400.0, 0.0, 0.0]);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// An undo entry is about a route when it changes a taxi table or a row
    /// of `gameobject_template`, and not when it moves a creature.
    #[test]
    fn an_edit_is_about_a_route_by_the_tables_it_changes() {
        use vale_edit::undo::{Change, Entry, ServerEntry};
        let server = |table: &str| {
            Change::from_entries("Edit", vec![Entry::Server(ServerEntry::column(table, "entry=1,patch=0", "data1", None, Some("45".into())))])
        };
        assert!(about_a_route(&server(gameobject::TEMPLATE)));
        assert!(!about_a_route(&server("creature")));
    }
}
