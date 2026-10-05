//! The window a loot set is edited in: what a creature drops, what a chest
//! holds, what an item contains or turns into, and the reference sets any of
//! them name.
//!
//! ## One window for every holder
//!
//! A loot row is the same nine columns whichever of the nine tables it is in,
//! so there is one window and three buttons that open it: *Loot* on a
//! selected creature, on a selected game object, and on the open item. What
//! differs is which sets the holder's columns name, and that is a row of tabs:
//! a creature's `loot_id`, `pickpocket_loot_id` and `skinning_loot_id`; an
//! object's `lootId`; an item's own entry and its `disenchant_id`. The window
//! follows the selection, as the quest window does — see
//! [`crate::tools::loot::Window`], which the shell rebuilds each frame.
//!
//! ## A row is drawn as the server reads it
//!
//! The columns are drawn as what they mean rather than as the two signed
//! numbers the table holds: a chance and a `quest` checkbox for
//! `ChanceOrQuestChance`, a count range for `mincountOrRef` on an item and a
//! link for a reference. The rows are grouped as `LootTemplate::AddEntry`
//! groups them, and each group's heading says in a sentence what one roll of
//! it does — `crate::tools::loot::Odds`, which is `LootGroup::Roll`'s
//! arithmetic — because a group is what a person is reasoning about when they
//! change one row's chance. A 0 in a group is drawn as `equal`, which is what
//! the server reads it as. A row the server would skip at load says why, in
//! red, off `vale_mangos::loot::Entry::check`.
//!
//! ## Edits go to the project, not the database
//!
//! An edit goes into the project's store and onto the undo stack; a save
//! writes `sql\loot.sql`; *Apply* is on the bar's *Server…* with the other
//! subjects' — see [`super::sync`] and [`crate::server::loot`].

use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::tools::items::Items;
use crate::tools::loot::{Loot, Set, Shown};
use crate::tools::quests::{PickFor, Picker, Quests, Target};
use vale_client::assets::GameAssets;
use vale_mangos::loot;
use vale_mangos::row::{Key, Life};
use bevy_egui::egui;

/// How big an item's icon is drawn beside its name.
const ICON: f32 = 18.0;

/// How wide the name cell is, icon included. Fixed so that the cells after it
/// start at one x on every row, which is what makes a list of rows read as a
/// table.
const NAME: f32 = 200.0;

/// How wide the chance cell is: room for `0.000001%`, the least chance the
/// server rolls (`LootMgr.cpp:309`), and for `29.78891%`.
const CHANCE: f32 = 90.0;

/// How wide a count or condition cell is. Both hold small integers.
const NUMBER: f32 = 44.0;

/// How wide the group drop-down is: room for `Group 12`.
const GROUP: f32 = 76.0;

/// How tall a cell is.
const CELL: f32 = 18.0;

/// Everything the window needs.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub loot: &'a mut Loot,
    /// The quest tool's state, for two things it already has: the batched
    /// item-name lookups, and the reference picker.
    pub quests: &'a mut Quests,
    /// The item workspace's state, for what a display id's icon is.
    pub items: &'a mut Items,
    pub assets: &'a GameAssets,
    pub thumbnails: &'a mut Thumbnails,
    /// The edit that would make the holder name a set, one per tab, where
    /// there is one: a creature's `loot_id` written as its own entry, an
    /// item's `LOOTABLE` flag set. `None` where nothing is offered.
    pub fixes: Vec<Option<Fix>>,
    pub now: f64,
}

/// One column edit offered from the window, on the holder's own row.
#[derive(Debug, Clone)]
pub struct Fix {
    pub table: &'static str,
    pub key: Key,
    pub column: &'static str,
    pub value: String,
    /// The button's text, and what it says on hover.
    pub label: String,
    pub about: String,
    /// What it goes on the undo stack as.
    pub gesture: &'static str,
}

