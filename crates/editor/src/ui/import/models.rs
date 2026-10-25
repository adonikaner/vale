//! The Import Model dialog. What it writes and why is
//! [`crate::import::models`].
//!
//! It is drawn by the shell under any tool, as the texture dialog is
//! ([`super::assets::import_dialog`] draws both), and is asked for by the
//! Assets workspace's Import and Re-import.
//!
//! Its rows, in order: what the model is for, where it goes and under what
//! name, its size and whether it is solid, its animations, and the texture
//! each material draws.

use bevy_egui::egui;

use crate::import::assets::{folder_of, Ask, Assets};
use crate::import::models::{ModelImport, ModelPurpose, CREATURE_ROOT, DOODAD_ROOT};
use crate::session::EditSession;
use crate::ui::widgets::theme;
use vale_assets::tables::dbc::AnimationData;
use vale_edit::m2::write::Collision;

/// Yards per metre. glTF is in metres and the game in yards.
const YARDS_PER_METRE: f32 = 1.0 / 0.9144;

/// The Import Model dialog, when one is open.
pub(crate) fn model_dialog(ctx: &egui::Context, assets: &mut Assets, session: &EditSession, game: &vale_client::assets::GameAssets) {
    let Some(import) = assets.model.as_mut() else { return };
    let tables = game.display_tables().ok();
    let animations = tables.as_deref().and_then(|t| t.animations());
    let mut close = false;
    let mut write = false;
    let response = egui::Modal::new(egui::Id::new("import-model")).show(ctx, |ui| {
        ui.set_width(660.0);
        ui.label(egui::RichText::new("Import Model").strong().size(15.0));
        ui.label(
            egui::RichText::new(import.source_path.display().to_string())
                .small()
                .color(theme::INK_DIM),
        );
        let source = &import.source;
        let count = |n: usize, one: &str, many: &str| match n {
            1 => format!("1 {one}"),
            n => format!("{n} {many}"),
        };
        ui.label(
            [
                count(source.vertices, "vertex", "vertices"),
                count(source.triangles, "triangle", "triangles"),
                count(source.bones.len(), "bone", "bones"),
                count(source.clips.len(), "animation", "animations"),
                count(source.slots.len(), "material", "materials"),
            ]
            .join(", "),
        );
        for note in &source.notes {
            ui.label(egui::RichText::new(note).small().color(theme::INK_DIM));
        }
        ui.add_space(6.0);
        let mut purpose = import.purpose;
        let options: Vec<(&str, ModelPurpose)> = ModelPurpose::OFFERED.iter().map(|p| (p.name(), *p)).collect();
        ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
            theme::segmented(ui, &mut purpose, &options, |a, b| a == b);
        });
        if purpose != import.purpose {
            import.set_purpose(purpose);
        }
        ui.label(egui::RichText::new(import.purpose.hint()).small().color(theme::INK_DIM));
        ui.add_space(6.0);
        egui::Grid::new("import-model-form")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| form(ui, import));
        ui.add_space(6.0);
        theme::heading(ui, "Animations");
        clips(ui, import, animations);
        ui.add_space(6.0);
        theme::heading(ui, "Textures");
        textures(ui, import);
        ui.add_space(6.0);
        let vpath = import.vpath();
        ui.label(format!("Writes {vpath}"));
        for slot in 0..import.textures.len() {
            if let Some(texture) = import.texture_vpath(slot) {
                ui.label(egui::RichText::new(format!("and {texture}")).small());
            }
        }
        if session.project.has(&vpath) {
            ui.label(
                egui::RichText::new("Replaces the project's model at this path.")
                    .small()
                    .color(theme::WARN),
            );
        } else if super::assets::in_archives(ui.ctx(), game, &vpath) {
            ui.label(
                egui::RichText::new("Replaces the archives' model at this path, wherever it is used.")
                    .small()
                    .color(theme::WARN),
            );
        }
        if import.purpose == ModelPurpose::Creature {
            let skins: Vec<String> = import
                .textures
                .iter()
                .filter(|t| (11..=13).contains(&t.kind))
                .map(|t| t.name.trim().to_string())
                .collect();
            if !skins.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "A CreatureDisplayInfo row draws it with the skins named in its texture \
                         columns, in the model's folder: {}.",
                        skins.join(", ")
                    ))
                    .small()
                    .color(theme::INK_DIM),
                );
            }
        }
        let problems = import.problems();
        for problem in &problems {
            ui.label(egui::RichText::new(problem).small().color(theme::BAD));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(problems.is_empty(), egui::Button::new("Import").fill(theme::ACCENT))
                .clicked()
            {
                write = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if response.should_close() {
        close = true;
    }
    if write {
        assets.ask = Some(Ask::ImportModel);
    } else if close {
        assets.model = None;
    }
}

/// Where it goes, under what name, how large and whether it is solid.
fn form(ui: &mut egui::Ui, import: &mut ModelImport) {
    ui.label(egui::RichText::new("Folder").color(theme::INK_DIM));
    // A creature's and a doodad's root is fixed, so the model is where the
    // display tables and the placement tools look for it.
    let root = match import.purpose {
        ModelPurpose::Creature => Some(CREATURE_ROOT),
        ModelPurpose::Doodad => Some(DOODAD_ROOT),
        ModelPurpose::Other => None,
    };
    match root {
        Some(root) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new(root).color(theme::INK_DIM));
                let mut under = import.folder.get(root.len()..).unwrap_or("").trim_end_matches('\\').to_string();
                if ui.add(egui::TextEdit::singleline(&mut under).desired_width(240.0)).changed() {
                    let under = under.trim().trim_matches(['\\', '/']);
                    import.folder = match under.is_empty() {
                        true => root.to_string(),
                        false => format!("{root}{under}\\"),
                    };
                }
            });
        }
        None => {
            if ui.add(egui::TextEdit::singleline(&mut import.folder).desired_width(280.0)).lost_focus() {
                import.folder = folder_of(&import.folder);
            }
        }
    }
    ui.end_row();
    ui.label(egui::RichText::new("Name").color(theme::INK_DIM));
    if ui.add(egui::TextEdit::singleline(&mut import.name).desired_width(280.0)).changed() {
        import.name_textures();
    }
    ui.end_row();
    ui.label(egui::RichText::new("Scale").color(theme::INK_DIM));
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut import.scale).speed(0.01).range(0.001..=1000.0).max_decimals(3));
        ui.label(
            egui::RichText::new(format!("stands {:.2} yards tall", import.height * import.scale))
                .color(theme::INK_DIM),
        );
        if ui
            .small_button("metres to yards")
            .on_hover_text(format!(
                "glTF is in metres and the game in yards. A model made to size in metres is the \
                 game's size at a scale of {YARDS_PER_METRE:.3}."
            ))
            .clicked()
        {
            import.scale = YARDS_PER_METRE;
        }
    });
    ui.end_row();
    ui.label(egui::RichText::new("Collision").color(theme::INK_DIM));
    ui.horizontal(|ui| {
        ui.radio_value(&mut import.collision, Collision::None, "None")
            .on_hover_text("Walked through, as grass, birds and every creature are.");
        ui.radio_value(&mut import.collision, Collision::Box, "Box")
            .on_hover_text("Solid inside the box round the model as it stands.");
        ui.radio_value(&mut import.collision, Collision::Mesh, "Triangles")
            .on_hover_text(
                "Solid where its triangles are. The server's collision is built from these when \
                 the model is placed on a map.",
            );
    });
    ui.end_row();
}

