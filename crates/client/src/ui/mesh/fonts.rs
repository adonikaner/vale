//! **The game's four typefaces, rasterised glyph by glyph into an atlas.**
//!
//! [`crate::render::labels`] rasterises whole strings, which is right for a
//! population of a few dozen names that rarely change. The interface is the
//! other case — thousands of short strings, a third of them numbers that move
//! at 30 Hz — so the unit here is the **glyph**: each `(face, size, char)` is
//! drawn once into a shelf-packed page and every string is quads over it.
//!
//! The glyphs are drawn **white** and the colour rides the vertex, exactly as
//! the labels atlas does it, so one page serves every tint in the game —
//! including the eight black offset copies an outlined face draws under
//! itself.
//!
//! The em convention matches [`vale_assets::interface::font`]: a
//! `<FontHeight>` is the em size, a glyph scales by `size / units_per_em`, and
//! `ab_glyph`'s `PxScale` — which is relative to the face's own
//! ascent-minus-descent — is converted at the one place a scale is built.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

use ab_glyph::Font;

/// One page of the atlas is this many pixels square. Two or three pages hold a
/// whole session's worth of sizes; a page is 4 MB.
const PAGE: u32 = 1024;

/// A texel of clear space around every glyph, so linear filtering at the quad's
/// edge cannot pull in a neighbour.
const GUTTER: u32 = 1;

/// One rasterised glyph: where it is in which page, and how to place it.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub page: usize,
    /// Texels, for the uv.
    pub at: [u32; 2],
    pub size: [u32; 2],
    /// `px_bounds().min` relative to a pen at the origin: add to the pen x and
    /// the baseline y to place the quad. Negative `y` above the baseline.
    pub bearing: [f32; 2],
}

/// The line metrics of one face at one size, in pixels — what the layout in
/// [`super::text`] spaces lines by.
#[derive(Clone, Copy, Debug)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    face: usize,
    /// Half-pixel quanta of the em size, so 12.0 and 12.24 share a raster.
    size: u16,
    ch: char,
}

struct Page {
    image: Handle<Image>,
    /// The CPU copy the rasteriser draws into; re-uploaded whole on a frame
    /// that added glyphs, which after warm-up is rare.
    pixels: Vec<u8>,
    pen_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    dirty: bool,
}

/// The four faces and every glyph drawn so far.
#[derive(Resource, Default)]
pub struct UiFonts {
    faces: Vec<Option<ab_glyph::FontArc>>,
    looked: bool,
    pages: Vec<Page>,
    glyphs: HashMap<Key, Option<Sprite>>,
}

impl UiFonts {
    /// Load the four faces out of the archives, once. A face that will not
    /// load falls back to the standard one at [`Self::face_index`], which is
    /// the same degradation the egui painter and the reference both take.
    fn load(&mut self, assets: &crate::assets::GameAssets) {
        if self.looked {
            return;
        }
        self.looked = true;
        self.faces = vale_assets::interface::font::FACES
            .iter()
            .map(|(_, path)| {
                assets
                    .with_archive(|archive| archive.read(path).map_err(|e| e.to_string()))
                    .ok()
                    .and_then(|bytes| ab_glyph::FontArc::try_from_vec(bytes).ok())
            })
            .collect();
        let loaded = self.faces.iter().flatten().count();
        if loaded < vale_assets::interface::font::FACES.len() {
            warn!(
                "interface mesh: {loaded} of {} game fonts loaded",
                vale_assets::interface::font::FACES.len()
            );
        }
    }

    /// Which face a `font=` path draws in — the file's own resolution rule,
    /// then the fallback to the standard face for one that did not load.
    pub fn face_index(&mut self, assets: &crate::assets::GameAssets, path: Option<&str>) -> Option<usize> {
        self.load(assets);
        let name = vale_assets::interface::font::face_of(path);
        let wanted = vale_assets::interface::font::FACES
            .iter()
            .position(|(n, _)| *n == name)?;
        if self.faces.get(wanted)?.is_some() {
            return Some(wanted);
        }
        let standard = vale_assets::interface::font::FACES
            .iter()
            .position(|(n, _)| *n == vale_assets::interface::font::DEFAULT_FACE)?;
        self.faces.get(standard)?.as_ref().map(|_| standard)
    }

    /// The scaled face itself, for the layout's metrics.
    pub fn scaled(&self, face: usize, em_px: f32) -> Option<ab_glyph::PxScaleFont<&ab_glyph::FontArc>> {
        let font = self.faces.get(face)?.as_ref()?;
        Some(font.as_scaled(em_scale(font, em_px)))
    }

