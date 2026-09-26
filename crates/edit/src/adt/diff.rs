//! What one tile changes against another, counted by kind, for a list that
//! has to say what a project did to a tile without drawing it.
//!
//! The comparison is by region bytes and by placement record, not by
//! meaning. Two chunks whose `MCVT` bytes differ are "heights changed"
//! whether one vertex moved or all 145 did; a doodad whose record differs at
//! the same unique id is "moved" whether its position, rotation, scale or
//! flags changed. The count is exact for what it counts, and it costs one
//! pass over the bytes of each tile.

use super::{AdtFile, Region};
use std::collections::BTreeMap;

/// How an edited tile differs from the one it was edited from.
///
/// The chunk counts are chunks out of 256 whose region differs. The
/// placement counts are records: a unique id in one list and not the other
/// is added or removed, and an id in both with different bytes is moved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TileDiff {
    /// Chunks whose `MCVT` differs.
    pub heights: usize,
    /// Chunks whose `MCLY` or `MCAL` differs, plus every chunk when `MTEX`
    /// differs, since a texture list edit changes what every layer index
    /// names.
    pub textures: usize,
    /// Chunks whose `MCSH` differs.
    pub shadows: usize,
    /// Chunks whose `MCLQ` differs.
    pub liquid: usize,
    /// Chunks whose `MCCV` differs, counting a region present on one side
    /// only.
    pub colours: usize,
    /// Chunks whose header holes word differs.
    pub holes: usize,
    /// Chunks whose header area id differs.
    pub areas: usize,
    /// Chunks whose header flags differ, which is the impassable bit and the
    /// `MCCV` flag.
    pub flags: usize,
    pub doodads_added: usize,
    pub doodads_removed: usize,
    pub doodads_moved: usize,
    pub buildings_added: usize,
    pub buildings_removed: usize,
    pub buildings_moved: usize,
    /// Whether the two tiles have a different number of map chunks, which a
    /// shipped tile never does. The chunk counts above cover the chunks both
    /// have.
    pub chunk_count_differs: bool,
}

impl TileDiff {
    pub fn is_empty(&self) -> bool {
        *self == TileDiff::default()
    }

    /// The counts as one line: `heights in 14 chunks, textures in 3, 5
    /// doodads added, 1 removed`. `no difference` when nothing differs.
    pub fn line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut chunks = |count: usize, what: &str| {
            if count > 0 {
                parts.push(match parts.is_empty() {
                    true => format!("{what} in {count} chunk{}", plural(count)),
                    false => format!("{what} in {count}"),
                });
            }
        };
        chunks(self.heights, "heights");
        chunks(self.textures, "textures");
        chunks(self.shadows, "shadows");
        chunks(self.liquid, "water");
        chunks(self.colours, "vertex colours");
        chunks(self.holes, "holes");
        chunks(self.areas, "area ids");
        chunks(self.flags, "chunk flags");
        placements(
            &mut parts,
            "doodad",
            self.doodads_added,
            self.doodads_removed,
            self.doodads_moved,
        );
        placements(
            &mut parts,
            "building",
            self.buildings_added,
            self.buildings_removed,
            self.buildings_moved,
        );
        if self.chunk_count_differs {
            parts.push("a different chunk count".to_string());
        }
        match parts.is_empty() {
            true => "no difference".to_string(),
            false => parts.join(", "),
        }
    }
}

/// `5 doodads added, 1 removed, 2 moved`, omitting the zero counts and naming
/// the kind once.
fn placements(parts: &mut Vec<String>, kind: &str, added: usize, removed: usize, moved: usize) {
    let mut named = false;
    for (count, verb) in [(added, "added"), (removed, "removed"), (moved, "moved")] {
        if count == 0 {
            continue;
        }
        parts.push(match named {
            false => format!("{count} {kind}{} {verb}", plural(count)),
            true => format!("{count} {verb}"),
        });
        named = true;
    }
}

fn plural(count: usize) -> &'static str {
    match count {
        1 => "",
        _ => "s",
    }
}

