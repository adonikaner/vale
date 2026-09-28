//! The attachment lab's card and pane, and the model view the pane is in
//! the rest of the time.
//!
//! The state and the systems are in `crate::lab`; this is what is drawn. The
//! card sits at the head of an effect's form and holds the numbers; the pane
//! takes the storyboard's place on the right and shows the stage with the
//! mannequin on it, the three arrows painted over the picture, and the drags
//! that move the model. In the model view — every effect row, until
//! **Position on character…** is pressed — the same pane shows the model by
//! itself, with what the file says about it under the title and the
//! reference tool's switches along the foot: wireframe, spin, particles.
//!
//! ## What a drag is
//!
//! A press on an arrow moves the model along that axis and nothing else; a
//! press with `Shift` held moves it across the screen; any other press orbits
//! the stage, which is what the storyboard's pane does with every press. The
//! arrows are hit-tested in pane points against the projection
//! `crate::lab::project_handles` wrote last frame, and a drag that began on
//! one belongs to it until the button comes up.

use super::data::Workspace;
use super::theme;
use crate::lab::{Drag, Handles, Lab, ARROW_YARDS, BODIES};
use crate::stage::Stage;
use crate::tools::tables;
use vale_assets::tables::schema;
use vale_assets::tables::spell::{effect_scale, fields};
use bevy::prelude::Vec2;
use bevy_egui::egui;

/// How close to an arrow a press has to be, in points.
const GRIP: f32 = 9.0;
/// The label column of the card.
const LABEL: f32 = 130.0;
const VALUE: f32 = 90.0;

/// Open the lab on the effect row that is open, with what the tables say
/// about it: its name, its model, its scale, and where it is used.
pub fn open_on(
    lab: &mut Lab,
    stage: &mut Stage,
    browser: &mut tables::Browser,
    session: &crate::session::EditSession,
    record: usize,
) {
    open_in(lab, stage, browser, session, record, false);
}

/// Open the model view on the effect row that is open: the same as
/// [`open_on`], with the model shown by itself.
pub fn open_alone(
    lab: &mut Lab,
    stage: &mut Stage,
    browser: &mut tables::Browser,
    session: &crate::session::EditSession,
    record: usize,
) {
    open_in(lab, stage, browser, session, record, true);
}

fn open_in(
    lab: &mut Lab,
    stage: &mut Stage,
    browser: &mut tables::Browser,
    session: &crate::session::EditSession,
    record: usize,
    alone: bool,
) {
    let Some(table) = session.table("SpellVisualEffectName") else {
        return;
    };
    let id = table.u32_at(record, 0).unwrap_or(0);
    let name = table
        .string_at(record, fields::EFFECT_NAME)
        .unwrap_or_default();
    let model = table
        .string_at(record, fields::EFFECT_MODEL)
        .unwrap_or_default();
    let scale = effect_scale(table.f32_at(record, fields::EFFECT_SCALE));
    let (anchor, ids) = tables::effect_anchor(browser, session, id);
    // A change of mode on the same effect keeps the orbit; a new effect gets
    // the framing one body wants, which is closer than the storyboard's two
    // actors need.
    let same = lab.is_open_on(id);
    match alone {
        true => lab.open_alone(id, &name, &model, scale, anchor, ids),
        false => lab.open(id, &name, &model, scale, anchor, ids),
    }
    if !same {
        stage.distance = 5.0;
        stage.pitch = 0.1;
    }
}

