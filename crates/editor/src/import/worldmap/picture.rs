//! A map's picture: the twelve sectors, the picture the archives and the
//! project hold now, and a new picture for some or all of the sectors.
//!
//! The sectors chosen on the picture are what Import picture, Draw from
//! minimap and Remove from project act on; with none chosen they act on the
//! whole map. A picture file is fitted to the box the chosen sectors cover
//! (`vale_edit::worldmap::paste`) and changes only them; a drawing from the
//! minimap is of the whole rectangle, and only the chosen sectors of it are
//! kept. Either is a [`Pending`] picture: the current one with those sectors
//! replaced, shown until it is written or discarded. Write saves the changed
//! pieces only.

use std::path::{Path, PathBuf};

use bevy::tasks::{block_on, futures_lite::future, Task};

use super::{map_directory, Entry, Which};
use crate::jobs::Progress;
use crate::session::EditSession;
use vale_assets::tables::minimap::{MinimapTiles, MD5_TRANSLATE};
use vale_assets::world::blp;
use vale_client::assets::GameAssets;
use vale_edit::worldmap::{self as pieces, Drawn, Fit, Sectors};

/// A picture waiting to be written over a map's.
pub(crate) struct Pending {
    pub(crate) which: Which,
    /// The current picture it was made over, kept so a change of [`Fit`]
    /// pastes the file again.
    base: Vec<u8>,
    /// `vale_edit::worldmap::WIDTH` by `HEIGHT` RGBA.
    pub(crate) picture: Vec<u8>,
    /// The sectors it changes, which are the pieces Write saves.
    pub(crate) sectors: Sectors,
    pub(crate) source: Source,
    /// Changed whenever [`Self::picture`] is, so the panel knows to upload it
    /// again.
    pub(crate) stamp: u64,
}

/// Where a pending picture came from.
pub(crate) enum Source {
    /// A picture file, kept at its own size so a change of [`Fit`] fits it
    /// again.
    File { path: PathBuf, width: u32, height: u32, rgba: Vec<u8>, fit: Fit },
    /// The minimap: how many tiles under the rectangle had a picture and how
    /// many had none.
    Minimap { drawn: usize, missing: usize },
}

/// A map's picture as the archives and the project hold it now.
pub(crate) struct Current {
    pub(crate) which: Which,
    directory: String,
    pub(crate) picture: Vec<u8>,
    /// How many of the twelve pieces were found.
    pub(crate) found: u32,
    /// Which pieces the project holds.
    pub(crate) own: Sectors,
    /// [`EditSession::files_revision`] when it was read.
    revision: u64,
    pub(crate) stamp: u64,
}

/// A minimap picture being drawn.
pub(crate) struct Drawing {
    which: Which,
    sectors: Sectors,
    base: Vec<u8>,
    pub(crate) progress: Progress,
    task: Task<Drawn>,
}

/// The picture's state. See the module comment.
#[derive(Default)]
pub(crate) struct Picture {
    /// The sectors chosen on the picture.
    pub(crate) selected: Sectors,
    pub(crate) pending: Option<Pending>,
    /// Whether the panel shows the current picture rather than the pending
    /// one.
    pub(crate) showing_current: bool,
    pub(crate) drawing: Option<Drawing>,
    current: Option<Current>,
    stamps: u64,
}

impl Picture {
    /// Drop what belongs to the map that was chosen.
    pub(crate) fn leave(&mut self) {
        self.selected = 0;
        self.pending = None;
        self.drawing = None;
        self.showing_current = false;
    }

    /// The sectors an action changes: the chosen ones, or all twelve.
    pub(crate) fn target(&self) -> Sectors {
        match self.selected {
            0 => pieces::EVERY,
            chosen => chosen,
        }
    }

    /// The pending picture of `which`, if it has one.
    pub(crate) fn pending_of(&self, which: Which) -> Option<&Pending> {
        self.pending.as_ref().filter(|pending| pending.which == which)
    }

    /// The map's picture as it is now, read again when the project's files
    /// change.
    pub(crate) fn current(&mut self, entry: &Entry, session: &EditSession, assets: &GameAssets) -> &Current {
        let fresh = self.current.as_ref().is_some_and(|current| {
            current.which == entry.which && current.directory == entry.directory && current.revision == session.files_revision
        });
        if !fresh {
            let directory = entry.directory.as_str();
            let (picture, found) = pieces::join(|n| {
                let bytes = assets.with_archive(|chain| Ok(chain.read(&pieces::piece_path(directory, n)).ok())).ok().flatten()?;
                let decoded = blp::decode(&bytes).ok()?;
                Some((decoded.width, decoded.height, decoded.rgba))
            });
            let own = (0..pieces::PIECES)
                .filter(|n| session.project.has(&pieces::piece_path(directory, n + 1)))
                .fold(0, |mask, n| mask | (1 << n));
            self.stamps += 1;
            self.current = Some(Current {
                which: entry.which,
                directory: entry.directory.clone(),
                picture,
                found,
                own,
                revision: session.files_revision,
                stamp: self.stamps,
            });
        }
        self.current.as_ref().expect("read above")
    }

    /// The current picture of `entry`, which a new picture is made over.
    fn base_of(&mut self, entry: &Entry, session: &EditSession, assets: &GameAssets) -> Vec<u8> {
        self.current(entry, session, assets).picture.clone()
    }

