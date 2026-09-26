//! **The floating combat text as meshes** — the mesh painter's form of
//! [`crate::ui::worldtext`]'s egui paint, over the same [`Floater`] state and
//! the same rules (`vale_assets::look::worldtext`).
//!
//! Rebuilt every frame while any floater lives, because everything about one
//! moves every frame — it rises, fades and (a critical) grows. That is a
//! handful of glyph quads written in place, not a rebuild of anything else:
//! the group has its own slots and its own z band, **under the interface**
//! (0.5 against the item list's 1.0+), which is the same order the egui
//! painter stated by registering its layer first.
//!
//! One deliberate match with the egui look rather than the interface tint
//! path: the fade ramp's alpha rides the vertex uncompensated, because the
//! egui painter passed it raw and this pass's whole warrant is drawing what
//! that one drew. The glyph atlas's coverage is raw for the same reason —
//! see the note in [`super::fonts`], which learned it from a report.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::collections::HashMap;

use vale_assets::look::worldtext as rules;

use super::fonts::UiFonts;
use super::material::{Blend, InterfaceMaterial};
use crate::lua::widgets::layout::{Viewport, VIRTUAL_HEIGHT};
use crate::ui::worldtext::{self, WorldText};

/// Under the interface's batches (1.0 and up), over the world blit.
const FLOATS_Z: f32 = 0.5;

#[derive(Default)]
pub(super) struct FloatState {
    slots: Vec<super::Slot>,
    materials: HashMap<(AssetId<Image>, Blend), Handle<InterfaceMaterial>>,
}

/// Paint every live floater — the mesh twin of `worldtext::paint`, field for
/// field: the same projection, the same whole-pixel size ramp, the same
/// outline-or-shadow rule off the fill's own alpha.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    mut commands: Commands,
    time: Res<Time>,
    text: Res<WorldText>,
    assets: Res<crate::assets::GameAssets>,
    mut fonts: ResMut<UiFonts>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<InterfaceMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &Transform), With<crate::world::camera::WorldCamera>>,
    mut state: Local<FloatState>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::WorldText);
    let state = &mut *state;
    if text.live.is_empty() && state.slots.is_empty() {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let screen = Vec2::new(window.width(), window.height());
    let Ok((camera, placed)) = camera.single() else {
        return;
    };
    let eye = GlobalTransform::from(*placed);
    // At scale 1, deliberately — the engine draws world text and `uiScale`
    // never touches it. See the egui twin, which states it at length.
    let view = Viewport::of(f64::from(screen.x), f64::from(screen.y), 1.0);
    let scale = view.scale as f32;
    // Device pixels per interface pixel — the glyphs are rastered against it
    // and placed on its grid. See `super::fonts::quad`.
    let dpi = window.scale_factor();
    let Some(face) = fonts.face_index(&assets, Some(rules::FONT)) else {
        return;
    };

    let mut emit = super::build::Emitter::new();
    let now = time.elapsed_secs();
    for floater in &text.live {
        let age_ms = ((now - floater.born) * 1000.0).max(0.0) as u32;
        let at = floater.from + Vec3::Y * rules::rise(floater.kind, age_ms);
        let Ok(pixels) = camera.world_to_viewport(&eye, at) else {
            continue;
        };
        let pixels = pixels / window.scale_factor();
        let alpha = rules::alpha(floater.kind, age_ms);
        if alpha <= 0.0 {
            continue;
        }
        // Whole pixels for the same atlas-economy reason the egui twin
        // rounds: a critical's ease would otherwise raster every glyph at a
        // fresh size every frame.
        let size = (rules::height(floater.kind, age_ms) * VIRTUAL_HEIGHT as f32 * scale)
            .round()
            .max(1.0);
        let (cx, cy) = rules::slot_offset(floater.slot);
        let pos = Vec2::new(
            pixels.x + cx * size * worldtext::CELL_WIDTH,
            pixels.y - cy * size * worldtext::CELL_HEIGHT,
        );
        let behind = [0.0, 0.0, 0.0, alpha.clamp(0.0, 1.0)];
        let shadow = worldtext::SHADOW * VIRTUAL_HEIGHT as f32 * scale;
        if worldtext::translucent(floater.colour) {
            let ring = (worldtext::OUTLINE * size).max(shadow);
            for (dx, dy) in worldtext::AROUND {
                one_line(
                    &mut emit,
                    &mut fonts,
                    &mut images,
                    face,
                    size,
                    dpi,
                    &floater.text,
                    pos + Vec2::new(dx * ring, dy * ring),
                    behind,
                );
            }
        } else {
            one_line(
                &mut emit,
                &mut fonts,
                &mut images,
                face,
                size,
                dpi,
                &floater.text,
                pos + Vec2::splat(shadow),
                behind,
            );
        }
        one_line(
            &mut emit,
            &mut fonts,
            &mut images,
            face,
            size,
            dpi,
            &floater.text,
            pos,
            argb(floater.colour, alpha),
        );
    }
    let mut batches = emit.into_batches();
    for batch in &mut batches {
        batch.z = FLOATS_Z;
    }
    super::apply(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut state.materials,
        &mut state.slots,
        batches,
        screen,
    );
    fonts.flush(&mut images);
}

