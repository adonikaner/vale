//! The pictures a panel draws: an archive texture, small enough to put in a row
//! of a list.
//!
//! ## A list of 621 names is not a way to choose a texture
//!
//! The tileset picker offered `elwynn\grassbase.blp` against
//! `elwynn\grassdark.blp` against `elwynn\grasslight.blp`, which is not a choice
//! anybody can make by reading. The list needs to show the thing.
//!
//! ## …and decoding 621 of them is not a way to draw one
//!
//! A tileset is a 256x256 DXT image, so decoding the catalogue eagerly is 621
//! archive reads and 40 MB of RGBA before the panel has drawn a row. Three
//! things keep it to nothing:
//!
//! * **only the visible rows ask.** The list is drawn through
//!   `ScrollArea::show_rows`, so the eight or so rows actually on screen are the
//!   only ones that request a picture — scrolling requests the next eight.
//! * **the mip chain is already in the file.** A thumbnail is [`SIDE`] pixels
//!   and Blizzard authored a 32x32 level of every tileset, so the decode picks
//!   the smallest level that is big enough instead of decoding the top one and
//!   throwing 98% of it away.
//! * **a few a frame.** [`BUDGET`] decodes per frame, which makes a scroll fill
//!   in over three or four frames rather than stalling one.
//!
//! Cached for the session and never evicted: the whole catalogue at this size is
//! 2.5 MB, which is less than one tile's alpha atlas.
//!
//! ## Why it is here and not in the tool
//!
//! `tools/` is what the pointer does to the files. Nothing in this file changes
//! anything — it exists so that a panel can show what it is offering, and it
//! would be the same code for a doodad picker, which is the next thing that
//! wants it.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiTextureHandle};

use vale_client::assets::GameAssets;

/// How many pixels a thumbnail is on a side.
///
/// 32, which is a level every 256x256 tileset already has — so the usual decode
/// is the file's own bytes at the size they are wanted, with no resampling at
/// all. See [`thumbnail`], which resamples only when a texture's chain does not
/// reach this.
pub const SIDE: u32 = 32;

/// …and the larger size a picker's grid draws an icon at. A spell icon is a
/// 64x64 BLP, so this is its top level at its own size, with no resampling;
/// at [`SIDE`] it is the file's next level down, which is what a list row
/// wants and a grid of them does not.
pub const ICON_SIDE: u32 = 64;

/// How many are decoded in one frame.
///
/// Four. A tileset at this size is a DXT block decode of a 32x32 level and a
/// handful of microseconds; the budget is here so that a fast scroll through the
/// whole catalogue cannot ask for six hundred of them in one frame.
const BUDGET: usize = 4;

/// The decoded previews, and what has been asked for.
///
/// **Keyed by the path and the size**, because one picture is wanted at two
/// sizes by two panels — the list's row at [`SIDE`] and the picker's grid at
/// [`ICON_SIDE`] — and a texture is one size once it is uploaded.
#[derive(Resource, Default)]
pub struct Thumbnails {
    ready: HashMap<(String, u32), egui::TextureId>,
    /// Asked for and not yet decoded, oldest first — so a scroll that runs past
    /// a row does not leave it ahead of the row somebody stopped on.
    wanted: Vec<(String, u32)>,
    /// **What could not be decoded, so it is not asked for again.** Without it a
    /// texture the archives do not answer for is one archive miss per frame for
    /// as long as its row is on screen.
    failed: HashSet<(String, u32)>,
}

impl Thumbnails {
    /// What egui draws this path by at [`SIDE`], if it has been decoded.
    pub fn get(&self, path: &str) -> Option<egui::TextureId> {
        self.get_at(path, SIDE)
    }

    /// …and at any size.
    pub fn get_at(&self, path: &str, side: u32) -> Option<egui::TextureId> {
        self.ready.get(&(path.to_ascii_lowercase(), side)).copied()
    }

    /// Ask for one at [`SIDE`]. Cheap and idempotent: a row calls this every
    /// frame it is drawn.
    pub fn want(&mut self, path: &str) {
        self.want_at(path, SIDE);
    }

    /// …and at any size.
    pub fn want_at(&mut self, path: &str, side: u32) {
        let key = (path.to_ascii_lowercase(), side);
        if self.ready.contains_key(&key) || self.failed.contains(&key) {
            return;
        }
        if !self.wanted.contains(&key) {
            self.wanted.push(key);
        }
    }
}

pub struct ThumbnailPlugin;

impl Plugin for ThumbnailPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Thumbnails>()
            .add_systems(Update, decode);
    }
}

/// Decode up to [`BUDGET`] of the asked-for textures.
fn decode(
    mut thumbnails: ResMut<Thumbnails>,
    assets: Res<GameAssets>,
    mut images: ResMut<Assets<Image>>,
    mut contexts: EguiContexts,
) {
    if thumbnails.wanted.is_empty() {
        return;
    }
    let take = thumbnails.wanted.len().min(BUDGET);
    let taking: Vec<(String, u32)> = thumbnails.wanted.drain(..take).collect();
    for key in taking {
        let (path, side) = &key;
        let bytes = assets.with_archive(|chain| Ok(chain.read(path).ok()));
        let Ok(Some(bytes)) = bytes else {
            thumbnails.failed.insert(key);
            continue;
        };
        let Some(rgba) = thumbnail(&bytes, *side) else {
            thumbnails.failed.insert(key);
            continue;
        };
        let handle = images.add(image_of(rgba, *side));
        // **A strong handle**, so egui owns it and the asset lives as long as
        // the cache does. A weak one would leave the id naming an image nothing
        // holds, which draws as whatever the atlas has there now.
        let id = contexts.add_image(EguiTextureHandle::Strong(handle));
        thumbnails.ready.insert(key, id);
    }
}

