//! The open tiles: opening, closing and switching maps, the overlay entry
//! for one tile, the change counts and live sets a tool feeds, and the
//! tiles a map's WDT claims.

use vale_client::assets::GameAssets;
use vale_edit::adt::AdtFile;
use vale_edit::TileKey;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use super::EditSession;

impl EditSession {
    /// The key a tile is edited under.
    pub(crate) fn key(&self, coord: (u32, u32)) -> TileKey {
        TileKey::new(self.map.clone(), coord.0, coord.1)
    }

    /// Parse a tile for editing, if it is not open already.
    ///
    /// Reads it through the same chain the renderer does, so an already-edited
    /// tile comes back as edited and a fresh one comes out of the archives.
    /// Returns false for a tile the map does not have, which is most of the 64
    /// by 64 grid on every map.
    pub(crate) fn open(&mut self, assets: &GameAssets, coord: (u32, u32)) -> bool {
        if self.tiles.contains_key(&coord) {
            return true;
        }
        // The map's locked placements, read with its first tile so that every
        // reader after this can answer without the session borrowed mutably.
        self.read_placement_locks();
        let path = self.key(coord).vpath();
        let bytes = match assets.with_archive(|chain| Ok(chain.read(&path).ok())) {
            Ok(Some(bytes)) => bytes,
            _ => return false,
        };
        match AdtFile::parse(&bytes) {
            Ok(tile) => {
                self.tiles.insert(coord, tile);
                true
            }
            Err(e) => {
                self.status = format!("{path}: {e}");
                false
            }
        }
    }

    /// Take a tile read and parsed elsewhere as open, unless one is already
    /// open at that place. `crate::map::open::open_tiles` reads tiles on the task
    /// pool and hands them over here; [`Self::open`] is the same on the
    /// calling thread.
    pub(crate) fn insert_opened(&mut self, coord: (u32, u32), tile: AdtFile) {
        if self.tiles.contains_key(&coord) {
            return;
        }
        self.read_placement_locks();
        self.tiles.insert(coord, tile);
    }

    /// Let go of an open tile, so that a session that has flown across a
    /// continent is not holding every tile it flew over.
    ///
    /// Only the parsed copy is dropped. What was published stays in the
    /// overlay, so a tile opened again reads back as it was — see
    /// [`Self::tile_bytes`] — and the caller is what decides which tiles may
    /// go: `crate::map::open::close_tiles`, which keeps anything unsaved or on
    /// the history.
    pub(crate) fn close(&mut self, coord: (u32, u32)) -> bool {
        match self.tiles.remove(&coord) {
            // A parsed tile is megabytes in many allocations, and a jump closes
            // a block of them at once; they are freed on the task pool rather
            // than in the frame that closes them.
            Some(tile) => {
                bevy::tasks::AsyncComputeTaskPool::get()
                    .spawn(async move { drop(tile) })
                    .detach();
                true
            }
            None => false,
        }
    }

    /// One tile's bytes, as they are now.
    ///
    /// An open tile answers from what is in hand, unsaved edits included; a tile
    /// that is not open is read through the same chain the renderer uses, so a
    /// tile edited in a previous session answers as edited. `None` for a tile
    /// the map does not have.
    ///
    /// This is what a copy reads — see `crate::map::tiles::paste_into`. Copying
    /// from disk instead would silently take the version before whatever the
    /// person had just done to it.
    pub(crate) fn tile_bytes(&self, assets: &GameAssets, coord: (u32, u32)) -> Option<Vec<u8>> {
        if let Some(tile) = self.tiles.get(&coord) {
            return Some(tile.write());
        }
        let path = self.key(coord).vpath();
        assets
            .with_archive(|chain| Ok(chain.read(&path).ok()))
            .ok()
            .flatten()
    }

