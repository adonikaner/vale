//! The Vendor and Trainer windows of a selected creature: what it sells and
//! what it teaches.
//!
//! ## Two lists per window
//!
//! Each window has two tabs: the creature's own list (`npc_vendor` or
//! `npc_trainer` under its entry) and the template list its `vendor_id` or
//! `trainer_id` names, which every creature naming that id shares. The
//! windows follow the selection, as the loot window does; the shell writes
//! [`crate::tools::services::About`] from the selection each frame.
//!
//! ## What a row shows
//!
//! A vendor row is its item, its place in the list, its stock (a limit and
//! the seconds between restocks, both or neither), its two restock flags and
//! its condition. A trainer row is the ability its teaching spell teaches,
//! named out of `Spell.dbc`, the level it needs (0 draws as the spell's own
//! level, which is what the loader reads it as), its price and the skill it
//! needs. Trainer rows are drawn by level, as the training window orders
//! them. A row the server would skip at load has a red dot, with every reason
//! on hover.
//!
//! ## What the header checks
//!
//! The client offers a vendor or a trainer window only for a creature whose
//! `npc_flags` carries `VENDOR` or `TRAINER`, and the trainer loader skips
//! every row of a creature's own list without `TRAINER`. The header says so
//! and offers to set the bit. The Trainer window also says who the trainer
//! trains, from `trainer_type`, `trainer_class`, `trainer_race` and
//! `trainer_spell` (`vale_mangos::trainer::who_words`).
//!
//! ## Edits go to the project, not the database
//!
//! An edit goes into the project's store and onto the undo stack; a save
//! writes `sql\services.sql`; Apply is on the bar's Server… with the other
//! subjects'. See [`crate::server::services`].

use super::theme;
use super::thumbnails::Thumbnails;
use crate::session::EditSession;
use crate::tools::items::Items;
use crate::tools::quests::{PickFor, Picker, Quests, Target};
use crate::tools::services::{self, About, Kind, List, Services, ShownLesson, ShownWare};
use vale_client::assets::GameAssets;
use vale_mangos::row::Life;
use vale_mangos::schema::{money_words, seconds_words};
use vale_mangos::trainer::{self, Taught};
use vale_mangos::vendor::{self, Ware};
use bevy_egui::egui;
use std::collections::HashSet;

/// How wide the name cell is, state dot and icon included. Fixed so the cells
/// after it start at one x on every row.
const NAME: f32 = 210.0;

/// How wide a small number is drawn: a slot, a count, a level, a condition.
const NUMBER: f32 = 52.0;

/// How wide a skill line's name and id is drawn. It holds
/// `Leatherworking (165)`.
const SKILL: f32 = 150.0;

/// How wide a duration is drawn. It holds `1h 30m`.
const TIME: f32 = 70.0;

/// How wide a price is drawn. It holds `12g 50s 3c`.
const MONEY: f32 = 90.0;

/// How wide the line saying what a condition tests is drawn. A longer line is
/// cut to this width, and the whole line is on hover.
const CONDITION: f32 = 130.0;

/// How tall a cell is.
const CELL: f32 = 18.0;

/// Everything the two windows need.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub services: &'a mut Services,
    /// The quest tool's state, for the batched item names and the reference
    /// picker.
    pub quests: &'a mut Quests,
    /// The item workspace's state, for what an item's icon is.
    pub items: &'a mut Items,
    pub assets: &'a GameAssets,
    pub thumbnails: &'a mut Thumbnails,
    pub now: f64,
}

