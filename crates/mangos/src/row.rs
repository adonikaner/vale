//! What an edit to one row is, whichever table the row is in.
//!
//! ## Two kinds of server edit, and only one of them has a file behind it
//!
//! [`crate::spell`] writes `spell_template` from the difference between two
//! copies of `Spell.dbc`. The client ships that file, so the project's own copy
//! of it *is* the record of what the project changes, and nothing else has to be
//! stored.
//!
//! `creature_template`, `creature`, `item_template` and `quest_template` have no
//! client file at all. The row in vmangos' database is the only copy there is,
//! so an edit to one is a value a person typed and there is nowhere for it to
//! live until it is written down. [`Edits`] is where: a table, a key, a column
//! and the SQL literal the column is to hold, one per line, in a file the
//! project keeps.
//!
//! ## The stored value is already a SQL literal
//!
//! `'Kobold Vermin'` rather than `Kobold Vermin`, quoted and escaped by
//! [`crate::sql::text`] at the moment it is typed. Escaping once, on the way in,
//! means there is exactly one place the rule lives — and the file stays
//! something a person can read and paste into a statement.
//!
//! ## A row is edited, created or removed, and the store says which
//!
//! Most edits are an `UPDATE` of a row that is already there, and for a long
//! time that was the only kind. A new creature spawn is not: there is no row to
//! update, and the project's claim is the whole of it. A removed spawn is the
//! third: the project claims the row should not be there at all, and carries no
//! columns for it.
//!
//! [`Life`] is that third piece of state, one per row beside its columns, and it
//! decides which statement the row becomes. It is also what stops a removal
//! from being mistaken for "no edit": a row marked [`Life::Delete`] has no
//! columns, and a store that dropped empty rows would lose it.
//!
//! ## A key is a list of columns
//!
//! `spell_template` and `creature` are keyed by one column. `creature_template`
//! is keyed by two — `(entry, patch)` — because vmangos keeps several content
//! patches of the same creature and loads the highest at or below its configured
//! `WowPatch` (`ObjectMgr::LoadCreatureTemplates`). An edit that named only the
//! entry would change every patch of it, including versions of the row the
//! running server is not using. So a [`Key`] carries whatever columns identify
//! the row, and the `WHERE` it builds names all of them.

use std::collections::BTreeMap;

/// One column and the value it is to hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub column: &'static str,
    /// Already a SQL literal — quoted and escaped if it is text.
    pub value: String,
}

impl Assignment {
    /// `` `column` = value ``.
    pub fn sql(&self) -> String {
        format!("{} = {}", crate::sql::name(self.column), self.value)
    }
}

/// **What this project says should become of a row**: it is edited, it is
/// created, or it goes.
///
/// One per row in an [`Edits`] store, beside its columns, and it decides the
/// statement the row becomes — an `UPDATE`, an `INSERT` or a `DELETE`. The
/// names are the statements' own, so the store, the file and the SQL all say
/// the same word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Life {
    /// The row is in the database and this project changes columns of it.
    #[default]
    Update,
    /// It is not, and this project creates it. Every column the table has is
    /// carried, because an `INSERT` that leaves one out takes the column's
    /// default instead of a value somebody chose.
    Insert,
    /// It is, and this project removes it. It carries no columns at all, which
    /// is why a store cannot drop a row for being empty.
    Delete,
}

impl Life {
    /// The word the file and the statement use.
    pub fn word(self) -> &'static str {
        match self {
            Life::Update => "update",
            Life::Insert => "insert",
            Life::Delete => "delete",
        }
    }

    /// …and back. `None` for anything else, which a hand-edited file can hold.
    pub fn named(word: &str) -> Option<Life> {
        match word.trim() {
            "update" => Some(Life::Update),
            "insert" => Some(Life::Insert),
            "delete" => Some(Life::Delete),
            _ => None,
        }
    }
}

/// Which row a statement is about.
///
/// The values are SQL literals, as [`Assignment`]'s are, for the same reason:
/// one escaping rule, applied once.
///
/// **Ordered by number, not by text.** A plan runs its rows in the store's
/// order, and a text order puts `2000456` before `852`: a claim on one content
/// patch of item 852 and a move of another would run the move first, which
/// takes every patch with it and leaves the claim's `UPDATE` matching nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key(pub Vec<(String, String)>);

