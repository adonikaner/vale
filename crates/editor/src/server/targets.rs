//! Which project's rows a world database holds, so two projects that change
//! the same row cannot restore over each other.
//!
//! ## Why a row is held by one project at a time
//!
//! Each project keeps its own revert files (`super::reconcile`): the rows it
//! applied, each as it was read just before the project wrote it. When two
//! projects apply the same row, the second reads the first project's value as
//! "before". Restoring the first project and then the second writes the first
//! project's value back, and the row ends changed by a project that has
//! restored. So a row one project has applied is refused to every other
//! project until the first restores it.
//!
//! ## Where it is read from
//!
//! The revert files are the record. Each names the world database its rows
//! were read from (`super::reconcile::Undo::database`), so the rows a
//! database holds for other projects are the entries of every other
//! project's revert files under `Edit\` that name the same database. A
//! revert file written before it named its database is counted as this
//! database's. Restoring a project removes its revert files, and clearing or
//! deleting a project removes them with its files, so nothing else has to be
//! told.
//!
//! Earlier builds kept a second record in `Edit\targets\`, one file per
//! database. It is not read, and [`Rows::open`] removes the folder.

use super::reconcile::{database_of, Undo};
use std::collections::BTreeMap;
use vale_edit::project::Project;
use vale_mangos::conn::Where;

/// The folder under `Edit\` that earlier builds kept the record in.
const OLD_FOLDER: &str = "targets";

/// Every revert file a project keeps: one per row subject and the client
/// tables' rows.
fn revert_vpaths() -> impl Iterator<Item = &'static str> {
    super::stack::Subject::ORDER.into_iter().map(|subject| subject.revert_vpath()).chain(std::iter::once(super::rows::REVERT_VPATH))
}

/// The rows a world database holds for the projects other than one. See the
/// module comment.
#[derive(Debug, Default)]
pub(crate) struct Rows {
    /// The project that applied each row, by `(table, key)`.
    owners: BTreeMap<(String, String), String>,
}

impl Rows {
    /// The rows the database at `at` holds for every project beside
    /// `project`, read from their revert files.
    pub(crate) fn open(project: &Project, at: &Where) -> Rows {
        let database = database_of(at);
        let mut owners = BTreeMap::new();
        let Some(edit) = project.root.parent() else { return Rows { owners } };
        let _ = std::fs::remove_dir_all(edit.join(OLD_FOLDER));
        let Ok(entries) = std::fs::read_dir(edit) else { return Rows { owners } };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == project.name || !entry.path().is_dir() {
                continue;
            }
            let Ok(other) = Project::at(entry.path()) else { continue };
            for vpath in revert_vpaths() {
                let Ok(undo) = Undo::open_at(&other, vpath) else { continue };
                if undo.database.as_deref().is_some_and(|had| !had.eq_ignore_ascii_case(&database)) {
                    continue;
                }
                for (table, key, _) in undo.entries {
                    owners.insert((table, key.text()), name.clone());
                }
            }
        }
        Rows { owners }
    }

    /// Refuse rows another project has applied and not restored, naming the
    /// first few.
    pub(crate) fn refuse_others(&self, project: &Project, rows: impl Iterator<Item = (String, String)>) -> Result<(), String> {
        let mut taken: Vec<String> = Vec::new();
        let mut owner = String::new();
        for (table, key) in rows {
            if let Some(holder) = self.owners.get(&(table.clone(), key.clone())).filter(|holder| **holder != project.name) {
                if taken.len() < 3 {
                    taken.push(format!("{table} {key}"));
                }
                owner = holder.clone();
            }
        }
        match taken.is_empty() {
            true => Ok(()),
            false => Err(format!(
                "{} applied by the project {owner}, which has not restored them; restore that project's rows first",
                taken.join(", ")
            )),
        }
    }
}

