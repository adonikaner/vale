//! The flight map: which nodes are on it, where they sit on the parchment, and
//! what it costs to fly to each one.
//!
//! ```text
//! TaxiNodes.dbc       85 rows: an id, a map, a world position, a name and two
//!                     mount creature ids
//! TaxiPath.dbc       287 rows: from, to, cost — the graph's edges
//! TaxiPathNode.dbc  9582 rows: the waypoints of each edge, from which its
//!                     length is computed
//! WorldMapContinent  the box a continent's nodes are projected through
//! ```
//!
//! None of this is sent by the server. `SMSG_SHOWTAXINODES` carries a guid, the
//! node the character is standing at and a 256-bit mask of the nodes it has
//! visited. Everything else the window shows (which nodes are drawn, where,
//! what a flight costs, which hops it takes) is the client's own arithmetic
//! over these tables. It is therefore a game rule and lives in `crates/assets`
//! rather than `crates/client`, where `vale taxi` can check it with no
//! session.
//!
//! ## The client's rules
//!
//! The client keeps one record per drawn node. The six steps that fill and
//! read it are:
//!
//! ```text
//! the SMSG_SHOWTAXINODES handler: prune, project, search
//! is there a direct path? paths[from * 256 + to] != 0
//! how long is it? the sum of its TaxiPathNode segments
//! the relaxation, which is what makes a node *drawn*
//! TaxiNodeCost, TaxiNodeGetType
//! TakeTaxiNode: direct -> ACTIVATETAXI, else EXPRESS
//! ```
//!
//! Three of these rules change what is on screen and cannot be inferred from
//! the tables:
//!
//! * The search minimises distance, not cost and not hop count. The client
//!   compares `dist[from] + length(from, to)` against `dist[to]` and carries the
//!   cost along beside it, so the window does not necessarily offer the
//!   cheapest route.
//! * A node is drawn only if the search reached it. The node's type
//!   starts at 0 and is set only in the relaxation's
//!   first-visit branch, so a known node with no route from the current node
//!   is `"NONE"` and is not drawn.
//! * The faction filter reads the values of the node's two mount columns, not
//!   their order. Booty Bay's Alliance node carries the gryphon in column 14 and
//!   the Horde node carries the wind rider in the same column. See [`serves`].
//!
//! ## `NodeKind::Distant` cannot occur in 5875
//!
//! [`NodeKind::Distant`], the yellow icon `TaxiFrame.lua` has art for, cannot
//! occur in 5875. The client returns it when the
//! node's taxi mask bit is clear, but the build loop clears that bit for every
//! node it rejects and adds a row for
//! every node it keeps, so every row in the list has its bit set. It is
//! implemented anyway because the branch is in the client and a shipped table
//! could reach it.

use std::collections::HashMap;

use crate::tables::dbc::Dbc;
use crate::AssetError;

/// `TaxiMaskSize`: 256 bits, one per node, indexed from node id 1.
pub const TAXI_MASK_WORDS: usize = 8;

/// The largest node id the flight map can reach. The client bounds both node
/// ids at `0x100` before it indexes `paths[from << 8 | to]`, so a node id past
/// 256 has no edges at all. The taxi mask is 256 bits for the same reason, on
/// both sides of the wire.
pub const MAX_NODE_ID: u32 = 0x100;

/// The length the client uses for a path whose waypoints are missing: `1e11`.
/// It is large enough that such an edge never wins a
/// relaxation, and finite so that a sum through one is still a number.
pub const MISSING_PATH_LENGTH: f32 = 1e11;

/// The four mount creature ids the client compares against.
///
/// These are values, not column positions; see [`serves`]. The Horde pair is
/// first because it is the pair the Alliance branch tests for.
const HORDE_MOUNTS: [u32; 2] = [2224, 3574];
const ALLIANCE_MOUNTS: [u32; 2] = [3837, 541];

/// Which side a character is on, as the taxi map decides it: the whole
/// `FactionTemplate.factionGroup` mask, compared against two literals.
///
/// The two literals are 3 and 5, which are
/// `Player | Alliance` and `Player | Horde`, since `FactionGroup.dbc`'s
/// `MaskID`s are Player 0, Alliance 1, Horde 2 (the same join
/// [`crate::tables::charcreate`] makes for the race buttons). The two literals
/// are copied as equality tests rather than bit tests because that is what the
/// branch does: a monster's or a creature's template matches neither and gets
/// no faction filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    Alliance,
    Horde,
}

impl Team {
    pub fn of_group(mask: u32) -> Option<Team> {
        match mask {
            3 => Some(Team::Alliance),
            5 => Some(Team::Horde),
            _ => None,
        }
    }
}

/// The 256-bit mask `SMSG_SHOWTAXINODES` ends with: which nodes this character
/// has visited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaxiMask(pub [u32; TAXI_MASK_WORDS]);