/// One row per animation in the file: taken, as which animation, looping,
/// and the speed it was made for.
fn clips(ui: &mut egui::Ui, import: &mut ModelImport, animations: Option<&AnimationData>) {
    if import.clips.is_empty() {
        theme::note(ui, "The file has no animations. The model is written standing still.");
        return;
    }
    let ids = animations.map(AnimationData::ids).unwrap_or_default();
    let name_of = |id: u16| animations.and_then(|a| a.name(id)).map_or_else(|| id.to_string(), |n| format!("{id} {n}"));
    let taken_ids: Vec<u16> = import.clips.iter().filter(|c| c.take).map(|c| c.id).collect();
    egui::ScrollArea::vertical()
        .id_salt("import-model-clips")
        .max_height(220.0)
        .show(ui, |ui| {
            egui::Grid::new("import-model-clip-rows")
                .num_columns(4)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    for (index, clip) in import.source.clips.iter().enumerate() {
                        let choice = &mut import.clips[index];
                        ui.checkbox(&mut choice.take, format!("{} ({} ms)", clip.name, clip.duration))
                            .on_hover_text("Whether this animation is written as one of the model's sequences.");
                        ui.add_enabled_ui(choice.take, |ui| {
                            let before = choice.id;
                            match animations {
                                Some(_) => {
                                    egui::ComboBox::from_id_salt(("import-model-clip", index))
                                        .selected_text(name_of(choice.id))
                                        .width(200.0)
                                        .height(320.0)
                                        .show_ui(ui, |ui| {
                                            for &id in &ids {
                                                ui.selectable_value(&mut choice.id, id, name_of(id));
                                            }
                                        });
                                }
                                None => {
                                    ui.add(egui::DragValue::new(&mut choice.id).range(0..=u16::MAX));
                                }
                            }
                            if choice.id != before {
                                let defaults = crate::import::models::defaults(true, choice.id);
                                choice.loops = defaults.loops;
                                choice.move_speed = defaults.move_speed;
                            }
                        })
                        .response
                        .on_hover_text(
                            "The AnimationData.dbc animation the client plays this as. Two \
                             animations written as one are its variations, which the client \
                             chooses between.",
                        );
                        ui.add_enabled_ui(choice.take, |ui| {
                            ui.checkbox(&mut choice.loops, "Loops")
                                .on_hover_text("Off, the client plays it once and holds its last frame.");
                        });
                        ui.add_enabled_ui(choice.take, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut choice.move_speed).speed(0.05).range(0.0..=100.0).max_decimals(2))
                                    .on_hover_text(
                                        "The speed it was made for, in yards a second. The client plays a \
                                         walk or a run faster or slower by the unit's speed against this. \
                                         Zero for an animation that does not travel.",
                                    );
                                if choice.take && taken_ids.iter().filter(|&&id| id == choice.id).count() > 1 {
                                    ui.label(egui::RichText::new("variation").small().color(theme::INK_DIM));
                                }
                            });
                        });
                        ui.end_row();
                    }
                });
        });
}

