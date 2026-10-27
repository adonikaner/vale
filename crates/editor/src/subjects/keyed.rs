//! A keyed server row with the project's edits over the database's, for the
//! subjects that read a table whole and draw a few rows of it at a time:
//! formations, pools and game events.
//!
//! The database's rows are read once and indexed by the subject that owns
//! them. A view finds its candidates through that index and through every
//! key the project's store holds for the table, lays the store over each with
//! [`overlay`], and keeps the ones that belong to it. A row the project
//! creates is in no index, and a row whose edit moves it to another leader,
//! pool or event is found through the store, so both are seen on the frame
//! they are made.

use crate::server::store::Gesture;
use crate::session::EditSession;
use vale_mangos::row::{Assignment, Edits, Key, Life};
use vale_mangos::schema::Row;

/// One row as a view draws it: the database's reading with the project's
/// edits over it, what the project says is to become of it, and the row as
/// the database has it, for an edit typed back to that value to be cleared.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Shown<T> {
    pub(crate) row: T,
    pub(crate) life: Life,
    /// `None` for a row this project creates.
    pub(crate) in_database: Option<T>,
}

impl<T> Shown<T> {
    /// Whether the project keeps the row: it is not marked for removal.
    pub(crate) fn kept(&self) -> bool {
        self.life != Life::Delete
    }
}

/// The row at `key` as the project leaves it, read back by `read`: `held`,
/// the database's, with the store over it, or the project's creation. `None`
/// when neither has the row, or `read` cannot read it.
pub(crate) fn overlay<T: Clone>(
    edits: &Edits,
    table: &str,
    key: &Key,
    held: Option<&T>,
    assignments: impl Fn(&T) -> Vec<Assignment>,
    read: impl Fn(&Row) -> Option<T>,
) -> Option<Shown<T>> {
    let columns = held.map(&assignments);
    let (row, life) = crate::tools::places::triggers::shown(edits, table, key, columns.as_deref())?;
    Some(Shown { row: read(&row)?, life, in_database: held.cloned() })
}

/// Every key the store holds for `table`, whatever it does to the row.
pub(crate) fn claimed_keys(edits: &Edits, table: &str) -> Vec<Key> {
    edits.table(table).map(|(key, _)| key.clone()).collect()
}

/// Make the row at `key` hold `columns`, as one undo entry: a creation when
/// neither the database nor the project has a row there, the project's
/// creation rewritten when it has one, and otherwise the database's row,
/// kept when it was marked for removal, with each column that differs from
/// `held` written over it.
pub(crate) fn put_row(
    session: &mut EditSession,
    table: &'static str,
    key: &Key,
    held: Option<&[Assignment]>,
    columns: &[Assignment],
    gesture: Gesture<'_>,
) {
    let life = session.rows.edits.row(table, key).map(|claim| claim.life);
    match (held, life) {
        (None, _) | (_, Some(Life::Insert)) => {
            let row = crate::subjects::services::creation(columns);
            session.set_server_row(table, key, Some(&row), Some(gesture));
        }
        (Some(held), life) => {
            session.as_one(gesture.label, gesture.subject, |session| {
                if life == Some(Life::Delete) {
                    session.set_server_row(table, key, None, Some(gesture));
                }
                for change in columns {
                    crate::subjects::services::write_column(
                        session,
                        table,
                        key,
                        Some(held),
                        change.column,
                        change.value.clone(),
                        gesture,
                    );
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::formation::{self, Member};

    /// A database row is shown with the store's columns over it, a creation
    /// as itself, a removal with the database's columns, and a key with
    /// neither as nothing.
    #[test]
    fn a_row_is_shown_with_the_store_over_the_database() {
        let held = Member::new(10, 11, 3.0, 1.0);
        let mut edits = Edits::default();
        edits.set(formation::GROUPS, &held.key(), "dist", Some("5".into()));
        let read = |edits: &Edits, key: &Key, held: Option<&Member>| {
            overlay(edits, formation::GROUPS, key, held, Member::assignments, Member::from_row)
        };
        let shown = read(&edits, &held.key(), Some(&held)).unwrap();
        assert_eq!((shown.row.dist, shown.life), (5.0, Life::Update));
        assert_eq!(shown.in_database.as_ref().unwrap().dist, 3.0);
        assert!(read(&edits, &formation::member_key(99), None).is_none());
        assert_eq!(claimed_keys(&edits, formation::GROUPS), vec![held.key()]);
        assert!(claimed_keys(&edits, formation::LINKING).is_empty());
    }
}
