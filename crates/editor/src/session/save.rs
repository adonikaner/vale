//! Saving tiles to the project folder, and packing the project into a patch
//! archive.

use bevy::prelude::*;
use super::EditSession;

impl EditSession {
    /// Write one tile to the project folder, or take it out of the folder
    /// when its bytes are the archives' own. Answers whether the folder now
    /// matches the tile; see [`Self::save_one`].
    pub(crate) fn save(&mut self, coord: (u32, u32)) -> bool {
        self.save_one(coord).is_some()
    }

    /// [`Self::save`], answering which of the two it did.
    ///
    /// A tile is marked unsaved by any edit, including one that is undone and
    /// a drag that ends where it began, so a save can hold exactly what the
    /// archives hold. Kept, such a file is listed as a tile the project
    /// changes, goes into every patch archive, and makes a server tile
    /// regeneration rebuild the tile. It is removed from the folder instead,
    /// and a file an earlier save wrote is removed with it, so a tile whose
    /// edits have all been taken back leaves the project.
    ///
    /// The comparison is of bytes, with the archives read past this project's
    /// own archives in `Data\`. `vale_edit::adt::diff`, which the project
    /// dialog lists tiles by, does not compare every chunk, so a tile it calls
    /// the same can still differ.
    fn save_one(&mut self, coord: (u32, u32)) -> Option<Saved> {
        let tile = self.tiles.get(&coord)?;
        let key = self.key(coord);
        let vpath = key.vpath();
        let bytes = tile.write();
        let unchanged = self.changes_nothing(&vpath, &bytes);
        let done = match unchanged {
            true => self.project.revert(&vpath).map(|_| {
                self.status = format!("{key} matches the archives, so it is not kept in the project");
                Saved::Dropped
            }),
            false => self.project.write(&vpath, &bytes).map(|path| {
                self.status = format!("saved {}", path.display());
                Saved::Written
            }),
        };
        match done {
            Ok(saved) => {
                self.unsaved.remove(&coord);
                Some(saved)
            }
            Err(e) => {
                self.status = format!("{key}: {e}");
                None
            }
        }
    }

    /// …and all of them. Answers how many tiles the folder now matches,
    /// written or removed.
    pub(crate) fn save_all(&mut self) -> usize {
        let unsaved: Vec<(u32, u32)> = self.unsaved.iter().copied().collect();
        let (mut written, mut dropped) = (0, 0);
        for coord in unsaved {
            match self.save_one(coord) {
                Some(Saved::Written) => written += 1,
                Some(Saved::Dropped) => dropped += 1,
                None => {}
            }
        }
        let root = self.project.root.display();
        match (written, dropped) {
            (0, 0) => {}
            (written, 0) => self.status = format!("saved {written} tiles to {root}"),
            (0, dropped) => {
                self.status =
                    format!("{dropped} tiles match the archives, so they are not kept in {root}")
            }
            (written, dropped) => {
                self.status = format!(
                    "saved {written} tiles to {root}; {dropped} match the archives, so they are \
                     not kept"
                )
            }
        }
        written + dropped
    }

    /// Pack the project into a patch archive beside the game's own, replacing
    /// the one it published before — see `Project::publish_into`. Answers
    /// where it landed and how many files went in, or `None` when there was
    /// nothing to pack or the write failed; the status line says which.
    pub(crate) fn publish_archive(&mut self, data_dir: &str) -> Option<(std::path::PathBuf, usize)> {
        self.save_all();
        match self.project.publish_into(data_dir) {
            Ok(report) => {
                let mut line = match &report.to {
                    Some(to) => format!("{} files written to {}", report.files, to.display()),
                    None => "nothing to publish".to_string(),
                };
                if !report.removed.is_empty() {
                    line.push_str(&format!(
                        "; this project's earlier {} removed",
                        report.removed.join(", ")
                    ));
                }
                for (name, why) in &report.left {
                    warn!("publish: {name} could not be removed: {why}");
                    line.push_str(&format!(
                        "; {name} could not be removed ({why}) and still holds the \
                         project's older files — close the program that has it open and publish again"
                    ));
                }
                self.status = line;
                report.to.map(|to| (to, report.files))
            }
            Err(e) => {
                self.status = format!("publish: {e}");
                None
            }
        }
    }

}

/// What [`EditSession::save_one`] did with a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Saved {
    /// The tile differs from the archives' copy, and its bytes are in the
    /// project folder.
    Written,
    /// The tile is the archives' copy, and the project folder has no file
    /// for it.
    Dropped,
}
