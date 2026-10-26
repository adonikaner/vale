//! The Game Files part of the Assets workspace's state: every file the
//! archives list, browsed by folder, and a file or a folder written out to
//! disk.
//!
//! ## What is listed, and what is read
//!
//! The list is a tree of the archive chain's own listing
//! ([`GameFiles::catalogue`]): a folder opens in place, under its parent.
//! The listing is lower case, and each name is drawn as the archives spell
//! it (`crate::session::files::Spellings`). The project's files are not
//! listed: they are the Project Files part, [`super::assets`]. A path the
//! project also has is marked, since the game reads the project's copy
//! there. What a file holds, and what an export
//! writes, is the archives' own copy, read past the project
//! (`GameAssets::read_past_overlay`), so an export is the shipped file even
//! where the project replaces it.
//!
//! ## Export
//!
//! Export writes one file's bytes unchanged. A BLP texture can be written as
//! a PNG instead ([`png_of_blp`]): its top mip decoded to RGBA, which a paint
//! program opens and the texture import reads back. Export Folder writes
//! every file under a folder into a chosen directory at its whole archive
//! path, so `Sound\Creature\Wolf\` lands in `<directory>\sound\creature\wolf\`
//! and a file's way back in is its own path. A folder export runs off the
//! main thread as a [`crate::jobs::Job`].
//!
//! ## Replace
//!
//! A replacement is an import at the file's own folder and name: the Import
//! Texture, Import Model or Import Sound dialog, opened on a file chosen with
//! the system's file dialog ([`super::assets::Ask::Replace`]). The project's
//! file at that path is then read in place of the archives'.
//!
//! The file dialogs are the system's own and block while they are open, so
//! they are run by [`answer`], outside the panel.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::prelude::*;

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::manifest::Kind;

/// The Game Files part's state.
#[derive(Resource, Default)]
pub(crate) struct GameFiles {
    /// The chosen folder, whose details and Export Folder the middle shows
    /// while no file is chosen: lower case and ending in `\`, or empty for
    /// the top.
    pub(crate) folder: String,
    /// The folders open in the tree, lower case and ending in `\`.
    pub(crate) open: std::collections::BTreeSet<String>,
    /// Moved each time [`Self::open`] changes, so the tree's rows are built
    /// again then and not every frame.
    pub(crate) open_revision: u64,
    /// Whether the list scrolls to the chosen file on its next frame: after
    /// Show in Game Files or `--game-file`, which choose a file the list has
    /// not shown.
    pub(crate) reveal: bool,
    /// The chosen file's archive path, as the listing spells it.
    pub(crate) selected: Option<String>,
    /// What the search box holds. Words in it search every path, not the
    /// open folder.
    pub(crate) query: String,
    /// Whether Export Folder writes each BLP texture as a PNG.
    pub(crate) as_png: bool,
    /// The directory the last export was written to, where the next file
    /// dialog opens.
    pub(crate) export_dir: Option<PathBuf>,
    /// What the panel asked for, carried out by [`answer`].
    pub(crate) ask: Option<Ask>,
    /// The folder export running, what it is called and how far it has got,
    /// for the panel's bar. One runs at a time.
    pub(crate) exporting: Option<(String, crate::jobs::Progress)>,
    /// Every path the archives list, read once.
    listed: Option<Arc<Vec<String>>>,
    /// The paths the project also has, lower case, and the
    /// `EditSession::files_revision` they were read at.
    own: Option<(u64, Arc<std::collections::HashSet<String>>)>,
}

/// What the Game Files part asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ask {
    /// Write one file to a path chosen with the system's save dialog: its
    /// bytes unchanged, or with `as_png` a BLP's top mip as a PNG. The
    /// archives' copy, or with `from_project` the project's, which the
    /// Project Files part exports.
    Export { vpath: String, as_png: bool, from_project: bool },
    /// Write every file under the archive folder `folder` into a directory
    /// chosen with the system's folder dialog, in the background.
    ExportFolder { folder: String },
}

