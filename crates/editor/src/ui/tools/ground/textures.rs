//! The texture brush's panel: the tileset picker, the chosen texture, and
//! the layers of the chunk under the pointer. The chunks and road panels use
//! its picker and swatches. The state is [`crate::tools::ground::textures`].

use bevy_egui::egui;

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::ground::textures::Textures;
use crate::tools::ground::textures;
use crate::tools::placements::place;
use crate::ui::widgets::folders::{breadcrumb, folder_rows, split, split_folder};
use crate::ui::widgets::theme;
use crate::ui::widgets::thumbnails::{SIDE, Thumbnails};

/// What `MCLY`'s animation bits come to, in words: which way the texture
/// crawls and how long it takes to cross itself.
///
/// The direction is printed as a compass point rather than the raw 0..7,
/// because the direction is what is being chosen. Step 0 is north (up the
/// texture) and each step is 45° clockwise. The seconds are the period of one
/// full width at `layer_flags::ANIMATION_BASE_RATE`; that rate is the client's,
/// not the file's.
fn crawls((turn, rate, on): (u32, u32, bool)) -> String {
    if !on {
        return "not animated".to_string();
    }
    const COMPASS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let speed =
        vale_assets::world::adt::layer_flags::ANIMATION_BASE_RATE * (1u32 << rate.min(7)) as f32;
    format!(
        "{} every {:.1}s",
        COMPASS[(turn & 7) as usize],
        1.0 / speed.max(1e-6)
    )
}

/// What a `GroundEffectTexture` row plants, in words: the models by their file
/// names and how many a cell gets, or that it plants nothing.
fn grows(assets: &vale_client::assets::GameAssets, effect_id: u32) -> String {
    if effect_id == 0 {
        return "no ground effect".to_string();
    }
    let effects = assets.ground_effects();
    let Some(effect) = effects.effect(effect_id) else {
        return format!("GroundEffectTexture {effect_id} not found");
    };
    let mut names: Vec<String> = effect.models
        [..usize::from(effect.choices).min(effect.models.len())]
        .iter()
        .map(|&model| {
            let path = effects.model(model);
            path.rsplit(['\\', '/'])
                .next()
                .unwrap_or(path)
                .trim_end_matches(".m2")
                .trim_end_matches(".M2")
                .to_string()
        })
        .collect();
    names.dedup();
    format!("{} · {} per cell", names.join(", "), effect.density)
}

