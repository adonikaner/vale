//! Locked vertices and locked placements: what no edit may move, read from
//! and written to the project as they change.

use bevy::prelude::*;
use super::EditSession;

/// The doodads and WMOs of one map that are locked: no tool selects, moves or
/// deletes them, and moving the ground does not carry them. Kept by
/// `uniqueId`, which every copy of a placement in the tiles it crosses shares
/// and which does not change when other placements are added or removed.
///
/// Stored in the project as `editor\locks\<Map>.placements.txt`, one line per
/// placement: `doodad <uniqueId>` or `wmo <uniqueId>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PlacementLocks {
    pub(crate) doodads: std::collections::BTreeSet<u32>,
    pub(crate) wmos: std::collections::BTreeSet<u32>,
}

impl PlacementLocks {
    /// The set for one kind of placement.
    pub(crate) fn of(&self, kind: crate::tools::placements::place::Kind) -> &std::collections::BTreeSet<u32> {
        match kind {
            crate::tools::placements::place::Kind::Doodad => &self.doodads,
            crate::tools::placements::place::Kind::Wmo => &self.wmos,
        }
    }

    fn of_mut(&mut self, kind: crate::tools::placements::place::Kind) -> &mut std::collections::BTreeSet<u32> {
        match kind {
            crate::tools::placements::place::Kind::Doodad => &mut self.doodads,
            crate::tools::placements::place::Kind::Wmo => &mut self.wmos,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.doodads.is_empty() && self.wmos.is_empty()
    }

    /// The stored form. See [`Self::from_text`].
    pub(crate) fn to_text(&self) -> String {
        let mut text = String::from(
            "# Locked doodads and WMOs of one map, by uniqueId, written by the world editor.\n",
        );
        for id in &self.doodads {
            text.push_str(&format!("doodad {id}\n"));
        }
        for id in &self.wmos {
            text.push_str(&format!("wmo {id}\n"));
        }
        text
    }

    /// Read [`Self::to_text`]'s form. Blank lines and `#` lines are skipped; a
    /// line that is neither `doodad <id>` nor `wmo <id>` is an error naming it.
    pub(crate) fn from_text(text: &str) -> Result<PlacementLocks, String> {
        let mut locks = PlacementLocks::default();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("line {}: {line:?} is not `doodad <id>` or `wmo <id>`", number + 1);
            let (kind, id) = line.split_once(char::is_whitespace).ok_or_else(bad)?;
            let id: u32 = id.trim().parse().map_err(|_| bad())?;
            match kind {
                "doodad" => locks.doodads.insert(id),
                "wmo" => locks.wmos.insert(id),
                _ => return Err(bad()),
            };
        }
        Ok(locks)
    }
}

impl EditSession {
    /// Where a tile's locked vertices are stored in the project, outside the
    /// game data: `editor\locks\<Map>_<x>_<y>.txt`.
    /// `editor` is one of `Project::NOT_GAME_DATA`, so a publish never packs
    /// it.
    fn locks_vpath(&self, coord: (u32, u32)) -> String {
        format!(r"editor\locks\{}_{}_{}.txt", self.map, coord.0, coord.1)
    }

    /// A tile's locked vertices: the ones no height operation may move. Read
    /// from the project the first time a tile is asked about. A file that
    /// cannot be read as locks is reported in the status line and treated as
    /// no locks.
    pub(crate) fn locked(&mut self, coord: (u32, u32)) -> &vale_edit::ops::vertices::Selected {
        if !self.locks.contains_key(&coord) {
            let vpath = self.locks_vpath(coord);
            let set = match self.project.read(&vpath) {
                None => Default::default(),
                Some(bytes) => {
                    match vale_edit::ops::vertices::Selected::from_text(&String::from_utf8_lossy(&bytes)) {
                        Ok(set) => set,
                        Err(e) => {
                            self.status = format!("{vpath}: {e}; the tile is treated as having no locked vertices");
                            Default::default()
                        }
                    }
                }
            };
            self.locks.insert(coord, set);
        }
        &self.locks[&coord]
    }

    /// Where the open map's locked placements are stored. See
    /// [`PlacementLocks`].
    fn placement_locks_vpath(&self) -> String {
        format!(r"editor\locks\{}.placements.txt", self.map)
    }

    /// Read the open map's locked placements, if they have not been read.
    pub(super) fn read_placement_locks(&mut self) {
        if self.placement_locks.is_some() {
            return;
        }
        let vpath = self.placement_locks_vpath();
        let locks = match self.project.read(&vpath) {
            None => PlacementLocks::default(),
            Some(bytes) => match PlacementLocks::from_text(&String::from_utf8_lossy(&bytes)) {
                Ok(locks) => locks,
                Err(e) => {
                    self.status = format!("{vpath}: {e}; no placement is treated as locked");
                    PlacementLocks::default()
                }
            },
        };
        self.placement_locks = Some(locks);
    }