impl TaxiMask {
    /// Node ids are one-based: `PlayerTaxi::IsTaximaskNodeKnown` computes
    /// `(nodeidx - 1) / 32` and the client computes `(id - 1) >> 6`.
    /// Node 0 does not exist and returns false rather than reading
    /// bit -1.
    pub fn knows(&self, node: u32) -> bool {
        let Some(bit) = node.checked_sub(1) else {
            return false;
        };
        let (word, bit) = ((bit / 32) as usize, bit % 32);
        self.0.get(word).is_some_and(|w| w & (1 << bit) != 0)
    }

    /// Every node id in the mask, ascending. This is the order the window's
    /// rows are built in, and therefore the order the interface indexes them by.
    pub fn known(&self) -> impl Iterator<Item = u32> + '_ {
        (1..=MAX_NODE_ID).filter(|id| self.knows(*id))
    }

    pub fn count(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }
}

/// One row of `TaxiNodes.dbc`.
#[derive(Debug, Clone, PartialEq)]
pub struct TaxiNode {
    pub id: u32,
    pub map: u32,
    /// The world position of the node: x, y, z in the server's own axes. The
    /// projection reads it and `Player::ActivateTaxiPathTo`'s range check
    /// measures against it.
    pub pos: [f32; 3],
    pub name: String,
    /// The two `MountCreatureID` columns, in file order. Their order does not
    /// indicate a faction; see [`serves`].
    pub mounts: [u32; 2],
}

/// One row of `TaxiPath.dbc`, with the length of its waypoints folded in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaxiPath {
    pub id: u32,
    pub from: u32,
    pub to: u32,
    /// Copper, before any reputation discount. See [`Board::cost`].
    pub cost: u32,
    /// The sum of the segment lengths of its `TaxiPathNode` rows: the weight
    /// the route search minimises.
    pub length: f32,
    /// The first and last waypoint. The client does not read them; `vale
    /// taxi` does. They are the only data that can be compared between this
    /// table's `from`/`to` columns and `TaxiNodes`' own positions, so a wrong
    /// field index in either table shows up as a path that starts far from the
    /// node it names as its start. Every other check here would still pass.
    pub ends: ([f32; 3], [f32; 3]),
}

/// One row of `TaxiPathNode.dbc`, whole.
///
/// The flight map needs three of its nine fields and computes the length from
/// them. A transport needs three others, and nothing else in the game reads
/// them. `mapid` lets a boat route cross from one map to another, `action_flag`
/// distinguishes a stop from a turn, and `delay` is how long the ship waits at
/// the dock. [`crate::tables::shiptransport`] is the only reader of any of the
/// three.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaxiWaypoint {
    /// `NodeIndex`. File order already matches it in every shipped row, and the
    /// list is sorted by it regardless.
    pub index: u32,
    /// `ContinentID`, field 3. A path may name more than one.
    pub map: u32,
    pub pos: [f32; 3],
    /// Field 7. Bit 0 marks a teleport: the path jumps rather than travels.
    /// The value 2 marks a stop; that is a whole-value test rather than a bit
    /// test because `KeyFrame::IsStopFrame` tests it that way.
    pub action_flag: u32,
    /// Field 8, in seconds. Meaningful only at a stop.
    pub delay: u32,
}

impl TaxiWaypoint {
    /// `KeyFrame::IsStopFrame`: `actionFlag == 2`, compared as a whole value.
    pub fn is_stop(&self) -> bool {
        self.action_flag == 2
    }
}

/// The box a continent's world positions are projected through:
/// `WorldMapContinent.dbc` fields 9..12.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaxiBox {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl TaxiBox {
    /// Whether a node stands inside the box: four comparisons, inclusive at
    /// both ends.
    fn contains(&self, pos: [f32; 3]) -> bool {
        (self.min[0]..=self.max[0]).contains(&pos[0])
            && (self.min[1]..=self.max[1]).contains(&pos[1])
    }

    /// Whether both spans are positive. The client requires this before it
    /// builds anything.
    fn usable(&self) -> bool {
        self.max[0] - self.min[0] > 0.0 && self.max[1] - self.min[1] > 0.0
    }

    /// Where a node lands on the parchment, in `0..1` from the bottom left.
    ///
    /// As the client computes it, including its crossed denominators:
    ///
    /// ```text
    /// x = (maxY - node.y) / (maxX - minX)
    /// y = (node.x - minX) / (maxY - minY)
    /// ```
    ///
    /// Each axis takes its numerator from its own world axis and its
    /// denominator from the other one. Both shipped boxes are square (Azeroth
    /// spans 21,797 on each axis and Kalimdor 24,340), so the crossing has no
    /// visible effect with the shipped tables. It is copied rather than
    /// corrected so that a table with a non-square box draws as the game draws
    /// it.
    pub fn project(&self, pos: [f32; 3]) -> [f32; 2] {
        [
            (self.max[1] - pos[1]) / (self.max[0] - self.min[0]),
            (pos[0] - self.min[0]) / (self.max[1] - self.min[1]),
        ]
    }
}

