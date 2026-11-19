//! The boats and the zeppelins: where a continent transport is, computed from
//! its template, its route and a clock, because no packet states it.
//!
//! ```text
//! wire     SMSG_GAMEOBJECT_QUERY_RESPONSE  type 15, data[0..2]
//!            data[0] taxiPathId   data[1] moveSpeed   data[2] accelRate
//!   +      UPDATEFLAG_TRANSPORT's trailing word -> ms into the route
//!   + file TaxiPathNode.dbc rows for that path: a map, a point, a flag, a wait
//!   = here a place on a continent, and the heading
//! ```
//!
//! ## The position on the wire is zero
//!
//! `GameObject::GetStationaryX` (and `Y`, and `Z`) is
//!
//! ```cpp
//! if (GetGOInfo()->type != GAMEOBJECT_TYPE_MO_TRANSPORT) return m_stationaryPosition.x;
//! return 0.f;
//! ```
//!
//! and `UPDATEFLAG_TRANSPORT` makes `BuildMovementUpdate` write the stationary
//! position in place of the live one. A client is therefore told that every
//! ship in the game is at the map's origin, and has to compute where it is
//! from the three numbers above. Nothing else on the wire mentions the
//! position again: `ShipTransport::Update` calls `Relocate` and broadcasts
//! nothing.
//!
//! Both ends run the same arithmetic over the same shipped table. The one word
//! that crosses the wire states how far into the route the ship is, in
//! milliseconds, and not where it is.
//!
//! ## The route is a schedule, and the client's schedule is the authority
//!
//! A ship does not travel its curve at a constant rate. It accelerates out of
//! each dock at `accelRate`, cruises at `moveSpeed`, brakes into the next dock
//! and waits there for the stop node's `delay`. The route is therefore turned
//! into a schedule. The schedule's total is the *period*, and both ends have
//! to agree about it, because the word on the wire is read modulo it.
//!
//! A period that is 1 ms out puts the boat in an unrelated place. The counter
//! is `GetTimeSinceCreation() + m_startProgress`, which runs from the moment
//! the server started, so after `n` cycles two periods that differ by `d` put
//! the two ends `n * d` apart. A week of uptime is two thousand cycles.
//!
//! `TransportMgr::GenerateWaypoints` is not the authority. vmangos'
//! `LoadTransportTemplates` ends by reading a `period` column from the world
//! database's `transports` table, under the comment *"load period override
//! from db since our algorithm is not perfect"*. Those numbers were measured
//! from the 1.12.1 client. A port of `GenerateWaypoints` reproduces its error,
//! which the server overrides and this client would not.
//!
//! This file follows the 1.12.1 client's ship path, which has three steps:
//!
//! ```text
//! build        walk the path, cut legs, stamp the clock
//! finish leg   one leg: its curve, its waits, its travel time
//! at           progress -> a place and a heading
//! ```
//!
//! It differs from the server's reconstruction in six ways:
//!
//! * No node is dropped, at either end of the path or at a teleport. The
//!   client walks every `TaxiPathNode` row; `GenerateWaypoints` loops
//!   `for (i = 1; i < size - 1; ++i)` and skips the node after a teleport as
//!   well.
//! * The end points are tangent handles and not places. A leg of `n` control
//!   points has `n - 3` measured segments, and the travelled path is node 1 to
//!   node `n - 2`. See [`Spline`].
//! * A leg is cut at a teleport and keeps both sides. The node carrying
//!   `actionFlag & 1` is the last control point of the leg it ends, and the
//!   next node is the first control point of the new one.
//! * The arc length is twenty chords, not three. See [`ARC_STEPS`].
//! * A leg's stop list carries one more entry than it has waits: the
//!   terminator, with delay zero and the whole curve as its distance.
//! * The cycle adds the last leg's waits once per leg. See
//!   [`Route::stamp_the_clock`].
//!
//! `vale ships` checks the nine periods this produces against the nine the
//! server holds, and they are equal.
//!
//! ## The heading
//!
//! The heading is the direction of travel and nothing else: no pitch, no roll,
//! and nothing read from the file about the model's own orientation. A boat on
//! a slope is level.
//!
//! The direction is the curve's tangent, so the hull turns continuously
//! through a corner and does not jump where one segment ends and the next
//! begins. The segment's chord replaces the tangent only where the tangent is
//! more than 60 degrees from the chord. See [`Spline::direction`].
//!
use crate::tables::taxi::TaxiWaypoint;

/// What `gameobject_template`'s `moTransport` union says, read from the
/// twenty-four words `SMSG_GAMEOBJECT_QUERY_RESPONSE` carries.
///
/// Three of the twenty-four are read. Every shipped transport but one has the
/// same two values for the last pair: `moveSpeed` 30 and `accelRate` 1. The
/// exception is Naxxramas, a platform that never moves, which states a speed
/// of 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoTransport {
    /// `data[0]`: the `TaxiPath.dbc` id whose nodes are the route.
    pub taxi_path: u32,
    /// `data[1]`, yards per second at cruise.
    pub move_speed: u32,
    /// `data[2]`, yards per second per second.
    pub accel_rate: u32,
}

impl MoTransport {
    /// `GAMEOBJECT_TYPE_MO_TRANSPORT`, the only type these three words have a
    /// meaning for.
    pub const TYPE: u32 = 15;