/// Draws the window, or nothing when it is shut or the selection names nothing.
///
/// Returns its rectangle so the viewport can keep a click inside it from also
/// being a click on the ground behind it.
pub fn window(ctx: &egui::Context, mut subject: Subject<'_>) -> Option<egui::Rect> {
    if !subject.loot.open {
        return None;
    }
    let Some(window) = subject.loot.window.clone() else {
        return None;
    };
    // An item chosen through the picker on the last frame, taken now — see
    // `PickFor::LootItem`.
    if let Some((table, entry, item)) = subject.quests.loot_pick.take() {
        let now = subject.now;
        subject
            .loot
            .add_item(subject.session, Set::new(table, entry), item, now);
    }
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{} \u{2014} loot", window.title))
        // One id whatever the title says, so the window keeps its place when
        // another creature is clicked — see `super::quests::holder_window`.
        .id(egui::Id::new("loot-window"))
        .open(&mut keep_open)
        // Wide enough for every cell of a row at the widths above, and tall
        // enough for the head, the tabs and about eight rows of 33 points.
        // A longer set scrolls; dragging the corner shows more of it.
        .default_size([680.0, 460.0])
        .default_pos([300.0, 120.0])
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(theme::SHELL)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(&window.about)
                    .small()
                    .color(theme::WARN),
            );
            // No sets is a holder with nothing to show, and `about` has
            // already said why: the template row is still being read, or the
            // type is one the server takes no loot from.
            if window.sets.is_empty() {
                return;
            }
            tabs(ui, &mut subject, &window);
            let Some(set) = subject.loot.showing() else {
                return;
            };
            ui.add_space(4.0);
            match subject.loot.reference {
                Some(reference) => reference_head(ui, &mut subject, reference),
                None => {
                    let tab = subject.loot.tab.min(window.sets.len() - 1);
                    let fix = subject.fixes.get(tab).cloned().flatten();
                    if !set_head(ui, &mut subject, set, fix) {
                        return;
                    }
                }
            }
            body(ui, &mut subject, set);
        });
    if !keep_open {
        subject.loot.open = false;
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
    shown.map(|shown| shown.response.rect)
}

/// The row of tabs, one per set the holder's columns name. A single set is
/// drawn as its heading rather than as a switch with one position.
fn tabs(ui: &mut egui::Ui, subject: &mut Subject<'_>, window: &crate::tools::loot::Window) {
    let labels: Vec<String> = window
        .sets
        .iter()
        .map(|set| match set.entry {
            0 => format!("{} \u{b7} none", set.table().word),
            entry => format!("{} \u{b7} {entry}", set.table().word),
        })
        .collect();
    if labels.len() == 1 {
        theme::heading(ui, &labels[0]);
        return;
    }
    let options: Vec<(&str, usize)> = labels
        .iter()
        .enumerate()
        .map(|(at, label)| (label.as_str(), at))
        .collect();
    let mut tab = subject.loot.tab.min(labels.len() - 1);
    theme::segmented(ui, &mut tab, &options, |a, b| a == b);
    if tab != subject.loot.tab {
        subject.loot.tab = tab;
        subject.loot.reference = None;
    }
}

/// Draws what the tab's set is keyed to, and the two things that can be wrong
/// with it: the column names nothing, or the set has no rows. `false` when
/// there is no set to draw.
fn set_head(ui: &mut egui::Ui, subject: &mut Subject<'_>, set: Set, fix: Option<Fix>) -> bool {
    let table = set.table();
    if set.entry == 0 {
        ui.label(
            egui::RichText::new(format!(
                "{} is 0, so the server takes nothing from this table for it.",
                table.keyed_by
            ))
            .small()
            .color(theme::INK_DIM),
        );
        offer(ui, subject, fix);
        return false;
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {}", set.table, set.entry))
                .small()
                .color(theme::INK_DIM),
        )
        .on_hover_text(format!("Keyed by {}.", table.keyed_by));
    });
    offer(ui, subject, fix);
    true
}

