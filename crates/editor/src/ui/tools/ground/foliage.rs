//! The Foliage field the ground panels share, and the Foliage window it opens.
//!
//! ```text
//! field     one line in a panel: the set's models as pictures and by name.
//!           A click opens the window for what the field belongs to
//! window    the sets on the left, filtered and searched; on the right the
//!           chosen set's four models, a turnable picture of one of them, its
//!           density, how many layers grow it, Apply, and for a project set
//!           the model list that changes a column
//! ```
//!
//! A field opens the window with a [`Target`]: the texture brush, one layer
//! of a pinned chunk, the road's surface or verge, or a texture in the chunk
//! tool's selection. Apply writes the shown set there. The window replaces a
//! drop-down of row numbers: a row number does not say what grows, and the
//! pictures do.
//!
//! The state is [`crate::tools::ground::foliage`], which says why shipped
//! sets are read-only and how a project set reaches the ground.

use bevy_egui::egui;

use crate::preview::portraits::{self, Portraits};
use crate::session::EditSession;
use crate::tools::ground::foliage::{self, Catalogue, Filter, Foliage, Set, Target};
use crate::tools::ground::textures;
use crate::ui::widgets::theme::{self, property};

/// The side of a model picture in a [`field`], in the window's list, and in
/// the window's model list.
const FIELD_PICTURE: f32 = 20.0;
const ROW_PICTURE: f32 = 30.0;
const MODEL_PICTURE: f32 = 52.0;
/// The side of a column's picture on the window's right.
const SLOT_PICTURE: f32 = 64.0;
/// The width of the window's list of sets.
const LIST_WIDTH: f32 = 330.0;

