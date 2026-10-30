//! The road tool's controls, in two places.
//!
//! ```text
//! viewport toolbar   the line: straight or curved, and where its height
//!                    comes from; the keys
//! inspector          Road (the points, the measurements and Apply Road), Point Heights
//!                    while the points set the height, Ground, Surface, Verge,
//!                    Edges and Tilesets
//! ```
//!
//! The toolbar holds what changes the line drawn over the ground, beside the
//! points it is drawn through. The inspector holds what the apply does to the
//! ground under that line. Ground, Surface and Verge each have their switch in
//! the section heading, and a section that is off shows nothing below it.
//!
//! The state is [`crate::tools::ground::road`].

use bevy_egui::egui;

use crate::preview::portraits::Portraits;
use crate::session::EditSession;
use crate::tools::ground::foliage::{Foliage, Target};
use crate::tools::ground::road::{self, RoadTool, Slot};
use crate::tools::ground::terrain::FALLOFFS;
use crate::tools::ground::textures::Textures;
use crate::ui::tools::ground::terrain::objects_follow;
use crate::ui::tools::ground::textures::{swatch, tileset_browser};
use crate::ui::widgets::folders::split;
use crate::ui::widgets::theme::{self, bar_choice, bar_gap, bar_setting, bar_toggle, property};
use crate::ui::widgets::thumbnails::{SIDE, Thumbnails};

/// The road tool's toolbar over the viewport:
///
/// ```text
/// Line  Straight  Curved | Height  Ground  Points  Smoothing
/// the keys
/// ```
///
/// Smoothing is shown only while the height comes from the ground. See
/// [`theme::toolbar`].
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    road: &mut RoadTool,
) -> Option<egui::Rect> {
    theme::toolbar(ctx, "road-toolbar", viewport, corner, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            bar_setting(ui, "Line", |ui| {
                if bar_toggle(ui, !road.curved, "Straight")
                    .on_hover_text("Straight segments between the points.")
                    .clicked()
                {
                    road.curved = false;
                }
                if bar_toggle(ui, road.curved, "Curved")
                    .on_hover_text("A smooth curve through the points.")
                    .clicked()
                {
                    road.curved = true;
                }
            });
            bar_gap(ui);
            bar_setting(ui, "Height", |ui| {
                if bar_toggle(ui, road.follow_ground, "Ground")
                    .on_hover_text(
                        "The road's height follows the ground under its centre line, averaged \
                         over the smoothing distance.",
                    )
                    .clicked()
                {
                    road.follow_ground = true;
                }
                if bar_toggle(ui, !road.follow_ground, "Points")
                    .on_hover_text(
                        "The road's height runs from each point's height to the next. The heights \
                         are listed in the inspector.",
                    )
                    .clicked()
                {
                    road.follow_ground = false;
                }
            });
            if road.follow_ground {
                bar_setting(ui, "Smoothing", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut road.smoothing)
                            .speed(0.5)
                            .range(0.0..=200.0)
                            .fixed_decimals(0)
                            .suffix(" yd"),
                    )
                    .on_hover_text(
                        "How many yards along the centre line the ground's height is averaged \
                         over. Zero follows every bump.",
                    );
                });
            }
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Left button: add a point \u{b7} Drag a point: move it \u{b7} Shift + click a \
                     point: remove it \u{b7} Backspace: remove the last point \u{b7} Enter: apply \
                     \u{b7} Right button: insert or remove a point",
                )
                .size(theme::SMALL)
                .color(theme::INK_FAINT),
            )
            .wrap(),
        );
    })
}

