//! The draw list as vertices: every [`Item`] kind turned into batched,
//! z-ordered geometry.
//!
//! The list comes from [`crate::lua::widgets::draw`], which decides what is
//! visible, where and in what order without a window; the assertions about
//! what the interface draws are unit tests there. This file only turns the
//! list into triangles.
//!
//! ## Coordinates
//!
//! The game's space is y-up from the bottom left, in the interface's units;
//! the window's is y-down from the top left, in pixels. [`px`] is the one
//! place an item's rectangle is converted, through the
//! [`crate::ui::scale::InterfaceViewport`] the pointer inverts. Positions
//! come from the viewport and sizes (a border edge, a font height, a
//! backdrop's insets and tile period) from its scale alone.
//!
//! ## Batching and paint order
//!
//! The list arrives sorted back to front, so batches must be drawn in list
//! order. A batch therefore merges only adjacent emissions that share a
//! texture and a blend; a panel's art on one sheet is one batch. Each batch's z
//! is the list index of its first item, so bevy's sorted transparent phase
//! reproduces the list order exactly. [`Emitter::barrier`] closes the open
//! batch where another pass draws an item (the minimap, whose batches rebuild
//! on their own cadence), so that no batch merges across that item and sorts
//! over it.
//!
//! ## Clipping on the CPU
//!
//! A 2d mesh has no scissor, so [`Item::clip`] is applied to the geometry: a
//! quad is clamped (with its uv), and arbitrary triangles — a disc, a turned
//! quad, a `<Model>`'s batches — go through a Sutherland–Hodgman clip that
//! interpolates uv and colour, which is exact because both are affine over the
//! triangle.

use bevy::prelude::*;

use crate::assets::GameAssets;
use crate::lua::widgets::draw::{Content, Item};
use crate::lua::widgets::layout::Viewport;
use crate::lua::widgets::regions::Paint;

use super::fonts::{self, UiFonts};
use super::material::{byte_space_alpha, coloured_alpha, Blend};
use super::text;
use super::textures::UiTextures;

/// Where the interface's batches sit among the 2d camera's meshes: above the
/// present quad (z 0), below nothing — egui draws in its own later pass.
const Z_BASE: f32 = 1.0;
/// One item of separation; 3,000 items reach z 4, far inside the camera.
const Z_STEP: f32 = 0.001;
/// One [`Emitter::layer`] of separation inside a single item.
///
/// A sixteenth of an item's step, so fifteen layers fit under the next item and
/// the list order across items is untouched. See [`Emitter::layer`] for why an
/// item needs internal order at all.
const SUB_STEP: f32 = Z_STEP / 16.0;
/// The most [`Emitter::layer`] calls one item may make. The sixteenth would
/// land on the next item's z and paint over it.
const SUB_MAX: usize = 15;

/// One draw: a texture, a blend, and the triangles that share them.
pub(crate) struct Batch {
    pub image: Handle<Image>,
    pub blend: Blend,
    pub z: f32,
    /// Window pixels, y-down — [`super::to_screen_mesh`] turns them into the
    /// 2d camera's units.
    pub positions: Vec<[f32; 2]>,
    pub(crate) uvs: Vec<[f32; 2]>,
    /// Linear rgb, byte-space-compensated straight alpha.
    pub colours: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// One vertex on its way into a batch.
#[derive(Clone, Copy, Debug)]
struct V {
    pos: Vec2,
    uv: Vec2,
    colour: [f32; 4],
}

/// The builder's shared state: the open batch and the z bookkeeping.
pub struct Emitter {
    batches: Vec<Batch>,
    index: usize,
    sub: usize,
    open: bool,
}

impl Emitter {
    pub fn new() -> Emitter {
        Emitter { batches: Vec::new(), index: 0, sub: 0, open: false }
    }

    /// The list index of the item about to be emitted — the z key.
    pub fn item(&mut self, index: usize) {
        self.index = index;
        self.sub = 0;
    }

    /// Start a new layer inside the current item, above everything already
    /// emitted for it.
    ///
    /// The z is per item, so an item that emits several batches gives them
    /// all the same z and leaves their relative order to the phase's iteration
    /// order, which this painter does not control. The minimap is the one item
    /// that does emit several: one batch per visible terrain tile and then the
    /// player's arrow. Sharing a z there hid the arrow whenever the tie broke
    /// the wrong way, and the tie moved as the character walked, because the
    /// arrow's batch index is the number of tiles in view and that is 1 to 4.
    ///
    /// Also closes the open batch: two layers must not merge even when they
    /// share a texture and a blend, since merging is what would put them back
    /// at one z.
    pub fn layer(&mut self) {
        self.sub = (self.sub + 1).min(SUB_MAX);
        self.open = false;
    }

    /// Close the open batch: the next emission starts a new one whatever its
    /// texture. Called where an item's pixels are drawn by another pass.
    pub(crate) fn barrier(&mut self) {
        self.open = false;
    }

    pub(crate) fn into_batches(self) -> Vec<Batch> {
        self.batches
    }

    fn batch(&mut self, image: &Handle<Image>, blend: Blend) -> &mut Batch {
        let mergeable = self.open
            && self
                .batches
                .last()
                .is_some_and(|b| b.image == *image && b.blend == blend);
        if !mergeable {
            self.batches.push(Batch {
                image: image.clone(),
                blend,
                z: Z_BASE + self.index as f32 * Z_STEP + self.sub as f32 * SUB_STEP,
                positions: Vec::new(),
                uvs: Vec::new(),
                colours: Vec::new(),
                indices: Vec::new(),
            });
            self.open = true;
        }
        self.batches.last_mut().expect("just ensured")
    }

