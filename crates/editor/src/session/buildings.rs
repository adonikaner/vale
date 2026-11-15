//! Buildings open for editing: a WMO's root and group files held parsed, the
//! way tiles are, edited with one undo entry per edit, published to the
//! overlay so the renderer draws the edit, and saved into the project.
//!
//! ## Open
//!
//! A building is read through the overlay ([`EditSession::open_building`]),
//! so a copy the project holds, or bytes an earlier edit published, is what
//! is opened. Its groups are read when an edit needs them: a move changes
//! the root alone, and an added or removed spawn moves the spawn lists of
//! every group.
//!
//! ## An edit
//!
//! [`EditSession::edit_building`] runs an edit on a copy, records what
//! changed ([`vale_edit::wmo::Building::diff`]) as `Building` entries of the
//! undo history, and publishes the files it changed. Undo and redo apply
//! the same entries ([`EditSession::step_buildings`]).
//!
//! ## What the renderer is told
//!
//! The published bytes are named in [`EditSession::republished`], which makes
//! the client's building cache forget the file. A building already standing
//! keeps what it was built from, so each open tile that places the building
//! is streamed again ([`EditSession::restream`]), and the host of a
//! WMO-only map's building with it. The building editor builds its own copy
//! again beside the one it draws (`crate::tools::placements::wmoview`). The building is then built from the new
//! bytes, with its spawns.
//!
//! Rebuilding a large building takes longer than a frame, and a drag on a
//! number edits it on every frame. So the renderer is told once edits to
//! buildings have stopped for [`REDRAW_SETTLE`] ([`redraw_buildings`]). The
//! overlay holds the new bytes at once, and the tools draw their marks from
//! the open files, so only the building's picture waits.
//!
//! ## Save
//!
//! A save writes each unsaved building's root and every group an edit
//! changed into the project ([`EditSession::save_buildings`]); a file that
//! is back to the archives' bytes is taken out of the project instead. An
//! undo that brings every changed file back to what the folder holds takes
//! the unsaved mark off ([`EditSession::unmark_matching_buildings`]), as it
//! does for a tile or a table, so the edit and its undo are not counted as a
//! change.

use std::collections::BTreeSet;

use bevy::prelude::*;
use vale_client::assets::GameAssets;
use vale_edit::wmo::{group_path, Building, GroupFile, RootFile};

use super::EditSession;

/// The tile key the client's terrain pass files a WMO-only map's building
/// under. Streaming it again rebuilds that building. See
/// `vale_client::render::globalwmo`.
const HOST_TILE: (u32, u32) = (64, 64);

/// How long edits to buildings must stop before the renderer is told. See the
/// module comment.
const REDRAW_SETTLE: f64 = 0.2;

/// Buildings published since the renderer was last told, and their files.
#[derive(Debug, Default)]
pub(crate) struct Redraw {
    keys: BTreeSet<String>,
    files: BTreeSet<String>,
    /// Bumped by every publish, so [`redraw_buildings`] can tell when the
    /// last one was.
    revision: u64,
    /// How many times the renderer has been told, for the building editor.
    pub(crate) told: u64,
}

/// A building open for editing, and which of its files are ahead of the
/// project folder.
#[derive(Debug, Clone)]
pub(crate) struct OpenBuilding {
    /// The root's path as it was asked for.
    pub(crate) path: String,
    pub(crate) building: Building,
    root_changed: bool,
    groups_changed: BTreeSet<usize>,
}

impl OpenBuilding {
    /// A building as read, with nothing changed.
    pub(crate) fn new(path: &str, root: RootFile) -> OpenBuilding {
        OpenBuilding { path: path.to_string(), building: Building::new(root), root_changed: false, groups_changed: BTreeSet::new() }
    }
}

