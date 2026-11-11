//! The parts of the doodad and WMO panels about more than one placement: a
//! group selection, and the placements locked against editing. The state is
//! [`crate::tools::placements::group`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::place::Kind;
use crate::ui::widgets::theme;

/// What the group lines under the Selection heading asked for.
pub(super) enum Group {
    Nothing,
    /// Keep the primary and drop the rest.
    Only,
    /// Drop the whole selection.
    Clear,
    /// Add every placement of the primary's model in the open tiles.
    AllOfModel,
    /// Lock every selected placement and drop the selection.
    Lock,
    /// Show or hide the list for replacing the selection's model.
    Replace,
    /// Make this member the primary, from the list of members.
    Primary(u32),
    /// Take this member out of the group, from the list of members.
    Drop(u32),
}

/// The Locked row: how many doodads or WMOs of the map are locked, and a
/// button that unlocks them all. Not drawn while none is locked. See
/// `crate::session::locks::PlacementLocks`.
pub(super) fn locked_placements(ui: &mut egui::Ui, session: &mut EditSession, kind: Kind) {
    let noun = match kind {
        Kind::Doodad => "doodads",
        Kind::Wmo => "WMOs",
    };
    let locked = session.placement_locks().of(kind).len();
    if locked == 0 {
        return;
    }
    let about = match kind {
        Kind::Doodad => "The doodads locked on this map. They are drawn red.",
        Kind::Wmo => "The WMOs locked on this map. They are drawn red.",
    };
    theme::property(ui, "Locked", about, |ui| {
        theme::value(ui, locked.to_string());
        if ui
            .button("Unlock All")
            .on_hover_text(format!(
                "Unlock every locked {} on this map. Right-click one to unlock it alone.",
                noun.trim_end_matches('s')
            ))
            .clicked()
        {
            let ids: Vec<u32> = session.placement_locks().of(kind).iter().copied().collect();
            let n = session.set_placements_locked(kind, ids, false);
            session.status = format!("unlocked {n} {noun}");
        }
    });
}

/// The lines about a selection of several, above the primary's own numbers:
/// how many, what they are, and the buttons that act on the group.
///
/// `chosen` is every member's unique id and path, the primary first. With one
/// placement this draws the buttons that act on one as well as on many:
/// Select All of This Model, which makes a group of one model in one press,
/// Replace Model and Lock. See `crate::tools::placements::group`.
pub(super) fn group(ui: &mut egui::Ui, chosen: &[(u32, &str)], noun: &str) -> Group {
    let Some(&(_, primary)) = chosen.first() else {
        return Group::Nothing;
    };
    let mut asked = Group::Nothing;
    if chosen.len() > 1 {
        theme::property(
            ui,
            "Selected",
            "The values below are the primary selection's, which is drawn brighter. \
             Dragging, the handles, the keys and Delete act on all of them.",
            |ui| {
                theme::value(ui, format!("{} {noun}", chosen.len()));
            },
        );
        // What the group is made of, most numerous first: a sweep of a forest
        // is thirty of one tree and a hand-picked group is a few of several.
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for (_, path) in chosen {
            let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
            match counts.iter_mut().find(|(had, _)| had.eq_ignore_ascii_case(leaf)) {
                Some((_, count)) => *count += 1,
                None => counts.push((leaf, 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (leaf, count) in counts.iter().take(6) {
            theme::note(ui, format!("{count} \u{d7} {leaf}"));
        }
        if counts.len() > 6 {
            theme::note(ui, format!("{} other models", counts.len() - 6));
        }
        let member = |index: usize| {
            let (id, path) = chosen[index];
            crate::ui::widgets::members::Member {
                id: u64::from(id),
                label: path.rsplit(['\\', '/']).next().unwrap_or(path).to_string(),
                detail: id.to_string(),
            }
        };
        match crate::ui::widgets::members::list(ui, noun, chosen.len(), member) {
            Some(crate::ui::widgets::members::Pick::Primary(id)) => asked = Group::Primary(id as u32),
            Some(crate::ui::widgets::members::Pick::Drop(id)) => asked = Group::Drop(id as u32),
            None => {}
        }
    }
    // The labels differ too much in length for equal columns: the longest
    // does not fit half the inspector at its narrowest. The row wraps.
    ui.horizontal_wrapped(|ui| {
        let leaf = primary.rsplit(['\\', '/']).next().unwrap_or(primary);
        if ui
            .button("Select All of This Model")
            .on_hover_text(format!(
                "Add every {leaf} in the open tiles to the selection."
            ))
            .clicked()
        {
            asked = Group::AllOfModel;
        }
        if ui
            .button("Replace Model\u{2026}")
            .on_hover_text(format!(
                "Choose a model for the selected {noun}. Each keeps its position \
                 and rotation."
            ))
            .clicked()
        {
            asked = Group::Replace;
        }
        if ui
            .button("Lock")
            .on_hover_text(format!(
                "Lock the selected {noun}: no tool selects, moves or deletes them, and \
                 moving the ground does not carry them, until they are unlocked. \
                 Right-click a locked one to unlock it."
            ))
            .clicked()
        {
            asked = Group::Lock;
        }
        if chosen.len() > 1 {
            if ui
                .button("Select Primary Only")
                .on_hover_text("Keep the primary (brighter) selection and deselect the rest.")
                .clicked()
            {
                asked = Group::Only;
            }
            if ui.button("Clear").clicked() {
                asked = Group::Clear;
            }
        }
    });
    // A little room between the buttons and the rows under them.
    ui.add_space(4.0);
    asked
}