/// The head of a followed reference set: which it is, and the way back.
fn reference_head(ui: &mut egui::Ui, subject: &mut Subject<'_>, reference: u32) {
    ui.horizontal(|ui| {
        // A button with a word on it: egui's bundled fonts carry no arrow, and
        // a missing glyph draws as a hollow box.
        if ui
            .small_button("Back")
            .on_hover_text("Back to the selection's own set.")
            .clicked()
        {
            subject.loot.back();
        }
        ui.label(
            egui::RichText::new(format!("{} {reference}", loot::REFERENCE))
                .small()
                .color(theme::INK_DIM),
        )
        .on_hover_text(
            "A set shared between loot tables: any row whose mincountOrRef is this entry \
             negated rolls it. Editing it changes every set that names it.",
        );
    });
}

/// The button that writes the holder's own column, when one is offered.
fn offer(ui: &mut egui::Ui, subject: &mut Subject<'_>, fix: Option<Fix>) {
    let Some(fix) = fix else { return };
    if ui.small_button(&fix.label).on_hover_text(&fix.about).clicked() {
        let subject_line = format!("{} {} {}", fix.table, fix.key.text(), fix.column);
        subject.session.set_server_edit(
            fix.table,
            &fix.key,
            fix.column,
            Some(fix.value.clone()),
            Some(crate::session::Gesture {
                label: fix.gesture,
                subject: &subject_line,
                now: subject.now,
            }),
        );
    }
}

/// The rows, by group, and the two ways to add one.
fn body(ui: &mut egui::Ui, subject: &mut Subject<'_>, set: Set) {
    if let Some(trouble) = subject.loot.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        return;
    }
    if !subject.loot.is_read(set) {
        theme::waiting(ui, format!("reading {} {}\u{2026}", set.table, set.entry));
        return;
    }
    let rows = subject.loot.rows_of(set, &subject.session.server_edits);
    // The two ways to add a row stay under the list, outside the scroll, so a
    // set of a hundred rows does not have to be scrolled to the end to be
    // added to.
    //
    // They sit in a bottom panel so that the scroll gets exactly the height
    // that is left. Until a window is dragged, egui's `Resize` makes it
    // `max(its size, its content)` (`resize.rs:269`). When the scroll was
    // given the available height less an estimate of the foot's height, the
    // foot was taller than the estimate, so the content was always a little
    // taller than the window and the window grew on every frame until it
    // filled the screen. A panel measures the foot itself, so the scroll is
    // never taller than the window, and the window stays at `default_size`
    // (about eight rows) until it is dragged.
    egui::Panel::bottom("loot-foot")
        .resizable(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 4)))
        .show(ui, |ui| adders(ui, subject, set));
    egui::ScrollArea::vertical()
        .max_height(ui.available_height().max(60.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if rows.is_empty() {
                theme::note(
                    ui,
                    "No rows. The server logs a loot id with no rows at start and takes \
                     nothing from it.",
                );
            }
            // The groups this set has, for the drop-down every row carries.
            let mut groups: Vec<u32> = rows
                .iter()
                .filter(|shown| shown.life != Life::Delete)
                .map(|shown| shown.entry.group)
                .collect();
            groups.sort_unstable();
            groups.dedup();
            let mut group: Option<u32> = None;
            let mut group_odds = crate::tools::loot::odds(&rows, 0);
            for shown in &rows {
                if group != Some(shown.entry.group) {
                    group = Some(shown.entry.group);
                    group_odds = crate::tools::loot::odds(&rows, shown.entry.group);
                    group_heading(ui, shown.entry.group, group_odds);
                }
                row(ui, subject, set, shown, &groups, group_odds);
            }
        });
}

