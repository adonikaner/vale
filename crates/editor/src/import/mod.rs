//! Files brought into the project from outside the archives.
//!
//! ```text
//! images.rs      a tile's heights and texture blends as PNG files, exported
//!                and imported from the map window: the folder, the file
//!                names, the encoding, and the one undo entry an import is
//! assets.rs      the Assets workspace: the textures and models the project
//!                adds to the archives, and a picture turned into a BLP file
//!                the 1.12.1 client reads
//! ```

pub(crate) mod assets;
pub(crate) mod images;
