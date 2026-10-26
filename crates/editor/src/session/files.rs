//! Any other file of the project: its bytes in the overlay, written to or
//! removed from the folder, and the paths listed under a folder.

use vale_client::assets::GameAssets;
use vale_edit::project::Project;
use bevy::prelude::*;
use super::EditSession;
use super::originals::changes_nothing;

/// How the archives and the project spell paths and folders that the
/// listing and [`EditSession::listed`] hold in lower case: `DBFilesClient\`
/// for `dbfilesclient\`. A list shows a path through [`Self::of`], and looks
/// it up and compares it in lower case.
#[derive(Debug, Default)]
pub(crate) struct Spellings {
    /// The archives' spellings, from `vale_assets::archive::Assets::spellings`.
    archives: std::sync::Arc<std::collections::HashMap<String, String>>,
    /// The project's own files and their folders, as the project folder
    /// spells them, for a path the archives do not list.
    project: std::collections::HashMap<String, String>,
}

impl Spellings {
    /// The spelling of the lower-case path or folder `lower`, or `lower`
    /// itself for one neither spells.
    pub(crate) fn of<'a>(&'a self, lower: &'a str) -> &'a str {
        self.archives.get(lower).or_else(|| self.project.get(lower)).map_or(lower, String::as_str)
    }

    /// The spelled name of the folder or file at the end of `lower`, which
    /// is a folder when it ends in `\`: `Creature` for `creature\`, and
    /// `Wolf.m2` for `creature\wolf\wolf.m2`.
    pub(crate) fn name_of<'a>(&'a self, lower: &'a str) -> &'a str {
        let spelled = self.of(lower).trim_end_matches('\\');
        spelled.rsplit('\\').next().unwrap_or(spelled)
    }
}