/// The road tool's inspector: Road, Point Heights while the points set the
/// height, Ground, Surface and Verge, then Edges and Tilesets while either
/// texture is painted.
#[allow(clippy::too_many_arguments)]
fn panel(
    ui: &mut egui::Ui,
    road: &mut RoadTool,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
    foliage: &mut Foliage,
    portraits: &mut Portraits,
) {
    // With one texture painted, the tileset list sets that one.
    let only = match (road.paint_surface, road.paint_verge) {
        (true, false) => Some(Slot::Surface),
        (false, true) => Some(Slot::Verge),
        _ => None,
    };
    if let Some(slot) = only {
        if road.slot != slot {
            road.slot = slot;
        }
    }

    summary(ui, road, session);
    if !road.follow_ground {
        point_heights(ui, road);
    }
    ground(ui, road);
    surface(ui, road, textures, thumbnails, session, assets, foliage, portraits);
    verge(ui, road, textures, thumbnails, session, assets, foliage, portraits);
    if road.paint_surface || road.paint_verge {
        edges(ui, road);
        tilesets(ui, road, textures, thumbnails);
    }
}

/// The Road section: the points with Remove Last and Clear, the line's
/// length, its height at the two ends and its steepest slope, then Apply Road
/// and what the last apply did.
fn summary(ui: &mut egui::Ui, road: &mut RoadTool, session: &mut EditSession) {
    theme::heading(ui, "Road");
    property(ui, "Points", "", |ui| {
        ui.label(theme::number(road.points.len().to_string()));
        ui.add_space(6.0);
        let any = !road.points.is_empty();
        if ui.add_enabled(any, egui::Button::new("Remove Last")).on_hover_text("Backspace.").clicked() {
            road.points.pop();
        }
        if ui.add_enabled(any, egui::Button::new("Clear")).on_hover_text("Remove every point.").clicked() {
            road.clear();
        }
    });
    if let Some(path) = road::path(session, road) {
        property(ui, "Length", "Along the centre line.", |ui| {
            ui.label(theme::number(format!("{:.1} yd", path.length())));
        });
        if let Some((first, last)) = path.ends() {
            property(ui, "Height", "The road's height at its first and last point.", |ui| {
                ui.label(theme::number(format!("{first:.1} to {last:.1} yd")));
            });
        }
        property(
            ui,
            "Steepest Slope",
            "The steepest slope along the centre line, measured over runs of 8 yards. Above \
             about 50° a character slides rather than climbs.",
            |ui| {
                let degrees = path.steepest(8.0).atan().to_degrees();
                // As on the Grade panel: amber near the limit, red over it.
                let colour = match degrees {
                    d if d > 50.0 => theme::BAD,
                    d if d > 35.0 => theme::WARN,
                    _ => theme::INK,
                };
                ui.label(theme::number(format!("{degrees:.0}°")).color(colour));
            },
        );
    }

    ui.add_space(6.0);
    let missing = road.missing();
    let apply = ui
        .add_enabled_ui(missing.is_none(), |ui| theme::primary(ui, "Apply Road"))
        .inner
        .on_hover_text(
            "Shape the ground along the road and paint its textures, as one undo entry. Enter \
             does the same.",
        );
    if apply.clicked() {
        road::apply_and_report(session, road);
    }
    match missing {
        Some(why) => theme::note(ui, why),
        None if !road.said.is_empty() => theme::note(ui, road.said.clone()),
        None => {}
    }
    theme::note(ui, "To change an applied road, undo it, adjust it and apply again.");
}

