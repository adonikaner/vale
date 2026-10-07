//! Flight paths: what adding and removing a node, a path or a point of a path
//! does to the three taxi tables.
//!
//! ```text
//! TaxiNodes     a node: where a flight master's flights start and end
//! TaxiPath      one flight from one node to another, in one direction, and
//!               its cost
//! TaxiPathNode  one point of one flight, with the path's id and the point's
//!               place along it
//! ```
//!
//! The field indices are `vale_assets::tables::taxi`'s. This module only
//! states what an edit to the rows is.
//!
//! ## The two rules every edit here keeps
//!
//! * A path's points are numbered 0, 1, 2 with no gap. vmangos
//!   (`DBCStores.cpp`, the `TaxiPathNode.dbc` block) sizes each path's list to
//!   the largest `NodeIndex` plus one and fills it by index, so a gap leaves an
//!   empty slot that the flight code reads as a point. Inserting a point moves
//!   every later point up by one, and removing one moves them down.
//! * A point names a path that exists. The same loader indexes an array
//!   sized by `TaxiPath.dbc`'s largest id with each point's path id, so a point
//!   naming a path past that writes outside the array. Removing a path removes
//!   its points, and a point is only ever added to a path that exists. The
//!   shipped tables break the weaker form of this: 99 points name two paths
//!   that do not exist, at ids below the largest, which is harmless.
//!
//! [`check`] reports a table that breaks either, whoever wrote it.
//!
//! ## What an operation returns
//!
//! Each operation writes the tables as it goes and returns the rows and cells
//! it wrote, in order, as [`Edits`]. The caller puts them on the undo stack as
//! one entry. Within one table every row is added or removed before any cell is
//! written, because `undo::Change` applies a table's rows before its cells and
//! reverts them after; a cell recorded before a row removal would be reverted
//! against the wrong record.

use super::{Cell, DbcFile, Row};
use vale_assets::tables::taxi::{node_fields as nf, path_fields as pf, path_node_fields as wf};
use std::collections::HashMap;

/// The largest node id the flight map and the taxi mask can hold, re-exported
/// for `vale-mangos`, which reads the client's files only through this
/// crate.
pub use vale_assets::tables::taxi::MAX_NODE_ID;

/// The three tables, by the names the caller's table map uses.
pub const NODES: &str = "TaxiNodes";
pub const PATHS: &str = "TaxiPath";
pub const POINTS: &str = "TaxiPathNode";

/// Every table an operation here may write, in the order they are opened.
pub const TABLES: [&str; 3] = [NODES, PATHS, POINTS];

/// What an operation wrote, for the undo stack.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Edits {
    /// Every record added or removed, with its table, in order.
    pub rows: Vec<(String, Row)>,
    /// Every field written, with its table, in order.
    pub cells: Vec<(String, Cell)>,
    /// The id of the node, path or point the operation made, if it made one.
    pub made: Option<u32>,
}

impl Edits {
    fn row(&mut self, table: &str, row: Row) {
        self.rows.push((table.to_string(), row));
    }

    fn cell(&mut self, table: &str, cell: Cell) {
        if cell.moves() {
            self.cells.push((table.to_string(), cell));
        }
    }
}

/// One `TaxiNodes` row, as read.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: u32,
    pub record: usize,
    pub map: u32,
    /// x, y, z in the server's axes: north, west, up.
    pub at: [f32; 3],
    pub name: String,
    /// Fields 14 and 15.
    pub mounts: [u32; 2],
}

/// One `TaxiPath` row, as read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Path {
    pub id: u32,
    pub record: usize,
    pub from: u32,
    pub to: u32,
    /// Copper.
    pub cost: u32,
}

/// One `TaxiPathNode` row, as read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub id: u32,
    pub record: usize,
    pub path: u32,
    pub index: u32,
    pub map: u32,
    pub at: [f32; 3],
    pub action_flag: u32,
    pub delay: u32,
}

/// What a new point is made of: everything but its id, path and index, which
/// the operation assigns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointSpec {
    pub map: u32,
    pub at: [f32; 3],
    pub action_flag: u32,
    pub delay: u32,
}

impl PointSpec {
    /// A flight's point: no stop, no delay.
    pub fn flying(map: u32, at: [f32; 3]) -> PointSpec {
        PointSpec {
            map,
            at,
            action_flag: 0,
            delay: 0,
        }
    }
}

/// Every node in the table, in file order.
pub fn nodes(table: &DbcFile) -> Vec<Node> {
    (0..table.record_count())
        .filter_map(|record| node_at(table, record))
        .collect()
}