/// Whether a node's mounts serve this side.
///
/// Both of the client's branches reduce to one rule: skip the node when
/// it names one of the other side's generic mounts and none of this side's.
/// Every other node is served: a node with no mounts, a node with one of each,
/// and a node naming a mount in neither pair (Northshire's 308, Alterac
/// Valley's 15665).
///
/// The test reads values rather than columns because the table does not use a
/// fixed column per faction: node 9 (Booty Bay, Alliance) carries the gryphon
/// 541 in column 14, node 2 (Stormwind) carries it in column 15, and node 18
/// (Booty Bay, Horde) carries the wind rider in column 14. There is no Horde
/// column.
pub fn serves(node: &TaxiNode, team: Option<Team>) -> bool {
    let Some(team) = team else {
        return true;
    };
    let (theirs, ours) = match team {
        Team::Alliance => (HORDE_MOUNTS, ALLIANCE_MOUNTS),
        Team::Horde => (ALLIANCE_MOUNTS, HORDE_MOUNTS),
    };
    let names = |set: [u32; 2]| node.mounts.iter().any(|m| set.contains(m));
    !(names(theirs) && !names(ours))
}

/// The three tables, joined.
#[derive(Debug, Clone, Default)]
pub struct TaxiTables {
    nodes: HashMap<u32, TaxiNode>,
    /// Keyed `(from, to)`, as the client's own `paths[from << 8 | to]` array
    /// is, so that "is there a direct path" is a lookup rather than a scan.
    paths: HashMap<(u32, u32), TaxiPath>,
    /// The waypoints, keyed by path id and in `NodeIndex` order. They are kept
    /// whole rather than reduced to a length because a transport route
    /// consists of these rows and nothing else; see
    /// [`crate::tables::shiptransport`].
    waypoints: HashMap<u32, Vec<TaxiWaypoint>>,
    boxes: HashMap<u32, TaxiBox>,
}

/// The fields of `TaxiNodes.dbc`, 16 wide.
///
/// Public because other crates write these rows and fill the server's
/// `taxi_nodes` table from them; both take the indices from here rather than
/// restating them.
pub mod node_fields {
    pub const ID: usize = 0;
    pub const MAP: usize = 1;
    /// x, y and z follow in order, in the server's axes: north, west, up.
    pub const X: usize = 2;
    pub const Y: usize = 3;
    pub const Z: usize = 4;
    /// The enUS name. Fields 6 to 12 are the seven other locales, empty in an
    /// enUS install.
    pub const NAME: usize = 5;
    /// The locale flags word after the eight names.
    pub const NAME_FLAGS: usize = 13;
    /// The two mount creature ids, fields 14 and 15. The client reads them by
    /// value (see [`super::serves`]); vmangos reads field 14 as the Horde mount
    /// and field 15 as the Alliance mount.
    pub const MOUNT: usize = 14;
    pub const COUNT: usize = 16;
}

/// The fields of `TaxiPath.dbc`, 4 wide.
pub mod path_fields {
    pub const ID: usize = 0;
    pub const FROM: usize = 1;
    pub const TO: usize = 2;
    /// Copper.
    pub const COST: usize = 3;
    pub const COUNT: usize = 4;
}

/// The fields of `TaxiPathNode.dbc`, 9 wide.
pub mod path_node_fields {
    pub const ID: usize = 0;
    pub const PATH: usize = 1;
    /// `NodeIndex`, counted from 0 along the path.
    pub const INDEX: usize = 2;
    /// `ContinentID`. Read only by the transports: a flight does not leave the
    /// map it started on, so the taxi map does not need it.
    pub const MAP: usize = 3;
    /// x, y and z follow in order, in the server's axes.
    pub const X: usize = 4;
    pub const Y: usize = 5;
    pub const Z: usize = 6;
    pub const ACTION_FLAG: usize = 7;
    /// Seconds. 1.12's table is nine fields wide and ends here; later builds
    /// add the two event columns.
    pub const DELAY: usize = 8;
    pub const COUNT: usize = 9;
}

mod continent_fields {
    pub const MAP: usize = 1;
    pub const TAXI_MIN_X: usize = 9;
}

impl TaxiTables {
    /// Returns `None` unless all four tables parse and the nodes, paths and
    /// boxes are non-empty. Unlike most optional tables here, a partial taxi
    /// chain does not degrade in a recognisable way: nodes with no paths draw a
    /// map on which nothing is reachable, and paths with no waypoints give every
    /// edge a length of `1e11` so the search picks an arbitrary one. Both look
    /// like a working window.
    pub fn parse(
        nodes: &[u8],
        paths: &[u8],
        path_nodes: &[u8],
        continents: &[u8],
    ) -> Option<TaxiTables> {
        let out = Self::read(nodes, paths, path_nodes, continents).ok()?;
        (!out.nodes.is_empty() && !out.paths.is_empty() && !out.boxes.is_empty()).then_some(out)
    }