    /// An axis-aligned textured rectangle, clamped to the clip window.
    ///
    /// `rect` and `uv` are `[left, top, right, bottom]`, pixels and 0..1.
    /// `pub(super)` for the two sibling painters ([`super::floats`],
    /// [`super::loading`]), which emit through the same batcher.
    pub(super) fn quad(
        &mut self,
        image: &Handle<Image>,
        blend: Blend,
        rect: [f32; 4],
        uv: [f32; 4],
        colour: [f32; 4],
        clip: Option<[f32; 4]>,
    ) {
        let Some((rect, uv)) = clamp(rect, uv, clip) else {
            return;
        };
        let batch = self.batch(image, blend);
        let base = batch.positions.len() as u32;
        let [l, t, r, b] = rect;
        let [ul, ut, ur, ub] = uv;
        batch.positions.extend([[l, t], [r, t], [r, b], [l, b]]);
        batch.uvs.extend([[ul, ut], [ur, ut], [ur, ub], [ul, ub]]);
        batch.colours.extend([colour; 4]);
        batch
            .indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Arbitrary triangles, clipped when the item is.
    fn raw(
        &mut self,
        image: &Handle<Image>,
        blend: Blend,
        vertices: &[V],
        indices: &[u32],
        clip: Option<[f32; 4]>,
    ) {
        match clip {
            None => {
                let batch = self.batch(image, blend);
                let base = batch.positions.len() as u32;
                for v in vertices {
                    batch.positions.push([v.pos.x, v.pos.y]);
                    batch.uvs.push([v.uv.x, v.uv.y]);
                    batch.colours.push(v.colour);
                }
                batch.indices.extend(indices.iter().map(|i| base + i));
            }
            Some(window) => {
                for tri in indices.chunks_exact(3) {
                    let poly = clip_triangle(
                        [
                            vertices[tri[0] as usize],
                            vertices[tri[1] as usize],
                            vertices[tri[2] as usize],
                        ],
                        window,
                    );
                    if poly.len() < 3 {
                        continue;
                    }
                    let batch = self.batch(image, blend);
                    let base = batch.positions.len() as u32;
                    for v in &poly {
                        batch.positions.push([v.pos.x, v.pos.y]);
                        batch.uvs.push([v.uv.x, v.uv.y]);
                        batch.colours.push(v.colour);
                    }
                    for corner in 1..poly.len() as u32 - 1 {
                        batch.indices.extend([base, base + corner, base + corner + 1]);
                    }
                }
            }
        }
    }
}

/// A quad and its uv cut down to the window. `None` when nothing is left.
fn clamp(rect: [f32; 4], uv: [f32; 4], clip: Option<[f32; 4]>) -> Option<([f32; 4], [f32; 4])> {
    let Some([cl, ct, cr, cb]) = clip else {
        return Some((rect, uv));
    };
    let [l, t, r, b] = rect;
    let (nl, nt, nr, nb) = (l.max(cl), t.max(ct), r.min(cr), b.min(cb));
    if nl >= nr || nt >= nb {
        return None;
    }
    let width = (r - l).max(f32::EPSILON);
    let height = (b - t).max(f32::EPSILON);
    let at = |v: f32, lo: f32, span: f32, ulo: f32, uhi: f32| ulo + (v - lo) / span * (uhi - ulo);
    let [ul, ut, ur, ub] = uv;
    Some((
        [nl, nt, nr, nb],
        [
            at(nl, l, width, ul, ur),
            at(nt, t, height, ut, ub),
            at(nr, l, width, ul, ur),
            at(nb, t, height, ut, ub),
        ],
    ))
}

/// Sutherland–Hodgman against one rectangle, interpolating uv and colour.
fn clip_triangle(tri: [V; 3], window: [f32; 4]) -> Vec<V> {
    let [wl, wt, wr, wb] = window;
    // Signed distance inside each of the four edges: `(x-axis?, flipped?,
    // bound)` spelled as data so the loop allocates nothing per triangle.
    let edges = [(true, false, wl), (true, true, wr), (false, false, wt), (false, true, wb)];
    let mut poly: Vec<V> = tri.to_vec();
    for (x_axis, flipped, bound) in edges {
        if poly.is_empty() {
            break;
        }
        let inside = |p: Vec2| {
            let v = if x_axis { p.x } else { p.y };
            if flipped {
                bound - v
            } else {
                v - bound
            }
        };
        let mut next = Vec::with_capacity(poly.len() + 1);
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            let da = inside(a.pos);
            let db = inside(b.pos);
            if da >= 0.0 {
                next.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                next.push(lerp(a, b, t));
            }
        }
        poly = next;
    }
    poly
}

fn lerp(a: V, b: V, t: f32) -> V {
    let mix = |x: f32, y: f32| x + (y - x) * t;
    V {
        pos: a.pos.lerp(b.pos, t),
        uv: a.uv.lerp(b.uv, t),
        colour: [
            mix(a.colour[0], b.colour[0]),
            mix(a.colour[1], b.colour[1]),
            mix(a.colour[2], b.colour[2]),
            mix(a.colour[3], b.colour[3]),
        ],
    }
}

/// A region's colour times the alpha it inherited: linear rgb and
/// byte-space-compensated straight alpha.
///
/// The file's numbers are straight alpha. The alpha goes through
/// [`byte_space_alpha`] and the three colour channels do not: the correction
/// adjusts how much is mixed, not the colours being mixed. A `<Color a="0.5">`
/// scrim and a `SetAlpha(0.5)` fade both go through this function.
fn tint(rgba: [f32; 4], alpha: f32) -> [f32; 4] {
    [
        linear(rgba[0]),
        linear(rgba[1]),
        linear(rgba[2]),
        byte_space_alpha((rgba[3] * alpha).clamp(0.0, 1.0)),
    ]
}

/// [`tint`] with [`coloured_alpha`] in place of [`byte_space_alpha`]: the tint
/// for a solid fill and a status bar, the two places where a saturated colour
/// carries its own translucency.
fn solid_tint(rgba: [f32; 4], alpha: f32) -> [f32; 4] {
    [
        linear(rgba[0]),
        linear(rgba[1]),
        linear(rgba[2]),
        coloured_alpha(rgba, alpha),
    ]
}

/// The IEC sRGB curve, through bevy's own conversion so that this file cannot
/// disagree with bevy about it.
fn linear(c: f32) -> f32 {
    bevy::color::Srgba::gamma_function(c.clamp(0.0, 1.0))
}

/// A `layout::Rect` (game units, y-up) as `[left, top, right, bottom]` window
/// pixels (y-down).
///
/// The only place in this file that turns game units into absolute pixels, an
/// item's rectangle and its [`Item::clip`] alike. Every other rectangle here
/// is derived from one of these, so a change to where the space sits in the
/// window (such as a pillarbox) is a change to [`Viewport::to_pixels`] alone.
fn px(r: crate::lua::widgets::layout::Rect, view: Viewport) -> [f32; 4] {
    let (left, top) = view.to_pixels(r.left, r.top());
    let (right, bottom) = view.to_pixels(r.right(), r.bottom);
    [left as f32, top as f32, right as f32, bottom as f32]
}

/// Everything the kind handlers read and write. One struct so the handlers'
/// signatures stay one line each.
pub struct Painter<'w> {
    pub assets: &'w GameAssets,
    pub images: &'w mut Assets<Image>,
    pub textures: &'w mut UiTextures,
    pub fonts: &'w mut UiFonts,
    pub models: &'w crate::lua::widgets::model::UiModels,
    pub portraits: &'w crate::render::portraits::Portraits,
    /// The paper-doll pictures; see [`crate::render::paperdoll`].
    pub(crate) dolls: &'w crate::render::paperdoll::Dolls,
    pub place: &'w crate::interface::minimap::MinimapView,
    pub view: Viewport,
    /// Device pixels per interface pixel: the window's scale factor. This is
    /// the only field in this painter that distinguishes the two.
    ///
    /// Art is stretched by this factor, as the 1.12.1 client stretches its art
    /// at any resolution. Text is not stretched: a glyph is rastered at
    /// `size * dpi` and its quad is placed by [`fonts::quad`], so one atlas
    /// texel lands on one screen pixel. See that function for the effect of
    /// rounding in interface pixels instead.
    pub(crate) dpi: f32,
    /// The wall clock in milliseconds, for a `<Model>`'s global sequences.
    pub(crate) now_ms: u32,
    /// `<Model>` files named on screen and not loaded, reported once each.
    ///
    /// Per file rather than one flag, because a session has models that draw
    /// and models that cannot: the glue screens' backdrops are 3D scenes that
    /// `render::glue` draws and this pass does not, so a single flag would be
    /// used up on `UI_Orc.mdx` before the first cooldown of the session.
    pub(crate) reported_models: &'w mut std::collections::HashSet<String>,
}

/// The static pass: every item except the minimap, which [`minimap_batches`]
/// rebuilds on its own cadence, and the `<Model>` frames, which
/// [`model_batches`] rebuilds.
pub(crate) fn item_batches(items: &[Item], painter: &mut Painter) -> Vec<Batch> {
    let mut emit = Emitter::new();
    for (index, item) in items.iter().enumerate() {
        emit.item(index);
        // The minimap and `<Model>` are drawn by passes of their own, because
        // their pictures change without any item changing: the minimap follows
        // the player, and a `<Model>` animates with the clock. Both still take
        // their index here, so they draw at the z of their place in the list.
        if matches!(item.content, Content::Minimap(_) | Content::Model(_)) {
            emit.barrier();
            continue;
        }
        one(&mut emit, painter, item);
    }
    emit.into_batches()
}

/// The model pass: every `<Model>` frame, rebuilt at frame rate.
///
/// `<Model>` frames have their own pass for the same reason the minimap does.
/// A flat model's triangles depend on the wall clock: `uimodel::sprites`
/// places every live sprite from `Painter::now_ms`. The clock is not in any
/// `Item`, so when models were in the static pass the deep compare in
/// [`super::rebuild`] found no change and the mesh was not rebuilt. The pet
/// bar's autocast border then froze while the interface was idle and jumped
/// forward when anything else on the screen changed; the report described it
/// as "inconsistent, freezes, behaves as if it has extremely low FPS".
///
/// The cooldown swirl had the opposite problem. `Scene::elapsed` is in the
/// item, so a running cooldown rebuilt the whole static list thirty times a
/// second, which is the cost this module exists to avoid. In this pass it
/// rebuilds only its own batches.
pub(crate) fn model_batches(items: &[Item], painter: &mut Painter) -> Vec<Batch> {
    let mut emit = Emitter::new();
    for (index, item) in items.iter().enumerate() {
        let Content::Model(scene) = &item.content else {
            continue;
        };
        emit.item(index);
        emit.barrier();
        let view = painter.view;
        let rect = px(item.rect, view);
        let clip = item.clip.map(|c| px(c, view));
        model(&mut emit, painter, scene, rect, item.alpha, clip);
    }
    emit.into_batches()
}

/// The minimap pass: the one widget that shows the world, so its batches
/// change on every frame the player moves, while the other items do not.
pub(crate) fn minimap_batches(items: &[Item], painter: &mut Painter) -> Vec<Batch> {
    let mut emit = Emitter::new();
    for (index, item) in items.iter().enumerate() {
        let Content::Minimap(widget) = &item.content else {
            continue;
        };
        emit.item(index);
        emit.barrier();
        minimap(&mut emit, painter, item, *widget);
    }
    emit.into_batches()
}

/// The z of the held item: above every strata. A held item drawn under a panel
/// would look dropped. The value is far above the ~4 the deepest item list
/// reaches and far below the camera.
const CARRIED_Z: f32 = 900.0;

/// The size a held item is drawn at, in the interface's units: the size of a
/// bag square's icon.
const CARRIED_ICON: f32 = 32.0;

