//! The browser: the list of a workspace's tables, and the list of the open
//! table's rows with the buttons that act on the table and on a row.

use bevy_egui::egui;

use crate::tables::{commands::Command, RowLabel};
use crate::tables;
use crate::ui::tables::commands::{row_menu, run_command};
use crate::ui::tables::previews::previews;
use crate::ui::tables::{ICON, ROW_HEIGHT, Workspace};
use crate::ui::widgets::theme;

use vale_assets::tables::schema;

/// The docked panel the list is drawn in, for the rows of a table and for
/// the Tables workspace's list of tables.
pub(super) fn browser_panel(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Panel::left("data-browser")
        .default_size(theme::LIST_WIDTH)
        .min_size(220.0)
        .max_size(520.0)
        .resizable(true)
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(8.0))
        .show(ui, contents);
}

/// What one file of `DBFilesClient\` is to the editor and to the server, for
/// its row in the Tables workspace's list.
fn table_about(name: &str) -> String {
    let fields = match schema::for_table(name) {
        Some(_) => "named fields",
        None => "numbered fields",
    };
    let rows = crate::server::rows::MAPPED
        .iter()
        .find(|(dbc, _)| *dbc == name)
        .map(|(_, table)| *table);
    match (rows, vale_mangos::datadir::server_reads_dbc(name)) {
        (Some(table), _) => format!("{fields} · the server reads {table}"),
        (None, true) => format!("{fields} · the server reads the file"),
        (None, false) => format!("{fields} · not sent to the server"),
    }
}

/// The Tables workspace's list before a table is chosen: every file of
/// `DBFilesClient\` the archives list, searched by name.
///
/// A row says how the table is drawn (named fields where a schema describes
/// it, numbered fields where none does) and how this editor sends an edit to
/// the server: as rows of a table, as the copied file, or not at all. The
/// last is a statement about the editor: vmangos reads some of those tables
/// from SQL tables this editor does not write. The mark on the right says the
/// project carries an edited copy.
pub(super) fn table_list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    if work.browser.table_names.is_none() {
        let mut names: Vec<String> = work
            .assets
            .with_archive(|chain| Ok(chain.list_prefix("DBFilesClient\\")))
            .unwrap_or_default()
            .iter()
            .filter_map(|path| {
                let file = tables::labels::basename(path);
                let stem = file.get(..file.len().checked_sub(4)?)?;
                file[stem.len()..]
                    .eq_ignore_ascii_case(".dbc")
                    .then(|| schema::table_name(stem).to_string())
            })
            .collect();
        names.sort_by_key(|name| name.to_ascii_lowercase());
        names.dedup();
        work.browser.table_names = Some(names);
    }
    let names = work.browser.table_names.clone().unwrap_or_default();

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Tables")
                .strong()
                .size(15.0)
                .color(theme::INK),
        );
        ui.label(theme::number(format!("{} files", names.len())));
    })
    .response
    .on_hover_text(
        "Every file of DBFilesClient\\ in the archives. A table with a schema opens as \
         named fields; any other opens as numbered fields.",
    );
    ui.add_space(4.0);
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.browser.table_query)
            .hint_text("table name")
            .desired_width(width),
    );
    ui.add_space(4.0);
    let query = work.browser.table_query.trim().to_ascii_lowercase();
    let matches: Vec<&String> = names
        .iter()
        .filter(|name| query.is_empty() || name.to_ascii_lowercase().contains(&query))
        .collect();
    ui.label(
        egui::RichText::new(format!("{} shown", matches.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);

    let mut chosen: Option<String> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            for at in range {
                let name = matches[at];
                let carried = work
                    .session
                    .project
                    .path_for(&vale_assets::tables::dbc::dbc_path(name))
                    .is_some_and(|disk| disk.exists());
                let mark = match (work.session.unsaved_tables.contains(name), carried) {
                    (true, _) => "unsaved",
                    (false, true) => "edited",
                    (false, false) => "",
                };
                let shape = theme::list_row(
                    ui,
                    theme::ListRow {
                        title: name,
                        sub: &table_about(name),
                        trailing: mark,
                        tint: theme::INK,
                        picture: false,
                    },
                    false,
                );
                if shape.response.clicked() {
                    chosen = Some(name.clone());
                }
            }
        });
    if let Some(name) = chosen {
        work.browser.look_at(&name);
    }
}

