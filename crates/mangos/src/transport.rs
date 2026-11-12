//! A boat or a zeppelin on the server: the `gameobject_template` row that is
//! one, and the `transports` row that holds the length of its round.
//!
//! ## What makes a transport
//!
//! `TransportMgr::LoadTransportTemplates` walks every `gameobject_template`
//! row of type 15, `GAMEOBJECT_TYPE_MO_TRANSPORT`, and builds a route for each
//! from the `TaxiPathNode.dbc` rows of the path its `data0` names.
//! `SpawnTransportsOnMap` then creates each of them on every map its route
//! touches. So a transport is its template: there is no spawn row, and a
//! type 15 template with a usable path is a ship in the world after the next
//! start.
//!
//! ```text
//! gameobject_template   type 15, displayId the hull, flags 40
//!                       data0 taxiPathId, data1 moveSpeed, data2 accelRate
//! TaxiPathNode.dbc      the route: the server reads it as a file from
//!                       DataDir\5875\dbc\
//! transports            (entry, build) -> period, the length of one round in
//!                       milliseconds
//! ```
//!
//! ## Why the period is a row
//!
//! The server and the client each turn the route into a schedule, and the
//! client is never told where a ship is: it is sent how far into the round
//! the ship is and works out the place. Both ends take that number modulo the
//! round's length, so the two lengths have to be the same integer.
//! `GenerateWaypoints` measures the curve more coarsely than the client does
//! and its own total is off by about a tenth of a percent, so
//! `LoadTransportTemplates` ends by reading `period` from `transports` and
//! using it in place of its own total. The row it reads per entry is the one
//! with the highest `build` at or below the build it serves.
//!
//! A project therefore writes a transport's period as a row at build 5875
//! ([`DEV_BUILD`]), which is read in place of every shipped row and is removed
//! by a `DELETE` of a key no shipped row has. The number itself comes from
//! the client's schedule, which this crate does not hold; the editor computes
//! it and hands it to [`Period::new`]. A period of 0 is read as no override.
//!
//! ## What stops the server
//!
//! `GenerateWaypoints` does not refuse a bad route; it reads past it or
//! asserts. [`route_faults`] names the shapes that do, from the route's
//! points alone. A template whose `data0` is past the last path id is the one
//! case the function refuses cleanly, and that transport is then not loaded.
//!
//! ## An edit reaches the server at a restart
//!
//! The templates, the routes and the periods are all read once, at startup.

use crate::row::{Assignment, Key, Table};
use crate::schema::{Column, Group, Kind, Row, RowValue};

/// The table of periods.
pub const TABLE: &str = "transports";

/// The game object type a boat or a zeppelin is:
/// `GAMEOBJECT_TYPE_MO_TRANSPORT`.
pub const TYPE: u32 = 15;

/// The `flags` every shipped transport has: `TRANSPORT` and `NODESPAWN`.
pub const FLAGS: u32 = 0x28;

/// The speed and the acceleration eight of the nine shipped transports have,
/// in yards a second and yards a second per second.
pub const SPEED: u32 = 30;
pub const ACCEL: u32 = 1;

/// The build a project's period row is written at: the client build, which
/// is the highest the loader reads and above every shipped row.
pub const DEV_BUILD: u32 = crate::CLIENT_BUILD;

/// `transports`: a row may be edited, created and removed.
pub const PERIOD_TABLE: Table = Table::new(TABLE, &COLUMNS);

/// `transports`, in table order.
pub(crate) const COLUMNS: [Column; 4] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the gameobject_template entry of the transport" },
    Column { name: "build", kind: Kind::Key, group: Group::Identity, about: "the first client build the row is read for" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "a label for the route; the server does not read it" },
    Column { name: "period", kind: Kind::Unsigned, group: Group::Behaviour, about: "milliseconds one round of the route takes; 0 leaves the server's own total" },
];

/// The key of the period row a project writes for a transport.
pub fn key(entry: u32) -> Key {
    Key::two(("entry", u64::from(entry)), ("build", u64::from(DEV_BUILD)))
}

/// One row of `transports`.
#[derive(Debug, Clone, PartialEq)]
pub struct Period {
    pub entry: u32,
    pub build: u32,
    pub name: String,
    /// Milliseconds one round takes.
    pub period: u32,
}

impl Period {
    /// The row a project writes: at [`DEV_BUILD`].
    pub fn new(entry: u32, name: &str, period: u32) -> Period {
        Period { entry, build: DEV_BUILD, name: name.to_string(), period }
    }

    pub fn key(&self) -> Key {
        Key::two(("entry", u64::from(self.entry)), ("build", u64::from(self.build)))
    }

    /// Every column but the key, as a created row names them.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "name", value: crate::sql::text(&self.name) },
            Assignment { column: "period", value: self.period.to_string() },
        ]
    }

    /// A row as the database returned it.
    pub fn from_row(row: &Row) -> Option<Period> {
        let int = |column: &str| row.text(column)?.trim().parse::<u32>().ok();
        Some(Period {
            entry: int("entry")?,
            build: int("build")?,
            name: row.text("name").unwrap_or("").to_string(),
            period: int("period").unwrap_or(0),
        })
    }
}

