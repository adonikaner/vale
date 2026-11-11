//! The DBC files the server reads, copied from the project into the live
//! `DataDir\5875\dbc\` and put back.
//!
//! vmangos reads 48 client tables as files from `DataDir\5875\dbc\`
//! (`vale_mangos::datadir::SERVER_DBCS`), among them `TaxiPath`,
//! `TaxiPathNode` and the four spell tables the spell tool edits
//! (`SpellCastTimes`, `SpellDuration`, `SpellRadius`, `SpellRange`). A publish
//! copies the project's copies of those into a patch folder; this module does
//! the same into this machine's live server, for a playtest after a restart.
//!
//! ## The server's own copies, kept by the DataDir
//!
//! The `DataDir`'s ledger (`vale_mangos::datadir::ledger`) records each file
//! the editor writes, which project wrote it last, and the server's own copy
//! from before the first write, in `DataDir\vale-editor\originals\`, as it
//! does for the server's tile files. The record is the server's and not the
//! project's: it outlasts a deleted project, shows the files another project
//! wrote, and cannot be put back into another server's `DataDir`, since each
//! `DataDir` has its own. Put back restores the files this project wrote
//! last. An apply first puts back any file the project no longer carries, so
//! the live folder is always the server's own files plus what the project
//! says now. The tile files dialog's Put back originals restores these with
//! the project's tile files.
//!
//! A project that applied before the ledger held DBC files keeps the
//! server's copies in its own `server\dbc-before\` ([`BEFORE_DIR`]). The next
//! apply or put back moves them into the ledger and removes them from the
//! project ([`adopt_the_projects_copies`]).
//!
//! vmangos reads these files at startup only, so a copied file takes effect
//! when the server is restarted.

use std::path::Path;
use vale_edit::project::Project;
use vale_mangos::datadir::Ledger;

/// Where a project kept the server's own copies before the `DataDir`'s
/// ledger did. See the module comment's section "The server's own copies,
/// kept by the DataDir".
pub(crate) const BEFORE_DIR: &str = "server\\dbc-before";

/// The file in [`BEFORE_DIR`] that lists the names of the files the server
/// did not have.
const ABSENT: &str = "server\\dbc-before\\absent.txt";

/// The DBCs the project carries that the server reads as files, as
/// `(file name, virtual path)`: `("TaxiPath.dbc", "DBFilesClient\TaxiPath.dbc")`.
pub(crate) fn carried(project: &Project) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = project
        .files()
        .into_iter()
        .filter_map(|vpath| {
            let parts: Vec<&str> = vpath.split(['\\', '/']).collect();
            let [folder, file] = parts[..] else {
                return None;
            };
            if !folder.eq_ignore_ascii_case("DBFilesClient") {
                return None;
            }
            let name = file.strip_suffix(".dbc")?;
            vale_mangos::datadir::server_reads_dbc(name)
                .then(|| (file.to_string(), vpath.clone()))
        })
        .collect();
    out.sort();
    out
}

/// The server copies a project still keeps in [`BEFORE_DIR`], by file name:
/// those saved as a file and those listed as absent.
pub(crate) fn kept_in_project(project: &Project) -> Vec<String> {
    let mut out: Vec<String> = project
        .path_for(BEFORE_DIR)
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.to_ascii_lowercase().ends_with(".dbc").then_some(name)
        })
        .collect();
    out.extend(absent(project));
    out.sort();
    out.dedup();
    out
}

