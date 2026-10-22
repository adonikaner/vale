//! The three rigs that drive the client's passes with no server. They depend
//! on one another: the lab uses the stage's camera and render layer, the
//! stage queries `lab::Mannequin`, and the portraits use `STAGE_LAYER`.
//!
//! ```text
//! stage/       the spell preview: two units and the values a cast changes, so
//!              the client's own passes draw it
//! lab.rs       the attachment lab: one effect model on a mannequin, moved by
//!              hand and baked into a copy of the model, because the tables
//!              carry no offset and the game stores it in the file
//! portraits.rs a picture of any model the archives name, one per slot per
//!              frame into a small image, for the pickers' rows
//! ```

pub mod lab;
pub mod portraits;
pub mod stage;