impl Ord for Key {
    fn cmp(&self, other: &Key) -> std::cmp::Ordering {
        let pairs = self.0.iter().zip(other.0.iter());
        for ((column, value), (other_column, other_value)) in pairs {
            let by_value = match (value.parse::<i128>(), other_value.parse::<i128>()) {
                (Ok(a), Ok(b)) => a.cmp(&b),
                _ => value.cmp(other_value),
            };
            let order = column.cmp(other_column).then(by_value);
            if order.is_ne() {
                return order;
            }
        }
        self.0.len().cmp(&other.0.len())
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Key) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Key {
    /// A key of one numeric column — `guid`, `entry`, `id`.
    pub fn one(column: &str, value: u64) -> Key {
        Key(vec![(column.to_string(), value.to_string())])
    }

    /// …and of two, which is `creature_template`'s `(entry, patch)`.
    pub fn two(first: (&str, u64), second: (&str, u64)) -> Key {
        Key(vec![
            (first.0.to_string(), first.1.to_string()),
            (second.0.to_string(), second.1.to_string()),
        ])
    }

    /// `` `entry` = 3296 AND `patch` = 0 ``.
    pub fn where_clause(&self) -> String {
        self.0
            .iter()
            .map(|(column, value)| format!("{} = {value}", crate::sql::name(column)))
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    /// `entry=3296;patch=0` — how the key is written in a file and in the undo
    /// file's marker.
    pub fn text(&self) -> String {
        self.0
            .iter()
            .map(|(column, value)| format!("{column}={value}"))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// …and read back. `None` for anything that is not `column=value` pairs.
    ///
    /// **Every key this crate writes is integers under plain column names, and
    /// nothing else is read.** A key goes into a `WHERE` as it stands, and the
    /// text it comes from is a file beside the project that a person may edit:
    /// `entry=1 OR 1=1` would otherwise be a `DELETE` of the whole table.
    pub fn parse(text: &str) -> Option<Key> {
        let mut out = Vec::new();
        for part in text.split(';') {
            let (column, value) = part.split_once('=')?;
            let (column, value) = (column.trim(), value.trim());
            let plain_column = !column.is_empty()
                && column.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            // Canonical as well as numeric, so that two keys the order calls
            // equal are also equal as text — `007` and `7` would be neither
            // one row nor two.
            if !plain_column || !is_canonical_integer(value) {
                return None;
            }
            out.push((column.to_string(), value.to_string()));
        }
        match out.is_empty() {
            true => None,
            false => Some(Key(out)),
        }
    }

    /// The first column's value as a number, for anything that wants one id to
    /// report — a status line, a sort, the entry a creature's row is about.
    pub fn first(&self) -> Option<u64> {
        self.0.first()?.1.parse().ok()
    }
}

/// **`UPDATE <table> SET … WHERE <key>`**, or `None` when nothing changed.
pub fn update(table: &str, key: &Key, changes: &[Assignment]) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    let sets: Vec<String> = changes.iter().map(Assignment::sql).collect();
    Some(format!(
        "UPDATE {} SET {} WHERE {};",
        crate::sql::name(table),
        sets.join(", "),
        key.where_clause()
    ))
}

/// **The `UPDATE` a row whose id changes becomes**: the key columns that differ
/// are set to where the project puts the row, the edited columns beside them,
/// and the `WHERE` names where the database has it. See [`RowEdit::from`].
pub fn update_moved(table: &str, from: &Key, to: &Key, changes: &[Assignment]) -> String {
    let mut sets = key_sets(from, to);
    sets.extend(changes.iter().map(Assignment::sql));
    format!(
        "UPDATE {} SET {} WHERE {};",
        crate::sql::name(table),
        sets.join(", "),
        from.where_clause()
    )
}

/// …and **the statement that puts such a row back**: the key columns returned
/// to where the database had them and the edited columns to what it held, with
/// a `WHERE` naming where the move leaves the row. An undo naming the old key
/// would match nothing once the move has run.
///
/// `None` when the row is not in the database, on [`undo`]'s own rule.
pub fn undo_moved(
    table: &str,
    from: &Key,
    to: &Key,
    changes: &[Assignment],
    now: Option<&std::collections::HashMap<String, Option<String>>>,
) -> Option<String> {
    let now = now?;
    let mut sets = key_sets(to, from);
    for change in changes {
        let value = match now.get(change.column) {
            Some(Some(value)) => literal(value),
            Some(None) => "NULL".to_string(),
            None => return_unchanged(change),
        };
        sets.push(format!("{} = {value}", crate::sql::name(change.column)));
    }
    Some(format!(
        "UPDATE {} SET {} WHERE {};",
        crate::sql::name(table),
        sets.join(", "),
        to.where_clause()
    ))
}

/// `` `column` = value `` for every key column `to` holds differently from
/// `from`.
fn key_sets(from: &Key, to: &Key) -> Vec<String> {
    to.0.iter()
        .filter(|(column, value)| {
            from.0
                .iter()
                .find(|(had, _)| had == column)
                .is_none_or(|(_, had)| had != value)
        })
        .map(|(column, value)| format!("{} = {value}", crate::sql::name(column)))
        .collect()
}

/// **One reference to a row's id held by another table**: the table, and the
/// column of it that names the id.
///
/// vmangos declares no foreign keys, so nothing follows a row whose id changes.
/// Each subject lists the columns that name its ids — `item::REFERENCES`,
/// `quest::REFERENCES`, `creature::TEMPLATE_REFERENCES`,
/// `creature::SPAWN_REFERENCES` — and a move writes one `UPDATE` per reference.
/// See [`move_statements`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reference {
    pub table: &'static str,
    pub column: &'static str,
    /// **A further condition the row must meet for the column to be this kind
    /// of id**, as SQL. A loot table's `item` is an item only while
    /// `mincountOrRef > 0`; below zero the row points at a reference template
    /// and `item` is a label.
    pub only: Option<&'static str>,
    /// **Whether the column holds the id negated.** `PrevQuestId = -783` and
    /// `ReqCreatureOrGOId1 = -1234` name the quest and the game object by the
    /// negative of their ids, in the same column that names another kind of
    /// row by a positive one.
    pub negated: bool,
}

impl Reference {
    pub const fn new(table: &'static str, column: &'static str) -> Reference {
        Reference { table, column, only: None, negated: false }
    }

    pub const fn only(table: &'static str, column: &'static str, only: &'static str) -> Reference {
        Reference { table, column, only: Some(only), negated: false }
    }

    pub const fn negated(table: &'static str, column: &'static str) -> Reference {
        Reference { table, column, only: None, negated: true }
    }

    /// The id as this column writes it.
    pub fn literal(&self, id: u64) -> String {
        match self.negated {
            true => format!("-{id}"),
            false => id.to_string(),
        }
    }

    /// **Whether a claim's own columns rule this reference out**: its
    /// [`Self::only`] condition, evaluated against the values the claim sets.
    ///
    /// `false` when the claim does not set the condition's column, because the
    /// database's value then decides and the statement's own `WHERE` asks it.
    /// The one condition in use is `` `column` > n ``; anything else is never
    /// ruled out here.
    pub fn ruled_out_by(&self, columns: &BTreeMap<String, String>) -> bool {
        let Some(only) = self.only else { return false };
        let Some((column, bound)) = only.split_once(" > ") else {
            return false;
        };
        let column = column.trim().trim_matches('`');
        let (Some(value), Ok(bound)) = (columns.get(column), bound.trim().parse::<i64>()) else {
            return false;
        };
        value.trim().parse::<i64>().is_ok_and(|value| value <= bound)
    }

    /// `` `column` = <id> ``, with the condition when there is one.
    fn names(&self, id: u64) -> String {
        let column = crate::sql::name(self.column);
        match self.only {
            Some(only) => format!("{column} = {} AND {only}", self.literal(id)),
            None => format!("{column} = {}", self.literal(id)),
        }
    }

    /// **The statement that makes this reference follow a move.** With the ids
    /// exchanged it is its own undo, which holds only while nothing named the
    /// target id before the move: see [`Reference::count`], which an apply asks
    /// of every reference before it runs anything.
    pub fn follow(&self, from: u64, to: u64) -> String {
        format!(
            "UPDATE {} SET {} = {} WHERE {};",
            crate::sql::name(self.table),
            crate::sql::name(self.column),
            self.literal(to),
            self.names(from)
        )
    }

    /// The query that counts the rows naming an id through this reference.
    pub fn count(&self, id: u64) -> String {
        format!(
            "SELECT COUNT(*) AS `n` FROM {} WHERE {}",
            crate::sql::name(self.table),
            self.names(id)
        )
    }

    /// …and the same count **leaving out the rows at `except`**: rows this
    /// project writes itself, whose own undo puts the column back. See
    /// `crates/editor/src/server/reconcile.rs`, which is the one caller.
    pub fn count_except(&self, id: u64, except: &[Key]) -> String {
        if except.is_empty() {
            return self.count(id);
        }
        let skipped: Vec<String> = except
            .iter()
            .map(|key| format!("({})", key.where_clause()))
            .collect();
        format!(
            "SELECT COUNT(*) AS `n` FROM {} WHERE {} AND NOT ({})",
            crate::sql::name(self.table),
            self.names(id),
            skipped.join(" OR ")
        )
    }

    /// **The rows naming an id through this reference, whole**, which is what
    /// the undo of a removal that takes them is read from.
    pub fn select(&self, id: u64) -> String {
        format!(
            "SELECT * FROM {} WHERE {}",
            crate::sql::name(self.table),
            self.names(id)
        )
    }