/// Formats a chance for display: at most six decimal places, which is the
/// server's own floor (`LootMgr.cpp:309`), with trailing zeros left off. A sum
/// or a share of f32 chances carries the rounding noise of each term, and this
/// function drops it.
fn percent(value: f32) -> String {
    let text = format!("{value:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Draws a group's heading, and one line under it that says what the group
/// does.
///
/// Group 0 is not a group: every row in it rolls its own chance and any number
/// of them can drop. A numbered group drops at most one row per roll — see
/// [`crate::tools::loot::Odds`], which is the server's own arithmetic.
fn group_heading(ui: &mut egui::Ui, group: u32, odds: crate::tools::loot::Odds) {
    let (title, meaning) = match group {
        0 => (
            "Rolls alone".to_string(),
            "Each row rolls its own chance, so any number of these can drop together."
                .to_string(),
        ),
        n => {
            let title = format!("Group {n} \u{2014} one of these per roll");
            let meaning = match (odds.stated > 100.0, odds.stated > 0.0, odds.equal) {
                (true, _, _) => format!(
                    "The stated chances sum to {}%, over 100: a row whose chance starts past \
                     100 is never reached, and the server logs the group at start.",
                    percent(odds.stated)
                ),
                (false, false, 0) => "No row has a chance, so nothing drops.".to_string(),
                (false, true, 0) => format!(
                    "The rows with a chance are tried in turn; {}% of rolls drop nothing.",
                    percent(odds.nothing)
                ),
                (false, false, equal) => format!(
                    "Every row is equal, so exactly one of the {equal} drops each roll: {}% each.",
                    percent(odds.share)
                ),
                (false, true, equal) => format!(
                    "The rows with a chance are tried first ({}% together); otherwise one of \
                     the {equal} equal rows drops, {}% each.",
                    percent(odds.stated),
                    percent(odds.share)
                ),
            };
            (title, meaning)
        }
    };
    theme::heading(ui, &title);
    theme::note(ui, meaning);
}

/// One row: the item or the reference, its chance, its count, its group, its
/// condition, and the button that removes it or keeps it.
fn row(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    set: Set,
    shown: &Shown,
    groups: &[u32],
    odds: crate::tools::loot::Odds,
) {
    let faults = shown.entry.check();
    let now = subject.now;
    let grouped = shown.entry.group != 0;
    ui.horizontal(|ui| {
        // The name cell: fixed width, left-aligned, whatever it holds. Its
        // first eight points are the row's state — a dot for a row this
        // project creates or removes, or one the server would skip — so the
        // state has a place whatever the window's width, and the names still
        // start at one x.
        ui.allocate_ui_with_layout(
            egui::vec2(NAME, CELL),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(NAME);
                ui.set_max_width(NAME);
                state_dot(ui, shown.life, &faults);
                match shown.entry.reference_to() {
                    Some(reference) => reference_link(ui, subject, reference),
                    None => item_name(
                        ui,
                        subject.quests,
                        subject.items,
                        subject.assets,
                        subject.thumbnails,
                        &subject.session.server_edits,
                        shown.entry.item,
                    ),
                }
            },
        );
        // The chance, and whether it is a quest drop.
        // The chance is written as the shortest decimal that reads back as the
        // same f32, with no cap on the decimal places. `ChanceOrQuestChance`
        // is a MySQL `float` and shipped rows go as low as 0.0001. A fixed cap
        // of four places drew those as 0%, and a higher cap draws a
        // single-precision value through f64 as `29.78890038` where the table
        // holds `29.7889`. Rust's `Display` for f32 is that shortest
        // round-trip form, and `vale_mangos::sql::float` writes the same form,
        // so the cell shows what the statement will say.
        //
        // A 0 in a group is drawn as `equal`, because the server reads it as
        // an equal share of what the group's stated chances leave. The share
        // itself is on hover and in the group's heading. Typing `equal` or 0
        // writes 0; typing a number gives the row a chance of its own.
        let mut chance = shown.entry.chance;
        let about = match (grouped, shown.entry.chance == 0.0) {
            (true, true) => format!(
                "ChanceOrQuestChance 0, which in a group means an equal share of what the \
                 rows with a chance leave: {}% per roll here. Type a number to give this \
                 row a chance of its own; it is then tried before the equal rows.",
                percent(odds.share)
            ),
            (true, false) => "ChanceOrQuestChance: this row's chance per roll of the group. \
                              The rows with a chance are tried in turn before the equal ones; \
                              0 makes it one of those."
                .to_string(),
            (false, _) => "ChanceOrQuestChance: the chance this row drops, rolled on its own."
                .to_string(),
        };
        let dragged = ui
            .add_sized(
                [CHANCE, CELL],
                egui::DragValue::new(&mut chance)
                    .range(0.0..=100.0)
                    .speed(0.5)
                    .custom_formatter(move |value, _| match (grouped, value == 0.0) {
                        (true, true) => "equal".to_string(),
                        _ => format!("{}%", value as f32),
                    })
                    .custom_parser(|text| {
                        let text = text.trim();
                        match text.eq_ignore_ascii_case("equal") {
                            true => Some(0.0),
                            false => text.trim_end_matches('%').trim().parse::<f64>().ok(),
                        }
                    }),
            )
            .on_hover_text(about);
        let mut quest = shown.entry.quest;
        let switched = ui
            .checkbox(&mut quest, egui::RichText::new("quest").small())
            .on_hover_text(
                "Offered only to a player on a quest that asks for the item: the chance \
                 stored negative. On a reference the server reads it as an ordinary \
                 chance and says so at load.",
            )
            .changed();
        if (dragged.changed() && chance != shown.entry.chance) || switched {
            let signed = match quest {
                true => -chance,
                false => chance,
            };
            subject.loot.set_column(
                subject.session,
                set,
                shown,
                "ChanceOrQuestChance",
                vale_mangos::sql::float(signed),
                now,
            );
        }
        // The count, for an item. On a reference the first cell is drawn
        // greyed with the set negated, which is what the column holds, and
        // the second is how many times the set is rolled
        // (`LootMgr.cpp:1268`), so the columns after them keep their place.
        let is_item = shown.entry.reference_to().is_none();
        let mut min = shown.entry.min_or_ref as i64;
        let mut max = shown.entry.max as i64;
        let low = ui
            .add_enabled_ui(is_item, |ui| {
                // The range only on an item: a `DragValue` clamps what it
                // shows to its range, and a reference's value is negative.
                let cell = egui::DragValue::new(&mut min);
                let cell = match is_item {
                    true => cell.range(1..=loot::MAX_COUNT as i64),
                    false => cell,
                };
                ui.add_sized([NUMBER, CELL], cell)
            })
            .inner
            .on_hover_text(match is_item {
                true => "mincountOrRef: the least that drops.".to_string(),
                false => format!(
                    "mincountOrRef: {} names {} {}.",
                    shown.entry.min_or_ref,
                    loot::REFERENCE,
                    shown.entry.min_or_ref.unsigned_abs()
                ),
            });
        ui.label(egui::RichText::new("\u{2013}").small().color(theme::INK_FAINT));
        let high = ui
            .add_sized(
                [NUMBER, CELL],
                egui::DragValue::new(&mut max)
                    .range(1..=loot::MAX_COUNT as i64)
                    .prefix(match is_item {
                        true => "",
                        false => "\u{d7}",
                    }),
            )
            .on_hover_text(match is_item {
                true => "maxcount: the most that drops.",
                false => "maxcount on a reference: how many times the set is rolled for \
                          one loot.",
            });
        if is_item && low.changed() && min != shown.entry.min_or_ref as i64 {
            subject
                .loot
                .set_column(subject.session, set, shown, "mincountOrRef", min.to_string(), now);
        }
        if high.changed() && max != shown.entry.max as i64 {
            subject
                .loot
                .set_column(subject.session, set, shown, "maxcount", max.to_string(), now);
        }
        // The group is chosen from a list. It is part of the row's key, so a
        // change removes the row and creates it again under the new key. A
        // list gives one discrete choice; a drag changes the value on every
        // frame it moves, and each change created another row.
        let mut chosen: Option<u32> = None;
        let next = groups.iter().copied().max().unwrap_or(0) + 1;
        let word = |group: u32| match group {
            0 => "Alone".to_string(),
            n => format!("Group {n}"),
        };
        ui.add_enabled_ui(shown.life != Life::Delete, |ui| {
            egui::ComboBox::from_id_salt(("loot-group", shown.entry.key().text()))
                .selected_text(egui::RichText::new(word(shown.entry.group)).small())
                .width(GROUP)
                .show_ui(ui, |ui| {
                    let mut options: Vec<u32> = vec![0];
                    options.extend(groups.iter().copied().filter(|group| *group != 0));
                    for group in options {
                        if ui
                            .selectable_label(group == shown.entry.group, word(group))
                            .clicked()
                        {
                            chosen = Some(group);
                        }
                    }
                    if next <= loot::MAX_GROUP
                        && ui
                            .selectable_label(false, format!("New group ({next})"))
                            .clicked()
                    {
                        chosen = Some(next);
                    }
                })
                .response
                .on_hover_text(
                    "groupid. Alone rolls this row's own chance; in a group, one row of the \
                     group drops per roll. It is part of the row's key, so a change removes \
                     the row and creates it again in the new group, as one undo entry.",
                )
                .on_disabled_hover_text("Keep the row before moving it.");
        });
        if let Some(group) = chosen.filter(|group| *group != shown.entry.group) {
            subject.loot.regroup(subject.session, set, shown, group, now);
        }
        // The condition, as its number.
        let mut condition = shown.entry.condition as i64;
        let conditioned = ui
            .add_sized(
                [NUMBER, CELL],
                egui::DragValue::new(&mut condition).range(0..=i64::from(u32::MAX)).prefix("c"),
            )
            .on_hover_text("condition_id: a row of `conditions` the player must meet, or 0.");
        if conditioned.changed() && condition != shown.entry.condition as i64 {
            subject
                .loot
                .set_column(subject.session, set, shown, "condition_id", condition.to_string(), now);
        }
        if shown.entry.condition != 0 {
            super::conditions::open_button(ui, shown.entry.condition);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (label, about) = match shown.life {
                Life::Delete => ("keep", "Take the removal mark off this row."),
                _ => ("\u{d7}", "Remove this row. The item itself is not touched."),
            };
            if ui.small_button(label).on_hover_text(about).clicked() {
                match shown.life {
                    Life::Delete => subject.loot.keep(subject.session, set, shown, now),
                    _ => subject.loot.remove(subject.session, set, shown, now),
                }
            }
        });
    });
}

