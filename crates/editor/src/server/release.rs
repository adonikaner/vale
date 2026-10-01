//! What a patch hands the server: one migration holding the rows, and the
//! DBCs the server reads as files.
//!
//! ## Why a patch carries a migration and not the `sql\` files
//!
//! `sql\*.sql` is the project's current state, rewritten on every save, with
//! no order between subjects and no record of having been applied. It is
//! useful to read and unsuitable to distribute. What vmangos applies to a
//! database is a migration: one stamped file, run once, recorded in the
//! `migrations` table, and never run again. See `vale_mangos::migration`,
//! which defines the form and the rule.
//!
//! A patch's migration holds all of the project's rows, in the order the
//! subjects stand in the database (`stack::Subject::ORDER`, then the
//! client-table rows: spells and flight nodes). It also puts back the rows an
//! earlier patch changed that the project no longer changes. Each patch is
//! self-contained: it applies correctly to a server with or without the
//! patches before it.
//!
//! ## Where a put-back comes from
//!
//! The statements that put a row back are read from the subject's revert
//! file, which an Apply wrote from the database immediately before it first
//! wrote the row. They are kept in `publish\released.txt` from the moment
//! they are known, so a row dropped after a later Put back, which removes
//! the revert file, is still put back. A row that was never applied to a
//! database has no put-back anywhere; when it is dropped, the migration
//! names it in a comment and the publish reports it.
//!
//! Apply the project to the dev database before publishing for this reason.
//! With Apply on save on, every save has already done so. The migration then
//! agrees with what that database holds, and the release database (the same
//! baseline plus this project's patches) ends up the same.
//!
//! ## What this module does not do
//!
//! * The migration is not applied by the editor. `DELIMITER` is the `mysql`
//!   client's, so the file is for that client: `mysql mangos < <file>`. The
//!   dev database already holds the rows through Apply.
//! * Two projects' migrations to one row are not reconciled: each project
//!   knows its own patches and nothing else's.
//! * A reserved id range per project is not written. Two projects that both
//!   create a spawn at 10,000,000 collide at the second patch.

