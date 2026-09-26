//! `taxi_nodes`: the server's copy of `TaxiNodes.dbc`, and how an edit to a
//! node reaches it.
//!
//! ## What the server reads, and from where
//!
//! vmangos reads the three taxi tables from two places:
//!
//! ```text
//! TaxiNodes     the SQL table taxi_nodes, one row per (id, build);
//!               ObjectMgr::LoadTaxiNodes (ObjectMgr.cpp:9277) takes, for each
//!               id, the row with the highest build at or below 5875
//! TaxiPath      DataDir\5875\dbc\TaxiPath.dbc      (DBCStores.cpp:340)
//! TaxiPathNode  DataDir\5875\dbc\TaxiPathNode.dbc  (DBCStores.cpp:347)
//! ```
//!
//! So an edited node reaches the server as a row of `taxi_nodes`, written here,
//! and an edited path reaches it as the two files, copied by a publish. Neither
//! is re-read while the server runs: there is no `.reload` for any of the
//! three, and the taxi network mask is built once at startup from all of them
//! (`DBCStores.cpp:366`). A change needs a restart.
//!
//! ## The row
//!
//! ```text
//! id                  smallint unsigned   TaxiNodes field 0
//! build               smallint unsigned   5875 for an edit; see BUILD
//! map_id              mediumint unsigned  field 1
//! x, y, z             float               fields 2, 3, 4
//! name                varchar(256)        field 5, the enUS name
//! mount_creature_id1  smallint unsigned   field 14; read as the Horde mount
//! mount_creature_id2  smallint unsigned   field 15; read as the Alliance mount
//! ```
//!
//! `ObjectMgr::GetNearestTaxiNode` (`ObjectMgr.cpp:7352`) skips a node whose
//! mount column for the player's side is 0, and `GetTaxiMountDisplayId`
//! (`ObjectMgr.cpp:7415`) reads the same column: `MountCreatureID[1]` for the
//! Alliance and `[0]` for the Horde. The client decides the side from the
//! values instead (`vale_assets::tables::taxi::serves`). `vale taxi`
//! counts where the two readings differ: 15 of the 85 shipped nodes. Eleven
//! name no mount at all, which the client draws for both sides and the server
//! offers to neither. The other four carry one mount in field 14: nodes 1 and 3
//! carry 308, which is in neither of the client's pairs, and nodes 9 and 15
//! carry the Alliance gryphon 541, which the server reads as a Horde mount.
//! Node 19 is the Booty Bay node with 541 in the Alliance column.
//!
//! ## The statements, on `spell_template`'s terms
//!
//! An edit is written as a dev row at build 5875, the same arrangement
//! `crate::spell` describes: copy the row the server uses to a 5875 row if
//! there is none, insert a whole row for an id the table has never had, then
//! update the changed columns. Removing the dev row makes the server fall back
//! to the highest shipped build, so the undo is a `DELETE` for a dev row this
//! project created and an `UPDATE` for one that was already there. On the
//! reference install the table has 105 rows for 85 ids, and four of the ids
//! (84 to 87) already have a 5875 row.

use crate::row::{self, Assignment, Key};
use vale_edit::dbc::taxi::Node;
use std::collections::HashMap;

/// The table.
pub const TABLE: &str = "taxi_nodes";

/// The build an edit is written at. `LoadTaxiNodes` takes the highest row at
/// or below `SUPPORTED_CLIENT_BUILD`, which is 5875, so a row here wins.
pub const BUILD: u32 = 5875;

const ID: &str = "id";
const BUILD_COLUMN: &str = "build";

/// The largest node id either side can use: the taxi mask is 256 bits on both.
/// The column itself is a `smallint unsigned` and would hold more.
pub const MAX_ID: u32 = vale_edit::dbc::taxi::MAX_NODE_ID;

/// Whether the server's table can hold this node, and the flight map reach it.
pub fn fits(id: u32) -> bool {
    (1..=MAX_ID).contains(&id)
}

