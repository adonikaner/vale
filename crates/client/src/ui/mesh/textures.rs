//! The interface's art as Bevy images.
//!
//! A path is decoded once through [`decode_rgba`], its alpha goes through the
//! byte-space compensation ([`super::material::byte_space_alpha`]), and the
//! answer is kept. A miss is kept as a miss, so a path the archives do not
//! have is tried once and skipped for the rest of the session rather than
//! read again on every rebuild. For a partly working interface a missing path
//! is common.
//!
//! An `ADD` region is drawn additively ([`super::material::Blend::Additive`]),
//! so its texture is uploaded as painted.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

use crate::assets::GameAssets;

/// Every picture the mesh painter has uploaded, by path — and the 1x1 white
/// square every solid fill is a tint of.
#[derive(Resource, Default)]
pub struct UiTextures {
    /// Clamped, linear — the ordinary case.
    plain: HashMap<String, Option<Handle<Image>>>,
    /// Repeating, for a backdrop's tiled fill.
    tiled: HashMap<String, Option<Handle<Image>>>,
    /// A backdrop's `edgeFile`, already cut into its eight upright pieces; see
    /// [`vale_assets::interface::backdrop`]. One decode per file rather than
    /// one per rebuild per piece.
    ///
    /// The four runs wrap and the four corners clamp. A run tiles along its own
    /// side at the cell's period, so it has to repeat. A corner maps `[0, 1]`
    /// exactly once; sampled with wrapping, linear filtering bleeds the
    /// opposite edge in and leaves a one-texel seam at every corner of every
    /// panel.
    edges: HashMap<String, Option<Vec<Handle<Image>>>>,
    /// The world's minimap pictures, with the tick each was last drawn on, in
    /// a bounded cache apart from [`Self::plain`].
    ///
    /// Every other texture here is an `Interface\` file: there are a few
    /// hundred, each is small, and a session touches most of them in its first
    /// minute, so they are never evicted. Minimap tiles differ. Each is
    /// 256x256 RGBA (256 KB resident), and there is one per map tile: a
    /// character who crosses Azeroth would load all 687 of its tiles, which in
    /// an unbounded map is 176 MB of texture. Four tiles are visible at the
    /// widest zoom, so the rest of [`MINIMAP_CACHE`] is hysteresis: a character
    /// walking back and forth across a tile seam reuses tiles rather than
    /// decoding them again.
    minimap: HashMap<String, (Handle<Image>, u64)>,
    pub(super) minimap_tick: u64,
    white: Option<Handle<Image>>,
}

/// How many minimap tile pictures to keep. Sixteen is 4 MB resident. See
/// [`UiTextures::minimap`].
const MINIMAP_CACHE: usize = 16;

impl UiTextures {
    /// The plain upload: clamped, linear-filtered.
    pub fn texture(
        &mut self,
        images: &mut Assets<Image>,
        assets: &GameAssets,
        path: &str,
    ) -> Option<Handle<Image>> {
        if !self.plain.contains_key(path) {
            let loaded = upload(images, assets, path, ImageAddressMode::ClampToEdge);
            self.plain.insert(path.to_string(), loaded);
        }
        self.plain.get(path)?.clone()
    }

    /// The same picture with a repeating sampler, for uv past 1.
    pub(crate) fn tiled(
        &mut self,
        images: &mut Assets<Image>,
        assets: &GameAssets,
        path: &str,
    ) -> Option<Handle<Image>> {
        if !self.tiled.contains_key(path) {
            let loaded = upload(images, assets, path, ImageAddressMode::Repeat);
            self.tiled.insert(path.to_string(), loaded);
        }
        self.tiled.get(path)?.clone()
    }

    /// The eight pieces of an edge strip, cut on first use.
    pub(crate) fn edge_pieces(
        &mut self,
        images: &mut Assets<Image>,
        assets: &GameAssets,
        path: &str,
    ) -> Option<&Vec<Handle<Image>>> {
        if !self.edges.contains_key(path) {
            let cut = decode_rgba(assets, path).and_then(
                |(width, height, rgba)| {
                    let cells = vale_assets::interface::backdrop::split_edges(
                        width as usize,
                        height as usize,
                        &rgba,
                    )?;
                    let size = height as usize;
                    Some(
                        cells
                            .into_iter()
                            .enumerate()
                            .map(|(piece, mut pixels)| {
                                let mode = if vale_assets::interface::backdrop::tiles(piece) {
                                    ImageAddressMode::Repeat
                                } else {
                                    ImageAddressMode::ClampToEdge
                                };
                                images.add(image([size as u32, size as u32], &mut pixels, mode))
                            })
                            .collect::<Vec<_>>(),
                    )
                },
            );
            self.edges.insert(path.to_string(), cut);
        }
        self.edges.get(path)?.as_ref()
    }