use super::stack::Subject;
use super::{behaviour, creatures, gameobjects, items, loot, quests, rows, services};
use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::project::Project;
use vale_mangos::migration::{self, Contents, Entry, Released};
use vale_mangos::row::Key;
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// Every row the project currently changes, as migration entries, in the
/// order a migration runs them. Each carries its put-back when the subject's
/// revert file has one. Client-table rows are labelled "spells" or "flight
/// nodes" by their table.
pub fn entries(session: &EditSession, assets: &GameAssets) -> Result<Vec<Entry>, String> {
    let project = &session.project;
    let mut out: Vec<Entry> = Vec::new();
    for subject in Subject::ORDER {
        let undo = creatures::Undo::open_at(project, revert_vpath(subject))?;
        let find = |table: &str, key: &Key| -> Option<Vec<String>> {
            let statements: Vec<String> = undo
                .entries
                .iter()
                .filter(|(had_table, had_key, _)| had_table == table && had_key == key)
                .flat_map(|(_, _, statements)| statements.iter().cloned())
                .collect();
            match statements.is_empty() {
                true => None,
                false => Some(statements),
            }
        };
        let name = subject.name().to_string();
        let push = |out: &mut Vec<Entry>, table: &str, key: Key, statements: Vec<String>| {
            if statements.is_empty() {
                return;
            }
            out.push(Entry {
                subject: name.clone(),
                table: table.to_string(),
                undo: find(table, &key),
                key,
                statements,
            });
        };
        match subject {
            Subject::Creatures => {
                let plan = creatures::plan(session);
                for row in &plan.rows {
                    push(&mut out, row.table, row.key.clone(), row.statements());
                }
                for path in &plan.paths {
                    push(
                        &mut out,
                        path.which.table(),
                        path.key(),
                        vale_mangos::path::statements(path),
                    );
                }
            }
            Subject::GameObjects => {
                for row in &gameobjects::plan(session).rows {
                    push(&mut out, row.table, row.key.clone(), row.statements());
                }
            }
            Subject::Items => {
                for row in &items::plan(session).rows {
                    push(&mut out, row.table, row.key.clone(), row.statements());
                }
            }
            Subject::Quests => {
                for row in &quests::plan(session).rows {
                    push(&mut out, row.table, row.key.clone(), row.statements());
                }
            }
            Subject::Loot => {
                for row in loot::plan(session).ordered() {
                    push(&mut out, row.table, row.key.clone(), row.statements());
                }
            }
            Subject::Services => {
                for row in services::plan(session).ordered() {
                    push(&mut out, row.table, row.key.clone(), row.statements());
                }
            }
            Subject::Behaviour => {
                let plan = behaviour::plan(session);
                for row in &plan.rows {
                    push(&mut out, row.table, row.key.clone(), row.statements());
                }
                for script in &plan.scripts {
                    push(
                        &mut out,
                        script.table,
                        script.key(),
                        vale_mangos::scripts::statements(script),
                    );
                }
            }
        }
    }
    // The client-table rows (spells, flight nodes and skill line abilities),
    // whose revert file holds
    // one statement per entry.
    let undo = rows::Undo::open_at(project)?;
    for row in rows::plan(session, assets)?.rows {
        let put_back: Vec<String> = undo
            .entries
            .iter()
            .filter(|(table, key, _)| *table == row.table && *key == row.key)
            .map(|(_, _, statement)| statement.clone())
            .collect();
        out.push(Entry {
            subject: match row.table {
                vale_mangos::taxi::TABLE => "flight nodes",
                vale_mangos::skills::TABLE => "skill line abilities",
                _ => "spells",
            }
            .to_string(),
            table: row.table.to_string(),
            key: row.key,
            statements: row.statements,
            undo: match put_back.is_empty() {
                true => None,
                false => Some(put_back),
            },
        });
    }
    Ok(out)
}

fn revert_vpath(subject: Subject) -> &'static str {
    subject.revert_vpath()
}

/// The record of the last migration — see `vale_mangos::migration`.
pub fn released(project: &Project) -> Released {
    match project.read(migration::RELEASED_VPATH) {
        Some(bytes) => Released::from_text(&String::from_utf8_lossy(&bytes)),
        None => Released::default(),
    }
}

/// What writing a migration did.
#[derive(Debug, Default)]
pub struct Written {
    /// The file, or `None` when the project changes no row and never has.
    pub file: Option<PathBuf>,
    pub id: Option<String>,
    pub contents: Contents,
}

impl Written {
    pub fn line(&self) -> String {
        match &self.file {
            Some(file) => format!(
                "migration {} ({})",
                file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                self.contents.line()
            ),
            None => "no migration: this project changes no row".to_string(),
        }
    }
}

/// Writes the migration into `into`, then the record of it. Nothing is written
/// when the project changes no row and no earlier patch did.
pub fn write_migration(session: &EditSession, assets: &GameAssets, into: &Path) -> Result<Written, String> {
    let project = &session.project;
    let now = entries(session, assets)?;
    let before = released(project);
    let contents = migration::contents(&before, &now);
    if contents.is_empty() {
        return Ok(Written::default());
    }
    let id = migration::stamp(before.id.as_deref());
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
    let file = into.join(migration::file_name(&id));
    vale_edit::project::write_replacing(&file, migration::text(&id, &project.name, &contents).as_bytes())
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let after = migration::released_after(&id, &contents);
    project
        .write(migration::RELEASED_VPATH, after.to_text(&project.name).as_bytes())
        .map_err(|e| format!("{}: {e}", migration::RELEASED_VPATH))?;
    Ok(Written {
        file: Some(file),
        id: Some(id),
        contents,
    })
}

