//! The texture brush's controls, in two places.
//!
//! ```text
//! viewport toolbar   what the next stroke does: Paint or Erase, radius,
//!                    strength, opacity and core; falloff, shape, spray, the
//!                    four-texture-limit switches and the drawing tablet; the
//!                    keys
//! inspector          the brush's texture and its foliage, the tileset list,
//!                    and the layers of the chunk under the pointer or pinned
//! ```
//!
//! The chunks and road panels use the tileset list and the swatches. The
//! state is [`crate::tools::ground::textures`].

use bevy_egui::egui;

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::ground::textures::Textures;
use crate::tools::ground::textures;
use crate::tools::placements::place;
use crate::ui::widgets::folders::{breadcrumb, folder_rows, split, split_folder};
use crate::ui::widgets::theme::{self, bar_choice, bar_gap, bar_setting, bar_toggle, property};
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
        return "Not animated".to_string();
    }
    const COMPASS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let speed =
        vale_assets::world::adt::layer_flags::ANIMATION_BASE_RATE * (1u32 << rate.min(7)) as f32;
    format!(
        "{}, {:.1} s per cycle",
        COMPASS[(turn & 7) as usize],
        1.0 / speed.max(1e-6)
    )
}

/// What a `GroundEffectTexture` row plants, in words: the models by their file
/// names and how many a cell gets, or that it plants nothing.
fn grows(assets: &vale_client::assets::GameAssets, effect_id: u32) -> String {
    if effect_id == 0 {
        return "None".to_string();
    }
    let effects = assets.ground_effects();
    let Some(effect) = effects.effect(effect_id) else {
        return format!("Missing GroundEffectTexture row {effect_id}");
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

/// The texture brush's inspector: the brush's texture, the tileset list, and
/// the chunk's layers. The brush's settings are on [`toolbar`].
pub(in crate::ui::tools) fn paint(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    cursor: &Cursor,
    assets: &vale_client::assets::GameAssets,
    now: f64,
) {
    tileset_chosen(ui, textures, thumbnails, session, assets);

    theme::heading(ui, "Tilesets");
    if !tileset_picker(ui, textures, thumbnails, session) {
        return;
    }
    theme::note(ui, "Click a tileset to paint with it.");

    ui.add_space(6.0);
    chunk_layers(ui, thumbnails, textures, session, cursor, assets, now);
}

/// The texture brush's toolbar over the viewport, three lines:
///
/// ```text
/// Paint  Erase | Radius  Strength  Opacity  Core
/// Falloff  Shape | Spray  Grain | Existing Layers Only  Reuse Hidden Layer | Show Full Chunks | Tablet
/// the keys
/// ```
///
/// Paint or Erase comes first because it decides what the numbers after it
/// mean; Shift held is the other one for the length of a stroke. Opacity and
/// Reuse Hidden Layer are disabled while erasing, which always goes to
/// nothing and never adds a layer. `full_chunks` is the view bar's Full chunks
/// guide. See [`theme::toolbar`].
pub(crate) fn toolbar(
    ctx: &egui::Context,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    textures: &mut Textures,
    full_chunks: &mut bool,
    tablet: &mut crate::tablet::Tablet,
    pen: &crate::tablet::Pen,
) -> Option<egui::Rect> {
    theme::toolbar(ctx, "textures-toolbar", viewport, corner, |ui| {
        let brush = &mut textures.brush;
        let erasing = brush.erase;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if bar_toggle(ui, !brush.erase, "Paint")
                .on_hover_text("Paint the brush's texture onto the ground.")
                .clicked()
            {
                brush.erase = false;
            }
            if bar_toggle(ui, brush.erase, "Erase")
                .on_hover_text("Remove the brush's texture from the ground. Shift held erases for one stroke.")
                .clicked()
            {
                brush.erase = true;
            }
            bar_gap(ui);
            bar_setting(ui, "Radius", |ui| {
                ui.add(egui::DragValue::new(&mut brush.radius).speed(0.25).range(textures::RADIUS).suffix(" yd"))
                    .on_hover_text("The outer ring. Ctrl + wheel.");
            });
            bar_setting(ui, "Strength", |ui| {
                ui.add(egui::DragValue::new(&mut brush.strength).speed(0.05).range(textures::RATE).suffix("/s"))
                    .on_hover_text("How fast a held stroke reaches its opacity. Shift + wheel.");
            });
            bar_setting(ui, "Opacity", |ui| {
                let mut percent = brush.opacity * 100.0;
                let response = ui
                    .add_enabled(
                        !erasing,
                        egui::DragValue::new(&mut percent).speed(0.5).range(1.0..=100.0).fixed_decimals(0).suffix("%"),
                    )
                    .on_hover_text(
                        "The texture's opacity at the end of a held stroke. At 100% the \
                         stroke converges on the texture alone; at 60%, on the texture at 60% \
                         over what is beneath it, from either side.",
                    )
                    .on_disabled_hover_text("Erase always reduces the texture to 0%.");
                if response.changed() {
                    brush.opacity = percent / 100.0;
                }
            });
            bar_setting(ui, "Core", |ui| {
                ui.add(egui::DragValue::new(&mut brush.core).speed(0.01).range(crate::tools::CORE).fixed_decimals(2))
                    .on_hover_text(
                        "The share of the radius painted at full strength, drawn as the inner \
                         ring. Outside it the paint fades along the falloff to nothing at the \
                         outer ring, and a held stroke goes no further than that fade. Alt + wheel.",
                    );
            });
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            bar_setting(ui, "Falloff", |ui| {
                bar_choice(ui, "textures-falloff", &mut brush.falloff, &textures::FALLOFFS, |a, b| a == b)
                    .on_hover_text("How the strength fades from the inner ring to the outer ring. Keys 1 to 5.");
            });
            bar_setting(ui, "Shape", |ui| {
                bar_choice(ui, "textures-shape", &mut brush.shape, &textures::SHAPES, |a, b| a == b)
                    .on_hover_text("The brush's outline. Shift + 1, 2 or 3.");
            });
            bar_gap(ui);
            // Spray: a stroke in patches. One number while it is solid, and
            // the patch size only once it is not.
            bar_setting(ui, "Spray", |ui| {
                let mut percent = brush.density * 100.0;
                if ui
                    .add(egui::DragValue::new(&mut percent).speed(0.5).range(5.0..=100.0).fixed_decimals(0).suffix("%"))
                    .on_hover_text(
                        "The share of the ground under the brush that is painted. At 100% the \
                         stroke is solid. Below 100% it paints in patches that stay fixed while \
                         the button is held, to break up the edge between two textures.",
                    )
                    .changed()
                {
                    brush.density = percent / 100.0;
                }
            });
            if brush.density < 1.0 {
                bar_setting(ui, "Grain", |ui| {
                    ui.add(egui::DragValue::new(&mut brush.grain).speed(0.05).range(0.5..=20.0).fixed_decimals(1).suffix(" yd"))
                        .on_hover_text("How many yards a patch is across. A blend map's texel is half a yard.");
                });
            }
            bar_gap(ui);
            // The two switches about the limit of four textures to a chunk,
            // which is what stops a stroke on about half the shipped ground.
            if bar_toggle(ui, brush.existing_only, "Existing Layers Only")
                .on_hover_text(
                    "Skip chunks that do not carry the texture. A stroke then changes only \
                     the blend of existing layers and never adds a layer.",
                )
                .clicked()
            {
                brush.existing_only = !brush.existing_only;
            }
            let reusable = !brush.existing_only && !erasing;
            if ui
                .add_enabled(reusable, egui::Button::selectable(brush.reuse_hidden, "Reuse Hidden Layer").min_size(egui::vec2(0.0, 24.0)))
                .on_hover_text(
                    "When a chunk already has four textures, replace the least visible layer \
                     if it covers under 2% of the chunk. Otherwise the stroke is refused on \
                     that chunk, and the inspector's Chunk section shows its four textures.",
                )
                .on_disabled_hover_text("Only a stroke that may add a texture can reuse a layer.")
                .clicked()
            {
                brush.reuse_hidden = !brush.reuse_hidden;
            }
            bar_gap(ui);
            if bar_toggle(ui, *full_chunks, "Show Full Chunks")
                .on_hover_text(
                    "Tint red the chunks that carry four textures. The same switch as Full chunks \
                     in the view bar's Guides menu.",
                )
                .clicked()
            {
                *full_chunks = !*full_chunks;
            }
            ui.menu_button("Tablet", |ui| crate::ui::tools::tablet::tablet(ui, tablet, pen))
                .response
                .on_hover_text("Drawing tablet settings: pen pressure and cursor motion.");
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Left button: paint \u{b7} Shift: erase \u{b7} Ctrl + wheel: radius \u{b7} \
                     Shift + wheel: strength \u{b7} Alt + wheel: core \u{b7} 1-5: falloff \u{b7} \
                     Shift + 1-3: shape \u{b7} Space: pin the chunk",
                )
                .size(theme::SMALL)
                .color(theme::INK_FAINT),
            )
            .wrap(),
        );
    })
}