/// The two windows, each drawn when it is open and a creature is selected.
/// Returns their rectangles, so the viewport can keep a click inside one from
/// also being a click on the world behind it.
pub fn windows(ctx: &egui::Context, mut subject: Subject<'_>) -> Vec<egui::Rect> {
    // An item or a spell chosen through the picker on the last frame, taken
    // now; see `PickFor::VendorItem`.
    if let Some((table, entry, id)) = subject.quests.service_pick.take() {
        add_chosen(&mut subject, table, entry, id);
    }
    let mut out = Vec::new();
    if let Some(about) = subject.services.about.clone() {
        for kind in [Kind::Vendor, Kind::Trainer] {
            if subject.services.is_open(kind) {
                out.extend(window(ctx, &mut subject, &about, kind));
            }
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

/// Add what the picker answered to the list it was opened for, and put what
/// happened on the status line. A spell that is not a teaching spell is
/// replaced by the one that teaches it.
fn add_chosen(subject: &mut Subject<'_>, table: &'static str, entry: u32, id: u32) {
    let Some(about) = subject.services.about.clone() else {
        return;
    };
    let list = List::new(table, entry);
    let kind = list.kind();
    let other = about.lists(kind).into_iter().find(|other| *other != list);
    let now = subject.now;
    let done = match kind {
        Kind::Vendor => subject.services.add_item(subject.session, list, other, id, now),
        Kind::Trainer => {
            if !open_spell_tables(subject) {
                subject.session.status = "Spell.dbc or Talent.dbc is not in the archives".to_string();
                return;
            }
            let talents = subject.services.talents.clone().unwrap_or_default();
            let Some(spells) = subject.session.table("Spell") else {
                return;
            };
            let (spell, replaced) = services::resolve_teaching(spells, &talents, id);
            let facts = services::taught(spells, &talents, spell);
            let done = subject.services.add_spell(subject.session, list, other, spell, facts, now);
            match (done, replaced) {
                (Ok(line), Some(chosen)) => Ok(format!(
                    "{line}: spell {chosen} is not a teaching spell, and {spell} teaches it"
                )),
                (done, _) => done,
            }
        }
    };
    subject.session.status = match done {
        Ok(line) => line,
        Err(e) => format!("not added: {e}"),
    };
}

/// Open `Spell.dbc`, `Talent.dbc` and `TalentTab.dbc`, and read the talent
/// spells once. `false` when a table is not in the archives.
fn open_spell_tables(subject: &mut Subject<'_>) -> bool {
    let opened = super::quests::open_for_names(subject.session, subject.assets, "Spell")
        && subject.session.open_table(subject.assets, "Talent")
        && subject.session.open_table(subject.assets, "TalentTab");
    if opened && subject.services.talents.is_none() {
        subject.services.talents = match (subject.session.table("Talent"), subject.session.table("TalentTab")) {
            (Some(talent), Some(tab)) => services::talent_spells(talent, tab),
            _ => None,
        };
    }
    opened
}

/// One window.
fn window(ctx: &egui::Context, subject: &mut Subject<'_>, about: &About, kind: Kind) -> Option<egui::Rect> {
    // A vendor row is wider than a trainer row by the width of its condition
    // cell.
    let (word, id, pos, width) = match kind {
        Kind::Vendor => ("vendor", "vendor-window", [320.0, 140.0], 900.0),
        Kind::Trainer => ("trainer", "trainer-window", [340.0, 160.0], 700.0),
    };
    let mut keep_open = true;
    let shown = egui::Window::new(format!("{} \u{2014} {word}", about.label))
        // One id whatever the title says, so the window keeps its place when
        // another creature is clicked.
        .id(egui::Id::new(id))
        .open(&mut keep_open)
        // Wide enough for every cell of a row, and tall enough for the head,
        // the tabs and about eight rows. A longer list scrolls.
        .default_size([width, 460.0])
        .default_pos(pos)
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
                egui::RichText::new(format!(
                    "creature_template entry {}: every spawn of it {} these.",
                    about.entry,
                    match kind {
                        Kind::Vendor => "sells",
                        Kind::Trainer => "teaches",
                    }
                ))
                .small()
                .color(theme::WARN),
            );
            flag_offer(ui, subject, about, kind);
            if kind == Kind::Trainer {
                who(ui, subject, about);
            }
            tabs(ui, subject, about, kind);
            let Some(list) = subject.services.showing(kind) else {
                return;
            };
            ui.add_space(4.0);
            if !list_head(ui, subject, about, kind, list) {
                return;
            }
            body(ui, subject, about, kind, list);
        });
    if !keep_open {
        subject.services.toggle(kind);
    }
    shown.map(|shown| shown.response.rect)
}

/// The `npc_flags` bit the client needs to offer the window, and the button
/// that sets it when it is missing.
fn flag_offer(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, kind: Kind) {
    if about.has_flag(kind) {
        return;
    }
    let (bit, name) = kind.flag();
    let sentence = match kind {
        Kind::Vendor => "npc_flags has no VENDOR bit, so the client offers no vendor window for it.",
        Kind::Trainer => {
            "npc_flags has no TRAINER bit, so the client offers no training window for it and \
             the server skips every row of its own list at load."
        }
    };
    ui.label(egui::RichText::new(sentence).small().color(theme::BAD));
    if ui
        .small_button(format!("Set {name}"))
        .on_hover_text(format!(
            "Write npc_flags with {bit:#x} added into creature_template. It is a creature edit: \
             it is applied from the Creatures block of the Server panel and needs the server \
             restarted."
        ))
        .clicked()
    {
        let line = format!("creature {} npc_flags", about.entry);
        subject.session.set_server_edit(
            vale_mangos::creature::TEMPLATE,
            &about.template_key,
            "npc_flags",
            Some((about.npc_flags | bit).to_string()),
            Some(crate::session::Gesture {
                label: "Edit creature",
                subject: &line,
                now: subject.now,
            }),
        );
    }
}