/// The texture brush.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui::tools) fn paint(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    cursor: &Cursor,
    assets: &vale_client::assets::GameAssets,
    guides: &mut crate::tools::guides::Guides,
    asks: &mut crate::ui::shell::asks::ShellAsks,
    now: f64,
) {
    tileset_chosen(ui, textures, thumbnails, session, assets);
    ui.add_space(4.0);
    if !tileset_picker(ui, textures, thumbnails, session) {
        return;
    }
    // A picture of the person's own, written as a ground texture and put on
    // the brush. See `crate::import::assets`.
    if ui
        .add_sized(egui::vec2(ui.available_width(), 22.0), egui::Button::new("Import Ground Texture\u{2026}"))
        .on_hover_text(
            "Choose a PNG, TGA or BLP picture and write it under Tileset\\ as a ground texture \
             with its _s file. The brush takes it once it is written.",
        )
        .clicked()
    {
        asks.push(crate::ui::shell::asks::ShellAsk::ImportGround);
    }

    ui.add_space(4.0);
    theme::heading(ui, "Brush");
    // Paint or Erase first, because it decides what every number under it
    // means. Shift held is the other one for the length of a stroke.
    theme::segmented(
        ui,
        &mut textures.brush.erase,
        &[("Paint", false), ("Erase", true)],
        |a, b| a == b,
    );
    ui.add_space(2.0);
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut textures.brush.radius)
                .speed(0.25)
                .range(textures::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::row(ui, "strength", |ui| {
        ui.add(
            egui::DragValue::new(&mut textures.brush.strength)
                .speed(0.05)
                .range(textures::RATE)
                .suffix("/s"),
        );
    });
    // Opacity is where a held stroke stops, which an eraser does not have:
    // it always goes to nothing.
    let erasing = textures.brush.erase;
    theme::row(ui, "opacity", |ui| {
        let mut percent = textures.brush.opacity * 100.0;
        let response = ui
            .add_enabled(
                !erasing,
                egui::DragValue::new(&mut percent)
                    .speed(0.5)
                    .range(1.0..=100.0)
                    .fixed_decimals(0)
                    .suffix("%"),
            )
            .on_hover_text(
                "The texture's opacity at the end of a held stroke. At 100% the \
                 stroke converges on the texture alone; at 60%, on the texture at 60% \
                 over what is beneath it, from either side.",
            )
            .on_disabled_hover_text("Erase always reduces the texture to 0%.");
        if response.changed() {
            textures.brush.opacity = percent / 100.0;
        }
    });

    theme::row(ui, "core", |ui| {
        ui.add(
            egui::DragValue::new(&mut textures.brush.core)
                .speed(0.01)
                .range(crate::tools::CORE)
                .fixed_decimals(2),
        )
        .on_hover_text(
            "The share of the radius painted at full strength, drawn as the inner              ring. Outside it the paint fades along the falloff to nothing at the              outer ring, and a held stroke goes no further than that fade.",
        );
    });

    ui.add_space(4.0);
    theme::heading(ui, "Falloff");
    theme::segmented(
        ui,
        &mut textures.brush.falloff,
        &textures::FALLOFFS,
        |a, b| a == b,
    );

    ui.add_space(4.0);
    theme::heading(ui, "Shape");
    theme::segmented(ui, &mut textures.brush.shape, &textures::SHAPES, |a, b| {
        a == b
    });

    // Spray: a stroke in patches. One number while it is solid, and the
    // patch size only once it is not.
    ui.add_space(4.0);
    theme::heading(ui, "Spray");
    theme::row(ui, "density", |ui| {
        let mut percent = textures.brush.density * 100.0;
        if ui
            .add(
                egui::DragValue::new(&mut percent)
                    .speed(0.5)
                    .range(5.0..=100.0)
                    .fixed_decimals(0)
                    .suffix("%"),
            )
            .on_hover_text(
                "The share of the ground under the brush that is painted. At 100% the \
                 stroke is solid. Below 100% it paints in patches that stay fixed while \
                 the button is held, to break up the edge between two textures.",
            )
            .changed()
        {
            textures.brush.density = percent / 100.0;
        }
        if textures.brush.density >= 1.0 {
            theme::note(ui, "solid");
        }
    });
    if textures.brush.density < 1.0 {
        theme::row(ui, "grain", |ui| {
            ui.add(
                egui::DragValue::new(&mut textures.brush.grain)
                    .speed(0.05)
                    .range(0.5..=20.0)
                    .fixed_decimals(1)
                    .suffix(" yd"),
            )
            .on_hover_text("How many yards a patch is across. A blend map's texel is half a yard.");
        });
    }

    // The three switches about the limit of four textures to a chunk, which
    // is what stops a stroke on about half the shipped ground.
    ui.add_space(4.0);
    theme::heading(ui, "Four-texture limit");
    ui.checkbox(&mut textures.brush.existing_only, "Paint existing layers only")
        .on_hover_text(
            "Skip chunks that do not carry the texture. A stroke then changes only \
             the blend of existing layers and never adds a layer.",
        );
    ui.add_enabled(
        !textures.brush.existing_only && !textures.brush.erase,
        egui::Checkbox::new(&mut textures.brush.reuse_hidden, "Reuse a hidden layer"),
    )
    .on_hover_text(
        "When a chunk already has four textures, replace the least visible layer \
         if it covers under 2% of the chunk. Otherwise the stroke is refused on \
         that chunk, and the Chunk list below shows its four textures.",
    )
    .on_disabled_hover_text("Only a stroke that may add a texture can reuse a layer.");
    ui.checkbox(&mut guides.layers, "Show full chunks").on_hover_text(
        "Tint red the chunks that carry four textures. The same switch as Full chunks \
         in the view bar's Guides menu.",
    );

    ui.add_space(6.0);
    chunk_layers(ui, thumbnails, textures, session, cursor, assets, now);

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button paints · with shift, erases");
    theme::note(ui, "ctrl + wheel resizes · shift + wheel the strength");
    theme::note(ui, "alt + wheel the core");
    theme::note(ui, "space pins the chunk under the pointer");
    theme::note(ui, "1 - 5 pick the falloff · shift + 1 · 2 · 3 the shape");
}