    /// One minimap tile, held in the bounded cache.
    ///
    /// A path the index named and the archive does not have is not cached as
    /// an absence: the index resolves 2,356 of 2,356 in the shipped chain, so a
    /// miss here means a broken install, not the common case the plain cache's
    /// kept misses are for.
    pub(crate) fn minimap_texture(
        &mut self,
        images: &mut Assets<Image>,
        assets: &GameAssets,
        path: &str,
    ) -> Option<Handle<Image>> {
        let tick = self.minimap_tick;
        if let Some((handle, seen)) = self.minimap.get_mut(path) {
            *seen = tick;
            return Some(handle.clone());
        }
        let handle = upload(images, assets, path, ImageAddressMode::ClampToEdge)?;
        self.minimap.insert(path.to_string(), (handle.clone(), tick));
        Some(handle)
    }

    /// **Forget one minimap picture**, so the next draw reads its path again.
    ///
    /// For a host that has changed the bytes the path reads back as, such as
    /// a caller that rewrote a tile and redrew its picture. The cache is
    /// keyed by the path the index names and kept for the life of the process,
    /// so without this a picture redrawn after the first playtest was never
    /// seen again. Matched without case, since the index and the writer spell
    /// the directory differently.
    pub(crate) fn forget_minimap(&mut self, path: &str) {
        self.minimap.retain(|key, _| !key.eq_ignore_ascii_case(path));
    }

    /// Forget every picture read from the file at `path`, in any of the
    /// maps, so the next draw reads the file again. An interface path is
    /// written without `.blp` and the client adds it, so a path matches with
    /// or without one, and without case. What is drawn already keeps its
    /// handle until the tree is rebuilt.
    pub(crate) fn forget(&mut self, path: &str) {
        let stem = |key: &str| {
            match key.len() >= 4 && key[key.len() - 4..].eq_ignore_ascii_case(".blp") {
                true => key[..key.len() - 4].to_ascii_lowercase(),
                false => key.to_ascii_lowercase(),
            }
        };
        let wanted = stem(path);
        self.plain.retain(|key, _| stem(key) != wanted);
        self.tiled.retain(|key, _| stem(key) != wanted);
        self.edges.retain(|key, _| stem(key) != wanted);
        self.forget_minimap(path);
    }

    /// …and every picture, for a host that has changed more than it can name.
    pub(crate) fn forget_all(&mut self) {
        self.plain.clear();
        self.tiled.clear();
        self.edges.clear();
        self.minimap.clear();
    }

    /// Drop all but the [`MINIMAP_CACHE`] most recently drawn tiles, never one
    /// drawn in the current pass. Inside a building one pass can draw more
    /// than sixteen pictures, and evicting those would decode them again on
    /// every rebuild.
    pub(crate) fn trim_minimap(&mut self) {
        while self.minimap.len() > MINIMAP_CACHE {
            let Some((oldest, seen)) = self
                .minimap
                .iter()
                .min_by_key(|(_, (_, seen))| *seen)
                .map(|(path, (_, seen))| (path.clone(), *seen))
            else {
                return;
            };
            if seen == self.minimap_tick {
                return;
            }
            self.minimap.remove(&oldest);
        }
    }

    /// How many uploads the cache holds: every decoded picture, each edge
    /// strip's eight pieces and the minimap tiles. The HUD line reports it.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn held(&self) -> usize {
        let present = |map: &HashMap<String, Option<Handle<Image>>>| {
            map.values().filter(|handle| handle.is_some()).count()
        };
        present(&self.plain)
            + present(&self.tiled)
            + self.edges.values().flatten().map(Vec::len).sum::<usize>()
            + self.minimap.len()
    }

    /// The 1x1 white square a solid fill tints.
    pub fn white(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        if let Some(white) = &self.white {
            return white.clone();
        }
        let mut pixels = [255u8; 4];
        let handle = images.add(image([1, 1], &mut pixels, ImageAddressMode::ClampToEdge));
        self.white = Some(handle.clone());
        handle
    }
}