/// Draws the row's state as a dot: amber for a row this project creates, red
/// for one it removes or one the server would skip at load, and nothing — the
/// same space, unpainted — for an ordinary row. What it means is on hover.
pub(super) fn state_dot(ui: &mut egui::Ui, life: Life, faults: &[String]) {
    const DOT: f32 = 8.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(DOT, CELL), egui::Sense::hover());
    let (colour, about) = match (life, faults.is_empty()) {
        (Life::Insert, true) => (theme::WARN, "New: this row is in no database yet. Apply writes it.".to_string()),
        (Life::Delete, _) => (theme::BAD, "Marked for removal. Apply deletes it; keep takes the mark off.".to_string()),
        (Life::Insert, false) => (
            theme::BAD,
            format!(
                "New, and the server would skip it at load: {}. It is not written until \
                 that is fixed.",
                faults.join("; ")
            ),
        ),
        (Life::Update, false) => (
            theme::BAD,
            format!("The server would skip this row at load: {}.", faults.join("; ")),
        ),
        (Life::Update, true) => return,
    };
    ui.painter().circle_filled(rect.center(), DOT / 2.0 - 1.0, colour);
    response.on_hover_text(about);
}

/// An item's icon and name, coloured by quality, opening the item workspace.
/// The loot, vendor and trainer windows draw an item this way.
pub(super) fn item_name(
    ui: &mut egui::Ui,
    quests: &mut Quests,
    items: &mut Items,
    assets: &GameAssets,
    thumbnails: &mut Thumbnails,
    edits: &vale_mangos::row::Edits,
    entry: u32,
) {
    let Some(found) = quests.item(entry, edits) else {
        let text = match quests.item_known(entry, edits) {
            true => format!("item {entry} \u{2014} not in item_template"),
            false => format!("item {entry}\u{2026}"),
        };
        let colour = match quests.item_known(entry, edits) {
            true => theme::BAD,
            false => theme::INK_DIM,
        };
        ui.add(egui::Label::new(egui::RichText::new(text).color(colour)).truncate());
        return;
    };
    let icon = items.look(assets, found.display_id, 0).and_then(|look| look.icon);
    let (rect, picture) = ui.allocate_exact_size(egui::Vec2::splat(ICON), egui::Sense::click());
    ui.painter().rect_filled(rect, 2.0, theme::SUNK);
    if let Some(path) = icon {
        thumbnails.want(&path);
        if let Some(texture) = thumbnails.get(&path) {
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }
    let colour = super::items::quality_colour(found.quality);
    let name = ui.add(
        egui::Label::new(egui::RichText::new(&found.name).color(colour))
            .truncate()
            .sense(egui::Sense::click()),
    );
    if (picture | name)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("{} ({entry}). Open it in the item workspace.", found.name))
        .clicked()
    {
        quests.show_item = Some(entry);
    }
}