/// Compare `edited` against `original`.
pub fn diff(edited: &AdtFile, original: &AdtFile) -> TileDiff {
    let mut out = TileDiff {
        chunk_count_differs: edited.chunks.len() != original.chunks.len(),
        ..TileDiff::default()
    };
    let textures_renamed = edited.textures != original.textures;
    for (a, b) in edited.chunks.iter().zip(original.chunks.iter()) {
        let differs = |region: Region| {
            a.region(region).map(|s| &s.data) != b.region(region).map(|s| &s.data)
        };
        if differs(Region::Heights) {
            out.heights += 1;
        }
        if textures_renamed || differs(Region::Layers) || differs(Region::Alpha) {
            out.textures += 1;
        }
        if differs(Region::Shadow) {
            out.shadows += 1;
        }
        if differs(Region::Liquid) {
            out.liquid += 1;
        }
        if differs(Region::Colours) {
            out.colours += 1;
        }
        let (ha, hb) = (a.head(), b.head());
        if ha.holes() != hb.holes() {
            out.holes += 1;
        }
        if ha.area_id() != hb.area_id() {
            out.areas += 1;
        }
        if ha.flags() != hb.flags() {
            out.flags += 1;
        }
    }

    let (added, removed, moved) = records(
        edited.doodad_list().into_iter().map(|d| (d.unique_id, d)),
        original.doodad_list().into_iter().map(|d| (d.unique_id, d)),
    );
    out.doodads_added = added;
    out.doodads_removed = removed;
    out.doodads_moved = moved;
    let (added, removed, moved) = records(
        edited.building_list().into_iter().map(|b| (b.unique_id, b)),
        original.building_list().into_iter().map(|b| (b.unique_id, b)),
    );
    out.buildings_added = added;
    out.buildings_removed = removed;
    out.buildings_moved = moved;
    out
}

/// `(added, removed, moved)` between two lists of records keyed by unique id.
///
/// A moved record is one present in both with different fields. The name id
/// is an index into the tile's own name list, so a record whose only change
/// is that index after a list edit counts as moved; the count says "differs",
/// which is true.
fn records<T: PartialEq>(
    edited: impl Iterator<Item = (u32, T)>,
    original: impl Iterator<Item = (u32, T)>,
) -> (usize, usize, usize) {
    let before: BTreeMap<u32, T> = original.collect();
    let after: BTreeMap<u32, T> = edited.collect();
    let added = after.keys().filter(|id| !before.contains_key(id)).count();
    let removed = before.keys().filter(|id| !after.contains_key(id)).count();
    let moved = after
        .iter()
        .filter(|(id, record)| before.get(id).is_some_and(|was| was != *record))
        .count();
    (added, removed, moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::{blank_tile, heights, holes, area, Doodad};

    fn tile() -> AdtFile {
        blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 100.0, 12)
    }

    #[test]
    fn an_unchanged_tile_has_no_difference() {
        let d = diff(&tile(), &tile());
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(d.line(), "no difference");
    }

    #[test]
    fn each_kind_of_edit_is_counted_where_it_lands() {
        let original = tile();
        let mut edited = tile();
        // Two chunks raised, one holed, one re-zoned.
        for index in [0, 1] {
            let chunk = edited.chunk_mut(index).unwrap();
            let mut raised = heights::heights(chunk);
            raised[0] += 1.0;
            heights::set_heights(chunk, &raised);
        }
        holes::set_holes(edited.chunk_mut(5).unwrap(), 0x0001);
        area::set_area(edited.chunk_mut(9).unwrap(), 40);
        // One doodad placed.
        let name = edited.name_model(r"World\Tree.m2");
        edited.add_doodad(
            Doodad {
                name_id: name,
                unique_id: 0,
                position: [100.0, 100.0, 100.0],
                rotation: [0.0; 3],
                scale: 1024,
                flags: 0,
            },
            5.0,
        );

        let d = diff(&edited, &original);
        assert_eq!(d.heights, 2);
        assert_eq!(d.holes, 1);
        assert_eq!(d.areas, 1);
        assert_eq!(d.doodads_added, 1);
        assert_eq!((d.doodads_removed, d.doodads_moved), (0, 0));
        assert_eq!(
            d.line(),
            "heights in 2 chunks, holes in 1, area ids in 1, 1 doodad added"
        );

        // The other way round is the same edit seen from the original: a
        // doodad the original lacks is one the edit removed.
        let back = diff(&original, &edited);
        assert_eq!(back.doodads_removed, 1);
        assert_eq!(back.heights, 2);
    }

    #[test]
    fn a_placement_with_the_same_id_and_different_fields_is_moved() {
        let mut original = tile();
        let name = original.name_model(r"World\Tree.m2");
        let placed = Doodad {
            name_id: name,
            unique_id: 0,
            position: [100.0, 100.0, 100.0],
            rotation: [0.0; 3],
            scale: 1024,
            flags: 0,
        };
        let at = original.add_doodad(placed, 5.0);
        let mut edited = original.clone();
        let mut list = edited.doodad_list();
        list[at].position[2] += 3.0;
        edited.set_doodad_list(&list);

        let d = diff(&edited, &original);
        assert_eq!((d.doodads_added, d.doodads_removed, d.doodads_moved), (0, 0, 1));
        assert_eq!(d.line(), "1 doodad moved");
    }
}
