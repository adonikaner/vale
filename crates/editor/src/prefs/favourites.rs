//! The models a person keeps coming back to: the ones they starred, and the
//! ones they placed last.
//!
//! ## Scattering a copse is the same three models over and over
//!
//! Every placement was a search: type the name, find the row, click. The
//! second tree of a copse is the first tree again, and the fourth is one of
//! the first three. So the picker keeps two short lists beside its folders —
//! what was **starred**, which a person chooses, and what was **used**, which
//! the tool notes — and either is one click.
//!
//! ## Per kind, in one file
//!
//! A doodad, a building and a spell effect's model are three pickers over
//! three catalogues, and a favourite of one is noise in the others: a starred
//! `Spells\Fire_Bolt.m2` has no place in a list of trees. Each line carries
//! its kind, and a picker asks for its own.
//!
//! ## Where it is written, and that it is invented
//!
//! `Edit\favourites.txt`, beside the project folders and not inside one. A
//! project is a place being worked and its files are packed into a patch
//! archive on publish; a list of what somebody likes is neither an edit nor a
//! file the game reads, and putting it in the project would ship it. The real
//! client has nowhere to put this and neither does Noggit, so the file is this
//! crate's own, on the terms `crate::session` states for the project folder:
//! plain lines, one fact each, readable in any editor.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

/// The file's name under `Edit\`.
pub const FILE: &str = "favourites.txt";

/// How many recent models are kept per kind.
pub const RECENT_MAX: usize = 12;

/// Which picker a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Doodad,
    Wmo,
    Effect,
}

impl Kind {
    /// The word on the line.
    fn word(self) -> &'static str {
        match self {
            Kind::Doodad => "doodad",
            Kind::Wmo => "wmo",
            Kind::Effect => "effect",
        }
    }

    fn parse(word: &str) -> Option<Kind> {
        match word {
            "doodad" => Some(Kind::Doodad),
            "wmo" => Some(Kind::Wmo),
            "effect" => Some(Kind::Effect),
            _ => None,
        }
    }
}

/// The lists, and where they were read from.
#[derive(Resource, Default)]
pub struct Favourites {
    /// The `Edit\` folder these were read from, so a project switch that
    /// stays in the same install does not re-read the file.
    loaded_from: Option<PathBuf>,
    /// Starred, in the order they were starred; lower-case paths.
    starred: Vec<(Kind, String)>,
    /// Used, newest first; lower-case paths.
    recent: Vec<(Kind, String)>,
}

impl Favourites {
    /// Read the file under `dir` once. Idempotent per directory.
    pub fn load_from(&mut self, dir: &Path) {
        if self.loaded_from.as_deref() == Some(dir) {
            return;
        }
        self.loaded_from = Some(dir.to_path_buf());
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
        let (starred, recent) = parse(&text);
        self.starred = starred;
        self.recent = recent;
    }

    /// Whether a path is starred for a kind.
    pub fn is_starred(&self, kind: Kind, path: &str) -> bool {
        let key = path.to_ascii_lowercase();
        self.starred.iter().any(|(k, p)| *k == kind && *p == key)
    }

    /// Star, or unstar, one path.
    pub fn toggle(&mut self, kind: Kind, path: &str) {
        let key = path.to_ascii_lowercase();
        match self
            .starred
            .iter()
            .position(|(k, p)| *k == kind && *p == key)
        {
            Some(at) => {
                self.starred.remove(at);
            }
            None => self.starred.push((kind, key)),
        }
        self.save();
    }

    /// Note that a path was used: it goes to the head of the recent list.
    pub fn used(&mut self, kind: Kind, path: &str) {
        let key = path.to_ascii_lowercase();
        self.recent.retain(|(k, p)| !(*k == kind && *p == key));
        self.recent.insert(0, (kind, key));
        let mut kept = 0usize;
        self.recent.retain(|(k, _)| {
            if *k != kind {
                return true;
            }
            kept += 1;
            kept <= RECENT_MAX
        });
        self.save();
    }