/// The card: the body, the point, the nine numbers, the export.
pub fn card(ui: &mut egui::Ui, work: &mut Workspace<'_>, lab: &mut Lab, record: usize) {
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            let width = ui.available_width();
            ui.set_max_width(width);
            // The title and the hint, left to right, and nothing else. Two
            // earlier layouts put a way out here, a `×` at the right, and both
            // broke the egui layout: a `with_layout` takes its parent's whole
            // rectangle (which filled the card), and an
            // `allocate_ui_with_layout` laying out from the right overflows to
            // the left when its contents do not fit (which pushed the card off
            // the panel). The way out is the button beside **Bake & Apply**,
            // where a person looks for it, and the card has only that one.
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("ATTACHMENT LAB")
                        .small()
                        .color(theme::INK_FAINT),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "drag an arrow to move along it, shift+drag to move across the screen",
                        )
                        .small()
                        .color(theme::INK_DIM),
                    )
                    .truncate(),
                );
            });
            ui.add_space(4.0);
            let now = work.now;

            // The body and the point: one row when both fit, two otherwise.
            let both = width >= LABEL * 2.0 + 180.0 + 200.0 + 24.0;
            ui.horizontal(|ui| {
                labelled(ui, "Reference character");
                let mut body = lab.body;
                let wide = fits(ui, 180.0);
                egui::ComboBox::from_id_salt("lab-body")
                    .selected_text(BODIES[body].name)
                    .width(wide)
                    .show_ui(ui, |ui| {
                        for (at, candidate) in BODIES.iter().enumerate() {
                            ui.selectable_value(&mut body, at, candidate.name);
                        }
                    });
                if body != lab.body {
                    lab.body = body;
                }
                if both {
                    labelled(ui, "Attachment point");
                    point_combo(ui, work, lab);
                }
            });
            if !both {
                ui.horizontal(|ui| {
                    labelled(ui, "Attachment point");
                    point_combo(ui, work, lab);
                });
            }

            // The seven numbers as label-value pairs, in as many columns as
            // the width holds: the panel beside the pane is narrow, and pairs
            // of fixed widths on one row were cut off at its edge.
            let pair = LABEL + VALUE + 12.0;
            let columns = ((width / pair).floor() as usize).clamp(1, 3);
            let mut values = [
                ("Offset X (+forward)", lab.offset[0], 0.01),
                ("Offset Y (+left)", lab.offset[1], 0.01),
                ("Offset Z (+up)", lab.offset[2], 0.01),
                ("Yaw °", lab.yaw, 1.0),
                ("Pitch °", lab.pitch, 1.0),
                ("Roll °", lab.roll, 1.0),
                ("Scale", lab.scale, 0.01),
            ];
            let mut changed = false;
            for row in values.chunks_mut(columns) {
                ui.horizontal(|ui| {
                    for (label, value, speed) in row.iter_mut() {
                        changed |= number(ui, label, value, *speed);
                    }
                });
            }
            if changed {
                lab.offset = [values[0].1, values[1].1, values[2].1];
                lab.yaw = values[3].1;
                lab.pitch = values[4].1;
                lab.roll = values[5].1;
                lab.scale = values[6].1;
            }
            ui.horizontal(|ui| {
                if ui
                    .button("Reset")
                    .on_hover_text("back to the model as it is")
                    .clicked()
                {
                    lab.offset = [0.0; 3];
                    lab.yaw = 0.0;
                    lab.pitch = 0.0;
                    lab.roll = 0.0;
                    lab.scale = 1.0;
                    changed = true;
                }
            });
            if !(lab.scale > 0.001) {
                lab.scale = 0.001;
            }
            if changed {
                lab.touched(now);
            }

            ui.add_space(4.0);
            // What the chosen point is: a column of the kit, so the tables
            // moved with it, or a point only the preview hangs from. Without
            // this the drop-down looks the same in both cases.
            let slot = lab
                .effect
                .and_then(|effect| tables::effect_slot(work.browser, work.session, effect));
            // One sentence in either case. The kit's own column explains both
            // where the point started and what choosing another does, so the
            // anchor sentence is drawn only when there is no kit to explain
            // it: for a missile or an area model, where it is the only
            // description of the point.
            let column = slot.and_then(|(_, field, _)| {
                schema::for_table("SpellVisualKit").and_then(|schema| schema.column(field))
            });
            let (colour, note) = match (slot, column, tables::slot_for_point(lab.point)) {
                (Some((_, _, kit)), Some(column), Some(_)) => (
                    theme::INK_FAINT,
                    format!(
                        "This point is kit {kit}'s {} column, which is why it started here. \
                         Choosing another of the six moves the effect to that column and the game \
                         hangs it there; one press of undo puts it back.",
                        column.name
                    ),
                ),
                // A kit hangs models from six attachments and no others: head,
                // chest, base and either hand. Any other point the body carries
                // is one the preview can hang a model from and nothing in the
                // tables can.
                (Some((_, _, kit)), _, None) => (
                    theme::WARN,
                    format!(
                        "No column of kit {kit} hangs from this point, so this is the preview \
                         only: in game the effect stays where the kit puts it."
                    ),
                ),
                _ => (theme::INK_FAINT, lab.anchor.clone()),
            };
            // The verdict first, in two words. The sentence after it is four
            // lines long in a narrow panel and was reported as easy to miss:
            // choosing a point that writes and one that does not looked the
            // same, so a choice that changed no table looked like a choice
            // that did not stick. See [`point_combo`], which groups the list
            // for the same reason.
            let writes = slot.is_some() && tables::slot_for_point(lab.point).is_some();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (word, mark) = match writes {
                    true => ("kit slot", theme::GOOD),
                    false => ("preview only", theme::WARN),
                };
                ui.label(egui::RichText::new(word).size(theme::SMALL).strong().color(mark));
                ui.label(egui::RichText::new(note).size(theme::SMALL).color(colour));
            });
            if let Some(report) = &lab.report {
                ui.label(egui::RichText::new(report).small().color(theme::WARN));
            }
            ui.add_space(6.0);

            // The export.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Export as").color(theme::INK_DIM));
                ui.add(
                    egui::TextEdit::singleline(&mut lab.export_path)
                        .desired_width(fits(ui, 300.0))
                        .font(egui::TextStyle::Monospace),
                );
            });
            ui.horizontal(|ui| {
                let can = lab.export_path.to_ascii_lowercase().ends_with(".m2")
                    && !lab.export_path.contains("..")
                    && lab.source().is_some();
                let bake = ui
                    .add_enabled(
                        can,
                        egui::Button::new(
                            egui::RichText::new("Bake & Apply")
                                .color(egui::Color32::from_rgb(0x0C, 0x16, 0x1E)),
                        )
                        .fill(theme::ACCENT),
                    )
                    .on_hover_text(
                        "Write the moved copy into the project under that path, and point this \
                         effect's Model at it. Only the offset is baked into the file — which \
                         attachment it hangs from is the point above, which writes the kit's own \
                         column. Publish packs the file into the patch archive.",
                    )
                    .on_disabled_hover_text("the export path has to end in .m2");
                if bake.clicked() {
                    export(work, lab, record);
                }
                // The way out without baking. It does not close the pane,
                // because the pane is what an effect row shows, so its hover
                // text says what is left on screen.
                if ui
                    .button("Close")
                    .on_hover_text(
                        "Leave the body and go back to the model by itself. Nothing is written \
                         and the numbers are kept, so pressing Position on character… again \
                         picks up where this left off.",
                    )
                    .clicked()
                {
                    lab.alone = true;
                }
            });
        });
}