/// The texture brush's chosen texture, and what a new layer of it grows.
fn tileset_chosen(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &EditSession,
    assets: &vale_client::assets::GameAssets,
) {
    theme::heading(ui, "Texture");
    match textures.brush.texture.is_empty() {
        true => theme::note(ui, "no texture chosen; pick one below"),
        false => {
            let chosen = textures.brush.texture.clone();
            ui.horizontal(|ui| {
                swatch(ui, thumbnails, &chosen);
                // Truncated. `leaf` is a folder and a file name, and many of
                // the 621 are wider than the panel. A label in a horizontal
                // layout extends rather than wrapping, so an untruncated name
                // runs off the right-hand edge. The whole path is on the hover.
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(textures::leaf(&chosen)).color(theme::INK),
                    )
                    .truncate(),
                )
                .on_hover_text(chosen.clone());
            });
            // The foliage a new layer of this texture grows: `MCLY`'s
            // `effectId`. It is set, when the texture is chosen, from what the
            // open ground pairs this texture with, and can be edited here. See
            // `PaintBrush::effect_id`.
            theme::row_about(
                ui,
                "foliage",
                "What a new layer of this texture grows: MCLY effectId, a \
                 GroundEffectTexture row. Set, when the texture is chosen, to what \
                 the open tiles grow on it most.",
                |ui| {
                    let picked = ground_effect(
                        ui,
                        "brush",
                        textures.brush.effect_id,
                        true,
                        session,
                        &chosen,
                        assets,
                    );
                    if let Some(id) = picked {
                        textures.brush.effect_id = id;
                    }
                },
            );
        }
    }
}

/// The tileset picker: the search box and the list by folder. A click sets
/// the texture brush's chosen texture.
///
/// Shared by the texture brush's panel and the chunk selection's. Both act
/// with the brush's chosen texture, so there is one picker and one chosen
/// texture. Returns `false` while the tileset list is still being read, and
/// draws nothing below the search box then.
pub(super) fn tileset_picker(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &EditSession,
) -> bool {
    let chosen = textures.brush.texture.clone();
    let Some(picked) = tileset_browser(ui, textures, thumbnails, &chosen) else {
        return false;
    };
    if let Some(path) = picked {
        // Picking a texture also sets its foliage, from the ground already
        // painted with it. A texture on which no open layer grows anything
        // grows nothing, which matches the shipped ground.
        textures.brush.effect_id = textures::usual_effect(session, &path).unwrap_or(0);
        textures.brush.texture = path;
    }
    true
}