/// One line naming the sets `ids` grow: their models' pictures and names.
/// More than one id is a mixed selection and reads "Mixed: n sets". Answers
/// whether it was clicked; a disabled field shows `disabled` on hover.
pub(in crate::ui) fn field(
    ui: &mut egui::Ui,
    catalogue: Option<&Catalogue>,
    portraits: &mut Portraits,
    ids: &[u32],
    enabled: bool,
    disabled: &str,
) -> bool {
    let height = FIELD_PICTURE + 6.0;
    let sense = match enabled {
        true => egui::Sense::click(),
        false => egui::Sense::hover(),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
    let first = ids.first().copied().unwrap_or(0);
    let set = catalogue.and_then(|catalogue| catalogue.set(first));
    let text = match (ids.len(), catalogue) {
        (n, _) if n > 1 => format!("Mixed: {n} sets"),
        (_, Some(catalogue)) => catalogue.describe(first),
        (_, None) => format!("Row {first}"),
    };
    if ui.is_rect_visible(rect) {
        let fill = match (enabled, response.hovered()) {
            (false, _) => theme::DEAD,
            (true, true) => theme::RAISED_HOT,
            (true, false) => theme::RAISED,
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(3), fill);
        ui.painter()
            .rect_stroke(rect, egui::CornerRadius::same(3), egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
        let mut x = rect.left() + 3.0;
        if let (Some(set), Some(catalogue)) = (set, catalogue) {
            for model in distinct(set) {
                if let Some(model) = catalogue.model(model) {
                    let square = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 3.0), egui::Vec2::splat(FIELD_PICTURE));
                    portraits::paint(ui, portraits, &model.path, square);
                    x = square.right() + 2.0;
                }
            }
        }
        let clip = egui::Rect::from_min_max(egui::pos2(x + 4.0, rect.top()), egui::pos2(rect.right() - 18.0, rect.bottom()));
        ui.painter().with_clip_rect(clip).text(
            egui::pos2(x + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &text,
            egui::FontId::proportional(theme::SMALL + 0.5),
            if enabled { theme::INK } else { theme::INK_FAINT },
        );
        ui.painter().text(
            egui::pos2(rect.right() - 7.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            "\u{2026}",
            egui::FontId::proportional(theme::BODY),
            theme::INK_DIM,
        );
    }
    let about = match (ids.len(), set) {
        (n, _) if n > 1 => format!("The layers grow {n} different sets. Click to choose one set for all of them."),
        (_, Some(set)) => format!(
            "{text}\nRow {} \u{b7} {} per cell\nClick to choose, preview or edit the foliage.",
            set.id, set.density
        ),
        _ => format!("{text}\nClick to choose the foliage."),
    };
    let response = match enabled {
        true => response.on_hover_text(about),
        false => response.on_hover_text(disabled),
    };
    response.clicked()
}

/// A set's models, each once, in column order.
fn distinct(set: &Set) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for &model in set.slots.iter().flatten() {
        if !out.contains(&model) {
            out.push(model);
        }
    }
    out
}

/// What a target is, in the window's subtitle.
fn target_name(target: &Target, texture: &str) -> String {
    let leaf = textures::leaf(texture);
    match target {
        Target::Brush => format!("The texture brush ({leaf})"),
        Target::Layer { chunk, layer, .. } => format!("Layer {layer} of chunk {chunk} ({leaf})"),
        Target::RoadSurface => format!("The road's surface ({leaf})"),
        Target::RoadVerge => format!("The road's verge ({leaf})"),
        Target::Selection { texture: Some(_) } => format!("{leaf} in the selected chunks"),
        Target::Selection { texture: None } => "Every layer of the selected chunks".to_string(),
    }
}

/// The set a target grows now, when it is one set.
fn current(target: &Target, editing: &crate::ui::Editing<'_>, session: &EditSession) -> Option<u32> {
    match target {
        Target::Brush => Some(editing.textures.brush.effect_id),
        Target::Layer { coord, chunk, layer } => {
            textures::layers_of(session, *coord, *chunk)?.get(*layer).map(|layer| layer.effect_id)
        }
        Target::RoadSurface => Some(editing.road.surface.effect_id),
        Target::RoadVerge => Some(editing.road.verge.effect_id),
        Target::Selection { .. } => None,
    }
}

/// Write set `id` to the target.
fn apply(target: &Target, id: u32, editing: &mut crate::ui::Editing<'_>, session: &mut EditSession, now: f64) {
    match target {
        Target::Brush => editing.textures.brush.effect_id = id,
        Target::Layer { coord, chunk, layer } => textures::set_layer_effect(session, *coord, *chunk, *layer, id, now),
        Target::RoadSurface => editing.road.surface.effect_id = id,
        Target::RoadVerge => editing.road.verge.effect_id = id,
        Target::Selection { texture } => {
            let changed =
                crate::tools::ground::chunks::set_foliage(session, &editing.chunks.selected, texture.as_deref(), id);
            session.status = format!("foliage set on {changed} chunk(s)");
        }
    }
}

/// The Foliage window, while a field has opened it. Drawn on the root context
/// from `ui::draw`, because it is larger than the inspector. Answers its
/// rectangle, which the shell adds to the areas a press on the world does not
/// reach.
pub(crate) fn window(
    ctx: &egui::Context,
    editing: &mut crate::ui::Editing<'_>,
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
    now: f64,
) -> Option<egui::Rect> {
    let target = editing.foliage.target.clone()?;
    let Some(catalogue) = editing.foliage.catalogue(session, assets) else {
        editing.foliage.close();
        return None;
    };
    let mut open = true;
    let mut applied = None;
    let shown = theme::window("Foliage", "foliage-window")
        .open(&mut open)
        .default_size([720.0, 640.0])
        .min_width(600.0)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!("Applies to: {}", target_name(&target, &editing.foliage.texture)))
                    .size(theme::SMALL)
                    .color(theme::INK_DIM),
            );
            ui.add_space(4.0);
            let current = current(&target, editing, session);
            let height = ui.available_height().max(320.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(LIST_WIDTH, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(LIST_WIDTH);
                        if let Some(id) = list(ui, &mut editing.foliage, &catalogue, &mut editing.portraits, current) {
                            applied = Some(id);
                        }
                    },
                );
                ui.separator();
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt("foliage-detail").show(ui, |ui| {
                        if let Some(id) = detail(ui, &mut editing.foliage, &catalogue, &mut editing.portraits, session, current, now) {
                            applied = Some(id);
                        }
                    });
                });
            });
        });
    if let Some(id) = applied {
        apply(&target, id, editing, session, now);
        editing.foliage.recount(session);
    }
    if !open {
        editing.foliage.close();
    }
    shown.map(|shown| shown.response.rect)
}

