//! Changing a row's id in the project's store, and what follows it there.
//!
//! An id is changed by **re-keying the row's one claim** — see
//! `vale_mangos::row::RowEdit::from`, which is where the claim records where
//! the database has the row. That is the whole of it for the row itself, and it
//! is the same operation for an item, a quest, a creature template and a spawn.
//!
//! What this module adds is the second half: **the project's other claims that
//! name the id are rewritten to name the new one.** A quest this project edits
//! to reward item 852 has `RewItemId1 = 852` in the store. The database's rows
//! follow a move when it is applied (`vale_mangos::row::move_statements`
//! writes one `UPDATE` per reference), and then the quest's own apply would
//! write 852 straight back over it. One list of references serves both: the
//! subject's `REFERENCES`.
//!
//! A reference whose column is part of the claim's *key* — a quest relation is
//! keyed by `(id, quest)` — re-keys that claim, with no origin recorded: by the
//! time such a row's statements run the database has already followed, because
//! a plan writes the moved rows first. See `super::quests::Plan::ordered`.
//!
//! Every write goes through the session under one gesture, so the move and
//! everything that followed it are one entry on the undo stack.

use crate::session::{EditSession, Gesture};
use vale_mangos::row::{Edits, Key, Reference, RowEdit};

/// **Re-key one row's claim and make the project's own references follow.**
///
/// `at_base` is where the database has the row, for a claim that has not
/// recorded it yet; a row the project creates records none.
pub fn rekey(
    session: &mut EditSession,
    table: &str,
    from: &Key,
    to: &Key,
    at_base: Option<&Key>,
    references: &[Reference],
    gesture: Gesture<'_>,
) -> Result<(), String> {
    if session.server_edits.touches(table, to) {
        return Err(format!("this project already claims {table} {}", to.text()));
    }
    // The re-key is `Edits::rekey`'s decision. It is made on a copy of the one
    // claim so the write itself can go through the session, which is what puts
    // it on the undo stack.
    let mut scratch = Edits::default();
    if let Some(line) = session.server_edits.row_line(table, from) {
        scratch.set_row_line(table, from, Some(&line));
    }
    scratch.rekey(table, from, to, at_base);
    let moved: Option<RowEdit> = scratch.row(table, to).cloned();
    session.set_server_row(table, from, None, Some(gesture));
    session.set_server_row(table, to, moved.as_ref(), Some(gesture));

    let (Some(old), Some(new)) = (from.first(), to.first()) else {
        return Ok(());
    };
    if old != new {
        follow(session, references, old, new, gesture);
    }
    Ok(())
}

/// One claim of the store that names an id.
#[derive(Debug, PartialEq)]
pub enum Hit {
    /// A column of a claim holds it.
    Column {
        table: String,
        key: Key,
        column: String,
        value: String,
    },
    /// A column of a claim's key holds it.
    KeyColumn { table: String, key: Key, to: Key },
}

/// Rewrite every claim of the project's store that names `old` through one of
/// `references` to name `new`.
fn follow(
    session: &mut EditSession,
    references: &[Reference],
    old: u64,
    new: u64,
    gesture: Gesture<'_>,
) {
    for hit in hits(&session.server_edits, references, old, new) {
        match hit {
            Hit::Column {
                table,
                key,
                column,
                value,
            } => {
                session.set_server_edit(&table, &key, &column, Some(value), Some(gesture));
            }
            Hit::KeyColumn { table, key, to } => {
                if session.server_edits.touches(&table, &to) {
                    continue;
                }
                let Some(row) = session.server_edits.row(&table, &key).cloned() else {
                    continue;
                };
                session.set_server_row(&table, &key, None, Some(gesture));
                session.set_server_row(&table, &to, Some(&row), Some(gesture));
            }
        }
    }
}

