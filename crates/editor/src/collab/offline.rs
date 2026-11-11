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
//! ## The copy of the host's log
//!
//! While joined, the copy keeps a copy of the host's log in its own `log\`
//! folder, asked for once a second (`vale_collab::wire::Request::Tail`), with
//! the host's allocator of new numbers beside it. The link records the edit
//! the copy's files were fetched at and the edits that were open then, which
//! the files do not hold.
//!
//! ## Taking over
//!
//! A copy whose host is gone can host the project: Take Over removes the
//! link, keeps the copy of the log as the project's log, and opens the
//! project again. The edits the files may not hold are the edits after the
//! one fetched at and those open then, which `fetched.txt` names
//! ([`Fetched`]); they are applied again from the log
//! (`super::changes::recovered`), each an absolute state, so one the files
//! already hold changes nothing. Another author's open edits are kept out of
//! the files, as a host keeps them, and the copy's own pending edits are
//! applied and logged as this editor's. The other editors join the new host
//! at its address; their copies name the same log, so their offline edits are
//! checked against it as before. `fetched.txt` is removed by the next full
//! save, which leaves the files holding the submitted changelists.
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
use crate::session::EditSession;
use bevy::log::info;
use std::io::Write;
use vale_collab::log::{hex, parse_hex, LOG_DIR};
use vale_collab::wire::{read_frame, write_frame};
use vale_collab::Seq;
use vale_edit::project::Project;

const LINK_FILE: &str = "link.txt";
const PENDING_FILE: &str = "pending.bin";
/// The blocks of new numbers the host handed this copy. See
/// `crate::session::numbers`.
const BLOCKS_FILE: &str = "blocks.txt";
/// Which edits a project's files may not hold after a take over. See
/// *Taking over*.
const FETCHED_FILE: &str = "fetched.txt";

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
    /// The host's last edit when the copy's files were fetched.
    pub(crate) fetched: Seq,
    /// The edits that were in no changelist then, which the files do not
    /// hold.
    pub(crate) open: Vec<Seq>,
}

