//! A column that names a row of another table: the name it resolves to, the
//! picker, and the buttons that make or copy the row it names. Also what the
//! open row references, which the inspector lists.

use bevy_egui::egui;

use crate::tables::server_users::Named;
use crate::tables::{commands::Command, Modal};
use crate::ui::shell::asks::{ShellAsk, ShellAsks};
use crate::tables;
use crate::ui::tables::commands::run_command;
use crate::ui::tables::field::{number, presence, write};
use crate::ui::tables::form::plural;
use crate::ui::tables::previews::previews;
use crate::ui::tables::sounds::sound_buttons;
use crate::ui::tables::{ICON, Workspace};
use crate::ui::tools::places::lights::{light_params_to_copy, new_light_params};
use crate::ui::widgets::theme;

use vale_assets::tables::schema::{Column, Kind};
use vale_assets::tables::schema;

/// A reference column: the id, what it resolves to, and the controls to pick
/// or follow it.
///
/// See the module comment. The id stays editable for a person who knows it;
/// the picker is for one who does not.
pub(super) fn reference(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    table_name: &str,
    record: usize,
    column: &Column,
    points_at: &'static str,
    raw: u32,
) {
    // Drawn signed, because the column is signed. A reference holds a row id,
    // which is positive, or one of the table's two values for "nothing": `0`
    // and `-1`. Drawn as the `u32` it is read as, `-1` appears as
    // `4294967295` in the id box. The kits hold many of these, because
    // `blank_defaults` and the shipped rows both write `-1`. The label beside
    // the box says "none" for it, and the signed box shows `-1` to match.
    let mut value = raw as i32;
    if number(ui, &mut value).changed() {
        write(work, table_name, record, column, value as u32);
    }
    // The picker, which searches the target table by what its rows resolve
    // to. It is a dialog rather than a popup; see the module comment.
    if ui
        .add(
            egui::Button::new(egui::RichText::new("…").size(13.0)).min_size(egui::vec2(24.0, 22.0)),
        )
        .on_hover_text(match previews(points_at) {
            true => format!("choose a {points_at} row; clicking a row previews it"),
            false => format!("choose a {points_at} row by name"),
        })
        .clicked()
    {
        work.browser.modal = Some(Modal::Pick {
            table: table_name.to_string(),
            record,
            field: column.field,
            points_at,
        });
        work.browser.pick_query.clear();
        work.browser.pick_focus = true;
    }
    if table_name == "Spell"
        && points_at == "SpellVisual"
        && ui
            .small_button("from spell\u{2026}")
            .on_hover_text(tables::commands::command_about(Command::LookLike))
            .clicked()
    {
        let ctx = ui.ctx().clone();
        run_command(&ctx, work, table_name, record, Command::LookLike);
    }
    // Before the row's name, which is long enough to push a button off the
    // panel.
    if points_at == "LightParams"
        && raw != 0
        && raw != u32::MAX
        && ui
            .small_button("copy")
            .on_hover_text(
                "copy this LightParams row with its 18 colour bands and 6 float bands, and \
                 set this field to the copy's id; other lights that use the original keep it",
            )
            .clicked()
    {
        if let Some(id) = new_light_params(work, raw, Some((table_name, record, column.field))) {
            work.browser.forget_buffers();
            follow_reference(work, points_at, id);
        }
    }
    let chain_table = matches!(
        points_at,
        "SpellVisual" | "SpellVisualKit" | "SpellVisualEffectName"
    );

    // Whether `0` means "nothing" depends on the target table. Every table of
    // the spell chain numbers from 1, so a `0` there is one of the two
    // "nothing" values. `Map.dbc` numbers from 0 and map 0 is Azeroth, so
    // treating `0` as nothing showed `none` for `Light.Map` on 232 of 374
    // lights, the most common value in the column. The rule used here: if the
    // target table has a row 0, `0` is a reference to it.
    let zero_is_a_row = work
        .session
        .table(points_at)
        .is_some_and(|table| table.row_of(0).is_some());
    if (raw == 0 && !zero_is_a_row) || raw == u32::MAX {
        ui.label(egui::RichText::new("none").color(theme::INK_FAINT));
        ui.label(
            egui::RichText::new(points_at)
                .small()
                .color(theme::INK_FAINT),
        );
        if points_at == "LightParams"
            && ui
                .small_button("+ new")
                .on_hover_text(
                    "create a LightParams row with its 18 colour bands and 6 float bands, \
                     copied from this light's clear LightParams (or from the Eastern Kingdoms \
                     default light's), and set this field to its id",
                )
                .clicked()
        {
            let from = light_params_to_copy(work, table_name, record);
            if let Some(id) = new_light_params(work, from, Some((table_name, record, column.field))) {
                work.browser.forget_buffers();
                follow_reference(work, points_at, id);
            }
        }
        if chain_table
            && ui
                .small_button("+ new")
                .on_hover_text(format!(
                    "create a blank {points_at} row and set this field to its id"
                ))
                .clicked()
        {
            if let Some(id) =
                tables::rows::link_new(work.session, table_name, record, column.field, points_at)
            {
                work.browser.forget_buffers();
                follow_reference(work, points_at, id);
            }
        }
        return;
    }

    // A sound is known by hearing it, so a reference to one has a button
    // that plays it, and stops it while it plays. It is ahead of the name so
    // that a long name cannot push it out of the column.
    sound_buttons(ui, work, table_name, points_at, raw, true);

    // What the target row is, in words, with a picture where the table has one.
    if points_at == "SpellIcon" {
        let icon = work.icon_by_id(raw);
        work.icon(ui, icon, ICON);
    }
    if points_at == "SpellVisualEffectName" {
        presence(ui, work.model_present(raw));
    }
    // The target table may not be open yet, because the chain's tables open
    // one per frame. Until it opens, only the id is shown.
    let resolved = work
        .session
        .open_table(work.assets, points_at)
        .then(|| work.browser.describe_id(work.session, points_at, raw))
        .flatten();
    match resolved {
        Some(label) => {
            let title = match label.title.is_empty() {
                true => format!("{points_at} {raw}"),
                false => label.title.clone(),
            };
            // The row's name and its detail line as one label that truncates.
            // Two labels each keep a minimum width even when truncated to
            // "…", and with the id box and the buttons before them that
            // minimum made the row wider than the inspector's maximum: the
            // right-docked panel then grew past its own edge line and clipped
            // the rows instead of truncating them. One label has one minimum.
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &title,
                0.0,
                egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), theme::ACCENT),
            );
            if !label.sub.is_empty() {
                job.append(
                    &label.sub,
                    8.0,
                    egui::TextFormat {
                        font_id: egui::TextStyle::Small.resolve(ui.style()),
                        color: theme::INK_DIM,
                        valign: egui::Align::Center,
                        ..Default::default()
                    },
                );
            }
            let whole = match label.sub.is_empty() {
                true => title.clone(),
                false => format!("{title}\n{}", label.sub),
            };
            let hover = format!("{whole}\n\nOpen {points_at} {raw}.");
            if crate::ui::widgets::reference::link_job(ui, job, theme::ACCENT, &hover) {
                follow_reference(work, points_at, raw);
            }
        }
        None => {
            ui.label(
                egui::RichText::new(format!("no {points_at} row has this id"))
                    .small()
                    .color(theme::BAD),
            );
        }
    }
    // Copying a shared row so that only this row uses it. A kit or an effect
    // is copied and this field pointed at the copy; a visual is copied with
    // its whole chain.
    if matches!(points_at, "SpellVisualKit" | "SpellVisualEffectName")
        && ui
            .small_button("copy")
            .on_hover_text(
                "copy the referenced row and set this field to the copy's id; other rows \
                 that reference the original keep it",
            )
            .clicked()
    {
        if let Some(id) =
            tables::rows::unshare(work.session, table_name, record, column.field, points_at)
        {
            work.browser.forget_buffers();
            follow_reference(work, points_at, id);
        }
    }
    if points_at == "SpellVisual"
        && ui
            .small_button("clone chain")
            .on_hover_text(
                "copy this visual with every kit and effect it references, and set this \
                 field to the copy's id; every other spell keeps the original",
            )
            .clicked()
    {
        if let Some(done) =
            tables::rows::clone_chain(work.session, raw, Some((table_name, record, column.field)))
        {
            let copy = done.new_id("SpellVisual", raw).unwrap_or(0);
            work.session.status = format!(
                "SpellVisual {raw} copied to {copy} with {} kits and {} effects; this field now references {copy}",
                done.count("SpellVisualKit"),
                done.count("SpellVisualEffectName")
            );
        }
    }
}

