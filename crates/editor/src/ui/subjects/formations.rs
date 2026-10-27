//! The Formation and Links sections of the creature inspector: which spawns
//! walk and fight with the selected one, and which answer it. The view of
//! [`crate::subjects::formations`].
//!
//! ## Formation
//!
//! The formation the selected spawn is in, by its leader, and each member
//! with its distance, angle and options from `creature_groups`. A member's
//! slot is the point its `dist` and `angle` name from the leader's position
//! and facing (`crate::subjects::formations::Placed::slot`). A diagram draws
//! the formation from above with the leader's facing up: each slot, and
//! where a member stands when that is away from its slot.
//!
//! Add Selected puts the other selected spawns in the formation where they
//! stand; with no formation, Start Formation makes the selected spawn the
//! leader of one. A member's Slot Where It Stands button writes its distance
//! and angle from where it stands now, so a member dragged in the viewport
//! has its slot there. The viewport draws a line from the leader to each
//! slot, a ring at each slot, a line from a member standing away from its
//! slot to the slot, and the leader's facing
//! (`crate::tools::spawns::creatures`).
//!
//! ## Links
//!
//! The selected spawn's master and what it does when the master acts, the
//! spawns that answer it, and the template links of its entry: every row of
//! `creature_linking` and `creature_linking_template` naming it.

use super::keyed::{columns, keep_row, listed_row, not_ready, Section};
use crate::subjects::formations::{Formations, IN_SLOT};
use crate::subjects::keyed::Shown;
use crate::ui::widgets::rowform::{section, Hook, RowAct};
use crate::ui::widgets::theme;
use bevy_egui::egui;
use vale_mangos::formation::{self, Link, Member, TemplateLink};

/// The Formation section.
pub(in crate::ui) fn formation_section(ui: &mut egui::Ui, sec: &mut Section<'_>, formations: &Formations) {
    let edits = sec.session.rows.edits.clone();
    let primary = sec.chosen.primary.clone();
    let formation = formations.formation_of(primary.guid, &edits);
    let edited = formation.as_ref().is_some_and(|formation| {
        formation.rows.iter().any(|shown| edits.touches(formation::GROUPS, &shown.row.key()))
    });
    section(ui, ("spawn-formation", primary.guid), "Formation", edited, formation.is_some(), |ui| {
        theme::note(
            ui,
            "creature_groups: the spawns that walk and fight with this one. A member's slot is \
             dist yards from the leader at angle from the leader's facing; while the leader \
             moves, the member walks at its slot. Live on .reload creature_groups.",
        );
        if not_ready(ui, formations.trouble(), formations.held().is_some(), "creature_groups") {
            return;
        }
        let others = sec.chosen.other_guids();
        let Some(formation) = formation else {
            ui.label(egui::RichText::new("No formation").color(theme::INK_DIM));
            if ui
                .add_enabled(!others.is_empty(), egui::Button::new(format!("Start Formation ({})", others.len())))
                .on_hover_text(
                    "Lead a formation with this spawn: the other selected spawns join it where \
                     they stand, moving in formation and aggroing together.",
                )
                .on_disabled_hover_text("Shift-click the spawns to add, then press again.")
                .clicked()
            {
                let members: Vec<_> = sec.chosen.others.iter().map(|other| other.placed()).collect();
                sec.session.status = formations.join(sec.session, primary.placed(), &members, sec.now);
            }
            return;
        };
        let leads = formation.leader == primary.guid;
        theme::row(ui, "Leader", |ui| {
            match (leads, sec.chosen.find(formation.leader)) {
                (true, _) => {
                    ui.label(egui::RichText::new("This spawn").color(theme::INK));
                }
                (false, Some(brief)) => {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    crate::ui::widgets::reference::template_link(
                        ui,
                        sec.asks,
                        crate::tools::spawns::spawn::SpawnKind::Creature,
                        brief.entry,
                        egui::RichText::new(&brief.label),
                    );
                    ui.label(egui::RichText::new(format!(" \u{b7} guid {}", formation.leader)).color(theme::INK));
                    ui.spacing_mut().item_spacing.x = 8.0;
                }
                (false, None) => {
                    ui.label(egui::RichText::new(sec.chosen.name(formation.leader)).color(theme::INK));
                }
            }
            sec.select_button(ui, formation.leader);
        });
        theme::row(ui, "Options", |ui| {
            ui.label(egui::RichText::new(formation::flag_words(&formation::GROUP_OPTIONS, formation.options())).small())
                .on_hover_text("Every member's flags, ORed: the server applies an option set on one member to the whole formation.");
        });
        shape(ui, sec, &formation);
        for shown in formation.rows.iter().filter(|shown| !shown.row.is_leader_row()) {
            member(ui, sec, formations, shown, formation.leader);
        }
        ui.horizontal_wrapped(|ui| {
            let leader = sec.chosen.find(formation.leader);
            let adding: Vec<_> = sec
                .chosen
                .others
                .iter()
                .filter(|other| other.guid != formation.leader)
                .map(|other| other.placed())
                .collect();
            if ui
                .add_enabled(leader.is_some() && !adding.is_empty(), egui::Button::new(format!("Add Selected ({})", adding.len())))
                .on_hover_text("The other selected spawns join this formation where they stand. A spawn in another formation leaves it.")
                .on_disabled_hover_text("Shift-click the spawns to add. The leader must be on this map.")
                .clicked()
            {
                if let Some(leader) = leader {
                    sec.session.status = formations.join(sec.session, leader.placed(), &adding, sec.now);
                }
            }
            if !leads
                && ui
                    .button("Make Leader")
                    .on_hover_text(
                        "Lead the formation with this spawn: every member is placed from it where \
                         it stands, and the old leader becomes a member.",
                    )
                    .clicked()
            {
                let chosen = sec.chosen;
                let placed = |guid: u64| chosen.find(guid).map(|brief| brief.placed());
                sec.session.status = match formations.make_leader(sec.session, primary.placed(), placed, sec.now) {
                    Ok(line) => line,
                    Err(e) => format!("not changed: {e}"),
                };
            }
            if !leads && ui.button("Leave Formation").clicked() {
                formations.leave(sec.session, primary.guid, sec.now);
            }
        });
    });
}