    /// **The statement that removes those rows**, for a removal that takes them
    /// with the row they name.
    pub fn delete(&self, id: u64) -> String {
        format!(
            "DELETE FROM {} WHERE {};",
            crate::sql::name(self.table),
            self.names(id)
        )
    }
}

/// **Whether any row holds this key's id**, at any content patch.
///
/// What an apply asks before it creates a row or moves one onto an id. The
/// whole key is the wrong question for a table keyed by `(entry, patch)`: an
/// entry that exists only at a patch the server does not load is still
/// somebody's, and a row written beside it is a newer version of their item.
pub fn id_exists_query(table: &str, key: &Key) -> String {
    let clause = match (key.0.first(), key.first()) {
        (Some((column, _)), Some(id)) => format!("{} = {id}", crate::sql::name(column)),
        _ => key.where_clause(),
    };
    format!("SELECT 1 FROM {} WHERE {clause} LIMIT 1", crate::sql::name(table))
}

/// **Every statement moving a row's id comes to**, in the order they run.
///
/// ```text
/// UPDATE t SET entry = N, <edited columns> WHERE entry = O AND patch = P;
/// UPDATE t SET entry = N WHERE entry = O;          the other content patches
/// UPDATE <reference> SET <column> = N WHERE <column> = O;     one per reference
/// ```
///
/// **The second statement is why a move names the id column and not the whole
/// key.** `item_template`, `quest_template` and `creature_template` are keyed
/// by `(entry, patch)` and the server loads the highest patch at or below its
/// own. Moving one version leaves the older ones at the old entry, where the
/// server loads them: the row would be at both ids. For a table keyed by the id
/// alone the statement matches nothing and is left out.
pub fn move_statements(
    table: &str,
    from: &Key,
    to: &Key,
    changes: &[Assignment],
    references: &[Reference],
) -> Vec<String> {
    let mut out = vec![update_moved(table, from, to, changes)];
    let (Some((column, _)), Some(old), Some(new)) = (from.0.first(), from.first(), to.first())
    else {
        return out;
    };
    if old == new {
        return out;
    }
    if from.0.len() > 1 {
        let column = crate::sql::name(column);
        out.push(format!(
            "UPDATE {} SET {column} = {new} WHERE {column} = {old};",
            crate::sql::name(table)
        ));
    }
    out.extend(references.iter().map(|reference| reference.follow(old, new)));
    out
}

/// …and **every statement that puts such a row back**: [`move_statements`]
/// undone last to first. `None` when the row is not in the database.
pub fn undo_move_statements(
    table: &str,
    from: &Key,
    to: &Key,
    changes: &[Assignment],
    references: &[Reference],
    now: Option<&std::collections::HashMap<String, Option<String>>>,
) -> Option<Vec<String>> {
    let row_back = undo_moved(table, from, to, changes, now)?;
    let mut out = Vec::new();
    if let (Some((column, _)), Some(old), Some(new)) = (from.0.first(), from.first(), to.first()) {
        if old != new {
            out.extend(references.iter().rev().map(|reference| reference.follow(new, old)));
            out.push(row_back);
            if from.0.len() > 1 {
                let column = crate::sql::name(column);
                out.push(format!(
                    "UPDATE {} SET {column} = {old} WHERE {column} = {new};",
                    crate::sql::name(table)
                ));
            }
            return Some(out);
        }
    }
    out.push(row_back);
    Some(out)
}

/// **`INSERT INTO <table> (…) VALUES (…)`**, or `None` when nothing is named.
///
/// The key's own columns come first and then the changes, so a row created by
/// this crate names its key in the statement rather than relying on a database
/// default or an auto-increment. A column the caller does not name takes the
/// column's own default, which is why the caller is expected to name all of
/// them — see [`Life::Insert`].
pub fn insert(table: &str, key: &Key, changes: &[Assignment]) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    for (column, value) in &key.0 {
        names.push(crate::sql::name(column));
        values.push(value.clone());
    }
    for change in changes {
        // A key column named twice would be a statement MySQL refuses. It
        // cannot come from this crate's own schemas, where a key column is
        // `Kind::Key` and never editable, but the store is a text file.
        if key.0.iter().any(|(column, _)| column == change.column) {
            continue;
        }
        names.push(crate::sql::name(change.column));
        values.push(change.value.clone());
    }
    if names.is_empty() {
        return None;
    }
    Some(format!(
        "INSERT INTO {} ({}) VALUES ({});",
        crate::sql::name(table),
        names.join(", "),
        values.join(", ")
    ))
}