impl Link {
    /// The link of a project, if it has one.
    pub(crate) fn read(project: &Project) -> Option<Link> {
        let text = std::fs::read_to_string(project.root.join(LOG_DIR).join(LINK_FILE)).ok()?;
        let mut link = Link { address: String::new(), project: String::new(), log: [0; 16], synced: 0, fetched: 0, open: Vec::new() };
        for line in text.lines() {
            let Some((key, value)) = line.split_once(' ') else { continue };
            match key {
                "address" => link.address = value.trim().to_string(),
                "project" => link.project = value.trim().to_string(),
                "log" => link.log = parse_hex(value.trim())?,
                "synced" => link.synced = value.trim().parse().ok()?,
                "fetched" => link.fetched = value.trim().parse().ok()?,
                "open" => link.open = value.split(',').filter_map(|seq| seq.trim().parse().ok()).collect(),
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
            "# The host this project is a copy of, written by the world editor.\naddress {}\nproject {}\nlog {}\nsynced {}\nfetched {}\nopen {}\n",
            self.address,
            self.project,
            hex(&self.log),
            self.synced,
            self.fetched,
            self.open.iter().map(|seq| seq.to_string()).collect::<Vec<_>>().join(",")
        );
        std::fs::write(dir.join(LINK_FILE), text)
    }

    /// Remove a project's link and its pending edits, which makes it a
    /// project of its own.
    pub(crate) fn remove(project: &Project) -> std::io::Result<()> {
        let dir = project.root.join(LOG_DIR);
        for file in [LINK_FILE, PENDING_FILE, BLOCKS_FILE] {
            match std::fs::remove_file(dir.join(file)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }
}

/// Remove a copy's copy of the host's log, so a copy made a project of its
/// own starts a history of its own.
pub(crate) fn remove_log_copy(project: &Project) -> std::io::Result<()> {
    let dir = project.root.join(LOG_DIR);
    for file in ["id.txt", "edits.bin", "changelists.txt", "compacted.txt", super::log::NUMBERS_FILE] {
        match std::fs::remove_file(dir.join(file)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Which edits a taken-over project's files may not hold. See *Taking over*.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Fetched {
    pub(crate) seq: Seq,
    pub(crate) open: Vec<Seq>,
}

impl Fetched {
    /// Whether the files may not hold the edit `seq`.
    pub(crate) fn wants(&self, seq: Seq) -> bool {
        seq > self.seq || self.open.contains(&seq)
    }

    /// A project's record, if it has one.
    pub(crate) fn read(project: &Project) -> Option<Fetched> {
        let text = std::fs::read_to_string(project.root.join(LOG_DIR).join(FETCHED_FILE)).ok()?;
        let mut fetched = Fetched { seq: 0, open: Vec::new() };
        for line in text.lines() {
            let Some((key, value)) = line.split_once(' ') else { continue };
            match key {
                "fetched" => fetched.seq = value.trim().parse().ok()?,
                "open" => fetched.open = value.split(',').filter_map(|seq| seq.trim().parse().ok()).collect(),
                _ => {}
            }
        }
        Some(fetched)
    }

    pub(crate) fn write(&self, project: &Project) -> std::io::Result<()> {
        let dir = project.root.join(LOG_DIR);
        std::fs::create_dir_all(&dir)?;
        let open: Vec<String> = self.open.iter().map(|seq| seq.to_string()).collect();
        let text = format!(
            "# The edits this project's files may not hold, written by the world editor when it took over from its host.\nfetched {}\nopen {}\n",
            self.seq,
            open.join(",")
        );
        std::fs::write(dir.join(FETCHED_FILE), text)
    }

    /// Remove a project's record: a full save left its files holding every
    /// submitted changelist.
    pub(crate) fn remove(project: &Project) {
        let _ = std::fs::remove_file(project.root.join(LOG_DIR).join(FETCHED_FILE));
    }
}

/// While joined: ask for the host's log past what the copy holds, once a
/// second, one request at a time. See *The copy of the host's log*.
pub(crate) fn keep_the_log_copy(collab: &mut super::Collab, session: &EditSession, now: f64) {
    if !matches!(collab.state, super::State::Joined { .. }) || collab.link.is_none() || collab.tail_asked || now - collab.tail_at < 1.0 {
        return;
    }
    collab.tail_at = now;
    let (_, edits_from, changelists_from) = vale_collab::log::copy_lengths(&session.project.root.join(LOG_DIR));
    if let Some(running) = collab.session.as_mut() {
        running.request(vale_collab::wire::Request::Tail { edits_from, changelists_from });
        collab.tail_asked = true;
    }
}

/// The host's answer: append it to the copy of the log, keep the allocator,
/// and ask again at once when more is left.
pub(crate) fn kept_tail(collab: &mut super::Collab, session: &mut EditSession, tail: vale_collab::log::Tail) {
    collab.tail_asked = false;
    let dir = session.project.root.join(LOG_DIR);
    match vale_collab::log::replicate(&dir, &tail) {
        Ok(whole) => {
            if !whole {
                collab.tail_at = f64::NEG_INFINITY;
            }
        }
        Err(e) => session.status = format!("the copy of the host's log could not be written: {e}"),
    }
    if !tail.numbers.is_empty() {
        let _ = std::fs::write(dir.join(super::log::NUMBERS_FILE), &tail.numbers);
    }
}

/// Take over from a host that is gone: the copy becomes the project, with
/// the host's log. See *Taking over*.
pub(crate) fn take_over(collab: &mut super::Collab, session: &mut EditSession, assets: &vale_client::assets::GameAssets) {
    let Some(link) = collab.link.clone() else { return };
    let dir = session.project.root.join(LOG_DIR);
    let (id, edits, _) = vale_collab::log::copy_lengths(&dir);
    if id != Some(link.log) || edits == 0 {
        session.status = "this copy holds no copy of the host's log, so it cannot host the project: it has not been joined since copies kept one".to_string();
        return;
    }
    let pending = read(&session.project);
    let fetched = Fetched { seq: link.fetched, open: link.open.clone() };
    if let Err(e) = fetched.write(&session.project) {
        session.status = format!("the copy could not take over: {e}");
        return;
    }
    if let Err(e) = Link::remove(&session.project) {
        Fetched::remove(&session.project);
        session.status = format!("the copy's link could not be removed: {e}");
        return;
    }
    collab.taken_over = Some(pending);
    collab.link = None;
    collab.log = None;
    collab.log_for = None;
    collab.pending = 0;
    let name = session.project.name.clone();
    if !session.open_again(assets, &name) {
        return;
    }
    info!("collab: {name} took over from {}", link.address);
}

/// The blocks of new numbers a copy holds, none when it has none.
pub(crate) fn read_blocks(project: &Project) -> vale_collab::ids::Blocks {
    let text = std::fs::read_to_string(project.root.join(LOG_DIR).join(BLOCKS_FILE)).unwrap_or_default();
    vale_collab::ids::Blocks::parse(&text)
}

/// Write the blocks of new numbers a copy holds.
pub(crate) fn write_blocks(project: &Project, blocks: &vale_collab::ids::Blocks) -> std::io::Result<()> {
    let dir = project.root.join(LOG_DIR);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(BLOCKS_FILE), blocks.text())
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
        let link = Link {
            address: "192.168.1.20:7878".into(),
            project: "Goldshire".into(),
            log: [3; 16],
            synced: 412,
            fetched: 400,
            open: vec![398, 399],
        };
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
