//! What a publish writes for a server. A publish changes nothing in this
//! machine's install.
//!
//! ```text
//! patch.rs       the patch a publish makes: one folder holding the archive,
//!                the DBCs, the migration, the server tiles, a README saying
//!                where each goes and a manifest with a hash of each
//! release.rs     what a patch gives the server as rows: one migration in
//!                vmangos' own form holding all of the project's rows, and the
//!                DBCs the server reads as files
//! ```

pub(crate) mod patch;
pub(crate) mod release;