    /// The starred paths of a kind, in the order they were starred.
    pub fn starred(&self, kind: Kind) -> Vec<String> {
        self.starred
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// The recently used paths of a kind, newest first.
    pub fn recent(&self, kind: Kind) -> Vec<String> {
        self.recent
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, p)| p.clone())
            .collect()
    }

    fn save(&self) {
        let Some(dir) = &self.loaded_from else { return };
        if let Err(e) = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(dir.join(FILE), render(&self.starred, &self.recent)))
        {
            warn!("{} could not be written: {e}", dir.join(FILE).display());
        }
    }
}

/// `star <kind> <path>` and `recent <kind> <path>`, one per line; anything
/// else is skipped.
fn parse(text: &str) -> (Vec<(Kind, String)>, Vec<(Kind, String)>) {
    let mut starred = Vec::new();
    let mut recent = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.splitn(3, ' ');
        let (Some(what), Some(kind), Some(path)) = (words.next(), words.next(), words.next())
        else {
            continue;
        };
        let Some(kind) = Kind::parse(kind) else {
            continue;
        };
        let path = path.trim().to_ascii_lowercase();
        if path.is_empty() {
            continue;
        }
        match what {
            "star" => starred.push((kind, path)),
            "recent" => recent.push((kind, path)),
            _ => {}
        }
    }
    (starred, recent)
}

fn render(starred: &[(Kind, String)], recent: &[(Kind, String)]) -> String {
    let mut out =
        String::from("# models starred and recently used in the world editor, one per line\n");
    for (kind, path) in starred {
        out.push_str(&format!("star {} {path}\n", kind.word()));
    }
    for (kind, path) in recent {
        out.push_str(&format!("recent {} {path}\n", kind.word()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_round_trips_and_keeps_kinds_apart() {
        let mut lists = Favourites::default();
        lists.toggle(Kind::Doodad, "World\\Tree.m2");
        lists.toggle(Kind::Effect, "Spells\\Bolt.m2");
        lists.used(Kind::Doodad, "World\\Rock.m2");
        lists.used(Kind::Doodad, "World\\Tree.m2");
        assert!(lists.is_starred(Kind::Doodad, "world\\tree.m2"));
        assert!(
            !lists.is_starred(Kind::Wmo, "world\\tree.m2"),
            "a star is per kind"
        );
        assert_eq!(
            lists.recent(Kind::Doodad),
            vec!["world\\tree.m2", "world\\rock.m2"]
        );
        assert!(lists.recent(Kind::Effect).is_empty());

        let text = render(&lists.starred, &lists.recent);
        let (starred, recent) = parse(&text);
        assert_eq!(starred, lists.starred);
        assert_eq!(recent, lists.recent);
    }

    #[test]
    fn a_star_toggles_off_and_the_recent_list_is_capped_per_kind() {
        let mut lists = Favourites::default();
        lists.toggle(Kind::Wmo, "a.wmo");
        lists.toggle(Kind::Wmo, "A.WMO");
        assert!(lists.starred(Kind::Wmo).is_empty());
        for i in 0..(RECENT_MAX + 5) {
            lists.used(Kind::Doodad, &format!("d{i}.m2"));
        }
        lists.used(Kind::Wmo, "b.wmo");
        assert_eq!(lists.recent(Kind::Doodad).len(), RECENT_MAX);
        assert_eq!(
            lists.recent(Kind::Doodad)[0],
            format!("d{}.m2", RECENT_MAX + 4)
        );
        assert_eq!(
            lists.recent(Kind::Wmo),
            vec!["b.wmo"],
            "the cap is per kind"
        );
        // A path used again moves to the head rather than appearing twice.
        lists.used(Kind::Doodad, "d3.m2");
        let recent = lists.recent(Kind::Doodad);
        assert_eq!(recent[0], "d3.m2");
        assert_eq!(recent.iter().filter(|p| *p == "d3.m2").count(), 1);
    }

    /// A line with a path containing spaces keeps the whole path, and a
    /// damaged line is skipped rather than refusing the file.
    #[test]
    fn a_path_may_carry_spaces_and_a_bad_line_is_skipped() {
        let (starred, recent) = parse(
            "star doodad world\\azeroth\\some folder\\a tree.m2\nnonsense\nrecent wmo\nrecent whatever x.wmo\n# note\n",
        );
        assert_eq!(
            starred,
            vec![(
                Kind::Doodad,
                "world\\azeroth\\some folder\\a tree.m2".to_string()
            )]
        );
        assert!(recent.is_empty());
    }
}
