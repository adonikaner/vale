//! The Equipment window of a selected creature: the `creature_equip_template`
//! set it holds, one row per roll of three items.
//!
//! ## One set per window
//!
//! The window shows the set the creature's template names in
//! `equipment_id`, or, under the creature tool, the set the selected spawn's
//! own addon row names when it has one. The window follows the selection,
//! as the loot window does; the shell writes [`crate::subjects::addons::About`]
//! from the selection each frame. The spawn's addon row itself is the Addon
//! section of the creature inspector, beside the spawn's position.
//!
//! ## What a row shows
//!
//! A row is its three items, main hand, off hand and ranged, each chosen
//! through the item picker or cleared; its probability, which is the row's
//! weight when a spawn rolls one row of the set; and the content patches it
//! is read in. A row the server would empty or skip at load has a red dot,
//! with every reason on hover: an item not in `item_template`, one not held
//! in a hand, a patch range that leaves the server's patch out.
//!
//! ## Edits go to the project, not the database
//!
//! An edit goes into the project's store and onto the undo stack; a save
//! writes `sql\addons.sql`; Apply is on the bar's Server… with the other
//! subjects'. See [`crate::server::addons`].

use crate::session::EditSession;
use crate::subjects::addons::{About, Addons, ShownEquipment};
use crate::subjects::items::Items;
use crate::subjects::quests::{PickFor, Picker, Quests, Target};
use crate::ui::widgets::theme;
use crate::ui::widgets::thumbnails::Thumbnails;
use vale_client::assets::GameAssets;
use vale_mangos::addon::{self, Equipment};
use vale_mangos::row::Life;
use bevy_egui::egui;

/// How wide an item's icon and name are in a slot cell, before the two
/// buttons. A longer name is truncated, with the whole of it on hover.
const NAME: f32 = 170.0;

/// How wide a content patch's menu is.
const PATCH: f32 = 56.0;

/// How wide a small number is drawn: a probability, a patch.
const NUMBER: f32 = 48.0;

/// How tall a cell is.
const CELL: f32 = 18.0;

/// Everything the window needs.
pub(crate) struct Subject<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) addons: &'a mut Addons,
    /// The quest tool's state, for the batched item names and the reference
    /// picker.
    pub(crate) quests: &'a mut Quests,
    /// The item workspace's state, for what an item's icon is.
    pub(crate) items: &'a mut Items,
    pub(crate) assets: &'a GameAssets,
    pub(crate) thumbnails: &'a mut Thumbnails,
    pub(crate) now: f64,
    /// Where an item's name asks the shell to open its row.
    pub(crate) asks: &'a mut crate::ui::shell::asks::ShellAsks,
}

/// The window, drawn when it is open and a creature is selected. Returns
/// its rectangle, so the viewport can keep a click inside it from also being
/// a click on the world behind it.
pub(crate) fn windows(ctx: &egui::Context, mut subject: Subject<'_>) -> Vec<egui::Rect> {
    // An item chosen through the picker on the last frame, taken now; see
    // `PickFor::EquipmentItem`.
    if let Some((set, row, slot, item)) = subject.quests.equipment_pick.take() {
        add_chosen(&mut subject, set, row, slot, item);
    }
    let mut out = Vec::new();
    if let Some(about) = subject.addons.about.clone() {
        if subject.addons.equipment_open {
            out.extend(window(ctx, &mut subject, &about));
        }
    }
    super::quests::picker(
        ctx,
        subject.session,
        subject.quests,
        subject.assets,
        subject.thumbnails,
        None,
        subject.now,
    );
    out
}

