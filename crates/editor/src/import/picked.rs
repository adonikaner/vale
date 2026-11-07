//! Several files chosen at once in the Project Files and Game Files lists,
//! and which files a right-click acts on.
//!
//! ## The rules
//!
//! ```text
//! click                chooses the file alone
//! ctrl + click         adds the file to the chosen files, or takes it out
//! shift + click        chooses every shown file from the anchor to this one,
//!                      in the order the list shows them, and nothing else
//! ctrl + shift + click adds that range to the chosen files
//! right-click          on a chosen file, acts on every chosen file; on any
//!                      other file, chooses it alone and acts on it
//! ```
//!
//! The anchor is the file the last click or ctrl + click landed on. A range
//! is taken from the anchor and does not move it, so a second shift + click
//! measures from the same file, as a file manager does. The map window adds
//! a tile to its selection with ctrl as well (`crate::ui::map::mapview`).
//! The placement tools add with shift (`crate::tools::placements::group`)
//! because a viewport has no order for a range; a list has one.
//!
//! The part's own `selected` stays the file whose details and preview are
//! shown, and is always one of the chosen files. [`Picked::follow`] keeps the
//! two in step when something else chooses a file: Show in Project Files, an
//! import that finished, `--game-file`.

use std::collections::BTreeSet;

/// How a click on a file's row was made, from the modifier keys held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Click {
    /// No modifier: choose the file alone.
    Only,
    /// Ctrl: add the file, or take it out.
    Toggle,
    /// Shift: choose the range from the anchor, and nothing else.
    Range,
    /// Ctrl and shift: add the range from the anchor.
    AddRange,
}

impl Click {
    /// The click the held modifiers make. `command` is ctrl on Windows and
    /// Linux and cmd on macOS, as egui reports it.
    pub(crate) fn of(command: bool, shift: bool) -> Click {
        match (command, shift) {
            (false, false) => Click::Only,
            (true, false) => Click::Toggle,
            (false, true) => Click::Range,
            (true, true) => Click::AddRange,
        }
    }
}

/// The files chosen in one list, by the path the list keys its rows with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Picked {
    chosen: BTreeSet<String>,
    /// Where a shift + click range starts.
    anchor: Option<String>,
}

impl Picked {
    /// Apply a click on `path`. `shown` is every file the list shows, in its
    /// order, which a range is taken from. Answers the file whose details the
    /// part should show next: the one clicked, or for a click that took it
    /// out, another chosen file or none.
    pub(crate) fn click(&mut self, path: &str, how: Click, shown: &[&str]) -> Option<String> {
        match how {
            Click::Only => {
                self.only(path);
                Some(path.to_string())
            }
            Click::Toggle => {
                self.anchor = Some(path.to_string());
                match self.chosen.remove(path) {
                    true => self.first(shown),
                    false => {
                        self.chosen.insert(path.to_string());
                        Some(path.to_string())
                    }
                }
            }
            Click::Range | Click::AddRange => {
                let at = shown.iter().position(|shown| *shown == path);
                let from = self
                    .anchor
                    .as_deref()
                    .and_then(|anchor| shown.iter().position(|shown| *shown == anchor));
                let (Some(at), Some(from)) = (at, from) else {
                    // No anchor among the shown files: a plain click.
                    self.only(path);
                    return Some(path.to_string());
                };
                if how == Click::Range {
                    self.chosen.clear();
                }
                let (low, high) = (from.min(at), from.max(at));
                self.chosen.extend(shown[low..=high].iter().map(|path| path.to_string()));
                Some(path.to_string())
            }
        }
    }

    /// Choose `path` alone, and make it the anchor.
    pub(crate) fn only(&mut self, path: &str) {
        self.chosen.clear();
        self.chosen.insert(path.to_string());
        self.anchor = Some(path.to_string());
    }

    /// Choose nothing.
    pub(crate) fn clear(&mut self) {
        self.chosen.clear();
        self.anchor = None;
    }

    pub(crate) fn contains(&self, path: &str) -> bool {
        self.chosen.contains(path)
    }

    pub(crate) fn len(&self) -> usize {
        self.chosen.len()
    }

    /// The chosen files, in path order.
    pub(crate) fn paths(&self) -> Vec<String> {
        self.chosen.iter().cloned().collect()
    }

    /// The first chosen file in `shown`'s order, or in path order when none
    /// of them is shown.
    fn first(&self, shown: &[&str]) -> Option<String> {
        shown
            .iter()
            .find(|path| self.chosen.contains(**path))
            .map(|path| path.to_string())
            .or_else(|| self.chosen.iter().next().cloned())
    }