/// The columns a node feeds, in the order `SELECT *` returns them, with the
/// value each takes from a node.
fn values(node: &Node) -> [(&'static str, String); 7] {
    [
        ("map_id", node.map.to_string()),
        ("x", crate::sql::float(node.at[0])),
        ("y", crate::sql::float(node.at[1])),
        ("z", crate::sql::float(node.at[2])),
        ("name", crate::sql::text(&node.name)),
        ("mount_creature_id1", node.mounts[0].to_string()),
        ("mount_creature_id2", node.mounts[1].to_string()),
    ]
}

/// Every column of `taxi_nodes`, in table order.
pub const COLUMNS: [&str; 9] = [
    ID,
    BUILD_COLUMN,
    "map_id",
    "x",
    "y",
    "z",
    "name",
    "mount_creature_id1",
    "mount_creature_id2",
];

/// The dev row's key.
pub fn key(id: u32) -> Key {
    Key::two((ID, u64::from(id)), (BUILD_COLUMN, u64::from(BUILD)))
}

/// What changed between the node the archives ship and the node this project
/// edited: one assignment per column whose value differs. A node the shipped
/// file does not have is new, and every column is a change.
pub fn changes(shipped: Option<&Node>, edited: &Node) -> Vec<Assignment> {
    let before = shipped.map(values);
    values(edited)
        .into_iter()
        .enumerate()
        .filter(|(n, (_, after))| before.as_ref().map(|b| &b[*n].1) != Some(after))
        .map(|(_, (column, value))| Assignment { column, value })
        .collect()
}

/// Copy the row the server is using for this node to a dev row, if it has no
/// dev row already. The sub-select is `LoadTaxiNodes`' own.
pub fn copy_forward(id: u32) -> String {
    let selected: Vec<String> = COLUMNS
        .iter()
        .map(|&column| match column {
            BUILD_COLUMN => BUILD.to_string(),
            name => crate::sql::name(name),
        })
        .collect();
    format!(
        "INSERT IGNORE INTO {table} SELECT {selected} FROM {table} t1 \
         WHERE t1.{id_col} = {id} \
         AND t1.{build_col} = (SELECT MAX(t2.{build_col}) FROM {table} t2 \
         WHERE t2.{id_col} = {id} AND t2.{build_col} <= {build});",
        table = crate::sql::name(TABLE),
        selected = selected.join(", "),
        id_col = crate::sql::name(ID),
        build_col = crate::sql::name(BUILD_COLUMN),
        build = BUILD,
    )
}

/// A whole dev row from the node, for an id the table has never had.
/// `INSERT IGNORE`, so it does nothing when [`copy_forward`] made the row.
pub fn insert_new(node: &Node) -> String {
    let mut names = vec![crate::sql::name(ID), crate::sql::name(BUILD_COLUMN)];
    let mut literals = vec![node.id.to_string(), BUILD.to_string()];
    for (column, value) in values(node) {
        names.push(crate::sql::name(column));
        literals.push(value);
    }
    format!(
        "INSERT IGNORE INTO {} ({}) VALUES ({});",
        crate::sql::name(TABLE),
        names.join(", "),
        literals.join(", ")
    )
}

/// Everything a save emits for one edited node, in order. Empty when nothing
/// changed or the id cannot be stored.
pub fn statements(shipped: Option<&Node>, edited: &Node) -> Vec<String> {
    if !fits(edited.id) {
        return Vec::new();
    }
    let changes = changes(shipped, edited);
    let Some(update) = row::update(TABLE, &key(edited.id), &changes) else {
        return Vec::new();
    };
    vec![copy_forward(edited.id), insert_new(edited), update]
}

/// The dev row of one node, for [`undo`] to read before an apply.
pub fn dev_row_query(id: u32) -> String {
    format!(
        "SELECT * FROM {} WHERE {};",
        crate::sql::name(TABLE),
        key(id).where_clause()
    )
}

/// The statement that puts one node's dev row back to what it holds now:
/// `DELETE` when there is no dev row yet, so the server falls back to the
/// shipped build, and an `UPDATE` of the changed columns when there is one.
pub fn undo(
    id: u32,
    changes: &[Assignment],
    now: Option<&HashMap<String, Option<String>>>,
) -> Option<String> {
    if changes.is_empty() || !fits(id) {
        return None;
    }
    match now {
        None => Some(row::delete(TABLE, &key(id))),
        Some(now) => row::undo(TABLE, &key(id), changes, Some(now)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32) -> Node {
        Node {
            id,
            record: 0,
            map: 0,
            at: [-8840.56, 489.7, 109.61],
            name: "Stormwind, Elwynn".into(),
            mounts: [0, 541],
        }
    }

    #[test]
    fn the_node_limit_is_the_flight_maps() {
        assert!(fits(1) && fits(256));
        assert!(!fits(0) && !fits(257));
    }

    #[test]
    fn an_unchanged_node_writes_nothing() {
        let shipped = node(2);
        assert!(changes(Some(&shipped), &shipped).is_empty());
        assert!(statements(Some(&shipped), &shipped).is_empty());
    }

    #[test]
    fn a_moved_node_changes_only_its_position() {
        let shipped = node(2);
        let mut edited = shipped.clone();
        edited.at[0] += 10.0;
        let changed: Vec<&str> = changes(Some(&shipped), &edited)
            .iter()
            .map(|change| change.column)
            .collect();
        assert_eq!(changed, ["x"]);
        let sql = statements(Some(&shipped), &edited);
        assert_eq!(sql.len(), 3);
        assert!(sql[0].starts_with("INSERT IGNORE INTO `taxi_nodes` SELECT `id`, 5875, `map_id`"));
        assert!(sql[1].starts_with("INSERT IGNORE INTO `taxi_nodes` (`id`, `build`, `map_id`"));
        assert!(sql[2].starts_with("UPDATE `taxi_nodes` SET `x` = "));
        assert!(sql[2].ends_with("WHERE `id` = 2 AND `build` = 5875;"));
    }

    #[test]
    fn a_new_node_names_every_column() {
        let edited = node(90);
        assert_eq!(changes(None, &edited).len(), 7);
        let insert = insert_new(&edited);
        for column in COLUMNS {
            assert!(insert.contains(&format!("`{column}`")), "{column} missing: {insert}");
        }
        assert!(insert.contains("'Stormwind, Elwynn'"));
    }

    #[test]
    fn a_node_past_the_mask_is_not_written() {
        assert!(statements(None, &node(300)).is_empty());
    }

    #[test]
    fn the_undo_deletes_a_dev_row_this_project_made_and_restores_one_it_did_not() {
        let changes = changes(Some(&node(2)), &{
            let mut n = node(2);
            n.mounts = [2224, 541];
            n
        });
        assert_eq!(
            undo(2, &changes, None).unwrap(),
            "DELETE FROM `taxi_nodes` WHERE `id` = 2 AND `build` = 5875;"
        );
        let now = HashMap::from([("mount_creature_id1".to_string(), Some("0".to_string()))]);
        assert_eq!(
            undo(2, &changes, Some(&now)).unwrap(),
            "UPDATE `taxi_nodes` SET `mount_creature_id1` = 0 WHERE `id` = 2 AND `build` = 5875;"
        );
    }
}