/// The node at one record.
pub fn node_at(table: &DbcFile, record: usize) -> Option<Node> {
    let float = |field: usize| table.f32_at(record, field).unwrap_or(0.0);
    Some(Node {
        id: table.u32_at(record, nf::ID)?,
        record,
        map: table.u32_at(record, nf::MAP)?,
        at: [float(nf::X), float(nf::Y), float(nf::Z)],
        name: table.string_at(record, nf::NAME).unwrap_or_default(),
        mounts: [
            table.u32_at(record, nf::MOUNT).unwrap_or(0),
            table.u32_at(record, nf::MOUNT + 1).unwrap_or(0),
        ],
    })
}

/// Every path in the table, in file order.
pub fn paths(table: &DbcFile) -> Vec<Path> {
    (0..table.record_count())
        .filter_map(|record| path_at(table, record))
        .collect()
}

/// The path at one record.
pub fn path_at(table: &DbcFile, record: usize) -> Option<Path> {
    Some(Path {
        id: table.u32_at(record, pf::ID)?,
        record,
        from: table.u32_at(record, pf::FROM)?,
        to: table.u32_at(record, pf::TO)?,
        cost: table.u32_at(record, pf::COST)?,
    })
}

/// Every point in the table, grouped by path and in `NodeIndex` order within
/// each. One pass over the table, for a caller that needs every path's points.
pub fn points_by_path(table: &DbcFile) -> HashMap<u32, Vec<Point>> {
    let mut out: HashMap<u32, Vec<Point>> = HashMap::new();
    for record in 0..table.record_count() {
        if let Some(point) = point_at(table, record) {
            out.entry(point.path).or_default().push(point);
        }
    }
    for points in out.values_mut() {
        points.sort_by_key(|point| point.index);
    }
    out
}

/// One path's points, in `NodeIndex` order.
pub fn points_of(table: &DbcFile, path: u32) -> Vec<Point> {
    let mut out: Vec<Point> = (0..table.record_count())
        .filter(|&record| table.u32_at(record, wf::PATH) == Some(path))
        .filter_map(|record| point_at(table, record))
        .collect();
    out.sort_by_key(|point| point.index);
    out
}

/// The point at one record.
pub fn point_at(table: &DbcFile, record: usize) -> Option<Point> {
    let float = |field: usize| table.f32_at(record, field).unwrap_or(0.0);
    Some(Point {
        id: table.u32_at(record, wf::ID)?,
        record,
        path: table.u32_at(record, wf::PATH)?,
        index: table.u32_at(record, wf::INDEX)?,
        map: table.u32_at(record, wf::MAP)?,
        at: [float(wf::X), float(wf::Y), float(wf::Z)],
        action_flag: table.u32_at(record, wf::ACTION_FLAG).unwrap_or(0),
        delay: table.u32_at(record, wf::DELAY).unwrap_or(0),
    })
}

/// The path from one node to another, if the table has one.
pub fn path_between(table: &DbcFile, from: u32, to: u32) -> Option<Path> {
    paths(table)
        .into_iter()
        .find(|path| path.from == from && path.to == to)
}

/// Why an operation was refused. Each is a sentence for a status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// One of the three tables is not in the map the caller passed.
    NotOpen(&'static str),
    NoSuchNode(u32),
    NoSuchPath(u32),
    NoSuchPoint(u32),
    /// A path from a node to itself.
    SameNode,
    /// The table already has a path in this direction between the two nodes.
    /// The client keys its path array on `(from, to)`, so a second one would
    /// shadow the first.
    AlreadyConnected { from: u32, to: u32, path: u32 },
    /// A path needs a start and an end.
    TooFewPoints,
    /// The next node id is past [`MAX_NODE_ID`], which neither side can use.
    NodeIdsExhausted,
    /// The record could not be written; the table is not the width this file
    /// expects.
    WrongWidth(&'static str),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NotOpen(table) => write!(f, "{table}.dbc is not open"),
            Refused::NoSuchNode(id) => write!(f, "TaxiNodes has no node {id}"),
            Refused::NoSuchPath(id) => write!(f, "TaxiPath has no path {id}"),
            Refused::NoSuchPoint(id) => write!(f, "TaxiPathNode has no point {id}"),
            Refused::SameNode => write!(f, "a path cannot start and end at the same node"),
            Refused::AlreadyConnected { from, to, path } => {
                write!(f, "path {path} already connects node {from} to node {to}")
            }
            Refused::TooFewPoints => write!(f, "a path needs at least two points"),
            Refused::NodeIdsExhausted => write!(
                f,
                "node ids stop at {MAX_NODE_ID}: the taxi mask has no bit for a higher one"
            ),
            Refused::WrongWidth(table) => write!(f, "{table}.dbc does not have the 1.12 record size"),
        }
    }
}