    /// The open map's locked placements. Empty before the first tile is open.
    pub(crate) fn placement_locks(&self) -> &PlacementLocks {
        static NONE: std::sync::OnceLock<PlacementLocks> = std::sync::OnceLock::new();
        self.placement_locks
            .as_ref()
            .unwrap_or_else(|| NONE.get_or_init(PlacementLocks::default))
    }

    /// Whether one placement is locked.
    pub(crate) fn placement_locked(&self, kind: crate::tools::placements::place::Kind, unique_id: u32) -> bool {
        self.placement_locks().of(kind).contains(&unique_id)
    }

    /// Lock or unlock placements, and write the map's locks to the project at
    /// once. Returns how many changed. Like the vertex locks, these are not on
    /// the undo history.
    pub(crate) fn set_placements_locked(
        &mut self,
        kind: crate::tools::placements::place::Kind,
        ids: impl IntoIterator<Item = u32>,
        locked: bool,
    ) -> usize {
        self.read_placement_locks();
        let mut locks = self.placement_locks.clone().unwrap_or_default();
        let set = locks.of_mut(kind);
        let mut changed = 0;
        for id in ids {
            let did = match locked {
                true => set.insert(id),
                false => set.remove(&id),
            };
            changed += usize::from(did);
        }
        if changed == 0 {
            return 0;
        }
        let vpath = self.placement_locks_vpath();
        let written = match locks.is_empty() {
            true => self.project.revert(&vpath).map(|_| ()),
            false => self.project.write(&vpath, locks.to_text().as_bytes()).map(|_| ()),
        };
        if let Err(e) = written {
            self.status = format!("{vpath}: the locked placements were not saved: {e}");
        }
        self.placement_locks = Some(locks);
        changed
    }

    /// A tile's locked vertices if they have been read, for a reader that
    /// cannot take the session mutably. The terrain tool reads every open
    /// tile's locks through [`Self::locked`] each frame it is chosen.
    pub(crate) fn locks_if_read(&self, coord: (u32, u32)) -> Option<&vale_edit::ops::vertices::Selected> {
        self.locks.get(&coord)
    }

    /// Replace a tile's locked vertices, and write them to the project at once.
    /// An empty set removes the file. Locks are not on the undo history: a
    /// lock changes no file the game reads.
    pub(crate) fn set_locked(&mut self, coord: (u32, u32), set: vale_edit::ops::vertices::Selected) {
        let vpath = self.locks_vpath(coord);
        let written = match set.is_empty() {
            true => self.project.revert(&vpath).map(|_| ()),
            false => self.project.write(&vpath, set.to_text().as_bytes()).map(|_| ()),
        };
        if let Err(e) = written {
            self.status = format!("{vpath}: the locked vertices were not saved: {e}");
        }
        self.locks.insert(coord, set);
    }

    /// Put a tile's locked vertices back where `edits` found them. Every
    /// operation that moves a tile's heights calls this after writing them and
    /// before recording the edits, so a lock holds against all of them. See
    /// `vale_edit::ops::vertices::Selected::hold`. Returns how many entries
    /// were held.
    pub(crate) fn hold_locked(&mut self, coord: (u32, u32), edits: &mut Vec<vale_edit::ops::Edit>) -> usize {
        if !edits.iter().any(|edit| matches!(edit, vale_edit::ops::Edit::Heights { .. })) {
            return 0;
        }
        self.locked(coord);
        let (Some(locks), Some(tile)) = (self.locks.get(&coord), self.tiles.get_mut(&coord)) else {
            return 0;
        };
        locks.hold(tile, edits)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_edit::project::Project;

    /// The map's locked placements read back as written, are saved to the
    /// project as they change, and the file goes when the last one is
    /// unlocked.
    #[test]
    fn locked_placements_are_stored_in_the_project() {
        use crate::tools::placements::place::Kind;
        let text = "# comment
doodad 7
wmo 12

doodad 3
";
        let locks = PlacementLocks::from_text(text).unwrap();
        assert_eq!(locks.doodads.iter().copied().collect::<Vec<_>>(), [3, 7]);
        assert_eq!(locks.wmos.iter().copied().collect::<Vec<_>>(), [12]);
        assert_eq!(PlacementLocks::from_text(&locks.to_text()).unwrap(), locks);
        assert!(PlacementLocks::from_text("tree 4").unwrap_err().starts_with("line 1:"));

        let install =
            std::env::temp_dir().join(format!("vale-session-placement-locks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        assert_eq!(session.set_placements_locked(Kind::Doodad, [5, 6], true), 2);
        assert_eq!(session.set_placements_locked(Kind::Doodad, [6], true), 0, "already locked");
        assert!(session.placement_locked(Kind::Doodad, 5));
        assert!(!session.placement_locked(Kind::Wmo, 5), "a doodad id is not a WMO id");
        let vpath = session.placement_locks_vpath();
        assert!(session.project.has(&vpath));
        assert_eq!(session.set_placements_locked(Kind::Doodad, [5, 6], false), 2);
        assert!(!session.project.has(&vpath), "no locks, no file");
        let _ = std::fs::remove_dir_all(&install);
    }
}