/// The search box and the tileset list by folder, with `chosen` marked.
/// Answers the path clicked this frame, or `None` while the list is still
/// being read. The search and the open folder are the texture tool's, so
/// every list opens where the last one was left.
pub(super) fn tileset_browser(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    chosen: &str,
) -> Option<Option<String>> {
    ui.add(
        egui::TextEdit::singleline(&mut textures.search)
            .hint_text("search the tilesets")
            .desired_width(f32::INFINITY),
    );
    if !textures.catalogue.iter().any(|path| !path.is_empty()) {
        theme::note(ui, "reading the tileset list…");
        return None;
    }
    // Folders, as in the model picker. `Tileset\Elwynn\` is the set a zone was
    // painted with, and with six hundred names in one scroller every one has
    // to be found by typing. Search still runs flat across the whole
    // catalogue; see [`crate::ui::tools::placements::place`] for the reason.
    let needle = textures.search.to_ascii_lowercase();
    let row = SIDE as f32 + 6.0;
    let mut picked: Option<String> = None;
    if needle.is_empty() {
        let here = textures.folder.clone();
        let mut go = breadcrumb(ui, &here, "");
        let (folders, files) = split_folder(&textures.catalogue, &here);
        theme::note(
            ui,
            match folders.len() {
                0 => format!("{} tilesets", files.len()),
                n => format!("{n} folders · {} tilesets", files.len()),
            },
        );
        egui::ScrollArea::vertical()
            .id_salt("tilesets")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                go = folder_rows(ui, &here, &folders, row).or(go.take());
                for path in &files {
                    if texture_row(ui, thumbnails, path, chosen) {
                        picked = Some(path.clone());
                    }
                }
            });
        if let Some(folder) = go {
            let mut folder = folder.to_ascii_lowercase();
            if !folder.ends_with(place::SEP) {
                folder.push(place::SEP);
            }
            textures.folder = folder;
        }
    } else {
        let matches: Vec<String> = textures
            .catalogue
            .iter()
            .filter(|path| !path.is_empty() && path.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
        theme::note(
            ui,
            format!("{} of {} match", matches.len(), textures.catalogue.len()),
        );
        // `show_rows` rather than `show`: it builds only the rows on screen,
        // so only those request a thumbnail and the other six hundred are
        // never decoded. See [`crate::ui::widgets::thumbnails`].
        egui::ScrollArea::vertical()
            .id_salt("tileset-search")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show_rows(ui, row, matches.len(), |ui, range| {
                for path in &matches[range] {
                    if texture_row(ui, thumbnails, path, chosen) {
                        picked = Some(path.clone());
                    }
                }
            });
    }
    Some(picked)
}

/// One row of the tileset list: a picture, a name, and whether it was clicked.
///
/// Laid out by hand rather than built from a `Button`, because the row must be
/// the same height whether or not its picture has been decoded. If rows changed
/// size as thumbnails arrived, the list would move under the pointer.
fn texture_row(ui: &mut egui::Ui, thumbnails: &mut Thumbnails, path: &str, chosen: &str) -> bool {
    let height = SIDE as f32 + 4.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    let on = chosen.eq_ignore_ascii_case(path);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if on || response.hovered() {
            painter.rect_filled(
                rect,
                egui::CornerRadius::same(3),
                match on {
                    true => theme::ACCENT_SUNK,
                    false => theme::RAISED,
                },
            );
        }
        if on {
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(3),
                egui::Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let square = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
            egui::Vec2::splat(SIDE as f32),
        );
        swatch_at(ui, thumbnails, path, square);
        // Two lines and a clip, because the names do not fit on one. The panel
        // is 288 points wide and `aeriepeaksgravelneedlesbase.blp` is wider;
        // painted as one line it runs off the edge of the window. The file name
        // identifies the texture and the folder groups it, so those are the two
        // lines. The extension is the same on all 621 and is dropped.
        let (folder, file) = split(path);
        let text = ui.painter().with_clip_rect(egui::Rect::from_min_max(
            egui::pos2(square.right() + 6.0, rect.top()),
            rect.right_bottom(),
        ));
        text.text(
            egui::pos2(square.right() + 6.0, rect.center().y - 7.0),
            egui::Align2::LEFT_CENTER,
            file,
            egui::FontId::proportional(12.0),
            match on {
                true => theme::INK,
                false => theme::INK_DIM,
            },
        );
        text.text(
            egui::pos2(square.right() + 6.0, rect.center().y + 7.0),
            egui::Align2::LEFT_CENTER,
            folder,
            egui::FontId::proportional(theme::SMALL),
            theme::INK_FAINT,
        );
    }
    response.on_hover_text(path.to_string()).clicked()
}