/// The attachment points the chosen body carries, with the six that are also
/// a kit column listed first.
///
/// The attachment an effect hangs from is not a field of the effect: it is
/// which column of the kit names it (`spell::fields::EFFECTS`). Choosing one
/// of those six writes the tables, moving the effect between two columns of
/// one row. Choosing any of the body's other points moves the preview, and the
/// card says that is all it does.
///
/// The choice used to write nothing. It changed only the preview and the card
/// did not say so, so changing it, baking and coming back showed the point the
/// tables still stated. That was reported as the attachment point not
/// sticking.
fn point_combo(ui: &mut egui::Ui, work: &mut Workspace<'_>, lab: &mut Lab) {
    let mut point = lab.point;
    let named = |id: u32| super::storyboard::attachment_name(id);
    let wide = fits(ui, 200.0);
    let slot = lab
        .effect
        .and_then(|effect| tables::effect_slot(work.browser, work.session, effect));
    // Two groups under two headings, the ones that write first.
    //
    // A body carries some thirty attachment points and a kit's columns name
    // six of them. Choosing one of the other twenty-four moves the preview
    // and writes nothing. That is a valid choice, but the list has to make it
    // visible. The list used to be one run of names in id order, with a
    // `· kit slot` suffix on six of them and a grey sentence underneath, so
    // choosing a point that does not write looked the same as choosing one
    // that does, and finding the old point on return was read as the choice
    // not sticking.
    let writes = |id: u32| slot.is_some() && tables::slot_for_point(id).is_some();
    let (mut kit_slots, mut preview_only): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
    for (id, _) in &lab.points {
        match writes(*id) {
            true => kit_slots.push(*id),
            false => preview_only.push(*id),
        }
    }
    egui::ComboBox::from_id_salt("lab-point")
        .selected_text(named(point))
        .width(wide)
        .show_ui(ui, |ui| {
            if !kit_slots.is_empty() {
                ui.label(
                    egui::RichText::new("THE KIT'S OWN — these move the table")
                        .small()
                        .color(theme::INK_FAINT),
                );
                for id in &kit_slots {
                    ui.selectable_value(&mut point, *id, named(*id));
                }
                ui.separator();
            }
            ui.label(
                egui::RichText::new(match slot.is_some() {
                    true => "PREVIEW ONLY — no kit column hangs from these",
                    false => "PREVIEW ONLY — no single kit names this effect",
                })
                .small()
                .color(theme::INK_FAINT),
            );
            for id in &preview_only {
                ui.selectable_value(&mut point, *id, named(*id));
            }
        });
    // Which kind the chosen point is goes under the row, not beside it.
    // Placed beside the box, it was cut to `kit…` at the panel's edge: the
    // card's inner `Ui` reports more width than the panel shows (the reason
    // `lab.panel_width` exists, one file along), so room reserved from
    // `available_width` is not there. See [`verdict`], which is drawn at the
    // head of the note.
    if point == lab.point {
        return;
    }
    lab.point = point;
    // If both the old and the new point are kit columns, the tables move too.
    let (Some((record, from, kit)), Some(to)) = (slot, tables::slot_for_point(point)) else {
        return;
    };
    // Two columns share the base point. `BaseEffect` and `GroundEffect` both
    // hang at the feet (see `spell::fields::EFFECTS`), so an effect already in
    // one of them, chosen again as the base, would otherwise be moved into the
    // other without being asked.
    if tables::point_for_slot(from) == Some(point) {
        return;
    }
    let Some(effect) = lab.effect else { return };
    if tables::move_effect_slot(work.session, record, from, to, effect) {
        work.browser.forget_buffers();
        // The sentence on the card names the column this effect is in, so it
        // is stale the moment the column changes.
        let (anchor, _) = tables::effect_anchor(work.browser, work.session, effect);
        lab.anchor = anchor;
        work.session.status = format!(
            "kit {kit}: {} now hangs from {}",
            lab.effect_name,
            named(point)
        );
    }
}