/// Open the table a reference names, at the row it names.
///
/// The table is opened here, not only looked up. `SpellVisual.dbc` is not
/// open until something asks for it, and the first thing to ask for it is the
/// press on the link. When this function only looked the table up, the button
/// did nothing.
pub(crate) fn follow_reference(work: &mut Workspace<'_>, table: &str, id: u32) {
    if !work.session.open_table(work.assets, table) {
        return;
    }
    work.browser.index(work.session, table);
    if !work.browser.follow(work.session, table, id) {
        work.session.status = format!("{table} has no row {id}");
    }
}

/// What this row points at, and what points at it: the inspector's part of
/// the workspace.
///
/// It is in the inspector rather than in the middle because it describes the
/// selection, and the shell's rule is that the inspector shows the selection.
/// See [`crate::ui`].
pub(crate) fn references(ui: &mut egui::Ui, work: &mut Workspace<'_>) {
    let table_name = work.browser.table.clone();
    let Some(record) = work.browser.open else {
        theme::note(ui, "Select a row to see its references.");
        return;
    };
    let Some(schema) = schema::for_table(&table_name) else {
        return;
    };

    theme::heading(ui, "References");
    let mut drawn = 0;
    let mut follow: Option<(&'static str, u32)> = None;
    for column in schema.columns.iter() {
        let Kind::Reference(points_at) = column.kind else {
            continue;
        };
        let Some(value) = work
            .session
            .table(&table_name)
            .and_then(|table| table.u32_at(record, column.field))
        else {
            continue;
        };
        if value == 0 || value == u32::MAX {
            continue;
        }
        drawn += 1;
        let resolved = work.browser.describe_id(work.session, points_at, value);
        let text = match resolved {
            Some(label) if !label.title.is_empty() => format!("{} [{value}]", label.title),
            _ => format!("{points_at} {value}"),
        };
        ui.horizontal(|ui| {
            ui.add_sized(
                [theme::LABEL_WIDTH + 30.0, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new(column.name).color(theme::INK_DIM))
                    .truncate()
                    .selectable(false),
            );
            let hover = format!("Open {points_at} {value}.");
            if crate::ui::widgets::reference::link_text(ui, egui::RichText::new(text).size(theme::SMALL), theme::ACCENT, &hover) {
                follow = Some((points_at, value));
            }
        });
    }
    if drawn == 0 {
        theme::note(ui, "Every reference column of this row is zero.");
    }
    if let Some((table, id)) = follow {
        follow_reference(work, table, id);
    }

    // The rows that point at this one, looked up in the reference index, and
    // the server templates that name it.
    theme::heading(ui, "Used by");
    let id = work
        .session
        .table(&table_name)
        .and_then(|table| table.u32_at(record, 0))
        .unwrap_or(0);
    let uses = work.browser.used_by(work.session, &table_name, id);
    let server = work.browser.server_users.of(&table_name, id, &work.session.rows.edits).unwrap_or_default();
    if uses.is_empty() && server.is_empty() {
        theme::note(ui, "No row of an open table references this row.");
        return;
    }
    server_users_list(ui, work.asks, &server);
    let mut sources: Vec<String> = uses.iter().map(|at| at.table.clone()).collect();
    sources.sort();
    sources.dedup();
    sources.sort_by_key(|table| table != "Spell");
    let mut follow: Option<(String, usize)> = None;
    for source in sources {
        let here: Vec<&tables::Use> = uses.iter().filter(|at| at.table == source).collect();
        ui.label(
            egui::RichText::new(format!("{} {}", here.len(), plural(&source, here.len())))
                .small()
                .color(theme::INK_FAINT),
        );
        for at in here.iter().take(12) {
            let label = work.browser.describe(work.session, &source, at.record);
            let column = schema::for_table(&source)
                .and_then(|schema| schema.column(at.field))
                .map(|column| column.name)
                .unwrap_or("?");
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let name = match label.title.is_empty() {
                    true => format!("#{}", label.id),
                    false => format!("{} [{}]", label.title, label.id),
                };
                let hover = format!("Open {source} {}.", label.id);
                if crate::ui::widgets::reference::link_text(ui, egui::RichText::new(name).size(theme::SMALL), theme::ACCENT, &hover) {
                    follow = Some((source.clone(), at.record));
                }
                ui.label(egui::RichText::new(column).small().color(theme::INK_FAINT));
            });
        }
        if here.len() > 12 {
            theme::note(ui, format!("…and {} more", here.len() - 12));
        }
    }
    if let Some((table, record)) = follow {
        let id = work
            .session
            .table(&table)
            .and_then(|open| open.u32_at(record, 0))
            .unwrap_or(0);
        follow_reference(work, &table, id);
    }
}