/// A texture's picture, inline, at [`SIDE`] square.
pub(super) fn swatch(ui: &mut egui::Ui, thumbnails: &mut Thumbnails, path: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(SIDE as f32), egui::Sense::hover());
    swatch_at(ui, thumbnails, path, rect);
}

/// [`swatch`] into a rectangle the caller has already allocated.
///
/// Draws a placeholder while the picture is being decoded, so the row keeps
/// its shape; see [`texture_row`], and [`crate::ui::widgets::thumbnails`] for why a picture
/// is not available the first time it is requested.
fn swatch_at(ui: &egui::Ui, thumbnails: &mut Thumbnails, path: &str, rect: egui::Rect) {
    thumbnails.want(path);
    let painter = ui.painter();
    match thumbnails.get(path) {
        Some(texture) => {
            painter.image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(rect, egui::CornerRadius::same(2), theme::SUNK);
        }
    }
}

/// The textures the panel's chunk is painted with, and the controls for
/// freeing a slot when the four-layer limit is reached.
///
/// This section shows why a brush stroke was refused. A chunk carries at most
/// four textures (measured over the shipped tiles, between half and three
/// quarters of chunks already carry four), so a stroke with a fifth is
/// refused. Each of the four is listed with how much of it is actually visible
/// rather than what its own blend map holds, because the decision is which one
/// to remove.
///
/// ## Why the list shows a pinned chunk, not the one under the pointer
///
/// If the list followed the pointer, moving the pointer to the `×` button
/// would take it off the chunk the list was about. Latching when the pointer
/// leaves the viewport does not work either, because the path to the panel
/// crosses other chunks.
///
/// So the chunk is pinned: by `Space`, which needs no pointer movement, and
/// automatically by a refused stroke, which is the case this list is for.
/// Unpinning returns to following the pointer, which is the default for
/// looking around.
#[allow(clippy::too_many_arguments)]
fn chunk_layers(
    ui: &mut egui::Ui,
    thumbnails: &mut Thumbnails,
    textures: &mut Textures,
    session: &mut EditSession,
    cursor: &Cursor,
    assets: &vale_client::assets::GameAssets,
    now: f64,
) {
    theme::heading(ui, "Chunk");
    let pinned = textures.pinned.is_some();
    let Some((coord, chunk)) = textures::shown(textures, cursor) else {
        theme::note(ui, "pointer is not over the ground");
        theme::note(ui, "space pins the chunk under the pointer");
        return;
    };
    let Some(layers) = textures::layers_of(session, coord, chunk) else {
        theme::note(ui, "that tile is not open yet");
        return;
    };

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match pinned {
                true => "pinned",
                false => "following the pointer",
            })
            .size(theme::SMALL)
            .color(match pinned {
                true => theme::WARN,
                false => theme::INK_FAINT,
            }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button(match pinned {
                    true => "Unpin",
                    false => "Pin",
                })
                .on_hover_text(
                    "Space over a chunk. A pinned chunk stays in this panel \
                     while the pointer moves to the buttons. Unpinned, the panel \
                     follows the pointer.",
                )
                .clicked()
            {
                textures.pinned = match pinned {
                    true => None,
                    false => Some((coord, chunk)),
                };
            }
        });
    });
    theme::note(
        ui,
        format!(
            "tile {},{} · chunk {chunk} · {} of {} textures",
            coord.0,
            coord.1,
            layers.len(),
            vale_edit::adt::alpha::MAX_LAYERS
        ),
    );

    let mut drop: Option<usize> = None;
    let mut rebase: Option<String> = None;
    let mut swap: Option<(usize, String)> = None;
    let mut grow: Option<(usize, u32)> = None;
    let mut crawl: Option<(usize, u32, u32, bool)> = None;
    // "The chosen texture" here is the brush's texture. There is no second
    // picker: the texture used for painting is also the one used for
    // re-basing or swapping a layer.
    let chosen = match textures.brush.texture.is_empty() {
        true => None,
        false => Some(textures.brush.texture.clone()),
    };
    let mut using: Option<(String, u32)> = None;
    for (index, layer) in layers.iter().enumerate() {
        if index > 0 {
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            swatch(ui, thumbnails, &layer.texture);
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(textures::leaf(&layer.texture))
                            .size(theme::SMALL)
                            .color(theme::INK),
                    )
                    .truncate(),
                )
                .on_hover_text(layer.texture.clone());
                // The base is the one layer with no blend map and cannot be
                // removed, so it is labelled "base" instead of showing a
                // coverage it cannot change. The row's buttons share the line.
                ui.horizontal(|ui| {
                    theme::note(
                        ui,
                        match index {
                            0 => "base".to_string(),
                            _ => format!("{:.0}% visible", layer.coverage * 100.0),
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        layer_buttons(ui, index, pinned, chosen.is_some(), &mut |ask| match ask {
                            LayerAsk::Drop => drop = Some(index),
                            LayerAsk::Swap => swap = Some((index, chosen.clone().unwrap_or_default())),
                            LayerAsk::Rebase => rebase = chosen.clone(),
                            LayerAsk::Use => using = Some((layer.texture.clone(), layer.effect_id)),
                        });
                    });
                });
            });
        });
        // The layer's foliage and animation, under the name: the two `MCLY`
        // fields no brush writes. Disabled while the chunk follows the
        // pointer, for the reason the buttons are.
        let picked = theme::row_about(
            ui,
            "foliage",
            "MCLY effectId: what this layer grows, a GroundEffectTexture row.",
            |ui| {
                ground_effect(
                    ui,
                    &format!("layer {index}"),
                    layer.effect_id,
                    pinned,
                    session,
                    &layer.texture,
                    assets,
                )
            },
        );
        if let Some(id) = picked {
            grow = Some((index, id));
        }
        // Only 164 layers in Azeroth are animated, all of them lava, so the
        // direction and speed are shown only once the switch is on.
        theme::row_about(
            ui,
            "animation",
            "MCLY texture animation: scrolls this layer's texture coordinates, as on \
             the Burning Steppes lava.",
            |ui| {
                let (mut turn, mut rate, mut on) = layer.animation;
                ui.add_enabled_ui(pinned, |ui| {
                    let mut moved = ui
                        .checkbox(&mut on, "scrolls")
                        .on_disabled_hover_text("Pin the chunk (press Space over it).")
                        .changed();
                    if on {
                        moved |= crawl_direction(ui, index, &mut turn);
                        moved |= ui
                            .add(egui::DragValue::new(&mut rate).speed(0.1).range(0..=7).prefix("speed "))
                            .on_hover_text("0 to 7: each step doubles it.")
                            .changed();
                        theme::note(ui, crawls((turn, rate, on)));
                    }
                    if moved {
                        crawl = Some((index, turn, rate, on));
                    }
                });
            },
        );
    }
    if let Some((path, effect_id)) = using {
        textures.brush.effect_id = effect_id;
        session.status = format!("painting with {}", textures::leaf(&path));
        textures.brush.texture = path;
    }
    if let Some(index) = drop {
        textures::drop_layer(session, coord, chunk, index);
    }
    if let Some(path) = rebase {
        textures::set_base(session, coord, chunk, &path);
    }
    if let Some((index, path)) = swap {
        textures::swap_layer(session, coord, chunk, index, &path);
    }
    if let Some((index, effect_id)) = grow {
        textures::set_layer_effect(session, coord, chunk, index, effect_id, now);
    }
    if let Some((index, turn, rate, on)) = crawl {
        textures::set_layer_animation(session, coord, chunk, index, turn, rate, on, now);
    }

    // The two buttons that act on more than one layer at a time.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                pinned && layers.len() > 1,
                egui::Button::new("Clear to base"),
            )
            .on_hover_text("Removes every layer from this chunk except the base.")
            .clicked()
        {
            textures::clear_paint(session, coord, chunk);
        }
        if ui
            .add_enabled(chosen.is_some(), egui::Button::new("Base whole tile"))
            .on_hover_text(
                "Makes the chosen texture the base of all 256 chunks of this tile, \
                 under whatever each already carries. One undo entry.",
            )
            .on_disabled_hover_text("Choose a texture below first.")
            .clicked()
        {
            if let Some(path) = chosen.clone() {
                let changed = textures::set_tile_base(session, coord, &path);
                session.status = format!("base texture of {changed} chunks set to {}", textures::leaf(&path));
            }
        }
    });
    if layers.len() >= vale_edit::adt::alpha::MAX_LAYERS {
        ui.add_space(2.0);
        theme::note(
            ui,
            "This chunk has four textures, the maximum. Remove or swap a layer \
             to paint a different texture.",
        );
    }
}

