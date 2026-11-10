//! Claims: what one author holds of a tile, a client table or a server row,
//! which the host keeps and sends to every editor.
//!
//! An author holds a claim in two ways, and can hold one both ways:
//!
//! ```text
//! checked out   asked for with ToHost::CheckOut, before any edit
//! edited        an accepted edit of theirs that is in no changelist yet
//! ```
//!
//! While an author holds a claim, the host refuses another author's edit to
//! anything in it ([`crate::host`]). A check out is never needed to edit: an
//! edit claims what it touches. A check out is for work that will take a
//! while or be done offline: checked out, the subject cannot change
//! underneath the edits, so they arrive with nothing to resolve.
//!
//! A Submit releases both kinds. A release, or another author breaking the
//! claim, ends a check out only: what an author's unsubmitted edits hold stays
//! theirs until they submit, their edits are withdrawn because they change
//! nothing, or another editor takes them back. The host's editor keeps the
//! check outs in the log folder, so they outlast the session; the edited
//! claims are read back from the log's edits in no changelist.
//!
//! A [`Claim`] is coarser than a [`Group`]: a tile claims every group of the
//! tile, since a person works on a place, not on one part of one chunk. A
//! file is in no claim. A step that writes or removes a tile's file touches
//! every group of the tile as well, so the tile's claim covers it.

use crate::groups::Group;
use crate::log::Author;
use serde::{Deserialize, Serialize};
use vale_edit::TileKey;

/// What a check out holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Claim {
    Tile(TileKey),
    /// A client table, by the name its DBC file has (`Spell`).
    Table(String),
    /// A server row, by its table and key as [`Group::Row`] names them.
    Row { table: String, key: String },
}

impl Claim {
    /// The claim a group is in. A file is in none; see the module comment.
    pub fn of(group: &Group) -> Option<Claim> {
        match group {
            Group::Chunk { tile, .. } | Group::Textures { tile } | Group::Placements { tile } => Some(Claim::Tile(tile.clone())),
            Group::Table { name } => Some(Claim::Table(name.clone())),
            Group::Row { table, key } => Some(Claim::Row { table: table.clone(), key: key.clone() }),
            Group::File { .. } => None,
        }
    }

    /// The claim as a person reads it: `Azeroth 32_48`, `Spell.dbc`,
    /// `creature guid=7`.
    pub fn name(&self) -> String {
        match self {
            Claim::Tile(tile) => format!("{} {}_{}", tile.map, tile.x, tile.y),
            Claim::Table(name) => format!("{name}.dbc"),
            Claim::Row { table, key } => format!("{table} {key}"),
        }
    }

    /// One line of the check outs file: the claim and its author.
    pub fn line(&self, author: &Author) -> String {
        let claim = match self {
            Claim::Tile(tile) => format!("tile\t{}\t{}\t{}", tile.map, tile.x, tile.y),
            Claim::Table(name) => format!("table\t{name}"),
            Claim::Row { table, key } => format!("row\t{table}\t{key}"),
        };
        format!("{}\t{}\t{claim}", crate::log::hex(&author.id), author.name.replace(['\t', '\n', '\r'], " "))
    }

    /// A line of the check outs file read back, as [`Self::line`] writes it.
    /// `None` for a line that does not read.
    pub fn parse(line: &str) -> Option<(Claim, Author)> {
        let mut fields = line.split('\t');
        let id = crate::log::parse_hex(fields.next()?)?;
        let name = fields.next()?.to_string();
        let claim = match fields.next()? {
            "tile" => Claim::Tile(TileKey::new(fields.next()?, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?)),
            "table" => Claim::Table(fields.next()?.to_string()),
            "row" => Claim::Row { table: fields.next()?.to_string(), key: fields.next()?.to_string() },
            _ => return None,
        };
        Some((claim, Author { id, name }))
    }
}

/// One claim held: what, by whom, and in which of the two ways. See the
/// module comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    pub claim: Claim,
    pub author: Author,
    /// Checked out with [`crate::wire::ToHost::CheckOut`].
    pub checked_out: bool,
    /// Held by the author's edits in no changelist.
    pub edited: bool,
}

impl Hold {
    /// How it is held, for a reason or a list: `has Azeroth 32_48 checked
    /// out`, `has unsubmitted edits on Spell.dbc`.
    pub fn words(&self) -> String {
        match self.checked_out {
            true => format!("has {} checked out", self.claim.name()),
            false => format!("has unsubmitted edits on {}", self.claim.name()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groups::Part;

    /// Every group of a tile is in the tile's claim and a file is in none; a
    /// claim's line reads back.
    #[test]
    fn a_tile_claims_its_groups_and_its_file() {
        let tile = TileKey::new("Azeroth", 32, 48);
        let claim = Claim::Tile(tile.clone());
        assert_eq!(Claim::of(&Group::Chunk { tile: tile.clone(), chunk: 9, part: Part::Holes }), Some(claim.clone()));
        assert_eq!(Claim::of(&Group::file(&tile.vpath())), None);
        let sam = Author { id: [4; 16], name: "Sam".into() };
        for claim in [claim, Claim::Table("Spell".into()), Claim::Row { table: "creature".into(), key: "guid=7".into() }] {
            assert_eq!(Claim::parse(&claim.line(&sam)), Some((claim, sam.clone())));
        }
    }
}