/// One BLP as `side` by `side` RGBA.
///
/// **The smallest level that is still big enough**, which for a 256x256 tileset
/// at [`SIDE`] is the 32x32 one the file already carries. Only a texture whose
/// chain stops above the wanted size is resampled, and then by a box filter
/// down to it — the same filter `alpha_atlas_mips` uses and for the same
/// reason, that every channel here is its own thing.
///
/// A non-square texture is squashed rather than cropped. Tilesets are square, so
/// this is about the day this is pointed at something else.
fn thumbnail(bytes: &[u8], side: u32) -> Option<Vec<u8>> {
    let blp = vale_assets::world::blp::decode_mipped(bytes).ok()?;
    let (mut level, mut size) = (0usize, blp.level_size(0));
    for (i, _) in blp.levels().enumerate() {
        let (w, h) = blp.level_size(i);
        if w.min(h) >= side {
            level = i;
            size = (w, h);
        }
    }
    let src = blp.levels().nth(level)?;
    Some(resample(src, size.0, size.1, side))
}

/// A box-filtered resample to `side` square.
///
/// The identity when the source is already that size, which is the usual case.
fn resample(src: &[u8], width: u32, height: u32, side: u32) -> Vec<u8> {
    if width == side && height == side && src.len() == (side * side * 4) as usize {
        return src.to_vec();
    }
    let mut out = vec![0u8; (side * side * 4) as usize];
    for y in 0..side {
        for x in 0..side {
            // The source box this output texel covers, clamped to at least one
            // texel so an upscale still reads something.
            let x0 = x * width / side;
            let x1 = (((x + 1) * width) / side).max(x0 + 1).min(width);
            let y0 = y * height / side;
            let y1 = (((y + 1) * height) / side).max(y0 + 1).min(height);
            let mut sum = [0u32; 4];
            let mut taps = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = ((sy * width + sx) * 4) as usize;
                    let Some(texel) = src.get(at..at + 4) else {
                        continue;
                    };
                    for channel in 0..4 {
                        sum[channel] += u32::from(texel[channel]);
                    }
                    taps += 1;
                }
            }
            if taps == 0 {
                continue;
            }
            let at = ((y * side + x) * 4) as usize;
            for channel in 0..4 {
                out[at + channel] = (sum[channel] / taps) as u8;
            }
        }
    }
    out
}

/// …as an image egui can be handed.
///
/// `Rgba8UnormSrgb` and `RenderAssetUsages::all()`: these are colour, unlike the
/// blend atlas, and egui's own pipeline wants them in the render world while the
/// asset stays in the main one so nothing drops it after upload.
fn image_of(rgba: Vec<u8>, side: u32) -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source already at the wanted size is handed back untouched, which is
    /// what a 256x256 tileset's own 32x32 level is.
    #[test]
    fn a_source_at_the_wanted_size_is_not_resampled() {
        let src: Vec<u8> = (0..(SIDE * SIDE * 4)).map(|i| (i % 251) as u8).collect();
        assert_eq!(resample(&src, SIDE, SIDE, SIDE), src);
    }

    /// …and one that is larger is averaged down to it, with the size exact.
    #[test]
    fn a_larger_source_is_averaged_down() {
        let side = SIDE * 4;
        // Two halves, so the average of each output texel is knowable.
        let src: Vec<u8> = (0..(side * side))
            .flat_map(|i| {
                let left = (i % side) < side / 2;
                match left {
                    true => [10u8, 20, 30, 255],
                    false => [200, 210, 220, 255],
                }
            })
            .collect();
        let out = resample(&src, side, side, SIDE);
        assert_eq!(out.len(), (SIDE * SIDE * 4) as usize);
        assert_eq!(&out[0..4], &[10, 20, 30, 255], "the left half");
        let right = ((SIDE - 1) * 4) as usize;
        assert_eq!(&out[right..right + 4], &[200, 210, 220, 255], "the right");
    }

    /// **A path that will not decode is asked for once.** Without that, a
    /// texture the archives do not answer for is an archive miss every frame for
    /// as long as its row is on screen.
    #[test]
    fn a_failed_path_is_not_asked_for_again() {
        let mut thumbnails = Thumbnails::default();
        thumbnails.want("Tileset\\Nowhere\\Missing.blp");
        assert_eq!(thumbnails.wanted.len(), 1);
        // …and asking twice before it is decoded queues it once.
        thumbnails.want("tileset\\nowhere\\missing.blp");
        assert_eq!(thumbnails.wanted.len(), 1, "the queue is a set");

        thumbnails.wanted.clear();
        thumbnails
            .failed
            .insert(("tileset\\nowhere\\missing.blp".into(), SIDE));
        thumbnails.want("Tileset\\Nowhere\\Missing.blp");
        assert!(thumbnails.wanted.is_empty(), "a failure is remembered");
        assert_eq!(thumbnails.get("Tileset\\Nowhere\\Missing.blp"), None);
        // …and the same path at another size is another picture, asked for on
        // its own terms.
        thumbnails.want_at("Tileset\\Nowhere\\Missing.blp", ICON_SIDE);
        assert_eq!(
            thumbnails.wanted.len(),
            1,
            "a size that has not failed is asked for"
        );
    }
}