/// Who a trainer trains, from its template's four trainer columns.
fn who(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About) {
    let spell = match about.trainer_spell {
        0 => String::new(),
        id => spell_name(subject, id).unwrap_or_else(|| format!("spell {id}")),
    };
    let sentence = trainer::who_words(about.trainer_type, about.trainer_class, about.trainer_race, &spell);
    ui.label(egui::RichText::new(sentence).small().color(theme::INK_DIM)).on_hover_text(
        "creature_template.trainer_type, trainer_class, trainer_race and trainer_spell, which \
         are edited in the template window.",
    );
}

/// The two tabs: the creature's own list and its template's.
fn tabs(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, kind: Kind) {
    let [own, template] = about.lists(kind);
    let labels = [
        format!("Own list \u{b7} {}", own.entry),
        match template.entry {
            0 => format!("Template \u{b7} {} none", kind.template_column()),
            entry => format!("Template \u{b7} {} {entry}", kind.template_column()),
        },
    ];
    let options: Vec<(&str, usize)> = labels.iter().enumerate().map(|(at, label)| (label.as_str(), at)).collect();
    let showing = subject.services.tab(kind);
    let mut chosen = showing;
    theme::segmented(ui, &mut chosen, &options, |a, b| a == b);
    if chosen != showing {
        subject.services.set_tab(kind, chosen);
    }
}

/// What the tab's list is keyed to, and who shares it. `false` when there is
/// no list to draw.
fn list_head(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, kind: Kind, list: List) -> bool {
    let keyed_by = match kind {
        Kind::Vendor => vendor::keyed_by(list.table),
        Kind::Trainer => trainer::keyed_by(list.table),
    };
    if list.entry == 0 {
        theme::note(
            ui,
            format!(
                "{keyed_by} is 0, so the creature {} from its own list only. Type a {} into the \
                 template window to share a list with other creatures.",
                match kind {
                    Kind::Vendor => "sells",
                    Kind::Trainer => "teaches",
                },
                kind.template_column()
            ),
        );
        return false;
    }
    ui.label(
        egui::RichText::new(format!("{} {}", list.table, list.entry))
            .small()
            .color(theme::INK_DIM),
    )
    .on_hover_text(format!("Keyed by {keyed_by}."));
    if list.is_template() {
        if let Some(users) = subject.services.users(list, &subject.session.server_edits) {
            let others: Vec<&(u32, String)> = users.iter().filter(|(entry, _)| *entry != about.entry).collect();
            match others.len() {
                0 => {
                    theme::note(ui, "Only this creature names the list.");
                }
                n => {
                    let names: Vec<String> = others
                        .iter()
                        .take(4)
                        .map(|(entry, name)| format!("{name} ({entry})"))
                        .collect();
                    let more = match n > 4 {
                        true => format!(" and {} more", n - 4),
                        false => String::new(),
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "{n} other creature(s) name this list, and an edit changes every one of \
                             them: {}{more}.",
                            names.join(", ")
                        ))
                        .small()
                        .color(theme::WARN),
                    );
                }
            }
        }
    }
    true
}

/// The rows, and the way to add one.
fn body(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, kind: Kind, list: List) {
    if let Some(trouble) = subject.services.trouble.clone() {
        ui.label(egui::RichText::new(trouble).small().color(theme::BAD));
        return;
    }
    let [own, template] = about.lists(kind);
    if !subject.services.is_read(own) || !subject.services.is_read(template) {
        theme::waiting(ui, format!("reading {} {}\u{2026}", list.table, list.entry));
        return;
    }
    // The add button sits in a bottom panel, so the scroll area gets exactly
    // the height that is left. The loot window's `body` uses the same layout
    // and says why.
    egui::Panel::bottom(match kind {
        Kind::Vendor => "vendor-foot",
        Kind::Trainer => "trainer-foot",
    })
    .resizable(false)
    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 4)))
    .show(ui, |ui| adders(ui, subject, kind, list));
    match kind {
        Kind::Vendor => vendor_rows(ui, subject, about, own, template, list),
        Kind::Trainer => trainer_rows(ui, subject, about, own, template, list),
    }
}