    /// Read the union, or `None` for a template of any other type.
    ///
    /// A zero in any of the three is refused. A speed or an acceleration of
    /// zero is a division by zero in [`Route::build`] and gives a schedule of
    /// infinities, and a path id of zero names no route. All three are
    /// non-zero in every shipped row, so a zero here is a template that did
    /// not arrive and not a ship that stands still.
    pub fn read(object_type: u32, data: &[u32]) -> Option<MoTransport> {
        if object_type != Self::TYPE || data.len() < 3 {
            return None;
        }
        let out = MoTransport {
            taxi_path: data[0],
            move_speed: data[1],
            accel_rate: data[2],
        };
        (out.taxi_path != 0 && out.move_speed != 0 && out.accel_rate != 0).then_some(out)
    }
}

// ---------------------------------------------------------------------------
// The spline
// ---------------------------------------------------------------------------

/// How many chords a segment's arc length is measured over, and the step in `t`
/// between them. The client steps `t` by 0.05, twenty times.
///
/// The number cannot be changed for accuracy. The schedule below is arithmetic
/// over these lengths and the server runs the same arithmetic, so a finer or
/// coarser measurement here is a disagreement about where the boat is. Twenty
/// chords is what the 1.12.1 client takes. vmangos' own
/// `SplineBase::STEPS_PER_SEGMENT` is 3, which is why its computed periods are
/// wrong by a tenth of a percent and why it overrides them from a database
/// table.
const ARC_STEPS: usize = 20;
const ARC_STEP: f64 = 0.05;

/// Round a value to single precision, at the points where the 1.12.1 client's
/// schedule holds one.
///
/// The client computes the schedule at 80 bits and rounds to single precision
/// at five points: each sampled point of an arc-length walk, the running total
/// of that walk, each segment's length, the curve's total, and the distance
/// carried from one wait to the next. Everything between those points is at
/// the wider precision.
///
/// The difference is 1 ms on four of the nine routes. The phase the wire
/// carries is a free-running counter taken modulo the period, so after a few
/// thousand cycles a period 1 ms out puts the boat anywhere on its route. This
/// file therefore computes in `f64` and stores `f32` at those five points, and
/// `vale ships` checks the nine periods that result.
fn f32_store(x: f64) -> f64 {
    x as f32 as f64
}

/// A Catmull-Rom curve through a run of control points, with each travelled
/// segment's arc length measured.
///
/// The end points are tangent handles, not places the ship goes. The client
/// copies the points it is given with no padding and measures `count - 3`
/// segment lengths, where segment `i` is evaluated from `points[i .. i + 3]`
/// and runs from `points[i + 1]` to `points[i + 2]`. A leg of `n` nodes is
/// therefore travelled from node 1 to node `n - 2`, and nodes 0 and `n - 1`
/// only shape the curve.
///
/// The basis is the standard Catmull-Rom one, evaluated as four weights times
/// four points:
///
/// ```text
/// w0 = 0.5(-t³ + 2t² - t)     w1 = 0.5(3t³ - 5t² + 2)
/// w2 = 0.5(-3t³ + 4t² + t)    w3 = 0.5(t³ - t²)
/// ```
#[derive(Debug, Clone, Default)]
pub struct Spline {
    points: Vec<[f32; 3]>,
    /// `lengths[i]` is the arc length of the segment from `points[i + 1]` to
    /// `points[i + 2]`. Empty for a curve of three points or fewer, which
    /// the client does not measure.
    ///
    /// Held at single precision; see [`f32_store`].
    lengths: Vec<f32>,
    total: f32,
}

impl Spline {
    /// Copy the points, measure every segment, sum them.
    pub fn new(points: &[[f32; 3]]) -> Spline {
        let mut spline = Spline {
            points: points.to_vec(),
            lengths: Vec::new(),
            total: 0.0,
        };
        // The client measures only when `count > 3`: a curve with no travelled
        // segment keeps a total length of zero, and the leg that holds it
        // takes no time.
        if points.len() > 3 {
            spline.lengths = (0..points.len() - 3)
                .map(|i| spline.segment_length(i) as f32)
                .collect();
            let total: f64 = spline.lengths.iter().map(|l| f64::from(*l)).sum();
            spline.total = total as f32;
        }
        spline
    }

    /// How many travelled segments this curve has.
    pub fn segments(&self) -> usize {
        self.lengths.len()
    }

    /// The whole travelled length, `points[1]` to `points[len - 2]`.
    pub fn total(&self) -> f32 {
        self.total
    }

    /// The total as the schedule reads it: at the single precision it was
    /// stored at.
    fn total64(&self) -> f64 {
        f64::from(self.total)
    }

    pub fn points(&self) -> &[[f32; 3]] {
        &self.points
    }

    /// The distance travelled to reach control point `n`: the sum of
    /// `lengths[0 ..= n - 2]`, which is `n - 1` of them.
    ///
    /// Zero for `n` of 0 or 1, because the travelled path starts at
    /// `points[1]`.
    pub(crate) fn length_to(&self, n: usize) -> f64 {
        let take = n.saturating_sub(1);
        self.lengths.iter().take(take).map(|l| f64::from(*l)).sum()
    }

    /// A point on segment `index` at `t` in `0..1`.
    ///
    /// Computed in `f64` and answered at single precision per axis, as the
    /// client stores it.
    pub fn at(&self, index: usize, t: f64) -> [f32; 3] {
        let w = weights(t);
        let mut out = [0.0f64; 3];
        for (n, weight) in w.iter().enumerate() {
            let Some(point) = self.points.get(index + n) else {
                continue;
            };
            for axis in 0..3 {
                out[axis] += f64::from(point[axis]) * weight;
            }
        }
        [out[0] as f32, out[1] as f32, out[2] as f32]
    }