/// One folder export's outcome: how many files were written, and the first
/// failures.
pub(crate) struct Exported {
    pub(crate) folder: String,
    pub(crate) into: PathBuf,
    pub(crate) written: usize,
    pub(crate) failed: Vec<String>,
}

/// The most failures an export keeps for its status line.
const FAILURES_KEPT: usize = 5;

impl GameFiles {
    /// Every path the archive chain lists, lower case and sorted. The chain
    /// does not change while the editor runs, so it is listed once.
    pub(crate) fn catalogue(&mut self, game: &GameAssets) -> Arc<Vec<String>> {
        if self.listed.is_none() {
            let listed = game.with_archive(|chain| Ok(chain.list_prefix(""))).unwrap_or_default();
            self.listed = Some(Arc::new(listed));
        }
        self.listed.clone().unwrap_or_default()
    }

    /// The archive paths, lower case, that the project has a file at.
    pub(crate) fn own(&mut self, session: &EditSession) -> Arc<std::collections::HashSet<String>> {
        let current = self.own.as_ref().is_some_and(|(at, _)| *at == session.files_revision);
        if !current {
            let own = session.project.files().into_iter().map(|vpath| vpath.to_ascii_lowercase()).collect();
            self.own = Some((session.files_revision, Arc::new(own)));
        }
        self.own.as_ref().map(|(_, own)| Arc::clone(own)).unwrap_or_default()
    }

    /// Choose the file at `vpath`, open every folder it is in, and scroll to
    /// it.
    pub(crate) fn show(&mut self, vpath: &str) {
        let lower = vpath.to_ascii_lowercase();
        self.open_to(folder_of(&lower));
        self.folder = folder_of(&lower).to_string();
        self.selected = Some(lower);
        self.query.clear();
        self.reveal = true;
    }

    /// Open `folder` and every folder it is in.
    pub(crate) fn open_to(&mut self, folder: &str) {
        for (at, _) in folder.match_indices('\\') {
            self.open.insert(folder[..=at].to_string());
        }
        self.open_revision += 1;
    }

    /// Open `folder` in the tree when it is closed, and close it and every
    /// folder under it when it is open.
    pub(crate) fn toggle(&mut self, folder: &str) {
        match self.open.contains(folder) {
            true => self.open.retain(|open| !open.starts_with(folder)),
            false => {
                self.open.insert(folder.to_string());
            }
        }
        self.open_revision += 1;
    }
}

/// The folder part of an archive path, ending in `\`, or empty.
pub(crate) fn folder_of(vpath: &str) -> &str {
    match vpath.rfind('\\') {
        Some(at) => &vpath[..=at],
        None => "",
    }
}

/// The archives' own bytes of the file at `vpath`, read past the project.
pub(crate) fn shipped(game: &GameAssets, vpath: &str) -> Option<Vec<u8>> {
    game.read_past_overlay(vpath).ok()
}