/// Every period row the loader could read: the ones at or below the build it
/// serves, lowest build first within an entry.
pub fn periods_query() -> String {
    format!("SELECT * FROM `{TABLE}` WHERE `build` <= {DEV_BUILD} ORDER BY `entry`, `build`")
}

/// The row the loader uses for one entry: the highest build of `rows`, which
/// are [`periods_query`]'s.
pub fn winning(rows: &[Period], entry: u32) -> Option<&Period> {
    rows.iter().filter(|row| row.entry == entry).max_by_key(|row| row.build)
}

/// A transport's template, as the server loads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub entry: u32,
    /// The content patch of the row, the other half of its key.
    pub patch: u32,
    pub name: String,
    /// `GameObjectDisplayInfo.dbc` id of the hull.
    pub display: u32,
    /// `data0`: the `TaxiPath.dbc` id of the route.
    pub path: u32,
    /// `data1`: yards a second at cruise.
    pub speed: u32,
    /// `data2`: yards a second per second.
    pub accel: u32,
}

impl Template {
    /// The key its row is edited under.
    pub fn key(&self) -> Key {
        crate::gameobject::template_key(self.entry, self.patch)
    }

    /// A row of [`templates_query`].
    pub fn from_row(row: &Row) -> Option<Template> {
        let int = |column: &str| row.text(column)?.trim().parse::<i64>().ok().map(|value| value.max(0) as u32);
        Some(Template {
            entry: int("entry")?,
            patch: int("patch").unwrap_or(0),
            name: row.text("name").unwrap_or("").to_string(),
            display: int("displayId").unwrap_or(0),
            path: int("data0").unwrap_or(0),
            speed: int("data1").unwrap_or(0),
            accel: int("data2").unwrap_or(0),
        })
    }
}

/// Every transport template the server would load: the type 15 rows among
/// the winning row of each entry.
pub fn templates_query(wow_patch: u32) -> String {
    format!(
        "SELECT t.`entry`, t.`patch`, t.`name`, t.`displayId`, t.`data0`, t.`data1`, t.`data2` \
         FROM `gameobject_template` t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `gameobject_template` \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch` \
         WHERE t.`type` = {TYPE} ORDER BY t.`entry`"
    )
}

/// Every column of a new transport's template: [`crate::gameobject::new_template`]
/// with the type, the hull, the two flags every shipped transport has, and the
/// three `data` columns the type reads first.
pub fn new_template(name: &str, display: u32, path: u32, speed: u32, accel: u32) -> Vec<Assignment> {
    let set = [
        ("type", TYPE),
        ("displayId", display),
        ("flags", FLAGS),
        ("data0", path),
        ("data1", speed),
        ("data2", accel),
    ];
    crate::gameobject::new_template(name)
        .into_iter()
        .map(|mut change| {
            if let Some((_, value)) = set.iter().find(|(column, _)| *column == change.column) {
                change.value = value.to_string();
            }
            change
        })
        .collect()
}

/// One point of a route, as [`route_faults`] reads it: its map and its
/// `ActionFlag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutePoint {
    pub map: u32,
    pub action_flag: u32,
}

