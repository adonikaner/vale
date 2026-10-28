//! The server templates that name a DBC row: the creatures drawn as a
//! `CreatureDisplayInfo` row, the items that cast a `Spell`, the game objects
//! a `Lock` opens. The DBC row has no column that says so; the templates do,
//! and [`Kind::Ref`] marks which of their columns those are. The search is
//! built from those marks, so a column marked there is searched here too.
//!
//! Each template is read at the row the server loads: the highest `patch` at
//! or below `WowPatch`, as [`crate::creature::winning_template_query`] picks
//! one. A template that named the row at an older patch and no longer does is
//! not a user.

use crate::row::Table;
use crate::schema::Kind;

/// The templates searched, each with the column that names one of its rows.
/// These four are the ones the editor opens by entry.
const TEMPLATES: [(&Table, &str); 4] = [
    (&crate::creature::TEMPLATE_TABLE, "name"),
    (&crate::gameobject::TEMPLATE_TABLE, "name"),
    (&crate::item::TEMPLATE_TABLE, "name"),
    (&crate::quest::TEMPLATE_TABLE, "Title"),
];

/// One template table's columns that name rows of a DBC table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    /// The template table, as vmangos spells it.
    pub table: &'static str,
    /// The column that names one of its rows.
    pub name: &'static str,
    /// The columns that hold an id of the DBC table.
    pub columns: Vec<&'static str>,
}

/// The template columns that name rows of `dbc` (a DBC's bare name, such as
/// `CreatureDisplayInfo`), one [`Search`] per template table that has any.
pub fn searches(dbc: &str) -> Vec<Search> {
    TEMPLATES
        .iter()
        .filter_map(|(table, name)| {
            let columns: Vec<&'static str> = table
                .columns
                .iter()
                .filter(|column| matches!(column.kind, Kind::Ref(target) if target == dbc))
                .map(|column| column.name)
                .collect();
            (!columns.is_empty()).then_some(Search { table: table.name, name, columns })
        })
        .collect()
}

impl Search {
    /// The templates that name `id` in any of [`Self::columns`], at
    /// `wow_patch`: `entry`, the name column and every searched column.
    pub fn query(&self, id: u32, wow_patch: u32) -> String {
        let table = crate::sql::name(self.table);
        let columns: Vec<String> = self.columns.iter().map(|column| format!("t.{}", crate::sql::name(column))).collect();
        let test = columns.iter().map(|column| format!("{column} = {id}")).collect::<Vec<_>>().join(" OR ");
        format!(
            "SELECT t.`entry`, t.{name}, {columns} FROM {table} t \
             WHERE ({test}) AND t.`patch` = \
             (SELECT MAX(w.`patch`) FROM {table} w WHERE w.`entry` = t.`entry` AND w.`patch` <= {wow_patch}) \
             ORDER BY t.`entry`",
            name = crate::sql::name(self.name),
            columns = columns.join(", "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three display tables are each named by their own template's
    /// columns, and a creature by its four models and its mount.
    #[test]
    fn a_display_is_named_by_its_templates_columns() {
        let creature = searches("CreatureDisplayInfo");
        assert_eq!(creature.len(), 1);
        assert_eq!(creature[0].table, "creature_template");
        assert_eq!(
            creature[0].columns,
            ["display_id1", "display_id2", "display_id3", "display_id4", "mount_display_id"]
        );
        let item = searches("ItemDisplayInfo");
        assert_eq!((item[0].table, item[0].columns.as_slice()), ("item_template", ["display_id"].as_slice()));
        let object = searches("GameObjectDisplayInfo");
        assert_eq!((object[0].table, object[0].columns.as_slice()), ("gameobject_template", ["displayId"].as_slice()));
        assert!(searches("NoSuchTable").is_empty());
    }

    /// The query tests every column and keeps only the row the server loads.
    #[test]
    fn the_query_reads_the_winning_row() {
        let sql = searches("ItemDisplayInfo")[0].query(1234, 7);
        assert!(sql.contains("t.`display_id` = 1234"), "{sql}");
        assert!(sql.contains("w.`patch` <= 7"), "{sql}");
        assert!(sql.starts_with("SELECT t.`entry`, t.`name`, t.`display_id` FROM `item_template` t"), "{sql}");
    }
}
