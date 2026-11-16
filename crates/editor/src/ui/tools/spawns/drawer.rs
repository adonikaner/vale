//! The asset drawer for the spawn tools' Place mode: every creature or game
//! object template, grouped on the left and drawn as pictures on the right.
//!
//! ```text
//! left   All, Starred, Recent, This project and On this map, then a heading
//!        per facet (Type, Rank or Gathering, and Model) that opens on its
//!        values; Model opens as a folder tree of the models' folders
//! right  the group chosen on the left, or while something is typed every
//!        template whose name, subname or entry matches, as a grid of
//!        pictures with the name and the subject's line under each
//! ```
//!
//! The drawer's frame, header and tiles are [`crate::ui::shell::drawer`]'s.
//! What differs from the file drawer is the list: a template is a row of the
//! server's template table, not a file in a folder. The rows are
//! [`SpawnTool::listed`](crate::tools::spawns::spawn::SpawnTool::listed), the
//! whole table that the subject's workspace also lists, read once while the
//! Place mode is open and kept in memory, with the project's edits laid over
//! each row. The groups come from the rows' own columns and from the model
//! each display id resolves to; see [`Facet`].
//!
//! A click keeps the template's entry for the Place mode's section, which
//! takes it on the next frame through `Drawer::offer`. Starred and Recent
//! are kept by entry in `crate::prefs::favourites`.

use bevy_egui::egui;

use super::spawn::{SpawnView, Subject};
use crate::subjects::listing::Listed;
use crate::tools::spawns::spawn::{Catalogued, Facet, Group, Spawned, Template};
use crate::ui::shell::drawer::{self, Drawer, Mark, Purpose};
use crate::ui::widgets::folders::{folder_row, split_folder, INDENT};
use crate::ui::widgets::theme;

/// What the left pane asked for this frame.
enum Go {
    /// Show a group.
    Group(Group),
    /// Open or close a facet's heading.
    Toggle(Facet),
}