/// Put what the picker answered into the set, and what happened on the
/// status line: a new row holding the item, or one slot of a row changed.
fn add_chosen(subject: &mut Subject<'_>, set: u32, row: Option<[u32; 3]>, slot: usize, item: u32) {
    let now = subject.now;
    let done = match row {
        None => {
            let mut items = [0; 3];
            items[slot] = item;
            subject.addons.add_equipment(subject.session, set, items, now)
        }
        Some(items) => {
            let shown = subject
                .addons
                .rows_of(set, &subject.session.rows.edits)
                .into_iter()
                .find(|shown| shown.row.items == items);
            match shown {
                Some(shown) => subject.addons.replace_item(subject.session, &shown, slot, item, now),
                None => Err(format!("set {set} no longer holds that row")),
            }
        }
    };
    subject.session.status = match done {
        Ok(line) => line,
        Err(e) => format!("not changed: {e}"),
    };
}

/// The window.
fn window(ctx: &egui::Context, subject: &mut Subject<'_>, about: &About) -> Option<egui::Rect> {
    let mut keep_open = true;
    let shown = theme::window(format!("{} \u{2014} equipment", about.label), "equipment-window")
        .open(&mut keep_open)
        // Wide enough for three item cells, the three numbers after them and
        // the remove button, and tall enough for the head and about eight
        // rows. A longer set scrolls.
        .default_size([980.0, 420.0])
        .default_pos([320.0, 140.0])
        .show(ctx, |ui| {
            let set = subject.addons.showing(about, &subject.session.rows.edits);
            head(ui, subject, about, set);
            if !set_head(ui, subject, about, set) {
                return;
            }
            body(ui, subject, set);
        });
    if !keep_open {
        subject.addons.equipment_open = false;
    }
    shown.map(|shown| shown.response.rect)
}

/// What the set applies to, and which column names it.
fn head(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, set: u32) {
    let own = about.guid.is_some() && set != about.template_set;
    let sentence = match own {
        true => format!(
            "This spawn's creature_addon row names set {set} in its equipment_id; every other \
             spawn of entry {} holds the template's set {}.",
            about.entry, about.template_set
        ),
        false => format!(
            "creature_template entry {}: every spawn of this entry rolls one row of this set \
             when it loads.",
            about.entry
        ),
    };
    ui.label(egui::RichText::new(sentence).small().color(theme::WARN));
    if let Some(guid) = about.guid.filter(|_| !own) {
        ui.label(
            egui::RichText::new(format!(
                "To give spawn {guid} alone a different set, set equipment_id in its Addon section \
                 in the inspector."
            ))
            .small()
            .color(theme::INK_FAINT),
        );
    }
    let _ = subject;
}

/// The set's number and who shares it, or, for a creature with no set, the
/// button that makes one. `false` when there is no set to draw.
fn set_head(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, set: u32) -> bool {
    if set == 0 {
        theme::note(
            ui,
            "equipment_id is 0, so the creature holds nothing. A set is numbered by the \
             creature's own entry, as the shipped sets are.",
        );
        let free = subject.addons.is_set_read(about.entry)
            && subject.addons.rows_of(about.entry, &subject.session.rows.edits).is_empty();
        if ui
            .add_enabled(free, egui::Button::new(format!("New set {}", about.entry)).small())
            .on_hover_text(format!(
                "Write {} into creature_template.equipment_id, which is a creature edit, and add \
                 rows to the new set here. Apply the creature from the Creatures block of the \
                 Server panel and the set from the Addons and equipment block.",
                about.entry
            ))
            .on_disabled_hover_text(format!(
                "Set {} already has rows, or has not been read yet. Type another number into \
                 equipment_id in the template window.",
                about.entry
            ))
            .clicked()
        {
            subject.session.status = match subject.addons.new_set(subject.session, about, subject.now) {
                Ok(set) => format!("equipment_id set to {set}; add rows to the set"),
                Err(e) => format!("no new set: {e}"),
            };
        }
        return false;
    }
    ui.label(
        egui::RichText::new(format!("{} {set}", addon::EQUIPMENT))
            .small()
            .color(theme::INK_DIM),
    )
    .on_hover_text("Keyed by creature_template.equipment_id, or by creature_addon.equipment_id for one spawn.");
    if let Some(users) = subject.addons.users(set, &subject.session.rows.edits) {
        let others: Vec<&(u32, String)> = users.iter().filter(|(entry, _)| *entry != about.entry).collect();
        let spawns = subject.addons.addon_users(set).unwrap_or(0);
        let mut parts: Vec<String> = Vec::new();
        if !others.is_empty() {
            let names: Vec<String> = others
                .iter()
                .take(4)
                .map(|(entry, name)| format!("{name} ({entry})"))
                .collect();
            let more = match others.len() > 4 {
                true => format!(" and {} more", others.len() - 4),
                false => String::new(),
            };
            parts.push(format!("{} other creature(s): {}{more}", others.len(), names.join(", ")));
        }
        if spawns > 0 {
            parts.push(format!("{spawns} spawn(s) name it in their addon row"));
        }
        match parts.is_empty() {
            true => theme::note(ui, "Only this creature uses this set."),
            false => {
                ui.label(
                    egui::RichText::new(format!(
                        "This set is shared, and an edit changes all of them: {}.",
                        parts.join("; ")
                    ))
                    .small()
                    .color(theme::WARN),
                );
            }
        }
    }
    true
}

