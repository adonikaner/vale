//! **A waypoint path**: the rows `creature_movement` holds for one creature,
//! and what an edit to them is.
//!
//! ## The unit of edit is the whole path, not the row
//!
//! [`crate::row::Edits`] is a column store — a table, a key, a column, a
//! literal — and it is the right shape for `creature_template`, where an edit
//! sets one of 78 columns of a row that already exists. It is the wrong shape
//! here, for a reason that is in vmangos' source rather than in taste.
//!
//! `WaypointManager::Cleanup` runs at **every server start** and asks each
//! movement table whether its points are a dense sequence per path:
//!
//! ```sql
//! SELECT 1 FROM `creature_movement` AS T
//!  WHERE `point` <> (SELECT COUNT(*) FROM `creature_movement`
//!                     WHERE `id` = T.`id` AND `point` <= T.`point`) LIMIT 1
//! ```
//!
//! If one row answers, vmangos drops the primary key, renumbers the whole
//! table's `point` column and puts the key back. `AddNode` and `DeleteNode` do
//! the same incrementally, with a cascade of `UPDATE … SET point = point ± 1`
//! over every node after the one that moved.
//!
//! So a per-row store would be wrong twice. It would have to reproduce that
//! renumbering arithmetic; and its `(id, point)` keys would **move under it**
//! the next time the server tidied the table, which is an undo file addressing
//! rows that are no longer the rows it was written for. That failure has been
//! paid for once already in this crate — see `spell::fits` and the `smallint`
//! clamp that wrote an undo matching nothing.
//!
//! A path edit is therefore one `DELETE` of everything under the key, then an
//! `INSERT` per node numbered `1..N`. The undo is the path that was there,
//! restored the same way. Nothing renumbers, a gap cannot be written, and the
//! thing the undo names is the same thing the edit named.
//!
//! ## `point` is 1-based on the wire to the database and 0-based in memory
//!
//! `WaypointManager::Load` reads a row and stores it at `path[point - 1]` under
//! `MANGOS_ASSERT(point >= 1)`; `AddNode` inserts at `pointId + 1`. This module
//! holds nodes in a `Vec` — index 0 is the first node the creature walks to —
//! and writes `index + 1` into the column. [`Path::nodes`] is the order a
//! person sees and the order the creature walks; the column is an
//! implementation detail of the table.
//!
//! ## A path is only walked when the movement type says so, and two types do
//!
//! `creature.movement_type`, else the template's. **Two of the four types a
//! database row may carry walk this path**, and they walk it differently:
//!
//! * `WAYPOINT_MOTION_TYPE = 2` — node to node, honouring each node's
//!   `waittime` and facing on arrival. `WaypointMovementGenerator`.
//! * `CYCLIC_MOTION_TYPE = 3` — the whole path as **one spline**, flown round
//!   without stopping. `CyclicMovementGenerator::LoadPath` calls the same
//!   `GetDefaultPath`, and refuses a path of fewer than two nodes.
//!
//! The other two — idle and random — walk none, and writing a path for one
//! applies cleanly, reloads cleanly and does nothing. That is this subject's
//! instance of the failure the whole of [`crate::creature`]'s patch handling
//! exists to avoid. Nothing here can set the column, because it belongs to
//! another table; [`Walk::of`] is what a caller asks so that it can say so.
//!
//! The second type cost a run to find: a first draft of this module had only
//! type 2, and `vale waypoints 3344` reported a Blackrock Drake with a
//! hand-authored eighteen-node patrol as a path that is never walked.
//!
//! ## Which path a creature uses
//!
//! `WaypointManager::GetDefaultPath` takes `creature_movement` keyed by the
//! spawn's **guid** first, and falls back to `creature_movement_template` keyed
//! by the template's **entry**. Both tables have the same ten columns under a
//! different key name, so [`Which`] is the difference and everything else here
//! is shared.
//!
//! ## What is deliberately not written
//!
//! `creature_movement_special`, the third table, whose paths are reached by a
//! node's `path_id` rather than by a creature. It is a path of a path, it has
//! no position of its own in any spawn, and nothing in this editor draws one
//! yet.

use crate::row::Key;

/// The per-spawn table, keyed by the creature's guid.
pub const MOVEMENT: &str = "creature_movement";

/// …and the per-template one, keyed by the creature's entry.
pub const MOVEMENT_TEMPLATE: &str = "creature_movement_template";

/// Every table this module writes.
pub const TABLES: [&str; 2] = [MOVEMENT, MOVEMENT_TEMPLATE];

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// **The orientation that means "no facing"**, which is not a sane orientation
/// and is therefore usable as one.
///
/// `WaypointManager::Load` tests coordinates with
/// `IsValidMapCoord(x, y, z, orientation == 100.0f ? 0.0f : orientation)`, and
/// `AddNode` writes exactly this for a node a GM adds. A node written with `0`
/// instead is a node with a real facing of due east, which the creature turns
/// to on arrival — visible, wrong, and produced by writing the value that looks
/// like the empty one.
pub const NO_FACING: f32 = 100.0;

/// Which of the two tables a path is in, and what its key column is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Which {
    /// `creature_movement`, keyed `id` — the spawn's guid. One creature.
    Spawn,
    /// `creature_movement_template`, keyed `entry`. Every creature of the kind
    /// that has no path of its own.
    Template,
}