/// What a layer row's buttons asked for.
enum LayerAsk {
    /// Paint with the layer's texture and ground effect.
    Use,
    /// Replace the layer's texture with the chosen one.
    Swap,
    /// Make the chosen texture the chunk's base.
    Rebase,
    /// Remove the layer.
    Drop,
}

/// A layer row's buttons, right to left: remove, swap (or set the base, on
/// the base) and use.
///
/// Use is always available, since it changes the brush and not the chunk. The
/// others are disabled rather than hidden while the chunk follows the pointer,
/// because a press would act on whichever chunk the pointer crossed on the
/// way to the button, with no sign that it was the wrong one.
fn layer_buttons(
    ui: &mut egui::Ui,
    index: usize,
    pinned: bool,
    chosen: bool,
    asked: &mut dyn FnMut(LayerAsk),
) {
    let unpinned = "Pin the chunk first (press Space over it). Unpinned, this would act on \
                    the last chunk the pointer crossed on the way to the button.";
    match index {
        // The base is replaced, never removed; see `Paint::set_base`.
        0 => {
            if ui
                .add_enabled(pinned && chosen, egui::Button::new("Set base").small())
                .on_hover_text(
                    "Make the chosen texture this chunk's base, under everything already \
                     painted on it. Every blend map is kept.",
                )
                .on_disabled_hover_text("Pin the chunk (press Space over it) and choose a texture.")
                .clicked()
            {
                asked(LayerAsk::Rebase);
            }
        }
        _ => {
            if ui
                .add_enabled(pinned, egui::Button::new("×").small())
                .on_hover_text(
                    "Remove this layer from the chunk, freeing a slot for another texture. \
                     Removing a layer at 0% changes nothing on screen.",
                )
                .on_disabled_hover_text(unpinned)
                .clicked()
            {
                asked(LayerAsk::Drop);
            }
            if ui
                .add_enabled(pinned && chosen, egui::Button::new("Swap").small())
                .on_hover_text(
                    "Replace this layer's texture with the chosen texture, keeping its blend \
                     map. On a full chunk this changes a texture without removing a layer.",
                )
                .on_disabled_hover_text("Pin the chunk (press Space over it) and choose a texture.")
                .clicked()
            {
                asked(LayerAsk::Swap);
            }
        }
    }
    if ui
        .button(egui::RichText::new("Use").size(theme::SMALL))
        .on_hover_text("Paint with this texture: it becomes the brush's, with this layer's foliage.")
        .clicked()
    {
        asked(LayerAsk::Use);
    }
}