/// The rows, and the way to add one.
///
/// The rows the set keeps are a grid under a line of column names, so the
/// cells of every row line up. A row of the database marked for removal is
/// listed under them on its own, with the button that keeps it: changing an
/// item removes the row and makes another, since the items are the key, and
/// the two drawn side by side in the grid read as a set that had grown.
fn body(ui: &mut egui::Ui, subject: &mut Subject<'_>, set: u32) {
    if let Some(trouble) = subject.addons.trouble().cloned() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        return;
    }
    if !subject.addons.is_set_read(set) {
        theme::waiting(ui, format!("reading {} {set}\u{2026}", addon::EQUIPMENT));
        return;
    }
    // The add button sits in a bottom panel, so the scroll area gets exactly
    // the height that is left. The loot window's `body` uses the same layout
    // and says why.
    egui::Panel::bottom("equipment-foot")
        .resizable(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 4)))
        .show(ui, |ui| {
            if ui
                .small_button("+ row\u{2026}")
                .on_hover_text(format!(
                    "Add a row to set {set} holding the chosen item in the main hand, at weight 1, \
                     read at every patch. The off hand and the ranged slot are chosen on the row."
                ))
                .clicked()
            {
                subject.quests.picker = Some(Picker::new(
                    Target::Item,
                    PickFor::EquipmentItem { set, row: None, slot: 0 },
                ));
            }
        });
    let (mut kept, removed): (Vec<ShownEquipment>, Vec<ShownEquipment>) = subject
        .addons
        .rows_of(set, &subject.session.rows.edits)
        .into_iter()
        .partition(|shown| shown.life != Life::Delete);
    // A row made by changing an item of another is drawn where that row was.
    kept.sort_by_key(|shown| {
        (
            subject.addons.origin(set, shown.row.items),
            shown.in_database.is_none(),
            shown.row.items,
        )
    });
    let read: Vec<&ShownEquipment> = kept
        .iter()
        .filter(|shown| shown.row.read_at(subject.addons.patch))
        .collect();
    let weight: u32 = read.iter().map(|shown| shown.row.probability).sum();
    theme::note(
        ui,
        format!(
            "{} row(s) read at patch {}; a spawn rolls one by probability, out of {weight}.",
            read.len(),
            subject.addons.patch
        ),
    );
    egui::ScrollArea::vertical()
        .max_height(ui.available_height().max(60.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if kept.is_empty() {
                theme::note(ui, "No rows.");
            } else {
                egui::Grid::new(("equipment-rows", set))
                    .num_columns(8)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        column_names(ui);
                        for shown in &kept {
                            let faults = faults(subject, shown);
                            row(ui, subject, shown, &faults);
                            ui.end_row();
                        }
                    });
            }
            if !removed.is_empty() {
                ui.add_space(10.0);
                theme::heading(ui, "Pending Delete");
                for shown in &removed {
                    removed_row(ui, subject, shown);
                }
            }
        });
}