fn open<'a>(
    tables: &'a mut HashMap<String, DbcFile>,
    name: &'static str,
) -> Result<&'a mut DbcFile, Refused> {
    tables.get_mut(name).ok_or(Refused::NotOpen(name))
}

/// Append a record with `fields` written into it, and note it as added.
fn append(
    table: &mut DbcFile,
    name: &'static str,
    id: u32,
    fields: &[(usize, u32)],
) -> Result<Row, Refused> {
    let bytes = table.blank_record(id);
    let at = table.push_record(&bytes).ok_or(Refused::WrongWidth(name))?;
    for &(field, value) in fields {
        if !table.set_u32(at, field, value) {
            table.remove_record(at);
            return Err(Refused::WrongWidth(name));
        }
    }
    Row::added(table, at).ok_or(Refused::WrongWidth(name))
}

fn point_fields(path: u32, index: u32, spec: &PointSpec) -> [(usize, u32); 8] {
    [
        (wf::PATH, path),
        (wf::INDEX, index),
        (wf::MAP, spec.map),
        (wf::X, spec.at[0].to_bits()),
        (wf::Y, spec.at[1].to_bits()),
        (wf::Z, spec.at[2].to_bits()),
        (wf::ACTION_FLAG, spec.action_flag),
        (wf::DELAY, spec.delay),
    ]
}

/// Remove one record, noting it first.
fn take(table: &mut DbcFile, name: &'static str, record: usize) -> Result<Row, Refused> {
    let row = Row::removed(table, record).ok_or(Refused::WrongWidth(name))?;
    table.remove_record(record);
    Ok(row)
}

/// Put a new point into a path at `index`, moving every point at or after that
/// index up by one. `index` may be the path's length, which appends. Returns
/// the new point's id in [`Edits::made`].
pub fn insert_point(
    tables: &mut HashMap<String, DbcFile>,
    path: u32,
    index: u32,
    spec: PointSpec,
) -> Result<Edits, Refused> {
    if open(tables, PATHS)?.row_of(path).is_none() {
        return Err(Refused::NoSuchPath(path));
    }
    let table = open(tables, POINTS)?;
    let before = points_of(table, path);
    let index = index.min(before.len() as u32);
    let id = table.max_id() + 1;
    let mut out = Edits::default();
    let row = append(table, POINTS, id, &point_fields(path, index, &spec))?;
    out.row(POINTS, row);
    // The records of the points already there did not move: the new one was
    // appended after them.
    for point in before.iter().filter(|point| point.index >= index) {
        if let Some(cell) = Cell::new(table, point.record, wf::INDEX, point.index + 1) {
            cell.apply(table);
            out.cell(POINTS, cell);
        }
    }
    out.made = Some(id);
    Ok(out)
}

/// Take one point out of its path, moving every later point down by one.
/// Refused when it would leave the path with fewer than two points.
pub fn remove_point(tables: &mut HashMap<String, DbcFile>, point: u32) -> Result<Edits, Refused> {
    let table = open(tables, POINTS)?;
    let record = table.row_of(point).ok_or(Refused::NoSuchPoint(point))?;
    let gone = point_at(table, record).ok_or(Refused::NoSuchPoint(point))?;
    if points_of(table, gone.path).len() <= 2 {
        return Err(Refused::TooFewPoints);
    }
    let mut out = Edits::default();
    out.row(POINTS, take(table, POINTS, record)?);
    // Read again: every record after the removed one has moved down by one.
    for later in points_of(table, gone.path)
        .into_iter()
        .filter(|later| later.index > gone.index)
    {
        if let Some(cell) = Cell::new(table, later.record, wf::INDEX, later.index - 1) {
            cell.apply(table);
            out.cell(POINTS, cell);
        }
    }
    Ok(out)
}

