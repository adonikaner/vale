//! The views of `crate::import`.
//!
//! ```text
//! assets.rs     the Assets workspace: the project's own files, which were
//!               imported and which generated, what each is and what made
//!               it, and the Import Texture dialog any form can open
//! files.rs      the Game Files part of the Assets workspace: every file the
//!               archives list, by folder, previewed, exported and replaced
//! models.rs     the Import Model dialog
//! sounds.rs     the Import Sound dialog, and a sound's details, waveform and
//!               play button in the Assets workspace
//! ```

pub(crate) mod assets;
pub(crate) mod files;
pub(crate) mod models;
pub(crate) mod sounds;