/// The grid's first line: what each cell of a row is. The item cells are
/// named by the hand, with the column on hover; the other three by their
/// column, as every server form names a cell.
fn column_names(ui: &mut egui::Ui) {
    let name = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(text).small().color(theme::INK_FAINT));
    name(ui, "");
    for (word, column) in ["Main Hand", "Off Hand", "Ranged"].into_iter().zip(addon::ITEM_COLUMNS) {
        name(ui, word).on_hover_text(column);
    }
    for word in ["probability", "patch_min", "patch_max", ""] {
        name(ui, word);
    }
    ui.end_row();
}

/// Why the server would empty or skip a row: the loader's own checks, the
/// items it cannot find, the items it would force to 0, and a patch range
/// that leaves the server's patch out.
fn faults(subject: &mut Subject<'_>, shown: &ShownEquipment) -> Vec<String> {
    let mut out = shown.row.check();
    let edits = &subject.session.rows.edits;
    for (slot, item) in shown.row.items.into_iter().enumerate() {
        if item == 0 {
            continue;
        }
        let known = subject.quests.item_known(item, edits);
        match subject.quests.item(item, edits) {
            None if known => out.push(format!(
                "item {item} in the {} is not in item_template; the server forces it to 0",
                addon::SLOT_WORDS[slot]
            )),
            Some(found) if !addon::held_in_a_hand(found.inventory_type) => out.push(format!(
                "{} is {}, which is not held in a hand; the server forces it to 0",
                found.name,
                vale_mangos::schema::value_word(&vale_mangos::item::INVENTORY_TYPES, found.inventory_type)
                    .to_lowercase()
            )),
            _ => {}
        }
    }
    if !shown.row.read_at(subject.addons.patch) {
        out.push(format!(
            "the row is read at patches {} to {}, and the server is at {}",
            shown.row.patch_min, shown.row.patch_max, subject.addons.patch
        ));
    }
    out
}

/// One row, one grid cell each: the state, the three items, the
/// probability, the patches, and the button that removes the row.
fn row(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEquipment, faults: &[String]) {
    let now = subject.now;
    let key = shown.row.key();
    let held = shown.in_database.as_ref().map(Equipment::assignments);
    let write = |subject: &mut Subject<'_>, column: &'static str, value: String| {
        subject
            .addons
            .set_equipment_column(subject.session, &key, held.clone(), column, value, now);
    };
    super::loot::state_dot(ui, shown.life, faults);
    for slot in 0..3 {
        slot_cell(ui, subject, shown, slot);
    }
    let mut probability = shown.row.probability as i64;
    let weighed = ui
        .add_sized(
            [NUMBER, CELL],
            egui::DragValue::new(&mut probability).range(0..=i64::from(u32::MAX)).speed(0.1),
        )
        .on_hover_text(
            "probability: the row's weight when a spawn rolls one row of the set. A set of one \
             row at weight 1 always holds that row.",
        );
    if weighed.changed() && probability != shown.row.probability as i64 {
        write(subject, "probability", probability.to_string());
    }
    for (column, value, word) in [
        ("patch_min", shown.row.patch_min, "lowest"),
        ("patch_max", shown.row.patch_max, "highest"),
    ] {
        let about = format!("{column}: the {word} content patch the row is read in.");
        if let Some(patch) = patch_cell(ui, (column, shown.row.items), value, &about) {
            write(subject, column, patch.to_string());
        }
    }
    let about = "Removes this row from the set. The items themselves are not changed.";
    if crate::ui::widgets::rowform::remove_keep(ui, shown.life, about).is_some() {
        subject
            .addons
            .remove_equipment(subject.session, &key, shown.in_database.is_some(), now);
    }
}