/// Draw the drawer for the spawn tool whose Place mode offered it this frame,
/// docked at the bottom of `root`. `chosen` holds the entries the section
/// marks as chosen.
pub(crate) fn draw<S: SpawnView>(root: &mut egui::Ui, mut subject: Subject<'_, S>, chosen: &[String]) {
    // Out of the subject, so the subject can be lent to the pictures while
    // the drawer's frame holds the drawer.
    let Some(drawer) = subject.drawer.take() else {
        return;
    };
    let purpose = Purpose::Spawn(S::KIND);
    refresh(&mut subject);
    let mut search = std::mem::take(&mut subject.tool.new_spawn.search);
    let mut go: Option<Go> = None;
    drawer::frame(root, drawer, purpose, &mut search, |ui, drawer, body, search| {
        let listed = &subject.tool.listed;
        if subject.tool.catalogue.rows.is_empty() {
            let note = match (&listed.trouble, listed.reading() || !listed.is_read()) {
                (Some(trouble), _) => format!("The template table could not be read: {trouble}"),
                (None, true) => format!("Reading the {} table\u{2026}", S::TEMPLATE),
                (None, false) => format!("The {} table is empty.", S::TEMPLATE),
            };
            theme::note(ui, note);
            return;
        }
        let (tree_rect, grid_rect) = drawer::halves(ui, body);
        let searching = !search.trim().is_empty();
        ui.scope_builder(egui::UiBuilder::new().max_rect(tree_rect), |ui| {
            go = tree_pane(ui, &mut subject, searching);
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(grid_rect), |ui| {
            grid_pane(ui, &mut subject, drawer, search, chosen);
        });
    });
    // A group chosen on the left ends the search, since the grid shows the
    // matches while there is one.
    match go {
        Some(Go::Group(group)) => {
            subject.tool.new_spawn.group = group;
            search.clear();
        }
        Some(Go::Toggle(facet)) => {
            let open = &mut subject.tool.new_spawn.open;
            match open.iter().position(|had| *had == facet) {
                Some(at) => {
                    open.remove(at);
                }
                None => open.push(facet),
            }
        }
        None => {}
    }
    subject.tool.new_spawn.search = search;
}

/// Bring the catalogue up to the template list, the project's edits and the
/// spawns on the map. A rebuild walks every template, so it runs only when
/// one of these has changed: the list's two lengths, the store's revision,
/// the map read, or the number of spawns, which a placement changes.
fn refresh<S: SpawnView>(subject: &mut Subject<'_, S>) {
    let revision = subject.session.rows.revision;
    let listed = &subject.tool.listed;
    let key = (
        listed.all.len(),
        listed.created.len(),
        revision,
        subject.tool.map_read(),
        subject.tool.spawns.len(),
    );
    if subject.tool.catalogue.built == Some(key) {
        return;
    }
    let shown: Vec<S::Known> = (0..listed.count())
        .filter_map(|at| listed.shown(at, &subject.session.rows.edits))
        .collect();
    // The display cache is taken out of the tool for the walk, because the
    // two lookups that fill it borrow the whole subject.
    let mut folders = std::mem::take(&mut subject.tool.catalogue.folders);
    let mut rows: Vec<Catalogued<S::Known>> = Vec::with_capacity(shown.len());
    for known in shown {
        let mut facets = S::sorts(subject, &known);
        let display = known.display_id();
        let folder = match folders.get(&display) {
            Some(had) => had.clone(),
            None => {
                let found = S::model_path(subject, display).map(|path| model_folder(&path));
                folders.insert(display, found.clone());
                found
            }
        };
        if let Some(folder) = folder {
            facets.push((Facet::Model, folder));
        }
        rows.push(Catalogued { haystack: known.haystack(), known, facets });
    }
    rows.sort_by(|a, b| {
        a.known
            .name()
            .to_lowercase()
            .cmp(&b.known.name().to_lowercase())
            .then(a.known.entry().cmp(&b.known.entry()))
    });
    let catalogue = &mut subject.tool.catalogue;
    catalogue.rows = rows;
    catalogue.folders = folders;
    catalogue.on_map = subject.tool.spawns.iter().map(|spawn| spawn.entry()).collect();
    catalogue.built = Some(key);
}

/// A model path's folder in lower case, with `\` after each name:
/// `Creature\Wolf\Wolf.m2` is `creature\wolf\`.
fn model_folder(path: &str) -> String {
    let lower = path.to_ascii_lowercase().replace('/', "\\");
    match lower.rfind('\\') {
        Some(at) => lower[..=at].to_string(),
        None => String::new(),
    }
}

/// The left pane: the five lists, then each facet's heading and, when open,
/// its values. Nothing is lit while a search is shown.
fn tree_pane<S: SpawnView>(ui: &mut egui::Ui, subject: &mut Subject<'_, S>, searching: bool) -> Option<Go> {
    let mut go: Option<Go> = None;
    let row = ui.spacing().interact_size.y + 4.0;
    let shown = match searching {
        true => None,
        false => Some(subject.tool.new_spawn.group.clone()),
    };
    let catalogue = &subject.tool.catalogue;
    let count = |group: &Group| catalogue.in_group(group).len();
    let starred = subject.favourites.starred(S::FAVOURITES).len();
    let recent = subject.favourites.recent(S::FAVOURITES).len();
    let lists = [
        ("All", Group::All, Mark::None, catalogue.rows.len()),
        ("Starred", Group::Starred, Mark::Star, starred),
        ("Recent", Group::Recent, Mark::Clock, recent),
        ("This project", Group::Project, Mark::None, count(&Group::Project)),
        ("On this map", Group::OnMap, Mark::None, count(&Group::OnMap)),
    ];
    egui::ScrollArea::vertical()
        .id_salt(("spawn-drawer-tree", S::TEMPLATE))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (name, group, mark, count) in lists {
                let on = shown.as_ref() == Some(&group);
                if drawer::list_row(ui, name, mark, count, on, row, 0.0).clicked() {
                    go = Some(Go::Group(group));
                }
            }
            ui.add_space(4.0);
            for &(facet, name) in S::FACETS {
                let open = subject.tool.new_spawn.open.contains(&facet);
                let values = catalogue.values(facet);
                if folder_row(ui, name, values.len(), 0, open, row).clicked() {
                    go = Some(Go::Toggle(facet));
                }
                if !open {
                    continue;
                }
                match facet {
                    Facet::Model => {
                        // One folder per template, so a folder's count is the
                        // number of templates under it.
                        let folders: Vec<String> = values
                            .iter()
                            .flat_map(|(folder, count)| std::iter::repeat_n(folder.clone(), *count))
                            .collect();
                        let here = match &shown {
                            Some(Group::Facet(Facet::Model, folder)) => folder.as_str(),
                            _ => "",
                        };
                        if let Some(to) = model_tree(ui, &folders, here, shown.is_some(), "", 1, row) {
                            go = Some(Go::Group(Group::Facet(Facet::Model, to)));
                        }
                    }
                    _ => {
                        for (value, count) in values {
                            let group = Group::Facet(facet, value.clone());
                            let on = shown.as_ref() == Some(&group);
                            if drawer::list_row(ui, &value, Mark::None, count, on, row, INDENT).clicked() {
                                go = Some(Go::Group(group));
                            }
                        }
                    }
                }
            }
        });
    go
}

