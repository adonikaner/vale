//! Changelists from the editor's side: Submit, Revert, and the conflicts a
//! revert or an offline submit leaves for a person to resolve. The window is
//! `crate::ui::shell::changes`.
//!
//! ## Submit and Save
//!
//! This editor's edits that are in no changelist are its open edits. Alone
//! or hosting, a save that leaves nothing unsaved puts them in a changelist
//! with the description typed ([`close_on_save`]), so a project worked on
//! alone has no separate Submit. Joined, the host saves, and Submit asks the
//! host to put them in a changelist ([`Request::Submit`]).
//!
//! ## Discard
//!
//! Discard takes back this editor's unsaved edits of a tile, a table, the
//! server rows or the whole project. Every save writes everything and
//! submits (`super`'s *What a save writes*), so those are its edits in no
//! changelist: read from the log alone or hosting, asked of the host when
//! joined ([`Request::Open`]), and the pending edits offline. Each is cut to
//! the subject's groups, inverted, and made as one new edit, newest first.
//! Every side of an edit is a whole state, so the subject ends as the files
//! hold it. Another author's edits are theirs, and Discard leaves them. The
//! new edit is logged, sent or kept like any other, so a discard in a session
//! reaches the other editors; the edits and their inverses then change
//! nothing and are withdrawn (*Edits that change nothing*). A file step is not
//! discarded, since it is written to the folder when it is made.
//!
//! ## Edits that change nothing
//!
//! Edits undone again, or discarded, leave their subjects as the folder
//! holds them, and they are not changes. Alone and hosting, once none of this
//! editor's subjects is unsaved, its open edits are withdrawn from the log
//! ([`withdraw_undone`]; `vale_collab::Log::withdraw`), so nothing counts
//! them, no changelist is made of them, and in a session what they claimed is
//! released. Only edits whose every subject is checked that way are
//! withdrawn: none that writes a file, and none on a map that is not open,
//! since those are written to the folder when they are made.
//!
//! A joined editor's edits are checked by the host when the editor asks
//! ([`withdraw_theirs`]): the author's unsubmitted changes are taken back on
//! copies of the tiles, tables and rows they touch, and when that gives what
//! the host holds now, they change nothing. Offline, the pending edits are
//! checked the same way over this editor's own state and emptied
//! ([`empty_pending_when_unchanged`]).
//!
//! ## Taking back another author's edits
//!
//! An author who is not connected can leave unsubmitted edits, which claim
//! what they touch. [`take_back`] closes them into a changelist named for
//! this editor and makes their inverses as one edit of this editor's, so the
//! folder, once saved, holds both and is net unchanged, and the history says
//! what happened. Their file steps were never written to the folder
//! (`super::files`'s *Another author's step on the host*), so they are
//! dropped.
//!
//! ## What a save writes after a Submit
//!
//! A guest's Submit makes their edits submitted, so the host writes their
//! staged files and saves ([`save_submitted`]). That save leaves out this
//! editor's own unsubmitted edits as well as the other authors', so it writes
//! exactly the submitted changelists, and does not submit this editor's
//! edits.
//!
//! ## What a changelist changed
//!
//! History shows a changelist's edits on request ([`describe`]): each edit's
//! name, then what it changed. A tile edit is counted by what it changed on
//! each tile; a client table field and a server column are shown before and
//! after; a record added or removed and a file written or removed are named.
//! Alone or hosting the edits are read from the log; joined, they are asked
//! of the host ([`Request::Edits`]).
//!
//! ## Recovered edits
//!
//! When the project's log is opened, its edits in no changelist are edits
//! the files do not hold: this editor's, which it closed without saving, and
//! every other author's, which a save leaves out. This editor's own file
//! steps are left out, since they were written when they were made. The
//! Changes window offers Restore, which applies them again unlogged (they are
//! in the log already), another author's files back into the overlay, and
//! Discard, which is the project's Discard.
//!
//! ## Revert
//!
//! A changelist is reverted by making the inverse of each of its edits as a
//! new edit of this editor's, newest first: a change applied the other way,
//! or a file step's files put back as they were before it. The new edits are
//! open edits like any other and are shared or logged as any other.
//!
//! Before that, the groups the changelist touched are checked against every
//! edit made after the changelist's first edit that is not part of it
//! ([`vale_collab::Log::stale`]; joined, [`Request::Stale`]). An inverse
//! written over such a group would undo the later edit as well. So each
//! inverse is split: the part in no stale group is made at once, and the part
//! in a stale group becomes a [`Conflict`].
//!
//! ## Conflicts
//!
//! A conflict is one subject, a tile, a table, a server row or a file, with
//! the steps this editor meant to make on it. The Changes window offers two
//! choices:
//!
//! ```text
//! Reapply       each step is rebased over the state now and made: its
//!               starting state is read from what the editor holds, so the
//!               subject ends as the step meant and the step can be undone
//! Keep Current  the steps are dropped
//! ```
//!
//! Reapply adds a height or vertex colour edit's change to the ground as it
//! is now and recomputes the shading, rather than writing the values the
//! step reached, so ground another editor changed since keeps its shape and
//! the chunks on either side of a border still meet. A step's edits of one
//! seam (heights and shading, colours, liquid, paint) are one conflict
//! however many tiles they cover ([`split`]), so Reapply and Keep Current
//! take all of a stroke's ground, colour, liquid or paint or none of it.
//!
//! A step that adds or removes a client table record cannot be rebased,
//! because the indexes of the records after it may have moved. Reapply
//! says so and leaves the conflict listed.
//!
//! A revert is not put on the undo stack. To take a revert back, revert the
//! changelist its edits are submitted in.

use super::{files, forget_overlapping, log, Collab, Session, State, Step};
use crate::session::EditSession;
use std::collections::HashMap;
use vale_client::assets::GameAssets;
use vale_collab::groups::entry_groups;
use vale_collab::log::Changelist;
use vale_collab::wire::{Answer, Request};
use vale_collab::log::AuthorId;
use vale_collab::{Group, Payload, Seq};
use vale_edit::undo::{Change, Entry};

/// The key of the server rows' subject, beside [`tile_key`] and
/// [`table_key`]. A subject is what Discard and the Changes window name: a
/// tile, a client table, or the project's server rows, which are saved
/// together.
pub(crate) const ROWS: &str = "rows";

/// A tile's subject key: `tile azeroth 32 48`.
pub(crate) fn tile_key(tile: &vale_edit::TileKey) -> String {
    format!("tile {} {} {}", tile.map.to_ascii_lowercase(), tile.x, tile.y)
}

/// A client table's subject key: `table spell`.
pub(crate) fn table_key(name: &str) -> String {
    format!("table {}", name.to_ascii_lowercase())
}

/// The subject key a group belongs to, or `None` for a file, which is written
/// as it is made.
pub(crate) fn subject_of(group: &Group) -> Option<String> {
    match group {
        Group::Chunk { tile, .. } | Group::Textures { tile } | Group::Placements { tile } => Some(tile_key(tile)),
        Group::Table { name } => Some(table_key(name)),
        Group::Row { .. } => Some(ROWS.to_string()),
        Group::File { .. } => None,
    }
}

/// Whether a step's groups are a file step's: those include a
/// [`Group::File`], and a change's never do.
pub(crate) fn is_files(groups: &[Group]) -> bool {
    groups.iter().any(|group| matches!(group, Group::File { .. }))
}

/// Which tab of the Changes window is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Changes,
    History,
    /// Where the project has been written. See `crate::server::targets`.
    Targets,
}

/// What the Changes window holds between frames.
#[derive(Default)]
pub(crate) struct Changes {
    pub(crate) tab: Tab,
    /// The description typed for the next Submit.
    pub(crate) description: String,
    /// The host's log id and changelists as last answered, for a joined
    /// editor. Alone and hosting, the window reads the editor's own log.
    pub(crate) history: Option<([u8; 16], Vec<Changelist>)>,
    /// Whether a joined editor has asked for the history since the window
    /// opened, so it is asked once per opening.
    pub(crate) asked: bool,
    /// A joined editor's revert, waiting for the host's answers.
    waiting: Option<Waiting>,
    /// The subjects left for a person to resolve, oldest first.
    pub(crate) conflicts: Vec<Conflict>,
    /// A joined editor's steps sent since its last Submit, for the window.
    pub(crate) unsubmitted: u64,
    /// The log's edits that the project's files do not hold, by number,
    /// found when the log was opened. See *Recovered edits*.
    pub(crate) recovered: Vec<Seq>,
    /// The changelist whose edits History shows, and the lines describing
    /// them, `None` while a joined editor waits for the host. See *What a
    /// changelist changed*.
    pub(crate) detail: Option<(u32, Option<Vec<String>>)>,
    /// What the Targets tab shows, and when it was read.
    pub(crate) targets: Option<(f64, crate::server::targets::Overview)>,
}

/// A request of a joined editor's, waiting for the host's answer.
enum Waiting {
    /// A revert: asked for the changelist's edits.
    Edits(Changelist),
    /// …then which of their groups are stale.
    Stale(Changelist, Vec<Step>),
    /// A copy back online: asked which groups of its pending edits are stale.
    /// See `super::offline`.
    Pending(Vec<Step>),
    /// The edits of a changelist History shows.
    Detail(u32),
    /// A Discard: asked for this editor's edits in no changelist, of one
    /// subject or, with `None`, of all.
    Discard(Option<String>),
}