/// One string, centred on `pos` — the mesh form of egui's
/// `Align2::CENTER_CENTER` text call.
#[allow(clippy::too_many_arguments)]
fn one_line(
    emit: &mut super::build::Emitter,
    fonts: &mut UiFonts,
    images: &mut Assets<Image>,
    face: usize,
    size: f32,
    dpi: f32,
    line: &str,
    pos: Vec2,
    colour: [f32; 4],
) {
    // Measure first, place second — the centring needs the whole width, and
    // one measuring pass keeps the immutable font borrow out of the mutable
    // rasterising one below.
    let (placed, ascent, descent) = {
        let Some(scaled) = fonts.scaled(face, size) else {
            return;
        };
        use ab_glyph::ScaleFont;
        let mut pen = 0.0f32;
        let mut previous = None;
        let mut placed = Vec::with_capacity(line.chars().count());
        for ch in line.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(last) = previous {
                pen += scaled.kern(last, id);
            }
            placed.push((ch, pen));
            pen += scaled.h_advance(id);
            previous = Some(id);
        }
        // `pen` is now the width; fold it into the origins outright.
        for glyph in &mut placed {
            glyph.1 -= pen / 2.0;
        }
        (placed, scaled.ascent(), scaled.descent())
    };
    // CENTER_CENTER: the line box is ascent minus descent tall, its middle on
    // `pos`, so the baseline sits half the box under it plus the ascent.
    let baseline = pos.y - (ascent - descent) / 2.0 + ascent;
    // The measuring above is in interface pixels and the raster below in
    // device ones, exactly as `build::label` splits them.
    let raster = size * if dpi.is_finite() && dpi > 0.0 { dpi } else { 1.0 };
    for (ch, x) in placed {
        let Some(sprite) = fonts.glyph(images, face, raster, ch) else {
            continue;
        };
        let Some(image) = fonts.page_image(sprite.page) else {
            continue;
        };
        let (uv_min, uv_max) = UiFonts::uv(&sprite);
        // On the device pixel grid, for `super::fonts::quad`'s reason: a glyph
        // stretched off it is the haze this painter was reported for.
        let rect = super::fonts::quad(&sprite, [pos.x + x, baseline], dpi);
        emit.quad(
            &image,
            Blend::Alpha,
            rect,
            [uv_min[0], uv_min[1], uv_max[0], uv_max[1]],
            colour,
            None,
        );
    }
}

/// The client's `ARGB` dword, scaled by the ramp, as a vertex colour — linear
/// rgb for the shader's contract, alpha raw for the egui twin's look.
fn argb(colour: u32, alpha: f32) -> [f32; 4] {
    let own = ((colour >> 24) & 0xff) as f32 / 255.0;
    let channel = |shift: u32| {
        bevy::color::Srgba::gamma_function(((colour >> shift) & 0xff) as f32 / 255.0)
    };
    [
        channel(16),
        channel(8),
        channel(0),
        (own * alpha).clamp(0.0, 1.0),
    ]
}