/// A layer's animation direction as a compass point, from `MCLY`'s 0 to 7:
/// step 0 is north (up the texture) and each step 45° clockwise. Returns
/// whether it changed.
fn crawl_direction(ui: &mut egui::Ui, index: usize, turn: &mut u32) -> bool {
    const COMPASS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let was = *turn;
    egui::ComboBox::from_id_salt(("layer-direction", index))
        .selected_text(COMPASS[(*turn & 7) as usize])
        .width(48.0)
        .show_ui(ui, |ui| {
            for (step, name) in COMPASS.iter().enumerate() {
                ui.selectable_value(turn, step as u32, *name);
            }
        })
        .response
        .on_hover_text("Which way the texture crawls.");
    *turn != was
}

/// A ground effect, chosen by what it grows rather than typed as a
/// `GroundEffectTexture` row id. Returns the row picked this frame.
///
/// The list offers nothing, then the effects the open tiles grow on this
/// texture, most used first, then the ones they grow on other textures. Each
/// is named by its models (see [`grows`]). Neither table names a texture, so
/// the open ground is the only record of which effect suits which, and a zone
/// generally reuses a handful. A row id the open tiles do not use can still be
/// typed at the bottom.
///
/// The counts read every open chunk. Worked out on every frame the list was
/// open, they made the editor slow, so they are worked out on the frame the
/// list opens and kept in egui's memory while it stays open.
fn ground_effect(
    ui: &mut egui::Ui,
    salt: &str,
    current: u32,
    enabled: bool,
    session: &EditSession,
    texture: &str,
    assets: &vale_client::assets::GameAssets,
) -> Option<u32> {
    let mut picked = None;
    let width = ui.available_width().max(80.0);
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(("ground-effect", salt))
            .selected_text(egui::RichText::new(grows(assets, current)).size(theme::SMALL))
            .width(width)
            .truncate()
            .show_ui(ui, |ui| {
                ui.set_min_width(width.max(260.0));
                if ui.selectable_label(current == 0, "nothing").clicked() {
                    picked = Some(0);
                }
                let counts = counts_while_open(ui, salt, session, texture);
                let mut section = |ui: &mut egui::Ui, title: &str, list: &[(u32, usize)]| {
                    if list.is_empty() {
                        return;
                    }
                    ui.add_space(4.0);
                    theme::note(ui, title);
                    for &(id, count) in list.iter().take(20) {
                        let label = format!("{}  · {count} layers · row {id}", grows(assets, id));
                        if ui.selectable_label(current == id, label).clicked() {
                            picked = Some(id);
                        }
                    }
                };
                section(ui, "grown on this texture in the open tiles", &counts.with);
                section(ui, "grown on other textures in the open tiles", &counts.elsewhere);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    theme::note(ui, "another row");
                    let mut id = current;
                    if ui
                        .add(egui::DragValue::new(&mut id).speed(1.0).range(0..=u32::MAX))
                        .on_hover_text("A GroundEffectTexture row id, for one the open tiles do not use.")
                        .changed()
                    {
                        picked = Some(id);
                    }
                });
            });
    })
    .response
    .on_disabled_hover_text("Pin the chunk (press Space over it).");
    picked
}

