//! This machine's own choices, kept in files of this crate's own under
//! Edit\ and shared by every project.
//!
//! ```text
//! favourites.rs the models a person starred and the ones they placed last,
//!              per picker
//! bookmarks.rs named camera positions to return to
//! ```

pub(crate) mod bookmarks;
pub(crate) mod favourites;