fn absent(project: &Project) -> Vec<String> {
    project
        .read(ABSENT)
        .map(|bytes| {
            String::from_utf8_lossy(&bytes)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The ledger's folder and name for a DBC file: `5875` and `dbc\TaxiPath.dbc`.
fn ledger_key(name: &str) -> (String, String) {
    let (first, rest) = dbc_folder();
    (first, format!("{rest}\\{name}"))
}

/// The DBC folder under a `DataDir` as the ledger names files: its first
/// part, `5875`, and the rest, `dbc`.
fn dbc_folder() -> (String, String) {
    let relative = vale_mangos::datadir::dbc_dir(Path::new(""));
    let mut parts = relative.iter().map(|part| part.to_string_lossy().into_owned());
    let first = parts.next().unwrap_or_default();
    let rest: Vec<String> = parts.collect();
    (first, rest.join("\\"))
}

/// The DBC files the ledger says `project` wrote last, by file name.
fn written(ledger: &Ledger, project: &Project) -> Vec<String> {
    let (first, rest) = dbc_folder();
    let prefix = format!("{first}\\{rest}\\");
    ledger
        .written_by(&project.name)
        .into_iter()
        .filter_map(|(key, _)| key.strip_prefix(&prefix).map(str::to_string))
        .collect()
}

/// Whether there is anything to copy or put back for `project`: a file it
/// carries, a copy it still keeps, or a file the `DataDir`'s ledger says it
/// wrote.
pub(crate) fn any(project: &Project, data_dir: Option<&Path>) -> bool {
    !carried(project).is_empty()
        || !kept_in_project(project).is_empty()
        || data_dir.and_then(|dir| Ledger::open(dir).ok()).is_some_and(|ledger| !written(&ledger, project).is_empty())
}

/// Move the server copies a project keeps in [`BEFORE_DIR`] into the
/// ledger, then remove them from the project. A file the ledger already
/// holds keeps the ledger's copy.
fn adopt_the_projects_copies(ledger: &mut Ledger, project: &Project) -> Result<(), String> {
    let missing = absent(project);
    for name in kept_in_project(project) {
        let (sub, file) = ledger_key(&name);
        let original = match missing.iter().any(|had| had.eq_ignore_ascii_case(&name)) {
            true => None,
            false => project.read(&format!("{BEFORE_DIR}\\{name}")),
        };
        ledger.adopt(&sub, &file, &project.name, original.as_deref())?;
        let _ = project.revert(&format!("{BEFORE_DIR}\\{name}"));
    }
    let _ = project.revert(ABSENT);
    let _ = project.revert(&format!("{BEFORE_DIR}\\folder.txt"));
    Ok(())
}

/// Where each carried file stands against the live folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Standing {
    /// Every carried file, by name.
    pub(crate) carried: Vec<String>,
    /// The carried files whose bytes differ from the live folder's.
    pub(crate) differ: Vec<String>,
    /// How many files this project wrote last, which is what Put back
    /// restores.
    pub(crate) saved: usize,
    /// Files this project wrote and no longer carries, which the next apply
    /// puts back.
    pub(crate) stale: Vec<String>,
}

impl Standing {
    /// Whether an apply would change the live folder.
    pub(crate) fn outstanding(&self) -> bool {
        !self.differ.is_empty() || !self.stale.is_empty()
    }

    /// One line for the panel.
    pub(crate) fn line(&self) -> String {
        if self.carried.is_empty() && self.saved == 0 {
            return "No server DBC file changed".to_string();
        }
        let files = |count: usize| match count {
            1 => "1 server DBC file".to_string(),
            n => format!("{n} server DBC files"),
        };
        let mut line = match (self.carried.len(), self.differ.len()) {
            (0, _) => "No server DBC file changed".to_string(),
            (n, 0) => format!("{}, all in DataDir\\5875\\dbc", files(n)),
            (n, d) => format!("{}, {d} not in DataDir\\5875\\dbc yet", files(n)),
        };
        if !self.stale.is_empty() {
            line.push_str(&format!(
                "; {} no longer changed, restored on the next apply",
                self.stale.len()
            ));
        }
        line
    }
}

/// Compare the project's files with the live folder of `data_dir`. Reads no
/// more than the files the project carries.
pub(crate) fn standing(project: &Project, data_dir: &Path) -> Standing {
    let dbc_dir = vale_mangos::datadir::dbc_dir(data_dir);
    let carried = carried(project);
    let mut saved = Ledger::open(data_dir).map(|ledger| written(&ledger, project)).unwrap_or_default();
    saved.extend(kept_in_project(project));
    saved.sort();
    saved.dedup();
    let names: Vec<&str> = carried.iter().map(|(name, _)| name.as_str()).collect();
    let differ = carried
        .iter()
        .filter(|(name, vpath)| {
            let live = std::fs::read(dbc_dir.join(name)).ok();
            live.as_deref() != project.read(vpath).as_deref()
        })
        .map(|(name, _)| name.clone())
        .collect();
    Standing {
        carried: names.iter().map(|name| name.to_string()).collect(),
        differ,
        saved: saved.len(),
        stale: saved
            .iter()
            .filter(|name| !names.iter().any(|had| had.eq_ignore_ascii_case(name)))
            .cloned()
            .collect(),
    }
}

/// What an apply or a put back did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Synced {
    /// Files written into the live folder.
    pub(crate) written: Vec<String>,
    /// Files returned to the server's own copy.
    pub(crate) restored: Vec<String>,
}

impl Synced {
    pub(crate) fn line(&self) -> String {
        let mut parts = Vec::new();
        if !self.written.is_empty() {
            parts.push(format!("{} copied into DataDir\\5875\\dbc", self.written.join(", ")));
        }
        if !self.restored.is_empty() {
            parts.push(format!("{} restored", self.restored.join(", ")));
        }
        match parts.is_empty() {
            true => "the server DBC files were already current".to_string(),
            false => format!("{}; restart the server to read them", parts.join("; ")),
        }
    }
}

/// Copy the project's server DBCs into `data_dir`'s DBC folder, the ledger
/// keeping the server's copy of each the first time, and put back any file
/// this project wrote and no longer carries.
pub(crate) fn apply_at(project: &Project, data_dir: &Path) -> Result<Synced, String> {
    let dbc_dir = vale_mangos::datadir::dbc_dir(data_dir);
    let mut ledger = Ledger::open(data_dir)?;
    adopt_the_projects_copies(&mut ledger, project)?;
    let carried = carried(project);
    let mut out = Synced::default();

    let wanted = |name: &str| carried.iter().any(|(had, _)| had.eq_ignore_ascii_case(name));
    for name in written(&ledger, project) {
        if !wanted(&name) {
            restore(&mut ledger, &name)?;
            out.restored.push(name);
        }
    }

    std::fs::create_dir_all(&dbc_dir).map_err(|e| format!("{}: {e}", dbc_dir.display()))?;
    for (name, vpath) in &carried {
        let Some(bytes) = project.read(vpath) else {
            continue;
        };
        let target = dbc_dir.join(name);
        if std::fs::read(&target).ok().as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        let (sub, file) = ledger_key(name);
        ledger.before_write(&sub, &file, &project.name)?;
        vale_edit::project::write_replacing(&target, &bytes)
            .map_err(|e| format!("{}: {e}", target.display()))?;
        out.written.push(name.clone());
    }
    Ok(out)
}

/// Return every file this project wrote last to the server's copy.
pub(crate) fn put_back_at(project: &Project, data_dir: &Path) -> Result<Synced, String> {
    let mut ledger = Ledger::open(data_dir)?;
    adopt_the_projects_copies(&mut ledger, project)?;
    let mut out = Synced::default();
    for name in written(&ledger, project) {
        restore(&mut ledger, &name)?;
        out.restored.push(name);
    }
    Ok(out)
}

/// Return one file to the server's copy, or remove it when the server had
/// none.
fn restore(ledger: &mut Ledger, name: &str) -> Result<(), String> {
    let (sub, file) = ledger_key(name);
    ledger.put_back_file(&vale_mangos::datadir::ledger::key(&sub, &file))?.map_err(|why| format!("{name}: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Place {
        root: std::path::PathBuf,
        project: Project,
        data: std::path::PathBuf,
        dbc: std::path::PathBuf,
    }

    fn place(tag: &str) -> Place {
        let root = std::env::temp_dir().join(format!("vale-dbcs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = Project::at(root.join("Edit").join("p")).unwrap();
        let data = root.join("data");
        let dbc = vale_mangos::datadir::dbc_dir(&data);
        std::fs::create_dir_all(&dbc).unwrap();
        Place { root, project, data, dbc }
    }

    impl Drop for Place {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn only_the_tables_the_server_reads_as_files_are_carried() {
        let p = place("carried");
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"path").unwrap();
        p.project.write(r"DBFilesClient\Spell.dbc", b"spell").unwrap();
        p.project.write(r"DBFilesClient\TaxiNodes.dbc", b"nodes").unwrap();
        let names: Vec<String> = carried(&p.project).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["TaxiPath.dbc"]);
    }

    /// An apply keeps the server's copy in the DataDir's ledger, not in the
    /// project, and a put back restores it.
    #[test]
    fn an_apply_keeps_the_servers_copy_and_a_put_back_restores_it() {
        let p = place("roundtrip");
        std::fs::write(p.dbc.join("TaxiPath.dbc"), b"server's own").unwrap();
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"edited").unwrap();
        p.project.write(r"DBFilesClient\TaxiPathNode.dbc", b"new file").unwrap();
        assert!(standing(&p.project, &p.data).outstanding());

        let done = apply_at(&p.project, &p.data).unwrap();
        assert_eq!(done.written, ["TaxiPath.dbc", "TaxiPathNode.dbc"]);
        assert_eq!(std::fs::read(p.dbc.join("TaxiPath.dbc")).unwrap(), b"edited");
        let now = standing(&p.project, &p.data);
        assert!(!now.outstanding(), "{now:?}");
        assert_eq!(now.saved, 2);
        assert!(kept_in_project(&p.project).is_empty(), "nothing is kept in the project");

        // A second apply keeps the first copy.
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"edited again").unwrap();
        apply_at(&p.project, &p.data).unwrap();

        let back = put_back_at(&p.project, &p.data).unwrap();
        assert_eq!(back.restored.len(), 2);
        assert_eq!(std::fs::read(p.dbc.join("TaxiPath.dbc")).unwrap(), b"server's own");
        assert!(!p.dbc.join("TaxiPathNode.dbc").exists(), "the server had none");
        assert_eq!(standing(&p.project, &p.data).saved, 0);
    }

    /// Copies a project kept in `server\dbc-before\` move into the ledger on
    /// the next apply, and a put back restores them.
    #[test]
    fn copies_kept_in_the_project_move_into_the_ledger() {
        let p = place("adopt");
        std::fs::write(p.dbc.join("TaxiPath.dbc"), b"edited").unwrap();
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"edited").unwrap();
        p.project.write(&format!("{BEFORE_DIR}\\TaxiPath.dbc"), b"server's own").unwrap();
        std::fs::write(p.dbc.join("TaxiPathNode.dbc"), b"made").unwrap();
        p.project.write(ABSENT, b"TaxiPathNode.dbc\n").unwrap();
        assert!(any(&p.project, Some(&p.data)));

        put_back_at(&p.project, &p.data).unwrap();
        assert_eq!(std::fs::read(p.dbc.join("TaxiPath.dbc")).unwrap(), b"server's own");
        assert!(!p.dbc.join("TaxiPathNode.dbc").exists());
        assert!(kept_in_project(&p.project).is_empty());
    }

    #[test]
    fn a_file_the_project_stops_carrying_is_put_back_on_the_next_apply() {
        let p = place("dropped");
        std::fs::write(p.dbc.join("TaxiPath.dbc"), b"server's own").unwrap();
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"edited").unwrap();
        apply_at(&p.project, &p.data).unwrap();
        p.project.revert(r"DBFilesClient\TaxiPath.dbc").unwrap();
        assert_eq!(standing(&p.project, &p.data).stale, ["TaxiPath.dbc"]);
        let done = apply_at(&p.project, &p.data).unwrap();
        assert_eq!(done.restored, ["TaxiPath.dbc"]);
        assert_eq!(std::fs::read(p.dbc.join("TaxiPath.dbc")).unwrap(), b"server's own");
    }

    #[test]
    fn the_saved_copies_are_not_packed_into_a_patch() {
        assert!(!Project::is_game_data(&format!("{BEFORE_DIR}\\TaxiPath.dbc")));
    }
}
