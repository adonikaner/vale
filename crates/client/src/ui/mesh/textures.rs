//! **The interface's art as Bevy images** — the mesh painter's counterpart to
//! [`crate::ui::framexml::Art`]'s egui texture cache.
//!
//! Same shape, same rules, different destination: a path is decoded once
//! through the one BLP door ([`crate::ui::framexml::decode_rgba`]), its alpha
//! goes through the same byte-space compensation every egui upload takes, and
//! the answer is memoised — a miss is cached as a miss, because the directory
//! names art the archives do not always carry.
//!
//! What this cache does **not** have is the egui path's additive
//! preprocessing: an `ADD` region here really is drawn additively
//! ([`super::material::Blend::Additive`]), so the texture is uploaded as
//! painted.

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
    /// The eight pieces of an edge strip — runs repeat, corners clamp, exactly
    /// as [`crate::ui::framexml::Art::edge_pieces`] cuts them and for the same
    /// seam reasons.
    edges: HashMap<String, Option<Vec<Handle<Image>>>>,
    /// One minimap tile picture per path, with the tick it was last drawn on —
    /// the same bounded cache `Art::minimap` keeps, for the same reason: four
    /// are visible and a continent has 687.
    minimap: HashMap<String, (Handle<Image>, u64)>,
    pub(super) minimap_tick: u64,
    white: Option<Handle<Image>>,
}

/// How many minimap tiles stay resident — [`crate::ui::framexml`] keeps the
/// same sixteen.
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
    pub fn tiled(
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
    pub fn edge_pieces(
        &mut self,
        images: &mut Assets<Image>,
        assets: &GameAssets,
        path: &str,
    ) -> Option<&Vec<Handle<Image>>> {
        if !self.edges.contains_key(path) {
            let cut = crate::ui::framexml::decode_rgba(assets, path).and_then(
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

    /// One minimap tile, held in the bounded cache. A path the index named and
    /// the archive lacks is **not** cached as an absence — see
    /// `Art::minimap_texture`, whose rule this is.
    pub fn minimap_texture(
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
    /// For a host that has changed the bytes the path reads back as — the
    /// world editor redrawing a tile's picture. The cache is keyed by the
    /// path the index names and kept for the life of the process, so without
    /// this a picture redrawn after the first playtest was never seen again.
    /// Matched without case, since the index and the writer spell the
    /// directory differently.
    pub fn forget_minimap(&mut self, path: &str) {
        self.minimap.retain(|key, _| !key.eq_ignore_ascii_case(path));
    }

    /// …and all of them, for a host that has changed more than it can name.
    pub fn forget_all_minimaps(&mut self) {
        self.minimap.clear();
    }

    /// Drop all but the [`MINIMAP_CACHE`] most recently drawn tiles.
    pub fn trim_minimap(&mut self) {
        while self.minimap.len() > MINIMAP_CACHE {
            let Some(oldest) = self
                .minimap
                .iter()
                .min_by_key(|(_, (_, seen))| *seen)
                .map(|(path, _)| path.clone())
            else {
                return;
            };
            self.minimap.remove(&oldest);
        }
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

/// Decode one BLP and upload it.
fn upload(
    images: &mut Assets<Image>,
    assets: &GameAssets,
    path: &str,
    mode: ImageAddressMode,
) -> Option<Handle<Image>> {
    let (width, height, mut rgba) = crate::ui::framexml::decode_rgba(assets, path)?;
    Some(images.add(image([width, height], &mut rgba, mode)))
}

/// The one door every upload goes through — the same byte-space alpha
/// compensation [`crate::ui::framexml::image`] applies, so a panel drawn by
/// either painter is exactly as solid as the other's.
fn image(size: [u32; 2], pixels: &mut [u8], mode: ImageAddressMode) -> Image {
    for texel in pixels.chunks_exact_mut(4) {
        let alpha = f32::from(texel[3]) / 255.0;
        texel[3] = (crate::ui::framexml::byte_space_alpha(alpha) * 255.0).round() as u8;
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
