//! A list of models and the pager over a long list: a model row with its
//! picture, name and star, used by the table workspace's model browser and
//! the placement picker, and the page controls every paged dialog uses.

use bevy_egui::egui;

use crate::ui::widgets::theme;

/// The model browser's rows: tall enough for a picture, and the picture's
/// side. See `crate::preview::portraits`.
pub(in crate::ui) const MODEL_ROW: f32 = 48.0;
const MODEL_PICTURE: f32 = 40.0;

/// What a press on a model row did.
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::ui) struct RowHit {
    pub(crate) clicked: bool,
    pub(crate) double: bool,
    /// The star was pressed, which is not a press on the row.
    pub(crate) starred: bool,
}

/// One row of a model list: the picture, the file's name with its folder
/// under it, and a star at the right.
///
/// Shared with the inspector's placement picker, which lists the same kind
/// of thing one tool over; `kind` is which list the star writes to.
pub(in crate::ui) fn model_row(
    ui: &mut egui::Ui,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &crate::prefs::favourites::Favourites,
    kind: crate::prefs::favourites::Kind,
    path: &str,
    shown: &str,
    lit: bool,
) -> RowHit {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), MODEL_ROW - 2.0),
        egui::Sense::click(),
    );
    let mut hit = RowHit::default();
    if !ui.is_rect_visible(rect) {
        return hit;
    }
    if lit || response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(3),
            match lit {
                true => theme::ACCENT_SUNK,
                false => theme::RAISED,
            },
        );
    }
    let picture = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 3.0, rect.center().y - MODEL_PICTURE * 0.5),
        egui::vec2(MODEL_PICTURE, MODEL_PICTURE),
    );
    crate::preview::portraits::paint(ui, portraits, path, picture);
    let star = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 14.0, rect.center().y),
        egui::vec2(18.0, 18.0),
    );
    let text = ui.painter().with_clip_rect(egui::Rect::from_min_max(
        egui::pos2(picture.right() + 8.0, rect.top()),
        egui::pos2(star.left() - 4.0, rect.bottom()),
    ));
    let (folder, file) = match shown.rfind(['\\', '/']) {
        Some(at) => (&shown[..at], &shown[at + 1..]),
        None => ("", shown),
    };
    text.text(
        egui::pos2(picture.right() + 8.0, rect.center().y - 7.0),
        egui::Align2::LEFT_CENTER,
        file,
        egui::FontId::proportional(12.0),
        match lit {
            true => theme::INK,
            false => theme::INK_DIM,
        },
    );
    text.text(
        egui::pos2(picture.right() + 8.0, rect.center().y + 7.0),
        egui::Align2::LEFT_CENTER,
        folder,
        egui::FontId::proportional(theme::SMALL),
        theme::INK_FAINT,
    );
    let on = favourites.is_starred(kind, path);
    let hint = match on {
        true => "Remove this model from Starred.",
        false => "Add this model to Starred.",
    };
    hit.starred = theme::star_button(ui, ui.id().with(("star", path)), star, on, hint);
    if !hit.starred {
        let response = response.on_hover_text(path.to_string());
        hit.double = response.double_clicked();
        hit.clicked = response.clicked();
    }
    hit
}

/// The page controls: first, back, `page 3 of 12 · 201–300 of 1,183`,
/// forward, last; `PageUp` and `PageDown` on the keyboard. Clamps the page
/// to the pages that exist and returns the range of the list on that page.
pub(in crate::ui) fn pager(
    ui: &mut egui::Ui,
    page: &mut usize,
    total: usize,
    per_page: usize,
) -> std::ops::Range<usize> {
    let (pages, _) = page_bounds(total, *page, per_page);
    let (back, forward) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::PageUp),
            i.key_pressed(egui::Key::PageDown),
        )
    });
    if back {
        *page = page.saturating_sub(1);
    }
    if forward {
        *page = (*page + 1).min(pages - 1);
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(*page > 0, egui::Button::new("|◀")).clicked() {
            *page = 0;
        }
        if ui.add_enabled(*page > 0, egui::Button::new("◀")).clicked() {
            *page -= 1;
        }
        let (_, range) = page_bounds(total, *page, per_page);
        let where_ = match total {
            0 => "no matches".to_string(),
            _ => format!(
                "page {} of {pages} · {}–{} of {total}",
                *page + 1,
                range.start + 1,
                range.end
            ),
        };
        ui.label(egui::RichText::new(where_).small().color(theme::INK_FAINT));
        if ui
            .add_enabled(*page + 1 < pages, egui::Button::new("▶"))
            .clicked()
        {
            *page += 1;
        }
        if ui
            .add_enabled(*page + 1 < pages, egui::Button::new("▶|"))
            .clicked()
        {
            *page = pages - 1;
        }
    });
    page_bounds(total, *page, per_page).1
}

/// How many pages a list of `total` makes at `per_page`, and which of its
/// rows page `page` holds — the last page when `page` is past the end, and
/// an empty range for an empty list. There is always at least one page, so
/// every page number is valid.
fn page_bounds(total: usize, page: usize, per_page: usize) -> (usize, std::ops::Range<usize>) {
    let per_page = per_page.max(1);
    let pages = total.div_ceil(per_page).max(1);
    let page = page.min(pages - 1);
    let start = (page * per_page).min(total);
    let end = (start + per_page).min(total);
    (pages, start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page is a range into the list, the last page is short, a page past
    /// the end is the last one, and an empty list is one empty page.
    #[test]
    fn a_page_is_a_range_and_never_off_the_end() {
        assert_eq!(page_bounds(0, 0, 100), (1, 0..0));
        assert_eq!(page_bounds(0, 7, 100), (1, 0..0));
        assert_eq!(page_bounds(100, 0, 100), (1, 0..100));
        assert_eq!(page_bounds(101, 0, 100), (2, 0..100));
        assert_eq!(page_bounds(101, 1, 100), (2, 100..101));
        assert_eq!(page_bounds(1183, 2, 100), (12, 200..300));
        assert_eq!(page_bounds(1183, 11, 100), (12, 1100..1183));
        assert_eq!(
            page_bounds(1183, 50, 100),
            (12, 1100..1183),
            "past the end is the last page"
        );
        assert_eq!(page_bounds(5, 0, 0), (5, 0..1), "a page holds at least one");
    }
}