    fn read(
        nodes: &[u8],
        paths: &[u8],
        path_nodes: &[u8],
        continents: &[u8],
    ) -> Result<TaxiTables, AssetError> {
        let nodes = Dbc::parse(nodes)?;
        let paths = Dbc::parse(paths)?;
        let path_nodes = Dbc::parse(path_nodes)?;
        let continents = Dbc::parse(continents)?;

        let mut out = TaxiTables::default();
        for record in 0..nodes.record_count {
            let Some(id) = nodes.u32_at(record, 0) else {
                continue;
            };
            let float = |f: usize| nodes.f32_at(record, f).unwrap_or(0.0);
            out.nodes.insert(
                id,
                TaxiNode {
                    id,
                    map: nodes.u32_at(record, node_fields::MAP).unwrap_or(0),
                    pos: [
                        float(node_fields::X),
                        float(node_fields::Y),
                        float(node_fields::Z),
                    ],
                    name: nodes.string_at(record, node_fields::NAME).unwrap_or_default(),
                    mounts: [
                        nodes.u32_at(record, node_fields::MOUNT).unwrap_or(0),
                        nodes.u32_at(record, node_fields::MOUNT + 1).unwrap_or(0),
                    ],
                },
            );
        }

        // The waypoints, gathered per path in file order. That is `NodeIndex`
        // order in every shipped row; they are sorted below so that a patched
        // table with rows out of order still produces the right length.
        let mut waypoints: HashMap<u32, Vec<TaxiWaypoint>> = HashMap::new();
        for record in 0..path_nodes.record_count {
            let Some(path) = path_nodes.u32_at(record, path_node_fields::PATH) else {
                continue;
            };
            let float = |f: usize| path_nodes.f32_at(record, f).unwrap_or(0.0);
            let word = |f: usize| path_nodes.u32_at(record, f).unwrap_or(0);
            waypoints.entry(path).or_default().push(TaxiWaypoint {
                index: word(path_node_fields::INDEX),
                map: word(path_node_fields::MAP),
                pos: [
                    float(path_node_fields::X),
                    float(path_node_fields::Y),
                    float(path_node_fields::Z),
                ],
                action_flag: word(path_node_fields::ACTION_FLAG),
                delay: word(path_node_fields::DELAY),
            });
        }
        for legs in waypoints.values_mut() {
            legs.sort_by_key(|node| node.index);
        }

        for record in 0..paths.record_count {
            let Some(id) = paths.u32_at(record, 0) else {
                continue;
            };
            let field = |f: usize| paths.u32_at(record, f).unwrap_or(0);
            let (from, to) = (field(path_fields::FROM), field(path_fields::TO));
            out.paths.insert(
                (from, to),
                TaxiPath {
                    id,
                    from,
                    to,
                    cost: field(path_fields::COST),
                    length: waypoints.get(&id).map_or(MISSING_PATH_LENGTH, |legs| {
                        legs.windows(2)
                            .map(|pair| distance(pair[0].pos, pair[1].pos))
                            .sum()
                    }),
                    ends: waypoints
                        .get(&id)
                        .and_then(|legs| Some((legs.first()?.pos, legs.last()?.pos)))
                        .unwrap_or_default(),
                },
            );
        }

        out.waypoints = waypoints;

        for record in 0..continents.record_count {
            let Some(map) = continents.u32_at(record, continent_fields::MAP) else {
                continue;
            };
            let float = |f: usize| continents.f32_at(record, f).unwrap_or(0.0);
            out.boxes.insert(
                map,
                TaxiBox {
                    min: [
                        float(continent_fields::TAXI_MIN_X),
                        float(continent_fields::TAXI_MIN_X + 1),
                    ],
                    max: [
                        float(continent_fields::TAXI_MIN_X + 2),
                        float(continent_fields::TAXI_MIN_X + 3),
                    ],
                },
            );
        }
        Ok(out)
    }