/// [`textures::effect_counts`] for a list that is open: worked out on the
/// first frame and kept while the list is drawn on every frame after it, or
/// again if the texture it was for has changed.
fn counts_while_open(
    ui: &egui::Ui,
    salt: &str,
    session: &EditSession,
    texture: &str,
) -> std::sync::Arc<textures::EffectCounts> {
    type Kept = (u64, String, std::sync::Arc<textures::EffectCounts>);
    let id = egui::Id::new(("ground-effect-counts", salt));
    let pass = ui.ctx().cumulative_pass_nr();
    let kept = ui.data(|data| data.get_temp::<Kept>(id));
    let counts = match kept {
        Some((last, path, counts)) if last + 1 >= pass && path == texture => counts,
        _ => std::sync::Arc::new(textures::effect_counts(session, texture)),
    };
    ui.data_mut(|data| data.insert_temp(id, (pass, texture.to_string(), counts.clone())));
    counts
}

/// The texture brush's inspector panel, with the tablet's settings under it.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, cursor, assets, guides, now, .. } = subject;
    paint(
        ui,
        &mut editing.textures,
        &mut editing.thumbnails,
        session,
        cursor,
        assets,
        guides,
        &mut editing.asks,
        now,
    );
    crate::ui::tools::tablet::tablet(ui, &mut editing.tablet, &editing.pen);
}