/// The carried pass: the icon on the pointer.
///
/// The one thing this painter draws that is not a widget: 1.12 draws a held
/// item as the cursor rather than as a frame, so there is nothing in the tree
/// to walk and nothing in `lua::draw` to sort. It has its own pass because it
/// follows the mouse at frame rate while the other items do not change.
///
/// A [`CARRIED_ICON`] square centred on the pointer, in the interface's space,
/// so it scales with the rest of the interface rather than with the window.
/// That is `Interface\Icons\` art at its own size, the size every action
/// button and bag square draws it at.
pub(crate) fn carried_batch(painter: &mut Painter, path: &str, at: Vec2) -> Vec<Batch> {
    let Some(image) = painter.textures.texture(painter.images, painter.assets, path) else {
        return Vec::new();
    };
    let mut emit = Emitter::new();
    let half = CARRIED_ICON * painter.view.scale as f32 / 2.0;
    emit.quad(
        &image,
        Blend::Alpha,
        [at.x - half, at.y - half, at.x + half, at.y + half],
        [0.0, 0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
        None,
    );
    let mut batches = emit.into_batches();
    for batch in &mut batches {
        batch.z = CARRIED_Z;
    }
    batches
}

/// One item: a textured quad, a solid fill, a line of text, or a frame's own
/// backdrop.
fn one(emit: &mut Emitter, painter: &mut Painter, item: &Item) {
    let view = painter.view;
    let rect = px(item.rect, view);
    let clip = item.clip.map(|c| px(c, view));
    let scale = view.scale as f32;
    let paint = match &item.content {
        Content::Region(paint) => paint,
        Content::Background(backdrop) => {
            background(emit, painter, backdrop, rect, item.alpha, scale, clip);
            return;
        }
        Content::Border(backdrop) => {
            border(emit, painter, backdrop, rect, item.alpha, scale, clip);
            return;
        }
        Content::Bar(bar) => {
            fill(emit, painter, bar, rect, item.alpha, clip);
            return;
        }
        // Both of these are drawn by their own pass: [`item_batches`] places a
        // barrier for them and does not call `one`. The arms remain so that a
        // caller of `one` outside that loop still draws a `<Model>`.
        Content::Model(scene) => {
            model(emit, painter, scene, rect, item.alpha, clip);
            return;
        }
        Content::Minimap(_) => return,
    };
    let colour = tint(paint.colour, item.alpha);

    if let Some(word) = paint.text.as_deref().filter(|t| !t.is_empty()) {
        label(emit, painter, rect, paint, word, colour, scale, clip);
        return;
    }
    // A `FontString` paints text or nothing. Its colour belongs to the glyphs;
    // when it reached the solid-fill fallback below, every empty
    // `MessageFrame`'s font declaration was drawn as a gold bar. See
    // [`Paint::is_font`].
    if paint.is_font {
        return;
    }
    // A portrait is checked before a path, because a region can have both:
    // `TargetPortrait` is declared with art in the XML and filled by
    // `SetPortraitTexture` at run time, so the picture takes precedence and
    // the path stays as the fallback for a unit whose model has not loaded.
    //
    // The portrait takes the same tint as every other texture. `TargetFrame.lua`
    // greys the portrait to `0.35` for a tapped mob, tints it blue for a
    // friendly one out of range and red for a hostile one, and fades it with
    // the whole frame when the target dies. Painting a portrait without the
    // tint would drop all four.
    if let Some(image) = paint
        .portrait
        .as_deref()
        .and_then(|unit| painter.portraits.image(unit))
    {
        let uv_of_rect = rect;
        disc(emit, &image, Blend::Alpha, rect, colour, clip, move |p| {
            Vec2::new(
                (p.x - uv_of_rect[0]) / (uv_of_rect[2] - uv_of_rect[0]).max(f32::EPSILON),
                (p.y - uv_of_rect[1]) / (uv_of_rect[3] - uv_of_rect[1]).max(f32::EPSILON),
            )
        });
        return;
    }
    match paint.texture.as_deref() {
        Some(path) => {
            let blend = Blend::of(paint.blend);
            let Some(image) = painter.textures.texture(painter.images, painter.assets, path)
            else {
                return;
            };
            // A rotated texture is drawn with the quad that can rotate. The
            // world map's player arrow is the only one in either shipped
            // directory; see [`crate::lua::widgets::regions::set_rotation`]. It
            // ignores `SetTexCoord`, which no rotated region sets.
            if paint.rotation != 0.0 {
                turned_quad(emit, &image, blend, rect, paint.rotation, colour, clip);
                return;
            }
            if let Some(corners) = paint.corners {
                corner_quad(emit, &image, blend, rect, corners, colour, clip);
                return;
            }
            let uv = paint.coords.map_or([0.0, 0.0, 1.0, 1.0], |[l, r, t, b]| [l, t, r, b]);
            emit.quad(&image, blend, rect, uv, colour, clip);
        }
        // A colour with no path is a solid fill: every backdrop in the
        // directory, and `SetTexture(0, 0, 0, 0.5)` is how they are written.
        None => {
            // Except a portrait still waiting for its picture: a portrait
            // region's colour is the picture's tint, and painting it alone
            // drew a white square in every round portrait hole while the
            // portrait was rendering, or permanently for a unit that gets no
            // portrait. Nothing is drawn, so the frame's own art shows
            // through.
            if paint.portrait.is_some() {
                return;
            }
            let white = painter.textures.white(painter.images);
            emit.quad(
                &white,
                Blend::Alpha,
                rect,
                [0.0, 0.0, 1.0, 1.0],
                solid_tint(paint.colour, item.alpha),
                clip,
            );
        }
    }
}

/// A backdrop's fill: the frame's rectangle, shrunk by the backdrop's insets,
/// tiled at its own period.
///
/// A `<Backdrop>` has no object: it is a record on the frame, painted at the
/// frame's rectangle as a tiled fill and eight border pieces. The layout of
/// the `edgeFile` is in [`vale_assets::interface::backdrop`] and the geometry
/// in [`crate::lua::widgets::backdrop`].
///
/// The insets place the fill's edge at the bright line inside each border
/// piece. A fill drawn to the frame's edge would show through the border's
/// semi-transparent outer texels as a halo. The tooltip's edge is 16 and its
/// insets are 5. A tiled fill repeats at `tileSize` however big the panel is;
/// an untiled one is stretched over the inset rectangle, which is what
/// `tile="false"` means. `<TileSize>` on a fill smaller than one tile is not
/// modelled.
fn background(
    emit: &mut Emitter,
    painter: &mut Painter,
    backdrop: &crate::lua::widgets::backdrop::Backdrop,
    rect: [f32; 4],
    alpha: f32,
    scale: f32,
    clip: Option<[f32; 4]>,
) {
    let Some(path) = backdrop.bg.as_deref() else {
        return;
    };
    let [left, right, top, bottom] = backdrop.insets.map(|inset| inset * scale);
    let inner = [rect[0] + left, rect[1] + top, rect[2] - right, rect[3] - bottom];
    if inner[2] - inner[0] <= 0.0 || inner[3] - inner[1] <= 0.0 {
        return;
    }
    let colour = tint(backdrop.colour, alpha);
    let (image, uv) = if backdrop.tile {
        let period = (backdrop.tile_size * scale).max(1.0);
        (
            painter.textures.tiled(painter.images, painter.assets, path),
            [
                0.0,
                0.0,
                (inner[2] - inner[0]) / period,
                (inner[3] - inner[1]) / period,
            ],
        )
    } else {
        (
            painter.textures.texture(painter.images, painter.assets, path),
            [0.0, 0.0, 1.0, 1.0],
        )
    };
    let Some(image) = image else { return };
    emit.quad(&image, Blend::Alpha, inner, uv, colour, clip);
}

/// A backdrop's border: eight pieces, inside the frame and flush with its
/// edges.
///
/// Four `edgeSize` squares at the corners and four runs between them. The
/// runs tile at the same period rather than stretching. A frame too small for
/// its two corners gets no runs and the corners overlap, as in the 1.12.1
/// client.
fn border(
    emit: &mut Emitter,
    painter: &mut Painter,
    backdrop: &crate::lua::widgets::backdrop::Backdrop,
    rect: [f32; 4],
    alpha: f32,
    scale: f32,
    clip: Option<[f32; 4]>,
) {
    use vale_assets::interface::backdrop as strip;
    let Some(path) = backdrop.edge.as_deref() else {
        return;
    };
    let colour = tint(backdrop.border_colour, alpha);
    let edge = (backdrop.edge_size * scale).max(1.0);
    let Some(pieces) =
        painter.textures.edge_pieces(painter.images, painter.assets, path).cloned()
    else {
        return;
    };
    let [l, t, r, b] = rect;
    let run_x = (r - l - 2.0 * edge).max(0.0);
    let run_y = (b - t - 2.0 * edge).max(0.0);
    let placed = [
        (strip::TOPLEFT, l, t, edge, edge),
        (strip::TOPRIGHT, r - edge, t, edge, edge),
        (strip::BOTTOMLEFT, l, b - edge, edge, edge),
        (strip::BOTTOMRIGHT, r - edge, b - edge, edge, edge),
        (strip::LEFT, l, t + edge, edge, run_y),
        (strip::RIGHT, r - edge, t + edge, edge, run_y),
        (strip::TOP, l + edge, t, run_x, edge),
        (strip::BOTTOM, l + edge, b - edge, run_x, edge),
    ];
    for (piece, x, y, width, height) in placed {
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let Some(image) = pieces.get(piece) else {
            continue;
        };
        emit.quad(
            image,
            Blend::Alpha,
            [x, y, x + width, y + height],
            [0.0, 0.0, width / edge, height / edge],
            colour,
            clip,
        );
    }
}

/// A status bar's fill, cropped rather than squashed.
///
/// [`crate::lua::widgets::draw`] has already cut the rectangle to the
/// fraction. Here the texture is cropped by the same amount, so a bar at 40%
/// shows the left 40% of `UI-StatusBar` at its own scale. Squashing the whole
/// gradient into 40% of the width draws something that looks like a bar but
/// is wrong at every value. The crop runs the way the rectangle was cut: from
/// the left, or from the bottom for a vertical bar.
///
/// A bar with a colour and no texture is a solid fill, which is what
/// `SetStatusBarColor` alone leaves: the loot-roll bars and the two colour
/// pickers.
fn fill(
    emit: &mut Emitter,
    painter: &mut Painter,
    bar: &crate::lua::widgets::statusbar::Bar,
    rect: [f32; 4],
    alpha: f32,
    clip: Option<[f32; 4]>,
) {
    let colour = solid_tint(bar.colour, alpha);
    let Some(path) = bar.texture.as_deref() else {
        let white = painter.textures.white(painter.images);
        emit.quad(&white, Blend::Alpha, rect, [0.0, 0.0, 1.0, 1.0], colour, clip);
        return;
    };
    let Some(image) = painter.textures.texture(painter.images, painter.assets, path) else {
        return;
    };
    let uv = if bar.vertical {
        [0.0, 1.0 - bar.fraction, 1.0, 1.0]
    } else {
        [0.0, 0.0, bar.fraction, 1.0]
    };
    emit.quad(&image, Blend::Alpha, rect, uv, colour, clip);
}

/// A font string — layout in [`text`], glyphs off the atlas, and the game's
/// own paint order: outline, then shadow, then the word itself.
#[allow(clippy::too_many_arguments)]
fn label(
    emit: &mut Emitter,
    painter: &mut Painter,
    rect: [f32; 4],
    paint: &Paint,
    word: &str,
    colour: [f32; 4],
    scale: f32,
    clip: Option<[f32; 4]>,
) {
    let size = if paint.font_height > 0.0 {
        paint.font_height
    } else {
        vale_assets::interface::font::DEFAULT_HEIGHT
    } * scale;
    let Some(face) = painter.fonts.face_index(painter.assets, paint.font.as_deref()) else {
        return;
    };

    let runs_owned = crate::lua::widgets::text::runs(word);
    let runs: Vec<text::Run> = runs_owned
        .iter()
        .map(|run| text::Run { text: run.text.as_ref(), colour: run.colour })
        .collect();
    let placed = {
        let Some(scaled) = painter.fonts.scaled(face, size) else {
            return;
        };
        struct M<'f>(ab_glyph::PxScaleFont<&'f ab_glyph::FontArc>);
        impl vale_assets::interface::font::Metrics for M<'_> {
            fn advance(&self, ch: char) -> f32 {
                use ab_glyph::ScaleFont;
                self.0.h_advance(self.0.glyph_id(ch))
            }
            fn kern(&self, a: char, b: char) -> f32 {
                use ab_glyph::ScaleFont;
                self.0.kern(self.0.glyph_id(a), self.0.glyph_id(b))
            }
        }
        impl text::Metrics for M<'_> {
            fn ascent(&self) -> f32 {
                use ab_glyph::ScaleFont;
                self.0.ascent()
            }
            fn descent(&self) -> f32 {
                use ab_glyph::ScaleFont;
                self.0.descent()
            }
            fn line_gap(&self) -> f32 {
                use ab_glyph::ScaleFont;
                self.0.line_gap()
            }
        }
        text::layout(
            &runs,
            &M(scaled),
            rect,
            paint.wrap,
            paint.max_rows,
            text::Justify { horizontal: paint.justify_h, vertical: paint.justify_v },
            scale,
        )
    };
    if placed.is_empty() {
        return;
    }
    // The layout above is in interface pixels and the raster below is in
    // device ones: `size` spaces the line, `size * dpi` draws the glyphs.
    let raster = size * if painter.dpi.is_finite() && painter.dpi > 0.0 { painter.dpi } else { 1.0 };

    // Whether the glyphs come from more than one atlas page. The passes below
    // are one batch per page run, and every batch of an item has the same z,
    // so the phase draws equal-z batches in whatever order it sorts them. On
    // one page each pass merges into the same batch and the order inside a
    // batch is the emission order, which is correct. Across pages, a page-1
    // outline batch could draw after the page-1 glyph batch and cover the
    // glyphs with their own black copies: zone text "Ironforge" with black
    // shapes over the n, f and g, which had been rasterised on a second page.
    // So a text that spans pages puts the shadow and the glyphs on layers of
    // their own above the outline. A text on one page stays one batch.
    let spans_pages = spans_pages(placed.iter().filter_map(|glyph| {
        painter
            .fonts
            .glyph(painter.images, face, raster, glyph.ch)
            .map(|sprite| sprite.page)
    }));

    // The outline first, then the shadow, then the glyphs: the game's order,
    // and the only one that leaves the outline around the text rather than
    // over it.
    //
    // Eight copies in black at the face's outline radius, with the text's
    // alpha so an outline fades with its string. At these sizes an outlined
    // glyph in the 1.12.1 client has the same shape a ring of offsets makes;
    // four copies would leave the diagonals open. The outline is skipped for a
    // face that declares none, which is most of the interface.
    let radius = paint.outline.radius() * scale;
    if radius > 0.0 {
        let ring = [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 0.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
        ];
        for (dx, dy) in ring {
            let black = [0.0, 0.0, 0.0, colour[3]];
            glyphs(emit, painter, &placed, face, raster, [dx * radius, dy * radius], Some(black), clip);
        }
    }
    if let Some((offset, shadow_colour)) = paint.shadow {
        if spans_pages {
            emit.layer();
        }
        glyphs(
            emit,
            painter,
            &placed,
            face,
            raster,
            // `<Shadow>`: one offset copy in the shadow's colour, under the
            // text. `MasterFont`'s shadow is `(1, -1)` black and
            // `GameFontNormal` inherits it, so it applies to nearly every word
            // the interface draws. The game's y is up and the screen's is
            // down, hence the negation.
            [offset[0] * scale, -offset[1] * scale],
            Some(tint(shadow_colour, 1.0)),
            clip,
        );
    }
    let run_tints: Vec<Option<[f32; 4]>> = placed
        .iter()
        .map(|p| p.colour.map(|c| tint(c, 1.0)))
        .collect();
    if spans_pages {
        emit.layer();
    }
    glyphs_with(emit, painter, &placed, face, raster, [0.0, 0.0], colour, &run_tints, clip);
}

