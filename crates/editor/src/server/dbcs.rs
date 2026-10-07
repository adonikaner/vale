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
//! ## Taking it back
//!
//! Before a file is first overwritten, the server's own copy is saved into the
//! project at `server\dbc-before\<Name>.dbc`, or its absence is listed in
//! `server\dbc-before\absent.txt`. `server\` is not packed into a patch
//! archive (`Project::NOT_GAME_DATA`). Put back copies every saved file back
//! and deletes the saved copies, the same arrangement `sql\revert.sql` gives
//! the rows. An apply first puts back any file the project no longer carries,
//! so the live folder is always the server's own files plus what the project
//! says now.
//!
//! vmangos reads these files at startup only, so a copied file takes effect
//! when the server is restarted.

use vale_edit::project::Project;
use std::path::Path;

/// Where the server's own copies are kept, under the project.
pub const BEFORE_DIR: &str = "server\\dbc-before";

/// The names of files the server did not have before this project wrote them.
const ABSENT: &str = "server\\dbc-before\\absent.txt";

/// The DBCs the project carries that the server reads as files, as
/// `(file name, virtual path)`: `("TaxiPath.dbc", "DBFilesClient\TaxiPath.dbc")`.
pub fn carried(project: &Project) -> Vec<(String, String)> {
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

/// The files whose server copies are saved in the project: those saved as a
/// file and those listed as absent.
pub fn saved(project: &Project) -> Vec<String> {
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

fn write_absent(project: &Project, names: &[String]) -> Result<(), String> {
    if names.is_empty() {
        if let Some(path) = project.path_for(ABSENT) {
            let _ = std::fs::remove_file(path);
        }
        return Ok(());
    }
    let mut text = String::from(
        "# The server's DataDir\\5875\\dbc had no file by these names before this project\n\
         # copied one there. Restore deletes them.\n",
    );
    for name in names {
        text.push_str(name);
        text.push('\n');
    }
    project
        .write(ABSENT, text.as_bytes())
        .map(|_| ())
        .map_err(|e| format!("{ABSENT}: {e}"))
}

/// Where each carried file stands against the live folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Standing {
    /// Every carried file, by name.
    pub carried: Vec<String>,
    /// The carried files whose bytes differ from the live folder's.
    pub differ: Vec<String>,
    /// How many files have a saved server copy, which is what Put back
    /// restores.
    pub saved: usize,
    /// Saved files the project no longer carries, which the next apply puts
    /// back.
    pub stale: Vec<String>,
}

impl Standing {
    /// Whether an apply would change the live folder.
    pub fn outstanding(&self) -> bool {
        !self.differ.is_empty() || !self.stale.is_empty()
    }

    /// One line for the panel.
    pub fn line(&self) -> String {
        if self.carried.is_empty() && self.saved == 0 {
            return "no server DBC file changed".to_string();
        }
        let mut line = match (self.carried.len(), self.differ.len()) {
            (0, _) => "no server DBC file changed".to_string(),
            (n, 0) => format!("{n} server DBC file(s), all in DataDir\\5875\\dbc"),
            (n, d) => format!("{n} server DBC file(s), {d} not in DataDir\\5875\\dbc yet"),
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

/// Compare the project's files with the live folder. Reads no more than the
/// files the project carries.
pub fn standing(project: &Project, dbc_dir: &Path) -> Standing {
    let carried = carried(project);
    let saved = saved(project);
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
pub struct Synced {
    /// Files written into the live folder.
    pub written: Vec<String>,
    /// Files returned to the server's own copy.
    pub restored: Vec<String>,
}

impl Synced {
    pub fn line(&self) -> String {
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

/// Copy the project's server DBCs into `dbc_dir`, saving each server copy the
/// first time, and put back any saved file the project no longer carries.
///
/// Every server copy is saved before any file is written, so a failure half
/// way leaves a put back that restores what was there.
pub fn apply_at(project: &Project, dbc_dir: &Path) -> Result<Synced, String> {
    let carried = carried(project);
    let mut out = Synced::default();

    let wanted = |name: &str| carried.iter().any(|(had, _)| had.eq_ignore_ascii_case(name));
    for name in saved(project) {
        if !wanted(&name) {
            restore(project, dbc_dir, &name)?;
            out.restored.push(name);
        }
    }

    let already = saved(project);
    let mut missing = absent(project);
    for (name, _) in &carried {
        if already.iter().any(|had| had.eq_ignore_ascii_case(name)) {
            continue;
        }
        match std::fs::read(dbc_dir.join(name)) {
            Ok(bytes) => {
                project
                    .write(&format!("{BEFORE_DIR}\\{name}"), &bytes)
                    .map_err(|e| format!("saving the server's original {name}: {e}"))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing.push(name.clone()),
            Err(e) => return Err(format!("{}: {e}", dbc_dir.join(name).display())),
        }
    }
    write_absent(project, &missing)?;

    std::fs::create_dir_all(dbc_dir).map_err(|e| format!("{}: {e}", dbc_dir.display()))?;
    for (name, vpath) in &carried {
        let Some(bytes) = project.read(vpath) else {
            continue;
        };
        let target = dbc_dir.join(name);
        if std::fs::read(&target).ok().as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        vale_edit::project::write_replacing(&target, &bytes)
            .map_err(|e| format!("{}: {e}", target.display()))?;
        out.written.push(name.clone());
    }
    Ok(out)
}

/// Return every saved file to the live folder and forget the saved copies.
pub fn put_back_at(project: &Project, dbc_dir: &Path) -> Result<Synced, String> {
    let mut out = Synced::default();
    for name in saved(project) {
        restore(project, dbc_dir, &name)?;
        out.restored.push(name);
    }
    Ok(out)
}

/// Return one file to the server's copy: write the saved bytes back, or
/// delete the file if the server had none, then forget the saved copy.
fn restore(project: &Project, dbc_dir: &Path, name: &str) -> Result<(), String> {
    let target = dbc_dir.join(name);
    let vpath = format!("{BEFORE_DIR}\\{name}");
    let mut missing = absent(project);
    if let Some(at) = missing.iter().position(|had| had.eq_ignore_ascii_case(name)) {
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", target.display())),
        }
        missing.remove(at);
        return write_absent(project, &missing);
    }
    let bytes = project
        .read(&vpath)
        .ok_or_else(|| format!("{vpath} is missing"))?;
    vale_edit::project::write_replacing(&target, &bytes)
        .map_err(|e| format!("{}: {e}", target.display()))?;
    project
        .revert(&vpath)
        .map_err(|e| format!("{vpath}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Place {
        root: std::path::PathBuf,
        project: Project,
        dbc: std::path::PathBuf,
    }

    fn place(tag: &str) -> Place {
        let root = std::env::temp_dir().join(format!("vale-dbcs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = Project::at(root.join("Edit").join("p")).unwrap();
        let dbc = root.join("server").join("dbc");
        std::fs::create_dir_all(&dbc).unwrap();
        Place { root, project, dbc }
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

    #[test]
    fn an_apply_saves_the_servers_copy_and_a_put_back_restores_it() {
        let p = place("roundtrip");
        std::fs::write(p.dbc.join("TaxiPath.dbc"), b"server's own").unwrap();
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"edited").unwrap();
        p.project.write(r"DBFilesClient\TaxiPathNode.dbc", b"new file").unwrap();
        assert!(standing(&p.project, &p.dbc).outstanding());

        let done = apply_at(&p.project, &p.dbc).unwrap();
        assert_eq!(done.written, ["TaxiPath.dbc", "TaxiPathNode.dbc"]);
        assert_eq!(std::fs::read(p.dbc.join("TaxiPath.dbc")).unwrap(), b"edited");
        let now = standing(&p.project, &p.dbc);
        assert!(!now.outstanding(), "{now:?}");
        assert_eq!(now.saved, 2);

        // A second apply writes nothing and keeps the first saved copy.
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"edited again").unwrap();
        apply_at(&p.project, &p.dbc).unwrap();
        assert_eq!(
            p.project.read(&format!("{BEFORE_DIR}\\TaxiPath.dbc")).unwrap(),
            b"server's own"
        );

        let back = put_back_at(&p.project, &p.dbc).unwrap();
        assert_eq!(back.restored.len(), 2);
        assert_eq!(std::fs::read(p.dbc.join("TaxiPath.dbc")).unwrap(), b"server's own");
        assert!(!p.dbc.join("TaxiPathNode.dbc").exists(), "the server had none");
        assert_eq!(saved(&p.project), Vec::<String>::new());
    }

    #[test]
    fn a_file_the_project_stops_carrying_is_put_back_on_the_next_apply() {
        let p = place("dropped");
        std::fs::write(p.dbc.join("TaxiPath.dbc"), b"server's own").unwrap();
        p.project.write(r"DBFilesClient\TaxiPath.dbc", b"edited").unwrap();
        apply_at(&p.project, &p.dbc).unwrap();
        p.project.revert(r"DBFilesClient\TaxiPath.dbc").unwrap();
        assert_eq!(standing(&p.project, &p.dbc).stale, ["TaxiPath.dbc"]);
        let done = apply_at(&p.project, &p.dbc).unwrap();
        assert_eq!(done.restored, ["TaxiPath.dbc"]);
        assert_eq!(std::fs::read(p.dbc.join("TaxiPath.dbc")).unwrap(), b"server's own");
    }

    #[test]
    fn the_saved_copies_are_not_packed_into_a_patch() {
        assert!(!Project::is_game_data(&format!("{BEFORE_DIR}\\TaxiPath.dbc")));
    }
}
