//! What each subsystem says about itself on the HUD.
//!
//! **This exists to stop `hud.rs` being edited by every round.** The HUD is the
//! surface every measurement in this project is read off, so anything new that
//! is worth watching wants a line on it — and until this module existed, adding
//! one meant editing `hud.rs`: a field on its `Scene` system parameter, a query,
//! a counter, and a `ui.label` in the middle of someone else's section. Nine
//! consecutive rounds did exactly that, and it is most of why a round that
//! changed one pass touched ten files.
//!
//! It had also run out of room. A Bevy `SystemParam` tuple holds sixteen, and
//! `Scene` was at sixteen — so the *next* subsystem that wanted a number on
//! screen would have had to restructure the window before it could report
//! anything at all.
//!
//! So the direction is inverted: a pass writes its own line into [`HudReport`]
//! from a system in its own file, holding its own resources, and the HUD prints
//! what it finds in slot order. `render::sky` says what the world is lit by;
//! `render::celestial` says where the sun and the moons stand;
//! `render::residency` says what the client is still holding on to. None of
//! them appears in `hud.rs` at all.
//!
//! ## The slot is a number, chosen locally
//!
//! Ordering has to be stable — a line that moves between runs is a line nobody
//! can watch — and Bevy promises nothing about the order two unordered systems
//! run in, so first-write order will not do. A registry naming every section in
//! one place would work and would be exactly the hub this module exists to
//! remove.
//!
//! So each pass declares its own [`Slot`] beside the system that writes it, and
//! the numbers are spaced by ten so that something can always be put between
//! two of them without renumbering anything. The convention is only that: a
//! collision is two lines in an arbitrary order relative to each other, which
//! is a cosmetic fault and not a bug.
//!
//! ## It is for the *diagnostic* surface and nothing else
//!
//! `ui/mod.rs` says three of its four files are throwaway when FrameXML
//! arrives. This is not one of them and neither is `hud`: the game has no
//! equivalent of a diagnostic window, and every number this project has argued
//! about was read off one.

use bevy::prelude::*;
use std::collections::BTreeMap;

/// Where a subsystem's line sits on the HUD, low first.
///
/// Declared next to the system that writes it — see the module doc. The
/// existing ones, so a new pass can pick a gap:
///
/// ```text
///  10  render::sky        what the world is lit by
///  15  render::terrain    what the tile streamer is holding and handing over
///  16  render::foliage    …and what it planted on it
///  20  render::celestial  where the sun and the moons stand
///  30  lua::host          what the interface is, and what it asked for in vain
///  31  ui::framexml       …and what came out of it as quads
///  32  render::portraits  …and the faces on it
///  33  render::labels     the names over the heads in the world
///  34  render::lightning  the bolts strung between them
///  40  game::combat       the range disagreement watch (pinned)
///  45  sound::mixer       what is audible, and what could not be found
///  90  render::residency  what the client is still holding on to
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Slot(pub u16);

/// **Which tab of the debug panel a line belongs on.**
///
/// The slot orders lines within a tab; this says which tab. Both are chosen
/// locally by the pass that writes the line, for the same reason: a registry
/// naming every section in one place would be exactly the hub this module
/// exists to remove.
///
/// It arrived with the tabbed panel and it is not cosmetic. The panel used to
/// be one column, so every line landed in the same collapsed `scene` section
/// whatever it was about — the interface's own numbers under a heading about
/// meshes, the sky's beside the residency sweep's. A tab that shows the lines
/// about its own subject is the whole reason the tabs are worth having.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Section {
    /// What the frame cost. Nothing writes here yet; it is the honest home for
    /// a pass that wants to report its own per-frame budget.
    Frame,
    /// What is *in* the world — the default, and where a pass that has not
    /// thought about it belongs.
    #[default]
    Scene,
    /// What the world looks like: the light, the sky, the hour.
    World,
    /// The game's own interface, and what it is made of.
    Interface,
    /// The wire, and what the server has been saying.
    Net,
}