/// The window's left: the search, the filter, and the sets. Answers a set
/// double-clicked, which applies it.
fn list(
    ui: &mut egui::Ui,
    foliage: &mut Foliage,
    catalogue: &Catalogue,
    portraits: &mut Portraits,
    current: Option<u32>,
) -> Option<u32> {
    ui.add(
        egui::TextEdit::singleline(&mut foliage.search)
            .hint_text("Search models or row ID")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(2.0);
    let mut filters: Vec<(&str, Filter)> = Vec::new();
    if !foliage.texture.is_empty() {
        filters.push(("This Texture", Filter::Texture));
    }
    filters.extend([("Open Tiles", Filter::Open), ("Project", Filter::Project), ("All", Filter::All)]);
    theme::segmented(ui, &mut foliage.filter, &filters, |a, b| a == b);

    // The ids the filter lists, most used first where there is a count.
    let ids: Vec<u32> = match (foliage.filter, &foliage.counts) {
        (Filter::Texture, Some(counts)) => counts.with.iter().map(|(id, _)| *id).collect(),
        (Filter::Open, Some(counts)) => counts.with.iter().chain(&counts.elsewhere).map(|(id, _)| *id).collect(),
        (Filter::Project, _) => catalogue.sets.iter().filter(|set| set.project).map(|set| set.id).collect(),
        _ => catalogue.listed().map(|set| set.id).collect(),
    };
    let needle = foliage.search.trim().to_ascii_lowercase();
    let ids: Vec<u32> = ids
        .into_iter()
        .filter(|&id| {
            needle.is_empty()
                || id.to_string().starts_with(&needle)
                || catalogue.describe(id).to_ascii_lowercase().contains(&needle)
        })
        .collect();
    theme::note(
        ui,
        match (foliage.filter, ids.len()) {
            (Filter::Texture, 0) => "No layer of this texture in the open tiles grows anything.".to_string(),
            (Filter::Project, 0) => "The project has no sets. Copy one with Copy as New Set.".to_string(),
            (_, 1) => "1 set".to_string(),
            (_, n) => format!("{n} sets"),
        },
    );

    let mut applied = None;
    // None comes first and is always listed.
    if let Some(id) = row(ui, foliage, catalogue, portraits, 0, current) {
        applied = Some(id);
    }
    let height = ROW_PICTURE + 8.0;
    egui::ScrollArea::vertical()
        .id_salt("foliage-sets")
        .auto_shrink([false, false])
        .show_rows(ui, height, ids.len(), |ui, range| {
            for &id in &ids[range] {
                if let Some(id) = row(ui, foliage, catalogue, portraits, id, current) {
                    applied = Some(id);
                }
            }
        });
    applied
}

/// One set in the list: its pictures, its models, and its row, density and
/// use. A click shows it; a double click applies it.
fn row(
    ui: &mut egui::Ui,
    foliage: &mut Foliage,
    catalogue: &Catalogue,
    portraits: &mut Portraits,
    id: u32,
    current: Option<u32>,
) -> Option<u32> {
    let height = ROW_PICTURE + 8.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    let on = foliage.shown.unwrap_or(0) == id;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if on || response.hovered() {
            painter.rect_filled(rect, egui::CornerRadius::same(3), if on { theme::ACCENT_SUNK } else { theme::RAISED });
        }
        if on {
            painter.rect_stroke(rect, egui::CornerRadius::same(3), egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
        }
        let set = catalogue.set(id);
        let mut x = rect.left() + 4.0;
        for slot in 0..4 {
            let square = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 4.0), egui::Vec2::splat(ROW_PICTURE));
            match set.and_then(|set| set.slots[slot]).and_then(|model| catalogue.model(model)) {
                Some(model) => portraits::paint(ui, portraits, &model.path, square),
                None => {
                    painter.rect_stroke(square, egui::CornerRadius::same(3), egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
                }
            }
            x = square.right() + 2.0;
        }
        let text = painter.with_clip_rect(egui::Rect::from_min_max(egui::pos2(x + 6.0, rect.top()), rect.right_bottom()));
        text.text(
            egui::pos2(x + 6.0, rect.center().y - 7.0),
            egui::Align2::LEFT_CENTER,
            catalogue.describe(id),
            egui::FontId::proportional(12.5),
            if on { theme::INK } else { theme::INK_DIM },
        );
        let mut facts = match (id, set) {
            (0, _) => "Footsteps: Dirt".to_string(),
            (_, Some(set)) if set.plants() => format!("Row {id} \u{b7} {} per cell", set.density),
            (_, Some(_)) => format!("Row {id} \u{b7} Footsteps: {}", catalogue.footsteps(id)),
            (_, None) => format!("Row {id}"),
        };
        // The window's right gives both counts; the row gives the one the
        // filter is about.
        let (with, all) = foliage.uses(id);
        let layers = match foliage.filter {
            Filter::Texture => with,
            _ => all,
        };
        if layers > 0 {
            facts.push_str(&format!(" \u{b7} {layers} layers"));
        }
        if set.is_some_and(|set| set.project) {
            facts.push_str(" \u{b7} Project");
        }
        // The set the target grows now: a bar down the row's left edge, which
        // a long line of facts cannot push out of sight as a word could.
        if current == Some(id) {
            let bar = egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 3.0), egui::vec2(3.0, rect.height() - 6.0));
            painter.rect_filled(bar, egui::CornerRadius::same(1), theme::ACCENT);
        }
        text.text(
            egui::pos2(x + 6.0, rect.center().y + 8.0),
            egui::Align2::LEFT_CENTER,
            facts,
            egui::FontId::proportional(theme::SMALL),
            theme::INK_FAINT,
        );
    }
    if response.clicked() {
        foliage.shown = (id != 0).then_some(id);
        foliage.slot = None;
    }
    response.double_clicked().then_some(id)
}

