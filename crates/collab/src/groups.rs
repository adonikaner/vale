//! What two edits conflict over.
//!
//! A [`Group`] is the smallest thing two editors' edits can be checked
//! against: two edits conflict when they share one. A group is as small as
//! the edit's own unit allows and no smaller, because an edit records whole
//! states, and applying one editor's state over another's loses whatever the
//! second changed inside it.
//!
//! * A chunk's heights and normals are one group, its colours, holes and area
//!   one each. Its flags and its liquid are one group, since the flags word
//!   carries the liquid bits and a liquid edit records the word.
//! * A chunk's paint records the tile's whole texture list (`MTEX`), which
//!   every chunk's layers index into, so paint is a group per tile.
//! * A doodad or building edit names an index into lists a placement edit
//!   renumbers, so every placement edit is a group per tile. A name edit
//!   changes both the texture list and the model lists.
//! * A client table edit is a group per table: a record added or removed
//!   moves the index every cell edit after it is addressed by.
//! * A server row edit is a group per row, whichever column it sets.
//! * A project file written or removed whole is a group per path. A tile's
//!   file also touches every group of the tile ([`whole_tile`]), and a
//!   table's file the table's group, since the file replaces all of them.

use serde::{Deserialize, Serialize};
use vale_edit::ops::Edit;
use vale_edit::undo::{Change, Entry};
use vale_edit::TileKey;

/// One thing two edits can conflict over. See the module comment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Group {
    Chunk { tile: TileKey, chunk: u16, part: Part },
    Textures { tile: TileKey },
    Placements { tile: TileKey },
    Table { name: String },
    Row { table: String, key: String },
    /// A project file, by its virtual path in lower case with `\` between
    /// its parts.
    File { path: String },
}

impl Group {
    /// The group of a project file at `vpath`, however it is spelled.
    pub fn file(vpath: &str) -> Group {
        Group::File { path: vpath.replace('/', "\\").to_ascii_lowercase() }
    }
}

/// Every group of a tile: what a step that writes or removes the tile's whole
/// file touches. 256 chunks of five parts each, the texture list and the
/// placement lists.
pub fn whole_tile(tile: &TileKey) -> Vec<Group> {
    let parts = [Part::Shape, Part::Colours, Part::Holes, Part::Area, Part::Flags];
    let mut out: Vec<Group> = (0..256u16)
        .flat_map(|chunk| parts.map(|part| Group::Chunk { tile: tile.clone(), chunk, part }))
        .collect();
    out.push(Group::Textures { tile: tile.clone() });
    out.push(Group::Placements { tile: tile.clone() });
    out.sort();
    out
}

/// Which part of a chunk a [`Group::Chunk`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Part {
    /// Heights and normals.
    Shape,
    Colours,
    Holes,
    Area,
    /// The flags word and the liquid.
    Flags,
}

/// What a session carries: anything that can say which groups it touches.
pub trait Payload: Clone + std::fmt::Debug + Serialize + serde::de::DeserializeOwned + Send + 'static {
    /// Every group this edit touches, sorted, each once.
    fn groups(&self) -> Vec<Group>;

    /// Whether this edit, made by the host's own editor, is accepted whatever
    /// was accepted before it: for an edit that cannot be taken back, such as
    /// a file deleted with unsaved edits dropped. See [`crate::peer`]'s
    /// *Forced edits*. Ignored for an edit from any other editor.
    fn forced(&self) -> bool {
        false
    }

    /// Whether the host accepts this edit from a joined editor, or why not.
    /// Not asked of the host's own edits.
    fn check(&self) -> Result<(), String> {
        Ok(())
    }
}

impl Payload for Change {
    fn groups(&self) -> Vec<Group> {
        let mut out = Vec::new();
        for entry in self.in_order(true) {
            match entry {
                Entry::Tile(tile, edit) => tile_groups(tile, edit, &mut out),
                Entry::Cell(table, _) | Entry::Row(table, _) => out.push(Group::Table { name: table.clone() }),
                Entry::Server(server) => {
                    out.push(Group::Row { table: server.table.clone(), key: server.key.clone() });
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

fn tile_groups(tile: &TileKey, edit: &Edit, out: &mut Vec<Group>) {
    let chunk = |chunk: &usize, part: Part| Group::Chunk { tile: tile.clone(), chunk: *chunk as u16, part };
    match edit {
        Edit::Heights { chunk: at, .. } | Edit::Normals { chunk: at, .. } => out.push(chunk(at, Part::Shape)),
        Edit::Colours { chunk: at, .. } => out.push(chunk(at, Part::Colours)),
        Edit::Holes { chunk: at, .. } => out.push(chunk(at, Part::Holes)),
        Edit::Area { chunk: at, .. } => out.push(chunk(at, Part::Area)),
        Edit::Flags { chunk: at, .. } | Edit::Liquid { chunk: at, .. } => out.push(chunk(at, Part::Flags)),
        Edit::Paint { .. } => out.push(Group::Textures { tile: tile.clone() }),
        Edit::Doodad { .. } | Edit::Building { .. } | Edit::Placements { .. } => {
            out.push(Group::Placements { tile: tile.clone() })
        }
        Edit::Names { .. } => {
            out.push(Group::Textures { tile: tile.clone() });
            out.push(Group::Placements { tile: tile.clone() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_edit::undo::History;

    /// A stroke over two chunks of one tile touches each chunk's shape once,
    /// and a server row edit touches its row, whichever column it sets.
    #[test]
    fn a_change_names_each_group_it_touches_once() {
        let tile = TileKey::new("Azeroth", 32, 48);
        let mut history = History::new();
        history.begin("Raise terrain");
        for at in [3, 4, 3] {
            history.record(
                &tile,
                [
                    Edit::Heights { chunk: at, before: vec![0.0], after: vec![1.0] },
                    Edit::Normals { chunk: at, before: vec![0], after: vec![1] },
                ],
            );
        }
        history.record_server(vale_edit::undo::ServerEntry::column("creature", "guid=7", "position_x", None, Some("1".into())));
        history.end();
        let change = history.undo().unwrap();
        let shape = |chunk| Group::Chunk { tile: tile.clone(), chunk, part: Part::Shape };
        assert_eq!(
            change.groups(),
            vec![shape(3), shape(4), Group::Row { table: "creature".into(), key: "guid=7".into() }]
        );

        // The same change after the wire's encoding.
        let mut bytes = Vec::new();
        crate::wire::write_frame(&mut bytes, &change).unwrap();
        let read: Change = crate::wire::read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!((read.label.as_str(), read.groups()), (change.label.as_str(), change.groups()));
        assert_eq!(read.server_steps(true).collect::<Vec<_>>(), change.server_steps(true).collect::<Vec<_>>());
    }

    /// A tile's whole file touches every group an edit to the tile can, and
    /// a file's group does not depend on how its path is spelled.
    #[test]
    fn a_whole_tile_covers_every_group_of_the_tile() {
        let tile = TileKey::new("Azeroth", 32, 48);
        let all = whole_tile(&tile);
        assert_eq!(all.len(), 256 * 5 + 2);
        assert!(all.windows(2).all(|pair| pair[0] < pair[1]), "sorted, each once");
        assert!(all.contains(&Group::Chunk { tile: tile.clone(), chunk: 255, part: Part::Flags }));
        assert!(all.contains(&Group::Placements { tile }));
        assert_eq!(Group::file("Textures/Ground/A.blp"), Group::file(r"textures\ground\a.BLP"));
    }
}
