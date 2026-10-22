//! Runs that measure the editor rather than edit anything.
//!
//! ```text
//! shot.rs      a picture of the whole editor, for checking a layout
//! tour.rs      the camera moved from place to place, with a line per stop of
//!              what the process holds, for finding a leak
//! heap.rs      the rust heap by size class, read off the allocator itself,
//!              for when every count on the tour line is flat
//! ```

pub mod heap;
pub mod shot;
pub mod tour;