impl EditSession {
    /// The spellings of every path the archives list and every file the
    /// project has. The archives' half is read once; the project's again
    /// when [`Self::files_revision`] moves.
    pub(crate) fn spellings(&self, assets: &GameAssets) -> std::sync::Arc<Spellings> {
        let mut held = self.spelled.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, spellings)) = held.as_ref() {
            if *at == self.files_revision {
                return std::sync::Arc::clone(spellings);
            }
        }
        let archives = match held.as_ref() {
            Some((_, known)) => std::sync::Arc::clone(&known.archives),
            None => std::sync::Arc::new(assets.with_archive(|chain| Ok(chain.spellings())).unwrap_or_default()),
        };
        let own = self.project.files();
        let project = vale_assets::archive::spellings_of(own.iter().map(String::as_str));
        let spellings = std::sync::Arc::new(Spellings { archives, project });
        *held = Some((self.files_revision, std::sync::Arc::clone(&spellings)));
        spellings
    }

    /// Put any file's bytes into the overlay under a virtual path, so the
    /// next read of that path answers with them — a model just baked, before
    /// or instead of writing it anywhere.
    ///
    /// The file counterpart of [`Self::publish_table`], for a path that is
    /// neither a tile nor a table. Nothing on disk changes; see
    /// [`Self::save_bytes`].
    pub(crate) fn publish_bytes(&mut self, vpath: &str, bytes: Vec<u8>) {
        self.publish_scratch(vpath, bytes);
        self.republished.push(vpath.to_string());
    }

    /// The same, for a path minted fresh on every publish — the lab's
    /// scratch copy, which is `custom\lab\<n>.m2` for a new `n` each time.
    ///
    /// This and [`Self::unpublish_scratch`] differ from [`Self::publish_bytes`]
    /// only in recording nothing in [`Self::republished`]. There is nothing to
    /// forget: no cache can be holding a reading of a path that did not exist
    /// a moment ago.
    ///
    /// Recording would also cause a loop. The lab publishes its copy and
    /// hangs it in one frame, and compares what it has hanging against
    /// [`Self::republished_revision`]; a scratch publish that bumped that
    /// counter would make the next frame's compare differ, which re-hangs,
    /// which publishes again, on every frame the lab is open.
    pub(crate) fn publish_scratch(&mut self, vpath: &str, bytes: Vec<u8>) {
        if let Ok(mut edited) = self.edited.write() {
            edited.insert(vpath.to_ascii_lowercase(), bytes);
        }
    }

    pub(crate) fn unpublish_scratch(&mut self, vpath: &str) {
        if let Ok(mut edited) = self.edited.write() {
            edited.remove(&vpath.to_ascii_lowercase());
        }
    }

    /// Write any file's bytes into the project folder under a virtual path,
    /// and publish them. `true` on success; the status line says otherwise.
    ///
    /// Bytes that are the archives' own file are not kept: a copy saved
    /// earlier is removed instead, as [`Self::save`] does for a tile, so the
    /// project lists no edit that changes nothing.
    pub(crate) fn save_bytes(&mut self, vpath: &str, bytes: Vec<u8>) -> bool {
        self.files_revision += 1;
        if self.changes_nothing(vpath, &bytes) {
            return match self.project.revert(vpath) {
                Ok(_) => {
                    self.unpublish_scratch(vpath);
                    // A copy saved before is gone, and whatever read it
                    // reads the archives' file again.
                    self.republished.push(vpath.to_string());
                    self.status = format!("{vpath} matches the archives, so it is not kept in the project");
                    true
                }
                Err(e) => {
                    self.status = format!("{vpath}: {e}");
                    false
                }
            };
        }
        match self.project.write(vpath, &bytes) {
            Ok(at) => {
                self.publish_bytes(vpath, bytes);
                self.status = format!("saved {}", at.display());
                true
            }
            Err(e) => {
                self.status = format!("{vpath}: {e}");
                false
            }
        }
    }

    /// Remove one of the project's own files, and take it out of the overlay:
    /// the archives' copy, or nothing, answers for the path again.
    pub(crate) fn remove_file(&mut self, vpath: &str) -> bool {
        match self.project.revert(vpath) {
            Ok(_) => {
                self.unpublish_scratch(vpath);
                self.republished.push(vpath.to_ascii_lowercase());
                self.files_revision += 1;
                self.status = format!("{vpath} removed from the project");
                true
            }
            Err(e) => {
                self.status = format!("{vpath}: {e}");
                false
            }
        }
    }

    /// Every path under `prefix` that a read through the overlay finds: the
    /// archives' listing and the project's own game files, lower case, sorted
    /// and without repeats.
    ///
    /// The archives' listing is built once, from the archives that were
    /// mounted when the chain opened, and holds nothing the project carries,
    /// so a file saved into the project is read through the overlay but is in
    /// no picker built from the listing alone. This is the listing with the
    /// project's files added, and every picker over the archives' files is
    /// built from it. Its cost is a walk of the project folder, so a caller
    /// keeps what it builds and builds it again when [`Self::files_revision`]
    /// moves.
    pub(crate) fn listed(&self, assets: &GameAssets, prefix: &str) -> Vec<String> {
        let prefix = prefix.to_ascii_lowercase();
        let mut all: Vec<String> = assets
            .with_archive(|chain| Ok(chain.list_prefix(&prefix)))
            .unwrap_or_default()
            .into_iter()
            .map(|path| path.to_ascii_lowercase())
            .collect();
        all.extend(self.own_files_under(&prefix));
        all.sort();
        all.dedup();
        all
    }

    /// The project's own game files under `prefix`, lower case. See
    /// [`Self::listed`].
    pub(crate) fn own_files_under(&self, prefix: &str) -> Vec<String> {
        let prefix = prefix.to_ascii_lowercase();
        self.project
            .files()
            .into_iter()
            .filter(|vpath| Project::is_game_data(vpath))
            .map(|vpath| vpath.to_ascii_lowercase())
            .filter(|vpath| vpath.starts_with(&prefix))
            .collect()
    }

    /// Whether a file's bytes change nothing the archives' copy has: a table
    /// with the same rows, any other file with the same bytes. False when
    /// the archives have no such file or there are no archives to ask. See
    /// [`changes_nothing`].
    pub(super) fn changes_nothing(&self, vpath: &str, bytes: &[u8]) -> bool {
        self.shipped
            .as_ref()
            .and_then(|shipped| shipped.original(&self.project, vpath))
            .is_some_and(|original| changes_nothing(vpath, bytes, &original))
    }
}