/// The width a control can have: what is left of the row, and never more than
/// it asked for.
///
/// A width handed to a widget is a request, not a limit (`theme::segmented`
/// carries the same note for the same reason), so a fixed 300-point field
/// inside a 320-point panel does not shrink; it runs off the edge and is
/// clipped there. Every fixed width on this card had that problem, and a
/// 1280-wide window showed it: a combo box, a note and the export path were
/// all cut mid-glyph.
fn fits(ui: &egui::Ui, wanted: f32) -> f32 {
    wanted.min(ui.available_width() - 4.0).max(60.0)
}

fn labelled(ui: &mut egui::Ui, text: &str) {
    // Half the row at most, so the label never squeezes the value it names
    // out of the panel.
    let width = LABEL.min(ui.available_width() * 0.5).max(60.0);
    ui.add_sized(
        egui::vec2(width, 22.0),
        egui::Label::new(egui::RichText::new(text).color(theme::INK_DIM)).truncate(),
    );
}

/// One labelled number, laid out as one unit so a wrapping row keeps the
/// label with its value; `true` when it moved this frame.
fn number(ui: &mut egui::Ui, label: &str, value: &mut f32, speed: f64) -> bool {
    ui.horizontal(|ui| {
        labelled(ui, label);
        ui.add_sized(
            egui::vec2(fits(ui, VALUE), 22.0),
            egui::DragValue::new(value).speed(speed).max_decimals(3),
        )
        .changed()
    })
    .inner
}

/// Bake, write the copy into the project, and point the effect at it.
fn export(work: &mut Workspace<'_>, lab: &mut Lab, record: usize) {
    let Some(source) = lab.source().map(str::to_string) else {
        return;
    };
    let bytes = work
        .assets
        .with_archive(|chain| Ok(chain.read(&source).ok()))
        .ok()
        .flatten();
    let Some(bytes) = bytes else {
        lab.report = Some(format!("{source} is not in the archives"));
        return;
    };
    let baked = match vale_edit::m2::bake(&bytes, &lab.bake()) {
        Ok(baked) => baked,
        Err(e) => {
            lab.report = Some(e.to_string());
            return;
        }
    };
    let path = lab.export_path.trim().replace('/', "\\");
    let size = baked.len();
    if !work.session.save_bytes(&path, baked) {
        lab.report = Some(work.session.status.clone());
        return;
    }
    tables::set_text(
        work.session,
        "SpellVisualEffectName",
        record,
        fields::EFFECT_MODEL,
        &path,
        "Bake model offset",
        work.now,
    );
    work.browser.forget_buffers();
    work.session.status = format!("{size} bytes written to {path} and set as this effect's model");
    lab.close();
}