/// The key a building is held under: its root's path in lower case, with
/// `\` between its parts.
pub(crate) fn building_key(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

impl EditSession {
    /// Read a building's root through the overlay, once. Answers why it
    /// could not be read.
    pub(crate) fn open_building(&mut self, assets: &GameAssets, path: &str) -> Result<(), String> {
        let key = building_key(path);
        if self.buildings.contains_key(&key) {
            return Ok(());
        }
        let bytes = read(assets, path).ok_or_else(|| format!("{path} is not in the archives or the project"))?;
        let root = RootFile::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        self.buildings.insert(key, OpenBuilding::new(path, root));
        Ok(())
    }

    /// Read every group of an open building that is not read yet.
    pub(crate) fn read_building_groups(&mut self, assets: &GameAssets, path: &str) -> Result<(), String> {
        let key = building_key(path);
        let Some(open) = self.buildings.get_mut(&key) else {
            return Err(format!("{path} is not open"));
        };
        for at in 0..open.building.groups.len() {
            if open.building.groups[at].is_some() {
                continue;
            }
            let group = group_path(&open.path, at);
            let bytes = read(assets, &group).ok_or_else(|| format!("{group} is not in the archives or the project"))?;
            open.building.groups[at] = Some(GroupFile::parse(&bytes).map_err(|e| format!("{group}: {e}"))?);
        }
        Ok(())
    }

    /// An open building.
    pub(crate) fn building(&self, path: &str) -> Option<&OpenBuilding> {
        self.buildings.get(&building_key(path))
    }

    /// Make an edit to an open building as one undo entry, folded into the
    /// entry before it when `subject` is that entry's and the edit follows
    /// it closely (a drag); then publish what it changed. With `groups`, every
    /// group is read first; without it, `edit` is given the root and no
    /// group, and must change the root alone. Answers why the edit was not
    /// made; an edit that changes nothing records nothing.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn edit_building(
        &mut self,
        assets: &GameAssets,
        path: &str,
        label: &str,
        subject: &str,
        now: f64,
        groups: bool,
        edit: impl FnOnce(&mut Building) -> Result<(), String>,
    ) -> Result<(), String> {
        self.open_building(assets, path)?;
        if groups {
            self.read_building_groups(assets, path)?;
        }
        let key = building_key(path);
        let Some(open) = self.buildings.get(&key) else { return Ok(()) };
        // An edit that leaves the groups alone is made on the root alone: a
        // city's groups are tens of megabytes, and a drag edits every frame.
        // `Building::diff` passes over a group either side has not read.
        let mut after = match groups {
            true => open.building.clone(),
            false => Building { root: open.building.root.clone(), groups: vec![None; open.building.groups.len()] },
        };
        edit(&mut after)?;
        let changes = Building::diff(&open.building, &after);
        if changes.is_empty() {
            return Ok(());
        }
        self.history.begin_gesture(label, subject, now);
        for change in &changes {
            self.history.record_building(&key, change.clone());
        }
        self.history.end();
        let open = self.buildings.get_mut(&key).expect("checked above");
        match groups {
            true => open.building = after,
            false => open.building.root = after.root,
        }
        let (root, touched) = touched(&changes);
        self.publish_building(&key, root, &touched);
        Ok(())
    }

    /// Put the building parts of an undone or redone change back in, and
    /// publish what they changed.
    pub(crate) fn step_buildings(&mut self, assets: &GameAssets, change: &vale_edit::undo::Change, forward: bool) {
        for key in change.building_files() {
            let steps = change.building_steps(&key, forward);
            let needs_groups = steps.iter().any(|step| !matches!(step, vale_edit::wmo::BuildingChange::Root { .. }));
            if let Err(why) = self.open_building(assets, &key) {
                warn!("building: {why}");
                continue;
            }
            if needs_groups {
                if let Err(why) = self.read_building_groups(assets, &key) {
                    warn!("building: {why}");
                }
            }
            let Some(open) = self.buildings.get_mut(&key) else { continue };
            for step in &steps {
                open.building.apply(step, forward);
            }
            let owned: Vec<vale_edit::wmo::BuildingChange> = steps.into_iter().cloned().collect();
            let (root, groups) = touched(&owned);
            self.publish_building(&key, root, &groups);
        }
    }

    /// Put an open building's changed files in the overlay, mark the
    /// building unsaved, and have the renderer build it again. See the
    /// module comment.
    fn publish_building(&mut self, key: &str, root: bool, groups: &[usize]) {
        let Some(open) = self.buildings.get_mut(key) else { return };
        open.root_changed |= root;
        open.groups_changed.extend(groups.iter().copied());
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        if root {
            files.push((open.path.clone(), open.building.root.write()));
        }
        for &at in groups {
            if let Some(Some(group)) = open.building.groups.get(at) {
                files.push((group_path(&open.path, at), group.write()));
            }
        }
        if let Ok(mut edited) = self.edited.write() {
            for (path, bytes) in &files {
                edited.insert(path.to_ascii_lowercase(), bytes.clone());
            }
        }
        self.unsaved_buildings.insert(key.to_string());
        self.building_redraw.files.extend(files.into_iter().map(|(path, _)| path.to_ascii_lowercase()));
        self.building_redraw.keys.insert(key.to_string());
        self.building_redraw.revision += 1;
    }

    /// Tell the renderer about every building published since it was last
    /// told: forget its files, and stream again each open tile that places
    /// it and the host of a WMO-only map's building.
    fn tell_the_renderer(&mut self) {
        let told = self.building_redraw.told;
        let redraw = std::mem::take(&mut self.building_redraw);
        self.building_redraw.told = told;
        if redraw.keys.is_empty() {
            return;
        }
        self.republished.extend(redraw.files);
        let places: Vec<(u32, u32)> = self
            .tiles
            .iter()
            .filter(|(_, tile)| tile.building_names().iter().any(|name| redraw.keys.contains(&building_key(name))))
            .map(|(coord, _)| *coord)
            .collect();
        self.restream.extend(places);
        self.restream.insert(HOST_TILE);
        // The building editor builds its own copy again from this count,
        // beside the one drawn, so its picture does not go blank. See
        // `crate::tools::placements::wmoview`.
        self.building_redraw.told = told + 1;
    }

    /// After an undo or a redo: each of these buildings whose changed files
    /// are back to what the folder holds is not unsaved. A save of it would
    /// write the same bytes.
    pub(crate) fn unmark_matching_buildings(&mut self, keys: &[String]) {
        for key in keys {
            if !self.unsaved_buildings.contains(key) {
                continue;
            }
            let Some(open) = self.buildings.get(key) else { continue };
            if !changed_files(open).iter().all(|(path, bytes)| self.matches_folder(path, bytes)) {
                continue;
            }
            self.unsaved_buildings.remove(key);
            if let Some(open) = self.buildings.get_mut(key) {
                open.root_changed = false;
                open.groups_changed.clear();
            }
        }
    }

    /// Write every unsaved building's changed files into the project, or
    /// take a file out of it when it is the archives' own bytes. Answers how
    /// many buildings were written; a file that could not be written is
    /// named on the status line and its building stays unsaved.
    pub(crate) fn save_buildings(&mut self) -> usize {
        let _unjournaled = self.project.unjournaled();
        let keys: Vec<String> = self.unsaved_buildings.iter().cloned().collect();
        let mut saved = 0;
        for key in keys {
            let Some(open) = self.buildings.get(&key) else {
                self.unsaved_buildings.remove(&key);
                continue;
            };
            let files = changed_files(open);
            let failed = files.iter().find_map(|(path, bytes)| {
                let written = match self.changes_nothing(path, bytes) {
                    true => self.project.revert(path).map(|_| ()),
                    false => self.project.write(path, bytes).map(|_| ()),
                };
                written.err().map(|e| format!("{path}: {e}"))
            });
            match failed {
                Some(why) => self.status = why,
                None => {
                    self.unsaved_buildings.remove(&key);
                    self.files_revision += 1;
                    saved += 1;
                }
            }
        }
        saved
    }
}