/// A content patch, chosen by its name. Answers the patch chosen, when it
/// differs from `value`.
fn patch_cell(ui: &mut egui::Ui, id: impl egui::AsIdSalt, value: u32, about: &str) -> Option<u32> {
    use vale_mangos::creature::PATCHES;
    let mut chosen = value;
    egui::ComboBox::from_id_salt(id)
        .width(PATCH)
        .selected_text(vale_mangos::schema::value_word(&PATCHES, value))
        .show_ui(ui, |ui| {
            for patch in PATCHES.iter() {
                ui.selectable_value(&mut chosen, patch.value, patch.name);
            }
        })
        .response
        .on_hover_text(about);
    (chosen != value).then_some(chosen)
}

/// A row of the database marked for removal: the button that keeps it, then
/// its items and weight struck through.
fn removed_row(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEquipment) {
    let key = shown.row.key();
    ui.horizontal(|ui| {
        super::loot::state_dot(ui, shown.life, &[]);
        let words: Vec<String> = shown
            .row
            .items
            .into_iter()
            .enumerate()
            .map(|(slot, item)| match item {
                0 => format!("{} none", addon::SLOT_WORDS[slot]),
                item => {
                    let name = subject
                        .quests
                        .item(item, &subject.session.rows.edits)
                        .map_or_else(|| format!("item {item}"), |found| found.name.clone());
                    format!("{} {name}", addon::SLOT_WORDS[slot])
                }
            })
            .collect();
        if crate::ui::widgets::rowform::remove_keep(ui, shown.life, "").is_some() {
            let now = subject.now;
            subject.addons.keep_equipment(subject.session, &key, now);
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "{} \u{b7} probability {}",
                    words.join(" \u{b7} "),
                    shown.row.probability
                ))
                .small()
                .color(theme::INK_FAINT)
                .strikethrough(),
            )
            .truncate(),
        );
    });
}

/// One slot of a row: the item's icon and name, or "none", truncated to the
/// cell; the button that chooses another item; and the one that empties the
/// slot, drawn disabled for an empty slot so the cells keep their widths.
fn slot_cell(ui: &mut egui::Ui, subject: &mut Subject<'_>, shown: &ShownEquipment, slot: usize) {
    let item = shown.row.items[slot];
    let set = shown.row.entry;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.allocate_ui_with_layout(
            egui::vec2(NAME, CELL),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(NAME);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                match item {
                    0 => {
                        ui.label(egui::RichText::new("none").small().color(theme::INK_FAINT));
                    }
                    item => super::loot::item_name(
                        ui,
                        subject.quests,
                        subject.items,
                        subject.assets,
                        subject.thumbnails,
                        subject.asks,
                        &subject.session.rows.edits,
                        item,
                    ),
                }
            },
        );
        if crate::ui::widgets::reference::by_name_button(
            ui,
            &format!(
                "Choose the item held in the {}. The row is keyed by its items, so this \
                 removes the row and creates one with the new item.",
                addon::SLOT_WORDS[slot]
            ),
        ) {
            subject.quests.picker = Some(Picker::new(
                Target::Item,
                PickFor::EquipmentItem { set, row: Some(shown.row.items), slot },
            ));
        }
        let empty = ui
            .add_enabled(item != 0, egui::Button::new("\u{d7}").min_size(egui::vec2(20.0, CELL)))
            .on_hover_text(format!(
                "Empty the {}. A row whose three slots are empty is kept: a spawn that rolls \
                 it holds nothing.",
                addon::SLOT_WORDS[slot]
            ));
        if empty.clicked() {
            let now = subject.now;
            subject.session.status = match subject.addons.replace_item(subject.session, shown, slot, 0, now) {
                Ok(line) => line,
                Err(e) => format!("not changed: {e}"),
            };
        }
    });
}