/// The model folders under `prefix`, `depth` levels in, and under each one
/// that `here` is inside, its own folders. Answers the folder a click asked
/// for: a click on the folder shown closes it and shows the one above.
fn model_tree(
    ui: &mut egui::Ui,
    folders: &[String],
    here: &str,
    lit: bool,
    prefix: &str,
    depth: usize,
    row: f32,
) -> Option<String> {
    let mut go: Option<String> = None;
    let (names, _) = split_folder(folders, prefix);
    for (name, count) in names {
        let folder = format!("{prefix}{name}\\");
        let open = here.starts_with(&folder);
        let under = ui.painter().add(egui::Shape::Noop);
        let response = folder_row(ui, &name, count, depth, open, row);
        if lit && here == folder {
            ui.painter().set(under, egui::Shape::rect_filled(response.rect, egui::CornerRadius::same(3), theme::ACCENT_SUNK));
        }
        if response.clicked() {
            go = Some(match here == folder {
                true => prefix.to_string(),
                false => folder.clone(),
            });
        }
        if open {
            go = model_tree(ui, folders, here, lit, &folder, depth + 1, row).or(go);
        }
    }
    go
}

/// The right pane: what the grid shows and how many, then the grid.
fn grid_pane<S: SpawnView>(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_, S>,
    drawer: &mut Drawer,
    search: &str,
    chosen: &[String],
) {
    let nouns = S::WORDS.nouns;
    let catalogue = &subject.tool.catalogue;
    let (heading, shown): (String, Vec<usize>) = match search.trim().is_empty() {
        false => {
            let matches = catalogue.search(search);
            (format!("{} of {} {nouns} match", matches.len(), catalogue.rows.len()), matches)
        }
        true => {
            let group = subject.tool.new_spawn.group.clone();
            let shown: Vec<usize> = match &group {
                Group::Starred | Group::Recent => {
                    let entries = match group {
                        Group::Starred => subject.favourites.starred(S::FAVOURITES),
                        _ => subject.favourites.recent(S::FAVOURITES),
                    };
                    entries
                        .iter()
                        .filter_map(|text| text.parse::<u32>().ok())
                        .filter_map(|entry| catalogue.rows.iter().position(|row| row.known.entry() == entry))
                        .collect()
                }
                other => catalogue.in_group(other),
            };
            (format!("{} \u{b7} {} {nouns}", group_name(&group), shown.len()), shown)
        }
    };
    theme::note(ui, heading);
    ui.add_space(2.0);

    let side = drawer.tile_side();
    let height = side + 4.0 + drawer::LINE * 2.0;
    let width = ui.available_width() - ui.spacing().scroll.allocated_width();
    let columns = ((width + drawer::GAP) / (side + drawer::GAP)).floor().max(1.0) as usize;
    let rows = shown.len().div_ceil(columns);
    // One scroll position per group and per search, so a group opens at its
    // top.
    let salt = match search.trim().is_empty() {
        true => format!("{:?}", subject.tool.new_spawn.group),
        false => "search".to_string(),
    };
    // `show_rows` builds only the rows on screen, so only their pictures are
    // asked for. See `crate::preview::portraits`.
    egui::ScrollArea::vertical()
        .id_salt(("spawn-drawer-grid", S::TEMPLATE, salt))
        .auto_shrink([false, false])
        .show_rows(ui, height, rows, |ui, range| {
            for at in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = drawer::GAP;
                    let end = ((at + 1) * columns).min(shown.len());
                    for &index in &shown[at * columns..end] {
                        let known = subject.tool.catalogue.rows[index].known.clone();
                        let on = chosen.iter().any(|had| *had == known.entry().to_string());
                        template_tile(ui, subject, drawer, &known, on, side, height);
                    }
                });
            }
        });
}

