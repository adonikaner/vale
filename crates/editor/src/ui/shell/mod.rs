//! The docked regions around the viewport, and the menus, forms and cards
//! they open.
//!
//! ```text
//! topbar.rs     the top bar: the Project menu and Save, the Server button,
//!               the Map menu, the workspace control and the playtest button
//! popover.rs    the five forms the top bar opens: where to go, which account
//!               to log in as, which project, where the server is, and what
//!               a publish writes
//! tilefiles.rs  the Server tile files dialog: the DataDir's tile files a
//!               decision is wanted on, and the Remove and Put back buttons
//! manifest.rs   the open project's files and what each one changes, drawn in
//!               the project dialog
//! sync.rs       what this project has changed on the server: one block per
//!               subject, the same four verbs for each. Drawn inside the top
//!               bar's Server… popover, which holds every server operation;
//!               the state and the operations are `crate::server::sync`
//! rail.rs       the subject list: what the editor edits now and what it will
//!               edit
//! viewbar.rs    every toggle that changes what the viewport shows
//! status.rs     the status line at the bottom
//! popout.rs     the pop-out window: a workspace in an OS window of its own,
//!               drawn with the same functions as the main window's middle
//! context.rs    the right-click menu over the viewport: each world tool's
//!               one-click operations on what was clicked
//! hovercard.rs  the card beside the pointer that names the creature or game
//!               object under it: a title and four or five attributes
//! servicemarks.rs the icons over a creature's head that say what it offers,
//!               drawn in the viewport under the creature tool
//! icons.rs      the toolbar icons compiled into the editor (none yet)
//! asks.rs       what a panel asks the shell to do after the frame is drawn:
//!               open a row in another workspace, switch tools, import a
//!               texture, make an area
//! ```

pub(crate) mod asks;
pub(crate) mod context;
pub(crate) mod hovercard;
pub(crate) mod icons;
pub(crate) mod manifest;
pub(crate) mod popout;
pub(crate) mod popover;
pub(crate) mod rail;
pub(crate) mod servicemarks;
pub(crate) mod status;
pub(crate) mod sync;
pub(crate) mod tilefiles;
pub(crate) mod topbar;
pub(crate) mod viewbar;