/// The left column: the workspace's parts and the chain's tabs, a search box,
/// and the rows that match.
pub(super) fn list(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    // The parts of a workspace of several tools, which for this panel is
    // Items and Sets. The shell switches after the frame is drawn.
    if let Some(part) = crate::ui::shell::rail::parts(ui, work.tool) {
        work.asks.push(crate::ui::shell::asks::ShellAsk::Part(part));
    }
    // The Tables workspace reaches every table from its list, so its way
    // to another table is back to that list.
    if work.tool == crate::tools::Tool::Tables
        && crate::ui::widgets::reference::link(ui, "\u{2039} Tables", theme::ACCENT, "Back to the list of tables.")
    {
        work.browser.look_at(tables::tabs::ANY);
        work.browser.back.clear();
        return;
    }
    // The chain's tables as tabs, like the reference tool's sidebar. A spell,
    // its visual, its kits and its effects are four tables a person switches
    // between often, so they need a way in other than following a reference.
    // The tool decides which tables they are (see `Tool::tabs`), because there
    // are two chains. A table outside the chain, such as `SpellIcon` reached by
    // following a reference, highlights no tab, and pressing a tab still opens
    // that tab's table.
    let tabs = work.tool.tabs();
    let before: &'static str = tabs
        .iter()
        .find(|(_, table)| *table == work.browser.table)
        .map(|(_, table)| *table)
        .unwrap_or("");
    // A single tab is not drawn, and more than a row's worth are drawn in
    // rows: the spell workspace's chain, then its skill tables.
    let mut current = before;
    if tabs.len() > 1 {
        for row in tabs.chunks(tables::tabs::TAB_ROW) {
            ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
                theme::segmented(ui, &mut current, row, |a, b| a == b);
            });
        }
        ui.add_space(6.0);
    }
    if current != before {
        work.browser.look_at(current);
    }

    let table_name = work.browser.table.clone();
    let count = work
        .session
        .table(&table_name)
        .map(|table| table.record_count())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&table_name)
                .strong()
                .size(15.0)
                .color(theme::INK),
        );
        ui.label(theme::number(format!("{count} rows")));
    });
    ui.add_space(4.0);
    let width = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(&mut work.browser.query)
            .hint_text(match previews(&table_name) {
                true => "name, id, a model's name, or what it belongs to",
                false => "name, id, or what it belongs to",
            })
            .desired_width(width),
    );
    ui.add_space(4.0);
    table_actions(ui, work);
    ui.add_space(2.0);
    row_actions(ui, work);
    ui.add_space(2.0);

    // `matches` is rebuilt only when the query or the table changed, so this is
    // a borrow of a cached list rather than a search per frame.
    let matches: Vec<usize> = work.browser.matches(work.session).to_vec();
    ui.label(
        egui::RichText::new(format!("{} shown", matches.len()))
            .small()
            .color(theme::INK_FAINT),
    );
    ui.add_space(4.0);

    // Bring the open row into view when it was opened from somewhere other
    // than this list. `matches` is in table order, so it is searched rather
    // than walked. A row the query hides is not marked as shown, so clearing
    // the query brings it into view.
    let open = work.browser.open.map(|record| (table_name.clone(), record));
    let mut reveal = None;
    if open != work.browser.revealed {
        match open.as_ref().map(|(_, record)| matches.binary_search(record)) {
            Some(Ok(at)) => {
                reveal = Some(at);
                work.browser.revealed = open;
            }
            Some(Err(_)) => {}
            None => work.browser.revealed = None,
        }
    }
    let mut run: Option<(usize, Command)> = None;
    theme::list_area(ui, reveal, ROW_HEIGHT)
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            for at in range {
                let record = matches[at];
                let label = work.browser.describe(work.session, &table_name, record);
                let chosen = work.browser.open == Some(record);
                let picture = row_picture(work, &table_name, record);
                let response = row_pictured(ui, work, &label, picture, chosen);
                if response.clicked() {
                    work.browser.open = Some(record);
                    work.browser.revealed = Some((table_name.clone(), record));
                    work.browser.forget_buffers();
                }
                response.context_menu(|ui| {
                    if let Some(command) = row_menu(ui, work, &table_name, record) {
                        run = Some((record, command));
                    }
                });
            }
        });
    // Run once the list is drawn: a command that adds or removes a row
    // changes which rows the indices the list was drawn from name.
    if let Some((record, command)) = run {
        let ctx = ui.ctx().clone();
        run_command(&ctx, work, &table_name, record, command);
    }
}