/// Make a path from one node to another through `points`, which are taken in
/// order and numbered from 0. Returns the new path's id in [`Edits::made`].
pub fn new_path(
    tables: &mut HashMap<String, DbcFile>,
    from: u32,
    to: u32,
    cost: u32,
    points: &[PointSpec],
) -> Result<Edits, Refused> {
    if from == to {
        return Err(Refused::SameNode);
    }
    if points.len() < 2 {
        return Err(Refused::TooFewPoints);
    }
    {
        let nodes = open(tables, NODES)?;
        for node in [from, to] {
            if nodes.row_of(node).is_none() {
                return Err(Refused::NoSuchNode(node));
            }
        }
    }
    let path_table = open(tables, PATHS)?;
    if let Some(had) = path_between(path_table, from, to) {
        return Err(Refused::AlreadyConnected {
            from,
            to,
            path: had.id,
        });
    }
    let id = path_table.max_id() + 1;
    let mut out = Edits::default();
    let row = append(
        path_table,
        PATHS,
        id,
        &[(pf::FROM, from), (pf::TO, to), (pf::COST, cost)],
    )?;
    out.row(PATHS, row);
    let point_table = open(tables, POINTS)?;
    let mut next = point_table.max_id() + 1;
    for (index, spec) in points.iter().enumerate() {
        let row = append(
            point_table,
            POINTS,
            next,
            &point_fields(id, index as u32, spec),
        )?;
        out.row(POINTS, row);
        next += 1;
    }
    out.made = Some(id);
    Ok(out)
}

/// Make the path back: the same points in the opposite order, between the same
/// two nodes the other way round, at the same cost. 270 of the 275 shipped
/// flights have one, as a row of its own; `vale taxi` lists the five that
/// do not.
pub fn reverse_path(tables: &mut HashMap<String, DbcFile>, path: u32) -> Result<Edits, Refused> {
    let paths_table = open(tables, PATHS)?;
    let record = paths_table.row_of(path).ok_or(Refused::NoSuchPath(path))?;
    let there = path_at(paths_table, record).ok_or(Refused::NoSuchPath(path))?;
    let points: Vec<PointSpec> = points_of(open(tables, POINTS)?, path)
        .iter()
        .rev()
        .map(|point| PointSpec {
            map: point.map,
            at: point.at,
            action_flag: point.action_flag,
            delay: point.delay,
        })
        .collect();
    new_path(tables, there.to, there.from, there.cost, &points)
}

/// Remove a path and every point of it.
pub fn remove_path(tables: &mut HashMap<String, DbcFile>, path: u32) -> Result<Edits, Refused> {
    let record = open(tables, PATHS)?
        .row_of(path)
        .ok_or(Refused::NoSuchPath(path))?;
    let mut out = Edits::default();
    let point_table = open(tables, POINTS)?;
    // Highest record first, so each removal leaves the records still to be
    // removed where they were read.
    let mut records: Vec<usize> = points_of(point_table, path)
        .iter()
        .map(|point| point.record)
        .collect();
    records.sort_unstable_by(|a, b| b.cmp(a));
    for at in records {
        out.row(POINTS, take(point_table, POINTS, at)?);
    }
    let path_table = open(tables, PATHS)?;
    out.row(PATHS, take(path_table, PATHS, record)?);
    Ok(out)
}

/// Make a node. `mounts` is fields 14 and 15: the Horde mount then the
/// Alliance mount, as vmangos reads them. Returns the new node's id in
/// [`Edits::made`].
pub fn new_node(
    tables: &mut HashMap<String, DbcFile>,
    map: u32,
    at: [f32; 3],
    name: &str,
    mounts: [u32; 2],
) -> Result<Edits, Refused> {
    let table = open(tables, NODES)?;
    let id = table.max_id() + 1;
    if id > MAX_NODE_ID {
        return Err(Refused::NodeIdsExhausted);
    }
    let bytes = table.blank_record(id);
    let record = table.push_record(&bytes).ok_or(Refused::WrongWidth(NODES))?;
    let words = [
        (nf::MAP, map),
        (nf::X, at[0].to_bits()),
        (nf::Y, at[1].to_bits()),
        (nf::Z, at[2].to_bits()),
        (nf::MOUNT, mounts[0]),
        (nf::MOUNT + 1, mounts[1]),
    ];
    for (field, value) in words {
        if !table.set_u32(record, field, value) {
            table.remove_record(record);
            return Err(Refused::WrongWidth(NODES));
        }
    }
    // The name is appended to the string block before the record is noted, so
    // the noted bytes carry its offset. The append is not undone and does not
    // need to be: see the module comment in `dbc/mod.rs`.
    table.set_string(record, nf::NAME, name);
    let mut out = Edits::default();
    out.row(
        NODES,
        Row::added(table, record).ok_or(Refused::WrongWidth(NODES))?,
    );
    out.made = Some(id);
    Ok(out)
}

