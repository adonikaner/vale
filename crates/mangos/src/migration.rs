//! A vmangos migration: the form the server's own `sql/migrations/` files
//! take, the stamp that names one, and the record a project keeps of what
//! its last migration said, so the next one carries the change since.
//!
//! ## The form
//!
//! `sql/make_migration.py` in the vmangos checkout writes this, and every one
//! of the 958 files under `sql/migrations/` is it:
//!
//! ```text
//! DROP PROCEDURE IF EXISTS add_migration;
//! DELIMITER ??
//! CREATE PROCEDURE `add_migration`()
//! BEGIN
//! DECLARE v INT DEFAULT 1;
//! SET v = (SELECT COUNT(*) FROM `migrations` WHERE `id`='20260922120000');
//! IF v = 0 THEN
//! INSERT INTO `migrations` VALUES ('20260922120000');
//! -- Add your query below.
//! …
//! -- End of migration.
//! END IF;
//! END??
//! DELIMITER ;
//! CALL add_migration();
//! DROP PROCEDURE IF EXISTS add_migration;
//! ```
//!
//! The id is the UTC time the file was made, `YYYYmmddHHMMSS`, and the file is
//! `<id>_world.sql`. The procedure runs the body once: a second run finds the
//! id in `migrations` and does nothing. `Database::CheckRequiredMigrations`
//! (`src/shared/Database/Database.cpp:562`) reads that table at startup and
//! refuses to start over an id the build lists and the table lacks; an id the
//! table holds and the build does not list — which is every migration a
//! project writes — is a warning, `has the following extra migrations`, and
//! the server starts.
//!
//! `DELIMITER` is the `mysql` command-line client's and not the server's, so
//! the file is applied with `mysql <database> < <file>` and not over the
//! connection [`crate::conn`] holds. The delimiter is chosen from
//! [`DELIMITERS`] as the first that no statement in the body contains: a quest
//! text holding `??` would otherwise end the procedure in the middle of a
//! string.
//!
//! ## What a project's migration holds: the whole of the project
//!
//! A patch is distributed, and the server it lands on may or may not hold
//! the project's earlier patches. So every migration is **self-contained**:
//! every row the project changes now, and, for every row an earlier migration
//! of the project changed and the project has since stopped changing, the
//! statements that put it back. Each statement is idempotent — an `UPDATE` to
//! absolute values, a `DELETE` and an `INSERT` — so running a later patch's
//! migration over an earlier one's leaves the rows the later one says, and
//! running it on a server that never saw the earlier one does the same.
//! [`Released`] is the record: every row any migration of the project has
//! written, with its statements and, where the project knew them, the
//! statements that put it back; [`contents`] is what the next migration
//! holds against it. Patches are applied in order; each is guarded by its own
//! stamp and applied once.
//!
//! **The put-back is known only if the project was applied to a database.**
//! A subject's revert file holds what each row held before the project first
//! wrote it, read from the database immediately before the write; a row the
//! project never applied has no such record. The record keeps the put-back
//! from the moment it is known, so a row dropped two patches after it was
//! applied is still put back. One dropped with no put-back known is named in
//! the migration as a comment and in [`Contents::unknown`], and the publish
//! reports it: the release database then keeps the row as the earlier
//! migration left it until somebody writes the statement by hand.
//!
//! The record is `publish\released.txt` under the project, which is not game
//! data and is not packed into the archive.

use crate::row::Key;
use std::collections::BTreeMap;

/// The server's own table of applied migration ids.
pub const TABLE: &str = "migrations";

/// Where the project keeps its record — see [`Released`].
pub const RELEASED_VPATH: &str = "publish\\released.txt";

/// …and where the migration files go, under the project.
pub const MIGRATIONS_DIR: &str = "publish\\migrations";

/// One row a migration changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The subject it belongs to, by the editor's own word: `creatures`,
    /// `spells`. Carried so a migration's body can be grouped and so a
    /// dropped row's comment can say where it came from.
    pub subject: String,
    pub table: String,
    pub key: Key,
    /// The statements that make the row what the project says.
    pub statements: Vec<String>,
    /// …and the ones that put it back to what it held before the project
    /// first wrote it, when the project knows them. See the module comment.
    pub undo: Option<Vec<String>>,
}