impl Which {
    pub fn table(self) -> &'static str {
        match self {
            Which::Spawn => MOVEMENT,
            Which::Template => MOVEMENT_TEMPLATE,
        }
    }

    /// The key column's name. `creature_movement` calls it `id` even though it
    /// holds a guid, which is vmangos' own spelling and not a choice here.
    pub fn key_column(self) -> &'static str {
        match self {
            Which::Spawn => "id",
            Which::Template => "entry",
        }
    }

    /// What the key names, for a sentence: a spawn's guid or a template's
    /// entry.
    pub fn about(self) -> &'static str {
        match self {
            Which::Spawn => "one spawn, by guid",
            Which::Template => "every spawn of this creature that has no path of its own",
        }
    }

    /// The table a name refers to, or `None`.
    pub fn named(table: &str) -> Option<Which> {
        match table {
            MOVEMENT => Some(Which::Spawn),
            MOVEMENT_TEMPLATE => Some(Which::Template),
            _ => None,
        }
    }
}

/// **One point of a path.**
///
/// The ten columns `WaypointManager::Load` selects, less `point`, which is the
/// node's position in [`Path::nodes`] and is never carried on the node itself —
/// two copies of an ordering is how they come to disagree.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Where the creature faces on arrival, or [`NO_FACING`].
    pub orientation: f32,
    /// How long it waits there, in milliseconds. vmangos calls the column
    /// `waittime` and the field `delay`.
    pub waittime: u32,
    /// How far it may wander from the point while waiting.
    pub wander_distance: f32,
    /// A `creature_movement_scripts` id, or 0.
    ///
    /// **A script id that table does not have costs this node and not the
    /// path**: `Load` logs and `continue`s, so the path comes up one point
    /// short with nothing on screen to say so. See [`Path::script_ids`].
    pub script_id: u32,
    /// A `creature_movement_special` path to run at this node, or 0.
    pub path_id: u32,
}

impl Node {
    /// A node at a place, with everything else at what `AddNode` writes for a
    /// new one.
    pub fn at(x: f32, y: f32, z: f32) -> Node {
        Node {
            x,
            y,
            z,
            orientation: NO_FACING,
            waittime: 0,
            wander_distance: 0.0,
            script_id: 0,
            path_id: 0,
        }
    }

    /// Whether this node turns the creature to face somewhere on arrival.
    ///
    /// **Two conditions, and the second is easy to miss.**
    /// `WaypointMovementGenerator::StartMove` applies the facing only when
    /// `orientation != 100 && delay != 0` — so a node carrying a real
    /// orientation and **no `waittime`** is a facing that never happens. The
    /// creature is moving through that point rather than stopping at it, and
    /// there is nothing to turn.
    ///
    /// That is why this asks both. A form that offered a facing on a node with
    /// no wait would be offering a setting with no effect, which is this
    /// subject's third "applies cleanly and does nothing".
    pub fn faces(&self) -> bool {
        self.orientation != NO_FACING && self.waittime != 0
    }

    /// Whether a facing is *stored* here, whatever the server does with it.
    ///
    /// The measurement behind [`Self::faces`]'s interpretation: a node can hold
    /// an orientation the server will never apply, and a tool that only ever
    /// asked the interpretation could not show a person the value that is
    /// actually in the row.
    pub fn has_orientation(&self) -> bool {
        self.orientation != NO_FACING
    }

    /// The distance to another node, on the ground plane and in three
    /// dimensions. The first is what a person reads off a map; the second is
    /// what the creature walks.
    pub fn flat_distance(&self, other: &Node) -> f32 {
        ((other.x - self.x).powi(2) + (other.y - self.y).powi(2)).sqrt()
    }

    pub fn distance(&self, other: &Node) -> f32 {
        ((other.x - self.x).powi(2) + (other.y - self.y).powi(2) + (other.z - self.z).powi(2)).sqrt()
    }
}

/// **A whole path**: which table it is in, which creature it belongs to, and
/// its nodes in the order they are walked.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub which: Which,
    /// The spawn's guid or the template's entry, by [`Which`].
    pub owner: u64,
    pub nodes: Vec<Node>,
}

impl Path {
    /// An empty path for a creature that has none.
    pub fn empty(which: Which, owner: u64) -> Path {
        Path { which, owner, nodes: Vec::new() }
    }

    /// The key every statement about this path is written under — the key
    /// column alone, since a path is addressed as a whole.
    pub fn key(&self) -> Key {
        Key::one(self.which.key_column(), self.owner)
    }

    /// How long the creature walks to go round once, in yards.
    ///
    /// The last leg back to the first node is included, because the default
    /// waypoint generator repeats: `m_repeating` sends `currPoint` back to
    /// `i_path->begin()` at the end rather than stopping. A path of fewer than
    /// two nodes has no length.
    pub fn loop_length(&self) -> f32 {
        if self.nodes.len() < 2 {
            return 0.0;
        }
        let mut total = 0.0;
        for pair in self.nodes.windows(2) {
            total += pair[0].distance(&pair[1]);
        }
        total + self.nodes[self.nodes.len() - 1].distance(&self.nodes[0])
    }

    /// The total time waited at the nodes, in milliseconds.
    pub fn total_wait(&self) -> u64 {
        self.nodes.iter().map(|node| u64::from(node.waittime)).sum()
    }