/// The button that opens the picker on the list.
fn adders(ui: &mut egui::Ui, subject: &mut Subject<'_>, kind: Kind, list: List) {
    // `--vendor-find` and `--trainer-find` open the picker this button opens,
    // with the text already typed.
    if let Some((wanted, text)) = subject.services.find.clone() {
        if wanted == kind {
            subject.services.find = None;
            let mut picker = Picker::new(
                match kind {
                    Kind::Vendor => Target::Item,
                    Kind::Trainer => Target::Dbc("Spell"),
                },
                match kind {
                    Kind::Vendor => PickFor::VendorItem {
                        table: list.table,
                        entry: list.entry,
                    },
                    Kind::Trainer => PickFor::TrainerSpell {
                        table: list.table,
                        entry: list.entry,
                    },
                },
            );
            picker.query = text;
            subject.quests.picker = Some(picker);
        }
    }
    ui.horizontal(|ui| match kind {
        Kind::Vendor => {
            if ui
                .small_button("+ item\u{2026}")
                .on_hover_text(format!(
                    "Add an item to {} {}, after every item it has, with no limit on its stock.",
                    list.table, list.entry
                ))
                .clicked()
            {
                subject.quests.picker = Some(Picker::new(
                    Target::Item,
                    PickFor::VendorItem {
                        table: list.table,
                        entry: list.entry,
                    },
                ));
            }
        }
        Kind::Trainer => {
            if ui
                .small_button("+ spell\u{2026}")
                .on_hover_text(format!(
                    "Add a spell to {} {}, free, at the spell's own level. Choosing the ability \
                     itself adds the spell that teaches it.",
                    list.table, list.entry
                ))
                .clicked()
            {
                subject.quests.picker = Some(Picker::new(
                    Target::Dbc("Spell"),
                    PickFor::TrainerSpell {
                        table: list.table,
                        entry: list.entry,
                    },
                ));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// The vendor rows
// ---------------------------------------------------------------------------

/// Every row of a vendor list, with each row's reasons the server would skip
/// it.
fn vendor_rows(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, own: List, template: List, list: List) {
    let edits = &subject.session.server_edits;
    let rows = subject.services.wares_of(list, edits);
    let kept = |rows: &[ShownWare]| rows.iter().filter(|s| s.life != Life::Delete && !s.forbidden).count();
    let template_rows = subject.services.wares_of(template, edits);
    let in_template: HashSet<u32> = template_rows
        .iter()
        .filter(|s| s.life != Life::Delete && !s.forbidden)
        .map(|s| s.ware.item)
        .collect();
    // The loader counts the template list and then the own list, and skips a
    // row once the two hold 255.
    let before = match list == own {
        true => kept(&template_rows),
        false => 0,
    };
    let count = kept(&rows);
    theme::note(
        ui,
        match (list == own, template.entry) {
            (true, entry) if entry != 0 => format!(
                "{count} item(s) here and {before} in the template list; the two together may \
                 hold {}.",
                vendor::MAX_ITEMS - 1
            ),
            _ => format!("{count} item(s); a creature's two lists together may hold {}.", vendor::MAX_ITEMS - 1),
        },
    );
    let movable: Vec<u32> = rows.iter().filter(|s| s.life != Life::Delete).map(|s| s.ware.item).collect();
    egui::ScrollArea::vertical()
        .max_height(ui.available_height().max(60.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if rows.is_empty() {
                theme::note(ui, "No rows.");
            }
            let mut place = before;
            for shown in &rows {
                let counted = shown.life != Life::Delete && !shown.forbidden;
                if counted {
                    place += 1;
                }
                let mut faults = shown.ware.check();
                let edits = &subject.session.server_edits;
                if subject.quests.item_known(shown.ware.item, edits) && subject.quests.item(shown.ware.item, edits).is_none() {
                    faults.push(format!("item {} is not in item_template", shown.ware.item));
                }
                if shown.forbidden {
                    faults.push("forbidden_items leaves the item out at the server's patch".to_string());
                }
                if list == own && in_template.contains(&shown.ware.item) {
                    faults.push(format!(
                        "the template list {} {} sells it already, so the server skips it here",
                        vendor::TEMPLATE,
                        about.vendor_id
                    ));
                }
                if counted && place >= vendor::MAX_ITEMS {
                    faults.push(format!("it is past the {}th item of the two lists", vendor::MAX_ITEMS - 1));
                }
                let at = movable.iter().position(|item| *item == shown.ware.item);
                vendor_row(ui, subject, list, shown, &faults, at, movable.len());
            }
        });
}

/// One vendor row: the item, its place, its stock, its flags, its condition,
/// and the button that removes it or keeps it.
fn vendor_row(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    list: List,
    shown: &ShownWare,
    faults: &[String],
    at: Option<usize>,
    count: usize,
) {
    let now = subject.now;
    let ware = &shown.ware;
    let held = shown.in_database.as_ref().map(Ware::assignments);
    let key = ware.key();
    let write = |subject: &mut Subject<'_>, column: &'static str, value: String| {
        subject
            .services
            .set_column(subject.session, list.table, &key, held.clone(), column, value, now);
    };
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(NAME, CELL),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(NAME);
                ui.set_max_width(NAME);
                super::loot::state_dot(ui, shown.life, faults);
                super::loot::item_name(
                    ui,
                    subject.quests,
                    subject.items,
                    subject.assets,
                    subject.thumbnails,
                    &subject.session.server_edits,
                    ware.item,
                );
            },
        );
        // The place in the list: two buttons that swap it with its
        // neighbour. The slot number itself is on hover.
        let (up, down) = match at {
            Some(at) => (at > 0, at + 1 < count),
            None => (false, false),
        };
        let about_slot = format!("slot {}: the list is sent to the client in slot order.", ware.slot);
        if ui
            .add_enabled(up, egui::Button::new("up").small())
            .on_hover_text(format!("Move it one place up. {about_slot}"))
            .clicked()
        {
            subject.services.move_ware(subject.session, list, ware.item, -1, now);
        }
        if ui
            .add_enabled(down, egui::Button::new("down").small())
            .on_hover_text(format!("Move it one place down. {about_slot}"))
            .clicked()
        {
            subject.services.move_ware(subject.session, list, ware.item, 1, now);
        }
        // The stock: how many, and how long one takes to come back.
        let mut maxcount = ware.maxcount as i64;
        let limit = ui
            .add_sized(
                [NUMBER, CELL],
                egui::DragValue::new(&mut maxcount)
                    .range(0..=vendor::MAX_COUNT as i64)
                    .speed(0.1)
                    .custom_formatter(|value, _| match value as i64 {
                        0 => "any".to_string(),
                        n => n.to_string(),
                    })
                    .custom_parser(|text| match text.trim() {
                        "any" | "" => Some(0.0),
                        text => text.parse().ok(),
                    }),
            )
            .on_hover_text(
                "maxcount: how many the vendor holds; any is no limit. A limit needs a restock \
                 time, and the server skips the row until both are set.",
            );
        if limit.changed() && maxcount != ware.maxcount as i64 {
            write(subject, "maxcount", maxcount.to_string());
        }
        let mut incrtime = ware.incrtime as i64;
        let restock = ui
            .add_sized(
                [TIME, CELL],
                egui::DragValue::new(&mut incrtime)
                    .range(0..=i64::from(u32::MAX))
                    .speed(10.0)
                    .custom_formatter(|value, _| seconds_words(value as i64, 0)),
            )
            .on_hover_text(format!(
                "incrtime: seconds before one more is restocked; {}. Set exactly when maxcount is.",
                seconds_words(ware.incrtime as i64, 0)
            ));
        if restock.changed() && incrtime != ware.incrtime as i64 {
            write(subject, "incrtime", incrtime.to_string());
        }
        // The two restock flags.
        for bit in &vendor::FLAGS {
            let mut on = ware.flags & bit.bit != 0;
            let word = match bit.bit {
                0x1 => "random",
                _ => "dynamic",
            };
            if ui
                .checkbox(&mut on, egui::RichText::new(word).small())
                .on_hover_text(format!("itemflags {}: {}.", bit.name, bit.about))
                .changed()
            {
                let flags = match on {
                    true => ware.flags | bit.bit,
                    false => ware.flags & !bit.bit,
                };
                write(subject, "itemflags", flags.to_string());
            }
        }
        let column = format!("{} {} condition_id", list.table, key.text());
        condition_cell(ui, &column, ware.condition, |value| write(subject, "condition_id", value));
        remove_or_keep(ui, subject, list.table, &key, shown.life, shown.in_database.is_some());
    });
}

// ---------------------------------------------------------------------------
// The trainer rows
// ---------------------------------------------------------------------------

/// Every row of a trainer list, by the level it needs, with each row's
/// reasons the server would skip it.
fn trainer_rows(ui: &mut egui::Ui, subject: &mut Subject<'_>, about: &About, own: List, template: List, list: List) {
    if !open_spell_tables(subject) {
        ui.label(
            egui::RichText::new("Spell.dbc or Talent.dbc is not in the archives.")
                .small()
                .color(theme::BAD),
        );
        return;
    }
    open_skill_lines(subject);
    let edits = &subject.session.server_edits;
    let rows = subject.services.lessons_of(list, edits);
    let in_template: HashSet<u32> = subject
        .services
        .lessons_of(template, edits)
        .iter()
        .filter(|s| s.life != Life::Delete)
        .map(|s| s.lesson.spell)
        .collect();
    let talents = subject.services.talents.clone().unwrap_or_default();
    // What Spell.dbc says of each row's spell, and the level it needs, for
    // the order and the faults.
    let mut drawn: Vec<(ShownLesson, Option<Taught>, u32, String)> = rows
        .into_iter()
        .map(|shown| {
            let facts = subject
                .session
                .table("Spell")
                .and_then(|spells| services::taught(spells, &talents, shown.lesson.spell));
            let level = shown.lesson.level_needed(facts.map_or(0, |facts| facts.level));
            let name = ability_name(subject, shown.lesson.spell, facts);
            (shown, facts, level, name)
        })
        .collect();
    // By level, then by skill rank, which is the order a recipe list reads in:
    // a profession's teaching spells have a spellLevel of 0.
    drawn.sort_by(|a, b| {
        (a.2, a.0.lesson.skill_value, &a.3, a.0.lesson.spell).cmp(&(b.2, b.0.lesson.skill_value, &b.3, b.0.lesson.spell))
    });
    let count = drawn.iter().filter(|(shown, ..)| shown.life != Life::Delete).count();
    theme::note(ui, format!("{count} spell(s), by the level and then the skill rank each needs."));
    egui::ScrollArea::vertical()
        .max_height(ui.available_height().max(60.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if drawn.is_empty() {
                theme::note(ui, "No rows.");
            }
            for (shown, facts, level, name) in &drawn {
                let teacher = match facts {
                    Some(facts) if !facts.is_teaching() => subject
                        .session
                        .table("Spell")
                        .and_then(|spells| services::teaching_spell(spells, shown.lesson.spell)),
                    _ => None,
                };
                let mut faults = shown.lesson.check();
                faults.extend(trainer::spell_faults(*facts, teacher));
                if list == own && in_template.contains(&shown.lesson.spell) {
                    faults.push(format!(
                        "the template list {} {} teaches it already, so the server skips it here",
                        trainer::TEMPLATE,
                        about.trainer_id
                    ));
                }
                if list == own && !about.has_flag(Kind::Trainer) {
                    faults.push("npc_flags has no TRAINER bit".to_string());
                }
                trainer_row(ui, subject, list, shown, *facts, *level, name, &faults);
            }
        });
}

/// What a trainer row is called: the name and rank of the ability its
/// teaching spell teaches, or of the spell itself when it teaches nothing.
fn ability_name(subject: &mut Subject<'_>, spell: u32, facts: Option<Taught>) -> String {
    let shown = match facts {
        Some(facts) if facts.is_teaching() && facts.teaches != 0 => facts.teaches,
        _ => spell,
    };
    spell_name(subject, shown).unwrap_or_else(|| format!("spell {spell}"))
}

/// A spell's name and rank, as the picker lists it: `Fireball (Rank 2)`.
fn spell_name(subject: &mut Subject<'_>, spell: u32) -> Option<String> {
    let name = super::quests::name_in(subject.session, "Spell", spell)?;
    let open = subject.session.table("Spell")?;
    let rank = open
        .row_of(spell)
        .and_then(|record| open.string_at(record, SPELL_RANK))
        .unwrap_or_default();
    Some(match rank.is_empty() {
        true => name,
        false => format!("{name} ({rank})"),
    })
}

/// `Spell.dbc`'s enUS rank string, field 129, which the picker's sub-line
/// reads too.
const SPELL_RANK: usize = 129;

/// One trainer row: the ability, the level, the price, the skill, and the
/// button that removes it or keeps it.
#[allow(clippy::too_many_arguments)]
fn trainer_row(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    list: List,
    shown: &ShownLesson,
    facts: Option<Taught>,
    level: u32,
    name: &str,
    faults: &[String],
) {
    let now = subject.now;
    let lesson = &shown.lesson;
    let held = shown.in_database.as_ref().map(|lesson| lesson.assignments());
    let key = lesson.key();
    let write = |subject: &mut Subject<'_>, column: &'static str, value: String| {
        subject
            .services
            .set_column(subject.session, list.table, &key, held.clone(), column, value, now);
    };
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(NAME, CELL),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(NAME);
                ui.set_max_width(NAME);
                super::loot::state_dot(ui, shown.life, faults);
                let hover = match facts {
                    Some(facts) if facts.is_teaching() => format!(
                        "Teaching spell {}, which teaches spell {}. Open the teaching spell in the \
                         table browser.",
                        lesson.spell, facts.teaches
                    ),
                    _ => format!("Spell {}. Open it in the table browser.", lesson.spell),
                };
                let link = ui.add(
                    egui::Label::new(egui::RichText::new(name).color(theme::ACCENT))
                        .truncate()
                        .sense(egui::Sense::click()),
                );
                if link.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hover).clicked() {
                    subject.quests.show_row = Some(("Spell", lesson.spell));
                }
            },
        );
        // The level: 0 is the teaching spell's own `spellLevel`.
        let own_level = facts.map_or(0, |facts| facts.level);
        let mut reqlevel = lesson.level as i64;
        let levelled = ui
            .add_sized(
                [NUMBER + 20.0, CELL],
                egui::DragValue::new(&mut reqlevel)
                    .range(0..=trainer::MAX_LEVEL as i64)
                    .speed(0.1)
                    .custom_formatter(move |value, _| match (value as i64, own_level) {
                        (0, 0) => "any level".to_string(),
                        (0, own) => format!("spell's {own}"),
                        (n, _) => format!("level {n}"),
                    })
                    .custom_parser(|text| {
                        let text = text.trim().trim_start_matches("level").trim();
                        match text.starts_with("spell") || text.starts_with("any") {
                            true => Some(0.0),
                            false => text.parse().ok(),
                        }
                    }),
            )
            .on_hover_text(format!(
                "reqlevel: the level a player needs, {level} here. 0 is the teaching spell's own \
                 spellLevel, {own_level}, and a spellLevel of 0 is any level; the server logs a \
                 reqlevel equal to spellLevel as redundant."
            ));
        if levelled.changed() && reqlevel != lesson.level as i64 {
            write(subject, "reqlevel", reqlevel.to_string());
        }
        // The price, in copper, drawn as coins.
        let mut cost = lesson.cost as i64;
        let priced = ui
            .add_sized(
                [MONEY, CELL],
                egui::DragValue::new(&mut cost)
                    .range(0..=i64::from(u32::MAX))
                    .speed(1.0)
                    .custom_formatter(|value, _| match value as i64 {
                        0 => "free".to_string(),
                        copper => money_words(copper as u64),
                    })
                    .custom_parser(parse_money),
            )
            .on_hover_text(
                "spellcost, in copper. Type 150, or 1s 50c, or 2g. The server sends it lowered by \
                 the player's reputation discount with the trainer's faction.",
            );
        if priced.changed() && cost != lesson.cost as i64 {
            write(subject, "spellcost", cost.to_string());
        }
        // The skill it needs, chosen from SkillLine.dbc, and the rank in it.
        let groups = subject.services.skill_lines.as_deref().unwrap_or(&[]);
        let menu = egui::Id::new(("reqskill", list.table, key.text()));
        if let Some(skill) = skill_cell(ui, menu, groups, lesson.skill) {
            if skill != lesson.skill {
                write(subject, "reqskill", skill.to_string());
            }
        }
        let mut value = lesson.skill_value as i64;
        let ranked = ui
            .add_enabled_ui(lesson.skill != 0, |ui| {
                ui.add_sized(
                    [NUMBER, CELL],
                    egui::DragValue::new(&mut value).range(0..=i64::from(u16::MAX)),
                )
            })
            .inner
            .on_hover_text("reqskillvalue: the rank in reqskill the player needs.")
            .on_disabled_hover_text("Set reqskill first.");
        if ranked.changed() && value != lesson.skill_value as i64 {
            write(subject, "reqskillvalue", value.to_string());
        }
        remove_or_keep(ui, subject, list.table, &key, shown.life, shown.in_database.is_some());
    });
}