/// The inspector's list of the server templates that name the row, grouped
/// by template table: each as a link with the columns that name the row.
fn server_users_list(ui: &mut egui::Ui, asks: &mut ShellAsks, server: &[Named]) {
    let mut tables: Vec<&'static str> = server.iter().map(|named| named.table).collect();
    tables.dedup();
    for table in tables {
        let here: Vec<&Named> = server.iter().filter(|named| named.table == table).collect();
        ui.label(egui::RichText::new(template_words(table, here.len())).small().color(theme::INK_FAINT));
        for named in here.iter().take(12) {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let text = egui::RichText::new(format!("{} [{}]", named.name, named.entry)).size(theme::SMALL);
                server_user_link(ui, asks, named, text);
                ui.label(egui::RichText::new(named.columns.join(", ")).small().color(theme::INK_FAINT));
            });
        }
        if here.len() > 12 {
            theme::note(ui, format!("…and {} more", here.len() - 12));
        }
    }
}

/// How many rows of a server template table: `1 creature`, `3 items`.
pub(super) fn template_words(table: &str, count: usize) -> String {
    let word = match table {
        "creature_template" => "creature",
        "gameobject_template" => "game object",
        "item_template" => "item",
        "quest_template" => "quest",
        other => other,
    };
    match count {
        1 => format!("1 {word}"),
        _ => format!("{count} {word}s"),
    }
}

/// A server template's name as a link that opens it where it is edited.
pub(super) fn server_user_link(ui: &mut egui::Ui, asks: &mut ShellAsks, named: &Named, text: egui::RichText) {
    use crate::tools::spawns::spawn::SpawnKind;
    use crate::ui::widgets::reference::{link_text, template_link};
    match named.table {
        "creature_template" => template_link(ui, asks, SpawnKind::Creature, named.entry, text),
        "gameobject_template" => template_link(ui, asks, SpawnKind::GameObject, named.entry, text),
        "item_template" => {
            let hover = format!("Open item {} in the item workspace.", named.entry);
            if link_text(ui, text, theme::ACCENT, &hover) {
                asks.push(ShellAsk::ShowItem(named.entry));
            }
        }
        "quest_template" => {
            let hover = format!("Open quest {} in the quest workspace.", named.entry);
            if link_text(ui, text, theme::ACCENT, &hover) {
                asks.push(ShellAsk::ShowQuest(named.entry));
            }
        }
        _ => {
            ui.label(text);
        }
    }
}