    /// One glyph, rasterised on first sight. `None` is a real answer — a
    /// space, or a codepoint the face has no contours for — and it is cached
    /// so the question costs a map hit thereafter.
    pub fn glyph(
        &mut self,
        images: &mut Assets<Image>,
        face: usize,
        em_px: f32,
        ch: char,
    ) -> Option<Sprite> {
        let key = Key {
            face,
            size: (em_px * 2.0).round().clamp(0.0, f32::from(u16::MAX)) as u16,
            ch,
        };
        if let Some(known) = self.glyphs.get(&key) {
            return *known;
        }
        let sprite = self.raster(images, key);
        self.glyphs.insert(key, sprite);
        sprite
    }

    fn raster(&mut self, images: &mut Assets<Image>, key: Key) -> Option<Sprite> {
        let font = self.faces.get(key.face)?.as_ref()?.clone();
        let em_px = f32::from(key.size) / 2.0;
        let scale = em_scale(&font, em_px);
        let glyph = font
            .glyph_id(key.ch)
            .with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        let outline = font.outline_glyph(glyph)?;
        let bounds = outline.px_bounds();
        let width = bounds.width().ceil().max(1.0) as u32;
        let height = bounds.height().ceil().max(1.0) as u32;
        let (page, at) = self.reserve(images, width + GUTTER * 2, height + GUTTER * 2)?;
        let paper = &mut self.pages[page];
        outline.draw(|gx, gy, coverage| {
            let x = at[0] + GUTTER + gx;
            let y = at[1] + GUTTER + gy;
            if x >= PAGE || y >= PAGE {
                return;
            }
            let texel = ((y * PAGE + x) * 4) as usize;
            // **Raw coverage, deliberately not the byte-space compensation
            // the art uploads take.** The lift was tried first, on the
            // one-alpha-door argument, and it is wrong for glyphs: it turns
            // every antialiased edge texel markedly more opaque, so all text
            // reads a weight heavier and softly haloed — reported from the
            // screen as "bold and hazy" against the egui painter's, whose
            // text this pass is judged against. Coverage is a geometric
            // fraction, not art authored for a byte-space ROP.
            let alpha = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
            paper.pixels[texel] = 255;
            paper.pixels[texel + 1] = 255;
            paper.pixels[texel + 2] = 255;
            paper.pixels[texel + 3] = paper.pixels[texel + 3].max(alpha);
        });
        paper.dirty = true;
        Some(Sprite {
            page,
            at: [at[0] + GUTTER, at[1] + GUTTER],
            size: [width, height],
            bearing: [bounds.min.x, bounds.min.y],
        })
    }

    /// Shelf-pack a rectangle, opening a new page when the current one is
    /// full. A glyph wider than a page is refused, which cannot happen under
    /// [`vale_assets::interface::font::RASTER_CAP`].
    fn reserve(&mut self, images: &mut Assets<Image>, width: u32, height: u32) -> Option<(usize, [u32; 2])> {
        if width > PAGE || height > PAGE {
            return None;
        }
        loop {
            if let Some(index) = self.pages.len().checked_sub(1) {
                let page = &mut self.pages[index];
                if page.pen_x + width > PAGE {
                    page.pen_x = 0;
                    page.shelf_y += page.shelf_h;
                    page.shelf_h = 0;
                }
                if page.shelf_y + height <= PAGE {
                    let at = [page.pen_x, page.shelf_y];
                    page.pen_x += width;
                    page.shelf_h = page.shelf_h.max(height);
                    return Some((index, at));
                }
            }
            let image = images.add(blank_page());
            self.pages.push(Page {
                image,
                pixels: vec![0; (PAGE * PAGE * 4) as usize],
                pen_x: 0,
                shelf_y: 0,
                shelf_h: 0,
                dirty: false,
            });
        }
    }

    /// The page a sprite's quad binds.
    pub fn page_image(&self, page: usize) -> Option<Handle<Image>> {
        Some(self.pages.get(page)?.image.clone())
    }

    /// The uv rectangle of one sprite, in its page's 0..1.
    pub fn uv(sprite: &Sprite) -> ([f32; 2], [f32; 2]) {
        let side = PAGE as f32;
        (
            [sprite.at[0] as f32 / side, sprite.at[1] as f32 / side],
            [
                (sprite.at[0] + sprite.size[0]) as f32 / side,
                (sprite.at[1] + sprite.size[1]) as f32 / side,
            ],
        )
    }

    /// Push every page a rasterisation touched this rebuild back to the GPU.
    pub fn flush(&mut self, images: &mut Assets<Image>) {
        for page in &mut self.pages {
            if !page.dirty {
                continue;
            }
            page.dirty = false;
            if let Some(mut image) = images.get_mut(&page.image) {
                image.data = Some(page.pixels.clone());
            }
        }
    }
}

