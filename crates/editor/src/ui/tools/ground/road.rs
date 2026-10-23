//! The road tool's panel. The state is [`crate::tools::ground::road`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::ground::terrain::FALLOFFS;
use crate::tools::ground::textures::Textures;
use crate::tools::ground::textures;
use crate::ui::tools::ground::terrain::objects_follow;
use crate::ui::tools::ground::textures::{swatch, tileset_browser};
use crate::ui::widgets::theme;
use crate::ui::widgets::thumbnails::Thumbnails;

/// The road tool: the points, the shape the road gives the ground, the two
/// textures it paints, and the tileset list that chooses them.
pub(in crate::ui::tools) fn road(
    ui: &mut egui::Ui,
    road: &mut crate::tools::ground::road::RoadTool,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
) {
    use crate::tools::ground::road::{self, Slot};

    theme::heading(ui, "Road");
    theme::note(
        ui,
        "Click the ground to add a point at the end of the road. Drag a point to \
         move it; Shift+click a point to remove it. Backspace removes the last \
         point and Enter applies. To change a road, undo it, adjust it and apply \
         again.",
    );

    // The line: its length, its ends and its steepest part.
    ui.add_space(4.0);
    let path = road::path(session, road);
    theme::row(ui, "points", |ui| {
        ui.label(theme::number(road.points.len().to_string()));
    });
    match &path {
        Some(path) => {
            theme::row(ui, "length", |ui| {
                ui.label(theme::number(format!("{:.1} y", path.length())));
            });
            if let Some((first, last)) = path.ends() {
                theme::row(ui, "ends", |ui| {
                    ui.label(theme::number(format!("{first:.1} z to {last:.1} z")));
                });
            }
            theme::row(ui, "steepest", |ui| {
                let degrees = path.steepest(8.0).atan().to_degrees();
                // As on the Grade panel: above about 50 degrees a character
                // slides rather than climbs.
                let colour = match degrees {
                    d if d > 50.0 => theme::BAD,
                    d if d > 35.0 => theme::WARN,
                    _ => theme::INK,
                };
                ui.label(theme::number(format!("{degrees:.0}°")).color(colour))
                    .on_hover_text("The steepest slope along the centre line, measured over runs of 8 yards.");
            });
        }
        None => theme::note(ui, "place at least two points"),
    }

    ui.add_space(6.0);
    theme::heading(ui, "Line");
    ui.checkbox(&mut road.curved, "Curve through the points")
        .on_hover_text("Off: straight segments between the points.");
    theme::segmented(
        ui,
        &mut road.follow_ground,
        &[("Height from ground", true), ("Height from points", false)],
        |a, b| a == b,
    );
    match road.follow_ground {
        true => {
            theme::row(ui, "smoothing", |ui| {
                ui.add(
                    egui::Slider::new(&mut road.smoothing, 0.0..=200.0)
                        .fixed_decimals(0)
                        .suffix(" y"),
                )
                .on_hover_text(
                    "The road follows the ground under its centre line, averaged over \
                     this many yards. Zero follows every bump.",
                );
            });
        }
        false => {
            theme::note(ui, "each point's height, joined along the line");
            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt("road-points")
                .max_height(140.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, point) in road.points.iter_mut().enumerate() {
                        theme::row(ui, &format!("point {}", i + 1), |ui| {
                            ui.add(
                                egui::DragValue::new(&mut point.height)
                                    .speed(0.25)
                                    .suffix(" z")
                                    .fixed_decimals(1),
                            );
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                });
            if let Some(i) = remove {
                road.remove(i);
            }
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Ground");
    ui.checkbox(&mut road.shape_ground, "Shape the ground")
        .on_hover_text("Off: the apply paints the textures and leaves the heights unchanged.");
    if road.shape_ground {
        theme::row(ui, "width", |ui| {
            ui.add(egui::Slider::new(&mut road.width, 0.5..=60.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("From the centre line to the edge of the part moved fully to the road's height.");
        });
        theme::row(ui, "shoulder", |ui| {
            ui.add(egui::Slider::new(&mut road.shoulder, 0.0..=100.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text(
                    "How far past the width the road blends back into the ground. Zero \
                     leaves a vertical face along both sides.",
                );
        });
        theme::row(ui, "strength", |ui| {
            ui.add(egui::Slider::new(&mut road.strength, 0.05..=1.0).fixed_decimals(2))
                .on_hover_text("How far toward the road's height the ground moves.");
        });
        theme::row(ui, "crown", |ui| {
            ui.add(egui::Slider::new(&mut road.crown, -2.0..=2.0).fixed_decimals(2).suffix(" y"))
                .on_hover_text(
                    "How far the centre stands above the edges. Negative makes a dished \
                     road.",
                );
        });
        theme::row(ui, "offset", |ui| {
            ui.add(egui::Slider::new(&mut road.offset, -10.0..=10.0).fixed_decimals(2).suffix(" y"))
                .on_hover_text(
                    "Added to the road's height everywhere. Negative sinks the road into \
                     the ground; positive raises it as a causeway.",
                );
        });
        ui.label(egui::RichText::new("shoulder falloff").size(theme::SMALL).color(theme::INK_DIM));
        theme::segmented(ui, &mut road.falloff, &FALLOFFS, |a, b| a == b);
        objects_follow(ui, &mut road.objects_follow);
    }

    ui.add_space(6.0);
    theme::heading(ui, "Surface texture");
    ui.checkbox(&mut road.paint_surface, "Paint a surface")
        .on_hover_text("Paint a texture down the middle of the road.");
    if road.paint_surface {
        road_texture(ui, thumbnails, &road.surface.texture, &mut road.surface.effect_id);
        theme::row(ui, "width", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.width, 0.5..=60.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("From the centre line to where the texture starts to fade.");
        });
        theme::row(ui, "softness", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.softness, 0.0..=20.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far past its width the texture fades out.");
        });
        theme::row(ui, "opacity", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.opacity, 0.05..=1.0).fixed_decimals(2))
                .on_hover_text("How visible the texture is where it is solid.");
        });
        theme::row(ui, "wear", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.wear, 0.0..=0.9).fixed_decimals(2))
                .on_hover_text(
                    "How much of the surface is left unpainted in patches, so the ground \
                     under it shows through. Zero is solid.",
                );
        });
    }

    ui.add_space(6.0);
    theme::heading(ui, "Verge texture");
    ui.checkbox(&mut road.paint_verge, "Paint a verge")
        .on_hover_text(
            "Paint a second texture in a band along both sides of the surface, starting \
             under the surface's fade.",
        );
    if road.paint_verge {
        road_texture(ui, thumbnails, &road.verge.texture, &mut road.verge.effect_id);
        theme::row(ui, "width", |ui| {
            ui.add(egui::Slider::new(&mut road.verge.width, 0.5..=40.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far the band reaches past the surface's width.");
        });
        theme::row(ui, "softness", |ui| {
            ui.add(egui::Slider::new(&mut road.verge.softness, 0.0..=20.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far past its width the band's outer edge fades out.");
        });
        theme::row(ui, "opacity", |ui| {
            ui.add(egui::Slider::new(&mut road.verge.opacity, 0.05..=1.0).fixed_decimals(2))
                .on_hover_text("How visible the texture is where it is solid.");
        });
    }

    if road.paint_surface || road.paint_verge {
        ui.add_space(6.0);
        theme::heading(ui, "Edges");
        theme::row(ui, "ragged", |ui| {
            ui.add(egui::Slider::new(&mut road.ragged, 0.0..=5.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far the painted edges wander in and out of a smooth line. Zero is smooth.");
        });
        theme::row(ui, "grain", |ui| {
            ui.add(egui::Slider::new(&mut road.grain, 1.0..=40.0).fixed_decimals(0).suffix(" y"))
                .on_hover_text("The size, in yards, of one wander of the edges and of one patch of wear.");
        });
        ui.checkbox(&mut road.reuse_hidden, "Reuse a hidden layer").on_hover_text(
            "When a chunk already has four textures, give the road's texture the layer \
             that shows least, if it shows almost nothing. Off: such a chunk is not painted.",
        );
    }

    ui.add_space(6.0);
    if theme::primary(ui, "Apply road").clicked() {
        road::apply_and_report(session, road);
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!road.points.is_empty(), egui::Button::new("Remove last point"))
            .clicked()
        {
            road.points.pop();
        }
        if ui
            .add_enabled(!road.points.is_empty(), egui::Button::new("Clear points"))
            .clicked()
        {
            road.clear();
        }
    });
    if !road.said.is_empty() {
        ui.add_space(4.0);
        theme::note(ui, road.said.clone());
    }

    // The tileset list, which sets whichever of the two textures is chosen
    // above it.
    if road.paint_surface || road.paint_verge {
        ui.add_space(6.0);
        theme::heading(ui, "Tilesets");
        theme::segmented(
            ui,
            &mut road.slot,
            &[("Set surface", Slot::Surface), ("Set verge", Slot::Verge)],
            |a, b| a == b,
        );
        if ui
            .add_enabled(
                !textures.brush.texture.is_empty(),
                egui::Button::new("Use the Textures tool's texture"),
            )
            .on_hover_text("Set the chosen texture to the one on the Textures tool's brush.")
            .clicked()
        {
            road.choose(textures.brush.texture.clone(), textures.brush.effect_id);
        }
        let chosen = match road.slot {
            Slot::Surface => road.surface.texture.clone(),
            Slot::Verge => road.verge.texture.clone(),
        };
        if let Some(Some(path)) = tileset_browser(ui, textures, thumbnails, &chosen) {
            // A road texture grows nothing unless asked: grass down the
            // middle of a road is rarely what was meant. The ground effect is
            // set beside the texture.
            road.choose(path, 0);
        }
    }
}

/// One of the road's textures: its picture and name, and its ground effect.
fn road_texture(ui: &mut egui::Ui, thumbnails: &mut Thumbnails, path: &str, effect_id: &mut u32) {
    if path.is_empty() {
        theme::note(ui, "no texture chosen; choose one in the tileset list below");
        return;
    }
    ui.horizontal(|ui| {
        swatch(ui, thumbnails, path);
        ui.add(egui::Label::new(egui::RichText::new(textures::leaf(path)).color(theme::INK)).truncate())
            .on_hover_text(path.to_string());
    });
    theme::row(ui, "ground effect", |ui| {
        ui.add(egui::DragValue::new(effect_id).speed(1.0).range(0..=u32::MAX))
            .on_hover_text(
                "MCLY effectId for a layer the road adds: the GroundEffectTexture row \
                 whose doodads grow on it. 0 grows none.",
            );
    });
}

/// The road tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, .. } = subject;
    road(ui, &mut editing.road, &mut editing.textures, &mut editing.thumbnails, session);
}
