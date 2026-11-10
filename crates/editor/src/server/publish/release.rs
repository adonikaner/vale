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
//! wrote the row. They are kept in the release record ([`Releases`]) from the
//! moment they are known, so a row dropped after a later Put back, which removes
//! the revert file, is still put back. A row that was never applied to a
//! database has no put-back anywhere; when it is dropped, the migration
//! names it in a comment and the publish reports it.
//!
//! For this reason a project is applied to the dev database before it is
//! published. With Apply on save on, every save has already done so. The migration then
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

use crate::server::stack::Subject;
use crate::server::reconcile::Undo;
use crate::server::rows;
use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::project::Project;
use vale_mangos::migration::{self, Contents, Entry, Released};
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// Every row the project currently changes, as migration entries, in the
/// order a migration runs them. Each carries its put-back when the subject's
/// revert file has one. Client-table rows are labelled "spells" or "flight
/// nodes" by their table.
pub(crate) fn entries(session: &EditSession, assets: &GameAssets) -> Result<Vec<Entry>, String> {
    let project = &session.project;
    let mut out: Vec<Entry> = Vec::new();
    for subject in Subject::ORDER {
        let undo = Undo::open_at(project, subject.revert_vpath())?;
        let plan = crate::server::subject::plan(session, subject.row_subject());
        for unit in plan.units() {
            if unit.statements.is_empty() {
                continue;
            }
            let put_back: Vec<String> = undo
                .entries
                .iter()
                .filter(|(had_table, had_key, _)| *had_table == unit.table && *had_key == unit.key)
                .flat_map(|(_, _, statements)| statements.iter().cloned())
                .collect();
            out.push(Entry {
                subject: subject.name().to_string(),
                table: unit.table,
                undo: match put_back.is_empty() {
                    true => None,
                    false => Some(put_back),
                },
                key: unit.key,
                statements: unit.statements,
            });
        }
    }
    // The client-table rows (spells, flight nodes, skill line abilities,
    // areas, area triggers, factions and faction templates), whose revert
    // file holds one statement per entry.
    let undo = Undo::open_at(project, rows::REVERT_VPATH)?;
    for row in rows::plan(session, assets)?.rows {
        let put_back: Vec<String> = undo
            .entries
            .iter()
            .filter(|(table, key, _)| *table == row.table && *key == row.key)
            .flat_map(|(_, _, statements)| statements.iter().cloned())
            .collect();
        out.push(Entry {
            subject: match row.table {
                vale_mangos::taxi::TABLE => "flight nodes",
                vale_mangos::skills::TABLE => "skill line abilities",
                vale_mangos::area::TABLE => "areas",
                vale_mangos::trigger::TEMPLATE => "area triggers",
                vale_mangos::faction::TABLE => "factions",
                vale_mangos::faction::TEMPLATE => "faction templates",
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

/// The record of the last migration — see `vale_mangos::migration`.
pub(crate) fn released(project: &Project) -> Released {
    read(project).migration
}

/// What the project has released, in one file, `publish\released.txt`
/// (`vale_mangos::migration::RELEASED_VPATH`): the last migration's rows and
/// put-backs, the last patch with the project's last log edit when it was
/// made (`crate::collab::log`), and the tiles that patch's server files were
/// built from (`crate::server::datadir::Record`). Each part is read from its
/// own lines, so a write of one part keeps the others.
///
/// Earlier builds kept the last patch in `publish\last-patch.txt` and the
/// tiles in `publish\server-tiles.txt`. A record without those parts reads
/// them from the old files, and the next [`write`] removes the old files.
#[derive(Debug, Clone, Default)]
pub(crate) struct Releases {
    pub(crate) migration: Released,
    /// The last patch's folder name.
    pub(crate) patch: Option<String>,
    /// The number of the project's last log edit when the last patch was
    /// made.
    pub(crate) edit: Option<u64>,
    pub(crate) tiles: crate::server::datadir::Record,
}

/// Where earlier builds kept the last patch, and the tiles its server files
/// were built from. See [`Releases`].
const OLD_LAST_PATCH: &str = "publish\\last-patch.txt";
const OLD_PATCH_TILES: &str = "publish\\server-tiles.txt";

/// The project's release record. See [`Releases`].
pub(crate) fn read(project: &Project) -> Releases {
    let text = project.read(migration::RELEASED_VPATH).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
    let mut out = Releases {
        migration: Released::from_text(&text),
        patch: None,
        edit: None,
        tiles: crate::server::datadir::Record::from_release_text(&text),
    };
    for line in text.lines() {
        let mut fields = line.trim_end().split('\t');
        match (fields.next(), fields.next()) {
            (Some("patch"), Some(name)) => out.patch = Some(name.trim().to_string()),
            (Some("edit"), Some(edit)) => out.edit = edit.trim().parse().ok(),
            _ => {}
        }
    }
    if out.patch.is_none() {
        if let Some(bytes) = project.read(OLD_LAST_PATCH) {
            let old = String::from_utf8_lossy(&bytes).into_owned();
            let mut lines = old.lines();
            out.patch = lines.next().map(|name| name.trim().to_string()).filter(|name| !name.is_empty());
            out.edit = lines.next().and_then(|line| line.trim().strip_prefix("edit ")?.parse().ok());
        }
    }
    if out.tiles == crate::server::datadir::Record::default() && project.has(OLD_PATCH_TILES) {
        out.tiles = crate::server::datadir::Record::read(project, OLD_PATCH_TILES);
    }
    out
}

/// Write the project's release record whole, and remove the old files it
/// replaces. See [`Releases`].
pub(crate) fn write(project: &Project, releases: &Releases) -> Result<(), String> {
    let mut text = releases.migration.to_text(&project.name);
    text.push_str(
        "# The last patch, the project's log edit it was made from, and the tiles its\n\
         # server files were built from, as the FNV-1a 64 of each ADT's bytes, with the\n\
         # Map.dbc id of each map they are on.\n",
    );
    if let Some(patch) = &releases.patch {
        text.push_str(&format!("patch\t{patch}\n"));
    }
    if let Some(edit) = releases.edit {
        text.push_str(&format!("edit\t{edit}\n"));
    }
    text.push_str(&releases.tiles.release_lines());
    project.write(migration::RELEASED_VPATH, text.as_bytes()).map_err(|e| format!("{}: {e}", migration::RELEASED_VPATH))?;
    for old in [OLD_LAST_PATCH, OLD_PATCH_TILES] {
        if project.has(old) {
            let _ = project.revert(old);
        }
    }
    Ok(())
}

/// What writing a migration did.
#[derive(Debug, Default)]
pub(crate) struct Written {
    /// The file, or `None` when the project changes no row and never has.
    pub(crate) file: Option<PathBuf>,
    pub(crate) contents: Contents,
}

impl Written {
    pub(crate) fn line(&self) -> String {
        match &self.file {
            Some(file) => format!(
                "migration {} ({})",
                file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                self.contents.line()
            ),
            None if self.contents.is_empty() => "no migration: this project changes no row".to_string(),
            None => "no migration: nothing changed since the last one".to_string(),
        }
    }
}

/// Writes the migration into `into`, then the record of it. Nothing is written
/// when the project changes no row and no earlier migration did. With
/// `only_if_new`, nothing is written either when the migration would say
/// nothing the last one did not ([`migration::says_more_than`]): Write
/// migration asks for that, and a publish does not, since every patch holds
/// its whole migration.
pub(crate) fn write_migration(session: &EditSession, assets: &GameAssets, into: &Path, only_if_new: bool) -> Result<Written, String> {
    let project = &session.project;
    let now = entries(session, assets)?;
    let before = released(project);
    let contents = migration::contents(&before, &now);
    if contents.is_empty() {
        return Ok(Written::default());
    }
    if only_if_new && !migration::says_more_than(&before, &contents) {
        return Ok(Written { file: None, contents });
    }
    let id = migration::stamp(before.id.as_deref());
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
    let file = into.join(migration::file_name(&id));
    vale_edit::project::write_replacing(&file, migration::text(&id, &project.name, &contents).as_bytes())
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let mut record = read(project);
    record.migration = migration::released_after(&id, &contents);
    write(project, &record)?;
    Ok(Written {
        file: Some(file),
        contents,
    })
}

/// Copies the project's edited DBCs that the server reads as files into
/// `DataDir\5875\dbc\`: the 48 named in `vale_mangos::datadir::SERVER_DBCS`.
/// A DBC the project carries that the server reads as a SQL table — `Spell`,
/// `AreaTable`, `TaxiNodes` — is not a file to it and is left out. Returns
/// the names copied.
pub(crate) fn copy_server_dbcs(project: &Project, data_dir: &Path) -> Result<Vec<String>, String> {
    let dbc_dir = vale_mangos::datadir::dbc_dir(data_dir);
    let mut copied = Vec::new();
    for (file, vpath) in crate::server::dbcs::carried(project) {
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
pub(crate) fn migrate(session: &mut EditSession, assets: &GameAssets) -> String {
    session.save_files_with(crate::server::subject::save_all);
    let Some(into) = session.project.path_for(migration::MIGRATIONS_DIR) else {
        return format!("{}: path is outside the project folder", migration::MIGRATIONS_DIR);
    };
    match write_migration(session, assets, &into, true) {
        Ok(written) => {
            report_unknown(&written.contents);
            written.line()
        }
        Err(e) => format!("migration not written: {e}"),
    }
}

/// Logs every dropped row whose put-back is not known.
pub(crate) fn report_unknown(contents: &Contents) {
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
pub(crate) fn migration_on_the_command_line(
    args: Res<crate::args::Args>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    waypoints: Res<crate::tools::spawns::waypoints::Waypoints>,
    creatures: Res<crate::tools::spawns::creatures::Creatures>,
    flights: Res<crate::tools::places::flightpaths::Flightpaths>,
    mut done: Local<bool>,
) {
    if *done
        || !args.server.migration
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