/// Remove a node, every path that starts or ends at it, and their points.
pub fn remove_node(tables: &mut HashMap<String, DbcFile>, node: u32) -> Result<Edits, Refused> {
    let record = open(tables, NODES)?
        .row_of(node)
        .ok_or(Refused::NoSuchNode(node))?;
    let touching: Vec<u32> = paths(open(tables, PATHS)?)
        .into_iter()
        .filter(|path| path.from == node || path.to == node)
        .map(|path| path.id)
        .collect();
    let mut out = Edits::default();
    for path in touching {
        let done = remove_path(tables, path)?;
        out.rows.extend(done.rows);
    }
    let table = open(tables, NODES)?;
    out.row(NODES, take(table, NODES, record)?);
    Ok(out)
}

/// Renumber a path's points 0, 1, 2 in their current order. `None` when they
/// already are, which is every shipped path.
pub fn compact(tables: &mut HashMap<String, DbcFile>, path: u32) -> Result<Option<Edits>, Refused> {
    let table = open(tables, POINTS)?;
    let mut out = Edits::default();
    for (want, point) in points_of(table, path).iter().enumerate() {
        if let Some(cell) = Cell::new(table, point.record, wf::INDEX, want as u32) {
            cell.apply(table);
            out.cell(POINTS, cell);
        }
    }
    Ok((!out.cells.is_empty()).then_some(out))
}

/// The length of a path in yards, point to point, over the points on one map.
/// A segment whose two ends are on different maps is not counted, because it
/// is a transport's crossing rather than a distance flown.
pub fn length(points: &[Point]) -> f32 {
    points
        .windows(2)
        .filter(|pair| pair[0].map == pair[1].map)
        .map(|pair| distance(pair[0].at, pair[1].at))
        .sum()
}

pub fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// What [`check`] reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    /// A path's point indices are not 0, 1, 2 with no gap or repeat. vmangos
    /// leaves an empty slot at each gap.
    NotContiguous { path: u32, indices: Vec<u32> },
    /// A point names a path id past the largest `TaxiPath` has. vmangos sizes
    /// its per-path array by that largest id and writes this point outside it.
    OutOfRange { point: u32, path: u32, highest: u32 },
    /// A path names a node the table does not have.
    Dangling { path: u32, node: u32 },
    /// A flight, both of whose nodes name a mount, with fewer than two points.
    Short { path: u32, points: usize },
    /// A node id the flight map cannot reach.
    NodeTooHigh { node: u32 },
    /// Two paths between the same two nodes in the same direction. The client
    /// keys its path array on the pair, so one of them is never flown.
    Duplicate { from: u32, to: u32, paths: [u32; 2] },
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Finding::NotContiguous { path, indices } => {
                write!(f, "path {path}: point indices are {indices:?}, not 0..{}", indices.len())
            }
            Finding::OutOfRange {
                point,
                path,
                highest,
            } => write!(
                f,
                "point {point} names path {path}, above the largest path id {highest}"
            ),
            Finding::Dangling { path, node } => {
                write!(f, "path {path} names node {node}, which does not exist")
            }
            Finding::Short { path, points } => write!(f, "path {path} has {points} point(s); a flight needs at least two"),
            Finding::NodeTooHigh { node } => {
                write!(f, "node {node} is above {MAX_NODE_ID}; the taxi mask has no bit for it")
            }
            Finding::Duplicate { from, to, paths } => write!(
                f,
                "paths {} and {} both connect node {from} to node {to}",
                paths[0], paths[1]
            ),
        }
    }
}

