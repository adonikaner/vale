//! The project's log from the editor's side: the author this editor writes
//! as, the log of the open project, and the steps written to it.
//!
//! ## The author
//!
//! An author is a random id kept per install in `Edit\author.txt`, made the
//! first time it is needed, and the name in the Session popover's Name
//! field. Two people with the same name are two authors.
//!
//! ## Who writes the log
//!
//! The log is `vale_collab::log::Log` in the open project's `log\` folder.
//!
//! ```text
//! no session   this editor writes its own steps as it makes them
//! hosting      this editor writes every step the host accepts, its own and
//!              the joined editors', in the host's order
//! joined       nothing: the host writes
//! ```
//!
//! Alone, a held stroke is one step from press to release: the journal's
//! segment is not cut ten times a second as it is in a session, because no
//! other editor is watching it being drawn.
//!
//! ## What a stored step holds
//!
//! The [`Step`] and, for a file step, what each file held before it, which
//! the wire leaves out ([`super::files::File::before`]). A revert writes
//! those bytes back. For a joined editor's file step the host reads them from
//! its own copy before it applies the step: the host accepted the step, so no
//! other editor changed those files since the joined editor read them.

use super::Step;
use serde::{Deserialize, Serialize};
use vale_collab::log::{parse_hex, Author, AuthorId, Log, LOG_DIR};
use vale_collab::wire::{Answer, Request};
use vale_edit::project::Project;

/// The file under `Edit\` that holds this install's author id.
const AUTHOR_FILE: &str = "author.txt";

/// This install's author id, made and written the first time.
pub(crate) fn author_id(install: &str) -> AuthorId {
    let path = std::path::Path::new(install).join(vale_edit::project::PROJECTS_DIR).join(AUTHOR_FILE);
    if let Some(id) = std::fs::read_to_string(&path).ok().and_then(|text| parse_hex(text.trim())) {
        return id;
    }
    let id = vale_collab::log::new_author();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, format!("{}\n", vale_collab::log::hex(&id))) {
        bevy::log::warn!("collab: {} could not be written, so the author id lasts until the editor closes: {e}", path.display());
    }
    id
}

/// The author this editor writes as: the install's id and the name in the
/// Session popover's Name field, or `Editor` when the field is empty.
pub(crate) fn author(id: AuthorId, name: &str) -> Author {
    let name = name.trim();
    Author { id, name: if name.is_empty() { "Editor".to_string() } else { name.to_string() } }
}

/// The log of a project, made when it is not there.
pub(crate) fn open(project: &Project) -> std::io::Result<Log> {
    Log::open(project.root.join(LOG_DIR))
}

/// A step as the log stores it. See *What a stored step holds*.
#[derive(Serialize, Deserialize)]
struct Stored {
    step: Step,
    /// What each file of a file step held before it, in the step's order.
    before: Vec<Option<Vec<u8>>>,
}

/// [`Stored`] as it was written before a change carried `continues`, read so
/// a log written then still reads. Its changes are whole edits.
#[derive(Deserialize)]
struct StoredBefore {
    step: StepBefore,
    before: Vec<Option<Vec<u8>>>,
}

/// [`Step`] before a change carried `continues`, variant for variant.
#[derive(Deserialize)]
enum StepBefore {
    Edit { change: vale_edit::undo::Change, forward: bool },
    Files(super::files::Files),
    Delete(Vec<String>),
}

/// A step's bytes for the log.
pub(crate) fn encode(step: &Step) -> Vec<u8> {
    let before = match step {
        Step::Files(files) => files.files.iter().map(|file| file.before.clone()).collect(),
        _ => Vec::new(),
    };
    vale_collab::wire::encode(&Stored { step: step.clone(), before }).unwrap_or_default()
}

/// A step read back from the log, with what its files held before.
pub(crate) fn decode(bytes: &[u8]) -> Option<Step> {
    let Stored { mut step, before } = vale_collab::wire::decode(bytes).ok().or_else(|| {
        let StoredBefore { step, before } = vale_collab::wire::decode(bytes).ok()?;
        let step = match step {
            StepBefore::Edit { change, forward } => Step::Edit { change, forward, continues: false },
            StepBefore::Files(files) => Step::Files(files),
            StepBefore::Delete(paths) => Step::Delete(paths),
        };
        Some(Stored { step, before })
    })?;
    if let Step::Files(files) = &mut step {
        for (file, before) in files.files.iter_mut().zip(before) {
            file.before = before;
        }
    }
    Some(step)
}