/// One subject left for a person to resolve. See *Conflicts*.
pub(crate) struct Conflict {
    /// What it is about, for the window: `Azeroth 32_48`, `Spell.dbc`.
    pub(crate) subject: String,
    /// What made it: `Revert of changelist 12`.
    pub(crate) source: String,
    /// The steps this editor meant to make on the subject, in order.
    steps: Vec<Step>,
    key: String,
}

impl Conflict {
    /// How many steps it holds, for the window.
    pub(crate) fn len(&self) -> usize {
        self.steps.len()
    }
}

/// The subject a group belongs to: a key two groups share when they are
/// about the same thing, and the name the window shows.
fn subject(group: &Group) -> (String, String) {
    match group {
        Group::Chunk { tile, .. } | Group::Textures { tile } | Group::Placements { tile } => {
            (format!("tile {}", tile.vpath().to_ascii_lowercase()), format!("{} {}_{}", tile.map, tile.x, tile.y))
        }
        Group::Table { name } => (format!("table {}", name.to_ascii_lowercase()), format!("{name}.dbc")),
        Group::Row { table, key } => (format!("row {table} {key}"), format!("{table} {key}")),
        Group::File { path } => match vale_edit::TileKey::from_vpath(path) {
            Some(tile) => subject(&Group::Placements { tile }),
            None => (format!("file {path}"), path.clone()),
        },
    }
}

/// The groups of one entry, sorted.
fn groups_of(entry: &Entry) -> Vec<Group> {
    let mut out = Vec::new();
    entry_groups(entry, &mut out);
    out.sort();
    out.dedup();
    out
}

/// A step taken apart: the part touching any of `stale` and the part
/// touching none. A change is split by entry and a file step by file; an
/// entry or a file is in the first part when any of its groups is stale.
///
/// A change's entries of one seam (heights and shading, vertex colours,
/// liquid, paint) are never split: when any of them is in the first part,
/// all of them are. Two chunks store what is on their shared border once
/// each, and a stroke changes both copies, so half a stroke's heights leave a
/// step in the ground, and half its colours, liquid or paint a line, at the
/// border between the half made and the half not made. See
/// `vale_edit::ops::Edit::seam`.
pub(crate) fn split(step: &Step, stale: &[Group]) -> (Option<Step>, Option<Step>) {
    let hits = |groups: &[Group]| groups.iter().any(|group| stale.binary_search(group).is_ok());
    let seam = |entry: &Entry| match entry {
        Entry::Tile(_, edit) => edit.seam(),
        _ => None,
    };
    match step {
        Step::Edit { change, forward, continues } => {
            let seams: Vec<vale_edit::ops::Seam> =
                change.entries().iter().filter(|entry| hits(&groups_of(entry))).filter_map(seam).collect();
            let (inside, outside): (Vec<Entry>, Vec<Entry>) = change
                .entries()
                .iter()
                .cloned()
                .partition(|entry| hits(&groups_of(entry)) || seam(entry).is_some_and(|seam| seams.contains(&seam)));
            let part = |entries: Vec<Entry>| {
                (!entries.is_empty()).then(|| Step::Edit {
                    change: Change::from_entries(change.label.clone(), entries),
                    forward: *forward,
                    continues: *continues,
                })
            };
            (part(inside), part(outside))
        }
        Step::Files(step) => {
            let (inside, outside): (Vec<files::File>, Vec<files::File>) = step.files.iter().cloned().partition(|file| {
                let one = files::Files { files: vec![file.clone()], imported: String::new(), forgotten: String::new() };
                hits(&one.groups())
            });
            let part = |files: Vec<files::File>| {
                (!files.is_empty()).then(|| {
                    Step::Files(files::Files { files, imported: step.imported.clone(), forgotten: step.forgotten.clone() })
                })
            };
            (part(inside), part(outside))
        }
        Step::Delete(_) => (None, Some(step.clone())),
    }
}

/// A step taken apart by subject, each part with its subject's key and name.
fn by_subject(step: Step) -> Vec<(String, String, Step)> {
    let mut keys: Vec<(String, String)> = Vec::new();
    let mut groups: HashMap<String, Vec<Group>> = HashMap::new();
    for group in step.groups() {
        let (key, name) = subject(&group);
        if !groups.contains_key(&key) {
            keys.push((key.clone(), name));
        }
        groups.entry(key).or_default().push(group);
    }
    let mut out = Vec::new();
    let mut rest = Some(step);
    for (key, name) in keys {
        let Some(step) = rest.take() else { break };
        let mut wanted = groups.remove(&key).unwrap_or_default();
        wanted.sort();
        let (part, left) = split(&step, &wanted);
        if let Some(part) = part {
            out.push((key, name, part));
        }
        rest = left;
    }
    out
}

/// The step that takes back what `step` did: a change applied the other way,
/// or each file put back as it was. `None` for a delete request, which is
/// not an edit. The inverse is a whole edit; a caller that makes several as
/// one marks the later ones with [`Step::continuing`].
pub(crate) fn inverse(step: &Step) -> Option<Step> {
    match step {
        Step::Edit { change, forward, .. } => Some(Step::Edit { change: change.clone(), forward: !forward, continues: false }),
        Step::Files(files) => Some(Step::Files(files.backward())),
        Step::Delete(_) => None,
    }
}

/// A step rebased over what this editor holds now. See *Conflicts*. A file
/// step's bytes before are read again; a change's entries have their starting
/// states read from the tiles, tables and server rows, each tile entry over a
/// copy of the tile with the entries before it applied.
pub(crate) fn rebased(session: &mut EditSession, assets: &GameAssets, step: &Step) -> Result<Step, String> {
    match step {
        Step::Edit { change, forward, continues } => {
            Ok(Step::Edit { change: rebase(session, assets, change, *forward)?, forward: *forward, continues: *continues })
        }
        Step::Files(step) => {
            let mut step = step.clone();
            for file in &mut step.files {
                file.before = session.project.read(&file.path);
            }
            Ok(Step::Files(step))
        }
        Step::Delete(_) => Err("a delete request is not an edit".to_string()),
    }
}

fn rebase(session: &mut EditSession, assets: &GameAssets, change: &Change, forward: bool) -> Result<Change, String> {
    let mut scratch: HashMap<vale_edit::TileKey, vale_edit::adt::AdtFile> = HashMap::new();
    let mut out = Vec::new();
    for entry in change.in_order(forward) {
        out.push(match entry {
            Entry::Tile(key, edit) => {
                if !scratch.contains_key(key) {
                    let tile = current_tile(session, assets, key).ok_or_else(|| format!("{} could not be read", key.vpath()))?;
                    scratch.insert(key.clone(), tile);
                }
                let tile = scratch.get_mut(key).expect("inserted above");
                // Shading is recomputed from the heights rebased before it;
                // see `vale_edit::ops::Edit::rebased`.
                let rebased = match edit {
                    vale_edit::ops::Edit::Normals { .. } => edit.reshaded(forward, tile),
                    _ => edit.rebased(forward, tile),
                }
                .ok_or_else(|| format!("{} no longer has what the edit changed", key.vpath()))?;
                match forward {
                    true => rebased.apply(tile),
                    false => rebased.revert(tile),
                }
                Entry::Tile(key.clone(), rebased)
            }
            Entry::Cell(table, cell) => {
                session.open_table(assets, table);
                let now = session
                    .tables
                    .get(table)
                    .and_then(|file| file.u32_at(cell.record, cell.field))
                    .ok_or_else(|| format!("{table}.dbc has no field {} in record {}", cell.field, cell.record))?;
                let mut cell = cell.clone();
                match forward {
                    true => cell.before = now,
                    false => cell.after = now,
                }
                Entry::Cell(table.clone(), cell)
            }
            Entry::Row(table, _) => {
                return Err(format!(
                    "a record added to or removed from {table}.dbc cannot be reapplied over a change made since, because the records after it may have moved"
                ))
            }
            Entry::Server(server) => {
                let now = session.rows.current(server).ok_or_else(|| format!("{} {} is not a row this editor reads", server.table, server.key))?;
                let mut server = server.clone();
                match forward {
                    true => server.before = now,
                    false => server.after = now,
                }
                Entry::Server(server)
            }
        });
    }
    // `in_order` gave them in the order they are applied in. Stored in the
    // order made, so `in_order` gives the same order again; a change with a
    // client table record in it was refused above, and a server row claim
    // stays first either way.
    if !forward {
        out.reverse();
    }
    Ok(Change::from_entries(change.label.clone(), out))
}

/// A tile as this editor holds it now: the open map's from the session, any
/// other map's through the project's overlay over the archives.
fn current_tile(session: &mut EditSession, assets: &GameAssets, key: &vale_edit::TileKey) -> Option<vale_edit::adt::AdtFile> {
    if key.map.eq_ignore_ascii_case(&session.map) {
        let coord = (key.x, key.y);
        session.open(assets, coord);
        return session.tiles.get(&coord).cloned();
    }
    let path = key.vpath();
    let bytes = assets.with_archive(|chain| Ok(chain.read(&path).ok())).ok().flatten()?;
    vale_edit::adt::AdtFile::parse(&bytes).ok()
}

