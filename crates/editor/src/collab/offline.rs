//! A joined editor's copy of the host's project, linked to the host: the
//! link, the edits made on the copy while it is offline, and what happens to
//! them when it connects again.
//!
//! ## The link
//!
//! A join writes `log\link.txt` into the copy: the host's address, the
//! project's name on the host, the id of the host's log, and the last of the
//! host's edits the copy has. The number is written again every few seconds
//! while joined, and when the session ends.
//!
//! ## Offline
//!
//! A copy with a link and no session is offline. Its edits are not written to
//! a log, since the history is the host's. They are kept in
//! `log\pending.bin`, in the order made, until the copy connects again. A
//! connection that is lost leaves the copy open and offline, and so does
//! Work Offline in the Session popover; the edits the host had not answered
//! become pending too, since whether the host accepted them is not known.
//! The copy can also be opened from the project list later and edited
//! offline.
//!
//! ## Connecting again
//!
//! The join fetches the host's project into the copy as any join does, which
//! replaces the files the offline edits changed. The pending edits still hold
//! those changes. Once the copy is open again, the editor asks the host which
//! groups of the pending edits an edit after the link's number touched
//! (`vale_collab::wire::Request::Stale`). Each pending edit is then split:
//! the part in no such group is made over the host's state and sent, and the
//! rest is listed as a conflict in the Changes window. The pending file is
//! emptied once that is done. A host whose log is not the one the link names
//! makes every group stale.

use super::Step;
use std::io::Write;
use vale_collab::log::{hex, parse_hex, LOG_DIR};
use vale_collab::wire::{read_frame, write_frame};
use vale_collab::Seq;
use vale_edit::project::Project;

const LINK_FILE: &str = "link.txt";
const PENDING_FILE: &str = "pending.bin";

/// Where a copy came from. See *The link*.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Link {
    /// The host's address, as the Join form took it: `192.168.1.20:7878`.
    pub(crate) address: String,
    /// The project's name on the host.
    pub(crate) project: String,
    /// The id of the host's log.
    pub(crate) log: [u8; 16],
    /// The last of the host's edits this copy has.
    pub(crate) synced: Seq,
}

impl Link {
    /// The link of a project, if it has one.
    pub(crate) fn read(project: &Project) -> Option<Link> {
        let text = std::fs::read_to_string(project.root.join(LOG_DIR).join(LINK_FILE)).ok()?;
        let mut link = Link { address: String::new(), project: String::new(), log: [0; 16], synced: 0 };
        for line in text.lines() {
            let Some((key, value)) = line.split_once(' ') else { continue };
            match key {
                "address" => link.address = value.trim().to_string(),
                "project" => link.project = value.trim().to_string(),
                "log" => link.log = parse_hex(value.trim())?,
                "synced" => link.synced = value.trim().parse().ok()?,
                _ => {}
            }
        }
        (!link.address.is_empty()).then_some(link)
    }

    /// Write the link into a project.
    pub(crate) fn write(&self, project: &Project) -> std::io::Result<()> {
        let dir = project.root.join(LOG_DIR);
        std::fs::create_dir_all(&dir)?;
        let text = format!(
            "# The host this project is a copy of, written by the world editor.\naddress {}\nproject {}\nlog {}\nsynced {}\n",
            self.address,
            self.project,
            hex(&self.log),
            self.synced
        );
        std::fs::write(dir.join(LINK_FILE), text)
    }

    /// Remove a project's link and its pending edits, which makes it a
    /// project of its own.
    pub(crate) fn remove(project: &Project) -> std::io::Result<()> {
        let dir = project.root.join(LOG_DIR);
        for file in [LINK_FILE, PENDING_FILE] {
            match std::fs::remove_file(dir.join(file)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }
}

/// Add edits to a copy's pending edits.
pub(crate) fn append(project: &Project, steps: &[Step]) -> std::io::Result<()> {
    if steps.is_empty() {
        return Ok(());
    }
    let dir = project.root.join(LOG_DIR);
    std::fs::create_dir_all(&dir)?;
    let mut bytes = Vec::new();
    for step in steps {
        write_frame(&mut bytes, &super::log::encode(step))?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(dir.join(PENDING_FILE))?.write_all(&bytes)
}

/// A copy's pending edits, oldest first. A damaged last frame ends the list.
pub(crate) fn read(project: &Project) -> Vec<Step> {
    let Ok(bytes) = std::fs::read(project.root.join(LOG_DIR).join(PENDING_FILE)) else { return Vec::new() };
    let mut rest = bytes.as_slice();
    let mut out = Vec::new();
    while !rest.is_empty() {
        let Ok(frame) = read_frame::<Vec<u8>>(&mut rest) else { break };
        let Some(step) = super::log::decode(&frame) else { break };
        out.push(step);
    }
    out
}

/// Empty a copy's pending edits.
pub(crate) fn clear(project: &Project) -> std::io::Result<()> {
    match std::fs::remove_file(project.root.join(LOG_DIR).join(PENDING_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link and pending edits read back; removing the link removes both.
    #[test]
    fn a_link_and_pending_edits_read_back() {
        let install = std::env::temp_dir().join(format!("vale-collab-offline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = Project::open(&install, "Goldshire (shared)").unwrap();
        assert_eq!(Link::read(&project), None);
        let link = Link { address: "192.168.1.20:7878".into(), project: "Goldshire".into(), log: [3; 16], synced: 412 };
        link.write(&project).unwrap();
        assert_eq!(Link::read(&project), Some(link));

        let tile = vale_edit::TileKey::new("Azeroth", 32, 48);
        let change = vale_edit::undo::Change::from_entries(
            "Raise terrain",
            vec![vale_edit::undo::Entry::Tile(tile, vale_edit::ops::Edit::Area { chunk: 1, before: 1, after: 2 })],
        );
        append(&project, &[Step::Edit { change: change.clone(), forward: true, continues: false }]).unwrap();
        append(&project, &[Step::Edit { change, forward: false, continues: false }]).unwrap();
        let read = read(&project);
        assert_eq!(read.len(), 2);
        assert!(matches!(read[1], Step::Edit { forward: false, .. }));

        Link::remove(&project).unwrap();
        assert_eq!((Link::read(&project), super::read(&project).len()), (None, 0));
        let _ = std::fs::remove_dir_all(&install);
    }
}
