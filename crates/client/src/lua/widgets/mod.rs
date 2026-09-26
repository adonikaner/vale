//! **The object model itself: what a widget *is*, where it lands, and how it is
//! drawn.**
//!
//! Nothing here knows what the game is about. A `StatusBar` fills; a `Frame`
//! has a rectangle; a `Backdrop` is eight border pieces and a tile — and the
//! panels one directory over are what say that one particular status bar is a
//! health bar.
//!
//! ```text
//! widget.rs      the state every UI object has, and the methods they all share
//! layout.rs      …and where that puts it: the anchor graph, solved
//! frames.rs      the frame kinds, their scripts, and where an error came from
//! regions.rs     …and the two things a frame draws: a texture and a font string
//! button.rs      …and the one that answers a press
//! statusbar.rs   …the one that fills, and the texture it crops rather than squashes
//! slider.rs      …and the one that is dragged along a track
//! scrollframe.rs …and the one whose contents are larger than it is
//! editbox.rs     …and the one that takes the keyboard
//! keyboard.rs   …and the frames that take a *key* rather than a character
//! messages.rs    …and the one that holds lines until they age out
//! tooltip.rs     …and the plate, which is the one widget the world fills in
//! model.rs       …and the one whose contents are a 3D scene rather than a quad
//! minimap.rs     …and the one whose contents are the world
//! backdrop.rs    the art behind a frame: an edge strip and a tile
//! text.rs        the four escape sequences any string can carry, and its width
//! draw.rs        …and what all of it comes to: what is visible, in what order
//! ```
pub mod backdrop;
pub mod button;
pub mod draw;
pub mod editbox;
pub mod frames;
pub mod keyboard;
pub mod layout;
pub mod messages;
pub mod minimap;
pub mod model;
pub mod regions;
pub mod scrollframe;
pub mod slider;
pub mod statusbar;
pub mod text;
pub mod tooltip;
pub mod widget;