/// **`DELETE FROM <table> WHERE <key>`.**
pub fn delete(table: &str, key: &Key) -> String {
    format!(
        "DELETE FROM {} WHERE {};",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// **A row read out of the database, as the `INSERT` that puts it back.**
///
/// What the undo of a removal is: every column the `SELECT` returned, named,
/// with the same rule the undo writes a value by and a SQL `NULL` written as
/// one. It needs no schema, which is the point — the five tables a removed
/// creature spawn drags with it are restored this way without this crate
/// knowing a column of any of them.
///
/// The columns are sorted, so the statement is the same text whichever order
/// the driver handed the row back in and two runs produce the same file.
pub fn insert_from_row(
    table: &str,
    row: &std::collections::HashMap<String, Option<String>>,
) -> Option<String> {
    if row.is_empty() {
        return None;
    }
    let mut columns: Vec<(&String, &Option<String>)> = row.iter().collect();
    columns.sort_by(|a, b| a.0.cmp(b.0));
    let names: Vec<String> = columns
        .iter()
        .map(|(column, _)| crate::sql::name(column))
        .collect();
    let values: Vec<String> = columns
        .iter()
        .map(|(_, value)| match value {
            Some(value) => literal(value),
            None => "NULL".to_string(),
        })
        .collect();
    Some(format!(
        "INSERT INTO {} ({}) VALUES ({});",
        crate::sql::name(table),
        names.join(", "),
        values.join(", ")
    ))
}

/// **The statement that puts a row back**, given what it holds now.
///
/// One `UPDATE` naming the same columns the edit names, with the values read
/// out of the database before the edit ran. `None` when the row is not there —
/// this crate never creates a creature or a spawn, so a key that matches
/// nothing is a key naming a row somebody else removed, and inventing an
/// `INSERT` for it would write a row this project never saw.
pub fn undo(
    table: &str,
    key: &Key,
    changes: &[Assignment],
    now: Option<&std::collections::HashMap<String, Option<String>>>,
) -> Option<String> {
    let now = now?;
    let back: Vec<Assignment> = changes
        .iter()
        .map(|change| Assignment {
            column: change.column,
            value: match now.get(change.column) {
                // A column the row has, as text. Everything comes back from
                // MySQL as text and goes into the statement as text, so the
                // value never passes through a typed round trip that could move
                // a float in its fourth decimal.
                Some(Some(value)) => literal(value),
                // A SQL NULL, which `subname` and `auras` can hold.
                Some(None) => "NULL".to_string(),
                // A column the select did not return. Nothing to put back, and
                // a guess would write over a value this end never read.
                None => return_unchanged(change),
            },
        })
        .collect();
    update(table, key, &back)
}

/// A value read out of the database, as a literal to write back.
///
/// **Unquoted only when it is an integer written the one way MySQL writes
/// one**, and quoted otherwise.
///
/// The value is read back without its column's type, so the literal has to
/// mean the same text in a text column as in a numeric one. A canonical
/// integer does: `12` stored in a `varchar` is `'12'`. Anything else does not.
/// `007` and `+5` come back as `7` and `5`, `1e3` as `1000`, and `Infinity`
/// or `Nan` — which `f64::from_str` accepts — are read by MySQL as column
/// names, so the statement fails and every later Put back fails with it. A
/// quoted literal is converted by MySQL exactly as it converts the unquoted
/// one in a numeric column, which covers every float.
pub(crate) fn literal(value: &str) -> String {
    match is_canonical_integer(value) {
        true => value.to_string(),
        false => crate::sql::text(value),
    }
}

/// `0`, or an optional minus and digits with no leading zero.
fn is_canonical_integer(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    canonical && value != "-0"
}

/// What an undo writes for a column the select did not return: the value the
/// edit itself is setting, which leaves that column where the edit put it.
///
/// It cannot happen for a column named by one of this crate's own schemas,
/// because the snapshot selects `*`. It is here so that a column added by hand
/// to an edits file cannot make the undo silently drop a column.
fn return_unchanged(change: &Assignment) -> String {
    change.value.clone()
}

/// **One row's whole edit**: what is to become of it, and the columns it sets.
///
/// A [`Life::Update`] row is the ordinary case and carries only the columns
/// that changed. A [`Life::Insert`] row carries every column the table has,
/// because the statement that creates it names them all. A [`Life::Delete`]
/// row carries none.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RowEdit {
    pub life: Life,
    /// Column to SQL literal, in name order.
    pub columns: BTreeMap<String, String>,
    /// **Where the row is in the database, when that is not the key this edit
    /// is stored under** — a row whose id the project changes.
    ///
    /// The store's key is the row's identity *as the project says it*: the new
    /// id. This is where the database had it before the project touched it, and
    /// it is what the `WHERE` of the `UPDATE` names. One row therefore has one
    /// claim however often its id is changed, every later column edit lands on
    /// that claim, and moving it back to where it came from clears this and
    /// leaves an ordinary edit.
    ///
    /// `None` for every row whose id the project leaves alone, and for a row
    /// the project creates, which was nowhere.
    pub from: Option<Key>,
}

impl RowEdit {
    /// Whether this row edit says nothing at all, which is when the store drops
    /// it.
    ///
    /// **A removal is never empty**, however few columns it has: the claim is
    /// the [`Life`], and a store that asked only about the columns would forget
    /// one on the frame it was made.
    pub fn is_empty(&self) -> bool {
        self.life == Life::Update && self.columns.is_empty() && self.from.is_none()
    }

    /// The pseudo-column a line carries [`Self::from`] under. Not a name a
    /// table can have for a column this crate writes: every schema here is
    /// vmangos' own and none holds a hyphen.
    const FROM: &'static str = "moved-from";

    /// **This row edit as one line**, for an undo stack to carry it whole.
    ///
    /// Its life, then a `column=value` per tab. Tab-separated because a value
    /// is a SQL literal, which may hold a semicolon, a comma or a space and
    /// cannot hold a tab — `crate::sql::text` writes one as `\t`.
    pub fn to_line(&self) -> String {
        let mut out = self.life.word().to_string();
        if let Some(from) = &self.from {
            out.push('\t');
            out.push_str(Self::FROM);
            out.push('=');
            out.push_str(&from.text());
        }
        for (column, value) in &self.columns {
            out.push('\t');
            out.push_str(column);
            out.push('=');
            out.push_str(value);
        }
        out
    }

    /// …and back. A field that is not `column=value` is dropped rather than
    /// failing the line, on [`Edits::from_text`]'s rule.
    pub fn from_line(line: &str) -> RowEdit {
        let mut fields = line.split('\t');
        let life = fields.next().and_then(Life::named).unwrap_or_default();
        let mut columns = BTreeMap::new();
        let mut from = None;
        for field in fields {
            let Some((column, value)) = field.split_once('=') else {
                continue;
            };
            if column.trim().is_empty() {
                continue;
            }
            if column.trim() == Self::FROM {
                from = Key::parse(value.trim());
                continue;
            }
            // Not a literal, so not a value a statement may carry — see
            // `crate::sql::is_literal`.
            if !crate::sql::is_literal(value.trim()) {
                continue;
            }
            columns.insert(column.trim().to_string(), value.trim().to_string());
        }
        RowEdit { life, columns, from }
    }
}

/// **Every row edit a project carries**, in the order they will be written.
///
/// Keyed by table and then by key, so two edits to the same row become one
/// statement and an edit typed twice does not appear twice.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Edits {
    rows: BTreeMap<(String, Key), RowEdit>,
}

impl Edits {
    /// The word a three-field line of the file carries an origin under,
    /// followed by the key: `from entry=852;patch=10`.
    const FROM: &'static str = "from ";

    /// Set a column, or clear it when `value` is `None`.
    ///
    /// Clearing is how an edit is taken back: the column is removed from the
    /// file, so the project stops claiming to change it and the next save emits
    /// no statement for it. A row with nothing left to say removes itself —
    /// which for a removal is never, because the claim is its [`Life`] rather
    /// than its columns.
    pub fn set(&mut self, table: &str, key: &Key, column: &str, value: Option<String>) {
        let at = (table.to_string(), key.clone());
        match value {
            Some(value) => {
                self.rows
                    .entry(at)
                    .or_default()
                    .columns
                    .insert(column.to_string(), value);
            }
            None => {
                if let Some(row) = self.rows.get_mut(&at) {
                    row.columns.remove(column);
                    if row.is_empty() {
                        self.rows.remove(&at);
                    }
                }
            }
        }
    }

    /// **Say what is to become of the row**: it is edited, created or removed.
    ///
    /// Setting [`Life::Update`] on a row with no columns removes it from the
    /// store, which is how a removal is taken back.
    pub fn set_life(&mut self, table: &str, key: &Key, life: Life) {
        let at = (table.to_string(), key.clone());
        let row = self.rows.entry(at.clone()).or_default();
        row.life = life;
        if row.is_empty() {
            self.rows.remove(&at);
        }
    }

    /// **Re-key a row's claim**, which is what changing a row's id is to the
    /// store — see [`RowEdit::from`].
    ///
    /// `at_base` is where the database holds the row when the project has no
    /// origin recorded for it; a claim that already names one keeps its own,
    /// because a second move is still a move from where the database has the
    /// row. A row the project creates has no origin and is re-keyed and nothing
    /// else. Moving a row back onto its origin clears it, and a claim left
    /// saying nothing is dropped.
    ///
    /// Answers `false`, writing nothing, when `to` is already claimed.
    pub fn rekey(&mut self, table: &str, key: &Key, to: &Key, at_base: Option<&Key>) -> bool {
        if key == to {
            return true;
        }
        if self.touches(table, to) {
            return false;
        }
        let mut row = self
            .rows
            .remove(&(table.to_string(), key.clone()))
            .unwrap_or_default();
        if row.life != Life::Insert && row.from.is_none() {
            row.from = Some(at_base.cloned().unwrap_or_else(|| key.clone()));
        }
        if row.from.as_ref() == Some(to) {
            row.from = None;
        }
        if !row.is_empty() {
            self.rows.insert((table.to_string(), to.clone()), row);
        }
        true
    }

    /// Every row of one table whose id this project changes, as
    /// `(where the database has it, where the project puts it)`.
    pub fn moves<'a>(&'a self, table: &'a str) -> impl Iterator<Item = (&'a Key, &'a Key)> + 'a {
        self.rows
            .iter()
            .filter(move |((had, _), _)| had == table)
            .filter_map(|((_, key), row)| Some((row.from.as_ref()?, key)))
    }