/// The window's right: the shown set. Answers the set to apply.
fn detail(
    ui: &mut egui::Ui,
    foliage: &mut Foliage,
    catalogue: &Catalogue,
    portraits: &mut Portraits,
    session: &mut EditSession,
    current: Option<u32>,
    now: f64,
) -> Option<u32> {
    let mut applied = None;
    let Some(id) = foliage.shown else {
        theme::heading(ui, "None");
        theme::note(ui, "The layer grows no foliage, and footsteps on it sound as Dirt.");
        ui.add_space(6.0);
        if theme::primary(ui, "Apply None").clicked() {
            applied = Some(0);
        }
        ui.add_space(6.0);
        if ui
            .button("New Empty Set")
            .on_hover_text("Make a project set with no models, to fill from the model list.")
            .clicked()
        {
            foliage.shown = foliage::new_set(session, None);
            foliage.slot = Some(0);
        }
        return applied;
    };
    let Some(set) = catalogue.set(id).cloned() else {
        theme::heading(ui, &format!("Row {id}"));
        theme::note(ui, "GroundEffectTexture has no such row. A layer that names it grows nothing.");
        return None;
    };

    theme::heading(ui, &format!("Set {id}"));
    property(ui, "Source", "", |ui| {
        ui.label(match set.project {
            true => "Project. Saved with the project's tables.",
            false => "Shipped. Read-only; copy it to change it.",
        });
    });
    let (with, all) = foliage.uses(id);
    property(ui, "Grown By", "Layers in the open tiles whose MCLY effectId is this row.", |ui| {
        ui.label(match (foliage.texture.is_empty(), all) {
            (_, 0) => "No layer in the open tiles".to_string(),
            (false, _) => format!("{all} layers, {with} of this texture"),
            (true, _) => format!("{all} layers"),
        });
    });
    ui.add_space(4.0);
    let label = match current == Some(id) {
        true => "Applied",
        false => "Apply Set",
    };
    if theme::primary(ui, label).on_hover_text("Double-click a set in the list to do the same.").clicked() {
        applied = Some(id);
    }

    theme::heading(ui, "Models");
    // The four columns, each a picture. A click picks the column the large
    // picture shows and, on a project set, the one the model list sets.
    ui.horizontal(|ui| {
        for slot in 0..4 {
            let model = set.slots[slot].and_then(|model| catalogue.model(model));
            let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(SLOT_PICTURE), egui::Sense::click());
            match model {
                Some(model) => portraits::paint(ui, portraits, &model.path, rect),
                None => {
                    ui.painter().rect_filled(rect, egui::CornerRadius::same(3), theme::SUNK);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Empty",
                        egui::FontId::proportional(theme::SMALL),
                        theme::INK_FAINT,
                    );
                }
            }
            let chosen = foliage.slot == Some(slot);
            ui.painter().rect_stroke(
                rect,
                egui::CornerRadius::same(3),
                egui::Stroke::new(1.0, if chosen { theme::ACCENT } else { theme::LINE }),
                egui::StrokeKind::Inside,
            );
            let about = match (model, set.project) {
                (Some(model), true) => format!("Column {}: {}\nClick to show it and change it.", slot + 1, model.path),
                (Some(model), false) => format!("Column {}: {}", slot + 1, model.path),
                (None, true) => format!("Column {}: empty. Click to choose a model.", slot + 1),
                (None, false) => format!("Column {}: empty. Its share of the tufts is not planted.", slot + 1),
            };
            if response.on_hover_text(about).clicked() {
                foliage.slot = Some(slot);
            }
        }
    });
    let shown_slot = foliage.slot.or_else(|| set.slots.iter().position(Option::is_some));
    if let Some(model) = shown_slot.and_then(|slot| set.slots[slot]).and_then(|model| catalogue.model(model)) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&model.name).color(theme::INK));
        let pane = portraits::pane_id(ui);
        let side = ui.available_width().min(200.0);
        portraits::paint_large(ui, portraits, pane, &model.path, side)
            .on_hover_text("Drag to turn the model.");
    }
    theme::note(
        ui,
        "A cell's tufts take the four columns in turn, so a model in two columns grows twice as \
         often, and an empty column leaves its share bare.",
    );

    property(
        ui,
        "Footsteps",
        "GroundEffectTexture terrainType: the TerrainType row that picks the footstep sound on \
         the layer. A layer with no set walks as Dirt.",
        |ui| match set.project {
            true => {
                let mut terrain = set.terrain;
                egui::ComboBox::from_id_salt("foliage-terrain")
                    .selected_text(catalogue.footsteps(id))
                    .show_ui(ui, |ui| {
                        for (row, name) in &catalogue.terrains {
                            ui.selectable_value(&mut terrain, *row, name);
                        }
                    });
                if terrain != set.terrain {
                    foliage::set_terrain(session, id, terrain);
                }
            }
            false => {
                ui.label(catalogue.footsteps(id));
            }
        },
    );
    property(ui, "Density", "How many tufts one draw of a detail cell plants. The shipped sets use 1 to 20.", |ui| {
        let mut density = set.density;
        let response = ui.add_enabled(
            set.project,
            egui::DragValue::new(&mut density).speed(0.1).range(foliage::DENSITIES).suffix(" per cell"),
        );
        if response.changed() {
            foliage::set_density(session, id, density, now);
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .button("Copy as New Set")
            .on_hover_text("Make a project set with this set's models and density, and show it.")
            .clicked()
        {
            if let Some(copy) = foliage::new_set(session, Some(id)) {
                foliage.shown = Some(copy);
                foliage.filter = Filter::Project;
            }
        }
        if set.project
            && ui
                .button("Remove Set")
                .on_hover_text("Remove this project set. Layers that grow it then grow nothing.")
                .clicked()
            && foliage::remove_set(session, id)
        {
            foliage.shown = None;
        }
    });

    if set.project {
        if let Some(slot) = foliage.slot {
            model_list(ui, foliage, catalogue, portraits, session, id, slot);
        } else {
            theme::note(ui, "Click a column above to change its model.");
        }
    }
    applied
}

