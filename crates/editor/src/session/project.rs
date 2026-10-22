//! Switching the project the session writes to, clearing it, and reading the
//! drawn tiles again afterwards.

use vale_client::assets::GameAssets;
use vale_edit::project::Project;
use vale_edit::undo::History;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::sync::{Arc, RwLock};
use super::{EditSession, overlay_over};
use super::tiles::claimed_tiles;
use super::rows::{read_server_edits, read_server_paths, read_server_scripts};
use crate::places::map_directories;

impl EditSession {
    /// Change which project this session writes to.
    ///
    /// A project is a folder of loose files and an overlay over the archives, so
    /// switching is four things: everything unsaved is written down first,
    /// because the alternative is losing it silently; the overlay is rebuilt
    /// over the new folder; everything read through the old one is forgotten —
    /// tiles, tables, the undo stack, the claims; and the world is told to read
    /// itself again, which is what puts the new project's ground on screen.
    ///
    /// The history goes. An undo entry holds bytes belonging to a file in
    /// the folder that was open; replaying one against another project's file
    /// would write bytes from one map's tile into another's. The stack is
    /// therefore dropped, so that `Ctrl+Z` cannot corrupt the new project.
    ///
    /// `false` when the folder cannot be made, which leaves the session exactly
    /// where it was.
    pub fn switch_to(&mut self, assets: &GameAssets, name: &str) -> bool {
        if name == self.project.name {
            return true;
        }
        let saved = self.save_all();
        let tables = self.save_all_tables();
        let project = match Project::open(&assets.root, name) {
            Ok(project) => project,
            Err(e) => {
                self.status = format!("could not open the project {name}: {e}");
                return false;
            }
        };

        self.adopt(assets, project);
        vale_edit::project::remember_opened(&assets.root, name);
        self.status = match (saved, tables) {
            (0, 0) => format!("opened {name}; the previous project had nothing unsaved"),
            (tiles, 0) => format!("opened {name}; saved {tiles} tiles of the previous project"),
            (0, count) => format!("opened {name}; saved {count} tables of the previous project"),
            (tiles, count) => format!("opened {name}; saved {tiles} tiles and {count} tables of the previous project"),
        };
        true
    }

    /// Take a project as the one this session writes to, forgetting
    /// everything read through whatever was open before.
    ///
    /// The half of [`Self::switch_to`] that is not about saving, so that
    /// [`Self::clear_open_project`] can reach it: emptying the open folder
    /// leaves the session holding tiles, tables and undo entries that came
    /// out of files which no longer exist, and every one of those has to go
    /// for the same reason a switch drops them.
    fn adopt(&mut self, assets: &GameAssets, project: Project) {
        self.edited = Arc::new(RwLock::new(HashMap::default()));
        assets.set_overlay(Some(overlay_over(&project, &self.edited)));
        self.project = project;
        self.tiles.clear();
        // The locks are the project's, read from its folder.
        self.locks.clear();
        self.placement_locks = None;
        self.tables.clear();
        self.unsaved.clear();
        self.unsaved_tables.clear();
        // …and every set naming work that is still owed on a tile. They are
        // keyed by coordinate and survived a project switch, so a tile marked
        // stale or unsaved under the old project was still named under the new
        // one — which for a project that had just been emptied meant the
        // session wrote its files back out again. Nothing in them can be true
        // of a project that has not been opened yet.
        self.dirty.clear();
        self.regrow.clear();
        self.moved.clear();
        self.moved_buildings.clear();
        self.repaint.clear();
        self.stale.clear();
        self.restream.clear();
        self.history = History::new();
        self.table_revision += 1;
        self.revision.clear();
        self.unpublished.clear();
        self.claimed = claimed_tiles(assets, &self.project, &self.map);
        self.claims_for = self.map.clone();
        // The maps are `Map.dbc`'s, which the project may change: a map the
        // old project made is not one this project has. Read again through
        // the new overlay, the list kept the old project's maps, and a new map
        // given the same id was listed twice.
        self.maps = map_directories(assets);
        // The server's half is a project's too. Its edits live in the
        // project folder, so a switch reads the new folder's and forgets the
        // old one's exactly as it does for the tiles and the tables — without
        // this, the rows one project changes would be applied under another
        // project's name on its first save.
        self.server_edits = read_server_edits(&self.project);
        self.server_paths = read_server_paths(&self.project);
        self.server_scripts = read_server_scripts(&self.project);
        self.server_edit_revision += 1;
        self.server_edits_unsaved = false;
        self.server_paths_unsaved = false;
        self.server_scripts_unsaved = false;
        // A different project's rows are a different set of statements, so
        // what the last one applied says nothing about whether this one has to.
        self.applied_signature = None;
        self.applied = [None; crate::server::stack::Subject::COUNT];
        // Every path the overlay answers now comes out of a different folder.
        self.republished_all = true;
        self.tables_republished = true;
        // Everything on screen came out of the old folder. See
        // [`Self::reread_everything`].
        self.reread_everything = true;
        // Every picker listing the project's own files lists another folder's.
        self.files_revision += 1;
        // The new folder may hold tiles it does not change. A check still
        // running for the old folder is dropped with the task it is in.
        self.unchanged = self
            .shipped
            .as_ref()
            .map(|shipped| shipped.look_for_unchanged(&self.project));
    }

    /// Throw away every file in the open project, and everything this
    /// session read out of them.
    ///
    /// What `default` gets where another project gets deleted: the folder is
    /// the one edits land in when nobody has said otherwise, so it stays and
    /// its contents go — see `vale_edit::project::clear`.
    ///
    /// Nothing is saved first, unlike a switch, because a save would write
    /// the same files that are being thrown away. The caller must have asked
    /// the person before calling this.
    ///
    /// Returns how many files went, or `None` when the folder would not
    /// empty; the status line says which.
    pub fn clear_open_project(&mut self, assets: &GameAssets) -> Option<usize> {
        let name = self.project.name.clone();
        let went = match vale_edit::project::clear(&assets.root, &name) {
            Ok(went) => went,
            Err(e) => {
                self.status = format!("could not clear {name}: {e}");
                return None;
            }
        };
        // Re-opened rather than kept: the folder is the same one, and what
        // has to be dropped is everything this session read out of it.
        match Project::open(&assets.root, &name) {
            Ok(project) => self.adopt(assets, project),
            Err(e) => {
                self.status = format!("{name} was cleared but could not be reopened: {e}");
                return Some(went);
            }
        }
        self.status = match went {
            0 => format!("{name} was already empty"),
            1 => format!("{name}: 1 file permanently deleted"),
            n => format!("{name}: {n} files permanently deleted"),
        };
        Some(went)
    }

}

/// Read every drawn tile again after the project changed.
///
/// The ground on screen was built from bytes the old project's overlay
/// answered with, and nothing about it is wrong in a way a patch could fix — a
/// different project is a different file. So every tile that is drawn is made
/// stale, which is the same route a texture edit takes: the streamer reads it
/// again and `terrain::swap` puts the new one up in one frame, leaving the old
/// one on screen until then.
pub fn reread_after_a_switch(
    session: Option<ResMut<EditSession>>,
    tiles: Query<&vale_client::render::terrain::TerrainTile>,
) {
    let Some(mut session) = session else { return };
    if !session.reread_everything {
        return;
    }
    session.reread_everything = false;
    let drawn: Vec<(u32, u32)> = tiles.iter().map(|tile| tile.coord).collect();
    for coord in drawn {
        session.stale.insert(coord);
    }
}
