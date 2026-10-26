//! A catalogue of paths browsed as folders: the breadcrumb, the folder rows,
//! and a path split into its folder and file. The model, tileset and area
//! pickers and the Assets workspace's Game Files part use them.
//!
//! A catalogue is lower case, so a folder is found and compared without
//! case. A name is drawn as the archives or the project spell it
//! ([`Spellings`]): `DBFilesClient`, not `dbfilesclient`. A catalogue that is
//! not of paths, such as the area tree, passes [`Spellings::default`], which
//! draws each name as it is.

use bevy_egui::egui;

use crate::session::files::Spellings;
use crate::tools::placements::place;
use crate::ui::widgets::theme;

/// A breadcrumb: the open folder's path as a row of segments. Returns the
/// folder a click asked for.
///
/// The open folder is a label and the folders above it are buttons, so one
/// click reaches any folder on the path.
///
/// `root` names the empty prefix, for a tree whose top level is not one
/// folder. The model and tileset pickers do not need it: everything they hold
/// is under `world\` or `tileset\`, so the root is a real segment, and passing
/// `""` leaves the row unchanged. The area tree needs it: its top level is
/// thirty-six maps, and without a name for the level above them the empty
/// segment draws as an empty button and a map cannot be left once opened.
pub(in crate::ui) fn breadcrumb(ui: &mut egui::Ui, here: &str, root: &str, spell: &Spellings) -> Option<String> {
    let mut go: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        let mut first = true;
        let mut crumb = |ui: &mut egui::Ui, label: &str, walked: Option<String>| {
            if !first {
                ui.label(egui::RichText::new("/").color(theme::INK_FAINT).size(theme::SMALL));
            }
            first = false;
            let text = egui::RichText::new(label).size(theme::SMALL).color(match walked {
                None => theme::INK,
                Some(_) => theme::ACCENT,
            });
            match walked {
                // The open folder.
                None => {
                    ui.label(text);
                }
                Some(to) => {
                    if ui.small_button(text).clicked() {
                        go = Some(to);
                    }
                }
            }
        };

        if !root.is_empty() {
            crumb(ui, root, (!here.is_empty()).then(String::new));
        }
        if here.is_empty() {
            return;
        }
        let mut walked = String::new();
        for segment in here.trim_end_matches(place::SEP).split(place::SEP) {
            walked.push_str(segment);
            walked.push(place::SEP);
            let last = walked.len() == here.len();
            let name = match spell.name_of(&walked) {
                "" => segment.to_string(),
                spelled => spelled.to_string(),
            };
            crumb(ui, &name, (!last).then(|| walked.clone()));
        }
    });
    go
}

/// The folders directly under the open folder, with how many entries each
/// holds. Returns the one a click opened.
pub(in crate::ui) fn folder_rows(
    ui: &mut egui::Ui,
    here: &str,
    folders: &[(String, usize)],
    row: f32,
    spell: &Spellings,
) -> Option<String> {
    let mut go: Option<String> = None;
    for (name, count) in folders {
        let folder = format!("{here}{name}{}", place::SEP);
        let shown = match spell.name_of(&folder) {
            "" => name.as_str(),
            spelled => spelled,
        };
        if folder_row(ui, shown, *count, 0, false, row).clicked() {
            go = Some(folder);
        }
    }
    go
}

/// How far a tree indents each level, in points.
pub(in crate::ui) const INDENT: f32 = 14.0;

/// One folder's row: an arrow, the name `shown`, and how many entries it
/// holds, indented `depth` levels. The arrow points down when `open`, for a
/// tree whose folders open in place. Answers the row's response.
///
/// The arrow is a painted triangle, not a glyph. egui's bundled fonts have no
/// `▸`, and a missing glyph draws as a hollow box beside every folder.
pub(in crate::ui) fn folder_row(
    ui: &mut egui::Ui,
    shown: &str,
    count: usize,
    depth: usize,
    open: bool,
    row: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row - 2.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, egui::CornerRadius::same(3), theme::RAISED);
        }
        let text = painter.with_clip_rect(rect);
        let left = rect.left() + depth as f32 * INDENT;
        let tip = egui::pos2(left + 12.0, rect.center().y);
        let arrow = match open {
            true => vec![
                egui::pos2(tip.x - 4.0, tip.y - 2.0),
                egui::pos2(tip.x + 4.0, tip.y - 2.0),
                egui::pos2(tip.x, tip.y + 3.0),
            ],
            false => vec![
                egui::pos2(tip.x - 3.0, tip.y - 4.0),
                egui::pos2(tip.x + 3.0, tip.y),
                egui::pos2(tip.x - 3.0, tip.y + 4.0),
            ],
        };
        text.add(egui::Shape::convex_polygon(arrow, theme::INK_DIM, egui::Stroke::NONE));
        text.text(
            egui::pos2(left + 20.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            shown,
            egui::FontId::proportional(theme::LIST_TITLE),
            theme::INK,
        );
        text.text(
            egui::pos2(rect.right() - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(theme::SMALL),
            theme::INK_FAINT,
        );
    }
    response
}

/// The folders directly under a prefix with how much is in each, and the files
/// that are directly in it.
///
/// One pass over the catalogue each frame rather than a tree built once and
/// kept. The catalogue is a few thousand strings and the panel is drawn sixty
/// times a second, so the cost is one prefix comparison per string; a kept
/// tree would be a second copy of the listing that has to be kept in step.
pub(in crate::ui) fn split_folder(catalogue: &[String], here: &str) -> (Vec<(String, usize)>, Vec<String>) {
    let mut folders: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut files: Vec<String> = Vec::new();
    for path in catalogue {
        let Some(rest) = path.strip_prefix(here) else {
            continue;
        };
        match rest.find(place::SEP) {
            Some(at) => *folders.entry(rest[..at].to_string()).or_default() += 1,
            None if !rest.is_empty() => files.push(path.clone()),
            None => {}
        }
    }
    (folders.into_iter().collect(), files)
}

/// An archive path as the two parts of a list row: the folder it is in, and its
/// file name without the extension every one of them shares.
pub(in crate::ui) fn split(path: &str) -> (&str, &str) {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let rest = &path[..path.len() - file.len()];
    let folder = rest.trim_end_matches(['\\', '/']);
    let folder = folder.rsplit(['\\', '/']).next().unwrap_or("");
    (folder, file.trim_end_matches(".blp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row names the file and the folder, without the extension all 621
    /// share. The panel is 288 points wide and a tileset path is wider: painted
    /// whole on one line it runs off the edge of the window.
    #[test]
    fn a_row_splits_into_a_folder_and_a_file() {
        assert_eq!(
            split("tileset\\elwynn\\elwynngrassbase.blp"),
            ("elwynn", "elwynngrassbase")
        );
        assert_eq!(split("tileset/elwynn/grass.blp"), ("elwynn", "grass"));
        // A path with nothing above it has no folder, and keeps its name.
        assert_eq!(split("grass.blp"), ("", "grass"));
        assert_eq!(split(""), ("", ""));
    }
}
