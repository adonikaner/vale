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
    pub fn key(&self, coord: (u32, u32)) -> TileKey {
        TileKey::new(self.map.clone(), coord.0, coord.1)
    }

    /// Parse a tile for editing, if it is not open already.
    ///
    /// Reads it through the same chain the renderer does, so an already-edited
    /// tile comes back as edited and a fresh one comes out of the archives.
    /// Returns false for a tile the map does not have, which is most of the 64
    /// by 64 grid on every map.
    pub fn open(&mut self, assets: &GameAssets, coord: (u32, u32)) -> bool {
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
    pub fn insert_opened(&mut self, coord: (u32, u32), tile: AdtFile) {
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
    pub fn close(&mut self, coord: (u32, u32)) -> bool {
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
    /// This is what a copy reads — see `crate::map::tiles::paste`. Copying
    /// from disk instead would silently take the version before whatever the
    /// person had just done to it.
    pub fn tile_bytes(&self, assets: &GameAssets, coord: (u32, u32)) -> Option<Vec<u8>> {
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
    /// back. It is undone by `create`, which writes real bytes over it.
    pub fn tombstone(&mut self, coord: (u32, u32)) {
        let path = self.key(coord).vpath().to_ascii_lowercase();
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(path, Vec::new());
        }
        self.tiles.remove(&coord);
        self.restream.insert(coord);
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
    pub fn switch_map(&mut self, name: String, id: u32, camera: &mut crate::camera::EditorCamera) {
        if self.map == name {
            return;
        }
        self.save_all();
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
    pub fn publish(&mut self, coord: (u32, u32)) {
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
    pub fn changes(&self, coord: (u32, u32)) -> u64 {
        self.changes.get(&coord).copied().unwrap_or_default()
    }

    /// Whether an open tile's bytes are ahead of what was last published.
    pub fn is_unpublished(&self, coord: (u32, u32)) -> bool {
        self.unpublished.contains(&coord)
    }

    /// Note an in-memory change to a tile, for [`Self::changes`] and
    /// [`Self::is_unpublished`].
    fn changed(&mut self, coord: (u32, u32)) {
        *self.changes.entry(coord).or_default() += 1;
        self.unpublished.insert(coord);
    }

    /// How many times this tile's bytes have been republished — see
    /// [`Self::revision`], the field, for what reads it and why.
    pub fn revision(&self, coord: (u32, u32)) -> u64 {
        self.revision.get(&coord).copied().unwrap_or_default()
    }

    /// Note that a chunk's heights have moved, so what stands on them can be
    /// caught up: the ground on screen this frame, and its lawn when the stroke
    /// is over.
    pub fn touched(&mut self, coord: (u32, u32), chunk: usize) {
        self.dirty.entry(coord).or_default().insert(chunk);
        self.regrow.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and for one chunk's shading, which moves a vertex attribute and
    /// nothing else.
    ///
    /// `dirty` and not `regrow`: `MCCV` does not move the ground, so the grass
    /// standing on it is exactly where it was. Marking it would rebuild a
    /// chunk's whole lawn on every shading stroke for no change at all — which
    /// is what calling [`Self::touched`] here would have done, and is the only
    /// reason this is its own method rather than that one.
    pub fn shaded(&mut self, coord: (u32, u32), chunk: usize) {
        self.dirty.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and the same for one `MDDF` entry, whose drawn copy has to be put where
    /// its record now says.
    pub fn moved(&mut self, coord: (u32, u32), index: usize) {
        self.moved.entry(coord).or_default().insert(index);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and the same for one `MODF` entry.
    pub fn moved_building(&mut self, coord: (u32, u32), index: usize) {
        self.moved_buildings.entry(coord).or_default().insert(index);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// …and for one chunk's paint, whose blend maps on the GPU are behind the
    /// file.
    pub fn repainted(&mut self, coord: (u32, u32), chunk: usize) {
        self.repaint.entry(coord).or_default().insert(chunk);
        self.unsaved.insert(coord);
        self.changed(coord);
    }

    /// Does the map claim this tile? — `MAIN`'s one bit, off
    /// [`EditSession::claimed`].
    pub fn wdt_claims(&self, coord: (u32, u32)) -> bool {
        self.claimed.contains(&coord)
    }

    /// Note that a tile's bytes are ahead of what is written down.
    ///
    /// The other edits reach this through `touched` and its neighbours, which
    /// also mark the ground or the foliage stale. A shadow rebake changes
    /// neither — nothing on screen is drawn from `MCSH` — so it needs the
    /// unsaved mark on its own.
    pub fn mark_unsaved(&mut self, coord: (u32, u32)) {
        self.unsaved.insert(coord);
    }

    /// Record that a tile has been claimed or unclaimed, beside the file.
    pub fn set_claimed(&mut self, coord: (u32, u32), claimed: bool) {
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
pub fn refresh_claims(
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