/// Everything in the three tables that breaks a flight or the server's
/// loader. Empty for the shipped tables.
///
/// Two things in the shipped tables look like faults and are not, and are
/// left out:
///
/// * 99 points name paths 248 and 403, which `TaxiPath.dbc` does not have.
///   Both ids are below its largest, so vmangos files them in slots nothing
///   reads. [`unused_points`] counts them.
/// * Path 472 has no points. It runs from node 81 to node 24, neither of which
///   names a mount, so no flight master offers it. [`Finding::Short`] is for
///   flights only.
pub fn check(nodes_table: &DbcFile, paths_table: &DbcFile, points_table: &DbcFile) -> Vec<Finding> {
    let mut out = Vec::new();
    let every = nodes(nodes_table);
    let mounted: HashMap<u32, bool> = every
        .iter()
        .map(|node| (node.id, node.mounts != [0, 0]))
        .collect();
    let mut ids: Vec<u32> = every.iter().map(|node| node.id).collect();
    ids.sort_unstable();
    for node in ids.into_iter().filter(|&id| id > MAX_NODE_ID) {
        out.push(Finding::NodeTooHigh { node });
    }
    let all_paths = paths(paths_table);
    let highest = all_paths.iter().map(|path| path.id).max().unwrap_or(0);
    let mut seen: HashMap<(u32, u32), u32> = HashMap::new();
    let by_path = points_by_path(points_table);
    for path in &all_paths {
        for node in [path.from, path.to] {
            if !mounted.contains_key(&node) {
                out.push(Finding::Dangling {
                    path: path.id,
                    node,
                });
            }
        }
        if let Some(&other) = seen.get(&(path.from, path.to)) {
            out.push(Finding::Duplicate {
                from: path.from,
                to: path.to,
                paths: [other, path.id],
            });
        } else {
            seen.insert((path.from, path.to), path.id);
        }
        let points = by_path.get(&path.id).map_or(&[][..], Vec::as_slice);
        let flight = [path.from, path.to]
            .iter()
            .all(|end| mounted.get(end).copied().unwrap_or(false));
        if flight && points.len() < 2 {
            out.push(Finding::Short {
                path: path.id,
                points: points.len(),
            });
        }
        let indices: Vec<u32> = points.iter().map(|point| point.index).collect();
        if indices.iter().enumerate().any(|(want, &had)| had != want as u32) {
            out.push(Finding::NotContiguous {
                path: path.id,
                indices,
            });
        }
    }
    let mut outside: Vec<(u32, u32)> = by_path
        .iter()
        .filter(|(&path, _)| path > highest)
        .flat_map(|(&path, points)| points.iter().map(move |point| (point.id, path)))
        .collect();
    outside.sort_unstable();
    out.extend(
        outside
            .into_iter()
            .map(|(point, path)| Finding::OutOfRange {
                point,
                path,
                highest,
            }),
    );
    out
}