    /// Every distinct non-zero `script_id` the path names, so a caller can ask
    /// the database whether `creature_movement_scripts` has them — a missing
    /// one silently costs that node at load.
    pub fn script_ids(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.nodes.iter().map(|node| node.script_id).filter(|id| *id != 0).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// …and the same for `path_id`, against `creature_movement_special`.
    pub fn path_ids(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.nodes.iter().map(|node| node.path_id).filter(|id| *id != 0).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Insert a node at an index, pushing everything after it along.
    ///
    /// An index past the end appends, which is what "add a point" means with
    /// nothing selected.
    pub fn insert(&mut self, at: usize, node: Node) {
        let at = at.min(self.nodes.len());
        self.nodes.insert(at, node);
    }

    /// Remove one, or `None` for an index the path does not have.
    pub fn remove(&mut self, at: usize) -> Option<Node> {
        match at < self.nodes.len() {
            true => Some(self.nodes.remove(at)),
            false => None,
        }
    }

    /// Move a node to another index, carrying it rather than swapping — which
    /// is what dragging a row in a list means, and what reordering a path by
    /// swapping would get wrong for any move of more than one place.
    pub fn move_node(&mut self, from: usize, to: usize) -> bool {
        if from >= self.nodes.len() || to >= self.nodes.len() || from == to {
            return false;
        }
        let node = self.nodes.remove(from);
        self.nodes.insert(to, node);
        true
    }
}

/// `MotionMaster.h`'s `WAYPOINT_MOTION_TYPE`: node to node, waiting at each.
pub const WAYPOINT_MOTION_TYPE: u32 = 2;

/// …and `CYCLIC_MOTION_TYPE`: the same path flown as one spline, without
/// stopping. `CyclicMovementGenerator` reads it through the same
/// `GetDefaultPath`.
pub const CYCLIC_MOTION_TYPE: u32 = 3;

/// **What a creature's `movement_type` does with a path.**
///
/// The question a tool asks before it lets somebody author one: a path on an
/// idle creature is a change that applies, reloads and does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// Node to node, honouring `waittime` and the facing at each.
    Waypoint,
    /// The whole path as one spline, flown round. Needs at least two nodes.
    Cyclic,
    /// Idle, random, or a value no database row should carry — the path is
    /// loaded and never used.
    Never,
}

impl Walk {
    /// What this movement type does.
    pub fn of(movement_type: u32) -> Walk {
        match movement_type {
            WAYPOINT_MOTION_TYPE => Walk::Waypoint,
            CYCLIC_MOTION_TYPE => Walk::Cyclic,
            _ => Walk::Never,
        }
    }

    /// Whether the path is walked at all.
    pub fn walks(self) -> bool {
        self != Walk::Never
    }

    /// The fewest nodes this generator will accept.
    ///
    /// `CyclicMovementGenerator::LoadPath` returns early on
    /// `i_path->size() < 2`, so a one-node cyclic path is a creature that
    /// stands still with a path in the table.
    pub fn needs_nodes(self) -> usize {
        match self {
            Walk::Cyclic => 2,
            Walk::Waypoint => 1,
            Walk::Never => 0,
        }
    }

    /// A sentence for a panel or a report.
    pub fn about(self) -> &'static str {
        match self {
            Walk::Waypoint => "walks it node to node, waiting at each",
            Walk::Cyclic => "flies the whole path as one spline, without stopping",
            Walk::Never => "never walks a path: the rows are loaded and unused",
        }
    }
}

/// The ten columns of both movement tables, in the order
/// `WaypointManager::Load` selects them, less the key.
///
/// Written out rather than derived because an `INSERT` names its columns, and a
/// column added to the table by a later vmangos is a column this writer must
/// leave at its default rather than guess at.
pub const COLUMNS: [&str; 8] =
    ["point", "position_x", "position_y", "position_z", "waittime", "wander_distance", "script_id", "orientation"];

/// **The statements that replace a path.**
///
/// A `DELETE` of everything under the key, then one `INSERT` per node numbered
/// from 1. The delete runs even for an empty path, because emptying a path is
/// how a path is removed and a caller that skipped the delete would leave the
/// old nodes in place while reporting that it had written none.
///
/// `path_id` is written only when it is non-zero. It is the one column of the
/// ten whose default is meaningful — a zero means "no special path" and is what
/// the column already holds — and leaving it out of the common case keeps the
/// statement readable in a file a person reviews.
pub fn statements(path: &Path) -> Vec<String> {
    let table = crate::sql::name(path.which.table());
    let key = crate::sql::name(path.which.key_column());
    let mut out = vec![format!("DELETE FROM {table} WHERE {key} = {};", path.owner)];
    for (index, node) in path.nodes.iter().enumerate() {
        let mut columns: Vec<String> = COLUMNS.iter().map(|name| crate::sql::name(name)).collect();
        let mut values = vec![
            // **1-based**, against a `Vec` that is 0-based. See the module
            // comment: `Load` asserts `point >= 1` and stores at `point - 1`.
            (index + 1).to_string(),
            crate::sql::float(node.x),
            crate::sql::float(node.y),
            crate::sql::float(node.z),
            node.waittime.to_string(),
            crate::sql::float(node.wander_distance),
            node.script_id.to_string(),
            crate::sql::float(node.orientation),
        ];
        if node.path_id != 0 {
            columns.push(crate::sql::name("path_id"));
            values.push(node.path_id.to_string());
        }
        out.push(format!(
            "INSERT INTO {table} ({}, {}) VALUES ({}, {});",
            key,
            columns.join(", "),
            path.owner,
            values.join(", ")
        ));
    }
    out
}

/// **Every column of every path row for one owner**, in point order: what an
/// undo is read from. See [`undo_from_rows`]. A path that was empty is undone
/// by the `DELETE` alone, which is correct — the undo for "a path was created"
/// is "there is no path".
pub fn whole_rows_query(which: Which, owner: u64) -> String {
    format!(
        "SELECT * FROM {} WHERE {} = {owner} ORDER BY `point`",
        crate::sql::name(which.table()),
        crate::sql::name(which.key_column())
    )
}