/// The lines the passes have written, in slot order.
///
/// A line **persists** until it is overwritten or removed: a pass that reports
/// on a timer of its own (which the residency sweep does, every few seconds)
/// leaves its last line up in between rather than blinking. That is deliberate
/// and it is why this is a map rather than a per-frame buffer that the HUD
/// drains.
///
/// ## Two shelves, and the second one is not decoration
///
/// `hud.rs` prints [`HudReport::lines`] inside its **`scene`** section, which
/// is collapsed until somebody opens it. That is right for what was there
/// first: the sky's line, the celestial bodies' line, the residency sweep's —
/// numbers you go *looking* for once you already suspect something.
///
/// It is wrong for an instrument somebody has been **asked to read**. The first
/// one of those (`combat::desync`, written to answer an open bug report) was
/// invisible twice over: buried in a collapsed section, and silent until the
/// fault it exists for had already fired — so there was nothing to find and no
/// way to tell an armed instrument from an absent one.
///
/// [`HudReport::set_pinned`] is the other shelf, printed **above the fold**
/// beside the session's own warnings, whose comment in `hud.rs` makes exactly
/// this argument: *the whole point of reporting them is that they are seen*.
/// Pin sparingly — a permanent line is a line everybody pays for.
#[derive(Resource, Default)]
pub struct HudReport {
    /// The key is in the sort key as the tie-break, so two passes that pick the
    /// same slot still order deterministically rather than by hash.
    lines: BTreeMap<(Slot, &'static str), Line>,
}

/// One pass's line, which tab it is on, and which shelf.
struct Line {
    text: String,
    section: Section,
    pinned: bool,
}

/// **The run condition every report system takes**: is there anything that will
/// read what this writes?
///
/// The window is closed until `F4` and nothing else prints these lines, so a
/// report written while it is shut is a string built for nobody. That is not
/// free and it is not small: `lua::host::report` walks the interface's whole
/// registration table **twice** — `unfired_events` and `registered_events` each
/// build a `BTreeSet<String>` over 3,746 frames — and a `bevy/trace_chrome`
/// capture at a Northshire framing ranked it at **1.0 ms of a 25.9 ms frame**,
/// third among this client's own systems, with the panel never once opened.
///
/// It is the same finding as the one that deleted the always-on `vale`
/// window — *an instrument nobody is reading is a cost with no output* — and
/// the same fix one layer down: that round removed the walk by removing the
/// window, and this removes the rest of them by asking whether the window is
/// there. A line is one frame stale on the frame the panel opens, which is the
/// whole of what it costs.
///
/// `Option<Res<_>>` so a test app that never installed the panel simply reports
/// nothing rather than panicking on a missing resource.
pub fn watched(panel: Option<Res<crate::ui::debug::SettingsPanel>>) -> bool {
    panel.is_some_and(|panel| panel.open)
}

impl HudReport {
    /// Write this pass's line, replacing whatever it said before.
    ///
    /// `key` names the *writer*, not the content — one pass, one key, however
    /// often it rewrites the text.
    pub fn set(
        &mut self,
        section: Section,
        slot: Slot,
        key: &'static str,
        text: impl Into<String>,
    ) {
        self.lines.insert(
            (slot, key),
            Line {
                text: text.into(),
                section,
                pinned: false,
            },
        );
    }

    /// …and the same, on the shelf that is **above the fold** — the header, on
    /// every tab. A pinned line has no section for that reason: it is not on a
    /// tab, it is over all of them.
    pub fn set_pinned(&mut self, slot: Slot, key: &'static str, text: impl Into<String>) {
        self.lines.insert(
            (slot, key),
            Line {
                text: text.into(),
                section: Section::default(),
                pinned: true,
            },
        );
    }

    /// Take this pass's line off the window — for a pass that has nothing to
    /// say right now, where a stale line would be read as current.
    pub fn clear(&mut self, slot: Slot, key: &'static str) {
        self.lines.remove(&(slot, key));
    }

    /// The unpinned lines on one tab, in slot order.
    pub fn lines(&self, section: Section) -> impl Iterator<Item = &str> {
        self.lines
            .values()
            .filter(move |line| !line.pinned && line.section == section)
            .map(|line| line.text.as_str())
    }

    /// …and the pinned ones, which the HUD prints above the fold.
    pub fn pinned(&self) -> impl Iterator<Item = &str> {
        self.lines
            .values()
            .filter(|line| line.pinned)
            .map(|line| line.text.as_str())
    }