/// The pane: the picture, the arrows over it, and the drags. `work.now` is
/// the app's clock, the one `crate::lab::hang_the_effect` debounces against
/// — egui keeps a clock of its own and the two do not agree.
///
/// In the model view the arrows are not drawn and every press orbits; the
/// title says which model is on show, and the foot carries the switches.
pub fn pane(ui: &mut egui::Ui, work: &mut Workspace<'_>, stage: &mut Stage, lab: &mut Lab) {
    let now = work.now;
    stage.showing = None;
    stage.lab = true;
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);
    let mut pane = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(all.shrink(6.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    pane.horizontal(|ui| {
        let title = match (lab.alone, &lab.preview) {
            (_, Some(path)) => format!("Model — {} (previewing)", tables::basename(path)),
            (true, None) => format!("Model — {}", lab.effect_name),
            (false, None) => format!("Attachment lab — {}", lab.effect_name),
        };
        ui.label(egui::RichText::new(title).color(theme::INK_DIM));
        if let Some(source) = lab.showing() {
            ui.add(
                egui::Label::new(egui::RichText::new(source).small().color(theme::INK_FAINT))
                    .truncate(),
            );
        }
    });
    // The counts on a line of their own: beside the title they overlapped it
    // on a pane narrower than the two together.
    if let Some(facts) = &lab.facts {
        pane.label(
            egui::RichText::new(facts.line())
                .small()
                .color(theme::INK_FAINT),
        );
    }

    // The foot wraps, so a narrow pane stacks its switches rather than
    // clipping them; the room left for it is what two wrapped rows take.
    let foot = match lab.alone {
        true => 96.0,
        false => 74.0,
    };
    let picture = egui::vec2(
        pane.available_width(),
        (pane.available_height() - foot).max(60.0),
    );
    let ppp = pane.ctx().pixels_per_point();
    stage.pane = Some(bevy::prelude::UVec2::new(
        (picture.x * ppp) as u32,
        (picture.y * ppp) as u32,
    ));

    match stage.texture {
        Some(id) => {
            let shown = pane.add(
                egui::Image::new(egui::load::SizedTexture::new(id, picture))
                    .sense(egui::Sense::click_and_drag()),
            );
            let rect = shown.rect;
            let to_pane = |ndc: Vec2| -> egui::Pos2 {
                egui::pos2(
                    rect.left() + (ndc.x + 1.0) * 0.5 * rect.width(),
                    rect.top() + (1.0 - ndc.y) * 0.5 * rect.height(),
                )
            };
            let shift = pane.ctx().input(|input| input.modifiers.shift);

            // Which handle a press lands on, decided on the press and held.
            // The model view has no handles, so every press orbits.
            if shown.drag_started() {
                lab.dragging = match (lab.alone, lab.handles, shown.interact_pointer_pos()) {
                    (true, _, _) => None,
                    (false, Some(handles), Some(at)) => grabbed(&handles, at, &to_pane),
                    _ => None,
                }
                .or((shift && !lab.alone).then_some(Drag::Free));
            }
            if shown.dragged() {
                let delta = shown.drag_delta();
                match (lab.dragging, lab.handles) {
                    (Some(drag), Some(handles)) if rect.width() > 0.0 && rect.height() > 0.0 => {
                        let dndc =
                            Vec2::new(delta.x / rect.width() * 2.0, -delta.y / rect.height() * 2.0);
                        move_by(lab, &handles, drag, dndc);
                        lab.touched(now);
                    }
                    _ => stage.turn(delta),
                }
            }
            if shown.drag_stopped() {
                lab.dragging = None;
            }

            stage.pointer = None;
            if shown.hovered() {
                let wheel = pane.ctx().input(|input| input.smooth_scroll_delta.y);
                if wheel != 0.0 {
                    stage.zoom(wheel);
                }
                if let Some(at) = shown.hover_pos() {
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        let ndc = Vec2::new(
                            (at.x - rect.left()) / rect.width() * 2.0 - 1.0,
                            1.0 - (at.y - rect.top()) / rect.height() * 2.0,
                        );
                        stage.pointer = Some((ndc, rect.width() / rect.height()));
                    }
                }
            }

            // The arrows, over the picture.
            if let Some(handles) = lab.handles {
                let painter = pane.painter_at(rect);
                let origin = to_pane(handles.origin);
                let hot = shown
                    .hover_pos()
                    .and_then(|at| grabbed(&handles, at, &to_pane));
                let colours = [theme::BAD, theme::GOOD, theme::ACCENT];
                let names = ["X", "Y", "Z"];
                for axis in 0..3 {
                    let tip = to_pane(handles.tips[axis]);
                    let lit = lab.dragging == Some(Drag::Axis(axis))
                        || (lab.dragging.is_none() && hot == Some(Drag::Axis(axis)));
                    let width = match lit {
                        true => 3.5,
                        false => 2.0,
                    };
                    painter.line_segment([origin, tip], egui::Stroke::new(width, colours[axis]));
                    painter.circle_filled(tip, width + 2.0, colours[axis]);
                    painter.text(
                        tip + egui::vec2(6.0, -6.0),
                        egui::Align2::LEFT_BOTTOM,
                        names[axis],
                        egui::FontId::proportional(theme::SMALL),
                        colours[axis],
                    );
                }
                painter.circle_stroke(origin, 4.0, egui::Stroke::new(1.5, theme::INK));
            }
        }
        None => {
            pane.allocate_space(picture);
        }
    }

    pane.add_space(4.0);
    pane.horizontal_wrapped(|ui| {
        // The switches are the world's own. Wireframe is the debug overlay's
        // and particles is the view bar's subtraction; the pane toggles the
        // same two the bar does, so a view left in wireframe here is in
        // wireframe there, and the bar's lit button shows it.
        match work.wireframe.as_deref_mut() {
            Some(wireframe) => {
                ui.checkbox(wireframe, "wireframe")
                    .on_hover_text("every mesh as its edges — the view bar's own overlay");
            }
            None => {
                ui.add_enabled(false, egui::Checkbox::new(&mut false, "wireframe"))
                    .on_disabled_hover_text("built without diagnostics");
            }
        }
        ui.checkbox(&mut stage.spinning, "spin");
        if let Some(particles) = work.particles.as_deref_mut() {
            ui.checkbox(particles, "particles")
                .on_hover_text("the emitters — the view bar's own switch");
        }
        if ui.button("Reset view").clicked() {
            stage.yaw = 0.9;
            stage.pitch = 0.1;
            stage.distance = 5.0;
        }
        if lab.alone {
            if let Some(facts) = &lab.facts {
                sequences(ui, work, facts);
            }
        } else {
            ui.label(
                egui::RichText::new(format!(
                    "arrows are {ARROW_YARDS} yd · drag to orbit · wheel to zoom · offsets are relative to the chosen point"
                ))
                .small()
                .color(theme::INK_FAINT),
            );
        }
    });
    if lab.alone {
        pane.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "hung at {} on a {} · drag to orbit · wheel to zoom",
                    super::storyboard::attachment_name(lab.point),
                    BODIES[lab.body].name
                ))
                .small()
                .color(theme::INK_FAINT),
            );
            if let Some(report) = &lab.report {
                ui.label(egui::RichText::new(report).small().color(theme::WARN));
            }
        });
    }
    ui.allocate_rect(all, egui::Sense::hover());
}