    /// Where this project puts the row the database holds at `from`, if it
    /// moves it.
    pub fn moved_to<'a>(&'a self, table: &'a str, from: &Key) -> Option<&'a Key> {
        self.moves(table)
            .find(|(had, _)| *had == from)
            .map(|(_, key)| key)
    }

    /// …and what it is. [`Life::Update`] for a row this project says nothing
    /// about, because that is what an edit to one would be.
    pub fn life(&self, table: &str, key: &Key) -> Life {
        self.row(table, key).map(|row| row.life).unwrap_or_default()
    }

    /// One row's whole edit, or `None` when this project says nothing about it.
    pub fn row(&self, table: &str, key: &Key) -> Option<&RowEdit> {
        self.rows.get(&(table.to_string(), key.clone()))
    }

    /// **One row's whole edit as a line** — what an undo entry for a creation
    /// or a removal carries. See [`RowEdit::to_line`].
    ///
    /// `None` is "this project says nothing about the row", which is a
    /// different claim from a line holding a life and no columns.
    pub fn row_line(&self, table: &str, key: &Key) -> Option<String> {
        self.row(table, key).map(RowEdit::to_line)
    }

    /// …and put one back, or take the project's claim on the row away.
    pub fn set_row_line(&mut self, table: &str, key: &Key, line: Option<&str>) {
        let at = (table.to_string(), key.clone());
        match line {
            Some(line) => {
                let row = RowEdit::from_line(line);
                match row.is_empty() {
                    true => self.rows.remove(&at),
                    false => self.rows.insert(at, row),
                };
            }
            None => {
                self.rows.remove(&at);
            }
        }
    }

    /// What this project sets that column to, if anything.
    pub fn get(&self, table: &str, key: &Key, column: &str) -> Option<&str> {
        self.row(table, key)?.columns.get(column).map(String::as_str)
    }

    /// Whether this project says anything at all about that row.
    pub fn touches(&self, table: &str, key: &Key) -> bool {
        self.rows.contains_key(&(table.to_string(), key.clone()))
    }

    /// **Give up every row in the tables one subject owns**, and answer how
    /// many went.
    ///
    /// The store holds every subject's rows in one map — a creature's and an
    /// item's, keyed by table — so *discard what I have changed* is a question
    /// about one subject's tables and never about the map. Dropping the whole
    /// store instead is a panel throwing away the panel next door's work, and
    /// counting what it dropped as its own.
    ///
    /// `wanted` is asked the table's name, which is the same test each writer
    /// already applies to decide whether a row is its to write — see
    /// `crate::creature::table_named` and `crate::item::table_named`.
    pub fn forget_where(&mut self, mut wanted: impl FnMut(&str) -> bool) -> usize {
        let before = self.rows.len();
        self.rows.retain(|(table, _), _| !wanted(table));
        before - self.rows.len()
    }

    /// How many rows are edited in the tables one subject owns.
    pub fn count_where(&self, mut wanted: impl FnMut(&str) -> bool) -> usize {
        self.rows.keys().filter(|(table, _)| wanted(table)).count()
    }

    /// How many rows are edited, over every table.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Every edited row: its table, its key, and what is to become of it.
    pub fn rows(&self) -> impl Iterator<Item = (&str, &Key, &RowEdit)> {
        self.rows
            .iter()
            .map(|((table, key), row)| (table.as_str(), key, row))
    }

    /// The file this is kept in, one edit a line.
    ///
    /// Tab-separated, with a header naming the fields, because the whole value
    /// of keeping this as text is that it can be read and corrected without the
    /// editor.
    ///
    /// **A row this project creates or removes carries one extra line of three
    /// fields** — its table, its key and the word — written before its columns.
    /// A row it merely edits carries none, so a file written before [`Life`]
    /// existed reads back as exactly what it was.
    pub fn to_text(&self, project: &str) -> String {
        let mut out = format!(
            "# {project} — the rows this project changes in the server's database.\n\
             # One edit a line: table, key, column, value. The value is already a\n\
             # SQL literal, so a name is quoted and escaped exactly as the statement\n\
             # will carry it. Written on every save; an edit taken back removes its\n\
             # own line. See crates/mangos/src/row.rs.\n\
             #\n\
             # A row this project creates or removes also carries a line of three\n\
             # fields — table, key, and insert or delete — before its columns.\n\
             #\n\
             # table\tkey\tcolumn\tvalue\n"
        );
        for ((table, key), row) in &self.rows {
            if row.life != Life::Update {
                out.push_str(&format!("{table}\t{}\t{}\n", key.text(), row.life.word()));
            }
            if let Some(from) = &row.from {
                out.push_str(&format!("{table}\t{}\t{}{}\n", key.text(), Self::FROM, from.text()));
            }
            for (column, value) in &row.columns {
                out.push_str(&format!("{table}\t{}\t{column}\t{value}\n", key.text()));
            }
        }
        out
    }

    /// …and read back. A line that is neither three fields nor four is skipped
    /// rather than failing the read: this file sits beside a project somebody
    /// may have edited by hand, and one bad line must not lose the rest.
    pub fn from_text(text: &str) -> Edits {
        Edits::read(text).0
    }

    /// [`Self::from_text`], with **every line it refused**, for a host to report.
    ///
    /// A line is refused when its key is not integers under plain column names
    /// ([`Key::parse`]) or its value is not one SQL literal
    /// ([`crate::sql::is_literal`]). Both go into statements as they stand, so
    /// a line that fails either is not read at all rather than read in part.
    pub fn read(text: &str) -> (Edits, Vec<String>) {
        let mut out = Edits::default();
        let mut refused = Vec::new();
        for line in text.lines() {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            match fields[..] {
                [table, key, word] => {
                    let Some(key) = Key::parse(key) else {
                        refused.push(line.to_string());
                        continue;
                    };
                    if table.trim().is_empty() {
                        continue;
                    }
                    if let Some(from) = word.trim().strip_prefix(Self::FROM) {
                        let Some(from) = Key::parse(from) else {
                            refused.push(line.to_string());
                            continue;
                        };
                        out.rows
                            .entry((table.trim().to_string(), key))
                            .or_default()
                            .from = Some(from);
                        continue;
                    }
                    let Some(life) = Life::named(word) else { continue };
                    out.set_life(table.trim(), &key, life);
                }
                [table, key, column, value] => {
                    let Some(key) = Key::parse(key) else {
                        refused.push(line.to_string());
                        continue;
                    };
                    if table.trim().is_empty() || column.trim().is_empty() {
                        continue;
                    }
                    if !crate::sql::is_literal(value.trim()) {
                        refused.push(line.to_string());
                        continue;
                    }
                    out.set(table.trim(), &key, column.trim(), Some(value.trim().to_string()));
                }
                _ => continue,
            }
        }
        out.read_old_moves();
        (out, refused)
    }

