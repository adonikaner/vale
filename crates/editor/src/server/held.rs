//! What a project folder has in the database and in the live server's DBC
//! folder, read from its revert files. It is the gate on Clear and Delete.
//!
//! Clear and Delete remove `sql\*revert*.sql` and `server\dbc-before\`, which
//! are the only record of what a project applied and the only thing Put back
//! reads. A project cleared while applied leaves its rows in the database with
//! nothing that knows they are there. Discard does not remove the record: a
//! discarded project can still be put back, and its next apply puts it back
//! first (`super::reconcile`). So the project dialog asks this before either
//! destructive button acts, lists what would be lost, and requires the
//! project's name typed to proceed.
//!
//! Read from the folder rather than the session, because the project being
//! deleted is usually not the open one.

use super::stack::Subject;
use super::{creatures, dbcs, rows};
use vale_edit::project::Project;

/// The label the client-table rows are held under, beside the five subjects'
/// own names.
const CLIENT_TABLES: &str = "client table";

/// What a project holds that Put back would need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    /// `(subject, rows)` for every subject whose revert file has entries, in
    /// `Subject::ORDER`, then the client-table rows. A revert file that could
    /// not be read is listed with `None`.
    pub rows: Vec<(&'static str, Option<usize>)>,
    /// The server DBC files whose original copies the project keeps.
    pub dbcs: Vec<String>,
}

impl Held {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.dbcs.is_empty()
    }

    /// One line per thing held, for the confirmation.
    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .rows
            .iter()
            .map(|(subject, rows)| match rows {
                Some(n) => format!("{n} {subject} row(s) applied to the database"),
                None => format!("a {subject} revert file that could not be read"),
            })
            .collect();
        if !self.dbcs.is_empty() {
            out.push(format!(
                "{} copied into the server's DataDir\\5875\\dbc, with the originals kept here",
                self.dbcs.join(", ")
            ));
        }
        out
    }
}

/// Read the project's revert files and saved server copies.
pub fn held_by(project: &Project) -> Held {
    let mut out = Held::default();
    for subject in Subject::ORDER {
        match creatures::Undo::open_at(project, subject.revert_vpath()) {
            Ok(undo) if undo.entries.is_empty() => {}
            Ok(undo) => out.rows.push((subject.name(), Some(undo.entries.len()))),
            Err(_) => out.rows.push((subject.name(), None)),
        }
    }
    match rows::Undo::open_at(project) {
        Ok(undo) if undo.entries.is_empty() => {}
        Ok(undo) => out.rows.push((CLIENT_TABLES, Some(undo.entries.len()))),
        Err(_) => out.rows.push((CLIENT_TABLES, None)),
    }
    out.dbcs = dbcs::saved(project);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tag: &str) -> (std::path::PathBuf, Project) {
        let root = std::env::temp_dir().join(format!("vale-held-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = Project::at(root.join("Edit").join("p")).unwrap();
        (root, project)
    }

    #[test]
    fn an_empty_project_holds_nothing() {
        let (root, project) = project("empty");
        project.write(r"DBFilesClient\TaxiNodes.dbc", b"edited").unwrap();
        let held = held_by(&project);
        assert!(held.is_empty(), "{held:?}");
        assert!(held.lines().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Each kind of record is found where its writer puts it, in the writer's
    /// own format, so a change to a format that this did not follow fails here
    /// rather than letting a project be cleared with rows applied.
    #[test]
    fn every_kind_of_record_is_found() {
        let (root, project) = project("full");
        project
            .write(
                creatures::REVERT_VPATH,
                b"-- p\n-- row creature guid=10000005\nDELETE FROM `creature` WHERE `guid` = 10000005;\n-- end\n",
            )
            .unwrap();
        project
            .write(
                rows::REVERT_VPATH,
                b"-- p\n-- row taxi_nodes id=88;build=5875\nDELETE FROM `taxi_nodes` WHERE `id` = 88 AND `build` = 5875;\n",
            )
            .unwrap();
        project
            .write(&format!("{}\\TaxiPath.dbc", dbcs::BEFORE_DIR), b"original")
            .unwrap();
        let held = held_by(&project);
        assert_eq!(
            held.rows,
            vec![("creatures", Some(1)), (CLIENT_TABLES, Some(1))]
        );
        assert_eq!(held.dbcs, vec!["TaxiPath.dbc".to_string()]);
        let lines = held.lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].starts_with("1 creatures row(s)"));
        assert!(lines[2].contains("TaxiPath.dbc"));
        let _ = std::fs::remove_dir_all(root);
    }
}