/// **The statements that put a path back as the database held it**: the
/// `DELETE` of whatever is there, then each row as it was read.
///
/// From the rows themselves, column for column, rather than through [`Node`].
/// A node is the nine columns a path is edited in, as `f32`, renumbered from 1
/// and with a missing value defaulted — the right shape for writing a path and
/// the wrong one for restoring one: a row with a gap in its points, a column
/// this crate does not name, or a coordinate with more precision than an `f32`
/// holds would come back different.
pub fn undo_from_rows(which: Which, owner: u64, rows: &[crate::creature::Row]) -> Vec<String> {
    let mut out = vec![format!(
        "DELETE FROM {} WHERE {} = {owner};",
        crate::sql::name(which.table()),
        crate::sql::name(which.key_column())
    )];
    out.extend(
        rows.iter()
            .filter_map(|row| crate::row::insert_from_row(which.table(), row)),
    );
    out
}

/// **Every path row for one owner**, in point order.
///
/// `ORDER BY point` rather than trusting the table: the primary key is
/// `(id, point)` so MySQL will generally return them in order, but a path read
/// out of order renders as a creature walking a tangle and nothing says why.
pub fn path_query(which: Which, owner: u64) -> String {
    format!(
        "SELECT `point`, `position_x`, `position_y`, `position_z`, `waittime`, \
         `wander_distance`, `script_id`, `orientation`, `path_id` FROM {} \
         WHERE {} = {owner} ORDER BY `point`",
        crate::sql::name(which.table()),
        crate::sql::name(which.key_column())
    )
}

/// **Every path on one map**, joined to the spawns that walk them.
///
/// One statement for the same reason [`crate::creature::spawns_on_map_query`]
/// is one: a map is tens of thousands of spawns and a query per creature on the
/// frame a person pans is what makes a tool feel broken.
///
/// It is the per-guid table only. A template path belongs to an entry rather
/// than to a place, so "every template path on this map" is a different
/// question with a different answer per spawn, and the fallback is resolved
/// when a creature is opened rather than for the whole map at once.
pub fn paths_on_map_query(map: u32) -> String {
    format!(
        "SELECT m.`id`, m.`point`, m.`position_x`, m.`position_y`, m.`position_z`, \
         m.`waittime`, m.`wander_distance`, m.`script_id`, m.`orientation`, m.`path_id` \
         FROM `creature_movement` m \
         JOIN `creature` c ON c.`guid` = m.`id` \
         WHERE c.`map` = {map} ORDER BY m.`id`, m.`point`"
    )
}

/// How many paths and nodes each table holds, for a report that has to say
/// whether there is anything there at all.
pub fn counts_query(which: Which) -> String {
    format!(
        "SELECT COUNT(DISTINCT {key}) AS `paths`, COUNT(*) AS `nodes` FROM {table}",
        key = crate::sql::name(which.key_column()),
        table = crate::sql::name(which.table())
    )
}

/// **The paths whose points are not a dense `1..N`**, which is the query
/// `WaypointManager::Cleanup` runs — so a check can report what the server is
/// about to renumber before it does it.
pub fn out_of_order_query(which: Which) -> String {
    let table = crate::sql::name(which.table());
    let key = crate::sql::name(which.key_column());
    format!(
        "SELECT DISTINCT T.{key} AS `owner` FROM {table} AS T \
         WHERE T.`point` <> (SELECT COUNT(*) FROM {table} \
         WHERE {key} = T.{key} AND `point` <= T.`point`)"
    )
}

/// One row of a movement table as a [`Node`] and its point, or `None` for a row
/// missing a column this reader needs.
///
/// The point comes back as read rather than as `point - 1`: a caller ordering
/// nodes wants to know what the table actually said, and a path with a gap in
/// it is a thing to report rather than to silently close up.
pub fn node_from_row(row: &crate::creature::Row) -> Option<(u32, Node)> {
    use crate::creature::RowValue;
    let point = row.integer("point")? as u32;
    Some((
        point,
        Node {
            x: row.number("position_x")? as f32,
            y: row.number("position_y")? as f32,
            z: row.number("position_z")? as f32,
            orientation: row.number("orientation").unwrap_or(f64::from(NO_FACING)) as f32,
            waittime: row.integer("waittime").unwrap_or(0).max(0) as u32,
            wander_distance: row.number("wander_distance").unwrap_or(0.0) as f32,
            script_id: row.integer("script_id").unwrap_or(0).max(0) as u32,
            path_id: row.integer("path_id").unwrap_or(0).max(0) as u32,
        },
    ))
}