/// Open `SkillLine.dbc` and `SkillLineCategory.dbc`, and group the lines
/// once, for the `reqskill` menu. Without the category table the groups are
/// headed by their ids.
fn open_skill_lines(subject: &mut Subject<'_>) {
    if subject.services.skill_lines.is_some() || !subject.session.open_table(subject.assets, "SkillLine") {
        return;
    }
    let _ = subject.session.open_table(subject.assets, "SkillLineCategory");
    let Some(lines) = subject.session.table("SkillLine") else {
        return;
    };
    subject.services.skill_lines = Some(services::skill_groups(lines, subject.session.table("SkillLineCategory")));
}

/// `reqskill` as a menu of `SkillLine.dbc`'s lines, grouped by category with
/// the professions first, and a filter at its head. Text in the filter
/// narrows the list by name or id; a number in it also offers that id as it
/// is, so a line the table does not hold can be written. Returns the id
/// chosen.
///
/// The menu stays open while the filter is typed into, and closes on a
/// choice or a click outside it. The filter is kept in egui's memory under
/// the menu's id, and is emptied on a choice.
fn skill_cell(ui: &mut egui::Ui, menu: egui::Id, groups: &[services::SkillGroup], skill: u32) -> Option<u32> {
    let shown = match skill {
        0 => "no skill".to_string(),
        id => match services::skill_name(groups, id) {
            Some(name) => format!("{name} ({id})"),
            None => format!("skill line {id}"),
        },
    };
    let filter_id = menu.with("filter");
    let opened_id = menu.with("opened");
    let mut chosen: Option<u32> = None;
    let response = egui::ComboBox::from_id_salt(menu)
        .selected_text(egui::RichText::new(shown).small())
        .width(SKILL)
        .height(320.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let pass = ui.ctx().cumulative_pass_nr();
            // Focus the filter on the pass the menu opens, so a person can
            // type straight away: the menu was not drawn on the pass before.
            let last: Option<u64> = ui.data(|data| data.get_temp(opened_id));
            let just_opened = last.is_none_or(|last| last + 1 < pass);
            ui.data_mut(|data| data.insert_temp(opened_id, pass));
            let mut filter: String = ui.data(|data| data.get_temp(filter_id)).unwrap_or_default();
            let typed = ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text("name, or a SkillLine id")
                    .desired_width(f32::INFINITY),
            );
            if just_opened {
                typed.request_focus();
            }
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            // A typed number is offered as it is, whether or not the table
            // holds it.
            if let Ok(id) = filter.trim().parse::<u32>() {
                let label = match services::skill_name(groups, id) {
                    Some(name) => format!("Use {id}, {name}"),
                    None => format!("Use {id}, which SkillLine.dbc does not hold"),
                };
                if ui.selectable_label(false, label).clicked() || typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    chosen = Some(id);
                }
            }
            if filter.trim().is_empty() && ui.selectable_label(skill == 0, "no skill (0)").clicked() {
                chosen = Some(0);
            }
            for group in groups {
                let hits: Vec<&services::SkillLine> = group.lines.iter().filter(|line| line.matches(&filter)).collect();
                if hits.is_empty() {
                    continue;
                }
                ui.label(egui::RichText::new(&group.name).small().color(theme::INK_DIM));
                for line in hits {
                    if ui
                        .selectable_label(line.id == skill, format!("{}  {}", line.name, line.id))
                        .clicked()
                    {
                        chosen = Some(line.id);
                    }
                }
            }
            if chosen.is_some() {
                ui.data_mut(|data| data.remove::<String>(filter_id));
                ui.close();
            }
        });
    response.response.on_hover_text(
        "reqskill: the SkillLine.dbc line the player must have, or no skill. Type in the menu to \
         narrow it, or type an id to use it as it is.",
    );
    chosen
}

