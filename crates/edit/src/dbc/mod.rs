//! One client table, taken apart and put back together byte for byte.
//!
//! ```text
//! file.rs     the container: the header, the records, the string block, the
//!             writer, and the append-only rule that keeps every offset in the
//!             file valid across a string edit
//! cell.rs     what an edit to one of its fields is, for the undo stack
//! row.rs      what adding or removing a whole record is, on the same terms
//! chain.rs    copying a row, and copying a row with everything it points at:
//!             the deep clone a spell's visual chain needs before it can be
//!             changed without changing every spell that shares it
//! taxi.rs     flight paths: adding and removing a node, a path or a point of
//!             a path across the three taxi tables, keeping each path's points
//!             numbered without a gap
//! diff.rs     what one table changes against another, row by row, by id
//! ```
//!
//! ## Why a container and not a model
//!
//! `vale_assets::tables::dbc::Dbc` reads a table: it slices the record block
//! and the string block and answers questions about fields by index. That is all
//! a reader needs, and every rule in `assets::tables` is built on it.
//!
//! An editor also needs writing the file back to produce the file it read. A
//! DBC states four numbers in its header and then two blocks of bytes, and a
//! writer that rebuilt either block from a parsed model would have to
//! understand every column of every table to do it. [`file::DbcFile`] carries
//! both blocks exactly as the file wrote them, edits bytes in place, and
//! recomputes only the header words an edit changed. [`tests`]' round trip over
//! every DBC in the archives checks that this holds.
//!
//! ## The layout, as measured
//!
//! Over the 158 entries `DBFilesClient\` answers with in a 1.12.1 install:
//!
//! * Four of them are zero bytes: `CharacterCreateCameras`,
//!   `SoundCharacterMacroLines`, `SpellAuraNames` and `SpellEffectNames`. The
//!   client asks for these names and gets nothing, so a table with no header is
//!   refused rather than read as an empty one, and a caller opening one for
//!   editing is told. The last two matter for a spell editor: the game ships no
//!   names for its own effect and aura numbers.
//! * The remaining 154 write back byte for byte.
//! * The header is 20 bytes: `WDBC`, then record count, field count, record
//!   size and string-block size as `u32`.
//! * `fieldCount` counts fields, not dwords, and in two tables the two differ.
//!   `CharBaseInfo.dbc` is 41 records of two bytes, two fields wide: a race and
//!   a class, one byte each. `CharStartOutfit.dbc` is 41 fields in 152 bytes:
//!   four byte-wide fields and then 37 dwords. Every other table is
//!   `fieldCount * 4`. Nothing here assumes the two agree, and a four-byte read
//!   that does not fit inside a record answers `None` rather than reading into
//!   the next one.
//! * The string block opens with a NUL in all 154, so offset 0 reads as the
//!   empty string, and every table uses it that way for a column with no text.
//! * Nothing follows the string block in a shipped table. A file that carries
//!   anything there keeps it: [`file::DbcFile`] writes the tail back.
//!
//! ## A string edit appends and never moves
//!
//! A string cannot be changed in place, because the new text is a different
//! length, and rewriting the block moves every offset after the edit in a file
//! where an offset is an ordinary `u32` field indistinguishable from an id.
//! `MCAL` has the same problem one container along. Here the block is only
//! ever appended to: an edited string is written at the end and the field
//! repointed at it, so no offset the file already holds changes, and no column
//! this crate does not know about can be broken by an edit to one it does.
//!
//! The cost is the old text left in the block, unread. A table edited a hundred
//! times carries a few kilobytes of it, and the same text written twice is found
//! and shared rather than appended again; see [`file::DbcFile::set_string`].

pub mod cell;
pub mod chain;
pub mod diff;
pub mod file;
pub mod row;
pub mod taxi;

pub use cell::Cell;
pub use file::DbcFile;
pub use row::Row;

#[cfg(test)]
mod tests;