/// **Which claims of a store name `old` through one of `references`**, and what
/// each becomes. The decision, apart from the writes, so it can be checked with
/// no session.
pub fn hits(edits: &Edits, references: &[Reference], old: u64, new: u64) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    for reference in references {
        let (was, will) = match reference.negated {
            true => (format!("-{old}"), format!("-{new}")),
            false => (old.to_string(), new.to_string()),
        };
        for (table, key, row) in edits.rows() {
            if table != reference.table {
                continue;
            }
            // A loot row whose `mincountOrRef` is negative names a reference
            // template, and its `item` is a label: the move's own statement
            // leaves it, so the claim must too.
            if reference.ruled_out_by(&row.columns) {
                continue;
            }
            if row
                .columns
                .get(reference.column)
                .is_some_and(|value| value.trim() == was)
            {
                hits.push(Hit::Column {
                    table: table.to_string(),
                    key: key.clone(),
                    column: reference.column.to_string(),
                    value: will.clone(),
                });
            }
            let in_key = key
                .0
                .iter()
                .any(|(column, value)| column == reference.column && *value == was);
            if in_key {
                let mut to = key.clone();
                for (column, value) in to.0.iter_mut() {
                    if column == reference.column {
                        value.clone_from(&will);
                    }
                }
                hits.push(Hit::KeyColumn {
                    table: table.to_string(),
                    key: key.clone(),
                    to,
                });
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::quest;
    use vale_mangos::row::Life;

    /// **A quest moved from 783 to 2,000,783 takes the project's own rows that
    /// name it**: a relation this project adds is re-keyed, and another quest's
    /// chain columns are rewritten, in the positive form and the negated one.
    /// A claim that names some other quest is left alone.
    #[test]
    fn the_projects_own_references_follow_a_move() {
        let mut edits = Edits::default();
        let relation = quest::relation_key(823, 783);
        edits.set_life(quest::CREATURE_GIVES, &relation, Life::Insert);
        let other = quest::template_key(784, 0);
        edits.set(quest::TEMPLATE, &other, "PrevQuestId", Some("-783".into()));
        edits.set(
            quest::TEMPLATE,
            &other,
            "NextQuestInChain",
            Some("783".into()),
        );
        edits.set(quest::TEMPLATE, &other, "NextQuestId", Some("785".into()));

        let found = hits(&edits, &quest::REFERENCES, 783, 2_000_783);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.contains(&Hit::KeyColumn {
            table: quest::CREATURE_GIVES.to_string(),
            key: relation,
            to: quest::relation_key(823, 2_000_783),
        }));
        assert!(found.contains(&Hit::Column {
            table: quest::TEMPLATE.to_string(),
            key: other.clone(),
            column: "PrevQuestId".to_string(),
            value: "-2000783".to_string(),
        }));
        assert!(found.contains(&Hit::Column {
            table: quest::TEMPLATE.to_string(),
            key: other,
            column: "NextQuestInChain".to_string(),
            value: "2000783".to_string(),
        }));
    }

    /// **A loot row that points at a reference template is not re-keyed** by
    /// an item move: its `item` is a label, and the move's own `UPDATE` leaves
    /// it because of `mincountOrRef > 0`. A real drop of the item is re-keyed.
    #[test]
    fn a_reference_rows_label_does_not_follow_an_item() {
        use vale_mangos::loot;
        let mut edits = Edits::default();
        let drop = loot::key(40, 852, 0, 0, 10);
        let pointer = loot::key(41, 852, 0, 0, 10);
        edits.set(loot::CREATURE, &drop, "mincountOrRef", Some("1".into()));
        edits.set(loot::CREATURE, &pointer, "mincountOrRef", Some("-852".into()));
        let found = hits(&edits, &vale_mangos::item::REFERENCES, 852, 2_000_456);
        let moved: Vec<&Key> = found
            .iter()
            .filter_map(|hit| match hit {
                Hit::KeyColumn { key, .. } => Some(key),
                _ => None,
            })
            .collect();
        assert_eq!(moved, vec![&drop]);
    }
}
