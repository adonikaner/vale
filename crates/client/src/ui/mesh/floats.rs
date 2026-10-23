//! The floating combat text: every live [`crate::ui::worldtext::Floater`],
//! risen, faded and scaled by the rules in `vale_assets::look::worldtext`.
//!
//! Rebuilt every frame while any floater lives, because everything about one
//! moves every frame — it rises, fades and (a critical) grows. That is a
//! handful of glyph quads written in place, not a rebuild of anything else:
//! the group has its own slots and its own z band, under the interface (0.5
//! against the item list's 1.0+), on `render::labels`' terms.
//!
//! The fade ramp's alpha rides the vertex without the byte-space compensation
//! the interface's tints take; the glyph atlas's coverage is raw for the same
//! reason. See the note in [`super::fonts`], which records the report that
//! settled it.

use bevy::prelude::*;
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

/// Paint every live floater: project it, size it on the whole-pixel ramp, and
/// draw it with an outline or a shadow by the fill's own alpha.
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
    viewport: Res<crate::ui::scale::InterfaceViewport>,
    // The camera is found by its marker, not as the first active camera:
    // `render::portraits` keeps one per unit frame with an image target, and
    // projecting a number through one of those puts it in a 64-pixel portrait.
    camera: Query<(&Camera, &Transform), With<crate::world::camera::WorldCamera>>,
    mut state: Local<FloatState>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::WorldText);
    let state = &mut *state;
    if text.live.is_empty() && state.slots.is_empty() {
        return;
    }
    let Some(screen) = viewport.window else { return };
    let Ok((camera, placed)) = camera.single() else {
        return;
    };
    let eye = GlobalTransform::from(*placed);
    // At scale 1. Floating combat text is one of the two parts of 1.12's
    // interface that ship no XML; the client draws it itself. It is therefore
    // not scaled by `uiScale` the way a widget is, and every size below is a
    // fraction of the window. See [`crate::ui::scale`]. `view` is read only for
    // [`Viewport::scale`], which at 1.0 is the window's height over 768.
    let view = Viewport::of(f64::from(screen.x), f64::from(screen.y), 1.0);
    let scale = view.scale as f32;
    // Device pixels per interface pixel — the glyphs are rastered against it
    // and placed on its grid. See `super::fonts::quad`.
    let dpi = viewport.dpi;
    let Some(face) = fonts.face_index(&assets, Some(rules::FONT)) else {
        return;
    };

    let mut emit = super::build::Emitter::new();
    let now = time.elapsed_secs();
    for floater in &text.live {
        let age_ms = ((now - floater.born) * 1000.0).max(0.0) as u32;
        let at = floater.from + Vec3::Y * rules::rise(floater.kind, age_ms);
        // `world_to_viewport` returns physical pixels and the batches are in
        // logical ones, so the result is divided by the window's scale factor;
        // without the division every number is drawn scaled away from the
        // window's top-left corner. `render::labels` describes the same
        // conversion.
        let Ok(pixels) = camera.world_to_viewport(&eye, at) else {
            continue;
        };
        let pixels = pixels / dpi;
        let alpha = rules::alpha(floater.kind, age_ms);
        if alpha <= 0.0 {
            continue;
        }
        // Rounded to a whole pixel. A critical's height changes on every frame
        // of its growth ([`rules::punch`]), so an unrounded size rasters every
        // glyph at a fresh size every frame and fills the atlas. Whole pixels
        // put the whole curve in about twenty sizes. `max(1)` because the
        // curve starts near zero.
        let size = (rules::height(floater.kind, age_ms) * VIRTUAL_HEIGHT as f32 * scale)
            .round()
            .max(1.0);
        // The spread, in multiples of the text's own size; see
        // [`rules::slot_offset`] and [`worldtext::CELL_WIDTH`]. Screen y is
        // down and the rows step up.
        let (cx, cy) = rules::slot_offset(floater.slot);
        let pos = Vec2::new(
            pixels.x + cx * size * worldtext::CELL_WIDTH,
            pixels.y - cy * size * worldtext::CELL_HEIGHT,
        );
        // A translucent fill gets an outline ([`worldtext::OUTLINE`]); an
        // opaque one gets the sub-pixel drop shadow ([`worldtext::SHADOW`]).
        // The dark colour follows the fade so it does not outlive the fill,
        // but not the fill's own alpha: an outline at a half-transparent
        // fill's alpha would be half as visible, which is the case the outline
        // exists to fix.
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

/// One string, centred on `pos` horizontally and by its line box vertically.
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

/// The client's `ARGB` dword, scaled by the ramp, as a vertex colour: linear
/// rgb for the shader's contract, and the alpha uncompensated (see the module
/// comment).
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

#[cfg(test)]
mod tests {
    use vale_assets::look::worldtext as rules;

    /// The fill carries the style table's own alpha, and the ring behind it
    /// does not.
    ///
    /// The experience row's fill is half-transparent, the style table's
    /// `0x8094008b`, so a ring that inherited the fill's alpha would be half as
    /// visible. The ring takes the fade curve instead (`paint`'s `behind`),
    /// which is full at the entry's peak.
    #[test]
    fn the_outline_does_not_inherit_a_translucent_fill() {
        let experience = rules::style(rules::Kind::Experience).colour;
        let fill = super::argb(experience, 1.0);
        assert_eq!((fill[3] * 255.0).round(), 128.0, "the table's own half alpha reaches the fill");

        // A damage number's fill is opaque, so the fill and the ring agree.
        let damage = rules::style(rules::Kind::Damage).colour;
        assert_eq!(super::argb(damage, 1.0)[3], 1.0);
    }
}
