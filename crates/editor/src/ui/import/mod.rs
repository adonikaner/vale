//! The views of `crate::import`.
//!
//! ```text
//! assets.rs     the Assets workspace: the project's own files, which were
//!               imported and which generated, what each is and what made
//!               it, and the Import Texture dialog any form can open
//! files.rs      the Game Files part of the Assets workspace: every file the
//!               archives list, by folder, previewed, exported and replaced
//! models.rs     the Import Model dialog
//! picked.rs     a click on a file's row read as a choice of several files,
//!               and the right-click menu over the chosen files
//! sounds.rs     the Import Sound dialog, and a sound's details, waveform and
//!               play button in the Assets workspace
//! worldmap/     the World Map part of the Zones workspace: the maps, the
//!               chosen map's picture with its sectors and explored areas, a
//!               new picture or new art beside it, the map's settings, and
//!               the New map form
//! ```

pub(crate) mod assets;
pub(crate) mod files;
pub(crate) mod models;
pub(crate) mod picked;
pub(crate) mod sounds;
pub(crate) mod worldmap;