/// The colour of a member's slot, the viewport's formation colour
/// (`crate::tools::spawns::creatures`).
const SLOT: egui::Color32 = egui::Color32::from_rgb(77, 217, 242);

/// The height of the formation's diagram, in points.
const SHAPE_HEIGHT: f32 = 170.0;

/// How near the pointer must come to a spawn in the diagram to name it, in
/// points.
const SHAPE_REACH: f32 = 9.0;

/// One spawn in the formation's diagram, in yards from the leader with the
/// leader's facing up: x to the right, y down.
struct Dot {
    guid: u64,
    /// The slot, or the leader's own place for the leader.
    slot: egui::Vec2,
    /// Where the spawn stands, when it is on this map and away from its slot.
    stray: Option<egui::Vec2>,
    about: String,
}

/// A point `dist` yards away at `angle` radians from the leader's facing, in
/// the diagram's axes. The angle turns toward +y in the world's axes, which
/// is to the leader's left.
fn ahead(dist: f32, angle: f32) -> egui::Vec2 {
    egui::vec2(-angle.sin() * dist, -angle.cos() * dist)
}

/// The formation from above with the leader's facing up: the leader, each
/// member's slot filled, and where a member stands away from its slot
/// hollow, joined to the slot. The pointer over a spawn names it, and a
/// press selects it.
fn shape(ui: &mut egui::Ui, sec: &mut Section<'_>, formation: &crate::subjects::formations::Formation) {
    let Some(leader) = sec.chosen.find(formation.leader) else { return };
    let placed = leader.placed();
    let mut dots = vec![Dot {
        guid: leader.guid,
        slot: egui::Vec2::ZERO,
        stray: None,
        about: format!("Leader: {}", sec.chosen.name(leader.guid)),
    }];
    for shown in formation.members() {
        let row = &shown.row;
        let stands = sec.chosen.find(row.member).map(|brief| brief.at);
        let off = stands.map(|at| placed.slot_distance(at, row));
        let stray = stands.filter(|_| off.is_some_and(|off| off > IN_SLOT)).map(|at| {
            let (dx, dy) = (at.x - leader.at.x, at.y - leader.at.y);
            ahead((dx * dx + dy * dy).sqrt(), dy.atan2(dx) - leader.facing)
        });
        let mut about = format!(
            "{}\n{:.1} yd at {:.0}\u{b0} from the leader's facing",
            sec.chosen.name(row.member),
            row.dist,
            row.angle.to_degrees()
        );
        match off {
            Some(off) if off > IN_SLOT => about.push_str(&format!("\nstands {off:.1} yd from its slot")),
            Some(_) => about.push_str("\nstands in its slot"),
            None => {}
        }
        dots.push(Dot { guid: row.member, slot: ahead(row.dist, row.angle), stray, about });
    }

    // The scale fits the farthest slot or stray spawn in a ring of whole
    // yards.
    let far = dots
        .iter()
        .flat_map(|dot| [Some(dot.slot), dot.stray].into_iter().flatten())
        .map(egui::Vec2::length)
        .fold(1.0_f32, f32::max)
        .ceil();
    let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width(), SHAPE_HEIGHT), egui::Sense::click());
    let rect = response.rect;
    painter.rect_filled(rect, 3.0, theme::SUNK);
    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
    let centre = rect.center();
    let scale = (rect.width().min(rect.height()) / 2.0 - 14.0).max(10.0) / far;
    let at = |yards: egui::Vec2| centre + yards * scale;
    painter.circle_stroke(centre, far * scale, egui::Stroke::new(1.0, theme::LINE));
    painter.text(
        rect.left_bottom() + egui::vec2(6.0, -4.0),
        egui::Align2::LEFT_BOTTOM,
        format!("ring {far:.0} yd"),
        egui::FontId::proportional(11.0),
        theme::INK_FAINT,
    );

    let primary = sec.chosen.primary.guid;
    for dot in &dots[1..] {
        painter.line_segment([centre, at(dot.slot)], egui::Stroke::new(1.0, SLOT.gamma_multiply(0.35)));
        if let Some(stray) = dot.stray {
            painter.line_segment([at(stray), at(dot.slot)], egui::Stroke::new(1.0, theme::INK_DIM));
            painter.circle_stroke(at(stray), 3.5, egui::Stroke::new(1.5, theme::INK_DIM));
        }
        painter.circle_filled(at(dot.slot), 4.5, SLOT);
    }
    painter.arrow(centre, egui::vec2(0.0, -22.0), egui::Stroke::new(2.0, theme::INK));
    painter.circle_filled(centre, 5.0, theme::INK);
    if let Some(chosen) = dots.iter().find(|dot| dot.guid == primary) {
        painter.circle_stroke(at(chosen.slot), 8.0, egui::Stroke::new(1.5, theme::ACCENT));
    }

    let hovered = response.hover_pos().and_then(|pointer| {
        dots.iter()
            .flat_map(|dot| [Some(dot.slot), dot.stray].into_iter().flatten().map(move |yards| (dot, at(yards).distance(pointer))))
            .filter(|(_, away)| *away <= SHAPE_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(dot, _)| dot)
    });
    if response.clicked() {
        if let Some(dot) = hovered.filter(|dot| dot.guid != primary) {
            sec.select = Some(dot.guid);
        }
    }
    if let Some(dot) = hovered {
        response.on_hover_text_at_pointer(dot.about.as_str());
    }

    let mut caption = "From above, the leader's facing up. Filled: each member's slot. Hollow: where \
                       a member stands away from its slot; it walks to the slot when the leader moves. \
                       The formation turns with the leader."
        .to_string();
    if formation.options() & FORMATION_MOVE == 0 {
        caption.push_str(" FORMATION_MOVE is off, so the members do not keep their slots.");
    }
    theme::note(ui, caption);
}

