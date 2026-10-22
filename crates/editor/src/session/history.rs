//! Undo and redo: one change's tiles, tables and server rows put back or
//! put in again.

use vale_client::assets::GameAssets;
use bevy::prelude::*;
use super::EditSession;

impl EditSession {
    /// Undo or redo one change's table half, which is the same three lines
    /// in both directions.
    ///
    /// Separate from the tile loop rather than folded into it because the two
    /// targets are different objects — see `vale_edit::undo::Change`, where
    /// the argument for two lists on one stack is. What a person sees is one
    /// stack: `Ctrl+Z` takes back the last thing done, whether that was a wall
    /// or a spell's name.
    fn step_tables(&mut self, change: &vale_edit::undo::Change, forward: bool) {
        for name in change.tables() {
            let Some(table) = self.tables.get_mut(&name) else {
                // The table is not open. Nothing to put back — and nothing is
                // lost, because opening it again reads the project folder,
                // which is where the change was written.
                continue;
            };
            match forward {
                true => change.apply_table(&name, table),
                false => change.revert_table(&name, table),
            }
            self.table_edited(&name);
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
    pub fn undo(&mut self, assets: &GameAssets) -> Vec<(u32, u32)> {
        let Some(change) = self.history.undo() else {
            return Vec::new();
        };
        self.status = format!("undo {}", change.label);
        self.step_tables(&change, false);
        self.step_server(&change, false);
        let mut touched = Vec::new();
        for key in change.tiles() {
            let coord = (key.x, key.y);
            self.open(assets, coord);
            if let Some(tile) = self.tiles.get_mut(&coord) {
                change.revert(&key, tile);
                self.publish(coord);
                for chunk in change.chunks(&key) {
                    self.touched(coord, chunk);
                }
                // …and the placements, which are neither a chunk nor in one.
                // A change that renumbers the lists marks the whole tile stale
                // instead: no index survives it, so reconciling entry by entry
                // would move the wrong ones.
                match change.renumbers_placements(&key) {
                    true => {
                        self.stale.insert(coord);
                    }
                    false => {
                        for index in change.placements(&key) {
                            self.moved(coord, index);
                        }
                        for index in change.buildings(&key) {
                            self.moved_building(coord, index);
                        }
                    }
                }
                // …and the paint. A change that gave a chunk a texture it did
                // not have also moves the chunk to another draw group, because
                // the set is part of the material the group was built with.
                if change.changes_the_texture_set(&key) {
                    self.regroup.insert(coord);
                }
                for chunk in change.painted(&key) {
                    self.repainted(coord, chunk);
                }
                // …and the third fork, which has no `false` arm at all: a hole
                // changes how many vertices a chunk contributes, so there is
                // nothing on screen to patch back and the tile is read again.
                if change.remeshes(&key) {
                    self.stale.insert(coord);
                }
                touched.push(coord);
            }
        }
        touched
    }

    /// …and put it back, opening what it names on [`Self::undo`]'s own terms.
    pub fn redo(&mut self, assets: &GameAssets) -> Vec<(u32, u32)> {
        let Some(change) = self.history.redo() else {
            return Vec::new();
        };
        self.status = format!("redo {}", change.label);
        self.step_tables(&change, true);
        self.step_server(&change, true);
        let mut touched = Vec::new();
        for key in change.tiles() {
            let coord = (key.x, key.y);
            self.open(assets, coord);
            if let Some(tile) = self.tiles.get_mut(&coord) {
                change.apply(&key, tile);
                self.publish(coord);
                for chunk in change.chunks(&key) {
                    self.touched(coord, chunk);
                }
                match change.renumbers_placements(&key) {
                    true => {
                        self.stale.insert(coord);
                    }
                    false => {
                        for index in change.placements(&key) {
                            self.moved(coord, index);
                        }
                        for index in change.buildings(&key) {
                            self.moved_building(coord, index);
                        }
                    }
                }
                // …and the paint. A change that gave a chunk a texture it did
                // not have also moves the chunk to another draw group, because
                // the set is part of the material the group was built with.
                if change.changes_the_texture_set(&key) {
                    self.regroup.insert(coord);
                }
                for chunk in change.painted(&key) {
                    self.repainted(coord, chunk);
                }
                // …and the third fork, which has no `false` arm at all: a hole
                // changes how many vertices a chunk contributes, so there is
                // nothing on screen to patch back and the tile is read again.
                if change.remeshes(&key) {
                    self.stale.insert(coord);
                }
                touched.push(coord);
            }
        }
        touched
    }
}