/// One row per material: its picture, how the model names its texture, and
/// the file or path.
fn textures(ui: &mut egui::Ui, import: &mut ModelImport) {
    let folder = folder_of(&import.folder);
    egui::Grid::new("import-model-textures")
        .num_columns(3)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for (index, slot) in import.source.slots.iter().enumerate() {
                let texture = &mut import.textures[index];
                ui.label(&slot.name).on_hover_text(match (&texture.picture, &texture.unread) {
                    (Some((w, h, _)), _) => format!("A {w}x{h} picture in the file, written as a BLP."),
                    (None, Some(why)) => format!("The file's picture could not be read: {why}"),
                    (None, None) => "The material has no picture in the file.".to_string(),
                });
                egui::ComboBox::from_id_salt(("import-model-texture", index))
                    .selected_text(kind_name(texture.kind))
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for kind in [0, 11, 12, 13] {
                            ui.selectable_value(&mut texture.kind, kind, kind_name(kind));
                        }
                    })
                    .response
                    .on_hover_text(
                        "A file is named in the model. A creature skin is named by a \
                         CreatureDisplayInfo row's texture column, so one model can wear several.",
                    );
                match (&texture.picture, texture.kind) {
                    (Some(_), _) => {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(egui::RichText::new(&folder).small().color(theme::INK_DIM));
                            ui.add(egui::TextEdit::singleline(&mut texture.name).desired_width(160.0));
                            ui.label(egui::RichText::new(".blp").small().color(theme::INK_DIM));
                        });
                    }
                    (None, 0) => {
                        ui.add(
                            egui::TextEdit::singleline(&mut texture.path)
                                .desired_width(280.0)
                                .hint_text("the path of a texture, such as Creature\\Wolf\\WolfSkinGrey.blp"),
                        );
                    }
                    (None, _) => {
                        ui.label(
                            egui::RichText::new("no picture in the file: the display row names one")
                                .small()
                                .color(theme::INK_DIM),
                        );
                    }
                }
                ui.end_row();
            }
        });
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        0 => "File",
        11 => "Creature skin 1",
        12 => "Creature skin 2",
        13 => "Creature skin 3",
        _ => "Other",
    }
}