    /// The files a right-click on `path` acts on: every chosen file when
    /// `path` is one of them, and otherwise `path` alone, which is then the
    /// only file chosen.
    pub(crate) fn right_click(&mut self, path: &str) -> Vec<String> {
        if !self.chosen.contains(path) {
            self.only(path);
        }
        self.paths()
    }

    /// Keep the chosen files in step with the part's `selected`: a file
    /// chosen by something other than a click is chosen alone, and with no
    /// file selected nothing is chosen.
    pub(crate) fn follow(&mut self, selected: Option<&str>) {
        match selected {
            Some(path) if !self.chosen.contains(path) => self.only(path),
            Some(_) => {}
            None => self.clear(),
        }
    }

    /// Take out every chosen file that `keep` refuses: files that are no
    /// longer listed.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.chosen.retain(|path| keep(path));
        if self.anchor.as_deref().is_some_and(|anchor| !self.chosen.contains(anchor)) {
            self.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWN: [&str; 5] = ["a", "b", "c", "d", "e"];

    /// A click chooses one file, ctrl + click adds and takes out, and taking
    /// out the shown file moves the details to another chosen one.
    #[test]
    fn a_click_chooses_one_and_ctrl_toggles() {
        let mut picked = Picked::default();
        assert_eq!(picked.click("b", Click::Only, &SHOWN).as_deref(), Some("b"));
        assert_eq!(picked.click("d", Click::Toggle, &SHOWN).as_deref(), Some("d"));
        assert_eq!(picked.paths(), ["b", "d"]);
        assert_eq!(picked.click("d", Click::Toggle, &SHOWN).as_deref(), Some("b"));
        assert_eq!(picked.paths(), ["b"]);
        assert_eq!(picked.click("b", Click::Toggle, &SHOWN), None);
        assert_eq!(picked.len(), 0);
        picked.click("c", Click::Toggle, &SHOWN);
        assert_eq!(picked.click("a", Click::Only, &SHOWN).as_deref(), Some("a"));
        assert_eq!(picked.paths(), ["a"]);
    }

    /// Shift + click takes the shown range from the anchor in either
    /// direction and keeps the anchor; ctrl + shift adds the range.
    #[test]
    fn shift_chooses_the_shown_range_from_the_anchor() {
        let mut picked = Picked::default();
        picked.click("b", Click::Only, &SHOWN);
        picked.click("d", Click::Range, &SHOWN);
        assert_eq!(picked.paths(), ["b", "c", "d"]);
        picked.click("a", Click::Range, &SHOWN);
        assert_eq!(picked.paths(), ["a", "b"], "the range is measured from the same anchor");
        picked.click("e", Click::Toggle, &SHOWN);
        picked.click("c", Click::AddRange, &SHOWN);
        assert_eq!(picked.paths(), ["a", "b", "c", "d", "e"]);
        let mut fresh = Picked::default();
        fresh.click("c", Click::Range, &SHOWN);
        assert_eq!(fresh.paths(), ["c"], "with no anchor a range is a plain click");
    }

    /// A right-click on a chosen file acts on every chosen file; on another
    /// file it chooses that file alone.
    #[test]
    fn a_right_click_acts_on_the_selection_or_the_file_alone() {
        let mut picked = Picked::default();
        picked.click("a", Click::Only, &SHOWN);
        picked.click("c", Click::Range, &SHOWN);
        assert_eq!(picked.right_click("b"), ["a", "b", "c"]);
        assert_eq!(picked.right_click("e"), ["e"]);
        assert_eq!(picked.paths(), ["e"]);
    }

    /// A file chosen from elsewhere is chosen alone; nothing selected chooses
    /// nothing; a file no longer listed is taken out.
    #[test]
    fn the_chosen_files_follow_the_selected_file() {
        let mut picked = Picked::default();
        picked.click("a", Click::Only, &SHOWN);
        picked.click("b", Click::Toggle, &SHOWN);
        picked.follow(Some("b"));
        assert_eq!(picked.paths(), ["a", "b"]);
        picked.follow(Some("e"));
        assert_eq!(picked.paths(), ["e"]);
        picked.follow(None);
        assert_eq!(picked.len(), 0);
        picked.click("a", Click::Only, &SHOWN);
        picked.click("c", Click::Range, &SHOWN);
        picked.retain(|path| path != "a");
        assert_eq!(picked.paths(), ["b", "c"]);
        picked.click("c", Click::Range, &SHOWN);
        assert_eq!(picked.paths(), ["c"], "the anchor went with its file");
    }
}