/// `plays Stand (2.7 s) · 3 anims`, with every sequence named on hover.
///
/// A label rather than a selector: an attached model plays the one sequence
/// `hang_model` chooses, and a control that changed nothing would say
/// otherwise. What is listed is what the file carries and which of it the
/// game uses.
fn sequences(ui: &mut egui::Ui, work: &mut Workspace<'_>, facts: &crate::lab::Facts) {
    if facts.sequences.is_empty() {
        ui.label(
            egui::RichText::new("no animation")
                .small()
                .color(theme::INK_FAINT),
        );
        return;
    }
    let name = |id: u16| -> String {
        work.session
            .table("AnimationData")
            .and_then(|table| table.row_of(u32::from(id)))
            .and_then(|row| work.session.table("AnimationData")?.string_at(row, 1))
            .unwrap_or_else(|| format!("anim {id}"))
    };
    let describe = |(id, variation, millis, loops): &(u16, u16, u32, bool)| -> String {
        let take = match variation {
            0 => String::new(),
            n => format!(" #{n}"),
        };
        let looped = match loops {
            true => "",
            false => ", once",
        };
        format!(
            "{}{take} ({:.1} s{looped})",
            name(*id),
            *millis as f32 / 1000.0
        )
    };
    let line = match facts.plays.and_then(|at| facts.sequences.get(at)) {
        Some(played) => format!(
            "plays {} · {} anims",
            describe(played),
            facts.sequences.len()
        ),
        None => format!("{} anims, none with any length", facts.sequences.len()),
    };
    let all: Vec<String> = facts
        .sequences
        .iter()
        .enumerate()
        .map(|(at, one)| match facts.plays == Some(at) {
            true => format!("▶ {}", describe(one)),
            false => format!("   {}", describe(one)),
        })
        .collect();
    ui.label(egui::RichText::new(line).small().color(theme::INK_FAINT))
        .on_hover_text(all.join("\n"));
}