/// A BLP texture's top mip as a PNG file: 8-bit RGBA.
pub(crate) fn png_of_blp(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let texture = vale_assets::world::blp::decode(bytes).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, texture.width, texture.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&texture.rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// The file an export of `vpath` writes: its name, and its bytes. With
/// `as_png`, a BLP is written as `<name>.png` ([`png_of_blp`]); any other file,
/// and a BLP without it, is written unchanged under its own name.
pub(crate) fn exported(vpath: &str, bytes: Vec<u8>, as_png: bool) -> Result<(String, Vec<u8>), String> {
    let name = vpath.rsplit('\\').next().unwrap_or(vpath);
    match as_png && is_texture(vpath) {
        true => {
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            Ok((format!("{stem}.png"), png_of_blp(&bytes)?))
        }
        false => Ok((name.to_string(), bytes)),
    }
}

/// Whether `vpath` is a BLP texture, which an export can write as a PNG.
pub(crate) fn is_texture(vpath: &str) -> bool {
    matches!(Kind::of(vpath), Kind::Texture | Kind::Minimap)
}

/// Where a folder export writes the file at `vpath` under `into`: its whole
/// archive path, folder by folder, with the name [`exported`] gives it.
pub(crate) fn export_path(into: &Path, vpath: &str, name: &str) -> PathBuf {
    let mut path = into.to_path_buf();
    for segment in folder_of(vpath).split('\\').filter(|segment| !segment.is_empty()) {
        path.push(segment);
    }
    path.push(name);
    path
}

/// Carry out what the Game Files part asked for, with the system's file
/// dialogs, and collect the folder exports that have finished. `--game-file`
/// opens the part on its path once.
pub(crate) fn answer(
    mut files: ResMut<GameFiles>,
    session: Option<ResMut<EditSession>>,
    game: Res<GameAssets>,
    args: Res<crate::args::Args>,
    mut seeded: Local<bool>,
    mut running: ResMut<crate::jobs::Running<Exported>>,
) {
    let Some(mut session) = session else { return };
    if let (Some(path), false) = (args.import.game_file.as_deref(), *seeded) {
        *seeded = true;
        let path = path.replace('/', "\\").to_ascii_lowercase();
        match path.ends_with('\\') {
            true => {
                files.open_to(&path);
                files.folder = path;
            }
            false => files.show(&path),
        }
    }
    for done in running.collect() {
        files.exporting = None;
        session.status = match done.failed.first() {
            None => format!("exported {} files of {} to {}", done.written, done.folder, done.into.display()),
            Some(first) => format!(
                "exported {} files of {} to {}; {} failed, first: {first}",
                done.written,
                done.folder,
                done.into.display(),
                done.failed.len()
            ),
        };
    }
    let Some(ask) = files.ask.take() else { return };
    match ask {
        Ask::Export { vpath, as_png, from_project } => {
            let read = match from_project {
                true => session.project.read(&vpath),
                false => shipped(&game, &vpath),
            };
            let Some(bytes) = read else {
                session.status = match from_project {
                    true => format!("{vpath} is not in the project"),
                    false => format!("{vpath} is not in the archives"),
                };
                return;
            };
            let (name, bytes) = match exported(&vpath, bytes, as_png) {
                Ok(file) => file,
                Err(why) => {
                    session.status = format!("{vpath}: {why}");
                    return;
                }
            };
            let mut dialog = rfd::FileDialog::new().set_file_name(&name);
            if let Some(dir) = &files.export_dir {
                dialog = dialog.set_directory(dir);
            }
            let Some(path) = dialog.save_file() else { return };
            files.export_dir = path.parent().map(Path::to_path_buf);
            session.status = match std::fs::write(&path, &bytes) {
                Ok(()) => format!("exported {vpath} to {}", path.display()),
                Err(why) => format!("{}: {why}", path.display()),
            };
        }
        Ask::ExportFolder { folder } => {
            if files.exporting.is_some() {
                session.status = "a folder export is already running".to_string();
                return;
            }
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = &files.export_dir {
                dialog = dialog.set_directory(dir);
            }
            let Some(into) = dialog.pick_folder() else { return };
            files.export_dir = Some(into.clone());
            // Written at the paths as the archives spell them.
            let spell = session.spellings(&game);
            let paths: Vec<String> = files
                .catalogue(&game)
                .iter()
                .filter(|path| path.starts_with(&folder))
                .map(|path| spell.of(path).to_string())
                .collect();
            let progress = crate::jobs::Progress::new(paths.len());
            let shown = match folder.is_empty() {
                true => "the archives".to_string(),
                false => spell.of(&folder).trim_end_matches('\\').to_string(),
            };
            let label = format!("exporting {} files of {shown}", paths.len());
            let worker = progress.clone();
            let as_png = files.as_png;
            let read = game.shared_reader_past_overlay();
            let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                let mut written = 0;
                let mut failed = Vec::new();
                for (index, vpath) in paths.iter().enumerate() {
                    let result = read(vpath)
                        .ok_or_else(|| "not in the archives".to_string())
                        .and_then(|bytes| exported(vpath, bytes, as_png))
                        .and_then(|(name, bytes)| {
                            let path = export_path(&into, vpath, &name);
                            if let Some(parent) = path.parent() {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&path, bytes).map_err(|e| e.to_string())
                        });
                    match result {
                        Ok(()) => written += 1,
                        Err(why) if failed.len() < FAILURES_KEPT => failed.push(format!("{vpath}: {why}")),
                        Err(_) => {}
                    }
                    worker.set(index + 1);
                }
                Exported { folder: shown, into, written, failed }
            });
            session.status = label.clone();
            files.exporting = Some((label.clone(), progress.clone()));
            running.jobs.push(crate::jobs::Job { label, progress, task });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BLP exported as a PNG decodes back to its pixels; any other file is
    /// written unchanged under its own name.
    #[test]
    fn a_texture_exports_as_a_png_and_other_files_unchanged() {
        use vale_assets::world::blp;
        let source: Vec<u8> = (0..16 * 16).flat_map(|i| [i as u8, 0, 255 - i as u8, 255]).collect();
        let texture = blp::encode(&source, 16, 16, blp::Encoding::Dxt3, true).expect("encoded");
        let rgba = blp::decode(&texture).expect("decoded").rgba;
        let (name, png) = exported("tileset\\elwynn\\grass.blp", texture.clone(), true).expect("a png");
        assert_eq!(name, "grass.png");
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().expect("a png header");
        assert_eq!((reader.info().width, reader.info().height), (16, 16));
        let mut pixels = vec![0; reader.output_buffer_size().expect("a size")];
        reader.next_frame(&mut pixels).expect("pixels");
        assert_eq!(pixels, rgba);
        let (name, same) = exported("tileset\\elwynn\\grass.blp", texture.clone(), false).expect("unchanged");
        assert_eq!((name.as_str(), same), ("grass.blp", texture));
        let (name, _) = exported("sound\\creature\\wolf\\howl.wav", vec![1, 2, 3], true).expect("unchanged");
        assert_eq!(name, "howl.wav");
    }

    /// A file shown opens every folder it is in; a folder closed closes the
    /// folders under it.
    #[test]
    fn a_shown_file_opens_its_folders_and_a_closed_folder_closes_its_own() {
        let mut files = GameFiles::default();
        files.show("Sound\\Creature\\Wolf\\Howl.wav");
        assert_eq!(files.selected.as_deref(), Some("sound\\creature\\wolf\\howl.wav"));
        assert_eq!(
            files.open.iter().map(String::as_str).collect::<Vec<_>>(),
            ["sound\\", "sound\\creature\\", "sound\\creature\\wolf\\"]
        );
        files.toggle("sound\\creature\\");
        assert_eq!(files.open.iter().map(String::as_str).collect::<Vec<_>>(), ["sound\\"]);
        files.toggle("world\\");
        assert!(files.open.contains("world\\"));
    }

    /// A folder export writes each file at its whole archive path.
    #[test]
    fn a_folder_export_keeps_the_archive_path() {
        let into = Path::new("out");
        assert_eq!(
            export_path(into, "sound\\creature\\wolf\\howl.wav", "howl.wav"),
            Path::new("out").join("sound").join("creature").join("wolf").join("howl.wav")
        );
        assert_eq!(export_path(into, "readme.txt", "readme.txt"), Path::new("out").join("readme.txt"));
        assert_eq!(folder_of("sound\\creature\\wolf\\howl.wav"), "sound\\creature\\wolf\\");
        assert_eq!(folder_of("readme.txt"), "");
    }
}