impl Entry {
    fn same_row(&self, other: &Entry) -> bool {
        self.table == other.table && self.key == other.key
    }
}

/// **What the last migration a project wrote said**: its id, and every row
/// it changed. Empty for a project that has written none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Released {
    /// The id of the last migration, or `None`.
    pub id: Option<String>,
    /// Every row the project's migrations so far leave changed, in the order
    /// the last migration wrote them.
    pub entries: Vec<Entry>,
}

/// **What a migration holds**: the whole of the project, and the put-backs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contents {
    /// Every row the project changes now, in the order they were given.
    pub changed: Vec<Entry>,
    /// Rows an earlier migration changed and the project no longer does, with
    /// their put-back known. Newest in the record first, which is the order
    /// that undoes them.
    pub dropped: Vec<Entry>,
    /// …and the ones dropped with no put-back known. Named in the migration
    /// as comments and nowhere else.
    pub unknown: Vec<Entry>,
}

impl Contents {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.dropped.is_empty() && self.unknown.is_empty()
    }

    /// How many statements the migration runs.
    pub fn statements(&self) -> usize {
        self.changed
            .iter()
            .map(|entry| entry.statements.len())
            .chain(self.dropped.iter().map(|entry| entry.undo.as_ref().map_or(0, Vec::len)))
            .sum()
    }

    /// One line for a status bar: `3 rows, 1 put back, 1 unknown`.
    pub fn line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.changed.is_empty() {
            parts.push(format!("{} row(s)", self.changed.len()));
        }
        if !self.dropped.is_empty() {
            parts.push(format!("{} put back", self.dropped.len()));
        }
        if !self.unknown.is_empty() {
            parts.push(format!(
                "{} dropped with no put-back known",
                self.unknown.len()
            ));
        }
        match parts.is_empty() {
            true => "no rows".to_string(),
            false => parts.join(", "),
        }
    }
}

/// **What the next migration holds**, given the record.
///
/// `now` is every row the project changes, in the order its migration should
/// run them — the subjects in the order they stand in the database, each
/// subject's rows in its plan's order. Every one of them is in the
/// migration. A row's put-back in `now` is taken as given; one that is
/// `None` falls back to the record's, which is how a put-back read at one
/// patch survives to a later one. A row the record has and `now` does not is
/// put back, or named when its put-back was never known.
pub fn contents(released: &Released, now: &[Entry]) -> Contents {
    let mut out = Contents::default();
    for entry in now {
        let had = released.entries.iter().find(|had| had.same_row(entry));
        let undo = entry
            .undo
            .clone()
            .or_else(|| had.and_then(|had| had.undo.clone()));
        out.changed.push(Entry {
            undo,
            ..entry.clone()
        });
    }
    for had in released.entries.iter().rev() {
        if now.iter().any(|entry| entry.same_row(had)) {
            continue;
        }
        match &had.undo {
            Some(undo) if !undo.is_empty() => out.dropped.push(had.clone()),
            _ => out.unknown.push(had.clone()),
        }
    }
    out
}

/// **The record after a migration of `contents` is written**: every row the
/// project changes now, and every row it dropped, kept so that the next
/// migration puts it back as well.
pub fn released_after(id: &str, contents: &Contents) -> Released {
    let mut entries = contents.changed.clone();
    entries.extend(contents.dropped.iter().rev().cloned());
    entries.extend(contents.unknown.iter().rev().cloned());
    Released {
        id: Some(id.to_string()),
        entries,
    }
}

/// **A migration id**: the UTC time now as `YYYYmmddHHMMSS`, and later than
/// `after` when that is given — two publishes inside one second would
/// otherwise write one id twice, and the second file would never run.
pub fn stamp(after: Option<&str>) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut id = stamp_of(now);
    if let Some(after) = after {
        let mut seconds = now;
        while id.as_str() <= after {
            seconds += 1;
            id = stamp_of(seconds);
        }
    }
    id
}

/// The id for a moment, in seconds since the epoch.
pub fn stamp_of(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = civil(seconds);
    format!("{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}")
}

/// Whether text is an id: fourteen digits.
pub fn is_id(text: &str) -> bool {
    text.len() == 14 && text.bytes().all(|b| b.is_ascii_digit())
}

/// `<id>_world.sql`.
pub fn file_name(id: &str) -> String {
    format!("{id}_world.sql")
}