/// **One glyph's quad, on the device's own pixel grid.**
///
/// `pen` is the pen in *interface* pixels — the x of the glyph's origin and
/// the y of its baseline, both in the space the layout works in. `dpi` is the
/// window's scale factor: device pixels per interface pixel.
///
/// The sprite must have been rasterised at `size * dpi`, so its texels *are*
/// device pixels. This multiplies the pen up into that space, rounds there,
/// and divides the finished rectangle back down — so the quad lands on whole
/// device pixels and spans exactly as many of them as the sprite has texels.
/// One texel to one screen pixel: the sampler never interpolates and the
/// glyph is drawn as it was rasterised.
///
/// **Rounding in interface pixels instead is the bug this replaces.** At a
/// scale factor of 1.25 a glyph rastered at the interface size is stretched
/// by a quarter across the screen, and a linear filter spreads every edge
/// texel over two device pixels: text reads bolder than the reference's,
/// softly haloed, and letter to letter unevenly so, because each glyph's
/// stretch lands on a different phase of the device grid. That is what
/// "hazy, bold and blocky" was, and it is why the atlas is keyed by the
/// device size rather than the interface one.
pub fn quad(sprite: &Sprite, pen: [f32; 2], dpi: f32) -> [f32; 4] {
    // A window that reports nothing sensible draws at 1:1 rather than
    // dividing by zero — the same degradation everything here takes.
    let dpi = if dpi.is_finite() && dpi > 0.0 { dpi } else { 1.0 };
    let left = (pen[0] * dpi + sprite.bearing[0]).round();
    let top = (pen[1] * dpi + sprite.bearing[1]).round();
    [
        left / dpi,
        top / dpi,
        (left + sprite.size[0] as f32) / dpi,
        (top + sprite.size[1] as f32) / dpi,
    ]
}

/// The `PxScale` that draws an em of `em_px` pixels.
///
/// `PxScale` is relative to the face's ascent-minus-descent, not its em — the
/// conversion is `em_px * height_unscaled / units_per_em`, and getting it wrong
/// draws every face at its own slightly different wrong size.
fn em_scale(font: &ab_glyph::FontArc, em_px: f32) -> ab_glyph::PxScale {
    // A face that states no em treats the height as one — the request passes
    // through unscaled rather than failing, the same degradation everything
    // font-shaped here takes.
    let upem = font.units_per_em().unwrap_or_else(|| font.height_unscaled());
    ab_glyph::PxScale::from(em_px * font.height_unscaled() / upem.max(f32::EPSILON))
}

/// A page of nothing, sampled clamped so a quad at a page edge stays clean.
#[allow(unused_mut)]
fn blank_page() -> Image {
    let mut image = Image::new(
        Extent3d {
            width: PAGE,
            height: PAGE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (PAGE * PAGE * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..ImageSamplerDescriptor::default()
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(bearing: [f32; 2], size: [u32; 2]) -> Sprite {
        Sprite { page: 0, at: [0, 0], size, bearing }
    }

    /// **The quad spans exactly the sprite's texels in device pixels**, at
    /// every scale factor — which is the whole property that keeps a glyph
    /// crisp, since the sampler then reads one texel per pixel.
    #[test]
    fn a_glyph_covers_one_device_pixel_per_texel() {
        let glyph = sprite([1.0, -9.0], [7, 11]);
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            let [left, top, right, bottom] = quad(&glyph, [13.7, 42.3], dpi);
            let width = (right - left) * dpi;
            let height = (bottom - top) * dpi;
            assert!((width - 7.0).abs() < 1e-3, "{dpi}: {width}");
            assert!((height - 11.0).abs() < 1e-3, "{dpi}: {height}");
        }
    }

    /// …and it starts on a whole device pixel, which is the other half of it.
    #[test]
    fn a_glyph_starts_on_a_whole_device_pixel() {
        let glyph = sprite([0.5, -8.5], [5, 9]);
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            let [left, top, ..] = quad(&glyph, [13.7, 42.3], dpi);
            for edge in [left * dpi, top * dpi] {
                assert!((edge - edge.round()).abs() < 1e-3, "{dpi}: {edge}");
            }
        }
    }

    /// A window with no sensible scale factor draws at 1:1 rather than
    /// producing infinities.
    #[test]
    fn a_nonsense_scale_factor_falls_back_to_one() {
        let glyph = sprite([0.0, -8.0], [4, 8]);
        let plain = quad(&glyph, [10.0, 20.0], 1.0);
        for dpi in [0.0, -2.0, f32::NAN] {
            assert_eq!(quad(&glyph, [10.0, 20.0], dpi), plain, "{dpi}");
        }
    }
}
