//! **A group selection's members, as a list**: one row per selected thing,
//! folded away by default under the group's count.
//!
//! A rectangle drag selects dozens of things at once, and the only way to work
//! on one of them was to shift-click the rest away or click it again in the
//! world, which drops the group. The list names every member. A click on a
//! row makes it the primary, so the panel's numbers and the handles move to it
//! and the group stays as it was; the × on a row takes that member out.
//!
//! Folded by default because a group is usually acted on whole, and a list of
//! forty rows would push the primary's numbers off the panel. The four tools
//! that select groups (doodads, WMOs, creatures, game objects) draw the same
//! list, so it reads the same in each. See `crate::tools::group`.

use bevy_egui::egui;

use super::theme;

/// One row of the list.
pub struct Member {
    /// The tool's own id for it: a placement's unique id or a spawn's guid.
    pub id: u64,
    /// What it is, such as a model's file name or a creature's name.
    pub label: String,
    /// Which one it is, drawn faint at the right: the id, as the panel
    /// below names it.
    pub detail: String,
}

/// What a press on the list asked for.
pub enum Pick {
    /// Make this member the primary, keeping the group.
    Primary(u64),
    /// Take this member out of the group.
    Drop(u64),
}

/// How tall one row is.
const ROW: f32 = 20.0;

/// How tall the list grows before it scrolls: about ten rows.
const MOST: f32 = 210.0;

/// **The list**, folded under a header that says how many there are.
///
/// `member(0)` is the primary and the rest follow in the group's order. It is
/// asked only for the rows on screen, so a tool that has to look a member up
/// to name it does so for a dozen rows a frame rather than for every member.
/// `salt` keeps one tool's fold state from another's.
pub fn list(
    ui: &mut egui::Ui,
    salt: &str,
    count: usize,
    member: impl Fn(usize) -> Member,
) -> Option<Pick> {
    let mut picked = None;
    egui::CollapsingHeader::new(egui::RichText::new(format!("All {count}")).size(theme::SMALL))
        .id_salt(("group-members", salt))
        .default_open(false)
        .show(ui, |ui| {
            theme::note(ui, "click one to make it the primary · × takes it out");
            // `show_rows` counts this ui's spacing into each row's height, so
            // it has to be the spacing the rows are drawn with.
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt(("group-members-scroll", salt))
                .max_height(MOST)
                .auto_shrink([false, true])
                .show_rows(ui, ROW, count, |ui, rows| {
                    for index in rows {
                        if let Some(pick) = row(ui, &member(index), index == 0) {
                            picked = Some(pick);
                        }
                    }
                });
        });
    picked
}

/// One member's row: painted, full width, the primary drawn as the selected
/// row of every list in the editor is.
fn row(ui: &mut egui::Ui, member: &Member, primary: bool) -> Option<Pick> {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
    // The × is its own target at the right end, and only on a member: the
    // primary is taken out by making another member the primary first.
    let cross = egui::Rect::from_min_max(egui::pos2(rect.right() - ROW, rect.top()), rect.max);
    let dropping = match primary {
        true => None,
        false => Some(ui.interact(cross, response.id.with("drop"), egui::Sense::click())),
    };
    let hovered = response.hovered() || dropping.as_ref().is_some_and(|drop| drop.hovered());

    let painter = ui.painter_at(rect);
    if primary {
        painter.rect(
            rect,
            egui::CornerRadius::same(3),
            theme::ACCENT_SUNK,
            egui::Stroke::new(1.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    } else if hovered {
        painter.rect_filled(rect, egui::CornerRadius::same(3), theme::RAISED);
    }
    let y = rect.center().y;
    let detail = painter.text(
        egui::pos2(cross.left() - 4.0, y),
        egui::Align2::RIGHT_CENTER,
        &member.detail,
        egui::FontId::monospace(theme::SMALL),
        theme::INK_FAINT,
    );
    // The label stops short of the id, so a long file name is cut rather
    // than drawn over it.
    let label = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 6.0, rect.top()),
        egui::pos2(detail.left() - 8.0, rect.bottom()),
    );
    ui.painter_at(label).text(
        egui::pos2(label.left(), y),
        egui::Align2::LEFT_CENTER,
        &member.label,
        egui::FontId::proportional(theme::SMALL + 1.0),
        match primary {
            true => theme::INK,
            false => theme::INK_DIM,
        },
    );
    if let Some(drop) = &dropping {
        if hovered {
            let colour = match drop.hovered() {
                true => theme::BAD,
                false => theme::INK_FAINT,
            };
            painter.text(
                cross.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                egui::FontId::proportional(theme::BODY),
                colour,
            );
        }
        if drop.clicked() {
            return Some(Pick::Drop(member.id));
        }
    }
    let response = response.on_hover_text(match primary {
        true => "The primary: the values below and the handles are this one's.",
        false => "Make this the primary. The group stays selected.",
    });
    match response.clicked() && !primary {
        true => Some(Pick::Primary(member.id)),
        false => None,
    }
}
