//! Named places the camera can be sent back to.
//!
//! ## Getting somewhere was a position, a tile or a zone
//!
//! All three name a place on the map and none of them names *the* place — the
//! corner of the village being worked on, seen from the angle it was being
//! worked on from. Noggit keeps a named list; this is that list. A bookmark
//! is the camera whole: the map, the focus with its height, the yaw, the
//! pitch and the distance, so going back is going back to the same view.
//!
//! ## Where it is written, and that it is invented
//!
//! `Edit\bookmarks.txt`, beside the project folders and for the reason
//! [`crate::favourites`] gives: a project's files are published into a patch
//! archive, and a list of places is not a file the game reads. One bookmark
//! per line, tab-separated, the name first so a person can edit the file.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

/// The file's name under `Edit\`.
pub const FILE: &str = "bookmarks.txt";

/// One place, as the camera stood.
#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark {
    pub name: String,
    /// The map's `World\Maps\` directory name, which is what the session
    /// keys the map by.
    pub map: String,
    /// The focus, in the world's own axes, height included.
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

/// The list, and where it was read from.
#[derive(Resource, Default)]
pub struct Bookmarks {
    loaded_from: Option<PathBuf>,
    pub list: Vec<Bookmark>,
}

impl Bookmarks {
    /// Read the file under `dir` once. Idempotent per directory.
    pub fn load_from(&mut self, dir: &Path) {
        if self.loaded_from.as_deref() == Some(dir) {
            return;
        }
        self.loaded_from = Some(dir.to_path_buf());
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
        self.list = parse(&text);
    }

    /// Add one, replacing a bookmark of the same name.
    pub fn add(&mut self, bookmark: Bookmark) {
        self.list.retain(|had| had.name != bookmark.name);
        self.list.push(bookmark);
        self.save();
    }

    pub fn remove(&mut self, name: &str) {
        self.list.retain(|had| had.name != name);
        self.save();
    }

    fn save(&self) {
        let Some(dir) = &self.loaded_from else { return };
        if let Err(e) = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(dir.join(FILE), render(&self.list)))
        {
            warn!("{} could not be written: {e}", dir.join(FILE).display());
        }
    }
}

/// `name<TAB>map<TAB>x<TAB>y<TAB>z<TAB>yaw<TAB>pitch<TAB>distance`, one per
/// line. A line that does not parse is skipped rather than refusing the file.
fn parse(text: &str) -> Vec<Bookmark> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 8 {
            continue;
        }
        let number = |at: usize| {
            fields[at]
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|n| n.is_finite())
        };
        let (Some(x), Some(y), Some(z), Some(yaw), Some(pitch), Some(distance)) = (
            number(2),
            number(3),
            number(4),
            number(5),
            number(6),
            number(7),
        ) else {
            continue;
        };
        let name = fields[0].trim();
        let map = fields[1].trim();
        if name.is_empty() || map.is_empty() {
            continue;
        }
        out.push(Bookmark {
            name: name.to_string(),
            map: map.to_string(),
            target: [x, y, z],
            yaw,
            pitch,
            distance,
        });
    }
    out
}

fn render(list: &[Bookmark]) -> String {
    let mut out =
        String::from("# world editor camera bookmarks: name, map, x, y, z, yaw, pitch, distance\n");
    for b in list {
        out.push_str(&format!(
            "{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.4}\t{:.4}\t{:.3}\n",
            b.name.replace('\t', " "),
            b.map,
            b.target[0],
            b.target[1],
            b.target[2],
            b.yaw,
            b.pitch,
            b.distance
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str) -> Bookmark {
        Bookmark {
            name: name.into(),
            map: "Azeroth".into(),
            target: [-9450.5, -50.25, 61.0],
            yaw: 0.7,
            pitch: 0.3,
            distance: 42.0,
        }
    }

    #[test]
    fn the_file_round_trips() {
        let list = vec![one("Goldshire inn"), one("the well")];
        let back = parse(&render(&list));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "Goldshire inn");
        assert!((back[0].target[0] + 9450.5).abs() < 1e-3);
        assert!((back[1].distance - 42.0).abs() < 1e-3);
    }

    #[test]
    fn a_same_named_bookmark_replaces_and_a_bad_line_is_skipped() {
        let mut marks = Bookmarks::default();
        marks.add(one("here"));
        let mut again = one("here");
        again.distance = 9.0;
        marks.add(again);
        assert_eq!(marks.list.len(), 1);
        assert_eq!(marks.list[0].distance, 9.0);
        marks.remove("here");
        assert!(marks.list.is_empty());

        let parsed = parse("ok\tAzeroth\t1\t2\t3\t4\t5\t6\nshort\tAzeroth\t1\n\tAzeroth\t1\t2\t3\t4\t5\t6\nnan\tAzeroth\tx\t2\t3\t4\t5\t6\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "ok");
    }
}
