//! The number a new record takes: a client table row's id, a spawn's guid, a
//! template's entry, a placement's `uniqueId`.
//!
//! An operation that makes a record works out `floor`, the number it would
//! use alone: one past the largest it can see, or the first free one under
//! its own rule. It asks a [`Numbers`] for the number to use, by type and
//! floor. Alone the answer is the floor ([`Alone`]). In a shared session the
//! editor answers from numbers the host handed it, so two editors never give
//! two new records one number; see `vale_collab::ids`.
//!
//! A type is named by the table and column the number goes in:
//! [`dbc`] for a client table's id, [`row`] for a server table's column, and
//! [`UNIQUE_ID`] for a placement.

/// Where new numbers come from. See the module comment.
pub trait Numbers {
    /// The number a new record of `kind` takes, given `floor`, or why no
    /// number can be given now. An operation that gets an error makes
    /// nothing and passes the reason on.
    fn take(&mut self, kind: &str, floor: u64) -> Result<u64, String>;
}

/// The numbers of an editor alone: each one is its floor.
#[derive(Debug, Default, Clone, Copy)]
pub struct Alone;

impl Numbers for Alone {
    fn take(&mut self, _kind: &str, floor: u64) -> Result<u64, String> {
        Ok(floor)
    }
}

/// The type of a client table's ids: `Spell.dbc`.
pub fn dbc(table: &str) -> String {
    format!("{table}.dbc")
}

/// The type of a server table column's numbers: `creature.guid`.
pub fn row(table: &str, column: &str) -> String {
    format!("{table}.{column}")
}

/// The type of a placement's `uniqueId`, which doodads and buildings share
/// across every map.
pub const UNIQUE_ID: &str = "uniqueId";

/// A 32-bit number from a source, or why there is none. A number past
/// `u32::MAX` is refused.
pub fn take_u32(numbers: &mut dyn Numbers, kind: &str, floor: u32) -> Result<u32, String> {
    let number = numbers.take(kind, u64::from(floor))?;
    u32::try_from(number).map_err(|_| format!("{kind} numbers have run past {}", u32::MAX))
}