/// Whether a text's glyphs are on more than one atlas page. See the note in
/// the text function where it is asked.
fn spans_pages(pages: impl IntoIterator<Item = usize>) -> bool {
    let mut pages = pages.into_iter();
    let Some(first) = pages.next() else {
        return false;
    };
    pages.any(|page| page != first)
}

/// One pass over the placed glyphs at one offset, in one colour.
#[allow(clippy::too_many_arguments)]
fn glyphs(
    emit: &mut Emitter,
    painter: &mut Painter,
    placed: &[text::Placed],
    face: usize,
    raster: f32,
    offset: [f32; 2],
    over: Option<[f32; 4]>,
    clip: Option<[f32; 4]>,
) {
    let fallback = over.unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let none: Vec<Option<[f32; 4]>> = vec![over; placed.len()];
    glyphs_with(emit, painter, placed, face, raster, offset, fallback, &none, clip);
}

/// The general form of [`glyphs`], with a per-glyph colour override for the
/// coloured runs of the main pass.
#[allow(clippy::too_many_arguments)]
fn glyphs_with(
    emit: &mut Emitter,
    painter: &mut Painter,
    placed: &[text::Placed],
    face: usize,
    raster: f32,
    offset: [f32; 2],
    fallback: [f32; 4],
    overrides: &[Option<[f32; 4]>],
    clip: Option<[f32; 4]>,
) {
    for (glyph, over) in placed.iter().zip(overrides) {
        let Some(sprite) = painter.fonts.glyph(painter.images, face, raster, glyph.ch) else {
            continue;
        };
        let Some(image) = painter.fonts.page_image(sprite.page) else {
            continue;
        };
        let (uv_min, uv_max) = UiFonts::uv(&sprite);
        // Placed on the device pixel grid, so the quad spans one screen pixel
        // per atlas texel; see [`fonts::quad`].
        let rect = fonts::quad(
            &sprite,
            [glyph.x + offset[0], glyph.baseline + offset[1]],
            painter.dpi,
        );
        emit.quad(
            &image,
            Blend::Alpha,
            rect,
            [uv_min[0], uv_min[1], uv_max[0], uv_max[1]],
            over.unwrap_or(fallback),
            clip,
        );
    }
}