    /// The chord of segment `index`: `points[index + 2] - points[index + 1]`,
    /// not normalised. It is the heading's fallback; see [`Self::direction`].
    pub fn chord(&self, index: usize) -> [f32; 3] {
        let (Some(a), Some(b)) = (self.points.get(index + 1), self.points.get(index + 2)) else {
            return [1.0, 0.0, 0.0];
        };
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
    }

    /// The curve's tangent at `(index, t)`, not normalised: the four points
    /// evaluated against the Catmull-Rom derivative weights.
    ///
    /// ```text
    /// w0' = -1.5t² + 2t - 0.5     w1' = 4.5t² - 5t
    /// w2' = -4.5t² + 4t + 0.5     w3' = 1.5t² - t
    /// ```
    fn tangent(&self, index: usize, t: f64) -> [f32; 3] {
        let t2 = t * t;
        let w = [
            -1.5 * t2 + 2.0 * t - 0.5,
            4.5 * t2 - 5.0 * t,
            -4.5 * t2 + 4.0 * t + 0.5,
            1.5 * t2 - t,
        ];
        let mut out = [0.0f64; 3];
        for (n, weight) in w.iter().enumerate() {
            let Some(point) = self.points.get(index + n) else {
                continue;
            };
            for axis in 0..3 {
                out[axis] += f64::from(point[axis]) * weight;
            }
        }
        [out[0] as f32, out[1] as f32, out[2] as f32]
    }

    /// The direction of travel at `(index, t)`, as the 1.12.1 client computes
    /// it:
    ///
    /// ```text
    /// chord   = normalize(points[index+2] - points[index+1])
    /// tangent = normalize(dC/dt at (index, t))
    /// dir     = tangent, unless dot(chord, tangent) < 0.5
    ///           in which case chord
    /// ```
    ///
    /// The heading is the tangent almost everywhere. A Catmull-Rom curve has
    /// one tangent at a node, so the heading is continuous where one segment
    /// ends and the next begins, and a ship turns through a corner at the rate
    /// the curve bends. The chord is answered only where the tangent is more
    /// than 60 degrees from it, which is the sharpest corners.
    ///
    /// An earlier version of this function answered the chord first and the
    /// tangent as the fallback. The heading was then constant along each
    /// segment and jumped at each node, by up to 76 degrees on the shipped
    /// routes, and the hull turned at the node and not along the bend.
    ///
    /// A tangent shorter than [`TANGENT_MIN`] has no direction, and the client
    /// then answers `+x`. A chord shorter than [`CHORD_MIN`] is compared and
    /// answered without being normalised.
    pub fn direction(&self, index: usize, t: f64) -> [f32; 3] {
        let chord = self.chord(index);
        let chord = normalize(chord, CHORD_MIN).unwrap_or(chord);
        let Some(tangent) = normalize(self.tangent(index, t), TANGENT_MIN) else {
            return [1.0, 0.0, 0.0];
        };
        let dot = chord[0] * tangent[0] + chord[1] * tangent[1] + chord[2] * tangent[2];
        if dot >= 0.5 {
            tangent
        } else {
            chord
        }
    }

    /// A point at a distance along the travelled path, for a reader that has
    /// a distance and not a time: `vale ships` listing a leg's stops.
    pub fn along(&self, dist: f64) -> [f32; 3] {
        let (index, t) = self.seek(dist.clamp(0.0, self.total64()));
        self.at(index, t)
    }

    /// The client's arc length: twenty chords at `t = 0.05k`.
    ///
    /// The sampled points and the running total are both stored at single
    /// precision; the chord itself is not. See [`f32_store`].
    fn segment_length(&self, index: usize) -> f64 {
        let mut previous = self.at(index, 0.0);
        let mut total = 0.0f64;
        let mut t = ARC_STEP;
        for _ in 0..ARC_STEPS {
            let next = self.at(index, t);
            total = f32_store(total + distance(previous, next));
            previous = next;
            t += ARC_STEP;
        }
        total
    }

    /// Where a distance along the travelled path falls, as a segment and a
    /// parameter in `0..1`.
    ///
    /// This is the 1.12.1 client's mapping: walk the measured lengths until
    /// the distance falls inside one, and answer the remaining distance
    /// divided by that segment's length. The parameter is therefore linear in
    /// distance inside a segment, although the curve's speed in `t` is not
    /// constant. A distance at or past the end of the last segment is
    /// answered on the last segment.
    fn seek(&self, distance: f64) -> (usize, f64) {
        if self.lengths.is_empty() {
            return (0, 0.0);
        }
        let mut left = distance.max(0.0);
        for (index, length) in self.lengths.iter().enumerate() {
            let length = f64::from(*length);
            if left < length || index + 1 == self.lengths.len() {
                let t = if length > 0.0 { left / length } else { 0.0 };
                return (index, t.clamp(0.0, 1.0));
            }
            left -= length;
        }
        (self.lengths.len() - 1, 1.0)
    }
}

/// The shortest chord the client normalises: 2^-22 yards. A shorter one is
/// used as it is.
const CHORD_MIN: f64 = 1.0 / 4_194_304.0;