impl Path {
    /// **The nodes as one line of text**, for a caller that has to carry a
    /// whole path where it can only keep a string — the undo stack, which is
    /// `vale-edit`'s and knows nothing about this crate.
    ///
    /// One node per `;`, eight numbers per node. Terse rather than the tabbed
    /// form [`Paths::to_text`] writes, because this is never read by a person:
    /// it is a value in an undo entry, and the file a person reads is the
    /// project's own.
    pub fn to_line(&self) -> String {
        self.nodes
            .iter()
            .map(|node| {
                format!(
                    "{},{},{},{},{},{},{},{}",
                    crate::sql::float(node.x),
                    crate::sql::float(node.y),
                    crate::sql::float(node.z),
                    crate::sql::float(node.orientation),
                    node.waittime,
                    crate::sql::float(node.wander_distance),
                    node.script_id,
                    node.path_id
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// …and back. A node that does not read is dropped rather than failing the
    /// whole path, on [`Paths::from_text`]'s rule.
    ///
    /// **An empty string is an empty path**, not an absent one. The difference
    /// is carried by the `Option` around this at the call site, because a path
    /// claimed-and-empty deletes the rows and a path not claimed leaves them.
    pub fn from_line(which: Which, owner: u64, line: &str) -> Path {
        let mut nodes = Vec::new();
        for part in line.split(';').filter(|part| !part.trim().is_empty()) {
            let fields: Vec<&str> = part.split(',').collect();
            let number = |at: usize| fields.get(at).and_then(|text| text.trim().parse::<f32>().ok());
            let count = |at: usize| fields.get(at).and_then(|text| text.trim().parse::<u32>().ok());
            let (Some(x), Some(y), Some(z)) = (number(0), number(1), number(2)) else {
                continue;
            };
            nodes.push(Node {
                x,
                y,
                z,
                orientation: number(3).unwrap_or(NO_FACING),
                waittime: count(4).unwrap_or(0),
                wander_distance: number(5).unwrap_or(0.0),
                script_id: count(6).unwrap_or(0),
                path_id: count(7).unwrap_or(0),
            });
        }
        Path { which, owner, nodes }
    }
}

/// **Every path a project has edited**, in the order they will be written.
///
/// The counterpart of [`crate::row::Edits`] for the one subject whose edit is a
/// set of rows rather than a set of columns. Keyed by table and owner, so a
/// path edited twice is one entry and the last reading wins.
///
/// **A path that is present but empty is an edit**, and it is the edit that
/// removes a path. That is why this cannot be a map that drops empty values:
/// "this project gives creature 12345 no path" and "this project says nothing
/// about creature 12345" are different claims and only the first emits a
/// `DELETE`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Paths {
    paths: std::collections::BTreeMap<(Which, u64), Vec<Node>>,
}

impl Paths {
    /// Set what this project says a creature's path is.
    ///
    /// An empty `Vec` is a path removed. [`Self::forget`] is the other thing —
    /// the project stops claiming anything about that creature at all.
    pub fn set(&mut self, path: &Path) {
        self.paths.insert((path.which, path.owner), path.nodes.clone());
    }

    /// Stop claiming anything about a creature's path, which is not the same as
    /// giving it none.
    pub fn forget(&mut self, which: Which, owner: u64) {
        self.paths.remove(&(which, owner));
    }

    /// What this project says that creature's path is, if it says anything.
    pub fn get(&self, which: Which, owner: u64) -> Option<Path> {
        let nodes = self.paths.get(&(which, owner))?;
        Some(Path { which, owner, nodes: nodes.clone() })
    }

    /// Whether the project claims that path at all.
    pub fn touches(&self, which: Which, owner: u64) -> bool {
        self.paths.contains_key(&(which, owner))
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Every edited path.
    pub fn iter(&self) -> impl Iterator<Item = Path> + '_ {
        self.paths
            .iter()
            .map(|((which, owner), nodes)| Path { which: *which, owner: *owner, nodes: nodes.clone() })
    }

    /// **The project's reading of a path laid over the database's.**
    ///
    /// What the tool draws, picks and edits — the same job
    /// `creature::Spawn::with_edits` does for a spawn's columns, and for the
    /// same reason: without it a path typed into the editor moves nothing on
    /// screen until it has been applied and read back.
    pub fn over(&self, from_database: &Path) -> Path {
        self.get(from_database.which, from_database.owner).unwrap_or_else(|| from_database.clone())
    }

    /// Every statement this store emits, path by path.
    pub fn statements(&self) -> Vec<String> {
        self.iter().flat_map(|path| statements(&path)).collect()
    }

    /// The file a project keeps this in.
    ///
    /// Tab-separated and two record kinds: a `path` line declaring that this
    /// project owns that creature's path, then a `node` line each. The `path`
    /// line is what carries an emptied path, which has no nodes and is still an
    /// edit.
    pub fn to_text(&self, project: &str) -> String {
        let mut out = format!(
            "# {project} — the waypoint paths this project changes.\n\
             # Two kinds of line. `path` declares that this project owns that\n\
             # creature's path; a `path` with no `node` lines after it is a path\n\
             # removed. `node` lines are in walk order and the point column is\n\
             # 1-based, as the database's is. See crates/mangos/src/path.rs.\n\
             #\n\
             # path\ttable\towner\n\
             # node\ttable\towner\tpoint\tx\ty\tz\torientation\twaittime\twander\tscript_id\tpath_id\n"
        );
        for path in self.iter() {
            out.push_str(&format!("path\t{}\t{}\n", path.which.table(), path.owner));
            for (index, node) in path.nodes.iter().enumerate() {
                out.push_str(&format!(
                    "node\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    path.which.table(),
                    path.owner,
                    index + 1,
                    crate::sql::float(node.x),
                    crate::sql::float(node.y),
                    crate::sql::float(node.z),
                    crate::sql::float(node.orientation),
                    node.waittime,
                    crate::sql::float(node.wander_distance),
                    node.script_id,
                    node.path_id,
                ));
            }
        }
        out
    }

    /// …and read back. A damaged line costs itself, on
    /// [`crate::row::Edits::from_text`]'s rule: this sits beside a project
    /// somebody may have edited by hand and one bad line must not lose a path.
    ///
    /// A `node` line for a path with no `path` line declares the path as well,
    /// so a file with the declaration hand-deleted still reads as the edit it
    /// plainly is.
    pub fn from_text(text: &str) -> Paths {
        let mut out = Paths::default();
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let Some(which) = fields.get(1).and_then(|name| Which::named(name.trim())) else {
                continue;
            };
            let Some(owner) = fields.get(2).and_then(|owner| owner.trim().parse::<u64>().ok()) else {
                continue;
            };
            match fields[0].trim() {
                "path" => {
                    out.paths.entry((which, owner)).or_default();
                }
                "node" => {
                    let number = |at: usize| fields.get(at).and_then(|text| text.trim().parse::<f32>().ok());
                    let count = |at: usize| fields.get(at).and_then(|text| text.trim().parse::<u32>().ok());
                    let (Some(x), Some(y), Some(z)) = (number(4), number(5), number(6)) else {
                        continue;
                    };
                    out.paths.entry((which, owner)).or_default().push(Node {
                        x,
                        y,
                        z,
                        orientation: number(7).unwrap_or(NO_FACING),
                        waittime: count(8).unwrap_or(0),
                        wander_distance: number(9).unwrap_or(0.0),
                        script_id: count(10).unwrap_or(0),
                        path_id: count(11).unwrap_or(0),
                    });
                }
                _ => continue,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> Path {
        Path {
            which: Which::Spawn,
            owner: 12345,
            nodes: vec![Node::at(-9449.0, -50.0, 56.0), Node::at(-9440.0, -50.0, 56.5), Node::at(-9440.0, -60.0, 57.0)],
        }
    }

    /// **A path is put back exactly as it was read**: every column, the point
    /// numbers as they were — a gap included — and full precision.
    #[test]
    fn a_path_is_put_back_column_for_column() {
        let row = |point: &str, x: &str| -> crate::creature::Row {
            [
                ("id", Some("12345")),
                ("point", Some(point)),
                ("position_x", Some(x)),
                ("comment", None),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
            .collect()
        };
        let rows = vec![row("1", "-8913.123456789"), row("4", "12")];
        let sql = undo_from_rows(Which::Spawn, 12345, &rows);
        assert_eq!(sql[0], "DELETE FROM `creature_movement` WHERE `id` = 12345;");
        assert_eq!(
            sql[1],
            "INSERT INTO `creature_movement` (`comment`, `id`, `point`, `position_x`) \
             VALUES (NULL, 12345, 1, '-8913.123456789');"
        );
        assert!(sql[2].contains("VALUES (NULL, 12345, 4, 12)"), "{}", sql[2]);
        assert_eq!(undo_from_rows(Which::Spawn, 12345, &[]).len(), 1);
    }

    /// **The column is 1-based and the `Vec` is 0-based**, which is the one
    /// thing in this module that takes the server down if it is wrong:
    /// `WaypointManager::Load` asserts `point >= 1`.
    #[test]
    fn the_first_node_is_written_as_point_one() {
        let sql = statements(&three());
        assert!(sql[1].contains("(`id`, `point`"), "{}", sql[1]);
        assert!(sql[1].contains("VALUES (12345, 1,"), "{}", sql[1]);
        assert!(sql[2].contains("VALUES (12345, 2,"), "{}", sql[2]);
        assert!(sql[3].contains("VALUES (12345, 3,"), "{}", sql[3]);
        // No node is ever written at point 0, which is the value that takes the
        // server down at its next start.
        assert!(!sql.iter().any(|line| line.contains("VALUES (12345, 0,")));
    }

    /// The points written are dense and in order however the path was built,
    /// which is what stops `Cleanup` renumbering the table under the undo file.
    #[test]
    fn a_path_edited_in_the_middle_still_writes_a_dense_sequence() {
        let mut path = three();
        path.insert(1, Node::at(0.0, 0.0, 0.0));
        path.remove(3);
        path.move_node(0, 2);
        let points: Vec<String> = statements(&path)
            .iter()
            .skip(1)
            .map(|line| line.split("VALUES (12345, ").nth(1).expect("a point").split(',').next().expect("a number").to_string())
            .collect();
        assert_eq!(points, ["1", "2", "3"]);
    }

    /// A path always deletes first, including an empty one — emptying a path is
    /// how a path is removed.
    #[test]
    fn an_emptied_path_is_a_delete_and_nothing_else() {
        let path = Path::empty(Which::Spawn, 77);
        assert_eq!(statements(&path), ["DELETE FROM `creature_movement` WHERE `id` = 77;"]);
    }

    /// The template table is the same statements under another name.
    #[test]
    fn the_template_table_is_keyed_by_entry() {
        let path = Path { which: Which::Template, owner: 68, nodes: vec![Node::at(1.0, 2.0, 3.0)] };
        let sql = statements(&path);
        assert_eq!(sql[0], "DELETE FROM `creature_movement_template` WHERE `entry` = 68;");
        assert!(sql[1].starts_with("INSERT INTO `creature_movement_template` (`entry`, `point`"), "{}", sql[1]);
    }

    /// **A new node is written with the orientation that means "no facing"**,
    /// which is 100 and not 0 — a 0 turns the creature due east on arrival, if
    /// it stops there at all.
    #[test]
    fn a_new_node_has_no_facing_rather_than_a_facing_of_zero() {
        let node = Node::at(1.0, 2.0, 3.0);
        assert!(!node.has_orientation());
        assert!(statements(&Path { which: Which::Spawn, owner: 1, nodes: vec![node] })[1].ends_with("0, 100);"));
    }

    /// **A facing with no wait never happens**, which is the second half of the
    /// server's condition and the half a form would otherwise offer as a
    /// setting that does nothing.
    #[test]
    fn a_facing_needs_a_wait_to_be_applied() {
        let mut node = Node::at(1.0, 2.0, 3.0);
        node.orientation = 1.5;
        // Stored, and never applied: the creature does not stop here.
        assert!(node.has_orientation());
        assert!(!node.faces());
        node.waittime = 2000;
        assert!(node.faces());
        // …and 100 is never applied however long it waits.
        node.orientation = NO_FACING;
        assert!(!node.faces() && !node.has_orientation());
    }

    /// The loop closes, because the default generator repeats.
    #[test]
    fn the_length_includes_the_leg_back_to_the_first_node() {
        let path = Path {
            which: Which::Spawn,
            owner: 1,
            nodes: vec![Node::at(0.0, 0.0, 0.0), Node::at(3.0, 0.0, 0.0), Node::at(3.0, 4.0, 0.0)],
        };
        // 3 + 4 + 5, the last leg being the hypotenuse home.
        assert!((path.loop_length() - 12.0).abs() < 0.001, "{}", path.loop_length());
        assert_eq!(Path::empty(Which::Spawn, 1).loop_length(), 0.0);
        assert_eq!(Path { nodes: vec![Node::at(0.0, 0.0, 0.0)], ..path }.loop_length(), 0.0);
    }

    /// Reordering carries the node rather than swapping two, which is the
    /// difference for any move of more than one place.
    #[test]
    fn moving_a_node_carries_it_past_the_others() {
        let mut path = three();
        let first = path.nodes[0].clone();
        assert!(path.move_node(0, 2));
        assert_eq!(path.nodes[2], first);
        assert_eq!(path.nodes.len(), 3);
        assert!(!path.move_node(0, 0));
        assert!(!path.move_node(0, 9));
    }

    /// A node's script and special-path ids are reported so a caller can check
    /// them: a script id `creature_movement_scripts` does not have costs that
    /// node at load and says nothing on screen.
    #[test]
    fn the_ids_that_have_to_exist_elsewhere_are_listed() {
        let mut path = three();
        path.nodes[0].script_id = 7;
        path.nodes[2].script_id = 7;
        path.nodes[1].path_id = 3;
        assert_eq!(path.script_ids(), [7]);
        assert_eq!(path.path_ids(), [3]);
        assert_eq!(three().script_ids(), Vec::<u32>::new());
    }

    /// A `path_id` of zero is left out of the statement and a real one is
    /// written.
    #[test]
    fn a_special_path_is_named_only_when_there_is_one() {
        let mut path = Path { which: Which::Spawn, owner: 1, nodes: vec![Node::at(0.0, 0.0, 0.0)] };
        assert!(!statements(&path)[1].contains("path_id"));
        path.nodes[0].path_id = 12;
        let sql = statements(&path);
        assert!(sql[1].contains("`path_id`") && sql[1].ends_with(", 12);"), "{}", sql[1]);
    }

    /// **Two movement types walk a path and two do not**, which is the "applies
    /// and does nothing" case for this subject. Type 3 was missing from the
    /// first draft and reported every flying patrol in the game as dead.
    #[test]
    fn both_the_waypoint_and_the_cyclic_types_walk_a_path() {
        assert_eq!(Walk::of(2), Walk::Waypoint);
        assert_eq!(Walk::of(3), Walk::Cyclic);
        assert_eq!(Walk::of(0), Walk::Never);
        assert_eq!(Walk::of(1), Walk::Never);
        assert!(Walk::of(2).walks() && Walk::of(3).walks());
        assert!(!Walk::of(0).walks());
        // A one-node cyclic path is refused by the generator.
        assert_eq!(Walk::of(3).needs_nodes(), 2);
        assert_eq!(Walk::of(2).needs_nodes(), 1);
    }

    /// The reader takes a row as the table states it, gap and all.
    #[test]
    fn a_row_reads_back_as_a_node_at_the_point_the_table_says() {
        let mut row = crate::creature::Row::new();
        for (column, value) in [
            ("point", "4"),
            ("position_x", "-9449.5"),
            ("position_y", "-50"),
            ("position_z", "56.25"),
            ("waittime", "5000"),
            ("wander_distance", "0"),
            ("script_id", "0"),
            ("orientation", "100"),
        ] {
            row.insert(column.to_string(), Some(value.to_string()));
        }
        let (point, node) = node_from_row(&row).expect("a full row");
        assert_eq!(point, 4);
        assert_eq!(node.x, -9449.5);
        assert_eq!(node.waittime, 5000);
        assert!(!node.faces());
        // A row with no position is not a node.
        row.remove("position_x");
        assert!(node_from_row(&row).is_none());
    }

    /// The renumbering query is the one vmangos runs, so a check can report
    /// what is about to be rewritten.
    #[test]
    fn the_out_of_order_query_matches_the_servers_own() {
        let sql = out_of_order_query(Which::Spawn);
        assert!(sql.contains("`point` <> (SELECT COUNT(*)"), "{sql}");
        assert!(sql.contains("`id` = T.`id` AND `point` <= T.`point`"), "{sql}");
    }

    /// **An emptied path is an edit and an absent one is not**, which is the
    /// whole reason [`Paths`] cannot be a map that drops empty values: one
    /// emits a `DELETE` and the other emits nothing.
    #[test]
    fn a_path_with_no_nodes_is_different_from_no_path() {
        let mut paths = Paths::default();
        assert!(!paths.touches(Which::Spawn, 12345));
        paths.set(&Path::empty(Which::Spawn, 12345));
        assert!(paths.touches(Which::Spawn, 12345));
        assert_eq!(paths.len(), 1);
        assert_eq!(paths.statements(), ["DELETE FROM `creature_movement` WHERE `id` = 12345;"]);
        paths.forget(Which::Spawn, 12345);
        assert!(!paths.touches(Which::Spawn, 12345));
        assert!(paths.statements().is_empty());
    }

    /// **The store round-trips through its file**, including the emptied path,
    /// which is what makes an edit survive closing the editor.
    #[test]
    fn the_paths_file_reads_back_what_it_wrote() {
        let mut paths = Paths::default();
        paths.set(&three());
        paths.set(&Path { which: Which::Template, owner: 68, nodes: vec![Node::at(1.5, -2.5, 3.0)] });
        paths.set(&Path::empty(Which::Spawn, 999));
        let back = Paths::from_text(&paths.to_text("a project"));
        assert_eq!(back, paths);
        assert_eq!(back.len(), 3);
        assert_eq!(back.get(Which::Spawn, 12345), Some(three()));
        assert_eq!(back.get(Which::Spawn, 999), Some(Path::empty(Which::Spawn, 999)));
    }

    /// Every field of a node survives the file, not just its position — a
    /// wait or a script id lost in the round trip is a path that reads back
    /// subtly different from the one that was drawn.
    #[test]
    fn a_nodes_other_columns_survive_the_file() {
        let node = Node {
            x: -9449.5,
            y: -50.25,
            z: 56.125,
            orientation: 1.75,
            waittime: 5000,
            wander_distance: 2.5,
            script_id: 7,
            path_id: 12,
        };
        let mut paths = Paths::default();
        paths.set(&Path { which: Which::Spawn, owner: 1, nodes: vec![node.clone()] });
        let back = Paths::from_text(&paths.to_text("p")).get(Which::Spawn, 1).expect("the path");
        assert_eq!(back.nodes, [node]);
    }

    /// A damaged line costs itself and nothing else.
    #[test]
    fn a_bad_line_does_not_lose_the_good_paths() {
        let text = "# a header
                    path	creature_movement	1
                    node	creature_movement	1	1	1	2	3	100	0	0	0	0
                    node	creature_movement	1	2	not-a-number	2	3	100	0	0	0	0
                    node	no_such_table	1	3	9	9	9	100	0	0	0	0
                    this line has no tabs at all
                    node	creature_movement	1	3	4	5	6	100	0	0	0	0
";
        let paths = Paths::from_text(text);
        assert_eq!(paths.len(), 1);
        let path = paths.get(Which::Spawn, 1).expect("the path");
        assert_eq!(path.nodes.len(), 2);
        assert_eq!(path.nodes[1].x, 4.0);
    }

    /// **A `node` line with no `path` line before it still declares its path**,
    /// so a file somebody hand-edited reads as the edit it plainly is.
    #[test]
    fn nodes_alone_declare_their_path() {
        let paths = Paths::from_text("node	creature_movement	5	1	1	2	3	100	0	0	0	0
");
        assert_eq!(paths.get(Which::Spawn, 5).map(|path| path.nodes.len()), Some(1));
    }

    /// The project's reading is laid over the database's, and a path the
    /// project says nothing about reads as the database has it.
    #[test]
    fn the_projects_path_is_what_is_drawn() {
        let from_database = three();
        let mut paths = Paths::default();
        assert_eq!(paths.over(&from_database), from_database);
        let mut edited = from_database.clone();
        edited.insert(0, Node::at(0.0, 0.0, 0.0));
        paths.set(&edited);
        assert_eq!(paths.over(&from_database).nodes.len(), 4);
    }

    /// **A path round-trips through the one line the undo stack carries it
    /// as**, which is what makes a path edit undoable at all: `vale-edit`
    /// holds strings and knows nothing about this crate.
    #[test]
    fn a_path_round_trips_through_its_undo_line() {
        let mut path = three();
        path.nodes[1].waittime = 4000;
        path.nodes[1].orientation = 2.25;
        path.nodes[2].script_id = 9;
        path.nodes[2].path_id = 4;
        let back = Path::from_line(Which::Spawn, 12345, &path.to_line());
        assert_eq!(back, path);
    }

    /// **An empty line is an empty path**, which is the edit that deletes one —
    /// and it is different from claiming no path at all, a difference the
    /// `Option` around this carries.
    #[test]
    fn an_empty_line_is_a_path_with_no_nodes() {
        let empty = Path::empty(Which::Spawn, 7);
        assert_eq!(empty.to_line(), "");
        assert_eq!(Path::from_line(Which::Spawn, 7, ""), empty);
        // …and a damaged node costs itself.
        let back = Path::from_line(Which::Spawn, 7, "1,2,3,100,0,0,0,0;nope;4,5,6,100,0,0,0,0");
        assert_eq!(back.nodes.len(), 2);
        assert_eq!(back.nodes[1].x, 4.0);
    }

    /// Both table names resolve and nothing else does.
    #[test]
    fn a_table_name_resolves_to_one_of_the_two() {
        assert_eq!(table_named("creature_movement"), Some(MOVEMENT));
        assert_eq!(Which::named("creature_movement_template"), Some(Which::Template));
        assert_eq!(table_named("creature"), None);
        assert_eq!(Which::named("gameobject"), None);
    }
}
