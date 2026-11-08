//! Undo and redo: one change's tiles, tables and server rows put back or
//! put in again. A shared session applies another editor's change through
//! the same path, [`EditSession::apply_step`], with no entry on the stack.

use vale_client::assets::GameAssets;
use bevy::prelude::*;
use super::EditSession;

impl EditSession {
    /// Undo or redo one change's table half, which is the same three lines
    /// in both directions.
    ///
    /// Separate from the tile loop rather than folded into it because the two
    /// targets are different objects — see `vale_edit::undo::Entry`. What a
    /// person sees is one stack: `Ctrl+Z` takes back the last thing done,
    /// whether that was a wall or a spell's name.
    fn step_tables(&mut self, change: &vale_edit::undo::Change, tables: &[String], forward: bool) {
        for name in tables {
            let Some(table) = self.tables.get_mut(name) else {
                // The table is not open. Nothing to put back — and nothing is
                // lost, because opening it again reads the project folder,
                // which is where the change was written.
                continue;
            };
            match forward {
                true => change.apply_table(name, table),
                false => change.revert_table(name, table),
            }
            self.table_edited(name);
        }
    }

    /// Undo the last change, publishing every tile it was about.
    ///
    /// One stack for every tile, which is what an undo key is expected to be:
    /// the last thing done, wherever it was done. A stroke that crossed a tile
    /// border is one entry naming two tiles and comes back off in one press.
    ///
    /// A tile the change names and this session does not have open is opened
    /// again, which is why this takes the archives. Skipping such a tile is
    /// safe only if [`crate::map::open::close_tiles`] refuses to close anything
    /// the history names, and that rule does not scale: a find-and-replace
    /// over 687 tiles would pin every one of them in memory for the rest of
    /// the session. Opening on demand puts a change back where it was made,
    /// whether or not the camera has been there since. Opening reads through
    /// the overlay, so what comes back is the edited tile, which is the one
    /// the change applies to.
    pub(crate) fn undo(&mut self, assets: &GameAssets) -> Vec<(u32, u32)> {
        self.step(assets, false)
    }

    /// …and put it back, opening what it names on [`Self::undo`]'s own terms.
    pub(crate) fn redo(&mut self, assets: &GameAssets) -> Vec<(u32, u32)> {
        self.step(assets, true)
    }

    /// Take the next change off the history, backward for an undo or forward
    /// for a redo, apply it to the tiles, tables and server rows it names, and
    /// mark what it leaves out of date on screen. Answers the tiles it changed.
    ///
    /// What has to be redrawn is [`vale_edit::undo::Change::invalidates`]'s
    /// answer: the chunks, placements and blend maps patched in place, the
    /// draw groups cut again when a texture set changed, and the whole tile
    /// read again where nothing on screen can be patched.
    fn step(&mut self, assets: &GameAssets, forward: bool) -> Vec<(u32, u32)> {
        let taken = match forward {
            true => self.history.redo(),
            false => self.history.undo(),
        };
        let Some(change) = taken else {
            return Vec::new();
        };
        let verb = if forward { "redo" } else { "undo" };
        self.status = format!("{verb} {}", change.label);
        self.apply_step(assets, &change, forward)
    }

    /// Apply one change forward, or revert it, to the tiles, tables and server
    /// rows it names, and mark what it leaves out of date on screen. Answers
    /// the open tiles it changed. Nothing is put on or taken off the history.
    ///
    /// A table the change names that is not open is opened first, so the
    /// change lands in what the next save writes. A tile on another map than
    /// the open one is read through the overlay, changed, and written back to
    /// the overlay and the project folder, since only the open map's tiles
    /// are held parsed.
    pub(crate) fn apply_step(&mut self, assets: &GameAssets, change: &vale_edit::undo::Change, forward: bool) -> Vec<(u32, u32)> {
        let invalidated = change.invalidates();
        for name in &invalidated.tables {
            self.open_table(assets, name);
        }
        self.step_tables(change, &invalidated.tables, forward);
        self.rows.step(change, forward);
        let mut touched = Vec::new();
        for redraw in &invalidated.tiles {
            if !redraw.tile.map.eq_ignore_ascii_case(&self.map) {
                self.step_elsewhere(assets, change, &redraw.tile, forward);
                continue;
            }
            let coord = (redraw.tile.x, redraw.tile.y);
            self.open(assets, coord);
            let Some(tile) = self.tiles.get_mut(&coord) else {
                continue;
            };
            match forward {
                true => change.apply(&redraw.tile, tile),
                false => change.revert(&redraw.tile, tile),
            }
            self.publish(coord);
            for &chunk in &redraw.chunks {
                self.touched(coord, chunk);
            }
            for &index in &redraw.placements {
                self.moved(coord, index);
            }
            for &index in &redraw.buildings {
                self.moved_building(coord, index);
            }
            if redraw.regroup {
                self.regroup.insert(coord);
            }
            for &chunk in &redraw.painted {
                self.repainted(coord, chunk);
            }
            if redraw.reread {
                self.stale.insert(coord);
            }
            touched.push(coord);
        }
        touched
    }

    /// [`Self::apply_step`] for a tile of a map that is not open.
    fn step_elsewhere(&mut self, assets: &GameAssets, change: &vale_edit::undo::Change, tile: &vale_edit::TileKey, forward: bool) {
        // In a shared session the change itself is sent, and each editor
        // writes this file for itself.
        let _unjournaled = self.project.unjournaled();
        let path = tile.vpath();
        let Ok(Some(bytes)) = assets.with_archive(|chain| Ok(chain.read(&path).ok())) else {
            self.status = format!("{path}: not found, so a change to it was not applied");
            return;
        };
        let mut parsed = match vale_edit::adt::AdtFile::parse(&bytes) {
            Ok(parsed) => parsed,
            Err(e) => {
                self.status = format!("{path}: {e}");
                return;
            }
        };
        match forward {
            true => change.apply(tile, &mut parsed),
            false => change.revert(tile, &mut parsed),
        }
        let bytes = parsed.write();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path.to_ascii_lowercase(), bytes.clone());
        }
        if let Err(e) = self.project.write(&path, &bytes) {
            self.status = format!("{path}: {e}");
        }
    }
}