/// The delimiters tried, in order, for the procedure. `??` is vmangos' own.
pub const DELIMITERS: [&str; 4] = ["??", "$$", "//", "|||"];

/// **The file**, in the form at the head of this module.
///
/// `project` names the project in the header. Sections are written in the
/// order [`Contents`] holds them: rows put back first, then the rows changed,
/// then a comment per row dropped with no put-back known.
pub fn text(id: &str, project: &str, delta: &Contents) -> String {
    let mut body = String::new();
    if !delta.dropped.is_empty() {
        body.push_str("-- Rows an earlier migration of this project changed and it no longer does,\n");
        body.push_str("-- put back to what they held before the project first wrote them.\n");
        for entry in &delta.dropped {
            body.push_str(&format!("-- {} {} {}\n", entry.subject, entry.table, entry.key.text()));
            for statement in entry.undo.iter().flatten() {
                body.push_str(statement);
                body.push('\n');
            }
        }
        body.push('\n');
    }
    let mut last_subject: Option<&str> = None;
    for entry in &delta.changed {
        if last_subject != Some(entry.subject.as_str()) {
            body.push_str(&format!("-- {}\n", entry.subject));
            last_subject = Some(entry.subject.as_str());
        }
        body.push_str(&format!("-- {} {}\n", entry.table, entry.key.text()));
        for statement in &entry.statements {
            body.push_str(statement);
            body.push('\n');
        }
    }
    if !delta.unknown.is_empty() {
        body.push('\n');
        body.push_str("-- Rows an earlier migration of this project changed and it no longer does,\n");
        body.push_str("-- whose earlier values this project never read. They are left as the earlier\n");
        body.push_str("-- migration wrote them; put them back by hand.\n");
        for entry in &delta.unknown {
            body.push_str(&format!("-- {} {} {}\n", entry.subject, entry.table, entry.key.text()));
        }
    }
    let delimiter = DELIMITERS
        .iter()
        .copied()
        .find(|d| !body.contains(d))
        .unwrap_or("??");
    format!(
        "-- {project}: migration {id}, written by the world editor.\n\
         -- Apply with the mysql client: mysql <world database> < {file}\n\
         DROP PROCEDURE IF EXISTS add_migration;\n\
         DELIMITER {delimiter}\n\
         CREATE PROCEDURE `add_migration`()\n\
         BEGIN\n\
         DECLARE v INT DEFAULT 1;\n\
         SET v = (SELECT COUNT(*) FROM `{table}` WHERE `id`='{id}');\n\
         IF v = 0 THEN\n\
         INSERT INTO `{table}` VALUES ('{id}');\n\
         -- Add your query below.\n\
         \n\
         {body}\n\
         -- End of migration.\n\
         END IF;\n\
         END{delimiter}\n\
         DELIMITER ;\n\
         CALL add_migration();\n\
         DROP PROCEDURE IF EXISTS add_migration;\n",
        file = file_name(id),
        table = TABLE,
    )
}

impl Released {
    /// The record as text — see [`Self::from_text`] for the form.
    pub fn to_text(&self, project: &str) -> String {
        let mut out = format!(
            "# {project} — what this project's last migration said. Written by a\n\
             # publish; the next migration is the change since. One row per block:\n\
             #   row <subject> <table> <key>\n\
             #   do <statement>            …one per statement the migration ran\n\
             #   undo <statement>          …one per statement that puts it back, if known\n"
        );
        if let Some(id) = &self.id {
            out.push_str(&format!("migration {id}\n"));
        }
        for entry in &self.entries {
            out.push_str(&format!(
                "row {} {} {}\n",
                entry.subject,
                entry.table,
                entry.key.text()
            ));
            for statement in &entry.statements {
                out.push_str("do ");
                out.push_str(&one_line(statement));
                out.push('\n');
            }
            for statement in entry.undo.iter().flatten() {
                out.push_str("undo ");
                out.push_str(&one_line(statement));
                out.push('\n');
            }
        }
        out
    }