/// Make a step as one of this editor's own edits: apply it here, drop the undo
/// entries it writes over, and share it, or log it when alone.
pub(crate) fn make(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, step: Step) {
    let whole = !step.continues();
    match &step {
        Step::Edit { change, forward, .. } => {
            session.apply_step(assets, change, *forward);
        }
        Step::Files(files) => {
            if let Err(e) = files::apply(session, assets, files) {
                session.status = format!("{} could not be written: {e}", files.what());
            }
            collab.changed.extend(files.files.iter().map(|file| file.path.clone()));
        }
        Step::Delete(_) => return,
    }
    forget_overlapping(session, &step.groups());
    if collab.offline() {
        match super::offline::append(&session.project, std::slice::from_ref(&step)) {
            Ok(()) => collab.pending += usize::from(whole),
            Err(e) => session.status = format!("an edit made offline could not be kept to send to the host: {e}"),
        }
        return;
    }
    match collab.state {
        State::Off | State::Ended(_) => {
            let author = collab.author(&assets.root);
            if let Some(log) = collab.log.as_mut() {
                let seq = log.last() + 1;
                log::append(log, seq, &author, &step, &mut session.status);
            }
        }
        _ => {
            if let Some(running) = collab.session.as_mut() {
                running.propose(step);
                collab.sent += u64::from(whole);
                collab.changes.unsubmitted += u64::from(whole);
            }
        }
    }
}

/// Put this editor's open edits in a changelist. See *Submit*.
pub(crate) fn submit(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    let description = collab.changes.description.trim().to_string();
    if collab.is_joined() {
        if let Some(running) = collab.session.as_mut() {
            running.request(Request::Submit { description });
            session.status = "submitting to the host".to_string();
        }
        return;
    }
    let author = collab.author(&assets.root);
    let Some(log) = collab.log.as_mut() else {
        session.status = "this project has no log to submit to".to_string();
        return;
    };
    match log.close(&author, vale_collab::log::now(), &description) {
        Ok(Some(closed)) => {
            submitted(collab, session, Some(closed));
            // A Submit releases the author's check outs.
            if let Some(Session::Hosting(hosting)) = collab.session.as_mut() {
                for action in hosting.release_author(&author.id) {
                    if let vale_collab::Action::Holds(list) = action {
                        super::holds_changed(collab, session, list);
                    }
                }
            }
        }
        Ok(None) => {
            session.status = "nothing to submit: every edit of this editor's is in a changelist".to_string();
            // A Submit with nothing to submit still ends the check outs.
            if let Some(Session::Hosting(hosting)) = collab.session.as_mut() {
                for action in hosting.release_author(&author.id) {
                    if let vale_collab::Action::Holds(list) = action {
                        super::holds_changed(collab, session, list);
                    }
                }
            }
        }
        Err(e) => session.status = format!("the changelist could not be written to the log: {e}"),
    }
}

fn submitted(collab: &mut Collab, session: &mut EditSession, closed: Option<Changelist>) {
    match closed {
        Some(closed) => {
            let edits = super::counted(closed.whole_edits, "edit", "edits");
            session.status = format!("submitted changelist {} with {edits}", closed.number);
            collab.changes.description.clear();
            collab.changes.unsubmitted = 0;
            if let Some((_, history)) = collab.changes.history.as_mut() {
                history.push(closed);
            }
        }
        None => session.status = "nothing to submit: every edit of this editor's is in a changelist".to_string(),
    }
}

/// Revert changelist `number`. See *Revert*.
pub(crate) fn revert(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, number: u32) {
    if collab.is_joined() {
        let Some(changelist) = collab.changes.history.as_ref().and_then(|(_, list)| list.iter().find(|cl| cl.number == number)).cloned()
        else {
            return;
        };
        if let Some(running) = collab.session.as_mut() {
            running.request(Request::Edits(changelist.edits.clone()));
            collab.changes.waiting = Some(Waiting::Edits(changelist));
            session.status = format!("asking the host for changelist {number}");
        }
        return;
    }
    let Some(log) = collab.log.as_ref() else { return };
    let Some(changelist) = log.changelist(number).cloned() else { return };
    let mut steps = Vec::new();
    for seq in &changelist.edits {
        match log.read(*seq).ok().and_then(|bytes| log::decode(&bytes)) {
            Some(step) => steps.push(step),
            None => {
                session.status = format!("changelist {number} was not reverted: edit {seq} could not be read from the log");
                return;
            }
        }
    }
    let stale = log.stale(&all_groups(&steps), after(&changelist), &changelist.edits);
    carry_out(collab, session, assets, &changelist, steps, stale);
}

/// The groups every step touches, sorted, each once.
fn all_groups(steps: &[Step]) -> Vec<Group> {
    let mut groups: Vec<Group> = steps.iter().flat_map(Payload::groups).collect();
    groups.sort();
    groups.dedup();
    groups
}

/// The number before a changelist's first edit: an edit after it that is not
/// in the changelist may have touched what the changelist touched.
fn after(changelist: &Changelist) -> Seq {
    changelist.edits.first().map_or(0, |first| first.saturating_sub(1))
}

/// Make the inverse of each step, newest first: the part in no stale group at
/// once, the rest as conflicts. The parts made at once are one edit.
fn carry_out(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, changelist: &Changelist, steps: Vec<Step>, stale: Vec<Group>) {
    let source = format!("Revert of changelist {}", changelist.number);
    let before = collab.changes.conflicts.len();
    let mut made = false;
    for step in steps.iter().rev() {
        let Some(inverse) = inverse(step) else { continue };
        let (conflicted, clean) = split(&inverse, &stale);
        if let Some(clean) = clean {
            make(collab, session, assets, clean.continuing(made));
            made = true;
        }
        if let Some(conflicted) = conflicted {
            add_conflicts(collab, &source, conflicted);
        }
    }
    if collab.changes.description.trim().is_empty() {
        collab.changes.description = match changelist.description.is_empty() {
            true => format!("Revert changelist {}", changelist.number),
            false => format!("Revert changelist {}: {}", changelist.number, changelist.description),
        };
    }
    session.status = match collab.changes.conflicts.len() - before {
        0 => format!("reverted changelist {}", changelist.number),
        n => format!("reverted changelist {} except {n} subjects changed since; resolve them in the Changes window", changelist.number),
    };
}

/// Add a step's parts to the conflicts, one per subject, joining a conflict
/// already listed for the same subject and source.
pub(crate) fn add_conflicts(collab: &mut Collab, source: &str, step: Step) {
    for (key, name, part) in by_subject(step) {
        match collab.changes.conflicts.iter_mut().find(|conflict| conflict.key == key && conflict.source == source) {
            Some(conflict) => conflict.steps.push(part),
            None => collab.changes.conflicts.push(Conflict { subject: name, source: source.to_string(), steps: vec![part], key }),
        }
    }
}

/// Reapply conflict `at`: rebase each step over the state now and make
/// it. A step that cannot be rebased leaves the conflict listed with what is
/// left of it.
pub(crate) fn reapply(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, at: usize) {
    if at >= collab.changes.conflicts.len() {
        return;
    }
    let mut conflict = collab.changes.conflicts.remove(at);
    let steps = std::mem::take(&mut conflict.steps);
    for (done, step) in steps.iter().enumerate() {
        match rebased(session, assets, step) {
            Ok(step) => make(collab, session, assets, step),
            Err(e) => {
                session.status = format!("{} was not reapplied: {e}", conflict.subject);
                conflict.steps = steps[done..].to_vec();
                collab.changes.conflicts.insert(at, conflict);
                return;
            }
        }
    }
    session.status = format!("reapplied the edits to {}", conflict.subject);
}

/// Keep the current version for conflict `at`: drop its steps.
pub(crate) fn keep_current(collab: &mut Collab, session: &mut EditSession, at: usize) {
    if at < collab.changes.conflicts.len() {
        let conflict = collab.changes.conflicts.remove(at);
        session.status = format!("kept the current version of {}", conflict.subject);
    }
}

/// The host's answers to this editor's requests, as they arrive.
pub(crate) fn answered(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    for answer in std::mem::take(&mut collab.answers) {
        match answer {
            Answer::Submitted(closed) => submitted(collab, session, closed),
            Answer::History { log, changelists } => collab.changes.history = Some((log, changelists)),
            Answer::Edits(edits) if matches!(collab.changes.waiting, Some(Waiting::Detail(_))) => {
                let Some(Waiting::Detail(number)) = collab.changes.waiting.take() else { continue };
                let lines = describe_all(edits.into_iter().map(|(_, step)| step).collect());
                if collab.changes.detail.as_ref().is_some_and(|(shown, _)| *shown == number) {
                    collab.changes.detail = Some((number, Some(lines)));
                }
            }
            Answer::Edits(edits) => {
                let Some(Waiting::Edits(changelist)) = collab.changes.waiting.take() else { continue };
                if edits.len() != changelist.edits.len() {
                    session.status = format!("changelist {} was not reverted: the host could not read all its edits", changelist.number);
                    continue;
                }
                let steps: Vec<Step> = edits.into_iter().map(|(_, step)| step).collect();
                let log = collab.changes.history.as_ref().map_or([0; 16], |(log, _)| *log);
                let request = Request::Stale { groups: all_groups(&steps), after: after(&changelist), log, except: changelist.edits.clone() };
                if let Some(running) = collab.session.as_mut() {
                    running.request(request);
                    collab.changes.waiting = Some(Waiting::Stale(changelist, steps));
                }
            }
            Answer::Stale { groups, .. } => match collab.changes.waiting.take() {
                Some(Waiting::Stale(changelist, steps)) => carry_out(collab, session, assets, &changelist, steps, groups),
                Some(Waiting::Pending(steps)) => pending_sent(collab, session, assets, steps, groups),
                other => collab.changes.waiting = other,
            },
            // Every author's, for an editor that has just joined: applied
            // before the edits held since the welcome. See `super`'s
            // *Joining*.
            Answer::Open { everyone: true, edits } => {
                for (_, _, step) in edits {
                    super::apply_open(session, assets, &step);
                }
                collab.open_wanted = false;
                for (user, step) in std::mem::take(&mut collab.held) {
                    super::apply_remote(collab, session, assets, user, &step);
                }
            }
            Answer::Open { everyone: false, edits } => {
                let Some(Waiting::Discard(wanted)) = collab.changes.waiting.take() else { continue };
                let me = collab.author_id.unwrap_or_default();
                let steps = edits
                    .into_iter()
                    .filter(|(_, author, step)| *author == me && !matches!(step, Step::Files(_)))
                    .map(|(_, _, step)| step)
                    .collect();
                discard_steps(collab, session, assets, steps, wanted);
            }
            Answer::Withdrawn => {
                collab.changes.unsubmitted = 0;
                session.status = "this editor's unsubmitted edits change nothing, and the host withdrew them".to_string();
            }
        }
    }
}