/// `FORMATION_MOVE`, the option that makes members keep their slots.
const FORMATION_MOVE: u32 = 0x001;

/// One member of a formation: its name, distance and angle on one line, and
/// its columns under it.
fn member(ui: &mut egui::Ui, sec: &mut Section<'_>, formations: &Formations, shown: &Shown<Member>, leader: u64) {
    let guid = shown.row.member;
    let key = shown.row.key();
    let mut faults = shown.row.check();
    if sec.chosen.find(guid).is_none() {
        faults.push("the spawn is not on this map; the server skips a member on another map".to_string());
    }
    let placed = sec.chosen.find(leader).zip(sec.chosen.find(guid)).map(|(a, b)| (a.placed(), b.placed()));
    let off = placed.map(|(leader, member)| leader.slot_distance(member.at, &shown.row));
    let mut title = format!(
        "{}  \u{b7}  {:.1} yd at {:.0}\u{b0}",
        sec.chosen.name(guid),
        shown.row.dist,
        shown.row.angle.to_degrees()
    );
    if let Some(off) = off.filter(|off| *off > IN_SLOT) {
        title.push_str(&format!("  \u{b7}  {off:.1} yd from its slot"));
    }
    let about = "Takes the spawn out of the formation.";
    let act = listed_row(ui, ("formation-member", guid), shown.life, &faults, title, about, |ui| {
        if shown.kept() {
            let held = shown.in_database.as_ref().map(Member::assignments);
            let written = columns(
                ui,
                &mut sec.view(),
                formation::GROUPS,
                &key,
                held.as_deref(),
                &formation::GROUP_COLUMNS,
                "Edit formation",
                |hook, ui, field, showing| {
                    if hook == Hook::After && field.column.name == "angle" {
                        let radians: f32 = showing.trim().parse().unwrap_or(0.0);
                        ui.label(egui::RichText::new(format!("{:.0}\u{b0}", radians.to_degrees())).small().color(theme::INK_DIM));
                    }
                    None
                },
            );
            if let Some((column, value)) = written {
                formations.set_member_column(sec.session, guid, column, value, sec.now);
            }
        }
        ui.horizontal(|ui| {
            let why_not = match off {
                None => Some("The member or the leader is not on this map."),
                Some(off) if off <= IN_SLOT => Some("The member stands in its slot."),
                Some(_) => None,
            };
            if shown.kept()
                && ui
                    .add_enabled(why_not.is_none(), egui::Button::new("Slot Where It Stands").small())
                    .on_hover_text(
                        "Write dist and angle so the member's slot is where it stands now, from the \
                         leader's position and facing. Drag the member in the viewport first to \
                         choose its slot.",
                    )
                    .on_disabled_hover_text(why_not.unwrap_or_default())
                    .clicked()
            {
                if let Some((leader, member)) = placed {
                    formations.place(sec.session, leader, member, sec.now);
                }
            }
            sec.select_button(ui, guid);
        });
    });
    match act {
        Some(RowAct::Keep) => keep_row(&mut sec.view(), formation::GROUPS, &key),
        Some(_) => formations.leave(sec.session, guid, sec.now),
        None => {}
    }
}