/// A reference row's link into the set it names.
fn reference_link(ui: &mut egui::Ui, subject: &mut Subject<'_>, reference: u32) {
    let link = ui.add(egui::Link::new(
        egui::RichText::new(format!("reference {reference}")).color(theme::ACCENT),
    ));
    if link
        .on_hover_text(format!(
            "Rolls {} {reference}, a set shared with every other row that names it. Open it.",
            loot::REFERENCE
        ))
        .clicked()
    {
        subject.loot.follow(reference);
    }
}

/// Draws the two ways to add a row: an item through the picker, and a reference
/// by its number, since a reference set has no name.
fn adders(ui: &mut egui::Ui, subject: &mut Subject<'_>, set: Set) {
    ui.horizontal(|ui| {
        if ui
            .small_button("+ item\u{2026}")
            .on_hover_text(format!(
                "Add a row to {} {}: one of the item, at 100%, in no group.",
                set.table, set.entry
            ))
            .clicked()
        {
            subject.quests.picker = Some(Picker::new(
                Target::Item,
                PickFor::LootItem {
                    table: set.table,
                    entry: set.entry,
                },
            ));
        }
        ui.add_space(8.0);
        ui.add_sized(
            [72.0, 18.0],
            egui::TextEdit::singleline(&mut subject.loot.reference_box).hint_text("reference"),
        );
        let wanted: Option<u32> = subject.loot.reference_box.trim().parse().ok();
        if ui
            .add_enabled(wanted.is_some_and(|n| n > 0), egui::Button::new("+ reference").small())
            .on_hover_text(format!(
                "Add a row that rolls {} <n>, at 100%, in no group.",
                loot::REFERENCE
            ))
            .on_disabled_hover_text("Type a reference_loot_template entry.")
            .clicked()
        {
            if let Some(reference) = wanted {
                let now = subject.now;
                subject
                    .loot
                    .add_reference(subject.session, set, reference, now);
                subject.loot.reference_box.clear();
            }
        }
    });
}