/// Tell the renderer about the buildings published, once edits to them have
/// stopped for [`REDRAW_SETTLE`]. Runs before
/// `super::republish::forget_what_changed`, so the files are forgotten on the
/// same frame.
pub(crate) fn redraw_buildings(time: Res<Time>, session: Option<ResMut<EditSession>>, mut seen: Local<(u64, f64)>) {
    let Some(mut session) = session else { return };
    let now = time.elapsed_secs_f64();
    let (revision, changed_at) = &mut *seen;
    if *revision != session.building_redraw.revision {
        *revision = session.building_redraw.revision;
        *changed_at = now;
    }
    if session.building_redraw.keys.is_empty() || now - *changed_at < REDRAW_SETTLE {
        return;
    }
    session.tell_the_renderer();
    // The revision starts again from zero with the list.
    *revision = 0;
}

/// An open building's files that an edit changed, with their bytes now.
fn changed_files(open: &OpenBuilding) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    if open.root_changed {
        files.push((open.path.clone(), open.building.root.write()));
    }
    for &at in &open.groups_changed {
        if let Some(Some(group)) = open.building.groups.get(at) {
            files.push((group_path(&open.path, at), group.write()));
        }
    }
    files
}

/// Whether a list of parts changes the root, and which groups it changes.
fn touched(changes: &[vale_edit::wmo::BuildingChange]) -> (bool, Vec<usize>) {
    let mut root = false;
    let mut groups = Vec::new();
    for change in changes {
        match change {
            vale_edit::wmo::BuildingChange::Root { .. } => root = true,
            vale_edit::wmo::BuildingChange::Group { group, .. } | vale_edit::wmo::BuildingChange::GroupLights { group, .. } => {
                if !groups.contains(&(*group as usize)) {
                    groups.push(*group as usize);
                }
            }
        }
    }
    (root, groups)
}

/// A file's bytes through the overlay: the edited bytes, the project's copy,
/// or the archives'.
fn read(assets: &GameAssets, path: &str) -> Option<Vec<u8>> {
    assets.with_archive(|chain| Ok(chain.read(path).ok())).ok().flatten()
}