/// Forget the pictures a [`crate::assets::PathsChanged`] names: an
/// interface texture replaced, a minimap picture drawn again.
pub(crate) fn forget_changed_textures(
    mut changed: MessageReader<crate::assets::PathsChanged>,
    mut textures: ResMut<UiTextures>,
) {
    for change in changed.read() {
        if change.all {
            textures.forget_all();
            continue;
        }
        for path in &change.paths {
            textures.forget(path);
        }
    }
}

/// One `Interface\` path, decoded to RGBA8: the archive read every texture in
/// the interface starts with, the loading screen's included.
///
/// A path such as `Interface\Buttons\UI-Quickslot2` has no extension, because
/// the files do not write one; the client appends `.blp`. A path that already
/// has one, as `SetTexture` from a script sometimes passes, is left alone.
pub(crate) fn decode_rgba(assets: &GameAssets, path: &str) -> Option<(u32, u32, Vec<u8>)> {
    // A mask: the file after the prefix, with every texel made white and its
    // alpha kept. The tabard designer draws an emblem's shape this way; see
    // [`crate::lua::panels::tabard::ALPHA_MASK_PREFIX`].
    if let Some(file) = path.strip_prefix(crate::lua::panels::tabard::ALPHA_MASK_PREFIX) {
        let (width, height, mut pixels) = decode_rgba(assets, file)?;
        for texel in pixels.chunks_exact_mut(4) {
            texel[..3].fill(255);
        }
        return Some((width, height, pixels));
    }
    // `.blp` first, then `.tga`: the order the 1.12.1 client tries for a path
    // without an extension. Every texture in the archives is BLP; an addon's
    // art is almost always TGA (45 of pfUI's 46). A path that names its own
    // extension is read as written.
    let lower = path.to_ascii_lowercase();
    let candidates: Vec<String> = if lower.ends_with(".blp") || lower.ends_with(".tga") {
        vec![path.to_string()]
    } else {
        vec![format!("{path}.blp"), format!("{path}.tga")]
    };
    for file in candidates {
        let decoded = assets
            .with_archive(|archive| {
                let raw = archive.read(&file).map_err(|e| e.to_string())?;
                if file.to_ascii_lowercase().ends_with(".tga") {
                    let tga = vale_assets::world::tga::decode(&raw).map_err(|e| e.to_string())?;
                    Ok((tga.width, tga.height, tga.rgba))
                } else {
                    let blp = vale_assets::world::blp::decode(&raw).map_err(|e| e.to_string())?;
                    Ok((blp.width, blp.height, blp.rgba))
                }
            })
            .ok();
        if decoded.is_some() {
            return decoded;
        }
    }
    None
}

/// Decode one picture and upload it.
fn upload(
    images: &mut Assets<Image>,
    assets: &GameAssets,
    path: &str,
    mode: ImageAddressMode,
) -> Option<Handle<Image>> {
    let (width, height, mut rgba) = decode_rgba(assets, path)?;
    Some(images.add(image([width, height], &mut rgba, mode)))
}

/// The one door every upload goes through. It applies
/// [`super::material::byte_space_alpha`] to every texel, so a panel's fill,
/// its border pieces and its icons are compensated alike. Applied to some
/// uploads and not others, a fill and its border would differ in opacity,
/// which looks like a property of the art rather than a bug.
fn image(size: [u32; 2], pixels: &mut [u8], mode: ImageAddressMode) -> Image {
    for texel in pixels.chunks_exact_mut(4) {
        let alpha = f32::from(texel[3]) / 255.0;
        texel[3] = (super::material::byte_space_alpha(alpha) * 255.0).round() as u8;
    }

    let mut image = Image::new(
        Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels.to_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: mode,
        address_mode_v: mode,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..ImageSamplerDescriptor::default()
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An interface picture is forgotten by its path with or without `.blp`
    /// and without case, and a minimap picture by its path.
    #[test]
    fn a_picture_is_forgotten_with_or_without_its_extension() {
        let mut textures = UiTextures::default();
        textures.plain.insert("Interface\\Icons\\INV_Misc_QuestionMark".to_string(), None);
        textures.tiled.insert("Interface\\Icons\\INV_Misc_QuestionMark.blp".to_string(), None);
        textures.plain.insert("Interface\\Buttons\\UI-Quickslot2".to_string(), None);
        textures.forget("interface\\icons\\inv_misc_questionmark.blp");
        assert!(textures.plain.len() == 1 && textures.tiled.is_empty());
        textures.forget_all();
        assert!(textures.plain.is_empty());
    }
}
