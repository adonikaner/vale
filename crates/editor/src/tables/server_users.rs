//! The server templates that name a DBC table's row, for the row's Used
//! by: the creatures drawn as a `CreatureDisplayInfo` row, the items that
//! cast a `Spell`. The reverse index only knows the client's own tables, and
//! nothing in a display row says which creature wears it.
//!
//! Which template columns are searched is `vale_mangos::dbc_users`'. One row
//! is read at a time, when a form or the inspector asks for it, and every
//! answer is forgotten when the database is written. The project's edits are
//! laid over the answer each time it is drawn, so a `display_id1` typed into
//! a creature's form moves the creature to the display it names on the next
//! frame, saved or not.

use bevy::prelude::*;
use vale_mangos::dbc_users::{searches, Search};
use vale_mangos::row::{Edits, Life};

use super::browser::Browser;
use crate::server::fresh::{claimed, FreshReads};
use crate::session::EditSession;

/// One template that names the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Named {
    /// The template table: `creature_template`, `gameobject_template`,
    /// `item_template` or `quest_template`.
    pub(crate) table: &'static str,
    pub(crate) entry: u32,
    pub(crate) name: String,
    /// The columns that name the row.
    pub(crate) columns: Vec<&'static str>,
}

/// One template the database answered, with its value in every searched
/// column, so the project's edits can be laid over it.
#[derive(Debug, Clone)]
pub(crate) struct Found {
    table: &'static str,
    entry: u32,
    name: String,
    values: Vec<(&'static str, u32)>,
}

/// The answers by DBC table and id, and the row last asked for.
#[derive(Default)]
pub(crate) struct ServerUsers {
    reads: FreshReads<(String, u32), Vec<Found>>,
    wanted: Option<(String, u32)>,
}

impl ServerUsers {
    /// The templates that name row `id` of `table`, the database's with the
    /// project's edits over them; `None` while it is read, and when there is
    /// no database to read. Asks for the read.
    pub(crate) fn of(&mut self, table: &str, id: u32, edits: &Edits) -> Option<Vec<Named>> {
        let key = (table.to_string(), id);
        let searched = searches(table);
        if searched.is_empty() {
            return Some(Vec::new());
        }
        match self.reads.get(&key) {
            Some(found) => Some(over(found, &searched, edits, id)),
            None => {
                self.wanted = Some(key);
                None
            }
        }
    }
}

/// The database's answer with the project's claims over it. A template the
/// project points elsewhere is left out; one it points here, created or
/// edited, is added under the name the project gives it.
fn over(found: &[Found], searched: &[Search], edits: &Edits, id: u32) -> Vec<Named> {
    let now = |table: &str, entry: u32, column: &str| {
        claimed(edits, table, entry, column).and_then(|value| value.trim().parse::<u32>().ok())
    };
    let name_of = |table: &'static str| searched.iter().find(|search| search.table == table).map(|search| search.name);
    let mut out: Vec<Named> = found
        .iter()
        .filter_map(|found| {
            let columns: Vec<&'static str> = found
                .values
                .iter()
                .filter(|(column, value)| now(found.table, found.entry, column).unwrap_or(*value) == id)
                .map(|(column, _)| *column)
                .collect();
            if columns.is_empty() {
                return None;
            }
            let name = name_of(found.table)
                .and_then(|column| claimed(edits, found.table, found.entry, column))
                .unwrap_or_else(|| found.name.clone());
            Some(Named { table: found.table, entry: found.entry, name, columns })
        })
        .collect();
    for (table, key, row) in edits.rows() {
        if row.life == Life::Delete {
            continue;
        }
        let Some(search) = searched.iter().find(|search| search.table == table) else {
            continue;
        };
        let Some(entry) = key.first().map(|entry| entry as u32) else {
            continue;
        };
        if found.iter().any(|had| had.table == search.table && had.entry == entry)
            || out.iter().any(|had| had.table == search.table && had.entry == entry)
        {
            continue;
        }
        let columns: Vec<&'static str> = search
            .columns
            .iter()
            .filter(|column| now(search.table, entry, column) == Some(id))
            .copied()
            .collect();
        if columns.is_empty() {
            continue;
        }
        let name = claimed(edits, search.table, entry, search.name).unwrap_or_else(|| format!("entry {entry}"));
        out.push(Named { table: search.table, entry, name, columns });
    }
    out.sort_by_key(|named| (named.table, named.entry));
    out
}

/// Read the row a form or the inspector last asked for, one at a time.
pub(super) fn read_server_users(
    browser: Option<ResMut<Browser>>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let (Some(mut browser), Some(session)) = (browser, session) else { return };
    let users = &mut browser.server_users;
    let writes = session.database_writes;
    users.reads.renew(writes);
    if let Some(Err(e)) = users.reads.poll() {
        warn!("server users: {e}");
    }
    let Some((table, id)) = users.wanted.take() else { return };
    let patch = crate::server::settings::server_patch(&settings);
    let searched = searches(&table);
    users.reads.want(writes, (table, id), || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            use vale_mangos::schema::RowValue;
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let mut found = Vec::new();
            for search in &searched {
                for row in db.rows(&search.query(id, patch))? {
                    let Some(entry) = row.integer("entry") else { continue };
                    let values = search
                        .columns
                        .iter()
                        .map(|column| (*column, row.integer(column).unwrap_or(0).max(0) as u32))
                        .collect();
                    found.push(Found {
                        table: search.table,
                        entry: entry as u32,
                        name: row.text(search.name).unwrap_or("").to_string(),
                        values,
                    });
                }
            }
            Ok(found)
        }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::creature::{template_key, TEMPLATE};

    fn creature(entry: u32, name: &str, display: u32) -> Found {
        Found {
            table: "creature_template",
            entry,
            name: name.to_string(),
            values: vec![("display_id1", display), ("display_id2", 0), ("display_id3", 0), ("display_id4", 0), ("mount_display_id", 0)],
        }
    }

    /// The database's creatures are listed by the column that names the
    /// display; one the project moves to another display is left out, and one
    /// it moves here is added.
    #[test]
    fn the_projects_edits_are_laid_over_the_database() {
        let searched = searches("CreatureDisplayInfo");
        let found = vec![creature(412, "Stitches", 1693), creature(1805, "Flesh Golem", 1693)];
        let mut edits = Edits::default();
        edits.set(TEMPLATE, &template_key(1805, 0), "display_id1", Some("11".into()));
        edits.set(TEMPLATE, &template_key(2000, 0), "display_id3", Some("1693".into()));
        edits.set(TEMPLATE, &template_key(2000, 0), "name", Some("'Patchwork'".into()));
        let named = over(&found, &searched, &edits, 1693);
        let shown: Vec<(u32, &str, &[&str])> =
            named.iter().map(|named| (named.entry, named.name.as_str(), named.columns.as_slice())).collect();
        assert_eq!(shown, [(412, "Stitches", ["display_id1"].as_slice()), (2000, "Patchwork", ["display_id3"].as_slice())]);
    }
}