/// Why `GenerateWaypoints` would read past a route or assert on it, which
/// stops the server at startup. Empty for a route it can load.
///
/// The function makes a key frame of each point from the second to the one
/// before the last. A point that carries the teleport flag, or whose
/// successor is on another map, makes no key frame: it marks the key frame
/// before it as a teleport, and the point after it is skipped. Three shapes
/// fail:
///
/// * fewer than three points: the first two are read without a check, and
///   with three or more the list of key frames must not be empty;
/// * the second point is a teleport or is followed by a map change: there is
///   no key frame before it to mark;
/// * no point on a map makes a key frame: `CreateTransport` asserts that it
///   finds one on each map it spawns the transport on.
pub fn route_faults(points: &[RoutePoint]) -> Vec<String> {
    let mut out = Vec::new();
    if points.len() < 3 {
        out.push(format!(
            "the route has {} point(s); the server reads at least three and stops without them",
            points.len()
        ));
        return out;
    }
    let leaves = |i: usize| points[i].action_flag & 1 != 0 || points[i].map != points[i + 1].map;
    if leaves(1) {
        out.push(
            "the second point is a teleport or the last on its map; the server needs a point \
             it travels to before the first teleport"
                .to_string(),
        );
        return out;
    }
    let mut framed: Vec<u32> = Vec::new();
    let mut skip = false;
    for (i, point) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        if skip {
            skip = false;
        } else if leaves(i) {
            skip = true;
        } else if !framed.contains(&point.map) {
            framed.push(point.map);
        }
    }
    let mut maps: Vec<u32> = points.iter().map(|point| point.map).collect();
    maps.sort_unstable();
    maps.dedup();
    for map in maps.into_iter().filter(|map| !framed.contains(map)) {
        out.push(format!(
            "no point on map {map} is one the server travels to: each map of the route needs \
             a point that is not a teleport, not next to one, and not the first or last"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(columns: &[(&str, &str)]) -> Row {
        let mut row = Row::new();
        for (column, value) in columns {
            row.insert((*column).to_string(), Some((*value).to_string()));
        }
        row
    }

    #[test]
    fn a_projects_period_is_a_row_at_the_client_build() {
        let period = Period::new(2_000_000, "Ratchet and Booty Bay", 350_818);
        assert_eq!(period.key().where_clause(), "`entry` = 2000000 AND `build` = 5875");
        assert_eq!(period.key(), key(2_000_000));
        let statements = PERIOD_TABLE.statements(&period.key(), crate::row::Life::Insert, &period.assignments());
        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "DELETE FROM `transports` WHERE `entry` = 2000000 AND `build` = 5875;");
        assert!(statements[1].contains("'Ratchet and Booty Bay'"), "{}", statements[1]);
        assert!(statements[1].contains("350818"), "{}", statements[1]);
    }

    /// The loader takes the highest build of an entry, so a row at build 5875
    /// is read in place of the shipped ones.
    #[test]
    fn the_highest_build_of_an_entry_is_the_one_read() {
        let rows = vec![
            Period { entry: 20808, build: 0, name: String::new(), period: 339_575 },
            Period { entry: 20808, build: 4695, name: String::new(), period: 350_818 },
            Period { entry: 164871, build: 0, name: String::new(), period: 360_016 },
        ];
        assert_eq!(winning(&rows, 20808).map(|row| row.period), Some(350_818));
        assert_eq!(winning(&rows, 164871).map(|row| row.build), Some(0));
        assert!(winning(&rows, 7).is_none());
        assert!(periods_query().contains("`build` <= 5875"));
    }

    #[test]
    fn a_row_reads_back_as_the_period_and_the_template_it_was() {
        let period = Period::from_row(&row(&[("entry", "20808"), ("build", "4695"), ("name", "Ratchet"), ("period", "350818")])).unwrap();
        assert_eq!((period.entry, period.build, period.period), (20808, 4695, 350_818));
        let template = Template::from_row(&row(&[
            ("entry", "164871"),
            ("patch", "0"),
            ("name", "Zeppelin"),
            ("displayId", "3031"),
            ("data0", "302"),
            ("data1", "30"),
            ("data2", "1"),
        ]))
        .unwrap();
        assert_eq!((template.path, template.speed, template.accel, template.display), (302, 30, 1, 3031));
        assert_eq!(template.key().where_clause(), "`entry` = 164871 AND `patch` = 0");
        assert!(templates_query(10).contains("t.`type` = 15"));
    }

    /// A new transport's template names every column the table has, with the
    /// type, the hull, the flags and the route's three columns set.
    #[test]
    fn a_new_template_is_a_whole_row_of_type_15() {
        let changes = new_template("Test Ferry", 3015, 900, SPEED, ACCEL);
        let value = |column: &str| changes.iter().find(|change| change.column == column).map(|change| change.value.as_str());
        assert_eq!(value("type"), Some("15"));
        assert_eq!(value("displayId"), Some("3015"));
        assert_eq!(value("flags"), Some("40"));
        assert_eq!(value("data0"), Some("900"));
        assert_eq!(value("data1"), Some("30"));
        assert_eq!(value("data2"), Some("1"));
        assert_eq!(value("name"), Some("'Test Ferry'"));
        assert_eq!(changes.len(), crate::gameobject::new_template("x").len());
    }

    fn route(points: &[(u32, u32)]) -> Vec<RoutePoint> {
        points.iter().map(|&(map, action_flag)| RoutePoint { map, action_flag }).collect()
    }

    #[test]
    fn a_route_the_server_can_load_has_no_fault() {
        // One map, a stop near each end.
        assert!(route_faults(&route(&[(0, 0), (0, 0), (0, 2), (0, 0), (0, 2), (0, 0)])).is_empty());
        // Two maps, a teleport at each crossing.
        let two = route(&[(0, 0), (0, 2), (0, 0), (0, 1), (1, 0), (1, 2), (1, 0), (1, 1), (0, 0)]);
        assert!(route_faults(&two).is_empty(), "{:?}", route_faults(&two));
    }

    #[test]
    fn a_route_that_stops_the_server_is_named() {
        assert_eq!(route_faults(&[]).len(), 1);
        assert_eq!(route_faults(&route(&[(0, 0), (0, 0)])).len(), 1);
        // The second point is a teleport.
        assert_eq!(route_faults(&route(&[(0, 0), (0, 1), (0, 0), (0, 0)])).len(), 1);
        // The second point is the last on its map.
        assert_eq!(route_faults(&route(&[(0, 0), (0, 0), (1, 0), (1, 0)])).len(), 1);
        // Map 1 holds only the point after the teleport and the last point.
        let thin = route(&[(0, 0), (0, 0), (0, 1), (1, 0), (1, 0)]);
        let faults = route_faults(&thin);
        assert_eq!(faults.len(), 1, "{faults:?}");
        assert!(faults[0].contains("map 1"), "{faults:?}");
    }
}