/// A copy back online: ask the host which groups of its pending edits an edit
/// after `after` in log `log` touched. See `super::offline`.
pub(crate) fn send_pending(collab: &mut Collab, steps: Vec<Step>, after: Seq, log: [u8; 16]) {
    let request = Request::Stale { groups: all_groups(&steps), after, log, except: Vec::new() };
    if let Some(running) = collab.session.as_mut() {
        running.request(request);
        collab.changes.waiting = Some(Waiting::Pending(steps));
    }
}

/// The host answered which groups of the pending edits are stale: make the
/// rest of each edit over the host's state, list the stale parts as
/// conflicts, and empty the pending file.
fn pending_sent(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, steps: Vec<Step>, stale: Vec<Group>) {
    let before = collab.changes.conflicts.len();
    let count = steps.len();
    for step in steps {
        let (conflicted, clean) = split(&step, &stale);
        if let Some(clean) = clean {
            make(collab, session, assets, clean);
        }
        if let Some(conflicted) = conflicted {
            add_conflicts(collab, "Offline edits", conflicted);
        }
    }
    if let Err(e) = super::offline::clear(&session.project) {
        session.status = format!("the pending edits were sent but their file could not be emptied: {e}");
        return;
    }
    collab.pending = 0;
    let edits = match count {
        1 => "1 edit".to_string(),
        n => format!("{n} edits"),
    };
    session.status = match collab.changes.conflicts.len() - before {
        0 => format!("sent the {edits} made offline; submit them in the Changes window"),
        n => format!("sent the {edits} made offline except {n} subjects changed since; resolve them in the Changes window"),
    };
}

/// Put this editor's open edits in a changelist after a save that left
/// nothing unsaved, with the description typed, and release its check outs.
/// See *Submit and Save*.
pub(crate) fn close_on_save(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    let author = collab.author(&assets.root);
    let description = collab.changes.description.trim().to_string();
    let Some(log) = collab.log.as_mut() else { return };
    if log.open_count(&author.id) == 0 {
        return;
    }
    match log.close(&author, vale_collab::log::now(), &description) {
        Ok(Some(closed)) => {
            let saved = std::mem::take(&mut session.status);
            submitted(collab, session, Some(closed));
            session.status = match saved.is_empty() {
                true => session.status.clone(),
                false => format!("{saved}; {}", session.status),
            };
            if let Some(Session::Hosting(hosting)) = collab.session.as_mut() {
                for action in hosting.release_author(&author.id) {
                    if let vale_collab::Action::Holds(list) = action {
                        super::holds_changed(collab, session, list);
                    }
                }
            }
        }
        Ok(None) => {}
        Err(e) => session.status = format!("the changelist could not be written to the log: {e}"),
    }
}

/// This editor's unsaved edits, oldest first, file steps left out: its edits
/// in no changelist from the log alone or hosting, the pending edits
/// offline. See *Discard*.
fn own_unsaved(collab: &Collab, session: &EditSession) -> Vec<Step> {
    if collab.offline() {
        return super::offline::read(&session.project).into_iter().filter(|step| !matches!(step, Step::Files(_))).collect();
    }
    own_open(collab).into_iter().map(|excluded| Step::Edit { change: excluded.change, forward: excluded.forward, continues: false }).collect()
}

/// Discard this editor's unsaved edits of `wanted`, a subject key, or of
/// every subject when `None`. See *Discard*.
pub(crate) fn discard(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, wanted: Option<String>) {
    if collab.is_joined() {
        if collab.changes.waiting.is_some() {
            session.status = "the host has not answered an earlier request yet".to_string();
            return;
        }
        if let Some(running) = collab.session.as_mut() {
            running.request(Request::Open { everyone: false });
            collab.changes.waiting = Some(Waiting::Discard(wanted));
            session.status = "asking the host for this editor's unsubmitted edits".to_string();
        }
        return;
    }
    let steps = own_unsaved(collab, session);
    discard_steps(collab, session, assets, steps, wanted);
}

/// Make the inverses of `steps`, newest first and cut to `wanted`'s groups,
/// as one edit, and mark what is left as the files hold it. See *Discard*.
fn discard_steps(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, steps: Vec<Step>, wanted: Option<String>) {
    let mut count = 0;
    let mut made = false;
    for step in steps.iter().rev() {
        let mut groups: Vec<Group> = step
            .groups()
            .into_iter()
            .filter(|group| subject_of(group).is_some_and(|key| wanted.as_ref().is_none_or(|wanted| *wanted == key)))
            .collect();
        groups.sort();
        if groups.is_empty() {
            continue;
        }
        let (part, _) = split(step, &groups);
        if let Some(inverse) = part.as_ref().and_then(inverse) {
            make(collab, session, assets, inverse.continuing(made));
            made = true;
            count += 1;
        }
    }
    match &wanted {
        None => {
            session.unsaved.clear();
            session.unsaved_tables.clear();
            mark_rows_saved(session);
            collab.changes.recovered.clear();
        }
        Some(key) if key == ROWS => mark_rows_saved(session),
        Some(key) => {
            let map = session.map.clone();
            session.unsaved.retain(|(x, y)| tile_key(&vale_edit::TileKey::new(&map, *x, *y)) != *key);
            session.unsaved_tables.retain(|name| table_key(name) != *key);
        }
    }
    // The edits and their inverses change nothing now. See *Edits that
    // change nothing*.
    if made && collab.is_joined() {
        if let Some(running) = collab.session.as_mut() {
            running.request(Request::Withdraw);
        }
    }
    if made && collab.offline() {
        empty_pending_when_unchanged(collab, session, assets);
    }
    session.status = match count {
        0 => "nothing to discard".to_string(),
        1 => "discarded 1 unsaved edit".to_string(),
        n => format!("discarded {n} unsaved edits"),
    };
}

/// The server rows' stores are as their files hold them.
fn mark_rows_saved(session: &mut EditSession) {
    session.rows.edits_unsaved = false;
    session.rows.paths_unsaved = false;
    session.rows.scripts_unsaved = false;
}

/// The log's edits the files do not hold: every edit in no changelist but
/// this editor's own file steps. `me` is this editor's author. See
/// *Recovered edits*.
pub(crate) fn recovered(log: &vale_collab::Log, me: &AuthorId) -> Vec<Seq> {
    log.headers()
        .iter()
        .filter(|header| header.changelist.is_none() && (header.author != *me || !is_files(&header.groups)))
        .map(|header| header.seq)
        .collect()
}

/// Apply the recovered edits again, in order, unlogged: the log has them. The
/// subjects of this editor's are then unsaved, as they were before the
/// editor closed; another author's stay out of the folder
/// (`EditSession::exclude`, `EditSession::staged`).
pub(crate) fn restore_recovered(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    let me = collab.author_id.unwrap_or_default();
    let Some(log) = collab.log.as_ref() else { return };
    let mut edits = Vec::new();
    for seq in &collab.changes.recovered {
        let Some(header) = log.header(*seq) else { continue };
        let Some(step) = log.read(*seq).ok().and_then(|bytes| log::decode(&bytes)) else { continue };
        edits.push((*seq, header.author, step));
    }
    let count = super::whole_edits(edits.iter().map(|(_, _, step)| step));
    for (seq, author, step) in edits {
        let theirs = author != me;
        match step {
            Step::Edit { change, forward, .. } => {
                session.apply_step(assets, &change, forward);
                if theirs {
                    unmark_theirs(collab, session, &change.groups());
                    session.exclude.push(crate::session::Excluded { seq, author, change, forward });
                }
            }
            Step::Files(files) if theirs => super::files::stage(session, assets, author, &files),
            _ => {}
        }
    }
    collab.changes.recovered.clear();
    session.status = format!("restored {} from the project's log; save to keep them", super::counted(count, "edit", "edits"));
}

/// This editor's unsubmitted edits, as changes to leave out of a save.
fn own_open(collab: &Collab) -> Vec<crate::session::Excluded> {
    let (Some(log), Some(me)) = (collab.log.as_ref(), collab.author_id) else { return Vec::new() };
    log.open_edits(&me)
        .into_iter()
        .filter_map(|seq| match log.read(seq).ok().and_then(|bytes| log::decode(&bytes))? {
            Step::Edit { change, forward, .. } => Some(crate::session::Excluded { seq, author: me, change, forward }),
            _ => None,
        })
        .collect()
}