/// A `<Model>` frame's contents: the file's own triangles, placed in the
/// frame's rectangle, or the unit's paper doll.
///
/// This adds two things to `vale_assets::interface::uimodel::flatten`: the
/// mapping from that function's `0..1` box to the rectangle on the screen, and
/// the y flip, because the game's models and rectangles are y-up and the
/// screen is y-down. Blend modes 3 and 4 are the additive pair and are drawn
/// additively.
fn model(
    emit: &mut Emitter,
    painter: &mut Painter,
    scene: &crate::lua::widgets::model::Scene,
    rect: [f32; 4],
    alpha: f32,
    clip: Option<[f32; 4]>,
) {
    // A paper doll is checked before a file, and the two are exclusive rather
    // than layered. A `<Model>` frame holds either a file (the cooldown swirl,
    // the login backdrop), flattened into triangles here, or a unit, which is
    // a render target [`crate::render::paperdoll`] drew and this only places.
    // None of the five `<PlayerModel>` frames is given a file, so the branch
    // is on which of the two the scene has. The picture may not be ready: on
    // the frame a panel is opened, and on any frame while the character's
    // model is loading. Nothing is drawn meanwhile rather than a fill, which
    // would show as a white square.
    if scene.unit.is_some() {
        if let Some(image) = painter.dolls.image(&scene.frame) {
            // Snapped to whole device pixels, as a glyph is: a quad that
            // starts part-way into a pixel is sampled between texels across
            // its whole width, which softens the picture.
            let dpi = if painter.dpi.is_finite() && painter.dpi > 0.0 { painter.dpi } else { 1.0 };
            let snap = |v: f32| (v * dpi).round() / dpi;
            let rect = [snap(rect[0]), snap(rect[1]), snap(rect[2]), snap(rect[3])];
            emit.quad(
                &image,
                Blend::Alpha,
                rect,
                [0.0, 0.0, 1.0, 1.0],
                tint([1.0, 1.0, 1.0, 1.0], alpha),
                clip,
            );
        }
        return;
    }
    // Read here, never loaded. The parse belongs to the `<Model>` tick in
    // [`crate::lua::widgets::model`], the system allowed to read the archives;
    // a paint pass that loaded on demand would do it in the middle of a frame.
    let Some(m2) = painter.models.get(&scene.file) else {
        if painter.reported_models.insert(scene.file.clone()) {
            info!("interface mesh: a <Model> is on screen and {} is not loaded", scene.file);
        }
        return;
    };
    let sequence = m2
        .skeleton
        .as_ref()
        .and_then(|skeleton| skeleton.sequences.get(scene.sequence as usize));
    let elapsed_ms = (scene.elapsed * 1000.0).max(0.0) as u32;
    let batches =
        vale_assets::interface::uimodel::flatten(m2, sequence, elapsed_ms, painter.now_ms);
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    for batch in batches {
        if batch.is_invisible() {
            continue;
        }
        let blend = if batch.blend == 3 || batch.blend == 4 {
            Blend::Additive
        } else {
            Blend::Alpha
        };
        let Some(image) = painter.textures.texture(painter.images, painter.assets, &batch.texture)
        else {
            continue;
        };
        let colour = tint(batch.colour, alpha);
        let vertices: Vec<V> = batch
            .vertices
            .iter()
            .map(|v| V {
                pos: Vec2::new(rect[0] + v.x * width, rect[3] - v.y * height),
                uv: Vec2::new(v.u, v.v),
                // The batch's colour times the vertex's own — white for
                // geometry, the over-life ramp for a sprite.
                colour: [
                    colour[0] * v.tint[0],
                    colour[1] * v.tint[1],
                    colour[2] * v.tint[2],
                    colour[3] * v.tint[3],
                ],
            })
            .collect();
        let indices: Vec<u32> = batch.indices.iter().map(|&i| u32::from(i)).collect();
        emit.raw(&image, blend, &vertices, &indices, clip);
    }
}