/// The shortest tangent the client normalises. A curve whose derivative is
/// shorter than this has no direction of its own; see [`Spline::direction`].
const TANGENT_MIN: f64 = 0.01;

/// The Catmull-Rom weights: the four coefficient rows.
fn weights(t: f64) -> [f64; 4] {
    let (t3, t2) = (t * t * t, t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    let d = |n: usize| f64::from(a[n]) - f64::from(b[n]);
    (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt()
}

/// Seconds as whole milliseconds, rounded half away from zero: add or
/// subtract 0.5, then truncate. Every leg time in the schedule is a whole
/// number of milliseconds produced this way, and the rounding is what makes
/// the period an exact integer both ends agree on.
fn round_ms(seconds: f64) -> u32 {
    let ms = seconds * 1000.0;
    let rounded = if ms >= 0.0 { ms + 0.5 } else { ms - 0.5 };
    if rounded <= 0.0 {
        0
    } else {
        rounded as u32
    }
}

// ---------------------------------------------------------------------------
// The schedule
// ---------------------------------------------------------------------------

/// One place the ship waits, and where it is when it gets there.
///
/// This is the client's stop record: the moment the ship arrives, the distance
/// along the leg's curve it arrives at, and how long it waits.
///
/// A leg always has one more of these than it has waits. The last entry is
/// the end of the leg, the moment the curve runs out, and its delay is zero.
/// With it the lookup is one loop with no special case for the last segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    /// Milliseconds into the cycle once the route is built: finishing the leg
    /// writes it relative to the leg, and the build adds the leg's start.
    pub time_ms: u32,
    /// How far along the leg's curve that is.
    pub dist: f64,
    /// The node's `delay`, in milliseconds. Zero for the terminator.
    pub delay_ms: u32,
}

/// One run of the route between teleports.
///
/// A leg is a map, a curve and a schedule. The client cuts a new one whenever
/// the node before a node has bit 0 of its `actionFlag` set, or the map
/// changes. Unlike vmangos it drops no node doing so: the node carrying the
/// teleport flag is the last control point of the leg it ends, and the node
/// after it is the first control point of the next.
#[derive(Debug, Clone)]
pub struct Leg {
    map: u32,
    spline: Spline,
    start_ms: u32,
    travel_ms: u32,
    end_ms: u32,
    stops: Vec<Stop>,
}

impl Leg {
    pub fn map(&self) -> u32 {
        self.map
    }
    pub fn spline(&self) -> &Spline {
        &self.spline
    }
    /// Milliseconds into the cycle at which this leg begins.
    pub fn start_ms(&self) -> u32 {
        self.start_ms
    }
    /// Milliseconds into the cycle at which this leg ends, waits included.
    pub fn end_ms(&self) -> u32 {
        self.end_ms
    }
    /// The travelling time alone, with no waits in it.
    pub fn travel_ms(&self) -> u32 {
        self.travel_ms
    }
    /// Every wait, and the terminator; see [`Stop`].
    pub fn stops(&self) -> &[Stop] {
        &self.stops
    }
}

/// Where a transport is, at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub map: u32,
    pub pos: [f32; 3],
    /// Radians, in the server's convention: `atan2(dy, dx) + PI`.
    pub facing: f32,
}

/// A boat or zeppelin's whole route, as the 1.12.1 client builds it.
#[derive(Debug, Clone, Default)]
pub struct Route {
    legs: Vec<Leg>,
    period: u32,
    speed: f64,
    accel: f64,
}

impl Route {
    /// Build the schedule: the client's build step.
    ///
    /// `None` when the path names no nodes at all.
    pub fn build(path: &[TaxiWaypoint], template: MoTransport) -> Option<Route> {
        if path.is_empty() {
            return None;
        }
        let speed = f64::from(template.move_speed);
        let accel = f64::from(template.accel_rate);

        let mut legs: Vec<Leg> = Vec::new();
        let mut points: Vec<[f32; 3]> = Vec::new();
        let mut waits: Vec<(usize, u32)> = Vec::new();
        let mut previous_teleport = false;
        // The last leg's waits are kept because the clock reads them once per
        // leg. See [`Route::stamp_the_clock`].
        let mut last_waits: Vec<(usize, u32)> = Vec::new();

        for node in path {
            let same_leg = legs
                .last()
                .is_some_and(|leg| leg.map == node.map && !previous_teleport);
            if !same_leg {
                if let Some(leg) = legs.last_mut() {
                    finish_leg(leg, &points, &waits, speed, accel);
                    last_waits = std::mem::take(&mut waits);
                }
                legs.push(Leg {
                    map: node.map,
                    spline: Spline::default(),
                    start_ms: 0,
                    travel_ms: 0,
                    end_ms: 0,
                    stops: Vec::new(),
                });
                points.clear();
                waits.clear();
            }
            // A wait at the first node of a leg is not recorded, as in the
            // client: there is no segment before it to have arrived along.
            if node.action_flag & 2 != 0 && !points.is_empty() {
                waits.push((points.len(), node.delay * 1000));
            }
            points.push(node.pos);
            previous_teleport = node.action_flag & 1 != 0;
        }
        if points.is_empty() {
            legs.pop();
        } else if let Some(leg) = legs.last_mut() {
            finish_leg(leg, &points, &waits, speed, accel);
            last_waits = waits;
        }
        if legs.is_empty() {
            return None;
        }

        let mut route = Route { legs, period: 0, speed, accel };
        route.stamp_the_clock(&last_waits);
        Some(route)
    }