/// Every group this editor's unsubmitted edits touch, files left out.
pub(crate) fn own_open_groups(collab: &Collab) -> Vec<Group> {
    let (Some(log), Some(me)) = (collab.log.as_ref(), collab.author_id) else { return Vec::new() };
    log.open_edits(&me)
        .into_iter()
        .filter_map(|seq| log.header(seq))
        .flat_map(|header| header.groups.iter().filter(|group| subject_of(group).is_some()).cloned())
        .collect()
}

/// Mark the subjects of `groups` unsaved: tiles of the open map, client
/// tables and the server rows.
pub(crate) fn mark_unsaved(session: &mut EditSession, groups: &[Group]) {
    for group in groups {
        match group {
            Group::Chunk { tile, .. } | Group::Textures { tile } | Group::Placements { tile } => {
                if tile.map.eq_ignore_ascii_case(&session.map) {
                    session.unsaved.insert((tile.x, tile.y));
                }
            }
            Group::Table { name } => {
                session.unsaved_tables.insert(name.clone());
            }
            Group::Row { .. } => session.rows.edits_unsaved = true,
            Group::File { .. } => {}
        }
    }
}

/// The subjects of `groups`, from another author's edit, marked saved again
/// unless this editor's own unsubmitted edits touch them: the folder holds
/// what this editor can save of them already.
pub(crate) fn unmark_theirs(collab: &Collab, session: &mut EditSession, groups: &[Group]) {
    let own: std::collections::HashSet<String> = own_open_groups(collab).iter().filter_map(subject_of).collect();
    let theirs = |group: &Group| subject_of(group).is_some_and(|key| !own.contains(&key));
    for group in groups.iter().filter(|group| theirs(group)) {
        match group {
            Group::Chunk { tile, .. } | Group::Textures { tile } | Group::Placements { tile } => {
                if tile.map.eq_ignore_ascii_case(&session.map) {
                    session.unsaved.remove(&(tile.x, tile.y));
                }
            }
            Group::Table { name } => {
                session.unsaved_tables.remove(name);
            }
            Group::Row { .. } => {
                if !own.contains(ROWS) {
                    mark_rows_saved(session);
                }
            }
            Group::File { .. } => {}
        }
    }
}

/// Another author submitted: their edits are submitted now, so the host
/// writes their staged files and saves, leaving this editor's own
/// unsubmitted edits out. See *What a save writes after a Submit*.
pub(crate) fn save_submitted(collab: &Collab, session: &mut EditSession, assets: &GameAssets, author: &AuthorId) {
    session.exclude.retain(|excluded| excluded.author != *author);
    if let Err(e) = super::files::unstage(session, assets, author, true) {
        session.status = format!("a submitted file could not be written to the project: {e}");
    }
    session.exclude_once = own_open(collab);
    session.save_asked = true;
}

/// Hosting: withdraw `author`'s edits in no changelist when they change
/// nothing, and release what they claimed. Answers whether they were. See
/// *Edits that change nothing*.
pub(crate) fn withdraw_theirs(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, author: &vale_collab::log::Author) -> bool {
    if !matches!(collab.state, State::Hosting { .. }) {
        return false;
    }
    let Some(log) = collab.log.as_ref() else { return false };
    let open = log.open_edits(&author.id);
    if open.is_empty() || open.iter().filter_map(|seq| log.header(*seq)).any(|header| is_files(&header.groups)) {
        return false;
    }
    // Every open edit must be applied here, or the check would be of part of
    // them.
    let theirs: Vec<&crate::session::Excluded> = session.exclude.iter().filter(|excluded| excluded.author == author.id).collect();
    if !open.iter().all(|seq| theirs.iter().any(|excluded| excluded.seq == *seq)) {
        return false;
    }
    let changes: Vec<(Change, bool)> = theirs.iter().map(|excluded| (excluded.change.clone(), excluded.forward)).collect();
    if !nets_to_nothing(session, assets, &changes) {
        return false;
    }
    let Some(log) = collab.log.as_mut() else { return false };
    if let Err(e) = log.withdraw(author, vale_collab::log::now()) {
        session.status = format!("{}'s undone edits could not be marked in the log: {e}", author.name);
        return false;
    }
    session.exclude.retain(|excluded| excluded.author != author.id);
    let actions = match collab.session.as_mut() {
        Some(Session::Hosting(hosting)) => hosting.release_pending(&author.id),
        _ => Vec::new(),
    };
    for action in actions {
        if let vale_collab::Action::Holds(list) = action {
            super::holds_changed(collab, session, list);
        }
    }
    true
}

/// Offline: empty the pending edits when together they change nothing. See
/// *Edits that change nothing*.
pub(crate) fn empty_pending_when_unchanged(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    let pending = super::offline::read(&session.project);
    if pending.is_empty() || pending.iter().any(|step| !matches!(step, Step::Edit { .. })) {
        return;
    }
    let changes: Vec<(Change, bool)> = pending
        .into_iter()
        .filter_map(|step| match step {
            Step::Edit { change, forward, .. } => Some((change, forward)),
            _ => None,
        })
        .collect();
    if !nets_to_nothing(session, assets, &changes) {
        return;
    }
    match super::offline::clear(&session.project) {
        Ok(()) => collab.pending = 0,
        Err(e) => session.status = format!("the pending edits change nothing, but their file could not be emptied: {e}"),
    }
}

/// Whether `changes`, each with its direction and in the order made, change
/// nothing: every tile and table they touch, with them taken back newest
/// first on a copy, is what this editor holds now, and every server row ends
/// as it started.
fn nets_to_nothing(session: &mut EditSession, assets: &GameAssets, changes: &[(Change, bool)]) -> bool {
    let mut tiles: Vec<vale_edit::TileKey> = Vec::new();
    let mut tables: Vec<String> = Vec::new();
    for (change, _) in changes {
        for tile in change.tiles() {
            if !tiles.contains(&tile) {
                tiles.push(tile);
            }
        }
        for table in change.tables() {
            if !tables.contains(&table) {
                tables.push(table);
            }
        }
    }
    for key in &tiles {
        let Some(now) = current_tile(session, assets, key) else { return false };
        let mut copy = now.clone();
        for (change, forward) in changes.iter().rev() {
            match forward {
                true => change.revert(key, &mut copy),
                false => change.apply(key, &mut copy),
            }
        }
        if copy.write() != now.write() {
            return false;
        }
    }
    for name in &tables {
        session.open_table(assets, name);
        let Some(now) = session.tables.get(name) else { return false };
        let mut copy = now.clone();
        for (change, forward) in changes.iter().rev() {
            match forward {
                true => change.revert_table(name, &mut copy),
                false => change.apply_table(name, &mut copy),
            }
        }
        if copy.write() != now.write() {
            return false;
        }
    }
    // A server row ends as it started when the first value each entry found
    // is the last one written.
    let mut rows: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for (change, forward) in changes {
        for entry in change.server_steps(*forward) {
            let key = format!("{:?}\t{}\t{}", entry.shape, entry.table, entry.key);
            match rows.iter_mut().find(|(had, _, _)| *had == key) {
                Some((_, _, last)) => *last = entry.after,
                None => rows.push((key, entry.before, entry.after)),
            }
        }
    }
    rows.iter().all(|(_, first, last)| first == last)
}

/// Withdraw this editor's open edits when they change nothing. See *Edits
/// that change nothing*.
pub(crate) fn withdraw_undone(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets) {
    if collab.is_joined() || collab.offline() || session.save_asked || session.history.segment().is_some() {
        return;
    }
    if !session.unsaved.is_empty() || !session.unsaved_tables.is_empty() || session.rows.unsaved() {
        return;
    }
    let Some(me) = collab.author_id else { return };
    let Some(log) = collab.log.as_ref() else { return };
    let open = log.open_edits(&me);
    if open.is_empty() {
        return;
    }
    let map = session.map.clone();
    let checked = |group: &Group| match group {
        Group::Chunk { tile, .. } | Group::Textures { tile } | Group::Placements { tile } => tile.map.eq_ignore_ascii_case(&map),
        Group::Table { .. } | Group::Row { .. } => true,
        Group::File { .. } => false,
    };
    if !open.iter().filter_map(|seq| log.header(*seq)).all(|header| header.groups.iter().all(checked)) {
        return;
    }
    let author = collab.author(&assets.root);
    let Some(log) = collab.log.as_mut() else { return };
    if let Err(e) = log.withdraw(&author, vale_collab::log::now()) {
        session.status = format!("the undone edits could not be marked in the log: {e}");
        return;
    }
    // The host's own list changes with the joined editors': the claims the
    // withdrawn edits held come back as an action for this editor too.
    let actions = match collab.session.as_mut() {
        Some(Session::Hosting(hosting)) => hosting.release_pending(&author.id),
        _ => Vec::new(),
    };
    for action in actions {
        if let vale_collab::Action::Holds(list) = action {
            super::holds_changed(collab, session, list);
        }
    }
}