    pub fn node(&self, id: u32) -> Option<&TaxiNode> {
        self.nodes.get(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &TaxiNode> {
        self.nodes.values()
    }

    pub fn paths(&self) -> impl Iterator<Item = &TaxiPath> {
        self.paths.values()
    }

    /// The single path from one node to the other, if there is one.
    /// The id bounds are the client's own; because of them a shipped node past
    /// 256 would be unreachable rather than mis-indexed.
    pub fn direct(&self, from: u32, to: u32) -> Option<&TaxiPath> {
        if from == 0 || from > MAX_NODE_ID || to == 0 || to > MAX_NODE_ID {
            return None;
        }
        self.paths.get(&(from, to))
    }

    pub fn taxi_box(&self, map: u32) -> Option<&TaxiBox> {
        self.boxes.get(&map)
    }

    /// The waypoints of one path, in `NodeIndex` order. They are keyed by the
    /// path's own id rather than by its endpoints because that is how a
    /// transport names its route: `gameobject_template`'s `taxiPathId`, which
    /// arrives in `SMSG_GAMEOBJECT_QUERY_RESPONSE`'s `data[0]` and has no
    /// `from`/`to` to look up. See [`crate::tables::shiptransport`].
    pub fn waypoints(&self, path: u32) -> Option<&[TaxiWaypoint]> {
        self.waypoints.get(&path).map(Vec::as_slice)
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// What `TaxiNodeGetType` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Green: the node the flight master is standing at.
    Current,
    /// White: the search reached it, so it can be flown to.
    Reachable,
    /// Yellow: drawn, but its mask bit is clear. As the module documentation
    /// explains, this cannot happen in 5875; it exists because the client has
    /// the branch.
    Distant,
    /// Not drawn at all.
    None,
}

impl NodeKind {
    /// The literal `TaxiFrame.lua` indexes `TaxiButtonTypes` with. A wrong
    /// spelling here is a `nil` index and a Lua error inside `TAXIMAP_OPENED`.
    pub fn word(self) -> &'static str {
        match self {
            NodeKind::Current => "CURRENT",
            NodeKind::Reachable => "REACHABLE",
            NodeKind::Distant => "DISTANT",
            NodeKind::None => "NONE",
        }
    }
}

/// One node on the open map.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub node: u32,
    pub name: String,
    pub kind: NodeKind,
    /// Where the button goes, in `0..1` from the bottom left of the map art.
    pub at: [f32; 2],
    /// The whole route's cost in copper, or 0 for a node with no route.
    pub cost: u32,
    /// The node ids from the source to this one, inclusive, so a direct flight
    /// is two entries and `GetNumRoutes` is one less than this length.
    pub route: Vec<u32>,
    /// The search's own weight: total path length in yards.
    pub distance: f32,
}

/// The open flight map, laid out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Board {
    /// One per known node on this continent, ascending by node id. That is the
    /// order the client's bit walk produces, and therefore the numbering every
    /// interface read uses.
    pub rows: Vec<Row>,
    /// The node the flight master stands at.
    pub current: u32,
    pub map: u32,
}

impl Board {
    /// Builds the taxi map from the packet's current node and mask and the
    /// tables.
    ///
    /// The build order is the client's, and each step is marked by its own
    /// comment: prune the mask to this continent's box, project the nodes that
    /// remain, then relax from the current node until nothing improves.
    pub fn build(tables: &TaxiTables, current: u32, mask: &TaxiMask, team: Option<Team>) -> Board {
        let Some(here) = tables.node(current) else {
            return Board::default();
        };
        let map = here.map;
        let Some(bounds) = tables.taxi_box(map).filter(|b| b.usable()) else {
            return Board::default();
        };

        let mut rows: Vec<Row> = mask
            .known()
            .filter_map(|id| tables.node(id))
            .filter(|node| node.map == map && bounds.contains(node.pos))
            .map(|node| Row {
                node: node.id,
                name: node.name.clone(),
                kind: NodeKind::None,
                at: bounds.project(node.pos),
                cost: 0,
                route: Vec::new(),
                distance: f32::INFINITY,
            })
            .collect();

        // The source is found by searching the rows, so it must
        // be in the mask. It always is, because `SendLearnNewTaxiNode` sets
        // that bit before the menu is sent.
        //
        // This deliberately departs from the reference in the case that cannot
        // happen. The reference leaves its index at 0 when the search fails and
        // seeds the first row, which draws a map centred on the wrong node.
        // Returning an empty board makes the failure visible instead of
        // plausible.
        let Some(source) = rows.iter().position(|row| row.node == current) else {
            return Board::default();
        };
        rows[source].kind = NodeKind::Current;
        rows[source].distance = 0.0;
        rows[source].route = vec![current];

        relax(&mut rows, tables, source, team);

        // The type of each drawn row, decided as the client decides it and in
        // its order: the current node first, then the mask.
        for row in &mut rows {
            if row.route.is_empty() {
                row.kind = NodeKind::None;
            } else if row.node == current {
                row.kind = NodeKind::Current;
            } else if mask.knows(row.node) {
                row.kind = NodeKind::Reachable;
            } else {
                row.kind = NodeKind::Distant;
            }
        }
        Board {
            rows,
            current,
            map,
        }
    }