    /// Set when each leg starts and ends, and how long the whole cycle is.
    ///
    /// The waits are added to the running total from the last leg's list and
    /// not from each leg's own, once per leg. That is what the 1.12.1 client
    /// computes, and it is kept because the server's periods were measured
    /// from that client and both ends have to agree: `vale ships` checks all
    /// nine periods against the ones vmangos ships in its `transports` table,
    /// and each is equal only with this.
    fn stamp_the_clock(&mut self, last_waits: &[(usize, u32)]) {
        let waits: u32 = last_waits.iter().map(|(_, delay)| delay).sum();
        let mut total: u32 = 0;
        for leg in &mut self.legs {
            leg.start_ms = total;
            total = total.saturating_add(leg.travel_ms).saturating_add(waits);
            for stop in &mut leg.stops {
                stop.time_ms = stop.time_ms.saturating_add(leg.start_ms);
            }
            leg.end_ms = total;
        }
        self.period = total;
    }

    /// The cycle length in milliseconds.
    pub fn period(&self) -> u32 {
        self.period
    }

    pub fn legs(&self) -> &[Leg] {
        &self.legs
    }

    /// Where the transport is at `progress` milliseconds into its route, as
    /// the 1.12.1 client places it.
    ///
    /// `None` for a route with no period, which cannot be indexed.
    pub fn at(&self, progress: u32) -> Option<Placement> {
        if self.period == 0 || self.legs.is_empty() {
            return None;
        }
        let now = progress % self.period;
        let leg = self.legs.iter().find(|leg| now < leg.end_ms)?;
        if leg.stops.is_empty() {
            return None;
        }

        // Walk the leg's stops: past each one whose wait has already elapsed,
        // held at one whose wait has not.
        let mut segment_start_ms = leg.start_ms;
        let mut segment_start_dist = 0.0f64;
        let mut index = 0usize;
        while index + 1 < leg.stops.len() {
            let stop = leg.stops[index];
            if now < stop.time_ms {
                break;
            }
            if now - stop.time_ms < stop.delay_ms {
                return Some(self.place(leg, stop.dist));
            }
            segment_start_ms = stop.time_ms + stop.delay_ms;
            segment_start_dist = stop.dist;
            index += 1;
        }

        let stop = leg.stops[index];
        let duration = f64::from(stop.time_ms.saturating_sub(segment_start_ms)) / 1000.0;
        let elapsed = f64::from(now.saturating_sub(segment_start_ms)) / 1000.0;
        // Which ends of this segment the ship is at rest at. The first
        // segment of a leg begins where the last one teleported from, at
        // speed, and the last ends the same way. Every segment between runs
        // dock to dock.
        let from_rest = index != 0;
        let to_rest = index + 1 != leg.stops.len();
        let covered = self.covered(elapsed, duration, from_rest, to_rest);
        Some(self.place(leg, segment_start_dist + covered))
    }

    /// How far into a segment the ship has got, as three phases: accelerate,
    /// cruise, brake.
    ///
    /// `from_rest` and `to_rest` say whether the segment begins and ends at a
    /// dock. A phase that is not taken contributes nothing, and the cruise is
    /// the time the two ramps leave over. A segment too short to reach `speed`
    /// splits its time evenly between the ramps it has, which is the case
    /// analysis the schedule was timed by.
    fn covered(&self, elapsed: f64, duration: f64, from_rest: bool, to_rest: bool) -> f64 {
        if duration <= 0.0 {
            return 0.0;
        }
        let elapsed = elapsed.clamp(0.0, duration);
        let full = self.speed / self.accel;
        // The ramps share the segment when it is too short for both at full
        // length, which is the client's `min(duration / 2, accelTime)`.
        let ramp = match (from_rest, to_rest) {
            (true, true) => full.min(duration * 0.5),
            (true, false) | (false, true) => full.min(duration),
            (false, false) => 0.0,
        };
        // The speed the ramp reaches, which is `speed` only when the segment
        // is long enough for a full ramp. A shorter segment ramps for its
        // whole half and has no cruise. This is the case analysis the
        // schedule was timed by, so a segment covers its own distance in its
        // own time.
        let level = if from_rest || to_rest { self.accel * ramp } else { self.speed };
        let up = if from_rest { ramp } else { 0.0 };
        let down = if to_rest { ramp } else { 0.0 };
        let cruise = (duration - up - down).max(0.0);

        let mut covered = 0.0;
        // Accelerating.
        let t = elapsed.min(up);
        covered += 0.5 * self.accel * t * t;
        if elapsed <= up {
            return covered;
        }
        // Cruising.
        let t = (elapsed - up).min(cruise);
        covered += level * t;
        if elapsed <= up + cruise {
            return covered;
        }
        // Braking.
        let t = elapsed - up - cruise;
        covered += level * t - 0.5 * self.accel * t * t;
        covered
    }

    /// A distance along a leg's curve, as a place on a map.
    fn place(&self, leg: &Leg, dist: f64) -> Placement {
        let total = leg.spline.total64();
        let along = if total > 0.0 { (dist / total).clamp(0.0, 1.0) * total } else { 0.0 };
        let (index, t) = leg.spline.seek(along);
        let dir = leg.spline.direction(index, t);
        Placement {
            map: leg.map,
            pos: leg.spline.at(index, t),
            // The client negates both, which is the `+ PI`.
            facing: normalize_orientation((-dir[1]).atan2(-dir[0])),
        }
    }
}