/// Take back the unsubmitted edits of an author who is not connected. See
/// *Taking back another author's edits*.
pub(crate) fn take_back(collab: &mut Collab, session: &mut EditSession, assets: &GameAssets, author: AuthorId) {
    if collab.is_joined() {
        return;
    }
    if collab.connected(&author) {
        session.status = "an editor in the session submits or takes back their own edits".to_string();
        return;
    }
    let me = collab.author(&assets.root);
    let Some(log) = collab.log.as_mut() else { return };
    let name = log.name_of(&author).unwrap_or("another editor").to_string();
    let description = format!("Taken back by {}", me.name);
    let closed = match log.close(&vale_collab::log::Author { id: author, name: name.clone() }, vale_collab::log::now(), &description) {
        Ok(Some(closed)) => closed,
        Ok(None) => {
            session.status = format!("{name} has no unsubmitted edits");
            return;
        }
        Err(e) => {
            session.status = format!("{name}'s edits could not be closed in the log: {e}");
            return;
        }
    };
    if let Some((_, history)) = collab.changes.history.as_mut() {
        history.push(closed.clone());
    }
    if let Some(Session::Hosting(hosting)) = collab.session.as_mut() {
        for action in hosting.release_author(&author) {
            if let vale_collab::Action::Holds(list) = action {
                super::holds_changed(collab, session, list);
            }
        }
    }
    // Their files never reached the folder. See *Taking back another
    // author's edits*.
    let _ = super::files::unstage(session, assets, &author, false);
    // The inverses of what is applied here, newest first, as one edit.
    let taken: Vec<crate::session::Excluded> = session.exclude.iter().filter(|excluded| excluded.author == author).cloned().collect();
    session.exclude.retain(|excluded| excluded.author != author);
    let mut made = false;
    for excluded in taken.iter().rev() {
        let step = Step::Edit { change: excluded.change.clone(), forward: !excluded.forward, continues: made };
        make(collab, session, assets, step);
        made = true;
    }
    session.status = format!(
        "took back {} of {name}'s in changelist {}; save to write it",
        super::counted(closed.whole_edits, "edit", "edits"),
        closed.number
    );
}

/// The subjects with unsaved edits, by subject key and as the Changes
/// window names them: each tile, each client table, and the server rows.
pub(crate) fn unsaved_subjects(session: &EditSession) -> Vec<(String, String)> {
    let mut tiles: Vec<(u32, u32)> = session.unsaved.iter().copied().collect();
    tiles.sort();
    let mut tables: Vec<String> = session.unsaved_tables.iter().cloned().collect();
    tables.sort();
    let mut out: Vec<(String, String)> = tiles
        .into_iter()
        .map(|(x, y)| (tile_key(&vale_edit::TileKey::new(&session.map, x, y)), format!("{} {x}_{y}", session.map)))
        .collect();
    out.extend(tables.into_iter().map(|name| (table_key(&name), format!("{name}.dbc"))));
    if session.rows.unsaved() {
        out.push((ROWS.to_string(), "Server rows".to_string()));
    }
    out
}

/// The most lines History shows for one changelist.
const DETAIL_LINES: usize = 400;

/// Show changelist `number`'s edits in History, or hide them when they are
/// shown. See *What a changelist changed*.
pub(crate) fn show_detail(collab: &mut Collab, session: &mut EditSession, number: u32) {
    if collab.changes.detail.as_ref().is_some_and(|(shown, _)| *shown == number) {
        collab.changes.detail = None;
        return;
    }
    if collab.is_joined() {
        let Some(changelist) = collab.changes.history.as_ref().and_then(|(_, list)| list.iter().find(|cl| cl.number == number)).cloned()
        else {
            return;
        };
        if collab.changes.waiting.is_some() {
            session.status = "the host has not answered an earlier request yet".to_string();
            return;
        }
        if let Some(running) = collab.session.as_mut() {
            running.request(Request::Edits(changelist.edits));
            collab.changes.waiting = Some(Waiting::Detail(number));
            collab.changes.detail = Some((number, None));
        }
        return;
    }
    let Some(log) = collab.log.as_ref() else { return };
    let Some(changelist) = log.changelist(number) else { return };
    let mut steps = Vec::new();
    let mut lines = Vec::new();
    for seq in &changelist.edits {
        match log.read(*seq).ok().and_then(|bytes| log::decode(&bytes)) {
            Some(step) => steps.push(step),
            None => lines.push(format!("edit {seq} could not be read from the log")),
        }
    }
    lines.extend(describe_all(steps));
    collab.changes.detail = Some((number, Some(lines)));
}

/// [`describe`] for a changelist's steps, each whole edit once: a step that
/// continues the one before is described with it. At most [`DETAIL_LINES`].
pub(crate) fn describe_all(steps: Vec<Step>) -> Vec<String> {
    let mut whole: Vec<Step> = Vec::new();
    for step in steps {
        match (whole.last_mut(), step) {
            (Some(Step::Edit { change: had, forward: true, .. }), Step::Edit { change, forward: true, continues: true }) => {
                let entries = had.entries().iter().chain(change.entries()).cloned().collect();
                *had = Change::from_entries(had.label.clone(), entries);
            }
            (_, step) => whole.push(step),
        }
    }
    let mut lines = Vec::new();
    for step in &whole {
        lines.extend(describe(step));
        if lines.len() > DETAIL_LINES {
            lines.truncate(DETAIL_LINES);
            lines.push("\u{2026}".to_string());
            break;
        }
    }
    lines
}

/// One edit as History shows it: its name, then one indented line per thing
/// it changed. See *What a changelist changed*.
pub(crate) fn describe(step: &Step) -> Vec<String> {
    match step {
        Step::Edit { change, forward, .. } => {
            let mut out = vec![match forward {
                true => change.label.clone(),
                false => format!("Undo {}", change.label),
            }];
            describe_change(change, *forward, &mut out);
            out
        }
        Step::Files(files) => vec![format!("Files: {}", files.words())],
        Step::Delete(_) => Vec::new(),
    }
}

fn describe_change(change: &Change, forward: bool, out: &mut Vec<String>) {
    use std::collections::BTreeMap;
    use vale_edit::ops::Edit;
    // A tile's edits counted by what they changed, in the order tiles were
    // first touched.
    let mut tiles: Vec<(String, BTreeMap<&'static str, std::collections::BTreeSet<usize>>)> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    let turn = |before: String, after: String| match forward {
        true => format!("{before} \u{2192} {after}"),
        false => format!("{after} \u{2192} {before}"),
    };
    let text = |value: &Option<String>| match value {
        Some(value) if value.chars().count() > 40 => format!("{}\u{2026}", value.chars().take(40).collect::<String>()),
        Some(value) => value.clone(),
        None => "nothing".to_string(),
    };
    for entry in change.entries() {
        match entry {
            Entry::Tile(tile, edit) => {
                let name = format!("{} {}_{}", tile.map, tile.x, tile.y);
                let at = match tiles.iter().position(|(had, _)| *had == name) {
                    Some(at) => at,
                    None => {
                        tiles.push((name, BTreeMap::new()));
                        tiles.len() - 1
                    }
                };
                let (what, index): (&'static str, usize) = match edit {
                    Edit::Heights { chunk, .. } | Edit::Normals { chunk, .. } => ("shape", *chunk),
                    Edit::Colours { chunk, .. } => ("shading", *chunk),
                    Edit::Paint { chunk, .. } => ("paint", *chunk),
                    Edit::Liquid { chunk, .. } => ("water", *chunk),
                    Edit::Holes { chunk, .. } => ("holes", *chunk),
                    Edit::Area { chunk, .. } => ("area", *chunk),
                    Edit::Flags { chunk, .. } => ("flags", *chunk),
                    Edit::Doodad { index, .. } => ("doodad", *index),
                    Edit::Building { index, .. } => ("building", *index),
                    Edit::Placements { .. } => ("placement lists", 0),
                    Edit::Names { .. } => ("path lists", 0),
                };
                tiles[at].1.entry(what).or_default().insert(index);
            }
            Entry::Cell(table, cell) => others.push(format!(
                "    {table}.dbc record {} field {}: {}",
                cell.record,
                cell.field,
                turn(cell.before.to_string(), cell.after.to_string())
            )),
            Entry::Row(table, row) => others.push(format!(
                "    {table}.dbc record {} {}",
                row.index,
                match row.added == forward {
                    true => "added",
                    false => "removed",
                }
            )),
            Entry::Server(server) => {
                let what = match &server.shape {
                    vale_edit::undo::Shape::Column(column) => column.clone(),
                    vale_edit::undo::Shape::Row => "row".to_string(),
                    vale_edit::undo::Shape::Path => "path".to_string(),
                };
                others.push(format!("    {} {} {what}: {}", server.table, server.key, turn(text(&server.before), text(&server.after))));
            }
        }
    }
    for (name, kinds) in tiles {
        let parts: Vec<String> = kinds
            .into_iter()
            .map(|(what, indexes)| match (what, indexes.len()) {
                ("placement lists" | "path lists", _) => what.to_string(),
                ("doodad" | "building", 1) => format!("1 {what}"),
                ("doodad" | "building", n) => format!("{n} {what}s"),
                (what, 1) => format!("{what} of 1 chunk"),
                (what, n) => format!("{what} of {n} chunks"),
            })
            .collect();
        out.push(format!("    {name}: {}", parts.join(", ")));
    }
    out.extend(others);
}