/// A texture turned about its rectangle's centre, clockwise on the screen.
///
/// The screen's y axis points down, so the ordinary positive rotation
/// (`x cos - y sin`, `x sin + y cos`) is clockwise here with no extra
/// negation. This is easy to get backwards;
/// [`crate::lua::panels::worldmap::arrow_angle`] holds the derivation, and
/// the test `the_arrow_points_where_the_character_looks` checks the chain.
fn turned_quad(
    emit: &mut Emitter,
    image: &Handle<Image>,
    blend: Blend,
    rect: [f32; 4],
    radians: f32,
    colour: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let centre = Vec2::new((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
    let half = Vec2::new((rect[2] - rect[0]) / 2.0, (rect[3] - rect[1]) / 2.0);
    turned(emit, image, blend, centre, half, radians, colour, clip);
}

/// The same quad given by centre and half-size, for the minimap's arrows.
/// The corners go top left, top right, bottom right, bottom left: the uv order
/// of a quad, so the picture's top is at the quad's top before the rotation.
#[allow(clippy::too_many_arguments)]
fn turned(
    emit: &mut Emitter,
    image: &Handle<Image>,
    blend: Blend,
    centre: Vec2,
    half: Vec2,
    radians: f32,
    colour: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let (sin, cos) = radians.sin_cos();
    let vertices: Vec<V> = [
        (-half.x, -half.y, 0.0, 0.0),
        (half.x, -half.y, 1.0, 0.0),
        (half.x, half.y, 1.0, 1.0),
        (-half.x, half.y, 0.0, 1.0),
    ]
    .into_iter()
    .map(|(dx, dy, u, v)| V {
        pos: Vec2::new(centre.x + dx * cos - dy * sin, centre.y + dx * sin + dy * cos),
        uv: Vec2::new(u, v),
        colour,
    })
    .collect();
    emit.raw(image, blend, &vertices, &[0, 1, 2, 0, 2, 3], clip);
}

/// A quad whose four corners each have their own texture coordinate: the
/// eight-argument `SetTexCoord`.
///
/// The rectangle is the region's own; the eight numbers rotate the picture
/// inside it. `TaxiFrame.lua`'s `DrawRouteLine` is the only caller in either
/// shipped directory. It anchors the texture to the bounding box of the line
/// it wants, then solves the eight uvs so that the horizontal line art
/// crosses that box at the right angle, so ignoring them draws every route as
/// a rectangle.
///
/// The argument order is the game's, and it is not the corner order of a
/// quad: `(ULx, ULy, LLx, LLy, URx, URy, LRx, LRy)`, that is upper-left,
/// lower-left, upper-right, lower-right. Taking them in the order they arrive
/// draws an hourglass.
#[allow(clippy::too_many_arguments)]
fn corner_quad(
    emit: &mut Emitter,
    image: &Handle<Image>,
    blend: Blend,
    rect: [f32; 4],
    corners: [f32; 8],
    colour: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let [ul_x, ul_y, ll_x, ll_y, ur_x, ur_y, lr_x, lr_y] = corners;
    let vertices = [
        V { pos: Vec2::new(rect[0], rect[1]), uv: Vec2::new(ul_x, ul_y), colour },
        V { pos: Vec2::new(rect[2], rect[1]), uv: Vec2::new(ur_x, ur_y), colour },
        V { pos: Vec2::new(rect[2], rect[3]), uv: Vec2::new(lr_x, lr_y), colour },
        V { pos: Vec2::new(rect[0], rect[3]), uv: Vec2::new(ll_x, ll_y), colour },
    ];
    emit.raw(image, blend, &vertices, &[0, 1, 2, 0, 2, 3], clip);
}

/// A textured disc inscribed in a rectangle, with a half-pixel faded rim, and
/// the caller giving the texture coordinate of each point.
///
/// A portrait render target is square and every portrait hole in the
/// interface is round, so the corners have to be removed. The 1.12.1 client
/// uses an alpha mask for this:
/// `Interface\CharacterFrame\TempPortraitAlphaMask.blp` (and a `Small`
/// sibling for the party frames). The mask is a hard disc. Decoded from the
/// archive it is 128x128 DXT3, and its alpha grouped by radius from the
/// centre is
///
/// ```text
/// r <= 63   255      (every pixel, no exception)
/// r == 64   34..255  the one transitional ring
/// r >= 65   0
/// ```
///
/// That is a circle inscribed in the square, touching the edges at the
/// midpoints, with a one-texel rim and no feathering, so cutting the quad into
/// a disc gives the same shape as the mask and needs no second texture. The
/// half-pixel fade stands in for the mask's rim texel.
///
/// The uv is a closure because the two callers map differently: a portrait's
/// texture covers the rectangle, and a minimap draws one 533-yard tile at a
/// time through the same outline, so its mapping is the tile's own affine
/// rather than the widget's. `vale_assets::tables::minimap` measures the
/// minimap's mask and finds the same shape.
fn disc(
    emit: &mut Emitter,
    image: &Handle<Image>,
    blend: Blend,
    rect: [f32; 4],
    colour: [f32; 4],
    clip: Option<[f32; 4]>,
    uv_at: impl Fn(Vec2) -> Vec2,
) {
    const SEGMENTS: usize = DISC_SEGMENTS;
    let centre = Vec2::new((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
    let radius = ((rect[2] - rect[0]).min(rect[3] - rect[1]) / 2.0).max(0.0);
    let faded = [colour[0], colour[1], colour[2], 0.0];
    let mut vertices = vec![V { pos: centre, uv: uv_at(centre), colour }];
    for step in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * step as f32 / SEGMENTS as f32;
        let (sin, cos) = angle.sin_cos();
        for (r, ring_colour) in [(radius - 0.5, colour), (radius, faded)] {
            let p = Vec2::new(centre.x + cos * r, centre.y + sin * r);
            vertices.push(V { pos: p, uv: uv_at(p), colour: ring_colour });
        }
    }
    let mut indices = Vec::with_capacity(SEGMENTS * 9);
    for step in 0..SEGMENTS as u32 {
        let inner = 1 + step * 2;
        let next_inner = inner + 2;
        indices.extend([0, inner, next_inner]);
        indices.extend([inner, inner + 1, next_inner]);
        indices.extend([inner + 1, next_inner + 1, next_inner]);
    }
    emit.raw(image, blend, &vertices, &indices, clip);
}

/// A `<Minimap>` frame's contents: the ground around the character through the
/// disc, or inside a building its pictures (see [`interior`]), then the dots
/// and the arrow that shows which way the character faces.
///
/// ```text
/// radius_yards(zoom, indoors)   how far it sees        one radius per zoom level
/// tiles_in_view(x, y, radius)   which pictures, where  north up, west left
/// md5translate.trs              …and what each is called
/// disc()                        the shape Textures\MinimapMask is
/// ```
///
/// Nothing is composited. Each 256x256 tile is uploaded once into a cache
/// ([`UiTextures::minimap_texture`]) and drawn through the disc, clipped to
/// the tile's own window by the triangle clipper. The tile boundaries are
/// axis-aligned because the map does not rotate, so a rectangular clip per
/// tile is exact. Each tile is sampled clamped to its own edge, which leaves a
/// one-texel seam between tiles: at the default zoom a texel is about a yard
/// and the frame is 140 units across, so the join is a sub-pixel
/// discontinuity rather than a visible line.
///
/// Nothing is drawn before there is a world, rather than a black disc. The two
/// glue screens and `--audit` are in this state, and in the 1.12.1 client the
/// frame there shows only its own border art too.
fn minimap(
    emit: &mut Emitter,
    painter: &mut Painter,
    item: &Item,
    widget: crate::lua::widgets::minimap::MinimapWidget,
) {
    let view = painter.place;
    let rect = px(item.rect, painter.view);
    let clip = item.clip.map(|c| px(c, painter.view));
    if !view.in_world || rect[2] - rect[0] <= 0.0 || rect[3] - rect[1] <= 0.0 {
        return;
    }
    let colour = tint([1.0, 1.0, 1.0, 1.0], item.alpha);
    let index = painter.assets.minimap_tiles();
    painter.textures.minimap_tick += 1;
    let radius =
        vale_assets::tables::minimap::radius_yards(widget.level(view.indoors), view.indoors);
    // Inside a building the terrain is not drawn; see `interior`.
    let terrain = if view.indoors {
        interior(emit, painter, rect, clip, colour, item.alpha, radius);
        Vec::new()
    } else {
        vale_assets::tables::minimap::tiles_in_view(view.position.0, view.position.1, radius)
    };
    for tile in terrain {
        let Some(path) = index.texture(&view.directory, tile.tile.0, tile.tile.1) else {
            // A tile the index does not list is black, which is what the
            // 1.12.1 client shows for a tile with no picture. The index is
            // sparse by construction.
            continue;
        };
        let Some(image) = painter.textures.minimap_texture(painter.images, painter.assets, &path)
        else {
            continue;
        };
        let [left, top, right, bottom] = tile.rect;
        let at = |u: f32, v: f32| {
            Vec2::new(
                rect[0] + u * (rect[2] - rect[0]),
                rect[1] + v * (rect[3] - rect[1]),
            )
        };
        let min = at(left, top);
        let max = at(right, bottom);
        // The tile's own window, cut down by the item's if it has one.
        let mut window = [min.x, min.y, max.x, max.y];
        if let Some([cl, ct, cr, cb]) = clip {
            window = [window[0].max(cl), window[1].max(ct), window[2].min(cr), window[3].min(cb)];
        }
        if window[2] - window[0] <= 0.0 || window[3] - window[1] <= 0.0 {
            continue;
        }
        let span = (max - min).max(Vec2::splat(f32::EPSILON));
        disc(emit, &image, Blend::Alpha, rect, colour, Some(window), move |p| {
            (p - min) / span
        });
    }
    painter.textures.trim_minimap();

    // The dots and the markers are drawn above the tiles, which sharing the
    // item's z does not guarantee; see [`Emitter::layer`]. Every tile's disc
    // covers the whole minimap rectangle and the tile under the character is
    // opaque, so a dot that loses the tie is hidden completely. The arrow takes
    // a layer of its own above these.
    emit.layer();
    blips(emit, painter, rect, radius, colour, clip);
    emit.layer();

    let Some(arrow) = painter.textures.texture(painter.images, painter.assets, PLAYER_ARROW)
    else {
        return;
    };
    let half = (rect[2] - rect[0]).min(rect[3] - rect[1]) * PLAYER_ARROW_FRACTION / 2.0;
    turned(
        emit,
        &arrow,
        Blend::Alpha,
        Vec2::new((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0),
        Vec2::splat(half),
        crate::lua::panels::worldmap::arrow_angle(view.facing),
        colour,
        clip,
    );
}

/// The arrow in the middle of the minimap: the widget's
/// `minimapPlayerModel`, with `.mdx` replaced by the texture the model uses.
const PLAYER_ARROW: &str = r"Interface\Minimap\MinimapArrow";

/// The size of [`PLAYER_ARROW`], as a fraction of the frame's width.
///
/// The art is 32x32 and the frame is 140, which is 0.229. This matches the
/// 1.12.1 client only if it draws the model at the texture's size, which has
/// not been checked. It is the one value on this widget chosen by eye.
const PLAYER_ARROW_FRACTION: f32 = 32.0 / 140.0;

/// How many sides the minimap's disc has: the same count [`disc`] draws, so a
/// building picture is cut along the edge the terrain is. Enough for the rim
/// to look round at the largest portrait in the interface (the character
/// sheet's, at about 60 pixels), and few enough that fourteen discs have
/// negligible cost.
const DISC_SEGMENTS: usize = 48;

/// The minimap inside a building: a black disc, which is what the 1.12.1
/// client clears its interior map to, and the building's pictures over it in
/// [`vale_assets::tables::minimap::place_pictures`]' order, each a turned
/// quad cut to the disc and alpha-blended so a floor below shows through the
/// gaps in the one above. The uv is inset by half a texel at each edge, as
/// the client draws them.
fn interior(
    emit: &mut Emitter,
    painter: &mut Painter,
    rect: [f32; 4],
    clip: Option<[f32; 4]>,
    colour: [f32; 4],
    alpha: f32,
    radius: f32,
) {
    use vale_assets::tables::minimap as rule;
    let white = painter.textures.white(painter.images);
    disc(
        emit,
        &white,
        Blend::Alpha,
        rect,
        solid_tint([0.0, 0.0, 0.0, 1.0], alpha),
        clip,
        |_| Vec2::splat(0.5),
    );
    let view = painter.place;
    let Some(inside) = view.interior.as_ref() else {
        return;
    };
    let index = painter.assets.minimap_tiles();
    let centre = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
    let reach = ((rect[2] - rect[0]).min(rect[3] - rect[1]) / 2.0 - 0.5).max(0.0);
    let at = |[u, v]: [f32; 2]| [rect[0] + u * (rect[2] - rect[0]), rect[1] + v * (rect[3] - rect[1])];
    let position = [view.position.0, view.position.1, view.elevation];
    for picture in inside.pictures(position, radius) {
        let polygon =
            rule::clip_to_disc(picture.corners.map(at), rule::PICTURE_UV, centre, reach, DISC_SEGMENTS);
        if polygon.is_empty() {
            continue;
        }
        let Some(path) =
            index.interior_texture(&inside.model.stem, picture.group, picture.x, picture.y)
        else {
            continue;
        };
        let Some(image) = painter.textures.minimap_texture(painter.images, painter.assets, &path)
        else {
            continue;
        };
        let (width, height) = painter
            .images
            .get(&image)
            .map_or((256.0, 256.0), |i| (i.width().max(1) as f32, i.height().max(1) as f32));
        let inset = Vec2::new(0.5 / width, 0.5 / height);
        let vertices: Vec<V> = polygon
            .iter()
            .map(|(p, uv)| V {
                pos: Vec2::from(*p),
                uv: inset + Vec2::from(*uv) * (Vec2::ONE - inset * 2.0),
                colour,
            })
            .collect();
        let indices: Vec<u32> =
            (1..vertices.len() as u32 - 1).flat_map(|i| [0, i, i + 1]).collect();
        emit.raw(&image, Blend::Alpha, &vertices, &indices, clip);
    }
}

/// The minimap dots and the two kinds of marker.
///
/// A dot is a cell of `ObjectIcons` at its projected place inside the disc; a
/// marker inside the disc is its `POIIcons` cell there, and beyond the rim it
/// is an arrow at the rim turned to point the way. The rule for which unit is
/// which dot is `vale_assets::look::blips`, and the list itself is
/// [`crate::interface::minimap::MinimapView::blips`].
fn blips(
    emit: &mut Emitter,
    painter: &mut Painter,
    rect: [f32; 4],
    radius: f32,
    colour: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    use vale_assets::look::blips as rule;
    use crate::interface::minimap::MarkerKind;
    let view = painter.place;
    if view.blips.is_empty() && view.markers.is_empty() {
        return;
    }
    let (width, height) = (rect[2] - rect[0], rect[3] - rect[1]);
    let side = width.min(height);
    let at = |u: f32, v: f32| Vec2::new(rect[0] + u * width, rect[1] + v * height);
    if !view.blips.is_empty() {
        if let Some(sheet) = painter.textures.texture(painter.images, painter.assets, rule::SHEET) {
            for blip in &view.blips {
                let Some((u, v)) = rule::on_disc((0.0, 0.0), (blip.north, blip.west), radius)
                else {
                    continue;
                };
                let half = side * rule::SIZE * blip.kind.scale() / 2.0;
                let c = at(u, v);
                emit.quad(
                    &sheet,
                    Blend::Alpha,
                    [c.x - half, c.y - half, c.x + half, c.y + half],
                    blip.kind.uv(),
                    colour,
                    clip,
                );
            }
        }
    }
    for marker in &view.markers {
        let place = (marker.north, marker.west);
        match rule::on_disc((0.0, 0.0), place, radius) {
            Some((u, v)) => {
                let Some(sheet) =
                    painter.textures.texture(painter.images, painter.assets, rule::POI_SHEET)
                else {
                    continue;
                };
                let cell = match marker.kind {
                    MarkerKind::Poi { icon } => icon,
                    MarkerKind::Corpse => rule::CORPSE_CELL,
                };
                // A `POIIcons` cell is 16 texels where a dot's is 32: drawn at
                // the same size as a dot, which is the parchment's own ratio
                // between the two sheets.
                let half = side * rule::SIZE / 2.0;
                let c = at(u, v);
                emit.quad(
                    &sheet,
                    Blend::Alpha,
                    [c.x - half, c.y - half, c.x + half, c.y + half],
                    rule::poi_uv(cell),
                    colour,
                    clip,
                );
            }
            None => {
                let path = match marker.kind {
                    MarkerKind::Poi { .. } => rule::POI_ARROW,
                    MarkerKind::Corpse => rule::GUIDE_ARROW,
                };
                let Some(art) = painter.textures.texture(painter.images, painter.assets, path)
                else {
                    continue;
                };
                // The world heading to the place, then the same turn the
                // player's arrow takes for a facing: north is up, west is left.
                let bearing = rule::bearing((0.0, 0.0), place);
                let (sin, cos) = bearing.sin_cos();
                let reach = side / 2.0 * rule::EDGE;
                let centre = Vec2::new(
                    (rect[0] + rect[2]) / 2.0 - reach * sin,
                    (rect[1] + rect[3]) / 2.0 - reach * cos,
                );
                let half = side * rule::ARROW_SIZE / 2.0;
                turned(
                    emit,
                    &art,
                    Blend::Alpha,
                    centre,
                    Vec2::splat(half),
                    crate::lua::panels::worldmap::arrow_angle(bearing),
                    colour,
                    clip,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Handle<Image> {
        Handle::default()
    }

    /// A text whose glyphs span two atlas pages draws its glyphs above its
    /// outline whatever order the phase puts equal-z batches in: the main pass
    /// is a layer of its own, so every glyph batch has a higher z than every
    /// outline batch. Without the layer, the page-2 outline and glyph batches
    /// had the same z, and the outline could draw over the glyphs.
    #[test]
    fn a_text_on_two_pages_draws_its_glyphs_over_its_outline() {
        assert!(!spans_pages([0, 0, 0]));
        assert!(spans_pages([0, 1, 0]));
        assert!(!spans_pages(std::iter::empty()));

        let page_a = image();
        let page_b = Handle::<Image>::Uuid(bevy::asset::uuid::Uuid::from_u128(7), std::marker::PhantomData);
        let mut emit = Emitter::new();
        emit.item(0);
        let quad = |emit: &mut Emitter, page: &Handle<Image>| {
            emit.quad(page, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        };
        // The outline pass over "In": I on page a, n on page b.
        quad(&mut emit, &page_a);
        quad(&mut emit, &page_b);
        emit.layer();
        // The glyph pass over the same two.
        quad(&mut emit, &page_a);
        quad(&mut emit, &page_b);
        let batches = emit.into_batches();
        assert_eq!(batches.len(), 4);
        let outline = batches[0].z.max(batches[1].z);
        let glyphs = batches[2].z.min(batches[3].z);
        assert!(glyphs > outline, "every glyph batch sorts above every outline batch");
    }

    #[test]
    fn adjacent_emissions_on_one_texture_and_blend_are_one_batch() {
        let mut emit = Emitter::new();
        let art = image();
        emit.item(0);
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        emit.item(1);
        emit.quad(&art, Blend::Alpha, [1.0, 0.0, 2.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        emit.item(2);
        emit.quad(&art, Blend::Additive, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        let batches = emit.into_batches();
        assert_eq!(batches.len(), 2, "same pair merges, a new blend does not");
        assert_eq!(batches[0].positions.len(), 8);
        // The batch's z is its first item's.
        assert_eq!(batches[0].z, Z_BASE);
        assert_eq!(batches[1].z, Z_BASE + 2.0 * Z_STEP);
    }

    #[test]
    fn a_barrier_stops_the_merge_so_z_can_sort_over_the_gap() {
        let mut emit = Emitter::new();
        let art = image();
        emit.item(0);
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        emit.item(1);
        emit.barrier();
        emit.item(2);
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        let batches = emit.into_batches();
        assert_eq!(batches.len(), 2);
        assert!(batches[1].z > Z_BASE + Z_STEP, "the later batch sorts over the gap");
    }

    /// A layer sorts over the batches before it and under the next item.
    ///
    /// Both conditions are needed. Without the first, the minimap's arrow ties
    /// with the tile under it and is hidden by it. Without the second, an item
    /// with several layers would paint over the panel after it in the list.
    #[test]
    fn a_layer_sorts_within_its_item_and_never_past_the_next() {
        let mut emit = Emitter::new();
        let art = image();
        emit.item(0);
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        emit.layer();
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        emit.item(1);
        // A barrier, because a batch merges across items when the texture and
        // the blend match, and then carries the first item's z. That is correct
        // for the paint order but would hide what this test asserts.
        emit.barrier();
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        let batches = emit.into_batches();
        assert_eq!(batches.len(), 3, "a layer does not merge into the batch under it");
        assert!(batches[1].z > batches[0].z, "the layer is over what preceded it");
        assert!(batches[1].z < batches[2].z, "…and under the next item");
        assert_eq!(batches[2].z, Z_BASE + Z_STEP, "the next item's z is untouched");
    }

    /// The layer counter is per item: an item that layered does not leave the
    /// next one raised, which would put it over the item after that.
    #[test]
    fn a_new_item_starts_back_at_its_own_z() {
        let mut emit = Emitter::new();
        let art = image();
        emit.item(0);
        emit.layer();
        emit.layer();
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        emit.item(1);
        emit.barrier();
        emit.quad(&art, Blend::Alpha, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], None);
        let batches = emit.into_batches();
        assert_eq!(batches[1].z, Z_BASE + Z_STEP);
    }

    #[test]
    fn a_clamped_quad_keeps_its_texels_where_they_were() {
        // A 10-wide quad mapping 0..1 clipped to its right half samples 0.5..1.
        let (rect, uv) = clamp(
            [0.0, 0.0, 10.0, 10.0],
            [0.0, 0.0, 1.0, 1.0],
            Some([5.0, 0.0, 15.0, 10.0]),
        )
        .expect("half survives");
        assert_eq!(rect, [5.0, 0.0, 10.0, 10.0]);
        assert_eq!(uv, [0.5, 0.0, 1.0, 1.0]);
        // A quad wholly outside the window is removed.
        assert!(clamp(
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            Some([5.0, 5.0, 6.0, 6.0])
        )
        .is_none());
    }

    #[test]
    fn the_triangle_clipper_interpolates_uv_across_the_cut() {
        let tri = [
            V { pos: Vec2::new(0.0, 0.0), uv: Vec2::new(0.0, 0.0), colour: [1.0; 4] },
            V { pos: Vec2::new(10.0, 0.0), uv: Vec2::new(1.0, 0.0), colour: [1.0; 4] },
            V { pos: Vec2::new(0.0, 10.0), uv: Vec2::new(0.0, 1.0), colour: [1.0; 4] },
        ];
        let poly = clip_triangle(tri, [0.0, 0.0, 5.0, 10.0]);
        assert!(poly.len() >= 3);
        for v in &poly {
            assert!(v.pos.x <= 5.0 + 1e-4);
            // uv tracks position: u == x/10 everywhere on this triangle.
            assert!((v.uv.x - v.pos.x / 10.0).abs() < 1e-4);
        }
        // A triangle wholly outside vanishes.
        assert!(clip_triangle(tri, [20.0, 20.0, 30.0, 30.0]).is_empty());
    }

    /// The conversion between the two coordinate spaces: the game's is y-up
    /// from the bottom left and the window's is y-down from the top left.
    /// Getting it backwards flips the whole interface top to bottom, which is
    /// hard to see for centred elements and obvious for a bar at the bottom of
    /// the screen.
    #[test]
    fn the_game_space_is_flipped_into_the_screens() {
        let rect = crate::lua::widgets::layout::Rect {
            left: 10.0,
            bottom: 55.0,
            width: 195.0,
            height: 13.0,
        };
        // A window that is exactly the interface's shape: scale 1.0, no bars,
        // and units are pixels.
        //
        // Rounded, because a window is a whole number of pixels and so is the
        // space; see [`crate::lua::widgets::layout::units_wide`]. 1365⅓ is the
        // exact 16:9 width and no window is that wide, so using it here would
        // test a third of a pixel of rounding rather than the flip.
        let exact = Viewport::of(
            crate::lua::widgets::layout::VIRTUAL_WIDTH.round(),
            crate::lua::widgets::layout::VIRTUAL_HEIGHT,
            1.0,
        );
        let [left, top, right, bottom] = px(rect, exact);
        assert_eq!(left, 10.0);
        assert_eq!(right, 205.0);
        // The game's top (68) is 700 from the top of a 768-high screen.
        assert_eq!(top, 700.0);
        assert_eq!(bottom, 713.0);
        // A window twice the size doubles every coordinate: the game's units
        // are virtual, not pixels (see `VIRTUAL_HEIGHT`).
        let doubled = px(
            rect,
            Viewport::of(
                crate::lua::widgets::layout::VIRTUAL_WIDTH.round() * 2.0,
                crate::lua::widgets::layout::VIRTUAL_HEIGHT * 2.0,
                1.0,
            ),
        );
        assert_eq!(doubled[0], 20.0);
        assert_eq!(doubled[1], 1400.0);
        assert_eq!(doubled[3] - doubled[1], 26.0);
    }

    /// A window wider than 16:9 gives the interface the extra width rather than
    /// a bar down each side. A fullscreen or maximised client on a wide monitor
    /// is such a window.
    ///
    /// The test also checks the opposite error: when a widget's rectangle was
    /// multiplied by a scale alone, the interface spread with the window and
    /// every declared size came out wrong. One uniform scale taken from the
    /// height prevents that, and the leftover width is part of the interface's
    /// space.
    #[test]
    fn a_wider_window_gives_the_interface_the_width() {
        let rect = crate::lua::widgets::layout::Rect {
            left: 0.0,
            bottom: 0.0,
            width: crate::lua::widgets::layout::VIRTUAL_WIDTH,
            height: crate::lua::widgets::layout::VIRTUAL_HEIGHT,
        };
        // 2:1 at 768 tall: the space is 1536 units across, the scale is 1 and
        // there is no bar. The rectangle, which covers the 16:9 space, starts
        // at the left edge and does not reach the right.
        let wide = Viewport::of(1536.0, 768.0, 1.0);
        assert!((wide.scale - 1.0).abs() < 1e-9);
        assert!(wide.left.abs() < 1e-9, "{wide:?}");
        let [left, top, right, bottom] = px(rect, wide);
        assert!(left.abs() < 0.01, "{left}");
        assert!(
            (right - crate::lua::widgets::layout::VIRTUAL_WIDTH as f32).abs() < 0.01,
            "{right}"
        );
        // Vertically nothing changes: the space is as tall as the window.
        assert_eq!(top, 0.0);
        assert_eq!(bottom, 768.0);
    }

    /// The player arrow points where the character is looking, on both maps. A
    /// screenshot does not show this reliably, because a mirrored arrow is
    /// correct at north and south and wrong everywhere else.
    ///
    /// The test covers the whole chain: the world's `o` through
    /// [`crate::lua::panels::worldmap::arrow_angle`] into [`turned`], with the
    /// art's tip (measured at row 7 of `MinimapArrow.blp`, so uv `(0.5, 0)`)
    /// checked at the four compass points.
    #[test]
    fn the_arrow_points_where_the_character_looks() {
        use std::f32::consts::{FRAC_PI_2, PI};
        let centre = Vec2::new(100.0, 100.0);
        // Where the middle of the quad's top edge lands, which is the tip.
        let tip = |facing: f32| {
            let mut emit = Emitter::new();
            turned(
                &mut emit,
                &image(),
                Blend::Alpha,
                centre,
                Vec2::splat(10.0),
                crate::lua::panels::worldmap::arrow_angle(facing),
                [1.0; 4],
                None,
            );
            let batches = emit.into_batches();
            let (a, b) = (batches[0].positions[0], batches[0].positions[1]);
            Vec2::new((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0)
        };
        // The screen is y-down, so "up" is a smaller y.
        let north = tip(0.0);
        assert!(north.y < centre.y - 9.0, "facing north draws the tip up: {north:?}");
        assert!((north.x - centre.x).abs() < 0.01);
        // `o` grows towards +y, which is west, and west is left.
        let west = tip(FRAC_PI_2);
        assert!(west.x < centre.x - 9.0, "facing west draws the tip left: {west:?}");
        let south = tip(PI);
        assert!(south.y > centre.y + 9.0, "facing south draws the tip down: {south:?}");
        let east = tip(-FRAC_PI_2);
        assert!(east.x > centre.x + 9.0, "facing east draws the tip right: {east:?}");
    }

    #[test]
    fn the_tints_take_the_byte_space_compensations() {

        // Alpha through byte_space_alpha, rgb linearised: spot the endpoints.
        assert_eq!(tint([1.0, 1.0, 1.0, 1.0], 1.0)[3], 1.0);
        assert_eq!(tint([1.0, 1.0, 1.0, 0.0], 1.0)[3], 0.0);
        let half = tint([0.0, 0.0, 0.0, 0.5], 1.0)[3];
        assert!(half > 0.5, "a byte-space half is more opaque to a linear ROP");
        // The solid fold answers less than the black-source fold for a
        // saturated colour — the rank-bar case the function exists for.
        let solid = solid_tint([0.0, 0.0, 0.75, 0.5], 1.0)[3];
        assert!(solid < half);
    }
}