/// One leg's curve and schedule: the client's finish-leg step.
///
/// Each segment of the leg runs from one wait to the next, and its time is a
/// closed form of the accelerate, cruise and brake shape with the ramps the
/// segment has:
///
/// ```text
/// no ramps        d / speed                     a leg with no waits at all
/// one ramp        accelTime + (d - aD)/speed    or sqrt(2d/a) if it never gets up to speed
/// two ramps       2accelTime + (d - 2aD)/speed  or 2 sqrt(d/a)
/// ```
///
/// The first segment of a leg has no ramp at its start and the last has none at
/// its end, because a leg begins and ends at a teleport and not at a dock.
fn finish_leg(leg: &mut Leg, points: &[[f32; 3]], waits: &[(usize, u32)], speed: f64, accel: f64) {
    leg.spline = Spline::new(points);
    leg.travel_ms = 0;
    leg.stops.clear();

    let accel_time = speed / accel;
    let accel_dist = 0.5 * speed * accel_time;
    let mut previous = 0.0f64;
    let mut waited = 0u32;
    let mut taken = 0usize;

    for (n, (point, delay)) in waits.iter().enumerate() {
        // A wait on the last control point is past the end of the travelled
        // path, and the client stops reading the list there.
        if *point + 1 >= points.len() {
            break;
        }
        let reached = leg.spline.length_to(*point);
        let d = reached - previous;
        let seconds = if n == 0 {
            one_ramp(d, speed, accel, accel_time, accel_dist)
        } else {
            two_ramps(d, speed, accel, accel_time, accel_dist)
        };
        leg.travel_ms = leg.travel_ms.saturating_add(round_ms(seconds));
        leg.stops.push(Stop {
            time_ms: leg.travel_ms.saturating_add(waited),
            dist: reached,
            delay_ms: *delay,
        });
        waited = waited.saturating_add(*delay);
        // The distance carried to the next wait is held at single precision.
        // It is the only place the schedule narrows. See [`f32_store`].
        previous = f32_store(reached);
        taken = n + 1;
    }

    let d = leg.spline.total64() - previous;
    let seconds = if taken == 0 {
        // A leg with no waits at all is walked at a constant speed: it starts
        // and ends at a teleport, so there is nothing to ramp against.
        if speed > 0.0 { d / speed } else { 0.0 }
    } else {
        one_ramp(d, speed, accel, accel_time, accel_dist)
    };
    leg.travel_ms = leg.travel_ms.saturating_add(round_ms(seconds));
    leg.stops.push(Stop {
        time_ms: leg.travel_ms.saturating_add(waited),
        dist: leg.spline.total64(),
        delay_ms: 0,
    });
}

/// The time of a segment with one ramp: one acceleration and then cruise, or
/// a ramp that never reaches speed.
fn one_ramp(d: f64, speed: f64, accel: f64, accel_time: f64, accel_dist: f64) -> f64 {
    if accel_dist < d {
        accel_time + (d - accel_dist) / speed
    } else {
        (2.0 * d / accel).sqrt()
    }
}

/// The time of a segment from dock to dock, which ramps at both ends.
fn two_ramps(d: f64, speed: f64, accel: f64, accel_time: f64, accel_dist: f64) -> f64 {
    if accel_dist < d / 2.0 {
        2.0 * accel_time + (d - 2.0 * accel_dist) / speed
    } else {
        2.0 * (d / accel).sqrt()
    }
}

/// A unit vector, or `None` for one shorter than `shortest`, which the client
/// does not normalise.
fn normalize(v: [f32; 3], shortest: f64) -> Option<[f32; 3]> {
    let len = (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2) + f64::from(v[2]).powi(2)).sqrt();
    if len < shortest {
        return None;
    }
    Some([
        (f64::from(v[0]) / len) as f32,
        (f64::from(v[1]) / len) as f32,
        (f64::from(v[2]) / len) as f32,
    ])
}