    /// One-based, like every index the interface passes.
    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index.checked_sub(1)?)
    }

    /// `GetNumRoutes(i)`: the number of hops, which is one less than the number
    /// of nodes in the route, and 0 for a node with no route.
    pub fn hops(&self, index: usize) -> usize {
        self.row(index)
            .map_or(0, |row| row.route.len().saturating_sub(1))
    }

    /// The endpoints of one hop, in map fractions: `(src, dest)`.
    ///
    /// `hop` is one-based, as `TaxiGetSrcX(index, i)`'s loop passes it. Every
    /// node in a route is a row on the board, so `None` here means only that an
    /// index was out of range.
    pub fn hop(&self, index: usize, hop: usize) -> Option<([f32; 2], [f32; 2])> {
        let route = &self.row(index)?.route;
        let (from, to) = (route.get(hop.checked_sub(1)?)?, route.get(hop)?);
        let at = |id: &u32| Some(self.rows.iter().find(|row| row.node == *id)?.at);
        Some((at(from)?, at(to)?))
    }

    /// `TaxiNodeCost(i)`: the route's total cost, with the reputation discount
    /// applied.
    ///
    /// Known gap: the discount factor here is always 1.0. The client reads the
    /// character's standing with the flight master's faction and multiplies by
    /// `1 - discount`, and this client parses no
    /// reputation. The server charges its own discounted price regardless
    /// (`Player::ActivateTaxiPathTo` applies `GetReputationPriceDiscount`), so
    /// the effect is a tooltip that shows too high a price for an exalted
    /// character, never a wrong deduction.
    pub fn cost(&self, index: usize) -> u32 {
        self.row(index).map_or(0, |row| row.cost)
    }

    /// What `TakeTaxiNode` should send, or the error key to show instead.
    ///
    /// The checks are the client's, in its order: the same-node refusal before
    /// the path lookup, and the direct path before the multi-hop route. The
    /// "already on a flight" check that comes first in the client is left to
    /// the caller, because it concerns the character rather than the board.
    pub fn take(&self, tables: &TaxiTables, index: usize) -> Take {
        let Some(row) = self.row(index) else {
            return Take::Nothing;
        };
        if row.node == self.current {
            return Take::Refused("ERR_TAXISAMENODE");
        }
        if tables.direct(self.current, row.node).is_some() {
            return Take::Direct {
                from: self.current,
                to: row.node,
            };
        }
        if row.route.len() > 2 {
            return Take::Express {
                nodes: row.route.clone(),
                cost: row.cost,
            };
        }
        Take::Refused("ERR_TAXINOSUCHPATH")
    }
}

/// What pressing a node button comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Take {
    /// `CMSG_ACTIVATETAXI`: two node ids.
    Direct { from: u32, to: u32 },
    /// `CMSG_ACTIVATETAXIEXPRESS`: the whole route, source first.
    Express { nodes: Vec<u32>, cost: u32 },
    /// A `GlobalStrings.lua` key for `UIErrorsFrame`.
    Refused(&'static str),
    /// An index that names no row: the reference returns without a sound.
    Nothing,
}

/// The relaxation: the route search that decides which nodes are drawn and
/// which route each one gets.
///
/// The client's search is recursive: it relaxes a node's neighbours as soon as it
/// improves that node, which is Bellman-Ford with an explicit stack rather than
/// Dijkstra. It is written as a worklist here, which gives the same result
/// without the recursion depth. A graph of 85 nodes and 287 edges settles in a
/// few passes.
///
/// The faction filter applies to the destination node and is checked before the
/// weight, as the client does, so a node the filter rejects is not only hidden:
/// no route may pass through it either.
fn relax(rows: &mut [Row], tables: &TaxiTables, source: usize, team: Option<Team>) {
    let mut queue = vec![source];
    while let Some(from) = queue.pop() {
        for to in 0..rows.len() {
            if to == from {
                continue;
            }
            let Some(node) = tables.node(rows[to].node) else {
                continue;
            };
            if !serves(node, team) {
                continue;
            }
            let Some(path) = tables.direct(rows[from].node, rows[to].node) else {
                continue;
            };
            let distance = rows[from].distance + path.length;
            // Strictly shorter only, matching the client's comparison, which
            // fails on an unordered result: an equal-length alternative does not
            // replace the route already found.
            let better = distance < rows[to].distance;
            if !better {
                continue;
            }
            let mut route = rows[from].route.clone();
            route.push(rows[to].node);
            let cost = rows[from].cost + path.cost;
            rows[to].distance = distance;
            rows[to].route = route;
            rows[to].cost = cost;
            queue.push(to);
        }
    }
}