/// The Links section.
pub(in crate::ui) fn links_section(ui: &mut egui::Ui, sec: &mut Section<'_>, formations: &Formations) {
    let edits = sec.session.rows.edits.clone();
    let primary = sec.chosen.primary.clone();
    let master = formations.link_of(primary.guid, &edits);
    let slaves = formations.slaves_of(primary.guid, &edits);
    let templates = formations.template_links_of(primary.entry, &edits);
    let any = master.is_some() || !slaves.is_empty() || !templates.is_empty();
    let edited = master.iter().map(|shown| (formation::LINKING, shown.row.key()))
        .chain(slaves.iter().map(|shown| (formation::LINKING, shown.row.key())))
        .chain(templates.iter().map(|shown| (formation::LINKING_TEMPLATE, shown.row.key())))
        .any(|(table, key)| edits.touches(table, &key));
    section(ui, ("spawn-links", primary.guid), "Links", edited, any, |ui| {
        theme::note(
            ui,
            "creature_linking: what this spawn does when its master acts, and the spawns that \
             answer it. Read at startup; a change needs a restart.",
        );
        if not_ready(ui, formations.trouble(), formations.held().is_some(), "creature_linking") {
            return;
        }
        match master {
            Some(shown) => link(ui, sec, formations, &shown, "Master: ", shown.row.master),
            None => {
                theme::row(ui, "Master", |ui| {
                    ui.label(egui::RichText::new("None").color(theme::INK_DIM));
                });
            }
        }
        if !slaves.is_empty() {
            theme::heading(ui, "Slaves");
            for shown in &slaves {
                link(ui, sec, formations, shown, "", shown.row.slave);
            }
        }
        let others = sec.chosen.other_guids();
        if ui
            .add_enabled(!others.is_empty(), egui::Button::new(format!("Link Selected as Slaves ({})", others.len())))
            .on_hover_text("The other selected spawns answer this one: each aggroes when it does. A spawn with another master is moved to this one.")
            .on_disabled_hover_text("Shift-click the spawns to link, then press again.")
            .clicked()
        {
            sec.session.status = formations.link(sec.session, primary.guid, &others, sec.now);
        }
        theme::heading(ui, "Template Links");
        if templates.is_empty() {
            theme::note(ui, "creature_linking_template: none names this creature.");
        }
        for shown in &templates {
            template_link(ui, sec, formations, shown);
        }
        let master_entry = sec.chosen.others.first().map(|other| other.entry);
        if ui
            .add_enabled(master_entry.is_some(), egui::Button::new("Add Template Link"))
            .on_hover_text(
                "Link every spawn of this creature on this map to a spawn of the first other \
                 selected spawn's creature, by creature_linking_template.",
            )
            .on_disabled_hover_text("Shift-click a spawn of the master creature, then press again.")
            .clicked()
        {
            if let Some(master_entry) = master_entry {
                sec.session.status =
                    match formations.add_template_link(sec.session, primary.entry, sec.chosen.map, master_entry, sec.now) {
                        Ok(line) => line,
                        Err(e) => format!("not changed: {e}"),
                    };
            }
        }
    });
}