/// What a group is called above the grid.
fn group_name(group: &Group) -> String {
    match group {
        Group::All => "All".to_string(),
        Group::Starred => "Starred".to_string(),
        Group::Recent => "Recent".to_string(),
        Group::Project => "This project".to_string(),
        Group::OnMap => "On this map".to_string(),
        Group::Facet(Facet::Model, folder) => folder.clone(),
        Group::Facet(_, value) => value.clone(),
    }
}

/// One template's tile: the model's picture, the name, the subject's line,
/// and the star. A click is kept in the drawer for the Place mode's section.
fn template_tile<S: SpawnView>(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_, S>,
    drawer: &mut Drawer,
    known: &S::Known,
    on: bool,
    side: f32,
    height: f32,
) {
    let response = drawer::tile(ui, side, height, on);
    if !ui.is_rect_visible(response.rect) {
        return;
    }
    let square = drawer::picture_rect(response.rect, side);
    // A display id the tables do not resolve: the template is still listed,
    // because the server loads a spawn of it.
    if !S::picture(ui, subject, known.display_id(), square) {
        ui.painter().rect_filled(square, egui::CornerRadius::same(2), theme::SUNK);
    }
    let line = match S::row_line(subject, known) {
        line if line.is_empty() => known.entry().to_string(),
        line => line,
    };
    drawer::label(ui, square, known.name(), Some(&line), on);

    let entry = known.entry().to_string();
    let starred_now = subject.favourites.is_starred(S::FAVOURITES, &entry);
    let mut starred = false;
    // Drawn on a starred template, and on any template under the pointer so
    // it can be starred.
    if starred_now || response.contains_pointer() {
        let star = egui::Rect::from_min_size(egui::pos2(square.right() - 18.0, square.top() + 2.0), egui::vec2(16.0, 16.0));
        let hint = match starred_now {
            true => "Remove this template from Starred.",
            false => "Add this template to Starred.",
        };
        starred = theme::star_button(ui, ui.id().with(("spawn-drawer-star", known.entry())), star, starred_now, hint);
        if starred {
            subject.favourites.toggle(S::FAVOURITES, &entry);
        }
    }
    let hover = format!("{}\n{}", known.label(), S::list_line(subject, known));
    if !starred && response.on_hover_text(hover).clicked() {
        subject.favourites.used(S::FAVOURITES, &entry);
        drawer.pick(Purpose::Spawn(S::KIND), entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model's folder is the path up to its last separator, in lower case,
    /// which is how the Model group compares folders.
    #[test]
    fn a_model_folder_is_its_path_up_to_the_file() {
        assert_eq!(model_folder("Creature\\Wolf\\Wolf.m2"), "creature\\wolf\\");
        assert_eq!(model_folder("World/Generic/Box.m2"), "world\\generic\\");
        assert_eq!(model_folder("Box.m2"), "");
    }
}