/// The texture brush's texture, and what a new layer of it grows.
fn tileset_chosen(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &EditSession,
    assets: &vale_client::assets::GameAssets,
) {
    theme::heading(ui, "Brush Texture");
    if textures.brush.texture.is_empty() {
        theme::note(ui, "No texture selected. Choose one from the tileset list below.");
        return;
    }
    let chosen = textures.brush.texture.clone();
    let spell = std::sync::Arc::clone(&textures.spellings);
    let (folder, file) = split(spell.of(&chosen));
    ui.horizontal(|ui| {
        swatch(ui, thumbnails, &chosen);
        ui.vertical(|ui| {
            // Truncated: many of the 621 names are wider than the panel, and a
            // label in a horizontal layout extends rather than wrapping. The
            // whole path is on the hover.
            ui.add(egui::Label::new(egui::RichText::new(file).color(theme::INK)).truncate())
                .on_hover_text(chosen.clone());
            ui.add(egui::Label::new(egui::RichText::new(folder).size(theme::SMALL).color(theme::INK_FAINT)).truncate());
        });
    });
    // The foliage a new layer of this texture grows: `MCLY`'s `effectId`. It
    // is set, when the texture is chosen, from what the open ground pairs this
    // texture with, and can be edited here. See `PaintBrush::effect_id`.
    property(
        ui,
        "Foliage",
        "What a new layer of this texture grows: MCLY effectId, a GroundEffectTexture row. \
         Set, when the texture is chosen, to what the open tiles grow on it most.",
        |ui| {
            let picked = ground_effect(ui, "brush", textures.brush.effect_id, true, session, &chosen, assets);
            if let Some(id) = picked {
                textures.brush.effect_id = id;
            }
        },
    );
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
            .hint_text("Search tilesets")
            .desired_width(f32::INFINITY),
    );
    if !textures.catalogue.iter().any(|path| !path.is_empty()) {
        theme::note(ui, "Loading tilesets…");
        return None;
    }
    // Folders, as in the model picker. `Tileset\Elwynn\` is the set a zone was
    // painted with, and with six hundred names in one scroller every one has
    // to be found by typing. Search still runs flat across the whole
    // catalogue; see [`crate::ui::tools::placements::place`] for the reason.
    let needle = textures.search.to_ascii_lowercase();
    let row = SIDE as f32 + 6.0;
    let spell = std::sync::Arc::clone(&textures.spellings);
    let mut picked: Option<String> = None;
    if needle.is_empty() {
        let here = textures.folder.clone();
        let mut go = breadcrumb(ui, &here, "", &spell);
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
                go = folder_rows(ui, &here, &folders, row, &spell).or(go.take());
                for path in &files {
                    if texture_row(ui, thumbnails, path, spell.of(path), chosen) {
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
            format!("{} of {} tilesets match", matches.len(), textures.catalogue.len()),
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
                    if texture_row(ui, thumbnails, path, spell.of(path), chosen) {
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
fn texture_row(ui: &mut egui::Ui, thumbnails: &mut Thumbnails, path: &str, shown: &str, chosen: &str) -> bool {
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
        let (folder, file) = split(shown);
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
/// If the list followed the pointer, moving the pointer to a layer's Remove
/// button would take it off the chunk the list was about. Latching when the
/// pointer leaves the viewport does not work either, because the path to the
/// panel crosses other chunks.
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
        theme::note(ui, "Move the pointer over the ground to show the chunk under it. Press Space to pin it.");
        return;
    };
    let Some(layers) = textures::layers_of(session, coord, chunk) else {
        theme::note(ui, "The tile is not loaded.");
        return;
    };

    property(ui, "Location", "", |ui| {
        ui.label(egui::RichText::new(format!("Tile {}, {} \u{b7} Chunk {chunk}", coord.0, coord.1)).color(theme::INK));
    });
    let full = layers.len() >= vale_edit::adt::alpha::MAX_LAYERS;
    property(ui, "Layers", "A chunk carries at most four textures.", |ui| {
        let colour = match full {
            true => theme::WARN,
            false => theme::INK,
        };
        ui.label(egui::RichText::new(format!("{} of {}", layers.len(), vale_edit::adt::alpha::MAX_LAYERS)).color(colour));
    });
    property(
        ui,
        "Status",
        "Space over a chunk pins it. A pinned chunk stays in this panel while the pointer \
         moves to the buttons. Unpinned, the panel follows the pointer.",
        |ui| {
            let (words, colour) = match pinned {
                true => ("Pinned", theme::WARN),
                false => ("Following pointer", theme::INK),
            };
            ui.label(egui::RichText::new(words).color(colour));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(match pinned {
                        true => "Unpin",
                        false => "Pin",
                    })
                    .clicked()
                {
                    textures.pinned = match pinned {
                        true => None,
                        false => Some((coord, chunk)),
                    };
                }
            });
        },
    );
    if !pinned {
        theme::note(ui, "Layer changes apply only to a pinned chunk. Press Space over a chunk to pin it.");
    }

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
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(theme::SUNK)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(6, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    swatch(ui, thumbnails, &layer.texture);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(textures::leaf(&layer.texture)).size(theme::SMALL).color(theme::INK),
                            )
                            .truncate(),
                        )
                        .on_hover_text(layer.texture.clone());
                        // The base is the one layer with no blend map and
                        // cannot be removed, so it is labelled as the base
                        // instead of showing a coverage it cannot change. The
                        // row's buttons share the line.
                        ui.horizontal(|ui| {
                            theme::note(
                                ui,
                                match index {
                                    0 => "Base".to_string(),
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
                // The layer's foliage and animation: the two `MCLY` fields no
                // brush writes. Disabled while the chunk follows the pointer,
                // for the reason the buttons are.
                let picked = property(
                    ui,
                    "Foliage",
                    "MCLY effectId: what this layer grows, a GroundEffectTexture row.",
                    |ui| ground_effect(ui, &format!("layer {index}"), layer.effect_id, pinned, session, &layer.texture, assets),
                );
                if let Some(id) = picked {
                    grow = Some((index, id));
                }
                // Only 164 layers in Azeroth are animated, all of them lava,
                // so the direction and speed are shown only once the switch is
                // on.
                property(
                    ui,
                    "Animation",
                    "MCLY texture animation: scrolls this layer's texture coordinates, as on \
                     the Burning Steppes lava.",
                    |ui| {
                        let (mut turn, mut rate, mut on) = layer.animation;
                        ui.add_enabled_ui(pinned, |ui| {
                            let mut moved = ui
                                .checkbox(&mut on, "Scrolls")
                                .on_disabled_hover_text("Pin the chunk (press Space over it).")
                                .changed();
                            if on {
                                moved |= crawl_direction(ui, index, &mut turn);
                                moved |= ui
                                    .add(egui::DragValue::new(&mut rate).speed(0.1).range(0..=7).prefix("Speed "))
                                    .on_hover_text("0 to 7: each step doubles it.")
                                    .changed();
                            }
                            if moved {
                                crawl = Some((index, turn, rate, on));
                            }
                        });
                    },
                );
                if layer.animation.2 {
                    theme::note(ui, crawls(layer.animation));
                }
            });
    }
    if let Some((path, effect_id)) = using {
        textures.brush.effect_id = effect_id;
        session.status = format!("Painting with {}", textures::leaf(&path));
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

    // The two commands that act on more than one layer at a time.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(pinned && layers.len() > 1, egui::Button::new("Clear to Base"))
            .on_hover_text("Remove every layer from this chunk except the base.")
            .on_disabled_hover_text("Pin a chunk with more than one layer (press Space over it).")
            .clicked()
        {
            textures::clear_paint(session, coord, chunk);
        }
        if ui
            .add_enabled(chosen.is_some(), egui::Button::new("Set Tile Base"))
            .on_hover_text(
                "Make the brush's texture the base of all 256 chunks of this tile, under \
                 whatever each already carries. One undo entry.",
            )
            .on_disabled_hover_text("Choose a texture from the tileset list first.")
            .clicked()
        {
            if let Some(path) = chosen.clone() {
                let changed = textures::set_tile_base(session, coord, &path);
                session.status = format!("Base texture of {changed} chunks set to {}", textures::leaf(&path));
            }
        }
    });
    if full {
        ui.add_space(2.0);
        theme::note(
            ui,
            "This chunk has four textures, the maximum. Remove or swap a layer to paint a \
             different texture, or turn on Reuse Hidden Layer on the toolbar.",
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

/// A layer row's buttons, right to left: Remove, Swap (or Set Base, on the
/// base) and Use.
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
                .add_enabled(pinned && chosen, egui::Button::new("Set Base").small())
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
                .add_enabled(pinned, egui::Button::new("Remove").small())
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
        .add(egui::Button::new("Use").small())
        .on_hover_text("Paint with this texture: it becomes the brush's texture, with this layer's foliage.")
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
                if ui.selectable_label(current == 0, "None").clicked() {
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
                        let label = format!("{} · {count} layers · row {id}", grows(assets, id));
                        if ui.selectable_label(current == id, label).clicked() {
                            picked = Some(id);
                        }
                    }
                };
                section(ui, "Used with this texture in the loaded tiles", &counts.with);
                section(ui, "Used with other textures in the loaded tiles", &counts.elsewhere);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    theme::note(ui, "Row ID");
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

/// The texture brush's inspector panel. The brush's settings and the drawing
/// tablet's are on the toolbar over the viewport, [`toolbar`].
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, cursor, assets, now, .. } = subject;
    paint(ui, &mut editing.textures, &mut editing.thumbnails, session, cursor, assets, now);
}