/// The Point Heights section, shown while the road's height comes from its
/// points: each point's height, and Remove.
fn point_heights(ui: &mut egui::Ui, road: &mut RoadTool) {
    theme::heading(ui, "Point Heights");
    if road.points.is_empty() {
        theme::note(ui, "A point takes the ground's height where it is placed.");
        return;
    }
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("road-points")
        .max_height(160.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, point) in road.points.iter_mut().enumerate() {
                property(ui, &format!("Point {}", i + 1), "", |ui| {
                    ui.add(egui::DragValue::new(&mut point.height).speed(0.25).fixed_decimals(1).suffix(" yd"))
                        .on_hover_text(
                            "The road's height at this point. It is set to the ground's height \
                             where the point is placed or dropped.",
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

/// The Ground section: how the apply moves the ground to the road's height.
fn ground(ui: &mut egui::Ui, road: &mut RoadTool) {
    switch_heading(
        ui,
        "Ground",
        &mut road.shape_ground,
        "Move the ground to the road's height. Off: the apply paints the textures and leaves \
         the heights unchanged.",
    );
    if !road.shape_ground {
        return;
    }
    property(
        ui,
        "Width",
        "From the centre line to the edge of the part moved fully to the road's height.",
        |ui| {
            ui.add(egui::DragValue::new(&mut road.width).speed(0.1).range(0.5..=60.0).fixed_decimals(1).suffix(" yd"));
        },
    );
    property(
        ui,
        "Shoulder",
        "How far past the width the road blends back into the ground. Zero leaves a vertical \
         face along both sides.",
        |ui| {
            ui.add(egui::DragValue::new(&mut road.shoulder).speed(0.1).range(0.0..=100.0).fixed_decimals(1).suffix(" yd"));
        },
    );
    property(ui, "Falloff", "How the shoulder blends from the road's height to the ground.", |ui| {
        bar_choice(ui, "road-falloff", &mut road.falloff, &FALLOFFS, |a, b| a == b);
    });
    property(ui, "Strength", "How far toward the road's height the ground moves.", |ui| {
        percent(ui, &mut road.strength, 5.0..=100.0);
    });
    property(
        ui,
        "Crown",
        "How far the centre stands above the edges. Negative makes a dished road.",
        |ui| {
            ui.add(egui::DragValue::new(&mut road.crown).speed(0.01).range(-2.0..=2.0).fixed_decimals(2).suffix(" yd"));
        },
    );
    property(
        ui,
        "Offset",
        "Added to the road's height everywhere. Negative sinks the road into the ground; \
         positive raises it as a causeway.",
        |ui| {
            ui.add(egui::DragValue::new(&mut road.offset).speed(0.05).range(-10.0..=10.0).fixed_decimals(2).suffix(" yd"));
        },
    );
    objects_follow(ui, &mut road.objects_follow);
}

/// The Surface section: the texture painted down the middle of the road.
#[allow(clippy::too_many_arguments)]
fn surface(
    ui: &mut egui::Ui,
    road: &mut RoadTool,
    textures: &Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
    foliage: &mut Foliage,
    portraits: &mut Portraits,
) {
    if switch_heading(ui, "Surface", &mut road.paint_surface, "Paint a texture down the middle of the road.") {
        road.slot = Slot::Surface;
    }
    if !road.paint_surface {
        return;
    }
    let target = road.paint_verge && road.slot == Slot::Surface;
    if texture_card(ui, "surface", thumbnails, textures, &road.surface.texture, target) {
        road.slot = Slot::Surface;
    }
    if !road.surface.texture.is_empty() {
        let catalogue = foliage.catalogue(session, assets);
        let effect = road.surface.effect_id;
        if property(ui, "Foliage", FOLIAGE, |ui| {
            super::foliage::field(ui, catalogue.as_deref(), portraits, &[effect], true, "")
        }) {
            foliage.open(Target::RoadSurface, &road.surface.texture, effect, session);
        }
    }
    let surface = &mut road.surface;
    property(ui, "Width", "From the centre line to where the texture starts to fade.", |ui| {
        ui.add(egui::DragValue::new(&mut surface.width).speed(0.1).range(0.5..=60.0).fixed_decimals(1).suffix(" yd"));
    });
    property(ui, "Softness", "How far past its width the texture fades out.", |ui| {
        ui.add(egui::DragValue::new(&mut surface.softness).speed(0.05).range(0.0..=20.0).fixed_decimals(1).suffix(" yd"));
    });
    property(ui, "Opacity", "How visible the texture is where it is solid.", |ui| {
        percent(ui, &mut surface.opacity, 5.0..=100.0);
    });
    property(
        ui,
        "Wear",
        "How much of the surface is left unpainted in patches, so the ground under it shows \
         through. Zero is solid.",
        |ui| {
            percent(ui, &mut surface.wear, 0.0..=90.0);
        },
    );
}

/// The Verge section: a second texture in a band along both sides of the
/// surface.
#[allow(clippy::too_many_arguments)]
fn verge(
    ui: &mut egui::Ui,
    road: &mut RoadTool,
    textures: &Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
    foliage: &mut Foliage,
    portraits: &mut Portraits,
) {
    if switch_heading(
        ui,
        "Verge",
        &mut road.paint_verge,
        "Paint a second texture in a band along both sides of the surface, starting under the \
         surface's fade.",
    ) {
        road.slot = Slot::Verge;
    }
    if !road.paint_verge {
        return;
    }
    let target = road.paint_surface && road.slot == Slot::Verge;
    if texture_card(ui, "verge", thumbnails, textures, &road.verge.texture, target) {
        road.slot = Slot::Verge;
    }
    if !road.verge.texture.is_empty() {
        let catalogue = foliage.catalogue(session, assets);
        let effect = road.verge.effect_id;
        if property(ui, "Foliage", FOLIAGE, |ui| {
            super::foliage::field(ui, catalogue.as_deref(), portraits, &[effect], true, "")
        }) {
            foliage.open(Target::RoadVerge, &road.verge.texture, effect, session);
        }
    }
    let verge = &mut road.verge;
    property(ui, "Width", "How far the band reaches past the surface's width.", |ui| {
        ui.add(egui::DragValue::new(&mut verge.width).speed(0.1).range(0.5..=40.0).fixed_decimals(1).suffix(" yd"));
    });
    property(ui, "Softness", "How far past its width the band's outer edge fades out.", |ui| {
        ui.add(egui::DragValue::new(&mut verge.softness).speed(0.05).range(0.0..=20.0).fixed_decimals(1).suffix(" yd"));
    });
    property(ui, "Opacity", "How visible the texture is where it is solid.", |ui| {
        percent(ui, &mut verge.opacity, 5.0..=100.0);
    });
}

/// The Edges section: how the painted edges wander, and what happens on a
/// chunk that already carries four textures.
fn edges(ui: &mut egui::Ui, road: &mut RoadTool) {
    theme::heading(ui, "Edges");
    property(
        ui,
        "Ragged",
        "How far the painted edges wander in and out of a smooth line. Zero is smooth.",
        |ui| {
            ui.add(egui::DragValue::new(&mut road.ragged).speed(0.05).range(0.0..=5.0).fixed_decimals(1).suffix(" yd"));
        },
    );
    property(ui, "Grain", "The size of one wander of the edges and of one patch of wear.", |ui| {
        ui.add(egui::DragValue::new(&mut road.grain).speed(0.25).range(1.0..=40.0).fixed_decimals(0).suffix(" yd"));
    });
    ui.checkbox(&mut road.reuse_hidden, "Reuse a hidden layer").on_hover_text(
        "When a chunk already has four textures, give the road's texture the layer that shows \
         least, if it shows almost nothing. Off: such a chunk is not painted.",
    );
}

/// The Tilesets section: which of the two textures a click sets, the Textures
/// tool's brush texture as a shortcut, and the list.
fn tilesets(ui: &mut egui::Ui, road: &mut RoadTool, textures: &mut Textures, thumbnails: &mut Thumbnails) {
    theme::heading(ui, "Tilesets");
    if road.paint_surface && road.paint_verge {
        property(
            ui,
            "Sets",
            "Which of the road's textures a click in the list sets. A click on a texture above \
             chooses it too.",
            |ui| {
                theme::segmented(ui, &mut road.slot, &[("Surface", Slot::Surface), ("Verge", Slot::Verge)], |a, b| a == b);
            },
        );
    }
    let (name, chosen) = match road.slot {
        Slot::Surface => ("surface", road.surface.texture.clone()),
        Slot::Verge => ("verge", road.verge.texture.clone()),
    };
    if ui
        .add_enabled(!textures.brush.texture.is_empty(), egui::Button::new("Use Brush Texture"))
        .on_hover_text(format!("Set the {name} texture to the Textures tool's brush texture and its foliage."))
        .on_disabled_hover_text("The Textures tool has no texture chosen.")
        .clicked()
    {
        road.choose(textures.brush.texture.clone(), textures.brush.effect_id);
    }
    theme::note(ui, format!("Click a tileset to set the {name} texture."));
    if let Some(Some(path)) = tileset_browser(ui, textures, thumbnails, &chosen) {
        // A texture chosen from the list grows nothing until Foliage is set:
        // grass down the middle of a road is rarely what was meant.
        road.choose(path, 0);
    }
}

/// The hover text of the Surface and Verge sections' Foliage rows.
const FOLIAGE: &str = "What a layer the road adds grows: MCLY effectId, a GroundEffectTexture row. \
                       A texture chosen from the list starts at None. Click to choose a set.";

/// A section heading with the section's switch in it: a checkbox, the name in
/// [`theme::heading`]'s style, and its rule. Answers whether the switch was
/// turned on this frame.
fn switch_heading(ui: &mut egui::Ui, text: &str, on: &mut bool, about: &str) -> bool {
    ui.add_space(4.0);
    let turned_on = ui
        .horizontal(|ui| {
            let response = ui
                .checkbox(on, egui::RichText::new(text.to_uppercase()).size(theme::SMALL).color(theme::INK_FAINT))
                .on_hover_text(about);
            let y = response.rect.center().y;
            let from = response.rect.right() + 6.0;
            let to = ui.max_rect().right();
            if to > from {
                ui.painter().hline(from..=to, y, egui::Stroke::new(1.0, theme::LINE));
            }
            response.changed() && *on
        })
        .inner;
    ui.add_space(2.0);
    turned_on
}

/// One of the road's two textures as a card: its picture, its name and its
/// folder. While both textures are painted, the card the tileset list sets
/// has the accent outline, and a click on the other card moves the list to
/// it. Answers whether the card was clicked.
fn texture_card(
    ui: &mut egui::Ui,
    salt: &str,
    thumbnails: &mut Thumbnails,
    textures: &Textures,
    path: &str,
    target: bool,
) -> bool {
    let frame = egui::Frame::new()
        .fill(theme::SUNK)
        .stroke(egui::Stroke::new(1.0, if target { theme::ACCENT } else { theme::LINE }))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if path.is_empty() {
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(SIDE as f32), egui::Sense::hover());
                    ui.painter().rect_stroke(
                        rect,
                        egui::CornerRadius::same(2),
                        egui::Stroke::new(1.0, theme::LINE),
                        egui::StrokeKind::Inside,
                    );
                    theme::note(ui, "No texture chosen. Click one in the tileset list below.");
                    return;
                }
                swatch(ui, thumbnails, path);
                let (folder, file) = split(textures.spellings.of(path));
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new(file).color(theme::INK)).truncate().selectable(false));
                    ui.add(
                        egui::Label::new(egui::RichText::new(folder).size(theme::SMALL).color(theme::INK_FAINT))
                            .truncate()
                            .selectable(false),
                    );
                });
            });
        })
        .response;
    let response = ui.interact(frame.rect, ui.id().with(("road-texture", salt)), egui::Sense::click());
    let response = match (path.is_empty(), target) {
        (true, _) => response,
        (false, true) => response.on_hover_text(format!("{path}\nThe tileset list sets this texture.")),
        (false, false) => response.on_hover_text(path.to_string()),
    };
    response.clicked()
}

/// A fraction from 0 to 1 edited as a percentage in `range`.
fn percent(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    let mut percent = *value * 100.0;
    if ui
        .add(egui::DragValue::new(&mut percent).speed(0.5).range(range).fixed_decimals(0).suffix("%"))
        .changed()
    {
        *value = percent / 100.0;
    }
}

/// The road tool's inspector panel. The line's settings are on the toolbar
/// over the viewport, [`toolbar`].
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, assets, .. } = subject;
    panel(
        ui,
        &mut editing.road,
        &mut editing.textures,
        &mut editing.thumbnails,
        session,
        assets,
        &mut editing.foliage,
        &mut editing.portraits,
    );
}