/// Write one step to the log as edit `seq`, and say so in the status line
/// when it cannot be written.
pub(crate) fn append(log: &mut Log, seq: vale_collab::Seq, author: &Author, step: &Step, status: &mut String) {
    use vale_collab::Payload;
    if let Err(e) = log.append(seq, author, vale_collab::log::now(), step.groups(), step.continues(), &encode(step)) {
        *status = format!("the edit was made but not written to the project's log: {e}");
        bevy::log::warn!("collab: log: {e}");
    }
}

/// When each tile was last edited and each file last written, by edit number.
/// See `crate::import::generated::Touched`.
pub(crate) fn touched(log: &Log) -> crate::import::generated::Touched {
    let mut out = crate::import::generated::Touched::default();
    for header in log.headers() {
        for group in &header.groups {
            match group {
                vale_collab::Group::Chunk { tile, .. } | vale_collab::Group::Textures { tile } | vale_collab::Group::Placements { tile } => {
                    out.tiles.insert(tile.vpath().to_ascii_lowercase(), header.seq);
                }
                vale_collab::Group::File { path } => {
                    out.files.insert(path.clone(), header.seq);
                }
                _ => {}
            }
        }
    }
    out
}

/// The host's answer to a joined editor's request about the log. See
/// `vale_collab::wire::Request`. A Withdraw is answered by
/// `super::changes::withdraw_theirs`, which reads the editor's state as well
/// as the log, so it has none here.
pub(crate) fn answer(log: &mut Log, author: &Author, request: Request, status: &mut String) -> Option<Answer<Step>> {
    Some(match request {
        Request::Withdraw => return None,
        Request::Submit { description } => match log.close(author, vale_collab::log::now(), &description) {
            Ok(closed) => Answer::Submitted(closed),
            Err(e) => {
                *status = format!("a changelist of {} could not be written to the log: {e}", author.name);
                Answer::Submitted(None)
            }
        },
        Request::History => Answer::History { log: log.id(), changelists: log.changelists().to_vec() },
        Request::Stale { groups, after, log: id, except } => {
            let groups = match id == log.id() {
                true => log.stale(&groups, after, &except),
                false => groups,
            };
            Answer::Stale { groups, seq: log.last() }
        }
        Request::Edits(seqs) => {
            Answer::Edits(seqs.into_iter().filter_map(|seq| Some((seq, decode(&log.read(seq).ok()?)?))).collect())
        }
        Request::Open { everyone } => {
            let edits = log
                .headers()
                .iter()
                .filter(|header| header.changelist.is_none() && (everyone || header.author == author.id))
                .filter_map(|header| Some((header.seq, header.author, decode(&log.read(header.seq).ok()?)?)))
                .collect();
            Answer::Open { everyone, edits }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab::files::{File, Files};

    /// A file step stored in the log keeps what its files held before, which
    /// the wire's encoding drops.
    #[test]
    fn a_stored_file_step_keeps_its_bytes_before() {
        let step = Step::Files(Files {
            files: vec![File { path: r"Textures\A.blp".into(), bytes: Some(b"new".to_vec()), before: Some(b"old".to_vec()) }],
            imported: String::new(),
            forgotten: String::new(),
        });
        let Some(Step::Files(read)) = decode(&encode(&step)) else { panic!("not a file step") };
        assert_eq!(read.files[0].before.as_deref(), Some(&b"old"[..]));
        assert_eq!(read.files[0].bytes.as_deref(), Some(&b"new"[..]));
    }

    /// A step stored before a change carried `continues` reads as a whole
    /// edit.
    #[test]
    fn a_step_stored_before_continues_reads() {
        #[derive(Serialize)]
        enum Before {
            Edit { change: vale_edit::undo::Change, forward: bool },
        }
        #[derive(Serialize)]
        struct Old {
            step: Before,
            before: Vec<Option<Vec<u8>>>,
        }
        let change = vale_edit::undo::Change::new("Raise terrain");
        let bytes = vale_collab::wire::encode(&Old { step: Before::Edit { change, forward: false }, before: Vec::new() }).unwrap();
        let Some(Step::Edit { change, forward: false, continues: false }) = decode(&bytes) else { panic!("not read") };
        assert_eq!(change.label, "Raise terrain");
    }
}