/// Which arrow, if any, a pane position is on.
fn grabbed(
    handles: &Handles,
    at: egui::Pos2,
    to_pane: &impl Fn(Vec2) -> egui::Pos2,
) -> Option<Drag> {
    let origin = to_pane(handles.origin);
    let mut best: Option<(f32, usize)> = None;
    for axis in 0..3 {
        let tip = to_pane(handles.tips[axis]);
        let d = distance_to_segment(at, origin, tip);
        if d <= GRIP && best.is_none_or(|(had, _)| d < had) {
            best = Some((d, axis));
        }
    }
    best.map(|(_, axis)| Drag::Axis(axis))
}

fn distance_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let len2 = ab.length_sq();
    let t = match len2 > 0.0 {
        true => (ap.dot(ab) / len2).clamp(0.0, 1.0),
        false => 0.0,
    };
    (a + ab * t - p).length()
}

/// Move the offset by a pane delta, along an axis or across the screen.
fn move_by(lab: &mut Lab, handles: &Handles, drag: Drag, dndc: Vec2) {
    match drag {
        Drag::Axis(axis) => {
            let along = handles.per_yard[axis];
            let len2 = along.length_squared();
            if len2 > 1e-9 {
                lab.offset[axis] += dndc.dot(along) / len2;
            }
        }
        Drag::Free => {
            let moved = handles.right_step * dndc.x + handles.up_step * dndc.y;
            lab.offset[0] += moved.x;
            lab.offset[1] += moved.y;
            lab.offset[2] += moved.z;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handles() -> Handles {
        Handles {
            origin: Vec2::ZERO,
            tips: [
                Vec2::new(0.3, 0.0),
                Vec2::new(0.0, 0.3),
                Vec2::new(0.0, 0.0),
            ],
            per_yard: [Vec2::new(0.5, 0.0), Vec2::new(0.0, 0.5), Vec2::ZERO],
            right_step: Vec2::new(2.0, 0.0).extend(0.0),
            up_step: Vec2::new(0.0, 0.0).extend(2.0),
        }
    }

    /// A drag along an axis moves by the yards the pane delta comes to along
    /// that axis, and an axis that projects to nothing moves nothing.
    #[test]
    fn an_axis_drag_moves_in_yards_along_that_axis() {
        let mut lab = Lab::default();
        move_by(&mut lab, &handles(), Drag::Axis(0), Vec2::new(0.25, 0.7));
        assert!((lab.offset[0] - 0.5).abs() < 1e-6);
        assert_eq!(lab.offset[1], 0.0);
        move_by(&mut lab, &handles(), Drag::Axis(2), Vec2::new(0.25, 0.7));
        assert_eq!(
            lab.offset[2], 0.0,
            "an axis edge-on to the camera cannot be dragged"
        );
    }

    #[test]
    fn a_free_drag_moves_across_the_screen() {
        let mut lab = Lab::default();
        move_by(&mut lab, &handles(), Drag::Free, Vec2::new(0.5, -0.25));
        assert_eq!(lab.offset, [1.0, 0.0, -0.5]);
    }

    #[test]
    fn a_press_near_an_arrow_grabs_it_and_one_far_away_does_not() {
        let to_pane = |ndc: Vec2| egui::pos2(100.0 + ndc.x * 100.0, 100.0 - ndc.y * 100.0);
        let h = handles();
        assert_eq!(
            grabbed(&h, egui::pos2(115.0, 103.0), &to_pane),
            Some(Drag::Axis(0))
        );
        assert_eq!(
            grabbed(&h, egui::pos2(102.0, 80.0), &to_pane),
            Some(Drag::Axis(1))
        );
        assert_eq!(grabbed(&h, egui::pos2(160.0, 160.0), &to_pane), None);
    }
}