    /// …and back. A block whose `row` line will not parse is left out; a
    /// `do` or `undo` line before any block is ignored.
    pub fn from_text(text: &str) -> Released {
        let mut out = Released::default();
        let mut skipping = false;
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(id) = line.strip_prefix("migration ") {
                let id = id.trim();
                if is_id(id) {
                    out.id = Some(id.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("row ") {
                let parts: Vec<&str> = rest.splitn(3, ' ').collect();
                let parsed = match parts[..] {
                    [subject, table, key] => Key::parse(key.trim()).map(|key| Entry {
                        subject: subject.to_string(),
                        table: table.to_string(),
                        key,
                        statements: Vec::new(),
                        undo: None,
                    }),
                    _ => None,
                };
                match parsed {
                    Some(entry) => {
                        out.entries.push(entry);
                        skipping = false;
                    }
                    None => skipping = true,
                }
                continue;
            }
            if skipping {
                continue;
            }
            let Some(entry) = out.entries.last_mut() else {
                continue;
            };
            if let Some(statement) = line.strip_prefix("do ") {
                entry.statements.push(from_one_line(statement));
            } else if let Some(statement) = line.strip_prefix("undo ") {
                entry.undo.get_or_insert_with(Vec::new).push(from_one_line(statement));
            }
        }
        out
    }

    /// The rows by subject, for a panel that counts them.
    pub fn by_subject(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for entry in &self.entries {
            *out.entry(entry.subject.clone()).or_default() += 1;
        }
        out
    }
}

/// A statement on one line of the record: every newline written as `\n`.
/// The escaping is exactly reversible, since a backslash in a statement is
/// already doubled by `sql::text` and a bare one never reaches here from this
/// crate — but one is escaped too, so a hand-written line survives.
fn one_line(statement: &str) -> String {
    let mut out = String::with_capacity(statement.len());
    for ch in statement.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

fn from_one_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Seconds since the epoch to a UTC civil date and time. Howard Hinnant's
/// `civil_from_days`, which is exact for every date the Gregorian calendar
/// has.
fn civil(seconds: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y as u64,
        m as u64,
        d as u64,
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(subject: &str, table: &str, id: u64, statement: &str, undo: Option<&str>) -> Entry {
        Entry {
            subject: subject.into(),
            table: table.into(),
            key: Key::one("entry", id),
            statements: vec![statement.into()],
            undo: undo.map(|s| vec![s.into()]),
        }
    }

    /// The stamp is UTC to the second, fourteen digits, and later than the
    /// one before it.
    #[test]
    fn a_stamp_is_the_utc_time_and_later_than_the_last() {
        assert_eq!(stamp_of(0), "19700101000000");
        assert_eq!(stamp_of(1_759_792_562), "20251006231602");
        assert!(is_id(&stamp(None)));
        let far = "99991231235959";
        let _ = far;
        let last = stamp(None);
        let next = stamp(Some(&last));
        assert!(next > last, "{next} after {last}");
        assert!(is_id("20260922120000"));
        assert!(!is_id("2026092212000"));
        assert!(!is_id("2026-09-22 12:00"));
        assert_eq!(file_name("20260922120000"), "20260922120000_world.sql");
    }

    /// **The first migration is the whole plan**, and the record after it
    /// holds every row.
    #[test]
    fn the_first_migration_is_everything() {
        let now = vec![
            entry("creatures", "creature", 1, "UPDATE a;", None),
            entry("items", "item_template", 2, "UPDATE b;", Some("UPDATE b0;")),
        ];
        let first = contents(&Released::default(), &now);
        assert_eq!(first.changed, now);
        assert!(first.dropped.is_empty() && first.unknown.is_empty());
        let after = released_after("20260922120000", &first);
        assert_eq!(after.id.as_deref(), Some("20260922120000"));
        assert_eq!(after.entries, now);
    }

    /// **The second is the whole plan again, and puts back what was
    /// dropped**: every row the project changes now, a dropped row put back,
    /// and a dropped row whose put-back was never known named rather than
    /// invented. The record keeps the dropped rows, so the third puts them
    /// back too.
    #[test]
    fn the_second_migration_is_whole_and_puts_back_the_dropped() {
        let first = vec![
            entry("creatures", "creature", 1, "UPDATE a;", Some("UPDATE a0;")),
            entry("creatures", "creature", 2, "UPDATE b;", None),
            entry("items", "item_template", 3, "UPDATE c;", Some("UPDATE c0;")),
        ];
        let released = released_after("20260922120000", &contents(&Released::default(), &first));
        let now = vec![
            entry("creatures", "creature", 1, "UPDATE a2;", None),
            entry("items", "item_template", 4, "UPDATE d;", None),
        ];
        let next = contents(&released, &now);
        assert_eq!(next.changed.len(), 2, "every row the project changes now");
        assert_eq!(next.changed[0].statements, vec!["UPDATE a2;".to_string()]);
        // The put-back read at the first patch survives to the second.
        assert_eq!(next.changed[0].undo, Some(vec!["UPDATE a0;".to_string()]));
        assert_eq!(next.dropped.len(), 1);
        assert_eq!(next.dropped[0].key, Key::one("entry", 3));
        assert_eq!(next.unknown.len(), 1);
        assert_eq!(next.unknown[0].key, Key::one("entry", 2));
        assert_eq!(next.statements(), 3);
        assert_eq!(next.line(), "2 row(s), 1 put back, 1 dropped with no put-back known");
        let after = released_after("20260922120001", &next);
        assert_eq!(after.entries.len(), 4, "the dropped rows stay in the record");
        assert_eq!(after.entries[0].undo, Some(vec!["UPDATE a0;".to_string()]));
        let third = contents(&after, &now);
        assert_eq!(third.dropped.len(), 1, "and are put back by the next patch as well");
    }

    /// A project with no rows and nothing ever released holds nothing.
    #[test]
    fn nothing_released_and_nothing_changed_is_empty() {
        assert!(contents(&Released::default(), &[]).is_empty());
        let now = vec![entry("loot", "creature_loot_template", 1, "UPDATE a;", None)];
        let released = released_after("20260922120000", &contents(&Released::default(), &now));
        assert_eq!(contents(&released, &now).changed.len(), 1, "a patch is whole");
    }

    /// The file is vmangos' own form, with the put-backs first, and the
    /// delimiter steps aside for a body that contains it.
    #[test]
    fn the_file_is_vmangos_own_form() {
        let mut d = Contents::default();
        d.dropped.push(entry("items", "item_template", 3, "UPDATE c;", Some("UPDATE c0;")));
        d.changed.push(entry("creatures", "creature", 1, "UPDATE a;", None));
        d.unknown.push(entry("creatures", "creature", 2, "UPDATE b;", None));
        let file = text("20260922120000", "goldshire", &d);
        assert!(file.starts_with("-- goldshire: migration 20260922120000"));
        assert!(file.contains("DELIMITER ??\n"));
        assert!(file.contains("WHERE `id`='20260922120000'"));
        assert!(file.contains("INSERT INTO `migrations` VALUES ('20260922120000');"));
        assert!(file.contains("END??\nDELIMITER ;\nCALL add_migration();"));
        let put_back = file.find("UPDATE c0;").unwrap();
        let changed = file.find("UPDATE a;").unwrap();
        assert!(put_back < changed, "put-backs first");
        assert!(file.contains("-- creatures creature entry=2\n"));
        assert!(!file.contains("UPDATE b;"), "an unknown row runs nothing");

        let mut d = Contents::default();
        d.changed.push(entry("quests", "quest_template", 1, "UPDATE `quest_template` SET `Title` = 'What??' WHERE `entry` = 1;", None));
        let file = text("20260922120000", "p", &d);
        assert!(file.contains("DELIMITER $$\n"));
        assert!(file.contains("END$$\n"));
    }

    /// The record reads back what it wrote, statement for statement, with a
    /// newline inside a literal intact.
    #[test]
    fn the_record_round_trips() {
        let released = Released {
            id: Some("20260922120000".into()),
            entries: vec![
                entry("creatures", "creature", 1, "UPDATE `creature` SET `name` = 'a\nb' WHERE `guid` = 1;", Some("UPDATE a0;")),
                Entry {
                    subject: "creatures".into(),
                    table: "creature_movement".into(),
                    key: Key::one("id", 5),
                    statements: vec!["DELETE …;".into(), "INSERT …;".into()],
                    undo: None,
                },
            ],
        };
        let text = released.to_text("p");
        assert_eq!(Released::from_text(&text), released);
        assert_eq!(released.by_subject().get("creatures"), Some(&2));
        // A block whose row line will not parse is left out, with its lines.
        let broken = format!("{text}row bad\ndo DROP TABLE x;\n");
        assert_eq!(Released::from_text(&broken), released);
        assert_eq!(Released::from_text(""), Released::default());
    }
}
