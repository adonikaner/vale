//! Files brought into the project from outside the archives, and the files
//! the editor generates beside them.
//!
//! ```text
//! images.rs      a tile's heights and texture blends as PNG files, exported
//!                and imported from the map window: the folder, the file
//!                names, the encoding, and the one undo entry an import is
//! assets.rs      the Assets workspace: the files the project adds to the
//!                archives, and a picture turned into a BLP file the 1.12.1
//!                client reads
//! models.rs      a glTF model turned into an M2 with its skeleton and
//!                animations, and its pictures into BLP files beside it
//! sounds.rs      WAV and MP3 files written at archive paths the client
//!                reads, and the SoundEntries row that plays them
//! sources.rs     the record of where each imported file came from, which
//!                tells an import from a generated file and is what
//!                Re-import reads
//! generated.rs   which of the editor's generators made a file, what from,
//!                and whether it is out of date or unused
//! ```

pub(crate) mod assets;
pub(crate) mod generated;
pub(crate) mod images;
pub(crate) mod models;
pub(crate) mod sounds;
pub(crate) mod sources;