/// Ask the host for its history, once per opening of the window.
pub(crate) fn ask_history(collab: &mut Collab) {
    if collab.is_joined() && !collab.changes.asked {
        if let Some(Session::Joined(_)) = collab.session.as_ref() {
            collab.changes.asked = true;
            if let Some(running) = collab.session.as_mut() {
                running.request(Request::History);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile() -> vale_edit::TileKey {
        vale_edit::TileKey::new("Azeroth", 32, 48)
    }

    fn raise(chunks: &[usize]) -> Step {
        let entries = chunks
            .iter()
            .map(|chunk| Entry::Tile(tile(), vale_edit::ops::Edit::Heights { chunk: *chunk, before: vec![0.0], after: vec![1.0] }))
            .collect();
        Step::Edit { change: Change::from_entries("Raise terrain", entries), forward: true, continues: false }
    }

    /// A step is split by the stale groups it touches, entry by entry, and
    /// taken apart by subject; its inverse is the same change the other way.
    #[test]
    fn a_step_splits_by_stale_group_and_by_subject() {
        let shape = |chunk| Group::Chunk { tile: tile(), chunk, part: vale_collab::Part::Shape };
        let area = |chunk| Group::Chunk { tile: tile(), chunk, part: vale_collab::Part::Area };
        let areas = |chunks: &[usize]| {
            let entries = chunks.iter().map(|chunk| Entry::Tile(tile(), vale_edit::ops::Edit::Area { chunk: *chunk, before: 1, after: 2 })).collect();
            Step::edit(Change::from_entries("Paint area", entries))
        };
        let (inside, outside) = split(&areas(&[3, 4, 5]), &[area(4)]);
        assert_eq!(inside.unwrap().groups(), vec![area(4)]);
        assert_eq!(outside.unwrap().groups(), vec![area(3), area(5)]);
        // A stroke's heights go together.
        let (inside, outside) = split(&raise(&[3, 4, 5]), &[shape(4)]);
        assert_eq!(inside.unwrap().groups(), vec![shape(3), shape(4), shape(5)]);
        assert!(outside.is_none());
        let (inside, outside) = split(&raise(&[3]), &[shape(9)]);
        assert!(inside.is_none() && outside.is_some());

        let other = vale_edit::TileKey::new("Azeroth", 33, 48);
        let mut entries = raise(&[3]);
        if let Step::Edit { change, .. } = &mut entries {
            let mut all = change.entries().to_vec();
            all.push(Entry::Tile(other.clone(), vale_edit::ops::Edit::Area { chunk: 1, before: 1, after: 2 }));
            *change = Change::from_entries("Edit", all);
        }
        let parts = by_subject(entries);
        let names: Vec<&str> = parts.iter().map(|(_, name, _)| name.as_str()).collect();
        assert_eq!(names, ["Azeroth 32_48", "Azeroth 33_48"]);

        let Some(Step::Edit { forward, .. }) = inverse(&raise(&[3])) else { panic!() };
        assert!(!forward);
    }

    /// The parts of one stroke are described as one edit, and counted as
    /// one.
    #[test]
    fn the_parts_of_a_stroke_are_one_edit() {
        let steps = vec![raise(&[3]), raise(&[4]).continuing(true), raise(&[5])];
        assert_eq!(crate::collab::whole_edits(&steps), 2);
        let lines = describe_all(steps);
        let names: Vec<&String> = lines.iter().filter(|line| !line.starts_with(' ')).collect();
        assert_eq!(names, ["Raise terrain", "Raise terrain"], "{lines:?}");
        assert!(lines[1].contains("3") && lines[1].contains("4"), "{lines:?}");
    }

    /// Alone: a raise over two chunks is logged and submitted; another
    /// author's later edit changes one of the chunks; reverting the
    /// changelist lists the raise's ground as one conflict, both chunks, since
    /// half of it would tear the border; Reapply takes the raise's change off
    /// both over what is there now, and every step is in the log.
    #[test]
    fn a_revert_puts_back_what_was_not_changed_since_and_lists_the_rest() {
        let install = std::env::temp_dir().join(format!("vale-collab-revert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Revert").unwrap());
        let mut assets = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        assets.root = install.to_string_lossy().into_owned();
        let coord = (32, 48);
        session.tiles.insert(coord, vale_edit::adt::blank::blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12));
        let key = session.key(coord);
        let heights = |session: &EditSession, chunk: usize| vale_edit::adt::heights::heights(session.tiles[&coord].chunk(chunk).unwrap());
        let original = heights(&session, 5);

        let mut collab = Collab::default();
        super::super::keep_the_log(&mut collab, &mut session, &assets);
        session.history.begin("Raise terrain");
        for chunk in [5, 6] {
            let edit = vale_edit::ops::Edit::Heights { chunk, before: original.clone(), after: vec![20.0; 145] };
            edit.apply(session.tiles.get_mut(&coord).unwrap());
            session.history.record(&key, [edit]);
        }
        session.history.end();
        super::super::log_own_steps(&mut collab, &mut session, &assets);
        collab.changes.description = "Raise".into();
        submit(&mut collab, &mut session, &assets);
        assert_eq!(collab.log.as_ref().unwrap().changelists().len(), 1, "{}", session.status);

        // Another author lowers chunk 5 after the changelist.
        let sam = vale_collab::log::Author { id: [9; 16], name: "Sam".into() };
        let lower = vale_edit::ops::Edit::Heights { chunk: 5, before: vec![20.0; 145], after: vec![15.0; 145] };
        lower.apply(session.tiles.get_mut(&coord).unwrap());
        let step = Step::Edit { change: Change::from_entries("Lower terrain", vec![Entry::Tile(key.clone(), lower)]), forward: true, continues: false };
        let log = collab.log.as_mut().unwrap();
        let seq = log.last() + 1;
        log::append(log, seq, &sam, &step, &mut session.status);

        revert(&mut collab, &mut session, &assets, 1);
        assert_eq!(heights(&session, 6), vec![20.0; 145], "the raise's ground is one conflict, chunk 6 with it");
        assert_eq!(heights(&session, 5), vec![15.0; 145], "the chunk Sam changed is left");
        assert_eq!(collab.changes.conflicts.len(), 1);
        assert_eq!(collab.changes.conflicts[0].subject, "Azeroth 32_48");

        // Reapply takes the raise's change back off the ground as it is now:
        // chunk 6 returns to where it was, and chunk 5 keeps Sam's lowering
        // under it.
        reapply(&mut collab, &mut session, &assets, 0);
        assert!(collab.changes.conflicts.is_empty(), "{}", session.status);
        assert_eq!(heights(&session, 6), original);
        let lowered: Vec<f32> = original.iter().map(|height| height - 5.0).collect();
        assert_eq!(heights(&session, 5), lowered);
        let log = collab.log.as_ref().unwrap();
        assert_eq!(log.last(), 3, "the raise, Sam's lower and the revert");
        let Some(Step::Edit { change, forward: false, .. }) = log::decode(&log.read(3).unwrap()) else { panic!("not a revert") };
        assert_eq!(change.entries().len(), 2);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// An edit is described by its name, its tiles counted by what changed,
    /// and its table fields and server columns before and after, the other
    /// way round for an undo.
    #[test]
    fn an_edit_is_described_by_what_it_changed() {
        let tile = tile();
        let entries = vec![
            Entry::Tile(tile.clone(), vale_edit::ops::Edit::Heights { chunk: 3, before: vec![0.0], after: vec![1.0] }),
            Entry::Tile(tile.clone(), vale_edit::ops::Edit::Normals { chunk: 3, before: vec![0], after: vec![1] }),
            Entry::Tile(tile, vale_edit::ops::Edit::Heights { chunk: 4, before: vec![0.0], after: vec![1.0] }),
            Entry::Cell("Spell".into(), vale_edit::dbc::Cell { record: 12, field: 5, before: 10, after: 20 }),
            Entry::Server(vale_edit::undo::ServerEntry::column("creature", "guid=7", "position_x", None, Some("1".into()))),
        ];
        let step = Step::Edit { change: Change::from_entries("Raise terrain", entries), forward: true, continues: false };
        assert_eq!(
            describe(&step),
            [
                "Raise terrain",
                "    Azeroth 32_48: shape of 2 chunks",
                "    Spell.dbc record 12 field 5: 10 \u{2192} 20",
                "    creature guid=7 position_x: nothing \u{2192} 1",
            ]
        );
        let Some(Step::Edit { change, .. }) = inverse(&step) else { panic!() };
        let undone = describe(&Step::Edit { change, forward: false, continues: false });
        assert_eq!(undone[0], "Undo Raise terrain");
        assert_eq!(undone[2], "    Spell.dbc record 12 field 5: 20 \u{2192} 10");
    }

    /// Alone: a raise on two tiles is logged; Discard of one tile puts it back
    /// and leaves the other unsaved; a save, with nothing left unsaved,
    /// closes a changelist with the description. Opened again with a raise
    /// logged and not saved, the log offers it as recovered, and Restore
    /// applies it.
    #[test]
    fn discard_save_and_recover_follow_the_open_edits() {
        let install = std::env::temp_dir().join(format!("vale-collab-discard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Discard").unwrap());
        let mut assets = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        assets.root = install.to_string_lossy().into_owned();
        for coord in [(32, 48), (33, 48)] {
            session.tiles.insert(coord, vale_edit::adt::blank::blank_tile(coord.0, coord.1, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12));
        }
        let heights = |session: &EditSession, coord: (u32, u32)| vale_edit::adt::heights::heights(session.tiles[&coord].chunk(5).unwrap());
        let original = heights(&session, (32, 48));
        let mut collab = Collab::default();
        super::super::keep_the_log(&mut collab, &mut session, &assets);
        assert!(collab.log.is_some());

        // One raise on each tile. A raise over both would be discarded whole,
        // since a stroke's heights go together; see `split`.
        for coord in [(32, 48), (33, 48)] {
            let key = session.key(coord);
            let edit = vale_edit::ops::Edit::Heights { chunk: 5, before: original.clone(), after: vec![20.0; 145] };
            session.history.begin("Raise terrain");
            edit.apply(session.tiles.get_mut(&coord).unwrap());
            session.history.record(&key, [edit]);
            session.history.end();
            session.unsaved.insert(coord);
        }
        super::super::log_own_steps(&mut collab, &mut session, &assets);

        let first = tile_key(&session.key((32, 48)));
        discard(&mut collab, &mut session, &assets, Some(first.clone()));
        assert_eq!(heights(&session, (32, 48)), original, "{}", session.status);
        assert_eq!(heights(&session, (33, 48)), vec![20.0; 145], "the other tile keeps its edit");
        assert_eq!(session.unsaved.iter().copied().collect::<Vec<_>>(), [(33, 48)]);

        collab.changes.description = "Raise the hill".into();
        session.save_files().unwrap();
        super::super::after_the_steps(&mut collab, &mut session, &assets);
        let log = collab.log.as_ref().unwrap();
        assert_eq!(log.changelists().len(), 1);
        assert_eq!(log.changelists()[0].description, "Raise the hill");
        assert_eq!(log.changelists()[0].whole_edits, 3, "the two raises and the discard");
        assert_eq!(collab.open_edits(), 0);

        // An edit logged and never saved, as a crash leaves it.
        let key = session.key((33, 48));
        let lower = vale_edit::ops::Edit::Heights { chunk: 5, before: vec![20.0; 145], after: vec![5.0; 145] };
        let step = Step::Edit { change: Change::from_entries("Lower terrain", vec![Entry::Tile(key, lower)]), forward: true, continues: false };
        let author = collab.author(&assets.root);
        let log = collab.log.as_mut().unwrap();
        let seq = log.last() + 1;
        log::append(log, seq, &author, &step, &mut session.status);
        let mut reopened = Collab::default();
        super::super::keep_the_log(&mut reopened, &mut session, &assets);
        assert_eq!(reopened.changes.recovered, vec![seq]);
        restore_recovered(&mut reopened, &mut session, &assets);
        assert_eq!(heights(&session, (33, 48)), vec![5.0; 145]);
        assert!(reopened.changes.recovered.is_empty());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// Another author's unsubmitted edit holds its chunk: this editor's edit
    /// there is undone, Discard leaves it, a save leaves it out of the
    /// folder, a reopened log recovers it, and Take Back closes it into a
    /// changelist and undoes it.
    #[test]
    fn another_authors_unsubmitted_edit_is_theirs() {
        let install = std::env::temp_dir().join(format!("vale-collab-theirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Theirs").unwrap());
        let mut assets = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        assets.root = install.to_string_lossy().into_owned();
        for coord in [(32, 48), (33, 48)] {
            session.tiles.insert(coord, vale_edit::adt::blank::blank_tile(coord.0, coord.1, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12));
        }
        let heights = |session: &EditSession, coord: (u32, u32)| vale_edit::adt::heights::heights(session.tiles[&coord].chunk(5).unwrap());
        let original = heights(&session, (32, 48));
        let mut collab = Collab::default();
        collab.author(&assets.root);
        super::super::keep_the_log(&mut collab, &mut session, &assets);

        // Alex's raise on 32_48, as a host logs and applies it.
        let alex = vale_collab::log::Author { id: [9; 16], name: "Alex".into() };
        let key = session.key((32, 48));
        let raise = vale_edit::ops::Edit::Heights { chunk: 5, before: original.clone(), after: vec![20.0; 145] };
        let change = Change::from_entries("Raise terrain", vec![Entry::Tile(key.clone(), raise)]);
        let log = collab.log.as_mut().unwrap();
        log::append(log, 1, &alex, &Step::edit(change.clone()), &mut session.status);
        session.apply_step(&assets, &change, true);
        session.exclude.push(crate::session::Excluded { seq: 1, author: alex.id, change, forward: true });

        // This editor's edits: one on 33_48, one on Alex's chunk.
        for coord in [(33, 48), (32, 48)] {
            let key = session.key(coord);
            let before = heights(&session, coord);
            let edit = vale_edit::ops::Edit::Heights { chunk: 5, before, after: vec![30.0; 145] };
            session.history.begin("Raise terrain");
            edit.apply(session.tiles.get_mut(&coord).unwrap());
            session.history.record(&key, [edit]);
            session.history.end();
            session.unsaved.insert(coord);
        }
        super::super::log_own_steps(&mut collab, &mut session, &assets);
        assert_eq!(heights(&session, (32, 48)), vec![20.0; 145], "undone: {}", session.status);
        assert!(session.status.contains("Alex has unsubmitted edits on Azeroth 32_48"), "{}", session.status);
        assert_eq!(collab.log.as_ref().unwrap().last(), 2, "the undone edit is not logged");

        discard(&mut collab, &mut session, &assets, None);
        assert_eq!(heights(&session, (33, 48)), original, "this editor's edit is discarded");
        assert_eq!(heights(&session, (32, 48)), vec![20.0; 145], "Alex's is not");

        assert!(session.save((32, 48)));
        let written = session.project.read(&key.vpath()).expect("the tile is written");
        let written = vale_edit::adt::AdtFile::parse(&written).unwrap();
        assert_eq!(vale_edit::adt::heights::heights(written.chunk(5).unwrap()), original, "the folder leaves Alex's edit out");

        let mut reopened = Collab::default();
        reopened.author_id = collab.author_id;
        super::super::keep_the_log(&mut reopened, &mut session, &assets);
        assert!(reopened.changes.recovered.contains(&1), "{:?}", reopened.changes.recovered);

        take_back(&mut collab, &mut session, &assets, alex.id);
        assert_eq!(heights(&session, (32, 48)), original, "{}", session.status);
        assert!(session.exclude.is_empty());
        let log = collab.log.as_ref().unwrap();
        assert!(log.changelists().iter().any(|closed| closed.edits == [1] && closed.description.starts_with("Taken back by")));
        assert!(log.pending().values().all(|by| *by != alex.id));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// An edit undone again leaves its tile as the folder holds it: the tile
    /// is not unsaved, and the edit and its undo are withdrawn from the log,
    /// so nothing counts them and no changelist is made.
    #[test]
    fn an_edit_undone_again_is_no_change() {
        let install = std::env::temp_dir().join(format!("vale-collab-undone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Undone").unwrap());
        let mut assets = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        assets.root = install.to_string_lossy().into_owned();
        let coord = (32, 48);
        session.tiles.insert(coord, vale_edit::adt::blank::blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12));
        let mut collab = Collab::default();
        collab.author(&assets.root);
        super::super::keep_the_log(&mut collab, &mut session, &assets);
        assert!(session.save(coord), "the folder holds the tile");
        super::super::after_the_steps(&mut collab, &mut session, &assets);

        let key = session.key(coord);
        let before = vale_edit::adt::heights::heights(session.tiles[&coord].chunk(5).unwrap());
        let edit = vale_edit::ops::Edit::Heights { chunk: 5, before, after: vec![20.0; 145] };
        session.history.begin("Raise terrain");
        edit.apply(session.tiles.get_mut(&coord).unwrap());
        session.history.record(&key, [edit]);
        session.history.end();
        session.unsaved.insert(coord);
        super::super::log_own_steps(&mut collab, &mut session, &assets);
        super::super::after_the_steps(&mut collab, &mut session, &assets);
        assert_eq!(collab.open_edits(), 1);

        session.undo(&assets);
        assert!(session.unsaved.is_empty(), "the tile is as the folder holds it");
        super::super::log_own_steps(&mut collab, &mut session, &assets);
        super::super::after_the_steps(&mut collab, &mut session, &assets);
        assert_eq!(collab.open_edits(), 0);
        let log = collab.log.as_ref().unwrap();
        assert_eq!((log.last(), log.changelists().len()), (2, 0), "both are in the log, in no changelist");
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A copy back online: its pending raise over two chunks, one of which
    /// the host changed since the link's number, is listed as one conflict,
    /// both chunks, since half of it would tear the border; the pending file
    /// is emptied.
    #[test]
    fn pending_edits_are_made_where_nothing_changed_and_listed_where_it_did() {
        let install = std::env::temp_dir().join(format!("vale-collab-pending-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Copy (shared)").unwrap());
        let assets = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        let coord = (32, 48);
        session.tiles.insert(coord, vale_edit::adt::blank::blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 10.0, 12));
        let key = session.key(coord);
        let heights = |session: &EditSession, chunk: usize| vale_edit::adt::heights::heights(session.tiles[&coord].chunk(chunk).unwrap());
        let original = heights(&session, 5);
        let entries = [5, 6]
            .map(|chunk| Entry::Tile(key.clone(), vale_edit::ops::Edit::Heights { chunk, before: original.clone(), after: vec![30.0; 145] }));
        let raise = Step::Edit { change: Change::from_entries("Raise terrain", entries.to_vec()), forward: true, continues: false };
        super::super::offline::append(&session.project, &[raise.clone()]).unwrap();

        let mut collab = Collab::default();
        collab.state = State::Joined { project: "Copy (shared)".into() };
        collab.pending = 1;
        let stale = vec![Group::Chunk { tile: key.clone(), chunk: 5, part: vale_collab::Part::Shape }];
        pending_sent(&mut collab, &mut session, &assets, vec![raise], stale);
        assert_eq!(heights(&session, 6), original, "the raise's ground is one conflict, chunk 6 with it");
        assert_eq!(heights(&session, 5), original);
        assert_eq!(collab.changes.conflicts.len(), 1);
        assert_eq!(collab.changes.conflicts[0].source, "Offline edits");
        assert_eq!((collab.pending, super::super::offline::read(&session.project).len()), (0, 0));
        let _ = std::fs::remove_dir_all(&install);
    }
}