    /// Make a tile's path answer with nothing.
    ///
    /// The overlay is asked before the archives, so an entry of zero bytes is a
    /// path that reads back empty. `Adt::parse` of nothing fails, and the
    /// streamer treats that failure as "there is no tile here", so a deleted
    /// tile stops being drawn.
    ///
    /// A tombstone rather than a removal, because the tile is still in the
    /// archives: dropping the override would hand the shipped bytes straight
    /// back. `crate::map::tiles::create` and a paste take it off again with
    /// [`Self::clear_tombstone`] after they write the tile's file.
    pub(crate) fn tombstone(&mut self, coord: (u32, u32)) {
        let path = self.key(coord).vpath().to_ascii_lowercase();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path, Vec::new());
        }
        self.tiles.remove(&coord);
        self.restream.insert(coord);
    }

    /// Take a [`Self::tombstone`] off a tile's path, so the project folder or
    /// the archives answer for it again. An overlay entry with bytes in it is
    /// an edit and is left.
    pub(crate) fn clear_tombstone(&mut self, coord: (u32, u32)) {
        let path = self.key(coord).vpath().to_ascii_lowercase();
        if let Ok(mut edited) = self.edited.write() {
            if edited.get(&path).is_some_and(Vec::is_empty) {
                edited.remove(&path);
            }
        }
    }

    /// Drop everything the session holds about a tile of the open map whose
    /// file has been deleted from the project: the parsed copy and its overlay
    /// entry, every set of pending work, its undo entries and its locked
    /// vertices. Unsaved edits are dropped, not saved. Answers how many undo
    /// entries were dropped.
    ///
    /// The streamer is told to read the tile again, and the path then answers
    /// with the archives' tile, or with nothing for a tile the archives do not
    /// have. A tile the map no longer claims is given a [`Self::tombstone`] by
    /// the caller. The two revision counters move, so a re-read already in
    /// flight is not taken for the new state; see [`Self::changes`].
    pub(crate) fn forget_tile(&mut self, coord: (u32, u32)) -> usize {
        let key = self.key(coord);
        self.tiles.remove(&coord);
        for set in [
            &mut self.dirty,
            &mut self.regrow,
            &mut self.moved,
            &mut self.moved_buildings,
            &mut self.repaint,
            &mut self.replant,
        ] {
            set.remove(&coord);
        }
        self.regroup.remove(&coord);
        self.stale.remove(&coord);
        self.unsaved.remove(&coord);
        self.unpublished.remove(&coord);
        self.locks.remove(&coord);
        *self.revision.entry(coord).or_default() += 1;
        *self.changes.entry(coord).or_default() += 1;
        if let Ok(mut edited) = self.edited.write() {
            edited.remove(&key.vpath().to_ascii_lowercase());
        }
        self.restream.insert(coord);
        self.history.forget_tile(&key)
    }

    /// Read the open map's claims again from its WDT, the project's copy
    /// first, after that copy was written or deleted outside the map window.
    /// A tile the map stops claiming is taken off the screen with a
    /// [`Self::tombstone`]; a tile it starts claiming loses any tombstone an
    /// earlier unclaim left, and the streamer is told to read it.
    pub(crate) fn reread_claims(&mut self, assets: &GameAssets) {
        let now = claimed_tiles(assets, &self.project, &self.map);
        let gone: Vec<(u32, u32)> = self.claimed.difference(&now).copied().collect();
        let came: Vec<(u32, u32)> = now.difference(&self.claimed).copied().collect();
        for coord in gone {
            self.tombstone(coord);
        }
        for coord in came {
            self.clear_tombstone(coord);
            self.restream.insert(coord);
        }
        self.claimed = now;
        self.claims_for = self.map.clone();
    }

    /// Open another map.
    ///
    /// Everything open belongs to the old map. A tile coordinate means a
    /// different place on each one, so carrying the parsed tiles across would
    /// edit Kalimdor's ground with Azeroth's bytes. The unsaved ones are saved
    /// first rather than dropped. The height the camera was at belonged to
    /// the old map's ground; asking for the new one's puts it back on the
    /// terrain as soon as the tile under it is parsed.
    ///
    /// The top bar's map list and a camera bookmark on another map both come
    /// through here. Nothing happens for the map already open.
    pub(crate) fn switch_map(&mut self, name: String, id: u32, camera: &mut crate::camera::EditorCamera) {
        if self.map == name {
            return;
        }
        // A save writes everything; see `Self::save_files`.
        let _ = self.save_files();
        self.tiles.clear();
        // Keyed by tile coordinate, which names a different tile on the next
        // map.
        self.locks.clear();
        self.placement_locks = None;
        self.unpublished.clear();
        self.stale.clear();
        self.map = name;
        self.map_id = id;
        camera.wants_the_ground = true;
    }

    /// Write an open tile's bytes into the overlay, so the next read of its path
    /// answers with them.
    ///
    /// After this, everything that reads the path sees the edit. It does not
    /// touch the disk: [`EditSession::save`] does that.
    pub(crate) fn publish(&mut self, coord: (u32, u32)) {
        let Some(tile) = self.tiles.get(&coord) else {
            return;
        };
        let path = self.key(coord).vpath().to_ascii_lowercase();
        let bytes = tile.write();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path, bytes);
        }
        self.unsaved.insert(coord);
        *self.revision.entry(coord).or_default() += 1;
        *self.changes.entry(coord).or_default() += 1;
        self.unpublished.remove(&coord);
    }

    /// How many times a tile has changed in memory or been published. See
    /// [`Self::changes`], the field.
    pub(crate) fn changes(&self, coord: (u32, u32)) -> u64 {
        self.changes.get(&coord).copied().unwrap_or_default()
    }

    /// Whether an open tile's bytes are ahead of what was last published.
    pub(crate) fn is_unpublished(&self, coord: (u32, u32)) -> bool {
        self.unpublished.contains(&coord)
    }

    /// Note an in-memory change to a tile, for [`Self::changes`] and
    /// [`Self::is_unpublished`].
    fn changed(&mut self, coord: (u32, u32)) {
        *self.changes.entry(coord).or_default() += 1;
        self.unpublished.insert(coord);
    }

    /// Note that a chunk's heights have moved, so what stands on them can be
    /// caught up: the ground on screen this frame, and its lawn when the stroke
    /// is over.
    pub(crate) fn touched(&mut self, coord: (u32, u32), chunk: usize) {
        self.dirty.entry(coord).or_default().insert(chunk);
        self.regrow.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// Note that a chunk's shading has changed, which moves a vertex attribute
    /// and nothing else.
    ///
    /// `dirty` and not `regrow`: `MCCV` does not move the ground, so the grass
    /// standing on it is exactly where it was. Marking it would rebuild a
    /// chunk's whole lawn on every shading stroke for no change at all — which
    /// is what calling [`Self::touched`] here would have done, and is the only
    /// reason this is its own method rather than that one.
    pub(crate) fn shaded(&mut self, coord: (u32, u32), chunk: usize) {
        self.dirty.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// Note that one `MDDF` entry has moved, so its drawn copy has to be put
    /// where its record now says.
    pub(crate) fn moved(&mut self, coord: (u32, u32), index: usize) {
        self.moved.entry(coord).or_default().insert(index);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// Note that one `MODF` entry has moved, as [`Self::moved`] does for an
    /// `MDDF` entry.
    pub(crate) fn moved_building(&mut self, coord: (u32, u32), index: usize) {
        self.moved_buildings.entry(coord).or_default().insert(index);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// Note that a chunk's paint has changed: its blend maps on the GPU are
    /// behind the file, and its foliage is behind it once the stroke is over.
    pub(crate) fn repainted(&mut self, coord: (u32, u32), chunk: usize) {
        self.repaint.entry(coord).or_default().insert(chunk);
        self.replant.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// Whether the map claims this tile: its bit in the WDT's `MAIN` grid, read
    /// from [`EditSession::claimed`].
    pub(crate) fn wdt_claims(&self, coord: (u32, u32)) -> bool {
        self.claimed.contains(&coord)
    }

    /// Record that a tile has been claimed or unclaimed, beside the file.
    pub(crate) fn set_claimed(&mut self, coord: (u32, u32), claimed: bool) {
        match claimed {
            true => self.claimed.insert(coord),
            false => self.claimed.remove(&coord),
        };
    }

}

/// Put [`EditSession::claimed`] back in step when the map changes.
///
/// A comparison rather than an event, so it cannot be forgotten by whatever
/// changes the map next: the drop-down does not have the archives to hand and
/// should not have to.
pub(crate) fn refresh_claims(
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut view: ResMut<crate::ui::map::mapview::MapView>,
) {
    let Some(mut session) = session else { return };
    if session.claims_for == session.map {
        return;
    }
    let map = session.map.clone();
    session.claimed = claimed_tiles(&assets, &session.project, &map);
    session.claims_for = map;
    // And nothing stays selected. A tile coordinate names a different place
    // on every map, so carrying a selection across would point the next Delete
    // at whatever happens to sit at those coordinates now.
    view.selection.clear();
    view.clipboard.clear();
}

/// Which tiles a map's WDT claims, the project's copy first.
pub(super) fn claimed_tiles(
    assets: &GameAssets,
    project: &vale_edit::project::Project,
    map: &str,
) -> HashSet<(u32, u32)> {
    let path = vale_edit::wdt::wdt_path(map);
    let bytes = project.read(&path).or_else(|| {
        assets
            .with_archive(|chain| Ok(chain.read(&path).ok()))
            .ok()
            .flatten()
    });
    let Some(bytes) = bytes else {
        error!("{path}: not found, so no tile is known to exist");
        return HashSet::default();
    };
    match vale_edit::wdt::WdtFile::parse(&bytes) {
        Ok(wdt) => wdt.tiles().into_iter().collect(),
        Err(e) => {
            error!("{path}: {e}");
            HashSet::default()
        }
    }
}