/// How many points name a path `TaxiPath.dbc` does not have, at an id below
/// its largest. Nothing flies them and nothing breaks.
pub fn unused_points(paths_table: &DbcFile, points_table: &DbcFile) -> usize {
    let ids: std::collections::HashSet<u32> =
        paths(paths_table).iter().map(|path| path.id).collect();
    let highest = ids.iter().copied().max().unwrap_or(0);
    points_by_path(points_table)
        .iter()
        .filter(|(path, _)| !ids.contains(path) && **path <= highest)
        .map(|(_, points)| points.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of `fields` words per record and the given rows, with a string
    /// block holding "" and whatever `text` adds.
    fn table(fields: usize, rows: &[Vec<u32>]) -> DbcFile {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(fields as u32).to_le_bytes());
        bytes.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for row in rows {
            for field in 0..fields {
                bytes.extend_from_slice(&row.get(field).copied().unwrap_or(0).to_le_bytes());
            }
        }
        bytes.push(0);
        DbcFile::parse(&bytes).expect("a table this test builds")
    }

    fn point_row(id: u32, path: u32, index: u32, x: f32) -> Vec<u32> {
        vec![id, path, index, 0, x.to_bits(), 0f32.to_bits(), 50f32.to_bits(), 0, 0]
    }

    /// Two nodes on map 0, one path 1 -> 2 with three points.
    fn tables() -> HashMap<String, DbcFile> {
        let node = |id: u32, x: f32| {
            let mut row = vec![0u32; nf::COUNT];
            row[nf::ID] = id;
            row[nf::X] = x.to_bits();
            row[nf::MOUNT] = 2224;
            row
        };
        let nodes = table(nf::COUNT, &[node(1, 0.0), node(2, 100.0)]);
        let paths = table(pf::COUNT, &[vec![10, 1, 2, 500]]);
        let points = table(
            wf::COUNT,
            &[
                point_row(100, 10, 0, 0.0),
                point_row(101, 10, 1, 50.0),
                point_row(102, 10, 2, 100.0),
            ],
        );
        HashMap::from([
            (NODES.to_string(), nodes),
            (PATHS.to_string(), paths),
            (POINTS.to_string(), points),
        ])
    }

    fn indices(tables: &HashMap<String, DbcFile>, path: u32) -> Vec<(u32, u32)> {
        points_of(&tables[POINTS], path)
            .iter()
            .map(|point| (point.id, point.index))
            .collect()
    }

    /// Undo an operation the way `undo::Change::revert_table` does: each
    /// table's cells in reverse, then its rows in reverse.
    fn revert(tables: &mut HashMap<String, DbcFile>, edits: &Edits) {
        for name in TABLES {
            let table = tables.get_mut(name).unwrap();
            for (_, cell) in edits.cells.iter().filter(|(had, _)| had == name).rev() {
                cell.revert(table);
            }
            for (_, row) in edits.rows.iter().filter(|(had, _)| had == name).rev() {
                row.revert(table);
            }
        }
    }

    /// …and redo it the way `apply_table` does: rows, then cells.
    fn apply(tables: &mut HashMap<String, DbcFile>, edits: &Edits) {
        for name in TABLES {
            let table = tables.get_mut(name).unwrap();
            for (_, row) in edits.rows.iter().filter(|(had, _)| had == name) {
                row.apply(table);
            }
            for (_, cell) in edits.cells.iter().filter(|(had, _)| had == name) {
                cell.apply(table);
            }
        }
    }

    fn bytes(tables: &HashMap<String, DbcFile>) -> Vec<Vec<u8>> {
        TABLES.iter().map(|name| tables[*name].write()).collect()
    }

    #[test]
    fn an_inserted_point_moves_the_later_ones_up() {
        let mut t = tables();
        let done = insert_point(&mut t, 10, 1, PointSpec::flying(0, [25.0, 0.0, 60.0])).unwrap();
        let new = done.made.unwrap();
        assert_eq!(new, 103);
        assert_eq!(indices(&t, 10), [(100, 0), (new, 1), (101, 2), (102, 3)]);
        assert!(check(&t[NODES], &t[PATHS], &t[POINTS]).is_empty());
    }

    #[test]
    fn a_point_past_the_end_is_appended() {
        let mut t = tables();
        let done = insert_point(&mut t, 10, 99, PointSpec::flying(0, [150.0, 0.0, 60.0])).unwrap();
        assert!(done.cells.is_empty(), "nothing after it to move");
        assert_eq!(indices(&t, 10).last(), Some(&(103, 3)));
    }

    #[test]
    fn a_removed_point_moves_the_later_ones_down() {
        let mut t = tables();
        remove_point(&mut t, 101).unwrap();
        assert_eq!(indices(&t, 10), [(100, 0), (102, 1)]);
        assert!(check(&t[NODES], &t[PATHS], &t[POINTS]).is_empty());
        // Two points left: the next removal is refused.
        assert_eq!(remove_point(&mut t, 100), Err(Refused::TooFewPoints));
    }

    /// Every operation undoes to the bytes it started from and redoes to the
    /// bytes it produced, under the undo stack's own order.
    #[test]
    fn every_operation_undoes_and_redoes_exactly() {
        type Op = fn(&mut HashMap<String, DbcFile>) -> Result<Edits, Refused>;
        let ops: [(&str, Op); 7] = [
            ("insert", |t| insert_point(t, 10, 1, PointSpec::flying(0, [1.0, 2.0, 3.0]))),
            ("remove point", |t| remove_point(t, 100)),
            ("new path", |t| {
                new_path(
                    t,
                    2,
                    1,
                    7,
                    &[PointSpec::flying(0, [100.0, 0.0, 0.0]), PointSpec::flying(0, [0.0, 0.0, 0.0])],
                )
            }),
            ("reverse", |t| reverse_path(t, 10)),
            ("remove path", |t| remove_path(t, 10)),
            ("new node", |t| new_node(t, 0, [5.0, 6.0, 7.0], "Test Node", [2224, 541])),
            ("remove node", |t| remove_node(t, 1)),
        ];
        for (name, op) in ops {
            let mut t = tables();
            let start = bytes(&t);
            let done = op(&mut t).unwrap_or_else(|e| panic!("{name}: {e}"));
            let end = bytes(&t);
            assert_ne!(start, end, "{name} changed nothing");
            revert(&mut t, &done);
            // The string block keeps an appended name after an undo, which is
            // the container's rule; compare the records alone for the node.
            if name == "new node" {
                assert_eq!(t[NODES].record_count(), 2, "{name}");
            } else {
                assert_eq!(bytes(&t), start, "{name} did not undo to the start");
            }
            apply(&mut t, &done);
            assert_eq!(bytes(&t), end, "{name} did not redo to the end");
        }
    }

    #[test]
    fn a_reversed_path_flies_the_same_points_backwards() {
        let mut t = tables();
        let done = reverse_path(&mut t, 10).unwrap();
        let back = done.made.unwrap();
        let path = path_between(&t[PATHS], 2, 1).unwrap();
        assert_eq!((path.id, path.cost), (back, 500));
        let xs: Vec<f32> = points_of(&t[POINTS], back).iter().map(|p| p.at[0]).collect();
        assert_eq!(xs, [100.0, 50.0, 0.0]);
        // …and a second one is refused, because the client keys paths on
        // (from, to).
        assert_eq!(
            reverse_path(&mut t, 10),
            Err(Refused::AlreadyConnected {
                from: 2,
                to: 1,
                path: back
            })
        );
    }

    #[test]
    fn removing_a_node_removes_its_paths_and_their_points() {
        let mut t = tables();
        reverse_path(&mut t, 10).unwrap();
        remove_node(&mut t, 2).unwrap();
        assert_eq!(t[NODES].record_count(), 1);
        assert_eq!(t[PATHS].record_count(), 0);
        assert_eq!(t[POINTS].record_count(), 0);
        assert!(check(&t[NODES], &t[PATHS], &t[POINTS]).is_empty());
    }

    #[test]
    fn a_new_node_carries_its_name_and_mounts() {
        let mut t = tables();
        let done = new_node(&mut t, 1, [1.0, 2.0, 3.0], "Test Node", [2224, 541]).unwrap();
        let node = nodes(&t[NODES]).into_iter().find(|n| Some(n.id) == done.made).unwrap();
        assert_eq!(node.name, "Test Node");
        assert_eq!(node.mounts, [2224, 541]);
        assert_eq!((node.map, node.at), (1, [1.0, 2.0, 3.0]));
    }

    #[test]
    fn a_node_id_past_the_mask_is_refused() {
        let mut t = tables();
        t.get_mut(NODES).unwrap().set_u32(1, nf::ID, MAX_NODE_ID);
        assert_eq!(
            new_node(&mut t, 0, [0.0; 3], "x", [0, 0]),
            Err(Refused::NodeIdsExhausted)
        );
    }

    #[test]
    fn the_checks_find_each_kind_of_damage() {
        let t = tables();
        let broken_points = table(
            wf::COUNT,
            &[
                point_row(1, 10, 0, 0.0),
                point_row(2, 10, 2, 0.0),
                point_row(3, 77, 0, 0.0),
                point_row(4, 5, 0, 0.0),
            ],
        );
        let broken_paths = table(
            pf::COUNT,
            &[vec![10, 1, 9, 0], vec![11, 1, 9, 0], vec![12, 1, 2, 0]],
        );
        let found = check(&t[NODES], &broken_paths, &broken_points);
        assert!(found.contains(&Finding::NotContiguous {
            path: 10,
            indices: vec![0, 2]
        }));
        assert!(found.contains(&Finding::OutOfRange {
            point: 3,
            path: 77,
            highest: 12
        }));
        assert!(found.contains(&Finding::Dangling { path: 10, node: 9 }));
        assert!(found.contains(&Finding::Duplicate {
            from: 1,
            to: 9,
            paths: [10, 11]
        }));
        // Path 12 is a flight between two mounted nodes with no points; path
        // 11 names a node that does not exist, so it is not a flight.
        assert!(found.contains(&Finding::Short { path: 12, points: 0 }));
        assert!(!found.contains(&Finding::Short { path: 11, points: 0 }));
        // Point 4 names path 5, which is missing and below the largest id.
        assert!(!found.iter().any(|f| matches!(f, Finding::OutOfRange { point: 4, .. })));
        assert_eq!(unused_points(&broken_paths, &broken_points), 1);
        // …and compact repairs the gap.
        let mut repaired = tables();
        repaired.insert(POINTS.to_string(), broken_points);
        let edits = compact(&mut repaired, 10).unwrap().unwrap();
        assert_eq!(edits.cells.len(), 1);
        assert_eq!(indices(&repaired, 10), [(1, 0), (2, 1)]);
        assert_eq!(compact(&mut repaired, 10).unwrap(), None);
    }

    #[test]
    fn a_path_needs_two_nodes_and_two_points() {
        let mut t = tables();
        let two = [PointSpec::flying(0, [0.0; 3]), PointSpec::flying(0, [1.0; 3])];
        assert_eq!(new_path(&mut t, 1, 1, 0, &two), Err(Refused::SameNode));
        assert_eq!(new_path(&mut t, 1, 3, 0, &two), Err(Refused::NoSuchNode(3)));
        assert_eq!(new_path(&mut t, 2, 1, 0, &two[..1]), Err(Refused::TooFewPoints));
    }

    #[test]
    fn a_crossing_between_maps_is_not_counted_as_length() {
        let a = |map: u32, x: f32| Point {
            id: 0,
            record: 0,
            path: 0,
            index: 0,
            map,
            at: [x, 0.0, 0.0],
            action_flag: 0,
            delay: 0,
        };
        assert_eq!(length(&[a(0, 0.0), a(0, 3.0), a(1, 1000.0), a(1, 1004.0)]), 7.0);
    }
}