/// `Undo`, `Redo`, `Save`, `Discard`: the four actions on the table as a file
/// rather than on one of its rows.
///
/// Undo and redo repeat the inspector's pair here, because egui takes
/// `Ctrl+Z` while a text box has keyboard focus, and in a workspace made of
/// text boxes one usually does. Discard reloads the table from the file and
/// removes its entries from the undo stack. `EditSession::discard_table`
/// explains why the entries are removed rather than kept.
fn table_actions(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let table_name = work.browser.table.clone();
    let unsaved = work.session.unsaved_tables.contains(&table_name);
    let undo = work.session.history.next_undo().map(|c| c.label.clone());
    let redo = work.session.history.next_redo().map(|c| c.label.clone());
    // The open row by its id rather than its index, so a discard that
    // renumbers the table can find it again.
    let open_id = work
        .browser
        .open
        .and_then(|record| work.session.table(&table_name)?.u32_at(record, 0));
    ui.horizontal(|ui| {
        let quarter = ((ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0).max(40.0);
        let size = egui::vec2(quarter, 22.0);
        if ui
            .add_enabled(undo.is_some(), egui::Button::new("Undo").min_size(size))
            .on_hover_text(match &undo {
                Some(label) => format!("Ctrl+Z — {label}"),
                None => "nothing to undo".to_string(),
            })
            .clicked()
        {
            work.session.undo(work.assets);
            work.browser.forget_buffers();
        }
        if ui
            .add_enabled(redo.is_some(), egui::Button::new("Redo").min_size(size))
            .on_hover_text(match &redo {
                Some(label) => format!("Ctrl+Y — {label}"),
                None => "nothing to redo".to_string(),
            })
            .clicked()
        {
            work.session.redo(work.assets);
            work.browser.forget_buffers();
        }
        if ui
            .add_enabled(unsaved, egui::Button::new("Save").min_size(size))
            .on_hover_text(format!(
                "write {table_name}.dbc into the project folder. Ctrl+S writes every table and tile."
            ))
            .on_disabled_hover_text("nothing has changed since the last save")
            .clicked()
        {
            work.session.save_table(&table_name);
        }
        if ui
            .add_enabled(unsaved, egui::Button::new("Discard").min_size(size))
            .on_hover_text(format!(
                "discard every unsaved change to {table_name}: reload the file and remove its \
                 entries from the undo stack. The rows return to the project folder's copy, or \
                 to the archives' copy when the project has none."
            ))
            .on_disabled_hover_text("nothing has changed since the last save")
            .clicked()
            && work.session.discard_table(work.assets, &table_name)
        {
            work.browser.forget_buffers();
            work.browser.open = open_id.and_then(|id| work.session.table(&table_name)?.row_of(id));
        }
    });
}

/// `+ New`, `Clone`, `Delete`: the three things that make or unmake a row of
/// the open table, each one entry on the undo stack. See `crate::tables`.
/// Clone and Delete are two of the row's commands, which its right-click
/// menu also lists.
fn row_actions(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let table_name = work.browser.table.clone();
    let open = work.browser.open;
    ui.horizontal(|ui| {
        let third = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(40.0);
        let size = egui::vec2(third, 22.0);
        // A new area is a zone on the open map, with an explore bit of its
        // own; a blank row would be on map 0 and share bit 0 with two shipped
        // rows. A sub-area is made from its zone's row.
        let zones = table_name == tables::zones::area::TABLE;
        let (word, about) = match zones {
            true => (
                "+ New zone",
                format!(
                    "a new zone on the open map, {} ({}), under the next id, with an explore \
                     bit no other area holds. A sub-area is made from its zone's row.",
                    work.session.map, work.session.map_id
                ),
            ),
            false => ("+ New", format!("a blank {table_name} row under the next id")),
        };
        if ui
            .add_sized(size, egui::Button::new(word))
            .on_hover_text(about)
            .clicked()
        {
            let made = match zones {
                true => {
                    let map = work.session.map_id;
                    tables::zones::add_zone(work.session, map)
                }
                false => tables::rows::add_row(work.session, &table_name),
            };
            if let Some(at) = made {
                work.browser.open_row(at);
            }
        }
        for (name, command) in [("Clone", Command::Clone), ("Delete", Command::Delete)] {
            // The button keeps its one word. What the menu's entry says after
            // it, which for a delete is what it breaks, is in the tooltip.
            let said = open.and_then(|record| {
                tables::commands::command_label(work.browser, work.session, &table_name, record, command)
            });
            let about = tables::commands::command_about(command);
            let tip = match &said {
                Some(said) if said != name => format!("{said}. {about}"),
                _ => about.to_string(),
            };
            if ui
                .add_enabled(open.is_some(), egui::Button::new(name).min_size(size))
                .on_hover_text(tip)
                .on_disabled_hover_text("Open a row first.")
                .clicked()
            {
                if let Some(record) = open {
                    let ctx = ui.ctx().clone();
                    run_command(&ctx, work, &table_name, record, command);
                }
            }
        }
    });
}

/// One row of the list: the picture, two lines of text, and the id on the
/// right. The whole rectangle is the button.
///
/// [`theme::list_row`] draws the row, as it does for every list in the editor
/// (see its own note). This function adds only the picture, which here is a
/// spell icon from the thumbnail cache.
pub(in crate::ui) fn row(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    label: &RowLabel,
    icon: Option<String>,
    chosen: bool,
) -> egui::Response {
    row_pictured(ui, work, label, icon.map(Picture::Icon), chosen)
}

/// [`row`] with the picture from either source: an icon from the thumbnail
/// cache, or a model from the portrait rig. See [`Picture`].
pub(super) fn row_pictured(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    label: &RowLabel,
    picture: Option<Picture>,
    chosen: bool,
) -> egui::Response {
    let title = match label.title.is_empty() {
        true => format!("row {}", label.id),
        false => label.title.clone(),
    };
    let shape = theme::list_row(
        ui,
        theme::ListRow {
            title: &title,
            sub: &label.sub,
            trailing: &label.id.to_string(),
            tint: theme::INK,
            picture: picture.is_some(),
        },
        chosen,
    );
    match picture {
        Some(Picture::Icon(path)) => {
            if let Some(texture) = work.picture(&path, ICON) {
                ui.painter().image(
                    texture,
                    shape.picture,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
        // The rig frames a model by its meshes, so an effect that is only
        // particles has no picture. Its square is left empty and is not
        // crossed out as a model that will not open is: nothing is wrong
        // with the model.
        Some(Picture::Model(path)) => {
            work.portraits.want(&path);
            if let crate::preview::portraits::Status::Ready(texture) = work.portraits.status(&path) {
                ui.painter().image(
                    texture,
                    shape.picture,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
        Some(Picture::Blank) | None => {}
    }
    shape.response
}

/// What a list row's picture is drawn from.
pub(super) enum Picture {
    /// An icon's archive path, drawn from the thumbnail cache.
    Icon(String),
    /// A model's archive path, drawn from the portrait rig.
    Model(String),
    /// Nothing, with the room for a picture left: a kit with no model in a
    /// list of kits that have one, so the names stay in one column.
    Blank,
}

/// The picture for a row of a list: a spell's or a skill's icon in the open
/// table's list, for the three tables of the spell chain the first model the
/// row draws, and for a display table the model or the item's icon, in any
/// list.
pub(super) fn row_picture(work: &mut Workspace<'_>, table: &str, record: usize) -> Option<Picture> {
    match table {
        "SpellVisual" | "SpellVisualKit" | "SpellVisualEffectName" => Some(
            tables::visuals::first_model(work.browser, work.session, table, record)
                .map_or(Picture::Blank, Picture::Model),
        ),
        "Spell" | "SpellIcon" | "SkillLine" | "SkillLineAbility"
            if table == work.browser.table =>
        {
            work.icon_of(record).map(Picture::Icon)
        }
        _ => super::appearance::display_table(table)
            .map(|table| super::appearance::row_picture(work, table, record)),
    }
}