/// `Geometry::NormalizeOrientation`: a modulo into `[0, 2pi)`, the one the
/// movement code uses.
fn normalize_orientation(o: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let wrapped = o % tau;
    if wrapped < 0.0 {
        wrapped + tau
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight run of waypoints along `+x`, with a wait near each end.
    ///
    /// The first and last node of a leg are tangent handles, not places the
    /// ship goes, so a run of `n` nodes is travelled from node 1 to node
    /// `n - 2`.
    fn straight(count: usize, spacing: f32) -> Vec<TaxiWaypoint> {
        (0..count)
            .map(|i| TaxiWaypoint {
                index: i as u32,
                map: 0,
                pos: [i as f32 * spacing, 0.0, 100.0],
                action_flag: if i == 2 || i + 3 == count { 2 } else { 0 },
                delay: if i == 2 || i + 3 == count { 10 } else { 0 },
            })
            .collect()
    }

    fn template() -> MoTransport {
        MoTransport { taxi_path: 1, move_speed: 30, accel_rate: 1 }
    }

    /// A Catmull-Rom segment runs between its two middle control points.
    ///
    /// This fails when the basis matrix is written a row out. The schedule
    /// measures arc lengths on this curve, so a curve that does not touch its
    /// waypoints gives a route that is wrong everywhere and fails no other
    /// test.
    #[test]
    fn a_segment_runs_between_the_middle_two_of_its_four_points() {
        let controls = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [20.0, 5.0, 0.0],
            [30.0, 5.0, 2.0],
        ];
        let spline = Spline::new(&controls);
        assert_eq!(spline.segments(), 1, "four points make one travelled segment");
        let start = spline.at(0, 0.0);
        let end = spline.at(0, 1.0);
        for axis in 0..3 {
            assert!((start[axis] - controls[1][axis]).abs() < 1e-3, "{start:?}");
            assert!((end[axis] - controls[2][axis]).abs() < 1e-3, "{end:?}");
        }
    }

    /// A straight line measures its own spacing. The twenty-chord sample is
    /// exact on a straight run, so this checks the sampling as well as the
    /// curve.
    #[test]
    fn a_straight_run_measures_its_own_spacing() {
        let controls: Vec<[f32; 3]> = (0..6).map(|i| [i as f32 * 12.0, 0.0, 0.0]).collect();
        let spline = Spline::new(&controls);
        assert_eq!(spline.segments(), 3);
        for index in 0..spline.segments() {
            let length = spline.length_to(index + 2) - spline.length_to(index + 1);
            assert!((length - 12.0).abs() < 1e-2, "segment {index} measured {length}");
        }
        assert!((spline.total() - 36.0).abs() < 1e-2, "{}", spline.total());
    }

    /// `length_to` is the distance from the first travelled point, which is
    /// `points[1]` and not `points[0]`.
    #[test]
    fn the_distance_to_a_node_is_measured_from_the_first_travelled_one() {
        let controls: Vec<[f32; 3]> = (0..6).map(|i| [i as f32 * 12.0, 0.0, 0.0]).collect();
        let spline = Spline::new(&controls);
        assert_eq!(spline.length_to(0), 0.0);
        assert_eq!(spline.length_to(1), 0.0, "the travelled path starts here");
        assert!((spline.length_to(2) - 12.0).abs() < 1e-2);
        assert!((spline.length_to(4) - 36.0).abs() < 1e-2);
    }

    /// The template is read only for type 15, and a zero in it is refused.
    #[test]
    fn only_a_transport_template_reads_as_one() {
        let data = [302u32, 30, 1, 0];
        assert_eq!(
            MoTransport::read(15, &data),
            Some(MoTransport { taxi_path: 302, move_speed: 30, accel_rate: 1 })
        );
        assert_eq!(MoTransport::read(0, &data), None);
        assert_eq!(MoTransport::read(15, &[302, 0, 1]), None);
        assert_eq!(MoTransport::read(15, &[0, 30, 1]), None);
        assert_eq!(MoTransport::read(15, &[302, 30]), None);
    }

    /// The route walks its whole length and comes back: at no point in the
    /// cycle is the ship off its own waypoints, and it visits both ends.
    #[test]
    fn a_route_walks_its_own_waypoints_and_returns() {
        let path = straight(12, 100.0);
        let route = Route::build(&path, template()).expect("a route");
        assert_eq!(route.legs().len(), 1);
        assert!(route.period() > 0);

        let (mut lowest, mut highest) = (f32::MAX, f32::MIN);
        let mut progress = 0;
        while progress < route.period() {
            let at = route.at(progress).expect("a placement");
            assert_eq!(at.map, 0);
            assert!(
                at.pos[1].abs() < 1.0 && (at.pos[2] - 100.0).abs() < 1.0,
                "at {progress} ms the ship is at {:?}",
                at.pos
            );
            lowest = lowest.min(at.pos[0]);
            highest = highest.max(at.pos[0]);
            progress += 250;
        }
        // The travelled path runs from node 1 (x = 100) to node 10 (x = 1000).
        assert!(lowest < 150.0, "never reached the first node: {lowest}");
        assert!(highest > 950.0, "never reached the last node: {highest}");
    }

    /// A ship waits at a dock for the stop's delay and does not move while it
    /// waits.
    #[test]
    fn a_stop_holds_the_ship_still_for_its_delay() {
        let path = straight(12, 100.0);
        let route = Route::build(&path, template()).expect("a route");
        let leg = &route.legs()[0];
        let stop = leg.stops().iter().find(|s| s.delay_ms > 0).expect("a dock");
        assert_eq!(stop.delay_ms, 10_000);
        let a = route.at(stop.time_ms).expect("a placement");
        let b = route.at(stop.time_ms + stop.delay_ms - 1).expect("a placement");
        for axis in 0..3 {
            assert!(
                (a.pos[axis] - b.pos[axis]).abs() < 1e-2,
                "the ship moved while docked: {:?} -> {:?}",
                a.pos,
                b.pos
            );
        }
    }

    /// Each segment's timing and its distance agree. The ship has to reach
    /// the next dock at the moment the schedule says it arrives, or every
    /// later wait is taken in the wrong place.
    #[test]
    fn a_segment_covers_its_own_distance_in_its_own_time() {
        let path = straight(14, 120.0);
        let route = Route::build(&path, template()).expect("a route");
        let leg = &route.legs()[0];
        let mut start_ms = leg.start_ms();
        let mut start_dist = 0.0f64;
        for (index, stop) in leg.stops().iter().enumerate() {
            let duration = f64::from(stop.time_ms - start_ms) / 1000.0;
            let covered = route.covered(
                duration,
                duration,
                index != 0,
                index + 1 != leg.stops().len(),
            );
            assert!(
                (covered - (stop.dist - start_dist)).abs() < 0.5,
                "segment {index} covers {covered} of {} in {duration}s",
                stop.dist - start_dist
            );
            start_ms = stop.time_ms + stop.delay_ms;
            start_dist = stop.dist;
        }
    }

    /// A route that changes map is cut into separate legs, and no node is
    /// dropped doing it. The client marks the leg boundary and keeps both
    /// sides; vmangos discards the node carrying the flag and the one after
    /// it.
    #[test]
    fn a_map_change_cuts_the_route_without_losing_a_node() {
        let mut path = straight(14, 100.0);
        for node in path.iter_mut().skip(7) {
            node.map = 1;
        }
        let route = Route::build(&path, template()).expect("a route");
        assert_eq!(route.legs().len(), 2, "one leg a continent");
        assert_eq!(route.legs()[0].map(), 0);
        assert_eq!(route.legs()[1].map(), 1);
        assert_eq!(
            route.legs()[0].spline().points().len() + route.legs()[1].spline().points().len(),
            path.len(),
            "a node went missing at the cut",
        );
        let mut progress = 0;
        while progress < route.period() {
            let at = route.at(progress).expect("a placement");
            assert!(at.map == 0 || at.map == 1);
            progress += 250;
        }
    }

    /// A path with no nodes makes no route. It does not make one that answers
    /// the origin.
    #[test]
    fn a_path_with_nothing_in_it_makes_no_route() {
        assert!(Route::build(&[], template()).is_none());
    }

    /// The heading is the curve's tangent while the tangent is within 60
    /// degrees of the segment's chord, and the chord past that.
    #[test]
    fn the_direction_is_the_tangent_until_it_leaves_the_chord() {
        // A gentle bend: the tangent is within 60 degrees of the chord
        // everywhere, so the tangent is answered.
        let gentle = Spline::new(&[
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [20.0, 2.0, 0.0],
            [30.0, 6.0, 0.0],
            [40.0, 12.0, 0.0],
        ]);
        for t in [0.0, 0.5, 1.0] {
            let tangent = normalize(gentle.tangent(0, t), TANGENT_MIN).expect("a real tangent");
            assert_eq!(gentle.direction(0, t), tangent, "t {t}");
        }
        // At the start of the segment the tangent is half the vector from the
        // point before it to the point after it, which is not the chord.
        let start = gentle.direction(0, 0.0);
        let chord = normalize(gentle.chord(0), CHORD_MIN).expect("a real chord");
        assert!((start[1] / start[0] - 0.1).abs() < 1e-4, "{start:?}");
        assert!((chord[1] / chord[0] - 0.2).abs() < 1e-4, "{chord:?}");

        // A hairpin. Segment 1 is the short cross-piece of the turn: its
        // chord runs +y, and the tangent entering it still points mostly +x.
        // The two are more than 60 degrees apart, so the chord is answered.
        let hairpin = Spline::new(&[
            [0.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [60.0, 0.0, 0.0],
            [60.0, 4.0, 0.0],
            [30.0, 4.0, 0.0],
            [0.0, 4.0, 0.0],
        ]);
        let chord = normalize(hairpin.chord(1), CHORD_MIN).expect("a real chord");
        let tangent = normalize(hairpin.tangent(1, 0.0), TANGENT_MIN).expect("a real tangent");
        let dot = chord[0] * tangent[0] + chord[1] * tangent[1] + chord[2] * tangent[2];
        assert!(dot < 0.5, "the hairpin is not sharp enough to reach the chord: {dot}");
        assert_eq!(hairpin.direction(1, 0.0), chord);
    }

    /// The heading does not jump where one segment ends and the next begins:
    /// a Catmull-Rom curve has one tangent at a node, and the heading is that
    /// tangent.
    #[test]
    fn the_heading_is_continuous_through_a_node() {
        let bend = Spline::new(&[
            [0.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
            [200.0, 30.0, 0.0],
            [300.0, 90.0, 0.0],
            [400.0, 180.0, 0.0],
            [500.0, 300.0, 0.0],
        ]);
        for node in 0..bend.segments() - 1 {
            let before = bend.direction(node, 1.0);
            let after = bend.direction(node + 1, 0.0);
            for axis in 0..3 {
                assert!(
                    (before[axis] - after[axis]).abs() < 1e-4,
                    "the heading jumps at the end of segment {node}: {before:?} -> {after:?}"
                );
            }
            // The two chords that meet there differ, which is the jump a
            // heading taken from the chord would make.
            let (a, b) = (bend.chord(node), bend.chord(node + 1));
            assert!((a[1] / a[0] - b[1] / b[0]).abs() > 0.1);
        }
    }

    /// A curve whose derivative is too short to have a direction answers +x.
    #[test]
    fn a_curve_with_no_tangent_answers_the_x_axis() {
        let still = Spline::new(&[[5.0, 5.0, 0.0]; 4]);
        assert_eq!(still.direction(0, 0.5), [1.0, 0.0, 0.0]);
    }

    /// An empty curve takes no time. The client does not measure a curve of
    /// three points or fewer, so its total length stays zero and nothing
    /// divides by a length that was never measured.
    #[test]
    fn a_leg_too_short_to_curve_measures_nothing() {
        let spline = Spline::new(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
        assert_eq!(spline.segments(), 0);
        assert_eq!(spline.total(), 0.0);
    }
}