/// Where the open project has been written, read from each target's record,
/// for the Changes window's Targets tab: this machine's world database, this
/// machine's `DataDir`, and the project's releases.
#[derive(Debug, Clone)]
pub(crate) struct Overview {
    /// The world database as a line (`mangos@127.0.0.1:3306/mangos`), or why
    /// there is none.
    pub(crate) database: Result<String, String>,
    /// `(subject, rows)` for each subject with rows applied; `None` for a
    /// revert file that could not be read. See `super::held`.
    pub(crate) applied: Vec<(&'static str, Option<usize>)>,
    /// The `DataDir`, or why there is none.
    pub(crate) data_dir: Result<String, String>,
    /// Tile files and server DBC files this project wrote last, and tiles it
    /// built, as the `DataDir`'s ledger records them.
    pub(crate) tile_files: usize,
    pub(crate) dbc_files: usize,
    pub(crate) tiles_built: usize,
    /// The last patch, with the log edit it was made from.
    pub(crate) last_patch: Option<(String, Option<u64>)>,
    pub(crate) patches: usize,
    /// The id of the last migration.
    pub(crate) migration: Option<String>,
}

impl Overview {
    /// Read every target's record for `session`'s project.
    pub(crate) fn read(session: &crate::session::EditSession, server: &super::settings::ServerSettings) -> Overview {
        let project = &session.project;
        let database = server.resolve().map(|(at, _)| at.line()).ok_or_else(|| "no world database is set".to_string());
        let applied = super::held::held_by(project).rows;
        let dir = super::datadir::data_dir(server);
        let (mut tile_files, mut dbc_files, mut tiles_built) = (0, 0, 0);
        if let Ok(ledger) = dir.as_ref().map_err(String::clone).and_then(|dir| vale_mangos::datadir::Ledger::open(dir)) {
            for (key, _) in ledger.written_by(&project.name) {
                match key.to_ascii_lowercase().contains("\\dbc\\") {
                    true => dbc_files += 1,
                    false => tile_files += 1,
                }
            }
            tiles_built = ledger.tiles.values().filter(|built| built.project == project.name).count();
        }
        let releases = super::publish::release::read(project);
        let last_patch = super::publish::patch::last(project).map(|name| (name, releases.edit));
        Overview {
            database,
            applied,
            data_dir: dir.map(|dir| dir.display().to_string()),
            tile_files,
            dbc_files,
            tiles_built,
            last_patch,
            patches: super::publish::patch::patches(project).len(),
            migration: releases.migration.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row in another project's revert file for the same database is
    /// refused; one for another database, or once the file is gone, is not.
    #[test]
    fn a_row_another_project_holds_is_refused() {
        let install = std::env::temp_dir().join(format!("vale-targets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let (a, b) = (Project::open(&install, "a").unwrap(), Project::open(&install, "b").unwrap());
        let at = Where::parse("127.0.0.1;3306;mangos;secret;mangos").unwrap();
        let row = || std::iter::once(("creature".to_string(), "guid=7".to_string()));
        let vpath = crate::server::stack::Subject::ORDER[0].revert_vpath();
        let file = format!("-- database: {}\n-- row creature guid=7\nUPDATE x;\n-- end\n", database_of(&at));
        a.write(vpath, file.as_bytes()).unwrap();

        let refused = Rows::open(&b, &at).refuse_others(&b, row()).unwrap_err();
        assert!(refused.contains("the project a"), "{refused}");
        assert!(Rows::open(&a, &at).refuse_others(&a, row()).is_ok(), "a project's own rows are not refused it");
        let other = Where::parse("127.0.0.1;3306;mangos;secret;world").unwrap();
        assert!(Rows::open(&b, &other).refuse_others(&b, row()).is_ok(), "another database holds nothing of a's");

        // Restored, cleared or deleted, a project's revert file is gone.
        a.revert(vpath).unwrap();
        assert!(Rows::open(&b, &at).refuse_others(&b, row()).is_ok());
        let _ = std::fs::remove_dir_all(&install);
    }
}