/// The models a project set's column can name, as a grid of pictures, with
/// Empty first.
fn model_list(
    ui: &mut egui::Ui,
    foliage: &mut Foliage,
    catalogue: &Catalogue,
    portraits: &mut Portraits,
    session: &mut EditSession,
    id: u32,
    slot: usize,
) {
    theme::heading(ui, &format!("Model for Column {}", slot + 1));
    ui.add(
        egui::TextEdit::singleline(&mut foliage.model_search)
            .hint_text("Search models")
            .desired_width(f32::INFINITY),
    );
    let needle = foliage.model_search.trim().to_ascii_lowercase();
    let models: Vec<usize> = (0..catalogue.models.len())
        .filter(|&index| needle.is_empty() || catalogue.models[index].name.to_ascii_lowercase().contains(&needle))
        .collect();
    theme::note(ui, format!("{} of {} models", models.len(), catalogue.models.len()));
    let current = catalogue.set(id).and_then(|set| set.slots[slot]);
    let cell = MODEL_PICTURE + 18.0;
    let across = ((ui.available_width() / (MODEL_PICTURE + 6.0)).floor() as usize).max(1);
    // Empty is the first cell, then the models.
    let count = models.len() + 1;
    let mut picked: Option<Option<u32>> = None;
    egui::ScrollArea::vertical()
        .id_salt("foliage-models")
        .max_height(260.0)
        .auto_shrink([false, true])
        .show_rows(ui, cell, count.div_ceil(across), |ui, range| {
            for line in range {
                ui.horizontal(|ui| {
                    for at in line * across..((line + 1) * across).min(count) {
                        let model = at.checked_sub(1).map(|at| models[at]);
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(MODEL_PICTURE, cell), egui::Sense::click());
                        let square = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(MODEL_PICTURE));
                        let name = match model.and_then(|model| catalogue.model(model)) {
                            Some(found) => {
                                portraits::paint(ui, portraits, &found.path, square);
                                found.name.clone()
                            }
                            None => {
                                ui.painter().rect_filled(square, egui::CornerRadius::same(3), theme::SUNK);
                                "Empty".to_string()
                            }
                        };
                        let on = model == current;
                        ui.painter().rect_stroke(
                            square,
                            egui::CornerRadius::same(3),
                            egui::Stroke::new(1.0, if on { theme::ACCENT } else if response.hovered() { theme::INK_DIM } else { theme::LINE }),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().with_clip_rect(rect).text(
                            egui::pos2(rect.center().x, square.bottom() + 8.0),
                            egui::Align2::CENTER_CENTER,
                            &name,
                            egui::FontId::proportional(10.5),
                            if on { theme::INK } else { theme::INK_DIM },
                        );
                        if response.on_hover_text(name).clicked() {
                            picked = Some(model.and_then(|model| catalogue.model(model)).map(|model| model.id));
                        }
                    }
                });
            }
        });
    if let Some(model) = picked {
        foliage::set_model(session, id, slot, model);
    }
}