/// Copies the project's edited DBCs that the server reads as files into
/// `DataDir\5875\dbc\`: the 48 named in `vale_mangos::datadir::SERVER_DBCS`.
/// A DBC the project carries that the server reads as a SQL table — `Spell`,
/// `AreaTable`, `TaxiNodes` — is not a file to it and is left out. Returns
/// the names copied.
pub fn copy_server_dbcs(project: &Project, data_dir: &Path) -> Result<Vec<String>, String> {
    let dbc_dir = vale_mangos::datadir::dbc_dir(data_dir);
    let mut copied = Vec::new();
    for (file, vpath) in super::dbcs::carried(project) {
        let Some(bytes) = project.read(&vpath) else {
            continue;
        };
        std::fs::create_dir_all(&dbc_dir).map_err(|e| format!("{}: {e}", dbc_dir.display()))?;
        let target = dbc_dir.join(&file);
        vale_edit::project::write_replacing(&target, &bytes)
            .map_err(|e| format!("{}: {e}", target.display()))?;
        copied.push(file);
    }
    Ok(copied)
}

/// Writes the migration on its own, for the Server panel's button and
/// `--migration`, into `publish\migrations\` under the project rather than
/// into a patch folder. The store and every subject's SQL are saved first, so
/// the migration reads what the panel shows. Returns the status line.
pub fn migrate(session: &mut EditSession, assets: &GameAssets) -> String {
    session.save_all_tables();
    super::creatures::save(session);
    super::gameobjects::save(session);
    super::items::save(session);
    super::quests::save(session);
    super::loot::save(session);
    super::services::save(session);
    super::behaviour::save(session);
    let Some(into) = session.project.path_for(migration::MIGRATIONS_DIR) else {
        return format!("{} leaves the project", migration::MIGRATIONS_DIR);
    };
    match write_migration(session, assets, &into) {
        Ok(written) => {
            report_unknown(&written.contents);
            written.line()
        }
        Err(e) => format!("migration not written: {e}"),
    }
}

/// Logs every dropped row whose put-back is not known.
pub fn report_unknown(contents: &Contents) {
    for entry in &contents.unknown {
        warn!(
            "migration: {} {} {} was in an earlier patch and is no longer changed, and its \
             earlier value is not known \u{2014} put it back by hand",
            entry.subject,
            entry.table,
            entry.key.text()
        );
    }
}

/// `--migration`: runs [`migrate`] from the command line once, after every
/// scripted edit (waypoints, creatures, flight paths) has been made.
pub fn migration_on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    waypoints: Res<crate::tools::waypoints::Waypoints>,
    creatures: Res<crate::tools::creatures::Creatures>,
    flights: Res<crate::tools::flightpaths::Flightpaths>,
    mut done: Local<bool>,
) {
    if *done
        || !args.migration
        || !waypoints.scripted_done
        || !creatures.scripted_done
        || !flights.scripted_done
    {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    let said = migrate(&mut session, &assets);
    info!("--migration: {said}");
    session.status = said;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server DBC copy takes the 48 and leaves the SQL-backed ones.
    #[test]
    fn only_the_tables_the_server_reads_as_files_are_copied() {
        let root = std::env::temp_dir().join(format!("vale-release-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = Project::at(root.join("Edit").join("p")).unwrap();
        project.write(r"DBFilesClient\SpellCastTimes.dbc", b"cast").unwrap();
        project.write(r"DBFilesClient\Spell.dbc", b"spell").unwrap();
        project.write(r"DBFilesClient\AreaTable.dbc", b"area").unwrap();
        let data_dir = root.join("server");
        let copied = copy_server_dbcs(&project, &data_dir).unwrap();
        assert_eq!(copied, vec!["SpellCastTimes.dbc".to_string()]);
        let dbc_dir = vale_mangos::datadir::dbc_dir(&data_dir);
        assert_eq!(std::fs::read(dbc_dir.join("SpellCastTimes.dbc")).unwrap(), b"cast");
        assert!(!dbc_dir.join("Spell.dbc").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