/// One link: `role`, the other spawn's name and the flag's words on one
/// line, and the flag under it. A slave is listed under its heading, so its
/// role is empty.
fn link(ui: &mut egui::Ui, sec: &mut Section<'_>, formations: &Formations, shown: &Shown<Link>, role: &str, other: u64) {
    let key = shown.row.key();
    let slave = shown.row.slave;
    let mut faults = shown.row.check();
    if sec.chosen.find(other).is_none() {
        faults.push("the spawn is not on this map; the server skips a link between maps".to_string());
    }
    let title = format!(
        "{role}{}  \u{b7}  {}",
        sec.chosen.name(other),
        formation::flag_words(&formation::LINKING_FLAGS, shown.row.flag)
    );
    let act = listed_row(ui, ("spawn-link", slave), shown.life, &faults, title, "Removes the link.", |ui| {
        if shown.kept() {
            let held = shown.in_database.as_ref().map(Link::assignments);
            let written = columns(
                ui,
                &mut sec.view(),
                formation::LINKING,
                &key,
                held.as_deref(),
                &formation::LINKING_COLUMNS,
                "Edit link",
                |_, _, _, _| None,
            );
            if let Some((column, value)) = written {
                formations.set_link_column(sec.session, slave, column, value, sec.now);
            }
        }
        sec.select_button(ui, other);
    });
    match act {
        Some(RowAct::Keep) => keep_row(&mut sec.view(), formation::LINKING, &key),
        Some(_) => formations.unlink(sec.session, slave, sec.now),
        None => {}
    }
}

/// One template link, with its columns under it.
fn template_link(ui: &mut egui::Ui, sec: &mut Section<'_>, formations: &Formations, shown: &Shown<TemplateLink>) {
    let row = &shown.row;
    let mut faults = row.check();
    if row.needs_one_master() {
        faults.push(format!(
            "search_range is 0 and the flag needs one master: the server skips the row unless map {} holds exactly one spawn of {}",
            row.map, row.master_entry
        ));
    }
    let title = format!(
        "entry {} on map {} \u{2192} entry {}  \u{b7}  {}",
        row.entry,
        row.map,
        row.master_entry,
        formation::flag_words(&formation::LINKING_FLAGS, row.flag)
    );
    let key = row.key();
    let about = "Removes the template link.";
    let act = listed_row(ui, ("template-link", key.text()), shown.life, &faults, title, about, |ui| {
        if shown.kept() {
            let held = shown.in_database.as_ref().map(TemplateLink::assignments);
            let written = columns(
                ui,
                &mut sec.view(),
                formation::LINKING_TEMPLATE,
                &key,
                held.as_deref(),
                &formation::LINKING_TEMPLATE_COLUMNS,
                "Edit template link",
                |_, _, _, _| None,
            );
            if let Some((column, value)) = written {
                formations.set_template_column(sec.session, &key, column, value, sec.now);
            }
        }
    });
    match act {
        Some(RowAct::Keep) => keep_row(&mut sec.view(), formation::LINKING_TEMPLATE, &key),
        Some(_) => formations.remove_template_link(sec.session, &key, sec.now),
        None => {}
    }
}
