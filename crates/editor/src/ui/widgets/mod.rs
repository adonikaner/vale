//! The theme and the pieces the editor's forms and lists are built from.
//!
//! ```text
//! theme.rs      the palette, the style, and the widgets built on them
//! rowform.rs    the widgets the server-row forms are built from: how a column
//!               of each type is drawn, what typing into one produces, one
//!               field with its revert arrow, the entry box and the remove or
//!               keep button. The form itself stays with its subject
//! reference.rs  a column that names a row of another table, as every
//!               server-row form draws it: the picker button, the name the
//!               number resolves to, and the link that opens the row
//! members.rs    a group selection's members as a folded list, in which one
//!               can be made the primary or taken out
//! thumbnails.rs the pictures the inspector draws in a list, decoded a few per
//!               frame
//! folders.rs    a catalogue of paths browsed as folders: the breadcrumb, the
//!               folder rows, and a path split into folder and file
//! preview.rs    the preview pane: one model's rendered picture
//! models.rs     a model list's row (picture, name and star) and the pager
//!               every paged list and dialog uses
//! ```

pub mod folders;
pub mod members;
pub mod models;
pub mod preview;
pub mod reference;
pub mod rowform;
pub mod theme;
pub mod thumbnails;