    /// One pass's line, for a test that wants to check what it said without
    /// opening a window.
    pub fn line(&self, slot: Slot, key: &'static str) -> Option<&str> {
        self.lines.get(&(slot, key)).map(|line| line.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKY: Slot = Slot(10);
    const RESIDENCY: Slot = Slot(90);

    /// The ordering is the slot's and nothing else's — not write order, which
    /// is whatever Bevy scheduled that run, and not the key's spelling.
    #[test]
    fn the_lines_come_out_in_slot_order_however_they_went_in() {
        let mut report = HudReport::default();
        report.set(Section::Scene, RESIDENCY, "residency", "held: 200 models");
        report.set(Section::Scene, SKY, "sky", "12:00 sun 255/136/0");
        assert_eq!(
            report.lines(Section::Scene).collect::<Vec<_>>(),
            ["12:00 sun 255/136/0", "held: 200 models"]
        );
    }

    /// **A line is on exactly one tab**, and the tab is the writer's choice.
    ///
    /// The failure this guards is the one the tabs were built to fix in
    /// reverse: a line that appears on every page is a line that means nothing
    /// on any of them.
    #[test]
    fn a_line_is_on_the_tab_its_writer_named_and_no_other() {
        let mut report = HudReport::default();
        report.set(Section::World, SKY, "sky", "12:00 sun 255/136/0");
        report.set(Section::Interface, Slot(30), "lua", "3746 frames");
        assert_eq!(
            report.lines(Section::World).collect::<Vec<_>>(),
            ["12:00 sun 255/136/0"]
        );
        assert_eq!(
            report.lines(Section::Interface).collect::<Vec<_>>(),
            ["3746 frames"]
        );
        assert_eq!(report.lines(Section::Scene).count(), 0);
        assert_eq!(report.lines(Section::Net).count(), 0);
        // …and a writer that changes its mind moves the line rather than
        // leaving a copy behind, because the key is what identifies it.
        report.set(Section::Net, SKY, "sky", "12:00 sun 255/136/0");
        assert_eq!(report.lines(Section::World).count(), 0);
        assert_eq!(report.lines(Section::Net).count(), 1);
    }

    /// **The two shelves are disjoint**, and a pass that pins is off every tab
    /// — otherwise the line appears twice, which reads as two passes reporting
    /// the same thing.
    #[test]
    fn a_pinned_line_is_above_the_fold_and_only_there() {
        let mut report = HudReport::default();
        report.set(Section::Scene, SKY, "sky", "12:00 sun 255/136/0");
        report.set_pinned(RESIDENCY, "desync", "range: armed");
        assert_eq!(
            report.lines(Section::Scene).collect::<Vec<_>>(),
            ["12:00 sun 255/136/0"]
        );
        assert_eq!(report.pinned().collect::<Vec<_>>(), ["range: armed"]);
        // …and `line` finds either, because it is what a test asks with.
        assert_eq!(report.line(RESIDENCY, "desync"), Some("range: armed"));
        // Pinning is a property of the write, so a pass may change its mind and
        // does not end up on both shelves.
        report.set(Section::Scene, RESIDENCY, "desync", "range: armed");
        assert_eq!(report.pinned().count(), 0);
        assert_eq!(report.lines(Section::Scene).count(), 2);
    }

    /// A pass rewriting its line replaces it rather than adding a second — the
    /// failure this guards is a panel that grows a line per frame.
    #[test]
    fn a_pass_has_one_line_however_often_it_writes() {
        let mut report = HudReport::default();
        for hour in 0..24 {
            report.set(Section::Scene, SKY, "sky", format!("{hour:02}:00"));
        }
        assert_eq!(report.lines(Section::Scene).count(), 1);
        assert_eq!(report.line(SKY, "sky"), Some("23:00"));
        report.clear(SKY, "sky");
        assert_eq!(report.lines(Section::Scene).count(), 0);
    }

    /// Two passes that pick the same slot still order deterministically. The
    /// slot convention is spacing rather than enforcement, so this is the
    /// behaviour when it is not followed: a fixed order, not a hash order that
    /// swaps between runs.
    #[test]
    fn a_slot_collision_is_cosmetic_and_not_unstable() {
        let mut report = HudReport::default();
        report.set(Section::Scene, Slot(50), "zebra", "z");
        report.set(Section::Scene, Slot(50), "alpha", "a");
        assert_eq!(report.lines(Section::Scene).collect::<Vec<_>>(), ["a", "z"]);
    }
}