/// A price typed as a person writes one: a bare number of copper, or coins
/// with `g`, `s` and `c` after them (`1g 20s`, `75c`).
fn parse_money(text: &str) -> Option<f64> {
    let text = text.trim().to_ascii_lowercase();
    if text == "free" {
        return Some(0.0);
    }
    if let Ok(copper) = text.parse::<f64>() {
        return Some(copper);
    }
    let mut total = 0.0;
    let mut digits = String::new();
    for c in text.chars() {
        match c {
            '0'..='9' | '.' => digits.push(c),
            'g' | 's' | 'c' => {
                let value: f64 = digits.parse().ok()?;
                digits.clear();
                total += value
                    * match c {
                        'g' => 10_000.0,
                        's' => 100.0,
                        _ => 1.0,
                    };
            }
            ' ' => {}
            _ => return None,
        }
    }
    match digits.is_empty() {
        true => Some(total),
        false => None,
    }
}

// ---------------------------------------------------------------------------
// The cells vendor rows and trainer rows share
// ---------------------------------------------------------------------------

/// A `condition_id`: its number, then what it tests and the buttons that
/// choose or make one. `column` is what the condition window calls it.
fn condition_cell(ui: &mut egui::Ui, column: &str, condition: u32, mut write: impl FnMut(String)) {
    let mut value = condition as i64;
    let changed = ui
        .add_sized(
            [NUMBER, CELL],
            egui::DragValue::new(&mut value).range(0..=i64::from(u32::MAX)).prefix("c"),
        )
        .on_hover_text("condition_id: a row of `conditions` the player must meet to see it, or 0.")
        .changed();
    let typed = (changed && value != condition as i64).then_some(value as u32);
    let answered = super::conditions::cell(ui, egui::Id::new(("service-condition", column)), column, condition, Some(CONDITION));
    if let Some(value) = typed.or(answered) {
        write(value.to_string());
    }
}