/// The map art for a continent: `Interface\TaxiFrame\TAXIMAP%d.blp`
/// formatted with the current node's map id.
///
/// `SetTaxiMap(texture)` puts it on the frame, so the client composes the path;
/// no file names it.
pub fn map_art(map: u32) -> String {
    format!(r"Interface\TaxiFrame\TAXIMAP{map}.blp")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four tiny tables: three nodes in a line on map 0, with the middle one
    /// reachable both directly and the long way round.
    ///
    /// ```text
    ///   1 --(path 10, cost 100, 4 long)-- 2 --(path 11, cost 100, 4 long)-- 3
    ///   1 --------(path 12, cost 500, 20 long)-------------------------- 3
    /// ```
    fn tables() -> TaxiTables {
        let mut tables = TaxiTables::default();
        for (id, x, y, name, mounts) in [
            (1u32, 0.0f32, 0.0f32, "Start", [541u32, 0u32]),
            (2, 4.0, 0.0, "Middle", [0, 541]),
            (3, 8.0, 0.0, "End", [0, 0]),
            (4, 12.0, 0.0, "Horde Only", [2224, 0]),
            (5, 0.0, 0.0, "Other Map", [0, 0]),
        ] {
            tables.nodes.insert(
                id,
                TaxiNode {
                    id,
                    map: if id == 5 { 1 } else { 0 },
                    pos: [x, y, 0.0],
                    name: name.into(),
                    mounts,
                },
            );
        }
        for (id, from, to, cost, length) in [
            (10u32, 1u32, 2u32, 100u32, 4.0f32),
            (11, 2, 3, 100, 4.0),
            (12, 1, 3, 500, 20.0),
            (13, 3, 4, 100, 4.0),
            (14, 4, 3, 100, 4.0),
        ] {
            tables.paths.insert(
                (from, to),
                TaxiPath {
                    id,
                    from,
                    to,
                    cost,
                    length,
                    ends: ([0.0; 3], [0.0; 3]),
                },
            );
        }
        tables.boxes.insert(
            0,
            TaxiBox {
                min: [0.0, -10.0],
                max: [20.0, 10.0],
            },
        );
        tables
    }

    fn mask(nodes: &[u32]) -> TaxiMask {
        let mut mask = TaxiMask::default();
        for node in nodes {
            let bit = node - 1;
            mask.0[(bit / 32) as usize] |= 1 << (bit % 32);
        }
        mask
    }

    /// The search minimises length and carries the cost along. Node 3 has a
    /// direct 20-yard edge for 500 and a two-hop 8-yard route for 200. The
    /// reference takes the shorter one, so the window offers two hops and 200
    /// copper for a flight that has a one-hop path.
    #[test]
    fn the_route_is_the_shortest_and_not_the_cheapest_or_the_fewest_hops() {
        let board = Board::build(&tables(), 1, &mask(&[1, 2, 3]), Some(Team::Alliance));
        assert_eq!(board.rows.len(), 3, "three known nodes on this continent");
        let end = board.row(3).expect("the third row");
        assert_eq!(end.route, vec![1, 2, 3]);
        assert_eq!(end.cost, 200);
        assert_eq!(board.hops(3), 2);
        // The direct edge is still what pressing the button sends, because
        // `TakeTaxiNode` consults the path table rather than the route.
        assert_eq!(
            board.take(&tables(), 3),
            Take::Direct { from: 1, to: 3 },
            "a direct path outranks the route the lines were drawn for"
        );
    }

    /// The three types, and the one the shipped tables cannot produce.
    #[test]
    fn the_current_node_is_green_and_the_rest_are_white() {
        let board = Board::build(&tables(), 1, &mask(&[1, 2, 3]), Some(Team::Alliance));
        assert_eq!(board.row(1).unwrap().kind, NodeKind::Current);
        assert_eq!(board.row(2).unwrap().kind, NodeKind::Reachable);
        assert_eq!(board.row(1).unwrap().kind.word(), "CURRENT");
        // A node with no edge from the current node is not drawn: the
        // relaxation never sets its type byte.
        let stranded = Board::build(&tables(), 3, &mask(&[3, 5]), Some(Team::Alliance));
        assert_eq!(stranded.rows.len(), 1, "node 5 is on another continent");
    }

    /// The faction filter hides a node and every node reachable only through
    /// it. Node 4 is Horde-only, so it is not drawn for an Alliance character.
    /// Node 3, reachable only through node 4 once 1→2→3 is removed, is not
    /// drawn either.
    #[test]
    fn a_node_of_the_other_side_is_not_drawn_and_is_not_a_stepping_stone() {
        let mut tables = tables();
        // Leave 1→2→4→3 as the only way to node 3.
        tables.paths.remove(&(2, 3));
        tables.paths.remove(&(1, 3));
        tables.paths.insert(
            (2, 4),
            TaxiPath {
                id: 15,
                from: 2,
                to: 4,
                cost: 100,
                length: 4.0,
                ends: ([0.0; 3], [0.0; 3]),
            },
        );
        let alliance = Board::build(&tables, 1, &mask(&[1, 2, 3, 4]), Some(Team::Alliance));
        for row in &alliance.rows {
            let expected = match row.node {
                1 => NodeKind::Current,
                2 => NodeKind::Reachable,
                _ => NodeKind::None,
            };
            assert_eq!(row.kind, expected, "node {}", row.node);
        }
        // With no team (a template neither literal matches) the filter does
        // not apply, as in the client's fall-through.
        let ungated = Board::build(&tables, 1, &mask(&[1, 2, 3, 4]), None);
        assert!(ungated.rows.iter().all(|row| row.kind != NodeKind::None));
    }

    /// Both directions of [`serves`], including the three shapes that are
    /// served by everybody.
    #[test]
    fn a_mount_column_carries_no_faction_and_the_values_do() {
        let node = |mounts: [u32; 2]| TaxiNode {
            id: 1,
            map: 0,
            pos: [0.0; 3],
            name: String::new(),
            mounts,
        };
        // Booty Bay's two nodes: the Alliance one carries 541 in column 14.
        assert!(serves(&node([541, 0]), Some(Team::Alliance)));
        assert!(!serves(&node([541, 0]), Some(Team::Horde)));
        assert!(serves(&node([2224, 0]), Some(Team::Horde)));
        assert!(!serves(&node([2224, 0]), Some(Team::Alliance)));
        // Neutral, unmounted, and a mount in neither pair (Northshire's 308).
        for mounts in [[0, 0], [308, 0], [2224, 541]] {
            assert!(serves(&node(mounts), Some(Team::Alliance)));
            assert!(serves(&node(mounts), Some(Team::Horde)));
        }
    }

    /// The mask is one-based and node 0 does not exist.
    #[test]
    fn the_mask_is_one_based() {
        let mask = mask(&[1, 32, 33, 256]);
        assert!(mask.knows(1) && mask.knows(32) && mask.knows(33) && mask.knows(256));
        assert!(!mask.knows(0), "there is no node 0");
        assert!(!mask.knows(2));
        assert_eq!(mask.known().collect::<Vec<_>>(), vec![1, 32, 33, 256]);
        assert_eq!(mask.count(), 4);
    }

    /// The projection against the shipped Azeroth box: the corner cases and
    /// the crossed denominator, which has no visible effect only because the
    /// box is square.
    #[test]
    fn a_node_projects_into_the_unit_square() {
        let bounds = TaxiBox {
            min: [-15980.0, -11880.0],
            max: [5817.0, 9917.0],
        };
        assert!(bounds.usable());
        assert_eq!(bounds.project([bounds.min[0], bounds.max[1], 0.0]), [0.0, 0.0]);
        assert_eq!(bounds.project([bounds.max[0], bounds.min[1], 0.0]), [1.0, 1.0]);
        // Stormwind, near enough: x rises as world y falls.
        let [x, y] = bounds.project([-8833.0, 628.0, 94.0]);
        assert!((0.4..0.5).contains(&x), "x = {x}");
        assert!((0.3..0.4).contains(&y), "y = {y}");
        assert!(!TaxiBox {
            min: [0.0, 0.0],
            max: [0.0, 1.0]
        }
        .usable());
    }

    /// A press with nothing under it, and the two refusals.
    #[test]
    fn the_two_local_refusals_are_the_keys_the_client_names() {
        let tables = tables();
        let board = Board::build(&tables, 1, &mask(&[1, 2, 3]), Some(Team::Alliance));
        assert_eq!(board.take(&tables, 1), Take::Refused("ERR_TAXISAMENODE"));
        assert_eq!(board.take(&tables, 99), Take::Nothing);
        // A route of more than one hop with no direct edge is sent as the
        // express.
        let mut sparse = tables.clone();
        sparse.paths.remove(&(1, 3));
        let board = Board::build(&sparse, 1, &mask(&[1, 2, 3]), Some(Team::Alliance));
        assert_eq!(
            board.take(&sparse, 3),
            Take::Express {
                nodes: vec![1, 2, 3],
                cost: 200
            }
        );
    }

    /// The hop endpoints are the ones the lines are drawn between, in order.
    #[test]
    fn the_hops_walk_the_route_in_order() {
        let mut tables = tables();
        tables.paths.remove(&(1, 3));
        let board = Board::build(&tables, 1, &mask(&[1, 2, 3]), Some(Team::Alliance));
        assert_eq!(board.hops(3), 2);
        let (src, dest) = board.hop(3, 1).expect("the first hop");
        assert_eq!(src, board.row(1).unwrap().at);
        assert_eq!(dest, board.row(2).unwrap().at);
        let (src, dest) = board.hop(3, 2).expect("the second hop");
        assert_eq!(src, board.row(2).unwrap().at);
        assert_eq!(dest, board.row(3).unwrap().at);
        assert_eq!(board.hop(3, 3), None, "there is no third hop");
        assert_eq!(board.hop(3, 0), None, "the hop index is one-based");
    }

    /// The art path is the one the client formats.
    #[test]
    fn the_map_art_is_named_after_the_continent() {
        assert_eq!(map_art(0), r"Interface\TaxiFrame\TAXIMAP0.blp");
        assert_eq!(map_art(1), r"Interface\TaxiFrame\TAXIMAP1.blp");
    }
}