    /// **A store written before [`RowEdit::from`] existed held a move as a
    /// column**: `entry = <new>` on the row at the old key. Read as what it
    /// meant — the claim re-keyed to the new id with the old key as its
    /// origin — so the project keeps its move and every writer sees one form.
    ///
    /// A column is a key column when the row's own key names it, which needs no
    /// schema. On a row the project creates it was never written (the writer
    /// refused it), and it is dropped.
    fn read_old_moves(&mut self) {
        let old: Vec<(String, Key)> = self
            .rows
            .iter()
            .filter(|((_, key), row)| {
                key.0.iter().any(|(column, _)| row.columns.contains_key(column))
            })
            .map(|(at, _)| at.clone())
            .collect();
        for (table, key) in old {
            let Some(mut row) = self.rows.remove(&(table.clone(), key.clone())) else {
                continue;
            };
            let mut to = key.clone();
            for (column, value) in to.0.iter_mut() {
                if let Some(moved) = row.columns.remove(column.as_str()) {
                    *value = moved.trim().to_string();
                }
            }
            let stays = row.life == Life::Insert
                || to == key
                || self.rows.contains_key(&(table.clone(), to.clone()));
            if stays {
                if !row.is_empty() {
                    self.rows.insert((table, key), row);
                }
                continue;
            }
            row.from = Some(key);
            self.rows.insert((table, to), row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A discard is about one subject's tables and never about the store.**
    ///
    /// Every subject's rows are in one map, keyed by table. The creature
    /// panel's Discard emptied the whole of it for a round, so giving up a
    /// creature edit threw away every item edit beside it — and counted them as
    /// creatures in the sentence it then printed.
    #[test]
    fn a_discard_leaves_the_other_subjects_rows() {
        let mut edits = Edits::default();
        let spawn = Key::one("guid", 7);
        let item = Key::two(("entry", 19019), ("patch", 10));
        edits.set("creature", &spawn, "position_z", Some("41.5".into()));
        edits.set("creature_template", &Key::two(("entry", 68), ("patch", 0)), "name", Some("'Wolf'".into()));
        edits.set("item_template", &item, "Quality", Some("4".into()));
        assert_eq!(edits.len(), 3);

        let creature = |table: &str| crate::creature::table_named(table).is_some();
        assert_eq!(edits.count_where(creature), 2);
        assert_eq!(edits.forget_where(creature), 2);

        assert_eq!(edits.len(), 1);
        assert_eq!(edits.get("item_template", &item, "Quality"), Some("4"));
        assert!(!edits.touches("creature", &spawn));
        // …and a second discard of the same subject gives up nothing rather
        // than reporting the item row it does not own.
        assert_eq!(edits.forget_where(creature), 0);
    }

    #[test]
    fn a_key_of_two_columns_names_both_in_the_where() {
        let key = Key::two(("entry", 3296), ("patch", 0));
        assert_eq!(key.where_clause(), "`entry` = 3296 AND `patch` = 0");
        assert_eq!(key.text(), "entry=3296;patch=0");
        assert_eq!(Key::parse("entry=3296;patch=0"), Some(key));
    }

    /// **The key round-trips**, which is what stops a save from editing every
    /// patch of a creature instead of the one the server loaded.
    #[test]
    fn a_key_reads_back_as_itself() {
        for key in [Key::one("guid", 12), Key::two(("entry", 1), ("patch", 3))] {
            assert_eq!(Key::parse(&key.text()), Some(key));
        }
        assert_eq!(Key::parse(""), None);
        assert_eq!(Key::parse("nokey"), None);
    }

    /// **A key is integers under plain column names, and nothing else reads**:
    /// the value goes into a `WHERE` as it stands.
    #[test]
    fn a_key_that_is_not_integers_is_not_a_key() {
        for bad in [
            "entry=1 OR 1=1",
            "entry=1;patch=0 OR 1",
            "entry='1'",
            "entry=007",
            "entry=-0",
            "entry=",
            "`entry`=1",
            "en try=1",
        ] {
            assert_eq!(Key::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Key::parse("entry=-5"), Some(Key(vec![("entry".into(), "-5".into())])));
    }

    /// **Keys order by number.** A text order runs item 2000456 before item
    /// 852, which puts a move of one content patch ahead of an edit of another.
    #[test]
    fn keys_order_by_number() {
        let mut keys = vec![
            Key::two(("entry", 2_000_456), ("patch", 0)),
            Key::two(("entry", 852), ("patch", 10)),
            Key::two(("entry", 852), ("patch", 9)),
        ];
        keys.sort();
        assert_eq!(
            keys,
            vec![
                Key::two(("entry", 852), ("patch", 9)),
                Key::two(("entry", 852), ("patch", 10)),
                Key::two(("entry", 2_000_456), ("patch", 0)),
            ]
        );
    }

    /// **A value read out of the database is put back as the same text**,
    /// whatever the column holds.
    #[test]
    fn only_a_canonical_integer_goes_back_unquoted() {
        assert_eq!(literal("12"), "12");
        assert_eq!(literal("-3"), "-3");
        assert_eq!(literal("0"), "0");
        // Each of these parses as a float, and each would come back changed or
        // not come back at all.
        assert_eq!(literal("007"), "'007'");
        assert_eq!(literal("+5"), "'+5'");
        assert_eq!(literal("1e3"), "'1e3'");
        assert_eq!(literal("-0"), "'-0'");
        assert_eq!(literal("Infinity"), "'Infinity'");
        assert_eq!(literal("Nan"), "'Nan'");
        assert_eq!(literal("1.5"), "'1.5'");
        let mut now = std::collections::HashMap::new();
        now.insert("name".to_string(), Some("Infinity".to_string()));
        let change = Assignment { column: "name", value: "'Guard'".into() };
        assert_eq!(
            undo("creature_template", &Key::one("entry", 1), &[change], Some(&now)).as_deref(),
            Some("UPDATE `creature_template` SET `name` = 'Infinity' WHERE `entry` = 1;")
        );
    }

    /// **A store line whose key or value is not a literal is refused and
    /// named**, and the lines around it are still read.
    #[test]
    fn a_store_line_that_is_not_a_literal_is_refused() {
        let text = "item_template\tentry=852;patch=10\tname\t'Axe'\n\
                    item_template\tentry=852;patch=10\tbuy_price\t0; DROP TABLE item_template\n\
                    item_template\tentry=1 OR 1=1\tname\t'x'\n\
                    item_template\tentry=853;patch=10\tdelay\t2600\n";
        let (edits, refused) = Edits::read(text);
        assert_eq!(refused.len(), 2);
        assert_eq!(edits.len(), 2);
        let axe = Key::two(("entry", 852), ("patch", 10));
        assert_eq!(edits.get("item_template", &axe, "name"), Some("'Axe'"));
        assert_eq!(edits.get("item_template", &axe, "buy_price"), None);
    }

    #[test]
    fn an_update_names_the_key_and_the_columns() {
        let key = Key::one("guid", 42);
        let changes = [Assignment { column: "position_z", value: "41.5".into() }];
        assert_eq!(
            update("creature", &key, &changes).expect("one change"),
            "UPDATE `creature` SET `position_z` = 41.5 WHERE `guid` = 42;"
        );
        assert!(update("creature", &key, &[]).is_none());
    }

    /// An undo reads the values the row holds now and writes them back under
    /// the same key.
    #[test]
    fn an_undo_puts_back_what_the_row_held() {
        let mut now = std::collections::HashMap::new();
        now.insert("level_min".to_string(), Some("3".to_string()));
        now.insert("name".to_string(), Some("Kobold Vermin".to_string()));
        now.insert("subname".to_string(), None);
        let key = Key::two(("entry", 3296), ("patch", 0));
        let changes = [
            Assignment { column: "level_min", value: "10".into() },
            Assignment { column: "name", value: "'Nope'".into() },
            Assignment { column: "subname", value: "'Miner'".into() },
        ];
        let sql = undo("creature_template", &key, &changes, Some(&now)).expect("three columns");
        assert!(sql.contains("`level_min` = 3"), "{sql}");
        assert!(sql.contains("`name` = 'Kobold Vermin'"), "{sql}");
        // A column that is NULL goes back as NULL rather than as an empty
        // string, which is a different value to every query that reads it.
        assert!(sql.contains("`subname` = NULL"), "{sql}");
        assert!(sql.ends_with("WHERE `entry` = 3296 AND `patch` = 0;"), "{sql}");
    }

    /// A row that is not there has no undo. Nothing here creates a creature, so
    /// the alternative would be inventing a row this project never saw.
    #[test]
    fn a_missing_row_has_no_undo() {
        let changes = [Assignment { column: "level_min", value: "10".into() }];
        assert!(undo("creature_template", &Key::one("entry", 1), &changes, None).is_none());
    }

    /// **The store round-trips through its file**, which is what makes an edit
    /// survive closing the editor.
    #[test]
    fn the_edits_file_reads_back_what_it_wrote() {
        let key = Key::two(("entry", 3296), ("patch", 0));
        let spawn = Key::one("guid", 12345);
        let mut edits = Edits::default();
        edits.set("creature_template", &key, "level_min", Some("3".into()));
        edits.set("creature_template", &key, "name", Some("'Kobold Vermin'".into()));
        edits.set("creature", &spawn, "position_z", Some("41.5".into()));
        let back = Edits::from_text(&edits.to_text("a project"));
        assert_eq!(back, edits);
        assert_eq!(back.len(), 2);
        assert_eq!(back.get("creature_template", &key, "name"), Some("'Kobold Vermin'"));
    }

    /// **A value with a tab in it cannot be written**, because the file is
    /// tab-separated — and it cannot occur, because every text value is escaped
    /// by `sql::text` first and that turns a tab into `\t`.
    #[test]
    fn an_escaped_name_carries_no_tab_or_newline() {
        let value = crate::sql::text("two\twords\nand a line");
        assert!(!value.contains('\t') && !value.contains('\n'), "{value}");
        let key = Key::one("entry", 1);
        let mut edits = Edits::default();
        edits.set("creature_template", &key, "name", Some(value.clone()));
        assert_eq!(
            Edits::from_text(&edits.to_text("p")).get("creature_template", &key, "name"),
            Some(value.as_str())
        );
    }

    /// Clearing a column removes it, and clearing the last one removes the row.
    #[test]
    fn clearing_an_edit_removes_it_from_the_file() {
        let key = Key::one("guid", 1);
        let mut edits = Edits::default();
        edits.set("creature", &key, "position_x", Some("1".into()));
        edits.set("creature", &key, "position_y", Some("2".into()));
        edits.set("creature", &key, "position_x", None);
        assert_eq!(edits.len(), 1);
        edits.set("creature", &key, "position_y", None);
        assert!(edits.is_empty());
        assert!(!edits.touches("creature", &key));
    }

    /// **A removal carries no columns and must survive anyway.**
    ///
    /// The store drops a row with nothing left in it, which is how an edit is
    /// taken back — and a `DELETE` has nothing in it by construction. Without
    /// the `Life` in the emptiness test, marking a spawn for removal would be
    /// forgotten on the frame it was made.
    #[test]
    fn a_row_marked_for_removal_is_not_empty() {
        let key = Key::one("guid", 19272);
        let mut edits = Edits::default();
        edits.set_life("creature", &key, Life::Delete);
        assert_eq!(edits.len(), 1);
        assert!(edits.touches("creature", &key));
        assert_eq!(edits.life("creature", &key), Life::Delete);
        // …and taking it back is saying the row is an ordinary one again.
        edits.set_life("creature", &key, Life::Update);
        assert!(edits.is_empty());
    }

    /// A row this project creates keeps its columns, and clearing the last one
    /// does **not** remove it — the claim is that the row should exist.
    #[test]
    fn clearing_a_created_rows_last_column_leaves_the_row() {
        let key = Key::one("guid", 10_000_001);
        let mut edits = Edits::default();
        edits.set_life("creature", &key, Life::Insert);
        edits.set("creature", &key, "map", Some("0".into()));
        edits.set("creature", &key, "map", None);
        assert_eq!(edits.life("creature", &key), Life::Insert);
        assert!(!edits.is_empty());
    }

    /// **The life round-trips through the file**, and a file written before it
    /// existed reads back as exactly what it was.
    #[test]
    fn the_edits_file_carries_what_becomes_of_a_row() {
        let new = Key::one("guid", 10_000_001);
        let gone = Key::one("guid", 19272);
        let edited = Key::two(("entry", 68), ("patch", 0));
        let mut edits = Edits::default();
        edits.set_life("creature", &new, Life::Insert);
        edits.set("creature", &new, "id", Some("68".into()));
        edits.set_life("creature", &gone, Life::Delete);
        edits.set("creature_template", &edited, "level_min", Some("57".into()));

        let text = edits.to_text("a project");
        assert!(text.contains("creature\tguid=10000001\tinsert\n"), "{text}");
        assert!(text.contains("creature\tguid=19272\tdelete\n"), "{text}");
        // The row this project merely edits carries no life line at all.
        assert!(!text.contains("creature_template\tentry=68;patch=0\tupdate"), "{text}");
        assert_eq!(Edits::from_text(&text), edits);
    }

    /// **A row's whole edit goes on the undo stack as one line**, which is what
    /// makes creating and removing one undoable at all: neither is a column.
    #[test]
    fn a_row_edit_round_trips_through_its_line() {
        let key = Key::one("guid", 10_000_001);
        let mut edits = Edits::default();
        edits.set_life("creature", &key, Life::Insert);
        edits.set("creature", &key, "id", Some("68".into()));
        edits.set("creature", &key, "name", Some("'two words'".into()));
        let line = edits.row_line("creature", &key).expect("a created row");

        let mut back = Edits::default();
        back.set_row_line("creature", &key, Some(&line));
        assert_eq!(back, edits);
        // …and `None` is the project saying nothing about the row, which is
        // what undoing the creation is.
        back.set_row_line("creature", &key, None);
        assert!(back.is_empty());
    }

    /// An `INSERT` names the key's own columns first and never names one twice.
    #[test]
    fn an_insert_names_the_key_and_then_the_columns() {
        let key = Key::one("guid", 10_000_001);
        let changes = [
            Assignment { column: "id", value: "68".into() },
            // A key column in the changes, which a hand-edited file can hold.
            Assignment { column: "guid", value: "5".into() },
            Assignment { column: "position_x", value: "-8817.5".into() },
        ];
        assert_eq!(
            insert("creature", &key, &changes).expect("three columns"),
            "INSERT INTO `creature` (`guid`, `id`, `position_x`) \
             VALUES (10000001, 68, -8817.5);"
        );
    }

    /// **A row read out of the database is the `INSERT` that puts it back**,
    /// which is the undo of a removal. Its columns are sorted, so two runs
    /// write the same file.
    #[test]
    fn a_row_comes_back_as_the_insert_that_restores_it() {
        let mut row = std::collections::HashMap::new();
        row.insert("guid".to_string(), Some("19272".to_string()));
        row.insert("id".to_string(), Some("68".to_string()));
        row.insert("auras".to_string(), None);
        row.insert("name".to_string(), Some("Kobold Vermin".to_string()));
        assert_eq!(
            insert_from_row("creature", &row).expect("four columns"),
            "INSERT INTO `creature` (`auras`, `guid`, `id`, `name`) \
             VALUES (NULL, 19272, 68, 'Kobold Vermin');"
        );
        assert!(insert_from_row("creature", &std::collections::HashMap::new()).is_none());
    }

    #[test]
    fn a_delete_names_the_whole_key() {
        assert_eq!(
            delete("creature", &Key::one("guid", 19272)),
            "DELETE FROM `creature` WHERE `guid` = 19272;"
        );
    }

    fn item(entry: u64) -> Key {
        Key::two(("entry", entry), ("patch", 10))
    }

    /// **A row whose id changes is one claim**, stored where the project puts
    /// it and naming where the database has it. A second move keeps the first
    /// origin, a column edit lands on the same claim, and the file reads back.
    #[test]
    fn a_moved_row_is_one_claim_with_an_origin() {
        let mut edits = Edits::default();
        edits.set("item_template", &item(852), "quality", Some("3".into()));
        assert!(edits.rekey("item_template", &item(852), &item(2_000_456), None));
        assert!(!edits.touches("item_template", &item(852)));
        let row = edits.row("item_template", &item(2_000_456)).expect("the claim followed");
        assert_eq!(row.from, Some(item(852)));
        assert_eq!(row.columns.get("quality").map(String::as_str), Some("3"));

        assert!(edits.rekey("item_template", &item(2_000_456), &item(2_000_457), None));
        let row = edits.row("item_template", &item(2_000_457)).expect("moved again");
        assert_eq!(row.from, Some(item(852)), "still from where the database has it");
        assert_eq!(edits.moved_to("item_template", &item(852)), Some(&item(2_000_457)));

        let back = Edits::from_text(&edits.to_text("test"));
        assert_eq!(back, edits);
        let line = row.to_line();
        assert_eq!(&RowEdit::from_line(&line), row);
    }

    /// Moving a row back to where the database has it is no move, and a claim
    /// that then says nothing is dropped.
    #[test]
    fn a_move_back_onto_the_origin_is_no_move() {
        let mut edits = Edits::default();
        assert!(edits.rekey("item_template", &item(852), &item(900), None));
        assert_eq!(edits.len(), 1);
        assert!(edits.rekey("item_template", &item(900), &item(852), None));
        assert!(edits.is_empty());
    }

    /// A row the project creates was nowhere, so re-keying it records no
    /// origin; and a key that is already claimed is refused.
    #[test]
    fn a_created_row_is_re_keyed_with_no_origin() {
        let mut edits = Edits::default();
        edits.set_life("item_template", &item(2_000_000), Life::Insert);
        edits.set("item_template", &item(5), "quality", Some("1".into()));
        assert!(!edits.rekey("item_template", &item(2_000_000), &item(5), None));
        assert!(edits.rekey("item_template", &item(2_000_000), &item(505_056), None));
        let row = edits.row("item_template", &item(505_056)).expect("re-keyed");
        assert_eq!(row.life, Life::Insert);
        assert_eq!(row.from, None);
    }

    /// **A store written before origins existed held a move as a key column.**
    /// It reads back as the move it meant.
    #[test]
    fn an_old_move_column_is_read_as_a_move() {
        let text = "item_template\tentry=852;patch=10\tentry\t2000456\n\
                    item_template\tentry=852;patch=10\tquality\t3\n";
        let edits = Edits::from_text(text);
        assert_eq!(edits.len(), 1);
        let row = edits.row("item_template", &item(2_000_456)).expect("re-keyed on read");
        assert_eq!(row.from, Some(item(852)));
        assert!(!row.columns.contains_key("entry"));
        assert_eq!(row.columns.get("quality").map(String::as_str), Some("3"));
    }

    /// The statements of a move, and of its undo, which is the same list taken
    /// backwards with the ids exchanged.
    #[test]
    fn a_move_takes_every_version_and_every_reference() {
        let references = [
            Reference::new("npc_vendor", "item"),
            Reference::only("creature_loot_template", "item", "`mincountOrRef` > 0"),
            Reference::negated("quest_template", "PrevQuestId"),
        ];
        let changes = [Assignment { column: "quality", value: "3".into() }];
        let sql = move_statements("item_template", &item(852), &item(900), &changes, &references);
        assert_eq!(
            sql,
            vec![
                "UPDATE `item_template` SET `entry` = 900, `quality` = 3 WHERE `entry` = 852 AND `patch` = 10;",
                "UPDATE `item_template` SET `entry` = 900 WHERE `entry` = 852;",
                "UPDATE `npc_vendor` SET `item` = 900 WHERE `item` = 852;",
                "UPDATE `creature_loot_template` SET `item` = 900 WHERE `item` = 852 AND `mincountOrRef` > 0;",
                "UPDATE `quest_template` SET `PrevQuestId` = -900 WHERE `PrevQuestId` = -852;",
            ]
        );
        let mut now = std::collections::HashMap::new();
        now.insert("quality".to_string(), Some("1".to_string()));
        let back = undo_move_statements(
            "item_template", &item(852), &item(900), &changes, &references, Some(&now),
        )
        .expect("the row is there");
        assert_eq!(
            back,
            vec![
                "UPDATE `quest_template` SET `PrevQuestId` = -852 WHERE `PrevQuestId` = -900;",
                "UPDATE `creature_loot_template` SET `item` = 852 WHERE `item` = 900 AND `mincountOrRef` > 0;",
                "UPDATE `npc_vendor` SET `item` = 852 WHERE `item` = 900;",
                "UPDATE `item_template` SET `entry` = 852, `quality` = 1 WHERE `entry` = 900 AND `patch` = 10;",
                "UPDATE `item_template` SET `entry` = 852 WHERE `entry` = 900;",
            ]
        );
        // A table keyed by the id alone has no other versions to take.
        let spawn = move_statements("creature", &Key::one("guid", 7), &Key::one("guid", 9), &[], &[]);
        assert_eq!(spawn, vec!["UPDATE `creature` SET `guid` = 9 WHERE `guid` = 7;"]);
    }

    /// A damaged line costs itself and nothing else.
    #[test]
    fn a_bad_line_does_not_lose_the_good_ones() {
        let text = "# a header\n\
                    creature\tguid=1\tposition_x\t1\n\
                    this line has no tabs at all\n\
                    creature\tnot-a-key\tposition_y\t2\n\
                    creature\tguid=1\tposition_y\t2\n";
        let edits = Edits::from_text(text);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits.get("creature", &Key::one("guid", 1), "position_y"), Some("2"));
    }
}