    /// Make `picture` the pending picture of `which`, changing `sectors`.
    fn hold(&mut self, which: Which, base: Vec<u8>, picture: Vec<u8>, sectors: Sectors, source: Source) {
        self.stamps += 1;
        self.pending = Some(Pending { which, base, picture, sectors, source, stamp: self.stamps });
        self.showing_current = false;
    }

    /// Read a picture file into the chosen sectors of `entry`'s picture.
    pub(crate) fn read_file(&mut self, entry: &Entry, session: &EditSession, assets: &GameAssets, path: &Path) -> Result<(), String> {
        let which = entry.which;
        let (width, height, rgba) = crate::import::assets::read_picture(path)?;
        let sectors = self.target();
        let base = self.base_of(entry, session, assets);
        let mut picture = base.clone();
        // A picture of the right shape is the same either way; stretching it
        // loses nothing.
        let fit = Fit::Stretch;
        pieces::paste(&mut picture, &rgba, width, height, fit, sectors);
        self.hold(which, base, picture, sectors, Source::File { path: path.to_path_buf(), width, height, rgba, fit });
        Ok(())
    }

    /// Fit the pending picture file again, as `fit` says.
    pub(crate) fn refit(&mut self, fit: Fit) {
        let Some(pending) = self.pending.as_mut() else { return };
        let Source::File { width, height, rgba, fit: was, .. } = &mut pending.source else { return };
        if *was == fit {
            return;
        }
        *was = fit;
        pending.picture = pending.base.clone();
        pieces::paste(&mut pending.picture, rgba, *width, *height, fit, pending.sectors);
        self.stamps += 1;
        pending.stamp = self.stamps;
    }

    /// Start drawing the map's rectangle from the minimap, off the main
    /// thread, for the chosen sectors.
    pub(crate) fn draw(&mut self, entry: &Entry, session: &mut EditSession, assets: &GameAssets) -> Result<(), String> {
        let Some(area) = entry.area.clone() else {
            return Err("the whole world has no rectangle to draw from the minimap; import a picture for it".to_string());
        };
        let directory = map_directory(session, assets, area.map)
            .ok_or_else(|| format!("map {} is not in Map.dbc, so its minimap tiles cannot be found", area.map))?;
        let read = assets.shared_reader();
        let minimaps = MinimapTiles::parse(&read(MD5_TRANSLATE).unwrap_or_default());
        let progress = Progress::new(pieces::HEIGHT as usize);
        let worker = progress.clone();
        let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
            pieces::from_minimap(
                &area,
                |col, row| {
                    let path = minimaps.texture(&directory, col, row)?;
                    let picture = blp::decode(&read(&path)?).ok()?;
                    (picture.width == pieces::TILE_SIDE && picture.height == pieces::TILE_SIDE).then_some(picture.rgba)
                },
                |row| worker.set(row as usize),
            )
        });
        self.pending = None;
        let base = self.base_of(entry, session, assets);
        self.drawing = Some(Drawing { which: entry.which, sectors: self.target(), base, progress, task });
        Ok(())
    }

    /// Take a finished drawing as the pending picture. Answers what it says
    /// about the tiles.
    pub(crate) fn collect(&mut self) -> Option<String> {
        let drawing = self.drawing.as_mut()?;
        let drawn = block_on(future::poll_once(&mut drawing.task))?;
        let Drawing { which, sectors, base, .. } = self.drawing.take().expect("polled above");
        let said = match (drawn.drawn, drawn.missing) {
            (0, _) => "drawn black: no tile under the rectangle has a minimap picture".to_string(),
            (n, 0) => format!("drawn from {n} minimap tiles"),
            (n, m) => format!("drawn from {n} minimap tiles; {m} under the rectangle have none and are black"),
        };
        let mut picture = base.clone();
        pieces::copy_sectors(&mut picture, &drawn.picture, sectors);
        self.hold(which, base, picture, sectors, Source::Minimap { drawn: drawn.drawn, missing: drawn.missing });
        Some(said)
    }

    /// Write the pending picture's changed pieces. Answers what happened.
    pub(crate) fn write(&mut self, entry: &Entry, session: &mut EditSession) -> String {
        let Some(pending) = self.pending_of(entry.which) else {
            return "there is no new picture to write".to_string();
        };
        let sectors = pending.sectors;
        let cut = pieces::pieces(&pending.picture);
        let mut written = 0;
        for (index, piece) in cut.iter().enumerate().filter(|(n, _)| sectors & (1 << n) != 0) {
            let path = pieces::piece_path(&entry.directory, index as u32 + 1);
            let bytes = match blp::encode_dxt1(piece, pieces::PIECE, pieces::PIECE) {
                Ok(bytes) => bytes,
                Err(why) => return format!("{path} could not be encoded: {why}"),
            };
            if !session.save_bytes(&path, bytes) {
                return format!("{written} pieces written; {}", session.status);
            }
            written += 1;
        }
        self.pending = None;
        self.selected = 0;
        format!("wrote {written} pieces to Interface\\WorldMap\\{}\\", entry.directory)
    }

    /// Remove the project's pieces of the chosen sectors, or of the whole map.
    /// Answers what happened.
    pub(crate) fn remove(&mut self, entry: &Entry, session: &mut EditSession) -> String {
        let sectors = self.target();
        let mut removed = 0;
        for n in (0..pieces::PIECES).filter(|n| sectors & (1 << n) != 0) {
            let path = pieces::piece_path(&entry.directory, n + 1);
            if session.project.has(&path) && session.remove_file(&path) {
                removed += 1;
            }
        }
        format!("removed {removed} pieces of {} from the project", entry.directory)
    }
}
