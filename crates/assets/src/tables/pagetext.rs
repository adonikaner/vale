//! **`PageTextMaterial.dbc`** — what a page of text is written *on*, which is
//! the one thing about the sign-and-book window that no packet states.
//!
//! Six rows, two columns, and the second is a name rather than a colour:
//!
//! ```text
//! 1 Parchment   2 Stone   3 Marble   4 Silver   5 Bronze   6 Valentine
//! ```
//!
//! The template's `pageMaterial` word is a row id here, and the *name* is what
//! the interface is written against: `ItemTextFrame_OnEvent` builds
//! `Interface\ItemTextFrame\ItemText-<material>-TopLeft` and its three corners
//! out of it, and hands the same string to the directory's own
//! `GetMaterialTextColors`, which is where the ink colour comes from. So this
//! table's whole job is a number to a word, and getting it wrong draws a
//! **missing** texture rather than a wrong one — which is why the names are
//! read out of the file rather than transcribed.
//!
//! **Parchment is the fallback and it is the file's own row 1**, not a
//! stand-in: `ItemTextFrame.lua` reads `if ( not material ) then material =
//! "Parchment"` for a page whose material is 0 or absent, and then hides the
//! four corner textures for exactly that value. So the plainest page is the one
//! with no material at all, and it is the commonest.

use std::collections::HashMap;

use super::dbc::Dbc;

/// The row id the interface treats as plain, and the name it falls back to.
pub const PARCHMENT: u32 = 1;

/// `PageTextMaterial.dbc`, as a row id to its name.
#[derive(Debug, Clone, Default)]
pub struct PageMaterials(HashMap<u32, String>);

impl PageMaterials {
    /// Parse, tolerating an absent or damaged file.
    ///
    /// **A missing table is every page drawn on parchment**, which is the
    /// interface's own behaviour for an unnamed material and therefore a
    /// degradation rather than a failure: the words are still readable and the
    /// four corner textures stay hidden.
    pub fn parse(bytes: &[u8]) -> PageMaterials {
        let mut names = HashMap::new();
        if let Ok(table) = Dbc::parse(bytes) {
            for record in 0..table.record_count {
                let (Some(id), Some(name)) = (table.u32_at(record, 0), table.string_at(record, 1))
                else {
                    continue;
                };
                if !name.is_empty() {
                    names.insert(id, name);
                }
            }
        }
        PageMaterials(names)
    }

    /// The name for a `pageMaterial` word, or `None` — which the panel reads as
    /// parchment. Zero is never a row and answers `None` without a lookup.
    pub fn name(&self, material: u32) -> Option<&str> {
        (material != 0).then(|| self.0.get(&material).map(String::as_str)).flatten()
    }

    /// How many rows came back, for `vale objects` to report.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// The file's own six, and the two answers that are not a lookup.
    #[test]
    fn a_material_word_names_a_row_and_zero_names_nothing() {
        // `\0Parchment\0Stone\0`
        let strings = b"\0Parchment\0Stone\0";
        let rows = [vec![1u32, 1], vec![2, 11]];
        let materials = PageMaterials::parse(&dbc(&rows, 2, strings));
        assert_eq!(materials.name(1), Some("Parchment"));
        assert_eq!(materials.name(2), Some("Stone"));
        assert_eq!(materials.name(0), None, "no material at all");
        assert_eq!(materials.name(99), None, "a row the file does not carry");
        assert_eq!(materials.len(), 2);
    }

    /// …and no file at all is every page on parchment, not a panic.
    #[test]
    fn a_missing_table_is_empty_rather_than_a_failure() {
        let materials = PageMaterials::parse(&[]);
        assert!(materials.is_empty());
        assert_eq!(materials.name(1), None);
    }
}