/// The button at the end of a row: remove it, or take the removal mark off.
fn remove_or_keep(
    ui: &mut egui::Ui,
    subject: &mut Subject<'_>,
    table: &'static str,
    key: &vale_mangos::row::Key,
    life: Life,
    in_database: bool,
) {
    let now = subject.now;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let (label, about) = match life {
            Life::Delete => ("keep", "Take the removal mark off this row."),
            _ => ("\u{d7}", "Remove this row. The item or the spell itself is not touched."),
        };
        if ui.small_button(label).on_hover_text(about).clicked() {
            match life {
                Life::Delete => subject.services.keep(subject.session, table, key, now),
                _ => subject.services.remove(subject.session, table, key, in_database, now),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A price reads as copper, as coins, or as the word the cell shows for 0.
    #[test]
    fn a_price_is_typed_as_coins_or_copper() {
        assert_eq!(parse_money("150"), Some(150.0));
        assert_eq!(parse_money("1s 50c"), Some(150.0));
        assert_eq!(parse_money("2g"), Some(20_000.0));
        assert_eq!(parse_money("1g 2s 3c"), Some(10_203.0));
        assert_eq!(parse_money("free"), Some(0.0));
        assert_eq!(parse_money("1g 2"), None);
        assert_eq!(parse_money("lots"), None);
    }
}
